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
//!
//! The widths are 2, 3, 4, 8, 12 and 16 bytes - `ascii16`, `ascii24`,
//! `ascii32`, `ascii64`, `ascii96` and `ascii128` - because those are the
//! shapes the codes this type exists for actually take: a twelve-character
//! ISIN, a ticker, a venue-local symbol. A width is the general form: it says
//! how many bytes a value may take and nothing about what the value means.
//!
//! Four identifiers are specific enough to be types of their own - `country`,
//! `currency`, `mic` and `cfi` - and live in [`super::coded`]. They share
//! every rule stated here, so [`DataType::ascii_width`], [`ascii_text`],
//! [`DataType::ascii_packed`], [`AsciiEnum`] and [`AsciiDictionary`] all
//! answer for a code exactly as they do for a width. What they add is
//! identity: a currency column reads back a currency, never three anonymous
//! bytes.
//!
//! [`AsciiDictionary`] is the per-column vocabulary over one of those widths:
//! an ordered set of values whose position is the code a `dictionary(key,
//! ascii-N)` column stores.

use std::collections::{BTreeMap, HashMap};
#[cfg(feature = "arrow")]
use std::sync::Arc;

#[cfg(feature = "arrow")]
use arrow_array::types::{Int32Type, Int64Type};
#[cfg(feature = "arrow")]
use arrow_array::{Array, ArrayRef, DictionaryArray, FixedSizeBinaryArray, Int32Array, Int64Array};
#[cfg(feature = "arrow")]
use arrow_schema::DataType as ArrowDataType;
use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, Scalar};

use super::coded::{CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, MIC_WIDTH};

/// The Arrow extension name of the three ASCII widths.
///
/// The storage is `FixedSizeBinary(4 | 8 | 16)` and the extension metadata
/// is the empty string: the storage width says the width.
pub(crate) const ASCII_EXTENSION_NAME: &str = "yggdryl.ascii";

impl DataType {
    /// Creates the ASCII width that holds `width` bytes.
    ///
    /// The family constructor selects the physical width once: 1 or 2 bytes
    /// is [`Self::Ascii16`], 3 [`Self::Ascii24`], 4 [`Self::Ascii32`], 5
    /// through 8 [`Self::Ascii64`], 9 through 12 [`Self::Ascii96`], and 13
    /// through 16 [`Self::Ascii128`].
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::ascii(2)?, DataType::Ascii16);
    /// assert_eq!(DataType::ascii(3)?, DataType::Ascii24);
    /// assert_eq!(DataType::ascii(6)?, DataType::Ascii64);
    /// assert_eq!(DataType::ascii(12)?, DataType::Ascii96);
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
            1..=2 => Ok(Self::Ascii16),
            3 => Ok(Self::Ascii24),
            4 => Ok(Self::Ascii32),
            5..=8 => Ok(Self::Ascii64),
            9..=12 => Ok(Self::Ascii96),
            13..=16 => Ok(Self::Ascii128),
            _ => Err(Error::InvalidDataType {
                kind: "ascii",
                reason: format_smolstr!("expected an ASCII width from 1 to 16 bytes, got {width}"),
            }),
        }
    }

    /// The storage width of an ASCII datatype in bytes, `None` for every other.
    pub const fn ascii_width(&self) -> Option<i32> {
        match self {
            Self::Ascii16 => Some(2),
            Self::Ascii24 => Some(3),
            Self::Ascii32 => Some(4),
            Self::Ascii64 => Some(8),
            Self::Ascii96 => Some(12),
            Self::Ascii128 => Some(16),
            // A registered code is the same fixed ASCII storage at the width
            // its standard fixes, so every rule stated for a width answers
            // for it: this is the accessor that says so.
            Self::Country => Some(COUNTRY_WIDTH as i32),
            Self::Currency => Some(CURRENCY_WIDTH as i32),
            Self::Mic => Some(MIC_WIDTH as i32),
            Self::Cfi => Some(CFI_WIDTH as i32),
            _ => None,
        }
    }

    /// The integer an ASCII value packs into: its storage bytes, big-endian.
    ///
    /// Storage pads with trailing NUL to the width, so the packed integer
    /// orders exactly as the text does and is the same integer in every
    /// process - what a stable hash and a portable enum member both need, and
    /// what a dictionary code, local to the one column that built it, can
    /// never be. An ASCII byte is at most `0x7F`, so the sign bit is never
    /// set and the value is never negative: `ascii32` fills an `i32`,
    /// `ascii64` an `i64`, and `ascii128` an `i128`.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // `USD` stores as `USD\0` under `ascii32`, which is that big-endian
    /// // `i32`; under `ascii24` it is the three bytes and nothing else.
    /// assert_eq!(DataType::Ascii32.ascii_packed(b"USD")?, 0x5553_4400);
    /// assert_eq!(DataType::Ascii32.ascii_packed(b"USD\0")?, 0x5553_4400);
    /// assert_eq!(DataType::Ascii32.ascii_value(0x5553_4400)?, "USD");
    /// assert_eq!(DataType::Ascii24.ascii_packed(b"USD")?, 0x0055_5344);
    ///
    /// // The order of the integers is the order of the text.
    /// assert!(DataType::Ascii32.ascii_packed(b"EUR")? < DataType::Ascii32.ascii_packed(b"USD")?);
    ///
    /// // Twelve bytes need 96 bits, and sixteen the whole `i128`.
    /// let isin = DataType::Ascii96.ascii_packed(b"US0378331005")?;
    /// assert_eq!(DataType::Ascii96.ascii_value(isin)?, "US0378331005");
    /// assert!(isin > i128::from(u64::MAX));
    ///
    /// assert!(DataType::Ascii32.ascii_packed(b"EURO!").is_err());
    /// assert!(DataType::Utf8.ascii_packed(b"USD").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when this is not one, and
    /// one naming the width when `value` is not ASCII text that fits it.
    pub fn ascii_packed(&self, value: &[u8]) -> Result<i128> {
        let width = self
            .ascii_width()
            .ok_or_else(|| ascii_values_refusal(self))?;
        let text = ascii_text(width, value)?;
        let mut slot = [0_u8; 16];
        // The text fits the width, and the width fits the slot.
        slot[..text.len()].copy_from_slice(text.as_bytes());
        Ok(i128::from_be_bytes(slot) >> (8 * (16 - i128::from(width))))
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
    pub fn ascii_value(&self, packed: i128) -> Result<SmolStr> {
        let width = self
            .ascii_width()
            .ok_or_else(|| ascii_values_refusal(self))?;
        let slot = usize::try_from(width).unwrap_or(0);
        let bytes = packed.to_be_bytes();
        let (above, stored) = bytes.split_at(bytes.len() - slot);
        if above.iter().any(|byte| *byte != 0) {
            return Err(ascii_refusal(
                slot,
                format_smolstr!("the integer {packed}, which is wider than the width"),
            ));
        }
        ascii_text(width, stored).map(SmolStr::new)
    }
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
    ascii_text_sized(usize::try_from(width).unwrap_or(0), bytes)
}

