//! Cross-cutting tests for the data-type, scalar/IO and field layers.

use std::collections::BTreeMap;
use std::collections::HashMap;

use crate::{
    AnyField, AnyScalar, AnyType, Binary, BinaryBased, BinaryType, DataType, Field, Io,
    PrimitiveField, Scalar, TypeCategory, TypeError, Utf8, Utf8Type, Whence,
};

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

#[test]
fn binary_value_and_io() {
    let buf = Binary::from_bytes(&[0u8, 1, 2, 3]);
    assert_eq!(buf.as_slice(), &[0u8, 1, 2, 3]);
    assert_eq!(buf.len(), 4);
    assert_eq!(buf.data_type(), AnyType::Binary(BinaryType::new()));
    assert_eq!(Binary::from_bytes(&buf.to_bytes()), buf);
    assert_eq!(Binary::from_mapping(&buf.to_mapping()).unwrap(), buf);

    let mut io = Binary::new();
    io.write(b"hello ").unwrap();
    io.write(b"world").unwrap();
    assert_eq!(io.size(), 11);
    io.seek(0, Whence::Start).unwrap();
    assert_eq!(io.read(5).unwrap().as_slice(), b"hello");
    assert_eq!(io.pread(6, 5).unwrap().as_slice(), b"world");
    assert!(io.seek(-100, Whence::Start).is_err());
}

#[test]
fn utf8_value_round_trips() {
    let s = Utf8::new("héllo");
    assert_eq!(s.as_str(), "héllo");
    assert_eq!(s.as_bytes(), "héllo".as_bytes());
    assert_eq!(s.data_type(), AnyType::Utf8(Utf8Type::new()));
    assert_eq!(Utf8::from_str("héllo"), s);
    assert_eq!(Utf8::from_bytes(&s.to_bytes()).unwrap(), s);
    assert_eq!(Utf8::from_mapping(&s.to_mapping()).unwrap(), s);
    assert!(Utf8::from_bytes(&[0xff, 0xfe]).is_err());

    let large = s.with_data_type(Utf8Type::large());
    assert_eq!(large.string_type(), Utf8Type::large());
    assert_eq!(Utf8::from_mapping(&large.to_mapping()).unwrap(), large);

    // content-based hashing
    let mut seen: HashMap<Utf8, u32> = HashMap::new();
    *seen.entry(Utf8::new("a")).or_default() += 1;
    *seen.entry(Utf8::new("a")).or_default() += 1;
    assert_eq!(seen[&Utf8::new("a")], 2);
}

#[test]
fn scalar_cast_and_set_data_type() {
    let bytes = Binary::from_bytes(b"hi");

    // same-family set_data_type re-labels the variant.
    let mut relabelled = bytes.clone();
    relabelled.set_data_type(&BinaryType::large()).unwrap();
    assert_eq!(relabelled.binary_type(), BinaryType::large());

    // cross-family set_data_type errors.
    let mut wrong = bytes.clone();
    assert!(wrong.set_data_type(&Utf8Type::new()).is_err());

    // cast binary -> string and back.
    let text = bytes.cast(&Utf8Type::new()).unwrap();
    assert_eq!(text, AnyScalar::Utf8(Utf8::new("hi")));
    assert_eq!(text.data_type(), AnyType::Utf8(Utf8Type::new()));
    assert_eq!(
        text.cast(&BinaryType::new()).unwrap(),
        AnyScalar::Binary(bytes.clone())
    );

    // cast binary -> string fails on non-UTF-8.
    assert!(Binary::from_bytes(&[0xff, 0xfe])
        .cast(&Utf8Type::new())
        .is_err());

    // cast via the type carrier works too.
    assert!(Utf8::new("x")
        .cast(&AnyType::from_str("binary").unwrap())
        .is_ok());
}

#[test]
fn field_round_trips_with_metadata() {
    let mut metadata = BTreeMap::new();
    metadata.insert("unit".to_string(), "bytes".to_string());
    let field = Field::new("payload", BinaryType::large().to_any(), false).with_metadata(metadata);

    let mapping = field.to_mapping();
    assert_eq!(mapping["type"], "large_binary");
    assert_eq!(AnyField::from_mapping(&mapping).unwrap(), field);
    assert_eq!(AnyField::from_bytes(&field.to_bytes()).unwrap(), field);
}

