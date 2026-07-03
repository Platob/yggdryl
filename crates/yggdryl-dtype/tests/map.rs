//! Integration tests for the `map` data type — the generic nested holder over a
//! key and a value type.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64Type, Map, MapType, Nested, SerieType, TypedDataType,
    TypedMap, TypedNested, UInt8Type,
};

#[test]
fn map_describes_itself_and_round_trips() {
    let map = MapType::new(UInt8Type, Int64Type);
    assert_eq!(map.name(), "map");
    assert_eq!(map.arrow_format(), "+m");
    assert_eq!(map.byte_width(), None);
    assert_eq!(map.child_count(), 1);
    assert_eq!((map.key_type(), map.value_type()), (&UInt8Type, &Int64Type));

    assert!(matches!(map.to_arrow(), arrow_schema::DataType::Map(..)));
    assert_eq!(MapType::from_arrow(&map.to_arrow()).unwrap(), map);
    assert!(matches!(
        MapType::<UInt8Type, Int64Type>::from_arrow(&SerieType::new(Int64Type).to_arrow()),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn map_codec_concatenates_entries() {
    let map = MapType::new(UInt8Type, Int64Type);
    let bytes = map.native_to_bytes(&vec![(7, 42), (8, 43)]);
    assert_eq!(bytes.len(), 18); // (1 + 8) * 2
    assert_eq!(
        map.native_from_bytes(&bytes).unwrap(),
        vec![(7, 42), (8, 43)]
    );

    let nested = MapType::new(UInt8Type, SerieType::new(Int64Type));
    assert!(matches!(
        nested.native_from_bytes(&[0; 9]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn map_is_the_generic_nested_holder() {
    fn typed_default<T, N: TypedNested<T>>(nested: &N) -> T {
        nested.default_value()
    }
    fn map_default<TK, TV, M: TypedMap<TK, TV>>(map: &M) -> Vec<(TK, TV)> {
        map.default_value()
    }
    assert_eq!(
        typed_default::<Vec<(u8, i64)>, _>(&MapType::new(UInt8Type, Int64Type)),
        Vec::<(u8, i64)>::new()
    );
    assert_eq!(
        map_default(&MapType::new(UInt8Type, Int64Type)),
        Vec::<(u8, i64)>::new()
    );
}

#[test]
fn map_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MapType<UInt8Type, Int64Type>>();
}
