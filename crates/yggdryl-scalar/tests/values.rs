//! Tests for the scalar value layer (including its cross-cutting JSON surface).

use std::collections::HashMap;

use yggdryl_core::{Io, Whence};
use yggdryl_dtype::{AnyType, BinaryType, Utf8Type};
use yggdryl_scalar::{AnyScalar, Binary, Scalar, Utf8};

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
    assert_eq!(io.read(5).unwrap().as_slice(), b"hello"); // zero-copy Buffer view
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

#[cfg(feature = "serde")]
#[test]
fn scalar_serde_round_trips_through_json() {
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

// The JSON form is cross-cutting: it ties the scalar values, the data types and a
// field together, so it lives here with `yggdryl-field` as a dev-dependency.
#[cfg(feature = "json")]
#[test]
fn json_helpers_round_trip() {
    use yggdryl_core::Jsonable;
    use yggdryl_dtype::DataType;
    use yggdryl_field::Field;

    let field = Field::new("c", Utf8Type::new().to_any(), false);
    assert_eq!(Field::from_json(&field.to_json()).unwrap(), field);

    let buf = Binary::from_bytes(&[9u8, 9, 9]);
    assert_eq!(Binary::from_json(&buf.to_json()).unwrap(), buf);
    assert_eq!(Binary::from_bson(&buf.to_bson()).unwrap(), buf);

    let text = Utf8::new("hi");
    assert_eq!(Utf8::from_json(&text.to_json()).unwrap(), text);
}

#[cfg(feature = "json")]
#[test]
fn global_json_params_control_to_json() {
    use yggdryl_core::{
        json_params, reset_json_params, set_json_params, Charset, JsonParams, Jsonable,
    };
    use yggdryl_dtype::DataType;
    use yggdryl_field::Field;

    let field = Field::new("c", BinaryType::new().to_any(), true);
    assert!(!field.to_json().contains('\n')); // compact by default

    set_json_params(JsonParams::pretty().with_indent(2));
    assert!(json_params().is_pretty());
    let pretty = field.to_json();
    assert!(pretty.contains('\n') && pretty.contains("  \"name\""));
    assert_eq!(Field::from_json(&pretty).unwrap(), field); // still round-trips

    // the charset drives the byte form: Latin-1 encodes 'é' as one byte (0xE9).
    set_json_params(JsonParams::compact().with_charset(Charset::Latin1));
    let text = Utf8::new("é");
    assert!(text.to_bson().contains(&0xe9));
    assert_eq!(Utf8::from_bson(&text.to_bson()).unwrap(), text);

    reset_json_params();
    assert!(!field.to_json().contains('\n'));
    assert_eq!(json_params(), JsonParams::default());
}
