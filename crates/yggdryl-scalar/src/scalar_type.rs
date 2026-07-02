//! The subtrait tying a data type to its one-element Arrow layout.

use core::mem::{align_of, size_of};
use core::str;

use yggdryl_schema::{
    Binary, Boolean, DataType, Date32, Date64, Decimal128, Decimal256, FixedSizeBinary, Float32,
    Float64, Int16, Int32, Int64, Int8, LargeBinary, LargeUtf8, PrimitiveType, Time32, Time32Unit,
    Time64, Time64Unit, TimeUnit, TypedDuration, TypedTimestamp, UInt16, UInt32, UInt64, UInt8,
    Utf8,
};

use crate::ScalarError;

/// A data type whose single elements have a one-buffer Arrow layout, so a
/// [`Scalar`](crate::Scalar) of it can be validated at construction.
///
/// Fixed-width types require exactly their native width (aligned for
/// zero-copy typed reads); variable-size types validate their payload
/// (UTF-8 for strings). A boolean scalar is one byte holding 0 or 1 — the
/// bit-packing of the Arrow spec applies to arrays, not to a single detached
/// element. Nested types gain scalar layouts together with the array views
/// that back them.
///
/// ```
/// use yggdryl_scalar::ScalarType;
/// use yggdryl_schema::Int32;
///
/// assert!(Int32.validate_scalar_bytes(&7i32.to_le_bytes()).is_ok());
/// assert!(Int32.validate_scalar_bytes(&[0u8; 3]).is_err());
/// ```
pub trait ScalarType: DataType {
    /// Validates one element's value bytes against this type's layout.
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError>;
}

/// Implements [`ScalarType`] for a fixed-width type — the value is exactly
/// one native, aligned so typed reads stay zero-copy. A leading `[generics]`
/// clause covers the unit-generic temporal types.
macro_rules! fixed_width_scalar_type {
    (@impl [$($generics:tt)*] $name:ty) => {
        impl<$($generics)*> ScalarType for $name {
            fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
                let expected = size_of::<<$name as PrimitiveType>::Native>();
                if bytes.len() != expected {
                    return Err(ScalarError::InvalidByteLength {
                        expected,
                        actual: bytes.len(),
                    });
                }
                let align = align_of::<<$name as PrimitiveType>::Native>();
                if !(bytes.as_ptr() as usize).is_multiple_of(align) {
                    return Err(ScalarError::MisalignedBuffer { align });
                }
                Ok(())
            }
        }
    };
    ($($name:ty),+ $(,)?) => {
        $(fixed_width_scalar_type!(@impl [] $name);)+
    };
}

fixed_width_scalar_type!(
    Int8, Int16, Int32, Int64, UInt8, UInt16, UInt32, UInt64, Float32, Float64, Decimal128,
    Decimal256, Date32, Date64,
);
fixed_width_scalar_type!(@impl [U: TimeUnit] TypedTimestamp<U>);
fixed_width_scalar_type!(@impl [U: TimeUnit] TypedDuration<U>);
fixed_width_scalar_type!(@impl [U: Time32Unit] Time32<U>);
fixed_width_scalar_type!(@impl [U: Time64Unit] Time64<U>);

impl ScalarType for Boolean {
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
        match bytes {
            [0 | 1] => Ok(()),
            [other] => Err(ScalarError::InvalidBoolean { value: *other }),
            _ => Err(ScalarError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
            }),
        }
    }
}

impl ScalarType for Utf8 {
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
        str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|_| ScalarError::InvalidUtf8)
    }
}

impl ScalarType for LargeUtf8 {
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
        str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|_| ScalarError::InvalidUtf8)
    }
}

impl ScalarType for Binary {
    fn validate_scalar_bytes(&self, _bytes: &[u8]) -> Result<(), ScalarError> {
        Ok(())
    }
}

impl ScalarType for LargeBinary {
    fn validate_scalar_bytes(&self, _bytes: &[u8]) -> Result<(), ScalarError> {
        Ok(())
    }
}

impl ScalarType for FixedSizeBinary {
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
        let expected = usize::try_from(self.size()).expect("validated non-negative");
        if bytes.len() == expected {
            Ok(())
        } else {
            Err(ScalarError::InvalidByteLength {
                expected,
                actual: bytes.len(),
            })
        }
    }
}
