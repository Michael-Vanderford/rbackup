//! Explicit creation of restic password files without replacing existing paths.

/// Create a profile-specific private password file and return its absolute path.
pub fn create_managed(
    base: &std::path::Path,
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
        let directory = base.join(".local/share/tauri-backup-manager/passwords");
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
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let directory = base.join("tauri-backup-manager/passwords");
        std::fs::create_dir_all(&directory)
            .map_err(|e| format!("Could not create password directory: {e}"))?;
        let metadata = std::fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("Password directory must be a real directory.".into());
        }
        let path = directory.join(format!("{profile_id}.password"));
        let path = path.to_str().ok_or("Password path is not valid UTF-8.")?;
        create(path, password)?;
        Ok(path.into())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (base, password);
        Err("Secure password-file creation is unavailable on this platform.".into())
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
    #[cfg(windows)]
    {
        create_windows(path, password)
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err("Secure password-file creation is unavailable on this platform.".into())
    }
}

#[cfg(windows)]
fn create_windows(path: &str, password: &str) -> Result<(), String> {
    use std::{
        io::Write,
        os::windows::process::CommandExt,
        path::PathBuf,
        process::{Command, Stdio},
    };

    let system = std::env::var_os("SystemRoot").ok_or("Cannot locate Windows PowerShell.")?;
    let mut child = Command::new(
        PathBuf::from(system).join("System32/WindowsPowerShell/v1.0/powershell.exe"),
    )
    .creation_flags(0x0800_0000)
    .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
    // Apply the ACL at creation. The password travels as raw UTF-8 over stdin.
    .arg(
        r#"
        $ErrorActionPreference = 'Stop'
        [Console]::OutputEncoding = [Text.UTF8Encoding]::new()
        $path = $env:RBACKUP_PASSWORD_PATH
        $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User
        $acl = [Security.AccessControl.FileSecurity]::new()
        $acl.SetOwner($sid)
        $acl.SetAccessRuleProtection($true, $false)
        $acl.SetAccessRule([Security.AccessControl.FileSystemAccessRule]::new($sid, 'FullControl', 'Allow'))
        $file = [IO.FileStream]::new($path, 'CreateNew', 'FullControl', 'None', 4096, 'None', $acl)
        try {
            try {
                $actual = $file.GetAccessControl()
                $rules = $actual.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier])
                if (!$actual.AreAccessRulesProtected -or $rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid.Value) {
                    throw 'Could not verify private access to the password file.'
                }
                [Console]::OpenStandardInput().CopyTo($file)
                if ($file.Length -ne [long]$env:RBACKUP_PASSWORD_BYTES) {
                    throw 'Could not receive the complete password.'
                }
                $file.Flush($true)
            } finally { $file.Dispose() }
        } catch {
            [IO.File]::Delete($path)
            throw
        }
        "#,
    )
    .env("RBACKUP_PASSWORD_PATH", path)
    .env("RBACKUP_PASSWORD_BYTES", password.len().to_string())
    .stdin(Stdio::piped())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| format!("Could not start Windows password-file creation: {e}"))?;
    let sent = child.stdin.take().unwrap().write_all(password.as_bytes());
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Could not create password file: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    sent.map_err(|e| format!("Could not send password: {e}"))
}

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    #[cfg(unix)]
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
        let storage = if cfg!(windows) {
            "tauri-backup-manager/passwords"
        } else {
            ".local/share/tauri-backup-manager/passwords"
        };
        assert_eq!(
            std::path::Path::new(&managed),
            directory.join(storage).join("job_123.password")
        );
        assert_eq!(std::fs::read_to_string(&managed).unwrap(), "managed-secret");
        #[cfg(unix)]
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
        create(name, " sëcret with spaces ").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            " sëcret with spaces "
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(create(name, "replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            " sëcret with spaces "
        );
        #[cfg(unix)]
        {
            let link = directory.join("link");
            symlink(&path, &link).unwrap();
            assert!(create(link.to_str().unwrap(), "replacement").is_err());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
