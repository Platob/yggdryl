//! The string value: the characters, held compactly, beside the layout and
//! charset its column stores them under.
//!
//! A string value holds UTF-8 whatever charset it arrived in. That is the
//! whole point of decoding at the seam: the bytes are read once, at the
//! boundary that knows the charset, and everything above it reads characters.
//! What the value keeps is the layout and charset it is *written* under - and
//! the width, on the fixed layout - so the same value goes back out the way
//! it came without the column being consulted twice. A maximum is the
//! column's rule and never the value's: a value read out of `utf8(32)` is a
//! `utf8`, exactly as an integer read out of a bounded column is an integer.
//!
//! [`Str`] is the one representation, and it is the crate's compact string:
//! up to [`INLINE_CAPACITY`] bytes live inside the value with no heap behind
//! them, a longer text is one shared `Arc<str>` that clones by reference
//! count, and a `&'static str` costs nothing at all. Equality, order and
//! hashing read the characters alone - a value is one value whichever column
//! holds it - and the parameters ride beside them.

use std::borrow::{Borrow, Cow};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::str::FromStr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use super::{StringLayout, StringParameters, trim_padding};
use crate::types::Scalar;
use crate::{
    Charset, DataType, DataTypeId, DataTypeKind, Error, Result, ScalarFamily, ScalarValue,
};

/// How many bytes of text a [`Str`] holds without reaching the heap.
///
/// The storage is `smol_str`'s, which does not export the number, so it is
/// pinned here and asserted in the tests beside it: a code, a currency pair,
/// a ticker or an ISO date all fit under it and never allocate.
pub const INLINE_CAPACITY: usize = 23;

/// One string value: its characters and the parameters it is stored under.
///
/// ```
/// use yggdryl::types::{INLINE_CAPACITY, Str, StringLayout, StringParameters};
/// use yggdryl::{Charset, DataType, Scalar};
///
/// # fn main() -> yggdryl::Result<()> {
/// // Short text lives inside the value; longer text is one shared handle.
/// let short = Str::new("AAPL");
/// assert!(short.is_inline());
/// let long = Str::new("a".repeat(INLINE_CAPACITY + 1));
/// assert!(!long.is_inline());
///
/// // A value is one value whichever layout or charset it is stored under.
/// let latin = StringParameters::new(StringLayout::LargeString, Charset::Cp1252);
/// let restated = short.clone().try_with_parameters(latin)?;
/// assert_eq!(restated, short);
/// assert_eq!(restated.charset(), Charset::Cp1252);
/// assert_eq!(restated.dtype()?, DataType::from_str("large_string(windows-1252)")?);
///
/// // It is the crate's string, so it is the `Scalar` string too.
/// assert_eq!(Scalar::from("AAPL"), Scalar::String(short));
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct Str {
    text: SmolStr,
    parameters: StringParameters,
}

const _: () = assert!(std::mem::size_of::<Str>() == 32);

impl Str {
    /// Text the binary already holds, under the default parameters.
    ///
    /// Costs nothing: no copy, no count, so a constant spelling is a
    /// constant value.
    #[must_use]
    pub const fn new_static(text: &'static str) -> Self {
        Self {
            text: SmolStr::new_static(text),
            parameters: StringParameters::utf8(StringLayout::String),
        }
    }

    /// A string value under the default parameters: UTF-8, `string` layout.
    ///
    /// Text up to [`INLINE_CAPACITY`] bytes is copied into the value and
    /// allocates nothing; longer text is one shared `Arc<str>`.
    pub fn new(text: impl AsRef<str>) -> Self {
        Self {
            text: SmolStr::new(text),
            parameters: StringParameters::default(),
        }
    }

