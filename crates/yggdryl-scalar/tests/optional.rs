//! Integration tests for the `optional` scalar — a value variant, or the null
//! variant, over union storage.

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError, DataType, Union, UnionType};
use yggdryl_scalar::{arrow_array, Int64Scalar, OptionalScalar, Scalar, TypedScalar, UInt8Scalar};

type OptionalInt64 = OptionalScalar<dtype::Int64Type, Int64Scalar>;

#[test]
fn optional_scalar_holds_a_value_or_the_null_variant() {
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    assert!(!answer.is_null());
    assert_eq!(answer.value(), Some(&42));
    assert_eq!(answer.scalar(), Some(&Int64Scalar::new(42)));
    assert_eq!(
        answer.data_type(),
        &dtype::OptionalType::new(dtype::Int64Type)
    );

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
    assert_eq!(answer.as_i64().unwrap(), 42);
    assert_eq!(answer.as_i8().unwrap(), 42);
    assert_eq!(answer.as_f64().unwrap(), 42.0);
    // The inner scalar's own contract shows through: an int64 has no str
    // conversion, and the null variant holds no value at all.
    assert!(matches!(
        answer.as_str(None),
        Err(DataError::UnsupportedConversion { .. })
    ));
    assert!(matches!(
        OptionalInt64::null().as_i64(),
        Err(DataError::NullValue)
    ));

    // Any inner scalar type works the same way, through the typed layer too.
    fn is_null_scalar<DT, AS, S: TypedScalar<DT, u8, AS>>(scalar: &S) -> bool
    where
        DT: DataType,
        AS: arrow_array::Array,
    {
        scalar.is_null()
    }
    let flag = OptionalScalar::new(UInt8Scalar::new(7));
    assert_eq!(flag.as_u8().unwrap(), 7);
    assert!(!is_null_scalar(&flag));
    assert_eq!(
        flag.data_type(),
        &dtype::OptionalType::new(dtype::UInt8Type)
    );
}

#[test]
fn optional_scalar_arrow_round_trips_both_variants() {
    // Value variant: a one-element sparse union selecting the value child.
    let answer = OptionalInt64::new(Int64Scalar::new(42));
    let arrow = answer.to_arrow_scalar();
    assert_eq!(arrow.len(), 1);
    assert_eq!(
        arrow.data_type(),
        &DataType::to_arrow(&dtype::OptionalType::new(dtype::Int64Type))
    );
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), UnionType::VALUE_TYPE_ID);
    assert_eq!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap(), answer);

    // Null variant: the type id selects the null child.
    let missing = OptionalInt64::null();
    let arrow = missing.to_arrow_scalar();
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), UnionType::NULL_TYPE_ID);
    assert_eq!(OptionalInt64::from_arrow(arrow.as_ref()).unwrap(), missing);

    // A null inner scalar normalized at construction: the round trip is the exact
    // inverse — full equality, not just agreement on nullness.
    let inner_null = OptionalInt64::new(Int64Scalar::null());
    let arrow = inner_null.to_arrow_scalar();
    let union_array = arrow
        .as_any()
        .downcast_ref::<arrow_array::UnionArray>()
        .unwrap();
    assert_eq!(union_array.type_id(0), UnionType::NULL_TYPE_ID);
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
    let other = OptionalScalar::new(UInt8Scalar::new(7)).to_arrow_scalar();
    assert!(matches!(
        OptionalInt64::from_arrow(other.as_ref()),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // The right layout at the wrong length.
    let fields = UnionType::optional(&dtype::Int64Type).fields().clone();
    let two = arrow_array::UnionArray::try_new(
        fields,
        vec![UnionType::VALUE_TYPE_ID, UnionType::NULL_TYPE_ID].into(),
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
fn optional_scalar_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OptionalInt64>();
}
