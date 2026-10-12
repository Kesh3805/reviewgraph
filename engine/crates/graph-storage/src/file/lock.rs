//! The advisory lock of the file adapter (GS-006).
//!
//! Two CLI invocations on the same repository serialize their mutations through an exclusive
//! lock on `.lock`; readers take a shared lock. The lock is released when the guard is dropped
//! (closing the handle releases it, also when the process dies).

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::StoreError;

/// How often a contended lock is retried.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

/// A held lock. Dropping it releases the lock.
#[derive(Debug)]
pub(crate) struct StoreLock {
    _file: File,
}

impl StoreLock {
    /// Takes the lock, waiting up to `timeout`; a lock that stays held yields
    /// `StoreError::Conflict("graph store locked")`.
    pub(crate) fn acquire(
        path: &Path,
        exclusive: bool,
        timeout: Duration,
    ) -> Result<Self, StoreError> {
        let file = open_lock_file(path).map_err(StoreError::backend)?;
        let deadline = Instant::now() + timeout;
        loop {
            // Fully qualified: newer `std::fs::File` has inherent methods with similar names.
            let attempt = if exclusive {
                fs2::FileExt::try_lock_exclusive(&file)
            } else {
                fs2::FileExt::try_lock_shared(&file)
            };
            match attempt {
                Ok(()) => return Ok(Self { _file: file }),
                Err(error) if is_contended(&error) => {
                    if Instant::now() >= deadline {
                        return Err(StoreError::Conflict("graph store locked".to_owned()));
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(error) => return Err(StoreError::backend(error)),
            }
        }
    }
}

fn is_contended(error: &io::Error) -> bool {
    error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

#[cfg(unix)]
fn open_lock_file(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_lock_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn exclusive_lock_excludes_and_releases_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".lock");
        let held = StoreLock::acquire(&path, true, Duration::from_secs(1)).unwrap();
        let blocked = StoreLock::acquire(&path, true, Duration::from_millis(60));
        assert!(matches!(blocked, Err(StoreError::Conflict(_))));
        drop(held);
        StoreLock::acquire(&path, true, Duration::from_secs(1)).unwrap();
    }

    #[test]
    fn shared_locks_coexist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".lock");
        let first = StoreLock::acquire(&path, false, Duration::from_secs(1)).unwrap();
        let second = StoreLock::acquire(&path, false, Duration::from_millis(60)).unwrap();
        drop((first, second));
    }
}
