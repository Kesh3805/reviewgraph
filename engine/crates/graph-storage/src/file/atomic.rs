//! Crash-safe file writes for the file adapter (GS-006).
//!
//! Every file is written as `temp -> sync -> rename`, so a reader (or a crash) sees either the old
//! file or the new one, never a half-written one. `std::fs::rename` replaces an existing file on
//! every platform: POSIX `rename(2)` is atomic, and on Windows Rust calls `MoveFileExW` with
//! `MOVEFILE_REPLACE_EXISTING`.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Marks a temporary file. Files carrying it are removed when a store opens.
pub(crate) const TMP_MARKER: &str = ".tmp-";

fn temp_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!(
        "{name}{TMP_MARKER}{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    ))
}

/// Writes `bytes` to `dir/name` atomically and durably.
pub(crate) fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    let tmp = temp_path(dir, name);
    let result = write_and_rename(dir, name, &tmp, bytes);
    if result.is_err() {
        // Best effort: a leftover temp file is also removed by the next `open`.
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_and_rename(dir: &Path, name: &str, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = create_private(tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, dir.join(name))?;
    sync_dir(dir)
}

#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// Makes the rename itself durable. Directories cannot be opened for syncing on Windows.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> io::Result<()> {
    Ok(())
}

/// Removes leftover temporary files from `dir`. Returns how many were removed.
pub(crate) fn remove_temp_files(dir: &Path) -> io::Result<usize> {
    let mut removed = 0;
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_string_lossy().contains(TMP_MARKER)
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn windows_rename_replaces_existing() {
        let dir = tempfile::tempdir().unwrap();
        write_atomic(dir.path(), "manifest.json", b"first").unwrap();
        write_atomic(dir.path(), "manifest.json", b"second").unwrap();
        assert_eq!(
            fs::read(dir.path().join("manifest.json")).unwrap(),
            b"second"
        );
        let leftovers = fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(TMP_MARKER)
            })
            .count();
        assert_eq!(leftovers, 0, "a successful write leaves no temp file");
    }

    #[test]
    fn remove_temp_files_only_touches_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.bin.zst"), b"x").unwrap();
        fs::write(dir.path().join("keep.bin.zst.tmp-1-abc"), b"x").unwrap();
        assert_eq!(remove_temp_files(dir.path()).unwrap(), 1);
        assert!(dir.path().join("keep.bin.zst").exists());
        assert!(!dir.path().join("keep.bin.zst.tmp-1-abc").exists());
        assert_eq!(
            remove_temp_files(&dir.path().join("missing")).unwrap(),
            0,
            "a missing directory has nothing to clean"
        );
    }
}
