//! ASCII text: variable, or padded with trailing NUL to a fixed byte width.
//!
//! The value contract, stated once: a value is ASCII text - every byte at
//! most `0x7F` - with no NUL byte, and at most the width in bytes where there
//! is one. [`DataType::FixedAscii`] pads storage with trailing `\0` to
//! exactly its width (Arrow `FixedSizeBinary(width)`) and every string
//! rendering trims that padding, so storage reads back as the text that went
//! in; [`DataType::Ascii`] has no width and stores the bytes it was given
//! (Arrow `Binary`). The canonical value spelling is `Scalar::String`;
//! `Scalar::Bytes` and a string carrying trailing NULs are accepted on the
//! way in under the same rule and canonicalize to the trimmed string.
//!
//! Two shapes are the whole family. A width says how many bytes a value may
//! take and nothing about what the value means, so `ascii(4)` is a ticker, a
//! venue code, or four bytes of anything else; `ascii` is the same rule with
//! no length to promise.
//!
//! Four identifiers are specific enough to be types of their own - `country`,
//! `currency`, `mic` and `cfi` - and live in [`super::coded`]. They share
//! every rule stated here, so [`DataType::ascii_width`], [`ascii_text`],
//! [`DataType::ascii_packed`] and [`AsciiEnum`] answer for a code exactly as
//! they do for a width. What they add is identity: a currency column reads
//! back a currency, never three anonymous bytes.
//!
//! [`AsciiEnum`] is the vocabulary a field declares: named members over ASCII
//! values, whose code is [`DataType::ascii_packed`] of the value under the
//! field's own width - the value's own bytes, the same integer in every
//! process, never a position in some column's listing.

use std::collections::BTreeMap;

use smol_str::{SmolStr, format_smolstr};

use crate::{DataType, Error, Result, Scalar};

use super::coded::{CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, MIC_WIDTH};

/// The Arrow extension name of the three ASCII widths.
///
/// The storage is `FixedSizeBinary(4 | 8 | 16)` and the extension metadata
/// is the empty string: the storage width says the width.
pub(crate) const ASCII_EXTENSION_NAME: &str = "yggdryl.ascii";

impl DataType {
    /// Creates the fixed ASCII width of exactly `width` bytes.
    ///
    /// The family has two shapes and this builds the fixed one: `ascii(4)` is
    /// four bytes and nothing else. [`Self::Ascii`] is the other, which takes
    /// values of any length and stores them variable-width.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::ascii(4)?, DataType::FixedAscii(4));
    /// assert_eq!(DataType::ascii(4)?.to_string(), "ascii(4)");
    /// assert_eq!(DataType::Ascii.to_string(), "ascii");
    /// assert_eq!(DataType::Ascii.ascii_width(), None);
    /// assert!(DataType::ascii(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when `width` is not at least one byte.
    pub fn ascii(width: i32) -> Result<Self> {
        if width < 1 {
            return Err(Error::InvalidDataType {
                kind: "ascii",
                reason: format_smolstr!("expected an ASCII width of at least 1 byte, got {width}"),
            });
        }
        Ok(Self::FixedAscii(width))
    }

