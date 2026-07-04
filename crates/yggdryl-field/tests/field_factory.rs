//! Integration tests for the [`FieldFactory`] surface: a typed data type builds its
//! field, equal to the one constructed directly.

use yggdryl_field::yggdryl_dtype::{
    BinaryType, Int64Type, TypedOptionalType, TypedSerieType, UInt8Type,
};
use yggdryl_field::{
    BinaryField, FieldFactory, Int64Field, TypedOptionalField, TypedSerieField, UInt8Field,
};

#[test]
fn typed_data_type_builds_its_field() {
    assert_eq!(Int64Type.field("id", false), Int64Field::new("id", false));
    assert_eq!(
        UInt8Type.field("flags", true),
        UInt8Field::new("flags", true)
    );
    assert_eq!(
        BinaryType.field("payload", true),
        BinaryField::new("payload", true)
    );
}

#[test]
fn parameterised_data_types_build_their_field() {
    assert_eq!(
        TypedSerieType::new(Int64Type).field("scores", true),
        TypedSerieField::<Int64Type>::new("scores", true)
    );
    assert_eq!(
        TypedOptionalType::new(Int64Type).field("score", true),
        TypedOptionalField::<Int64Type>::new("score", true)
    );
}
