//! Unit tests for the serie foundation: the [factory](crate::from_arrow), the
//! [`Serie`](crate::Serie) base accessors and the typed primitive series.

use std::sync::Arc;

use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Date32Array, Date64Array, DurationSecondArray,
    Float32Array, Float64Array, Int32Array, Int64Array, LargeBinaryArray, LargeStringArray,
    NullArray, StringArray, TimestampMicrosecondArray, UInt32Array,
};
use yggdryl_schema::{DataType, Field, TypeCategory};

use crate::{
    child, child_range, from_array, from_arrow, BinarySerie, BooleanSerie, Date32Serie,
    Date64Serie, DateRangeSerie, Float32Serie, Float64Serie, IndexSerie, Int32Serie, Int64Serie,
    RangeSerie, Scalar, Serie, SerieRef, TimestampMicrosecondSerie, TypedSerie, UInt64Serie,
    VarcharSerie,
};

#[test]
fn from_array_builds_int_serie() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(3)]));
    let serie = from_array("id", array).unwrap();

    assert_eq!(serie.name(), "id");
    assert_eq!(serie.len(), 3);
    assert!(!serie.is_empty());
    assert_eq!(serie.null_count(), 1);
    assert_eq!(serie.data_type(), &DataType::int(32, true));
    assert_eq!(serie.category(), TypeCategory::Primitive);
    assert!(serie.is_null(1));
    assert!(serie.is_valid(0));
    assert!(serie.is_null(99)); // out of bounds reads as null

    let ints = serie.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(ints.get(0), Some(1));
    assert_eq!(ints.get(1), None);
    assert_eq!(ints.value(2), 3);
    assert_eq!(ints.to_vec(), vec![Some(1), None, Some(3)]);
}

#[test]
fn from_values_constructors() {
    let ints = Int32Serie::from_values("n", vec![Some(1), None, Some(3)]);
    assert_eq!(ints.len(), 3);
    assert_eq!(ints.get(1), None);
    assert_eq!(ints.data_type(), &DataType::int(32, true));

    let strings = VarcharSerie::<i32>::from_values("s", vec![Some("x"), None, Some("z")]);
    assert_eq!(strings.len(), 3);
    assert_eq!(strings.str_value(0), Some("x"));
    assert_eq!(strings.get(2), Some("z".to_string()));

    let flags = BooleanSerie::from_values("b", vec![Some(true), Some(false), None]);
    assert_eq!(flags.get(0), Some(true));
    assert_eq!(flags.null_count(), 1);
}

#[test]
fn float_and_boolean_series() {
    let floats: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));
    let serie = from_array("x", floats).unwrap();
    let typed = serie.as_any().downcast_ref::<Float64Serie>().unwrap();
    assert_eq!(typed.value(1), 2.5);

    let bools: ArrayRef = Arc::new(BooleanArray::from(vec![Some(true), None]));
    let serie = from_array("flag", bools).unwrap();
    assert_eq!(serie.data_type(), &DataType::Boolean);
    let typed = serie.as_any().downcast_ref::<BooleanSerie>().unwrap();
    assert_eq!(typed.iter().collect::<Vec<_>>(), vec![Some(true), None]);
}

#[test]
fn string_and_binary_series() {
    let utf8: ArrayRef = Arc::new(StringArray::from(vec![Some("a"), None, Some("c")]));
    let serie = from_array("name", utf8).unwrap();
    assert_eq!(serie.data_type(), &DataType::varchar());
    let typed = serie.as_any().downcast_ref::<VarcharSerie<i32>>().unwrap();
    assert_eq!(typed.str_value(0), Some("a"));
    assert_eq!(typed.get(2), Some("c".to_string()));

    let large: ArrayRef = Arc::new(LargeStringArray::from(vec![Some("big")]));
    let serie = from_array("big", large).unwrap();
    assert!(serie.data_type().is_large());

    let bin: ArrayRef = Arc::new(BinaryArray::from(vec![Some(&b"ab"[..]), None]));
    let serie = from_array("blob", bin).unwrap();
    assert_eq!(serie.data_type(), &DataType::binary());
    let typed = serie.as_any().downcast_ref::<BinarySerie<i32>>().unwrap();
    assert_eq!(typed.bytes_value(0), Some(&b"ab"[..]));
    assert_eq!(typed.get(1), None);
}

