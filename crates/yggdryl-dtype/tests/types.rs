//! Tests for the Arrow data-type layer.

use std::collections::HashMap;

use yggdryl_core::TypeError;
use yggdryl_dtype::{AnyType, BinaryBased, BinaryType, DataType, TypeCategory, Utf8Type};

#[test]
fn datatype_string_round_trips() {
    for (name, expected) in [
        ("binary", AnyType::Binary(BinaryType::new())),
        ("large_binary", AnyType::Binary(BinaryType::large())),
        ("string", AnyType::Utf8(Utf8Type::new())),
        ("large_string", AnyType::Utf8(Utf8Type::large())),
    ] {
        let parsed = AnyType::from_str(name).unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(parsed.to_str(), name);
        assert_eq!(AnyType::from_bytes(&parsed.to_bytes()).unwrap(), parsed);
        assert_eq!(AnyType::from_mapping(&parsed.to_mapping()).unwrap(), parsed);
    }
}

#[test]
fn datatype_aliases_and_errors() {
    assert_eq!(Utf8Type::from_str("utf8").unwrap(), Utf8Type::new());
    assert_eq!(Utf8Type::from_str("large_utf8").unwrap(), Utf8Type::large());
    assert!(matches!(
        AnyType::from_str("flob"),
        Err(TypeError::UnknownType(_))
    ));
}

#[test]
fn datatype_categories_and_flags() {
    assert_eq!(BinaryType::new().category(), TypeCategory::Primitive);
    assert!(!BinaryType::new().is_utf8());
    assert!(BinaryType::large().is_large());
    assert!(Utf8Type::new().is_utf8());
    assert!(!Utf8Type::new().is_large());
}

#[test]
fn datatype_is_hashable() {
    let mut counts: HashMap<AnyType, u32> = HashMap::new();
    *counts.entry(BinaryType::new().to_any()).or_default() += 1;
    *counts.entry(BinaryType::new().to_any()).or_default() += 1;
    *counts.entry(Utf8Type::new().to_any()).or_default() += 1;
    assert_eq!(counts[&AnyType::Binary(BinaryType::new())], 2);
    assert_eq!(counts[&AnyType::Utf8(Utf8Type::new())], 1);
}

#[cfg(feature = "serde")]
#[test]
fn datatype_serde_round_trips_to_canonical_string() {
    let ty = AnyType::Utf8(Utf8Type::large());
    assert_eq!(serde_json::to_string(&ty).unwrap(), "\"large_string\"");
    assert_eq!(
        serde_json::from_str::<AnyType>("\"large_string\"").unwrap(),
        ty
    );
}

#[cfg(feature = "json")]
#[test]
fn datatype_json_helpers_round_trip() {
    use yggdryl_core::Jsonable;

    let ty = AnyType::Binary(BinaryType::large());
    assert_eq!(AnyType::from_json(&ty.to_json()).unwrap(), ty);
    assert_eq!(AnyType::from_bson(&ty.to_bson()).unwrap(), ty);
}
