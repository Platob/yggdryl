//! Partition columns move between the path and the data without loss.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray};

use super::{partitioned_reader, with_partitions, without_partitions};
use crate::media::RecordOptions;
use crate::{ArrowCast, DataType, Field, IOBase};

fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
        DataType::Utf8.required_field("month"),
    ])
    .unwrap()
    .required_field("row")
}

fn prices() -> RecordBatch {
    RecordBatch::try_from_iter([(
        "price",
        Arc::new(Int64Array::from(vec![10, 20, 30])) as ArrayRef,
    )])
    .unwrap()
}

fn partitions() -> Vec<(String, String)> {
    vec![
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ]
}

#[test]
fn restored_columns_take_the_type_the_schema_declares() {
    let restored = with_partitions(&prices(), &partitions(), Some(&schema())).unwrap();

    assert_eq!(restored.num_columns(), 3);
    assert_eq!(restored.num_rows(), 3);

    let year = restored
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("an Int32 column, as the schema declares");
    assert_eq!(year.values(), &[2024, 2024, 2024]);

    let month = restored
        .column_by_name("month")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("a Utf8 column");
    assert_eq!(month.value(0), "01");
    // A path value is spelled out, so it is never null.
    assert!(!restored.schema().field(1).is_nullable());
}

#[test]
fn an_ascii_partition_column_is_restored_padded_with_its_identity() {
    let declared = DataType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::FixedAscii(4).required_field("ccy"),
    ])
    .unwrap()
    .required_field("row");

    let restored = with_partitions(
        &prices(),
        &[("ccy".to_owned(), "USD".to_owned())],
        Some(&declared),
    )
    .unwrap();

    // The path spells the trimmed text; the column holds the padded storage
    // and keeps the extension identity the declaration carries.
    let ccy = restored
        .column_by_name("ccy")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::FixedSizeBinaryArray>()
        .expect("the ASCII storage, as the schema declares");
    assert_eq!(ccy.value(0), b"USD\0");
    let field = Field::from_arrow(restored.schema().field(1)).unwrap();
    assert_eq!(field.dtype(), &DataType::FixedAscii(4));
    assert!(field.is_partition());
}

#[test]
fn a_restored_column_is_text_when_no_schema_says_otherwise() {
    let restored = with_partitions(&prices(), &partitions(), None).unwrap();

    assert!(
        restored
            .column_by_name("year")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .is_some()
    );
}

#[test]
fn a_column_the_data_already_carries_is_left_alone() {
    let batch = RecordBatch::try_from_iter([
        ("price", Arc::new(Int64Array::from(vec![10])) as ArrayRef),
        ("year", Arc::new(Int32Array::from(vec![1999])) as ArrayRef),
    ])
    .unwrap();

    let restored = with_partitions(&batch, &partitions(), Some(&schema())).unwrap();

    assert_eq!(restored.num_columns(), 3);
    let year = restored
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    // The stored value wins, so a mismatch stays visible instead of being
    // rewritten from the directory name.
    assert_eq!(year.values(), &[1999]);
}

#[test]
fn a_value_that_does_not_fit_its_declared_type_is_an_error() {
    let broken = vec![("year".to_owned(), "not-a-year".to_owned())];

    assert!(with_partitions(&prices(), &broken, Some(&schema())).is_err());
}

#[test]
fn nothing_changes_without_partitions() {
    let batch = prices();

    assert_eq!(with_partitions(&batch, &[], None).unwrap(), batch);
    assert_eq!(without_partitions(&batch, &[]).unwrap(), batch);
    // A partition the batch does not carry removes nothing.
    assert_eq!(without_partitions(&batch, &partitions()).unwrap(), batch);
}

#[test]
fn removing_every_column_keeps_the_row_count() {
    let batch = RecordBatch::try_from_iter([(
        "year",
        Arc::new(Int32Array::from(vec![2024, 2024])) as ArrayRef,
    )])
    .unwrap();

    let narrowed = without_partitions(&batch, &partitions()).unwrap();

    assert_eq!(narrowed.num_columns(), 0);
    assert_eq!(narrowed.num_rows(), 2);
}

#[test]
fn the_reader_reports_the_widened_schema_before_the_first_batch() {
    let inner: crate::arrow::BatchReader = Box::new(arrow_array::RecordBatchIterator::new(
        [Ok(prices())],
        prices().schema(),
    ));

    let reader = partitioned_reader(inner, partitions(), Some(schema())).unwrap();

    assert_eq!(reader.schema().fields().len(), 3);
    let batches: Vec<_> = reader.collect::<std::result::Result<Vec<_>, _>>().unwrap();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_columns(), 3);
}