    /// Read the bytes a column stores, under the parameters it declares.
    ///
    /// This is the one door bytes take into a string value, and the charset
    /// decides how strict it is. UTF-8 and US-ASCII are validated
    /// repertoires - Arrow guarantees the first and the second rides Arrow's
    /// text storage - so bytes that are not what they claim are refused
    /// through [`Charset::decode`], and a US-ASCII value holds no NUL and no
    /// byte above `0x7F`. Every other charset is a declaration that the
    /// column holds legacy bytes, and those are read through
    /// [`Charset::transcribe`]: a byte the charset leaves unassigned reads as
    /// the scalar ISO 8859-1 gives it, because a legacy export with one bad
    /// byte in a million is a file that still has to be read.
    ///
    /// On the fixed layout the trailing NUL padding the storage writes is
    /// taken off first, and the value carries the width the parameters
    /// declare. A maximum is checked and not carried.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] naming the charset and the first byte it
    /// refuses, [`Error::InvalidDataType`] for a fixed layout with no width,
    /// and [`Error::InvalidRecord`] when the text does not fit the bound.
    pub fn from_bytes(bytes: &[u8], parameters: StringParameters) -> Result<Self> {
        let payload = match parameters.is_fixed() {
            true => trim_padding(bytes),
            false => bytes,
        };
        let charset = parameters.charset();
        let text = match charset {
            Charset::Utf8 | Charset::Ascii => SmolStr::new(charset.decode(payload)?),
            _ => charset.transcribe_smol(payload),
        };
        Self {
            text,
            parameters: StringParameters::default(),
        }
        .try_with_parameters(parameters)
    }

    /// The text a column's own storage holds, under the parameters it
    /// declares, checked when it was written and not again here.
    #[cfg(feature = "arrow")]
    pub(crate) fn from_storage(text: &str, parameters: StringParameters) -> Self {
        Self {
            text: SmolStr::new(text),
            parameters: parameters.without_max(),
        }
    }

    /// Restate the same characters under other parameters.
    ///
    /// The characters do not change and the storage is shared, not copied;
    /// what changes is the bytes [`Self::encode`] answers and the datatype
    /// [`Self::dtype`] declares. On the fixed layout trailing NUL is padding,
    /// so it is taken off here rather than left for every reader to trim.
    ///
    /// The bound is checked and, on a variable layout, not carried: it is the
    /// column's rule, and the value answers the layout alone. The bound counts
    /// stored bytes, which [`Charset::encoded_len`] answers without building
    /// them. US-ASCII is a repertoire and is judged here - a value holding a
    /// scalar above `0x7F` or a NUL is refused - but every other charset is
    /// only counted: whether its bytes can be written is the write seam's
    /// question, because a value read back through [`Charset::transcribe`]
    /// carries scalars the charset does not assign, and that is what
    /// recovering damage means.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a fixed layout with no width,
    /// and [`Error::InvalidRecord`] naming the bound and the stored length
    /// when the text does not fit it, or the byte that is not US-ASCII.
    pub fn try_with_parameters(mut self, parameters: StringParameters) -> Result<Self> {
        parameters.validate()?;
        if parameters.is_fixed() {
            let trimmed = self.text.trim_end_matches('\0');
            if trimmed.len() != self.text.len() {
                self.text = SmolStr::new(trimmed);
            }
        }
        let charset = parameters.charset();
        if charset == Charset::Ascii {
            ascii_repertoire(self.text.as_bytes())?;
        }
        if let Some(bound) = parameters.bound() {
            let stored = charset.encoded_len(&self.text);
            if stored > bound as usize {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        format_args!("at most {bound} bytes of {charset}"),
                        format_smolstr!("{stored}"),
                    ),
                });
            }
        }
        self.parameters = parameters.without_max();
        Ok(self)
    }

    /// Borrow the characters.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// Borrow the shared storage without copying the text.
    ///
    /// The storage is the crate's ordinary compact string, so a name, a key
    /// or a code adopts a value's text by cloning this handle.
    #[must_use]
    pub const fn storage(&self) -> &SmolStr {
        &self.text
    }

    /// The parameters this value is stored under.
    ///
    /// Never a maximum: that is the column's declaration, and a value read
    /// out of a bounded column answers its layout and charset alone.
    #[must_use]
    pub const fn parameters(&self) -> StringParameters {
        self.parameters
    }

    /// The layout this value is stored in.
    #[must_use]
    pub const fn layout(&self) -> StringLayout {
        self.parameters.layout()
    }

    /// The charset this value's bytes are written in.
    #[must_use]
    pub const fn charset(&self) -> Charset {
        self.parameters.charset()
    }

    /// The padded storage width, on the fixed layout alone.
    #[must_use]
    pub const fn fixed(&self) -> Option<u32> {
        self.parameters.fixed()
    }

    /// Whether the characters live inside the value with no heap behind them.
    #[must_use]
    pub fn is_inline(&self) -> bool {
        !self.text.is_heap_allocated()
    }

    /// The stored bytes of this value, in the charset it declares.
    ///
    /// UTF-8 text borrows, and so does any all-ASCII value in any of the
    /// ASCII-compatible charsets, so the ordinary column costs nothing to
    /// write back out. The fixed layout answers its whole padded slot.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Codec`] naming the first scalar the charset has no
    /// byte for.
    pub fn encode(&self) -> Result<Cow<'_, [u8]>> {
        let encoded = self.charset().encode(self.as_str())?;
        Ok(match self.fixed() {
            Some(width) => {
                let mut padded = encoded.into_owned();
                padded.resize(width as usize, 0);
                Cow::Owned(padded)
            }
            None => encoded,
        })
    }

    /// How many bytes [`Self::encode`] answers, without building them.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        match self.fixed() {
            Some(width) => width as usize,
            None => self.charset().encoded_len(self.as_str()),
        }
    }

    /// Consume this value and return its characters as an owned `String`.
    #[must_use]
    pub fn into_string(self) -> String {
        self.text.to_string()
    }

    /// Consume this value and return its compact storage.
    #[must_use]
    pub fn into_inner(self) -> SmolStr {
        self.text
    }

    /// The datatype this value materializes into.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] only for parameters a constructor
    /// would have refused, which no door here builds.
    pub fn dtype(&self) -> Result<DataType> {
        DataType::string(self.parameters)
    }
}

