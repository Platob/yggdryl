//! Integration tests for the `optional` family — the logical [`Optional`] data
//! type over union storage, its field, and the [`OptionalScalar`].

use yggdryl_data::arrow_array::Array;
use yggdryl_data::{
    arrow_array, arrow_schema, DataError, DataType, Field, Int64, Int64Scalar, Logical, Optional,
    OptionalField, OptionalScalar, RawDataType, RawField, RawScalar, Scalar, UInt8, UInt8Scalar,
    Union,
};

type OptionalInt64 = OptionalScalar<Int64, Int64Scalar>;

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
fn optional_field_carries_both_layers() {
    let score = OptionalField::<Int64>::new("score", true);
    assert_eq!(score.name(), "score");
    assert_eq!(score.data_type(), &Optional::new(Int64));
    assert!(score.is_nullable());

    // Raw round trip through Arrow.
    let arrow = score.to_arrow();
    assert_eq!(arrow.name(), "score");
    assert!(matches!(
        arrow.data_type(),
        arrow_schema::DataType::Union(..)
    ));
    assert_eq!(OptionalField::from_arrow(&arrow).unwrap(), score);

    // The typed layer: a generic bound over Field<i64> accepts it.
    fn type_name<F: Field<i64>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&score), "optional");

    // A field of a non-optional shape is refused.
    let wrong = arrow_schema::Field::new("score", arrow_schema::DataType::Int64, true);
    assert!(matches!(
        OptionalField::<Int64>::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn optional_scalar_holds_a_value_or_the_null_variant() {
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    assert!(!answer.is_null());
    assert_eq!(answer.value(), Some(&42));
    assert_eq!(answer.scalar(), Some(&Int64Scalar::new(42)));
    assert_eq!(answer.data_type(), &Optional::new(Int64));

    let missing = OptionalInt64::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert_eq!(missing.scalar(), None);
    assert_eq!(OptionalInt64::default(), missing);

    // Built from the inner scalar, or an Option of it.
    assert_eq!(OptionalInt64::from(Int64Scalar::new(42)), answer);
    assert_eq!(OptionalInt64::from(Some(Int64Scalar::new(42))), answer);
    assert_eq!(OptionalInt64::from(None::<Int64Scalar>), missing);

    // A null *inner* scalar normalizes to the null variant: null is one state, so
    // observationally identical scalars are also *equal*.
    let inner_null = OptionalInt64::new(Int64Scalar::null());
    assert!(inner_null.is_null());
    assert_eq!(inner_null.scalar(), None); // normalized away
    assert_eq!(inner_null, OptionalInt64::null());
}

#[test]
fn optional_scalar_redirects_access_to_the_inner_scalar() {
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    assert_eq!(answer.as_i64(), Some(42));
    assert_eq!(answer.as_i8(), Some(42));
    assert_eq!(answer.as_f64(), Some(42.0));
    assert_eq!(answer.as_str(), None);
    assert_eq!(OptionalInt64::null().as_i64(), None);

    // Any inner scalar type works the same way, through the typed layer too.
    fn is_null_scalar<S: Scalar<u8>>(scalar: &S) -> bool {
        scalar.is_null()
    }
    let flag = OptionalScalar::new(UInt8Scalar::new(7));
    assert_eq!(flag.as_u8(), Some(7));
    assert!(!is_null_scalar(&flag));
    assert_eq!(flag.data_type(), &Optional::new(UInt8));
}

#[test]
fn optional_scalar_arrow_round_trips_both_variants() {
    // Value variant: a one-element sparse union selecting the value child.
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    let arrow = answer.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(arrow.data_type(), &Optional::new(Int64).to_arrow());
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), Union::VALUE_TYPE_ID);
    assert_eq!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap(), answer);

    // Null variant: the type id selects the null child.
    let missing = OptionalInt64::null();
    let arrow = missing.to_arrow();
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), Union::NULL_TYPE_ID);
    assert_eq!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap(), missing);

    // A null inner scalar normalized at construction: the round trip is the exact
    // inverse — full equality, not just agreement on nullness.
    let inner_null = OptionalInt64::new(Int64Scalar::null());
    let arrow = inner_null.to_arrow();
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), Union::NULL_TYPE_ID);
    assert_eq!(
        OptionalInt64::from_arrow(arrow.as_ref()).unwrap(),
        inner_null
    );
}

#[test]
fn optional_scalar_from_arrow_rejects_other_shapes() {
    // A non-union array of the right length.
    let plain = arrow_array::Int64Array::from_iter_values([42]);
    assert!(matches!(
        OptionalInt64::from_arrow(&plain),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // The right union layout but for a different value type.
    let other = OptionalScalar::new(UInt8Scalar::new(7)).to_arrow();
    assert!(matches!(
        OptionalInt64::from_arrow(other.as_ref()),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // The right layout at the wrong length.
    let fields = Union::optional(&Int64).fields().clone();
    let two = arrow_array::UnionArray::try_new(
        fields,
        vec![Union::VALUE_TYPE_ID, Union::NULL_TYPE_ID].into(),
        None,
        vec![
            std::sync::Arc::new(arrow_array::NullArray::new(2)),
            std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([1, 2])),
        ],
    )
    .unwrap();
    assert!(matches!(
        OptionalInt64::from_arrow(&two),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
}

#[test]
fn optional_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Optional<Int64>>();
    assert_send_sync::<OptionalField<Int64>>();
    assert_send_sync::<OptionalInt64>();
}
