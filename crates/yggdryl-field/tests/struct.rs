//! Integration tests for the `struct` field.

use yggdryl_field::yggdryl_dtype::{self as dtype, arrow_schema};
use yggdryl_field::{RawField, Struct};

fn point_type() -> dtype::Struct {
    dtype::Struct::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]))
}

#[test]
fn struct_field_round_trips() {
    let field = Struct::new("point", point_type(), false);
    assert_eq!(field.name(), "point");
    assert_eq!(field.data_type(), &point_type());
    assert!(!field.is_nullable());
    assert_eq!(Struct::from_arrow(&field.to_arrow()).unwrap(), field);
}

#[test]
fn struct_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Struct>();
}
