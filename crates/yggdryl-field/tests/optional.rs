//! Integration tests for the `optional` field — a field of the logical
//! value-or-null type over union storage.

use yggdryl_field::yggdryl_dtype::{self as dtype, DataError, Int64Type, TypedDataType};
use yggdryl_field::{arrow_schema, Field, OptionalField, TypedField};

#[test]
fn optional_field_carries_both_layers() {
    let score = OptionalField::<Int64Type>::new("score", true);
    assert_eq!(score.name(), "score");
    assert_eq!(score.data_type(), &dtype::OptionalType::new(Int64Type));
    assert!(score.is_nullable());

    // Base round trip through Arrow.
    let arrow = score.to_arrow();
    assert_eq!(arrow.name(), "score");
    assert!(matches!(
        arrow.data_type(),
        arrow_schema::DataType::Union(..)
    ));
    assert_eq!(OptionalField::from_arrow(&arrow).unwrap(), score);

    // The typed layer: a generic bound over TypedField accepts it.
    fn type_name<DT: TypedDataType<i64>, F: TypedField<DT, i64>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&score), "optional");

    // A field of a non-optional shape is refused.
    let wrong = arrow_schema::Field::new("score", arrow_schema::DataType::Int64, true);
    assert!(matches!(
        OptionalField::<Int64Type>::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn optional_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OptionalField<Int64Type>>();
}
