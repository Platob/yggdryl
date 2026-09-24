//! US-ASCII: the charset, and the six string leaves that carry it. One module
//! because a leaf is its charset's promise about the bytes, and here that
//! promise is one scan: the word-at-a-time `ascii_len` that the codec
//! decodes and encodes with is the scan that judges a value's repertoire,
//! validates a registered code's text and packs a fixed-width value into the
//! integer a `StringEnum` member reads as - and every other charset in the
//! crate starts its own decode with it, because they all agree with US-ASCII
//! below `0x80`.

use std::borrow::Cow;

use smol_str::{SmolStr, format_smolstr};

use crate::charset::sink::Utf8Sink;
use crate::charset::{REPLACEMENT, REPLACEMENT_UTF8, undecodable, unencodable};
use crate::string::trim_padding;
use crate::{DataType, Error, Result, Scalar, Str, StringType};

/// The canonical name this charset reports itself by.
pub(crate) const NAME: &str = "us-ascii";

/// The bit that leaves US-ASCII, in every byte of a machine word.
///
/// `usize::MAX / 0xFF` is one in the low bit of every byte, so multiplying it
/// by `0x80` raises the high bit of every byte whatever the word width is.
const HIGH_BITS: usize = usize::MAX / 0xFF * 0x80;

/// How many leading bytes of `input` are US-ASCII.
///
/// The scan reads a machine word at a time while a whole one remains, so plain
/// text costs one masked load per eight bytes instead of one comparison per
/// byte. Every decode in the charset layer begins here: bytes below `0x80`
/// are their own UTF-8, so an ASCII run is copied whole - or, when it is the
/// whole input, borrowed rather than transcoded at all.
pub(crate) fn ascii_len(input: &[u8]) -> usize {
    const LANE: usize = size_of::<usize>();

    let mut index = 0;
    while index + LANE <= input.len() {
        let Ok(lane) = <[u8; LANE]>::try_from(&input[index..index + LANE]) else {
            break;
        };
        if usize::from_ne_bytes(lane) & HIGH_BITS != 0 {
            break;
        }
        index += LANE;
    }
    while index < input.len() && input[index] < 0x80 {
        index += 1;
    }
    index
}

/// Borrow a run [`ascii_len`] proved to be US-ASCII as text.
///
/// Every byte below `0x80` is its own one-byte UTF-8 encoding, so this
/// conversion always succeeds. It is spelled as a checked one because the
/// crate denies `unsafe`, and over an all-ASCII run the standard library's
/// check is the same word-at-a-time scan written above.
pub(crate) fn text(run: &[u8]) -> Result<&str> {
    std::str::from_utf8(run).map_err(|error| {
        let position = error.valid_up_to();
        undecodable(
            NAME,
            position,
            run.get(position).copied().unwrap_or_default(),
        )
    })
}

/// Decode a complete buffer, borrowing it when it is already US-ASCII.
pub(crate) fn decode(input: &[u8]) -> Result<Cow<'_, str>> {
    let boundary = ascii_len(input);
    if boundary < input.len() {
        return Err(undecodable(NAME, boundary, input[boundary]));
    }
    Ok(Cow::Borrowed(text(input)?))
}

/// Decode a complete buffer, replacing every byte above `0x7F` with `U+FFFD`.
pub(crate) fn decode_lossy(input: &[u8]) -> Cow<'static, str> {
    let mut target = String::new();
    match decode_into::<true>(input, &mut target) {
        Ok(()) => Cow::Owned(target),
        Err(_) => Cow::Owned(String::from(REPLACEMENT)),
    }
}

/// Decode into a sink, replacing bytes above `0x7F` when `LOSSY`.
pub(crate) fn decode_into<const LOSSY: bool>(
    input: &[u8],
    target: &mut impl Utf8Sink,
) -> Result<()> {
    target.reserve(input.len());
    let mut index = 0;
    while index < input.len() {
        let run = ascii_len(&input[index..]);
        if run > 0 {
            target.push_utf8(&input[index..index + run])?;
            index += run;
            continue;
        }
        if !LOSSY {
            return Err(undecodable(NAME, index, input[index]));
        }
        target.push_scalar(REPLACEMENT, REPLACEMENT_UTF8);
        index += 1;
    }
    Ok(())
}

