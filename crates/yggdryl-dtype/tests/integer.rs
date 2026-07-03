//! Integration tests for the concrete integer data types — every signed and
//! unsigned integer — and the trait stack they exercise (raw, typed, category,
//! Arrow interop).

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, DataTypeId, Int16, Int32, Int64, Int8, Primitive,
    RawDataType, UInt16, UInt32, UInt64, UInt8,
};

// Every integer type shares the same shape, so one macro drives one test module per
// type: the data type describes itself, its codec round-trips little-endian, and it
// is cross-checked against its `DataTypeId` and round-tripped through its Arrow
// equivalent.
macro_rules! integer_tests {
    ($mod:ident, $ty:ident, $native:ty, $name:literal, $format:literal, $width:literal) => {
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
            fn defaults_are_zero() {
                assert_eq!($ty.default_value(), 0);
            }

            #[test]
            fn generic_bounds_compose() {
                fn first_byte<D: DataType<$native>>(data_type: &D, value: $native) -> u8 {
                    data_type.native_to_bytes(&value)[0]
                }
                fn primitive_bit_width<P: Primitive>(primitive: &P) -> usize {
                    primitive.bit_width().expect("a primitive has a fixed bit width")
                }

                assert_eq!(first_byte(&$ty, 5), 5);
                assert_eq!(primitive_bit_width(&$ty), $width * 8);
            }

            #[test]
            fn is_send_sync_and_object_safe() {
                fn assert_send_sync<T: Send + Sync>() {}
                assert_send_sync::<$ty>();

                let types: Vec<Box<dyn RawDataType>> = vec![Box::new($ty)];
                assert_eq!(types[0].name(), $name);
                assert_eq!(types[0].arrow_format(), $format);
                // `to_arrow` stays on the vtable (only `from_arrow` is `Self: Sized`).
                assert_eq!(types[0].to_arrow(), arrow_schema::DataType::$ty);
            }
        }
    };
}

integer_tests!(int8, Int8, i8, "int8", "c", 1);
integer_tests!(int16, Int16, i16, "int16", "s", 2);
integer_tests!(int32, Int32, i32, "int32", "i", 4);
integer_tests!(int64, Int64, i64, "int64", "l", 8);
integer_tests!(uint8, UInt8, u8, "uint8", "C", 1);
integer_tests!(uint16, UInt16, u16, "uint16", "S", 2);
integer_tests!(uint32, UInt32, u32, "uint32", "I", 4);
integer_tests!(uint64, UInt64, u64, "uint64", "L", 8);

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
