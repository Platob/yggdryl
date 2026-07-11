//! `std::io::{Read, Write, Seek}` for `dyn IOBase`.
//!
//! These impls let any [`IOBase`] cursor drive — and be driven by — the standard
//! streaming ecosystem (`io::copy`, codec backends, `read_to_end`, …) directly, with no
//! wrapper type: a `&mut dyn IOBase` **is** a `Read` + `Write` + `Seek` resource. Read
//! and write happen at, and advance, the cursor ([`Whence::Current`]); [`Seek`] maps to
//! [`byte_seek`](IOBase::byte_seek), so byte counts come from the cursor position itself.
//!
//! `std::io` is a Rust-only surface (it does not cross the FFI boundary), so — like the
//! generic IO traits — these impls have no Python/Node counterpart.

use std::io::{self, Read, Seek, SeekFrom, Write};

use crate::{IOBase, IoError, Whence};

/// Maps an [`IoError`] to a `std::io::Error` so it can flow through `Read` / `Write` /
/// `Seek`; codec error types map it back via their `From<io::Error>`.
fn to_io(error: IoError) -> io::Error {
    io::Error::other(error.to_string())
}

impl Read for dyn IOBase + '_ {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `pread_into` reads straight into `buf` (no allocation on contiguous cursors)
        // and advances the cursor itself.
        self.pread_into(buf, Whence::Current).map_err(to_io)
    }
}

impl Write for dyn IOBase + '_ {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // `pwrite_byte_array` advances the cursor and returns the count.
        self.pwrite_byte_array(buf, Whence::Current).map_err(to_io)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Writes land in the backing resource immediately; nothing is buffered here.
        Ok(())
    }
}

impl Seek for dyn IOBase + '_ {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let (offset, whence) = match pos {
            SeekFrom::Start(absolute) => (
                i64::try_from(absolute).map_err(|_| {
                    to_io(IoError::InvalidSeek {
                        offset: i64::MAX,
                        whence: Whence::Start,
                    })
                })?,
                Whence::Start,
            ),
            SeekFrom::Current(delta) => (delta, Whence::Current),
            SeekFrom::End(delta) => (delta, Whence::End),
        };
        self.byte_seek(offset, whence).map_err(to_io)
    }

    fn stream_position(&mut self) -> io::Result<u64> {
        // Cheaper than the default `seek(Current(0))` — read the position directly.
        self.byte_tell().map_err(to_io)
    }
}
