//! A writer that takes UTF-8 and emits one charset's bytes.

use std::io::{self, Write};

use super::{Charset, unicode};
use crate::{DEFAULT_STREAM_BATCH_SIZE, Result};

/// The longest UTF-8 scalar, which bounds what one write can cut short.
const SCALAR: usize = 4;

/// A [`Write`] target that encodes the UTF-8 written to it.
///
/// Built by [`Charset::writer`]. A scalar split across two writes is held
/// until the write that finishes it, so a caller may write in any chunks at
/// all - including one byte at a time.
///
/// The writer must be finished with [`Writer::finish`]: dropping one that is
/// still holding half a scalar loses those bytes silently, and `finish` is
/// where that becomes a refusal.
pub struct Writer<'target> {
    charset: Charset,
    target: Box<dyn Write + 'target>,
    /// The bytes of a scalar the previous write cut short.
    carry: [u8; SCALAR],
    /// How much of `carry` is filled.
    carried: usize,
    /// Encoded bytes, refilled in place rather than reallocated per write.
    buffer: Vec<u8>,
}

impl<'target> Writer<'target> {
    /// Encode everything written here as `charset`.
    pub(super) fn new<W: Write + 'target>(charset: Charset, target: W) -> Self {
        Self {
            charset,
            target: Box::new(target),
            carry: [0; SCALAR],
            carried: 0,
            buffer: Vec::with_capacity(DEFAULT_STREAM_BATCH_SIZE),
        }
    }

    /// The charset being written.
    pub const fn charset(&self) -> Charset {
        self.charset
    }

    /// Flush the target and refuse a payload that stops mid-scalar.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Codec`] when a scalar is still half-written,
    /// and the target's own failure otherwise.
    pub fn finish(mut self) -> Result<()> {
        if self.carried > 0 {
            return Err(super::truncated(
                Charset::Utf8.as_str(),
                self.carried,
                "a whole UTF-8 scalar",
            ));
        }
        self.target.flush()?;
        Ok(())
    }

    /// Encode a run of complete UTF-8 and write it out.
    fn encode_run(&mut self, utf8: &[u8]) -> io::Result<()> {
        if utf8.is_empty() {
            return Ok(());
        }
        // UTF-8 out of UTF-8 changes nothing, so the run goes straight through.
        if self.charset.is_utf8() {
            return self.target.write_all(utf8);
        }
        let text = std::str::from_utf8(utf8).map_err(io::Error::other)?;
        self.buffer.clear();
        self.charset
            .encode_into(text, &mut self.buffer)
            .map_err(io::Error::other)?;
        self.target.write_all(&self.buffer)
    }
}

impl Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut rest = bytes;

        // Finish the scalar the previous write cut short, one byte at a time:
        // a scalar is at most four bytes, so this never touches the bulk.
        while self.carried > 0 && !rest.is_empty() {
            self.carry[self.carried] = rest[0];
            self.carried += 1;
            rest = &rest[1..];
            if unicode::utf8_pending(&self.carry[..self.carried]) == 0 {
                let held = self.carry;
                let carried = self.carried;
                self.carried = 0;
                self.encode_run(&held[..carried])?;
            } else if self.carried == SCALAR {
                return Err(io::Error::other(super::truncated(
                    Charset::Utf8.as_str(),
                    0,
                    "a whole UTF-8 scalar",
                )));
            }
        }

        if rest.is_empty() {
            return Ok(bytes.len());
        }
        let held = unicode::utf8_pending(rest);
        let complete = rest.len() - held;
        self.encode_run(&rest[..complete])?;
        self.carry[..held].copy_from_slice(&rest[complete..]);
        self.carried = held;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.target.flush()
    }
}