#[test]
fn temporal_series() {
    let dates: ArrayRef = Arc::new(Date32Array::from(vec![0, 1, 2]));
    let serie = from_array("d", dates).unwrap();
    assert_eq!(serie.data_type(), &DataType::date());
    assert_eq!(serie.category(), TypeCategory::Logical);
    let typed = serie.as_any().downcast_ref::<Date32Serie>().unwrap();
    assert_eq!(typed.value(2), 2);

    let ts: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![10, 20]));
    let serie = from_array("ts", ts).unwrap();
    assert!(serie.data_type().is_temporal());
    let typed = serie
        .as_any()
        .downcast_ref::<TimestampMicrosecondSerie>()
        .unwrap();
    assert_eq!(typed.get(0), Some(10));
}

#[test]
fn slice_preserves_type_and_name() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![10, 20, 30, 40]));
    let serie = from_array("n", array).unwrap();
    let sliced = serie.slice(1, 2);

    assert_eq!(sliced.len(), 2);
    assert_eq!(sliced.name(), "n");
    assert_eq!(sliced.data_type(), &DataType::int(32, true));
    let typed = sliced.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(typed.value(0), 20);
    assert_eq!(typed.value(1), 30);
}

#[test]
fn from_arrow_checks_field_type() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![1, 2]));
    // field says int64 but the array is int32
    let err = from_arrow(Field::new("x", DataType::int(64, true), true), array).unwrap_err();
    assert!(matches!(err, crate::SerieError::TypeMismatch { .. }));
}

#[test]
fn unsupported_arrow_type_errors() {
    let array: ArrayRef = Arc::new(NullArray::new(3));
    let err = from_array("n", array).unwrap_err();
    assert!(matches!(err, crate::SerieError::Unsupported(_)));
}

#[test]
fn index_range_is_lazy_uint64() {
    let index = IndexSerie::range(4);
    assert_eq!(index.len(), 4);
    assert_eq!(index.num_rows(), 4);
    assert!(index.is_range());
    assert!(!index.is_materialized()); // lazy range
    assert_eq!(index.name(), "index");
    assert_eq!(index.data_type(), &DataType::int(64, false));
    assert_eq!(index.category(), TypeCategory::Primitive);
    assert_eq!(index.null_count(), 0);

    assert_eq!(index.at(0), Some(0));
    assert_eq!(index.at(3), Some(3));
    assert_eq!(index.at(4), None); // out of bounds
    assert_eq!(index.position(2), Some(2));
    assert_eq!(index.position(9), None);
    assert!(index.contains(1));
    assert!(!index.contains(4));

    // materialise into an in-memory index that is still range-flagged
    let materialized = index.materialize();
    let mat = materialized.as_any().downcast_ref::<IndexSerie>().unwrap();
    assert!(mat.is_materialized());
    assert!(mat.is_range());
    assert_eq!(mat.at(2), Some(2));
}

#[test]
fn index_default_is_empty_lazy_uint64() {
    let index = IndexSerie::default();
    assert!(index.is_empty());
    assert!(index.is_range());
    assert!(!index.is_materialized());
    assert_eq!(index.data_type(), &DataType::int(64, false));
}

#[test]
fn index_from_serie_labels() {
    let labels: SerieRef = Arc::new(UInt64Serie::from_values(
        "k",
        vec![Some(10), Some(20), Some(30)],
    ));
    let index = IndexSerie::from_serie(labels);
    assert!(!index.is_range());
    assert_eq!(index.name(), "k");
    assert_eq!(index.at(0), Some(10));
    assert_eq!(index.position(20), Some(1));
    assert_eq!(index.position(99), None);
    assert!(index.contains(30));
}

#[test]
fn index_from_array_wraps_any_type() {
    let array: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "c"]));
    let index = IndexSerie::from_array("name", array).unwrap();
    assert!(!index.is_range());
    assert_eq!(index.len(), 3);
    assert_eq!(index.data_type(), &DataType::varchar());
    // a non-integer index has no integer-label lookup
    assert_eq!(index.at(0), None);
    assert_eq!(index.position(0), None);
}

