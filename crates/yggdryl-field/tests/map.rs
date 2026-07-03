//! Integration tests for the `map` field.

use yggdryl_field::yggdryl_dtype::{Int64, RawDataType, RawMap, UInt8};
use yggdryl_field::{Field, Map, RawField};

#[test]
fn map_field_carries_both_layers() {
    let ranks = Map::<UInt8, Int64>::new("ranks", true);
    assert_eq!(ranks.name(), "ranks");
    assert_eq!(ranks.data_type().name(), "map");
    assert_eq!(ranks.data_type().key_type().name(), "uint8");
    assert_eq!(ranks.data_type().value_type().name(), "int64");
    assert_eq!(Map::from_arrow(&ranks.to_arrow()).unwrap(), ranks);

    fn type_name<F: Field<Vec<(u8, i64)>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&ranks), "map");
}

#[test]
fn map_field_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Map<UInt8, Int64>>();
}
