//! `local` — the **local-filesystem family**: every type here implements the one
//! [`IOBase`](crate::io::memory::IOBase) contract — bytes, address, *and* the filesystem
//! graph (`ls` / `rm` and, for a directory, the memory-tree byte surface) — addressed by
//! [`Uri`](crate::uri::Uri)s.
//!
//! - [`LocalIO`] — the family's **single access point**: one lazy handle that decides per
//!   call how to serve reads and writes (ad-hoc positioned reads before any write; after the
//!   first auto-creating write, a kept memory-mapped backing at memory speed), and carries
//!   the whole graph surface (streamed `ls`, CRUD, `mkdir`).
//! - [`Mmap`] — the raw memory-mapped file `LocalIO` builds on (usable directly when a
//!   pre-existing file and explicit control are wanted).

mod io;
mod mmap;

pub use io::{LocalChildren, LocalIO, LocalWalk};
pub use mmap::Mmap;

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
    // A `Uri` stores its path percent-ENCODED (`Uri::from_path` escapes spaces and every
    // other non-pchar byte); the filesystem wants the decoded form back.
    let decoded = crate::uri::percent::decode(uri.path());
    let path = decoded.as_ref();
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
