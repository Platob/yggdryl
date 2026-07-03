//! Integration tests for the concrete integer fields — every signed and unsigned
//! integer — and the trait stack they exercise (base, typed, Arrow interop).

use yggdryl_field::yggdryl_dtype::{self as dtype, DataError, DataType, TypedDataType};
use yggdryl_field::{
    arrow_schema, Field, Int16Field, Int32Field, Int64Field, Int8Field, TypedField, UInt16Field,
    UInt32Field, UInt64Field, UInt8Field,
};

// Every integer field shares the same shape, so one macro drives one test module
// per type: the field pairs a name with the type, round-trips through its Arrow
// equivalent, and applies the shared metadata policy.
macro_rules! integer_field_tests {
    ($mod:ident, $ty:ident, $dtype:ident, $native:ty, $name:literal) => {
        mod $mod {
            use super::*;

            #[test]
            fn field_pairs_a_name_with_the_type() {
                let id = $ty::new("id", false);
                assert_eq!(id.name(), "id");
                assert_eq!(id.data_type().name(), $name);
                assert!(!id.is_nullable());

                let maybe = $ty::new(String::from("maybe"), true);
                assert!(maybe.is_nullable());
            }

            #[test]
            fn arrow_field_round_trips() {
                let field = $ty::new("id", true);
                let arrow = field.to_arrow();
                assert_eq!(arrow.name(), "id");
                assert_eq!(arrow.data_type(), &dtype::$dtype.to_arrow());
                assert!(arrow.is_nullable());
                assert_eq!($ty::from_arrow(&arrow).unwrap(), field);

                // A field of a different Arrow data type is refused.
                let wrong = arrow_schema::Field::new("id", arrow_schema::DataType::Utf8, true);
                assert!(matches!(
                    $ty::from_arrow(&wrong),
                    Err(DataError::IncompatibleArrowType { .. })
                ));
            }

            #[test]
            fn arrow_field_metadata_policy() {
                use std::collections::HashMap;

                // An extension type is a different logical type: refused.
                let extension = $ty::new("id", true)
                    .to_arrow()
                    .with_metadata(HashMap::from([(
                        "ARROW:extension:name".to_string(),
                        "arrow.uuid".to_string(),
                    )]));
                assert!(matches!(
                    $ty::from_arrow(&extension),
                    Err(DataError::IncompatibleArrowType { .. })
                ));

                // Other metadata is not modeled: accepted, and dropped on the way in
                // (the model carries name, data type and nullability only).
                let annotated = $ty::new("id", true)
                    .to_arrow()
                    .with_metadata(HashMap::from([(
                        "PARQUET:field_id".to_string(),
                        "7".to_string(),
                    )]));
                let field = $ty::from_arrow(&annotated).unwrap();
                assert_eq!(field, $ty::new("id", true));
                assert!(field.to_arrow().metadata().is_empty());
            }

            #[test]
            fn generic_bounds_compose() {
                fn field_type_name<DT: TypedDataType<$native>, F: TypedField<DT, $native>>(
                    field: &F,
                ) -> String {
                    field.data_type().name().to_string()
                }
                assert_eq!(field_type_name(&$ty::new("x", false)), $name);
            }

            #[test]
            fn is_send_sync() {
                fn assert_send_sync<T: Send + Sync>() {}
                assert_send_sync::<$ty>();
            }
        }
    };
}

integer_field_tests!(int8, Int8Field, Int8Type, i8, "int8");
integer_field_tests!(int16, Int16Field, Int16Type, i16, "int16");
integer_field_tests!(int32, Int32Field, Int32Type, i32, "int32");
integer_field_tests!(int64, Int64Field, Int64Type, i64, "int64");
integer_field_tests!(uint8, UInt8Field, UInt8Type, u8, "uint8");
integer_field_tests!(uint16, UInt16Field, UInt16Type, u16, "uint16");
integer_field_tests!(uint32, UInt32Field, UInt32Type, u32, "uint32");
integer_field_tests!(uint64, UInt64Field, UInt64Type, u64, "uint64");

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