/// A declared folder shape must leave both its listing and its leaves lazy.
/// A root declaring `year` as `year(event)` beside the column it reads.
fn derived_schema() -> Field {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut().set_sources(["event"]).unwrap();
    year.as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();
    DataType::from_fields([DataType::Date32.required_field("event"), year])
        .unwrap()
        .required_field("row")
}

/// Two days, one in 2024 and one in 2025.
fn events() -> RecordBatch {
    RecordBatch::try_from_iter([(
        "event",
        Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
    )])
    .unwrap()
}

#[test]
fn a_declared_derived_column_the_batch_lacks_is_computed_from_its_source() {
    let filled = derived_schema()
        .as_partition()
        .apply_arrow_batch(&events())
        .unwrap();

    assert_eq!(filled.num_columns(), 2);
    assert_eq!(filled.num_rows(), 2);
    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .expect("an Int32 column, as the schema declares");
    assert_eq!(year.values(), &[2024, 2025]);
}

#[test]
fn a_derived_column_is_not_marked_as_one_a_path_spells_out() {
    let filled = derived_schema()
        .as_partition()
        .apply_arrow_batch(&events())
        .unwrap();

    // `field:partition` says a directory carries the column. This one is
    // computed from the rows, so the declaration travels unchanged.
    let declared = Field::from_arrow(filled.schema().field(1)).unwrap();
    assert!(!declared.is_partition());
    assert_eq!(
        declared.as_partition().sources().unwrap(),
        Some(vec!["event".to_owned()])
    );
}

#[test]
fn a_derived_column_carrying_values_is_left_alone() {
    let batch = RecordBatch::try_from_iter([
        (
            "event",
            Arc::new(arrow_array::Date32Array::from(vec![19_723])) as ArrayRef,
        ),
        ("year", Arc::new(Int32Array::from(vec![1999])) as ArrayRef),
    ])
    .unwrap();

    let filled = derived_schema()
        .as_partition()
        .apply_arrow_batch(&batch)
        .unwrap();

    // The stored value wins, exactly as it does against a directory name, so a
    // mismatch stays visible instead of being recomputed away.
    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(year.values(), &[1999]);
}

#[test]
fn a_column_holding_nothing_but_nulls_is_filled_from_the_batchs_own_schema() {
    // A batch cast to its root carries the declared column null-filled, which
    // is a column that was never written rather than one written null.
    let placeholder = derived_schema().cast_arrow_batch(events(), true).unwrap();
    assert_eq!(placeholder.column(1).null_count(), 2);

    let filled = Field::from_arrow_schema("row", placeholder.schema().as_ref())
        .unwrap()
        .as_partition()
        .apply_arrow_batch(&placeholder)
        .unwrap();

    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(year.values(), &[2024, 2025]);
    assert_eq!(filled.num_columns(), 2);
}

#[test]
fn an_absent_transform_copies_the_source_value_unchanged() {
    let mut day = DataType::Date32.nullable_field("event_day");
    day.as_partition_mut().set_sources(["event"]).unwrap();
    let root = DataType::from_fields([DataType::Date32.required_field("event"), day])
        .unwrap()
        .required_field("row");

    let filled = root.as_partition().apply_arrow_batch(&events()).unwrap();

    assert_eq!(filled.column(1).as_ref(), filled.column(0).as_ref());
}

#[test]
fn a_source_path_reaches_a_struct_child() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut()
        .set_sources(["trade.event"])
        .unwrap();
    year.as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();
    let trade = DataType::from_fields([DataType::Date32.required_field("event")])
        .unwrap()
        .required_field("trade");
    let root = DataType::from_fields([trade, year])
        .unwrap()
        .required_field("row");

    let batch = RecordBatch::try_from_iter([(
        "trade",
        Arc::new(arrow_array::StructArray::from(vec![(
            Arc::new(arrow_schema::Field::new(
                "event",
                arrow_schema::DataType::Date32,
                false,
            )),
            Arc::new(arrow_array::Date32Array::from(vec![19_723])) as ArrayRef,
        )])) as ArrayRef,
    )])
    .unwrap();

    let filled = root.as_partition().apply_arrow_batch(&batch).unwrap();

    let year = filled
        .column_by_name("year")
        .unwrap()
        .as_any()
        .downcast_ref::<Int32Array>()
        .unwrap();
    assert_eq!(year.values(), &[2024]);
}

