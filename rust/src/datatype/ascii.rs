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
//! [`AsciiDictionary`] is the per-column vocabulary over one of those widths:
//! an ordered set of values whose position is the code a `dictionary(key,
//! ascii-N)` column stores.

use std::collections::HashMap;
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

/// A per-column ASCII vocabulary and the codes that name its values.
///
/// The vocabulary is a value, never a process-global registry: a caller holds
/// one per column, and a code is stable exactly as long as that value is
/// carried. Two independent encodes build two vocabularies, and their codes
/// agree only when the same `AsciiDictionary` crossed both. Nothing in the
/// write path registers on its own - a caller that wants the encoding
/// declares it and registers as it encodes.
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

    /// The generated-enum members: one name per value, paired with its code.
    ///
    /// The name rule, stated once here and used by every binding: an ASCII
    /// letter is kept uppercased, a digit is kept, every other byte becomes
    /// `_`, a name starting with a digit is prefixed with `_`, a name that
    /// both opens and closes with `_` drops its trailing underscores - the
    /// shape Python reserves for `_sunder_` and `__dunder__` names - and the
    /// empty value becomes `_`. Two values that would name one member are an
    /// error naming both, never a silent rename.
    ///
    /// ```
    /// use yggdryl::{AsciiDictionary, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let currencies = AsciiDictionary::from_values(DataType::Ascii32, ["USD", "n/a", "-a-"])?;
    /// assert_eq!(
    ///     currencies.into_members()?,
    ///     [("USD".into(), 0), ("N_A".into(), 1), ("_A".into(), 2)]
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the two supported widths when the values are
    /// `Ascii128` - a sixteen-byte vocabulary is text, not enum members - and
    /// one naming both values when two of them collide.
    pub fn into_members(&self) -> Result<Vec<(SmolStr, i64)>> {
        if matches!(self.width, DataType::Ascii128) {
            return Err(Error::InvalidDataType {
                kind: "ascii-dictionary",
                reason: crate::text::expected_got(
                    format_args!("ascii32 or ascii64 values to name enum members"),
                    format_args!("{}", self.width),
                ),
            });
        }
        let mut members = Vec::with_capacity(self.values.len());
        let mut taken: HashMap<SmolStr, &str> = HashMap::with_capacity(self.values.len());
        for (code, value) in self.values.iter().enumerate() {
            let name = member_name(value);
            if let Some(first) = taken.insert(name.clone(), value.as_str()) {
                return Err(Error::InvalidDataType {
                    kind: "ascii-dictionary",
                    reason: format_smolstr!(
                        "the values {first:?} and {value:?} both name the member {name}"
                    ),
                });
            }
            let code = i64::try_from(code).map_err(|_| self.key_capacity_refusal())?;
            members.push((name, code));
        }
        Ok(members)
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

/// The enum member name of one ASCII value, under the rule stated on
/// [`AsciiDictionary::into_members`].
fn member_name(value: &str) -> SmolStr {
    let mut name = String::with_capacity(value.len() + 1);
    // Every registered value passed `ascii_text`, so one byte is one character.
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

fn ascii_values_refusal(values: &DataType) -> Error {
    Error::InvalidDataType {
        kind: "ascii-dictionary",
        reason: crate::text::expected_got(
            format_args!("an ASCII width (ascii32, ascii64, or ascii128)"),
            format_args!("{values}"),
        ),
    }
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
    /// # Errors
    ///
    /// Returns an error naming the layout it was given when `array` is not a
    /// dictionary of `Int32` or `Int64` keys over an ASCII width, one naming
    /// the position when its vocabulary holds a null, and one naming the value
    /// and both positions when its vocabulary repeats a value - a repeat would
    /// give one code away and shift every later one.
    pub fn from_arrow_array(array: &dyn Array) -> Result<Self> {
        let ArrowDataType::Dictionary(key, value) = array.data_type() else {
            return Err(layout_refusal(array.data_type()));
        };
        let ArrowDataType::FixedSizeBinary(width) = **value else {
            return Err(layout_refusal(array.data_type()));
        };
        let values_dtype = match width {
            4 => DataType::Ascii32,
            8 => DataType::Ascii64,
            16 => DataType::Ascii128,
            _ => return Err(layout_refusal(array.data_type())),
        };
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
            format_args!("a dictionary array of int32 or int64 keys over an ASCII width"),
            format_args!("{actual}"),
        ),
    }
}
