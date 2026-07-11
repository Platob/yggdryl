//! The `primitive_type!` macro — the single source of a numeric primitive data type,
//! stamped out once per native type (`I8Type`, …, `F64Type`) so every primitive
//! shares one implementation, mirroring the core's `primitive_buffer!` macro.

/// Generates one value-free primitive data type named `$name` for the native primitive
/// `$native`, mapping to the Arrow variant `$arrow` and the core runtime tag `$tag`,
/// with canonical name `$lit`.
///
/// A primitive data type is a unit marker (its identity *is* its type, so it derives
/// `Eq`/`Hash` trivially and serialises to an empty payload). It implements
/// [`DataType`](crate::DataType), [`TypedDataType<$native>`](crate::TypedDataType)
/// (value codec delegating to [`yggdryl_buffer::IoPrimitive`]), and
/// [`PrimitiveType`](crate::PrimitiveType) (mapping to the core tag).
macro_rules! primitive_type {
    ($name:ident, $native:ty, $arrow:ident, $tag:ident, $lit:literal) => {
        #[doc = concat!("The `", $lit, "` primitive data type (Arrow `", stringify!($arrow), "`, native `", stringify!($native), "`).")]
        ///
        /// A value-free marker: all instances are equal, it serialises to an empty
        /// payload, and it converts to and from its Arrow type and the core
        /// [`PrimitiveType`](yggdryl_converter::PrimitiveType) tag.
        ///
        #[doc = concat!("```")]
        #[doc = concat!("use yggdryl_dtype::{DataType, PrimitiveType, TypedDataType, ", stringify!($name), "};")]
        #[doc = concat!("let dt = ", stringify!($name), "::new();")]
        #[doc = concat!("assert_eq!(dt.name(), \"", $lit, "\");")]
        #[doc = concat!("assert_eq!(dt.byte_width(), Some(core::mem::size_of::<", stringify!($native), ">()));")]
        #[doc = concat!("assert_eq!(dt.primitive_tag(), Some(yggdryl_converter::PrimitiveType::", stringify!($tag), "));")]
        #[doc = concat!("// Byte round-trip (empty payload) and Arrow round-trip.")]
        #[doc = concat!("assert_eq!(", stringify!($name), "::deserialize_bytes(&dt.serialize_bytes()).unwrap(), dt);")]
        #[doc = concat!("assert_eq!(", stringify!($name), "::from_arrow(&dt.to_arrow()).unwrap(), dt);")]
        #[doc = concat!("// Value codec.")]
        #[doc = concat!("let bytes = dt.value_to_bytes(1 as ", stringify!($native), ");")]
        #[doc = concat!("assert_eq!(dt.value_from_bytes(&bytes).unwrap(), 1 as ", stringify!($native), ");")]
        #[doc = concat!("```")]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
        pub struct $name;

        impl $name {
            #[doc = concat!("Creates the `", $lit, "` data type.")]
            pub const fn new() -> Self {
                Self
            }

            #[doc = concat!("Reconstructs the type from its (empty) serialised payload.")]
            ///
            /// # Errors
            /// [`DTypeError::UnexpectedPayload`](crate::DTypeError::UnexpectedPayload) if
            /// `bytes` is non-empty (a primitive type carries no parameters).
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, $crate::DTypeError> {
                if bytes.is_empty() {
                    Ok(Self)
                } else {
                    Err($crate::DTypeError::UnexpectedPayload {
                        ty: $lit,
                        len: bytes.len(),
                    })
                }
            }

            #[doc = concat!("Builds the type from an Arrow [`DataType`](arrow_schema::DataType), validating it is `", stringify!($arrow), "`.")]
            ///
            /// # Errors
            /// [`DTypeError::ArrowTypeMismatch`](crate::DTypeError::ArrowTypeMismatch) if
            /// `arrow` is a different variant.
            pub fn from_arrow(arrow: &arrow_schema::DataType) -> Result<Self, $crate::DTypeError> {
                if matches!(arrow, arrow_schema::DataType::$arrow) {
                    Ok(Self)
                } else {
                    Err($crate::DTypeError::ArrowTypeMismatch {
                        expected: $lit,
                        got: format!("{arrow:?}"),
                    })
                }
            }

            #[doc = concat!("Builds the type from the core [`PrimitiveType`](yggdryl_converter::PrimitiveType) tag, or `None` if the tag is not `", stringify!($tag), "`.")]
            pub fn from_primitive_tag(tag: yggdryl_converter::PrimitiveType) -> Option<Self> {
                matches!(tag, yggdryl_converter::PrimitiveType::$tag).then_some(Self)
            }
        }

        impl $crate::DataType for $name {
            fn name(&self) -> &'static str {
                $lit
            }

            fn byte_width(&self) -> Option<usize> {
                Some(core::mem::size_of::<$native>())
            }

            fn to_arrow(&self) -> arrow_schema::DataType {
                arrow_schema::DataType::$arrow
            }

            fn serialize_bytes(&self) -> Vec<u8> {
                Vec::new()
            }
        }

        impl $crate::TypedDataType<$native> for $name {
            fn native_default(&self) -> $native {
                <$native as yggdryl_buffer::IoPrimitive>::ZERO
            }

            fn value_to_bytes(&self, value: $native) -> Vec<u8> {
                <$native as yggdryl_buffer::IoPrimitive>::to_le_vec(value)
            }

            fn value_from_bytes(&self, bytes: &[u8]) -> Result<$native, $crate::DTypeError> {
                const W: usize = core::mem::size_of::<$native>();
                if bytes.len() != W {
                    return Err($crate::DTypeError::InvalidValueLength {
                        ty: $lit,
                        len: bytes.len(),
                        width: W,
                    });
                }
                Ok(<$native as yggdryl_buffer::IoPrimitive>::from_le_slice(bytes))
            }
        }

        impl $crate::PrimitiveType for $name {
            fn primitive_tag(&self) -> Option<yggdryl_converter::PrimitiveType> {
                Some(yggdryl_converter::PrimitiveType::$tag)
            }
        }
    };
}

pub(crate) use primitive_type;