/// [`ascii_text`] over a width the caller already holds as a length.
///
/// The one body both families run: a width passes its runtime width and a
/// [`super::coded`] code passes its constant, which lets the length checks
/// and the padding arithmetic fold at each code's call site.
#[inline]
pub(crate) fn ascii_text_sized(width: usize, bytes: &[u8]) -> Result<&str> {
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
    if text.len() > width {
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

fn ascii_refusal(width: usize, actual: SmolStr) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got(
            format_args!("ASCII text of at most {width} bytes"),
            actual,
        ),
    }
}

/// A per-column ASCII vocabulary and the codes that name its values.
///
/// The vocabulary is a value, never a process-global registry: a caller holds
/// one per column, and a code is stable exactly as long as that value is
/// carried. Two independent encodes build two vocabularies, and their codes
/// agree only when the same `AsciiDictionary` crossed both. Nothing in the
/// write path registers on its own - a caller that wants the encoding
/// declares it and registers as it encodes.
///
/// A dictionary code is a position, which is why it is local to one column.
/// The other integer an ASCII value has is [`DataType::ascii_packed`], its own
/// storage bytes, which is the same everywhere and is what an enum member and
/// [`Self::into_members`] are.
///
/// The values are one ASCII width ([`DataType::ascii_width`]) and the keys
/// are `Int32` or `Int64`; [`Self::dtype`] is the `dictionary(key, ascii-N)`
/// those two describe.
///
/// ```
/// use yggdryl::{AsciiDictionary, DataType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut currencies = AsciiDictionary::from_values(DataType::Ascii32, ["USD", "EUR"])?;
/// assert_eq!(currencies.push("JPY")?, 2);
/// assert_eq!(currencies.get_code("USD"), Some(0));
/// assert_eq!(currencies.dtype()?.to_string(), "dictionary(int32,ascii32)");
/// # Ok(())
/// # }
/// ```
///
/// The lookup index is a function of the values, so equality is exactly the
/// width, the key type, and the values in first-appearance order. There is no
/// `Hash`: this is a growing builder rather than a map key, and hashing it
/// would cost the whole vocabulary on every use.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsciiDictionary {
    /// The ASCII width the values are stored as.
    width: DataType,
    /// The integer type the codes are read as: `Int32` or `Int64`.
    key: DataType,
    /// The vocabulary in first-appearance order; a position is its code.
    values: Vec<SmolStr>,
    /// Value to code. `SmolStr` holds every ASCII width inline, so the second
    /// copy of a value costs no allocation.
    index: HashMap<SmolStr, i64>,
}