#[test]
fn index_slice_stays_an_index() {
    let index = IndexSerie::range(5);
    let sliced = index.slice(1, 2);
    let view = sliced.as_any().downcast_ref::<IndexSerie>().unwrap();
    assert_eq!(view.len(), 2);
    assert!(!view.is_range()); // a slice no longer starts at 0
    assert_eq!(view.at(0), Some(1));
    assert_eq!(view.at(1), Some(2));
    assert_eq!(view.position(2), Some(1));
}

#[test]
fn index_is_usable_as_a_serie() {
    let index = IndexSerie::range(3);
    let column: SerieRef = Arc::new(index);
    assert_eq!(column.len(), 3);
    assert_eq!(column.null_count(), 0);
    // recover the IndexSerie through the base handle
    let recovered = column.as_any().downcast_ref::<IndexSerie>().unwrap();
    assert!(recovered.is_range());
}

#[test]
fn lazy_range_serie_computes_and_materializes() {
    let range = RangeSerie::new("r", 100, 5, 4); // 100, 105, 110, 115
    assert!(!range.is_materialized());
    assert_eq!(range.len(), 4);
    assert_eq!(range.get(0), Some(100));
    assert_eq!(range.get(3), Some(115));
    assert_eq!(range.get(4), None);
    assert_eq!(range.value_at(2), Scalar::Int(110));
    assert_eq!(
        range.to_vec(),
        vec![Some(100), Some(105), Some(110), Some(115)]
    );

    // materialising yields a real uint64 column with the same values
    let mat = range.materialize();
    assert!(mat.is_materialized());
    assert_eq!(mat.value_at(1), Scalar::Int(105));
    let ints = mat.as_any().downcast_ref::<UInt64Serie>().unwrap();
    assert_eq!(ints.value(3), 115);
}

#[test]
fn lazy_date_range_serie_computes_dates() {
    use yggdryl_core::Date;
    let start = Date::from_ymd(2024, 1, 30).unwrap();
    let range = DateRangeSerie::from_dates("d", start.clone(), 1, 3); // Jan 30, 31, Feb 1
    assert!(!range.is_materialized());
    assert_eq!(range.len(), 3);
    assert_eq!(range.data_type(), &DataType::date());
    assert_eq!(range.date_at(0), Some(start.clone()));
    assert_eq!(range.date_at(2), Some(Date::from_ymd(2024, 2, 1).unwrap()));
    assert_eq!(range.date_at(3), None);

    // value_at exposes the physical day-since-epoch
    assert_eq!(range.value_at(0), Scalar::Int(start.epoch_days() as i128));

    let mat = range.materialize();
    assert!(mat.is_materialized());
    assert_eq!(mat.data_type(), &DataType::date());
}

#[test]
fn value_at_and_slice_range() {
    let serie = from_array(
        "n",
        Arc::new(Int32Array::from(vec![Some(5), None, Some(7)])) as ArrayRef,
    )
    .unwrap();
    assert_eq!(serie.value_at(0), Scalar::Int(5));
    assert_eq!(serie.value_at(1), Scalar::Null); // null cell
    assert_eq!(serie.value_at(9), Scalar::Null); // out of bounds

    // zero-copy slice by range
    let window = serie.slice_range(1..3);
    assert_eq!(window.len(), 2);
    assert_eq!(window.value_at(0), Scalar::Null);
    assert_eq!(window.value_at(1), Scalar::Int(7));

    // strings and floats round-trip through Scalar
    let s = from_array("s", Arc::new(StringArray::from(vec!["hi"])) as ArrayRef).unwrap();
    assert_eq!(s.value_at(0).as_str(), Some("hi"));
    let f = from_array("f", Arc::new(Float64Array::from(vec![2.5])) as ArrayRef).unwrap();
    assert_eq!(f.value_at(0), Scalar::Float(2.5));
}

