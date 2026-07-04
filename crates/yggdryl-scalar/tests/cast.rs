//! Integration tests for the cast surface: the exact [`Scalar::cast_dtype`] and the
//! `unsafe` byte-reinterpret [`Scalar::cast_dtype_unchecked`].
//!
//! The crate has no float, utf8 or boolean *scalar* yet, so a cast to those Arrow
//! types is exercised against the small ad-hoc [`DataType`]s below (a cast targets any
//! `&dyn DataType`) and the result is read straight off the Arrow array.

use yggdryl_scalar::arrow_array::{self, Array};
use yggdryl_scalar::yggdryl_dtype::{
    arrow_schema, BinaryType, DataError, DataType, Int16Type, Int32Type, Int64Type, UInt8Type,
};
use yggdryl_scalar::{BinaryScalar, Int16Scalar, Int32Scalar, Int64Scalar, Scalar};

// Ad-hoc data types naming the Arrow targets the crate has no scalar for.
macro_rules! arrow_only_dtype {
    ($ty:ident, $name:literal, $format:literal, $width:expr, $variant:ident) => {
        #[derive(Debug)]
        struct $ty;
        impl DataType for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn arrow_format(&self) -> String {
                $format.to_string()
            }
            fn byte_width(&self) -> Option<usize> {
                $width
            }
            fn to_arrow(&self) -> arrow_schema::DataType {
                arrow_schema::DataType::$variant
            }
            fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
                match data_type {
                    arrow_schema::DataType::$variant => Ok($ty),
                    other => Err(DataError::IncompatibleArrowType {
                        expected: $name.to_string(),
                        got: other.to_string(),
                    }),
                }
            }
        }
    };
}
arrow_only_dtype!(Float32Like, "float32", "f", Some(4), Float32);
arrow_only_dtype!(Float64Like, "float64", "g", Some(8), Float64);
arrow_only_dtype!(Utf8Like, "utf8", "u", None, Utf8);

#[test]
fn exact_numeric_casts_follow_the_as_contract() {
    // int64 → int32, exact.
    let cast = Int64Scalar::new(42).cast_dtype(&Int32Type).unwrap();
    assert_eq!(
        Int32Scalar::from_arrow(cast.as_ref()).unwrap(),
        Int32Scalar::new(42)
    );

    // int64 → float64, exact (through the `as_f64` accessor).
    let as_float = Int64Scalar::new(42).cast_dtype(&Float64Like).unwrap();
    let floats = as_float
        .as_any()
        .downcast_ref::<arrow_array::Float64Array>()
        .unwrap();
    assert_eq!(floats.value(0), 42.0);

    // A value that would not fit narrows to an error, not a wrapped value.
    assert!(matches!(
        Int64Scalar::new(1 << 40).cast_dtype(&Int32Type),
        Err(DataError::InexactConversion { .. })
    ));
    assert!(matches!(
        Int16Scalar::new(-1).cast_dtype(&UInt8Type),
        Err(DataError::InexactConversion { .. })
    ));
}

#[test]
fn casting_the_same_type_is_the_identity() {
    let cast = Int64Scalar::new(7).cast_dtype(&Int64Type).unwrap();
    assert_eq!(
        Int64Scalar::from_arrow(cast.as_ref()).unwrap(),
        Int64Scalar::new(7)
    );
}

#[test]
fn null_casts_to_a_null_of_the_target() {
    let cast = Int64Scalar::null().cast_dtype(&Int16Type).unwrap();
    assert_eq!((cast.len(), cast.null_count()), (1, 1));
    assert!(cast.is_null(0));
    assert_eq!(cast.data_type(), &arrow_schema::DataType::Int16);
}

#[test]
fn binary_and_utf8_cast_is_validated() {
    // binary → utf8 validates the bytes as UTF-8 (through `as_str`).
    let text = BinaryScalar::new(b"hi".to_vec())
        .cast_dtype(&Utf8Like)
        .unwrap();
    let strings = text
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0), "hi");

    // Non-UTF-8 bytes are refused by the exact cast.
    assert!(matches!(
        BinaryScalar::new(vec![0xFF, 0xFE]).cast_dtype(&Utf8Like),
        Err(DataError::InexactConversion { .. })
    ));
}

#[test]
fn targets_the_source_cannot_reach_error() {
    // A binary scalar has no numeric conversion — the exact cast refuses it.
    assert!(matches!(
        BinaryScalar::new(vec![1, 2, 3]).cast_dtype(&Int64Type),
        Err(DataError::UnsupportedConversion { .. })
    ));
    // A nested target is outside the castable set entirely.
    let serie_type = yggdryl_scalar::yggdryl_dtype::TypedSerieType::new(Int64Type);
    assert!(matches!(
        Int64Scalar::new(1).cast_dtype(&serie_type),
        Err(DataError::UnsupportedCast { .. })
    ));
}

#[test]
fn unchecked_reinterprets_bytes_between_fixed_and_binary() {
    // int64 → binary: its eight little-endian bytes.
    let bytes = unsafe { Int64Scalar::new(1).cast_dtype_unchecked(&BinaryType) }.unwrap();
    assert_eq!(
        BinaryScalar::from_arrow(bytes.as_ref()).unwrap(),
        BinaryScalar::new(1i64.to_le_bytes().to_vec())
    );

    // binary (exactly eight bytes) → int64, reinterpreting the bytes.
    let round =
        unsafe { BinaryScalar::new(1i64.to_le_bytes().to_vec()).cast_dtype_unchecked(&Int64Type) }
            .unwrap();
    assert_eq!(
        Int64Scalar::from_arrow(round.as_ref()).unwrap(),
        Int64Scalar::new(1)
    );

    // int32 → float32 reinterprets the bit pattern (same width).
    let bits = 1.5f32.to_bits() as i32;
    let as_f32 = unsafe { Int32Scalar::new(bits).cast_dtype_unchecked(&Float32Like) }.unwrap();
    let floats = as_f32
        .as_any()
        .downcast_ref::<arrow_array::Float32Array>()
        .unwrap();
    assert_eq!(floats.value(0), 1.5);

    // A width mismatch is refused (int64's 8 bytes cannot become an int16).
    assert!(matches!(
        unsafe { Int64Scalar::new(1).cast_dtype_unchecked(&Int16Type) },
        Err(DataError::InvalidByteLength {
            expected: 2,
            got: 8
        })
    ));
}

#[test]
fn unchecked_reads_utf8_without_validation() {
    // Bytes that are not valid UTF-8 still produce a utf8 scalar (unsafe: the str may
    // be invalid). The exact cast would refuse these same bytes.
    let text =
        unsafe { BinaryScalar::new(vec![0xFF, 0xFE]).cast_dtype_unchecked(&Utf8Like) }.unwrap();
    let strings = text
        .as_any()
        .downcast_ref::<arrow_array::StringArray>()
        .unwrap();
    assert_eq!(strings.value(0).as_bytes(), &[0xFF, 0xFE]);
}

#[test]
fn unchecked_sees_through_an_optional_to_the_inner_bytes() {
    use yggdryl_scalar::TypedOptionalScalar;

    // An optional's reinterpret is its inner value's bytes.
    let optional = TypedOptionalScalar::new(Int64Scalar::new(1));
    let bytes = unsafe { optional.cast_dtype_unchecked(&BinaryType) }.unwrap();
    assert_eq!(
        BinaryScalar::from_arrow(bytes.as_ref()).unwrap(),
        BinaryScalar::new(1i64.to_le_bytes().to_vec())
    );
}
