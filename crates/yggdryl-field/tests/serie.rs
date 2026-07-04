//! Integration tests for the `serie` field — the dynamic [`SerieField`] and the
//! statically-typed [`TypedSerieField`].

use yggdryl_field::yggdryl_dtype::{
    arrow_schema, DataType, Int64Type, TypedDataType, TypedSerie, UInt8Type,
};
use yggdryl_field::{Field, SerieField, TypedField, TypedSerieField};

#[test]
fn typed_serie_field_carries_both_layers() {
    let scores = TypedSerieField::<Int64Type>::new("scores", true);
    assert_eq!(scores.name(), "scores");
    assert_eq!(scores.data_type().name(), "list");
    assert_eq!(scores.data_type().value_type().name(), "int64");
    assert_eq!(
        TypedSerieField::from_arrow(&scores.to_arrow()).unwrap(),
        scores
    );

    fn type_name<DT: TypedDataType<Vec<i64>>, F: TypedField<DT, Vec<i64>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&scores), "list");
}

#[test]
fn typed_serie_field_is_generic_over_the_value_type() {
    // The same field shape holds for any value type; the widths differ only in
    // the value type they report.
    let flags = TypedSerieField::<UInt8Type>::new("flags", false);
    assert_eq!(flags.data_type().value_type().name(), "uint8");
    assert!(!flags.is_nullable());
    assert_eq!(
        TypedSerieField::from_arrow(&flags.to_arrow()).unwrap(),
        flags
    );
}

#[test]
fn dynamic_serie_field_wraps_the_dynamic_type() {
    use yggdryl_field::yggdryl_dtype::SerieType;

    let scores = SerieField::new(
        "scores",
        SerieType::new(arrow_schema::DataType::Int64),
        true,
    );
    assert_eq!(scores.name(), "scores");
    assert_eq!(scores.data_type().name(), "list");
    assert_eq!(SerieField::from_arrow(&scores.to_arrow()).unwrap(), scores);
}

#[test]
fn serie_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SerieField>();
    assert_send_sync::<TypedSerieField<Int64Type>>();
}
