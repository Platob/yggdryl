//! Behavioural tests for the `yggdryl-field` primitive fields — Arrow interop, the byte
//! codec, value semantics, and the guided error paths.

use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};
use yggdryl_dtype::DataType;
use yggdryl_field::{BooleanField, F64Field, Field, FieldError, I32Field, I64Field, TypedField};
use yggdryl_http::{Headers, HeadersBased};

#[test]
fn name_nullable_and_data_type() {
    let field = I64Field::new("id", false);
    assert_eq!(field.name(), "id");
    assert!(!field.is_nullable());
    assert_eq!(TypedField::data_type(&field).name(), "int64");
    assert_eq!(field.arrow_data_type(), ArrowDataType::Int64);

    let nullable = BooleanField::new("flag", true);
    assert!(nullable.is_nullable());
    assert_eq!(nullable.arrow_data_type(), ArrowDataType::Boolean);
}

#[test]
fn arrow_round_trips_and_mismatch_is_guided() {
    let field = I32Field::new("count", true);
    let arrow = field.to_arrow();
    assert_eq!(arrow.name(), "count");
    assert_eq!(arrow.data_type(), &ArrowDataType::Int32);
    assert!(arrow.is_nullable());
    assert_eq!(I32Field::from_arrow(&arrow).unwrap(), field);

    // A field whose Arrow data type is the wrong variant is a guided error.
    let wrong = ArrowField::new("count", ArrowDataType::Utf8, true);
    let err = I32Field::from_arrow(&wrong).unwrap_err();
    assert!(matches!(err, FieldError::Dtype(_)));
    assert!(err.to_string().contains("int32"));
}

#[test]
fn byte_round_trip_preserves_name_and_nullable() {
    for field in [
        I64Field::new("id", false),
        I64Field::new("", true),         // empty name is valid
        I64Field::new("mesure_€", true), // non-ASCII name
    ] {
        let bytes = field.serialize_bytes();
        assert_eq!(bytes[0], u8::from(field.is_nullable())); // first byte = nullable flag
        assert_eq!(I64Field::deserialize_bytes(&bytes).unwrap(), field);
    }
    // The float field round-trips through its own decoder too.
    let f = F64Field::new("x", true);
    assert_eq!(
        F64Field::deserialize_bytes(&f.serialize_bytes()).unwrap(),
        f
    );
}

#[test]
fn headers_round_trip_and_are_identity_bearing() {
    let headers = Headers::from_pairs([
        (b"unit".to_vec(), b"ms".to_vec()),
        (vec![0xFF, 0x00], b"binary".to_vec()), // non-UTF-8 key is fine
    ]);
    let field = I64Field::new("ts", true).with_headers(headers.clone());
    assert_eq!(field.headers(), Some(&headers));

    // Byte round-trip carries the headers.
    assert_eq!(
        I64Field::deserialize_bytes(&field.serialize_bytes()).unwrap(),
        field
    );

    // Headers are part of the field's identity.
    assert_ne!(field, I64Field::new("ts", true));

    // Arrow conversion drops the (bytes→bytes) headers.
    assert!(field.to_arrow().metadata().is_empty());
    assert_eq!(
        I64Field::from_arrow(&field.to_arrow()).unwrap().headers(),
        None
    );
}

#[test]
fn headers_based_get_add_update_delete() {
    let mut field = I64Field::new("ts", true);
    assert_eq!(field.get_header(b"unit"), None);

    // add (bytes) then update (string) — set returns the previous value.
    assert_eq!(field.set_header(b"unit".to_vec(), b"ms".to_vec()), None);
    assert_eq!(field.get_header_str("unit"), Some(b"ms".as_slice()));
    assert_eq!(field.set_header_str("unit", "us"), Some(b"ms".to_vec()));
    assert_eq!(field.get_header(b"unit"), Some(b"us".as_slice()));

    // Zero-copy in-place mutation of the value bytes.
    field
        .get_header_mut(b"unit")
        .unwrap()
        .extend_from_slice(b"ec");
    assert_eq!(field.get_header(b"unit"), Some(b"usec".as_slice()));

    // Pre-built common-key accessor.
    field.set_content_type("application/x.int64");
    assert_eq!(
        field.content_type(),
        Some(b"application/x.int64".as_slice())
    );

    // delete; the slot clears to None once empty.
    assert_eq!(field.remove_header_str("unit"), Some(b"usec".to_vec()));
    assert_eq!(
        field.remove_header(Headers::CONTENT_TYPE),
        Some(b"application/x.int64".to_vec())
    );
    assert!(field.headers().is_none());
}

#[test]
fn deserialize_rejects_empty_and_bad_utf8() {
    let err = I64Field::deserialize_bytes(&[]).unwrap_err();
    assert_eq!(err, FieldError::EmptyPayload);
    assert!(err.to_string().contains("nullable flag"));

    // A flag with an incomplete name-length prefix is truncated.
    let err = I64Field::deserialize_bytes(&[1, 0xFF, 0xFE]).unwrap_err();
    assert!(matches!(err, FieldError::Truncated { .. }));

    // A flag + name_len=2 followed by invalid UTF-8 name bytes.
    let bytes = [1, 2, 0, 0, 0, 0xFF, 0xFE];
    let err = I64Field::deserialize_bytes(&bytes).unwrap_err();
    assert!(matches!(err, FieldError::InvalidUtf8 { valid_up_to: 0 }));
}

#[test]
fn value_semantics() {
    use std::collections::HashSet;

    assert_eq!(I64Field::new("a", true), I64Field::new("a", true));
    assert_ne!(I64Field::new("a", true), I64Field::new("a", false));
    assert_ne!(I64Field::new("a", true), I64Field::new("b", true));

    let mut set = HashSet::new();
    set.insert(I64Field::new("a", true));
    set.insert(I64Field::new("a", true));
    set.insert(I64Field::new("a", false));
    assert_eq!(set.len(), 2);
}
