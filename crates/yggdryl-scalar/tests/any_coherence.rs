//! Global coherence checks: **every** model value round-trips through the
//! type-erased [`AnyScalar`] / [`AnySerie`] holders, and `unwrap` recovers the
//! concrete typed value. A new type is not coherent with the Any layer until it is
//! added here — the check is the executable contract that the erasure is lossless.

use std::fmt::Debug;
use std::sync::Arc;

use yggdryl_scalar::half::f16;
use yggdryl_scalar::yggdryl_dtype::{DataError, DataType};
use yggdryl_scalar::{
    arrow_array, AnyScalar, AnySerie, BinaryScalar, Float16Scalar, Float16Serie, Float32Scalar,
    Float32Serie, Float64Scalar, Float64Serie, Int16Scalar, Int16Serie, Int32Scalar, Int32Serie,
    Int64Scalar, Int64Serie, Int8Scalar, Int8Serie, NullScalar, Scalar, StringScalar, UInt16Scalar,
    UInt16Serie, UInt32Scalar, UInt32Serie, UInt64Scalar, UInt64Serie, UInt8Scalar, UInt8Serie,
};

/// Assert a scalar survives the `AnyScalar` erasure losslessly: `is_null`,
/// `data_type`, the Arrow round trip, and `unwrap` all agree with the original.
fn scalar_coheres<S: Scalar + PartialEq + Debug + Clone>(scalar: S) {
    let name = scalar.data_type().name().to_string();
    let any = AnyScalar::from_arrow(scalar.to_arrow_scalar());

    assert_eq!(
        any.is_null(),
        scalar.is_null(),
        "is_null mismatch for {name}"
    );
    assert_eq!(
        any.data_type(),
        scalar.data_type().to_arrow(),
        "data_type mismatch for {name}"
    );
    assert_eq!(
        S::from_arrow(any.to_arrow_scalar().as_ref()).unwrap(),
        scalar,
        "Arrow round trip mismatch for {name}"
    );
    assert_eq!(
        any.unwrap::<S>().unwrap(),
        scalar,
        "unwrap mismatch for {name}"
    );
}

#[test]
fn every_scalar_type_coheres_with_any_scalar() {
    scalar_coheres(NullScalar::default());
    scalar_coheres(Int8Scalar::new(1));
    scalar_coheres(Int16Scalar::new(2));
    scalar_coheres(Int32Scalar::new(3));
    scalar_coheres(Int64Scalar::new(4));
    scalar_coheres(UInt8Scalar::new(5));
    scalar_coheres(UInt16Scalar::new(6));
    scalar_coheres(UInt32Scalar::new(7));
    scalar_coheres(UInt64Scalar::new(8));
    scalar_coheres(Float16Scalar::new(f16::from_f32(1.5)));
    scalar_coheres(Float32Scalar::new(2.5));
    scalar_coheres(Float64Scalar::new(3.5));
    scalar_coheres(BinaryScalar::new(vec![1, 2, 3]));
    scalar_coheres(StringScalar::new("héllo".to_string()));

    // The null of every family also coheres (decomposed and fallback alike).
    scalar_coheres(Int64Scalar::null());
    scalar_coheres(Float16Scalar::null());
    scalar_coheres(BinaryScalar::null());
    scalar_coheres(StringScalar::null());
    scalar_coheres(NullScalar::default());
}

/// Assert a serie column survives the `AnySerie` erasure: the Arrow column
/// round-trips, and `unwrap` recovers the concrete serie.
fn serie_coheres<S: Scalar + PartialEq + Debug>(concrete: S, column: arrow_array::ArrayRef) {
    let any = AnySerie::from_arrow(column.clone());
    assert_eq!(
        any.to_arrow().as_ref(),
        column.as_ref(),
        "column round trip"
    );
    assert_eq!(
        any.unwrap::<S>().unwrap(),
        concrete,
        "serie unwrap mismatch"
    );
}

#[test]
fn every_serie_column_coheres_with_any_serie() {
    serie_coheres(
        Int8Serie::from(vec![1i8, 2]),
        Arc::new(arrow_array::Int8Array::from(vec![1, 2])),
    );
    serie_coheres(
        Int16Serie::from(vec![1i16, 2]),
        Arc::new(arrow_array::Int16Array::from(vec![1, 2])),
    );
    serie_coheres(
        Int32Serie::from(vec![1i32, 2]),
        Arc::new(arrow_array::Int32Array::from(vec![1, 2])),
    );
    serie_coheres(
        Int64Serie::from(vec![1i64, 2, 3]),
        Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3])),
    );
    serie_coheres(
        UInt8Serie::from(vec![1u8, 2]),
        Arc::new(arrow_array::UInt8Array::from(vec![1, 2])),
    );
    serie_coheres(
        UInt16Serie::from(vec![1u16, 2]),
        Arc::new(arrow_array::UInt16Array::from(vec![1, 2])),
    );
    serie_coheres(
        UInt32Serie::from(vec![1u32, 2]),
        Arc::new(arrow_array::UInt32Array::from(vec![1, 2])),
    );
    serie_coheres(
        UInt64Serie::from(vec![1u64, 2]),
        Arc::new(arrow_array::UInt64Array::from(vec![1, 2])),
    );
    serie_coheres(
        Float16Serie::from(vec![f16::from_f32(1.5)]),
        Arc::new(arrow_array::Float16Array::from_iter_values([
            f16::from_f32(1.5),
        ])),
    );
    serie_coheres(
        Float32Serie::from(vec![1.5f32, 2.5]),
        Arc::new(arrow_array::Float32Array::from(vec![1.5, 2.5])),
    );
    serie_coheres(
        Float64Serie::from(vec![1.5f64, 2.5]),
        Arc::new(arrow_array::Float64Array::from(vec![1.5, 2.5])),
    );

    // A not-yet-decomposed element type stays in the Arrow fallback, still
    // round-tripping its column losslessly.
    let binary: arrow_array::ArrayRef = Arc::new(arrow_array::BinaryArray::from_iter_values([
        b"a".as_ref(),
        b"bc".as_ref(),
    ]));
    let any = AnySerie::from_arrow(binary.clone());
    assert!(matches!(any, AnySerie::Arrow(_)));
    assert_eq!(any.to_arrow().as_ref(), binary.as_ref());
    assert_eq!(any.arrow().unwrap().as_ref(), binary.as_ref());
}

#[test]
fn per_variant_accessors_borrow_the_decomposed_typed_value() {
    // AnyScalar: the matching accessor borrows, the others answer None.
    let any = AnyScalar::from(Int64Scalar::new(42));
    assert_eq!(any.int64(), Some(&Int64Scalar::new(42)));
    assert!(any.int32().is_none());
    assert!(any.float64().is_none());
    assert!(any.arrow().is_none()); // decomposed, not a fallback

    // AnySerie: likewise.
    let column = AnySerie::from(Float32Serie::from(vec![1.5f32, 2.5]));
    assert_eq!(column.float32().map(Float32Serie::len), Some(2));
    assert!(column.int64().is_none());
    assert!(column.arrow().is_none());
}

#[test]
fn unwrap_to_a_mismatched_type_is_an_actionable_error() {
    // The value is int64; unwrapping to float64 names the incompatibility.
    let any = AnyScalar::from(Int64Scalar::new(1));
    assert!(matches!(
        any.unwrap::<Float64Scalar>(),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    let column = AnySerie::from(Int64Serie::from(vec![1, 2]));
    assert!(column.unwrap::<Float64Serie>().is_err());
}
