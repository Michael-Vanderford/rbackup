//! Per-user launchd schedules. Calendar triggers are installed in the GUI login domain.
use super::*;
use std::{ffi::OsStr, process::Command};

fn label(id: &str) -> String {
    format!("com.michael.backup-manager.schedule.{id}")
}

fn agent_path(id: &str) -> Result<PathBuf, String> {
    Ok(home()?
        .join("Library/LaunchAgents")
        .join(format!("{}.plist", label(id))))
}

fn domain() -> String {
    // SAFETY: geteuid has no arguments or memory preconditions.
    format!("gui/{}", unsafe { libc::geteuid() })
}

fn target(id: &str) -> String {
    format!("{}/{}", domain(), label(id))
}

fn launchctl(args: &[&OsStr]) -> Result<Option<String>, String> {
    let output = Command::new("/bin/launchctl")
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .map_err(|e| format!("Could not start launchctl: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        Ok(Some(String::from_utf8_lossy(&output.stdout).into_owned()))
    } else if args.first() == Some(&OsStr::new("print"))
        && stderr.contains("Could not find service")
    {
        Ok(None)
    } else {
        Err(format!(
            "launchctl {} failed ({}): {}",
            args[0].to_string_lossy(),
            output.status,
            stderr.trim()
        ))
    }
}

fn print(target: &str) -> Result<Option<String>, String> {
    launchctl(&[OsStr::new("print"), OsStr::new(target)])
}

fn available() -> Result<(), String> {
    print(&domain()).map(|_| ())
}

fn bootstrap(path: &Path) -> Result<(), String> {
    launchctl(&[
        OsStr::new("bootstrap"),
        OsStr::new(&domain()),
        path.as_os_str(),
    ])
    .map(|_| ())
}

fn bootout(id: &str) -> Result<(), String> {
    launchctl(&[OsStr::new("bootout"), OsStr::new(&target(id))]).map(|_| ())
}

fn field<'a>(service: &'a str, name: &str) -> Option<&'a str> {
    // Only read the top-level properties, not nested trigger or environment data.
    service
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .filter_map(|line| line.split_once(" = "))
        .find_map(|(key, value)| (key == name).then_some(value))
}

fn running(service: &str) -> bool {
    field(service, "pid")
        .and_then(|pid| pid.parse::<u32>().ok())
        .is_some()
}

fn plist(id: &str, schedule: &Schedule, executable: &Path) -> Result<String, String> {
    let days = schedule_days(schedule)?;
    let executable = executable
        .to_str()
        .filter(|value| !value.chars().any(char::is_control))
        .ok_or("Executable path must be valid UTF-8 without control characters.")?
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let hour = schedule.time[..2]
        .parse::<u8>()
        .map_err(|e| e.to_string())?;
    let minute = schedule.time[3..]
        .parse::<u8>()
        .map_err(|e| e.to_string())?;
    let calendar = |weekday: &str| {
        format!(
            "<dict><key>Hour</key><integer>{hour}</integer><key>Minute</key><integer>{minute}</integer>{weekday}</dict>"
        )
    };
    let intervals = if days.is_empty() {
        calendar("")
    } else {
        let mut entries = String::from("<array>");
        for (index, day) in WEEKDAYS.iter().enumerate() {
            if days.contains(day) {
                let weekday = (index + 1) % 7;
                entries.push_str(&calendar(&format!(
                    "<key>Weekday</key><integer>{weekday}</integer>"
                )));
            }
        }
        entries.push_str("</array>");
        entries
    };
    let label = label(id);
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array><string>{executable}</string><string>--scheduled-backup</string><string>{id}</string></array>
<key>StartCalendarInterval</key>{intervals}
<key>ExitTimeOut</key><integer>15</integer>
</dict></plist>
"#
    ))
}