impl AsciiDictionary {
    /// Creates an empty vocabulary over an ASCII width, with `Int32` keys.
    ///
    /// # Errors
    ///
    /// Returns an error naming what it was given when `values` is not one of
    /// the ASCII widths.
    pub fn new(values: DataType) -> Result<Self> {
        if values.ascii_width().is_none() {
            return Err(ascii_values_refusal(&values));
        }
        Ok(Self {
            width: values,
            key: DataType::Int32,
            values: Vec::new(),
            index: HashMap::new(),
        })
    }

    /// Returns this vocabulary read under another key type.
    ///
    /// # Errors
    ///
    /// Returns an error naming what it was given when `key` is neither
    /// `Int32` nor `Int64`.
    pub fn with_key(mut self, key: DataType) -> Result<Self> {
        if !matches!(key, DataType::Int32 | DataType::Int64) {
            return Err(Error::InvalidDataType {
                kind: "ascii-dictionary",
                reason: crate::text::expected_got(
                    format_args!("an int32 or int64 key datatype"),
                    format_args!("{key}"),
                ),
            });
        }
        self.key = key;
        Ok(self)
    }

    /// Creates a vocabulary pre-seeded in first-appearance order.
    ///
    /// A repeat keeps the code of its first appearance, and every value goes
    /// through the same width rule [`Self::push`] applies.
    ///
    /// # Errors
    ///
    /// Returns an error when `values` is not an ASCII width or a seed value
    /// does not fit it.
    pub fn from_values<I, V>(values: DataType, seen: I) -> Result<Self>
    where
        I: IntoIterator<Item = V>,
        V: AsRef<str>,
    {
        let mut dictionary = Self::new(values)?;
        for value in seen {
            dictionary.push(value.as_ref())?;
        }
        Ok(dictionary)
    }

    /// Registers `value` and returns its code, existing or newly appended.
    ///
    /// This is the whole auto-registration: an unseen value takes the next
    /// code, and a seen one answers the code it already has. The value passes
    /// the ASCII rule of the width first, so nothing that would not store is
    /// silently registered.
    ///
    /// ```
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut currencies = AsciiDictionary::new(DataType::Ascii32)?;
    /// assert_eq!(currencies.push("USD")?, 0);
    /// assert_eq!(currencies.push("EUR")?, 1);
    /// assert_eq!(currencies.push("USD")?, 0);
    /// assert_eq!(currencies.as_values(), ["USD", "EUR"]);
    ///
    /// // Storage pads with trailing NUL, so the padded spelling is the same
    /// // value; what the width refuses is refused here.
    /// assert_eq!(currencies.push("USD\0")?, 0);
    /// let refused = currencies.push("EURO!").unwrap_err().to_string();
    /// assert!(refused.contains("at most 4 bytes"), "{refused}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the width when `value` is not ASCII text that
    /// fits it, and one naming the key type when the vocabulary already fills
    /// the key space.
    pub fn push(&mut self, value: &str) -> Result<i64> {
        self.push_bytes(value.as_bytes())
    }

    /// Registers the bytes storage holds and returns their code.
    ///
    /// [`Self::push`] over the other accepted spelling: the bytes go through
    /// the same width rule, so the padded storage slot and the trimmed text
    /// register as one value.
    ///
    /// ```
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut currencies = AsciiDictionary::new(DataType::Ascii32)?;
    /// assert_eq!(currencies.push_bytes(b"USD\0")?, 0);
    /// assert_eq!(currencies.push("USD")?, 0);
    /// let refused = currencies.push_bytes(b"\xff").unwrap_err().to_string();
    /// assert!(refused.contains("at most 4 bytes"), "{refused}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the width when `value` is not ASCII text that
    /// fits it, and one naming the key type when the vocabulary already fills
    /// the key space.
    pub fn push_bytes(&mut self, value: &[u8]) -> Result<i64> {
        let text = ascii_text(self.byte_width()?, value)?;
        if let Some(code) = self.index.get(text) {
            return Ok(*code);
        }
        let code = i64::try_from(self.values.len())
            .ok()
            .filter(|length| *length < self.key_limit())
            .ok_or_else(|| self.key_capacity_refusal())?;
        let text = SmolStr::new(text);
        self.values.push(text.clone());
        self.index.insert(text, code);
        Ok(code)
    }

    /// The value a code names, or `None` when the vocabulary has no such code.
    pub fn get(&self, code: i64) -> Option<&str> {
        usize::try_from(code)
            .ok()
            .and_then(|index| self.values.get(index))
            .map(SmolStr::as_str)
    }

