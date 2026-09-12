//! The one door into the string family, and the questions every string answers.

use super::{StringLayout, StringParameters};
use crate::{Charset, DataType, Result};

impl DataType {
    /// The string datatype these parameters name.
    ///
    /// This is the family's one constructor. Every string is
    /// [`DataType::String`]; what differs is what it declares, and the
    /// parameters say all of it - a layout, a charset, and a bound.
    ///
    /// ```
    /// use yggdryl::types::{StringLayout, StringParameters};
    /// use yggdryl::{Charset, DataType};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// // Plain UTF-8 renders under Arrow's own name.
    /// let plain = StringParameters::utf8(StringLayout::LargeString);
    /// assert_eq!(DataType::string(plain)?, DataType::large_utf8());
    /// assert_eq!(DataType::large_utf8().to_string(), "large_utf8");
    ///
    /// // A charset or a bound is what a string declares.
    /// let bounded = StringParameters::utf8(StringLayout::String).try_with_bound(32)?;
    /// assert_eq!(DataType::string(bounded)?.to_string(), "utf8(32)");
    /// assert_eq!(DataType::string(Charset::Cp1252)?.to_string(), "string(windows-1252)");
    /// assert_eq!(DataType::fixed_ascii(4)?.to_string(), "fixed_ascii(4)");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a fixed layout with no width:
    /// the width is what makes it fixed.
    pub fn string(parameters: impl Into<StringParameters>) -> Result<Self> {
        let parameters = parameters.into();
        parameters.validate()?;
        Ok(Self::String(parameters))
    }

    /// Unbounded UTF-8 with 32-bit offsets - Arrow's `Utf8`.
    #[must_use]
    pub const fn utf8() -> Self {
        Self::String(StringParameters::utf8(StringLayout::String))
    }

    /// Unbounded UTF-8 with 64-bit offsets - Arrow's `LargeUtf8`.
    #[must_use]
    pub const fn large_utf8() -> Self {
        Self::String(StringParameters::utf8(StringLayout::LargeString))
    }

    /// Unbounded UTF-8 in the view layout - Arrow's `Utf8View`.
    #[must_use]
    pub const fn utf8_view() -> Self {
        Self::String(StringParameters::utf8(StringLayout::StringView))
    }

    /// Unbounded US-ASCII with 32-bit offsets.
    #[must_use]
    pub const fn ascii() -> Self {
        Self::String(StringParameters::ascii(StringLayout::String))
    }

    /// UTF-8 of exactly `width` stored bytes, padded with trailing NUL.
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_utf8(width: u32) -> Result<Self> {
        Self::string(StringParameters::utf8(StringLayout::FixedString).try_with_bound(width)?)
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
        Self::string(StringParameters::ascii(StringLayout::FixedString).try_with_bound(width)?)
    }

    /// The parameters a string datatype declares, `None` for every other.
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
    /// let utf8 = DataType::utf8().string_parameters().expect("a string datatype");
    /// assert_eq!(utf8.layout(), StringLayout::String);
    /// assert_eq!(utf8.charset(), Charset::Utf8);
    ///
    /// let ascii = DataType::fixed_ascii(3)?.string_parameters().expect("a string datatype");
    /// assert_eq!(ascii.charset(), Charset::Ascii);
    /// assert_eq!(ascii.fixed(), Some(3));
    ///
    /// assert!(DataType::Currency.string_parameters().is_none());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn string_parameters(&self) -> Option<StringParameters> {
        match self {
            Self::String(parameters) => Some(*parameters),
            _ => None,
        }
    }

    /// Return whether this datatype is a string.
    #[must_use]
    pub const fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }

    /// The charset a string column's bytes are written in.
    ///
    /// `None` is a datatype that is not a string; every string has one,
    /// because UTF-8 is what a string with nothing declared is in.
    #[must_use]
    pub const fn charset(&self) -> Option<Charset> {
        match self.string_parameters() {
            Some(parameters) => Some(parameters.charset()),
            None => None,
        }
    }

    /// The fixed byte width of one value, when this datatype has one.
    ///
    /// [`crate::DataTypeId::fixed_byte_width`] answers for every
    /// parameter-free variant - the numbers, the codes, a UUID; this adds the
    /// two whose width is a parameter: a fixed string and fixed bytes.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_ascii(4)?.fixed_byte_width(), Some(4));
    /// assert_eq!(DataType::fixed_size_binary(16)?.fixed_byte_width(), Some(16));
    /// assert_eq!(DataType::Currency.fixed_byte_width(), Some(3));
    /// assert_eq!(DataType::utf8().fixed_byte_width(), None);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn fixed_byte_width(&self) -> Option<usize> {
        match self {
            Self::String(parameters) => match parameters.fixed() {
                Some(width) => Some(width as usize),
                None => None,
            },
            Self::Bytes(parameters) => match parameters.fixed() {
                Some(width) => Some(width as usize),
                None => None,
            },
            _ => self.id().fixed_byte_width(),
        }
    }
}
