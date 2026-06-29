//! The [`Io`] byte abstraction and its in-memory implementation.
//!
//! `Io` centralises positional and cursor-based byte access behind one trait, so
//! every byte source (an in-memory [`MemoryIo`] today; files / cloud objects /
//! HTTP bodies later) exposes the same `pread`/`pwrite`, `size`, `tell`/`seek`
//! and capacity surface. Memory-resident sources hand reads back as zero-copy
//! [`BinaryScalar`] views, so a read does not copy the bytes out of the store.

mod memory;

pub use memory::MemoryIo;

use crate::error::IoError;
use crate::scalar::BinaryScalar;

/// Where a [`seek`](Io::seek) offset is measured from (`SEEK_SET`/`CUR`/`END`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Whence {
    /// From the start of the stream (absolute).
    #[default]
    Start,
    /// From the current cursor position.
    Current,
    /// From the end of the stream (its size).
    End,
}

/// Positional and cursor-based byte access shared by every byte source.
///
/// Implementors provide the positional primitives ([`pread_into`](Io::pread_into),
/// [`pread`](Io::pread), [`pwrite`](Io::pwrite)), [`size`](Io::size) and the
/// cursor ([`tell`](Io::tell)/[`seek`](Io::seek)); the cursor-based
/// [`read`](Io::read)/[`read_into`](Io::read_into)/[`write`](Io::write) helpers are
/// derived from them by default.
pub trait Io {
    /// The number of valid bytes.
    fn size(&self) -> u64;

    /// The current cursor position.
    fn tell(&self) -> u64;

    /// Moves the cursor; returns the new absolute position.
    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError>;

    /// Reads up to `dst.len()` bytes at absolute `offset` into `dst`; returns the
    /// number read (`0` at or past the end). Does not move the cursor.
    fn pread_into(&self, offset: u64, dst: &mut [u8]) -> Result<usize, IoError>;

    /// Reads up to `len` bytes at absolute `offset` as a (zero-copy, when in
    /// memory) [`BinaryScalar`]. Does not move the cursor.
    fn pread(&self, offset: u64, len: usize) -> Result<BinaryScalar, IoError>;

    /// Writes `src` at absolute `offset`, growing the stream if needed; returns
    /// the number of bytes written. Does not move the cursor.
    fn pwrite(&mut self, offset: u64, src: &[u8]) -> Result<usize, IoError>;

    /// The allocated capacity in bytes (always `>= size()`). Defaults to `size()`.
    fn capacity(&self) -> u64 {
        self.size()
    }

    /// Sets the allocated capacity. Unsupported by default.
    fn set_capacity(&mut self, _capacity: u64) -> Result<(), IoError> {
        Err(IoError::Unsupported("set_capacity"))
    }

    /// Resizes the logical length to `new_size`, filling any new bytes with
    /// `fill`. Unsupported by default.
    fn resize(&mut self, _new_size: u64, _fill: u8) -> Result<(), IoError> {
        Err(IoError::Unsupported("resize"))
    }

    /// Cursor-based read of up to `len` bytes; advances the cursor by the number
    /// of bytes read.
    fn read(&mut self, len: usize) -> Result<BinaryScalar, IoError> {
        let scalar = self.pread(self.tell(), len)?;
        let read = scalar.len().unwrap_or(0);
        self.seek(read as i64, Whence::Current)?;
        Ok(scalar)
    }

    /// Cursor-based read into `dst`; advances the cursor by the number of bytes
    /// read.
    fn read_into(&mut self, dst: &mut [u8]) -> Result<usize, IoError> {
        let read = self.pread_into(self.tell(), dst)?;
        self.seek(read as i64, Whence::Current)?;
        Ok(read)
    }

    /// Cursor-based write; advances the cursor by the number of bytes written.
    fn write(&mut self, src: &[u8]) -> Result<usize, IoError> {
        let written = self.pwrite(self.tell(), src)?;
        self.seek(written as i64, Whence::Current)?;
        Ok(written)
    }
}
