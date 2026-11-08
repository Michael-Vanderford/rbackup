//! Explicit creation of restic password files without replacing existing paths.

/// Create a profile-specific password file under ~/.local/share/tauri-backup-manager/passwords.
/// Restrict the managed directory to its owner and return the saved absolute path.
pub fn create_managed(
    home: &std::path::Path,
    profile_id: &str,
    password: &str,
) -> Result<String, String> {
    if profile_id.is_empty()
        || profile_id.len() > 128
        || !profile_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid repository profile ID.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let directory = home.join(".local/share/tauri-backup-manager/passwords");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(|e| format!("Could not create password directory: {e}"))?;
        let metadata = std::fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("Password directory must be a real directory.".into());
        }
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        let path = directory.join(format!("{profile_id}.password"));
        let path = path.to_str().ok_or("Password path is not valid UTF-8.")?;
        create(path, password)?;
        Ok(path.into())
    }
    #[cfg(not(unix))]
    {
        let _ = (home, password);
        Err("Secure password-file creation is currently supported on Linux and macOS only.".into())
    }
}

/// Write a nonempty, single-line password to a new absolute path with owner-only access.
/// The parent directory must exist. Existing files and symlinks are never replaced.
/// Platforms without the implemented private-permission mechanism return an error.
pub fn create(path: &str, password: &str) -> Result<(), String> {
    if !std::path::Path::new(path).is_absolute() {
        return Err("Password file must be an absolute path.".into());
    }
    if password.is_empty() || password.contains(['\r', '\n']) {
        return Err("Enter a nonempty password without line breaks.".into());
    }
    #[cfg(unix)]
    {
        use std::{fs::OpenOptions, io::Write, os::unix::fs::OpenOptionsExt};
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("Could not create password file: {e}"))?;
        if let Err(error) = file
            .write_all(password.as_bytes())
            .and_then(|_| file.sync_all())
        {
            drop(file);
            let _ = std::fs::remove_file(path);
            return Err(format!("Could not write password file: {error}"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err("Secure password-file creation is currently supported on Linux and macOS only.".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn creates_private_file_and_refuses_overwrites_and_invalid_passwords() {
        let directory = std::env::temp_dir().join(format!(
            "password-file-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        assert!(create_managed(&directory, "../escape", "secret").is_err());
        let managed = create_managed(&directory, "job_123", "managed-secret").unwrap();
        assert_eq!(
            std::path::Path::new(&managed),
            directory.join(".local/share/tauri-backup-manager/passwords/job_123.password")
        );
        assert_eq!(std::fs::read_to_string(&managed).unwrap(), "managed-secret");
        assert_eq!(
            std::fs::metadata(std::path::Path::new(&managed).parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert!(create_managed(&directory, "job_123", "replacement").is_err());
        let path = directory.join("password");
        let name = path.to_str().unwrap();
        assert!(create("relative", "secret").is_err());
        for password in ["", "first\nsecond", "first\rsecond"] {
            assert!(create(name, password).is_err());
            assert!(!path.exists());
        }
        create(name, " secret with spaces ").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            " secret with spaces "
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(create(name, "replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            " secret with spaces "
        );
        let link = directory.join("link");
        symlink(&path, &link).unwrap();
        assert!(create(link.to_str().unwrap(), "replacement").is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