/// Refuse what US-ASCII text never holds: a NUL, or a byte above `0x7F`.
///
/// The byte class is one fact and [`Charset`] owns it: this is that
/// charset's own scan, which reads a machine word at a time.
fn ascii_repertoire(bytes: &[u8]) -> Result<()> {
    let refusal = |actual: SmolStr| Error::InvalidRecord {
        path: SmolStr::new_static("$"),
        reason: crate::text::expected_got(format_args!("US-ASCII text"), actual),
    };
    if let Some(position) = bytes.iter().position(|byte| *byte == 0) {
        return Err(refusal(format_smolstr!("a NUL byte at {position}")));
    }
    let position = crate::charset::ascii_len(bytes);
    if position < bytes.len() {
        return Err(refusal(format_smolstr!(
            "a non-ASCII byte 0x{:02X} at {position}",
            bytes[position]
        )));
    }
    Ok(())
}

impl Default for Str {
    fn default() -> Self {
        Self::new_static("")
    }
}

impl Deref for Str {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for Str {
    fn as_ref(&self) -> &[u8] {
        self.as_str().as_bytes()
    }
}

/// Sound because [`Eq`], [`Ord`] and [`Hash`] read the characters alone, as
/// `str`'s do; folding the parameters into any of them would break every
/// `BTreeMap<Str, _>::get(&str)` in the crate.
impl Borrow<str> for Str {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for Str {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Debug for Str {
    /// The characters, and the parameters when they are not the default, so
    /// two values that compare equal but declare different columns print
    /// apart in a failing assertion.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)?;
        if self.parameters != StringParameters::default() {
            write!(formatter, " as {}", self.parameters)?;
        }
        Ok(())
    }
}

impl PartialEq for Str {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Str {}

impl PartialEq<str> for Str {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Str {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Str> for str {
    fn eq(&self, other: &Str) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Str> for &str {
    fn eq(&self, other: &Str) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<String> for Str {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Str> for String {
    fn eq(&self, other: &Str) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<SmolStr> for Str {
    fn eq(&self, other: &SmolStr) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Str> for SmolStr {
    fn eq(&self, other: &Str) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialOrd for Str {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Str {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Str {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl From<&str> for Str {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<&mut str> for Str {
    fn from(value: &mut str) -> Self {
        Self::new(value)
    }
}

impl From<&String> for Str {
    fn from(value: &String) -> Self {
        Self::new(value)
    }
}

impl From<String> for Str {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<Box<str>> for Str {
    fn from(value: Box<str>) -> Self {
        Self::new(value)
    }
}

impl From<Arc<str>> for Str {
    fn from(value: Arc<str>) -> Self {
        Self::new(value)
    }
}

impl From<Cow<'_, str>> for Str {
    fn from(value: Cow<'_, str>) -> Self {
        Self::new(value)
    }
}

impl From<char> for Str {
    fn from(value: char) -> Self {
        let mut encoded = [0_u8; 4];
        Self::new(value.encode_utf8(&mut encoded))
    }
}

impl From<SmolStr> for Str {
    fn from(value: SmolStr) -> Self {
        Self {
            text: value,
            parameters: StringParameters::default(),
        }
    }
}

impl From<&SmolStr> for Str {
    fn from(value: &SmolStr) -> Self {
        Self::from(value.clone())
    }
}

impl From<Str> for String {
    fn from(value: Str) -> Self {
        value.into_string()
    }
}

impl From<&Str> for String {
    fn from(value: &Str) -> Self {
        value.as_str().to_owned()
    }
}

impl From<Str> for SmolStr {
    fn from(value: Str) -> Self {
        value.text
    }
}

impl From<&Str> for SmolStr {
    fn from(value: &Str) -> Self {
        value.text.clone()
    }
}

impl From<Str> for Arc<str> {
    fn from(value: Str) -> Self {
        Self::from(value.as_str())
    }
}

impl FromStr for Str {
    type Err = std::convert::Infallible;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::new(value))
    }
}

impl FromIterator<char> for Str {
    fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<SmolStr>())
    }
}

impl<'a> FromIterator<&'a str> for Str {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<SmolStr>())
    }
}

/// The serde representation of a string that declares more than its text.
///
/// The ordinary value - UTF-8, the `string` layout - serializes its
/// characters and nothing else, exactly as it always has; a layout, a
/// charset or a fixed width is what makes a value carry more than that.
#[derive(Deserialize, Serialize)]
struct Declared<'a> {
    layout: StringLayout,
    charset: Charset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixed: Option<u32>,
    text: Cow<'a, str>,
}

impl Serialize for Str {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if self.parameters == StringParameters::default() {
            return serializer.serialize_str(self.as_str());
        }
        Declared {
            layout: self.layout(),
            charset: self.charset(),
            fixed: self.fixed(),
            text: Cow::Borrowed(self.as_str()),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Str {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Representation<'a> {
            Plain(Cow<'a, str>),
            Declared(Declared<'a>),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Plain(text) => Ok(Self::from(text)),
            Representation::Declared(declared) => {
                let mut parameters = StringParameters::new(declared.layout, declared.charset);
                if let Some(width) = declared.fixed {
                    parameters = parameters
                        .try_with_bound(width)
                        .map_err(serde::de::Error::custom)?;
                }
                Self::from(declared.text)
                    .try_with_parameters(parameters)
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

impl ScalarValue for Str {
    type Family = Str;

    /// The family's default layout; [`ScalarFamily::id`] answers the exact
    /// layout a value is stored in.
    const ID: DataTypeId = DataTypeId::String;
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_family(self) -> Self::Family {
        self
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        Some(family)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::String(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::String(value) => Some(value),
            _ => None,
        }
    }
}

impl ScalarFamily for Str {
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn id(&self) -> DataTypeId {
        self.layout().id()
    }

    fn dtype(&self) -> Result<DataType> {
        Self::dtype(self)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::String(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::String(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Str> for Scalar {
    fn from(value: Str) -> Self {
        Self::String(value)
    }
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Self::String(Str::new(value))
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Self {
        Self::String(Str::new(value))
    }
}

impl From<SmolStr> for Scalar {
    fn from(value: SmolStr) -> Self {
        Self::String(Str::from(value))
    }
}

/// The canonical text a value spells, shared rather than rebuilt.
///
/// A string column stores one string per row, and this is the spelling every
/// tier prints: the canonical [`std::fmt::Display`] each family owns for the
/// numbers and the boolean, [`Scalar::into_temporal_text`] for a temporal -
/// which is what a temporal *column* renders too, zone rules included - the
/// WKT a geometry column renders, and the payload's own characters for bytes.
/// A string, a code and a generic enum member already hold their text and
/// hand it over without allocating.
///
/// `None` is a kind that spells no text at all; `Some(Err)` is a payload that
/// was read and refused, so the refusal names what was wrong with it rather
/// than which kind arrived.
pub(crate) fn str_from_value(value: &Scalar) -> Option<Result<Str>> {
    Some(match value {
        Scalar::String(text) => Ok(text.clone()),
        Scalar::Code(code) => Ok(Str::from(code.storage())),
        Scalar::Enum(member) => Ok(Str::new_static(member.as_str())),
        Scalar::Integer(number) => Ok(Str::from(format_smolstr!("{number}"))),
        Scalar::Floating(number) => Ok(Str::from(format_smolstr!("{number}"))),
        Scalar::Decimal(number) => Ok(Str::from(format_smolstr!("{number}"))),
        Scalar::Boolean(flag) => Ok(Str::from(format_smolstr!("{flag}"))),
        Scalar::Uuid(uuid) => Ok(Str::from(format_smolstr!("{uuid}"))),
        Scalar::Version(version) => Ok(Str::from(format_smolstr!("{version}"))),
        Scalar::Url(url) => Ok(Str::from(format_smolstr!("{url}"))),
        // An interval has no classic spelling, so a temporal answers for the
        // seven that do and leaves the rest to the ordinary refusal.
        Scalar::Temporal(_) => return value.into_temporal_text().map(|text| Ok(Str::from(text))),
        Scalar::Bytes(bytes) => {
            std::str::from_utf8(bytes.as_bytes())
                .map(Str::new)
                .map_err(|error| Error::InvalidRecord {
                    path: SmolStr::new_static("$"),
                    reason: format_smolstr!("payload is not UTF-8: {error}"),
                })
        }
        Scalar::Geospatial(geospatial) => {
            crate::types::geospatial::wkb::into_wkt(geospatial.as_bytes()).map(Str::from)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{INLINE_CAPACITY, Str};
    use crate::types::{StringLayout, StringParameters};
    use crate::{Charset, DataType, Scalar};

    #[test]
    fn short_text_is_inline_and_long_text_is_shared() {
        let short = Str::new("a".repeat(INLINE_CAPACITY));
        assert!(short.is_inline());
        assert_eq!(short.len(), INLINE_CAPACITY);
        let long = Str::new("a".repeat(INLINE_CAPACITY + 1));
        assert!(!long.is_inline());
        assert_eq!(long.len(), INLINE_CAPACITY + 1);
        assert!(Str::new_static("held").is_inline());
        assert_eq!(Str::default(), "");
        assert_eq!(std::mem::size_of::<Str>(), 32);
    }

    #[test]
    fn equality_order_and_hash_read_the_characters_only() {
        use std::collections::HashSet;

        let plain = Str::new("Grüße");
        let latin = Str::new("Grüße")
            .try_with_parameters(StringParameters::new(
                StringLayout::LargeString,
                Charset::Cp1252,
            ))
            .unwrap();
        assert_eq!(plain, latin);
        assert_eq!(plain.cmp(&latin), std::cmp::Ordering::Equal);
        assert_eq!(HashSet::from([plain.clone(), latin.clone()]).len(), 1);
        assert_ne!(plain.parameters(), latin.parameters());
        assert!(Str::new("b") > Str::new("a"));
        assert_eq!(
            format!("{latin:?}"),
            "\"Grüße\" as large_string(windows-1252)"
        );
        assert_eq!(format!("{plain:?}"), "\"Grüße\"");
    }

    #[test]
    fn restating_shares_the_storage_and_checks_but_never_carries_a_maximum() {
        let text = "x".repeat(INLINE_CAPACITY + 22);
        let shared = Str::new(&text);
        let bounded = StringParameters::utf8(StringLayout::String)
            .try_with_bound(64)
            .unwrap();
        let restated = shared.clone().try_with_parameters(bounded).unwrap();
        assert!(std::ptr::eq(shared.as_str(), restated.as_str()));
        assert_eq!(restated.parameters(), StringParameters::default());
        assert_eq!(restated.dtype().unwrap(), DataType::utf8());

        let tight = StringParameters::utf8(StringLayout::String)
            .try_with_bound(8)
            .unwrap();
        let refused = shared.try_with_parameters(tight).unwrap_err().to_string();
        assert!(refused.contains("at most 8 bytes"), "{refused}");

        // The bound counts stored bytes, so five scalars are five bytes in
        // windows-1252 and seven in UTF-8.
        let five = StringParameters::new(StringLayout::String, Charset::Cp1252)
            .try_with_bound(5)
            .unwrap();
        assert!(Str::new("Grüße").try_with_parameters(five).is_ok());
        let five = StringParameters::utf8(StringLayout::String)
            .try_with_bound(5)
            .unwrap();
        assert!(Str::new("Grüße").try_with_parameters(five).is_err());
    }

    #[test]
    fn us_ascii_is_a_repertoire_and_every_other_charset_is_counted() {
        let ascii = StringParameters::ascii(StringLayout::String);
        assert!(Str::new("plain").try_with_parameters(ascii).is_ok());
        let refused = Str::new("café")
            .try_with_parameters(ascii)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("non-ASCII byte"), "{refused}");
        assert!(Str::new("a\0b").try_with_parameters(ascii).is_err());
        // `U+0081` has no windows-1252 byte, and the value door only counts.
        let latin = StringParameters::new(StringLayout::String, Charset::Cp1252);
        let recovered = Str::new("ok\u{0081}").try_with_parameters(latin).unwrap();
        assert!(recovered.encode().is_err());
        assert_eq!(recovered.encoded_len(), 3);
    }

    #[test]
    fn a_fixed_layout_trims_its_padding_and_pads_on_the_way_out() {
        let fixed = StringParameters::ascii(StringLayout::FixedString)
            .try_with_bound(4)
            .unwrap();
        let value = Str::new("USD\0").try_with_parameters(fixed).unwrap();
        assert_eq!(value, "USD");
        assert_eq!(value.fixed(), Some(4));
        assert_eq!(value.parameters(), fixed);
        assert_eq!(value.encode().unwrap().as_ref(), b"USD\0");
        assert_eq!(value.encoded_len(), 4);
        assert!(Str::new("EURO!").try_with_parameters(fixed).is_err());
        assert!(
            Str::new("x")
                .try_with_parameters(StringParameters::utf8(StringLayout::FixedString))
                .is_err()
        );
    }

    #[test]
    fn bytes_are_read_strictly_or_transcribed_by_their_charset() {
        let latin = StringParameters::new(StringLayout::String, Charset::Cp1252);
        let value = Str::from_bytes(b"Gr\xFC\xDFe", latin).unwrap();
        assert_eq!(value, "Grüße");
        assert_eq!(value.charset(), Charset::Cp1252);
        assert_eq!(value.encode().unwrap().as_ref(), b"Gr\xFC\xDFe");
        // `0x81` is unassigned in windows-1252 and still reads.
        assert_eq!(Str::from_bytes(b"ok\x81", latin).unwrap(), "ok\u{0081}");
        // UTF-8 and US-ASCII are validated, not transcribed.
        assert!(Str::from_bytes(b"caf\xe9", StringParameters::default()).is_err());
        assert!(
            Str::from_bytes(
                b"caf\xc3\xa9",
                StringParameters::ascii(StringLayout::String)
            )
            .is_err()
        );
        assert_eq!(
            Str::from_bytes(b"caf\xc3\xa9", StringParameters::default()).unwrap(),
            "café"
        );
        // A padded slot comes back trimmed and carries its width.
        let slot = StringParameters::utf8(StringLayout::FixedString)
            .try_with_bound(6)
            .unwrap();
        let padded = Str::from_bytes(b"ab\0\0\0\0", slot).unwrap();
        assert_eq!(padded, "ab");
        assert_eq!(padded.fixed(), Some(6));
    }

    #[test]
    fn serde_writes_the_text_alone_unless_the_value_declares_more() {
        let plain = Str::new("plain");
        assert_eq!(serde_json::to_string(&plain).unwrap(), "\"plain\"");
        assert_eq!(serde_json::from_str::<Str>("\"plain\"").unwrap(), plain);
        let latin = Str::new("Grüße")
            .try_with_parameters(
                StringParameters::new(StringLayout::FixedString, Charset::Cp1252)
                    .try_with_bound(8)
                    .unwrap(),
            )
            .unwrap();
        let document = serde_json::to_string(&latin).unwrap();
        assert_eq!(
            document,
            r#"{"layout":"fixed_string","charset":"windows-1252","fixed":8,"text":"Grüße"}"#
        );
        let back = serde_json::from_str::<Str>(&document).unwrap();
        assert_eq!(back.parameters(), latin.parameters());
        assert_eq!(back, latin);
        // A maximum is never part of a value, so it never reaches the wire.
        let bounded = Str::new("x")
            .try_with_parameters(
                StringParameters::utf8(StringLayout::LargeString)
                    .try_with_bound(8)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            serde_json::to_string(&bounded).unwrap(),
            r#"{"layout":"large_string","charset":"utf-8","text":"x"}"#
        );
    }

    #[test]
    fn a_string_scalar_names_its_own_datatype() {
        assert_eq!(Scalar::from("x").as_str(), Some("x"));
        assert_eq!(Scalar::from("x").dtype().unwrap(), DataType::utf8());
        let latin = Str::new("x")
            .try_with_parameters(StringParameters::new(
                StringLayout::StringView,
                Charset::Latin1,
            ))
            .unwrap();
        assert_eq!(
            Scalar::String(latin).dtype().unwrap(),
            DataType::from_str("string_view(iso-8859-1)").unwrap()
        );
    }
}
