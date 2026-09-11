//! String values: the text, the charset it is stored in, and its layout.
//!
//! A string value holds UTF-8 whatever charset it arrived in. That is the
//! whole point of decoding at the seam: the bytes are read once, at the
//! boundary that knows the charset, and everything above it reads characters.
//! What the value keeps is the charset it is *written* in, so the same value
//! goes back out the way it came without the column being consulted twice.
//!
//! The decode is permissive by default - [`Charset::transcribe`] reads every
//! byte it can rather than refusing a payload or peppering it with `U+FFFD` -
//! because a legacy export with one bad byte in a million is a file that
//! still has to be read.

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{StringLayout, StringParameters, trim_padding};
use crate::types::Scalar;
use crate::{Charset, DataType, DataTypeId, DataTypeKind, Result, ScalarFamily, ScalarValue};

/// Borrowing access shared by every string representation.
pub trait TextValue: crate::ScalarValue {
    /// The layout this representation stores its values in.
    const LAYOUT: StringLayout;

    /// Borrow the Unicode text.
    fn as_str(&self) -> &str;
    /// Borrow the shared storage behind the text.
    ///
    /// Every string representation stores the same [`SmolStr`], so one
    /// representation adopts another's storage by cloning this handle rather
    /// than copying the bytes.
    fn storage(&self) -> &SmolStr;
    /// The charset this value's bytes are written in.
    fn charset(&self) -> Charset;
}

/// Whether a charset is the one a string is written in by default.
///
/// UTF-8 is that charset, so the ordinary value serializes its characters
/// and nothing else, exactly as it did before strings could declare one.
fn is_default_charset(charset: &Charset) -> bool {
    charset.is_utf8()
}

/// A charset a string value may carry.
///
/// US-ASCII is a string family of its own - [`DataType::Ascii`] and the codes
/// above it - and its repertoire is a subset of UTF-8's, so a string value
/// asked to hold it holds UTF-8 instead. That keeps one fact in one place:
/// every value whose datatype is US-ASCII is an [`crate::types::AsciiFamily`]
/// value, and no string value ever answers a datatype from that family.
const fn stored_charset(charset: Charset) -> Charset {
    match charset {
        Charset::Ascii => Charset::Utf8,
        other => other,
    }
}

macro_rules! string_leaf {
    ($name:ident, $layout:ident, $plain:ident) => {
        #[doc = concat!("One `", stringify!($name), "` string value.")]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        pub struct $name {
            text: SmolStr,
            /// UTF-8 is what a string with nothing declared is written in, so
            /// the ordinary value writes its characters and nothing else.
            #[serde(default, skip_serializing_if = "is_default_charset")]
            charset: Charset,
        }

        impl $name {
            /// Construct this representation from UTF-8 text.
            pub fn new(value: impl Into<SmolStr>) -> Self {
                Self {
                    text: value.into(),
                    charset: Charset::Utf8,
                }
            }

            /// Read bytes in one charset, transcribing what it cannot read.
            ///
            /// This is the permissive door [`Charset::transcribe`] opens: a
            /// byte the charset leaves unassigned reads as the scalar ISO
            /// 8859-1 gives it, and bytes offered as UTF-8 that are not UTF-8
            /// read as ISO 8859-1 rather than as replacement characters. A
            /// payload already US-ASCII costs a borrow, not a transcode.
            pub fn from_bytes(value: &[u8], charset: Charset) -> Self {
                Self {
                    text: charset.transcribe_smol(value),
                    charset: stored_charset(charset),
                }
            }

            /// Borrow the Unicode text.
            pub fn as_str(&self) -> &str {
                self.text.as_str()
            }

            /// Borrow the shared storage without copying the text.
            pub fn storage(&self) -> &SmolStr {
                &self.text
            }

            /// The charset this value's bytes are written in.
            pub const fn charset(&self) -> Charset {
                self.charset
            }

            /// Return this value written in another charset.
            ///
            /// The characters do not change; what changes is the bytes
            /// [`Self::encode`] answers and the datatype [`Self::dtype`]
            /// declares.
            #[must_use]
            pub fn with_charset(mut self, charset: Charset) -> Self {
                self.charset = stored_charset(charset);
                self
            }

            /// The stored bytes of this value, in the charset it declares.
            ///
            /// UTF-8 text borrows, and so does any all-ASCII value in any of
            /// the ASCII-compatible charsets, so the ordinary column costs
            /// nothing to write back out.
            ///
            /// # Errors
            ///
            /// Returns [`crate::Error::Codec`] naming the first scalar the
            /// charset has no byte for.
            pub fn encode(&self) -> Result<Cow<'_, [u8]>> {
                self.charset.encode(self.as_str())
            }

            /// Consume this value and return its compact string.
            pub fn into_inner(self) -> SmolStr {
                self.text
            }

            /// The identifier this value carries.
            const fn identifier(&self) -> DataTypeId {
                match self.charset.is_utf8() {
                    true => DataTypeId::$plain,
                    false => StringLayout::$layout.id(),
                }
            }

            /// The parameters this value's datatype declares.
            const fn parameters(&self) -> StringParameters {
                StringParameters::new(StringLayout::$layout, self.charset)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<SmolStr> for $name {
            fn from(value: SmolStr) -> Self {
                Self::new(value)
            }
        }
    };
}

