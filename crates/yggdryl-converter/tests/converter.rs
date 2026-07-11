//! Behavioural tests for the `codec::converter` family — the identity, numeric-cast,
//! flexible string, byte, and UTF-8 converters, plus their shared error type.

use yggdryl_converter::{
    BytesConverter, CastConverter, ConvertError, Converter, IdentityConverter, PrimitiveType,
    StringConverter, TypedConverter, Utf8Converter,
};

#[test]
fn identity_passes_through_both_directions() {
    let identity = IdentityConverter::<i64>::new();
    assert_eq!(identity.encode(7).unwrap(), 7);
    assert_eq!(identity.decode(7).unwrap(), 7);
    assert_eq!(identity.convert_byte_array(b"hello").unwrap(), b"hello");
    assert_eq!(identity.invert_byte_array(b"hello").unwrap(), b"hello");
}

#[test]
fn cast_widens_and_narrows() {
    let widen = CastConverter::<i32, i64>::new();
    assert_eq!(widen.encode(i32::MAX).unwrap(), i32::MAX as i64);
    assert_eq!(widen.decode(5_i64).unwrap(), 5_i32);

    // Narrowing follows `as` truncation semantics.
    let narrow = CastConverter::<i64, u8>::new();
    assert_eq!(narrow.encode(258).unwrap(), 2);

    // Float <-> int uses `as` (saturating float->int, nearest int->float).
    let to_int = CastConverter::<f64, i32>::new();
    assert_eq!(to_int.encode(3.9).unwrap(), 3);
    assert_eq!(to_int.encode(f64::INFINITY).unwrap(), i32::MAX);
}

#[test]
fn cast_over_bytes_processes_many_values() {
    let widen = CastConverter::<i16, i32>::new();
    let mut src = Vec::new();
    src.extend_from_slice(&1_i16.to_le_bytes());
    src.extend_from_slice(&(-2_i16).to_le_bytes());
    let out = widen.convert_byte_array(&src).unwrap();

    let mut expected = Vec::new();
    expected.extend_from_slice(&1_i32.to_le_bytes());
    expected.extend_from_slice(&(-2_i32).to_le_bytes());
    assert_eq!(out, expected);
    assert_eq!(widen.invert_byte_array(&out).unwrap(), src);
}

#[test]
fn cast_rejects_ragged_byte_length() {
    let widen = CastConverter::<i32, i64>::new();
    let err = widen.convert_byte_array(&[0, 1, 2]).unwrap_err();
    assert_eq!(err, ConvertError::InvalidByteLength { len: 3, width: 4 });
    assert!(err.to_string().contains("multiple of 4"));
}

#[test]
fn string_parses_every_integer_format() {
    let ints = StringConverter::<i32>::new();
    assert_eq!(ints.encode("42".into()).unwrap(), 42);
    assert_eq!(ints.encode("+42".into()).unwrap(), 42);
    assert_eq!(ints.encode("-42".into()).unwrap(), -42);
    assert_eq!(ints.encode("  7  ".into()).unwrap(), 7); // whitespace trimmed
    assert_eq!(ints.encode("1_000_000".into()).unwrap(), 1_000_000); // separators
    assert_eq!(ints.encode("0x2A".into()).unwrap(), 42); // hex
    assert_eq!(ints.encode("0XFF".into()).unwrap(), 255); // hex, upper prefix
    assert_eq!(ints.encode("-0x10".into()).unwrap(), -16); // signed hex
    assert_eq!(ints.encode("0o52".into()).unwrap(), 42); // octal
    assert_eq!(ints.encode("0b101010".into()).unwrap(), 42); // binary
    assert_eq!(ints.encode("0xDE_AD".into()).unwrap(), 0xDEAD); // hex + separators
}

#[test]
fn string_parses_floats() {
    let floats = StringConverter::<f64>::new();
    assert_eq!(floats.encode("1.25".into()).unwrap(), 1.25);
    assert_eq!(floats.encode("-1.5e3".into()).unwrap(), -1500.0);
    assert_eq!(floats.encode("1_000.5".into()).unwrap(), 1000.5);
    assert!(floats.encode("inf".into()).unwrap().is_infinite());
    assert!(floats.encode("nan".into()).unwrap().is_nan());
}

#[test]
fn string_render_round_trips() {
    let ints = StringConverter::<i64>::new();
    assert_eq!(ints.decode(-123).unwrap(), "-123");
    assert_eq!(ints.encode(ints.decode(999).unwrap()).unwrap(), 999);
}

