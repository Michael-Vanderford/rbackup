//! Linux user timers and a headless backup runner. Configuration never contains passwords.
use crate::{coordination, restic};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub password_file: String,
    pub sources: String,
    pub excludes: String,
    pub tags: String,
    #[serde(default)]
    pub maintenance: Option<restic::Maintenance>,
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Schedule {
    pub enabled: bool,
    pub frequency: String,
    pub time: String,
    pub weekdays: Vec<u8>,
}
impl Default for Schedule {
    fn default() -> Self {
        Self {
            enabled: false,
            frequency: "daily".into(),
            time: "03:00".into(),
            weekdays: vec![0],
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    profile: Profile,
    schedule: Schedule,
    restic_path: PathBuf,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunState {
    pub state: String,
    pub started: u64,
    pub finished: Option<u64>,
    pub code: Option<i32>,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub available: bool,
    pub reason: String,
    pub schedule: Schedule,
    pub active: bool,
    pub service_error: String,
    pub next_run: String,
    pub last_run: Option<RunState>,
    pub logs: String,
}

/// Reject path traversal and unit-name syntax in profile identifiers.
fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid repository profile ID.".into());
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or("Cannot locate the user's home directory.".into())
}
fn root() -> Result<PathBuf, String> {
    Ok(home()?.join(".local/share/tauri-backup-manager"))
}
fn directory() -> Result<PathBuf, String> {
    Ok(root()?.join("schedules"))
}
fn unit_directory() -> Result<PathBuf, String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or(home()?.join(".config"));
    Ok(config.join("systemd/user"))
}
fn stem(id: &str) -> String {
    format!("tauri-backup-manager-{id}")
}
fn job_path(id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(directory()?.join(format!("{id}.json")))
}
fn state_path(id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    Ok(directory()?.join(format!("{id}.status.json")))
}

/// Create private app-owned directories, refusing symlinks for the leaf directory.
fn private_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| e.to_string())?;
    if !fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .is_dir()
    {
        return Err("App data directory must not be a symlink.".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

/// Atomically replace private metadata; interrupted writes leave the previous file intact.
fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(contents)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
fn read_job(id: &str) -> Result<Option<Job>, String> {
    match fs::read(job_path(id)?) {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|e| format!("Invalid schedule configuration: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// All app restic processes coordinate through this stable lock inode.
pub fn operation_lock() -> Result<PathBuf, String> {
    let root = root()?;
    private_directory(&root)?;
    Ok(root.join("operation.lock"))
}
fn management_lock() -> Result<fs::File, String> {
    private_directory(&directory()?)?;
    coordination::acquire(
        &directory()?.join("management.lock"),
        false,
        &AtomicBool::new(false),
    )
}

fn tool(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("Could not run {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}
fn systemctl(args: &[&str]) -> Result<String, String> {
    let mut arguments = vec!["--user"];
    arguments.extend_from_slice(args);
    tool("systemctl", &arguments)
}
fn available() -> Result<(), String> {
    systemctl(&["show-environment"]).map(|_| ())
}

/// Translate constrained daily/weekly fields into systemd's native local calendar syntax.
fn calendar(schedule: &Schedule) -> Result<String, String> {
    let bytes = schedule.time.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || ![bytes[0], bytes[1], bytes[3], bytes[4]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        return Err("Choose a valid time (HH:MM).".into());
    }
    let hour: u8 = schedule.time[..2].parse().map_err(|_| "Invalid hour.")?;
    let minute: u8 = schedule.time[3..].parse().map_err(|_| "Invalid minute.")?;
    if hour > 23 || minute > 59 {
        return Err("Choose a valid local time.".into());
    }
    match schedule.frequency.as_str() {
        "daily" => Ok(format!("*-*-* {}:00", schedule.time)),
        "weekly" => {
            if schedule.weekdays.is_empty() || schedule.weekdays.iter().any(|d| *d > 6) {
                return Err("Choose at least one valid weekday.".into());
            }
            let mut days = schedule.weekdays.clone();
            days.sort_unstable();
            days.dedup();
            let names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
            Ok(format!(
                "{} *-*-* {}:00",
                days.iter()
                    .map(|d| names[*d as usize])
                    .collect::<Vec<_>>()
                    .join(","),
                schedule.time
            ))
        }
        _ => Err("Schedule frequency must be daily or weekly.".into()),
    }
}
fn split(value: &str) -> Vec<String> {
    value
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}
fn request(profile: &Profile) -> Result<restic::Request, String> {
    serde_json::from_value(serde_json::json!({"repository": profile.repository, "password":"", "passwordFile":profile.password_file,
        "operation":"backup", "paths":split(&profile.sources), "excludes":split(&profile.excludes), "tags":split(&profile.tags), "maintenance":profile.maintenance})).map_err(|e| e.to_string())
}
fn validate_backup(profile: &Profile) -> Result<(), String> {
    validate_id(&profile.id)?;
    let password = Path::new(&profile.password_file);
    if !password.is_absolute() || !password.is_file() {
        return Err("Scheduling requires an existing absolute password-file path.".into());
    }
    fs::File::open(password).map_err(|e| format!("Cannot read password file: {e}"))?;
    restic::arguments(&request(profile)?)?;
    if Path::new(&profile.repository).is_absolute()
        && !Path::new(&profile.repository).join("config").is_file()
    {
        return Err("Local repository is unavailable or not mounted (config is missing).".into());
    }
    Ok(())
}
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
fn resolve_restic() -> Result<PathBuf, String> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("restic"))
        .find(|p| executable(p))
        .and_then(|p| fs::canonicalize(p).ok())
        .ok_or("Cannot find an executable restic on PATH.".into())
}
fn app_executable() -> Result<PathBuf, String> {
    let path = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_exe)
        .map_err(|e| e.to_string())?;
    if !executable(&path) {
        return Err("The app executable must be an existing absolute executable path.".into());
    }
    Ok(path)
}
/// Quote systemd ExecStart words (including literal percent and dollar characters), not shell words.
fn quote(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control) {
        return Err("Executable paths cannot contain control characters.".into());
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
            .replace('$', "$$")
    ))
}
fn units(id: &str, schedule: &Schedule, executable: &Path) -> Result<(String, String), String> {
    validate_id(id)?;
    let executable = executable
        .to_str()
        .ok_or("Executable path is not valid UTF-8.")?;
    let service = format!(
        "[Unit]\nDescription=Restic scheduled backup ({id})\n\n[Service]\nType=oneshot\nExecStart={} --scheduled-backup {}\nTimeoutStartSec=infinity\nTimeoutStopSec=15s\nKillMode=mixed\nKillSignal=SIGINT\nUMask=0077\nStandardOutput=journal\nStandardError=journal\n",
        quote(executable)?,
        quote(id)?
    );
    let timer = format!(
        "[Unit]\nDescription=Restic backup schedule ({id})\n\n[Timer]\nOnCalendar={}\nPersistent=true\nAccuracySec=1s\nUnit={}.service\n\n[Install]\nWantedBy=timers.target\n",
        calendar(schedule)?,
        stem(id)
    );
    Ok((service, timer))
}

/// Restore all previous configuration files if applying a schedule fails.
fn transaction(
    paths: &[PathBuf],
    apply: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let previous = paths
        .iter()
        .map(|path| match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Err(error) = apply() {
        let mut failures = Vec::new();
        for (path, bytes) in paths.iter().zip(previous) {
            let result = match bytes {
                Some(data) => atomic_write(path, &data),
                None => remove_file(path),
            };
            if let Err(e) = result {
                failures.push(e);
            }
        }
        return Err(if failures.is_empty() {
            error
        } else {
            format!("{error}; rollback failed: {}", failures.join("; "))
        });
    }
    Ok(())
}
fn remove_file(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

/// Install or replace a timer explicitly; failed installation restores the previous files.
pub fn save(profile: Profile, schedule: Schedule) -> Result<(), String> {
    let _lock = management_lock()?;
    save_locked(profile, schedule)
}
fn save_locked(profile: Profile, schedule: Schedule) -> Result<(), String> {
    validate_id(&profile.id)?;
    if schedule.enabled {
        calendar(&schedule)?;
    }
    let old = read_job(&profile.id)?;
    let id = &profile.id;
    if !schedule.enabled {
        if old.as_ref().is_some_and(|j| j.schedule.enabled)
            || unit_directory()?
                .join(format!("{}.timer", stem(id)))
                .exists()
        {
            available()?;
            stop(id)?;
        }
        let job = Job {
            profile,
            schedule,
            restic_path: old.map(|j| j.restic_path).unwrap_or_default(),
        };
        return write_json(&job_path(&job.profile.id)?, &job);
    }
    available()?;
    validate_backup(&profile)?;
    let expression = calendar(&schedule)?;
    tool("systemd-analyze", &["calendar", &expression])?;
    let (service, timer) = units(id, &schedule, &app_executable()?)?;
    let job = Job {
        profile: profile.clone(),
        schedule,
        restic_path: resolve_restic()?,
    };
    let dir = unit_directory()?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let service_path = dir.join(format!("{}.service", stem(id)));
    let timer_path = dir.join(format!("{}.timer", stem(id)));
    let timer_name = format!("{}.timer", stem(id));
    let previously_enabled = systemctl(&["is-enabled", &timer_name]).is_ok();
    let result = transaction(
        &[job_path(id)?, service_path.clone(), timer_path.clone()],
        || {
            // Pause the timer during replacement; an already-running backup uses its loaded snapshot.
            if timer_path.exists() {
                systemctl(&["stop", &timer_name])?;
            }
            write_json(&job_path(id)?, &job)?;
            atomic_write(&service_path, service.as_bytes())?;
            atomic_write(&timer_path, timer.as_bytes())?;
            systemctl(&["daemon-reload"])?;
            systemctl(&["enable", "--now", &timer_name])?;
            Ok(())
        },
    );
    if let Err(error) = result {
        let recovery = (|| {
            systemctl(&["disable", "--now", &timer_name])?;
            systemctl(&["daemon-reload"])?;
            if previously_enabled {
                systemctl(&["enable", "--now", &timer_name])?;
            }
            Ok::<(), String>(())
        })();
        return Err(match recovery {
            Ok(()) => error,
            Err(e) => format!("{error}; could not restore timer state: {e}"),
        });
    }
    Ok(())
}
fn stop(id: &str) -> Result<(), String> {
    let timer = format!("{}.timer", stem(id));
    let service = format!("{}.service", stem(id));
    systemctl(&["disable", "--now", &timer])?;
    systemctl(&["stop", &service])?;
    Ok(())
}

/// Sync saved profile metadata without changing its schedule. Invalid profiles disable timers first.
pub fn sync(profile: Profile) -> Result<(), String> {
    let _lock = management_lock()?;
    let Some(mut job) = read_job(&profile.id)? else {
        return Ok(());
    };
    if job.schedule.enabled
        && let Err(error) = validate_backup(&profile)
    {
        stop(&profile.id)?;
        job.schedule.enabled = false;
        job.profile = profile;
        write_json(&job_path(&job.profile.id)?, &job)?;
        return Err(format!("Schedule disabled: {error}"));
    }
    job.profile = profile;
    write_json(&job_path(&job.profile.id)?, &job)
}

/// Disable and stop both units before removing configuration; never delete repository/password data.
pub fn remove(id: &str) -> Result<(), String> {
    validate_id(id)?;
    let _lock = management_lock()?;
    let dir = unit_directory()?;
    let service = dir.join(format!("{}.service", stem(id)));
    let timer = dir.join(format!("{}.timer", stem(id)));
    if !service.exists() && !timer.exists() && read_job(id)?.is_none() {
        return Ok(());
    }
    let had_units = service.exists() || timer.exists();
    if had_units {
        available()?;
        stop(id)?;
    }
    remove_file(&service)?;
    remove_file(&timer)?;
    if read_job(id)?.is_some() {
        remove_file(&job_path(id)?)?;
        remove_file(&state_path(id)?)?;
    }
    if had_units {
        systemctl(&["daemon-reload"])?;
    }
    Ok(())
}

/// Stop the scheduled service, including jobs waiting for the cross-process lock.
pub fn cancel(id: &str) -> Result<(), String> {
    validate_id(id)?;
    systemctl(&["stop", &format!("{}.service", stem(id))]).map(|_| ())
}

/// Read systemd's actual timer/service state and bounded recent journal output.
pub fn status(id: &str) -> Result<Status, String> {
    validate_id(id)?;
    let job = read_job(id)?;
    let availability = available();
    let service = format!("{}.service", stem(id));
    let mut last_run = match fs::read(state_path(id)?) {
        Ok(bytes) => Some(serde_json::from_slice::<RunState>(&bytes).map_err(|e| e.to_string())?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    let state = if availability.is_ok() {
        systemctl(&["show", &service, "--property=ActiveState", "--value"]).unwrap_or_default()
    } else {
        String::new()
    };
    let active = matches!(state.as_str(), "active" | "activating" | "deactivating");
    if !active
        && availability.is_ok()
        && let Some(run) = &mut last_run
        && matches!(run.state.as_str(), "running" | "waiting")
    {
        run.state = "failed".into();
        run.message =
            "The scheduled service stopped before recording a result. Check the journal.".into();
    }
    let service_error = if state == "failed" {
        let exit_status = systemctl(&["show", &service, "--property=ExecMainStatus", "--value"])
            .unwrap_or_default();
        let code = exit_status.parse::<i32>().ok();
        if last_run.as_ref().is_some_and(|run| {
            run.code == code && !matches!(run.state.as_str(), "waiting" | "running")
        }) {
            String::new()
        } else {
            format!(
                "The scheduled service failed to start or finish (systemd exit {exit_status}). Review the journal. If the app executable moved, save the schedule again."
            )
        }
    } else {
        String::new()
    };
    let next_run = if job.as_ref().is_some_and(|j| j.schedule.enabled) && availability.is_ok() {
        systemctl(&[
            "show",
            &format!("{}.timer", stem(id)),
            "--property=NextElapseUSecRealtime",
            "--value",
        ])
        .unwrap_or_default()
    } else {
        String::new()
    };
    let logs = if job.is_some() && availability.is_ok() {
        tool(
            "journalctl",
            &[
                "--user",
                "--unit",
                &service,
                "--lines=100",
                "--no-pager",
                "--output=short-iso",
            ],
        )
        .unwrap_or_else(|e| e)
    } else {
        String::new()
    };
    let logs: String = logs
        .chars()
        .rev()
        .take(64000)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    Ok(Status {
        available: availability.is_ok(),
        reason: availability.err().unwrap_or_default(),
        schedule: job.map(|j| j.schedule).unwrap_or_default(),
        active,
        service_error,
        next_run,
        last_run,
        logs,
    })
}

extern "C" fn signal_cancel(_: libc::c_int) {
    restic::SIGNAL_CANCELLED.store(true, Ordering::SeqCst);
}
/// Register async-signal-safe cancellation before loading or waiting on scheduled work.
fn install_signals() -> Result<(), String> {
    // SAFETY: handler only updates a lock-free atomic; sigaction structs are fully initialized.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = signal_cancel as *const () as usize;
        libc::sigemptyset(&mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            if libc::sigaction(signal, &action, std::ptr::null_mut()) != 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
        }
    }
    Ok(())
}

/// Run one saved backup without initializing Tauri or requiring a graphical session.
pub fn run(id: &str) -> i32 {
    match run_job(id) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("Scheduled backup failed: {error}");
            1
        }
    }
}
fn run_job(id: &str) -> Result<i32, String> {
    validate_id(id)?;
    install_signals()?;
    let job = read_job(id)?.ok_or("Scheduled backup configuration was not found.")?;
    if job.profile.id != id || !job.schedule.enabled {
        return Err("Schedule is disabled or its profile ID does not match.".into());
    }
    let state_file = state_path(id)?;
    let mut state = RunState {
        state: "waiting".into(),
        started: now(),
        finished: None,
        code: None,
        message: "Waiting for other app operations to finish.".into(),
    };
    write_json(&state_file, &state)?;
    let mut current_operation = String::new();
    let result = (|| {
        let _lock = coordination::acquire(&operation_lock()?, true, &restic::SIGNAL_CANCELLED)?;
        if restic::SIGNAL_CANCELLED.load(Ordering::SeqCst) {
            return Ok(130);
        }
        // Disable/remove can occur while this service waits. Never run a stale enabled snapshot.
        let current = read_job(id)?.ok_or("Schedule was removed.")?;
        if current.profile.id != id {
            return Err("Scheduled profile ID does not match its configuration.".into());
        }
        if !current.schedule.enabled {
            return Ok(130);
        }
        validate_backup(&current.profile)?;
        if !executable(&current.restic_path) {
            return Err("Saved restic executable is unavailable. Save the schedule again.".into());
        }
        state.state = "running".into();
        state.message = "Backup in progress.".into();
        write_json(&state_file, &state)?;
        println!("Scheduled backup started: {}", current.profile.name);
        let result = restic::execute_with(&request(&current.profile)?, |request, args| {
            current_operation = request.operation.clone();
            if matches!(current_operation.as_str(), "check" | "forget" | "prune") {
                state.message = format!(
                    "Backup completed; automatic {} in progress.",
                    current_operation
                );
                write_json(&state_file, &state)?;
                println!("{}", state.message);
            }
            restic::run_command(
                request,
                args,
                &restic::SIGNAL_CANCELLED,
                &current.restic_path,
            )
        })?;
        print!("{}", result.stdout);
        eprint!("{}", result.stderr);
        Ok::<i32, String>(result.code)
    })();
    state.finished = Some(now());
    let code = if restic::SIGNAL_CANCELLED.load(Ordering::SeqCst) {
        130
    } else {
        result.as_ref().copied().unwrap_or(1)
    };
    state.code = Some(code);
    state.state = match code {
        0 => "success",
        3 if current_operation == "backup" => "incomplete",
        130 => "cancelled",
        _ => "failed",
    }
    .into();
    state.message = match result {
        Err(e) if code != 130 => e,
        _ if matches!(current_operation.as_str(), "check" | "forget" | "prune") && code != 0 => {
            format!(
                "Backup completed; automatic {} {}. Later maintenance steps skipped. Review the journal output.",
                current_operation,
                if code == 130 { "cancelled" } else { "failed" }
            )
        }
        _ => match code {
            0 => "Backup and any configured automatic maintenance completed.",
            3 => "Backup incomplete: some files could not be read.",
            130 => "Backup cancelled; completed changes are not undone.",
            _ => "Backup failed. Review the journal output.",
        }
        .into(),
    };
    if state.message.len() > 2000 {
        state.message = state.message.chars().take(1000).collect();
    }
    write_json(&state_file, &state)?;
    println!("{}", state.message);
    Ok(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_calendar_and_quotes_unit_arguments() {
        let mut schedule = Schedule::default();
        assert_eq!(calendar(&schedule).unwrap(), "*-*-* 03:00:00");
        schedule.frequency = "weekly".into();
        schedule.weekdays = vec![6, 0, 0];
        assert_eq!(calendar(&schedule).unwrap(), "Mon,Sun *-*-* 03:00:00");
        schedule.weekdays.clear();
        assert!(calendar(&schedule).is_err());
        schedule.weekdays = vec![7];
        assert!(calendar(&schedule).is_err());
        schedule.frequency = "daily".into();
        for time in ["24:00", "03:60", "3:00", "💥:0", "03:00\n"] {
            schedule.time = time.into();
            assert!(calendar(&schedule).is_err());
        }
        for id in ["", "../bad", "bad.service", "bad\n", "%n"] {
            assert!(validate_id(id).is_err());
        }
        assert_eq!(
            quote("/my app/100%/$x/\"run\"").unwrap(),
            "\"/my app/100%%/$$x/\\\"run\\\"\""
        );
        assert!(quote("/tmp/new\nline").is_err());
        let (service, timer) =
            units("job_123", &Schedule::default(), Path::new("/usr/bin/true")).unwrap();
        assert!(timer.contains("Persistent=true"));
        assert_eq!(timer.matches("OnCalendar=").count(), 1);
        assert!(service.contains("TimeoutStartSec=infinity"));
        assert!(!service.contains("Restart="));
    }
    #[test]
    fn private_storage_rollback_and_credential_free_configuration() {
        use std::os::unix::fs::PermissionsExt;
        let directory = std::env::temp_dir().join(format!(
            "schedule-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        private_directory(&directory).unwrap();
        let path = directory.join("job.json");
        atomic_write(&path, b"previous").unwrap();
        let new = directory.join("new.timer");
        assert!(
            transaction(&[path.clone(), new.clone()], || {
                atomic_write(&path, b"replacement")?;
                atomic_write(&new, b"timer")?;
                Err("installation failed".into())
            })
            .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"previous");
        assert!(!new.exists());
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let profile: Profile = serde_json::from_value(serde_json::json!({"id":"test", "name":"test", "repository":"/backup", "passwordFile":"/private/password", "sources":" /one\n/two\n", "excludes":"*.tmp", "tags":"daily\nlocal", "password":"must-not-persist"})).unwrap();
        let request = request(&profile).unwrap();
        assert!(request.password.is_empty());
        assert!(!request.dry_run);
        assert_eq!(request.paths, ["/one", "/two"]);
        assert_eq!(request.excludes, ["*.tmp"]);
        assert_eq!(request.tags, ["daily", "local"]);
        let job = Job {
            profile,
            schedule: Schedule::default(),
            restic_path: "/usr/bin/restic".into(),
        };
        let json = serde_json::to_string(&job).unwrap();
        assert!(!json.contains("must-not-persist"));
        assert!(!json.contains("\"password\":"));
        write_json(&path, &job).unwrap();
        let restored: Job = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.schedule, Schedule::default());
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn management_lifecycle_and_failed_installation() {
        use std::os::unix::fs::PermissionsExt;
        if std::env::var_os("TBM_SCHEDULER_TEST_CHILD").is_some() {
            let home = home().unwrap();
            let profile = Profile {
                id: "management_test".into(),
                name: "test".into(),
                repository: home.join("repository").to_str().unwrap().into(),
                password_file: home.join("password").to_str().unwrap().into(),
                sources: home.join("source").to_str().unwrap().into(),
                excludes: "*.tmp".into(),
                maintenance: None,
                tags: "nightly".into(),
            };
            let mut schedule = Schedule::default();
            save(profile.clone(), schedule.clone()).unwrap();
            assert!(!read_job(&profile.id).unwrap().unwrap().schedule.enabled);
            schedule.enabled = true;
            save(profile.clone(), schedule.clone()).unwrap();
            let saved = read_job(&profile.id).unwrap().unwrap();
            assert!(saved.schedule.enabled);
            assert!(saved.restic_path.is_absolute());
            assert!(home.join("enabled").exists());
            let mut edited = profile.clone();
            edited.tags = "edited".into();
            sync(edited.clone()).unwrap();
            assert_eq!(
                read_job(&profile.id).unwrap().unwrap().profile.tags,
                "edited"
            );
            // One failed enable must restore the prior working enabled configuration and timer.
            fs::write(home.join("fail-once"), "").unwrap();
            let mut failed = schedule.clone();
            failed.time = "04:00".into();
            assert!(save(edited.clone(), failed).is_err());
            assert_eq!(
                read_job(&profile.id).unwrap().unwrap().schedule.time,
                "03:00"
            );
            assert!(home.join("enabled").exists());
            // A service that fails before the runner launches must not display an old success as current.
            write_json(
                &state_path(&profile.id).unwrap(),
                &RunState {
                    state: "success".into(),
                    started: now(),
                    finished: Some(now()),
                    code: Some(0),
                    message: "Previous success".into(),
                },
            )
            .unwrap();
            fs::write(home.join("service-failure"), "").unwrap();
            assert!(status(&profile.id).unwrap().service_error.contains("203"));
            fs::remove_file(home.join("service-failure")).unwrap();
            // Invalid profile edits stop the timer before saving the disabled configuration.
            edited.sources.clear();
            assert!(sync(edited).is_err());
            assert!(!read_job(&profile.id).unwrap().unwrap().schedule.enabled);
            assert!(!home.join("enabled").exists());
            remove(&profile.id).unwrap();
            assert!(read_job(&profile.id).unwrap().is_none());
            assert!(Path::new(&profile.password_file).exists());
            // A failed first installation leaves no enabled schedule behind.
            fs::write(home.join("fail-once"), "").unwrap();
            assert!(save(profile.clone(), schedule).is_err());
            assert!(read_job(&profile.id).unwrap().is_none());
            assert!(!home.join("enabled").exists());
            assert!(
                !unit_directory()
                    .unwrap()
                    .join(format!("{}.timer", stem(&profile.id)))
                    .exists()
            );
            return;
        }
        let temporary = std::env::temp_dir().join(format!(
            "scheduler-management-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(temporary.join("bin")).unwrap();
        fs::create_dir(temporary.join("repository")).unwrap();
        fs::create_dir(temporary.join("source")).unwrap();
        fs::write(temporary.join("repository/config"), "test").unwrap();
        fs::write(temporary.join("password"), "test-only").unwrap();
        let systemctl = temporary.join("bin/systemctl");
        fs::write(&systemctl, r#"#!/bin/sh
shift
case "$1" in
 is-enabled) test -f "$HOME/enabled" ;;
 show) if test -f "$HOME/service-failure"; then case "$3" in --property=ActiveState) echo failed ;; --property=ExecMainStatus) echo 203 ;; esac; fi ;;
 enable) if test -f "$HOME/fail-once"; then rm "$HOME/fail-once"; echo 'injected installation failure' >&2; exit 1; fi; touch "$HOME/enabled" ;;
 disable) rm -f "$HOME/enabled" ;;
 *) exit 0 ;;
esac
"#).unwrap();
        fs::set_permissions(&systemctl, fs::Permissions::from_mode(0o700)).unwrap();
        let journal = temporary.join("bin/journalctl");
        fs::write(&journal, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(journal, fs::Permissions::from_mode(0o700)).unwrap();
        let restic = temporary.join("bin/restic");
        fs::write(&restic, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(restic, fs::Permissions::from_mode(0o700)).unwrap();
        let path = format!(
            "{}:{}",
            temporary.join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scheduler::tests::management_lifecycle_and_failed_installation",
                "--nocapture",
            ])
            .env("TBM_SCHEDULER_TEST_CHILD", "1")
            .env("HOME", &temporary)
            .env("XDG_CONFIG_HOME", temporary.join("config"))
            .env("PATH", path)
            .env("APPIMAGE", "/usr/bin/true")
            .output()
            .unwrap();
        fs::remove_dir_all(temporary).unwrap();
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[test]
    #[ignore = "requires systemd-analyze sandbox permissions; does not install or activate units"]
    fn generated_units_pass_systemd_validation() {
        let directory = std::env::temp_dir().join(format!("schedule-units-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let (service, timer) =
            units("test", &Schedule::default(), Path::new("/usr/bin/true")).unwrap();
        let service_path = directory.join("tauri-backup-manager-test.service");
        let timer_path = directory.join("tauri-backup-manager-test.timer");
        fs::write(&service_path, service).unwrap();
        fs::write(&timer_path, timer).unwrap();
        let result = Command::new("systemd-analyze")
            .arg("--user")
            .arg("verify")
            .arg(&service_path)
            .arg(&timer_path)
            .output();
        fs::remove_dir_all(directory).unwrap();
        if let Ok(result) = result {
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
