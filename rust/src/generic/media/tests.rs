//! One enum over every record encoding, exercised through one surface.

use arrow_array::{Int64Array, RecordBatch, StringArray};
use std::sync::Arc;

use super::Media;
use crate::generic::Holder;
use crate::io::{Buffer, IOBase};
use crate::{DataType, Field, MimeType, Url};

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

/// Two rows as one Arrow batch.
fn batch() -> RecordBatch {
    let arrow = crate::arrow::schema_from_field(&schema()).unwrap();
    RecordBatch::try_new(
        arrow,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap()
}

/// The batches a write takes: one reader over one two-row batch.
fn reader() -> crate::arrow::BatchReader {
    crate::arrow::batch_reader(
        crate::arrow::schema_from_field(&schema()).unwrap(),
        [batch()],
    )
}

/// A holder whose media type comes from a name, so the encoding is declared.
fn handle(name: &str) -> Holder {
    Holder::buffer(
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        ),
    )
}

#[test]
fn the_name_picks_the_implementation() {
    assert!(matches!(
        Media::open(handle("trades.arrows")).unwrap(),
        Media::Ipc(_)
    ));
    #[cfg(feature = "parquet")]
    assert!(matches!(
        Media::open(handle("trades.parquet")).unwrap(),
        Media::Parquet(_)
    ));
}

#[test]
fn an_unimplemented_encoding_is_named_rather_than_guessed() {
    let message = Media::open(handle("trades.csv")).unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
}

#[test]
fn every_variant_round_trips_batches_through_the_same_calls() {
    let mut names = vec!["trades.arrows", "trades.arrows.gz"];
    if cfg!(feature = "parquet") {
        names.push("trades.parquet");
    }

    for name in names {
        let mut media = Media::open(handle(name)).unwrap().with_schema(schema());
        media
            .write_batch_reader(reader())
            .unwrap_or_else(|error| panic!("{name}: {error}"));

        let rows = media
            .read_batch_reader(None)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>();
        assert_eq!(rows, 2, "{name}");
        assert_eq!(media.schema().unwrap(), schema(), "{name}");
    }
}

#[test]
fn media_mirrors_the_bytes_of_its_handle() {
    let mut media = Media::open(handle("trades.arrows"))
        .unwrap()
        .with_schema(schema());
    media.write_batch_reader(reader()).unwrap();

    // An Arrow IPC stream opens with the continuation marker.
    assert_eq!(media.read_range(0, 4).unwrap(), [0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(media.size(), media.read_all().unwrap().len() as u64);
    assert_eq!(media.media_type().base(), &MimeType::ARROW_STREAM);
}

#[test]
fn a_compressed_handle_mirrors_the_compressed_bytes() {
    let mut media = Media::open(handle("trades.arrows.gz"))
        .unwrap()
        .with_schema(schema());
    media.write_batch_reader(reader()).unwrap();

    assert_eq!(media.read_range(0, 2).unwrap(), [0x1F, 0x8B]);
}

#[test]
fn open_and_close_carry_the_cached_schema() {
    let mut media = Media::open(handle("trades.arrows"))
        .unwrap()
        .with_schema(schema());
    media.write_batch_reader(reader()).unwrap();

    // A reader with no declared schema recovers it once and remembers it.
    let mut reader = Media::ipc(Holder::buffer(Buffer::from_bytes(
        media.read_all().unwrap(),
    )));
    assert!(!reader.opened());
    reader.open().unwrap();
    assert!(reader.opened());
    assert_eq!(reader.schema().unwrap(), schema());

    reader.close().unwrap();
    assert!(!reader.opened());
    // Still usable after closing; it simply re-derives.
    assert_eq!(reader.read_batch_reader(None).unwrap().count(), 1);
}

#[test]
fn a_missing_resource_reads_as_empty_rather_than_failing() {
    let media = Media::ipc(Holder::buffer(Buffer::new())).with_schema(schema());
    assert_eq!(media.read_batch_reader(None).unwrap().count(), 0);
}