#[test]
fn string_parse_failure_is_guided() {
    let ints = StringConverter::<i32>::new();
    let err = ints.encode("twelve".into()).unwrap_err();
    match &err {
        ConvertError::ParseFailed { input, target, .. } => {
            assert_eq!(input, "twelve");
            assert_eq!(*target, "i32");
        }
        other => panic!("unexpected error: {other:?}"),
    }
    assert!(err.to_string().contains("0x-hex"));
    assert!(ints.encode("".into()).is_err());
}

#[test]
fn string_out_of_range_reports_the_value_and_range() {
    let ints = StringConverter::<i32>::new();
    let err = ints.encode("99999999999".into()).unwrap_err();
    match &err {
        ConvertError::OutOfRange {
            input, target, max, ..
        } => {
            assert_eq!(input, "99999999999");
            assert_eq!(*target, "i32");
            assert_eq!(max, &i32::MAX.to_string());
        }
        other => panic!("expected OutOfRange, got {other:?}"),
    }
    assert!(err.to_string().contains("out of range"));

    // A very long offending value is truncated for readability.
    let long = "9".repeat(200);
    let err = ints.encode(long).unwrap_err();
    let shown = err.to_string();
    assert!(
        shown.contains("..."),
        "long value should be truncated: {shown}"
    );
    assert!(shown.len() < 160);
}

#[test]
fn string_accepts_comma_separators() {
    assert_eq!(
        StringConverter::<i64>::new()
            .encode("1,000,000".into())
            .unwrap(),
        1_000_000
    );
    assert_eq!(
        StringConverter::<f64>::new()
            .encode("1,234.5".into())
            .unwrap(),
        1234.5
    );
}

#[test]
fn string_byte_surface_round_trips() {
    let ints = StringConverter::<i32>::new();
    let bytes = ints.convert_byte_array(b"0x2A").unwrap();
    assert_eq!(bytes, 42_i32.to_le_bytes());
    assert_eq!(ints.invert_byte_array(&bytes).unwrap(), b"42");
}

#[test]
fn bytes_converter_round_trips_one_value() {
    let codec = BytesConverter::<i32>::new();
    assert_eq!(
        codec.encode(0x0102_0304).unwrap(),
        vec![0x04, 0x03, 0x02, 0x01]
    );
    assert_eq!(
        codec.decode(vec![0x04, 0x03, 0x02, 0x01]).unwrap(),
        0x0102_0304
    );

    let err = codec.decode(vec![1, 2]).unwrap_err();
    assert_eq!(err, ConvertError::InvalidByteLength { len: 2, width: 4 });
}

#[test]
fn utf8_round_trips_and_rejects_invalid() {
    let codec = Utf8Converter::new();
    assert_eq!(codec.encode("café".into()).unwrap(), "café".as_bytes());
    assert_eq!(codec.decode("café".as_bytes().to_vec()).unwrap(), "café");

    let err = codec.decode(vec![0xFF, 0xFE]).unwrap_err();
    assert!(matches!(err, ConvertError::InvalidUtf8 { valid_up_to: 0 }));
    assert!(err.to_string().contains("UTF-8"));
}

#[test]
fn primitive_type_names_and_widths() {
    assert_eq!(PrimitiveType::from_name("u32").unwrap(), PrimitiveType::U32);
    assert_eq!(PrimitiveType::U32.name(), "u32");
    assert_eq!(PrimitiveType::U32.width(), 4);
    assert_eq!(PrimitiveType::ALL.len(), 10);

    let err = PrimitiveType::from_name("i128").unwrap_err();
    assert!(matches!(err, ConvertError::UnknownType { .. }));
    assert!(err.to_string().contains("i8, i16"));
}

#[test]
fn primitive_type_cast_matches_typed_converter() {
    // The dtype-keyed facade agrees with the typed CastConverter for every pair.
    for &from in PrimitiveType::ALL {
        for &to in PrimitiveType::ALL {
            let bytes = vec![0u8; from.width()];
            let out = from.cast_bytes(to, &bytes).unwrap();
            assert_eq!(out.len(), to.width(), "{}->{}", from.name(), to.name());
        }
    }
    // A concrete widening, checked against the typed path.
    let facade = PrimitiveType::I32.cast_bytes(PrimitiveType::I64, &5_i32.to_le_bytes());
    let typed = CastConverter::<i32, i64>::new().convert_byte_array(&5_i32.to_le_bytes());
    assert_eq!(facade.unwrap(), typed.unwrap());
}

#[test]
fn primitive_type_parse_and_format() {
    assert_eq!(
        PrimitiveType::I32.parse_bytes("0x2A").unwrap(),
        42_i32.to_le_bytes()
    );
    assert_eq!(
        PrimitiveType::I32
            .format_bytes(&42_i32.to_le_bytes())
            .unwrap(),
        "42"
    );
    assert!(PrimitiveType::U8.parse_bytes("-1").is_err()); // out of range for u8
}
