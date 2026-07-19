//! Functional tests for [`DataTypeId`](yggdryl_core::datatype_id::DataTypeId) — the primitive element
//! data types: the `u16` round-trip, names, widths, predicates, and element counting.

use yggdryl_core::datatype_id::{DataTypeCategory, DataTypeId};

#[test]
fn u16_round_trip_is_total() {
    for dt in DataTypeId::ALL {
        assert_eq!(DataTypeId::from_u16(dt.as_u16()), dt);
    }
    assert_eq!(DataTypeId::from_u16(0), DataTypeId::Unknown);
    assert_eq!(DataTypeId::from_u16(9999), DataTypeId::Unknown); // foreign id degrades to raw

    // Ids live in per-category bands with reserved gaps, not a dense 0..N counter.
    assert_eq!(DataTypeId::I8.as_u16(), 0x0100);
    assert_eq!(DataTypeId::F32.as_u16(), 0x0201);
    assert_eq!(DataTypeId::Utf8.as_u16(), 0x0600);
    assert_eq!(DataTypeId::from_u16(0x0011), DataTypeId::Unknown); // a gap in the bool band
}

#[test]
fn categories_partition_the_bands() {
    assert_eq!(DataTypeId::Unknown.category(), DataTypeCategory::Null);
    assert_eq!(DataTypeId::Bool.category(), DataTypeCategory::Boolean);
    assert_eq!(DataTypeId::I64.category(), DataTypeCategory::Integer);
    assert_eq!(DataTypeId::F32.category(), DataTypeCategory::Float);
    assert_eq!(DataTypeId::Decimal128.category(), DataTypeCategory::Decimal);
    assert_eq!(DataTypeId::Binary.category(), DataTypeCategory::Binary);
    assert_eq!(DataTypeId::FixedUtf8.category(), DataTypeCategory::Utf8);
    assert_eq!(DataTypeCategory::Integer.name(), "integer");
    // Numeric = integer | float | decimal (not bool, not byte-like).
    assert!(
        DataTypeId::I64.is_numeric()
            && DataTypeId::F64.is_numeric()
            && DataTypeId::Decimal32.is_numeric()
    );
    assert!(!DataTypeId::Bool.is_numeric() && !DataTypeId::Utf8.is_numeric());
    // The reserved bands answer their predicates without any member yet.
    assert!(!DataTypeId::I64.is_temporal() && !DataTypeId::I64.is_nested());
}

#[test]
fn names_and_widths() {
    assert_eq!(DataTypeId::I64.name(), "i64");
    assert_eq!(DataTypeId::from_name("I64"), Some(DataTypeId::I64)); // case-insensitive
    assert_eq!(DataTypeId::from_name("nope"), None);
    assert_eq!(DataTypeId::I64.byte_size(), 8);
    assert_eq!(DataTypeId::I128.byte_size(), 16);
    assert_eq!(DataTypeId::Bool.byte_size(), 1);
    assert_eq!(DataTypeId::Bool.bit_size(), 1);
    assert_eq!(DataTypeId::I32.bit_size(), 32);
    assert_eq!(DataTypeId::Unknown.byte_size(), 0);
    assert_eq!(DataTypeId::to_string(&DataTypeId::F64), "f64");
}

#[test]
fn predicates_and_element_count() {
    assert!(DataTypeId::I32.is_integer() && DataTypeId::I32.is_signed());
    assert!(!DataTypeId::U32.is_signed() && DataTypeId::U32.is_integer());
    assert!(DataTypeId::F64.is_float() && DataTypeId::F64.is_signed());
    assert!(DataTypeId::Bool.is_bool() && !DataTypeId::Bool.is_integer());
    assert!(!DataTypeId::Unknown.is_fixed_width());

    assert_eq!(DataTypeId::I64.element_count(24), 3);
    assert_eq!(DataTypeId::I64.element_count(25), 3); // whole elements only
    assert_eq!(DataTypeId::Unknown.element_count(100), 0);
}

#[test]
fn decimal_variants() {
    assert_eq!(DataTypeId::Decimal32.byte_size(), 4);
    assert_eq!(DataTypeId::Decimal64.byte_size(), 8);
    assert_eq!(DataTypeId::Decimal128.byte_size(), 16);
    assert_eq!(DataTypeId::Decimal256.byte_size(), 32);
    assert!(DataTypeId::Decimal128.is_decimal());
    assert!(DataTypeId::Decimal128.is_signed());
    assert!(!DataTypeId::Decimal128.is_integer() && !DataTypeId::Decimal128.is_float());
    assert!(!DataTypeId::I64.is_decimal());
    assert_eq!(
        DataTypeId::from_name("decimal256"),
        Some(DataTypeId::Decimal256)
    );
    assert_eq!(DataTypeId::Decimal64.to_string(), "decimal64");
    // Still round-trips through u16 (the decimal band is 0x0300..).
    assert_eq!(DataTypeId::from_u16(0x0302), DataTypeId::Decimal128);
}

#[test]
fn variable_and_fixed_size_variants() {
    assert!(DataTypeId::Binary.is_binary() && DataTypeId::Binary.is_variable_length());
    assert!(DataTypeId::Utf8.is_utf8() && DataTypeId::Utf8.is_variable_length());
    assert!(DataTypeId::FixedBinary.is_binary() && !DataTypeId::FixedBinary.is_variable_length());
    assert!(DataTypeId::FixedUtf8.is_utf8());
    // No id-derivable width (a fixed-size type's width is field metadata).
    assert!(!DataTypeId::Binary.is_fixed_width() && DataTypeId::Binary.byte_size() == 0);
    assert!(!DataTypeId::FixedBinary.is_fixed_width());
    assert_eq!(DataTypeId::from_name("utf8"), Some(DataTypeId::Utf8));
    assert_eq!(
        DataTypeId::from_name("fixed_binary"),
        Some(DataTypeId::FixedBinary)
    );
    assert_eq!(DataTypeId::from_u16(0x0510), DataTypeId::FixedBinary);
    assert!(DataTypeId::FixedBinary.is_fixed_size() && !DataTypeId::Binary.is_fixed_size());
    assert_eq!(DataTypeId::Utf8.to_string(), "utf8");
}
