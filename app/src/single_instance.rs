//! Single instance: a second launch asks the running one to show its window.
//!
//! The primary instance holds an exclusive `flock` on `<socket>.lock` (so two
//! simultaneous launches cannot both win) and listens on a Unix socket in
//! `$XDG_RUNTIME_DIR`. A secondary instance connects, writes `activate` and exits.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub fn default_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(rt) if Path::new(&rt).is_dir() => PathBuf::from(rt).join("openfuse.sock"),
        _ => std::env::temp_dir().join(format!("openfuse-{}.sock", unsafe { libc::getuid() })),
    }
}

/// Held by the primary instance; dropping it stops listening.
pub struct Guard {
    path: PathBuf,
    _lock: std::fs::File,
    stop: Arc<AtomicBool>,
}

pub enum Acquire {
    Primary(Guard),
    /// another instance is running and has been told to show itself
    Secondary,
}

fn lock_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

fn notify(path: &Path) -> bool {
    // the primary may still be starting up: retry for a moment
    for _ in 0..20 {
        if let Ok(mut s) = UnixStream::connect(path) {
            let _ = s.set_write_timeout(Some(Duration::from_millis(500)));
            return s.write_all(b"activate\n").is_ok();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// Become the primary instance, or activate the one that is running.
/// `on_activate` runs on a background thread for every activation request.
pub fn acquire(path: &Path, on_activate: impl Fn() + Send + 'static) -> io::Result<Acquire> {
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(lock_path(path))?;
    // SAFETY: valid fd owned by `lock`.
    let rc = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        notify(path);
        return Ok(Acquire::Secondary);
    }
    // We hold the lock: any socket file left is stale (crash).
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    std::thread::Builder::new().name("openfuse-instance".into()).spawn(move || {
        for conn in listener.incoming() {
            if stop2.load(Ordering::SeqCst) {
                break;
            }
            let Ok(mut c) = conn else { continue };
            let _ = c.set_read_timeout(Some(Duration::from_millis(500)));
            let mut buf = [0u8; 64];
            let n = c.read(&mut buf).unwrap_or(0);
            if buf[..n].windows(8).any(|w| w == b"activate") {
                on_activate();
            }
        }
    })?;
    Ok(Acquire::Primary(Guard { path: path.to_path_buf(), _lock: lock, stop }))
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path); // wake the accept loop
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn second_instance_activates_the_first() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("of.sock");
        let (tx, rx) = mpsc::channel();
        let first = acquire(&path, move || tx.send(()).unwrap()).unwrap();
        assert!(matches!(first, Acquire::Primary(_)));
        let second = acquire(&path, || panic!("secondary must not listen")).unwrap();
        assert!(matches!(second, Acquire::Secondary));
        rx.recv_timeout(Duration::from_secs(2)).expect("activation request delivered");
        drop(first);
        assert!(!path.exists(), "socket removed on exit");
        // after the primary is gone, the next launch becomes primary again
        let third = acquire(&path, || {}).unwrap();
        assert!(matches!(third, Acquire::Primary(_)));
    }

    #[test]
    fn stale_socket_is_replaced() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("stale.sock");
        drop(UnixListener::bind(&path).unwrap()); // leaves the file behind, nobody listening
        assert!(path.exists());
        assert!(matches!(acquire(&path, || {}).unwrap(), Acquire::Primary(_)));
    }
}
