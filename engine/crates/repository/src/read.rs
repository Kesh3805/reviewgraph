//! The only file-content read path of this crate (INIT-002).
//!
//! Detectors never open files themselves: they call [`BoundedReader`], which refuses files that
//! were classified as sensitive, symlinks, binary or too large. That is how "never read
//! sensitive files" is enforced in code rather than by convention.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::walk::{FileClass, FileEntry};

/// Raw access to file prefixes. The default implementation reads the real file system; tests
/// substitute a counting double to prove that some paths are never opened.
pub trait FileSource: Send + Sync + std::fmt::Debug {
    /// Reads at most `max` bytes from the start of `absolute`.
    fn read_prefix(&self, absolute: &Path, max: usize) -> io::Result<Vec<u8>>;
}

/// [`FileSource`] over the operating system.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsFiles;

impl FileSource for OsFiles {
    fn read_prefix(&self, absolute: &Path, max: usize) -> io::Result<Vec<u8>> {
        let file = File::open(absolute)?;
        let mut buf = Vec::new();
        file.take(max as u64).read_to_end(&mut buf)?;
        Ok(buf)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    /// The entry's class forbids reading its content.
    #[error("reading {0:?} files is refused")]
    Refused(FileClass),
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// Reads bounded prefixes of classified inventory entries. `Sync` and stateless.
#[derive(Debug, Clone)]
pub struct BoundedReader {
    root: PathBuf,
    source: Arc<dyn FileSource>,
}

impl BoundedReader {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_source(root, Arc::new(OsFiles))
    }

    pub fn with_source(root: impl Into<PathBuf>, source: Arc<dyn FileSource>) -> Self {
        Self {
            root: root.into(),
            source,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn check(entry: &FileEntry) -> Result<(), ReadError> {
        match entry.class {
            FileClass::Source | FileClass::LfsPointer => Ok(()),
            other => Err(ReadError::Refused(other)),
        }
    }

    /// At most `max` bytes from the start of the file.
    pub fn read_prefix(&self, entry: &FileEntry, max: usize) -> Result<Vec<u8>, ReadError> {
        Self::check(entry)?;
        let absolute = self.root.join(entry.path.as_str());
        Ok(self.source.read_prefix(&absolute, max)?)
    }

    /// At most `max` bytes decoded as UTF-8 (invalid sequences are replaced).
    pub fn read_text(&self, entry: &FileEntry, max: usize) -> Result<String, ReadError> {
        let bytes = self.read_prefix(entry, max)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}
