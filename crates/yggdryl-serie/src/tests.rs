//! Unit tests for the serie foundation: the [factory](crate::from_arrow), the
//! [`Serie`](crate::Serie) base accessors and the typed primitive series.

use std::sync::Arc;

use arrow_array::builder::{Int32Builder, MapBuilder, StringBuilder};
use arrow_array::types::Int32Type;
use arrow_array::{
    Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Date64Array, DurationSecondArray,
    Float32Array, Float64Array, Int32Array, Int64Array, LargeBinaryArray, LargeStringArray,
    ListArray, NullArray, StringArray, StructArray, Time32SecondArray, TimestampMicrosecondArray,
    TimestampMillisecondArray, TimestampNanosecondArray, TimestampSecondArray, UInt32Array,
};
use arrow_schema::{DataType as ADataType, Field as AField};
use yggdryl_schema::{DataType, Field, TypeCategory};

use crate::{
    child, child_range, from_array, from_arrow, BinarySerie, BooleanSerie, CategoricalSerie,
    Date32Serie, Date64Serie, DateRangeSerie, DateTimeRangeSerie, DatetimeSerie, DisplayOptions,
    DurationSerie, Float32Serie, Float64Serie, IndexSerie, Int32Serie, Int64Serie, ListSerie,
    MapSerie, NestedSerie, RangeSerie, Scalar, Serie, SerieRef, StructSerie, TemporalSerie,
    TimeRangeSerie, TimeSerie, TypedSerie, UInt64Serie, VarcharSerie,
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
    // every timestamp unit unifies into DatetimeSerie
    let typed = serie.as_any().downcast_ref::<DatetimeSerie>().unwrap();
    assert_eq!(typed.unit(), yggdryl_core::TimeUnit::Microsecond);
    assert_eq!(typed.physical_at(0), Some(10));
    assert_eq!(serie.value_at(0), Scalar::Int(10));
    // exposed as a core DateTime (10 microseconds past the epoch = 10_000 ns)
    let dt = typed.datetime_at(0).unwrap();
    assert_eq!(dt.epoch_nanos(), 10_000);
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

#[test]
fn convenience_field_accessors() {
    let field = Field::new("id", DataType::int(32, true), false).with_comment("pk");
    let serie = from_arrow(field, Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef).unwrap();

    assert_eq!(serie.name(), "id");
    assert_eq!(serie.dtype(), &DataType::int(32, true));
    assert_eq!(serie.dtype(), serie.data_type()); // dtype is the alias
    assert_eq!(serie.get_metadata("comment"), Some("pk"));
    assert_eq!(serie.get_metadata("missing"), None);
}

#[test]
fn datetime_serie_handles_units() {
    use yggdryl_core::TimeUnit;

    let secs = from_array(
        "ts",
        Arc::new(TimestampSecondArray::from(vec![1, 2])) as ArrayRef,
    )
    .unwrap();
    let dts = secs.as_any().downcast_ref::<DatetimeSerie>().unwrap();
    assert_eq!(dts.unit(), TimeUnit::Second);
    assert_eq!(dts.datetime_at(1).unwrap().epoch_seconds(), 2);

    let nanos = from_array(
        "ts",
        Arc::new(TimestampNanosecondArray::from(vec![1_500])) as ArrayRef,
    )
    .unwrap();
    let dtn = nanos.as_any().downcast_ref::<DatetimeSerie>().unwrap();
    assert_eq!(dtn.unit(), TimeUnit::Nanosecond);
    assert_eq!(dtn.datetime_at(0).unwrap().epoch_nanos(), 1_500);
}

#[test]
fn datetime_range_is_lazy_temporal() {
    use yggdryl_core::{DateTime, Duration};

    let start = DateTime::from_epoch_seconds(0, None);
    let step = Duration::from_secs(3600); // 1 hour
    let r = DateTimeRangeSerie::new("ts", &start, &step, 3);

    assert!(!r.is_materialized());
    assert_eq!(r.len(), 3);
    assert!(r.data_type().is_temporal());
    assert_eq!(r.datetime_at(0).unwrap().epoch_seconds(), 0);
    assert_eq!(r.datetime_at(2).unwrap().epoch_seconds(), 7200);

    let mat = r.materialize();
    assert!(mat.is_materialized());
    assert!(mat.as_any().downcast_ref::<DatetimeSerie>().is_some());

    let sub = r.slice(1, 2);
    assert!(!sub.is_materialized());
    let subt = sub.as_any().downcast_ref::<DateTimeRangeSerie>().unwrap();
    assert_eq!(subt.datetime_at(0).unwrap().epoch_seconds(), 3600);
}

#[test]
fn time_range_wraps_within_day_and_is_temporal() {
    use yggdryl_core::{Duration, Time};

    let start = Time::from_hms(23, 0, 0).unwrap();
    let step = Duration::from_secs(3600); // 1 hour
    let r = TimeRangeSerie::new("t", start, step, 3); // 23:00, 00:00, 01:00

    assert!(!r.is_materialized());
    assert_eq!(r.time(0).unwrap(), Time::from_hms(23, 0, 0).unwrap());
    assert_eq!(r.time(1).unwrap(), Time::from_hms(0, 0, 0).unwrap()); // wrapped
    assert_eq!(r.time(2).unwrap(), Time::from_hms(1, 0, 0).unwrap());
    assert_eq!(r.time_at(1).unwrap(), Time::from_hms(0, 0, 0).unwrap());
    assert_eq!(r.array().len(), 3); // materialising must not panic
}

#[test]
fn date_range_is_temporal() {
    use yggdryl_core::Date;

    let start = Date::from_ymd(2024, 1, 1).unwrap();
    let r = DateRangeSerie::from_dates("d", start.clone(), 1, 3);
    assert_eq!(r.date_at(0), Some(start.clone()));
    assert_eq!(r.datetime_at(0).unwrap().date(), start);
}

#[test]
fn unified_time_and_duration_series() {
    use yggdryl_core::{Duration, Time, TimeUnit};

    let t = from_array(
        "t",
        Arc::new(Time32SecondArray::from(vec![3600, 7200])) as ArrayRef,
    )
    .unwrap();
    assert_eq!(
        t.data_type(),
        &DataType::Time {
            unit: TimeUnit::Second
        }
    );
    let ts = t.as_any().downcast_ref::<TimeSerie>().unwrap();
    assert_eq!(ts.unit(), TimeUnit::Second);
    assert_eq!(ts.time_at(0), Some(Time::from_hms(1, 0, 0).unwrap()));
    assert_eq!(ts.get(1), Some(Time::from_hms(2, 0, 0).unwrap()));

    let d = from_array(
        "d",
        Arc::new(DurationSecondArray::from(vec![60, 120])) as ArrayRef,
    )
    .unwrap();
    assert_eq!(
        d.data_type(),
        &DataType::Duration {
            unit: TimeUnit::Second
        }
    );
    let ds = d.as_any().downcast_ref::<DurationSerie>().unwrap();
    assert_eq!(ds.duration_at(0), Some(Duration::from_secs(60)));
    assert_eq!(ds.value_at(1), Scalar::Int(120));
}

#[test]
fn display_renders_header_rows_and_truncation() {
    let serie = Int32Serie::from_values("n", vec![Some(1), None, Some(3)]);
    let text = serie.display(&DisplayOptions::default());
    assert!(text.contains("n: int32"));
    assert!(text.contains("null"));
    assert!(text.contains('1') && text.contains('3'));

    let big = Int32Serie::from_values("n", (0..20).map(Some));
    let limited = big.display(&DisplayOptions::default().with_max_rows(5));
    assert!(limited.contains("15 more rows"));

    // fixed width truncates with an ellipsis; no-header drops the title line
    let wide = VarcharSerie::<i32>::from_values("s", vec![Some("abcdef")]);
    let w = wide.display(&DisplayOptions::default().with_width(3).with_header(false));
    assert_eq!(w, "ab…");
}

#[test]
fn struct_serie_children_and_recursion() {
    // struct { id: int32, name: utf8 }
    let id = Arc::new(Int32Array::from(vec![1, 2, 3])) as ArrayRef;
    let name = Arc::new(StringArray::from(vec!["a", "b", "c"])) as ArrayRef;
    let sa = StructArray::from(vec![
        (Arc::new(AField::new("id", ADataType::Int32, false)), id),
        (Arc::new(AField::new("name", ADataType::Utf8, true)), name),
    ]);
    let serie = from_array("rec", Arc::new(sa) as ArrayRef).unwrap();
    assert_eq!(serie.category(), TypeCategory::Nested);

    let st = serie.as_any().downcast_ref::<StructSerie>().unwrap();
    assert_eq!(st.child_count(), 2);
    assert_eq!(st.child_by_name("id").unwrap().value_at(0), Scalar::Int(1));
    assert_eq!(st.child(1).unwrap().value_at(2), Scalar::Utf8("c".into()));
    // a record renders as {field=value, …}
    assert_eq!(st.value_at(0), Scalar::Other("{id=1, name=a}".into()));

    // recursion: a struct whose child is a list<int32>
    let inner = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
        Some(vec![Some(1), Some(2)]),
        Some(vec![Some(3)]),
    ]);
    let dt = inner.data_type().clone();
    let nested = StructArray::from(vec![(
        Arc::new(AField::new("tags", dt, true)),
        Arc::new(inner) as ArrayRef,
    )]);
    let serie = from_array("rec", Arc::new(nested) as ArrayRef).unwrap();
    let st = serie.as_any().downcast_ref::<StructSerie>().unwrap();
    let tags = st.child_by_name("tags").unwrap();
    let tags_list = tags.as_any().downcast_ref::<ListSerie<i32>>().unwrap();
    assert_eq!(tags_list.value_slice(0).unwrap().len(), 2);
}