#[test]
fn the_batch_is_returned_unchanged_when_nothing_declares_a_derivation() {
    let filled = schema()
        .as_partition()
        .apply_arrow_batch(&prices())
        .unwrap();

    assert_eq!(filled.num_columns(), 1);
    assert_eq!(filled.schema(), prices().schema());
}

#[test]
fn a_widened_batch_keeps_the_schema_metadata_it_arrived_with() {
    let batch = events();
    let schema = Arc::new(
        batch.schema().as_ref().clone().with_metadata(
            [("origin".to_owned(), "tape".to_owned())]
                .into_iter()
                .collect(),
        ),
    );
    let batch = RecordBatch::try_new(schema, batch.columns().to_vec()).unwrap();

    let filled = derived_schema()
        .as_partition()
        .apply_arrow_batch(&batch)
        .unwrap();

    assert_eq!(
        filled.schema().metadata().get("origin"),
        Some(&"tape".to_owned())
    );
}

#[test]
fn a_transform_without_sources_beside_it_is_refused() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();

    let error = year.as_partition().expression().unwrap_err().to_string();
    assert!(error.contains("partition:sources"), "{error}");
}

#[test]
fn a_transform_that_is_not_a_function_of_one_argument_is_refused() {
    let mut year = DataType::Int32.nullable_field("year");
    assert!(
        year.as_partition_mut()
            .set_transform(crate::expression::Function::Truncate)
            .is_err()
    );
    // The refused write leaves the field untouched, and the generic mutation
    // path runs the same validator.
    assert!(year.as_partition().is_empty());
    let error = year
        .as_partition_mut()
        .insert("transform", "epoch")
        .unwrap_err()
        .to_string();
    assert!(error.contains("partition:transform"), "{error}");
    assert!(year.as_partition().is_empty());
}

#[test]
fn a_transform_reading_more_than_one_source_is_refused_for_now() {
    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut()
        .set_sources(["event", "venue"])
        .unwrap();
    year.as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();

    // The list shape is stored, so the intent survives; only evaluating it is
    // refused, and by a message that says which shape reads today.
    assert_eq!(
        year.as_partition().sources().unwrap(),
        Some(vec!["event".to_owned(), "venue".to_owned()])
    );
    let error = year.as_partition().expression().unwrap_err().to_string();
    assert!(error.contains("exactly one source"), "{error}");

    // The one shape every `sources` property refuses, whatever declares it.
    assert!(
        year.as_partition_mut()
            .set_sources(["event", "event"])
            .is_err()
    );
    assert!(year.as_partition_mut().set_sources(["*", "event"]).is_err());
}

#[test]
fn a_source_column_the_batch_does_not_carry_is_refused() {
    let error = derived_schema()
        .as_partition()
        .apply_arrow_batch(&prices())
        .unwrap_err()
        .to_string();
    assert!(error.contains("event"), "{error}");
}

