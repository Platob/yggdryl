//! Every data type exposes the identifier of its constructor, and the
//! identifiers round-trip through their stable integer values.

use std::sync::Arc;

use yggdryl_schema::{
    AnyDataType, BinaryType, BooleanType, DataType, DataTypeError, DataTypeId, Date32Type,
    Date64Type, Decimal128Type, Decimal256Type, DecimalType, Duration, DurationType, Field,
    FixedSizeBinaryType, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type, Int8Type,
    LargeBinaryType, LargeListType, LargeUtf8Type, ListType, MapType, Millisecond, Nanosecond,
    Second, StructType, Time, Time32Type, Time64Type, Timestamp, TimestampType, TypedField,
    UInt16Type, UInt32Type, UInt64Type, UInt8Type, Utf8Type,
};

#[test]
fn every_type_exposes_its_constructor_id() {
    assert_eq!(BooleanType.type_id(), DataTypeId::Boolean);
    assert_eq!(Int8Type.type_id(), DataTypeId::Int8);
    assert_eq!(Int16Type.type_id(), DataTypeId::Int16);
    assert_eq!(Int32Type.type_id(), DataTypeId::Int32);
    assert_eq!(Int64Type.type_id(), DataTypeId::Int64);
    assert_eq!(UInt8Type.type_id(), DataTypeId::UInt8);
    assert_eq!(UInt16Type.type_id(), DataTypeId::UInt16);
    assert_eq!(UInt32Type.type_id(), DataTypeId::UInt32);
    assert_eq!(UInt64Type.type_id(), DataTypeId::UInt64);
    assert_eq!(Float32Type.type_id(), DataTypeId::Float32);
    assert_eq!(Float64Type.type_id(), DataTypeId::Float64);
    assert_eq!(Utf8Type.type_id(), DataTypeId::Utf8);
    assert_eq!(LargeUtf8Type.type_id(), DataTypeId::LargeUtf8);
    assert_eq!(BinaryType.type_id(), DataTypeId::Binary);
    assert_eq!(LargeBinaryType.type_id(), DataTypeId::LargeBinary);
    assert_eq!(Date32Type.type_id(), DataTypeId::Date32);
    assert_eq!(Date64Type.type_id(), DataTypeId::Date64);
}

#[test]
fn parameterized_types_share_their_constructor_id() {
    assert_eq!(
        Decimal128Type::from_parts(38, 10).unwrap().type_id(),
        DataTypeId::Decimal128
    );
    assert_eq!(
        Decimal128Type::from_parts(1, 0).unwrap().type_id(),
        DataTypeId::Decimal128
    );
    assert_eq!(
        Decimal256Type::from_parts(76, 0).unwrap().type_id(),
        DataTypeId::Decimal256
    );
    assert_eq!(
        FixedSizeBinaryType::from_parts(16).unwrap().type_id(),
        DataTypeId::FixedSizeBinary
    );
    assert_eq!(Time32Type::from_parts(Second).type_id(), DataTypeId::Time32);
    assert_eq!(
        Time64Type::from_parts(Nanosecond).type_id(),
        DataTypeId::Time64
    );
    assert_eq!(
        TimestampType::from_parts(Millisecond, Some("UTC".into())).type_id(),
        DataTypeId::Timestamp
    );
    assert_eq!(
        DurationType::from_parts(Second).type_id(),
        DataTypeId::Duration
    );

    // Every parameterization of a generic list shares the constructor id.
    let int_item = Arc::new(TypedField::from_parts(
        "item",
        Int32Type,
        true,
        Default::default(),
    ));
    let utf8_item = Arc::new(TypedField::from_parts(
        "item",
        Utf8Type,
        true,
        Default::default(),
    ));
    assert_eq!(
        ListType::from_parts(int_item.clone()).type_id(),
        DataTypeId::List
    );
    assert_eq!(ListType::from_parts(utf8_item).type_id(), DataTypeId::List);
    assert_eq!(
        LargeListType::from_parts(int_item).type_id(),
        DataTypeId::LargeList
    );

    let entries = StructType::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "key",
            Utf8Type.into(),
            false,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "value",
            Int32Type.into(),
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
    let map = MapType::from_parts(entries, false).unwrap();
    assert_eq!(map.type_id(), DataTypeId::Map);

    // The erased type reports the wrapped constructor's id.
    assert_eq!(AnyDataType::from(Int32Type).type_id(), DataTypeId::Int32);
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
