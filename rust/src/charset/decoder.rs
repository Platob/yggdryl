//! A decode fed in whatever chunks a caller already holds its bytes in.

use super::Charset;
use super::sink::Utf8Sink;
use crate::Result;

/// The most bytes a decoder ever retains between chunks.
///
/// The longest sequence any charset here spells is four bytes - a four-byte
/// UTF-8 scalar, a UTF-16 surrogate pair - so a carry this size is twice what
/// an unfinished one can need. Filling it means the bytes are broken rather
/// than unfinished, and the decoder says so where they are instead of waiting
/// for a continuation that cannot arrive.
const CARRY: usize = 8;

/// A decode of one charset, fed chunk by chunk.
///
/// Built by [`Charset::decoder`]. Each [`Decoder::push`] decodes everything the
/// chunk completes and retains only the bytes a boundary split a sequence
/// across, so decoding a multi-gigabyte payload costs one pass and at most
/// three retained bytes. [`Decoder::finish`] is what turns a payload that stops
/// mid-sequence into a refusal rather than silence.
///
/// ```
/// use yggdryl::Charset;
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut decoder = Charset::Utf8.decoder();
/// let mut text = String::new();
///
/// // The two bytes of "é" arrive in different chunks.
/// decoder.push(b"caf\xc3", &mut text)?;
/// assert_eq!(text, "caf");
/// decoder.push(b"\xa9", &mut text)?;
/// assert_eq!(text, "café");
/// decoder.finish()?;
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct Decoder {
    charset: Charset,
    /// The tail of the previous chunk that finishes no sequence yet.
    carry: [u8; CARRY],
    /// How much of `carry` is filled.
    carried: usize,
    /// Input bytes this decoder has taken.
    consumed: u64,
}

impl Decoder {
    /// Begin a decode of `charset`.
    pub(super) const fn new(charset: Charset) -> Self {
        Self {
            charset,
            carry: [0; CARRY],
            carried: 0,
            consumed: 0,
        }
    }

    /// The charset being decoded.
    pub const fn charset(&self) -> Charset {
        self.charset
    }

    /// How many input bytes this decoder has taken.
    pub const fn consumed(&self) -> u64 {
        self.consumed
    }

    /// Whether a sequence is half-read, so more input is needed.
    pub const fn is_pending(&self) -> bool {
        self.carried > 0
    }

    /// Decode the next chunk onto the end of `target`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Codec`] naming the charset and the position of
    /// the byte it refused, measured from the first byte this decoder was fed.
    pub fn push(&mut self, chunk: &[u8], target: &mut String) -> Result<()> {
        self.push_sink(chunk, target)
    }

    /// Decode the next chunk onto the end of a UTF-8 byte target.
    ///
    /// # Errors
    ///
    /// Returns the same refusal as [`Decoder::push`].
    pub fn push_bytes(&mut self, chunk: &[u8], target: &mut Vec<u8>) -> Result<()> {
        self.push_sink(chunk, target)
    }

    /// Finish the decode, refusing a payload that stops mid-sequence.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Codec`] when bytes are still held back.
    pub fn finish(self) -> Result<()> {
        if self.carried == 0 {
            return Ok(());
        }
        Err(super::truncated(
            self.charset.as_str(),
            usize::try_from(self.consumed).unwrap_or(usize::MAX),
            "a whole sequence",
        ))
    }

    /// The one chunked decode, written against whichever target a caller
    /// brought.
    fn push_sink(&mut self, chunk: &[u8], target: &mut impl Utf8Sink) -> Result<()> {
        let mut rest = chunk;
        if self.carried > 0 {
            rest = self.join(rest, target)?;
        }
        if rest.is_empty() {
            return Ok(());
        }
        // Everything but the tail a sequence is split across decodes now; the
        // tail waits for the chunk that finishes it.
        let held = self.charset.pending(rest);
        let complete = rest.len() - held;
        self.charset
            .decode_sink::<false>(&rest[..complete], target)?;
        self.carry[..held].copy_from_slice(&rest[complete..]);
        self.carried = held;
        self.consumed = self.consumed.saturating_add(rest.len() as u64);
        Ok(())
    }

    /// Finish the sequence the previous chunk cut short, returning what is
    /// left of this one.
    ///
    /// Bytes are added one at a time because the shortest completion is the
    /// right one: a sequence is at most four bytes, so this runs at most a
    /// handful of times and never touches the bulk of a chunk.
    fn join<'chunk>(
        &mut self,
        rest: &'chunk [u8],
        target: &mut impl Utf8Sink,
    ) -> Result<&'chunk [u8]> {
        let carried = self.carried;
        let mut joined = [0_u8; CARRY];
        joined[..carried].copy_from_slice(&self.carry[..carried]);

        let window = (CARRY - carried).min(rest.len());
        let mut taken = 0;
        while taken < window {
            joined[carried + taken] = rest[taken];
            taken += 1;
            let filled = carried + taken;
            let complete = filled - self.charset.pending(&joined[..filled]);
            if complete < carried {
                continue;
            }
            // The held bytes are decodable now, so they leave the carry - as
            // text, or as the refusal they always were.
            self.charset
                .decode_sink::<false>(&joined[..complete], target)?;
            let leftover = filled - complete;
            self.carry[..leftover].copy_from_slice(&joined[complete..filled]);
            self.carried = leftover;
            self.consumed = self.consumed.saturating_add(taken as u64);
            return Ok(&rest[taken..]);
        }

        let filled = carried + taken;
        self.consumed = self.consumed.saturating_add(taken as u64);
        if filled == CARRY {
            // Longer than any sequence: broken rather than unfinished.
            self.charset
                .decode_sink::<false>(&joined[..filled], target)?;
            self.carried = 0;
            return Ok(&rest[taken..]);
        }
        self.carry[..filled].copy_from_slice(&joined[..filled]);
        self.carried = filled;
        Ok(&rest[taken..])
    }
}
