//! With the `serde` feature on, every schema type round-trips through JSON
//! and deserialization re-validates constructor invariants.

#![cfg(feature = "serde")]

use std::sync::Arc;

use serde::{de::DeserializeOwned, Serialize};
use yggdryl_schema::{
    AnyDataType, Boolean, Decimal128, Field, FixedSizeBinary, Int32, List, Map, Nanosecond, Struct,
    Time32, TimeUnitId, Timestamp, TypedField, Utf8,
};

fn assert_roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: T) {
    let json = serde_json::to_string(&value).unwrap();
    assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
}

#[test]
fn schema_types_roundtrip_through_json() {
    assert_roundtrip(Boolean);
    assert_roundtrip(Int32);
    assert_roundtrip(Decimal128::from_parts(38, 10).unwrap());
    assert_roundtrip(FixedSizeBinary::from_parts(16).unwrap());
    assert_roundtrip(Time32::from_parts(TimeUnitId::Millisecond).unwrap());
    assert_roundtrip(Timestamp::from_parts(Nanosecond, Some("UTC".into())));

    let metadata = [("k".to_string(), "v".to_string())].into_iter().collect();
    assert_roundtrip(TypedField::from_parts("id", Int32, false, metadata));

    let item = Arc::new(TypedField::from_parts(
        "item",
        Int32,
        true,
        Default::default(),
    ));
    assert_roundtrip(List::from_parts(item));

    let person = Struct::from_parts(vec![
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
    assert_roundtrip(person.clone());
    let entries = Arc::new(TypedField::from_parts(
        "entries",
        person,
        false,
        Default::default(),
    ));
    let map = Map::from_parts(entries, false).unwrap();
    assert_roundtrip(map.clone());
    assert_roundtrip(AnyDataType::from(map));
}

#[test]
fn deserialization_revalidates_invariants() {
    assert!(serde_json::from_str::<Decimal128>(r#"{"precision":39,"scale":0}"#).is_err());
    assert!(serde_json::from_str::<Decimal128>(r#"{"precision":10,"scale":11}"#).is_err());
    assert!(serde_json::from_str::<FixedSizeBinary>(r#"{"size":-1}"#).is_err());
    assert!(serde_json::from_str::<Time32>(r#"{"unit":"Nanosecond"}"#).is_err());

    // A map with a nullable key is re-validated on the way in.
    let person = Struct::from_parts(vec![
        Arc::new(TypedField::from_parts(
            "key",
            Utf8.into(),
            true,
            Default::default(),
        )),
        Arc::new(TypedField::from_parts(
            "value",
            Int32.into(),
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
    assert!(serde_json::from_value::<Map>(json).is_err());
}