#[test]
fn child_records_parent_and_materialize_detaches() {
    let parent = from_array(
        "n",
        Arc::new(Int32Array::from(vec![10, 20, 30, 40])) as ArrayRef,
    )
    .unwrap();
    let view = child(&parent, 1, 2);

    assert_eq!(view.len(), 2);
    assert_eq!(view.value_at(0), Scalar::Int(20));
    assert_eq!(view.value_at(1), Scalar::Int(30));

    // the child remembers its parent
    let p = view.parent().expect("child has a parent");
    assert_eq!(p.len(), 4);
    assert_eq!(p.name(), "n");

    // child_range builds the same graph node
    let view2 = child_range(&parent, 0..2);
    assert_eq!(view2.value_at(1), Scalar::Int(20));
    assert!(view2.parent().is_some());

    // materialising detaches from the parent graph
    let independent = view.materialize();
    assert!(independent.parent().is_none());
    assert_eq!(independent.value_at(0), Scalar::Int(20));
}

#[test]
fn explicit_field_carries_metadata() {
    let array: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));
    let field = Field::new("id", DataType::int(32, true), false).with_comment("primary key");
    let serie = from_arrow(field, array).unwrap();
    assert!(!serie.is_nullable());
    assert_eq!(serie.field().comment(), Some("primary key"));
}

#[test]
fn empty_and_single_element_series() {
    let empty = Int64Serie::from_values("e", Vec::<Option<i64>>::new());
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_eq!(empty.value_at(0), Scalar::Null);
    assert_eq!(empty.iter().count(), 0);

    let one = Int64Serie::from_values("o", vec![Some(42i64)]);
    assert_eq!(one.len(), 1);
    assert_eq!(one.value(0), 42);
    assert_eq!(one.value_at(0), Scalar::Int(42));
    assert_eq!(one.value_at(1), Scalar::Null); // just past the end

    let empty_range = RangeSerie::new("r", 0, 1, 0);
    assert!(empty_range.is_empty());
    assert_eq!(empty_range.array().len(), 0); // must not panic
}

#[test]
fn slice_boundary_cases() {
    let s = from_array(
        "n",
        Arc::new(Int32Array::from(vec![1, 2, 3, 4])) as ArrayRef,
    )
    .unwrap();

    let zero = s.slice(0, 0);
    assert_eq!(zero.len(), 0);
    assert!(zero.is_empty());

    let tail = s.slice_range(2..4); // the last two
    assert_eq!(tail.len(), 2);
    assert_eq!(tail.value_at(0), Scalar::Int(3));
    assert_eq!(tail.value_at(1), Scalar::Int(4));
}

#[test]
fn range_serie_saturates_on_overflow() {
    // start near the top: at(1) would overflow, so it clamps instead of wrapping/panicking
    let r = RangeSerie::new("r", u64::MAX - 1, 10, 4);
    assert_eq!(r.value_at(0), Scalar::Int((u64::MAX - 1) as i128));
    assert_eq!(r.value_at(1), Scalar::Int(u64::MAX as i128)); // saturated
    assert_eq!(r.value_at(3), Scalar::Int(u64::MAX as i128)); // still clamped
    assert_eq!(r.array().len(), 4); // materialising must not panic
}

#[test]
fn date_range_negative_step_and_accessors() {
    use yggdryl_core::Date;
    let start = Date::from_ymd(2024, 3, 1).unwrap();
    let r = DateRangeSerie::from_dates("d", start.clone(), -1, 3); // Mar 1, Feb 29 (leap), Feb 28
    assert_eq!(r.date_at(0), Some(start.clone()));
    assert_eq!(r.date_at(1), Some(Date::from_ymd(2024, 2, 29).unwrap()));
    assert_eq!(r.date_at(2), Some(Date::from_ymd(2024, 2, 28).unwrap()));
    assert_eq!(r.start_days(), start.epoch_days());
    assert_eq!(r.step_days(), -1);
    assert_eq!(r.array().len(), 3); // must not panic
}

#[test]
fn scalar_variants_and_accessors() {
    let b = from_array(
        "b",
        Arc::new(BooleanArray::from(vec![Some(true), Some(false)])) as ArrayRef,
    )
    .unwrap();
    assert_eq!(b.value_at(0), Scalar::Boolean(true));
    assert_eq!(b.value_at(1), Scalar::Boolean(false));

    let bin = from_array(
        "bin",
        Arc::new(BinaryArray::from(vec![Some(&b"ab"[..])])) as ArrayRef,
    )
    .unwrap();
    assert_eq!(bin.value_at(0), Scalar::Binary(b"ab".to_vec()));

    assert_eq!(Scalar::Int(7).as_int(), Some(7));
    assert_eq!(Scalar::Float(1.5).as_float(), Some(1.5));
    assert!(Scalar::Null.is_null());
    assert_eq!(Scalar::Int(7).as_float(), None); // wrong-arm accessor returns None
    assert_eq!(Scalar::Float(1.0).as_str(), None);
}

