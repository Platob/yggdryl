//! Comprehensive Arrow interop edge cases for the temporal columnar family (feature `arrow`):
//! the `TemporalSerie<B>` ↔ Arrow `Date*` / `Time*` / `Timestamp` / `Duration` / `FixedSizeBinary`
//! round-trips across every unit + timezone, the widen/narrow `ts32`/`duration32` path, the wide
//! `ts96` `FixedSizeBinary(12)` form, garbage-under-null and sliced-import canonicalization, the
//! **zero-copy** export *and* (post-optimization) import Arc-sharing, the calendar-unit export
//! error, the logical-type / unit / timezone field metadata, and a temporal column carried as a
//! struct child through a `RecordBatch`.
#![cfg(feature = "arrow")]

use arrow_array::types::{Date32Type, DurationSecondType, TimestampSecondType};
use arrow_array::{Array, FixedSizeBinaryArray, PrimitiveArray};
use arrow_buffer::{Buffer as ArrowBuffer, NullBuffer, ScalarBuffer};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, TimeUnit as ArrowTimeUnit};

use yggdryl_core::io::boxed;
use yggdryl_core::io::fixed::temporal::{
    Date32, Date64, Duration32, Duration64, Time32, Time64, TimeUnit, Ts32, Ts64, Ts96, Tz,
};
use yggdryl_core::io::fixed::{
    Date32Field, Date32Serie, Date64Field, Date64Serie, Duration32Field, Duration32Serie,
    Duration64Field, Duration64Serie, Field, Serie, Time32Field, Time32Serie, Time64Field,
    Time64Serie, Ts32Field, Ts32Serie, Ts64Field, Ts64Serie, Ts96Field, Ts96Serie,
};
use yggdryl_core::io::nested::StructSerie;
use yggdryl_core::io::{DataTypeId, FieldType};

// -------------------------------------------------------------------------------------
// 1. All 9 backings: value -> column (with a null) -> serialize -> deserialize == original.
// -------------------------------------------------------------------------------------

#[test]
fn all_nine_backings_serialize_round_trip() {
    macro_rules! round_trip {
        ($Serie:ty, $unit:expr, $tz:expr, $value:expr) => {{
            let col =
                <$Serie>::from_options($unit, $tz, &[Some($value), None, Some($value)]).unwrap();
            assert_eq!(col.len(), 3);
            assert_eq!(col.null_count(), 1);
            let back = <$Serie>::deserialize_bytes(&col.serialize_bytes()).unwrap();
            assert_eq!(back, col);
        }};
    }

    round_trip!(
        Date32Serie,
        TimeUnit::Day,
        Tz::NAIVE,
        Date32::from_days(19_000)
    );
    round_trip!(
        Date64Serie,
        TimeUnit::Millisecond,
        Tz::NAIVE,
        Date64::from_millis(1_700_000_000_000)
    );
    round_trip!(
        Time32Serie,
        TimeUnit::Second,
        Tz::NAIVE,
        Time32::new(3_661, TimeUnit::Second).unwrap()
    );
    round_trip!(
        Time64Serie,
        TimeUnit::Microsecond,
        Tz::NAIVE,
        Time64::new(3_661_000_000, TimeUnit::Microsecond).unwrap()
    );
    round_trip!(
        Ts32Serie,
        TimeUnit::Second,
        Tz::UTC,
        Ts32::from_epoch(1_000_000, TimeUnit::Second, Tz::UTC).unwrap()
    );
    round_trip!(
        Ts64Serie,
        TimeUnit::Nanosecond,
        Tz::UTC,
        Ts64::from_epoch(1_700_000_000_000_000_000, TimeUnit::Nanosecond, Tz::UTC).unwrap()
    );
    round_trip!(
        Ts96Serie,
        TimeUnit::Nanosecond,
        Tz::UTC,
        Ts96::from_epoch(1_700_000_000_000_000_000, TimeUnit::Nanosecond, Tz::UTC).unwrap()
    );
    round_trip!(
        Duration32Serie,
        TimeUnit::Millisecond,
        Tz::NAIVE,
        Duration32::milliseconds(1_500)
    );
    round_trip!(
        Duration64Serie,
        TimeUnit::Millisecond,
        Tz::NAIVE,
        Duration64::milliseconds(1_500)
    );
}

