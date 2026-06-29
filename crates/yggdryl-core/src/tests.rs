//! Cross-cutting tests for the data-type, scalar/IO and field layers.

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::{
    AnyField, AnyType, Binary, BinaryBased, BinaryType, DataType, Field, Io, PrimitiveField,
    Scalar, TypeCategory, TypeError, Utf8, Whence,
};

#[test]
fn datatype_string_round_trips() {
    for (name, expected) in [
        ("binary", AnyType::Binary(BinaryType::new())),
        ("large_binary", AnyType::Binary(BinaryType::large())),
        ("string", AnyType::Utf8(Utf8::new())),
        ("large_string", AnyType::Utf8(Utf8::large())),
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
    assert_eq!(Utf8::from_str("utf8").unwrap(), Utf8::new());
    assert_eq!(Utf8::from_str("large_utf8").unwrap(), Utf8::large());
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
    assert!(Utf8::new().is_utf8());
    assert!(!Utf8::new().is_large());
}

#[test]
fn datatype_is_hashable() {
    let mut counts: HashMap<AnyType, u32> = HashMap::new();
    *counts.entry(BinaryType::new().to_any()).or_default() += 1;
    *counts.entry(BinaryType::new().to_any()).or_default() += 1;
    *counts.entry(Utf8::new().to_any()).or_default() += 1;
    assert_eq!(counts[&AnyType::Binary(BinaryType::new())], 2);
    assert_eq!(counts[&AnyType::Utf8(Utf8::new())], 1);
}

#[test]
fn binary_borrows_without_copy_and_is_hashable() {
    let buf = Binary::from_bytes(&[0u8, 1, 2, 3]);
    assert_eq!(buf.as_slice(), &[0u8, 1, 2, 3]);
    assert_eq!(buf.len(), 4);
    assert!(!buf.is_empty());
    assert_eq!(buf.data_type(), AnyType::Binary(BinaryType::new()));

    // Equality/hashing are content-based (cursor and capacity excluded).
    let mut seen: HashMap<Binary, u32> = HashMap::new();
    *seen.entry(Binary::from_bytes(b"x")).or_default() += 1;
    *seen.entry(Binary::from_bytes(b"x")).or_default() += 1;
    assert_eq!(seen[&Binary::from_bytes(b"x")], 2);
}

#[test]
fn binary_byte_and_mapping_round_trips() {
    // to_bytes/from_bytes round-trips the raw content (always `binary` back).
    for bytes in [vec![0u8, 1, 255], Vec::new(), b"hello".to_vec()] {
        let buf = Binary::from_bytes(&bytes);
        assert_eq!(buf.to_bytes(), bytes);
        assert_eq!(Binary::from_bytes(&buf.to_bytes()), buf);
    }
    // The component map carries the type variant too.
    for buf in [
        Binary::from_bytes(b"x"),
        Binary::from_bytes(b"x").with_data_type(BinaryType::large()),
    ] {
        assert_eq!(Binary::from_mapping(&buf.to_mapping()).unwrap(), buf);
    }
    assert_eq!(
        Binary::from_bytes(b"x")
            .with_data_type(BinaryType::large())
            .binary_type(),
        BinaryType::large()
    );
}

#[test]
fn binary_implements_io_with_zero_copy_reads() {
    let mut buf = Binary::new();
    buf.write(b"hello ").unwrap();
    buf.write(b"world").unwrap();
    assert_eq!(buf.size(), 11);
    assert!(buf.capacity() >= 11);

    buf.seek(0, Whence::Start).unwrap();
    let head = buf.read(5).unwrap();
    assert_eq!(head.as_slice(), b"hello");
    assert_eq!(buf.tell(), 5);
    assert_eq!(buf.seek(-1, Whence::End).unwrap(), 10);
    assert!(buf.seek(-100, Whence::Start).is_err());

    // The whole buffer as a zero-copy view, and the positional API.
    assert_eq!(buf.to_buffer().as_slice(), b"hello world");
    assert_eq!(buf.pread(6, 5).unwrap().as_slice(), b"world");
}

#[test]
fn field_round_trips_with_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert("unit".to_string(), "bytes".to_string());
    let field = Field::new("payload", BinaryType::large().to_any(), false).with_metadata(metadata);

    let mapping = field.to_mapping();
    assert_eq!(mapping["name"], "payload");
    assert_eq!(mapping["type"], "large_binary");
    assert_eq!(mapping["nullable"], "false");
    assert_eq!(mapping["metadata.unit"], "bytes");

    assert_eq!(AnyField::from_mapping(&mapping).unwrap(), field);
    assert_eq!(AnyField::from_bytes(&field.to_bytes()).unwrap(), field);
}

#[test]
fn field_nullable_defaults_to_true() {
    let mut mapping = BTreeMap::new();
    mapping.insert("name".to_string(), "id".to_string());
    mapping.insert("type".to_string(), "string".to_string());
    let field = AnyField::from_mapping(&mapping).unwrap();
    assert!(field.is_nullable());
}

#[test]
fn typed_field_is_a_primitive_field() {
    fn assert_primitive<F: PrimitiveField>(_: &F) {}
    let field = Field::new("x", BinaryType::new(), false);
    assert_primitive(&field); // compile-time proof Field<BinaryType>: PrimitiveField
    assert_eq!(field.to_any().data_type().to_str(), "binary");
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trips_through_json() {
    let ty = AnyType::Utf8(Utf8::large());
    let json = serde_json::to_string(&ty).unwrap();
    assert_eq!(json, "\"large_string\"");
    assert_eq!(serde_json::from_str::<AnyType>(&json).unwrap(), ty);

    let field = Field::new("c", BinaryType::new().to_any(), true);
    let json = serde_json::to_string(&field).unwrap();
    assert_eq!(serde_json::from_str::<AnyField>(&json).unwrap(), field);

    let buf = Binary::from_bytes(b"hi");
    let json = serde_json::to_string(&buf).unwrap();
    assert_eq!(serde_json::from_str::<Binary>(&json).unwrap(), buf);
}

#[cfg(feature = "json")]
#[test]
fn json_helpers_round_trip() {
    let ty = AnyType::Binary(BinaryType::large());
    assert_eq!(AnyType::from_json(&ty.to_json()).unwrap(), ty);

    let field = Field::new("c", Utf8::new().to_any(), false);
    assert_eq!(AnyField::from_json(&field.to_json()).unwrap(), field);

    let buf = Binary::from_bytes(&[9u8, 9, 9]);
    assert_eq!(Binary::from_json(&buf.to_json()).unwrap(), buf);
}