string_leaf!(Utf8, String, Utf8);
string_leaf!(Utf8View, StringView, Utf8View);
string_leaf!(LargeUtf8, LargeString, LargeUtf8);
// Arrow has one view layout, so the large view has no plain spelling of its
// own and reads as itself in every charset, UTF-8 included.
string_leaf!(LargeUtf8View, LargeStringView, LargeStringView);

/// One string value carrying its fixed padded storage width.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FixedUtf8 {
    text: SmolStr,
    #[serde(default, skip_serializing_if = "is_default_charset")]
    charset: Charset,
    width: u32,
}

impl FixedUtf8 {
    /// Construct fixed-width text of exactly `width` stored bytes.
    ///
    /// The text is the value without its padding; the width is how many bytes
    /// the storage holds. Trailing NUL is the padding, exactly as
    /// [`crate::types::FixedAscii`] spells it.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero, and
    /// [`crate::Error::InvalidRecord`] when the text does not fit the width
    /// in its charset.
    pub fn new(value: impl Into<SmolStr>, width: u32, charset: Charset) -> Result<Self> {
        let charset = stored_charset(charset);
        let text = value.into();
        let trimmed = text.trim_end_matches('\0');
        // The stored length is a property of the text, so it is counted
        // rather than built: nothing is encoded until the value is written.
        let stored = u32::try_from(charset.encoded_len(trimmed)).unwrap_or(u32::MAX);
        if width == 0 {
            return Err(crate::Error::InvalidDataType {
                kind: "string",
                reason: smol_str::SmolStr::new_static(
                    "expected a width of at least one byte, got 0",
                ),
            });
        }
        if stored > width {
            return Err(crate::Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    format_args!("at most {width} bytes of {charset}"),
                    smol_str::format_smolstr!("{stored}"),
                ),
            });
        }
        let text = match trimmed.len() == text.len() {
            true => text,
            false => SmolStr::new(trimmed),
        };
        Ok(Self {
            text,
            charset,
            width,
        })
    }

    /// Read fixed-width bytes in one charset, transcribing what it cannot read.
    ///
    /// The padding is the trailing NUL the storage writes, so it is trimmed
    /// here rather than left for every reader to trim again.
    pub fn from_bytes(value: &[u8], charset: Charset) -> Self {
        let width = u32::try_from(value.len()).unwrap_or(u32::MAX);
        let trimmed = trim_padding(value);
        Self {
            text: charset.transcribe_smol(trimmed),
            charset: stored_charset(charset),
            width: width.max(1),
        }
    }

    /// Borrow the Unicode text, without its padding.
    pub fn as_str(&self) -> &str {
        self.text.as_str()
    }

    /// Borrow the shared storage without copying the text.
    pub fn storage(&self) -> &SmolStr {
        &self.text
    }

    /// The charset this value's bytes are written in.
    pub const fn charset(&self) -> Charset {
        self.charset
    }

    /// The padded storage width, in bytes.
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// The stored bytes of this value, padded to its declared width.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Codec`] naming the first scalar the charset
    /// has no byte for.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let encoded = self.charset.encode(self.as_str())?;
        let mut padded = encoded.into_owned();
        padded.resize(self.width as usize, 0);
        Ok(padded)
    }

    /// Consume this value and return its compact string.
    pub fn into_inner(self) -> SmolStr {
        self.text
    }

    /// The parameters this value's datatype declares.
    fn parameters(&self) -> Result<StringParameters> {
        StringParameters::new(StringLayout::FixedString, self.charset).try_with_bound(self.width)
    }
}