#[test]
fn list_serie_slices_and_nulls() {
    let la = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
        Some(vec![Some(1), Some(2)]),
        Some(vec![Some(3)]),
        None,
    ]);
    let serie = from_array("l", Arc::new(la) as ArrayRef).unwrap();
    let ls = serie.as_any().downcast_ref::<ListSerie<i32>>().unwrap();

    assert_eq!(ls.len(), 3);
    assert_eq!(ls.values().len(), 3); // flattened 1,2,3
    let sub = ls.value_slice(0).unwrap();
    assert_eq!(sub.len(), 2);
    assert_eq!(sub.value_at(0), Scalar::Int(1));
    assert!(ls.value_slice(2).is_none()); // null row
    assert_eq!(ls.value_at(1), Scalar::Other("[3]".into()));
    assert_eq!(ls.value_at(2), Scalar::Null);
}

#[test]
fn map_serie_keys_values_and_render() {
    let mut b = MapBuilder::new(None, StringBuilder::new(), Int32Builder::new());
    b.keys().append_value("a");
    b.values().append_value(1);
    b.keys().append_value("b");
    b.values().append_value(2);
    b.append(true).unwrap(); // row 0: {a=1, b=2}
    b.keys().append_value("c");
    b.values().append_value(3);
    b.append(true).unwrap(); // row 1: {c=3}
    let ma = b.finish();

    let serie = from_array("m", Arc::new(ma) as ArrayRef).unwrap();
    let ms = serie.as_any().downcast_ref::<MapSerie>().unwrap();
    assert_eq!(ms.len(), 2);
    assert_eq!(ms.keys().len(), 3); // flattened a,b,c
    assert_eq!(
        ms.child_by_name("value").unwrap().value_at(0),
        Scalar::Int(1)
    );
    assert_eq!(ms.value_at(0), Scalar::Other("{a=1, b=2}".into()));
    assert_eq!(ms.value_at(1), Scalar::Other("{c=3}".into()));
}

