//! Typed-field Arrow array projections.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, Scalar, UInt32Array, UInt64Array};
use arrow_buffer::ScalarBuffer;

use super::ArrowCast as _;
use crate::arrow::{Error, Result};
use crate::types::typed::{FieldType, TypedField, TypedFieldRef};
use crate::{DataType, Field};

/// The Arrow array a field's values materialize into.
///
/// Implemented for every [`FieldType`] marker. A variant whose physical array
/// depends on a datatype parameter reports [`ArrayRef`], because there is no
/// single concrete type to name.
pub trait ArrowFieldType: FieldType {
    /// The array produced by casting to this field's datatype.
    type Array: Array + Clone + 'static;

    /// Narrow a cast array to this marker's array type.
    ///
    /// # Errors
    ///
    /// Returns an error when the cast produced a different physical array,
    /// which would mean the cast engine and this table disagree.
    fn downcast_array(array: ArrayRef) -> Result<Self::Array>;
}

/// Narrow one cast array, naming both sides when the narrowing fails.
fn downcast<A: Array + Clone + 'static>(array: ArrayRef, expected: &'static str) -> Result<A> {
    array.as_any().downcast_ref::<A>().cloned().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "expected a cast to produce an Arrow {expected} array, got {}",
            array.data_type()
        ))
    })
}

/// Bind a marker to the concrete Arrow array its datatype materializes into.
macro_rules! typed_array {
    ($marker:path, $array:ty) => {
        impl ArrowFieldType for $marker {
            type Array = $array;

            fn downcast_array(array: ArrayRef) -> Result<Self::Array> {
                downcast(array, stringify!($array))
            }
        }
    };
}

/// Bind a marker whose physical array depends on a datatype parameter.
macro_rules! opaque_array {
    ($marker:path) => {
        impl ArrowFieldType for $marker {
            type Array = ArrayRef;

            fn downcast_array(array: ArrayRef) -> Result<Self::Array> {
                Ok(array)
            }
        }
    };
}

