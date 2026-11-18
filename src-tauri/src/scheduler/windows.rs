//! Windows Task Scheduler registration and cooperative cancellation.
use super::*;
use std::{
    io::ErrorKind,
    os::windows::process::CommandExt,
    process::Command,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn powershell(
    script: &str,
    id: &str,
    configure: impl FnOnce(&mut Command),
) -> Result<String, String> {
    let system = std::env::var_os("SystemRoot").ok_or("Cannot locate Windows PowerShell.")?;
    let mut command =
        Command::new(PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe"));
    // Use static scripts and pass data separately, including paths with spaces or shell syntax.
    command
        .creation_flags(0x0800_0000)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "$ErrorActionPreference = 'Stop'; [Console]::OutputEncoding = [Text.UTF8Encoding]::new(); \
             $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value; \
             $name = 'tauri-backup-manager-' + $sid + '-' + $env:RBACKUP_PROFILE_ID; {script}"
        ))
        .env("RBACKUP_PROFILE_ID", id);
    configure(&mut command);
    let output = command
        .output()
        .map_err(|e| format!("Could not run Windows PowerShell: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "Windows Task Scheduler: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().into())
}

fn available() -> Result<(), String> {
    powershell(
        "Import-Module ScheduledTasks; if ((Get-Service -Name Schedule).Status -ne 'Running') { throw 'The Task Scheduler service is stopped.' }",
        "",
        |_| {},
    ).map(|_| ())
}

// Query an exact per-user name. Enumerating tasks also lets a missing task return null
// while permission, module and service errors still surface.
const FIND_TASK: &str =
    "Get-ScheduledTask | Where-Object { $_.TaskPath -eq '\\' -and $_.TaskName -eq $name }";

pub(super) fn save_locked(profile: Profile, schedule: Schedule) -> Result<(), String> {
    let id = profile.id.clone();
    let path = job_path(&id)?;
    let previous = read_job(&id)?;
    if !schedule.enabled {
        if previous.as_ref().is_some_and(|job| job.schedule.enabled) {
            stop(&id)?;
        }
        return write_json(
            &path,
            &Job {
                profile,
                schedule,
                restic_path: previous.map(|job| job.restic_path).unwrap_or_default(),
            },
        );
    }

    let days = schedule_days(&schedule)?.join(",");
    validate_backup(&profile)?;
    let executable = app_executable()?;
    let job = Job {
        profile,
        schedule,
        restic_path: resolve_restic()?,
    };
    transaction(&[path.clone()], || {
        write_json(&path, &job)?;
        powershell(
            r#"
            $at = [DateTime]::Today.Add([TimeSpan]::ParseExact($env:RBACKUP_TIME, 'hh\:mm', [Globalization.CultureInfo]::InvariantCulture))
            if ($at -le [DateTime]::Now) { $at = $at.AddDays(1) }
            $trigger = if ($env:RBACKUP_DAYS) {
                New-ScheduledTaskTrigger -Weekly -At $at -DaysOfWeek ($env:RBACKUP_DAYS -split ',')
            } else { New-ScheduledTaskTrigger -Daily -At $at }
            $action = New-ScheduledTaskAction -Execute $env:RBACKUP_EXE -Argument ('--scheduled-backup ' + $env:RBACKUP_PROFILE_ID) -WorkingDirectory ([IO.Path]::GetDirectoryName($env:RBACKUP_EXE))
            $principal = New-ScheduledTaskPrincipal -UserId $sid -LogonType Interactive -RunLevel Limited
            $settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero)
            Register-ScheduledTask -TaskPath '\' -TaskName $name -Action $action -Trigger $trigger -Principal $principal -Settings $settings -Force | Out-Null
            "#,
            &id,
            |command| {
                command
                    .env("RBACKUP_EXE", executable)
                    .env("RBACKUP_TIME", &job.schedule.time)
                    .env("RBACKUP_DAYS", days);
            },
        )?;
        Ok(())
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskState {
    enabled: bool,
    active: bool,
    next_run: String,
    last_result: Option<u32>,
}

fn query(id: &str) -> Result<Option<TaskState>, String> {
    let output = powershell(
        &format!(
            r#"
        $task = {FIND_TASK}
        if ($task) {{
            $info = Get-ScheduledTaskInfo -InputObject $task
            $service = New-Object -ComObject 'Schedule.Service'
            $service.Connect()
            $registered = $service.GetFolder('\').GetTask($name)
            @{{
                enabled = $task.Settings.Enabled
                active = ([string]$task.State -in 'Running', 'Queued') -or ($registered.GetInstances(0).Count -gt 0)
                nextRun = if ($info.NextRunTime.Year -gt 2000) {{ $info.NextRunTime.ToString('g') }} else {{ '' }}
                lastResult = if ($info.LastRunTime.Year -gt 2000) {{ [uint32]([int64]$info.LastTaskResult -band 4294967295) }} else {{ $null }}
            }} | ConvertTo-Json -Compress
        }} else {{ 'null' }}
        "#
        ),
        id,
        |_| {},
    )?;
    serde_json::from_str(&output).map_err(|e| format!("Invalid Task Scheduler status: {e}"))
}

fn cancel_path(id: &str) -> Result<PathBuf, String> {
    Ok(job_path(id)?.with_extension("cancel"))
}

fn output_path(id: &str) -> Result<PathBuf, String> {
    Ok(job_path(id)?.with_extension("log"))
}

fn request_cancel(id: &str) -> Result<(), String> {
    let path = cancel_path(id)?;
    if query(id)?.is_some_and(|task| task.active) {
        atomic_write(&path, b"cancel")?;
        let deadline = Instant::now() + Duration::from_secs(20);
        while query(id)?.is_some_and(|task| task.active) {
            if Instant::now() >= deadline {
                return Err(
                    "Cancellation was requested. Windows still reports that the task is active."
                        .into(),
                );
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
    remove_file(&path)
}

pub fn cancel(id: &str) -> Result<(), String> {
    validate_id(id)?;
    let _lock = management_lock()?;
    request_cancel(id)
}

pub(super) fn stop(id: &str) -> Result<(), String> {
    powershell(
        &format!(
            "$task = {FIND_TASK}; if ($task -and $task.Settings.Enabled) {{ $task | Disable-ScheduledTask | Out-Null }}"
        ),
        id,
        |_| {},
    )?;
    request_cancel(id)
}

pub(super) fn remove_locked(id: &str) -> Result<(), String> {
    if read_job(id)?.is_some() {
        stop(id)?;
        powershell(
            &format!(
                "$task = {FIND_TASK}; if ($task) {{ $task | Unregister-ScheduledTask -Confirm:$false }}"
            ),
            id,
            |_| {},
        )?;
    }
    remove_file(&job_path(id)?)?;
    remove_file(&state_path(id)?)?;
    remove_file(&output_path(id)?)
}

pub fn status(id: &str) -> Result<Status, String> {
    validate_id(id)?;
    let job = read_job(id)?;
    let mut schedule = job
        .as_ref()
        .map(|job| job.schedule.clone())
        .unwrap_or_default();
    let result = if job.is_some() {
        query(id)
    } else {
        available().map(|_| None)
    };
    let (task, available, reason) = match result {
        Ok(task) => (task, true, String::new()),
        Err(reason) => (None, false, reason),
    };
    let mut last_run = read_state(id)?;
    let active = task.as_ref().is_some_and(|task| task.active);
    let mut service_error = String::new();
    if available && schedule.enabled && !task.as_ref().is_some_and(|task| task.enabled) {
        schedule.enabled = false;
        service_error =
            "The Windows task is missing or disabled. Save the schedule to enable it again.".into();
    }
    if available && !active {
        if let Some(run) = &mut last_run
            && matches!(run.state.as_str(), "running" | "waiting")
        {
            run.state = "failed".into();
            run.message = "The scheduled task stopped before recording a result.".into();
        }
        if let Some(code) = task.as_ref().and_then(|task| task.last_result)
            && code != 0
            && !last_run
                .as_ref()
                .is_some_and(|run| run.code.map(|code| code as u32) == Some(code))
        {
            service_error = format!(
                "Windows task result: 0x{code:08X}. Check the task in Task Scheduler. If the app moved, save the schedule again."
            );
        }
    }
    let logs = match fs::read_to_string(output_path(id)?) {
        Ok(logs) => logs,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.to_string()),
    };
    Ok(Status {
        available,
        reason,
        active,
        next_run: task
            .filter(|_| schedule.enabled)
            .map(|task| task.next_run)
            .unwrap_or_default(),
        schedule,
        service_error,
        last_run,
        logs,
    })
}

pub(super) fn write_output(id: &str, output: &restic::Output) -> Result<(), String> {
    let text = format!("{}\n{}", output.stdout, output.stderr);
    let offset = text
        .char_indices()
        .rev()
        .nth(63_999)
        .map_or(0, |(index, _)| index);
    atomic_write(&output_path(id)?, text[offset..].as_bytes())
}

pub(super) struct CancelWatch {
    done: mpsc::Sender<()>,
    worker: Option<thread::JoinHandle<()>>,
    _run_lock: fs::File,
}

impl Drop for CancelWatch {
    fn drop(&mut self) {
        let _ = self.done.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(super) fn watch_cancel(id: &str) -> Result<CancelWatch, String> {
    let path = cancel_path(id)?;
    let run_lock = coordination::acquire(
        &path.with_extension("run.lock"),
        false,
        &restic::SIGNAL_CANCELLED,
    )?;
    remove_file(&output_path(id)?)?;
    let (done, receiver) = mpsc::channel();
    if path.is_file() {
        restic::SIGNAL_CANCELLED.store(true, Ordering::SeqCst);
        remove_file(&path)?;
    }
    let worker = thread::spawn(move || {
        loop {
            if path.is_file() {
                restic::SIGNAL_CANCELLED.store(true, Ordering::SeqCst);
                let _ = remove_file(&path);
                break;
            }
            if receiver.recv_timeout(Duration::from_millis(100))
                != Err(mpsc::RecvTimeoutError::Timeout)
            {
                break;
            }
        }
    });
    Ok(CancelWatch {
        done,
        worker: Some(worker),
        _run_lock: run_lock,
    })
}
