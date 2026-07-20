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
fn float16_and_small_decimal_variants() {
    // Float16 is the reserved f16 slot (0x0200) at the head of the float band.
    assert_eq!(DataTypeId::Float16.as_u16(), 0x0200);
    assert_eq!(DataTypeId::from_u16(0x0200), DataTypeId::Float16);
    assert_eq!(DataTypeId::Float16.name(), "float16");
    assert_eq!(DataTypeId::from_name("FLOAT16"), Some(DataTypeId::Float16)); // case-insensitive
    assert_eq!(DataTypeId::Float16.category(), DataTypeCategory::Float);
    assert!(DataTypeId::Float16.is_float() && DataTypeId::Float16.is_signed());
    assert!(DataTypeId::Float16.is_numeric() && !DataTypeId::Float16.is_decimal());
    assert_eq!(DataTypeId::Float16.byte_size(), 2);
    assert_eq!(DataTypeId::Float16.bit_size(), 16);
    assert!(DataTypeId::Float16.is_fixed_width());
    assert_eq!(DataTypeId::Float16.to_string(), "float16");

    // Decimal8 / Decimal16 extend the decimal band (0x0304 / 0x0305).
    assert_eq!(DataTypeId::Decimal8.as_u16(), 0x0304);
    assert_eq!(DataTypeId::Decimal16.as_u16(), 0x0305);
    assert_eq!(DataTypeId::from_u16(0x0304), DataTypeId::Decimal8);
    assert_eq!(DataTypeId::from_u16(0x0305), DataTypeId::Decimal16);
    assert_eq!(DataTypeId::Decimal8.name(), "decimal8");
    assert_eq!(DataTypeId::Decimal16.name(), "decimal16");
    assert_eq!(
        DataTypeId::from_name("decimal8"),
        Some(DataTypeId::Decimal8)
    );
    assert_eq!(
        DataTypeId::from_name("decimal16"),
        Some(DataTypeId::Decimal16)
    );
    assert_eq!(DataTypeId::Decimal8.byte_size(), 1);
    assert_eq!(DataTypeId::Decimal16.byte_size(), 2);
    for dt in [DataTypeId::Decimal8, DataTypeId::Decimal16] {
        assert_eq!(dt.category(), DataTypeCategory::Decimal);
        assert!(dt.is_decimal() && dt.is_signed() && dt.is_numeric());
        assert!(!dt.is_integer() && !dt.is_float());
        assert!(dt.is_fixed_width());
    }
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

#[test]
fn large_variants() {
    // Ids, names, and the u16 / name round-trips (the reserved 0x0502 / 0x0602 slots).
    assert_eq!(DataTypeId::LargeBinary.as_u16(), 0x0502);
    assert_eq!(DataTypeId::LargeUtf8.as_u16(), 0x0602);
    assert_eq!(DataTypeId::LargeBinary.name(), "large_binary");
    assert_eq!(DataTypeId::LargeUtf8.name(), "large_utf8");
    assert_eq!(DataTypeId::from_u16(0x0502), DataTypeId::LargeBinary);
    assert_eq!(DataTypeId::from_u16(0x0602), DataTypeId::LargeUtf8);
    assert_eq!(
        DataTypeId::from_name("large_binary"),
        Some(DataTypeId::LargeBinary)
    );
    assert_eq!(
        DataTypeId::from_name("LARGE_UTF8"), // case-insensitive
        Some(DataTypeId::LargeUtf8)
    );

    // They fall in the Binary / Utf8 bands automatically via the `>>8` category.
    assert_eq!(DataTypeId::LargeBinary.category(), DataTypeCategory::Binary);
    assert_eq!(DataTypeId::LargeUtf8.category(), DataTypeCategory::Utf8);

    // Binary / Utf8 + variable-length + large; NOT fixed-size, no id-derivable width.
    assert!(DataTypeId::LargeBinary.is_binary() && DataTypeId::LargeBinary.is_variable_length());
    assert!(DataTypeId::LargeUtf8.is_utf8() && DataTypeId::LargeUtf8.is_variable_length());
    assert!(DataTypeId::LargeBinary.is_large() && DataTypeId::LargeUtf8.is_large());
    assert!(!DataTypeId::Binary.is_large() && !DataTypeId::FixedBinary.is_large());
    assert!(!DataTypeId::LargeBinary.is_fixed_size() && !DataTypeId::LargeUtf8.is_fixed_size());
    assert_eq!(DataTypeId::LargeBinary.byte_size(), 0);
    assert_eq!(DataTypeId::LargeUtf8.byte_size(), 0);
    assert!(!DataTypeId::LargeBinary.is_fixed_width());
    assert_eq!(DataTypeId::LargeBinary.to_string(), "large_binary");
}

#[test]
fn null_and_any_special_band() {
    // The typed all-null `Null` (0x0001) and the erased "holds any type" `Any` (0x00F0) live in the
    // special band next to `Unknown` (0x0000), each with a stable name and a u16 round-trip.
    assert_eq!(DataTypeId::Null.as_u16(), 0x0001);
    assert_eq!(DataTypeId::Any.as_u16(), 0x00F0);
    assert_eq!(DataTypeId::from_u16(0x0001), DataTypeId::Null);
    assert_eq!(DataTypeId::from_u16(0x00F0), DataTypeId::Any);
    assert_eq!(DataTypeId::Null.name(), "null");
    assert_eq!(DataTypeId::Any.name(), "any");
    assert_eq!(DataTypeId::from_name("null"), Some(DataTypeId::Null));
    assert_eq!(DataTypeId::from_name("ANY"), Some(DataTypeId::Any)); // case-insensitive
    assert_eq!(DataTypeId::Null.to_string(), "null");

    // Predicates: only `Null` is the typed all-null, only `Any` is the erased meta-tag; `Unknown`
    // is neither (it is raw bytes).
    assert!(DataTypeId::Null.is_null_type() && !DataTypeId::Null.is_any());
    assert!(DataTypeId::Any.is_any() && !DataTypeId::Any.is_null_type());
    assert!(!DataTypeId::Unknown.is_null_type() && !DataTypeId::Unknown.is_any());
    assert!(!DataTypeId::I64.is_null_type() && !DataTypeId::I64.is_any());

    // Both fold into the special `Null` category, carry no element width, and are not fixed-width.
    assert_eq!(DataTypeId::Null.category(), DataTypeCategory::Null);
    assert_eq!(DataTypeId::Any.category(), DataTypeCategory::Null);
    assert_eq!(DataTypeId::Null.byte_size(), 0);
    assert_eq!(DataTypeId::Any.byte_size(), 0);
    assert!(!DataTypeId::Null.is_fixed_width() && !DataTypeId::Any.is_fixed_width());
    // Neither is numeric / nested / byte-like.
    assert!(!DataTypeId::Null.is_numeric() && !DataTypeId::Any.is_nested());
}

#[test]
fn nested_categories_split_struct_list_map() {
    // The nested band splits into three distinct categories.
    assert_eq!(DataTypeId::Struct.category(), DataTypeCategory::Struct);
    assert_eq!(DataTypeId::List.category(), DataTypeCategory::List);
    assert_eq!(DataTypeId::Map.category(), DataTypeCategory::Map);
    assert_eq!(DataTypeCategory::Struct.name(), "struct");
    assert_eq!(DataTypeCategory::List.name(), "list");
    assert_eq!(DataTypeCategory::Map.name(), "map");

    // All three are still "nested", and each answers its own specific predicate.
    for dt in [DataTypeId::Struct, DataTypeId::List, DataTypeId::Map] {
        assert!(dt.is_nested());
    }
    assert!(
        DataTypeId::Struct.is_struct()
            && !DataTypeId::Struct.is_list()
            && !DataTypeId::Struct.is_map()
    );
    assert!(
        DataTypeId::List.is_list() && !DataTypeId::List.is_struct() && !DataTypeId::List.is_map()
    );
    assert!(DataTypeId::Map.is_map() && !DataTypeId::Map.is_struct() && !DataTypeId::Map.is_list());

    // A non-nested type answers every nested predicate false.
    assert!(
        !DataTypeId::I64.is_nested()
            && !DataTypeId::I64.is_struct()
            && !DataTypeId::I64.is_list()
            && !DataTypeId::I64.is_map()
    );

    // The nested ids still round-trip through their u16 band (0x0700 / 0x0710 / 0x0720).
    assert_eq!(DataTypeId::from_u16(0x0700), DataTypeId::Struct);
    assert_eq!(DataTypeId::from_u16(0x0710), DataTypeId::List);
    assert_eq!(DataTypeId::from_u16(0x0720), DataTypeId::Map);
}
