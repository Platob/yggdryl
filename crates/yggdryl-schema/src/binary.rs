//! Arrow's binary data types: the variable-length [`BinaryType`] /
//! [`LargeBinaryType`], the view-backed [`BinaryViewType`] /
//! [`LargeBinaryViewType`], the fixed-width [`FixedSizeBinaryType`], and the
//! size-capped [`MaxedSizeBinaryType`]. All are [`PhysicalType`]s.
//!
//! ```
//! use yggdryl_schema::{BinaryType, DataType, DataTypeId, FixedSizeBinaryType};
//!
//! assert_eq!(BinaryType.name(), "binary");
//! assert_eq!(BinaryType.type_id(), DataTypeId::Binary);
//! assert!(BinaryType.is_physical());
//!
//! let fixed = FixedSizeBinaryType::new(16);
//! assert_eq!(fixed.byte_size(), 16);
//! assert_eq!(fixed.large_byte_size(), 16_i64);
//! assert_eq!(fixed.with_byte_size(32).byte_size(), 32);
//! ```

use crate::data_type::{DataType, PhysicalType};
use crate::data_type_id::DataTypeId;

/// Defines a parameterless binary data type: a unit struct implementing
/// [`DataType`] (with its Apache Arrow mapping under the `arrow` feature) and
/// [`PhysicalType`]. `$arrow` is the Arrow type it maps to.
macro_rules! binary_type {
    ($(#[$meta:meta])* $name:ident => $type_name:literal, $id:ident, $arrow:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name;

        impl DataType for $name {
            fn name(&self) -> &'static str {
                $type_name
            }

            fn type_id(&self) -> DataTypeId {
                DataTypeId::$id
            }

            #[cfg(feature = "arrow")]
            fn to_arrow_type(&self) -> arrow_schema::DataType {
                $arrow
            }

            #[cfg(feature = "arrow")]
            fn from_arrow_type(
                dtype: &arrow_schema::DataType,
                _metadata: &crate::Metadata,
            ) -> Result<Self, crate::SchemaError> {
                if *dtype == $arrow {
                    Ok($name)
                } else {
                    Err(crate::SchemaError::UnsupportedArrowType(dtype.clone()))
                }
            }
        }

        impl PhysicalType for $name {}
    };
}

binary_type! {
    /// Arrow's variable-length binary type (32-bit offsets).
    BinaryType => "binary", Binary, arrow_schema::DataType::Binary
}

binary_type! {
    /// Arrow's variable-length binary type (64-bit offsets).
    LargeBinaryType => "large_binary", LargeBinary, arrow_schema::DataType::LargeBinary
}

binary_type! {
    /// Arrow's view-backed variable-length binary type.
    BinaryViewType => "binary_view", BinaryView, arrow_schema::DataType::BinaryView
}

binary_type! {
    /// A 64-bit-sized, view-backed variable-length binary type. Arrow has no large
    /// binary-view, so [`to_arrow_type`](DataType::to_arrow_type) maps it to
    /// `BinaryView` (the `large` distinction is not preserved on round-trip).
    LargeBinaryViewType => "large_binary_view", LargeBinaryView, arrow_schema::DataType::BinaryView
}

/// Generates the byte-size accessors shared by the fixed- and max-size binary
/// types (both wrap an `i32` `byte_size`).
macro_rules! byte_size_accessors {
    () => {
        /// A value of this type holding `byte_size` bytes.
        pub fn new(byte_size: i32) -> Self {
            Self { byte_size }
        }

        /// The byte size.
        pub fn byte_size(&self) -> i32 {
            self.byte_size
        }

        /// The byte size, widened to 64 bits.
        pub fn large_byte_size(&self) -> i64 {
            i64::from(self.byte_size)
        }

        /// Returns a copy with a new byte size.
        pub fn with_byte_size(&self, byte_size: i32) -> Self {
            Self { byte_size }
        }

        /// Returns a copy with a new byte size given as a 64-bit value, clamping to
        /// the `i32` Arrow width (a warning is logged if it overflows).
        pub fn with_large_byte_size(&self, byte_size: i64) -> Self {
            let byte_size = i32::try_from(byte_size).unwrap_or_else(|_| {
                crate::log_event!(
                    warn,
                    "with_large_byte_size {byte_size} exceeds i32; clamping to i32::MAX"
                );
                i32::MAX
            });
            Self { byte_size }
        }
    };
}

