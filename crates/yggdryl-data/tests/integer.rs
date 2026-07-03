//! Integration tests for the concrete integer types — every signed and unsigned
//! integer — and the trait stack they exercise (raw, typed, category, Arrow interop).

use yggdryl_data::{
    arrow_array, arrow_schema, DataError, DataType, DataTypeId, Field, Int16, Int16Field,
    Int16Scalar, Int32, Int32Field, Int32Scalar, Int64, Int64Field, Int64Scalar, Int8, Int8Field,
    Int8Scalar, Primitive, RawDataType, RawField, RawScalar, Scalar, UInt16, UInt16Field,
    UInt16Scalar, UInt32, UInt32Field, UInt32Scalar, UInt64, UInt64Field, UInt64Scalar, UInt8,
    UInt8Field, UInt8Scalar,
};

// Every integer type shares the same shape, so one macro drives one test module per
// type: the data type describes itself, its codec round-trips little-endian, its field
// pairs a name with the type, and its scalar holds a value or null — all cross-checked
// against the type's `DataTypeId` and round-tripped through its Arrow equivalents.
macro_rules! integer_tests {
    ($mod:ident, $ty:ident, $field:ident, $scalar:ident, $native:ty, $as_native:ident, $array:ident, $name:literal, $format:literal, $width:literal) => {
        mod $mod {
            use super::*;

            #[test]
            fn describes_itself() {
                assert_eq!($ty.name(), $name);
                assert_eq!($ty.arrow_format(), $format);
                assert_eq!($ty.byte_width(), Some($width));
                assert_eq!($ty.bit_width(), Some($width * 8));
                assert_eq!($ty::ID, DataTypeId::$ty);
            }

            #[test]
            fn matches_its_type_id() {
                // The concrete type and its id agree on name and format.
                assert_eq!($ty::ID.name(), $ty.name());
                assert_eq!($ty::ID.arrow_format(), Some($ty.arrow_format().as_str()));
                assert!($ty::ID.is_primitive());
            }

            #[test]
            fn codec_round_trips() {
                for value in [<$native>::MIN, 0, 1, 42, <$native>::MAX] {
                    let bytes = $ty.native_to_bytes(&value);
                    assert_eq!(bytes.len(), $width);
                    assert_eq!($ty.native_from_bytes(&bytes).unwrap(), value);
                }
                // Little-endian layout: the low byte comes first.
                assert_eq!($ty.native_to_bytes(&1)[0], 1);
            }

            #[test]
            fn decode_rejects_the_wrong_length() {
                let error = $ty.native_from_bytes(&[0; $width + 1]).unwrap_err();
                assert!(matches!(
                    error,
                    DataError::InvalidByteLength {
                        expected: $width,
                        got,
                    } if got == $width + 1
                ));
            }

            #[test]
            fn arrow_data_type_round_trips() {
                // `$ty` doubles as the arrow-schema variant name.
                let arrow = $ty.to_arrow();
                assert_eq!(arrow, arrow_schema::DataType::$ty);
                assert_eq!($ty::from_arrow(&arrow).unwrap(), $ty);

                let error = $ty::from_arrow(&arrow_schema::DataType::Utf8).unwrap_err();
                assert!(matches!(error, DataError::IncompatibleArrowType { .. }));
            }

            #[test]
            fn field_pairs_a_name_with_the_type() {
                let id = $field::new("id", false);
                assert_eq!(id.name(), "id");
                assert_eq!(id.data_type().name(), $name);
                assert!(!id.is_nullable());

                let maybe = $field::new(String::from("maybe"), true);
                assert!(maybe.is_nullable());
            }

            #[test]
            fn arrow_field_round_trips() {
                let field = $field::new("id", true);
                let arrow = field.to_arrow();
                assert_eq!(arrow.name(), "id");
                assert_eq!(arrow.data_type(), &arrow_schema::DataType::$ty);
                assert!(arrow.is_nullable());
                assert_eq!($field::from_arrow(&arrow).unwrap(), field);

                // A field of a different Arrow data type is refused.
                let wrong =
                    arrow_schema::Field::new("id", arrow_schema::DataType::Utf8, true);
                assert!(matches!(
                    $field::from_arrow(&wrong),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
            }

            #[test]
            fn arrow_field_metadata_policy() {
                use std::collections::HashMap;

                // An extension type is a different logical type: refused.
                let extension = $field::new("id", true).to_arrow().with_metadata(
                    HashMap::from([(
                        "ARROW:extension:name".to_string(),
                        "arrow.uuid".to_string(),
                    )]),
                );
                assert!(matches!(
                    $field::from_arrow(&extension),
                    Err(DataError::IncompatibleArrowType { .. })
                ));

                // Other metadata is not modeled: accepted, and dropped on the way in
                // (the model carries name, data type and nullability only).
                let annotated = $field::new("id", true).to_arrow().with_metadata(
                    HashMap::from([("PARQUET:field_id".to_string(), "7".to_string())]),
                );
                let field = $field::from_arrow(&annotated).unwrap();
                assert_eq!(field, $field::new("id", true));
                assert!(field.to_arrow().metadata().is_empty());
            }

            #[test]
            fn scalar_holds_a_value_or_null() {
                let answer = $scalar::new(42);
                assert!(!answer.is_null());
                assert_eq!(answer.value(), Some(&42));
                assert_eq!(answer.data_type().name(), $name);

                let missing = $scalar::null();
                assert!(missing.is_null());
                assert_eq!(missing.value(), None);
                assert_eq!($scalar::default(), missing); // default is null
            }

            #[test]
            fn accessors_convert_exactly() {
                let answer = $scalar::new(42);
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
                    answer.as_str(),
                    Err(DataError::UnsupportedConversion { .. })
                ));
                assert!(matches!(
                    answer.as_bytes(),
                    Err(DataError::UnsupportedConversion { .. })
                ));

                // A null scalar holds no value: every accessor errors.
                assert!(matches!(
                    $scalar::null().$as_native(),
                    Err(DataError::NullValue)
                ));
                assert!(matches!($scalar::null().as_i64(), Err(DataError::NullValue)));

                // The extremes convert exactly where `try_from` says they fit;
                // anything else is an inexact-conversion error.
                let max = $scalar::new(<$native>::MAX);
                assert_eq!(max.$as_native().unwrap(), <$native>::MAX);
                assert_eq!(max.as_i8().ok(), i8::try_from(<$native>::MAX).ok());
                assert_eq!(max.as_u64().ok(), u64::try_from(<$native>::MAX).ok());
                let min = $scalar::new(<$native>::MIN);
                assert_eq!(min.as_i64().ok(), i64::try_from(<$native>::MIN).ok());
                assert_eq!(min.as_u8().ok(), u8::try_from(<$native>::MIN).ok());
            }

            #[test]
            fn defaults_are_zero() {
                assert_eq!($ty.default_value(), 0);
                assert_eq!($ty.default_scalar(), $scalar::new(0));
            }

            #[test]
            fn scalar_builds_from_its_native_value() {
                assert_eq!($scalar::from(42), $scalar::new(42));
                assert_eq!($scalar::from(Some(42)), $scalar::new(42));
                assert_eq!($scalar::from(None::<$native>), $scalar::null());

                // `Into` flows through generic bounds too.
                fn build<S: From<$native>>(value: $native) -> S {
                    value.into()
                }
                let built: $scalar = build(7);
                assert_eq!(built.value(), Some(&7));
            }

            #[test]
            fn arrow_scalar_round_trips() {
                use arrow_array::Array;

                // A value: a one-element array with no null.
                let answer = $scalar::new(42);
                let arrow = answer.to_arrow();
                assert_eq!(arrow.len(), 1);
                assert_eq!(arrow.null_count(), 0);
                assert_eq!(arrow.data_type(), &arrow_schema::DataType::$ty);
                assert_eq!($scalar::from_arrow(arrow.as_ref()).unwrap(), answer);

                // Null: a one-element array holding a null.
                let missing = $scalar::null();
                let arrow = missing.to_arrow();
                assert_eq!((arrow.len(), arrow.null_count()), (1, 1));
                assert_eq!($scalar::from_arrow(arrow.as_ref()).unwrap(), missing);

                // More (or fewer) than one value is not a scalar.
                let two = arrow_array::$array::from_iter_values([1, 2]);
                assert!(matches!(
                    $scalar::from_arrow(&two),
                    Err(DataError::InvalidScalarLength { got: 2 })
                ));

                // A different Arrow array type is refused.
                let wrong = arrow_array::StringArray::from(vec!["x"]);
                assert!(matches!(
                    $scalar::from_arrow(&wrong),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
            }

            #[test]
            fn generic_bounds_compose() {
                fn first_byte<D: DataType<$native>>(data_type: &D, value: $native) -> u8 {
                    data_type.native_to_bytes(&value)[0]
                }
                fn is_null_scalar<S: Scalar<$native>>(scalar: &S) -> bool {
                    scalar.is_null()
                }
                fn primitive_bit_width<P: Primitive>(primitive: &P) -> usize {
                    primitive.bit_width().expect("a primitive has a fixed bit width")
                }
                fn field_type_name<F: Field<$native>>(field: &F) -> String {
                    field.data_type().name().to_string()
                }

                assert_eq!(first_byte(&$ty, 5), 5);
                assert!(is_null_scalar(&$scalar::null()));
                assert!(!is_null_scalar(&$scalar::new(1)));
                assert_eq!(primitive_bit_width(&$ty), $width * 8);
                assert_eq!(field_type_name(&$field::new("x", false)), $name);
            }

            #[test]
            fn is_send_sync_and_object_safe() {
                fn assert_send_sync<T: Send + Sync>() {}
                assert_send_sync::<$ty>();
                assert_send_sync::<$field>();
                assert_send_sync::<$scalar>();

                let types: Vec<Box<dyn RawDataType>> = vec![Box::new($ty)];
                assert_eq!(types[0].name(), $name);
                assert_eq!(types[0].arrow_format(), $format);
                // `to_arrow` stays on the vtable (only `from_arrow` is `Self: Sized`).
                assert_eq!(types[0].to_arrow(), arrow_schema::DataType::$ty);
            }
        }
    };
}

integer_tests!(int8, Int8, Int8Field, Int8Scalar, i8, as_i8, Int8Array, "int8", "c", 1);
integer_tests!(
    int16,
    Int16,
    Int16Field,
    Int16Scalar,
    i16,
    as_i16,
    Int16Array,
    "int16",
    "s",
    2
);
integer_tests!(
    int32,
    Int32,
    Int32Field,
    Int32Scalar,
    i32,
    as_i32,
    Int32Array,
    "int32",
    "i",
    4
);
integer_tests!(
    int64,
    Int64,
    Int64Field,
    Int64Scalar,
    i64,
    as_i64,
    Int64Array,
    "int64",
    "l",
    8
);
integer_tests!(
    uint8,
    UInt8,
    UInt8Field,
    UInt8Scalar,
    u8,
    as_u8,
    UInt8Array,
    "uint8",
    "C",
    1
);
integer_tests!(
    uint16,
    UInt16,
    UInt16Field,
    UInt16Scalar,
    u16,
    as_u16,
    UInt16Array,
    "uint16",
    "S",
    2
);
integer_tests!(
    uint32,
    UInt32,
    UInt32Field,
    UInt32Scalar,
    u32,
    as_u32,
    UInt32Array,
    "uint32",
    "I",
    4
);
integer_tests!(
    uint64,
    UInt64,
    UInt64Field,
    UInt64Scalar,
    u64,
    as_u64,
    UInt64Array,
    "uint64",
    "L",
    8
);

// A heterogeneous schema holds boxed data types of *different* widths together.
#[test]
fn a_heterogeneous_schema_mixes_widths() {
    let schema: Vec<Box<dyn RawDataType>> = vec![
        Box::new(Int8),
        Box::new(UInt16),
        Box::new(Int32),
        Box::new(UInt64),
    ];
    let widths: Vec<_> = schema.iter().map(|d| d.byte_width()).collect();
    assert_eq!(widths, vec![Some(1), Some(2), Some(4), Some(8)]);
}

// A heterogeneous set of fields converts straight into an Arrow schema.
#[test]
fn fields_assemble_into_an_arrow_schema() {
    let schema = arrow_schema::Schema::new(vec![
        Int64Field::new("id", false).to_arrow(),
        UInt8Field::new("flags", true).to_arrow(),
    ]);
    assert_eq!(schema.field(0).data_type(), &arrow_schema::DataType::Int64);
    assert_eq!(schema.field(1).data_type(), &arrow_schema::DataType::UInt8);
    assert!(schema.field(1).is_nullable());
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