    /// The code a value has, or `None` when it was never registered.
    ///
    /// A value carrying the storage's trailing NUL padding resolves the same
    /// as its trimmed form.
    pub fn get_code(&self, value: &str) -> Option<i64> {
        self.index.get(value.trim_end_matches('\0')).copied()
    }

    /// The vocabulary in code order.
    pub fn as_values(&self) -> &[SmolStr] {
        &self.values
    }

    /// The number of registered values.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns whether nothing is registered yet.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// The integer type the codes are read as.
    pub const fn key(&self) -> &DataType {
        &self.key
    }

    /// The ASCII width the values are stored as.
    pub const fn values_dtype(&self) -> &DataType {
        &self.width
    }

    /// The datatype an encoded column carries: `dictionary(key, ascii-N)`.
    ///
    /// ```
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let currencies = AsciiDictionary::new(DataType::Ascii32)?;
    /// let dtype = currencies.dtype()?;
    /// assert_eq!(dtype.to_string(), "dictionary(int32,ascii32)");
    /// assert_eq!(DataType::from_str("dictionary(int32,ascii32)")?, dtype);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the key type is not a valid dictionary key.
    pub fn dtype(&self) -> Result<DataType> {
        DataType::dictionary(self.key.clone(), self.width.clone())
    }

    /// The generated-enum members: one name per value, paired with its packed
    /// code.
    ///
    /// The name rule, stated once here and used by every binding: an ASCII
    /// letter is kept uppercased, a digit is kept, every other byte becomes
    /// `_`, a name starting with a digit is prefixed with `_`, a name that
    /// both opens and closes with `_` drops its trailing underscores - the
    /// shape Python reserves for `_sunder_` and `__dunder__` names - and the
    /// empty value becomes `_`. Two values that would name one member are an
    /// error naming both, never a silent rename.
    ///
    /// The code is [`DataType::ascii_packed`], never the value's position, so
    /// two vocabularies that hold one value name it with one integer and the
    /// member survives a process, a file, and a hash.
    ///
    /// ```
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let currencies = AsciiDictionary::from_values(DataType::Ascii32, ["USD", "n/a", "-a-"])?;
    /// assert_eq!(
    ///     currencies.into_members()?,
    ///     [
    ///         ("USD".into(), 0x5553_4400),
    ///         ("N_A".into(), 0x6E2F_6100),
    ///         ("_A".into(), 0x2D61_2D00),
    ///     ]
    /// );
    ///
    /// // Sixteen bytes name members too: the code is the whole `i128`.
    /// let isins = AsciiDictionary::from_values(DataType::Ascii128, ["US0378331005"])?;
    /// assert_eq!(
    ///     isins.into_members()?,
    ///     [("US0378331005".into(), 0x5553_3033_3738_3333_3130_3035_0000_0000)]
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming both values when two of them collide.
    pub fn into_members(&self) -> Result<Vec<(SmolStr, i128)>> {
        let mut members = Vec::with_capacity(self.values.len());
        let mut taken: HashMap<SmolStr, &str> = HashMap::with_capacity(self.values.len());
        for value in &self.values {
            let name = Self::member_name(value);
            if let Some(first) = taken.insert(name.clone(), value.as_str()) {
                return Err(Error::InvalidDataType {
                    kind: "ascii-dictionary",
                    reason: format_smolstr!(
                        "the values {first:?} and {value:?} both name the member {name}"
                    ),
                });
            }
            members.push((name, self.width.ascii_packed(value.as_bytes())?));
        }
        Ok(members)
    }

    /// The enum member name of one value, under the rule stated on
    /// [`Self::into_members`].
    ///
    /// The rule belongs to the vocabulary rather than to one width, so an enum
    /// that registers one value at a time names it exactly as generating the
    /// whole vocabulary at once would.
    ///
    /// ```
    /// use yggdryl::AsciiDictionary;
    ///
    /// assert_eq!(AsciiDictionary::member_name("USD").as_str(), "USD");
    /// assert_eq!(AsciiDictionary::member_name("n/a").as_str(), "N_A");
    /// assert_eq!(AsciiDictionary::member_name("-a-").as_str(), "_A");
    /// assert_eq!(AsciiDictionary::member_name("").as_str(), "_");
    /// ```
    pub fn member_name(value: &str) -> SmolStr {
        let mut name = String::with_capacity(value.len() + 1);
        // A registered value passed `ascii_text`, so one byte is one character.
        // Any other byte is not ASCII alphanumeric, so it becomes `_` like the
        // rest of what the rule replaces.
        for byte in value.bytes() {
            name.push(if byte.is_ascii_alphanumeric() {
                char::from(byte.to_ascii_uppercase())
            } else {
                '_'
            });
        }
        if name.starts_with(|first: char| first.is_ascii_digit()) {
            name.insert(0, '_');
        }
        // A name that both opens and closes with `_` carries the shape Python
        // reserves for `_sunder_` and `__dunder__`, where a member is refused or
        // silently dropped; a name of nothing but `_` has no other spelling.
        let named = name.trim_end_matches('_').len();
        if name.starts_with('_') && named > 0 {
            name.truncate(named);
        }
        if name.is_empty() {
            return SmolStr::new_static("_");
        }
        SmolStr::new(name)
    }

