//! The cast pairs the engine covers, pinned family by family.
//!
//! The scalar engine is Arrow's own kernel and the nested families recurse
//! through dedicated plans; these tests pin the seams between them - the view
//! layouts, the wrapper changes, the encodings - so a regression in either
//! half names the pair it broke.

use std::sync::Arc;

use arrow_array::builder::{Int32Builder, ListBuilder, MapBuilder, StringBuilder};
use arrow_array::{
    Array, ArrayRef, BinaryViewArray, DictionaryArray, FixedSizeListArray, Int32Array, Int64Array,
    ListArray, StringArray, StringViewArray, StructArray,
};
use yggdryl::field::cast::ArrowCast as _;
use yggdryl::{DataType, Field};

fn cast(field: &Field, array: ArrayRef) -> yggdryl::arrow::Result<ArrayRef> {
    field.cast_arrow_array(array, false)
}

#[test]
fn view_layouts_cast_in_and_out_of_their_plain_spellings() {
    // Utf8View -> Utf8 and back, BinaryView -> Binary: the view layouts are
    // first-class datatypes, not lossy coercions.
    let view: ArrayRef = Arc::new(StringViewArray::from(vec![Some("alpha"), None]));
    let plain = cast(&DataType::Utf8.nullable_field("text"), Arc::clone(&view)).unwrap();
    assert_eq!(plain.data_type(), &arrow_schema::DataType::Utf8);

    let back = cast(&DataType::Utf8View.nullable_field("text"), plain).unwrap();
    assert_eq!(back.data_type(), &arrow_schema::DataType::Utf8View);

    let bytes: ArrayRef = Arc::new(BinaryViewArray::from(vec![Some(b"ab".as_slice()), None]));
    let plain = cast(&DataType::Binary.nullable_field("raw"), bytes).unwrap();
    assert_eq!(plain.data_type(), &arrow_schema::DataType::Binary);
}

#[test]
fn dictionaries_encode_and_decode_scalars() {
    // Utf8 -> Dictionary(Int32, Utf8): encoding a plain column.
    let plain: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "a"]));
    let target = DataType::from_str("dictionary<int32, utf8>")
        .unwrap()
        .nullable_field("tag");
    let encoded = cast(&target, plain).unwrap();
    let dictionary = encoded
        .as_any()
        .downcast_ref::<DictionaryArray<arrow_array::types::Int32Type>>()
        .expect("a dictionary");
    assert_eq!(dictionary.len(), 3);

    // And back out: Dictionary -> Int64 widens the decoded values.
    let keys = Int32Array::from(vec![0, 1, 0]);
    let values: ArrayRef = Arc::new(Int32Array::from(vec![10, 20]));
    let source: ArrayRef =
        Arc::new(DictionaryArray::<arrow_array::types::Int32Type>::try_new(keys, values).unwrap());
    let decoded = cast(&DataType::Int64.nullable_field("count"), source).unwrap();
    let decoded = decoded.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(decoded.values(), &[10, 20, 10]);
}

#[test]
fn list_wrappers_change_layout_and_cast_their_children() {
    let mut builder = ListBuilder::new(Int32Builder::new());
    builder.values().append_value(1);
    builder.values().append_value(2);
    builder.append(true);
    builder.values().append_value(3);
    builder.append(true);
    let list: ArrayRef = Arc::new(builder.finish());

    // List<Int32> -> LargeList<Int64>: the offset width and the child change.
    let large = cast(
        &DataType::from_str("large_list<int64>")
            .unwrap()
            .nullable_field("xs"),
        Arc::clone(&list),
    )
    .unwrap();
    assert!(matches!(
        large.data_type(),
        arrow_schema::DataType::LargeList(_)
    ));

    // FixedSizeList<Int32, 2> -> List<Int32>: a size becomes offsets.
    let fixed: ArrayRef = Arc::new(FixedSizeListArray::from_iter_primitive::<
        arrow_array::types::Int32Type,
        _,
        _,
    >(
        vec![Some(vec![Some(1), Some(2)]), Some(vec![Some(3), Some(4)])],
        2,
    ));
    let unsized_list = cast(
        &DataType::from_str("list<int32>")
            .unwrap()
            .nullable_field("xs"),
        fixed,
    )
    .unwrap();
    assert!(matches!(
        unsized_list.data_type(),
        arrow_schema::DataType::List(_)
    ));
}

