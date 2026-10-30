//! Single-instance file lock for the Service.
//!
//! Holds an OS-level exclusive lock on `<app_data>/courly.lock` for the
//! lifetime of the Service. A second Service spawned against the same data
//! dir gets `AcquireError::Contended` and exits with
//! `BootExitCode::AnotherInstanceRunning`, letting the UI surface "Courly
//! is already running" rather than "Service crashed."
//!
//! Lock release is kernel-managed: on Linux fs2 uses `flock`, which the kernel
//! releases on process exit (clean, panic, or SIGKILL); on Windows fs2 uses
//! `LockFile`, which releases on handle close - again kernel-managed at
//! process exit. Microsoft documents that the released lock may not become
//! available immediately; the respawn algorithm's wait-then-spawn ordering
//! covers the typical case.

use fs2::FileExt;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// RAII handle on the instance lock. Holding the file handle holds the lock;
/// dropping the guard (or terminating the process) releases it.
#[derive(Debug)]
pub(crate) struct LockGuard {
    _file: File,
    _legacy: Option<File>,
}

const LOCK_FILE_NAME: &str = "courly.lock";
/// The lock file of a Service built before the rename to Courly, which may
/// still be running against the same data dir during an update.
const LEGACY_LOCK_FILE_NAME: &str = "ratatoskr.lock";

#[derive(Debug, thiserror::Error)]
pub(crate) enum AcquireError {
    /// Another Service holds the lock for this data dir. Maps to
    /// `BootExitCode::AnotherInstanceRunning` at the call site.
    #[error("instance lock contended: another Service is running")]
    Contended,
    /// Filesystem-level failure (couldn't open the lockfile, couldn't create
    /// the data directory, etc.). Distinct from contention so the call site
    /// can surface a generic IO error rather than the user-friendly
    /// "already running" message.
    #[error("instance lock io error: {0}")]
    Io(#[from] io::Error),
}

/// Try to take the exclusive lock at `<app_data>/courly.lock`. Creates the
/// parent directory if missing (the same pattern `logging::init` uses for
/// `<app_data>/logs/`).
///
/// A lock file left under the name used before the rename is locked too, so
/// a Service of the previous version still running keeps this one out; once
/// held, that file is removed.
pub(crate) fn acquire(app_data_dir: &Path) -> Result<LockGuard, AcquireError> {
    std::fs::create_dir_all(app_data_dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(app_data_dir.join(LOCK_FILE_NAME))?;
    lock(&file)?;
    let legacy_path = app_data_dir.join(LEGACY_LOCK_FILE_NAME);
    let legacy = match OpenOptions::new().write(true).open(&legacy_path) {
        Ok(legacy) => {
            lock(&legacy)?;
            std::fs::remove_file(&legacy_path)?;
            Some(legacy)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(AcquireError::Io(error)),
    };
    Ok(LockGuard {
        _file: file,
        _legacy: legacy,
    })
}

fn lock(file: &File) -> Result<(), AcquireError> {
    match file.try_lock_exclusive() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(AcquireError::Contended),
        Err(error) => Err(AcquireError::Io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn temp_data_dir(suffix: &str) -> std::io::Result<std::path::PathBuf> {
        let path = std::env::current_dir()?.join("target").join(format!(
            "instance-lock-test-{}-{}-{}",
            std::process::id(),
            suffix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(path)
    }

    #[test]
    fn first_acquire_succeeds_and_creates_lockfile() {
        let dir = temp_data_dir("first").expect("temp dir");
        let guard = acquire(&dir).expect("first acquire");
        assert!(dir.join(LOCK_FILE_NAME).exists());
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn second_acquire_returns_contended_while_first_is_held() {
        let dir = temp_data_dir("contended").expect("temp dir");
        let first = acquire(&dir).expect("first acquire");
        match acquire(&dir) {
            Err(AcquireError::Contended) => {}
            Err(other) => panic!("expected Contended, got {other:?}"),
            Ok(_) => panic!("second acquire unexpectedly succeeded while first held"),
        }
        drop(first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn release_after_drop_lets_a_subsequent_acquire_succeed() {
        let dir = temp_data_dir("release").expect("temp dir");
        {
            let _first = acquire(&dir).expect("first acquire");
        }
        let _second = acquire(&dir).expect("second acquire after first released");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_data_dirs_do_not_contend() {
        let a = temp_data_dir("dir-a").expect("temp dir a");
        let b = temp_data_dir("dir-b").expect("temp dir b");
        let _ga = acquire(&a).expect("acquire a");
        let _gb = acquire(&b).expect("acquire b - distinct dir, should not contend");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Holding the guard across an `Arc` clone (the lifetime guarantee the
    /// real Service relies on - the guard moves into the runtime closure)
    /// must keep the lock active.
    #[test]
    fn guard_remains_active_when_held_indirectly() {
        let dir = temp_data_dir("indirect").expect("temp dir");
        let guard = Arc::new(acquire(&dir).expect("first acquire"));
        let _clone = Arc::clone(&guard);
        match acquire(&dir) {
            Err(AcquireError::Contended) => {}
            other => panic!("expected Contended while Arc-held guard is alive, got {other:?}"),
        }
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Panic safety: if a thread holding the lock unwinds via a panic, the
    /// guard's Drop must release the underlying file lock. This is the
    /// kernel-managed-on-process-exit guarantee fs2 was chosen for - locks
    /// release on handle close (which Drop triggers, even on unwind). A
    /// regression that switched to a non-RAII lock primitive would leave
    /// the lock held forever after a panic; this test catches that.
    #[test]
    fn lock_released_when_holder_panics() {
        let dir = temp_data_dir("panic_release").expect("temp dir");
        let dir_for_thread = dir.clone();
        let panicked = std::thread::spawn(move || {
            let _guard = acquire(&dir_for_thread).expect("first acquire");
            panic!("intentional test panic - guard must drop on unwind");
        })
        .join();
        assert!(panicked.is_err(), "thread should have panicked");
        // After the panicking thread's stack unwound, the guard's Drop
        // closed the file handle and the kernel released the lock. A new
        // acquire on the same directory must succeed.
        let _retry = acquire(&dir).expect("acquire after panic-released lock");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_held_lock_of_the_previous_version_keeps_the_service_out() {
        let dir = temp_data_dir("legacy_held").expect("temp dir");
        let old = File::create(dir.join(LEGACY_LOCK_FILE_NAME)).expect("legacy lock");
        old.try_lock_exclusive().expect("old service holds it");
        match acquire(&dir) {
            Err(AcquireError::Contended) => {}
            other => panic!("expected Contended, got {other:?}"),
        }
        drop(old);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_free_lock_of_the_previous_version_is_taken_and_removed() {
        let dir = temp_data_dir("legacy_free").expect("temp dir");
        File::create(dir.join(LEGACY_LOCK_FILE_NAME)).expect("legacy lock");
        let guard = acquire(&dir).expect("acquire");
        assert!(!dir.join(LEGACY_LOCK_FILE_NAME).exists());
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
