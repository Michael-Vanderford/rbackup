//! Cross-process coordination shared by interactive and scheduled restic operations.
use std::{
    fs::File,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

/// Hold an exclusive advisory lock until the returned file is dropped.
/// Scheduled jobs wait interruptibly; interactive callers fail immediately when busy.
pub fn acquire(path: &Path, wait: bool, cancelled: &AtomicBool) -> Result<File, String> {
    #[cfg(unix)]
    {
        use std::{
            fs::OpenOptions,
            os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
        };
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| e.to_string())?;
        loop {
            if cancelled.load(Ordering::SeqCst) {
                return Err("Operation cancelled.".into());
            }
            // SAFETY: file owns a valid descriptor for the duration of this call.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(file);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::WouldBlock {
                return Err(error.to_string());
            }
            if !wait {
                return Err("Another app operation or scheduled backup is running. Wait or cancel it before continuing.".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, wait, cancelled);
        Err("Cross-process locking is unavailable on this platform.".into())
    }
}
