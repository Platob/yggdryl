//! The complete synchronous Arrow filesystem seam.

use std::any::Any;
use std::collections::BTreeMap;

use crate::{Error, IOKind, Result};

use super::{ByteReader, ByteWriter, RandomAccessReader};

/// Options for one directory listing.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileSelector {
    /// Directory or prefix to list.
    pub base_dir: String,
    /// Whether descendants are included.
    pub recursive: bool,
    /// Whether a missing base directory lists empty.
    pub allow_not_found: bool,
}

impl FileSelector {
    /// Build a selector for `base_dir`.
    pub fn new(base_dir: impl Into<String>, recursive: bool, allow_not_found: bool) -> Self {
        Self {
            base_dir: base_dir.into(),
            recursive,
            allow_not_found,
        }
    }
}

/// Key/value metadata supplied while opening an output stream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OutputMetadata(BTreeMap<String, String>);

impl OutputMetadata {
    /// Construct metadata from key/value entries.
    pub fn from_entries(
        entries: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
        )
    }

    /// Iterate metadata in deterministic key order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// Return whether no entry is present.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a filesystem reports about one exact path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileInfo {
    /// Filesystem-owned path spelling.
    pub path: String,
    /// File, directory, or not-found (`Unknown`).
    pub kind: IOKind,
    /// Byte length for files; absent for every other kind.
    pub size: Option<u64>,
    /// UTC nanoseconds since the Unix epoch, when reported by the backend.
    pub mtime_ns: Option<i64>,
}

impl FileInfo {
    /// Describe a file.
    pub fn file(
        path: impl Into<String>,
        size: impl Into<Option<u64>>,
        mtime_ns: Option<i64>,
    ) -> Self {
        Self {
            path: path.into(),
            kind: IOKind::File,
            size: size.into(),
            mtime_ns,
        }
    }

    /// Describe a directory.
    pub fn directory(path: impl Into<String>, mtime_ns: Option<i64>) -> Self {
        Self {
            path: path.into(),
            kind: IOKind::Directory,
            size: None,
            mtime_ns,
        }
    }

    /// Describe an absent path.
    pub fn not_found(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: IOKind::Unknown,
            size: None,
            mtime_ns: None,
        }
    }
}

/// A deterministic listing that fuses after its first failure.
pub struct FileInfos {
    entries: Option<Box<dyn Iterator<Item = Result<FileInfo>> + Send + Sync>>,
}

impl FileInfos {
    /// An empty listing.
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    /// Wrap an already ordered iterator.
    pub fn new(entries: impl Iterator<Item = Result<FileInfo>> + Send + Sync + 'static) -> Self {
        Self {
            entries: Some(Box::new(entries)),
        }
    }

    /// Yield one failure and then stop.
    pub fn failing(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }
}

impl Iterator for FileInfos {
    type Item = Result<FileInfo>;

    fn next(&mut self) -> Option<Self::Item> {
        let entries = self.entries.as_mut()?;
        match entries.next() {
            Some(Ok(info)) => Some(Ok(info)),
            Some(Err(error)) => {
                self.entries = None;
                Some(Err(error))
            }
            None => {
                self.entries = None;
                None
            }
        }
    }
}

impl std::iter::FusedIterator for FileInfos {}

impl std::fmt::Debug for FileInfos {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileInfos")
            .field("spent", &self.entries.is_none())
            .finish()
    }
}

impl Default for FileInfos {
    fn default() -> Self {
        Self::empty()
    }
}

/// The complete synchronous Arrow filesystem contract.
pub trait FileSystem: Send + Sync + Any {
    /// Backend name used only for safe diagnostics.
    fn type_name(&self) -> &str;
    /// Filesystem equality, including configuration and credential scope.
    fn equals(&self, other: &dyn FileSystem) -> bool;
    /// Fallible host-runtime equality. Native implementations use [`Self::equals`];
    /// language adapters override this so a handler exception is never
    /// reclassified as inequality.
    fn try_equals(&self, other: &dyn FileSystem) -> Result<bool> {
        Ok(self.equals(other))
    }
    /// Explicitly normalize one path according to this filesystem's rules.
    fn normalize_path(&self, path: &str) -> Result<String>;
    /// Inspect one exact path. Absence is a not-found `FileInfo`.
    fn file_info(&self, path: &str) -> Result<FileInfo>;
    /// List one selector in ascending path order.
    fn list(&self, selector: &FileSelector) -> FileInfos;
    /// Create a directory.
    fn create_dir(&self, path: &str, recursive: bool) -> Result<()>;
    /// Delete an empty directory itself.
    fn delete_dir(&self, path: &str) -> Result<()>;
    /// Delete a directory and all descendants when the backend provides one
    /// native recursive operation.
    fn delete_dir_recursive(&self, path: &str) -> Result<()> {
        self.delete_dir_contents(path, false)?;
        self.delete_dir(path)
    }
    /// Delete descendants while retaining the selected directory.
    fn delete_dir_contents(&self, path: &str, missing_dir_ok: bool) -> Result<()>;
    /// Delete every root child while retaining the root.
    fn delete_root_dir_contents(&self) -> Result<()>;
    /// Delete a file. Directories are rejected.
    fn delete_file(&self, path: &str) -> Result<()>;
    /// Copy one file inside this filesystem.
    fn copy_file(&self, source: &str, target: &str) -> Result<()>;
    /// Move one file inside this filesystem.
    fn move_file(&self, source: &str, target: &str) -> Result<()>;
    /// Open a random-access input file.
    fn open_input_file(&self, path: &str) -> Result<Box<dyn RandomAccessReader>>;
    /// Open a sequential input stream.
    fn open_input_stream(&self, path: &str) -> Result<Box<dyn ByteReader>>;
    /// Open a truncating output stream.
    fn open_output_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>>;
    /// Open an append stream.
    fn open_append_stream(
        &self,
        path: &str,
        metadata: Option<&OutputMetadata>,
    ) -> Result<Box<dyn ByteWriter>>;
    /// Borrow the concrete backend for a language binding.
    fn as_any(&self) -> &dyn Any;
}