// -------------------------------------------------------------------------------------
// 2. to_arrow_array -> from_arrow_array for EVERY unit variant.
// -------------------------------------------------------------------------------------

fn assert_arrow_round_trip<B>(col: yggdryl_core::io::fixed::temporal::TemporalSerie<B>)
where
    B: yggdryl_core::io::fixed::temporal::TemporalBacking,
{
    let array = col.to_arrow_array().unwrap();
    let field = col.to_field("t").to_arrow();
    let back = yggdryl_core::io::fixed::temporal::TemporalSerie::<B>::from_arrow_array(
        array.as_ref(),
        &field,
    )
    .unwrap();
    assert_eq!(back, col);
}

#[test]
fn every_unit_variant_round_trips_through_arrow() {
    // Time32: Second, Millisecond.
    for unit in [TimeUnit::Second, TimeUnit::Millisecond] {
        let v = Time32::new(3_600, unit).unwrap();
        assert_arrow_round_trip(
            Time32Serie::from_options(unit, Tz::NAIVE, &[Some(v), None]).unwrap(),
        );
    }
    // Time64: Microsecond, Nanosecond.
    for unit in [TimeUnit::Microsecond, TimeUnit::Nanosecond] {
        let v = Time64::new(3_600_000_000, unit).unwrap();
        assert_arrow_round_trip(
            Time64Serie::from_options(unit, Tz::NAIVE, &[Some(v), None]).unwrap(),
        );
    }
    // Timestamp (Ts64): Second, Millisecond, Microsecond, Nanosecond.
    for unit in [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        let v = Ts64::from_epoch(1_700_000_000, unit, Tz::UTC).unwrap();
        assert_arrow_round_trip(Ts64Serie::from_options(unit, Tz::UTC, &[Some(v), None]).unwrap());
    }
    // Duration (Duration64): Second, Millisecond, Microsecond, Nanosecond.
    for unit in [
        TimeUnit::Second,
        TimeUnit::Millisecond,
        TimeUnit::Microsecond,
        TimeUnit::Nanosecond,
    ] {
        let v = Duration64::new(1_234, unit).unwrap();
        assert_arrow_round_trip(
            Duration64Serie::from_options(unit, Tz::NAIVE, &[Some(v), None]).unwrap(),
        );
    }
}

// -------------------------------------------------------------------------------------
// 3. Timezone round-trip through Ts64 Timestamp(unit, Some(tz)).
// -------------------------------------------------------------------------------------

#[test]
fn timezone_round_trip_through_timestamp() {
    for zone in ["", "UTC", "+02:00", "-05:30", "Europe/Paris"] {
        let tz = Tz::parse(zone).expect(zone);
        let v = Ts64::from_epoch(1_700_000_000, TimeUnit::Second, tz).unwrap();
        let col = Ts64Serie::from_options(TimeUnit::Second, tz, &[Some(v), None]).unwrap();
        let array = col.to_arrow_array().unwrap();
        let field = col.to_field("t").to_arrow();
        let back = Ts64Serie::from_arrow_array(array.as_ref(), &field).unwrap();
        assert_eq!(back.timezone(), col.timezone(), "zone {zone}");
        assert_eq!(back, col, "zone {zone}");
    }
}

// -------------------------------------------------------------------------------------
// 4. Ts32 / Duration32 widen -> i64 -> narrow, and the overflow-on-import error.
// -------------------------------------------------------------------------------------

