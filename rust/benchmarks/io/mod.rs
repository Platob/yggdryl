pub(crate) mod pushdown;
pub(crate) mod record;

use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{DataType, Field, Url};

/// Rows per fixture, chosen so a column chunk is worth skipping.
pub(crate) const ROWS: i64 = 65_536;

/// A four-column root: two cheap numeric columns and two wide string ones.
///
/// The split is the point of the pushdown measurements - a projection that
/// keeps only the numeric pair leaves the bulk of the payload untouched.
pub(crate) fn wide() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])
    .expect("a valid struct root")
    .required_field("row")
}

/// The two columns a projected read asks for.
pub(crate) fn narrow() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])
    .expect("a valid struct root")
    .required_field("row")
}

/// One batch holding every row of the wide fixture.
pub(crate) fn batch() -> RecordBatch {
    let ids: Vec<i64> = (0..ROWS).collect();
    #[allow(clippy::cast_precision_loss)]
    let prices: Vec<f64> = ids.iter().map(|id| *id as f64).collect();
    RecordBatch::try_new(
        yggdryl::arrow::schema_from_field(&wide()).expect("a projectable root"),
        vec![
            Arc::new(Int64Array::from(ids.clone())),
            Arc::new(StringArray::from(
                ids.iter()
                    .map(|id| format!("SYMBOL-{id:08}"))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(prices)),
            Arc::new(StringArray::from(
                ids.iter()
                    .map(|id| format!("VENUE-{id:08}"))
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .expect("a batch matching the root")
}

/// A handle whose media type comes from a name, so the encoding is declared.
pub(crate) fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .expect("a valid location")
            .media_type(),
    )
}

/// A handle already holding the wide fixture in the named encoding.
pub(crate) fn stored(name: &str) -> Buffer {
    let mut handle = handle(name);
    let options = handle.record_options().expect("an implemented encoding");
    let batch = batch();
    handle
        .write_arrow_batch_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .expect("the fixture must write");
    handle
}

/// Bytes every array a read materializes occupies.
///
/// This is the quantity a column pushdown is supposed to reduce, so the
/// benchmarks report it as throughput rather than claiming a saving from
/// timings alone.
pub(crate) fn materialized(reader: yggdryl::arrow::BatchReader) -> u64 {
    reader
        .map(|batch| {
            let batch = batch.expect("a decodable batch");
            batch
                .columns()
                .iter()
                .map(|column| arrow_array::Array::get_array_memory_size(column) as u64)
                .sum::<u64>()
        })
        .sum()
}
