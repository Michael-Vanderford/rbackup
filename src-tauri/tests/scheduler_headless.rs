//! Headless smoke tests use disposable homes/repositories and never activate a timer.
#![cfg(target_os = "linux")]
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::{fs::PermissionsExt, io::AsRawFd},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Fixture {
    home: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!(
            "scheduler-headless-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(home.join(".local/share/tauri-backup-manager/schedules")).unwrap();
        fs::create_dir(home.join("source")).unwrap();
        fs::write(home.join("source/document.txt"), "disposable test data").unwrap();
        fs::write(home.join("password"), "test-only-password").unwrap();
        Self { home }
    }
    fn root(&self) -> PathBuf {
        self.home.join(".local/share/tauri-backup-manager")
    }
    fn configure(&self, id: &str, restic: &Path, enabled: bool) {
        let repository = self.home.join("repository");
        fs::create_dir_all(&repository).unwrap();
        if !repository.join("config").exists() {
            fs::write(repository.join("config"), "fake config").unwrap();
        }
        let job = json!({"profile":{"id":id,"name":"Disposable backup", "repository":repository,"passwordFile":self.home.join("password"),"sources":self.home.join("source").to_str().unwrap(),"excludes":"*.tmp","tags":"scheduled\ntest"},"schedule":{"enabled":enabled,"frequency":"daily","time":"03:00","weekdays":[0]},"resticPath":restic});
        fs::write(
            self.root().join(format!("schedules/{id}.json")),
            serde_json::to_vec(&job).unwrap(),
        )
        .unwrap();
    }
    fn command(&self, id: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rbackup"));
        command
            .args(["--scheduled-backup", id])
            .env("HOME", &self.home)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("RESTIC_PASSWORD_COMMAND")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
    fn state(&self, id: &str) -> Option<Value> {
        fs::read(self.root().join(format!("schedules/{id}.status.json")))
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
    }
    fn wait_state(&self, id: &str, state: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if self.state(id).is_some_and(|s| s["state"] == state) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("never reached {state}: {:?}", self.state(id));
    }
    fn fake_restic(&self) -> PathBuf {
        let path = self.home.join("fake-restic");
        fs::write(&path, "#!/bin/sh\nfor arg do\n if [ \"$arg\" = backup ]; then\n printf '%s\\n' \"$@\" > \"$HOME/arguments\"\n echo $$ > \"$HOME/child-pid\"\n exec /bin/sleep 30\n fi\ndone\nexit 0\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}
fn wait(child: &mut Child) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status.code().unwrap_or(-1);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("headless process did not exit");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn interrupt(child: &Child) {
    // SAFETY: child is owned and has not been reaped.
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
}

#[test]
fn scheduled_backup_runs_saved_maintenance_and_reports_check_failure() {
    let fixture = Fixture::new();
    let fake = fixture.home.join("maintenance-restic");
    fs::write(
        &fake,
        r#"#!/bin/sh
for arg do
 case "$arg" in
 unlock|backup|check|forget|prune)
  echo "$arg" >> "$HOME/operations"
  if [ "$arg" = check ] && [ -f "$HOME/fail-check" ]; then exit 1; fi
  exit 0
  ;;
 esac
done
exit 2
"#,
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.configure("maintenance", &fake, true);
    let path = fixture.root().join("schedules/maintenance.json");
    let mut job: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    job["profile"]["maintenance"] =
        json!({"keepLast":3,"keepDaily":7,"keepWeekly":4,"keepMonthly":6,"readData":false});
    fs::write(&path, serde_json::to_vec(&job).unwrap()).unwrap();
    assert_eq!(
        wait(&mut fixture.command("maintenance").spawn().unwrap()),
        0
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("operations")).unwrap(),
        "unlock\nbackup\ncheck\nforget\nprune\n"
    );
    fs::remove_file(fixture.home.join("operations")).unwrap();
    fs::write(fixture.home.join("fail-check"), "").unwrap();
    assert_eq!(
        wait(&mut fixture.command("maintenance").spawn().unwrap()),
        1
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("operations")).unwrap(),
        "unlock\nbackup\ncheck\n"
    );
    assert_eq!(fixture.state("maintenance").unwrap()["state"], "failed");
    assert!(
        fixture.state("maintenance").unwrap()["message"]
            .as_str()
            .unwrap()
            .contains("automatic check failed")
    );
}

#[test]
fn cancels_queued_work_without_starting_restic_and_releases_lock() {
    let fixture = Fixture::new();
    let fake = fixture.fake_restic();
    fixture.configure("queued", &fake, true);
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(fixture.root().join("operation.lock"))
        .unwrap();
    // SAFETY: lock owns a valid descriptor.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let mut child = fixture.command("queued").spawn().unwrap();
    fixture.wait_state("queued", "waiting");
    assert!(!fixture.home.join("arguments").exists());
    interrupt(&child);
    assert_eq!(wait(&mut child), 130);
    assert_eq!(fixture.state("queued").unwrap()["state"], "cancelled");
    assert!(!fixture.home.join("arguments").exists());
    drop(lock);
    fixture.configure("next", Path::new("/usr/bin/true"), true);
    assert_eq!(wait(&mut fixture.command("next").spawn().unwrap()), 0);
}

#[test]
fn serializes_separate_runners_and_cancels_running_child() {
    let fixture = Fixture::new();
    let fake = fixture.fake_restic();
    fixture.configure("first", &fake, true);
    fixture.configure("second", Path::new("/usr/bin/true"), true);
    let mut first = fixture.command("first").spawn().unwrap();
    fixture.wait_state("first", "running");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.home.join("child-pid").exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let pid: i32 = fs::read_to_string(fixture.home.join("child-pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut second = fixture.command("second").spawn().unwrap();
    fixture.wait_state("second", "waiting");
    assert!(second.try_wait().unwrap().is_none());
    interrupt(&first);
    assert_eq!(wait(&mut first), 130);
    assert_eq!(wait(&mut second), 0);
    assert_eq!(fixture.state("first").unwrap()["state"], "cancelled");
    assert_eq!(fixture.state("second").unwrap()["state"], "success");
    // SAFETY: signal zero only checks whether a process exists.
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        -1,
        "restic child must be reaped"
    );
    let arguments = fs::read_to_string(fixture.home.join("arguments")).unwrap();
    assert!(arguments.contains("--exclude\n*.tmp"));
    assert!(arguments.contains("--tag\nscheduled\n--tag\ntest"));
    assert!(!arguments.contains("test-only-password"));
}

#[test]
fn rejects_disabled_invalid_and_unreadable_credentials_without_a_gui() {
    let fixture = Fixture::new();
    fixture.configure("disabled", Path::new("/usr/bin/true"), false);
    assert_eq!(wait(&mut fixture.command("disabled").spawn().unwrap()), 1);
    assert_eq!(wait(&mut fixture.command("../bad").spawn().unwrap()), 1);
    fixture.configure("missing-password", Path::new("/usr/bin/true"), true);
    fs::remove_file(fixture.home.join("password")).unwrap();
    assert_eq!(
        wait(&mut fixture.command("missing-password").spawn().unwrap()),
        1
    );
    assert_eq!(
        fixture.state("missing-password").unwrap()["state"],
        "failed"
    );
}

#[test]
fn makes_real_scheduled_snapshots_with_a_password_file() {
    let restic = Command::new("sh")
        .args(["-c", "command -v restic"])
        .output()
        .unwrap();
    if !restic.status.success() {
        eprintln!("restic not installed; skipping real backup check");
        return;
    }
    let executable = PathBuf::from(String::from_utf8(restic.stdout).unwrap().trim());
    let fixture = Fixture::new();
    let init = Command::new(&executable)
        .arg("--repo")
        .arg(fixture.home.join("repository"))
        .arg("--password-file")
        .arg(fixture.home.join("password"))
        .arg("--no-cache")
        .arg("init")
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    fixture.configure("real", &executable, true);
    assert_eq!(wait(&mut fixture.command("real").spawn().unwrap()), 0);
    let output = Command::new(&executable)
        .arg("--repo")
        .arg(fixture.home.join("repository"))
        .arg("--password-file")
        .arg(fixture.home.join("password"))
        .args(["--no-cache", "--json", "snapshots"])
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshots: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0]["tags"], json!(["scheduled", "test"]));
}

/// Installs only uniquely named runtime units; requires a real systemd user bus.
#[test]
#[ignore = "installs disposable runtime user units; requires a systemd user session"]
fn systemd_user_timer_runs_closed_app_and_catches_up() {
    fn ctl(args: &[&str]) -> std::process::Output {
        Command::new("systemctl")
            .arg("--user")
            .args(args)
            .output()
            .unwrap()
    }
    fn checked(args: &[&str]) {
        let result = ctl(args);
        assert!(
            result.status.success(),
            "{:?}: {}",
            args,
            String::from_utf8_lossy(&result.stderr)
        );
    }
    struct Units {
        service: String,
        timer: String,
    }
    impl Drop for Units {
        fn drop(&mut self) {
            let _ = ctl(&["disable", "--runtime", "--now", &self.timer]);
            let _ = ctl(&["stop", &self.service]);
            let _ = ctl(&["clean", "--what=state", &self.timer]);
            let _ = ctl(&["disable", "--runtime", &self.service]);
            let runtime = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() }))
                });
            let _ = fs::remove_file(runtime.join("systemd/user").join(&self.service));
            let _ = fs::remove_file(runtime.join("systemd/user").join(&self.timer));
            let _ = ctl(&["daemon-reload"]);
            let _ = ctl(&["reset-failed", &self.service]);
        }
    }
    checked(&["show-environment"]);
    let fixture = Fixture::new();
    let restic = Command::new("sh")
        .args(["-c", "command -v restic"])
        .output()
        .unwrap();
    assert!(restic.status.success(), "restic required");
    let executable = PathBuf::from(String::from_utf8(restic.stdout).unwrap().trim());
    let init = Command::new(&executable)
        .arg("--repo")
        .arg(fixture.home.join("repository"))
        .arg("--password-file")
        .arg(fixture.home.join("password"))
        .args(["--no-cache", "init"])
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .output()
        .unwrap();
    assert!(init.status.success());
    fixture.configure("timer-test", &executable, true);
    let unique = format!(
        "tbm-disposable-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let names = Units {
        service: format!("{unique}.service"),
        timer: format!("{unique}.timer"),
    };
    let service_path = fixture.home.join(&names.service);
    let timer_path = fixture.home.join(&names.timer);
    let quote = |s: &str| {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
                .replace('$', "$$")
        )
    };
    let service = format!(
        "[Service]\nType=oneshot\nEnvironment={}\nExecStart={} --scheduled-backup timer-test\nTimeoutStartSec=infinity\nTimeoutStopSec=15s\nKillMode=mixed\nKillSignal=SIGINT\nUMask=0077\n",
        quote(&format!("HOME={}", fixture.home.display())),
        quote(env!("CARGO_BIN_EXE_rbackup"))
    );
    let date = |seconds: u32| {
        let output = Command::new("date")
            .args([
                "-u",
                "-d",
                &format!("+{seconds} seconds"),
                "+%Y-%m-%d %H:%M:%S UTC",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    let first = date(4);
    let second = date(16);
    fs::write(&service_path, service).unwrap();
    fs::write(&timer_path, format!("[Timer]\nOnCalendar={first}\nOnCalendar={second}\nPersistent=true\nAccuracySec=1s\nUnit={}\n[Install]\nWantedBy=timers.target\n", names.service)).unwrap();
    checked(&[
        "link",
        "--runtime",
        service_path.to_str().unwrap(),
        timer_path.to_str().unwrap(),
    ]);
    checked(&["daemon-reload"]);
    checked(&["enable", "--runtime", "--now", &names.timer]);
    fixture.wait_state("timer-test", "success");
    let initial = fixture.state("timer-test").unwrap()["started"]
        .as_u64()
        .unwrap();
    checked(&["stop", &names.timer]);
    // Leave the timer stopped across the second occurrence. A single catch-up is due.
    std::thread::sleep(Duration::from_secs(17));
    checked(&["start", &names.timer]);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if fixture
            .state("timer-test")
            .is_some_and(|s| s["state"] == "success" && s["started"].as_u64().unwrap() > initial)
        {
            break;
        }
        assert!(Instant::now() < deadline, "missed run did not catch up");
        std::thread::sleep(Duration::from_millis(100));
    }
    checked(&["stop", &names.timer]);
    let output = Command::new(&executable)
        .arg("--repo")
        .arg(fixture.home.join("repository"))
        .arg("--password-file")
        .arg(fixture.home.join("password"))
        .args(["--no-cache", "--json", "snapshots"])
        .env_remove("RESTIC_PASSWORD_COMMAND")
        .output()
        .unwrap();
    assert!(output.status.success());
    let snapshots: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        snapshots.len(),
        2,
        "one initial run and one catch-up; no duplicate service instances"
    );
    let logs = Command::new("journalctl")
        .args(["--user", "--unit", &names.service, "--no-pager"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&logs.stdout).contains("Backup completed."));
}