    /// The storage width in bytes, which [`Self::new`] validated.
    fn byte_width(&self) -> Result<i32> {
        self.width
            .ascii_width()
            .ok_or_else(|| ascii_values_refusal(&self.width))
    }

    /// The number of values the key type can address.
    fn key_limit(&self) -> i64 {
        match self.key {
            DataType::Int64 => i64::MAX,
            _ => i64::from(i32::MAX),
        }
    }

    fn key_capacity_refusal(&self) -> Error {
        Error::InvalidDataType {
            kind: "ascii-dictionary",
            reason: format_smolstr!(
                "a vocabulary of {} values fills the {} key space",
                self.values.len(),
                self.key
            ),
        }
    }
}

fn ascii_values_refusal(values: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "ascii-dictionary",
        reason: crate::text::expected_got(
            format_args!(
                "an ASCII width (ascii16, ascii24, ascii32, ascii64, ascii96, ascii128) \
                 or a registered code (country, currency, mic, cfi)"
            ),
            format_args!("{values}"),
        ),
    }
}

/// The enum an ASCII field's values name: one value per member name.
///
/// [`AsciiDictionary`] is a vocabulary and derives its member names from its
/// values; this is the vocabulary a declaration named itself, and it is what a
/// [`crate::Field`] stores under `field:enum` so the enum crosses Arrow, a
/// file, and another runtime intact.
///
/// The width lives in the field's datatype and is never copied here: a
/// member's code is [`DataType::ascii_packed`] of its value under that width,
/// so every reader of one enum answers the same integers. Members are held by
/// name, which is what makes the rendered document deterministic - the order a
/// declaration happened to use is not part of a member's identity once the
/// code is the value's own bytes.
///
/// ```
/// use yggdryl::{AsciiEnum, DataType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")])?;
/// assert_eq!(side.get("BUY"), Some("B"));
/// assert_eq!(side.get_member("S"), Some("SELL"));
/// assert_eq!(
///     side.into_members(&DataType::Ascii32)?,
///     [("BUY".into(), 0x4200_0000), ("SELL".into(), 0x5300_0000)]
/// );
/// assert_eq!(
///     side.into_json(),
///     r#"{"members":{"BUY":"B","SELL":"S"},"name":"Side"}"#
/// );
/// assert_eq!(AsciiEnum::from_json(&side.into_json())?, side);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AsciiEnum {
    /// The enum's own name, which is not the field's name.
    name: SmolStr,
    /// Member name to ASCII value, ordered by name so the document is one text.
    members: BTreeMap<SmolStr, SmolStr>,
}

impl AsciiEnum {
    /// Creates an enum of no members under one name.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty or holds a control character.
    pub fn new(name: impl Into<SmolStr>) -> Result<Self> {
        let name = name.into();
        validate_enum_text("enum name", &name)?;
        Ok(Self {
            name,
            members: BTreeMap::new(),
        })
    }

    /// Creates an enum from its members, one ASCII value per member name.
    ///
    /// A repeated member name keeps the last value, exactly as [`Self::insert`]
    /// would; two members may share a value, because two spellings of one code
    /// is what an alias is.
    ///
    /// # Errors
    ///
    /// Returns an error when the enum name or a member name is empty or holds
    /// a control character.
    pub fn from_members<I, N, V>(name: impl Into<SmolStr>, members: I) -> Result<Self>
    where
        I: IntoIterator<Item = (N, V)>,
        N: Into<SmolStr>,
        V: Into<SmolStr>,
    {
        let mut enumeration = Self::new(name)?;
        for (member, value) in members {
            enumeration.insert(member, value)?;
        }
        Ok(enumeration)
    }