#[test]
fn more_numeric_aliases_dispatch_and_read() {
    let i64s = from_array("a", Arc::new(Int64Array::from(vec![10i64])) as ArrayRef).unwrap();
    assert!(i64s.as_any().downcast_ref::<Int64Serie>().is_some());
    assert_eq!(i64s.value_at(0), Scalar::Int(10));

    let u32s = from_array("b", Arc::new(UInt32Array::from(vec![5u32])) as ArrayRef).unwrap();
    assert_eq!(u32s.data_type(), &DataType::int(32, false));
    assert_eq!(u32s.value_at(0), Scalar::Int(5));

    let f32s = from_array("c", Arc::new(Float32Array::from(vec![1.5f32])) as ArrayRef).unwrap();
    assert!(f32s.as_any().downcast_ref::<Float32Serie>().is_some());
    assert_eq!(f32s.value_at(0), Scalar::Float(1.5));

    let d64 = from_array(
        "d",
        Arc::new(Date64Array::from(vec![86_400_000i64])) as ArrayRef,
    )
    .unwrap();
    assert!(d64.as_any().downcast_ref::<Date64Serie>().is_some());
    assert_eq!(d64.value_at(0), Scalar::Int(86_400_000));

    let dur = from_array(
        "e",
        Arc::new(DurationSecondArray::from(vec![3i64])) as ArrayRef,
    )
    .unwrap();
    assert_eq!(dur.value_at(0), Scalar::Int(3));
}

#[test]
fn large_offset_string_and_binary() {
    let lu = from_array(
        "s",
        Arc::new(LargeStringArray::from(vec![Some("hi")])) as ArrayRef,
    )
    .unwrap();
    let v = lu.as_any().downcast_ref::<VarcharSerie<i64>>().unwrap();
    assert_eq!(v.str_value(0), Some("hi"));

    let lb = from_array(
        "b",
        Arc::new(LargeBinaryArray::from(vec![Some(&b"xy"[..])])) as ArrayRef,
    )
    .unwrap();
    let vb = lb.as_any().downcast_ref::<BinarySerie<i64>>().unwrap();
    assert_eq!(vb.bytes_value(0), Some(&b"xy"[..]));
}

#[test]
fn lazy_slice_stays_lazy_then_materializes() {
    let r = RangeSerie::new("r", 0, 2, 6); // 0, 2, 4, 6, 8, 10 (lazy)
    let sub = r.slice(1, 3); // 2, 4, 6 — still lazy
    assert!(!sub.is_materialized());
    assert_eq!(sub.len(), 3);
    assert_eq!(sub.value_at(0), Scalar::Int(2));
    assert_eq!(sub.value_at(2), Scalar::Int(6));

    let mat = sub.materialize();
    assert!(mat.is_materialized());
    assert_eq!(mat.value_at(2), Scalar::Int(6));
}

#[test]
fn nested_children_walk_up_the_graph() {
    let parent = from_array(
        "n",
        Arc::new(Int32Array::from(vec![0, 1, 2, 3, 4, 5])) as ArrayRef,
    )
    .unwrap();
    let c1 = child(&parent, 2, 4); // 2, 3, 4, 5
    let c2 = child(&c1, 1, 2); // 3, 4

    assert_eq!(c2.value_at(0), Scalar::Int(3));
    assert_eq!(c2.value_at(1), Scalar::Int(4));
    // c2's parent is c1 (length 4)
    assert_eq!(c2.parent().unwrap().len(), 4);
}

#[test]
fn metadata_survives_slice_and_materialize() {
    let field = Field::new("id", DataType::int(32, true), false).with_comment("pk");
    let serie = from_arrow(field, Arc::new(Int32Array::from(vec![1, 2, 3])) as ArrayRef).unwrap();

    let sliced = serie.slice(0, 2);
    assert_eq!(sliced.field().comment(), Some("pk"));

    let mat = sliced.materialize();
    assert_eq!(mat.field().comment(), Some("pk"));
}
