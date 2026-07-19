//! Functional tests for the isolated any→any conversion framework
//! ([`convert_column`](yggdryl_core::typed::convert_column)) and the
//! [`LogicalType`](yggdryl_core::typed::LogicalType) logical↔physical descriptor.
//!
//! Every matrix arm is exercised on a small column — values preserved, nulls preserved, saturating
//! narrow, utf8↔binary byte-for-byte, decimal↔numeric unscaled (zero-copy relabel), Null→X all-null,
//! X→Null, `Any`/same-dtype no-ops, and an unsupported pair's guided error — plus the proof that the
//! erased [`Column::cast_field`] (the binding-facing cast) and the typed
//! [`FixedSerie::cast_field`](yggdryl_core::typed::FixedSerie) route through the single
//! `convert_column` implementation.

use yggdryl_core::datatype_id::DataTypeId;
use yggdryl_core::io::memory::IoError;
use yggdryl_core::typed::fixedbit::Bit;
use yggdryl_core::typed::fixedbyte::{Decimal128, Decimal256, Int128, Int32, Int64, I256};
use yggdryl_core::typed::varbyte::{LargeUtf8, Utf8};
use yggdryl_core::typed::{
    convert_column, convert_column_in_place, Column, ColumnField, FixedBinary, FixedSerie,
    FixedSizeSerie, HeaderField, LogicalType, NullSerie, Scalar, Value, VarSerie,
};

// -------------------------------------------------------------------------------------
// numeric ↔ numeric — resize_dtype (widen / narrow / saturating)
// -------------------------------------------------------------------------------------

#[test]
fn numeric_to_numeric_widen_and_values_preserved() {
    let col = Column::from(FixedSerie::<Int64>::from_values(&[1, -2, 3]));
    let widened = convert_column(&col, DataTypeId::I32, None).unwrap();
    assert_eq!(widened.data_type_id(), DataTypeId::I32);
    assert_eq!(widened.len(), 3);
    assert_eq!(widened.get(0), Value::Int32(1));
    assert_eq!(widened.get(1), Value::Int32(-2));
    assert_eq!(widened.get(2), Value::Int32(3));

    // i32 -> f64 widening.
    let as_f64 = convert_column(&widened, DataTypeId::F64, None).unwrap();
    assert_eq!(as_f64.get(1), Value::Float64(-2.0));
}

#[test]
fn numeric_narrow_saturates() {
    // 300 does not fit i8; the resize saturates to i8::MAX, -5 is preserved.
    let col = Column::from(FixedSerie::<Int32>::from_values(&[300, -5, 42]));
    let narrowed = convert_column(&col, DataTypeId::I8, None).unwrap();
    assert_eq!(narrowed.data_type_id(), DataTypeId::I8);
    assert_eq!(narrowed.get(0), Value::Int8(127));
    assert_eq!(narrowed.get(1), Value::Int8(-5));
    assert_eq!(narrowed.get(2), Value::Int8(42));
}

#[test]
fn nulls_preserved_across_numeric_cast() {
    let col = Column::from(FixedSerie::<Int64>::from_options(&[Some(1), None, Some(3)]));
    let out = convert_column(&col, DataTypeId::I32, None).unwrap();
    assert_eq!(out.len(), 3);
    assert!(out.is_null(1));
    assert_eq!(out.null_count(), 1);
    assert_eq!(out.get(0), Value::Int32(1));
    assert_eq!(out.get(1), Value::Null);
    assert_eq!(out.get(2), Value::Int32(3));
}

// -------------------------------------------------------------------------------------
// bool ↔ numeric — the bit pack/unpack
// -------------------------------------------------------------------------------------