typed_array!(crate::types::boolean::NullType, arrow_array::NullArray);
typed_array!(
    crate::types::boolean::BooleanType,
    arrow_array::BooleanArray
);
typed_array!(crate::types::integer::Int8Type, arrow_array::Int8Array);
typed_array!(crate::types::integer::Int16Type, arrow_array::Int16Array);
typed_array!(crate::types::integer::Int32Type, arrow_array::Int32Array);
typed_array!(crate::types::integer::Int64Type, arrow_array::Int64Array);
typed_array!(crate::types::integer::UInt8Type, arrow_array::UInt8Array);
typed_array!(crate::types::integer::UInt16Type, arrow_array::UInt16Array);
typed_array!(crate::types::integer::UInt32Type, arrow_array::UInt32Array);
typed_array!(crate::types::integer::UInt64Type, arrow_array::UInt64Array);
typed_array!(
    crate::types::floating::Float16Type,
    arrow_array::Float16Array
);
typed_array!(
    crate::types::floating::Float32Type,
    arrow_array::Float32Array
);
typed_array!(
    crate::types::floating::Float64Type,
    arrow_array::Float64Array
);
typed_array!(crate::types::temporal::Date32Type, arrow_array::Date32Array);
typed_array!(crate::types::temporal::Date64Type, arrow_array::Date64Array);
typed_array!(
    crate::types::decimal::Decimal32Type,
    arrow_array::Decimal32Array
);
typed_array!(
    crate::types::decimal::Decimal64Type,
    arrow_array::Decimal64Array
);
typed_array!(
    crate::types::decimal::Decimal128Type,
    arrow_array::Decimal128Array
);
typed_array!(
    crate::types::decimal::Decimal256Type,
    arrow_array::Decimal256Array
);
typed_array!(crate::types::bytes::BinaryType, arrow_array::BinaryArray);
typed_array!(
    crate::types::bytes::LargeBinaryType,
    arrow_array::LargeBinaryArray
);
typed_array!(
    crate::types::bytes::BinaryViewType,
    arrow_array::BinaryViewArray
);
typed_array!(
    crate::types::bytes::FixedSizeBinaryType,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(crate::types::text::Utf8Type, arrow_array::StringArray);
typed_array!(crate::types::version::VersionType, arrow_array::StringArray);
typed_array!(
    crate::types::text::LargeUtf8Type,
    arrow_array::LargeStringArray
);
typed_array!(
    crate::types::text::Utf8ViewType,
    arrow_array::StringViewArray
);
// Variable ASCII stores as binary; a fixed width as fixed binary.
typed_array!(crate::types::ascii::AsciiType, arrow_array::BinaryArray);
typed_array!(
    crate::types::ascii::FixedAsciiType,
    arrow_array::FixedSizeBinaryArray
);
// A registered code stores as the fixed binary its standard fixes.
typed_array!(
    crate::types::ascii::CountryType,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(
    crate::types::ascii::CurrencyType,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(
    crate::types::ascii::MicType,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(
    crate::types::ascii::CfiType,
    arrow_array::FixedSizeBinaryArray
);
// A UUID stores as the fixed binary of its sixteen bytes.
typed_array!(
    crate::types::uuid::UuidType,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(crate::types::nested::ListType, arrow_array::ListArray);
typed_array!(
    crate::types::nested::ListViewType,
    arrow_array::ListViewArray
);
typed_array!(
    crate::types::nested::LargeListType,
    arrow_array::LargeListArray
);
typed_array!(
    crate::types::nested::LargeListViewType,
    arrow_array::LargeListViewArray
);
typed_array!(
    crate::types::nested::FixedSizeListType,
    arrow_array::FixedSizeListArray
);
typed_array!(crate::types::nested::StructType, arrow_array::StructArray);
typed_array!(crate::types::nested::UnionType, arrow_array::UnionArray);
typed_array!(crate::types::nested::MapTypeMarker, arrow_array::MapArray);
// A variant's storage is the canonical struct of two required binaries, and a
// geospatial value is its WKB payload, so their physical arrays are fixed.
typed_array!(crate::types::nested::VariantType, arrow_array::StructArray);
typed_array!(
    crate::types::geospatial::GeometryType,
    arrow_array::BinaryArray
);
typed_array!(
    crate::types::geospatial::GeographyType,
    arrow_array::BinaryArray
);

// A unit decides the physical width of a temporal value, and a key type decides
// the physical width of a dictionary index, so these have no single array type.
opaque_array!(crate::types::temporal::DateTime64Type);
opaque_array!(crate::types::temporal::Time32Type);
opaque_array!(crate::types::temporal::Time64Type);
opaque_array!(crate::types::temporal::Duration32Type);
opaque_array!(crate::types::temporal::Duration64Type);
opaque_array!(crate::types::temporal::IntervalType);
opaque_array!(crate::types::nested::DictionaryTypeMarker);
opaque_array!(crate::types::nested::RunEndEncodedTypeMarker);

impl Field {
    /// Casts between same-width signed and unsigned Arrow integers by bits.
    ///
    /// This is deliberately separate from [`ArrowCast::cast_arrow_array`],
    /// which preserves the numeric value and therefore rejects values outside
    /// the target integer's range. A bit cast instead maps every bit pattern:
    /// `u32::MAX` becomes `-1_i32`, and the reverse cast restores `u32::MAX`.
    /// Only `uint32` <-> `int32` and `uint64` <-> `int64` are accepted.
    ///
    /// The value buffer is shared without copying unless a required target
    /// must replace source nulls with its canonical default. A nullable target
    /// retains nulls.
    ///
    /// # Errors
    ///
    /// Returns an error when this Field is not one of the four supported
    /// integer targets, when `array` is not its opposite-signed counterpart,
    /// or when the target's null/default contract cannot be satisfied.
    pub fn cast_arrow_array_bits(&self, array: ArrayRef) -> Result<ArrayRef> {
        self.validate_bounded()?;
        let cast: ArrayRef = match self.dtype() {
            DataType::Int32 => {
                let source = opposite_array::<UInt32Array>(self, array.as_ref(), "uint32")?;
                let values = ScalarBuffer::<i32>::from(source.values().inner().clone());
                Arc::new(Int32Array::new(values, source.nulls().cloned()))
            }
            DataType::UInt32 => {
                let source = opposite_array::<Int32Array>(self, array.as_ref(), "int32")?;
                let values = ScalarBuffer::<u32>::from(source.values().inner().clone());
                Arc::new(UInt32Array::new(values, source.nulls().cloned()))
            }
            DataType::Int64 => {
                let source = opposite_array::<UInt64Array>(self, array.as_ref(), "uint64")?;
                let values = ScalarBuffer::<i64>::from(source.values().inner().clone());
                Arc::new(Int64Array::new(values, source.nulls().cloned()))
            }
            DataType::UInt64 => {
                let source = opposite_array::<Int64Array>(self, array.as_ref(), "int64")?;
                let values = ScalarBuffer::<u64>::from(source.values().inner().clone());
                Arc::new(UInt64Array::new(values, source.nulls().cloned()))
            }
            dtype => {
                return Err(Error::IncompatibleSchema(format!(
                    "field {:?} bit-preserving Arrow integer casts require an int32, uint32, int64, or uint64 target, got {} for source {}",
                    self.name(),
                    dtype,
                    array.data_type()
                )));
            }
        };
        self.cast_arrow_array(cast, false)
    }
}

fn opposite_array<'array, A: Array + 'static>(
    field: &Field,
    array: &'array dyn Array,
    expected: &'static str,
) -> Result<&'array A> {
    array.as_any().downcast_ref::<A>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "field {:?} bit-preserving Arrow cast to {} requires a {expected} array, got {}",
            field.name(),
            field.dtype(),
            array.data_type()
        ))
    })
}

