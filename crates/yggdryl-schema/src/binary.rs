//! Arrow's binary data types: the variable-length [`BinaryType`] /
//! [`LargeBinaryType`] and the view-backed [`BinaryViewType`] /
//! [`LargeBinaryViewType`]. All are [`PhysicalType`]s.
//!
//! Each carries an optional `byte_size` cap: `None` is unbounded; `Some(n)` caps a
//! value at `n` bytes (the scalar layer truncates an over-long payload). Arrow has
//! no size-capped binary, so the cap travels in the field metadata rather than the
//! Arrow type.
//!
//! ```
//! use yggdryl_schema::{BinaryType, DataType, DataTypeId};
//!
//! let b = BinaryType::new();
//! assert_eq!(b.name(), "binary");
//! assert_eq!(b.type_id(), DataTypeId::Binary);
//! assert!(b.is_physical());
//! assert_eq!(b.byte_size(), None);
//!
//! let capped = b.with_byte_size(16);
//! assert_eq!(capped.byte_size(), Some(16));
//! assert_eq!(capped.max_byte_size(), Some(16));
//! assert_eq!(capped.without_byte_size().byte_size(), None);
//! ```

use crate::data_type::{DataType, PhysicalType};
use crate::data_type_id::DataTypeId;

/// Defines a binary data type: a struct carrying an optional `byte_size` cap that
/// implements [`DataType`] (with its Apache Arrow mapping under the `arrow`
/// feature) and [`PhysicalType`]. `$arrow` is the variable Arrow type it maps to.
macro_rules! binary_type {
    ($(#[$meta:meta])* $name:ident => $type_name:literal, $id:ident, $arrow:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name {
            byte_size: Option<i32>,
        }

        impl $name {
            /// An unbounded value of this type.
            pub fn new() -> Self {
                Self::default()
            }

            /// The optional byte-size cap (`None` when unbounded). A scalar of this
            /// type truncates an over-long payload to it.
            pub fn byte_size(&self) -> Option<i32> {
                self.byte_size
            }

            /// Returns a copy capped at `byte_size` bytes.
            pub fn with_byte_size(&self, byte_size: i32) -> Self {
                Self { byte_size: Some(byte_size) }
            }

            /// Returns a copy with no byte-size cap.
            pub fn without_byte_size(&self) -> Self {
                Self { byte_size: None }
            }
        }

        impl DataType for $name {
            fn name(&self) -> &'static str {
                $type_name
            }

            fn type_id(&self) -> DataTypeId {
                DataTypeId::$id
            }

            fn max_byte_size(&self) -> Option<i64> {
                self.byte_size.map(i64::from)
            }

            fn metadata(&self) -> crate::Metadata {
                let mut metadata = crate::metadata::type_metadata(self.name());
                crate::metadata::set_byte_size(&mut metadata, self.byte_size);
                metadata
            }

            #[cfg(feature = "arrow")]
            fn to_arrow_type(&self) -> arrow_schema::DataType {
                $arrow
            }

            #[cfg(feature = "arrow")]
            fn from_arrow_type(
                dtype: &arrow_schema::DataType,
                metadata: &crate::Metadata,
            ) -> Result<Self, crate::SchemaError> {
                if *dtype != $arrow {
                    return Err(crate::SchemaError::UnsupportedArrowType(dtype.clone()));
                }
                Ok(Self {
                    byte_size: crate::metadata::get_byte_size(metadata)?,
                })
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
