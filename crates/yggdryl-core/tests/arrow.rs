//! Apache Arrow **leaf** interop round-trips (feature `arrow`).
//!
//! Each leaf column is converted `Column -> Arrow array` (asserted through Arrow's own accessors)
//! and back `Arrow array -> Column` (values + nulls preserved), plus the `DataTypeId <-> DataType`
//! and `HeaderField <-> Field` maps and a sliced-input round-trip.
#![cfg(feature = "arrow")]

use arrow_array::{
    Array, BinaryArray, BooleanArray, Decimal128Array, FixedSizeBinaryArray, Float64Array,
    Int32Array, Int64Array, StringArray,
};
use arrow_schema::{DataType, Field as ArrowField};

use yggdryl_core::arrow::{
    column_from_arrow, column_to_arrow, from_arrow_data_type, from_arrow_field, to_arrow_data_type,
    to_arrow_field,
};
use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::typed::fixedbyte::{Decimal128, FixedBinary, Float64, Int32, Int64};
use yggdryl_core::typed::varbyte::{Binary, Utf8};
use yggdryl_core::typed::{
    Column, Field, FixedSerie, FixedSizeSerie, HeaderField, StructSerie, Value, VarSerie,
};

// ---- B. Column <-> Arrow array leaf round-trips ----------------------------------------------

#[test]
fn int64_column_round_trips() {
    // NOTE: to-Arrow copies the 24 value bytes once (borrowed `&Column`); from-Arrow re-encodes once.
    let column = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let ints = array.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(ints.len(), 3);
    assert_eq!(ints.values(), &[1, 2, 3]);
    assert_eq!(ints.null_count(), 0);

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.len(), 3);
    assert_eq!(back.get(0), Value::Int64(1));
    assert_eq!(back.get(2), Value::Int64(3));
    assert_eq!(back.null_count(), 0);
}

