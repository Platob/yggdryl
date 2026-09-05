//! Typed-field Arrow array projections.

use arrow_array::{Array, ArrayRef, Scalar};

use super::ArrowCast as _;
use crate::arrow::{Error, Result};
use crate::types::typed::{FieldType, TypedField, TypedFieldRef};

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
