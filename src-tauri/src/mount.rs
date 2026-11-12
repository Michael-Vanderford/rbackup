//! One app-owned read-only FUSE mount. Never delete recursively through a mountpoint.
use crate::restic::{self, Request};
use serde::Serialize;
use std::{
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};
use tauri::Manager;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MountInfo {
    snapshot: String,
    path: String,
    repository: String,
}

#[derive(Default)]
pub struct MountState(Mutex<Option<Mounted>>);
struct Mounted {
    child: Child,
    root: PathBuf,
    repository: String,
    info: MountInfo,
}
impl Drop for Mounted {
    fn drop(&mut self) {
        // Interrupt restic so it unmounts FUSE and releases its repository lock.
        if matches!(self.child.try_wait(), Ok(None)) {
            // SAFETY: the child is still owned and unreaped, so its PID cannot be reused.
            unsafe {
                libc::kill(self.child.id() as libc::pid_t, libc::SIGINT);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while matches!(self.child.try_wait(), Ok(None)) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            for executable in ["fusermount3", "fusermount"] {
                if std::process::Command::new(executable)
                    .arg("-u")
                    .arg(self.root.join("files"))
                    .status()
                    .is_ok_and(|s| s.success())
                {
                    break;
                }
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        let _ = std::fs::remove_dir(self.root.join("files"));
        let _ = std::fs::remove_file(self.root.join("error.log"));
        let _ = std::fs::remove_dir(&self.root);
    }
}

impl MountState {
    pub fn stop(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.take();
        }
    }
}

fn mount_command(request: &Request) -> Result<std::process::Command, String> {
    let mut request = request.clone();
    request.operation = "ls".into();
    let mut args = restic::arguments(&request)?;
    if request.snapshot.len() != 64 {
        return Err(
            "Mounting requires the full snapshot ID. Refresh snapshots and select one.".into(),
        );
    }
    args.truncate(args.len() - 3); // Remove ls, snapshot and directory.
    args.retain(|arg| arg != "--json");
    args.extend(["mount".into(), "--path-template".into(), "ids/%I".into()]);
    let mut command = restic::command(&request, &args, Path::new("restic"));
    // An empty --host still filters snapshots in some restic versions. Remove
    // the inherited default instead so every snapshot remains accessible.
    command.env_remove("RESTIC_HOST");
    Ok(command)
}

pub fn action(
    app: &tauri::AppHandle,
    action: &str,
    request: Option<Request>,
) -> Result<Option<MountInfo>, String> {
    let state = app.state::<MountState>();
    let mut state = state
        .0
        .try_lock()
        .map_err(|_| "A mount operation is already running.")?;
    // Reap failed/disconnected mounts before reporting their state.
    if let Some(mounted) = state.as_mut() {
        if mounted
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            state.take();
        }
    }
    match action {
        "status" => {}
        "stop" => {
            state.take();
        }
        "start" => {
            let r = request.ok_or("Repository and snapshot are required.")?;
            let mut command = mount_command(&r)?;
            if let Some(mounted) = state.as_mut() {
                if mounted.repository != r.repository {
                    return Err("Unmount the current repository before mounting another.".into());
                }
                let path = mounted.root.join("files/ids").join(&r.snapshot);
                if !path.is_dir() {
                    return Err("Snapshot is not available in the mounted repository. Unmount and try again.".into());
                }
                mounted.info = MountInfo {
                    snapshot: r.snapshot,
                    repository: r.repository.clone(),
                    path: path.to_string_lossy().into_owned(),
                };
            } else {
                use std::os::unix::fs::DirBuilderExt;
                let root = std::env::temp_dir().join(format!(
                    "rbackup-mount-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ));
                std::fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&root)
                    .map_err(|e| e.to_string())?;
                let files = root.join("files");
                std::fs::create_dir(&files).map_err(|e| e.to_string())?;
                let log =
                    std::fs::File::create(root.join("error.log")).map_err(|e| e.to_string())?;
                let child = match command
                    .arg(&files)
                    .stdout(Stdio::null())
                    .stderr(log)
                    .spawn()
                {
                    Ok(child) => child,
                    Err(error) => {
                        let _ = std::fs::remove_file(root.join("error.log"));
                        let _ = std::fs::remove_dir(&files);
                        let _ = std::fs::remove_dir(&root);
                        return Err(format!("Could not start restic mount: {error}"));
                    }
                };
                let info = MountInfo {
                    snapshot: r.snapshot.clone(),
                    repository: r.repository.clone(),
                    path: files
                        .join("ids")
                        .join(&r.snapshot)
                        .to_string_lossy()
                        .into_owned(),
                };
                let mut mounted = Mounted {
                    child,
                    root,
                    repository: r.repository,
                    info,
                };
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    if let Some(status) = mounted.child.try_wait().map_err(|e| e.to_string())? {
                        let error = std::fs::read_to_string(mounted.root.join("error.log"))
                            .unwrap_or_default();
                        return Err(format!(
                            "Mount failed ({status}). Linux FUSE and fusermount3/fusermount must be installed.\n{error}"
                        ));
                    }
                    if Path::new(&mounted.info.path).is_dir() {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err("Mount did not become ready within 30 seconds. Check the connection and FUSE installation.".into());
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                *state = Some(mounted);
            }
        }
        _ => return Err("Unsupported mount action.".into()),
    }
    Ok(state.as_ref().map(|mounted| mounted.info.clone()))
}

#[derive(Serialize)]
pub struct Entry {
    path: String,
    r#type: &'static str,
    size: u64,
    mtime: Option<u64>,
}

fn directory_path(root: &Path, directory: &str) -> Result<PathBuf, String> {
    if !directory.starts_with('/') || directory.contains('\0') {
        return Err("Choose an absolute snapshot directory.".into());
    }
    let mut path = root.to_path_buf();
    for component in Path::new(directory).components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(part) => {
                path.push(part);
                if std::fs::symlink_metadata(&path)
                    .map_err(|e| e.to_string())?
                    .file_type()
                    .is_symlink()
                {
                    return Err("Snapshot symbolic links are shown without following them.".into());
                }
            }
            _ => return Err("Snapshot directory must not contain parent traversal.".into()),
        }
    }
    Ok(path)
}

/// List only the managed mounted snapshot, never arbitrary host directories or link targets.
pub fn entries(
    app: &tauri::AppHandle,
    snapshot: &str,
    directory: &str,
) -> Result<Vec<Entry>, String> {
    let state = app.state::<MountState>();
    let mut state = state
        .0
        .try_lock()
        .map_err(|_| "Another mount operation is running.")?;
    let mounted = state
        .as_mut()
        .ok_or("This snapshot is no longer mounted.")?;
    if mounted.info.snapshot != snapshot
        || mounted
            .child
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Err("This snapshot is no longer mounted. Reopen the snapshot browser.".into());
    }
    let path = directory_path(Path::new(&mounted.info.path), directory)?;
    std::fs::read_dir(path)
        .map_err(|e| e.to_string())?
        .map(|entry| {
            let entry = entry.map_err(|e| e.to_string())?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|e| e.to_string())?;
            let kind = if metadata.is_symlink() {
                "symlink"
            } else if metadata.is_dir() {
                "dir"
            } else {
                "file"
            };
            Ok(Entry {
                path: format!(
                    "{}/{}",
                    directory.trim_end_matches('/'),
                    entry.file_name().to_string_lossy()
                ),
                r#type: kind,
                size: metadata.len(),
                mtime: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_millis() as u64),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn test_request(repository: &Path) -> Request {
        serde_json::from_value(serde_json::json!({
            "repository":repository, "password":"disposable-mount-test", "passwordFile":"",
            "operation":"ls", "snapshot":"a".repeat(64)
        }))
        .unwrap()
    }

    #[test]
    fn mount_does_not_filter_out_snapshots_with_an_empty_host() {
        let command = mount_command(&test_request(Path::new("/tmp/repository"))).unwrap();
        assert!(!command.get_args().any(|arg| arg == "--host"));
        assert!(
            command
                .get_envs()
                .any(|(key, value)| key == "RESTIC_HOST" && value.is_none())
        );
    }

    #[test]
    #[ignore = "requires native Linux FUSE and restic; uses only a disposable repository"]
    fn real_fuse_mount_exposes_selected_snapshot_and_unmounts() {
        use std::sync::atomic::AtomicBool;
        let base = std::env::temp_dir().join(format!(
            "rbackup-fuse-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = base.join("source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("sample.txt"), "mounted contents").unwrap();
        let mut request = test_request(&base.join("repository"));
        let mut run = |operation: &str| {
            request.operation = operation.into();
            let result = restic::run_command(
                &request,
                restic::arguments(&request).unwrap(),
                &AtomicBool::new(false),
                Path::new("restic"),
            )
            .unwrap();
            assert_eq!(result.code, 0, "{}", result.stderr);
            result
        };
        run("init");
        request.paths = vec![source.to_string_lossy().into_owned()];
        for operation in ["backup", "snapshots"] {
            request.operation = operation.into();
            let result = restic::run_command(
                &request,
                restic::arguments(&request).unwrap(),
                &AtomicBool::new(false),
                Path::new("restic"),
            )
            .unwrap();
            assert_eq!(result.code, 0, "{}", result.stderr);
            if operation == "snapshots" {
                let snapshots: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
                request.snapshot = snapshots[0]["id"].as_str().unwrap().into();
            }
        }
        let files = base.join("files");
        std::fs::create_dir(&files).unwrap();
        let log = std::fs::File::create(base.join("error.log")).unwrap();
        let child = mount_command(&request)
            .unwrap()
            .arg(&files)
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let snapshot_path = files.join("ids").join(&request.snapshot);
        let mut mounted = Mounted {
            child,
            root: base.clone(),
            repository: request.repository.clone(),
            info: MountInfo {
                snapshot: request.snapshot,
                path: snapshot_path.to_string_lossy().into_owned(),
                repository: request.repository,
            },
        };
        let deadline = Instant::now() + Duration::from_secs(15);
        while !snapshot_path.is_dir() {
            assert!(
                mounted.child.try_wait().unwrap().is_none(),
                "{}",
                std::fs::read_to_string(base.join("error.log")).unwrap()
            );
            assert!(
                Instant::now() < deadline,
                "Selected snapshot did not appear in FUSE mount"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            std::fs::read_to_string(
                snapshot_path
                    .join(source.strip_prefix("/").unwrap())
                    .join("sample.txt")
            )
            .unwrap(),
            "mounted contents"
        );
        drop(mounted);
        assert!(!files.exists(), "mountpoint must be unmounted and removed");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn mounted_directory_rejects_parent_traversal_and_symlink_targets() {
        let root =
            std::env::temp_dir().join(format!("rbackup-mount-path-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("folder")).unwrap();
        std::os::unix::fs::symlink("/tmp", root.join("link")).unwrap();
        assert_eq!(
            directory_path(&root, "/folder").unwrap(),
            root.join("folder")
        );
        assert_eq!(directory_path(&root, "/").unwrap(), root);
        for path in [
            "relative",
            "/../outside",
            "/link",
            "/link/folder",
            "/bad\0name",
        ] {
            assert!(directory_path(&root, path).is_err(), "must reject {path}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dropping_a_mount_reaps_only_its_child_and_removes_empty_directories() {
        let root = std::env::temp_dir().join(format!("rbackup-mount-test-{}", std::process::id()));
        std::fs::create_dir_all(root.join("files")).unwrap();
        std::fs::write(root.join("error.log"), "").unwrap();
        let child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let mut mounted = Mounted {
            child,
            root: root.clone(),
            repository: String::new(),
            info: MountInfo {
                snapshot: "12345678".into(),
                repository: String::new(),
                path: String::new(),
            },
        };
        // A reaped child must never be signalled again during cleanup.
        mounted.child.kill().unwrap();
        mounted.child.wait().unwrap();
        drop(mounted);
        assert!(!root.exists());
    }
}
