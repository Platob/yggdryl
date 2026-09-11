//! A reader that yields UTF-8 whatever its source is encoded in.

use std::io::{self, Read};

use super::{Charset, Decoder};
use crate::DEFAULT_STREAM_BATCH_SIZE;

/// A [`Read`] source decoded to UTF-8 as it is read.
///
/// Built by [`Charset::reader`]. Neither the encoded nor the decoded payload
/// is buffered whole: one source chunk is decoded at a time and handed out
/// before the next is fetched, so a multi-gigabyte file costs two buffers.
///
/// A source that stops in the middle of a sequence fails on the read that
/// reaches its end, rather than quietly returning short text.
pub struct Reader<'source> {
    source: Box<dyn Read + 'source>,
    decoder: Decoder,
    /// One chunk of source bytes, refilled in place.
    chunk: Vec<u8>,
    /// Decoded UTF-8 waiting to be handed out.
    decoded: Vec<u8>,
    /// How much of `decoded` has been handed out.
    offset: usize,
    /// Whether the source has answered its end.
    drained: bool,
}

impl<'source> Reader<'source> {
    /// Decode `source` as `charset`.
    pub(super) fn new<R: Read + 'source>(charset: Charset, source: R) -> Self {
        Self {
            source: Box::new(source),
            decoder: charset.decoder(),
            chunk: vec![0; DEFAULT_STREAM_BATCH_SIZE],
            decoded: Vec::with_capacity(DEFAULT_STREAM_BATCH_SIZE),
            offset: 0,
            drained: false,
        }
    }

    /// The charset being decoded.
    pub const fn charset(&self) -> Charset {
        self.decoder.charset()
    }
}

impl Read for Reader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.offset < self.decoded.len() {
                let take = (self.decoded.len() - self.offset).min(buffer.len());
                buffer[..take].copy_from_slice(&self.decoded[self.offset..self.offset + take]);
                self.offset += take;
                return Ok(take);
            }
            if self.drained {
                return Ok(0);
            }

            self.decoded.clear();
            self.offset = 0;
            let filled = self.source.read(&mut self.chunk)?;
            if filled == 0 {
                self.drained = true;
                if self.decoder.is_pending() {
                    return Err(io::Error::other(super::truncated(
                        self.decoder.charset().as_str(),
                        usize::try_from(self.decoder.consumed()).unwrap_or(usize::MAX),
                        "a whole sequence",
                    )));
                }
                return Ok(0);
            }
            self.decoder
                .push_bytes(&self.chunk[..filled], &mut self.decoded)
                .map_err(io::Error::other)?;
        }
    }
}