#[test]
fn ts32_and_duration32_widen_narrow_extremes() {
    // Ts32 at i32::MIN / i32::MAX epoch seconds.
    let hi = Ts32::from_epoch(i32::MAX as i128, TimeUnit::Second, Tz::UTC).unwrap();
    let lo = Ts32::from_epoch(i32::MIN as i128, TimeUnit::Second, Tz::UTC).unwrap();
    let col =
        Ts32Serie::from_options(TimeUnit::Second, Tz::UTC, &[Some(hi), Some(lo), None]).unwrap();
    let array = col.to_arrow_array().unwrap();
    assert!(matches!(array.data_type(), ArrowDataType::Timestamp(_, _)));
    let field = col.to_field("t").to_arrow();
    assert_eq!(
        Ts32Serie::from_arrow_array(array.as_ref(), &field).unwrap(),
        col
    );

    // Duration32 at i32::MIN / i32::MAX span values.
    let dhi = Duration32::new(i32::MAX, TimeUnit::Millisecond).unwrap();
    let dlo = Duration32::new(i32::MIN, TimeUnit::Millisecond).unwrap();
    let dcol =
        Duration32Serie::from_options(TimeUnit::Millisecond, Tz::NAIVE, &[Some(dhi), Some(dlo)])
            .unwrap();
    let darray = dcol.to_arrow_array().unwrap();
    assert!(matches!(darray.data_type(), ArrowDataType::Duration(_)));
    let dfield = dcol.to_field("d").to_arrow();
    assert_eq!(
        Duration32Serie::from_arrow_array(darray.as_ref(), &dfield).unwrap(),
        dcol
    );
}

#[test]
fn foreign_i64_exceeding_i32_errors_on_narrow_import() {
    // A foreign Arrow i64 Timestamp whose value exceeds i32 cannot narrow into a ts32 column.
    let over = i32::MAX as i64 + 1;
    let array = PrimitiveArray::<TimestampSecondType>::from(vec![over]);
    let field = ArrowField::new(
        "t",
        ArrowDataType::Timestamp(ArrowTimeUnit::Second, None),
        false,
    );
    let err = Ts32Serie::from_arrow_array(&array, &field).unwrap_err();
    assert!(
        matches!(err, yggdryl_core::io::IoError::Unsupported { .. }),
        "{err:?}"
    );

    // Same for a foreign i64 Duration into a duration32 column.
    let darray = PrimitiveArray::<DurationSecondType>::from(vec![over]);
    let dfield = ArrowField::new("d", ArrowDataType::Duration(ArrowTimeUnit::Second), false);
    assert!(Duration32Serie::from_arrow_array(&darray, &dfield).is_err());
}

// -------------------------------------------------------------------------------------
// 5. Ts96 extremes through FixedSizeBinary(12), + foreign FSB with metadata imports as Ts96.
// -------------------------------------------------------------------------------------

fn ts96_le_bytes(v: i128) -> [u8; 12] {
    let mut bytes = [0u8; 12];
    bytes.copy_from_slice(&v.to_le_bytes()[..12]);
    bytes
}

#[test]
fn ts96_extremes_round_trip_through_fixed_size_binary() {
    let hi = Ts96::from_epoch((1i128 << 95) - 1, TimeUnit::Nanosecond, Tz::UTC).unwrap();
    let lo = Ts96::from_epoch(-(1i128 << 95), TimeUnit::Nanosecond, Tz::UTC).unwrap();
    let col = Ts96Serie::from_options(TimeUnit::Nanosecond, Tz::UTC, &[Some(hi), Some(lo), None])
        .unwrap();
    let array = col.to_arrow_array().unwrap();
    assert_eq!(array.data_type(), &ArrowDataType::FixedSizeBinary(12));
    let field = col.to_field("t").to_arrow();
    assert_eq!(
        Ts96Serie::from_arrow_array(array.as_ref(), &field).unwrap(),
        col
    );
}