#[test]
fn field_nullable_defaults_to_true() {
    let mut mapping = BTreeMap::new();
    mapping.insert("name".to_string(), "id".to_string());
    mapping.insert("type".to_string(), "string".to_string());
    assert!(AnyField::from_mapping(&mapping).unwrap().is_nullable());
}

#[test]
fn typed_field_is_a_primitive_field() {
    fn assert_primitive<F: PrimitiveField>(_: &F) {}
    let field = Field::new("x", Utf8Type::new(), false);
    assert_primitive(&field); // compile-time proof Field<Utf8Type>: PrimitiveField
    assert_eq!(field.to_any().data_type().to_str(), "string");
}

#[cfg(feature = "serde")]
#[test]
fn serde_round_trips_through_json() {
    let ty = AnyType::Utf8(Utf8Type::large());
    assert_eq!(serde_json::to_string(&ty).unwrap(), "\"large_string\"");
    assert_eq!(
        serde_json::from_str::<AnyType>("\"large_string\"").unwrap(),
        ty
    );

    let field = Field::new("c", BinaryType::new().to_any(), true);
    let json = serde_json::to_string(&field).unwrap();
    assert_eq!(serde_json::from_str::<AnyField>(&json).unwrap(), field);

    let buf = Binary::from_bytes(b"hi");
    assert_eq!(
        serde_json::from_str::<Binary>(&serde_json::to_string(&buf).unwrap()).unwrap(),
        buf
    );

    let text = Utf8::new("hi");
    let json = serde_json::to_string(&text).unwrap();
    assert_eq!(json, r#"{"type":"string","value":"hi"}"#);
    assert_eq!(serde_json::from_str::<Utf8>(&json).unwrap(), text);
}

#[cfg(feature = "json")]
#[test]
fn json_helpers_round_trip() {
    use crate::Jsonable;

    let ty = AnyType::Binary(BinaryType::large());
    assert_eq!(AnyType::from_json(&ty.to_json()).unwrap(), ty);
    assert_eq!(AnyType::from_bson(&ty.to_bson()).unwrap(), ty); // bytes round-trip too

    let field = Field::new("c", Utf8Type::new().to_any(), false);
    assert_eq!(AnyField::from_json(&field.to_json()).unwrap(), field);

    let buf = Binary::from_bytes(&[9u8, 9, 9]);
    assert_eq!(Binary::from_json(&buf.to_json()).unwrap(), buf);
    assert_eq!(Binary::from_bson(&buf.to_bson()).unwrap(), buf);

    let text = Utf8::new("hi");
    assert_eq!(Utf8::from_json(&text.to_json()).unwrap(), text);
}

#[cfg(feature = "json")]
#[test]
fn global_json_params_control_to_json() {
    use crate::{json_params, reset_json_params, set_json_params, Charset, JsonParams, Jsonable};

    let field = Field::new("c", BinaryType::new().to_any(), true);
    assert!(!field.to_json().contains('\n')); // compact by default

    set_json_params(JsonParams::pretty().with_indent(2));
    assert!(json_params().is_pretty());
    let pretty = field.to_json();
    assert!(pretty.contains('\n') && pretty.contains("  \"name\""));
    assert_eq!(AnyField::from_json(&pretty).unwrap(), field); // still round-trips

    // the charset drives the byte form: Latin-1 encodes 'é' as one byte (0xE9),
    // not the two UTF-8 bytes, and still round-trips.
    set_json_params(JsonParams::compact().with_charset(Charset::Latin1));
    let text = Utf8::new("é");
    assert!(text.to_bson().contains(&0xe9));
    assert_eq!(Utf8::from_bson(&text.to_bson()).unwrap(), text);

    reset_json_params();
    assert!(!field.to_json().contains('\n'));
    assert_eq!(json_params(), JsonParams::default());
}
