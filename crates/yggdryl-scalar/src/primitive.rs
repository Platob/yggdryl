//! The `primitive_scalar!` macro — the single source of a primitive scalar, stamped out
//! once per data type (`I8Scalar`, …, `F64Scalar`, `BooleanScalar`) so every
//! scalar shares one implementation, mirroring the dtype layer's `primitive_type!`.
//!
//! The value↔bytes codec is delegated to the data type's
//! [`TypedDataType`](yggdryl_dtype::TypedDataType), so the **same** macro covers every
//! primitive — including `Boolean`. A scalar is **always present** (non-nullable): it holds
//! its native value directly and serialises to just the value's little-endian bytes.
//! Equality and hashing are **by serialised bytes** (like the core buffers), so the float
//! scalars behave bitwise (`0.0 != -0.0`, two `NaN`s with the same bits are equal).

/// Generates one primitive scalar named `$scalar` holding a `$native` value of data type
/// `$dtype`, with canonical name `$lit`. `$example` is a sample value used in the generated
/// doctest.
macro_rules! primitive_scalar {
    ($scalar:ident, $dtype:ident, $native:ty, $lit:literal, $example:literal) => {
        #[doc = concat!("A single `", $lit, "` value.")]
        ///
        /// Holds its native value; its data type is
        #[doc = concat!("[`", stringify!($dtype), "`](yggdryl_dtype::", stringify!($dtype), ").")]
        /// It round-trips through the value's little-endian bytes and compares/hashes by
        /// those bytes.
        ///
        #[doc = concat!("```")]
        #[doc = concat!("use yggdryl_scalar::{Scalar, TypedScalar, ", stringify!($scalar), "};")]
        #[doc = concat!("let value = ", stringify!($scalar), "::new(", stringify!($example), ");")]
        #[doc = concat!("assert_eq!(value.value(), ", stringify!($example), ");")]
        #[doc = concat!("// Byte round-trip.")]
        #[doc = concat!("assert_eq!(", stringify!($scalar), "::deserialize_bytes(&value.serialize_bytes()).unwrap(), value);")]
        #[doc = concat!("```")]
        #[derive(Clone, Copy, Debug)]
        pub struct $scalar {
            value: $native,
        }

        impl $scalar {
            #[doc = concat!("Creates a `", $lit, "` scalar holding `value`.")]
            pub fn new(value: $native) -> Self {
                Self { value }
            }

            /// Reconstructs the scalar from its serialised bytes (the value's little-endian
            /// bytes).
            ///
            /// # Errors
            /// [`ScalarError`](crate::ScalarError) if the bytes do not decode to a value of
            /// this data type (e.g. a wrong length).
            pub fn deserialize_bytes(bytes: &[u8]) -> Result<Self, $crate::ScalarError> {
                let value = <yggdryl_dtype::$dtype as yggdryl_dtype::TypedDataType<$native>>::value_from_bytes(
                    &yggdryl_dtype::$dtype::new(),
                    bytes,
                )?;
                Ok(Self { value })
            }
        }

        impl $crate::Scalar for $scalar {
            fn arrow_data_type(&self) -> arrow_schema::DataType {
                <yggdryl_dtype::$dtype as yggdryl_dtype::DataType>::to_arrow(&yggdryl_dtype::$dtype::new())
            }

            fn serialize_bytes(&self) -> Vec<u8> {
                <yggdryl_dtype::$dtype as yggdryl_dtype::TypedDataType<$native>>::value_to_bytes(
                    &yggdryl_dtype::$dtype::new(),
                    self.value,
                )
            }

            fn default_any_scalar(&self) -> Box<dyn $crate::Scalar> {
                Box::new(<$scalar as $crate::TypedScalar<yggdryl_dtype::$dtype, $native>>::default_scalar())
            }
        }

        impl $crate::TypedScalar<yggdryl_dtype::$dtype, $native> for $scalar {
            fn value(&self) -> $native {
                self.value
            }

            fn data_type(&self) -> yggdryl_dtype::$dtype {
                yggdryl_dtype::$dtype::new()
            }

            fn default_scalar() -> Self {
                Self::new(
                    <yggdryl_dtype::$dtype as yggdryl_dtype::TypedDataType<$native>>::default_value(
                        &yggdryl_dtype::$dtype::new(),
                    ),
                )
            }
        }

        impl $crate::PrimitiveScalar for $scalar {}

        // Value semantics by serialised bytes (rule 7): equal iff `serialize_bytes` are
        // equal, and equal values hash equal. Byte-based so the float scalars behave
        // bitwise (`0.0 != -0.0`, same-bit `NaN`s are equal).
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
