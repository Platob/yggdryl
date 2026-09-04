//! Partition columns move between the path and the data without loss.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray};

use super::{partitioned_reader, with_partitions, without_partitions};
use crate::IOBase;
use crate::media::RecordOptions;
use crate::{DataType, Field};

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
            DataType::Timestamp(TimeUnit::Second, None),
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::NAIVE).unwrap(),
        ),
        (
            DataType::Timestamp(TimeUnit::Second, Some(Timezone::UTC)),
            Scalar::datetime64(1_700_000_000, TimeUnit::Second, Timezone::UTC).unwrap(),
        ),
        (
            DataType::Timestamp(TimeUnit::Second, Some(paris)),
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
