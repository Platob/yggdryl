//! Behavioural tests for the `yggdryl-dtype` primitive data types — Arrow interop,
//! the byte and value codecs, the core-tag mapping, and the guided error paths.

use arrow_schema::DataType as ArrowDataType;
use yggdryl_dtype::{
    BooleanType, DTypeError, DataType, F32Type, F64Type, I32Type, I64Type, I8Type, PrimitiveType,
    TypedDataType, U16Type, U64Type, U8Type,
};

#[test]
fn numeric_names_widths_and_arrow() {
    assert_eq!(I8Type::new().name(), "int8");
    assert_eq!(I8Type::new().byte_width(), Some(1));
    assert_eq!(I64Type::new().byte_width(), Some(8));
    assert_eq!(F32Type::new().byte_width(), Some(4));

    assert_eq!(I64Type::new().to_arrow(), ArrowDataType::Int64);
    assert_eq!(U16Type::new().to_arrow(), ArrowDataType::UInt16);
    assert_eq!(F64Type::new().to_arrow(), ArrowDataType::Float64);
}

#[test]
fn arrow_round_trips_and_mismatch_is_guided() {
    let dt = I32Type::new();
    assert_eq!(I32Type::from_arrow(&dt.to_arrow()).unwrap(), dt);

    let err = I32Type::from_arrow(&ArrowDataType::Utf8).unwrap_err();
    match &err {
        DTypeError::ArrowTypeMismatch { expected, got } => {
            assert_eq!(*expected, "int32");
            assert_eq!(got, "Utf8");
        }
        other => panic!("expected ArrowTypeMismatch, got {other:?}"),
    }
    assert!(err.to_string().contains("int32"));
}

#[test]
fn dtype_payload_round_trips_and_rejects_non_empty() {
    let dt = I64Type::new();
    assert!(dt.serialize_bytes().is_empty());
    assert_eq!(
        I64Type::deserialize_bytes(&dt.serialize_bytes()).unwrap(),
        dt
    );

    let err = I64Type::deserialize_bytes(&[1, 2, 3]).unwrap_err();
    assert_eq!(
        err,
        DTypeError::UnexpectedPayload {
            ty: "int64",
            len: 3
        }
    );
    assert!(err.to_string().contains("carries no parameters"));
}

#[test]
fn value_codec_round_trips_and_length_is_guided() {
    let dt = I32Type::new();
    let bytes = dt.value_to_bytes(-5);
    assert_eq!(bytes, (-5_i32).to_le_bytes());
    assert_eq!(dt.value_from_bytes(&bytes).unwrap(), -5);
    assert_eq!(dt.default_value(), 0);
    assert_eq!(dt.default_any_value().downcast_ref::<i32>(), Some(&0));

    let err = dt.value_from_bytes(&[1, 2]).unwrap_err();
    assert_eq!(
        err,
        DTypeError::InvalidValueLength {
            ty: "int32",
            len: 2,
            width: 4
        }
    );
    assert!(err.to_string().contains("4-byte"));
}

#[test]
fn float_value_codec() {
    let dt = F64Type::new();
    let bytes = dt.value_to_bytes(1.5);
    assert_eq!(bytes, 1.5_f64.to_le_bytes());
    assert_eq!(dt.value_from_bytes(&bytes).unwrap(), 1.5);
}

#[test]
fn primitive_tags_map_to_core_enum() {
    use yggdryl_converter::PrimitiveType as Tag;

    assert_eq!(I8Type::new().primitive_tag(), Some(Tag::I8));
    assert_eq!(U64Type::new().primitive_tag(), Some(Tag::U64));
    assert_eq!(F32Type::new().primitive_tag(), Some(Tag::F32));

    // Round-trip through the core tag.
    assert_eq!(I64Type::from_primitive_tag(Tag::I64), Some(I64Type::new()));
    assert_eq!(I64Type::from_primitive_tag(Tag::I32), None);
    assert_eq!(U8Type::from_primitive_tag(Tag::U8), Some(U8Type::new()));
}

#[test]
fn boolean_is_the_bit_packed_member() {
    let dt = BooleanType::new();
    assert_eq!(dt.name(), "boolean");
    assert_eq!(dt.byte_width(), None); // bit-packed
    assert_eq!(dt.primitive_tag(), None); // outside the core numeric tags
    assert_eq!(dt.to_arrow(), ArrowDataType::Boolean);
    assert_eq!(
        BooleanType::from_arrow(&ArrowDataType::Boolean).unwrap(),
        dt
    );

    assert_eq!(dt.value_to_bytes(true), vec![1]);
    assert_eq!(dt.value_to_bytes(false), vec![0]);
    assert!(dt.value_from_bytes(&[1]).unwrap());
    assert!(!dt.value_from_bytes(&[0]).unwrap());

    let err = dt.value_from_bytes(&[0, 1]).unwrap_err();
    assert_eq!(
        err,
        DTypeError::InvalidValueLength {
            ty: "boolean",
            len: 2,
            width: 1
        }
    );
}

#[test]
fn value_semantics_all_instances_equal() {
    use std::collections::HashSet;

    assert_eq!(I64Type::new(), I64Type);
    let mut set = HashSet::new();
    set.insert(I64Type::new());
    set.insert(I64Type::new());
    assert_eq!(set.len(), 1); // dtype markers dedup
}

#[test]
fn null_type() {
    use yggdryl_dtype::NullType;

    let dt = NullType::new();
    assert_eq!(dt.name(), "null");
    assert_eq!(dt.byte_width(), Some(0)); // a null value is zero bytes
    assert_eq!(dt.default_value(), ()); // the unit default
    assert!(dt.default_any_value().downcast_ref::<()>().is_some());
    assert_eq!(dt.to_arrow(), ArrowDataType::Null);
    assert_eq!(NullType::from_arrow(&dt.to_arrow()).unwrap(), dt);
    assert!(NullType::from_arrow(&ArrowDataType::Int64).is_err());

    // The unit value round-trips through zero bytes; anything else is a guided error.
    assert!(dt.value_to_bytes(()).is_empty());
    assert_eq!(dt.value_from_bytes(&[]).unwrap(), ());
    assert!(matches!(
        dt.value_from_bytes(&[0]).unwrap_err(),
        DTypeError::InvalidValueLength {
            ty: "null",
            width: 0,
            ..
        }
    ));

    // An empty parameter payload round-trips; a non-empty one is rejected.
    assert_eq!(
        NullType::deserialize_bytes(&dt.serialize_bytes()).unwrap(),
        dt
    );
    assert!(NullType::deserialize_bytes(&[1]).is_err());
}
