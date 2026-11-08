// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Start the desktop executable by delegating to the library entry point. Release builds suppress the additional Windows console window.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = tauri_backup_manager_lib::headless(&args) {
        std::process::exit(code);
    }
    tauri_backup_manager_lib::run()
}
