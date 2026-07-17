//! `local` — the **local-filesystem family**: every type here implements both the byte
//! contract ([`IOBase`](crate::io::memory::IOBase)) and the filesystem-graph contract
//! ([`Path`](crate::io::Path)), addressed by [`Uri`](crate::uri::Uri)s.
//!
//! - [`LocalPath`] — the **lazy** node: constructing one never touches the disk; reads on a
//!   missing path are empty, a **write auto-creates** the missing parent folders and the
//!   file. Opens per operation — for repeated access it sub-instantiates the optimized
//!   concrete types below.
//! - [`LocalFile`] — an auto-created, **memory-mapped** file (backed by [`Mmap`]): the
//!   optimized handle for repeated byte access.
//! - [`LocalFolder`] — an auto-created directory: navigation, streamed discovery, CRUD.
//! - [`Mmap`] — the raw memory-mapped file the family builds on.

mod file;
mod folder;
mod mmap;
mod path;

pub use file::LocalFile;
pub use folder::LocalFolder;
pub use mmap::Mmap;
pub use path::{LocalChildren, LocalPath, LocalWalk};

use std::fs::File;
use std::path::{Path as StdPath, PathBuf};

use crate::io::IoError;
use crate::uri::Uri;

/// Builds the guided [`IoError::FileIo`] from an OS error.
pub(crate) fn file_err(op: &'static str, path: &StdPath, error: &std::io::Error) -> IoError {
    IoError::FileIo {
        op,
        path: path.to_string_lossy().into_owned(),
        detail: error.to_string(),
    }
}

/// Resolves a [`Uri`] to a filesystem path: a `file://` URL or a plain-path URI (no scheme).
/// A `file:///C:/x` path keeps its drive letter (the leading slash is stripped on Windows).
pub(crate) fn uri_to_path(uri: &Uri) -> Result<PathBuf, IoError> {
    match uri.scheme() {
        None | Some("file") => {}
        Some(other) => {
            return Err(IoError::FileIo {
                op: "open",
                path: uri.to_string(),
                detail: format!(
                    "unsupported scheme {other:?}: the local family needs a file:// URL or a \
                     plain path URI"
                ),
            });
        }
    }
    let path = uri.path();
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => path,
    };
    if path.is_empty() {
        return Err(IoError::FileIo {
            op: "open",
            path: uri.to_string(),
            detail: "the URI has an empty path; give it a file path".to_string(),
        });
    }
    Ok(PathBuf::from(path))
}

/// Positioned read into `buf` at `offset` — fills as much as the file provides (short at
/// EOF), without relying on any cursor state.
pub(crate) fn read_at(file: &File, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            match file.read_at(&mut buf[done..], offset + done as u64)? {
                0 => break,
                n => done += n,
            }
        }
        Ok(done)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            match file.seek_read(&mut buf[done..], offset + done as u64)? {
                0 => break,
                n => done += n,
            }
        }
        Ok(done)
    }
}

/// Positioned write of all of `data` at `offset` — extends the file (zero-filling any gap)
/// as the OS does for writes past the end.
pub(crate) fn write_at_all(file: &File, offset: u64, data: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(data, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < data.len() {
            let n = file.seek_write(&data[done..], offset + done as u64)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "positioned write made no progress",
                ));
            }
            done += n;
        }
        Ok(())
    }
}
