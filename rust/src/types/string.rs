//! The string datatype family: one layout, one charset, one length bound.
//!
//! Arrow has three string layouts and no way to say what a string is *in*: a
//! `Utf8` array declares UTF-8 and nothing else declares anything, and there
//! is no `varchar(n)` or `char(n)` anywhere in the format. This family is the
//! one place this crate answers all three questions - which layout, which
//! charset, how long - and every string the crate has is one member of it.
//!
//! [`StringLayout`] names the five layouts. [`StringParameters`] is a layout
//! beside the charset its bytes are written in and the bound its values are
//! held to. [`crate::DataType::string`] builds the one string datatype,
//! [`crate::DataType::String`], from them; `utf8`, `ascii`, `varchar(32)`
//! and `char(8)` are spellings of it, never datatypes of their own.
//! [`Str`] is the one string value, and the eight registered codes beside it
//! are identities with a storage rather than strings with a charset.
//!
//! ```
//! use yggdryl::types::{StringLayout, StringParameters};
//! use yggdryl::{Charset, DataType};
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every spelling is one datatype, and it reads back as itself.
//! assert_eq!(DataType::from_str("string")?, DataType::utf8());
//! assert_eq!(DataType::from_str("largestringview")?.to_string(), "large_utf8_view");
//! assert_eq!(DataType::from_str("fixed_string(us-ascii,4)")?.to_string(), "fixed_ascii(4)");
//!
//! // The layout, the charset and the bound are what a string declares.
//! let latin = DataType::from_str("string(windows-1252,32)")?;
//! let parameters = latin.string_parameters().expect("a string datatype");
//! assert_eq!(parameters.layout(), StringLayout::String);
//! assert_eq!(parameters.charset(), Charset::Cp1252);
//! assert_eq!(parameters.max(), Some(32));
//! assert_eq!(latin.to_string(), "string(windows-1252,32)");
//! # Ok(())
//! # }
//! ```

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::{DataTypeId, Error, Result};

mod arrow;
#[cfg(feature = "arrow")]
pub(crate) mod casts;
/// The eight registered codes' values.
mod code;
mod codes;
mod dictionary;
mod dtypes;
mod fields;
mod parameters;
mod parser;
mod registries;
mod scalars;

pub(crate) use arrow::{arrow_storage, describes_storage, is_text_storage, needs_extension};
pub use code::{Cfi, Code, CodeValue, Country, Currency, Isin, Mic, Side, State, TimeInForce};
// The padding is the payload a declared width stores, which a value answers
// with or without an Arrow array around it.
pub(crate) use codes::ascii_padded;
pub(crate) use codes::{
    CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, ISIN_WIDTH, MIC_WIDTH, SIDE_WIDTH, STATE_WIDTH,
    TIMEINFORCE_WIDTH, ascii_bytes, ascii_text, code_cell_text, code_extension_name,
    code_for_extension,
};
#[cfg(feature = "arrow")]
pub(crate) use codes::{code_refusal, code_text};
pub use dictionary::StringEnum;
pub use fields::*;
pub use parameters::{STRING_EXTENSION_NAME, StringParameters};
pub(crate) use scalars::str_from_value;
pub use scalars::{INLINE_CAPACITY, Str};

/// One of the five ways this crate lays a string out.
///
/// The layout is the physical shape alone, how a value's bytes are addressed.
/// It says nothing about what the bytes mean; that is the charset beside it in
/// [`StringParameters`].
///
/// Each layout has three spellings, and they are one datatype: the `string`
/// name is the general one, the `utf8` name is what the same layout is called
/// when its charset is UTF-8, which is the default, and the `ascii` name is
/// what it is called when its charset is US-ASCII. So `large_string`,
/// `large_utf8` and `large_ascii` name one layout, and a value renders under
/// whichever name its charset earns.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum StringLayout {
    /// Variable width, 32-bit offsets - Arrow's `Utf8` and `Binary`.
    #[default]
    String,
    /// One fixed byte width every value fills, padded with trailing NUL.
    ///
    /// Arrow has no fixed-width string at all, so this rides its
    /// `FixedSizeBinary` and the width travels in this crate's own metadata.
    FixedString,
    /// The view layout: a short prefix inline, the rest out of line.
    StringView,
    /// Variable width, 64-bit offsets - Arrow's `LargeUtf8` and `LargeBinary`.
    LargeString,
    /// The view layout, declared large.
    ///
    /// Arrow has one view layout and no large form of it, so this projects as
    /// that one view and the `large` declaration travels in this crate's own
    /// metadata. Nothing about the buffers differs; what differs is what the
    /// column promises about the offsets a writer may emit.
    LargeStringView,
}

