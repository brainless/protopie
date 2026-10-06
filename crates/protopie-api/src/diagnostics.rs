//! Local, project-scoped JSONL diagnostics shared by the GUI and server.
//! The caller chooses fields; source files and diff bodies must not be passed in.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

const MAX_LOG_BYTES: u64 = 1_048_576;
const MAX_LINE_BYTES: usize = 65_536;
const MAX_TEXT_BYTES: usize = 16_384;

fn repo_log_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(".logs")
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Stable directory for the canonical project path. Canonicalization also keeps
/// invalid project paths from creating arbitrary diagnostic directories.
pub fn project_log_dir(project_path: &Path) -> io::Result<PathBuf> {
    project_log_dir_at(&repo_log_root(), project_path)
}

fn project_log_dir_at(root: &Path, project_path: &Path) -> io::Result<PathBuf> {
    let canonical = project_path.canonicalize()?;
    let digest = Sha256::digest(path_bytes(&canonical));
    Ok(root.join(format!("{digest:x}")))
}

/// Preserve ordinary prompt text, but cap pathological inputs before logging.
pub fn bounded_text(value: &str) -> String {
    if value.len() <= MAX_TEXT_BYTES {
        return value.to_string();
    }
    let mut end = MAX_TEXT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}… [truncated; {} UTF-8 bytes total]",
        &value[..end],
        value.len()
    )
}

/// Append one JSON line. The current file and two rotated copies are retained.
/// Logging errors are returned so callers can choose whether to surface them.
pub fn log_project_event(
    project_path: &Path,
    source: &str,
    event: impl Serialize,
) -> io::Result<()> {
    log_project_event_at(&repo_log_root(), project_path, source, event)
}

fn log_project_event_at(
    root: &Path,
    project_path: &Path,
    source: &str,
    event: impl Serialize,
) -> io::Result<()> {
    let canonical = project_path.canonicalize()?;
    let dir = project_log_dir_at(root, &canonical)?;
    fs::create_dir_all(root)?;
    if fs::symlink_metadata(root)?.file_type().is_symlink() {
        return Err(io::Error::other("diagnostic root is a symlink"));
    }
    fs::create_dir_all(&dir)?;
    if fs::symlink_metadata(&dir)?.file_type().is_symlink() {
        return Err(io::Error::other(
            "project diagnostic directory is a symlink",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
    }
    let serialized = serde_json::to_vec(&serde_json::json!({
        "ts_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
        "source": source,
        "event": event,
    }))?;
    let mut line = if serialized.len() > MAX_LINE_BYTES {
        serde_json::to_vec(&serde_json::json!({
            "ts_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
            "source": source,
            "event": {"kind": "oversized_event_omitted", "serialized_bytes": serialized.len()},
        }))?
    } else {
        serialized
    };
    line.push(b'\n');

    // One process can have several GUI workers, and separate server processes
    // can target the same project. The mutex plus lock file covers both cases.
    static PROCESS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = PROCESS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| io::Error::other("diagnostic lock poisoned"))?;
    let lock = private_file(&dir.join(".lock"))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    let result = append_locked(&dir, &canonical, &line);
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let _ = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) };
    }
    result
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

fn append_locked(dir: &Path, canonical: &Path, line: &[u8]) -> io::Result<()> {
    let metadata = dir.join("project.json");
    if !metadata.exists() {
        let mut file = private_file(&metadata)?;
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({"canonical_project_path": canonical.to_string_lossy()}),
        )?;
        file.write_all(b"\n")?;
    } else {
        if fs::symlink_metadata(&metadata)?.file_type().is_symlink() {
            return Err(io::Error::other("project diagnostic metadata is a symlink"));
        }
        let recorded: serde_json::Value = serde_json::from_slice(&fs::read(&metadata)?)?;
        if recorded["canonical_project_path"] != canonical.to_string_lossy().as_ref() {
            return Err(io::Error::other("project diagnostic key collision"));
        }
    }
    let current = dir.join("events.jsonl");
    let first = dir.join("events.1.jsonl");
    let second = dir.join("events.2.jsonl");
    let size = fs::metadata(&current).map(|m| m.len()).unwrap_or(0);
    if size + line.len() as u64 > MAX_LOG_BYTES {
        if second.exists() {
            fs::remove_file(&second)?;
        }
        if first.exists() {
            fs::rename(&first, &second)?;
        }
        if current.exists() {
            fs::rename(&current, &first)?;
        }
    }
    private_file(&current)?.write_all(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_canonical_key_and_parseable_concurrent_lines() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("logs");
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        assert_eq!(
            project_log_dir_at(&root, &a).unwrap(),
            project_log_dir_at(&root, &a.join(".")).unwrap()
        );
        assert_ne!(
            project_log_dir_at(&root, &a).unwrap(),
            project_log_dir_at(&root, &b).unwrap()
        );
        std::thread::scope(|scope| {
            for worker in 0..8 {
                let root = &root;
                let a = &a;
                scope.spawn(move || {
                    for n in 0..40 {
                        log_project_event_at(
                            root,
                            a,
                            "test",
                            serde_json::json!({"worker":worker,"n":n}),
                        )
                        .unwrap();
                    }
                });
            }
        });
        let log = fs::read_to_string(project_log_dir_at(&root, &a).unwrap().join("events.jsonl"))
            .unwrap();
        assert_eq!(log.lines().count(), 320);
        for line in log.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }

    #[test]
    fn long_text_is_bounded_on_utf8_boundary_and_logs_rotate() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("logs");
        let project = temp.path().join("project");
        fs::create_dir(&project).unwrap();
        let input = "é".repeat(MAX_TEXT_BYTES);
        let bounded = bounded_text(&input);
        assert!(bounded.contains("[truncated"));
        assert!(bounded.is_char_boundary(bounded.len()));
        for n in 0..90 {
            log_project_event_at(
                &root,
                &project,
                "test",
                serde_json::json!({"n":n,"text":bounded}),
            )
            .unwrap();
        }
        let dir = project_log_dir_at(&root, &project).unwrap();
        assert!(dir.join("events.1.jsonl").exists());
        for name in ["events.jsonl", "events.1.jsonl", "events.2.jsonl"] {
            if let Ok(log) = fs::read_to_string(dir.join(name)) {
                assert!(log.len() <= MAX_LOG_BYTES as usize);
                for line in log.lines() {
                    serde_json::from_str::<serde_json::Value>(line).unwrap();
                }
            }
        }
    }
}
