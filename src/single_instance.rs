//! One GUI process per data directory.
//!
//! Two GUIs on the same directory share `session.json` and the SQLite store: two WebSocket logins
//! on one account, racing writes and confusing sync. The first GUI holds an exclusive lock on
//! [`LOCK_FILE`] for its whole life (the OS releases it when the process exits or crashes); a
//! second launch asks the first to show its window and exits.
//!
//! The lock file name matches upstream ctdozlite on purpose, so the two apps also exclude each
//! other on a shared data directory (their databases are not compatible, see AGENTS.md §7).

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub const LOCK_FILE: &str = "ctdozlite.lock";
/// Unix socket the running GUI listens on for `show` requests from a second launch.
#[cfg(unix)]
pub const SOCKET_FILE: &str = "ctdozlite.sock";

#[cfg(unix)]
const SHOW_REQUEST: &[u8] = b"show\n";
/// `sun_path` holds 104 bytes on macOS (108 on Linux), including the terminating NUL.
#[cfg(unix)]
const MAX_SOCKET_PATH: usize = 103;

static SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Whether a second launch asked for the window since the last call (consumed by the UI loop).
pub fn take_show_request() -> bool {
    SHOW_REQUESTED.swap(false, Ordering::Relaxed)
}

/// Outcome of [`acquire`].
pub enum Instance {
    /// This process is the GUI for the directory. Keep the value alive until exit.
    Primary(#[allow(dead_code)] Option<File>),
    /// Another process already holds the lock.
    AlreadyRunning,
}

/// Take the directory's GUI lock, retrying for up to `wait` while another process holds it.
///
/// A relaunch by the updater passes a wait so it does not lose the race against the instance
/// that is quitting; a normal launch passes zero.
pub fn acquire_waiting(dir: &Path, wait: std::time::Duration) -> Instance {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match acquire(dir) {
            Instance::AlreadyRunning if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            other => return other,
        }
    }
}

/// Take the directory's GUI lock without blocking.
///
/// Fails open: when the lock file cannot be opened or locked for another reason, the error is
/// logged and the app starts as if it were alone (the pre-lock behaviour).
pub fn acquire(dir: &Path) -> Instance {
    let path = dir.join(LOCK_FILE);
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&path) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("! Không mở được tệp khóa {}: {error}", path.display());
            return Instance::Primary(None);
        }
    };
    match file.try_lock() {
        Ok(()) => Instance::Primary(Some(file)),
        Err(TryLockError::WouldBlock) => Instance::AlreadyRunning,
        Err(TryLockError::Error(error)) => {
            eprintln!("! Không khóa được {}: {error}", path.display());
            Instance::Primary(None)
        }
    }
}

/// Ask the GUI that holds the lock to bring its window to the front. Returns whether the request
/// was delivered (always `false` off Unix).
pub fn notify_running(dir: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::net::UnixStream;
        let Some(path) = socket_path(dir) else {
            return false;
        };
        let Ok(mut stream) = UnixStream::connect(path) else {
            return false;
        };
        let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(2)));
        stream.write_all(SHOW_REQUEST).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        false
    }
}

/// Listen for `show` requests on a background thread. Call only while holding the lock: a stale
/// socket left by a crashed run is replaced. Each request sets the flag read by
/// [`take_show_request`] and wakes the UI with `wake`.
pub fn serve_show_requests(dir: &Path, wake: impl Fn() + Send + 'static) {
    #[cfg(unix)]
    {
        let dir = dir.to_path_buf();
        // Resolving and binding touch the filesystem: keep them off the caller (the GUI thread).
        let spawned = std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                use std::io::Read;
                let path = socket_path(&dir);
                let listener = match path.as_deref().map(bind_private_socket) {
                    Some(Ok(listener)) => listener,
                    Some(Err(error)) => {
                        eprintln!("! Không mở được kênh mở lại cửa sổ: {error:#}");
                        return;
                    }
                    None => {
                        eprintln!("! Đường dẫn thư mục dữ liệu quá dài cho kênh mở lại cửa sổ.");
                        return;
                    }
                };
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(500)));
                    let mut buf = [0u8; 16];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if buf[..n] == *SHOW_REQUEST {
                        SHOW_REQUESTED.store(true, Ordering::Relaxed);
                        wake();
                    }
                }
            });
        if let Err(error) = spawned {
            eprintln!("! Không tạo được luồng mở lại cửa sổ: {error}");
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, wake);
    }
}

/// Socket address for `dir`: the full path, or the bare file name when the full path is too long
/// for a Unix socket address and `dir` is the working directory (the data directory always is,
/// see `data_dir::initialize`).
#[cfg(unix)]
fn socket_path(dir: &Path) -> Option<std::path::PathBuf> {
    let full = dir.join(SOCKET_FILE);
    if full.as_os_str().len() <= MAX_SOCKET_PATH {
        return Some(full);
    }
    let cwd = std::env::current_dir().ok()?;
    let same = |a: &Path, b: &Path| match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    same(dir, &cwd).then(|| std::path::PathBuf::from(SOCKET_FILE))
}

/// Bind `path` as an owner-only socket, replacing a stale socket file (never another file type).
#[cfg(unix)]
fn bind_private_socket(path: &Path) -> anyhow::Result<std::os::unix::net::UnixListener> {
    use anyhow::{bail, Context};
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            std::fs::remove_file(path).context("không xóa được socket cũ")?;
        }
        Ok(_) => bail!("{} không phải socket", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("không đọc được socket cũ"),
    }
    let listener = std::os::unix::net::UnixListener::bind(path)
        .with_context(|| format!("không tạo được {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .context("không đặt được quyền socket")?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_lock_on_the_same_directory_is_refused() {
        let dir = crate::data_dir::PrivateTempDir::new("single-instance").unwrap();
        let first = acquire(dir.path());
        assert!(matches!(first, Instance::Primary(Some(_))));
        assert!(matches!(acquire(dir.path()), Instance::AlreadyRunning));
        drop(first);
        assert!(matches!(acquire(dir.path()), Instance::Primary(Some(_))));
    }

    #[cfg(unix)]
    #[test]
    fn show_request_reaches_the_running_instance_through_a_stale_socket() {
        use std::sync::mpsc;
        let dir = crate::data_dir::PrivateTempDir::new("single-instance-sock").unwrap();
        // A socket file left by a crashed run.
        drop(std::os::unix::net::UnixListener::bind(dir.path().join(SOCKET_FILE)).unwrap());
        let (tx, rx) = mpsc::channel();
        serve_show_requests(dir.path(), move || {
            let _ = tx.send(());
        });
        // The listener binds on its own thread; retry until it is up.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !notify_running(dir.path()) {
            assert!(
                std::time::Instant::now() < deadline,
                "listener never came up"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(take_show_request());
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(SOCKET_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_long_data_directory_falls_back_to_a_relative_socket_or_none() {
        let long = std::path::PathBuf::from("/").join("d".repeat(200));
        // Not the working directory: no usable address rather than a bind that always fails.
        assert_eq!(socket_path(&long), None);
        let short = std::path::Path::new("/tmp/x");
        assert_eq!(socket_path(short), Some(short.join(SOCKET_FILE)));
    }

    #[cfg(unix)]
    #[test]
    fn a_regular_file_in_place_of_the_socket_is_never_deleted() {
        let dir = crate::data_dir::PrivateTempDir::new("single-instance-file").unwrap();
        let path = dir.path().join(SOCKET_FILE);
        std::fs::write(&path, b"not a socket").unwrap();
        assert!(bind_private_socket(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not a socket");
    }
}