#[test]
fn bool_to_numeric_as_zero_one() {
    let col = Column::from(FixedSerie::<Bit>::from_values(&[true, false, true, true]));
    let as_i8 = convert_column(&col, DataTypeId::I8, None).unwrap();
    assert_eq!(as_i8.data_type_id(), DataTypeId::I8);
    assert_eq!(as_i8.get(0), Value::Int8(1));
    assert_eq!(as_i8.get(1), Value::Int8(0));
    assert_eq!(as_i8.get(3), Value::Int8(1));

    // bool -> i64 goes bool -> i8 -> resize.
    let as_i64 = convert_column(&col, DataTypeId::I64, None).unwrap();
    assert_eq!(as_i64.get(2), Value::Int64(1));
}

#[test]
fn numeric_to_bool_as_nonzero() {
    let col = Column::from(FixedSerie::<Int32>::from_values(&[0, 5, -1, 0]));
    let as_bool = convert_column(&col, DataTypeId::Bool, None).unwrap();
    assert_eq!(as_bool.data_type_id(), DataTypeId::Bool);
    assert_eq!(as_bool.get(0), Value::Bool(false));
    assert_eq!(as_bool.get(1), Value::Bool(true));
    assert_eq!(as_bool.get(2), Value::Bool(true));
    assert_eq!(as_bool.get(3), Value::Bool(false));
}

// -------------------------------------------------------------------------------------
// numeric → utf8 (format) and utf8 → numeric (flexible parse)
// -------------------------------------------------------------------------------------

#[test]
fn numeric_to_utf8_and_parse_back() {
    let col = Column::from(FixedSerie::<Int64>::from_values(&[10, -20, 30]));
    let text = convert_column(&col, DataTypeId::Utf8, None).unwrap();
    assert_eq!(text.data_type_id(), DataTypeId::Utf8);
    assert_eq!(text.get(0), Value::Utf8("10".into()));
    assert_eq!(text.get(1), Value::Utf8("-20".into()));

    // utf8 -> i64 round-trips the values.
    let back = convert_column(&text, DataTypeId::I64, None).unwrap();
    assert_eq!(back.get(0), Value::Int64(10));
    assert_eq!(back.get(1), Value::Int64(-20));
    assert_eq!(back.get(2), Value::Int64(30));
}

#[test]
fn utf8_to_numeric_is_flexible_and_null_aware() {
    // Thousands separators, radix, whitespace — the flexible parse; a null stays null.
    let col = Column::from(VarSerie::<Utf8>::from_options(&[
        Some("1,000".into()),
        None,
        Some("0xFF".into()),
        Some(" 42 ".into()),
    ]));
    let out = convert_column(&col, DataTypeId::I64, None).unwrap();
    assert_eq!(out.get(0), Value::Int64(1000));
    assert_eq!(out.get(1), Value::Null);
    assert!(out.is_null(1));
    assert_eq!(out.get(2), Value::Int64(255));
    assert_eq!(out.get(3), Value::Int64(42));
}

#[test]
fn numeric_to_utf8_preserves_nulls() {
    let col = Column::from(FixedSerie::<Int64>::from_options(&[Some(7), None]));
    let text = convert_column(&col, DataTypeId::Utf8, None).unwrap();
    assert_eq!(text.get(0), Value::Utf8("7".into()));
    assert!(text.is_null(1));
}

// -------------------------------------------------------------------------------------
// binary ↔ utf8 (and Large*) — the offsets+data reinterpret
// -------------------------------------------------------------------------------------

#[test]
fn utf8_to_binary_same_bytes_and_back() {
    let col = Column::from(VarSerie::<Utf8>::from_values(&[
        "ab".into(),
        "cde".into(),
        "".into(),
    ]));
    let bin = convert_column(&col, DataTypeId::Binary, None).unwrap();
    assert_eq!(bin.data_type_id(), DataTypeId::Binary);
    assert_eq!(bin.get(0), Value::Binary(b"ab".to_vec()));
    assert_eq!(bin.get(1), Value::Binary(b"cde".to_vec()));
    assert_eq!(bin.get(2), Value::Binary(Vec::new()));

    // binary -> utf8 recovers the strings.
    let text = convert_column(&bin, DataTypeId::Utf8, None).unwrap();
    assert_eq!(text.get(0), Value::Utf8("ab".into()));
    assert_eq!(text.get(1), Value::Utf8("cde".into()));
}

