//! The **category drill-down** accessors (`is_integer` / `is_floating` / `is_utf8` / …),
//! exercised uniformly across *both* typed families through the erased root `&dyn DataType`.
//! This is the point of `DataTypeCategory`: a caller classifies any type — fixed or variable —
//! with one cheap predicate instead of matching the concrete type.

use yggdryl_core::io::fixed::PrimitiveType;
use yggdryl_core::io::var::{Binary, ByteType, Utf8};
use yggdryl_core::io::{DataType, DataTypeCategory, DataTypeId};

/// Every `DataTypeId` variant, for exhaustive round-trip / range checks.
const ALL_IDS: &[DataTypeId] = &[
    DataTypeId::Null,
    DataTypeId::U8,
    DataTypeId::U16,
    DataTypeId::U32,
    DataTypeId::U64,
    DataTypeId::U96,
    DataTypeId::U128,
    DataTypeId::U256,
    DataTypeId::I8,
    DataTypeId::I16,
    DataTypeId::I32,
    DataTypeId::I64,
    DataTypeId::I96,
    DataTypeId::I128,
    DataTypeId::I256,
    DataTypeId::F16,
    DataTypeId::F32,
    DataTypeId::F64,
    DataTypeId::FixedBinary,
    DataTypeId::FixedUtf8,
    DataTypeId::D32,
    DataTypeId::D64,
    DataTypeId::D128,
    DataTypeId::D256,
    DataTypeId::Date32,
    DataTypeId::Date64,
    DataTypeId::Time32,
    DataTypeId::Time64,
    DataTypeId::Ts32,
    DataTypeId::Ts64,
    DataTypeId::Ts96,
    DataTypeId::Duration32,
    DataTypeId::Duration64,
    DataTypeId::Binary,
    DataTypeId::LargeBinary,
    DataTypeId::Utf8,
    DataTypeId::LargeUtf8,
];

#[test]
fn data_type_id_decode_is_checked_and_round_trips() {
    // Every defined id round-trips through its u16 discriminant.
    for &id in ALL_IDS {
        assert_eq!(DataTypeId::from_u16(id.as_u16()), Some(id));
    }
    // Reserved-gap values decode to `None` (a checked match, never a transmute over a gap).
    for gap in [
        0x0001, 0x0017, 0x0027, 0x0033, 0x0041, 0x0054, 0x0062, 0x0080, 0x0102, 0x0200, 0xFFFF,
    ] {
        assert_eq!(DataTypeId::from_u16(gap), None);
    }
}

#[test]
fn data_type_id_names_round_trip_and_match_the_concrete_types() {
    for &id in ALL_IDS {
        assert_eq!(DataTypeId::from_name(id.name()), Some(id), "{id:?}");
    }
    assert_eq!(DataTypeId::from_name("not_a_type"), None);

    // The centralized names agree with what the concrete descriptors report (`DataType::name`),
    // so the metadata discriminator and the type names never diverge.
    use yggdryl_core::io::fixed::{f16, U96};
    assert_eq!(<PrimitiveType<u8>>::new().name(), DataTypeId::U8.name());
    assert_eq!(<PrimitiveType<f16>>::new().name(), DataTypeId::F16.name());
    assert_eq!(<PrimitiveType<U96>>::new().name(), DataTypeId::U96.name());
    assert_eq!(<PrimitiveType<i128>>::new().name(), DataTypeId::I128.name());
    assert_eq!(<ByteType<Utf8>>::new().name(), DataTypeId::Utf8.name());
    assert_eq!(<ByteType<Binary>>::new().name(), DataTypeId::Binary.name());
}