    /// The storage width of a fixed ASCII datatype, `None` for every other.
    ///
    /// [`Self::Ascii`] has no width - that is what makes it the variable one -
    /// and a [registered code](super::coded) has the width its standard
    /// fixes, so this one accessor answers for every fixed ASCII storage.
    pub const fn ascii_width(&self) -> Option<i32> {
        match self {
            Self::FixedAscii(width) => Some(*width),
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

    /// Whether this is one of the ASCII datatypes, fixed or variable.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// assert!(DataType::Ascii.is_ascii());
    /// assert!(DataType::FixedAscii(4).is_ascii());
    /// assert!(DataType::Currency.is_ascii());
    /// assert!(!DataType::Utf8.is_ascii());
    /// ```
    #[must_use]
    pub const fn is_ascii(&self) -> bool {
        matches!(self, Self::Ascii) || self.ascii_width().is_some()
    }

    /// The integer an ASCII value packs into: its storage bytes, big-endian.
    ///
    /// Storage pads with trailing NUL to the width, so the packed integer
    /// orders exactly as the text does and is the same integer in every
    /// process - what a stable hash and a portable enum member both need. An
    /// ASCII byte is at most `0x7F`, so the sign bit is never set and the
    /// value is never negative: four bytes fill an `i32`, eight an `i64`, and
    /// sixteen the whole `i128`.
    ///
    /// Only a fixed width has one. [`Self::Ascii`] takes a value of any
    /// length, so there is no integer its bytes always fit, and a width above
    /// sixteen bytes outgrows the widest integer this crate carries; both are
    /// refused rather than truncated.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // `USD` stores as `USD\0` under `ascii(4)`, which is that big-endian
    /// // `i32`; under `currency` it is the three bytes and nothing else.
    /// assert_eq!(DataType::FixedAscii(4).ascii_packed(b"USD")?, 0x5553_4400);
    /// assert_eq!(DataType::FixedAscii(4).ascii_packed(b"USD\0")?, 0x5553_4400);
    /// assert_eq!(DataType::FixedAscii(4).ascii_value(0x5553_4400)?, "USD");
    /// assert_eq!(DataType::Currency.ascii_packed(b"USD")?, 0x0055_5344);
    ///
    /// // The order of the integers is the order of the text.
    /// let ccy = DataType::FixedAscii(4);
    /// assert!(ccy.ascii_packed(b"EUR")? < ccy.ascii_packed(b"USD")?);
    ///
    /// // Twelve bytes need 96 bits, and sixteen the whole `i128`.
    /// let isin = DataType::FixedAscii(12).ascii_packed(b"US0378331005")?;
    /// assert_eq!(DataType::FixedAscii(12).ascii_value(isin)?, "US0378331005");
    /// assert!(isin > i128::from(u64::MAX));
    ///
    /// assert!(ccy.ascii_packed(b"EURO!").is_err());
    /// assert!(DataType::Ascii.ascii_packed(b"USD").is_err());
    /// assert!(DataType::FixedAscii(17).ascii_packed(b"USD").is_err());
    /// assert!(DataType::Utf8.ascii_packed(b"USD").is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted datatypes when this has no width
    /// or its width outgrows an `i128`, and one naming the width when `value`
    /// is not ASCII text that fits it.
    pub fn ascii_packed(&self, value: &[u8]) -> Result<i128> {
        let width = self.packed_width()?;
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
        let width = self.packed_width()?;
        let slot = usize::try_from(width).unwrap_or(0);
        let bytes = packed.to_be_bytes();
        let (above, stored) = bytes.split_at(bytes.len() - slot);
        if above.iter().any(|byte| *byte != 0) {
            return Err(ascii_refusal(
                Some(slot),
                format_smolstr!("the integer {packed}, which is wider than the width"),
            ));
        }
        ascii_text(width, stored).map(SmolStr::new)
    }

    /// The width a packed integer may be built from, refusing the rest.
    fn packed_width(&self) -> Result<i32> {
        match self.ascii_width() {
            Some(width) if width <= PACKED_LIMIT => Ok(width),
            _ => Err(ascii_values_refusal(self)),
        }
    }
}

/// The widest fixed ASCII storage one `i128` holds, in bytes.
const PACKED_LIMIT: i32 = 16;

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
    ascii_text_sized(Some(usize::try_from(width).unwrap_or(0)), bytes)
}

/// [`ascii_text`] with no width to fit, for [`DataType::Ascii`].
///
/// Every other rule is the same - no NUL, every byte at most `0x7F`, trailing
/// NULs trimmed - because the value contract is the family's, not the width's.
///
/// # Errors
///
/// Returns an error when the bytes hold a NUL or a non-ASCII byte.
pub(crate) fn ascii_free_text(bytes: &[u8]) -> Result<&str> {
    ascii_text_sized(None, bytes)
}

/// [`ascii_text`] over the width the caller already holds, if there is one.
///
/// The one body every shape runs: a fixed width passes its length, a
/// [`super::coded`] code passes its constant - which lets the length check
/// fold at each code's call site - and the variable shape passes `None`.
#[inline]
pub(crate) fn ascii_text_sized(width: Option<usize>, bytes: &[u8]) -> Result<&str> {
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
    if width.is_some_and(|width| text.len() > width) {
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
                "a fixed ASCII width of at most {PACKED_LIMIT} bytes, or a registered \
                 code (country, currency, mic, cfi)"
            ),
            format_args!("{values}"),
        ),
    }
}

/// The enum an ASCII field's values name: one value per member name.
///
/// This is the vocabulary a declaration named itself, and it is what a
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
///     side.into_members(&DataType::FixedAscii(4))?,
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

    /// The enum member name one ASCII value takes.
    ///
    /// An ASCII letter is kept uppercased, a digit is kept, every other byte
    /// becomes `_`, a leading digit takes a `_` in front, and a name that both
    /// opens and closes with `_` drops its trailing underscores - that shape
    /// is what Python reserves for `_sunder_` and `__dunder__` names, where a
    /// member is refused or silently dropped.
    ///
    /// The rule belongs to the vocabulary rather than to one width, so an enum
    /// that registers one value at a time names it exactly as generating a
    /// whole listing at once would.
    ///
    /// ```
    /// use yggdryl::AsciiEnum;
    ///
    /// assert_eq!(AsciiEnum::member_name("USD").as_str(), "USD");
    /// assert_eq!(AsciiEnum::member_name("n/a").as_str(), "N_A");
    /// assert_eq!(AsciiEnum::member_name("-a-").as_str(), "_A");
    /// assert_eq!(AsciiEnum::member_name("").as_str(), "_");
    /// ```
    #[must_use]
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
