//! Declared schemas drive native projection and casting.

use super::{Buffer, DataType, Field, IORecordOptions, RecordBatchReader, handle};

use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};

use crate::IOMedia;

/// Four columns, so a two-column read is a genuine subset.
fn wide() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .unwrap()
    .required_field("row")
}

/// The two columns a caller actually wants.
fn narrow() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])
    .unwrap()
    .required_field("row")
}

fn stored(name: &str) -> Buffer {
    let mut handle = handle(name);
    let options = handle.record_options().unwrap();
    let batch = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&wide()).unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
            Arc::new(Float64Array::from(vec![1.5, 2.5])),
            Arc::new(StringArray::from(vec![Some("XNAS"), None])),
        ],
    )
    .unwrap();
    handle
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
    handle
}

#[test]
fn a_subset_schema_narrows_the_batches_every_encoding_yields() {
    let mut names = vec!["pushdown.arrows"];
    if cfg!(feature = "parquet") {
        names.push("pushdown.parquet");
    }

    for name in names {
        let handle = stored(name);
        let plain = handle.record_options().unwrap();

        // The resource still holds four columns.
        assert_eq!(
            handle.read_arrow_field(&plain).unwrap().field_len(),
            4,
            "{name}"
        );

        let options = plain.with_field(narrow());
        let reader = handle
            .read_arrow_reader(&options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        // The schema is narrowed before a single batch is decoded.
        assert_eq!(reader.schema().fields().len(), 2, "{name}");
        let batches = reader.map(std::result::Result::unwrap).collect::<Vec<_>>();
        assert_eq!(batches.len(), 1, "{name}");
        assert_eq!(batches[0].num_columns(), 2, "{name}");
        assert_eq!(batches[0].num_rows(), 2, "{name}");
        assert_eq!(batches[0].schema().field(0).name(), "id", "{name}");
        assert_eq!(batches[0].schema().field(1).name(), "price", "{name}");
    }
}

#[test]
fn the_projection_only_drops_columns_and_the_cast_does_the_rest() {
    let handle = stored("unprojected.arrows");
    let plain = handle.record_options().unwrap();

    // Every stored column: there is nothing to skip.
    let all = handle
        .read_arrow_reader(&plain.clone().with_field(wide()))
        .unwrap()
        .schema();
    assert_eq!(all.fields().len(), 4);

    // A column the resource does not hold cannot be projected out of
    // it, so the encoding reads everything and the cast supplies it.
    let invented = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("nowhere"),
    ])
    .unwrap()
    .required_field("row");
    let batches = handle
        .read_arrow_reader(&plain.with_field(invented))
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(batches[0].num_columns(), 2);
    assert_eq!(batches[0].schema().field(1).name(), "nowhere");
    assert_eq!(batches[0].column(1).null_count(), 2);
}

#[test]
fn a_declared_schema_reorders_what_the_resource_stores() {
    let handle = stored("reordered.arrows");
    let reversed = DataType::from_fields([
        DataType::Float64.required_field("price"),
        DataType::Int64.required_field("id"),
    ])
    .unwrap()
    .required_field("row");
    let options = handle.record_options().unwrap().with_field(reversed);

    let batches = handle
        .read_arrow_reader(&options)
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(batches[0].schema().field(0).name(), "price");
    assert_eq!(batches[0].schema().field(1).name(), "id");
}

#[test]
fn an_absent_resource_narrows_its_declared_schema_too() {
    let handle = handle("absent.arrows");
    let options = handle.record_options().unwrap().with_field(narrow());

    let reader = handle.read_arrow_reader(&options).unwrap();
    assert_eq!(reader.schema().fields().len(), 2);
    assert_eq!(reader.count(), 0);
}
