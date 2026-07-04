//! Integration tests for [`Field::cast_dtype`] — re-typing a field, keeping its name
//! and nullability.

use yggdryl_field::yggdryl_dtype::{arrow_schema, Int64Type, UInt8Type};
use yggdryl_field::{Field, Int64Field, TypedOptionalField};

#[test]
fn cast_keeps_name_and_nullability_swaps_the_type() {
    let id = Int64Field::new("id", false);
    let cast = id.cast_dtype(&UInt8Type);
    assert_eq!(cast.name(), "id");
    assert!(!cast.is_nullable());
    assert_eq!(cast.data_type(), &arrow_schema::DataType::UInt8);

    // A nullable field stays nullable through the cast.
    let score = TypedOptionalField::<Int64Type>::new("score", true);
    let cast = score.cast_dtype(&UInt8Type);
    assert_eq!(cast.name(), "score");
    assert!(cast.is_nullable());
    assert_eq!(cast.data_type(), &arrow_schema::DataType::UInt8);
}
