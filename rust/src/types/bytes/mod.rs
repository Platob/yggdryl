//! The byte datatype family: one layout, one length bound.
//!
//! Arrow has four binary layouts and no way to bound one: a `Binary` array
//! declares 32-bit offsets and nothing else, and `varbinary(n)` exists
//! nowhere in the format. This family is the one place this crate answers
//! both questions - which layout, how long - and every byte column the crate
//! has is one member of it.
//!
//! [`BytesLayout`] names the four layouts. [`BytesParameters`] is a layout
//! beside the bound its values are held to. [`crate::DataType::bytes`] builds
//! the one byte datatype, [`crate::DataType::Bytes`], from them; `binary`,
//! `varbinary(16)` and `fixed_size_binary(16)` are spellings of it, never
//! datatypes of their own. [`Bytes`] is the one byte value.
//!
//! ```
//! use yggdryl::types::{BytesLayout, BytesParameters};
//! use yggdryl::DataType;
//!
//! # fn main() -> yggdryl::Result<()> {
//! // Every spelling is one datatype, and it reads back as itself.
//! assert_eq!(DataType::from_str("bytes")?, DataType::binary());
//! assert_eq!(DataType::from_str("fixed_binary(16)")?.to_string(), "fixed_size_binary(16)");
//!
//! // The layout and the bound are what a byte column declares.
//! let bounded = DataType::from_str("varbinary(32)")?;
//! let parameters = bounded.bytes_parameters().expect("a byte datatype");
//! assert_eq!(parameters.layout(), BytesLayout::Binary);
//! assert_eq!(parameters.max(), Some(32));
//! assert_eq!(bounded.to_string(), "binary(32)");
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
mod dtypes;
mod fields;
mod parameters;
mod parser;
mod scalars;

pub(crate) use arrow::{arrow_storage, describes_storage, needs_extension};
pub use fields::*;
pub use parameters::{BYTES_EXTENSION_NAME, BytesParameters};
pub(crate) use scalars::bytes_from_value;
pub use scalars::{Bytes, INLINE_BYTES};

/// One of the four ways this crate lays bytes out - Arrow's four.
///
/// The layout is the physical shape alone, how a value's bytes are addressed;
/// the bound beside it in [`BytesParameters`] is how many there may be.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum BytesLayout {
    /// Variable width, 32-bit offsets - Arrow's `Binary`.
    #[default]
    Binary,
    /// One fixed byte width every value fills exactly - Arrow's
    /// `FixedSizeBinary`. Bytes are never padded: a value is its width.
    FixedSizeBinary,
    /// Variable width, 64-bit offsets - Arrow's `LargeBinary`.
    LargeBinary,
    /// The view layout: a short prefix inline, the rest out of line.
    BinaryView,
}

impl BytesLayout {
    /// Every layout in canonical declaration order.
    pub const ALL: [Self; 4] = [
        Self::Binary,
        Self::FixedSizeBinary,
        Self::LargeBinary,
        Self::BinaryView,
    ];

    /// The canonical name of this layout.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Binary => "binary",
            Self::FixedSizeBinary => "fixed_size_binary",
            Self::LargeBinary => "large_binary",
            Self::BinaryView => "binary_view",
        }
    }

    /// Resolve a layout from its name.
    ///
    /// Case, underscores, hyphens and spaces are all ignored, so
    /// `LARGE_BINARY`, `large-binary` and `largebinary` are one layout, and
    /// `fixed_binary` is the fixed one's second spelling.
    ///
    /// # Errors
    ///
    /// Returns [`Error::UnknownDataType`] for a name no layout answers to.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<Self> {
        if super::parser::folds_equal(value, "fixed_binary") {
            return Ok(Self::FixedSizeBinary);
        }
        Self::ALL
            .into_iter()
            .find(|layout| super::parser::folds_equal(value, layout.as_str()))
            .ok_or_else(|| Error::UnknownDataType(format_smolstr!("{value}")))
    }

    /// The identifier naming this layout.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Binary => DataTypeId::Binary,
            Self::FixedSizeBinary => DataTypeId::FixedSizeBinary,
            Self::LargeBinary => DataTypeId::LargeBinary,
            Self::BinaryView => DataTypeId::BinaryView,
        }
    }

    /// The layout one identifier names, `None` for an identifier that is not
    /// a byte layout's.
    #[must_use]
    pub const fn from_id(id: DataTypeId) -> Option<Self> {
        match id {
            DataTypeId::Binary => Some(Self::Binary),
            DataTypeId::FixedSizeBinary => Some(Self::FixedSizeBinary),
            DataTypeId::LargeBinary => Some(Self::LargeBinary),
            DataTypeId::BinaryView => Some(Self::BinaryView),
            _ => None,
        }
    }

    /// Return whether every value in this layout is the same width.
    #[must_use]
    pub const fn is_fixed(self) -> bool {
        matches!(self, Self::FixedSizeBinary)
    }

    /// Return whether this layout addresses its bytes through a view.
    #[must_use]
    pub const fn is_view(self) -> bool {
        matches!(self, Self::BinaryView)
    }

    /// Return whether this layout declares 64-bit offsets.
    #[must_use]
    pub const fn is_large(self) -> bool {
        matches!(self, Self::LargeBinary)
    }
}

impl fmt::Display for BytesLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for BytesLayout {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BytesLayout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = <std::borrow::Cow<'_, str>>::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}