#[test]
fn categorical_serie_encodes_repeats_and_materializes() {
    let values = VarcharSerie::<i32>::from_values(
        "c",
        vec![Some("a"), Some("b"), Some("a"), None, Some("c")],
    );
    let cat = CategoricalSerie::from_serie(&values).unwrap();

    // a, b, c are the distinct categories (null is not a category); it is a lazy view
    assert_eq!(cat.category_count(), 3);
    assert_eq!(cat.len(), 5);
    assert_eq!(cat.null_count(), 1);
    assert!(!cat.is_materialized());

    // per-row codes index into the dictionary; the null row has no code
    let a_code = cat.code_at(0).unwrap();
    assert_eq!(cat.code_at(2), Some(a_code)); // row 2 repeats "a"
    assert_eq!(cat.code_at(3), None); // null row

    // value access decodes through the dictionary
    assert_eq!(cat.value_at(0), Scalar::Utf8("a".into()));
    assert_eq!(cat.value_at(2), Scalar::Utf8("a".into()));
    assert_eq!(cat.value_at(3), Scalar::Null);
    assert_eq!(cat.category(a_code as usize), Scalar::Utf8("a".into()));

    // the distinct values are exposed as their own column
    assert_eq!(cat.categories().unwrap().len(), 3);

    // materialize decodes back to a real, flat column
    let flat = cat.materialize();
    assert!(flat.is_materialized());
    assert_eq!(flat.len(), 5);
    assert_eq!(flat.value_at(0), Scalar::Utf8("a".into()));
    assert_eq!(flat.value_at(3), Scalar::Null);
    assert_eq!(flat.data_type(), &DataType::varchar());
}

