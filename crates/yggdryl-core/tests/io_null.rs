//! The **null type** — Arrow's 0-width `Null`: [`NullType`] / [`NullField`] / [`NullScalar`] /
//! [`NullSerie`]. Every value is null, so the descriptor is 0-width, a scalar's wire form is
//! empty, and a column is just its length.

use yggdryl_core::io::fixed::{NullField, NullScalar, NullSerie, NullType};
use yggdryl_core::io::{
    Bytes, DataType, DataTypeCategory, DataTypeId, FieldType, IOCursor, ScalarType, SerieType,
};

#[test]
fn null_type_descriptor_is_zero_width() {
    let dt = NullType::new();
    assert_eq!(dt.name(), "null");
    assert_eq!(dt.byte_width(), 0);
    assert_eq!(dt.type_id(), DataTypeId::Null);
    assert_eq!(dt.category(), DataTypeCategory::Null);
    // Null is its own category — neither numeric/temporal nor binary/utf8.
    assert!(!dt.is_numeric() && !dt.is_temporal() && !dt.is_binary() && !dt.is_utf8());
    assert_eq!(NullType::NAME, "null");
    assert_eq!(NullType::BYTE_WIDTH, 0);
}

#[test]
fn null_field_is_always_nullable() {
    let field = NullField::new("maybe").with_metadata_entry("note", "always null");
    assert_eq!(FieldType::name(&field), "maybe");
    assert_eq!(field.type_name(), "null");
    assert_eq!(field.byte_width(), 0);
    assert!(field.nullable()); // a null column is all-null
    assert_eq!(FieldType::type_id(&field), DataTypeId::Null);
    assert_eq!(field.metadata().get("note"), Some("always null"));
    // Erases to a runtime `Field`, metadata preserved.
    let erased = field.erase();
    assert_eq!(FieldType::type_id(&erased), DataTypeId::Null);
    assert_eq!(erased.metadata().get("note"), Some("always null"));
}

#[test]
fn null_scalar_is_a_single_value() {
    let a = NullScalar::null();
    let b = NullScalar::new();
    // Every null scalar is equal and hashes the same — usable as a map key.
    assert_eq!(a, b);
    assert!(a.is_null() && !a.is_valid());
    assert!(ScalarType::is_null(&a));
    assert_eq!(a.data_type().type_id(), DataTypeId::Null);
    // Empty wire form, round-trips.
    assert!(a.serialize_bytes().is_empty());
    assert_eq!(NullScalar::deserialize_bytes(&[]), a);
    assert_eq!(NullScalar::serialized_width(), 0);
    let mut sink = Bytes::new();
    a.write_to(&mut sink).unwrap(); // writes nothing
    sink.rewind();
    assert_eq!(NullScalar::read_from(&mut sink).unwrap(), a);
    let set = std::collections::HashSet::from([NullScalar::null(), NullScalar::new()]);
    assert_eq!(set.len(), 1);
}

#[test]
fn null_serie_is_a_run_of_nulls() {
    let mut col = NullSerie::with_len(2);
    col.push();
    col.extend(2);
    assert_eq!(col.len(), 5);
    assert_eq!(col.null_count(), 5);
    assert!(col.has_nulls() && !col.is_empty());
    assert_eq!(SerieType::get(&col, 0), None); // every element is null
    assert_eq!(SerieType::len(&col), 5);
    assert_eq!(col.get_scalar(3), NullScalar::null());
    assert_eq!(col.to_field("x").type_id(), DataTypeId::Null);
    // Round-trips through a byte sink (just the length).
    let mut sink = Bytes::new();
    col.write_to(&mut sink).unwrap();
    sink.rewind();
    assert_eq!(NullSerie::read_from(&mut sink).unwrap(), col);
    assert_eq!(NullSerie::new().len(), 0);
}

#[cfg(feature = "arrow")]
#[test]
fn null_arrow_interop() {
    use arrow_array::Array;

    // Descriptor maps to Arrow `Null`.
    assert_eq!(NullType::new().to_arrow(), arrow_schema::DataType::Null);

    // Column <-> Arrow `NullArray`.
    let col = NullSerie::with_len(4);
    let array = col.to_arrow_array();
    assert_eq!(array.len(), 4);
    assert_eq!(array.data_type(), &arrow_schema::DataType::Null);
    assert_eq!(NullSerie::from_arrow_array(&array), col);

    // Field round-trips through Arrow, preserving metadata.
    let field = NullField::new("n").with_metadata_entry("k", "v");
    let arrow_field = field.to_arrow();
    assert_eq!(arrow_field.data_type(), &arrow_schema::DataType::Null);
    let back = NullField::from_arrow(&arrow_field).unwrap();
    assert_eq!(back, field);
}
