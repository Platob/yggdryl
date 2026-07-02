//! Integration tests for the nested families — `list`, `map` and `struct` — and
//! the defaults across the typed layer.

use yggdryl_data::arrow_array::Array;
use yggdryl_data::{
    arrow_array, arrow_schema, DataError, DataType, Field, Int64, Int64Scalar, List, ListField,
    ListScalar, ListType, Map, MapField, MapScalar, MapType, Nested, OptionalScalar, OptionalType,
    RawDataType, RawField, RawList, RawMap, RawScalar, RawStruct, StructField, StructScalar,
    StructType, UInt8, UInt8Scalar,
};

type Int64List = ListScalar<Int64, Int64Scalar>;
type RankMap = MapScalar<UInt8, Int64, UInt8Scalar, Int64Scalar>;

fn point_type() -> StructType {
    StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]))
}

#[test]
fn list_describes_itself_and_round_trips() {
    let list = ListType::new(Int64);
    assert_eq!(list.name(), "list");
    assert_eq!(list.arrow_format(), "+l");
    assert_eq!(list.byte_width(), None);
    assert_eq!(list.child_count(), 1);
    assert_eq!(list.value_type(), &Int64);

    assert_eq!(ListType::from_arrow(&list.to_arrow()).unwrap(), list);
    assert!(matches!(
        ListType::<Int64>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn list_codec_concatenates_elements() {
    let list = ListType::new(Int64);
    let bytes = list.native_to_bytes(&vec![1, 2, 3]);
    assert_eq!(bytes.len(), 24);
    assert_eq!(list.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
    assert_eq!(list.native_from_bytes(&[]).unwrap(), Vec::<i64>::new());

    // A remainder is a length error.
    assert!(matches!(
        list.native_from_bytes(&[0; 9]),
        Err(DataError::InvalidByteLength {
            expected: 8,
            got: 9
        })
    ));

    // A variable-width element cannot be split from bytes.
    let nested = ListType::new(ListType::new(Int64));
    assert!(matches!(
        nested.native_from_bytes(&[0; 8]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn list_field_carries_both_layers() {
    let scores = ListField::<Int64>::new("scores", true);
    assert_eq!(scores.name(), "scores");
    assert_eq!(scores.data_type().name(), "list");
    assert_eq!(ListField::from_arrow(&scores.to_arrow()).unwrap(), scores);

    fn type_name<F: Field<Vec<i64>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&scores), "list");
}

#[test]
fn list_scalar_round_trips_all_shapes() {
    // Elements, the empty list and null are three distinct states.
    let numbers = Int64List::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
    let arrow = numbers.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(Int64List::from_arrow(arrow.as_ref()).unwrap(), numbers);

    let empty = Int64List::new(Vec::new());
    assert!(!empty.is_null());
    assert_eq!(
        Int64List::from_arrow(empty.to_arrow().as_ref()).unwrap(),
        empty
    );
    assert_eq!(Int64List::default(), empty);

    let missing = Int64List::null();
    assert!(missing.is_null());
    assert_eq!(
        Int64List::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // Construction from native shapes.
    assert_eq!(Int64List::from(None::<Vec<Int64Scalar>>), missing);

    // A non-list array is refused.
    assert!(matches!(
        Int64List::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn map_describes_itself_and_round_trips() {
    let map = MapType::new(UInt8, Int64);
    assert_eq!(map.name(), "map");
    assert_eq!(map.arrow_format(), "+m");
    assert_eq!(map.child_count(), 1);
    assert_eq!((map.key_type(), map.value_type()), (&UInt8, &Int64));

    assert_eq!(MapType::from_arrow(&map.to_arrow()).unwrap(), map);
    assert!(matches!(
        MapType::<UInt8, Int64>::from_arrow(&ListType::new(Int64).to_arrow()),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn map_codec_concatenates_entries() {
    let map = MapType::new(UInt8, Int64);
    let bytes = map.native_to_bytes(&vec![(7, 42), (8, 43)]);
    assert_eq!(bytes.len(), 18); // (1 + 8) * 2
    assert_eq!(
        map.native_from_bytes(&bytes).unwrap(),
        vec![(7, 42), (8, 43)]
    );

    let nested = MapType::new(UInt8, ListType::new(Int64));
    assert!(matches!(
        nested.native_from_bytes(&[0; 9]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn map_field_and_scalar_round_trip() {
    let ranks = MapField::<UInt8, Int64>::new("ranks", true);
    assert_eq!(MapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
    fn type_name<F: Field<Vec<(u8, i64)>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&ranks), "map");

    let scalar = RankMap::new(vec![
        (UInt8Scalar::new(7), Int64Scalar::new(42)),
        (UInt8Scalar::new(8), Int64Scalar::null()),
    ]);
    let arrow = scalar.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(RankMap::from_arrow(arrow.as_ref()).unwrap(), scalar);

    let missing = RankMap::null();
    assert_eq!(
        RankMap::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );
    assert_eq!(RankMap::default(), RankMap::new(Vec::new()));
}

#[test]
fn struct_describes_itself_and_round_trips_losslessly() {
    let point = point_type();
    assert_eq!(point.name(), "struct");
    assert_eq!(point.arrow_format(), "+s");
    assert_eq!(point.child_count(), 2);
    assert_eq!(point.fields().len(), 2);
    assert_eq!(StructType::ID.name(), point.name());

    assert_eq!(StructType::from_arrow(&point.to_arrow()).unwrap(), point);

    let field = StructField::new("point", point, false);
    assert_eq!(StructField::from_arrow(&field.to_arrow()).unwrap(), field);
}

#[test]
fn struct_scalar_validates_and_round_trips() {
    let point = point_type();
    let column = |value: i64| -> arrow_array::ArrayRef {
        std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([value]))
    };

    let row = StructScalar::new(point.clone(), vec![column(1), column(2)]).unwrap();
    assert!(!row.is_null());
    assert_eq!(row.value().map(<[_]>::len), Some(2));
    let arrow = row.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(StructScalar::from_arrow(arrow.as_ref()).unwrap(), row);

    let missing = StructScalar::null(point.clone());
    assert!(missing.is_null());
    assert_eq!(
        StructScalar::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // Wrong column count, wrong length and wrong type are all actionable errors.
    assert!(matches!(
        StructScalar::new(point.clone(), vec![column(1)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    let two: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([1, 2]));
    assert!(matches!(
        StructScalar::new(point.clone(), vec![two, column(2)]),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    let wrong: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::UInt8Array::from_iter_values([1]));
    assert!(matches!(
        StructScalar::new(point, vec![wrong, column(2)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn defaults_flow_through_the_typed_layer() {
    // Integers default to zero, held in a value scalar.
    assert_eq!(Int64.default_value(), 0);
    assert_eq!(Int64.default_scalar(), Int64Scalar::new(0));

    // The optional's scalar models nullness: its default is the null variant.
    let optional = OptionalType::new(Int64);
    assert_eq!(optional.default_value(), 0);
    assert_eq!(
        optional.default_scalar(),
        OptionalScalar::<Int64, Int64Scalar>::null()
    );

    // Sequences default to empty, not null.
    assert_eq!(ListType::new(Int64).default_value(), Vec::<i64>::new());
    assert_eq!(
        ListType::new(Int64).default_scalar(),
        Int64List::new(Vec::new())
    );
    assert_eq!(
        MapType::new(UInt8, Int64).default_value(),
        Vec::<(u8, i64)>::new()
    );
    assert_eq!(
        MapType::new(UInt8, Int64).default_scalar(),
        RankMap::new(Vec::new())
    );

    // Generic code reaches defaults through the typed trait pairs.
    fn list_default<T, L: List<T>>(list: &L) -> Vec<T> {
        list.default_value()
    }
    fn map_default<TK, TV, M: Map<TK, TV>>(map: &M) -> Vec<(TK, TV)> {
        map.default_value()
    }
    assert_eq!(list_default(&ListType::new(Int64)), Vec::<i64>::new());
    assert_eq!(
        map_default(&MapType::new(UInt8, Int64)),
        Vec::<(u8, i64)>::new()
    );
}

#[test]
fn nested_types_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ListType<Int64>>();
    assert_send_sync::<Int64List>();
    assert_send_sync::<MapType<UInt8, Int64>>();
    assert_send_sync::<RankMap>();
    assert_send_sync::<StructType>();
    assert_send_sync::<StructScalar>();

    // Nested types join heterogeneous schemas through the vtable.
    let types: Vec<Box<dyn RawDataType>> = vec![
        Box::new(ListType::new(Int64)),
        Box::new(MapType::new(UInt8, Int64)),
        Box::new(point_type()),
    ];
    let names: Vec<_> = types.iter().map(|t| t.name().to_string()).collect();
    assert_eq!(names, vec!["list", "map", "struct"]);
}