#[test]
fn a_nested_declaration_is_filled_before_the_level_above_reads_it() {
    let mut inner_year = DataType::Int32.nullable_field("year");
    inner_year
        .as_partition_mut()
        .set_sources(["event"])
        .unwrap();
    inner_year
        .as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();
    // A source path is relative to the Struct that declares it, so the nested
    // column names `event`, and the level above names `trade.year`.
    let trade = DataType::from_fields([DataType::Date32.required_field("event"), inner_year])
        .unwrap()
        .required_field("trade");
    let mut top = DataType::Int32.nullable_field("top_year");
    top.as_partition_mut().set_sources(["trade.year"]).unwrap();
    let root = DataType::from_fields([trade, top])
        .unwrap()
        .required_field("row");

    let batch = RecordBatch::try_from_iter([(
        "trade",
        Arc::new(arrow_array::StructArray::from(vec![(
            Arc::new(arrow_schema::Field::new(
                "event",
                arrow_schema::DataType::Date32,
                false,
            )),
            Arc::new(arrow_array::Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
        )])) as ArrayRef,
    )])
    .unwrap();

    let filled = root.as_partition().apply_arrow_batch(&batch).unwrap();

    let trade = filled
        .column_by_name("trade")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StructArray>()
        .expect("the struct the declaration widened");
    assert_eq!(trade.num_columns(), 2);
    assert_eq!(
        trade
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
    // The level above read what the nested declaration had just written.
    assert_eq!(
        filled
            .column_by_name("top_year")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
}

#[test]
fn a_nested_struct_keeps_its_own_null_mask_through_a_fill() {
    let mut inner_year = DataType::Int32.nullable_field("year");
    inner_year
        .as_partition_mut()
        .set_sources(["event"])
        .unwrap();
    inner_year
        .as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();
    let trade = DataType::from_fields([DataType::Date32.required_field("event"), inner_year])
        .unwrap()
        .nullable_field("trade");
    let root = DataType::from_fields([trade])
        .unwrap()
        .required_field("row");

    let held = arrow_array::StructArray::try_new(
        vec![Arc::new(arrow_schema::Field::new(
            "event",
            arrow_schema::DataType::Date32,
            false,
        ))]
        .into(),
        vec![Arc::new(arrow_array::Date32Array::from(vec![19_723, 0])) as ArrayRef],
        Some(arrow_buffer::NullBuffer::from(vec![true, false])),
    )
    .unwrap();
    let batch = RecordBatch::try_from_iter([("trade", Arc::new(held) as ArrayRef)]).unwrap();

    let filled = root.as_partition().apply_arrow_batch(&batch).unwrap();

    let trade = filled
        .column_by_name("trade")
        .unwrap()
        .as_any()
        .downcast_ref::<arrow_array::StructArray>()
        .unwrap();
    assert_eq!(trade.null_count(), 1);
    assert!(trade.is_null(1));
}

#[test]
fn a_required_column_still_holding_its_canonical_default_is_filled() {
    let mut year = DataType::Int32.required_field("year");
    year.as_partition_mut().set_sources(["event"]).unwrap();
    year.as_partition_mut()
        .set_transform(crate::expression::Function::Year)
        .unwrap();
    let root = DataType::from_fields([DataType::Date32.required_field("event"), year])
        .unwrap()
        .required_field("row");

    // A required column cannot be null, so a cast fills it with the canonical
    // default rather than nothing, and that is what "never written" looks like.
    let placeholder = root.cast_arrow_batch(events(), true).unwrap();
    assert_eq!(
        placeholder
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[0, 0]
    );

    let filled = root.as_partition().apply_arrow_batch(&placeholder).unwrap();

    assert_eq!(
        filled
            .column(1)
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap()
            .values(),
        &[2024, 2025]
    );
}

mod lazy_folder_reader;

/// A Hive layout addressed as a folder, so the three write intents have to
/// resolve its children themselves.
mod lake;

#[test]
fn every_temporal_family_survives_the_directory_name_it_spells() {
    use crate::{Scalar, TimeUnit, Timezone};

    // A partition name is written by one renderer and read by the field cast,
    // so every temporal family has to make the round trip - a zoned instant
    // included, which Arrow's own formatter refuses to spell at all.
    let paris = Timezone::from_str("Europe/Paris").unwrap();
    for (dtype, value) in [
        (DataType::Date32, Scalar::date32(20_682)),
        (DataType::Date64, Scalar::date64(1_786_924_800_000)),
        (
            DataType::time32(TimeUnit::Second).unwrap(),
            Scalar::time32(37_425, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::time64(TimeUnit::Nanosecond).unwrap(),
            Scalar::time64(1, TimeUnit::Nanosecond, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::NAIVE,
            },
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: Timezone::UTC,
            },
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        (
            DataType::DateTime64 {
                unit: TimeUnit::Second,
                timezone: paris,
            },
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, paris).unwrap(),
        ),
        (
            DataType::duration64(TimeUnit::Second).unwrap(),
            Scalar::duration64(90, TimeUnit::Second).unwrap(),
        ),
    ] {
        let spelled = super::partition_text(&value)
            .unwrap_or_else(|error| panic!("{dtype} has no partition name: {error}"));
        let schema = DataType::from_fields([
            DataType::Int64.required_field("price"),
            Field::new("at", dtype.clone(), false),
        ])
        .unwrap()
        .required_field("row");
        let restored = with_partitions(
            &prices(),
            &[("at".to_owned(), spelled.to_string())],
            Some(&schema),
        )
        .unwrap_or_else(|error| panic!("{dtype} did not read {spelled:?}: {error}"));
        let read = crate::arrow::value::value_from_array(
            &dtype,
            restored.column_by_name("at").unwrap().as_ref(),
            0,
        )
        .unwrap();
        assert_eq!(
            read, value,
            "{dtype} did not round trip through {spelled:?}"
        );
    }
}
