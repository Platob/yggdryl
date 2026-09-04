//! Casting an Arrow array into the exact array a typed field describes.
//!
//! [`ArrowCast`] answers "make this array fit that field" for any
//! field, and returns an [`ArrayRef`] because any field could be any datatype.
//! A [`TypedField`] already knows its variant, so it can answer with the array
//! type itself: [`Int64Field`](crate::field::Int64Field) casts to an
//! [`Int64Array`](arrow_array::Int64Array), and the caller reads values without
//! a downcast of its own.
//!
//! The field is always the *target*: an incoming array is reconciled to the
//! field's datatype and nullability, never the other way around.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, StringArray};
//! use yggdryl::field::Int64Field;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Int64Field::new("id", false);
//! let source: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));
//!
//! // The result is an Int64Array, not an ArrayRef needing a downcast.
//! let ids = field.cast_arrow_array(source, false)?;
//! assert_eq!(ids.values(), &[1, 2]);
//! # Ok(())
//! # }
//! ```
//!
//! A few datatypes carry a parameter that decides their physical array -
//! a timestamp's unit, a dictionary's key type - so those cast to an
//! [`ArrayRef`]. Every other variant casts to its concrete array.

use arrow_array::{Array, ArrayRef, RecordBatch, Scalar};

use super::Field;
use super::typed::{FieldType, TypedField, TypedFieldRef};
use crate::arrow::{Error, Result, arrow_schema_from_field};

mod plan;

pub use plan::ArrowCast;
pub(crate) use plan::{cast_field_array, cast_record_batch};

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

typed_array!(super::scalar::Null, arrow_array::NullArray);
typed_array!(super::scalar::Boolean, arrow_array::BooleanArray);
typed_array!(super::integer::Int8, arrow_array::Int8Array);
typed_array!(super::integer::Int16, arrow_array::Int16Array);
typed_array!(super::integer::Int32, arrow_array::Int32Array);
typed_array!(super::integer::Int64, arrow_array::Int64Array);
typed_array!(super::integer::UInt8, arrow_array::UInt8Array);
typed_array!(super::integer::UInt16, arrow_array::UInt16Array);
typed_array!(super::integer::UInt32, arrow_array::UInt32Array);
typed_array!(super::integer::UInt64, arrow_array::UInt64Array);
typed_array!(super::floating::Float16, arrow_array::Float16Array);
typed_array!(super::floating::Float32, arrow_array::Float32Array);
typed_array!(super::floating::Float64, arrow_array::Float64Array);
typed_array!(super::temporal::Date32, arrow_array::Date32Array);
typed_array!(super::temporal::Date64, arrow_array::Date64Array);
typed_array!(super::decimal::Decimal32, arrow_array::Decimal32Array);
typed_array!(super::decimal::Decimal64, arrow_array::Decimal64Array);
typed_array!(super::decimal::Decimal128, arrow_array::Decimal128Array);
typed_array!(super::decimal::Decimal256, arrow_array::Decimal256Array);
typed_array!(super::binary::Binary, arrow_array::BinaryArray);
typed_array!(super::binary::LargeBinary, arrow_array::LargeBinaryArray);
typed_array!(super::binary::BinaryView, arrow_array::BinaryViewArray);
typed_array!(
    super::binary::FixedSizeBinary,
    arrow_array::FixedSizeBinaryArray
);
typed_array!(super::binary::Utf8, arrow_array::StringArray);
typed_array!(super::binary::LargeUtf8, arrow_array::LargeStringArray);
typed_array!(super::binary::Utf8View, arrow_array::StringViewArray);
// An ASCII width stores as the fixed binary of its width.
typed_array!(super::ascii::Ascii16, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Ascii24, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Ascii32, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Ascii64, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Ascii96, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Ascii128, arrow_array::FixedSizeBinaryArray);
// A registered code stores as the fixed binary its standard fixes.
typed_array!(super::ascii::Country, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Currency, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Mic, arrow_array::FixedSizeBinaryArray);
typed_array!(super::ascii::Cfi, arrow_array::FixedSizeBinaryArray);
// A GUID stores as the fixed binary of its sixteen bytes.
typed_array!(super::nested::Guid, arrow_array::FixedSizeBinaryArray);
typed_array!(super::nested::List, arrow_array::ListArray);
typed_array!(super::nested::ListView, arrow_array::ListViewArray);
typed_array!(super::nested::LargeList, arrow_array::LargeListArray);
typed_array!(
    super::nested::LargeListView,
    arrow_array::LargeListViewArray
);
typed_array!(
    super::nested::FixedSizeList,
    arrow_array::FixedSizeListArray
);
typed_array!(super::nested::Struct, arrow_array::StructArray);
typed_array!(super::nested::Union, arrow_array::UnionArray);
typed_array!(super::nested::Map, arrow_array::MapArray);
// A variant's storage is the canonical struct of two required binaries, and a
// geospatial value is its WKB payload, so their physical arrays are fixed.
typed_array!(super::nested::Variant, arrow_array::StructArray);
typed_array!(super::geospatial::Geometry, arrow_array::BinaryArray);
typed_array!(super::geospatial::Geography, arrow_array::BinaryArray);

