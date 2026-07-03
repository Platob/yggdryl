//! Integration tests for the `optional` field — a field of the logical
//! value-or-null type over union storage.

use yggdryl_field::yggdryl_dtype::{self as dtype, DataError, Int64, RawDataType};
use yggdryl_field::{arrow_schema, Field, Optional, RawField};

#[test]
fn optional_field_carries_both_layers() {
    let score = Optional::<Int64>::new("score", true);
    assert_eq!(score.name(), "score");
    assert_eq!(score.data_type(), &dtype::Optional::new(Int64));
    assert!(score.is_nullable());

    // Raw round trip through Arrow.
    let arrow = score.to_arrow();
    assert_eq!(arrow.name(), "score");
    assert!(matches!(
        arrow.data_type(),
        arrow_schema::DataType::Union(..)
    ));
    assert_eq!(Optional::from_arrow(&arrow).unwrap(), score);

    // The typed layer: a generic bound over Field<i64> accepts it.
    fn type_name<F: Field<i64>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&score), "optional");

    // A field of a non-optional shape is refused.
    let wrong = arrow_schema::Field::new("score", arrow_schema::DataType::Int64, true);
    assert!(matches!(
        Optional::<Int64>::from_arrow(&wrong),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn optional_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Optional<Int64>>();
}
