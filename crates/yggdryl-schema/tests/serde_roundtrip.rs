//! With the `serde` feature on, every schema type round-trips through JSON
//! and deserialization re-validates constructor invariants.

#![cfg(feature = "serde")]

use std::sync::Arc;

use serde::{de::DeserializeOwned, Serialize};
use yggdryl_schema::{
    AnyDataType, AnyTime32Unit, BooleanType, Decimal128Type, DecimalType, Field,
    FixedSizeBinaryType, Int32Type, ListType, MapType, Millisecond, Nanosecond, StructType, Time,
    Time32Type, Timestamp, TimestampType, TypedField, Utf8Type,
};

fn assert_roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
}

#[test]
fn schema_types_roundtrip_through_json() {
    assert_roundtrip(BooleanType);
    assert_roundtrip(Int32Type);
    assert_roundtrip(Decimal128Type::from_parts(38, 10).unwrap());
    assert_roundtrip(FixedSizeBinaryType::from_parts(16).unwrap());
    assert_roundtrip(Time32Type::from_parts(Millisecond));
    assert_roundtrip(TimestampType::from_parts(Nanosecond, Some("UTC".into())));

    let metadata = [("k".to_string(), "v".to_string())].into_iter().collect();
    assert_roundtrip(TypedField::from_parts("id", Int32Type, false, metadata));

    let item = Arc::new(TypedField::from_parts(
        "item",
        Int32Type,
        true,
        Default::default(),
    ));
    assert_roundtrip(ListType::from_parts(item));

    let person = StructType::from_parts(vec![
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
    assert_roundtrip(person.clone());
    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person,
        false,
        Default::default(),
    ));
    let map = MapType::from_parts(entries, false).unwrap();
    assert_roundtrip(map.clone());
    assert_roundtrip(AnyDataType::from(map));
}

#[test]
fn deserialization_revalidates_invariants() {
    assert!(serde_json::from_str::<Decimal128Type>(r#"{"precision":39,"scale":0}"#).is_err());
    assert!(serde_json::from_str::<Decimal128Type>(r#"{"precision":10,"scale":11}"#).is_err());
    assert!(serde_json::from_str::<FixedSizeBinaryType>(r#"{"size":-1}"#).is_err());
    assert!(serde_json::from_str::<Time32Type<AnyTime32Unit>>(
        r#"{"unit":{"unit_id":"Nanosecond"}}"#
    )
    .is_err());

    // A map with a nullable key is re-validated on the way in.
    let person = StructType::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "key",
            Utf8Type.into(),
            true,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "value",
            Int32Type.into(),
            true,
            Default::default(),
        )),
    ]);
    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person,
        false,
        Default::default(),
    ));
    let json = serde_json::json!({ "entries": &*entries, "sorted": false });
    assert!(serde_json::from_value::<MapType>(json).is_err());
}
