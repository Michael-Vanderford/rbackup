use tauri::Manager;

#[cfg(target_os = "linux")]
mod coordination;
mod password_file;
mod restic;
#[cfg(target_os = "linux")]
mod scheduler;

/// Dispatch a headless scheduled job before creating a Tauri application/webview.
/// Malformed scheduled-backup arguments fail instead of opening the GUI.
pub fn headless(args: &[String]) -> Option<i32> {
    if args.first().is_none_or(|arg| arg != "--scheduled-backup") {
        return None;
    }
    if args.len() != 2 {
        eprintln!("Usage: --scheduled-backup <profile-id>");
        return Some(2);
    }
    #[cfg(target_os = "linux")]
    {
        Some(scheduler::run(&args[1]))
    }
    #[cfg(not(target_os = "linux"))]
    {
        eprintln!("Scheduled backups require Linux and systemd user services.");
        Some(2)
    }
}

/// Manage Linux schedules off the IPC thread. Password text is never accepted here.
#[tauri::command]
async fn scheduler_action(
    action: String,
    profile_id: String,
    profile: Option<serde_json::Value>,
    schedule: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(move || {
            match action.as_str() {
                "status" => {
                    return serde_json::to_value(scheduler::status(&profile_id)?)
                        .map_err(|e| e.to_string());
                }
                "cancel" => scheduler::cancel(&profile_id)?,
                "remove" => scheduler::remove(&profile_id)?,
                "save" | "sync" => {
                    let profile: scheduler::Profile =
                        serde_json::from_value(profile.ok_or("Profile is required.")?)
                            .map_err(|e| e.to_string())?;
                    if profile.id != profile_id {
                        return Err("Profile ID mismatch.".into());
                    }
                    if action == "sync" {
                        scheduler::sync(profile)?;
                    } else {
                        let schedule =
                            serde_json::from_value(schedule.ok_or("Schedule is required.")?)
                                .map_err(|e| e.to_string())?;
                        scheduler::save(profile, schedule)?;
                    }
                }
                _ => return Err("Unsupported scheduler action.".into()),
            }
            Ok(serde_json::Value::Null)
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (profile_id, profile, schedule);
        if action == "sync" || action == "remove" {
            return Ok(serde_json::Value::Null);
        }
        Err("Scheduled backups require Linux and systemd user services. Manual backups remain available.".into())
    }
}

#[tauri::command]
/// Tauri IPC command. Execute the blocking restic work on the async runtime blocking pool and convert worker join failures into string errors. Returns the captured process result or a validation/execution error.
async fn run_restic(request: restic::Request) -> Result<restic::Output, String> {
    tauri::async_runtime::spawn_blocking(move || restic::execute(request))
        .await
        .map_err(|e| e.to_string())?
}

/// Request cancellation without blocking the IPC thread on the running operation.
#[tauri::command]
fn cancel_restic() -> Result<(), String> {
    restic::cancel()
}

/// Save the explicitly supplied session password to a new private password file.
#[tauri::command]
async fn create_password_file(
    app: tauri::AppHandle,
    profile_id: String,
    password: String,
) -> Result<String, String> {
    let home = app.path().home_dir().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        password_file::create_managed(&home, &profile_id, &password)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Initialize Tauri, register the opener plugin and run_restic command, then enter the application event loop. Panics if the Tauri application cannot run.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            run_restic,
            create_password_file,
            cancel_restic,
            scheduler_action
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
