//! `Field::display()` — the compact `name: type` form, with a `?` for nullable
//! fields and the data type's own recursive signature for nested fields.

use yggdryl_field::yggdryl_dtype::{arrow_schema, SerieType};
use yggdryl_field::{Field, Int64Field, TypedSerieField, Utf8Field};

#[test]
fn atomic_fields_show_name_type_and_nullability() {
    assert_eq!(Int64Field::new("id", false).display(), "id: int64");
    assert_eq!(Int64Field::new("age", true).display(), "age: int64?");
    assert_eq!(Utf8Field::new("name", true).display(), "name: utf8?");
}

#[test]
fn nested_fields_show_the_recursive_signature() {
    // A serie field prints the element type inside `list<…>`.
    let scores: TypedSerieField<yggdryl_field::yggdryl_dtype::Int64Type> =
        TypedSerieField::new("scores", false);
    assert_eq!(scores.display(), "scores: list<int64>");

    // A dynamic serie field of a struct element nests both levels.
    let point =
        arrow_schema::DataType::Struct(arrow_schema::Fields::from(vec![arrow_schema::Field::new(
            "x",
            arrow_schema::DataType::Int64,
            false,
        )]));
    let items = yggdryl_field::SerieField::new("items", SerieType::new(point), true);
    assert_eq!(items.display(), "items: list<struct<x: int64>>?");
}