/// Encode complete text, borrowing it when every scalar is US-ASCII.
pub(crate) fn encode(input: &str) -> Result<Cow<'_, [u8]>> {
    let bytes = input.as_bytes();
    let boundary = ascii_len(bytes);
    if boundary < bytes.len() {
        let scalar = input
            .get(boundary..)
            .and_then(|rest| rest.chars().next())
            .unwrap_or(REPLACEMENT);
        return Err(unencodable(NAME, boundary, scalar));
    }
    Ok(Cow::Borrowed(bytes))
}

/// Encode text into a byte target.
pub(crate) fn encode_into(input: &str, target: &mut Vec<u8>) -> Result<()> {
    match encode(input)? {
        Cow::Borrowed(bytes) => target.extend_from_slice(bytes),
        Cow::Owned(bytes) => target.extend_from_slice(&bytes),
    }
    Ok(())
}

/// The scalar one byte decodes to: itself below `0x80`, nothing above.
pub(crate) fn scalar_of(byte: u8) -> Option<char> {
    byte.is_ascii().then(|| char::from(byte))
}

/// The byte one scalar encodes to: itself below `U+0080`, nothing above.
pub(crate) fn byte_of(scalar: char) -> Option<u8> {
    scalar.is_ascii().then_some(scalar as u8)
}

/// The six leaves that carry this charset, in shape order: plain, large,
/// view, large view, fixed, sized - with the placeholder `1` where a leaf
/// carries a number.
pub const LEAVES: [StringType; 6] = [
    StringType::AsciiString,
    StringType::LargeAsciiString,
    StringType::AsciiStringView,
    StringType::LargeAsciiStringView,
    StringType::FixedAsciiString(1),
    StringType::SizedAsciiString(1),
];

/// The fixed leaf of this charset, stating `width`.
pub(crate) const fn fixed_leaf(width: u32) -> StringType {
    StringType::FixedAsciiString(width)
}

/// The sized leaf of this charset, stating `max`.
pub(crate) const fn sized_leaf(max: u32) -> StringType {
    StringType::SizedAsciiString(max)
}

impl DataType {
    /// Unbounded US-ASCII with 32-bit offsets.
    #[must_use]
    pub const fn ascii() -> Self {
        Self::AsciiString
    }

    /// Unbounded US-ASCII with 64-bit offsets.
    #[must_use]
    pub const fn large_ascii() -> Self {
        Self::LargeAsciiString
    }

    /// Unbounded US-ASCII in the view layout.
    #[must_use]
    pub const fn ascii_view() -> Self {
        Self::AsciiStringView
    }

    /// Unbounded US-ASCII in the view layout over 64-bit offsets.
    #[must_use]
    pub const fn large_ascii_view() -> Self {
        Self::LargeAsciiStringView
    }

    /// US-ASCII of exactly `width` stored bytes, padded with trailing NUL.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_ascii(4)?.to_string(), "fixed_ascii(4)");
    /// assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    /// assert_eq!(DataType::ascii().to_string(), "ascii");
    /// assert_eq!(DataType::ascii().fixed_byte_width(), None);
    /// assert!(DataType::fixed_ascii(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_ascii(width: u32) -> Result<Self> {
        Self::string(StringType::FixedAsciiString(width))
    }

    /// US-ASCII of at most `max` stored bytes, over 32-bit offsets.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a maximum of zero.
    pub fn sized_ascii(max: u32) -> Result<Self> {
        Self::string(StringType::SizedAsciiString(max))
    }
}