#[test]
fn foreign_fixed_size_binary_with_metadata_imports_as_ts96() {
    // A genuinely foreign FixedSizeBinary(12) array (not built by our column) plus a field carrying
    // the yggdryl.logical_type=ts96 + unit/timezone tags imports as a Ts96 column, not FixedBinary.
    let v = Ts96::from_epoch(42_000_000_000, TimeUnit::Nanosecond, Tz::UTC).unwrap();
    let mut raw = Vec::new();
    raw.extend_from_slice(&ts96_le_bytes(v.epoch_value()));
    raw.extend_from_slice(&ts96_le_bytes(v.epoch_value()));
    let array = FixedSizeBinaryArray::new(12, ArrowBuffer::from_vec(raw), None);

    // The field is what a Ts96 field serializes to (the ts96 tag + unit/timezone metadata).
    let field = Ts96Field::new("t", TimeUnit::Nanosecond, Tz::UTC, false).to_arrow();
    assert_eq!(
        field.metadata().get("yggdryl.logical_type"),
        Some(&"ts96".to_string())
    );
    // The erased field recovers the exact logical type Ts96 (not the FixedBinary default).
    assert_eq!(
        FieldType::type_id(&Field::from_arrow(&field).unwrap()),
        DataTypeId::Ts96
    );

    let col = Ts96Serie::from_arrow_array(&array, &field).unwrap();
    assert_eq!(col.len(), 2);
    assert_eq!(col.timezone(), Tz::UTC);
    assert_eq!(col.unit(), TimeUnit::Nanosecond);
    assert_eq!(
        col,
        Ts96Serie::from_values(TimeUnit::Nanosecond, Tz::UTC, &[v, v]).unwrap()
    );
}

// -------------------------------------------------------------------------------------
// 6. Garbage under a null slot is canonicalized to zero on import.
// -------------------------------------------------------------------------------------

#[test]
fn garbage_under_null_is_canonicalized() {
    // A foreign Date32 array with a non-zero value UNDER a null slot (Arrow leaves those bytes
    // undefined; IPC/Parquet arrays carry garbage there).
    let values = ScalarBuffer::<i32>::from(vec![10, 999, 30]);
    let nulls = NullBuffer::from(vec![true, false, true]);
    let garbage = PrimitiveArray::<Date32Type>::new(values, Some(nulls));
    let field = ArrowField::new("d", ArrowDataType::Date32, true);

    let imported = Date32Serie::from_arrow_array(&garbage, &field).unwrap();
    assert_eq!(imported.get(1), None);
    // The undefined garbage under the null must not leak into identity: equal to a clean build.
    let clean = Date32Serie::from_options(
        TimeUnit::Day,
        Tz::NAIVE,
        &[
            Some(Date32::from_days(10)),
            None,
            Some(Date32::from_days(30)),
        ],
    )
    .unwrap();
    assert_eq!(imported, clean);
    assert_eq!(imported.serialize_bytes(), clean.serialize_bytes());
}

// -------------------------------------------------------------------------------------
// 7. A sliced / offset import reads its logical window.
// -------------------------------------------------------------------------------------

#[test]
fn sliced_import_reads_the_logical_window() {
    let a = Ts64::from_epoch(1, TimeUnit::Second, Tz::UTC).unwrap();
    let b = Ts64::from_epoch(2, TimeUnit::Second, Tz::UTC).unwrap();
    let c = Ts64::from_epoch(3, TimeUnit::Second, Tz::UTC).unwrap();
    let d = Ts64::from_epoch(4, TimeUnit::Second, Tz::UTC).unwrap();
    let col = Ts64Serie::from_options(
        TimeUnit::Second,
        Tz::UTC,
        &[Some(a), None, Some(c), Some(d)],
    )
    .unwrap();
    let _ = b;
    let array = col.to_arrow_array().unwrap();
    let sliced = array.slice(1, 3); // logical window: [None, Some(c), Some(d)]
    let field = col.to_field("t").to_arrow();
    let imported = Ts64Serie::from_arrow_array(sliced.as_ref(), &field).unwrap();

    let expected =
        Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[None, Some(c), Some(d)]).unwrap();
    assert_eq!(imported, expected);
}

// -------------------------------------------------------------------------------------
// 8. Zero-copy: export shares the column's buffer, and a dense import Arc-shares it back.
// -------------------------------------------------------------------------------------