#[test]
fn utf8_to_large_utf8_widens_offsets() {
    let col = Column::from(VarSerie::<Utf8>::from_options(&[
        Some("hello".into()),
        None,
        Some("world".into()),
    ]));
    let large = convert_column(&col, DataTypeId::LargeUtf8, None).unwrap();
    assert_eq!(large.data_type_id(), DataTypeId::LargeUtf8);
    assert_eq!(large.get(0), Value::Utf8("hello".into()));
    assert!(large.is_null(1));
    assert_eq!(large.get(2), Value::Utf8("world".into()));

    // and back down to the i32-offset form.
    let small = convert_column(&large, DataTypeId::Utf8, None).unwrap();
    assert_eq!(small.data_type_id(), DataTypeId::Utf8);
    assert_eq!(small.get(2), Value::Utf8("world".into()));
}

#[test]
fn variable_to_fixed_size_utf8() {
    let col = Column::from(VarSerie::<Utf8>::from_values(&["ab".into(), "cdef".into()]));
    let fixed = convert_column(&col, DataTypeId::FixedUtf8, None).unwrap();
    assert_eq!(fixed.data_type_id(), DataTypeId::FixedUtf8);
    // Width is the longest element; the shorter one is zero-padded but decodes back to "ab".
    assert_eq!(fixed.get(1), Value::Utf8("cdef".into()));
}

#[test]
fn fixed_binary_to_fixed_utf8_reinterpret() {
    let col = Column::from(FixedSizeSerie::<FixedBinary>::from_values(
        3,
        &[b"abc".to_vec(), b"xyz".to_vec()],
    ));
    let text = convert_column(&col, DataTypeId::FixedUtf8, None).unwrap();
    assert_eq!(text.data_type_id(), DataTypeId::FixedUtf8);
    assert_eq!(text.get(0), Value::Utf8("abc".into()));
    assert_eq!(text.get(1), Value::Utf8("xyz".into()));
}

// -------------------------------------------------------------------------------------
// decimal ↔ numeric — the unscaled physical (zero-copy relabel), + scale-aware → utf8
// -------------------------------------------------------------------------------------

#[test]
fn decimal128_to_i128_is_unscaled_reinterpret() {
    // A magnitude beyond 2^53 proves the relabel does NOT funnel through the f64 carrier.
    let big = 9_000_000_000_000_000_123i128;
    let col = Column::from(
        FixedSerie::<Decimal128>::from_values(&[12345, big]).with_precision_scale(38, 2),
    );
    let ints = convert_column(&col, DataTypeId::I128, None).unwrap();
    assert_eq!(ints.data_type_id(), DataTypeId::I128);
    assert_eq!(ints.get(0), Value::Int128(12345));
    assert_eq!(ints.get(1), Value::Int128(big)); // exact — no precision loss

    // and i128 -> decimal128 relabels back (same unscaled bytes).
    let dec = convert_column(&ints, DataTypeId::Decimal128, None).unwrap();
    assert_eq!(dec.data_type_id(), DataTypeId::Decimal128);
    assert_eq!(dec.get(1), Value::Decimal128(big));
}

#[test]
fn decimal128_narrow_physical_to_i32() {
    let col = Column::from(FixedSerie::<Decimal128>::from_values(&[12345, -5]));
    let ints = convert_column(&col, DataTypeId::I32, None).unwrap();
    assert_eq!(ints.get(0), Value::Int32(12345));
    assert_eq!(ints.get(1), Value::Int32(-5));
}

#[test]
fn decimal_to_utf8_is_scale_aware() {
    let col = Column::from(
        FixedSerie::<Decimal128>::from_values(&[12345, 5, -5]).with_precision_scale(10, 2),
    );
    let text = convert_column(&col, DataTypeId::Utf8, None).unwrap();
    assert_eq!(text.get(0), Value::Utf8("123.45".into()));
    assert_eq!(text.get(1), Value::Utf8("0.05".into()));
    assert_eq!(text.get(2), Value::Utf8("-0.05".into()));
}

