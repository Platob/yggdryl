//! Functional tests for [`DataTypeId`](yggdryl_core::datatype_id::DataTypeId) — the primitive element
//! data types: the `u16` round-trip, names, widths, predicates, and element counting.

use yggdryl_core::datatype_id::DataTypeId;

#[test]
fn u16_round_trip_is_total() {
    for dt in DataTypeId::ALL {
        assert_eq!(DataTypeId::from_u16(dt.as_u16()), dt);
    }
    assert_eq!(DataTypeId::from_u16(0), DataTypeId::Unknown);
    assert_eq!(DataTypeId::from_u16(9999), DataTypeId::Unknown); // foreign id degrades to raw
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