/// Strip the padding a fixed-width storage writes.
///
/// Trailing NUL is that padding wherever this crate lays a value out in a
/// slot wider than itself - a [`StringLayout::FixedString`] cell or a
/// registered code - so the rule lives here once rather than at each reader.
pub(crate) fn trim_padding(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map_or(0, |last| last + 1);
    &bytes[..end]
}

impl StringLayout {
    /// Every layout in canonical declaration order.
    pub const ALL: [Self; 5] = [
        Self::String,
        Self::FixedString,
        Self::StringView,
        Self::LargeString,
        Self::LargeStringView,
    ];

    /// The canonical name of this layout in any charset.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::FixedString => "fixed_string",
            Self::StringView => "string_view",
            Self::LargeString => "large_string",
            Self::LargeStringView => "large_string_view",
        }
    }

    /// The canonical name of this layout when its charset is UTF-8.
    #[must_use]
    pub const fn as_utf8_str(self) -> &'static str {
        match self {
            Self::String => "utf8",
            Self::FixedString => "fixed_utf8",
            Self::StringView => "utf8_view",
            Self::LargeString => "large_utf8",
            Self::LargeStringView => "large_utf8_view",
        }
    }

    /// The canonical name of this layout when its charset is US-ASCII.
    #[must_use]
    pub const fn as_ascii_str(self) -> &'static str {
        match self {
            Self::String => "ascii",
            Self::FixedString => "fixed_ascii",
            Self::StringView => "ascii_view",
            Self::LargeString => "large_ascii",
            Self::LargeStringView => "large_ascii_view",
        }
    }

    /// Resolve a layout from its general spelling or its UTF-8 one.
    ///
    /// Case, underscores, hyphens and spaces are all ignored, so
    /// `LARGE_STRING`, `large-string` and `largestring` are one layout, and
    /// so are `large_utf8` and `largeutf8` - UTF-8 is the charset a layout
    /// has when nothing is declared, so that spelling names no more than the
    /// layout. The US-ASCII spellings do name more, and a layout alone would
    /// drop it, so they are refused here and read only where the charset
    /// travels with them: [`crate::DataType::from_str`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownDataType`] for a name no layout answers to,
    /// and for a US-ASCII spelling, naming the charset it would lose.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        if let Some(layout) = Self::ALL
            .into_iter()
            .find(|layout| super::parser::folds_equal(value, layout.as_ascii_str()))
        {
            return Err(Error::UnknownDataType(format_smolstr!(
                "{value} names the {} layout in the us-ascii charset, not a layout alone",
                layout.as_str()
            )));
        }
        Self::ALL
            .into_iter()
            .find(|layout| {
                super::parser::folds_equal(value, layout.as_str())
                    || super::parser::folds_equal(value, layout.as_utf8_str())
            })
            .ok_or_else(|| Error::UnknownDataType(format_smolstr!("{value}")))
    }

    /// The identifier naming this layout.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::String => DataTypeId::String,
            Self::FixedString => DataTypeId::FixedString,
            Self::StringView => DataTypeId::StringView,
            Self::LargeString => DataTypeId::LargeString,
            Self::LargeStringView => DataTypeId::LargeStringView,
        }
    }

    /// The layout one identifier names, `None` for an identifier that is not
    /// a string's.
    #[must_use]
    pub const fn from_id(id: DataTypeId) -> Option<Self> {
        match id {
            DataTypeId::String => Some(Self::String),
            DataTypeId::FixedString => Some(Self::FixedString),
            DataTypeId::StringView => Some(Self::StringView),
            DataTypeId::LargeString => Some(Self::LargeString),
            DataTypeId::LargeStringView => Some(Self::LargeStringView),
            _ => None,
        }
    }

    /// Return whether every value in this layout is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Self::FixedString)
    }

    /// Return whether this layout addresses its bytes through a view.
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(self, Self::StringView | Self::LargeStringView)
    }

    /// Return whether this layout declares 64-bit offsets.
    #[must_use]
    pub const fn is_large(self) -> bool {
        matches!(self, Self::LargeString | Self::LargeStringView)
    }
}

impl fmt::Display for StringLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for StringLayout {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StringLayout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}