#[test]
fn decimal256_to_utf8_scale_aware() {
    let col = Column::from(
        FixedSerie::<Decimal256>::from_values(&[I256::from_i128(12345)])
            .with_precision_scale(76, 2),
    );
    let text = convert_column(&col, DataTypeId::Utf8, None).unwrap();
    assert_eq!(text.get(0), Value::Utf8("123.45".into()));
}

// -------------------------------------------------------------------------------------
// Null ↔ X, Any / same-dtype no-ops
// -------------------------------------------------------------------------------------

#[test]
fn null_to_x_is_all_null() {
    let col = Column::from(NullSerie::new(4));
    let ints = convert_column(&col, DataTypeId::I32, None).unwrap();
    assert_eq!(ints.data_type_id(), DataTypeId::I32);
    assert_eq!(ints.len(), 4);
    assert_eq!(ints.null_count(), 4);
    assert!(ints.is_null(0));

    // Null -> utf8 is an all-null string column too.
    let text = convert_column(&col, DataTypeId::Utf8, None).unwrap();
    assert_eq!(text.data_type_id(), DataTypeId::Utf8);
    assert_eq!(text.null_count(), 4);
}

#[test]
fn x_to_null_is_bufferless_null_run() {
    let col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
    let nulls = convert_column(&col, DataTypeId::Null, None).unwrap();
    assert_eq!(nulls.data_type_id(), DataTypeId::Null);
    assert_eq!(nulls.len(), 3);
    assert!(matches!(nulls, Column::Null(3)));
}

#[test]
fn any_target_is_a_noop() {
    let col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
    let same = convert_column(&col, DataTypeId::Any, None).unwrap();
    assert_eq!(same.data_type_id(), DataTypeId::I64);
    assert_eq!(same.get(0), Value::Int64(1));
    assert_eq!(same.len(), 3);
}

#[test]
fn same_dtype_is_a_noop_clone() {
    let col = Column::from(FixedSerie::<Int64>::from_values(&[10, 20]));
    let same = convert_column(&col, DataTypeId::I64, None).unwrap();
    assert_eq!(same.data_type_id(), DataTypeId::I64);
    assert_eq!(same.get(0), Value::Int64(10));
    assert_eq!(same.get(1), Value::Int64(20));
}

// -------------------------------------------------------------------------------------
// in-place, unsupported pairs, and the field-cast routing
// -------------------------------------------------------------------------------------

#[test]
fn convert_in_place_replaces_the_column() {
    let mut col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
    convert_column_in_place(&mut col, DataTypeId::I16, None).unwrap();
    assert_eq!(col.data_type_id(), DataTypeId::I16);
    assert_eq!(col.get(2), Value::Int16(3));

    // Any / same-dtype in place is a genuine no-op (still the same column).
    convert_column_in_place(&mut col, DataTypeId::Any, None).unwrap();
    assert_eq!(col.data_type_id(), DataTypeId::I16);
}

#[test]
fn unsupported_pair_is_a_guided_error() {
    // decimal256 <-> numeric has no faithful physical route. (Column isn't Debug, so match the
    // Result rather than unwrap_err.)
    let col = Column::from(FixedSerie::<Decimal256>::from_values(&[I256::from_i128(5)]));
    match convert_column(&col, DataTypeId::I128, None) {
        Err(IoError::TypedCast { detail }) => {
            assert!(
                detail.contains("decimal256"),
                "message names the type: {detail}"
            );
            assert!(
                detail.contains("i128"),
                "message names the target: {detail}"
            );
        }
        Err(other) => panic!("expected a guided TypedCast, got {other:?}"),
        Ok(_) => panic!("expected decimal256 -> i128 to be unsupported"),
    }

    // numeric -> struct is unsupported (no flat reinterpretation).
    let ints = Column::from(FixedSerie::<Int64>::from_values(&[1]));
    assert!(matches!(
        convert_column(&ints, DataTypeId::Struct, None),
        Err(IoError::TypedCast { .. })
    ));
}