/// Arrow's fixed-width binary type: every value is exactly
/// [`byte_size`](FixedSizeBinaryType::byte_size) bytes.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, FixedSizeBinaryType};
///
/// let ty = FixedSizeBinaryType::new(16);
/// assert_eq!(ty.name(), "fixed_size_binary");
/// assert_eq!(ty.type_id(), DataTypeId::FixedSizeBinary);
/// assert_eq!(ty.byte_size(), 16);
/// assert_eq!(ty.large_byte_size(), 16_i64);
/// assert_eq!(ty.with_byte_size(4).byte_size(), 4);
/// assert_eq!(ty.with_large_byte_size(8).byte_size(), 8);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FixedSizeBinaryType {
    byte_size: i32,
}

impl FixedSizeBinaryType {
    byte_size_accessors!();
}

impl DataType for FixedSizeBinaryType {
    fn name(&self) -> &'static str {
        "fixed_size_binary"
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::FixedSizeBinary
    }

    fn max_byte_size(&self) -> Option<i64> {
        Some(self.large_byte_size())
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_type(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::FixedSizeBinary(self.byte_size)
    }

    #[cfg(feature = "arrow")]
    fn from_arrow_type(
        dtype: &arrow_schema::DataType,
        _metadata: &crate::Metadata,
    ) -> Result<Self, crate::SchemaError> {
        match dtype {
            arrow_schema::DataType::FixedSizeBinary(byte_size) => Ok(Self::new(*byte_size)),
            other => Err(crate::SchemaError::UnsupportedArrowType(other.clone())),
        }
    }
}

impl PhysicalType for FixedSizeBinaryType {}

/// A variable-length binary type capped at a maximum byte size. Unlike
/// [`FixedSizeBinaryType`] (an exact width), values may be shorter; the scalar
/// layer truncates any payload longer than
/// [`byte_size`](MaxedSizeBinaryType::byte_size).
///
/// Arrow has no size-capped binary, so [`to_arrow_type`](DataType::to_arrow_type)
/// maps to a plain `Binary` and the cap is stashed in
/// [`metadata`](DataType::metadata); the two together rebuild the type in
/// [`from_arrow_type`](DataType::from_arrow_type).
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, MaxedSizeBinaryType};
///
/// let ty = MaxedSizeBinaryType::new(8);
/// assert_eq!(ty.name(), "maxed_size_binary");
/// assert_eq!(ty.type_id(), DataTypeId::MaxedSizeBinary);
/// assert_eq!(ty.byte_size(), 8);
/// assert_eq!(ty.max_byte_size(), Some(8));
/// assert!(ty.is_physical());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct MaxedSizeBinaryType {
    byte_size: i32,
}

impl MaxedSizeBinaryType {
    byte_size_accessors!();
}

impl DataType for MaxedSizeBinaryType {
    fn name(&self) -> &'static str {
        "maxed_size_binary"
    }

    fn type_id(&self) -> DataTypeId {
        DataTypeId::MaxedSizeBinary
    }

    fn max_byte_size(&self) -> Option<i64> {
        Some(self.large_byte_size())
    }

    fn metadata(&self) -> crate::Metadata {
        // Record the identity plus the cap Arrow drops, so `from_arrow_type` can
        // rebuild the exact type.
        let mut metadata = crate::metadata::type_metadata(self.name());
        metadata.insert(
            crate::metadata::reserved_key("byte_size"),
            self.byte_size().to_string().into_bytes(),
        );
        metadata
    }

    #[cfg(feature = "arrow")]
    fn to_arrow_type(&self) -> arrow_schema::DataType {
        // Arrow has no size-capped binary; map to the closest variable type.
        arrow_schema::DataType::Binary
    }

    #[cfg(feature = "arrow")]
    fn from_arrow_type(
        dtype: &arrow_schema::DataType,
        metadata: &crate::Metadata,
    ) -> Result<Self, crate::SchemaError> {
        if *dtype != arrow_schema::DataType::Binary {
            return Err(crate::SchemaError::UnsupportedArrowType(dtype.clone()));
        }
        let byte_size = metadata
            .get(&crate::metadata::reserved_key("byte_size"))
            .and_then(|value| std::str::from_utf8(value).ok())
            .and_then(|value| value.parse::<i32>().ok())
            .ok_or(crate::SchemaError::MissingTypeMetadata("byte_size"))?;
        Ok(Self::new(byte_size))
    }
}

impl PhysicalType for MaxedSizeBinaryType {}