#[test]
fn fixed_byte_width_agrees_with_the_descriptors() {
    use yggdryl_core::io::fixed::{f16, U256};
    assert_eq!(DataTypeId::U8.fixed_byte_width(), Some(1));
    assert_eq!(DataTypeId::U256.fixed_byte_width(), Some(32));
    assert_eq!(DataTypeId::F16.fixed_byte_width(), Some(2));
    assert_eq!(DataTypeId::Utf8.fixed_byte_width(), Some(4));
    assert_eq!(DataTypeId::LargeUtf8.fixed_byte_width(), Some(8));
    assert_eq!(DataTypeId::FixedBinary.fixed_byte_width(), None); // runtime N
                                                                  // Agrees with what the concrete descriptors report.
    assert_eq!(
        <PrimitiveType<U256>>::new().byte_width(),
        DataTypeId::U256.fixed_byte_width().unwrap()
    );
    assert_eq!(
        <PrimitiveType<f16>>::new().byte_width(),
        DataTypeId::F16.fixed_byte_width().unwrap()
    );
    assert_eq!(
        <ByteType<Utf8>>::new().byte_width(),
        DataTypeId::Utf8.fixed_byte_width().unwrap()
    );
}

#[test]
fn field_of_builds_from_parts() {
    use yggdryl_core::io::fixed::Field;
    use yggdryl_core::io::FieldType;
    let field = Field::of("hash", DataTypeId::U96, 12, false);
    assert_eq!(field.name(), "hash");
    assert_eq!(field.type_name(), "u96");
    assert_eq!(field.byte_width(), 12);
    assert!(!field.nullable());
    assert_eq!(FieldType::type_id(&field), DataTypeId::U96);
    assert!(FieldType::is_unsigned_integer(&field));
    assert!(field.metadata().is_empty());
}

#[test]
fn data_type_id_ranges_are_mutually_consistent() {
    for &id in ALL_IDS {
        // Fixed-width and variable-length partition every non-null id exactly.
        if !id.is_null() {
            assert_ne!(id.is_fixed_width(), id.is_variable_length(), "{id:?}");
        }
        // An integer is never a float and vice versa; both are numeric.
        assert!(!(id.is_integer() && id.is_floating()));
        if id.is_integer() || id.is_floating() {
            assert!(id.is_numeric() && id.is_fixed_width());
        }
        // Unsigned/signed integers are disjoint and both are integers.
        assert!(!(id.is_unsigned_integer() && id.is_signed_integer()));
        if id.is_unsigned_integer() || id.is_signed_integer() {
            assert!(id.is_integer());
        }
        // A signed integer is signed; an unsigned integer is not.
        if id.is_signed_integer() {
            assert!(id.is_signed());
        }
        if id.is_unsigned_integer() {
            assert!(!id.is_signed());
        }
        // Binary and utf8 are disjoint; the fixed-size byte ids are fixed-width byte types.
        assert!(!(id.is_binary() && id.is_utf8()));
    }
    // The fixed-size byte ids are BOTH fixed-width AND binary/utf8 (dual classification).
    assert!(DataTypeId::FixedBinary.is_fixed_width() && DataTypeId::FixedBinary.is_binary());
    assert!(DataTypeId::FixedUtf8.is_fixed_width() && DataTypeId::FixedUtf8.is_utf8());
    // The category buckets are correct.
    assert_eq!(
        DataTypeId::U256.category(),
        DataTypeCategory::UnsignedInteger
    );
    assert_eq!(DataTypeId::I256.category(), DataTypeCategory::SignedInteger);
    assert_eq!(DataTypeId::F16.category(), DataTypeCategory::Float);
    assert_eq!(DataTypeId::FixedUtf8.category(), DataTypeCategory::Utf8);
    assert_eq!(DataTypeId::LargeBinary.category(), DataTypeCategory::Binary);
    assert_eq!(DataTypeId::Null.category(), DataTypeCategory::Null);
    // Decimals are their own category: signed, numeric, fixed-width — but neither integer nor float.
    for id in [
        DataTypeId::D32,
        DataTypeId::D64,
        DataTypeId::D128,
        DataTypeId::D256,
    ] {
        assert_eq!(id.category(), DataTypeCategory::Decimal, "{id:?}");
        assert!(id.is_decimal() && id.is_numeric() && id.is_signed() && id.is_fixed_width());
        assert!(!id.is_integer() && !id.is_floating() && !id.is_binary() && !id.is_utf8());
    }
    assert!(DataTypeCategory::Decimal.is_decimal() && DataTypeCategory::Decimal.is_numeric());
    assert!(DataTypeCategory::Decimal.is_signed() && !DataTypeCategory::Decimal.is_integer());
    // Temporals are their own category: fixed-width, but not numeric and not binary/utf8.
    for id in [
        DataTypeId::Date32,
        DataTypeId::Time64,
        DataTypeId::Ts96,
        DataTypeId::Duration64,
    ] {
        assert_eq!(id.category(), DataTypeCategory::Temporal, "{id:?}");
        assert!(id.is_temporal() && id.is_fixed_width());
        assert!(!id.is_numeric() && !id.is_binary() && !id.is_utf8() && !id.is_decimal());
    }
    assert!(DataTypeCategory::Temporal.is_temporal() && !DataTypeCategory::Temporal.is_numeric());
}

