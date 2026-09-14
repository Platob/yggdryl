//! A position over a positional resource.
//!
//! [`IOBase`] is deliberately positional: `pread`/`pwrite` take an offset, so
//! two readers never fight over shared state. A cursor is that state made
//! explicit and *owned by the caller who wants it*: one position, `tell` and
//! `seek`, and read/write operations that advance it. [`Cursor`] carries the
//! position for any handle - it is itself an [`IOBase`], mirroring the handle
//! it wraps - so every implementation gets its cursor through one generic
//! wrapper, and the standard [`Read`](std::io::Read),
//! [`Write`](std::io::Write), and [`Seek`](std::io::Seek) traits come with it.
//!
//! ```
//! use std::io::Read;
//!
//! use yggdryl::{IOBase, IOCursor, holder::Buffer};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut handle = Buffer::new();
//! handle.write_all_bytes(b"symbol,price\n")?;
//!
//! let mut cursor = handle.cursor();
//! cursor.write_next(b"symbol")?;
//! assert_eq!(cursor.tell(), 6);
//!
//! cursor.seek_to(0);
//! let mut head = [0_u8; 6];
//! cursor.read_exact(&mut head)?; // std::io::Read, off the same position
//! assert_eq!(&head, b"symbol");
//! # Ok(())
//! # }
//! ```

use std::io::SeekFrom;

use super::{ByteStream, IOBase};
use crate::{Error, Result};

/// A positioned view over a positional resource.
///
/// The position is the cursor's own state, never the handle's: two cursors
/// over one location advance independently, exactly as two `pread` callers
/// do. Reads and writes advance it; `tell` and `seek` move it; everything a
/// handle answers, the cursor answers unchanged.
pub trait IOCursor: IOBase {
    /// The current position, in bytes from the start.
    fn tell(&self) -> u64;

    /// Set the absolute position.
    ///
    /// Past the end is allowed: a read there yields nothing, and a write
    /// there zero-fills the gap, exactly as `pwrite` does.
    fn seek_to(&mut self, position: u64);

    /// Move the position as [`std::io::SeekFrom`] spells it, returning it.
    ///
    /// # Errors
    ///
    /// Returns an error when the target would sit before the start.
    fn seek(&mut self, from: SeekFrom) -> Result<u64> {
        let base = match from {
            SeekFrom::Start(position) => {
                self.seek_to(position);
                return Ok(position);
            }
            SeekFrom::Current(delta) => (self.tell(), delta),
            SeekFrom::End(delta) => (self.size(), delta),
        };
        let (origin, delta) = base;
        let target = origin
            .checked_add_signed(delta)
            .ok_or_else(|| Error::Codec {
                format: "cursor",
                position: 0,
                reason: "a seek cannot land before the start".into(),
            })?;
        self.seek_to(target);
        Ok(target)
    }

    /// Read at the position, advancing it by what was read.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn read_next(&mut self, buffer: &mut [u8]) -> Result<usize> {
        let read = self.pread(self.tell(), buffer)?;
        self.seek_to(self.tell() + read as u64);
        Ok(read)
    }

    /// Stream byte arrays from the current position and advance as consumed.
    ///
    /// The source is untouched until the first item is requested. Dropping the
    /// iterator leaves the cursor immediately after the last yielded byte.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::ErrorKind::InvalidInput`] when `batch_size` is zero.
    fn stream_bytes(&mut self, batch_size: usize) -> Result<ByteStream<'_>> {
        ByteStream::from_cursor(self, batch_size)
    }

    /// Write at the position, advancing it by what was written.
    ///
    /// # Errors
    ///
    /// Returns the backing store's write failure.
    fn write_next(&mut self, bytes: &[u8]) -> Result<usize> {
        let written = self.pwrite(self.tell(), bytes)?;
        self.seek_to(self.tell() + written as u64);
        Ok(written)
    }
}

/// The one cursor every [`IOBase`] implementation shares.
///
/// Built by [`IOBase::cursor`] and [`IOBase::cursor_at`]. It owns its handle
/// and its position, mirrors the handle's bytes and metadata as an `IOBase`,
/// and implements the standard I/O traits over the same position, so it goes
/// wherever `std::io` readers and writers go.
pub struct Cursor<H: IOBase> {
    handle: H,
    position: u64,
}

impl<H: IOBase> Cursor<H> {
    /// Wrap a handle with the position at the start.
    pub fn new(handle: H) -> Self {
        Self::at(handle, 0)
    }

    /// Wrap a handle with the position at `position`.
    pub fn at(handle: H, position: u64) -> Self {
        Self { handle, position }
    }

    /// Borrow the wrapped handle.
    pub const fn handle(&self) -> &H {
        &self.handle
    }

    /// Borrow the wrapped handle mutably.
    ///
    /// The position is the cursor's own, so nothing the handle does here can
    /// move it.
    pub const fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    /// Consume the cursor and return its handle.
    pub fn into_handle(self) -> H {
        self.handle
    }
}

impl<H: IOBase> crate::IOMedia for Cursor<H> {
    crate::delegate_iomedia!(handle);
}

impl<H: IOBase> IOBase for Cursor<H> {
    crate::delegate_iobase!(handle);
}

impl<H: IOBase> IOCursor for Cursor<H> {
    fn tell(&self) -> u64 {
        self.position
    }

    fn seek_to(&mut self, position: u64) {
        self.position = position;
    }

    /// Keep the wrapped handle's native sequential path alive for the whole
    /// stream while advancing this cursor's disjoint position field.
    fn stream_bytes(&mut self, batch_size: usize) -> Result<ByteStream<'_>> {
        let Self { handle, position } = self;
        let stream = handle.pstream_bytes(*position, batch_size)?;
        ByteStream::from_advancing_stream(stream, position, batch_size)
    }
}

impl<H: IOBase> std::io::Read for Cursor<H> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        self.read_next(buffer).map_err(std::io::Error::other)
    }

    /// Take the whole remainder in one call rather than in a ladder of them.
    ///
    /// [`std::io::Read::read_to_end`] grows its buffer by doubling and asks for what
    /// fits each time, so draining a value costs a call per doubling - on a
    /// store, a round trip per doubling, which for a four-megabyte object is
    /// seventeen of them. The handle answers the remainder in one.
    fn read_to_end(&mut self, into: &mut Vec<u8>) -> std::io::Result<usize> {
        let rest = crate::iobase::rest_of(&self.handle, self.position)?;
        self.position += rest.len() as u64;
        let read = rest.len();
        into.extend_from_slice(&rest);
        Ok(read)
    }
}

impl<H: IOBase> std::io::Write for Cursor<H> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.write_next(bytes).map_err(std::io::Error::other)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        IOBase::flush(&mut self.handle).map_err(std::io::Error::other)
    }
}

impl<H: IOBase> std::io::Seek for Cursor<H> {
    fn seek(&mut self, from: SeekFrom) -> std::io::Result<u64> {
        IOCursor::seek(self, from).map_err(std::io::Error::other)
    }
}