impl DataType {
    /// The integer an ASCII value packs into: its bytes padded with trailing
    /// NUL to the width, big-endian.
    ///
    /// The padding is the packing's, never a column's - a code stores as the
    /// text it is - and it is what makes the integer order exactly as the
    /// text does and be the same integer in every process, which is what a
    /// stable hash and a portable enum member both need. An ASCII byte is at
    /// most `0x7F`, so the sign bit is never set and the value is never
    /// negative: four bytes fill an `i32`, eight an `i64`, and sixteen the
    /// whole `i128`.
    ///
    /// Only a fixed US-ASCII width and a registered code have one. A variable
    /// string takes a value of any length, so there is no integer its bytes
    /// always fit; another charset can set the sign bit; and a width above
    /// sixteen bytes outgrows the widest integer this crate carries. All
    /// three are refused rather than truncated.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // `USD` packs into `USD\0` under `fixed_ascii(4)`, which is that
    /// // big-endian `i32`; under `ccy` it is the three bytes alone.
    /// let ccy = DataType::fixed_ascii(4)?;
    /// assert_eq!(ccy.ascii_packed(b"USD")?, 0x5553_4400);
    /// assert_eq!(ccy.ascii_packed(b"USD\0")?, 0x5553_4400);
    /// assert_eq!(ccy.ascii_value(0x5553_4400)?, "USD");
    /// assert_eq!(DataType::Ccy.ascii_packed(b"USD")?, 0x0055_5344);
    ///
    /// // The order of the integers is the order of the text.
    /// assert!(ccy.ascii_packed(b"EUR")? < ccy.ascii_packed(b"USD")?);
    ///
    /// // Twelve bytes need 96 bits, and sixteen the whole `i128`.
    /// let twelve = DataType::fixed_ascii(12)?;
    /// let isin = twelve.ascii_packed(b"US0378331005")?;
    /// assert_eq!(twelve.ascii_value(isin)?, "US0378331005");
    /// assert!(isin > i128::from(u64::MAX));
    ///
    /// assert!(ccy.ascii_packed(b"EURO!").is_err());
    /// assert!(DataType::ascii().ascii_packed(b"USD").is_err());
    /// assert!(DataType::fixed_ascii(17)?.ascii_packed(b"USD").is_err());
    /// assert!(DataType::fixed_utf8(4)?.ascii_packed(b"USD").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted datatypes when this has no
    /// US-ASCII width or its width outgrows an `i128`, and one naming the
    /// width when `value` is not ASCII text that fits it.
    pub fn ascii_packed(&self, value: &[u8]) -> Result<i128> {
        let width = self.packed_width()?;
        let text = ascii_text(width, value)?;
        let mut slot = [0_u8; 16];
        // The text fits the width, and the width fits the slot.
        slot[..text.len()].copy_from_slice(text.as_bytes());
        Ok(i128::from_be_bytes(slot) >> (8 * (16 - width as i128)))
    }

    /// The ASCII value a packed integer carries, without its padding.
    ///
    /// The inverse of [`Self::ascii_packed`], and it refuses exactly what that
    /// refuses: an integer wider than the width, a negative one, and one whose
    /// bytes are not the padded storage of an ASCII value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when this is not one, and
    /// one naming the width when `packed` is not the storage of an ASCII value
    /// of it.
    pub fn ascii_value(&self, packed: i128) -> Result<Str> {
        let width = self.packed_width()?;
        let bytes = packed.to_be_bytes();
        let (above, stored) = bytes.split_at(bytes.len() - width);
        if above.iter().any(|byte| *byte != 0) {
            return Err(ascii_refusal(
                Some(width),
                format_smolstr!("the integer {packed}, which is wider than the width"),
            ));
        }
        let text = ascii_text(width, stored)?;
        self.string_parameters()
            // A code is US-ASCII bounded at the width its standard fixes.
            .unwrap_or(StringType::SizedAsciiString(width as u32))
            .admit(Str::new(text))
    }

    /// The width a packed integer may be built from, refusing the rest.
    ///
    /// A code packs into the width its standard fixes; a string packs into
    /// the width that makes it fixed, and only in US-ASCII, where no byte
    /// sets the sign bit.
    fn packed_width(&self) -> Result<usize> {
        let width = match self.code_width() {
            Some(width) => Some(width),
            None if matches!(
                self.string_parameters(),
                Some(StringType::FixedAsciiString(_))
            ) =>
            {
                self.fixed_byte_width()
            }
            None => None,
        };
        match width {
            Some(width) if width <= PACKED_LIMIT => Ok(width),
            _ => Err(ascii_values_refusal(self)),
        }
    }
}