/// The full predicate fingerprint of a type, in a fixed order, so a whole row can be asserted
/// at once: (integer, signed, floating, numeric, utf8, binary, fixed_width, variable_length).
fn fingerprint(dt: &dyn DataType) -> [bool; 8] {
    [
        dt.is_integer(),
        dt.is_signed(),
        dt.is_floating(),
        dt.is_numeric(),
        dt.is_utf8(),
        dt.is_binary(),
        dt.is_fixed_width(),
        dt.is_variable_length(),
    ]
}

#[test]
fn unsigned_integer_predicates() {
    //                                 int    sign   float  num    utf8   bin    fixed  var
    let expected = [true, false, false, true, false, false, true, false];
    assert_eq!(fingerprint(&<PrimitiveType<u8>>::new()), expected);
    assert_eq!(fingerprint(&<PrimitiveType<u64>>::new()), expected);
}

#[test]
fn signed_integer_predicates() {
    let expected = [true, true, false, true, false, false, true, false];
    assert_eq!(fingerprint(&<PrimitiveType<i8>>::new()), expected);
    assert_eq!(fingerprint(&<PrimitiveType<i64>>::new()), expected);
}

#[test]
fn float_predicates() {
    // A float is signed and numeric, but not an integer.
    let expected = [false, true, true, true, false, false, true, false];
    assert_eq!(fingerprint(&<PrimitiveType<f32>>::new()), expected);
    assert_eq!(fingerprint(&<PrimitiveType<f64>>::new()), expected);
}

#[test]
fn decimal_predicates() {
    use yggdryl_core::io::fixed::{Dec128, Dec32, DecimalType};
    // A decimal is signed and numeric, fixed-width, but neither integer nor float.
    let expected = [false, true, false, true, false, false, true, false];
    assert_eq!(fingerprint(&DecimalType::<Dec32>::new(9, 2)), expected);
    assert_eq!(fingerprint(&DecimalType::<Dec128>::new(38, 10)), expected);
    assert!(DecimalType::<Dec128>::new(20, 4).is_decimal());
}

#[test]
fn utf8_predicates() {
    let expected = [false, false, false, false, true, false, false, true];
    assert_eq!(fingerprint(&<ByteType<Utf8>>::new()), expected);
}

#[test]
fn binary_predicates() {
    let expected = [false, false, false, false, false, true, false, true];
    assert_eq!(fingerprint(&<ByteType<Binary>>::new()), expected);
}

#[test]
fn a_heterogeneous_schema_classifies_each_column_with_one_predicate() {
    // Columns of differing shapes behind one erased type — the drill-down needs no `match`.
    let columns: [&dyn DataType; 4] = [
        &<PrimitiveType<i32>>::new(),
        &<PrimitiveType<f64>>::new(),
        &<ByteType<Utf8>>::new(),
        &<ByteType<Binary>>::new(),
    ];
    let numeric = columns.iter().filter(|c| c.is_numeric()).count();
    let variable = columns.iter().filter(|c| c.is_variable_length()).count();
    assert_eq!(numeric, 2);
    assert_eq!(variable, 2);
}