#[test]
fn datetime_serie_units_zone_and_get() {
    use yggdryl_core::TimeUnit;

    // millisecond unit
    let ms = from_array(
        "ts",
        Arc::new(TimestampMillisecondArray::from(vec![1_500])) as ArrayRef,
    )
    .unwrap();
    let d = ms.as_any().downcast_ref::<DatetimeSerie>().unwrap();
    assert_eq!(d.unit(), TimeUnit::Millisecond);
    assert_eq!(d.datetime_at(0).unwrap().epoch_nanos(), 1_500_000_000);
    assert_eq!(d.value_at(9), Scalar::Null); // out of bounds

    // zoned (timezone-aware) timestamp
    let zoned = TimestampMicrosecondArray::from(vec![0i64, 1_000_000]).with_timezone("UTC");
    let serie = from_array("ts", Arc::new(zoned) as ArrayRef).unwrap();
    let dz = serie.as_any().downcast_ref::<DatetimeSerie>().unwrap();
    assert!(dz.timezone().is_some());
    assert_eq!(dz.get(1).unwrap().epoch_seconds(), 1); // TypedSerie<DateTime>::get
}

#[test]
fn datetime_range_overflow_and_boundaries() {
    use yggdryl_core::{DateTime, Duration};

    // saturating: values clamp at the i64 nanosecond bound instead of wrapping
    let near_max = DateTime::from_epoch_nanos(i64::MAX as i128 - 5, None);
    let r = DateTimeRangeSerie::new("ts", &near_max, &Duration::from_nanos(100), 3);
    assert_eq!(r.array().len(), 3); // must not panic
    assert_eq!(r.value_at(2), Scalar::Int(i64::MAX as i128));

    // empty + single-element
    let zero = DateTimeRangeSerie::new("t", &near_max, &Duration::from_secs(1), 0);
    assert!(zero.is_empty());
    let one = DateTimeRangeSerie::new(
        "t",
        &DateTime::from_epoch_seconds(5, None),
        &Duration::from_secs(1),
        1,
    );
    assert_eq!(one.len(), 1);
    assert_eq!(one.datetime_at(0).unwrap().epoch_seconds(), 5);
}

#[test]
fn time_range_materialize_iter_and_empty() {
    use yggdryl_core::{Duration, Time};

    let r = TimeRangeSerie::new(
        "t",
        Time::from_hms(0, 0, 0).unwrap(),
        Duration::from_secs(3600),
        3,
    );
    let mat = r.materialize();
    assert!(mat.is_materialized());
    assert_eq!(mat.len(), 3);

    let times: Vec<_> = r.iter().collect();
    assert_eq!(times[1], Some(Time::from_hms(1, 0, 0).unwrap()));

    let empty = TimeRangeSerie::new(
        "t",
        Time::from_hms(0, 0, 0).unwrap(),
        Duration::from_secs(1),
        0,
    );
    assert!(empty.is_empty());
    assert_eq!(empty.array().len(), 0);
}

#[test]
fn date_range_materialize_timeat_and_tovec() {
    use yggdryl_core::Time;

    let r = DateRangeSerie::new("d", 100, 2, 3); // days 100, 102, 104
    let mat = r.materialize();
    assert!(mat.is_materialized());
    let darr = mat.as_any().downcast_ref::<Date32Serie>().unwrap();
    assert_eq!(darr.value(2), 104);

    assert_eq!(r.time_at(0), Some(Time::from_hms(0, 0, 0).unwrap())); // midnight
    assert_eq!(r.to_vec(), vec![Some(100), Some(102), Some(104)]);
}