impl Default for FixedUtf8 {
    fn default() -> Self {
        Self {
            text: SmolStr::default(),
            charset: Charset::Utf8,
            width: 1,
        }
    }
}

impl fmt::Display for FixedUtf8 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One exact string storage representation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Text {
    /// Variable width, 32-bit offsets.
    Utf8(Utf8),
    /// One fixed padded byte width.
    FixedUtf8(FixedUtf8),
    /// The view layout.
    Utf8View(Utf8View),
    /// Variable width, 64-bit offsets.
    LargeUtf8(LargeUtf8),
    /// The view layout, declared large.
    LargeUtf8View(LargeUtf8View),
}

impl Text {
    /// Borrow the Unicode text independently of its storage layout.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Utf8(value) => value.as_str(),
            Self::FixedUtf8(value) => value.as_str(),
            Self::Utf8View(value) => value.as_str(),
            Self::LargeUtf8(value) => value.as_str(),
            Self::LargeUtf8View(value) => value.as_str(),
        }
    }

    /// Borrow the shared storage independently of its layout.
    ///
    /// The layout is the offset width, not the bytes, so rewriting a value
    /// into another layout clones this handle instead of the text.
    pub fn storage(&self) -> &SmolStr {
        match self {
            Self::Utf8(value) => value.storage(),
            Self::FixedUtf8(value) => value.storage(),
            Self::Utf8View(value) => value.storage(),
            Self::LargeUtf8(value) => value.storage(),
            Self::LargeUtf8View(value) => value.storage(),
        }
    }

    /// The charset this value's bytes are written in.
    pub const fn charset(&self) -> Charset {
        match self {
            Self::Utf8(value) => value.charset(),
            Self::FixedUtf8(value) => value.charset(),
            Self::Utf8View(value) => value.charset(),
            Self::LargeUtf8(value) => value.charset(),
            Self::LargeUtf8View(value) => value.charset(),
        }
    }

    /// The layout this value is stored in.
    pub const fn layout(&self) -> StringLayout {
        match self {
            Self::Utf8(_) => StringLayout::String,
            Self::FixedUtf8(_) => StringLayout::FixedString,
            Self::Utf8View(_) => StringLayout::StringView,
            Self::LargeUtf8(_) => StringLayout::LargeString,
            Self::LargeUtf8View(_) => StringLayout::LargeStringView,
        }
    }

    /// The padded storage width, on the fixed layout alone.
    pub const fn width(&self) -> Option<u32> {
        match self {
            Self::FixedUtf8(value) => Some(value.width()),
            _ => None,
        }
    }

    /// The identifier this value carries.
    pub const fn identifier(&self) -> DataTypeId {
        match self {
            Self::Utf8(value) => value.identifier(),
            Self::FixedUtf8(_) => DataTypeId::FixedString,
            Self::Utf8View(value) => value.identifier(),
            Self::LargeUtf8(value) => value.identifier(),
            Self::LargeUtf8View(value) => value.identifier(),
        }
    }

    /// The stored bytes of this value, in the charset it declares.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::Codec`] naming the first scalar the charset
    /// has no byte for.
    pub fn encode(&self) -> Result<Cow<'_, [u8]>> {
        match self {
            Self::Utf8(value) => value.encode(),
            Self::FixedUtf8(value) => value.encode().map(Cow::Owned),
            Self::Utf8View(value) => value.encode(),
            Self::LargeUtf8(value) => value.encode(),
            Self::LargeUtf8View(value) => value.encode(),
        }
    }

    /// Read bytes into one layout and charset, transcribing what it cannot read.
    pub fn from_bytes(value: &[u8], parameters: StringParameters) -> Self {
        let charset = parameters.charset();
        match parameters.layout() {
            StringLayout::String => Self::Utf8(Utf8::from_bytes(value, charset)),
            StringLayout::FixedString => Self::FixedUtf8(FixedUtf8::from_bytes(value, charset)),
            StringLayout::StringView => Self::Utf8View(Utf8View::from_bytes(value, charset)),
            StringLayout::LargeString => Self::LargeUtf8(LargeUtf8::from_bytes(value, charset)),
            StringLayout::LargeStringView => {
                Self::LargeUtf8View(LargeUtf8View::from_bytes(value, charset))
            }
        }
    }

    /// Restate this text in another layout and charset, sharing its storage.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidRecord`] when the text does not fit a
    /// declared fixed width.
    pub fn restated(&self, parameters: StringParameters) -> Result<Self> {
        let text = self.storage().clone();
        let charset = parameters.charset();
        Ok(match parameters.layout() {
            StringLayout::String => Self::Utf8(Utf8::new(text).with_charset(charset)),
            StringLayout::FixedString => Self::FixedUtf8(FixedUtf8::new(
                text,
                parameters.bound().unwrap_or(1),
                charset,
            )?),
            StringLayout::StringView => Self::Utf8View(Utf8View::new(text).with_charset(charset)),
            StringLayout::LargeString => {
                Self::LargeUtf8(LargeUtf8::new(text).with_charset(charset))
            }
            StringLayout::LargeStringView => {
                Self::LargeUtf8View(LargeUtf8View::new(text).with_charset(charset))
            }
        })
    }
}