// A unit decides the physical width of a temporal value, and a key type decides
// the physical width of a dictionary index, so these have no single array type.
opaque_array!(super::temporal::Timestamp);
opaque_array!(super::temporal::Time32);
opaque_array!(super::temporal::Time64);
opaque_array!(super::temporal::Duration32);
opaque_array!(super::temporal::Duration64);
opaque_array!(super::temporal::Interval);
opaque_array!(super::nested::Dictionary);
opaque_array!(super::nested::RunEndEncoded);

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

/// Validates an exact source batch against one declared Struct root Field.
///
/// This low-level hook is public for native runtime bindings that already own
/// Arrow arrays but intentionally hidden from their user-facing APIs. It uses
/// IPC-compatible schema comparison and rejects recursive logical values that
/// would require default filling or another canonical repair.
///
/// # Errors
///
/// Returns an error unless `field` is a valid non-null Struct root and the
/// batch has a compatible schema plus valid recursive values.
#[doc(hidden)]
pub fn validate_arrow_batch(field: &Field, batch: &RecordBatch) -> Result<()> {
    field.validate_bounded()?;
    let root_field = field.clone();
    root_field.validate_struct_root()?;

    // Valid means "needs no repair": casting an exact batch returns the very
    // arrays it was given, so a changed column is a validation failure.
    let cast = plan::cast_record_batch(field, batch.clone(), true)?;
    for (index, (before, after)) in batch.columns().iter().zip(cast.columns()).enumerate() {
        if !std::sync::Arc::ptr_eq(before, after) {
            let name = batch.schema().field(index).name().clone();
            return Err(Error::IncompatibleSchema(format!(
                "field {name:?} requires canonical repair and is not valid as stored"
            )));
        }
    }
    Ok(())
}

/// Preflights an empty source-to-target batch cast for runtime readers.
///
/// This binding hook validates both Struct roots and constructs the recursive
/// cast plan from the canonical source schema without materializing arrays. It
/// lets an empty backend reject an invalid target before a lazy checked read is
/// attempted, without maintaining another schema table.
///
/// # Errors
///
/// Returns an error when either root Field is invalid/nullable/non-Struct or
/// the recursive source-to-target cast plan cannot be constructed.
#[doc(hidden)]
pub fn preflight_arrow_batch_cast(
    source: &Field,
    target: Option<&Field>,
    safe: bool,
) -> Result<()> {
    let schema = arrow_schema_from_field(source)?;
    let target = target.unwrap_or(source);
    // An empty batch of the source schema exercises the whole recursive plan
    // without materializing a row.
    let empty = RecordBatch::new_empty(schema);
    plan::cast_record_batch(target, empty, safe)?;
    Ok(())
}

#[cfg(test)]
mod tests;
