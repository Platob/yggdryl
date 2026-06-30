//! The string data types: [`StringType`], [`LargeStringType`], [`StringViewType`]
//! and [`LargeStringViewType`]. Each is a [`LogicalType`] backed by the matching
//! binary [`PhysicalType`](crate::PhysicalType), carrying a [`Charset`] (default
//! [UTF-8](Charset::Utf8)) — a string is just binary bytes read with a charset.
//!
//! Arrow only has UTF-8 strings: a UTF-8 string maps to the Arrow string type,
//! while any other charset falls back to its binary storage type (with the charset
//! preserved in the field metadata).
//!
//! ```
//! use yggdryl_schema::{Charset, DataType, DataTypeId, LogicalType, StringType};
//!
//! let s = StringType::new();
//! assert_eq!(s.name(), "string");
//! assert_eq!(s.type_id(), DataTypeId::String);
//! assert!(s.is_logical());
//! assert_eq!(s.charset(), Charset::Utf8);
//! assert_eq!(s.physical().type_id(), DataTypeId::Binary);
//!
//! let latin1 = s.with_charset(Charset::Latin1);
//! assert_eq!(latin1.charset(), Charset::Latin1);
//! ```

use crate::binary::{BinaryType, BinaryViewType, LargeBinaryType, LargeBinaryViewType};
use crate::charset::Charset;
use crate::data_type::{DataType, LogicalType};
use crate::data_type_id::DataTypeId;

/// Defines a string logical type: a charset-carrying unit-ish struct backed by the
/// binary physical type `$physical`, mapping to the Arrow string type `$arrow`.
macro_rules! string_type {
    ($(#[$meta:meta])* $name:ident => $type_name:literal, $id:ident, $physical:ty, $arrow:expr) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name {
            charset: Charset,
        }

        impl $name {
            /// A string of the default ([UTF-8](Charset::Utf8)) charset.
            pub fn new() -> Self {
                Self::default()
            }

            /// The charset its bytes are read with.
            pub fn charset(&self) -> Charset {
                self.charset
            }

            /// Returns a copy with a new charset.
            pub fn with_charset(&self, charset: Charset) -> Self {
                Self { charset }
            }
        }

        impl DataType for $name {
            fn name(&self) -> &'static str {
                $type_name
            }

            fn type_id(&self) -> DataTypeId {
                DataTypeId::$id
            }

            fn metadata(&self) -> crate::Metadata {
                let mut metadata = crate::metadata::type_metadata(self.name());
                // The default charset is implied; store only a non-default one.
                if self.charset != Charset::Utf8 {
                    metadata.insert(
                        crate::metadata::reserved_key("charset"),
                        self.charset.name().as_bytes().to_vec(),
                    );
                }
                metadata
            }

            #[cfg(feature = "arrow")]
            fn to_arrow_type(&self) -> arrow_schema::DataType {
                if self.charset == Charset::Utf8 {
                    $arrow
                } else {
                    // Arrow only has UTF-8 strings; a non-UTF-8 charset falls back to
                    // the binary physical storage (its charset lives in metadata).
                    self.physical().to_arrow_type()
                }
            }

            #[cfg(feature = "arrow")]
            fn from_arrow_type(
                dtype: &arrow_schema::DataType,
                metadata: &crate::Metadata,
            ) -> Result<Self, crate::SchemaError> {
                let charset = match metadata.get(&crate::metadata::reserved_key("charset")) {
                    Some(value) => std::str::from_utf8(value)
                        .ok()
                        .and_then(Charset::from_name)
                        .ok_or(crate::SchemaError::MissingTypeMetadata("charset"))?,
                    None => Charset::Utf8,
                };
                // The Arrow type must match what this charset would produce — a
                // string type for UTF-8, the binary storage otherwise.
                let candidate = Self { charset };
                if *dtype == candidate.to_arrow_type() {
                    Ok(candidate)
                } else {
                    Err(crate::SchemaError::UnsupportedArrowType(dtype.clone()))
                }
            }
        }

        impl LogicalType for $name {
            type Physical = $physical;

            fn physical(&self) -> $physical {
                <$physical>::default()
            }
        }
    };
}

string_type! {
    /// A string backed by [`BinaryType`] (32-bit offsets). Maps to Arrow `Utf8`.
    StringType => "string", String, BinaryType, arrow_schema::DataType::Utf8
}

string_type! {
    /// A string backed by [`LargeBinaryType`] (64-bit offsets). Maps to Arrow
    /// `LargeUtf8`.
    LargeStringType => "large_string", LargeString, LargeBinaryType, arrow_schema::DataType::LargeUtf8
}

string_type! {
    /// A view-backed string backed by [`BinaryViewType`]. Maps to Arrow `Utf8View`.
    StringViewType => "string_view", StringView, BinaryViewType, arrow_schema::DataType::Utf8View
}

string_type! {
    /// A 64-bit view-backed string backed by [`LargeBinaryViewType`]. Arrow has no
    /// large string-view, so [`to_arrow_type`](DataType::to_arrow_type) maps it to
    /// `Utf8View` (the `large` distinction is not preserved on round-trip).
    LargeStringViewType => "large_string_view", LargeStringView, LargeBinaryViewType, arrow_schema::DataType::Utf8View
}