#[test]
fn categorical_serie_integer_and_all_null() {
    let ints = Int32Serie::from_values("c", vec![Some(1), Some(2), Some(1), None]);
    let cat = CategoricalSerie::from_serie(&ints).unwrap();
    assert_eq!(cat.category_count(), 2);
    assert_eq!(cat.value_at(0), Scalar::Int(1));
    assert_eq!(cat.value_at(1), Scalar::Int(2));
    assert_eq!(cat.value_at(2), Scalar::Int(1));
    assert_eq!(cat.code_at(3), None); // null
    assert_eq!(cat.len(), 4);
    assert_eq!(cat.null_count(), 1);

    let nulls = Int32Serie::from_values("c", vec![Option::<i32>::None, None]);
    let cat2 = CategoricalSerie::from_serie(&nulls).unwrap();
    assert_eq!(cat2.category_count(), 0); // null is not a category
    assert_eq!(cat2.len(), 2);
    assert_eq!(cat2.null_count(), 2);
}

#[test]
fn lazy_temporal_field_accessors() {
    let index = IndexSerie::range(3);
    assert_eq!(index.dtype(), &DataType::int(64, false));
    assert_eq!(index.get_metadata("missing"), None);
}

#[test]
fn scalar_default_for_each_kind() {
    assert_eq!(
        Scalar::default_for(&DataType::int(32, true)),
        Scalar::Int(0)
    );
    assert_eq!(
        Scalar::default_for(&DataType::float(64)),
        Scalar::Float(0.0)
    );
    assert_eq!(
        Scalar::default_for(&DataType::Boolean),
        Scalar::Boolean(false)
    );
    assert_eq!(
        Scalar::default_for(&DataType::varchar()),
        Scalar::Utf8(String::new())
    );
    assert_eq!(
        Scalar::default_for(&DataType::binary()),
        Scalar::Binary(Vec::new())
    );
    assert_eq!(Scalar::default_for(&DataType::date()), Scalar::Int(0));
    assert_eq!(Scalar::default_for(&DataType::Null), Scalar::Null);
}

#[test]
fn resize_grows_with_nulls_or_defaults_and_shrinks() {
    // nullable column grows with nulls
    let s = Int32Serie::from_values("n", vec![Some(1), Some(2)]);
    let grown = s.resize(4).unwrap();
    assert_eq!(grown.len(), 4);
    assert_eq!(grown.value_at(1), Scalar::Int(2));
    assert_eq!(grown.value_at(2), Scalar::Null);
    assert_eq!(grown.value_at(3), Scalar::Null);

    // shrink is a slice
    let shrunk = s.resize(1).unwrap();
    assert_eq!(shrunk.len(), 1);
    assert_eq!(shrunk.value_at(0), Scalar::Int(1));

    // non-nullable column grows with the type default (0)
    let field = Field::new("n", DataType::int(32, true), false);
    let nn = from_arrow(field, Arc::new(Int32Array::from(vec![7])) as ArrayRef).unwrap();
    let gd = nn.resize(3).unwrap();
    assert_eq!(gd.value_at(0), Scalar::Int(7));
    assert_eq!(gd.value_at(1), Scalar::Int(0));
    assert_eq!(gd.value_at(2), Scalar::Int(0));

    // non-nullable varchar grows with the empty string
    let vfield = Field::new("s", DataType::varchar(), false);
    let vs = from_arrow(vfield, Arc::new(StringArray::from(vec!["a"])) as ArrayRef).unwrap();
    let vg = vs.resize(2).unwrap();
    assert_eq!(vg.value_at(0), Scalar::Utf8("a".into()));
    assert_eq!(vg.value_at(1), Scalar::Utf8(String::new()));
}

