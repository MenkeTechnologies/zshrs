// Singleton enforcement via flock(LOCK_EX) on daemon.pid.
//
// Per docs/DAEMON.md "Daemon lifecycle":
//   - daemon takes flock(LOCK_EX) on daemon.pid at startup
//   - second instance sees lock held, exits with AlreadyRunning(pid)
//   - if a previous daemon crashed leaving a stale pidfile but no live process,
//     the next attempt acquires the lock cleanly (kernel releases on process exit)
//
// We hold the file open + locked for the lifetime of the daemon process. Returning
// the guard transfers ownership; dropping it closes the fd which releases the lock.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

// `nix::fcntl::flock` is the simpler API; the newer `Flock` wrapper takes
// ownership of the file which clashes with our PidLock { file, path } design
// (we want to hold the file for writing the pid AND keep the lock alive).
// Keeping the legacy fn-form on purpose; suppress the deprecation lint.
#[allow(deprecated)]
use nix::fcntl::{flock, FlockArg};

use super::{paths::CachePaths, DaemonError, Result};

/// Guard that holds the exclusive lock on the daemon.pid file.
#[derive(Debug)]
pub struct PidLock {
    /// `file` field.
    file: File,
    /// `path` field.
    path: PathBuf,
}

impl PidLock {
    /// Acquire the singleton lock for this daemon. If another daemon is running,
    /// returns DaemonError::AlreadyRunning(pid) where pid was read from the file.
    pub fn acquire(paths: &CachePaths) -> Result<Self> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&paths.pid_file)?;

        #[allow(deprecated)]
        let lock_res = flock(file.as_raw_fd(), FlockArg::LockExclusiveNonblock);
        match lock_res {
            Ok(()) => {}
            Err(nix::errno::Errno::EWOULDBLOCK) => {
                let mut buf = String::new();
                file.read_to_string(&mut buf).ok();
                let pid = buf.trim().parse::<i32>().unwrap_or(0);
                return Err(DaemonError::AlreadyRunning(pid));
            }
            Err(e) => return Err(e.into()),
        }

        // Have the lock — write our pid.
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{}", std::process::id())?;
        file.flush()?;

        super::paths::ensure_file_600(&paths.pid_file)?;

        Ok(Self {
            file,
            path: paths.pid_file.clone(),
        })
    }

    /// Path of the locked pid file.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        // Best-effort: clear the file content so a subsequent peek doesn't show stale pid.
        // The kernel releases the flock when the fd closes (which happens after this fn).
        let _ = self.file.set_len(0);
    }
}

/// Acquire the lock; returns the guard or DaemonError::AlreadyRunning.
pub fn acquire(paths: &CachePaths) -> Result<PidLock> {
    PidLock::acquire(paths)
}

/// Like [`acquire`], but while another daemon holds the lock keep retrying for
/// up to `wait`. A replacement spawned by `zcache daemon restart` starts while
/// the old daemon is still draining, so one non-blocking attempt would lose the
/// race and leave no daemon at all.
pub fn acquire_waiting(paths: &CachePaths, wait: std::time::Duration) -> Result<PidLock> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match PidLock::acquire(paths) {
            Err(DaemonError::AlreadyRunning(_)) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// Read the pid stored in daemon.pid without acquiring the lock. Used by clients to
/// check whether a daemon is (likely) running before opening the socket.
pub fn read_pid(paths: &CachePaths) -> Option<i32> {
    let mut s = String::new();
    let mut f = File::open(&paths.pid_file).ok()?;
    f.read_to_string(&mut s).ok()?;
    s.trim().parse::<i32>().ok().filter(|p| *p > 0)
}

/// Try to confirm a process with the given pid is alive. Used for stale-lock cleanup.
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    matches!(kill(Pid::from_raw(pid), None::<Signal>), Ok(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_succeeds_first_time() {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();

        let guard = acquire(&paths).unwrap();
        assert_eq!(guard.path(), paths.pid_file);

        let pid = read_pid(&paths).unwrap();
        assert_eq!(pid, std::process::id() as i32);
    }

    #[test]
    fn second_acquire_blocks_with_already_running() {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();

        let _guard = acquire(&paths).unwrap();

        let err = acquire(&paths).unwrap_err();
        match err {
            DaemonError::AlreadyRunning(pid) => {
                assert_eq!(pid, std::process::id() as i32);
            }
            other => panic!("expected AlreadyRunning, got {:?}", other),
        }
    }

    #[test]
    fn re_acquire_after_drop_succeeds() {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();

        {
            let _guard = acquire(&paths).unwrap();
        }
        // Lock released — second attempt should succeed.
        let _again = acquire(&paths).unwrap();
    }

    #[test]
    fn acquire_waiting_takes_the_lock_once_the_holder_drops_it() {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();

        let guard = acquire(&paths).unwrap();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            drop(guard);
        });
        let started = std::time::Instant::now();
        let again = acquire_waiting(&paths, std::time::Duration::from_secs(5)).unwrap();
        assert!(started.elapsed() >= std::time::Duration::from_millis(150));
        drop(again);
        releaser.join().unwrap();
    }

    #[test]
    fn acquire_waiting_gives_up_after_its_deadline() {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();

        let _guard = acquire(&paths).unwrap();
        let err = acquire_waiting(&paths, std::time::Duration::from_millis(120)).unwrap_err();
        assert!(matches!(err, DaemonError::AlreadyRunning(_)));
    }

    #[test]
    fn pid_alive_self() {
        assert!(pid_alive(std::process::id() as i32));
    }

    #[test]
    fn pid_alive_zero_is_false() {
        assert!(!pid_alive(0));
    }
}
