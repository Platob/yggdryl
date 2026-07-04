//! Integration tests for the `optional` field — the dynamic [`OptionalField`] and
//! the statically-typed [`TypedOptionalField`], over union storage.

use yggdryl_field::yggdryl_dtype::{self as dtype, DataError, Int64Type, TypedDataType};
use yggdryl_field::{arrow_schema, Field, OptionalField, TypedField, TypedOptionalField};

#[test]
fn typed_optional_field_carries_both_layers() {
    let score = TypedOptionalField::<Int64Type>::new("score", true);
    assert_eq!(score.name(), "score");
    assert_eq!(score.data_type(), &dtype::TypedOptionalType::new(Int64Type));
    assert!(score.is_nullable());

    // Base round trip through Arrow.
    let arrow = score.to_arrow();
    assert_eq!(arrow.name(), "score");
    assert!(matches!(
        arrow.data_type(),
        arrow_schema::DataType::Union(..)
    ));
    assert_eq!(TypedOptionalField::from_arrow(&arrow).unwrap(), score);

    // The typed layer: a generic bound over TypedField accepts it.
    fn type_name<DT: TypedDataType<i64>, F: TypedField<DT, i64>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&score), "optional");

    // A field of a non-optional shape is refused.
    let wrong = arrow_schema::Field::new("score", arrow_schema::DataType::Int64, true);
    assert!(matches!(
        TypedOptionalField::<Int64Type>::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn dynamic_optional_field_wraps_the_dynamic_type() {
    // The dynamic field carries the dynamic OptionalType and erases from the typed.
    let score = OptionalField::new("score", dtype::OptionalType::new(&Int64Type), true);
    assert_eq!(score.name(), "score");
    assert_eq!(score.data_type(), &dtype::OptionalType::new(&Int64Type));
    assert_eq!(OptionalField::from_arrow(&score.to_arrow()).unwrap(), score);
}

#[test]
fn optional_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OptionalField>();
    assert_send_sync::<TypedOptionalField<Int64Type>>();
}