macro_rules! typed_bit_cast {
    ($marker:path, $array:ty) => {
        impl TypedField<$marker> {
            /// Casts the opposite-signed same-width Arrow integer array by bits.
            ///
            /// Every bit pattern is accepted. The value buffer is shared unless
            /// a required target must fill nulls with its canonical default.
            ///
            /// # Errors
            ///
            /// Returns any error [`Field::cast_arrow_array_bits`] returns.
            pub fn cast_arrow_array_bits(&self, array: ArrayRef) -> Result<$array> {
                downcast(
                    self.as_field().cast_arrow_array_bits(array)?,
                    stringify!($array),
                )
            }
        }

        impl TypedFieldRef<'_, $marker> {
            /// Casts the opposite-signed same-width Arrow integer array by bits.
            ///
            /// # Errors
            ///
            /// Returns any error [`Field::cast_arrow_array_bits`] returns.
            pub fn cast_arrow_array_bits(&self, array: ArrayRef) -> Result<$array> {
                downcast(
                    self.as_field().cast_arrow_array_bits(array)?,
                    stringify!($array),
                )
            }
        }
    };
}

typed_bit_cast!(crate::types::integer::Int32Type, Int32Array);
typed_bit_cast!(crate::types::integer::UInt32Type, UInt32Array);
typed_bit_cast!(crate::types::integer::Int64Type, Int64Array);
typed_bit_cast!(crate::types::integer::UInt64Type, UInt64Array);

impl<K: ArrowFieldType> TypedField<K> {
    /// Cast an incoming Arrow array to this field, returning its exact array.
    ///
    /// The field is the target: `array` is reconciled to the field's datatype
    /// and nullability. `safe` is Arrow's cast option - conversion failures
    /// become null when it is true and errors when it is false - and a
    /// non-nullable field then replaces any resulting null with its canonical
    /// default.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported cast, a value that cannot satisfy
    /// the field, or a default that cannot be materialized.
    pub fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<K::Array> {
        K::downcast_array(self.as_field().cast_arrow_array(array, safe)?)
    }

    /// Cast a one-element Arrow array to this field as a typed scalar.
    ///
    /// # Errors
    ///
    /// Returns an error when `array` does not hold exactly one value, or any
    /// error [`Self::cast_arrow_array`] returns.
    pub fn cast_arrow_scalar(&self, array: ArrayRef, safe: bool) -> Result<Scalar<K::Array>> {
        if array.len() != 1 {
            return Err(Error::IncompatibleSchema(format!(
                "expected exactly 1 value to cast as a scalar, got {}",
                array.len()
            )));
        }
        Ok(Scalar::new(self.cast_arrow_array(array, safe)?))
    }
}

impl<K: ArrowFieldType> TypedFieldRef<'_, K> {
    /// Cast an incoming Arrow array to the borrowed field's exact array.
    ///
    /// # Errors
    ///
    /// Returns any error [`TypedField::cast_arrow_array`] returns.
    pub fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<K::Array> {
        K::downcast_array(self.as_field().cast_arrow_array(array, safe)?)
    }
}