#[test]
fn native_export_and_dense_import_are_zero_copy() {
    let values: Vec<Ts64> = (0..256)
        .map(|i| Ts64::from_epoch(i, TimeUnit::Second, Tz::UTC).unwrap())
        .collect();
    let col = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &values).unwrap();
    let col_ptr = col.count_bytes().as_ptr();

    // Export: the exported array's values buffer IS the column's counts (an Arc bump).
    let array = col.to_arrow_array().unwrap();
    let exported_ptr = array.to_data().buffers()[0].as_ptr();
    assert_eq!(
        exported_ptr, col_ptr,
        "native export must share the column's buffer"
    );

    // Import (post-optimization): a dense/offset-0 array Arc-shares rather than copying, so the
    // reimported column keeps the very same allocation.
    let field = col.to_field("t").to_arrow();
    let back = Ts64Serie::from_arrow_array(array.as_ref(), &field).unwrap();
    assert_eq!(
        back.count_bytes().as_ptr(),
        col_ptr,
        "dense import must Arc-share the payload"
    );
    assert_eq!(back, col);
}

// -------------------------------------------------------------------------------------
// 9. A fixed-but-non-Arrow unit errors on export.
// -------------------------------------------------------------------------------------

#[test]
fn non_arrow_unit_errors_on_export() {
    let a = Ts64::from_epoch(5, TimeUnit::Minute, Tz::UTC).unwrap();
    let col = Ts64Serie::from_values(TimeUnit::Minute, Tz::UTC, &[a]).unwrap();
    let err = col.to_arrow_array().unwrap_err();
    assert!(
        matches!(err, yggdryl_core::io::IoError::Unsupported { .. }),
        "{err:?}"
    );
}

// -------------------------------------------------------------------------------------
// 10. Metadata: erase writes unit/timezone; the logical_type tag is present only for the
//     ambiguous narrow forms; the shadow keys are stripped on from_arrow.
// -------------------------------------------------------------------------------------

#[test]
fn erase_writes_unit_and_timezone_metadata() {
    let field = Ts64Field::new("t", TimeUnit::Microsecond, Tz::UTC, true).erase();
    assert_eq!(field.metadata().get("unit"), Some("microsecond"));
    assert_eq!(field.metadata().get("timezone"), Some("UTC"));
}

#[test]
fn logical_type_tag_only_for_ambiguous_narrow_forms() {
    // Tagged: ts32 / ts96 / duration32 (their plain Arrow mapping is not reversible to them).
    assert_eq!(
        Ts32Field::new("t", TimeUnit::Second, Tz::UTC, false)
            .to_arrow()
            .metadata()
            .get("yggdryl.logical_type"),
        Some(&"ts32".to_string())
    );
    assert_eq!(
        Ts96Field::new("t", TimeUnit::Nanosecond, Tz::UTC, false)
            .to_arrow()
            .metadata()
            .get("yggdryl.logical_type"),
        Some(&"ts96".to_string())
    );
    assert_eq!(
        Duration32Field::new("d", TimeUnit::Millisecond, Tz::NAIVE, false)
            .to_arrow()
            .metadata()
            .get("yggdryl.logical_type"),
        Some(&"duration32".to_string())
    );

    // Untagged: Date32/Date64/Time32/Time64/Ts64/Duration64 map reversibly, so no tag.
    assert!(Date32Field::new("d", TimeUnit::Day, Tz::NAIVE, false)
        .to_arrow()
        .metadata()
        .get("yggdryl.logical_type")
        .is_none());
    assert!(
        Date64Field::new("d", TimeUnit::Millisecond, Tz::NAIVE, false)
            .to_arrow()
            .metadata()
            .get("yggdryl.logical_type")
            .is_none()
    );
    assert!(Time32Field::new("t", TimeUnit::Second, Tz::NAIVE, false)
        .to_arrow()
        .metadata()
        .get("yggdryl.logical_type")
        .is_none());
    assert!(
        Time64Field::new("t", TimeUnit::Microsecond, Tz::NAIVE, false)
            .to_arrow()
            .metadata()
            .get("yggdryl.logical_type")
            .is_none()
    );
    assert!(Ts64Field::new("t", TimeUnit::Nanosecond, Tz::UTC, false)
        .to_arrow()
        .metadata()
        .get("yggdryl.logical_type")
        .is_none());
    assert!(
        Duration64Field::new("d", TimeUnit::Nanosecond, Tz::NAIVE, false)
            .to_arrow()
            .metadata()
            .get("yggdryl.logical_type")
            .is_none()
    );
}

