//! A decode fed in whatever chunks a caller already holds its bytes in.

use super::Charset;
use super::sink::Utf8Sink;
use crate::{Error, Result};

/// The most bytes a decoder ever retains between chunks.
///
/// The longest sequence any charset here spells is four bytes - a four-byte
/// UTF-8 scalar, a UTF-16 surrogate pair - so a carry this size is twice what
/// an unfinished one can need. Filling it means the bytes are broken rather
/// than unfinished, and the decoder says so where they are instead of waiting
/// for a continuation that cannot arrive.
const CARRY: usize = 8;

/// What a decoder does with bytes it cannot read exactly.
///
/// The carry - which bytes wait for the chunk that finishes them - is the
/// same in both; only the reading of a complete run differs, and it is the
/// difference between [`Charset::decode`] and [`Charset::transcribe`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    /// Refuse, naming the byte: [`Charset::decoder`].
    Strict,
    /// Read every byte the way [`Charset::transcribe`] reads it, and never
    /// refuse: [`Charset::transcriber`].
    Transcribing,
}

/// A decode of one charset, fed chunk by chunk.
///
/// Built by [`Charset::decoder`], or by [`Charset::transcriber`] for one that
/// never refuses. Each [`Decoder::push`] decodes everything the chunk
/// completes and retains only the bytes a boundary split a sequence across,
/// so decoding a multi-gigabyte payload costs one pass and at most three
/// retained bytes. [`Decoder::finish`] is what turns a payload that stops
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
    mode: Mode,
    /// The tail of the previous chunk that finishes no sequence yet.
    carry: [u8; CARRY],
    /// How much of `carry` is filled.
    carried: usize,
    /// Input bytes this decoder has taken.
    consumed: u64,
}

impl Decoder {
    /// Begin a decode of `charset` that refuses what it cannot read.
    pub(super) const fn new(charset: Charset) -> Self {
        Self::with_mode(charset, Mode::Strict)
    }

    /// Begin a decode of `charset` that transcribes what it cannot read.
    pub(super) const fn transcribing(charset: Charset) -> Self {
        Self::with_mode(charset, Mode::Transcribing)
    }

    const fn with_mode(charset: Charset, mode: Mode) -> Self {
        Self {
            charset,
            mode,
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
    /// A decoder built by [`Charset::transcriber`] refuses nothing.
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
    /// Returns [`crate::Error::Codec`] when bytes are still held back,
    /// whichever way the decoder was built: a transcribing decoder reads every
    /// byte, but bytes still waiting for a continuation were not read, and a
    /// caller with nowhere to put `U+FFFD` is told so.
    pub fn finish(self) -> Result<()> {
        if self.carried == 0 {
            return Ok(());
        }
        Err(self.truncated())
    }

    /// Finish the decode onto the end of `target`.
    ///
    /// A strict decoder refuses a payload that stops mid-sequence exactly as
    /// [`Decoder::finish`] does. A transcribing one has nothing to refuse and
    /// nothing to read either - the bytes left are a sequence that never
    /// finished - so it writes one `U+FFFD` per sequence left, not one
    /// per byte, as a lossy decode of the tail does, and ends cleanly.
    pub(super) fn finish_into(self, target: &mut impl Utf8Sink) -> Result<()> {
        if self.carried == 0 {
            return Ok(());
        }
        match self.mode {
            Mode::Strict => Err(self.truncated()),
            Mode::Transcribing => self
                .charset
                .decode_sink::<true>(&self.carry[..self.carried], target),
        }
    }

    /// The refusal of a payload that stops inside the sequence being held.
    fn truncated(&self) -> Error {
        // A whole decode names the sequence's own start and the charset's own
        // reason; a chunked one held that sequence back, so it is decoded here
        // alone, strictly, and its refusal rebased to where the sequence began
        // - the same answer whichever chunk the cut landed in.
        let base = self.consumed.saturating_sub(self.carried as u64);
        let mut scratch = String::new();
        match self
            .charset
            .decode_sink::<false>(&self.carry[..self.carried], &mut scratch)
        {
            Err(error) => rebased(error, base),
            Ok(()) => super::truncated(
                self.charset.as_str(),
                usize::try_from(base).unwrap_or(usize::MAX),
                "a whole sequence",
            ),
        }
    }

    /// Read the complete sequences at the front of `run`, the way the mode
    /// says.
    ///
    /// `complete` is how much of `run` finishes a sequence; what follows is
    /// the tail the next chunk finishes. `base` is where `run` begins,
    /// measured from the first byte this decoder was fed: a refusal from the
    /// charset names a position inside `run`, and the position the caller is
    /// promised is the one from the start.
    fn decode_run(
        &self,
        run: &[u8],
        complete: usize,
        base: u64,
        target: &mut impl Utf8Sink,
    ) -> Result<()> {
        let front = &run[..complete];
        let result = match self.mode {
            Mode::Strict => self.charset.decode_sink::<false>(front, target),
            Mode::Transcribing => self.charset.transcribe_sink(front, target),
        };
        let Err(error) = result else {
            return Ok(());
        };
        if self.mode != Mode::Strict || complete == run.len() {
            return Err(rebased(error, base));
        }
        // The front refused, and bytes follow it: what the charset saw as
        // input ending inside a sequence is a sequence the next byte breaks
        // - a lead byte held from one chunk and followed by another lead -
        // and the refusal a whole decode of the bytes gives, at the same
        // position and naming that byte, is the one the caller is promised.
        // Read again into nothing, because `target` may already hold the
        // text before the fault.
        let mut scratch = String::new();
        let whole = self.charset.decode_sink::<false>(run, &mut scratch);
        Err(rebased(whole.err().unwrap_or(error), base))
    }

    /// The one chunked decode, written against whichever target a caller
    /// brought.
    fn push_sink(&mut self, chunk: &[u8], target: &mut impl Utf8Sink) -> Result<()> {
        let mut rest = chunk;
        // A join can end with a carry and bytes still to read: a lead byte
        // held from the last chunk, broken by the lead that follows it, is
        // read alone and the breaker takes its place in the carry. That carry
        // is joined with what follows too, or the run below would overwrite
        // it and the byte would be lost - `a\xc3` then `\xc3\xa9` read `aÃ©`
        // for `aÃé`. Every join takes at least one byte, so this ends.
        while self.carried > 0 && !rest.is_empty() {
            rest = self.join(rest, target)?;
        }
        if rest.is_empty() {
            return Ok(());
        }
        // Everything but the tail a sequence is split across decodes now; the
        // tail waits for the chunk that finishes it.
        let held = self.charset.pending(rest);
        let complete = rest.len() - held;
        self.decode_run(rest, complete, self.consumed, target)?;
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
        // The carried bytes were counted when they were taken, so the joined
        // sequence begins that many bytes before the count.
        let base = self.consumed.saturating_sub(carried as u64);

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
            self.decode_run(&joined[..filled], complete, base, target)?;
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
            self.decode_run(&joined[..filled], filled, base, target)?;
            self.carried = 0;
            return Ok(&rest[taken..]);
        }
        self.carry[..filled].copy_from_slice(&joined[..filled]);
        self.carried = filled;
        Ok(&rest[taken..])
    }
}

/// A refusal re-measured from the first byte the decoder was fed.
///
/// A charset names the position inside the run it was handed; `base` is
/// where that run began.
fn rebased(error: Error, base: u64) -> Error {
    match error {
        Error::Codec {
            format,
            position,
            reason,
        } => Error::Codec {
            format,
            position: position.saturating_add(usize::try_from(base).unwrap_or(usize::MAX)),
            reason,
        },
        other => other,
    }
}
