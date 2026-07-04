//! Integration tests for the `map` field — the dynamic [`MapField`] and the
//! statically-typed [`TypedMapField`].

use yggdryl_field::yggdryl_dtype::{
    arrow_schema, DataType, Int64Type, TypedDataType, TypedMap, UInt8Type,
};
use yggdryl_field::{Field, MapField, TypedField, TypedMapField};

#[test]
fn typed_map_field_carries_both_layers() {
    let ranks = TypedMapField::<UInt8Type, Int64Type>::new("ranks", true);
    assert_eq!(ranks.name(), "ranks");
    assert_eq!(ranks.data_type().name(), "map");
    assert_eq!(ranks.data_type().key_type().name(), "uint8");
    assert_eq!(ranks.data_type().value_type().name(), "int64");
    assert_eq!(TypedMapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);

    fn type_name<DT: TypedDataType<Vec<(u8, i64)>>, F: TypedField<DT, Vec<(u8, i64)>>>(
        field: &F,
    ) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&ranks), "map");
}

#[test]
fn dynamic_map_field_wraps_the_dynamic_type() {
    use yggdryl_field::yggdryl_dtype::MapType;

    let ranks = MapField::new(
        "ranks",
        MapType::new(arrow_schema::DataType::UInt8, arrow_schema::DataType::Int64),
        true,
    );
    assert_eq!(ranks.name(), "ranks");
    assert_eq!(ranks.data_type().name(), "map");
    assert_eq!(MapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
}

#[test]
fn map_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MapField>();
    assert_send_sync::<TypedMapField<UInt8Type, Int64Type>>();
}
