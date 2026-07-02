//! Integration tests for the concrete integer types — every signed and unsigned
//! integer — and the trait stack they exercise (raw, typed, category).

use yggdryl_data::{
    DataError, DataType, DataTypeId, Field, Int16, Int16Field, Int16Scalar, Int32, Int32Field,
    Int32Scalar, Int64, Int64Field, Int64Scalar, Int8, Int8Field, Int8Scalar, Primitive,
    RawDataType, RawField, RawScalar, Scalar, UInt16, UInt16Field, UInt16Scalar, UInt32,
    UInt32Field, UInt32Scalar, UInt64, UInt64Field, UInt64Scalar, UInt8, UInt8Field, UInt8Scalar,
};

// Every integer type shares the same shape, so one macro drives one test module per
// type: the data type describes itself, its codec round-trips little-endian, its field
// pairs a name with the type, and its scalar holds a value or null — all cross-checked
// against the type's `DataTypeId`.
macro_rules! integer_tests {
    ($mod:ident, $ty:ident, $field:ident, $scalar:ident, $native:ty, $id:ident, $name:literal, $format:literal, $width:literal) => {
        mod $mod {
            use super::*;

            #[test]
            fn describes_itself() {
                assert_eq!($ty.name(), $name);
                assert_eq!($ty.arrow_format(), $format);
                assert_eq!($ty.byte_width(), Some($width));
                assert_eq!($ty.bit_width(), Some($width * 8));
                assert_eq!($ty::ID, DataTypeId::$id);
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
            fn field_pairs_a_name_with_the_type() {
                let id = $field::new("id", false);
                assert_eq!(id.name(), "id");
                assert_eq!(id.data_type().name(), $name);
                assert!(!id.is_nullable());

                let maybe = $field::new(String::from("maybe"), true);
                assert!(maybe.is_nullable());
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
            }
        }
    };
}

integer_tests!(int8, Int8, Int8Field, Int8Scalar, i8, Int8, "int8", "c", 1);
integer_tests!(
    int16,
    Int16,
    Int16Field,
    Int16Scalar,
    i16,
    Int16,
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
    Int32,
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
    Int64,
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
    UInt8,
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
    UInt16,
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
    UInt32,
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
    UInt64,
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
