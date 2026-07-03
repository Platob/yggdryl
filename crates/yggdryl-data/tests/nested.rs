//! Integration tests for the nested families — `list`, `map` and `struct` — and
//! the defaults across the typed layer.

use yggdryl_data::arrow_array::Array;
use yggdryl_data::{
    arrow_array, arrow_buffer, arrow_schema, DataError, DataType, Field, Int64, Int64Serie,
    Int64Type, ListField, ListType, Map, MapField, MapType, Optional, OptionalType, RawDataType,
    RawField, RawList, RawMap, RawNested, RawScalar, RawStruct, Serie, Struct, StructField,
    StructType, TypedList, TypedMap, UInt8, UInt8Type,
};

type Int64ListScalar = Serie<Int64Type, Int64>;
type RankMap = Map<UInt8Type, Int64Type, UInt8, Int64>;

fn point_type() -> StructType {
    StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]))
}

#[test]
fn list_describes_itself_and_round_trips() {
    let list = ListType::new(Int64Type);
    assert_eq!(list.name(), "list");
    assert_eq!(list.arrow_format(), "+l");
    assert_eq!(list.byte_width(), None);
    assert_eq!(list.child_count(), 1);
    assert_eq!(list.value_type(), &Int64Type);

    assert_eq!(ListType::from_arrow(&list.to_arrow()).unwrap(), list);
    assert!(matches!(
        ListType::<Int64Type>::from_arrow(&arrow_schema::DataType::Int64),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn list_codec_concatenates_elements() {
    let list = ListType::new(Int64Type);
    let bytes = list.native_to_bytes(&vec![1, 2, 3]);
    assert_eq!(bytes.len(), 24);
    assert_eq!(list.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
    assert_eq!(list.native_from_bytes(&[]).unwrap(), Vec::<i64>::new());

    // A remainder is a length error naming the nearest whole-element length.
    assert!(matches!(
        list.native_from_bytes(&[0; 9]),
        Err(DataError::InvalidByteLength {
            expected: 16,
            got: 9
        })
    ));

    // An optional element delegates its codec width to the value type, so the
    // round trip stays exact even though the *physical* width is indeterminate.
    let optional_list = ListType::new(OptionalType::new(Int64Type));
    let bytes = optional_list.native_to_bytes(&vec![1, 2]);
    assert_eq!(optional_list.native_from_bytes(&bytes).unwrap(), vec![1, 2]);

    // A variable-width element cannot be split from bytes.
    let nested = ListType::new(ListType::new(Int64Type));
    assert!(matches!(
        nested.native_from_bytes(&[0; 8]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn list_field_carries_both_layers() {
    let scores = ListField::<Int64Type>::new("scores", true);
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
    let numbers = Int64ListScalar::new(vec![Int64::new(1), Int64::null()]);
    let arrow = numbers.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(
        Int64ListScalar::from_arrow(arrow.as_ref()).unwrap(),
        numbers
    );

    // The scalar accessors read elements back out, as scalars or native values.
    assert_eq!(numbers.get_scalar_at(0), Some(Int64::new(1)));
    assert_eq!(numbers.get_scalar_at(1), Some(Int64::null()));
    assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1);
    assert_eq!(numbers.get_at::<i32>(0).unwrap(), 1); // converted, exact-or-error
    assert!(matches!(
        numbers.get_at::<i64>(1),
        Err(DataError::NullValue) // a null element holds no value
    ));
    assert!(matches!(
        numbers.get_at::<i64>(2),
        Err(DataError::OutOfBounds { index: 2, len: 2 })
    ));

    let empty = Int64ListScalar::new(Vec::new());
    assert!(!empty.is_null());
    assert_eq!(
        Int64ListScalar::from_arrow(empty.to_arrow().as_ref()).unwrap(),
        empty
    );
    assert_eq!(Int64ListScalar::default(), empty);

    let missing = Int64ListScalar::null();
    assert!(missing.is_null());
    assert_eq!(
        Int64ListScalar::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // Construction from native shapes.
    assert_eq!(Int64ListScalar::from(None::<Vec<Int64>>), missing);

    // A non-list array is refused.
    assert!(matches!(
        Int64ListScalar::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn int64_array_reads_borrowed_buffers() {
    let numbers = Int64Serie::from(vec![1, 2, 3]);
    assert!(!numbers.is_null());
    assert_eq!(numbers.len(), 3);
    assert_eq!(numbers.values(), Some(&[1, 2, 3][..]));
    assert_eq!(numbers.value(), Some(&[1, 2, 3][..]));
    assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1);
    assert!(matches!(
        numbers.get_at::<i64>(3),
        Err(DataError::OutOfBounds { index: 3, len: 3 })
    ));
    assert_eq!(numbers.get_scalar_at(2), Some(Int64::new(3)));
    assert_eq!(numbers.get_scalar_at(3), None);
    assert!(numbers.nulls().is_none());

    // The reassembled Arrow array borrows the same buffer — zero copy.
    let arrow = numbers.array().unwrap();
    assert_eq!(arrow.values().as_ptr(), numbers.values().unwrap().as_ptr());

    // Per-element nulls are read null-aware; the raw buffer keeps the slots.
    let sparse = Int64Serie::from(vec![Some(1), None]);
    assert_eq!(sparse.get_at::<i64>(0).unwrap(), 1);
    assert!(matches!(sparse.get_at::<i64>(1), Err(DataError::NullValue)));
    assert_eq!(sparse.get_scalar_at(1), Some(Int64::null()));
    assert_eq!(sparse.values().map(<[i64]>::len), Some(2));
    assert_eq!(
        sparse.nulls().map(arrow_buffer::NullBuffer::null_count),
        Some(1)
    );

    // An all-valid null buffer is normalized away at construction, so the stored
    // form is canonical and equality holds trivially.
    let buffered = Int64Serie::new(
        arrow_buffer::ScalarBuffer::from(vec![1, 2, 3]),
        Some(arrow_buffer::NullBuffer::new_valid(3)),
    )
    .unwrap();
    assert!(buffered.nulls().is_none());
    assert_eq!(buffered, numbers);

    // A null buffer of the wrong length is refused with an actionable error.
    assert!(matches!(
        Int64Serie::new(
            arrow_buffer::ScalarBuffer::from(vec![1, 2, 3]),
            Some(arrow_buffer::NullBuffer::new_valid(2)),
        ),
        Err(DataError::MismatchedNullBufferLength {
            expected: 3,
            got: 2
        })
    ));
}

#[test]
fn int64_serie_bridges_to_core_io_resources() {
    use yggdryl_data::yggdryl_core::{ByteBuffer, RawIOBase, Whence};

    // pwrite_io lays the elements out little-endian through pwrite_i64 ...
    let numbers = Int64Serie::from(vec![1, -2, 3]);
    let mut buffer = ByteBuffer::new();
    numbers.pwrite_io(&mut buffer, 0, Whence::Start).unwrap();
    assert_eq!(buffer.byte_size(), 24);
    assert_eq!(buffer.pread_i64(8, Whence::Start).unwrap(), -2);

    // ... and from_io reads them back: the exact inverse for all-valid elements.
    assert_eq!(Int64Serie::from_io(&buffer).unwrap(), numbers);

    // A byte size that is not a whole number of elements is refused.
    buffer.resize_bytes(25).unwrap();
    assert!(matches!(
        Int64Serie::from_io(&buffer),
        Err(DataError::InvalidByteLength {
            expected: 32,
            got: 25
        })
    ));

    // A null serie holds no elements to write.
    assert!(matches!(
        Int64Serie::null().pwrite_io(&mut buffer, 0, Whence::Start),
        Err(DataError::NullValue)
    ));
}

#[test]
fn int64_array_round_trips_through_arrow_zero_copy() {
    let numbers = Int64Serie::from(vec![Some(1), None, Some(3)]);
    let arrow = numbers.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(Int64Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);

    // The list's child elements are the same buffer, shared, not copied.
    let list = arrow
        .as_any()
        .downcast_ref::<arrow_array::ListArray>()
        .unwrap();
    let child = list
        .values()
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .unwrap();
    assert_eq!(child.values().as_ptr(), numbers.values().unwrap().as_ptr());

    // The generic and the buffer-backed list scalar agree on the Arrow shape.
    let generic = Int64ListScalar::new(vec![Int64::new(1), Int64::null(), Int64::new(3)]);
    assert_eq!(generic.to_arrow().as_ref(), arrow.as_ref());

    // Empty and null are distinct states, both round-tripped.
    let empty = Int64Serie::default();
    assert!(!empty.is_null());
    assert!(empty.is_empty());
    assert_eq!(
        Int64Serie::from_arrow(empty.to_arrow().as_ref()).unwrap(),
        empty
    );

    let missing = Int64Serie::null();
    assert!(missing.is_null());
    assert_eq!((missing.values(), missing.array()), (None, None));
    assert!(matches!(
        missing.get_at::<i64>(0),
        Err(DataError::NullValue)
    ));
    assert_eq!(
        Int64Serie::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // A non-list array is refused.
    assert!(matches!(
        Int64Serie::from_arrow(&arrow_array::Int64Array::from_iter_values([1])),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn map_describes_itself_and_round_trips() {
    let map = MapType::new(UInt8Type, Int64Type);
    assert_eq!(map.name(), "map");
    assert_eq!(map.arrow_format(), "+m");
    assert_eq!(map.child_count(), 1);
    assert_eq!((map.key_type(), map.value_type()), (&UInt8Type, &Int64Type));

    assert_eq!(MapType::from_arrow(&map.to_arrow()).unwrap(), map);
    assert!(matches!(
        MapType::<UInt8Type, Int64Type>::from_arrow(&ListType::new(Int64Type).to_arrow()),
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

    let nested = MapType::new(UInt8Type, ListType::new(Int64Type));
    assert!(matches!(
        nested.native_from_bytes(&[0; 9]),
        Err(DataError::IndeterminateElementWidth { .. })
    ));
}

#[test]
fn map_field_and_scalar_round_trip() {
    let ranks = MapField::<UInt8Type, Int64Type>::new("ranks", true);
    assert_eq!(MapField::from_arrow(&ranks.to_arrow()).unwrap(), ranks);
    fn type_name<F: Field<Vec<(u8, i64)>>>(field: &F) -> String {
        field.data_type().name().to_string()
    }
    assert_eq!(type_name(&ranks), "map");

    let scalar = RankMap::new(vec![
        (UInt8::new(7), Int64::new(42)),
        (UInt8::new(8), Int64::null()),
    ])
    .unwrap();
    let arrow = scalar.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(RankMap::from_arrow(arrow.as_ref()).unwrap(), scalar);

    let missing = RankMap::null();
    assert_eq!(
        RankMap::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );
    assert_eq!(RankMap::default(), RankMap::new(Vec::new()).unwrap());
    // A null key is refused: Arrow map keys are non-nullable.
    assert!(matches!(
        RankMap::new(vec![(UInt8::null(), Int64::new(1))]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
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

    let row = Struct::new(point.clone(), vec![column(1), column(2)]).unwrap();
    assert!(!row.is_null());
    assert_eq!(row.value().map(<[_]>::len), Some(2));
    let arrow = row.to_arrow();
    assert_eq!(arrow.len(), 1);
    assert_eq!(Struct::from_arrow(arrow.as_ref()).unwrap(), row);

    let missing = Struct::null(point.clone());
    assert!(missing.is_null());
    assert_eq!(
        Struct::from_arrow(missing.to_arrow().as_ref()).unwrap(),
        missing
    );

    // Wrong column count, wrong length and wrong type are all actionable errors.
    assert!(matches!(
        Struct::new(point.clone(), vec![column(1)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    let two: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([1, 2]));
    assert!(matches!(
        Struct::new(point.clone(), vec![two, column(2)]),
        Err(DataError::InvalidScalarLength { got: 2 })
    ));
    let wrong: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::UInt8Array::from_iter_values([1]));
    assert!(matches!(
        Struct::new(point.clone(), vec![wrong, column(2)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    // A null in a non-nullable child is refused at construction, not a panic later.
    let null_column: arrow_array::ArrayRef =
        std::sync::Arc::new(arrow_array::Int64Array::new_null(1));
    assert!(matches!(
        Struct::new(point, vec![null_column, column(2)]),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn defaults_flow_through_the_typed_layer() {
    // Integers default to zero, held in a value scalar.
    assert_eq!(Int64Type.default_value(), 0);
    assert_eq!(Int64Type.default_scalar(), Int64::new(0));

    // The optional's scalar models nullness: its default is the null variant.
    let optional = OptionalType::new(Int64Type);
    assert_eq!(optional.default_value(), 0);
    assert_eq!(
        optional.default_scalar(),
        Optional::<Int64Type, Int64>::null()
    );

    // Sequences default to empty, not null.
    assert_eq!(ListType::new(Int64Type).default_value(), Vec::<i64>::new());
    assert_eq!(
        ListType::new(Int64Type).default_scalar(),
        Int64ListScalar::new(Vec::new())
    );
    assert_eq!(
        MapType::new(UInt8Type, Int64Type).default_value(),
        Vec::<(u8, i64)>::new()
    );
    assert_eq!(
        MapType::new(UInt8Type, Int64Type).default_scalar(),
        RankMap::default()
    );

    // Generic code reaches defaults through the typed trait pairs.
    fn list_default<T, L: TypedList<T>>(list: &L) -> Vec<T> {
        list.default_value()
    }
    fn map_default<TK, TV, M: TypedMap<TK, TV>>(map: &M) -> Vec<(TK, TV)> {
        map.default_value()
    }
    assert_eq!(list_default(&ListType::new(Int64Type)), Vec::<i64>::new());
    assert_eq!(
        map_default(&MapType::new(UInt8Type, Int64Type)),
        Vec::<(u8, i64)>::new()
    );
}

#[test]
fn list_and_map_are_the_generic_nested_holders() {
    // The typed pair: RawNested counts children, Nested<T> adds the native codec.
    fn raw_children<N: RawNested>(nested: &N) -> usize {
        nested.child_count()
    }
    fn typed_default<T, N: yggdryl_data::Nested<T>>(nested: &N) -> T {
        nested.default_value()
    }
    assert_eq!(raw_children(&ListType::new(Int64Type)), 1);
    assert_eq!(raw_children(&point_type()), 2);
    assert_eq!(
        typed_default::<Vec<i64>, _>(&ListType::new(Int64Type)),
        Vec::<i64>::new()
    );
    assert_eq!(
        typed_default::<Vec<(u8, i64)>, _>(&MapType::new(UInt8Type, Int64Type)),
        Vec::<(u8, i64)>::new()
    );
}

#[test]
fn nested_types_are_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<ListType<Int64Type>>();
    assert_send_sync::<Int64ListScalar>();
    assert_send_sync::<Int64Serie>();
    assert_send_sync::<MapType<UInt8Type, Int64Type>>();
    assert_send_sync::<RankMap>();
    assert_send_sync::<StructType>();
    assert_send_sync::<Struct>();

    // Nested types join heterogeneous schemas through the vtable.
    let types: Vec<Box<dyn RawDataType>> = vec![
        Box::new(ListType::new(Int64Type)),
        Box::new(MapType::new(UInt8Type, Int64Type)),
        Box::new(point_type()),
    ];
    let names: Vec<_> = types.iter().map(|t| t.name().to_string()).collect();
    assert_eq!(names, vec!["list", "map", "struct"]);
}