#[test]
fn resize_non_nullable_struct_fills_nested_defaults() {
    // non-nullable struct rec { id: int32, name: utf8 } (both children non-nullable)
    let id = Arc::new(Int32Array::from(vec![7])) as ArrayRef;
    let name = Arc::new(StringArray::from(vec!["a"])) as ArrayRef;
    let array = StructArray::from(vec![
        (Arc::new(AField::new("id", ADataType::Int32, false)), id),
        (Arc::new(AField::new("name", ADataType::Utf8, false)), name),
    ]);
    let field = Field::new(
        "rec",
        DataType::struct_(vec![
            Field::new("id", DataType::int(32, true), false),
            Field::new("name", DataType::varchar(), false),
        ]),
        false,
    );
    let s = from_arrow(field, Arc::new(array) as ArrayRef).unwrap();
    let grown = s.resize(3).unwrap();
    assert_eq!(grown.len(), 3);
    assert_eq!(grown.value_at(0), Scalar::Other("{id=7, name=a}".into()));
    // fill rows are the record of child defaults (0 / empty string), not nulls
    assert_eq!(grown.value_at(1), Scalar::Other("{id=0, name=}".into()));
    assert_eq!(grown.value_at(2), Scalar::Other("{id=0, name=}".into()));
}

#[test]
fn default_array_covers_all_interval_units() {
    use arrow_schema::IntervalUnit as AIntervalUnit;
    // every interval unit a column can dispatch to must also produce a non-null default
    // (so a non-nullable interval column is resizable, not just YearMonth).
    for unit in [
        AIntervalUnit::YearMonth,
        AIntervalUnit::DayTime,
        AIntervalUnit::MonthDayNano,
    ] {
        let arr = crate::build::default_array(&ADataType::Interval(unit), 3).unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr.null_count(), 0);
    }
}

#[test]
fn one_line_temporal_and_struct_constructors() {
    use yggdryl_core::{DateTime, Duration, Time};

    let dt =
        DatetimeSerie::from_values("t", vec![Some(DateTime::from_epoch_seconds(1, None)), None]);
    assert_eq!(dt.len(), 2);
    assert_eq!(dt.datetime_at(0).unwrap().epoch_seconds(), 1);
    assert!(dt.is_null(1));

    let tm = TimeSerie::from_values("t", vec![Some(Time::from_hms(1, 0, 0).unwrap())]);
    assert_eq!(tm.time_at(0), Some(Time::from_hms(1, 0, 0).unwrap()));

    let du = DurationSerie::from_values("d", vec![Some(Duration::from_secs(5))]);
    assert_eq!(du.duration_at(0), Some(Duration::from_secs(5)));

    // struct from child columns, in one line
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values(
        "name",
        vec![Some("a"), Some("b")],
    ));
    let st = StructSerie::from_children("rec", vec![id, name]).unwrap();
    assert_eq!(st.child_count(), 2);
    assert_eq!(
        st.child_by_name("name").unwrap().value_at(1),
        Scalar::Utf8("b".into())
    );
    assert_eq!(st.value_at(0), Scalar::Other("{id=1, name=a}".into()));

    // mismatched child lengths are rejected with an actionable message
    let two: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let three: SerieRef = Arc::new(VarcharSerie::<i32>::from_values(
        "name",
        vec![Some("a"), Some("b"), Some("c")],
    ));
    let bad = StructSerie::from_children("rec", vec![two, three]);
    assert!(matches!(bad, Err(crate::SerieError::Arrow(msg)) if msg.contains("equal length")));
}

