//! Crate-internal macros generating the shared shape of parameter-free data
//! types, so each type's file states only what is unique to it.

/// Generates a parameter-free (unit-struct) data type: the struct itself, its
/// [`DataType`](crate::DataType) implementation mapping to the given
/// `arrow_schema::DataType` variant, and its render-only `Display`.
///
/// The byte encoding of a unit type is empty — the concrete Rust type already
/// identifies it — so `to_bytes` writes no payload and `from_bytes` accepts
/// only an empty slice.
macro_rules! unit_data_type {
    (
        $(#[$doc:meta])*
        $name:ident, $arrow:ident, $display:literal
    ) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
        pub struct $name;

        impl $crate::DataType for $name {
            fn to_arrow(&self) -> ::arrow_schema::DataType {
                ::arrow_schema::DataType::$arrow
            }

            fn from_arrow(
                data_type: &::arrow_schema::DataType,
            ) -> Result<Self, $crate::DataTypeError> {
                match data_type {
                    ::arrow_schema::DataType::$arrow => Ok(Self),
                    other => Err($crate::DataTypeError::ArrowTypeMismatch {
                        expected: $display,
                        actual: other.clone(),
                    }),
                }
            }

            fn to_bytes(&self) -> Vec<u8> {
                Vec::new()
            }

            fn from_bytes(bytes: &[u8]) -> Result<Self, $crate::DataTypeError> {
                if bytes.is_empty() {
                    Ok(Self)
                } else {
                    Err($crate::DataTypeError::InvalidByteLength {
                        expected: 0,
                        actual: bytes.len(),
                    })
                }
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str($display)
            }
        }
    };
}

/// Generates a fixed-width unit data type: everything [`unit_data_type!`]
/// generates plus the [`PrimitiveType`](crate::PrimitiveType) implementation
/// tying it to its native Rust value type and bit width.
macro_rules! primitive_data_type {
    (
        $(#[$doc:meta])*
        $name:ident, $native:ty, $bit_width:expr, $arrow:ident, $display:literal
    ) => {
        $crate::datatype::macros::unit_data_type! {
            $(#[$doc])*
            $name, $arrow, $display
        }

        impl $crate::PrimitiveType for $name {
            type Native = $native;
            const BIT_WIDTH: usize = $bit_width;
        }
    };
}

pub(crate) use primitive_data_type;
pub(crate) use unit_data_type;
