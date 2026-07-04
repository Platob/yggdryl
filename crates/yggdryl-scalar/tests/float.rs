//! Integration tests for the concrete float scalars and series — single and double
//! precision — covering the float-specific numeric contract (exact-or-error integer
//! and narrowing conversions, `NaN`), the bulk-IO serie bridge, and their
//! type-erased [`AnySerie`] / [`AnyScalar`] decomposition.

use yggdryl_scalar::yggdryl_core::{ByteBuffer, RawIOBase, Whence};
use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError, DataType};
use yggdryl_scalar::{
    arrow_array, AnyScalar, AnySerie, Float32Scalar, Float32Serie, Float64Scalar, Float64Serie,
    Int64Scalar, Scalar, TypedScalar,
};

// The two float scalars share the same shape, so one macro drives a module per type:
// the value/null states, the exact-or-error accessors, and the Arrow round trip.
macro_rules! float_scalar_tests {
    ($mod:ident, $ty:ident, $dtype:ident, $native:ty, $as_native:ident, $array:ident, $name:literal) => {
        mod $mod {
            use super::*;

            #[test]
            fn holds_a_value_or_null() {
                let weight = $ty::new(1.5);
                assert!(!weight.is_null());
                assert_eq!(weight.value(), Some(&1.5));
                assert_eq!(weight.data_type().name(), $name);

                let missing = $ty::null();
                assert!(missing.is_null());
                assert_eq!(missing.value(), None);
                assert_eq!($ty::default(), missing); // default is null

                // Floats compare with PartialEq (no Eq): equal values, NaN never equal.
                assert_eq!($ty::new(1.5), $ty::new(1.5));
                assert_ne!($ty::new(<$native>::NAN), $ty::new(<$native>::NAN));
            }

            #[test]
            fn reads_as_a_float_directly_and_widens() {
                let weight = $ty::new(1.5);
                assert_eq!(weight.$as_native().unwrap(), 1.5);
                assert_eq!(weight.as_f64().unwrap(), 1.5); // f32 widens exactly; f64 is native

                // NaN survives both float accessors (its own inequality aside).
                assert!($ty::new(<$native>::NAN).as_f32().unwrap().is_nan());
                assert!($ty::new(<$native>::NAN).as_f64().unwrap().is_nan());

                // A float is never a bool, str or bytes (the trait defaults).
                assert!(matches!(
                    weight.as_bool(),
                    Err(DataError::UnsupportedConversion { .. })
                ));
                assert!(matches!(
                    weight.as_str(None),
                    Err(DataError::UnsupportedConversion { .. })
                ));

                // A null scalar holds no value.
                assert!(matches!(
                    $ty::null().$as_native(),
                    Err(DataError::NullValue)
                ));
                assert!(matches!($ty::null().as_i64(), Err(DataError::NullValue)));
            }

            #[test]
            fn reads_as_an_integer_only_when_whole_and_in_range() {
                // A whole number converts to every integer target it fits.
                let whole = $ty::new(42.0);
                assert_eq!(whole.as_i8().unwrap(), 42i8);
                assert_eq!(whole.as_i64().unwrap(), 42i64);
                assert_eq!(whole.as_u64().unwrap(), 42u64);

                // A fractional value is inexact for every integer target.
                let fraction = $ty::new(1.5);
                assert!(matches!(
                    fraction.as_i64(),
                    Err(DataError::InexactConversion { .. })
                ));

                // A whole value out of the narrow target's range is inexact but still
                // fits a wider one.
                let big = $ty::new(1000.0);
                assert!(matches!(
                    big.as_i8(),
                    Err(DataError::InexactConversion { .. })
                ));
                assert_eq!(big.as_i64().unwrap(), 1000i64);

                // A negative value has no unsigned reading.
                assert!(matches!(
                    $ty::new(-1.0).as_u8(),
                    Err(DataError::InexactConversion { .. })
                ));

                // Non-finite values are never integers.
                assert!($ty::new(<$native>::INFINITY).as_i64().is_err());
                assert!($ty::new(<$native>::NAN).as_i64().is_err());
            }

            #[test]
            fn round_trips_through_arrow() {
                let weight = $ty::new(1.5);
                let arrow = weight.to_arrow_scalar();
                assert_eq!(arrow.len(), 1);
                assert_eq!($ty::from_arrow(arrow.as_ref()).unwrap(), weight);
                assert!($ty::null().to_arrow_scalar().is_null(0));

                // More than one value is not a scalar; a wrong array type is refused.
                let two = arrow_array::$array::from_iter_values([1.0, 2.0]);
                assert!(matches!(
                    $ty::from_arrow(&two),
                    Err(DataError::InvalidScalarLength { got: 2 })
                ));
                let wrong = arrow_array::StringArray::from(vec!["x"]);
                assert!(matches!(
                    $ty::from_arrow(&wrong),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
            }

            #[test]
            fn generic_bounds_compose_and_is_send_sync() {
                fn is_null<S: TypedScalar<dtype::$dtype, $native, arrow_array::$array>>(
                    scalar: &S,
                ) -> bool {
                    scalar.is_null()
                }
                assert!(is_null(&$ty::null()));
                assert!(!is_null(&$ty::new(1.0)));

                fn assert_send_sync<T: Send + Sync>() {}
                assert_send_sync::<$ty>();
            }
        }
    };
}

float_scalar_tests!(
    float32,
    Float32Scalar,
    Float32Type,
    f32,
    as_f32,
    Float32Array,
    "float32"
);
float_scalar_tests!(
    float64,
    Float64Scalar,
    Float64Type,
    f64,
    as_f64,
    Float64Array,
    "float64"
);

#[test]
fn float16_reads_widen_and_narrow_exact_or_error() {
    use yggdryl_scalar::half::f16;
    use yggdryl_scalar::{Float16Scalar, Float16Serie};

    let half = f16::from_f32(1.5);
    let scalar = Float16Scalar::new(half);
    assert_eq!(scalar.value(), Some(&half));
    assert_eq!(scalar.data_type().name(), "float16");
    assert_eq!(scalar.as_f16().unwrap(), half);
    // f16 ⊂ f32 ⊂ f64: widening is always exact.
    assert_eq!(scalar.as_f32().unwrap(), 1.5f32);
    assert_eq!(scalar.as_f64().unwrap(), 1.5f64);
    // Whole-number-in-range reads as an integer; a fractional one is inexact.
    assert_eq!(Float16Scalar::new(f16::from_f32(3.0)).as_i64().unwrap(), 3);
    assert!(matches!(
        scalar.as_i64(),
        Err(DataError::InexactConversion { .. })
    ));

    // Every scalar can narrow to f16 when exact: 1.5 fits, 0.1 does not.
    assert_eq!(Float64Scalar::new(1.5).as_f16().unwrap(), half);
    assert!(Float64Scalar::new(0.1).as_f16().is_err());
    assert_eq!(Int64Scalar::new(3).as_f16().unwrap(), f16::from_f32(3.0));
    assert!(Int64Scalar::new(100_000).as_f16().is_err()); // beyond f16's exact range

    // NaN passes through; null and Arrow round-trip.
    assert!(Float16Scalar::new(f16::NAN).as_f16().unwrap().is_nan());
    assert!(Float16Scalar::null().is_null());
    assert_eq!(
        Float16Scalar::from_arrow(scalar.to_arrow_scalar().as_ref()).unwrap(),
        scalar
    );

    // Serie: buffer-backed, decomposes in AnySerie, reads elements widened.
    let weights = Float16Serie::from(vec![f16::from_f32(1.5), f16::from_f32(2.5)]);
    assert_eq!(weights.len(), 2);
    assert_eq!(weights.get_at::<f32>(1).unwrap(), 2.5);
    assert_eq!(weights.get_scalar_at(0), Some(Float16Scalar::new(half)));
    let column = AnySerie::from(weights.clone());
    assert!(matches!(column, AnySerie::Float16(_)));
    assert_eq!(
        column.get_any_scalar_at(0),
        Some(AnyScalar::from(Float16Scalar::new(half)))
    );
    assert_eq!(
        Float16Serie::from_arrow(weights.to_arrow_scalar().as_ref()).unwrap(),
        weights
    );

    // cast_dtype to/from float16 is exact-or-error.
    let cast = Int64Scalar::new(3).cast_dtype(&dtype::Float16Type).unwrap();
    assert_eq!(
        Float16Scalar::from_arrow(cast.as_ref()).unwrap(),
        Float16Scalar::new(f16::from_f32(3.0))
    );
    assert!(Float64Scalar::new(0.1)
        .cast_dtype(&dtype::Float16Type)
        .is_err());
}

#[test]
fn f64_narrows_to_f32_only_when_exact() {
    // 1.5 has an exact f32; 0.1 does not.
    assert_eq!(Float64Scalar::new(1.5).as_f32().unwrap(), 1.5f32);
    assert!(matches!(
        Float64Scalar::new(0.1).as_f32(),
        Err(DataError::InexactConversion { .. })
    ));
    // The scalar's own width always answers.
    assert_eq!(Float64Scalar::new(0.1).as_f64().unwrap(), 0.1);
    assert_eq!(Float32Scalar::new(0.5).as_f32().unwrap(), 0.5);
}

#[test]
fn serie_borrows_buffers_and_reads_null_aware() {
    let weights = Float64Serie::from(vec![1.5, 2.5, 3.5]);
    assert_eq!(weights.len(), 3);
    assert_eq!(weights.values(), Some(&[1.5, 2.5, 3.5][..]));
    assert_eq!(weights.get_at::<f64>(1).unwrap(), 2.5);
    assert_eq!(weights.get_scalar_at(2), Some(Float64Scalar::new(3.5)));
    assert_eq!(weights.get_scalar_at(3), None); // out of bounds

    let sparse = Float32Serie::from(vec![Some(1.5f32), None]);
    assert!(sparse.get_at::<f32>(1).is_err());
    assert_eq!(sparse.get_scalar_at(1), Some(Float32Scalar::null()));

    // Empty and null are distinct states.
    assert!(Float64Serie::from(Vec::<f64>::new()).is_empty());
    assert!(Float64Serie::null().is_null());
}

#[test]
fn serie_bridges_positioned_io_in_one_bulk_transfer() {
    let weights = Float32Serie::from(vec![1.5f32, 2.5, 3.5]);

    // pwrite_io lays the elements out little-endian in one bulk write...
    let mut buffer = ByteBuffer::new();
    weights.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
    assert_eq!(buffer.byte_size(), 3 * 4);

    // ...and from_io reads them back: the exact inverse.
    assert_eq!(Float32Serie::from_io(&buffer).unwrap(), weights);

    // A byte size that is not a whole number of elements is rejected.
    buffer.resize_bytes(4 * 3 - 1).unwrap();
    assert!(matches!(
        Float32Serie::from_io(&buffer),
        Err(DataError::InvalidByteLength { .. })
    ));

    // A null serie has no bytes to write.
    let mut sink = ByteBuffer::new();
    assert!(matches!(
        Float32Serie::null().pwrite_io(&mut sink, 0, Whence::Start),
        Err(DataError::NullValue)
    ));
}

#[test]
fn any_serie_decomposes_floats() {
    // A float Arrow array decomposes into the concrete buffer-backed serie...
    let arrow: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::Float64Array::from(vec![1.5, 2.5]));
    let column = AnySerie::from_arrow(arrow.clone());
    assert!(matches!(column, AnySerie::Float64(_)));
    assert_eq!(column.len(), 2);
    // ...reconstitutes to the same array, and equals its concrete twin.
    assert_eq!(column.to_arrow().as_ref(), arrow.as_ref());
    assert_eq!(AnySerie::from(Float64Serie::from(vec![1.5, 2.5])), column);

    // A single element reads out as a decomposed AnyScalar.
    assert_eq!(
        column.get_any_scalar_at(0),
        Some(AnyScalar::from(Float64Scalar::new(1.5)))
    );
    assert!(matches!(
        AnyScalar::from_arrow(Float32Scalar::new(1.5).to_arrow_scalar()),
        AnyScalar::Float32(_)
    ));
}

#[test]
fn casts_between_floats_and_integers_are_exact_or_error() {
    // int64 → float64 (exact), and back when whole.
    let cast = yggdryl_scalar::Int64Scalar::new(3)
        .cast_dtype(&dtype::Float64Type)
        .unwrap();
    assert_eq!(
        Float64Scalar::from_arrow(cast.as_ref()).unwrap(),
        Float64Scalar::new(3.0)
    );
    let back = Float64Scalar::new(3.0)
        .cast_dtype(&dtype::Int64Type)
        .unwrap();
    assert_eq!(
        yggdryl_scalar::Int64Scalar::from_arrow(back.as_ref()).unwrap(),
        yggdryl_scalar::Int64Scalar::new(3)
    );

    // A fractional float cannot cast to an integer.
    assert!(Float64Scalar::new(1.5)
        .cast_dtype(&dtype::Int64Type)
        .is_err());
    // float64 → float32, exact-or-error.
    assert!(Float64Scalar::new(1.5)
        .cast_dtype(&dtype::Float32Type)
        .is_ok());
    assert!(Float64Scalar::new(0.1)
        .cast_dtype(&dtype::Float32Type)
        .is_err());

    // The unchecked reinterpret bridges the raw bytes to binary and back.
    let bytes =
        unsafe { Float64Scalar::new(1.5).cast_dtype_unchecked(&dtype::BinaryType) }.unwrap();
    let recovered = yggdryl_scalar::BinaryScalar::from_arrow(bytes.as_ref()).unwrap();
    assert_eq!(
        recovered.value_le_bytes().unwrap(),
        1.5f64.to_le_bytes().to_vec()
    );
}
