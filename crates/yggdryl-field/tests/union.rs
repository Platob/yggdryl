//! Integration tests for the `union` field.

use yggdryl_field::yggdryl_dtype::{self as dtype, DataError, Int64Type};
use yggdryl_field::{arrow_schema, Field, UnionField};

#[test]
fn union_field_round_trips_and_applies_the_metadata_policy() {
    let field = UnionField::new("value", dtype::UnionType::optional(&Int64Type), true);
    let arrow = field.to_arrow();
    assert_eq!(arrow.name(), "value");
    assert!(arrow.is_nullable());
    assert_eq!(UnionField::from_arrow(&arrow).unwrap(), field);

    // An extension-typed field is a different logical type.
    let extension = field
        .to_arrow()
        .with_metadata(std::collections::HashMap::from([(
            "ARROW:extension:name".to_string(),
            "arrow.opaque".to_string(),
        )]));
    assert!(matches!(
        UnionField::from_arrow(&extension),
        Err(DataError::IncompatibleArrowType { .. })
    ));

    // A field of a non-union type is refused.
    let wrong = arrow_schema::Field::new("value", arrow_schema::DataType::Int64, true);
    assert!(matches!(
        UnionField::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn union_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<UnionField>();
}
