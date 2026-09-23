//! UTF-8: the charset every decoded value in this crate is already in, and the
//! six string leaves that carry it. One module because the codec and the
//! leaves make one promise - a value holds UTF-8 - so under this charset a
//! decode is a validation and a borrow, an encode is the bytes themselves, and
//! the one reading of a byte offered as UTF-8 that is not - every valid run
//! kept, every other byte read as windows-1252 - is written here once for the
//! transcribing door of both charsets that ride Arrow's text storage.

use std::borrow::Cow;

use crate::charset::sink::Utf8Sink;
use crate::charset::{truncated, undecodable};
use crate::{DataType, Error, Result, StringType};

/// The canonical name this charset reports itself by.
pub(crate) const NAME: &str = "utf-8";

/// Decode UTF-8, which is a validation and a borrow rather than a transcode.
pub(crate) fn decode(input: &[u8]) -> Result<Cow<'_, str>> {
    std::str::from_utf8(input)
        .map(Cow::Borrowed)
        .map_err(|error| fault(input, error))
}

/// Decode UTF-8, replacing each broken sequence with `U+FFFD`.
pub(crate) fn decode_lossy(input: &[u8]) -> Cow<'_, str> {
    String::from_utf8_lossy(input)
}

/// Decode UTF-8 into a target.
pub(crate) fn decode_into<const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    target.reserve(input.len());
    if LOSSY {
        return match String::from_utf8_lossy(input) {
            Cow::Borrowed(text) => target.push_utf8(text.as_bytes()),
            Cow::Owned(text) => target.push_utf8(text.as_bytes()),
        };
    }
    match std::str::from_utf8(input) {
        Ok(text) => target.push_utf8(text.as_bytes()),
        Err(error) => Err(fault(input, error)),
    }
}

/// How many trailing bytes of `input` begin a sequence it cuts short.
///
/// A genuinely invalid byte is not pending: it answers zero so the decode that
/// follows reports it with its position rather than waiting forever for a
/// continuation that would not fix it. Only the tail is judged, because only
/// the tail can be cut short: a fault earlier in the input is the decode's
/// to refuse or transcribe, and must not hide a sequence the boundary split
/// after it - a transcribing decode reads on past the fault, and would
/// otherwise read the held lead byte as a stray one.
pub(crate) fn pending(input: &[u8]) -> usize {
    // A sequence is at most four bytes, so one cut short is a lead byte
    // among the last three followed only by continuation bytes.
    let tail = &input[input.len().saturating_sub(3)..];
    let Some(lead) = tail.iter().rposition(|byte| byte & 0xC0 != 0x80) else {
        return 0;
    };
    match std::str::from_utf8(&tail[lead..]) {
        Err(error) if error.error_len().is_none() => tail.len() - lead,
        _ => 0,
    }
}

/// Read bytes offered as UTF-8 that may not be, borrowing them when they are.
///
/// Valid UTF-8 is its own transcription; anything else is
/// [`transcribe_into`], the one rule for a stray byte. A US-ASCII declaration
/// is a UTF-8 declaration with a narrower promise, so
/// [`Charset::transcribe`](crate::Charset::transcribe) reads both through
/// this door.
pub(crate) fn transcribe(input: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(input) {
        Ok(borrowed) => Cow::Borrowed(borrowed),
        Err(_) => {
            let mut target = String::new();
            transcribe_into(input, &mut target);
            Cow::Owned(target)
        }
    }
}

/// Read bytes offered as UTF-8 that may not be, appending the text to
/// `target` and answering how many bytes were not UTF-8.
///
/// This is the one reading of a stray byte, and every door that reads bytes
/// offered as UTF-8 or as US-ASCII without refusing them is this function:
/// [`Charset::transcribe`](crate::Charset::transcribe) under both charsets,
/// and the text line where it is made. Every valid UTF-8 run is kept as it
/// is, and every other byte reads as the character Windows-1252 gives it. Per
/// invalid run rather than per buffer, because a buffer is mostly UTF-8 with
/// a stray byte far more often than it is wholly Windows-1252: reading a valid
/// `é` (`C3 A9`) as `Ã©` because a lone `0xE9` stands elsewhere would destroy
/// what was right to repair what was wrong, and a wholly Windows-1252 buffer
/// has no valid multi-byte run to keep and reads byte for byte either way.
///
/// The table and the rule for its five holes are not restated here because
/// they are [`crate::cp1252`]'s: the table is the generated `windows-1252`,
/// checked against Python's codec registry in both directions, and the holes
/// rule is the one `SingleByte::transcribe_sink` already applies to an
/// unassigned byte of any Windows page. `utf8_chunks()` never puts a byte
/// below `0x80` in an invalid run, so that walk is one table entry per byte.
/// The walk is byte-wise over the whole buffer and never the chunked
/// [`Decoder`](crate::charset::Decoder), which would hold a sequence the
/// buffer cuts short as pending: `E2 82` at the end is the two bytes that are
/// left, and they read `â‚`.
pub(crate) fn transcribe_into(input: &[u8], target: &mut String) -> usize {
    // A `String` re-checks each run it is handed, and every run here is one
    // `utf8_chunks` proved or one the table walked scalar by scalar, so the
    // refusal the sink trait lets a target spell cannot arrive. This door
    // exists because the trait is the charset layer's and the reading is the
    // crate's.
    transcribe_sink(input, target).expect("a run utf8_chunks proved is text by construction")
}

