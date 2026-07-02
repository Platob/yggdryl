//! Every data type exposes the identifier of its constructor, and the
//! identifiers round-trip through their stable integer values.

use std::sync::Arc;

use yggdryl_schema::{
    AnyDataType, Binary, Boolean, DataType, DataTypeError, DataTypeId, Date32, Date64, Decimal128,
    Decimal256, Duration, Field, FixedSizeBinary, Float32, Float64, Int16, Int32, Int64, Int8,
    LargeBinary, LargeList, LargeUtf8, List, Map, Millisecond, Nanosecond, Second, Struct, Time,
    Time32, Time64, Timestamp, TypedDuration, TypedField, TypedTimestamp, UInt16, UInt32, UInt64,
    UInt8, Utf8,
};

#[test]
fn every_type_exposes_its_constructor_id() {
    assert_eq!(Boolean.type_id(), DataTypeId::Boolean);
    assert_eq!(Int8.type_id(), DataTypeId::Int8);
    assert_eq!(Int16.type_id(), DataTypeId::Int16);
    assert_eq!(Int32.type_id(), DataTypeId::Int32);
    assert_eq!(Int64.type_id(), DataTypeId::Int64);
    assert_eq!(UInt8.type_id(), DataTypeId::UInt8);
    assert_eq!(UInt16.type_id(), DataTypeId::UInt16);
    assert_eq!(UInt32.type_id(), DataTypeId::UInt32);
    assert_eq!(UInt64.type_id(), DataTypeId::UInt64);
    assert_eq!(Float32.type_id(), DataTypeId::Float32);
    assert_eq!(Float64.type_id(), DataTypeId::Float64);
    assert_eq!(Utf8.type_id(), DataTypeId::Utf8);
    assert_eq!(LargeUtf8.type_id(), DataTypeId::LargeUtf8);
    assert_eq!(Binary.type_id(), DataTypeId::Binary);
    assert_eq!(LargeBinary.type_id(), DataTypeId::LargeBinary);
    assert_eq!(Date32.type_id(), DataTypeId::Date32);
    assert_eq!(Date64.type_id(), DataTypeId::Date64);
}

#[test]
fn parameterized_types_share_their_constructor_id() {
    assert_eq!(
        Decimal128::from_parts(38, 10).unwrap().type_id(),
        DataTypeId::Decimal128
    );
    assert_eq!(
        Decimal128::from_parts(1, 0).unwrap().type_id(),
        DataTypeId::Decimal128
    );
    assert_eq!(
        Decimal256::from_parts(76, 0).unwrap().type_id(),
        DataTypeId::Decimal256
    );
    assert_eq!(
        FixedSizeBinary::from_parts(16).unwrap().type_id(),
        DataTypeId::FixedSizeBinary
    );
    assert_eq!(Time32::from_parts(Second).type_id(), DataTypeId::Time32);
    assert_eq!(Time64::from_parts(Nanosecond).type_id(), DataTypeId::Time64);
    assert_eq!(
        TypedTimestamp::from_parts(Millisecond, Some("UTC".into())).type_id(),
        DataTypeId::Timestamp
    );
    assert_eq!(
        TypedDuration::from_parts(Second).type_id(),
        DataTypeId::Duration
    );

    // Every parameterization of a generic list shares the constructor id.
    let int_item = Arc::new(TypedField::from_parts(
        "item",
        Int32,
        true,
        Default::default(),
    ));
    let utf8_item = Arc::new(TypedField::from_parts(
        "item",
        Utf8,
        true,
        Default::default(),
    ));
    assert_eq!(
        List::from_parts(int_item.clone()).type_id(),
        DataTypeId::List
    );
    assert_eq!(List::from_parts(utf8_item).type_id(), DataTypeId::List);
    assert_eq!(
        LargeList::from_parts(int_item).type_id(),
        DataTypeId::LargeList
    );

    let entries = Struct::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "key",
            Utf8.into(),
            false,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "value",
            Int32.into(),
            true,
            Default::default(),
        )),
    ]);
    assert_eq!(entries.type_id(), DataTypeId::Struct);
    let entries = Arc::new(TypedField::from_parts(
        "entries",
        entries,
        false,
        Default::default(),
    ));
    let map = Map::from_parts(entries, false).unwrap();
    assert_eq!(map.type_id(), DataTypeId::Map);

    // The erased type reports the wrapped constructor's id.
    assert_eq!(AnyDataType::from(Int32).type_id(), DataTypeId::Int32);
    assert_eq!(AnyDataType::from(map).type_id(), DataTypeId::Map);
}

#[test]
fn ids_roundtrip_exhaustively_and_reject_unassigned_values() {
    let mut assigned = 0;
    for value in 0..=u8::MAX {
        match DataTypeId::from_u8(value) {
            Ok(id) => {
                assert_eq!(id.to_u8(), value);
                assert_eq!(DataTypeId::from_bytes(&id.to_bytes()), Ok(id));
                assigned += 1;
            }
            Err(DataTypeError::UnknownTypeId { id, .. }) => assert_eq!(id, value),
            Err(other) => panic!("unexpected error for {value}: {other}"),
        }
    }
    assert_eq!(assigned, 28);

    assert!(matches!(
        DataTypeId::from_bytes(&[]),
        Err(DataTypeError::InvalidByteLength {
            expected: 1,
            actual: 0
        })
    ));
    assert!(matches!(
        DataTypeId::from_bytes(&[0, 0]),
        Err(DataTypeError::InvalidByteLength {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn ids_render_like_their_types() {
    assert_eq!(DataTypeId::Int8.to_string(), "int8");
    assert_eq!(DataTypeId::Decimal128.to_string(), "decimal128");
    assert_eq!(DataTypeId::LargeList.to_string(), "large_list");
}
