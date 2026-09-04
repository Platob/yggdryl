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

typed_array!(crate::types::boolean::Null, arrow_array::NullArray);
typed_array!(crate::types::boolean::Boolean, arrow_array::BooleanArray);
typed_array!(crate::types::integer::Int8, arrow_array::Int8Array);
typed_array!(crate::types::integer::Int16, arrow_array::Int16Array);
typed_array!(crate::types::integer::Int32, arrow_array::Int32Array);
typed_array!(crate::types::integer::Int64, arrow_array::Int64Array);
typed_array!(crate::types::integer::UInt8, arrow_array::UInt8Array);
typed_array!(crate::types::integer::UInt16, arrow_array::UInt16Array);
typed_array!(crate::types::integer::UInt32, arrow_array::UInt32Array);
typed_array!(crate::types::integer::UInt64, arrow_array::UInt64Array);
typed_array!(crate::types::floating::Float16, arrow_array::Float16Array);
typed_array!(crate::types::floating::Float32, arrow_array::Float32Array);
typed_array!(crate::types::floating::Float64, arrow_array::Float64Array);
typed_array!(crate::types::temporal::Date32, arrow_array::Date32Array);
typed_array!(crate::types::temporal::Date64, arrow_array::Date64Array);
typed_array!(
    crate::types::decimal::Decimal32,
    arrow_array::Decimal32Array
);
typed_array!(
    crate::types::decimal::Decimal64,
    arrow_array::Decimal64Array
);
typed_array!(
    crate::types::decimal::Decimal128,
    arrow_array::Decimal128Array
);
typed_array!(
    crate::types::decimal::Decimal256,
    arrow_array::Decimal256Array
);
typed_array!(crate::types::bytes::Binary, arrow_array::BinaryArray);
typed_array!(
    crate::types::bytes::LargeBinary,
    arrow_array::LargeBinaryArray
);
typed_array!(
    crate::types::bytes::BinaryView,
    arrow_array::BinaryViewArray
);
typed_array!(
    crate::types::bytes::FixedSizeBinary,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(crate::types::text::Utf8, arrow_array::StringArray);
typed_array!(crate::types::text::LargeUtf8, arrow_array::LargeStringArray);
typed_array!(crate::types::text::Utf8View, arrow_array::StringViewArray);
// Variable ASCII stores as binary; a fixed width as fixed binary.
typed_array!(crate::types::ascii::Ascii, arrow_array::BinaryArray);
typed_array!(
    crate::types::ascii::FixedAscii,
    arrow_array::FixedSizeBinaryArray
);
// A registered code stores as the fixed binary its standard fixes.
typed_array!(
    crate::types::ascii::Country,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(
    crate::types::ascii::Currency,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(crate::types::ascii::Mic, arrow_array::FixedSizeBinaryArray);
typed_array!(crate::types::ascii::Cfi, arrow_array::FixedSizeBinaryArray);
// A GUID stores as the fixed binary of its sixteen bytes.
typed_array!(crate::types::guid::Guid, arrow_array::FixedSizeBinaryArray);
typed_array!(crate::types::nested::List, arrow_array::ListArray);
typed_array!(crate::types::nested::ListView, arrow_array::ListViewArray);
typed_array!(crate::types::nested::LargeList, arrow_array::LargeListArray);
typed_array!(
    crate::types::nested::LargeListView,
    arrow_array::LargeListViewArray
);
typed_array!(
    crate::types::nested::FixedSizeList,
    arrow_array::FixedSizeListArray
);
typed_array!(crate::types::nested::Struct, arrow_array::StructArray);
typed_array!(crate::types::nested::Union, arrow_array::UnionArray);
typed_array!(crate::types::nested::Map, arrow_array::MapArray);
// A variant's storage is the canonical struct of two required binaries, and a
// geospatial value is its WKB payload, so their physical arrays are fixed.
typed_array!(crate::types::nested::Variant, arrow_array::StructArray);
typed_array!(crate::types::geospatial::Geometry, arrow_array::BinaryArray);
typed_array!(
    crate::types::geospatial::Geography,
    arrow_array::BinaryArray
);

// A unit decides the physical width of a temporal value, and a key type decides
// the physical width of a dictionary index, so these have no single array type.
opaque_array!(crate::types::temporal::Timestamp);
opaque_array!(crate::types::temporal::Time32);
opaque_array!(crate::types::temporal::Time64);
opaque_array!(crate::types::temporal::Duration32);
opaque_array!(crate::types::temporal::Duration64);
opaque_array!(crate::types::temporal::Interval);
opaque_array!(crate::types::nested::Dictionary);
opaque_array!(crate::types::nested::RunEndEncoded);

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
