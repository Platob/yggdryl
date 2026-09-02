//! The ASCII widths: text padded with trailing NUL to a fixed byte width.
//!
//! The value contract, stated once: a value is ASCII text - every byte at
//! most `0x7F` - of at most the width in bytes, with no NUL byte. Storage
//! pads with trailing `\0` to exactly the width (Arrow
//! `FixedSizeBinary(width)`), and every string rendering trims the padding,
//! so storage reads back as the text that went in. The canonical value
//! spelling is `Scalar::String(trimmed)`; `Scalar::Bytes` and a string
//! carrying trailing NULs are accepted on the way in under the same rule and
//! canonicalize to the trimmed string.

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, Scalar};

/// The Arrow extension name of the three ASCII widths.
///
/// The storage is `FixedSizeBinary(4 | 8 | 16)` and the extension metadata
/// is the empty string: the storage width says the width.
pub(crate) const ASCII_EXTENSION_NAME: &str = "yggdryl.ascii";

impl DataType {
    /// The logical names registered over an ASCII width, in registration order.
    ///
    /// A registration names a vocabulary that fits one width; it parses to that
    /// width and is otherwise not a type of its own: the ASCII widths are the
    /// whole type system, and a registration adds one spelling to the grammar.
    /// `currency` is ISO 4217's three-letter code, stored `USD\0`.
    pub const LOGICAL_NAMES: &'static [(&'static str, DataType)] =
        &[("currency", DataType::Ascii32)];

    /// Creates the ASCII width that holds `width` bytes.
    ///
    /// The family constructor selects the physical width once: 1 through 4
    /// bytes is [`Self::Ascii32`], 5 through 8 [`Self::Ascii64`], and 9
    /// through 16 [`Self::Ascii128`].
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::ascii(3)?, DataType::Ascii32);
    /// assert_eq!(DataType::ascii(12)?, DataType::Ascii128);
    /// assert!(DataType::ascii(17).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when `width` is outside 1 through 16.
    pub fn ascii(width: i32) -> Result<Self> {
        match width {
            1..=4 => Ok(Self::Ascii32),
            5..=8 => Ok(Self::Ascii64),
            9..=16 => Ok(Self::Ascii128),
            _ => Err(Error::InvalidDataType {
                kind: "ascii",
                reason: format_smolstr!("expected an ASCII width from 1 to 16 bytes, got {width}"),
            }),
        }
    }

    /// The storage width of an ASCII datatype in bytes, `None` for every other.
    pub const fn ascii_width(&self) -> Option<i32> {
        match self {
            Self::Ascii32 => Some(4),
            Self::Ascii64 => Some(8),
            Self::Ascii128 => Some(16),
            _ => None,
        }
    }

    /// Resolves a registered logical name, ASCII case-insensitively and trimmed.
    ///
    /// ```
    /// use arrow_array::{Array, FixedSizeBinaryArray};
    /// use yggdryl::arrow::{scalar_array, scalar_value};
    /// use yggdryl::{DataType, Scalar};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let currency = DataType::from_logical_name("Currency")?;
    /// assert_eq!(currency, DataType::Ascii32);
    /// assert_eq!(DataType::from_str("currency")?, currency);
    ///
    /// // Storage pads to the width; the scalar reads back trimmed.
    /// let field = currency.required_field("ccy");
    /// let array = scalar_array(&field, &Scalar::from("USD"))?;
    /// let stored = array.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    /// assert_eq!(stored.value(0), b"USD\0");
    /// assert_eq!(scalar_value(&field, array.as_ref())?, Scalar::from("USD"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the registered vocabulary when `name` is not
    /// in it.
    pub fn from_logical_name(name: &str) -> Result<Self> {
        let name = name.trim();
        Self::LOGICAL_NAMES
            .iter()
            .find(|(registered, _)| registered.eq_ignore_ascii_case(name))
            .map(|(_, dtype)| dtype.clone())
            .ok_or_else(|| Error::InvalidDataType {
                kind: "ascii",
                reason: crate::text::expected_got(
                    format_args!("a registered logical name ({})", logical_vocabulary()),
                    format_args!("{name:?}"),
                ),
            })
    }
}

fn logical_vocabulary() -> String {
    DataType::LOGICAL_NAMES
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

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
pub(crate) fn ascii_text(width: i32, bytes: &[u8]) -> Result<&str> {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    let text = &bytes[..end];
    if let Some(position) = text.iter().position(|byte| *byte == 0) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a NUL byte at {position}"),
        ));
    }
    if let Some(position) = text.iter().position(|byte| !byte.is_ascii()) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("a non-ASCII byte 0x{:02X} at {position}", text[position]),
        ));
    }
    if text.len() > usize::try_from(width).unwrap_or(0) {
        return Err(ascii_refusal(
            width,
            format_smolstr!("{} bytes", text.len()),
        ));
    }
    // Every byte is ASCII, so the slice is UTF-8 by construction.
    std::str::from_utf8(text).map_err(|error| ascii_refusal(width, format_smolstr!("{error}")))
}

/// Pads `text` with trailing NUL into one storage slot.
///
/// The slot is one value of the fixed-width storage; `text` has already
/// passed [`ascii_text`] for that width, so it fits.
#[cfg(feature = "arrow")]
pub(crate) fn ascii_padded(slot: &mut [u8], text: &str) {
    let length = text.len().min(slot.len());
    slot[..length].copy_from_slice(&text.as_bytes()[..length]);
    slot[length..].fill(0);
}

/// The bytes an ASCII value carries, in either accepted spelling.
pub(crate) fn ascii_bytes(value: &Scalar) -> Option<&[u8]> {
    match value {
        Scalar::String(text) => Some(text.as_bytes()),
        Scalar::Bytes(bytes) => Some(bytes),
        _ => None,
    }
}

fn ascii_refusal(width: i32, actual: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got(
            format_args!("ASCII text of at most {width} bytes"),
            actual,
        ),
    }
}