/// [`transcribe_into`] over whichever target a caller brought.
///
/// # Errors
///
/// Returns the refusal of a target that re-checks a run, which a run
/// `utf8_chunks` proved never trips.
pub(crate) fn transcribe_sink(input: &[u8], target: &mut impl Utf8Sink) -> Result<usize> {
    // Every byte answers at least one UTF-8 byte and a stray byte at most
    // three, so the input length is a floor: reserved once here, and the
    // table's walk reserves its exact answer for each run it reads.
    target.reserve(input.len());
    let mut read = 0;
    for chunk in input.utf8_chunks() {
        target.push_utf8(chunk.valid().as_bytes())?;
        let invalid = chunk.invalid();
        if !invalid.is_empty() {
            crate::cp1252::transcribe_sink(invalid, target)?;
            read += invalid.len();
        }
    }
    Ok(read)
}

/// Turn a UTF-8 validation failure into this crate's located refusal.
fn fault(input: &[u8], error: std::str::Utf8Error) -> Error {
    let position = input.len().min(error.valid_up_to());
    match error.error_len() {
        Some(_) => undecodable(
            NAME,
            position,
            input.get(position).copied().unwrap_or_default(),
        ),
        None => truncated(NAME, position, "a whole UTF-8 sequence"),
    }
}

/// Encode text as UTF-8, which is the bytes it already holds.
pub(crate) const fn encode(input: &str) -> &[u8] {
    input.as_bytes()
}

/// Encode text onto the end of a byte target, which is a copy of its bytes.
pub(crate) fn encode_into(input: &str, target: &mut Vec<u8>) {
    target.extend_from_slice(input.as_bytes());
}

/// The six leaves that carry this charset, in shape order: plain, large,
/// view, large view, fixed, sized - with the placeholder `1` where a leaf
/// carries a number.
pub const LEAVES: [StringType; 6] = [
    StringType::Utf8String,
    StringType::LargeUtf8String,
    StringType::Utf8StringView,
    StringType::LargeUtf8StringView,
    StringType::FixedUtf8String(1),
    StringType::SizedUtf8String(1),
];

/// The fixed leaf of this charset, stating `width`.
pub(crate) const fn fixed_leaf(width: u32) -> StringType {
    StringType::FixedUtf8String(width)
}

/// The sized leaf of this charset, stating `max`.
pub(crate) const fn sized_leaf(max: u32) -> StringType {
    StringType::SizedUtf8String(max)
}

impl DataType {
    /// Unbounded UTF-8 with 32-bit offsets - Arrow's `Utf8`.
    #[must_use]
    pub const fn utf8() -> Self {
        Self::Utf8String
    }

    /// Unbounded UTF-8 with 64-bit offsets - Arrow's `LargeUtf8`.
    #[must_use]
    pub const fn large_utf8() -> Self {
        Self::LargeUtf8String
    }

    /// Unbounded UTF-8 in the view layout - Arrow's `Utf8View`.
    #[must_use]
    pub const fn utf8_view() -> Self {
        Self::Utf8StringView
    }

    /// Unbounded UTF-8 in the view layout over 64-bit offsets.
    #[must_use]
    pub const fn large_utf8_view() -> Self {
        Self::LargeUtf8StringView
    }

    /// UTF-8 of exactly `width` stored bytes, padded with trailing NUL.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_utf8(width: u32) -> Result<Self> {
        Self::string(StringType::FixedUtf8String(width))
    }

    /// UTF-8 of at most `max` stored bytes, over 32-bit offsets.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a maximum of zero.
    pub fn sized_utf8(max: u32) -> Result<Self> {
        Self::string(StringType::SizedUtf8String(max))
    }
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/utf8.rs` pins and a caller cannot reach.
    //!
    //! `transcribe_into` is the reading every legacy-charset door goes
    //! through, and what it counts is the whole contract; everything a caller
    //! can observe is pinned through `yggdryl::` like any other test.
    /// Read every byte `input` can give, counting the ones that were not
    /// UTF-8 and were taken as ISO 8859-1 instead.
    pub fn transcribe_into(input: &[u8], target: &mut String) -> usize {
        super::transcribe_into(input, target)
    }
}