#[test]
fn column_cast_field_routes_through_convert_and_applies_metadata() {
    let col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2, 3]));
    // A different-dtype field: convert + apply the name/nullable metadata in one call.
    let field = ColumnField::Leaf(HeaderField::new(Some("count"), DataTypeId::I32, false));
    let out = col.cast_field(&field).unwrap();
    assert_eq!(out.data_type_id(), DataTypeId::I32);
    assert_eq!(out.name(), Some("count"));
    assert_eq!(out.get(0), Value::Int32(1));

    // It agrees value-for-value with a bare convert_column to the same dtype.
    let bare = convert_column(&col, DataTypeId::I32, None).unwrap();
    assert_eq!(out.get(2), bare.get(2));
}

#[test]
fn column_cast_field_same_dtype_reshapes_metadata() {
    // Same dtype + a new name / nullability = a metadata reshape over the same bytes.
    let col = Column::from(FixedSerie::<Int64>::from_values(&[1, 2]));
    let field = ColumnField::Leaf(HeaderField::new(Some("renamed"), DataTypeId::I64, true));
    let out = col.cast_field(&field).unwrap();
    assert_eq!(out.data_type_id(), DataTypeId::I64);
    assert_eq!(out.name(), Some("renamed"));
    // Non-nullable -> nullable added an all-valid bitmap (no real nulls).
    assert_eq!(out.null_count(), 0);
    assert_eq!(out.get(1), Value::Int64(2));
}

#[test]
fn cast_field_carries_decimal_precision_scale() {
    let col = Column::from(FixedSerie::<Int128>::from_values(&[12345]));
    let field = ColumnField::Leaf(HeaderField::decimal(
        Some("price"),
        DataTypeId::Decimal128,
        10,
        2,
        false,
    ));
    let out = col.cast_field(&field).unwrap();
    assert_eq!(out.data_type_id(), DataTypeId::Decimal128);
    // The scale rides onto the column, so the scale-aware render places the point.
    if let Column::Decimal128(dec) = &out {
        assert_eq!(dec.to_decimal_string(0).as_deref(), Some("123.45"));
    } else {
        panic!("expected a Decimal128 column");
    }
}

// -------------------------------------------------------------------------------------
// The typed FixedSerie::cast_field still behaves (unchanged — a same-dtype metadata reshape)
// -------------------------------------------------------------------------------------

#[test]
fn typed_fixed_serie_cast_field_unchanged() {
    let col = FixedSerie::<Int64>::from_values(&[1, 2, 3]);
    // Same dtype, new name + nullable: a metadata reshape.
    let field = HeaderField::new(Some("x"), DataTypeId::I64, true);
    let reshaped = col.cast_field(&field).unwrap();
    assert_eq!(reshaped.name(), Some("x"));
    assert_eq!(reshaped.null_count(), 0);

    // A dtype change is still refused at the typed layer (that is the erased convert_column's job).
    let wrong = HeaderField::new(None, DataTypeId::I32, false);
    assert!(matches!(
        col.cast_field(&wrong),
        Err(IoError::TypedCast { .. })
    ));
}

// -------------------------------------------------------------------------------------
// LogicalType — the logical↔physical mapping the converter reuses
// -------------------------------------------------------------------------------------

#[test]
fn logical_type_physical_mapping() {
    use yggdryl_core::typed::fixedbyte::{Decimal32, Decimal64};

    assert_eq!(Decimal32::physical_dtype(), DataTypeId::I32);
    assert_eq!(Decimal64::physical_dtype(), DataTypeId::I64);
    assert_eq!(Decimal128::physical_dtype(), DataTypeId::I128);
    assert_eq!(Decimal256::physical_dtype(), DataTypeId::Decimal256); // no numeric physical
    assert_eq!(Utf8::physical_dtype(), DataTypeId::Binary);
    assert_eq!(LargeUtf8::physical_dtype(), DataTypeId::LargeBinary);
    assert_eq!(Decimal32::LOGICAL_ID, DataTypeId::Decimal32);
}