    /// Parses the `field:enum` document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a JSON object of a string
    /// `"name"` and an object `"members"` of strings, and one naming the part
    /// when a name is empty or holds a control character.
    pub fn from_json(document: &str) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_str(document.trim()).map_err(|error| {
            enum_document_refusal(format_smolstr!(
                "expected an enum JSON document, got unparsable JSON: {error}"
            ))
        })?;
        let Some(object) = value.as_object() else {
            return Err(enum_document_refusal(format_smolstr!(
                "expected an enum JSON object, got {}",
                crate::text::elide_display(&value)
            )));
        };
        let Some(serde_json::Value::String(name)) = object.get("name") else {
            return Err(enum_document_refusal(SmolStr::new_static(
                "expected a JSON string \"name\"",
            )));
        };
        let empty = serde_json::Map::new();
        let members = match object.get("members") {
            None | Some(serde_json::Value::Null) => &empty,
            Some(serde_json::Value::Object(members)) => members,
            Some(other) => {
                return Err(enum_document_refusal(format_smolstr!(
                    "expected a JSON object \"members\", got {}",
                    crate::text::elide_display(other)
                )));
            }
        }
        .iter()
        .map(|(member, value)| match value {
            serde_json::Value::String(value) => Ok((member.as_str(), value.as_str())),
            other => Err(enum_document_refusal(format_smolstr!(
                "expected a JSON string for the member {member:?}, got {}",
                crate::text::elide_display(other)
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
        Self::from_members(name.as_str(), members)
    }

    /// Renders the `field:enum` document: every name in order, so one enum
    /// is one text however it was built.
    pub fn into_json(&self) -> String {
        let members = self
            .members
            .iter()
            .map(|(member, value)| {
                (
                    member.as_str().to_owned(),
                    serde_json::Value::String(value.as_str().to_owned()),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        serde_json::Value::Object(serde_json::Map::from_iter([
            (
                "name".to_owned(),
                serde_json::Value::String(self.name.as_str().to_owned()),
            ),
            ("members".to_owned(), serde_json::Value::Object(members)),
        ]))
        .to_string()
    }

    /// The enum's own name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The ASCII value one member names, or `None` for a member it has not.
    pub fn get(&self, member: &str) -> Option<&str> {
        self.members.get(member).map(SmolStr::as_str)
    }

    /// The first member naming one ASCII value, or `None` when none does.
    ///
    /// Two members may share a value; the first by name answers, so an alias
    /// never changes which member a stored value reads back as.
    pub fn get_member(&self, value: &str) -> Option<&str> {
        self.members
            .iter()
            .find(|(_, held)| held.as_str() == value)
            .map(|(member, _)| member.as_str())
    }

    /// Names one ASCII value and returns the value the member had.
    ///
    /// # Errors
    ///
    /// Returns an error when `member` is empty or holds a control character.
    pub fn insert(
        &mut self,
        member: impl Into<SmolStr>,
        value: impl Into<SmolStr>,
    ) -> Result<Option<SmolStr>> {
        let member = member.into();
        validate_enum_text("member name", &member)?;
        Ok(self.members.insert(member, value.into()))
    }

    /// Removes one member and returns the ASCII value it named.
    pub fn remove(&mut self, member: &str) -> Option<SmolStr> {
        self.members.remove(member)
    }

    /// The number of members.
    pub fn len(&self) -> usize {
        self.members.len()
    }

    /// Returns whether this enum names nothing.
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The members by name, each with the ASCII value it names.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.members
            .iter()
            .map(|(member, value)| (member.as_str(), value.as_str()))
    }

    /// The members paired with their packed codes under one ASCII width.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when `width` is not one,
    /// and one naming the width when a value does not fit it.
    pub fn into_members(&self, width: &DataType) -> Result<Vec<(SmolStr, i128)>> {
        self.members
            .iter()
            .map(|(member, value)| Ok((member.clone(), width.ascii_packed(value.as_bytes())?)))
            .collect()
    }

    /// The vocabulary this enum names, as a dictionary over one width.
    ///
    /// The values are in member-name order, which is the order this enum holds
    /// them in; a dictionary code is that position and remains local to the
    /// column it encodes, exactly as it is for any other vocabulary.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted widths when `width` is not one,
    /// and one naming the width when a value does not fit it.
    pub fn into_dictionary(&self, width: DataType) -> Result<AsciiDictionary> {
        AsciiDictionary::from_values(width, self.members.values())
    }
}

fn enum_document_refusal(reason: SmolStr) -> Error {
    Error::InvalidDataType {
        kind: "ascii-enum",
        reason,
    }
}

/// Refuses the two spellings a stored document could not carry back.
fn validate_enum_text(part: &'static str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(enum_document_refusal(format_smolstr!(
            "expected a non-empty {part}"
        )));
    }
    if let Some(position) = value.chars().position(char::is_control) {
        return Err(enum_document_refusal(format_smolstr!(
            "expected a {part} with no control character, got one at {position}"
        )));
    }
    Ok(())
}

#[cfg(feature = "arrow")]
impl AsciiDictionary {
    /// Encodes a column, registering unseen values in first-appearance order.
    ///
    /// The answer is a `DictionaryArray` whose keys are the codes and whose
    /// values are this vocabulary in the width's padded `FixedSizeBinary`
    /// storage; a `None` item is a null key. The vocabulary grows to the union
    /// of everything encoded so far, so two calls on one dictionary answer two
    /// arrays whose codes agree.
    ///
    /// ```
    /// use arrow_array::types::Int32Type;
    /// use arrow_array::{Array, DictionaryArray, FixedSizeBinaryArray};
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut currencies = AsciiDictionary::new(DataType::Ascii32)?;
    /// let array = currencies.into_arrow_array([Some("USD"), None, Some("EUR"), Some("USD")])?;
    /// let encoded = array
    ///     .as_any()
    ///     .downcast_ref::<DictionaryArray<Int32Type>>()
    ///     .unwrap();
    /// assert_eq!(
    ///     encoded.keys().iter().collect::<Vec<_>>(),
    ///     [Some(0), None, Some(1), Some(0)]
    /// );
    /// let stored = encoded
    ///     .values()
    ///     .as_any()
    ///     .downcast_ref::<FixedSizeBinaryArray>()
    ///     .unwrap();
    /// assert_eq!(stored.value(0), b"USD\0");
    ///
    /// // A second column continues the same codes.
    /// let array = currencies.into_arrow_array([Some("JPY"), Some("EUR")])?;
    /// let encoded = array
    ///     .as_any()
    ///     .downcast_ref::<DictionaryArray<Int32Type>>()
    ///     .unwrap();
    /// assert_eq!(encoded.keys().values(), &[2, 1]);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A refused value leaves the vocabulary exactly as it was: a column that
    /// does not encode registers nothing.
    ///
    /// # Errors
    ///
    /// Returns an error naming the width when a value does not fit it, and one
    /// naming the key type when the vocabulary fills the key space.
    pub fn into_arrow_array<I, V>(&mut self, values: I) -> Result<ArrayRef>
    where
        I: IntoIterator<Item = Option<V>>,
        V: AsRef<[u8]>,
    {
        let registered = self.values.len();
        let encoded = self.encode_arrow_array(values);
        if encoded.is_err() {
            // The mutation fails atomically: drop what this column registered.
            for value in self.values.drain(registered..) {
                self.index.remove(&value);
            }
        }
        encoded
    }

    /// Encodes a column, registering as it goes; [`Self::into_arrow_array`]
    /// owns the rollback that makes a refusal register nothing.
    fn encode_arrow_array<I, V>(&mut self, values: I) -> Result<ArrayRef>
    where
        I: IntoIterator<Item = Option<V>>,
        V: AsRef<[u8]>,
    {
        let values = values.into_iter();
        let mut codes = Vec::with_capacity(values.size_hint().0);
        for value in values {
            codes.push(match value {
                Some(text) => Some(self.push_bytes(text.as_ref())?),
                None => None,
            });
        }
        let vocabulary = self.arrow_vocabulary()?;
        if matches!(self.key, DataType::Int64) {
            return Ok(Arc::new(DictionaryArray::<Int64Type>::try_new(
                Int64Array::from(codes),
                vocabulary,
            )?));
        }
        // `push` refuses past `i32::MAX` under an `Int32` key, so this narrows.
        let codes = codes
            .into_iter()
            .map(|code| code.map(i32::try_from).transpose())
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|_| self.key_capacity_refusal())?;
        Ok(Arc::new(DictionaryArray::<Int32Type>::try_new(
            Int32Array::from(codes),
            vocabulary,
        )?))
    }

    /// Recovers the vocabulary of a dictionary array over an ASCII width.
    ///
    /// The values keep the array's own order, so a code read from that array
    /// names the same value here.
    ///
    /// An array carries storage and no identity, so this reads the values
    /// datatype off the storage width: two, three, four, eight, twelve and
    /// sixteen bytes are the six ASCII widths. A [registered
    /// code](super::coded) shares that storage - a `mic` and an `ascii32`
    /// vocabulary are both `FixedSizeBinary(4)` - so a width cannot name one,
    /// and a vocabulary declared over a code is recovered by naming it:
    /// [`Self::from_arrow_array_as`]. A `cfi`'s six bytes are no ASCII width
    /// at all and are refused here rather than guessed at.
    ///
    /// # Errors
    ///
    /// Returns an error naming the layout it was given when `array` is not a
    /// dictionary of `Int32` or `Int64` keys over an ASCII width, one naming
    /// the position when its vocabulary holds a null, and one naming the value
    /// and both positions when its vocabulary repeats a value - a repeat would
    /// give one code away and shift every later one.
    pub fn from_arrow_array(array: &dyn Array) -> Result<Self> {
        let ArrowDataType::Dictionary(_, value) = array.data_type() else {
            return Err(layout_refusal(array.data_type()));
        };
        let ArrowDataType::FixedSizeBinary(width) = **value else {
            return Err(layout_refusal(array.data_type()));
        };
        let values_dtype = match width {
            2 => DataType::Ascii16,
            3 => DataType::Ascii24,
            4 => DataType::Ascii32,
            8 => DataType::Ascii64,
            12 => DataType::Ascii96,
            16 => DataType::Ascii128,
            _ => return Err(layout_refusal(array.data_type())),
        };
        Self::from_arrow_array_as(values_dtype, array)
    }

    /// Recovers the vocabulary of a dictionary array over a declared datatype.
    ///
    /// The reader that names its own vocabulary: `values` says which ASCII
    /// width or [registered code](super::coded) the storage holds, which is
    /// the half an array cannot carry. The array's storage width has to be
    /// that datatype's, so a `currency` vocabulary is not read as a `mic` one.
    ///
    /// # Errors
    ///
    /// Returns an error naming `values` when it is neither an ASCII width nor
    /// a registered code, and the same errors [`Self::from_arrow_array`]
    /// returns for a layout, a null, or a repeated value.
    pub fn from_arrow_array_as(values: DataType, array: &dyn Array) -> Result<Self> {
        let expected = values.ascii_width().ok_or_else(|| ascii_values_refusal(&values))?;
        let ArrowDataType::Dictionary(key, value) = array.data_type() else {
            return Err(layout_refusal(array.data_type()));
        };
        let ArrowDataType::FixedSizeBinary(width) = **value else {
            return Err(layout_refusal(array.data_type()));
        };
        if width != expected {
            return Err(Error::InvalidDataType {
                kind: "ascii-dictionary",
                reason: crate::text::expected_got(
                    format_args!("a vocabulary of {expected} bytes for {values}"),
                    format_args!("{width}"),
                ),
            });
        }
        let values_dtype = values;
        let (key_dtype, vocabulary) = match **key {
            ArrowDataType::Int32 => (
                DataType::Int32,
                Arc::clone(
                    array
                        .as_any()
                        .downcast_ref::<DictionaryArray<Int32Type>>()
                        .ok_or_else(|| layout_refusal(array.data_type()))?
                        .values(),
                ),
            ),
            ArrowDataType::Int64 => (
                DataType::Int64,
                Arc::clone(
                    array
                        .as_any()
                        .downcast_ref::<DictionaryArray<Int64Type>>()
                        .ok_or_else(|| layout_refusal(array.data_type()))?
                        .values(),
                ),
            ),
            _ => return Err(layout_refusal(array.data_type())),
        };
        let stored = vocabulary
            .as_any()
            .downcast_ref::<FixedSizeBinaryArray>()
            .ok_or_else(|| layout_refusal(array.data_type()))?;
        // Position is the code, so the vocabulary is built positionally
        // rather than through `from_values`, which would collapse a repeat.
        let mut dictionary = Self::new(values_dtype)?.with_key(key_dtype)?;
        for index in 0..stored.len() {
            if stored.is_null(index) {
                return Err(Error::InvalidDataType {
                    kind: "ascii-dictionary",
                    reason: format_smolstr!("a null vocabulary value at {index}"),
                });
            }
            let value = SmolStr::new(ascii_text(width, stored.value(index))?);
            let code = i64::try_from(index)
                .ok()
                .filter(|code| *code < dictionary.key_limit())
                .ok_or_else(|| dictionary.key_capacity_refusal())?;
            if let Some(first) = dictionary.index.insert(value.clone(), code) {
                return Err(Error::InvalidDataType {
                    kind: "ascii-dictionary",
                    reason: crate::text::expected_got(
                        format_args!("a vocabulary with no repeated value"),
                        format_args!("{value:?} at {first} and {code}"),
                    ),
                });
            }
            dictionary.values.push(value);
        }
        Ok(dictionary)
    }

    /// The vocabulary as the width's padded `FixedSizeBinary` storage.
    fn arrow_vocabulary(&self) -> Result<ArrayRef> {
        let width = self.byte_width()?;
        let slot = usize::try_from(width).map_err(|_| ascii_values_refusal(&self.width))?;
        let mut bytes = vec![0_u8; self.values.len() * slot];
        for (index, value) in self.values.iter().enumerate() {
            ascii_padded(&mut bytes[index * slot..][..slot], value);
        }
        Ok(Arc::new(FixedSizeBinaryArray::try_new(
            width,
            arrow_buffer::Buffer::from(bytes),
            None,
        )?))
    }
}

#[cfg(feature = "arrow")]
fn layout_refusal(actual: &ArrowDataType) -> Error {
    Error::InvalidDataType {
        kind: "ascii-dictionary",
        reason: crate::text::expected_got(
            format_args!(
                "a dictionary array of int32 or int64 keys over one of the six ASCII \
                 widths, or over the datatype `from_arrow_array_as` was given"
            ),
            format_args!("{actual}"),
        ),
    }
}
