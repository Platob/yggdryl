//! The one door into the string family, and the one way back out.

use smol_str::format_smolstr;

use super::{StringLayout, StringParameters};
use crate::{Charset, DataType, Error, Result};

impl DataType {
    /// The string datatype these parameters name.
    ///
    /// This is the family's one constructor, and it *redirects*: a string
    /// that another datatype already spells comes back as that datatype
    /// rather than as a second spelling of it.
    ///
    /// | parameters | datatype |
    /// | --- | --- |
    /// | unbounded UTF-8, `string` | [`DataType::Utf8`] |
    /// | unbounded UTF-8, `large_string` | [`DataType::LargeUtf8`] |
    /// | unbounded UTF-8, `string_view` | [`DataType::Utf8View`] |
    /// | unbounded US-ASCII, `string` | [`DataType::Ascii`] |
    /// | US-ASCII, `fixed_string(n)` | [`DataType::FixedAscii`] |
    /// | anything else | [`DataType::String`] |
    ///
    /// ```
    /// use yggdryl::types::{StringLayout, StringParameters};
    /// use yggdryl::{Charset, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // Plain UTF-8 is the datatype it already was.
    /// let plain = StringParameters::utf8(StringLayout::LargeString);
    /// assert_eq!(DataType::string(plain)?, DataType::LargeUtf8);
    ///
    /// // A charset or a bound is what makes it its own.
    /// let bounded = StringParameters::utf8(StringLayout::String).try_with_bound(32)?;
    /// assert_eq!(DataType::string(bounded)?.to_string(), "utf8(32)");
    /// assert_eq!(DataType::string(Charset::Cp1252)?.to_string(), "string(windows-1252)");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] for a fixed layout with no width,
    /// and for US-ASCII in a layout the [`DataType::Ascii`] family has no
    /// shape for - a view or a maximum - because US-ASCII text is that
    /// family's, and one fact has one owner.
    pub fn string(parameters: impl Into<StringParameters>) -> Result<Self> {
        let parameters = parameters.into();
        parameters.validate()?;
        if parameters.charset() == Charset::Ascii {
            return ascii_redirect(parameters);
        }
        if parameters.charset().is_utf8() && parameters.is_plain() {
            if let Some(plain) = plain_utf8(parameters.layout()) {
                return Ok(plain);
            }
        }
        Ok(Self::String(parameters))
    }

    /// The parameters every string datatype declares, `None` for the rest.
    ///
    /// This is where the family is one family: [`DataType::Utf8`] and its two
    /// siblings answer their layout in UTF-8, the two ASCII datatypes answer
    /// theirs in US-ASCII, and [`DataType::String`] answers what it carries.
    /// So a reader asking which charset a column is in, or how long its
    /// values may be, asks one question of every string column there is.
    ///
    /// The registered codes are deliberately not here. A currency is three
    /// ASCII bytes the way a UUID is sixteen binary ones - an identity with a
    /// storage, not a string with a charset - and answering for it would
    /// invite a cast that reads it as text.
    ///
    /// ```
    /// use yggdryl::types::StringLayout;
    /// use yggdryl::{Charset, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let utf8 = DataType::Utf8.string_parameters().expect("a string datatype");
    /// assert_eq!(utf8.layout(), StringLayout::String);
    /// assert_eq!(utf8.charset(), Charset::Utf8);
    ///
    /// let ascii = DataType::ascii(3)?.string_parameters().expect("a string datatype");
    /// assert_eq!(ascii.charset(), Charset::Ascii);
    /// assert_eq!(ascii.fixed(), Some(3));
    ///
    /// assert!(DataType::Currency.string_parameters().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn string_parameters(&self) -> Option<StringParameters> {
        match self {
            Self::String(parameters) => Some(*parameters),
            Self::Utf8 => Some(StringParameters::utf8(StringLayout::String)),
            Self::LargeUtf8 => Some(StringParameters::utf8(StringLayout::LargeString)),
            Self::Utf8View => Some(StringParameters::utf8(StringLayout::StringView)),
            Self::Ascii => Some(StringParameters::new(StringLayout::String, Charset::Ascii)),
            Self::FixedAscii(width) => u32::try_from(*width).ok().and_then(|width| {
                StringParameters::new(StringLayout::FixedString, Charset::Ascii)
                    .try_with_bound(width)
                    .ok()
            }),
            _ => None,
        }
    }

    /// Return whether this datatype is one of the strings.
    #[must_use]
    pub fn is_string(&self) -> bool {
        self.string_parameters().is_some()
    }

    /// The charset a string column's bytes are written in.
    ///
    /// `None` is a datatype that is not a string; every string has one,
    /// because UTF-8 is what a string with nothing declared is in.
    #[must_use]
    pub fn charset(&self) -> Option<Charset> {
        self.string_parameters().map(StringParameters::charset)
    }
}

/// The plain UTF-8 datatype one layout already has, if it has one.
const fn plain_utf8(layout: StringLayout) -> Option<DataType> {
    match layout {
        StringLayout::String => Some(DataType::Utf8),
        StringLayout::LargeString => Some(DataType::LargeUtf8),
        StringLayout::StringView => Some(DataType::Utf8View),
        // Arrow has no fixed string and no large view, so neither has a
        // plain spelling to redirect to.
        StringLayout::FixedString | StringLayout::LargeStringView => None,
    }
}

/// The ASCII datatype US-ASCII parameters name, or the refusal.
fn ascii_redirect(parameters: StringParameters) -> Result<DataType> {
    match (parameters.layout(), parameters.bound()) {
        (StringLayout::String, None) => Ok(DataType::Ascii),
        (StringLayout::FixedString, Some(width)) => {
            DataType::ascii(i32::try_from(width).map_err(|_| {
                invalid(format_smolstr!(
                    "ASCII width {width} is outside the i32 range"
                ))
            })?)
        }
        _ => Err(invalid(format_smolstr!(
            "expected ascii or ascii(width) for US-ASCII text, got {parameters}"
        ))),
    }
}

/// The refusal an unbuildable string answers with.
fn invalid(reason: smol_str::SmolStr) -> Error {
    Error::InvalidDataType {
        kind: "string",
        reason,
    }
}
