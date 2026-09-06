//! Object-safe byte streams used by [`super::FileSystem`].

use std::any::Any;
use std::io::SeekFrom;

use smol_str::SmolStr;

use crate::{Error, Result};

/// A sequential input stream.
pub trait ByteReader: Send {
    /// Read up to `buffer.len()` bytes and advance the stream.
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize>;

    /// Return the current byte offset.
    fn tell(&self) -> u64;

    /// Close the stream. Repeated calls after a successful close are successful.
    fn close(&mut self) -> Result<()>;

    /// Return whether the stream has been closed.
    fn closed(&self) -> bool;

    /// Borrow the concrete implementation for a language binding.
    fn as_any(&self) -> &dyn Any;

    /// Consume the stream as its concrete implementation.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// A seekable input file with positional reads.
pub trait RandomAccessReader: ByteReader {
    /// Read at `offset` without changing the current position.
    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    /// Move the current position and return it.
    fn seek(&mut self, from: SeekFrom) -> Result<u64>;

    /// Consume the stream as its concrete implementation.
    fn into_random_any(self: Box<Self>) -> Box<dyn Any>;
}

/// A sequential output or append stream.
pub trait ByteWriter: Send {
    /// Write bytes at the current position and advance it.
    fn write(&mut self, bytes: &[u8]) -> Result<usize>;

    /// Return the current byte offset.
    fn tell(&self) -> u64;

    /// Publish buffered writes without closing the stream.
    fn flush(&mut self) -> Result<()>;

    /// Flush and close exactly once. A failed close remains visible; repeated
    /// calls after a successful close are successful.
    fn close(&mut self) -> Result<()>;

    /// Return whether the stream has been closed.
    fn closed(&self) -> bool;

    /// Borrow the concrete implementation for a language binding.
    fn as_any(&self) -> &dyn Any;

    /// Consume the stream as its concrete implementation.
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// A repeatable stream failure. `std::io::Error` is not cloneable, so a
/// writer retains the typed fields that affect boundary translation rather
/// than dropping a write or close failure after reporting it once.
pub(crate) enum StreamFailure {
    Io(std::io::ErrorKind, String),
    Absent {
        expected: &'static str,
        path: SmolStr,
    },
    Conflict {
        expected: &'static str,
        actual: &'static str,
        path: SmolStr,
    },
    Unsupported {
        operation: &'static str,
        filesystem: SmolStr,
    },
    Other(String),
}

impl StreamFailure {
    pub(crate) fn from_error(error: &Error) -> Self {
        match error {
            Error::Io(error) => Self::Io(error.kind(), error.to_string()),
            Error::Absent { expected, path } => Self::Absent {
                expected,
                path: path.clone(),
            },
            Error::Conflict {
                expected,
                actual,
                path,
            } => Self::Conflict {
                expected,
                actual,
                path: path.clone(),
            },
            Error::Unsupported {
                operation,
                filesystem,
            } => Self::Unsupported {
                operation,
                filesystem: filesystem.clone(),
            },
            other => Self::Other(other.to_string()),
        }
    }

    pub(crate) fn error(&self) -> Error {
        match self {
            Self::Io(kind, message) => Error::Io(std::io::Error::new(*kind, message.clone())),
            Self::Absent { expected, path } => Error::Absent {
                expected,
                path: path.clone(),
            },
            Self::Conflict {
                expected,
                actual,
                path,
            } => Error::Conflict {
                expected,
                actual,
                path: path.clone(),
            },
            Self::Unsupported {
                operation,
                filesystem,
            } => Error::Unsupported {
                operation,
                filesystem: filesystem.clone(),
            },
            Self::Other(message) => Error::Io(std::io::Error::other(message.clone())),
        }
    }
}
