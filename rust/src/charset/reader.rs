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
/// The source is held as it was given rather than boxed, so the reader is
/// `Send` exactly when its source is. What it does with a source that stops
/// in the middle of a sequence is the decoder's: a strict one fails on the
/// read that reaches the end, rather than quietly returning short text, and a
/// transcribing one answers one `U+FFFD` per sequence left and ends
/// cleanly.
pub struct Reader<R: Read> {
    source: R,
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

impl<R: Read> Reader<R> {
    /// Decode `source` through `decoder`.
    pub(crate) fn new(decoder: Decoder, source: R) -> Self {
        Self {
            source,
            decoder,
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

impl<R: Read> Read for Reader<R> {
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
                // The source is done, so the decoder is: what it still holds
                // is finished the way it was built to - a refusal, or the
                // `U+FFFD` a transcribing decoder writes for the bytes left,
                // which the loop hands out before answering the end. A copy
                // is finished because finishing consumes, and the reader is
                // still held.
                self.decoder
                    .clone()
                    .finish_into(&mut self.decoded)
                    .map_err(io::Error::other)?;
                continue;
            }
            self.decoder
                .push_bytes(&self.chunk[..filled], &mut self.decoded)
                .map_err(io::Error::other)?;
        }
    }
}
