//! Crash-safe file replacement: write a temp file in the target directory, `sync_all`, rename
//! over the target (atomic replace on POSIX; `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on
//! Windows), then fsync the directory on Unix. A crash leaves either the old or the new file.

use std::io::Write;
use std::path::Path;

/// Marker in the names of in-flight temp files; [`clean_temp_files`] removes leftovers.
pub const TEMP_MARKER: &str = ".tmp-";

/// Atomically replaces `target` with `bytes`.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = target
        .parent()
        .ok_or_else(|| std::io::Error::other("target has no parent directory"))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut temp = tempfile::Builder::new()
        .prefix(&format!("{name}{TEMP_MARKER}"))
        .tempfile_in(dir)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(target).map_err(|e| e.error)?;
    sync_dir(dir);
    Ok(())
}

#[cfg(unix)]
fn sync_dir(dir: &Path) {
    // Best effort: the rename is already atomic; this makes it durable across power loss.
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) {}

/// Removes leftover temp files of interrupted writes in `dir`. Returns how many were removed.
pub fn clean_temp_files(dir: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.contains(TEMP_MARKER) && std::fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}