#[test]
fn structs_reconcile_by_name_inside_a_list() {
    // List<Struct{id, name}> -> List<Struct{ID: int64}>: the dedicated arms
    // recurse, select case-insensitively, and drop the extra column.
    let ids = Int32Array::from(vec![1, 2]);
    let names = StringArray::from(vec!["a", "b"]);
    let entries = StructArray::from(vec![
        (
            Arc::new(arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int32,
                false,
            )),
            Arc::new(ids) as ArrayRef,
        ),
        (
            Arc::new(arrow_schema::Field::new(
                "name",
                arrow_schema::DataType::Utf8,
                false,
            )),
            Arc::new(names) as ArrayRef,
        ),
    ]);
    let offsets = arrow_buffer::OffsetBuffer::new(vec![0, 1, 2].into());
    let child = Arc::new(arrow_schema::Field::new(
        "item",
        entries.data_type().clone(),
        true,
    ));
    let list: ArrayRef = Arc::new(ListArray::new(child, offsets, Arc::new(entries), None));

    let target = DataType::from_str("list<struct<ID: int64>>")
        .unwrap()
        .nullable_field("rows");
    let narrowed = cast(&target, list).unwrap();
    let arrow_schema::DataType::List(item) = narrowed.data_type() else {
        panic!("expected a list, got {:?}", narrowed.data_type());
    };
    let arrow_schema::DataType::Struct(fields) = item.data_type() else {
        panic!("expected a struct item, got {:?}", item.data_type());
    };
    assert_eq!(fields.len(), 1);
    assert_eq!(fields[0].name(), "ID");
    assert_eq!(fields[0].data_type(), &arrow_schema::DataType::Int64);
}

#[test]
fn a_map_wrapper_change_stays_refused_with_the_reconciliation_reason() {
    // A Map target from a non-Map source would bypass key/value semantics;
    // the guard names why instead of letting positional casting run.
    // The core reconciles map children on the conventional key/value names.
    let names = arrow_array::builder::MapFieldNames {
        entry: "entries".into(),
        key: "key".into(),
        value: "value".into(),
    };
    let mut builder = MapBuilder::new(Some(names), StringBuilder::new(), Int32Builder::new());
    builder.keys().append_value("k");
    builder.values().append_value(1);
    builder.append(true).unwrap();
    let map: ArrayRef = Arc::new(builder.finish());

    // Map -> Map with a cast value type goes through the dedicated arm.
    let widened = cast(
        &DataType::from_str("map<utf8, int64>")
            .unwrap()
            .nullable_field("m"),
        map,
    )
    .unwrap();
    assert!(matches!(
        widened.data_type(),
        arrow_schema::DataType::Map(..)
    ));

    // Utf8 -> Map is not a cast anything defines; the refusal says so.
    let refused = cast(
        &DataType::from_str("map<utf8, int64>")
            .unwrap()
            .nullable_field("m"),
        Arc::new(StringArray::from(vec!["x"])) as ArrayRef,
    );
    assert!(refused.is_err());
}

#[test]
fn unsafe_and_safe_disagree_exactly_where_a_value_cannot_convert() {
    let source: ArrayRef = Arc::new(StringArray::from(vec![Some("12"), Some("nope")]));
    let target = DataType::Int64.nullable_field("n");

    // Unsafe: the unconvertible value is an error naming the cast.
    assert!(target.cast_arrow_array(Arc::clone(&source), false).is_err());

    // Safe: it becomes null instead.
    let softened = target.cast_arrow_array(source, true).unwrap();
    let softened = softened.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(softened.value(0), 12);
    assert!(softened.is_null(1));
}
