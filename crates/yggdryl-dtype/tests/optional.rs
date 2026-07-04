//! Integration tests for the `optional` data type — the dynamic [`OptionalType`]
//! and the statically-typed [`TypedOptionalType`], a logical value-or-null type over
//! union storage.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64Type, Logical, Optional, OptionalType, TypedDataType,
    TypedLogical, TypedOptional, TypedOptionalType, UInt8Type, UnionType,
};

#[test]
fn typed_optional_is_a_logical_type_over_union_storage() {
    let optional = TypedOptionalType::new(Int64Type);
    assert_eq!(optional.name(), "optional");
    assert_eq!(optional.value_type(), &Int64Type);

    // The physical storage is the null-or-value union, and the Arrow surface
    // delegates to it.
    assert_eq!(optional.storage(), &UnionType::optional(&Int64Type));
    assert_eq!(optional.arrow_format(), "+us:0,1");
    assert_eq!(optional.byte_width(), None);
    assert_eq!(optional.bit_width(), None);
    assert_eq!(
        optional.to_arrow(),
        UnionType::optional(&Int64Type).to_arrow()
    );
}

#[test]
fn dynamic_optional_is_arrow_backed_and_erases() {
    let optional = OptionalType::new(&Int64Type);
    assert_eq!(optional.name(), "optional");
    assert_eq!(optional.value_field().name(), "int64");
    assert_eq!(optional.storage(), &UnionType::optional(&Int64Type));

    // erase() and from_arrow agree; the round trip is lossless.
    assert_eq!(TypedOptionalType::new(Int64Type).erase(), optional);
    assert_eq!(
        OptionalType::from_arrow(&optional.to_arrow()).unwrap(),
        optional
    );
    assert!(matches!(
        OptionalType::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn optional_codec_is_the_value_types() {
    // The typed layer delegates the other way: the byte codec is the value type's.
    let optional = TypedOptionalType::new(Int64Type);
    for value in [0i64, 1, -1, i64::MIN, i64::MAX] {
        assert_eq!(
            optional.native_to_bytes(&value),
            Int64Type.native_to_bytes(&value)
        );
        assert_eq!(
            optional
                .native_from_bytes(&Int64Type.native_to_bytes(&value))
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
fn typed_optional_arrow_round_trips() {
    let optional = TypedOptionalType::new(Int64Type);
    assert_eq!(
        TypedOptionalType::from_arrow(&optional.to_arrow()).unwrap(),
        optional
    );

    // A non-union, and a union of a mismatched value type, are both refused by the
    // typed decoder (the dynamic one accepts any value type).
    assert!(matches!(
        TypedOptionalType::<Int64Type>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    assert!(matches!(
        TypedOptionalType::<Int64Type>::from_arrow(&UnionType::optional(&UInt8Type).to_arrow()),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn optional_is_the_generic_logical_holder() {
    // The typed pair: Logical gives storage access, TypedLogical<T> pins it, and
    // TypedOptional pins the value type.
    fn raw_storage_name<L: Logical>(logical: &L) -> String {
        logical.storage().name().to_string()
    }
    fn typed_storage_name<T, L: TypedLogical<T>>(logical: &L) -> String {
        logical.storage().name().to_string()
    }
    fn typed_default<T, O: TypedOptional<T>>(optional: &O) -> T {
        optional.default_value()
    }
    let optional = TypedOptionalType::new(Int64Type);
    assert_eq!(raw_storage_name(&optional), "union");
    assert_eq!(raw_storage_name(&OptionalType::new(&Int64Type)), "union");
    assert_eq!(typed_storage_name(&optional), "union");
    assert_eq!(typed_default(&optional), 0);
}

#[test]
fn optional_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OptionalType>();
    assert_send_sync::<TypedOptionalType<Int64Type>>();
}
