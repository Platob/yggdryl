//! The one door into the byte family, and the questions every byte column
//! answers.

use super::{BytesLayout, BytesParameters};
use crate::{DataType, Result};

impl DataType {
    /// The byte datatype these parameters name.
    ///
    /// This is the family's one constructor. Every byte column is
    /// [`DataType::Bytes`]; what differs is what it declares, and the
    /// parameters say all of it - a layout and a bound.
    ///
    /// ```
    /// use yggdryl::types::{BytesLayout, BytesParameters};
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::bytes(BytesLayout::LargeBinary)?, DataType::large_binary());
    /// let bounded = BytesParameters::new(BytesLayout::Binary).try_with_bound(32)?;
    /// assert_eq!(DataType::bytes(bounded)?.to_string(), "binary(32)");
    /// assert_eq!(DataType::fixed_size_binary(16)?.to_string(), "fixed_size_binary(16)");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for the fixed layout with no
    /// width: the width is what makes it fixed.
    pub fn bytes(parameters: impl Into<BytesParameters>) -> Result<Self> {
        let parameters = parameters.into();
        parameters.validate()?;
        Ok(Self::Bytes(parameters))
    }

    /// Unbounded bytes with 32-bit offsets - Arrow's `Binary`.
    #[must_use]
    pub const fn binary() -> Self {
        Self::Bytes(BytesParameters::new(BytesLayout::Binary))
    }

    /// Unbounded bytes with 64-bit offsets - Arrow's `LargeBinary`.
    #[must_use]
    pub const fn large_binary() -> Self {
        Self::Bytes(BytesParameters::new(BytesLayout::LargeBinary))
    }

    /// Unbounded bytes in the view layout - Arrow's `BinaryView`.
    #[must_use]
    pub const fn binary_view() -> Self {
        Self::Bytes(BytesParameters::new(BytesLayout::BinaryView))
    }

    /// Exactly `width` bytes per value - Arrow's `FixedSizeBinary`.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// assert_eq!(DataType::fixed_size_binary(16)?.fixed_byte_width(), Some(16));
    /// assert!(DataType::fixed_size_binary(0).is_err());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`crate::Error::InvalidDataType`] for a width of zero.
    pub fn fixed_size_binary(width: u32) -> Result<Self> {
        Self::bytes(BytesParameters::new(BytesLayout::FixedSizeBinary).try_with_bound(width)?)
    }

    /// The parameters a byte datatype declares, `None` for every other.
    ///
    /// A UUID and a geospatial value are bytes with an identity rather than
    /// byte columns, so they answer `None` here exactly as a code answers no
    /// [`Self::string_parameters`].
    #[must_use]
    pub const fn bytes_parameters(&self) -> Option<BytesParameters> {
        match self {
            Self::Bytes(parameters) => Some(*parameters),
            _ => None,
        }
    }
}
