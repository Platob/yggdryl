//! Private `std::io` adapters bridging the [`IOBase`](crate::IOBase) surface to the
//! `Read` / `Write` traits that streaming codec backends consume.
//!
//! They let compression stream chunk-by-chunk between two IO resources (e.g. two
//! [`ByteBuffer`](crate::ByteBuffer)s) without materialising the whole input or
//! output. Not part of the public surface — lifetimes stay internal (rule 2).

use std::io::{self, Read, Write};

use crate::{IOBase, IoError, Whence};

/// Maps an [`IoError`] to a `std::io::Error` so it can flow through `Read`/`Write`;
/// the codec error types map it back on the way out via their `From<io::Error>`.
fn to_io(error: IoError) -> io::Error {
    io::Error::other(error.to_string())
}

/// A sequential [`Read`] over an [`IOBase`], consuming from and advancing its
/// cursor.
pub(crate) struct IoReader<'a> {
    io: &'a mut dyn IOBase,
}

impl<'a> IoReader<'a> {
    pub(crate) fn new(io: &'a mut dyn IOBase) -> Self {
        Self { io }
    }
}

impl Read for IoReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // `pread_into` reads straight into `buf` and advances the cursor itself.
        self.io.pread_into(buf, Whence::Current).map_err(to_io)
    }
}

/// A sequential [`Write`] over an [`IOBase`], advancing its cursor and counting the
/// bytes written.
pub(crate) struct IoWriter<'a> {
    io: &'a mut dyn IOBase,
    written: u64,
}

impl<'a> IoWriter<'a> {
    pub(crate) fn new(io: &'a mut dyn IOBase) -> Self {
        Self { io, written: 0 }
    }

    /// The total number of bytes written through this adapter.
    pub(crate) fn written(&self) -> u64 {
        self.written
    }
}

impl Write for IoWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // `pwrite_byte_array` advances the cursor itself.
        let n = self
            .io
            .pwrite_byte_array(buf, Whence::Current)
            .map_err(to_io)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
