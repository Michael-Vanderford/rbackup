//! Cross-process coordination shared by interactive and scheduled restic operations.
use std::{
    fs::{File, OpenOptions, TryLockError},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// Hold an exclusive lock until the returned file is dropped.
/// Scheduled jobs wait interruptibly; interactive callers fail immediately when busy.
pub fn acquire(path: &Path, wait: bool, cancelled: &AtomicBool) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    loop {
        if cancelled.load(Ordering::SeqCst) {
            return Err("Operation cancelled.".into());
        }
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::Error(error)) => return Err(error.to_string()),
            Err(TryLockError::WouldBlock) if !wait => {
                return Err("Another app operation or scheduled backup is running. Wait or cancel it before continuing.".into());
            }
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    }
}
