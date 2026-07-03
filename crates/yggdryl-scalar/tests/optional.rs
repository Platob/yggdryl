//! Integration tests for the `optional` scalar — a value variant, or the null
//! variant, over union storage.

use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError, RawUnion, Union};
use yggdryl_scalar::{arrow_array, Int64, Optional, RawScalar, Scalar, UInt8};

type OptionalInt64 = Optional<dtype::Int64, Int64>;

#[test]
fn optional_scalar_holds_a_value_or_the_null_variant() {
    let answer = OptionalInt64::new(Int64::new(42));
    assert!(!answer.is_null());
    assert_eq!(answer.value(), Some(&42));
    assert_eq!(answer.scalar(), Some(&Int64::new(42)));
    assert_eq!(answer.data_type(), &dtype::Optional::new(dtype::Int64));

    let missing = OptionalInt64::null();
    assert!(missing.is_null());
    assert_eq!(missing.value(), None);
    assert_eq!(missing.scalar(), None);
    assert_eq!(OptionalInt64::default(), missing);

    // Built from the inner scalar, or an Option of it.
    assert_eq!(OptionalInt64::from(Int64::new(42)), answer);
    assert_eq!(OptionalInt64::from(Some(Int64::new(42))), answer);
    assert_eq!(OptionalInt64::from(None::<Int64>), missing);

    // A null *inner* scalar normalizes to the null variant: null is one state, so
    // observationally identical scalars are also *equal*.
    let inner_null = OptionalInt64::new(Int64::null());
    assert!(inner_null.is_null());
    assert_eq!(inner_null.scalar(), None); // normalized away
    assert_eq!(inner_null, OptionalInt64::null());
}

#[test]
fn optional_scalar_redirects_access_to_the_inner_scalar() {
    let answer = OptionalInt64::new(Int64::new(42));
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
    fn is_null_scalar<S: Scalar<u8>>(scalar: &S) -> bool {
        scalar.is_null()
    }
    let flag = Optional::new(UInt8::new(7));
    assert_eq!(flag.as_u8().unwrap(), 7);
    assert!(!is_null_scalar(&flag));
    assert_eq!(flag.data_type(), &dtype::Optional::new(dtype::UInt8));
}

#[test]
fn optional_scalar_arrow_round_trips_both_variants() {
    // Value variant: a one-element sparse union selecting the value child.
    let answer = OptionalInt64::new(Int64::new(42));
    let arrow = answer.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(
        arrow.data_type(),
        &yggdryl_scalar::yggdryl_dtype::RawDataType::to_arrow(&dtype::Optional::new(dtype::Int64))
    );
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
    let inner_null = OptionalInt64::new(Int64::null());
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
    let other = Optional::new(UInt8::new(7)).to_arrow();
    assert!(matches!(
        OptionalInt64::from_arrow(other.as_ref()),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // The right layout at the wrong length.
    let fields = Union::optional(&dtype::Int64).fields().clone();
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
fn optional_scalar_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OptionalInt64>();
}
