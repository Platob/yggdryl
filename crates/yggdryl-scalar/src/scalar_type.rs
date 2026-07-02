//! The subtrait tying a data type to its one-element Arrow layout.

use core::mem::{align_of, size_of};
use core::str;

use yggdryl_schema::{
    BinaryType, BooleanType, DataType, Date32Type, Date64Type, Decimal128Type, Decimal256Type,
    DurationType, FixedSizeBinaryType, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type,
    Int8Type, LargeBinaryType, LargeUtf8Type, PrimitiveType, Time32Type, Time32Unit, Time64Type,
    Time64Unit, TimeUnit, TimestampType, UInt16Type, UInt32Type, UInt64Type, UInt8Type, Utf8Type,
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
/// use yggdryl_schema::Int32Type;
///
/// assert!(Int32Type.validate_scalar_bytes(&7i32.to_le_bytes()).is_ok());
/// assert!(Int32Type.validate_scalar_bytes(&[0u8; 3]).is_err());
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
    Int8Type,
    Int16Type,
    Int32Type,
    Int64Type,
    UInt8Type,
    UInt16Type,
    UInt32Type,
    UInt64Type,
    Float32Type,
    Float64Type,
    Decimal128Type,
    Decimal256Type,
    Date32Type,
    Date64Type,
);
fixed_width_scalar_type!(@impl [U: TimeUnit] TimestampType<U>);
fixed_width_scalar_type!(@impl [U: TimeUnit] DurationType<U>);
fixed_width_scalar_type!(@impl [U: Time32Unit] Time32Type<U>);
fixed_width_scalar_type!(@impl [U: Time64Unit] Time64Type<U>);

impl ScalarType for BooleanType {
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

impl ScalarType for Utf8Type {
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
        str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|_| ScalarError::InvalidUtf8)
    }
}

impl ScalarType for LargeUtf8Type {
    fn validate_scalar_bytes(&self, bytes: &[u8]) -> Result<(), ScalarError> {
        str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|_| ScalarError::InvalidUtf8)
    }
}

impl ScalarType for BinaryType {
    fn validate_scalar_bytes(&self, _bytes: &[u8]) -> Result<(), ScalarError> {
        Ok(())
    }
}

impl ScalarType for LargeBinaryType {
    fn validate_scalar_bytes(&self, _bytes: &[u8]) -> Result<(), ScalarError> {
        Ok(())
    }
}

impl ScalarType for FixedSizeBinaryType {
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