#[test]
fn shadow_unit_timezone_keys_stripped_on_from_arrow() {
    let field = Ts64Field::new("t", TimeUnit::Microsecond, Tz::UTC, true)
        .with_metadata_entry("owner", "events");
    let arrow = field.to_arrow();
    let back = Ts64Field::from_arrow(&arrow).unwrap();
    // The user metadata survives; the shadow unit/timezone keys are consumed, not leaked.
    assert_eq!(back.metadata().get("owner"), Some("events"));
    assert!(back.metadata().get("unit").is_none());
    assert!(back.metadata().get("timezone").is_none());
    assert_eq!(back, field);
}

// -------------------------------------------------------------------------------------
// 11. Empty, all-null, and single-element columns export/import.
// -------------------------------------------------------------------------------------

#[test]
fn empty_all_null_and_single_element_columns() {
    // Empty.
    let empty = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[]).unwrap();
    let field = empty.to_field("t").to_arrow();
    assert_eq!(
        Ts64Serie::from_arrow_array(empty.to_arrow_array().unwrap().as_ref(), &field).unwrap(),
        empty
    );

    // All-null.
    let all_null = Ts64Serie::from_options(TimeUnit::Second, Tz::UTC, &[None, None, None]).unwrap();
    assert_eq!(all_null.null_count(), 3);
    let array = all_null.to_arrow_array().unwrap();
    assert_eq!(array.null_count(), 3);
    assert_eq!(
        Ts64Serie::from_arrow_array(array.as_ref(), &field).unwrap(),
        all_null
    );

    // Single element.
    let a = Ts64::from_epoch(7, TimeUnit::Second, Tz::UTC).unwrap();
    let single = Ts64Serie::from_values(TimeUnit::Second, Tz::UTC, &[a]).unwrap();
    assert_eq!(
        Ts64Serie::from_arrow_array(single.to_arrow_array().unwrap().as_ref(), &field).unwrap(),
        single
    );
}

// -------------------------------------------------------------------------------------
// 12. A temporal column as a struct child, round-tripped through a RecordBatch.
// -------------------------------------------------------------------------------------

#[test]
fn temporal_column_as_struct_child_through_record_batch() {
    let a = Ts64::from_epoch(1_700_000_000, TimeUnit::Microsecond, Tz::UTC).unwrap();
    let b = Ts64::from_epoch(1_700_000_100, TimeUnit::Microsecond, Tz::UTC).unwrap();
    let ts =
        Ts64Serie::from_options(TimeUnit::Microsecond, Tz::UTC, &[Some(a), None, Some(b)]).unwrap();
    let ids = Serie::from_values(&[1i64, 2, 3]);

    let table = StructSerie::from_named(vec![("id", boxed(ids)), ("event_at", boxed(ts))]).unwrap();
    let batch = table.to_record_batch().unwrap();

    // The temporal child's unit + timezone survive in the schema — a Ts64 encodes them in its
    // Arrow `Timestamp(unit, tz)` type itself (so the redundant shadow metadata keys are stripped).
    let ts_field = batch.schema().field_with_name("event_at").unwrap().clone();
    assert_eq!(
        ts_field.data_type(),
        &ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, Some("UTC".into()))
    );

    let back = StructSerie::from_record_batch(&batch).unwrap();
    assert_eq!(back, table);
}