impl fmt::Display for Text {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq for Text {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Text {}

impl PartialOrd for Text {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Text {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for Text {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

const _: () = assert!(std::mem::size_of::<Text>() == 40);

macro_rules! string_value {
    ($leaf:ident, $variant:ident, $plain:ident, $layout:ident) => {
        impl ScalarValue for $leaf {
            type Family = Text;

            const ID: DataTypeId = DataTypeId::$plain;
            const KIND: DataTypeKind = DataTypeKind::Text;

            fn dtype(&self) -> Result<DataType> {
                DataType::string(self.parameters())
            }

            fn into_family(self) -> Self::Family {
                Text::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Text::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Text(Text::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Text(Text::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl TextValue for $leaf {
            const LAYOUT: StringLayout = StringLayout::$layout;

            fn as_str(&self) -> &str {
                <$leaf>::as_str(self)
            }

            fn storage(&self) -> &SmolStr {
                <$leaf>::storage(self)
            }

            fn charset(&self) -> Charset {
                <$leaf>::charset(self)
            }
        }
    };
}

string_value!(Utf8, Utf8, Utf8, String);
string_value!(Utf8View, Utf8View, Utf8View, StringView);
string_value!(LargeUtf8, LargeUtf8, LargeUtf8, LargeString);
string_value!(
    LargeUtf8View,
    LargeUtf8View,
    LargeStringView,
    LargeStringView
);

impl ScalarValue for FixedUtf8 {
    type Family = Text;

    const ID: DataTypeId = DataTypeId::FixedString;
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn dtype(&self) -> Result<DataType> {
        DataType::string(self.parameters()?)
    }

    fn into_family(self) -> Self::Family {
        Text::FixedUtf8(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            Text::FixedUtf8(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Text(Text::FixedUtf8(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Text(Text::FixedUtf8(value)) => Some(value),
            _ => None,
        }
    }
}

impl TextValue for FixedUtf8 {
    const LAYOUT: StringLayout = StringLayout::FixedString;

    fn as_str(&self) -> &str {
        Self::as_str(self)
    }

    fn storage(&self) -> &SmolStr {
        Self::storage(self)
    }

    fn charset(&self) -> Charset {
        Self::charset(self)
    }
}

impl ScalarFamily for Text {
    const KIND: DataTypeKind = DataTypeKind::Text;

    fn id(&self) -> DataTypeId {
        self.identifier()
    }

    fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Utf8(value) => value.dtype(),
            Self::FixedUtf8(value) => value.dtype(),
            Self::Utf8View(value) => value.dtype(),
            Self::LargeUtf8(value) => value.dtype(),
            Self::LargeUtf8View(value) => value.dtype(),
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Text(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Text(value) => Some(value),
            _ => None,
        }
    }
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Self::Text(Text::Utf8(Utf8::new(value)))
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Self {
        Self::Text(Text::Utf8(Utf8::new(value)))
    }
}

impl From<SmolStr> for Scalar {
    fn from(value: SmolStr) -> Self {
        Self::Text(Text::Utf8(Utf8::new(value)))
    }
}

/// The canonical text a value spells, shared rather than rebuilt.
///
/// A string column stores one string per row, and this is the spelling every
/// tier prints: the canonical [`std::fmt::Display`] each family owns for the
/// numbers and the boolean, [`Scalar::into_temporal_text`] for a temporal -
/// which is what a temporal *column* renders too, zone rules included - the
/// WKT a geometry column renders, and the payload's own characters for bytes.
/// Text, ASCII and a generic enum member already hold their storage and hand
/// it over without allocating.
///
/// `None` is a kind that spells no text at all; `Some(Err)` is a payload that
/// was read and refused, so the refusal names what was wrong with it rather
/// than which kind arrived.
pub(crate) fn text_from_value(value: &Scalar) -> Option<Result<SmolStr>> {
    Some(match value {
        Scalar::Text(text) => Ok(text.storage().clone()),
        Scalar::Ascii(ascii) => Ok(ascii.storage().clone()),
        Scalar::Enum(member) => Ok(SmolStr::new_static(member.as_str())),
        Scalar::Integer(number) => Ok(smol_str::format_smolstr!("{number}")),
        Scalar::Floating(number) => Ok(smol_str::format_smolstr!("{number}")),
        Scalar::Decimal(number) => Ok(smol_str::format_smolstr!("{number}")),
        Scalar::Boolean(flag) => Ok(smol_str::format_smolstr!("{flag}")),
        Scalar::Uuid(uuid) => Ok(smol_str::format_smolstr!("{uuid}")),
        Scalar::Version(version) => Ok(smol_str::format_smolstr!("{version}")),
        Scalar::Url(url) => Ok(smol_str::format_smolstr!("{url}")),
        // An interval has no classic spelling, so a temporal answers for the
        // seven that do and leaves the rest to the ordinary refusal.
        Scalar::Temporal(_) => return value.into_temporal_text().map(Ok),
        Scalar::Bytes(bytes) => std::str::from_utf8(bytes.as_bytes())
            .map(SmolStr::new)
            .map_err(|error| crate::Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: smol_str::format_smolstr!("payload is not UTF-8: {error}"),
            }),
        Scalar::Geospatial(geospatial) => {
            crate::types::geospatial::wkb::into_wkt(geospatial.as_bytes()).map(SmolStr::new)
        }
        _ => return None,
    })
}

/// The serde representation of one string value.
///
/// The common case - UTF-8 in one of the four variable layouts - writes the
/// text alone under the layout's own tag, exactly as it always has. A width
/// or a charset is what makes a value carry more than its characters.
#[derive(Deserialize, Serialize)]
pub(crate) struct TextRepresentation {
    pub(crate) layout: StringLayout,
    pub(crate) charset: Charset,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) width: Option<u32>,
    pub(crate) text: SmolStr,
}

impl From<&Text> for TextRepresentation {
    fn from(value: &Text) -> Self {
        Self {
            layout: value.layout(),
            charset: value.charset(),
            width: value.width(),
            text: value.storage().clone(),
        }
    }
}

impl TryFrom<TextRepresentation> for Text {
    type Error = crate::Error;

    fn try_from(value: TextRepresentation) -> Result<Self> {
        let mut parameters = StringParameters::new(value.layout, value.charset);
        if let Some(width) = value.width {
            parameters = parameters.try_with_bound(width)?;
        }
        Self::Utf8(Utf8::new(value.text)).restated(parameters)
    }
}
