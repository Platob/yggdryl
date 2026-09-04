//! The landing-page example, compiled and run so the site cannot drift.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::IOMedia;
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, Url};

#[test]
fn the_landing_page_example_runs() -> Result<(), Box<dyn std::error::Error>> {
    // A non-null struct field is the schema. Nothing else describes the rows.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    // The name decides the encoding and the compression; nothing else changes.
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
    let options = handle.record_options()?.with_field(schema);

    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;

    // Reading streams: batches arrive one at a time, not as one big vector.
    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    Ok(())
}