/// The widest fixed ASCII storage one `i128` holds, in bytes.
const PACKED_LIMIT: usize = 16;

/// Validates bytes as an ASCII value of at most `width` bytes and trims the
/// trailing NUL padding.
///
/// The one validator every arm calls: field validation and canonicalization,
/// Arrow ingest, and casts all answer the same trimmed text or the same
/// refusal naming the width.
///
/// # Errors
///
/// Returns an error naming the width when the trimmed bytes hold a NUL, a
/// non-ASCII byte, or more than `width` bytes.
pub(crate) fn ascii_text(width: usize, bytes: &[u8]) -> Result<&str> {
    ascii_text_sized(Some(width), bytes)
}

/// [`ascii_text`] over the width the caller already holds, if there is one.
///
/// The one body every shape runs: a fixed width passes its length, and a
/// registered code passes its constant, which lets the length check fold at
/// each code's call site.
#[inline]
pub(crate) fn ascii_text_sized(width: Option<usize>, bytes: &[u8]) -> Result<&str> {
    let text = trim_padding(bytes);
    if let Some(position) = text.iter().position(|byte| *byte == 0) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a NUL byte at {position}"),
        ));
    }
    // The byte class is one fact and this module owns it: [`ascii_len`] is
    // the codec's own word-at-a-time scan rather than a second one written
    // for the value.
    let position = ascii_len(text);
    if position < text.len() {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a non-ASCII byte 0x{:02X} at {position}", text[position]),
        ));
    }
    if width.is_some_and(|width| text.len() > width) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("{} bytes", text.len()),
        ));
    }
    // Every byte is ASCII, so the slice is UTF-8 by construction.
    std::str::from_utf8(text).map_err(|error| ascii_refusal(width, format_smolstr!("{error}")))
}

/// The bytes a code value carries, in any accepted spelling.
pub(crate) fn ascii_bytes(value: &Scalar) -> Option<&[u8]> {
    match value {
        crate::string_scalars!(text) => Some(text.as_str().as_bytes()),
        code if code.is_code() => code.as_str().map(str::as_bytes),
        crate::bytes_scalars!(bytes) => Some(bytes.as_bytes()),
        _ => None,
    }
}

/// Refuse what US-ASCII text never holds: a NUL, or a byte above `0x7F`.
///
/// This is the value door's judgment of the repertoire, which
/// [`StringType::scalar`] passes every value restated under a US-ASCII leaf
/// through: the same scan the codec decodes with, so a value and a buffer
/// cannot disagree about which bytes are US-ASCII.
pub(crate) fn ascii_repertoire(bytes: &[u8]) -> Result<()> {
    let refusal = |actual: SmolStr| Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got(format_args!("US-ASCII text"), actual),
    };
    if let Some(position) = bytes.iter().position(|byte| *byte == 0) {
        return Err(refusal(format_smolstr!("a NUL byte at {position}")));
    }
    let position = ascii_len(bytes);
    if position < bytes.len() {
        return Err(refusal(format_smolstr!(
            "a non-ASCII byte 0x{:02X} at {position}",
            bytes[position]
        )));
    }
    Ok(())
}

fn ascii_refusal(width: Option<usize>, actual: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: match width {
            Some(width) => crate::text::expected_got(
                format_args!("ASCII text of at most {width} bytes"),
                actual,
            ),
            None => crate::text::expected_got(format_args!("ASCII text"), actual),
        },
    }
}

/// Refuses a datatype that has no packed integer.
fn ascii_values_refusal(values: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "ascii",
        reason: crate::text::expected_got(
            format_args!(
                "a fixed US-ASCII string of at most {PACKED_LIMIT} bytes, or a registered code"
            ),
            format_args!("{values}"),
        ),
    }
}
