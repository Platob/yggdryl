//! Integration tests for the `optional` data type — the logical value-or-null type
//! over union storage.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64, Logical, Optional, RawDataType, RawLogical,
    RawOptional, TypedOptional, UInt8, Union,
};

#[test]
fn optional_is_a_logical_type_over_union_storage() {
    let optional = Optional::new(Int64);
    assert_eq!(optional.name(), "optional");
    assert_eq!(optional.value_type(), &Int64);

    // The physical storage is the null-or-value union, and the Arrow surface
    // delegates to it.
    assert_eq!(optional.storage(), &Union::optional(&Int64));
    assert_eq!(optional.arrow_format(), "+us:0,1");
    assert_eq!(optional.byte_width(), None);
    assert_eq!(optional.bit_width(), None);
    assert_eq!(optional.to_arrow(), Union::optional(&Int64).to_arrow());
}

#[test]
fn optional_codec_is_the_value_types() {
    // The typed layer delegates the other way: the byte codec is the value type's.
    let optional = Optional::new(Int64);
    for value in [0i64, 1, -1, i64::MIN, i64::MAX] {
        assert_eq!(
            optional.native_to_bytes(&value),
            Int64.native_to_bytes(&value)
        );
        assert_eq!(
            optional
                .native_from_bytes(&Int64.native_to_bytes(&value))
                .unwrap(),
            value
        );
    }
    assert!(matches!(
        optional.native_from_bytes(&[1, 2, 3]),
        Err(DataError::InvalidByteLength {
            expected: 8,
            got: 3
        })
    ));

    // The codec width is the value type's too (the physical width is the union's).
    assert_eq!(optional.codec_byte_width(), Some(8));
    assert_eq!(optional.default_value(), 0);
}

#[test]
fn optional_arrow_round_trips() {
    let optional = Optional::new(Int64);
    assert_eq!(
        Optional::from_arrow(&optional.to_arrow()).unwrap(),
        optional
    );

    // A non-union, a union of another shape, and a mismatched value type are all
    // refused.
    assert!(matches!(
        Optional::<Int64>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    assert!(matches!(
        Optional::<Int64>::from_arrow(&Union::optional(&UInt8).to_arrow()),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn optional_is_the_generic_logical_holder() {
    // The typed pair: RawLogical gives storage access, Logical<T> pins it, and
    // TypedOptional pins the value type.
    fn raw_storage_name<S: RawDataType, L: RawLogical<S>>(logical: &L) -> String {
        logical.storage().name().to_string()
    }
    fn typed_storage_name<T, L: Logical<T>>(logical: &L) -> String {
        logical.storage().name().to_string()
    }
    fn typed_default<T, O: TypedOptional<T>>(optional: &O) -> T {
        optional.default_value()
    }
    let optional = Optional::new(Int64);
    assert_eq!(raw_storage_name(&optional), "union");
    assert_eq!(typed_storage_name(&optional), "union");
    assert_eq!(typed_default(&optional), 0);
}

#[test]
fn optional_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Optional<Int64>>();
}
