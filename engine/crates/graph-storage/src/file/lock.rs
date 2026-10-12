//! The advisory lock on `.review/graph/.lock` that serializes mutations across processes.

use std::fs::File;
use std::path::Path;
use std::time::{Duration, Instant};

use fs2::FileExt;

use crate::StoreError;

/// How long a caller waits for another process before giving up.
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// An exclusive lock, released on drop.
#[derive(Debug)]
pub struct StoreLock {
    file: File,
}

impl StoreLock {
    /// Takes the exclusive lock on `path`, waiting up to `timeout`.
    pub fn acquire(path: &Path, timeout: Duration) -> Result<Self, StoreError> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(StoreError::backend)?;
        let started = Instant::now();
        loop {
            match file.try_lock_exclusive() {
                Ok(()) => return Ok(Self { file }),
                Err(_) if started.elapsed() < timeout => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(_) => return Err(StoreError::Conflict("graph store locked".to_owned())),
            }
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}