#[test]
fn nested_child_access_by_index_name_and_path() {
    // struct rec { inner: struct { a: int32, b: utf8 }, N: int64 }
    let a = Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef;
    let b = Arc::new(StringArray::from(vec!["x", "y"])) as ArrayRef;
    let inner = StructArray::from(vec![
        (Arc::new(AField::new("a", ADataType::Int32, true)), a),
        (Arc::new(AField::new("b", ADataType::Utf8, true)), b),
    ]);
    let n = Arc::new(Int64Array::from(vec![10i64, 20])) as ArrayRef;
    let outer = StructArray::from(vec![
        (
            Arc::new(AField::new("inner", inner.data_type().clone(), true)),
            Arc::new(inner) as ArrayRef,
        ),
        (Arc::new(AField::new("N", ADataType::Int64, true)), n),
    ]);
    let serie = from_array("rec", Arc::new(outer) as ArrayRef).unwrap();
    let st = serie.as_nested().unwrap();

    // by index
    assert_eq!(st.child(0).unwrap().name(), "inner");
    assert_eq!(st.children().len(), 2);

    // by name: exact (cs) vs case-insensitive fallback
    assert!(st.child_named("n").is_none()); // exact, case-sensitive
    assert_eq!(st.child_named("N").unwrap().name(), "N");
    assert_eq!(st.child_by_name("n").unwrap().name(), "N"); // cs miss -> ci

    // by node path
    assert_eq!(
        serie.select("inner.a").unwrap().unwrap().value_at(0),
        Scalar::Int(1)
    );
    assert_eq!(
        serie.select("inner.b").unwrap().unwrap().value_at(1),
        Scalar::Utf8("y".into())
    );
    assert_eq!(serie.select("n").unwrap().unwrap().name(), "N"); // ci segment
    assert_eq!(
        serie.select("[inner].a").unwrap().unwrap().value_at(0),
        Scalar::Int(1)
    ); // bracket-wrapped exact
    assert_eq!(
        serie.select(r#""inner".a"#).unwrap().unwrap().value_at(0),
        Scalar::Int(1)
    ); // quote-wrapped exact
    assert!(serie.select("inner.zzz").unwrap().is_none()); // well-formed but missing

    // navigating *through* a non-nested intermediate ('a' is int32) yields Ok(None)
    assert!(serie.select("inner.a.something").unwrap().is_none());

    // a malformed path is an Err (parsing validates), not a silent miss
    assert!(serie.select("inner.").is_err()); // trailing dot
    assert!(serie.select("[inner.a").is_err()); // unclosed wrapper

    // a leaf column is not navigable
    let leaf = from_array("x", Arc::new(Int32Array::from(vec![1])) as ArrayRef).unwrap();
    assert!(leaf.as_nested().is_none());
    assert!(leaf.select("a.b").unwrap().is_none());
}

#[test]
fn cast_primitive_and_struct_with_fill() {
    // widening primitive cast (int32 -> int64) keeps the column's name
    let s = Int32Serie::from_values("n", vec![Some(1), Some(2), None]);
    let wide = s.cast(&DataType::int(64, true)).unwrap();
    assert_eq!(wide.data_type(), &DataType::int(64, true));
    assert_eq!(wide.name(), "n");
    assert_eq!(wide.value_at(0), Scalar::Int(1));
    assert_eq!(wide.value_at(2), Scalar::Null);

    // lossy / narrowing cast (int32 -> int8) yields null on overflow
    let big = Int32Serie::from_values("n", vec![Some(1000), Some(5)]);
    let narrow = big.cast(&DataType::int(8, true)).unwrap();
    assert_eq!(narrow.value_at(0), Scalar::Null); // 1000 overflows i8
    assert_eq!(narrow.value_at(1), Scalar::Int(5));

    // struct -> struct cast: child 'id' is widened, missing 'extra' is filled with nulls
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let src = StructSerie::from_children("rec", vec![id]).unwrap();
    let target = DataType::struct_(vec![
        Field::new("id", DataType::int(64, true), true),
        Field::new("extra", DataType::varchar(), true),
    ]);
    let casted = src.cast(&target).unwrap();
    assert_eq!(casted.data_type(), &target);
    assert_eq!(
        casted.select("id").unwrap().unwrap().value_at(0),
        Scalar::Int(1)
    );
    assert!(casted.select("extra").unwrap().unwrap().is_null(0)); // filled column is null
}

#[test]
fn struct_from_children_is_lazy_until_materialized() {
    // a lazy child (a computed range) keeps the struct lazy until materialize
    let id: SerieRef = Arc::new(RangeSerie::new("id", 0, 1, 3));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values(
        "name",
        vec![Some("a"), Some("b"), Some("c")],
    ));
    let st = StructSerie::from_children("rec", vec![id, name]).unwrap();
    assert!(!st.is_materialized()); // no backing StructArray yet
    assert_eq!(st.len(), 3);
    assert_eq!(st.value_at(0), Scalar::Other("{id=0, name=a}".into()));

    // array() assembles on demand; materialize() realises the children and caches it
    assert_eq!(st.array().len(), 3);
    let mat = st.materialize();
    assert!(mat.is_materialized());
    assert_eq!(mat.len(), 3);
    assert_eq!(mat.value_at(2), Scalar::Other("{id=2, name=c}".into()));
}
