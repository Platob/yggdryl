//! Integration tests for the `map` data type — the dynamic [`MapType`] and the
//! statically-typed [`TypedMapType`] over a key and a value type.

use yggdryl_dtype::{
    arrow_schema, DataError, DataType, Int64Type, Map, MapType, Nested, TypedDataType, TypedMap,
    TypedMapType, TypedNested, TypedSerieType, UInt8Type,
};

#[test]
fn typed_map_describes_itself_and_round_trips() {
    let map = TypedMapType::new(UInt8Type, Int64Type);
    assert_eq!(map.name(), "map");
    assert_eq!(map.arrow_format(), "+m");
    assert_eq!(map.byte_width(), None);
    assert_eq!(map.child_count(), 1);
    assert_eq!((map.key_type(), map.value_type()), (&UInt8Type, &Int64Type));

    assert!(matches!(map.to_arrow(), arrow_schema::DataType::Map(..)));
    assert_eq!(TypedMapType::from_arrow(&map.to_arrow()).unwrap(), map);
    assert!(matches!(
        TypedMapType::<UInt8Type, Int64Type>::from_arrow(
            &TypedSerieType::new(Int64Type).to_arrow()
        ),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn dynamic_map_is_arrow_backed_and_erases() {
    let map = MapType::new(arrow_schema::DataType::UInt8, arrow_schema::DataType::Int64);
    assert_eq!(map.name(), "map");
    assert_eq!(map.child_count(), 1);
    assert_eq!(map.entry_fields().len(), 2);
    assert_eq!(map.entries_field().name(), "entries");

    // erase() and from_arrow agree; the round trip is lossless.
    assert_eq!(TypedMapType::new(UInt8Type, Int64Type).erase(), map);
    assert_eq!(MapType::from_arrow(&map.to_arrow()).unwrap(), map);
    assert!(matches!(
        MapType::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn map_codec_concatenates_entries() {
    let map = TypedMapType::new(UInt8Type, Int64Type);
    let bytes = map.native_to_bytes(&vec![(7, 42), (8, 43)]);
    assert_eq!(bytes.len(), 18); // (1 + 8) * 2
    assert_eq!(
        map.native_from_bytes(&bytes).unwrap(),
        vec![(7, 42), (8, 43)]
    );

    let nested = TypedMapType::new(UInt8Type, TypedSerieType::new(Int64Type));
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
        typed_default::<Vec<(u8, i64)>, _>(&TypedMapType::new(UInt8Type, Int64Type)),
        Vec::<(u8, i64)>::new()
    );
    assert_eq!(
        map_default(&TypedMapType::new(UInt8Type, Int64Type)),
        Vec::<(u8, i64)>::new()
    );
}

#[test]
fn map_is_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MapType>();
    assert_send_sync::<TypedMapType<UInt8Type, Int64Type>>();
}