#[test]
fn nullable_int32_column_preserves_nulls() {
    let column = Column::from(FixedSerie::<Int32>::from_options(&[
        Some(10),
        None,
        Some(30),
    ]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let ints = array.as_any().downcast_ref::<Int32Array>().unwrap();
    assert_eq!(ints.len(), 3);
    assert_eq!(ints.null_count(), 1);
    assert!(ints.is_valid(0));
    assert!(ints.is_null(1));
    assert_eq!(ints.value(0), 10);
    assert_eq!(ints.value(2), 30);

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(0), Value::Int32(10));
    assert_eq!(back.get(1), Value::Null);
    assert_eq!(back.get(2), Value::Int32(30));
    assert!(back.is_null(1));
    assert_eq!(back.null_count(), 1);
}

#[test]
fn float64_column_round_trips() {
    let column = Column::from(FixedSerie::<Float64>::from_values(&[1.5, -2.25, 3.0]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let floats = array.as_any().downcast_ref::<Float64Array>().unwrap();
    assert_eq!(floats.values(), &[1.5, -2.25, 3.0]);

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(1), Value::Float64(-2.25));
}

#[test]
fn bool_column_round_trips() {
    use yggdryl_core::typed::fixedbit::Bit;
    let column = Column::from(FixedSerie::<Bit>::from_options(&[
        Some(true),
        Some(false),
        None,
        Some(true),
    ]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let bools = array.as_any().downcast_ref::<BooleanArray>().unwrap();
    assert_eq!(bools.len(), 4);
    assert!(bools.value(0));
    assert!(!bools.value(1));
    assert!(bools.is_null(2));
    assert!(bools.value(3));

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(0), Value::Bool(true));
    assert_eq!(back.get(1), Value::Bool(false));
    assert_eq!(back.get(2), Value::Null);
    assert_eq!(back.get(3), Value::Bool(true));
}

#[test]
fn decimal128_column_preserves_precision_scale() {
    // Unscaled i128 values with precision 10, scale 2 (e.g. 123.45 == 12345 unscaled).
    let column = Column::from(
        FixedSerie::<Decimal128>::from_values(&[12345, -678, 0]).with_precision_scale(10, 2),
    );
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let decimals = array.as_any().downcast_ref::<Decimal128Array>().unwrap();
    assert_eq!(decimals.precision(), 10);
    assert_eq!(decimals.scale(), 2);
    assert_eq!(decimals.value(0), 12345);
    assert_eq!(decimals.value(1), -678);
    assert_eq!(array.data_type(), &DataType::Decimal128(10, 2));

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(0), Value::Decimal128(12345));
    assert_eq!(back.get(1), Value::Decimal128(-678));
    // precision/scale restored on the rebuilt field.
    let back_field = back.field();
    assert_eq!(back_field.data_type_id(), DataTypeId::Decimal128);
}

#[test]
fn utf8_column_with_multibyte_and_null_round_trips() {
    // A multibyte value ("héllo" — the 'é' is 2 bytes) and a null.
    let column = Column::from(VarSerie::<Utf8>::from_options(&[
        Some("héllo".to_string()),
        None,
        Some("z".to_string()),
    ]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let strings = array.as_any().downcast_ref::<StringArray>().unwrap();
    assert_eq!(strings.len(), 3);
    assert_eq!(strings.value(0), "héllo");
    assert!(strings.is_null(1));
    assert_eq!(strings.value(2), "z");

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(0), Value::Utf8("héllo".to_string()));
    assert_eq!(back.get(1), Value::Null);
    assert_eq!(back.get(2), Value::Utf8("z".to_string()));
}

#[test]
fn binary_column_round_trips() {
    let column = Column::from(VarSerie::<Binary>::from_values(&[
        vec![0u8, 1, 2],
        vec![255],
        Vec::new(),
    ]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let bins = array.as_any().downcast_ref::<BinaryArray>().unwrap();
    assert_eq!(bins.value(0), &[0u8, 1, 2]);
    assert_eq!(bins.value(1), &[255u8]);
    assert_eq!(bins.value(2), &[] as &[u8]);

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(0), Value::Binary(vec![0, 1, 2]));
    assert_eq!(back.get(1), Value::Binary(vec![255]));
    assert_eq!(back.get(2), Value::Binary(Vec::new()));
}

#[test]
fn fixed_binary_column_preserves_width() {
    // Fixed width 3: "abc" fits, "xy" zero-pads to "xy\0".
    let column = Column::from(FixedSizeSerie::<FixedBinary>::from_values(
        3,
        &[b"abc".to_vec(), b"xy".to_vec()],
    ));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let fixed = array
        .as_any()
        .downcast_ref::<FixedSizeBinaryArray>()
        .unwrap();
    assert_eq!(fixed.value_length(), 3);
    assert_eq!(fixed.value(0), b"abc");
    assert_eq!(fixed.value(1), b"xy\0");
    assert_eq!(array.data_type(), &DataType::FixedSizeBinary(3));

    let back = column_from_arrow(&array, &field).unwrap();
    assert_eq!(back.get(0), Value::Binary(b"abc".to_vec()));
    assert_eq!(back.get(1), Value::Binary(b"xy\0".to_vec()));
    assert_eq!(back.field().data_type_id(), DataTypeId::FixedBinary);
}

#[test]
fn sliced_array_round_trips() {
    // A 4-element Int32 column -> Arrow -> slice(1, 2) -> back should be [20, 30].
    let column = Column::from(FixedSerie::<Int32>::from_values(&[10, 20, 30, 40]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let sliced = array.slice(1, 2); // logical [20, 30]
    assert_eq!(sliced.len(), 2);

    let back = column_from_arrow(&sliced, &field).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(0), Value::Int32(20));
    assert_eq!(back.get(1), Value::Int32(30));
}

#[test]
fn sliced_nullable_array_round_trips() {
    let column = Column::from(FixedSerie::<Int32>::from_options(&[
        Some(1),
        None,
        Some(3),
        None,
    ]));
    let field = column.field();

    let array = column_to_arrow(&column).unwrap();
    let sliced = array.slice(1, 2); // logical [None, Some(3)]
    let back = column_from_arrow(&sliced, &field).unwrap();
    assert_eq!(back.len(), 2);
    assert_eq!(back.get(0), Value::Null);
    assert_eq!(back.get(1), Value::Int32(3));
    assert_eq!(back.null_count(), 1);
}

// ---- A. DataTypeId <-> DataType map ----------------------------------------------------------

#[test]
fn data_type_map_representatives() {
    assert_eq!(
        to_arrow_data_type(DataTypeId::I32, None, None, None),
        DataType::Int32
    );
    assert_eq!(
        to_arrow_data_type(DataTypeId::F64, None, None, None),
        DataType::Float64
    );
    assert_eq!(
        to_arrow_data_type(DataTypeId::Bool, None, None, None),
        DataType::Boolean
    );
    assert_eq!(
        to_arrow_data_type(DataTypeId::Decimal128, Some(10), Some(2), None),
        DataType::Decimal128(10, 2)
    );
    assert_eq!(
        to_arrow_data_type(DataTypeId::FixedBinary, None, None, Some(16)),
        DataType::FixedSizeBinary(16)
    );
    assert_eq!(
        to_arrow_data_type(DataTypeId::Utf8, None, None, None),
        DataType::Utf8
    );
    // Lossy: FixedUtf8 -> FixedSizeBinary.
    assert_eq!(
        to_arrow_data_type(DataTypeId::FixedUtf8, None, None, Some(4)),
        DataType::FixedSizeBinary(4)
    );
    // Lossy: Decimal32 widens to Decimal128.
    assert_eq!(
        to_arrow_data_type(DataTypeId::Decimal32, Some(9), Some(3), None),
        DataType::Decimal128(9, 3)
    );

    // Reverse (leaf-only inverse).
    assert_eq!(
        from_arrow_data_type(&DataType::Int64),
        (DataTypeId::I64, None, None, None)
    );
    assert_eq!(
        from_arrow_data_type(&DataType::Decimal128(10, 2)),
        (DataTypeId::Decimal128, Some(10), Some(2), None)
    );
    // FixedSizeBinary always maps back to FixedBinary (FixedUtf8 needs our field metadata).
    assert_eq!(
        from_arrow_data_type(&DataType::FixedSizeBinary(8)),
        (DataTypeId::FixedBinary, None, None, Some(8))
    );
    // An Arrow type with no leaf here degrades to Unknown.
    assert_eq!(
        from_arrow_data_type(&DataType::Float16),
        (DataTypeId::Unknown, None, None, None)
    );
}

// ---- A. HeaderField <-> Arrow Field ----------------------------------------------------------

#[test]
fn header_field_round_trips() {
    let field = HeaderField::new(Some("price"), DataTypeId::I64, true).with_metadata("unit", "USD");

    let arrow = to_arrow_field(&field);
    assert_eq!(arrow.name(), "price");
    assert_eq!(arrow.data_type(), &DataType::Int64);
    assert!(arrow.is_nullable());
    assert_eq!(
        arrow.metadata().get("unit").map(String::as_str),
        Some("USD")
    );

    let back = from_arrow_field(&arrow);
    assert_eq!(back.name(), Some("price"));
    assert_eq!(back.data_type_id(), DataTypeId::I64);
    assert!(back.nullable());
    assert_eq!(back.metadata_value("unit").as_deref(), Some("USD"));

    // A directly-built Arrow field converts too.
    let direct = from_arrow_field(&ArrowField::new("q", DataType::Boolean, false));
    assert_eq!(direct.name(), Some("q"));
    assert_eq!(direct.data_type_id(), DataTypeId::Bool);
    assert!(!direct.nullable());
}

#[test]
fn decimal_header_field_round_trips() {
    let field = HeaderField::decimal(Some("amount"), DataTypeId::Decimal128, 12, 4, false);

    let arrow = to_arrow_field(&field);
    assert_eq!(arrow.data_type(), &DataType::Decimal128(12, 4));
    assert!(!arrow.is_nullable());

    let back = from_arrow_field(&arrow);
    assert_eq!(back.data_type_id(), DataTypeId::Decimal128);
    assert_eq!(back.precision(), Some(12));
    assert_eq!(back.scale(), Some(4));
}

#[test]
fn nested_column_is_guided_error() {
    // A struct column is refused with a guided message (the nested phase owns nested interop).
    let column = Column::from(StructSerie::new("s"));
    let err = column_to_arrow(&column).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("nested"), "message was: {message}");
}
