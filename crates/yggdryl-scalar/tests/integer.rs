//! Integration tests for the concrete integer scalars — every signed and unsigned
//! integer — and the trait stack they exercise (base, typed, Arrow interop).

use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataError, DataType};
use yggdryl_scalar::{
    arrow_array, arrow_schema, Int16Scalar, Int32Scalar, Int64Scalar, Int8Scalar, Scalar,
    TypedScalar, UInt16Scalar, UInt32Scalar, UInt64Scalar, UInt8Scalar,
};

// Every integer scalar shares the same shape, so one macro drives one test module
// per type: the scalar holds a value or null, converts exactly through the `as_*`
// contract, and round-trips through its Arrow equivalent.
macro_rules! integer_scalar_tests {
    ($mod:ident, $ty:ident, $dtype:ident, $native:ty, $as_native:ident, $array:ident, $name:literal) => {
        mod $mod {
            use super::*;

            #[test]
            fn scalar_holds_a_value_or_null() {
                let answer = $ty::new(42);
                assert!(!answer.is_null());
                assert_eq!(answer.value(), Some(&42));
                assert_eq!(answer.data_type().name(), $name);

                let missing = $ty::null();
                assert!(missing.is_null());
                assert_eq!(missing.value(), None);
                assert_eq!($ty::default(), missing); // default is null
            }

            #[test]
            fn accessors_convert_exactly() {
                let answer = $ty::new(42);
                // The scalar's own width answers directly.
                assert_eq!(answer.$as_native().unwrap(), 42);
                // A small value converts to every numeric target.
                assert_eq!(answer.as_i8().unwrap(), 42i8);
                assert_eq!(answer.as_i16().unwrap(), 42i16);
                assert_eq!(answer.as_i32().unwrap(), 42i32);
                assert_eq!(answer.as_i64().unwrap(), 42i64);
                assert_eq!(answer.as_u8().unwrap(), 42u8);
                assert_eq!(answer.as_u16().unwrap(), 42u16);
                assert_eq!(answer.as_u32().unwrap(), 42u32);
                assert_eq!(answer.as_u64().unwrap(), 42u64);
                assert_eq!(answer.as_f32().unwrap(), 42.0f32);
                assert_eq!(answer.as_f64().unwrap(), 42.0f64);
                // An integer is never a bool, a str or bytes (the trait defaults).
                assert!(matches!(
                    answer.as_bool(),
                    Err(DataError::UnsupportedConversion { .. })
                ));
                assert!(matches!(
                    answer.as_str(None),
                    Err(DataError::UnsupportedConversion { .. })
                ));
                assert!(matches!(
                    answer.as_bytes(),
                    Err(DataError::UnsupportedConversion { .. })
                ));

                // A null scalar holds no value: every accessor errors.
                assert!(matches!(
                    $ty::null().$as_native(),
                    Err(DataError::NullValue)
                ));
                assert!(matches!($ty::null().as_i64(), Err(DataError::NullValue)));

                // The extremes convert exactly where `try_from` says they fit;
                // anything else is an inexact-conversion error.
                let max = $ty::new(<$native>::MAX);
                assert_eq!(max.$as_native().unwrap(), <$native>::MAX);
                assert_eq!(max.as_i8().ok(), i8::try_from(<$native>::MAX).ok());
                assert_eq!(max.as_u64().ok(), u64::try_from(<$native>::MAX).ok());
                let min = $ty::new(<$native>::MIN);
                assert_eq!(min.as_i64().ok(), i64::try_from(<$native>::MIN).ok());
                assert_eq!(min.as_u8().ok(), u8::try_from(<$native>::MIN).ok());
            }

            #[test]
            fn scalar_builds_from_its_native_value() {
                assert_eq!($ty::from(42), $ty::new(42));
                assert_eq!($ty::from(Some(42)), $ty::new(42));
                assert_eq!($ty::from(None::<$native>), $ty::null());

                // `Into` flows through generic bounds too.
                fn build<S: From<$native>>(value: $native) -> S {
                    value.into()
                }
                let built: $ty = build(7);
                assert_eq!(built.value(), Some(&7));
            }

            #[test]
            fn arrow_scalar_round_trips() {
                use arrow_array::Array;

                // A value: a one-element array with no null.
                let answer = $ty::new(42);
                let arrow = answer.to_arrow();
                assert_eq!(arrow.len(), 1);
                assert_eq!(arrow.null_count(), 0);
                assert_eq!(arrow.data_type(), &dtype::$dtype.to_arrow());
                assert_eq!($ty::from_arrow(arrow.as_ref()).unwrap(), answer);

                // Null: a one-element array holding a null.
                let missing = $ty::null();
                let arrow = missing.to_arrow();
                assert_eq!((arrow.len(), arrow.null_count()), (1, 1));
                assert_eq!($ty::from_arrow(arrow.as_ref()).unwrap(), missing);

                // More (or fewer) than one value is not a scalar.
                let two = arrow_array::$array::from_iter_values([1, 2]);
                assert!(matches!(
                    $ty::from_arrow(&two),
                    Err(DataError::InvalidScalarLength { got: 2 })
                ));

                // A different Arrow array type is refused.
                let wrong = arrow_array::StringArray::from(vec!["x"]);
                assert!(matches!(
                    $ty::from_arrow(&wrong),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
            }

            #[test]
            fn generic_bounds_compose() {
                fn is_null_scalar<S: TypedScalar<dtype::$dtype, $native>>(scalar: &S) -> bool {
                    scalar.is_null()
                }
                assert!(is_null_scalar(&$ty::null()));
                assert!(!is_null_scalar(&$ty::new(1)));
            }

            #[test]
            fn is_send_sync() {
                fn assert_send_sync<T: Send + Sync>() {}
                assert_send_sync::<$ty>();
            }
        }
    };
}

integer_scalar_tests!(int8, Int8Scalar, Int8Type, i8, as_i8, Int8Array, "int8");
integer_scalar_tests!(
    int16,
    Int16Scalar,
    Int16Type,
    i16,
    as_i16,
    Int16Array,
    "int16"
);
integer_scalar_tests!(
    int32,
    Int32Scalar,
    Int32Type,
    i32,
    as_i32,
    Int32Array,
    "int32"
);
integer_scalar_tests!(
    int64,
    Int64Scalar,
    Int64Type,
    i64,
    as_i64,
    Int64Array,
    "int64"
);
integer_scalar_tests!(
    uint8,
    UInt8Scalar,
    UInt8Type,
    u8,
    as_u8,
    UInt8Array,
    "uint8"
);
integer_scalar_tests!(
    uint16,
    UInt16Scalar,
    UInt16Type,
    u16,
    as_u16,
    UInt16Array,
    "uint16"
);
integer_scalar_tests!(
    uint32,
    UInt32Scalar,
    UInt32Type,
    u32,
    as_u32,
    UInt32Array,
    "uint32"
);
integer_scalar_tests!(
    uint64,
    UInt64Scalar,
    UInt64Type,
    u64,
    as_u64,
    UInt64Array,
    "uint64"
);

// Cross-checked against arrow-schema: the scalar's data type mirrors the variant.
#[test]
fn scalar_data_types_match_arrow() {
    assert_eq!(
        Int64Scalar::new(1).data_type().to_arrow(),
        arrow_schema::DataType::Int64
    );
    assert_eq!(
        UInt8Scalar::new(1).data_type().to_arrow(),
        arrow_schema::DataType::UInt8
    );
}

// Float access is exact-or-error: the boundary cases that a lossy `as` cast would
// silently round.
#[test]
fn float_access_is_exact_or_error() {
    // 2^53 is the last contiguous integer in f64; 2^53 + 1 rounds.
    assert_eq!(
        Int64Scalar::new(1 << 53).as_f64().unwrap(),
        9_007_199_254_740_992.0
    );
    let inexact =
        |result: Result<f64, DataError>| matches!(result, Err(DataError::InexactConversion { .. }));
    assert!(inexact(Int64Scalar::new((1 << 53) + 1).as_f64()));
    // i64::MIN is a power of two: exactly representable. MAX is not.
    assert_eq!(
        Int64Scalar::new(i64::MIN).as_f64().unwrap(),
        -9.223372036854776e18
    );
    assert!(inexact(Int64Scalar::new(i64::MAX).as_f64()));
    assert!(inexact(UInt64Scalar::new(u64::MAX).as_f64()));
    // f32's contiguous range ends at 2^24.
    assert_eq!(Int32Scalar::new(1 << 24).as_f32().unwrap(), 16_777_216.0);
    assert!(matches!(
        Int32Scalar::new((1 << 24) + 1).as_f32(),
        Err(DataError::InexactConversion { .. })
    ));
    assert!(matches!(
        Int32Scalar::new(i32::MAX).as_f32(),
        Err(DataError::InexactConversion { .. })
    ));
    // Sign changes never pass, and the error names the offending value.
    assert!(matches!(
        Int8Scalar::new(-1).as_u64(),
        Err(DataError::InexactConversion { value, target: "u64" }) if value == "-1"
    ));
    assert!(matches!(
        UInt8Scalar::new(200).as_i8(),
        Err(DataError::InexactConversion { value, target: "i8" }) if value == "200"
    ));
}
