//! The `primitive_scalar!` macro — the single source of a primitive scalar, stamped out
//! once per data type (`I8Scalar`, …, `F64Scalar`, `BooleanScalar`) so every
//! scalar shares one implementation, mirroring the dtype layer's `primitive_type!`.
//!
//! The value↔bytes codec is delegated to the data type's
//! [`TypedDataType`](yggdryl_dtype::TypedDataType), so the **same** macro covers every
//! primitive — including `Boolean`. Equality and hashing are **by serialised bytes**
//! (like the core buffers), so the float scalars behave bitwise (`0.0 != -0.0`, two
//! `NaN`s with the same bits are equal) and a present value never equals a null.

/// Generates one primitive scalar named `$scalar` holding an `Option<$native>` of data
/// type `$dtype`, with canonical name `$lit`. `$example` is a sample value used in the
/// generated doctest.
macro_rules! primitive_scalar {
    ($scalar:ident, $dtype:ident, $native:ty, $lit:literal, $example:literal) => {
        #[doc = concat!("A single, possibly-null `", $lit, "` value.")]
        ///
        /// Holds an `Option` of its native value; its data type is
        #[doc = concat!("[`", stringify!($dtype), "`](yggdryl_dtype::", stringify!($dtype), ").")]
        /// It round-trips through bytes (a null flag followed by the value's little-endian
        /// bytes when present) and compares/hashes by those bytes.
        ///
        #[doc = concat!("```")]
        #[doc = concat!("use yggdryl_scalar::{Scalar, TypedScalar, ", stringify!($scalar), "};")]
        #[doc = concat!("let value = ", stringify!($scalar), "::new(", stringify!($example), ");")]
        #[doc = concat!("assert_eq!(value.value(), Some(", stringify!($example), "));")]
        #[doc = concat!("assert!(!value.is_null());")]
        #[doc = concat!("assert!(", stringify!($scalar), "::null().is_null());")]
        #[doc = concat!("// Byte round-trip, present and null.")]
        #[doc = concat!("assert_eq!(", stringify!($scalar), "::deserialize_bytes(&value.serialize_bytes()).unwrap(), value);")]
        #[doc = concat!("assert_eq!(", stringify!($scalar), "::deserialize_bytes(&", stringify!($scalar), "::null().serialize_bytes()).unwrap(), ", stringify!($scalar), "::null());")]
        #[doc = concat!("```")]
        #[derive(Clone, Copy, Debug)]
        pub struct $scalar {
            value: Option<$native>,
        }

        impl $scalar {
            #[doc = concat!("Creates a present `", $lit, "` scalar holding `value`.")]
            pub fn new(value: $native) -> Self {
                Self { value: Some(value) }
            }

            #[doc = concat!("Creates a null `", $lit, "` scalar.")]
            pub fn null() -> Self {
                Self { value: None }
            }

            /// Reconstructs the scalar from its serialised bytes (a null flag followed by
            /// the value's little-endian bytes when present).
            ///
            /// # Errors
            /// [`ScalarError`](crate::ScalarError) if the payload is empty, the flag is
            /// not `0`/`1`, a null carries trailing bytes, or the value does not decode.
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, $crate::ScalarError> {
                let (&flag, rest) =
                    bytes.split_first().ok_or($crate::ScalarError::EmptyPayload)?;
                match flag {
                    0 => {
                        if rest.is_empty() {
                            Ok(Self { value: None })
                        } else {
                            Err($crate::ScalarError::NullWithValue { len: rest.len() })
                        }
                    }
                    1 => {
                        let value = <yggdryl_dtype::$dtype as yggdryl_dtype::TypedDataType<$native>>::value_from_bytes(
                            &yggdryl_dtype::$dtype::new(),
                            rest,
                        )?;
                        Ok(Self { value: Some(value) })
                    }
                    other => Err($crate::ScalarError::InvalidNullFlag { flag: other }),
                }
            }
        }

        impl $crate::Scalar for $scalar {
            fn is_null(&self) -> bool {
                self.value.is_none()
            }

            fn arrow_data_type(&self) -> arrow_schema::DataType {
                <yggdryl_dtype::$dtype as yggdryl_dtype::DataType>::to_arrow(&yggdryl_dtype::$dtype::new())
            }

            fn serialize_bytes(&self) -> Vec<u8> {
                match self.value {
                    Some(value) => {
                        let mut out = Vec::new();
                        out.push(1);
                        out.extend_from_slice(
                            &<yggdryl_dtype::$dtype as yggdryl_dtype::TypedDataType<$native>>::value_to_bytes(
                                &yggdryl_dtype::$dtype::new(),
                                value,
                            ),
                        );
                        out
                    }
                    None => vec![0],
                }
            }
        }

        impl $crate::TypedScalar<yggdryl_dtype::$dtype, $native> for $scalar {
            fn value(&self) -> Option<$native> {
                self.value
            }

            fn data_type(&self) -> yggdryl_dtype::$dtype {
                yggdryl_dtype::$dtype::new()
            }
        }

        impl $crate::PrimitiveScalar for $scalar {}

        // Value semantics by serialised bytes (rule 7): equal iff `serialize_bytes` are
        // equal, and equal values hash equal. Byte-based so the float scalars behave
        // bitwise and a present value never equals a null.
        impl PartialEq for $scalar {
            fn eq(&self, other: &Self) -> bool {
                use $crate::Scalar;
                self.serialize_bytes() == other.serialize_bytes()
            }
        }

        impl Eq for $scalar {}

        impl core::hash::Hash for $scalar {
            fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
                use $crate::Scalar;
                self.serialize_bytes().hash(state);
            }
        }
    };
}

pub(crate) use primitive_scalar;
