use serde::{Deserialize, Serialize};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub static SIGNAL_CANCELLED: AtomicBool = AtomicBool::new(false);

static CANCELLATION: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

/// Request cancellation of the current app-owned operation, including lock cleanup.
/// Does not signal unrelated restic processes or operations from earlier app instances.
pub fn cancel() -> Result<(), String> {
    let state = CANCELLATION.lock().map_err(|e| e.to_string())?;
    if let Some(cancelled) = state.as_ref() {
        cancelled.store(true, Ordering::SeqCst);
    }
    Ok(())
}
use std::{
    path::Path,
    process::{Command, Stdio},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Maintenance {
    pub keep_last: u32,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    pub read_data: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Deserialize the frontend command payload using camelCase field names. repository, password, passwordFile, and operation are required; other fields default to empty strings, empty arrays, false, or zero. See the operation table for conditional requirements.
pub struct Request {
    pub repository: String,
    pub password: String,
    pub password_file: String,
    pub operation: String,
    #[serde(default)]
    pub maintenance: Option<Maintenance>,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub excludes: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub snapshot: String,
    /// Absolute snapshot directory to browse; empty means the snapshot root.
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub include: String,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub read_data: bool,
    #[serde(default)]
    pub keep_last: u32,
    #[serde(default)]
    pub keep_daily: u32,
    #[serde(default)]
    pub keep_weekly: u32,
    #[serde(default)]
    pub keep_monthly: u32,
    #[serde(default)]
    pub confirmation: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
/// Captured stdout and stderr as UTF-8 strings, replacing invalid bytes, plus the process exit code. A missing exit code is represented by -1. A nonzero exit is returned as output rather than a Rust error.
pub struct Output {
    pub stdout: String,
    pub stderr: String,
    pub code: i32,
}

/// Validate a snapshot identifier as 8 through 64 ASCII hexadecimal characters. Return a user-facing error for invalid identifiers.
fn snapshot(value: &str) -> Result<(), String> {
    if (8..=64).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err("Select a valid snapshot first.".into())
    }
}

/// Validate an allowlisted operation and construct separate restic arguments without a shell. Returns a user-facing error for unsupported operations or invalid credentials, paths, snapshot IDs, retention values, or confirmations. Performs filesystem checks for backup and restore paths.
pub fn arguments(r: &Request) -> Result<Vec<String>, String> {
    if r.operation == "version" {
        return Ok(vec!["version".into()]);
    }
    if r.repository.trim().is_empty() {
        return Err("Repository location is required.".into());
    }
    if r.password.is_empty() && r.password_file.trim().is_empty() {
        return Err("Enter a password or password-file path.".into());
    }
    let mut a = vec!["--repo".into(), r.repository.clone(), "--json".into()];
    if !r.password_file.is_empty() {
        a.extend(["--password-file".into(), r.password_file.clone()]);
    }
    if r.repository.starts_with("sftp:") {
        // Transport authentication is separate from the restic encryption password.
        // Keep these options in the shared builder so cleanup and scheduled jobs use them too.
        a.extend([
            "-o".into(),
            "sftp.args=-oBatchMode=yes -oStrictHostKeyChecking=yes -oNumberOfPasswordPrompts=0"
                .into(),
        ]);
    }
    a.push(r.operation.clone());
    match r.operation.as_str() {
        "init" | "snapshots" | "stats" | "unlock" => {}
        "backup" => {
            if r.paths.is_empty() {
                return Err("Add at least one source path.".into());
            }
            for path in &r.paths {
                if !Path::new(path).is_absolute() || !Path::new(path).exists() {
                    return Err(format!("Source must be an existing absolute path: {path}"));
                }
            }
            for exclude in &r.excludes {
                a.extend(["--exclude".into(), exclude.clone()]);
            }
            for tag in &r.tags {
                a.extend(["--tag".into(), tag.clone()]);
            }
            if r.dry_run {
                a.push("--dry-run".into());
            }
            a.push("--".into());
            a.extend(r.paths.clone());
        }
        "ls" => {
            snapshot(&r.snapshot)?;
            let directory = if r.directory.is_empty() {
                "/"
            } else {
                &r.directory
            };
            if !directory.starts_with('/')
                || directory.contains('\0')
                || directory.split('/').any(|part| part == "..")
            {
                return Err("Snapshot directory must be an absolute snapshot path without parent traversal.".into());
            }
            // An explicit directory limits ls to its immediate children; omit --recursive.
            a.extend([r.snapshot.clone(), directory.into()]);
        }
        "restore" => {
            snapshot(&r.snapshot)?;
            let p = Path::new(&r.target);
            if !p.is_absolute() {
                return Err("Restore destination must be an absolute path.".into());
            }
            if p.exists()
                && (!p.is_dir() || p.read_dir().map_err(|e| e.to_string())?.next().is_some())
            {
                return Err("Restore destination must be a new or empty directory.".into());
            }
            a.extend([
                r.snapshot.clone(),
                "--target".into(),
                r.target.clone(),
                "--verify".into(),
                "--overwrite".into(),
                "never".into(),
            ]);
            if !r.include.is_empty() {
                a.extend(["--include".into(), r.include.clone()]);
            }
        }
        "check" => {
            if r.read_data {
                a.push("--read-data".into());
            }
        }
        "forget" => {
            if [r.keep_last, r.keep_daily, r.keep_weekly, r.keep_monthly]
                .iter()
                .all(|n| *n == 0)
            {
                return Err("Keep at least one snapshot interval.".into());
            }
            for (flag, n) in [
                ("--keep-last", r.keep_last),
                ("--keep-daily", r.keep_daily),
                ("--keep-weekly", r.keep_weekly),
                ("--keep-monthly", r.keep_monthly),
            ] {
                if n > 10000 {
                    return Err("Retention counts must be at most 10000.".into());
                }
                if n > 0 {
                    a.extend([flag.into(), n.to_string()]);
                }
            }
            if r.dry_run {
                a.push("--dry-run".into());
            } else if r.confirmation != "FORGET" {
                return Err("Type FORGET to remove snapshots.".into());
            }
        }
        "prune" => {
            if r.confirmation != "PRUNE" {
                return Err("Type PRUNE to reclaim unused data.".into());
            }
        }
        _ => return Err("Unsupported restic operation.".into()),
    }
    Ok(a)
}

/// Acquire a process-wide nonblocking job lock, validate arguments, and run restic from PATH with stdin closed. Remove inherited RESTIC_PASSWORD_COMMAND, RESTIC_PASSWORD_FILE, and RESTIC_PASSWORD; set the request password only when no password file is supplied. Run stale-only unlock before existing-repository operations, aborting if cleanup fails. Capture operation output until exit. Validation, lock, and process launch failures return string errors; nonzero process exits remain Output values.
pub fn execute(r: Request) -> Result<Output, String> {
    // Keep the IPC guard as well as the cross-process lock.
    static JOB: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = JOB
        .try_lock()
        .map_err(|_| "Another restic operation is running.".to_string())?;
    let cancelled = Arc::new(AtomicBool::new(false));
    *CANCELLATION.lock().map_err(|e| e.to_string())? = Some(cancelled.clone());
    let result = (|| {
        #[cfg(target_os = "linux")]
        let _process_lock =
            crate::coordination::acquire(&crate::scheduler::operation_lock()?, false, &cancelled)?;
        execute_with(&r, |request, args| {
            run_command(request, args, &cancelled, Path::new("restic"))
        })
    })();
    *CANCELLATION.lock().map_err(|e| e.to_string())? = None;
    result
}

/// Validate before modifying the repository, then remove only locks restic considers stale.
/// Skip cleanup for initialization and version queries; abort if cleanup fails.
pub(crate) fn execute_with(
    r: &Request,
    mut run: impl FnMut(&Request, Vec<String>) -> Result<Output, String>,
) -> Result<Output, String> {
    let args = arguments(r)?;
    let mut maintenance = Vec::new();
    if r.operation == "backup"
        && !r.dry_run
        && let Some(policy) = &r.maintenance
    {
        for operation in ["check", "forget", "prune"] {
            let mut step = r.clone();
            step.operation = operation.into();
            step.maintenance = None;
            step.read_data = policy.read_data;
            step.keep_last = policy.keep_last;
            step.keep_daily = policy.keep_daily;
            step.keep_weekly = policy.keep_weekly;
            step.keep_monthly = policy.keep_monthly;
            step.confirmation = if operation == "forget" {
                "FORGET"
            } else {
                "PRUNE"
            }
            .into();
            arguments(&step)?; // Validate the complete policy before creating a snapshot.
            maintenance.push(step);
        }
    }
    if !matches!(r.operation.as_str(), "init" | "version" | "unlock") {
        let mut cleanup = r.clone();
        cleanup.operation = "unlock".into();
        let unlocked = run(&cleanup, arguments(&cleanup)?)?;
        if unlocked.code == 130 {
            return Ok(unlocked);
        }
        if unlocked.code != 0 {
            return Err(format!(
                "Could not remove stale repository locks (exit {}). Operation cancelled.\n{}\n{}",
                unlocked.code, unlocked.stdout, unlocked.stderr
            ));
        }
    }
    let mut result = run(r, args)?;
    for step in maintenance {
        if result.code != 0 {
            break;
        }
        let next = match run(&step, arguments(&step)?) {
            Ok(output) => output,
            Err(error) => Output {
                stdout: String::new(),
                stderr: error,
                code: 1,
            },
        };
        result.stdout.push_str(&format!(
            "\nAutomatic maintenance: {}\n{}",
            step.operation, next.stdout
        ));
        result.stderr.push_str(&next.stderr);
        result.code = next.code;
        if next.code != 0 {
            result.stderr.push_str(&format!("\nBackup completed; automatic {} failed or was cancelled. Remaining maintenance skipped.\n", step.operation));
        }
    }
    Ok(result)
}

/// Run one restic process with the request credentials and capture its output.
pub(crate) fn run_command(
    r: &Request,
    args: Vec<String>,
    cancelled: &AtomicBool,
    executable: &Path,
) -> Result<Output, String> {
    let mut command = Command::new(executable);
    command.args(args).stdin(Stdio::null());
    // Do not accidentally run a password command inherited from the launching shell.
    command
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .env_remove("RESTIC_PASSWORD_FILE")
        .env_remove("RESTIC_PASSWORD");
    if r.password_file.is_empty() {
        command.env("RESTIC_PASSWORD", &r.password);
    }
    if r.repository.starts_with("sftp:") {
        command.env("SSH_ASKPASS_REQUIRE", "never");
    }
    let mut result = capture_command(command, cancelled)?;
    if r.repository.starts_with("sftp:") && result.code != 0 && result.code != 130 {
        let error = result.stderr.to_ascii_lowercase();
        if [
            "permission denied",
            "host key verification failed",
            "read_passphrase",
            "no supported authentication",
            "unable to authenticate",
        ]
        .iter()
        .any(|message| error.contains(message))
        {
            result.stderr.push_str("\nSFTP requires non-interactive SSH authentication and a verified host key. Configure an SSH key or agent and verify the server in your SSH known_hosts file outside the app. The repository password/password file only unlocks restic encryption; it does not sign in to the SSH server. No password, key-passphrase, or host-trust prompt is opened by the app.\n");
        }
    }
    Ok(result)
}

/// Drain both output pipes while polling for cancellation. On Unix, interrupt restic
/// so it can release its lock; force termination after ten seconds if it does not exit.
fn capture_command(mut command: Command, cancelled: &AtomicBool) -> Result<Output, String> {
    use std::{
        io::Read,
        thread,
        time::{Duration, Instant},
    };
    if cancelled.load(Ordering::SeqCst) || SIGNAL_CANCELLED.load(Ordering::SeqCst) {
        return Ok(Output {
            stdout: String::new(),
            stderr: "Operation cancelled.".into(),
            code: 130,
        });
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Restic is a console executable, but all interaction belongs in the desktop UI.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start restic (check that it is on PATH): {e}"))?;
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let mut interrupted = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(format!("Could not wait for restic: {error}"));
            }
        }
        if cancelled.load(Ordering::SeqCst) || SIGNAL_CANCELLED.load(Ordering::SeqCst) {
            if interrupted.is_none() {
                #[cfg(unix)]
                // SAFETY: child is still owned and unreaped, so its PID cannot be reused.
                unsafe {
                    libc::kill(child.id() as libc::pid_t, libc::SIGINT);
                }
                #[cfg(not(unix))]
                let _ = child.kill();
                interrupted = Some(Instant::now());
            } else if interrupted.is_some_and(|time| time.elapsed() >= Duration::from_secs(10)) {
                let _ = child.kill();
            }
        }
        thread::sleep(Duration::from_millis(100));
    };
    let stdout = out
        .join()
        .map_err(|_| "Output reader failed.")?
        .map_err(|e| e.to_string())?;
    let stderr = err
        .join()
        .map_err(|_| "Error reader failed.")?
        .map_err(|e| e.to_string())?;
    Ok(Output {
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        code: if interrupted.is_some() {
            130
        } else {
            status.code().unwrap_or(-1)
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(repository: &Path, operation: &str) -> Request {
        serde_json::from_value(serde_json::json!({
            "repository": repository.to_str().unwrap(), "password": "test-only-password",
            "passwordFile": "", "operation": operation
        }))
        .unwrap()
    }
    fn success(r: Request) -> Output {
        let output = execute(r).unwrap();
        assert_eq!(output.code, 0, "{}\n{}", output.stdout, output.stderr);
        output
    }
    #[cfg(unix)]
    #[test]
    fn cancels_and_reaps_a_running_process() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let trigger = cancelled.clone();
        let thread = std::thread::spawn(move || {
            let mut command = Command::new("sh");
            command.args(["-c", "echo started; exec sleep 30"]);
            capture_command(command, &trigger).unwrap()
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        cancelled.store(true, Ordering::SeqCst);
        let result = thread.join().unwrap();
        assert_eq!(result.code, 130);
        assert!(result.stdout.contains("started"));
        let result = capture_command(Command::new("true"), &AtomicBool::new(false)).unwrap();
        assert_eq!(result.code, 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn manual_operations_observe_the_cross_process_lock() {
        if std::env::var_os("TBM_MANUAL_LOCK_CHILD").is_some() {
            let r = request(Path::new("/unused"), "version");
            let error = execute(r)
                .err()
                .expect("manual work must not overlap a headless job");
            assert!(error.contains("scheduled backup is running"), "{error}");
            return;
        }
        let home = std::env::temp_dir().join(format!(
            "manual-lock-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = home.join(".local/share/tauri-backup-manager");
        std::fs::create_dir_all(&directory).unwrap();
        let lock = crate::coordination::acquire(
            &directory.join("operation.lock"),
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "restic::tests::manual_operations_observe_the_cross_process_lock",
                "--nocapture",
            ])
            .env("TBM_MANUAL_LOCK_CHILD", "1")
            .env("HOME", &home)
            .output()
            .unwrap();
        drop(lock);
        std::fs::remove_dir_all(home).unwrap();
        assert!(
            child.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
    }

    #[test]
    fn listing_defaults_to_root_and_validates_directory_filters() {
        let mut r = request(Path::new("/tmp/test"), "ls");
        r.snapshot = "12345678".into();
        let args = arguments(&r).unwrap();
        assert_eq!(&args[args.len() - 2..], &["12345678", "/"]);
        r.directory = "/home/test folder".into();
        let args = arguments(&r).unwrap();
        assert_eq!(args.last().unwrap(), "/home/test folder");
        assert!(!args.iter().any(|arg| arg == "--recursive"));
        for directory in ["relative", "/home/../etc", "/bad\0path"] {
            r.directory = directory.into();
            assert!(arguments(&r).is_err());
        }
    }

    #[test]
    #[ignore = "requires restic on PATH; creates only a disposable temporary repository"]
    fn real_restic_directory_browsing() {
        let base = std::env::temp_dir().join(format!(
            "directory-browser-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = base.join("source");
        let nested = source.join("nested");
        let repository = base.join("repository");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(source.join("visible.txt"), "visible").unwrap();
        std::fs::write(nested.join("deep.txt"), "only on opening nested").unwrap();
        success(request(&repository, "init"));
        let mut backup = request(&repository, "backup");
        backup.paths = vec![source.to_str().unwrap().into()];
        success(backup);
        let snapshots = success(request(&repository, "snapshots"));
        let snapshots: Vec<serde_json::Value> = serde_json::from_str(&snapshots.stdout).unwrap();
        let mut ls = request(&repository, "ls");
        ls.snapshot = snapshots[0]["id"].as_str().unwrap().into();
        let root = success(ls.clone());
        assert!(!root.stdout.contains("visible.txt"));
        ls.directory = source.to_str().unwrap().into();
        let listing = success(ls.clone());
        assert!(listing.stdout.contains("visible.txt"));
        assert!(listing.stdout.contains("nested"));
        assert!(
            !listing.stdout.contains("deep.txt"),
            "must not traverse descendants"
        );
        ls.directory = nested.to_str().unwrap().into();
        let listing = success(ls);
        assert!(listing.stdout.contains("deep.txt"));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn rejects_unsafe_requests() {
        let mut r = request(Path::new("/tmp/test"), "repair");
        assert!(arguments(&r).is_err());
        r.operation = "forget".into();
        r.keep_last = 1;
        assert!(arguments(&r).is_err());
        r.dry_run = true;
        assert!(arguments(&r).unwrap().contains(&"--dry-run".into()));
        r.operation = "ls".into();
        r.snapshot = "--help".into();
        assert!(arguments(&r).is_err());
        r.operation = "backup".into();
        r.paths = vec!["relative/path".into()];
        assert!(arguments(&r).is_err());
        r.operation = "prune".into();
        assert!(arguments(&r).is_err());
        r.operation = "forget".into();
        r.keep_last = u32::MAX;
        assert!(arguments(&r).is_err());
    }
    #[test]
    fn sftp_authentication_is_noninteractive_for_every_operation() {
        for repository in ["sftp:user@host:/backup", "sftp://user@host:2222//backup"] {
            let mut r = request(Path::new("/unused"), "snapshots");
            r.repository = repository.into();
            r.paths = vec![std::env::temp_dir().to_str().unwrap().into()];
            r.keep_last = 3;
            for operation in [
                "init",
                "snapshots",
                "unlock",
                "backup",
                "check",
                "forget",
                "prune",
            ] {
                r.operation = operation.into();
                r.confirmation = if operation == "forget" {
                    "FORGET"
                } else {
                    "PRUNE"
                }
                .into();
                let args = arguments(&r).unwrap();
                assert!(args.windows(2).any(|pair| pair == ["--repo", repository]));
                assert!(args.windows(2).any(|pair| pair == ["-o", "sftp.args=-oBatchMode=yes -oStrictHostKeyChecking=yes -oNumberOfPasswordPrompts=0"]));
                assert!(!args.iter().any(|arg| arg.contains(&r.password)));
                assert!(
                    !args
                        .iter()
                        .any(|arg| arg.contains("StrictHostKeyChecking=no"))
                );
            }
            r.operation = "snapshots".into();
            let mut operations = Vec::new();
            execute_with(&r, |step, args| {
                assert!(args.iter().any(|arg| arg.contains("BatchMode=yes")));
                operations.push(step.operation.clone());
                Ok(Output {
                    stdout: String::new(),
                    stderr: String::new(),
                    code: 0,
                })
            })
            .unwrap();
            assert_eq!(operations, ["unlock", "snapshots"]);
        }
        for repository in [
            "/backup",
            "s3:https://host/bucket",
            "rclone:remote:backup",
            "rest:https://host/backup",
        ] {
            let mut r = request(Path::new("/unused"), "snapshots");
            r.repository = repository.into();
            assert!(
                !arguments(&r)
                    .unwrap()
                    .iter()
                    .any(|arg| arg.starts_with("sftp.args="))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn sftp_failure_reports_ssh_setup_separately_from_repository_password() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "sftp-diagnostic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join("fake-restic");
        std::fs::write(&executable, "#!/bin/sh\ntest \"$SSH_ASKPASS_REQUIRE\" = never || exit 99\necho 'Permission denied (publickey).' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut r = request(Path::new("/unused"), "snapshots");
        r.repository = "sftp:user@host:/backup".into();
        let output = run_command(
            &r,
            arguments(&r).unwrap(),
            &AtomicBool::new(false),
            &executable,
        )
        .unwrap();
        assert_eq!(output.code, 1);
        assert!(output.stderr.contains("Permission denied"));
        assert!(
            output
                .stderr
                .contains("repository password/password file only unlocks restic encryption")
        );
        assert!(!output.stderr.contains(&r.password));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn automatic_maintenance_orders_steps_and_stops_on_failure() {
        let mut r = request(Path::new("/tmp/repository"), "backup");
        r.paths = vec![std::env::temp_dir().to_str().unwrap().into()];
        r.maintenance = Some(Maintenance {
            keep_last: 3,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            read_data: true,
        });
        for (failed, code, expected) in [
            ("", 0, vec!["unlock", "backup", "check", "forget", "prune"]),
            ("backup", 3, vec!["unlock", "backup"]),
            ("backup", 130, vec!["unlock", "backup"]),
            ("check", 1, vec!["unlock", "backup", "check"]),
            ("forget", 1, vec!["unlock", "backup", "check", "forget"]),
            ("check", 130, vec!["unlock", "backup", "check"]),
        ] {
            let mut calls = Vec::new();
            let result = execute_with(&r, |step, args| {
                calls.push(step.operation.clone());
                if step.operation == "check" {
                    assert!(args.contains(&"--read-data".into()));
                }
                if step.operation == "forget" {
                    assert!(args.windows(2).any(|pair| pair == ["--keep-last", "3"]));
                    assert!(!args.contains(&"--dry-run".into()));
                }
                Ok(Output {
                    stdout: step.operation.clone(),
                    stderr: String::new(),
                    code: if step.operation == failed { code } else { 0 },
                })
            })
            .unwrap();
            assert_eq!(calls, expected);
            assert_eq!(result.code, code);
        }
        r.dry_run = true;
        let mut calls = Vec::new();
        execute_with(&r, |step, _| {
            calls.push(step.operation.clone());
            Ok(Output {
                stdout: String::new(),
                stderr: String::new(),
                code: 0,
            })
        })
        .unwrap();
        assert_eq!(calls, ["unlock", "backup"]);
        r.dry_run = false;
        let policy = r.maintenance.as_mut().unwrap();
        policy.keep_last = 0;
        policy.keep_daily = 0;
        policy.keep_weekly = 0;
        policy.keep_monthly = 0;
        assert!(
            execute_with(&r, |_, _| panic!(
                "invalid retention must fail before backup"
            ))
            .is_err()
        );
        r.maintenance = None;
        let mut calls = Vec::new();
        execute_with(&r, |step, _| {
            calls.push(step.operation.clone());
            Ok(Output {
                stdout: String::new(),
                stderr: String::new(),
                code: 0,
            })
        })
        .unwrap();
        assert_eq!(calls, ["unlock", "backup"]);
    }

    #[test]
    fn stale_lock_cleanup_precedes_operations_and_stops_on_failure() {
        let mut r = request(Path::new("/tmp/test"), "snapshots");
        for operation in ["snapshots", "init", "version", "unlock"] {
            r.operation = operation.into();
            let mut calls = Vec::new();
            let result = execute_with(&r, |request, args| {
                assert!(!args.iter().any(|a| a == "--remove-all" || a == "--no-lock"));
                assert_eq!(request.password, r.password);
                assert_eq!(request.repository, r.repository);
                calls.push(request.operation.clone());
                Ok(Output {
                    stdout: "[]".into(),
                    stderr: String::new(),
                    code: 0,
                })
            })
            .unwrap();
            assert_eq!(
                result.stdout, "[]",
                "cleanup must not contaminate JSON output"
            );
            let expected = if operation == "snapshots" {
                vec!["unlock", "snapshots"]
            } else {
                vec![operation]
            };
            assert_eq!(calls, expected);
        }
        r.operation = "snapshots".into();
        r.password_file = "/tmp/password".into();
        let mut calls = 0;
        let failure = execute_with(&r, |request, args| {
            calls += 1;
            assert_eq!(request.operation, "unlock");
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--password-file", "/tmp/password"])
            );
            Ok(Output {
                stdout: String::new(),
                stderr: "access denied".into(),
                code: 1,
            })
        })
        .err()
        .unwrap();
        assert_eq!(calls, 1);
        assert!(failure.contains("access denied"));
        r.operation = "prune".into();
        assert!(execute_with(&r, |_, _| panic!("invalid requests must not run cleanup")).is_err());
    }

    #[test]
    #[ignore = "requires restic on PATH; creates only disposable temporary data"]
    fn real_restic_lifecycle() {
        let base = std::env::temp_dir().join(format!(
            "restic-manager-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = base.join("source");
        let repository = base.join("repository");
        let target = base.join("restore");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("important.txt"), "recovery round trip\n").unwrap();
        std::fs::write(source.join("excluded.tmp"), "not backed up").unwrap();
        success(request(&repository, "init"));
        let mut backup = request(&repository, "backup");
        backup.paths = vec![source.to_str().unwrap().into()];
        backup.excludes = vec!["*.tmp".into()];
        backup.tags = vec!["test".into()];
        backup.dry_run = true;
        success(backup.clone());
        let empty = success(request(&repository, "snapshots"));
        assert!(
            serde_json::from_str::<Vec<serde_json::Value>>(&empty.stdout)
                .unwrap()
                .is_empty()
        );
        backup.dry_run = false;
        success(backup);
        let output = success(request(&repository, "snapshots"));
        let snapshots: Vec<serde_json::Value> = serde_json::from_str(&output.stdout).unwrap();
        assert_eq!(snapshots.len(), 1);
        let id = snapshots[0]["id"].as_str().unwrap();
        let mut ls = request(&repository, "ls");
        ls.snapshot = id.into();
        ls.directory = source.to_str().unwrap().into();
        let files = success(ls);
        assert!(files.stdout.contains("important.txt"));
        assert!(!files.stdout.contains("excluded.tmp"));
        let mut restore = request(&repository, "restore");
        restore.snapshot = id.into();
        restore.target = target.to_str().unwrap().into();
        success(restore.clone());
        let restored = target
            .join(source.strip_prefix("/").unwrap())
            .join("important.txt");
        assert_eq!(
            std::fs::read_to_string(restored).unwrap(),
            "recovery round trip\n"
        );
        assert!(
            arguments(&restore).is_err(),
            "must refuse nonempty restore target"
        );
        // A second snapshot makes retention exercise actual deletion, not just a no-op.
        std::fs::write(source.join("important.txt"), "second revision\n").unwrap();
        let mut second = request(&repository, "backup");
        second.paths = vec![source.to_str().unwrap().into()];
        success(second);
        success(request(&repository, "stats"));
        let mut check = request(&repository, "check");
        check.read_data = true;
        success(check);
        let mut retention = request(&repository, "forget");
        retention.keep_last = 1;
        retention.dry_run = true;
        success(retention.clone());
        retention.dry_run = false;
        retention.confirmation = "FORGET".into();
        success(retention);
        let remaining = success(request(&repository, "snapshots"));
        assert_eq!(
            serde_json::from_str::<Vec<serde_json::Value>>(&remaining.stdout)
                .unwrap()
                .len(),
            1
        );
        let password_file = base.join("password");
        std::fs::write(&password_file, "test-only-password\n").unwrap();
        let mut file_auth = request(&repository, "snapshots");
        file_auth.password = "ignored-when-file-is-present".into();
        file_auth.password_file = password_file.to_str().unwrap().into();
        success(file_auth);
        let mut prune = request(&repository, "prune");
        prune.confirmation = "PRUNE".into();
        success(prune);
        let mut wrong = request(&repository, "snapshots");
        wrong.password = "wrong".into();
        assert!(execute(wrong).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
}
