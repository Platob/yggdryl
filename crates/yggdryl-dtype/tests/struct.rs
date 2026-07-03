//! Integration tests for the `struct` data type — the dynamic ordered set of named
//! child fields.

use yggdryl_dtype::{arrow_schema, DataType, Nested, Struct, StructType};

fn point_type() -> StructType {
    StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]))
}

#[test]
fn struct_describes_itself_and_round_trips_losslessly() {
    let point = point_type();
    assert_eq!(point.name(), "struct");
    assert_eq!(point.arrow_format(), "+s");
    assert_eq!(point.byte_width(), None);
    assert_eq!(point.child_count(), 2);
    assert_eq!(point.fields().len(), 2);
    assert_eq!(StructType::ID.name(), point.name());

    assert_eq!(StructType::from_arrow(&point.to_arrow()).unwrap(), point);
}

#[test]
fn struct_is_send_sync_and_joins_dyn_schemas() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StructType>();

    // Nested types join heterogeneous schemas through the vtable.
    let types: Vec<Box<dyn DataType>> = vec![Box::new(point_type())];
    assert_eq!(types[0].name(), "struct");
}