pub(super) fn save_locked(profile: Profile, schedule: Schedule) -> Result<(), String> {
    validate_id(&profile.id)?;
    let id = profile.id.clone();
    let path = agent_path(&id)?;
    let old = read_job(&id)?;
    let (definition, restic_path) = if schedule.enabled {
        validate_backup(&profile)?;
        (
            Some(plist(&id, &schedule, &app_executable()?)?),
            resolve_restic()?,
        )
    } else {
        (
            None,
            old.as_ref()
                .map(|job| job.restic_path.clone())
                .unwrap_or_default(),
        )
    };
    let previous =
        if definition.is_some() || path.exists() || old.is_some_and(|job| job.schedule.enabled) {
            print(&target(&id))?
        } else {
            None
        };
    if definition.is_some() && previous.as_deref().is_some_and(running) {
        return Err("Wait for the scheduled backup to finish before changing its schedule.".into());
    }
    let job = Job {
        profile,
        schedule,
        restic_path,
    };
    let job_path = job_path(&id)?;
    let mut unloaded = false;
    let result = transaction(&[job_path.clone(), path.clone()], || {
        if previous.is_some() {
            bootout(&id)?;
            unloaded = true;
        }
        write_json(&job_path, &job)?;
        if let Some(definition) = definition {
            let dir = path.parent().ok_or("LaunchAgent path has no parent.")?;
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            atomic_write(&path, definition.as_bytes())?;
            bootstrap(&path)
        } else {
            remove_file(&path)
        }
    });
    if let Err(error) = result {
        if unloaded && let Err(recovery) = bootstrap(&path) {
            return Err(format!(
                "{error}; could not restore the previous LaunchAgent: {recovery}"
            ));
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn stop(id: &str) -> Result<(), String> {
    if print(&target(id))?.is_some() {
        bootout(id)?;
    }
    remove_file(&agent_path(id)?)
}

pub(super) fn remove_locked(id: &str) -> Result<(), String> {
    if agent_path(id)?.exists() || read_job(id)?.is_some_and(|job| job.schedule.enabled) {
        stop(id)?;
    }
    remove_file(&job_path(id)?)?;
    remove_file(&state_path(id)?)?;
    clear_output(id)
}

pub fn cancel(id: &str) -> Result<(), String> {
    validate_id(id)?;
    launchctl(&[
        OsStr::new("kill"),
        OsStr::new("SIGINT"),
        OsStr::new(&target(id)),
    ])
    .map(|_| ())
}

fn log_path(id: &str) -> Result<PathBuf, String> {
    Ok(directory()?.join(format!("{id}.log")))
}

pub(super) fn clear_output(id: &str) -> Result<(), String> {
    remove_file(&log_path(id)?)
}

pub(super) fn write_output(id: &str, output: &restic::Output) -> Result<(), String> {
    let log = output
        .stdout
        .lines()
        .filter(|line| !line.starts_with(r#"{"message_type":"status""#))
        .chain(output.stderr.lines())
        .collect::<Vec<_>>()
        .join("\n");
    let mut start = log.len().saturating_sub(64_000);
    while !log.is_char_boundary(start) {
        start += 1;
    }
    atomic_write(&log_path(id)?, &log.as_bytes()[start..])
}

pub fn status(id: &str) -> Result<Status, String> {
    validate_id(id)?;
    let schedule = read_job(id)?.map(|job| job.schedule).unwrap_or_default();
    let agent = if schedule.enabled {
        print(&target(id))
    } else {
        available().map(|_| None)
    };
    let active = agent
        .as_ref()
        .ok()
        .and_then(Option::as_deref)
        .is_some_and(running);
    let mut last_run = read_state(id)?;
    let mut service_error = String::new();
    let mut next_run = String::new();
    if let Ok(service) = &agent {
        if !active
            && let Some(run) = &mut last_run
            && matches!(run.state.as_str(), "waiting" | "running")
        {
            run.state = "failed".into();
            run.message = "The scheduled backup stopped before recording its result.".into();
        }
        if let Some(service) = service {
            next_run = "Managed by launchd".into();
            if let Some(exit) = field(service, "last exit code")
                && let Ok(code) = exit.parse::<i32>()
                && code != 0
                && !last_run.as_ref().is_some_and(|run| run.code == Some(code))
                && !active
            {
                service_error = format!(
                    "LaunchAgent exited with code {code}. If rbackup moved, save the schedule again."
                );
            }
        } else if schedule.enabled {
            service_error = "The LaunchAgent is unloaded. Check macOS background item settings and save the schedule again.".into();
        }
    }
    let logs = match fs::read_to_string(log_path(id)?) {
        Ok(logs) => logs,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e.to_string()),
    };
    Ok(Status {
        available: agent.is_ok(),
        reason: agent.err().unwrap_or_default(),
        schedule,
        active,
        service_error,
        next_run,
        last_run,
        logs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_maps_calendar_days_and_escapes_the_executable() {
        let app = Path::new("/Applications/rbackup & friends.app/Contents/MacOS/rbackup");
        let daily = plist("profile", &Schedule::default(), app).unwrap();
        assert!(daily.contains("rbackup &amp; friends.app"));
        assert!(daily.contains("<key>Hour</key><integer>3</integer>"));
        assert!(!daily.contains("<key>Weekday</key>"));
        let weekly = Schedule {
            frequency: "weekly".into(),
            weekdays: vec![0, 6, 0],
            ..Schedule::default()
        };
        let definition = plist("profile", &weekly, app).unwrap();
        assert_eq!(definition.matches("<key>Weekday</key>").count(), 2);
        assert!(definition.contains("<key>Weekday</key><integer>1</integer>"));
        assert!(definition.contains("<key>Weekday</key><integer>0</integer>"));
        assert!(running("service = {\n\tpid = 123\n}"));
        assert!(!running("service = {\n\t\tpid = 123\n}"));
    }
}
