//! IPC round trips through one owning media handle.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};

use super::Ipc;
use crate::io::{Buffer, IOBase};
use crate::{DataType, Field, Url};

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
    RecordBatch::try_new(
        crate::arrow::schema_from_field(&schema()).unwrap(),
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

/// A reader that declares the schema and yields nothing.
fn empty_reader() -> crate::arrow::BatchReader {
    empty_reader_for(&schema())
}

/// A reader that declares `field` and yields nothing.
fn empty_reader_for(field: &Field) -> crate::arrow::BatchReader {
    crate::arrow::batch_reader(crate::arrow::schema_from_field(field).unwrap(), [])
}

/// A handle whose media type comes from a name, so codings are declared.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

#[test]
fn one_instance_owns_the_handle_and_the_configuration() {
    let mut media = Ipc::new(handle("t.arrows")).with_schema(schema());

    // Neither call repeats the schema, the root name, or the coding.
    media.write_batch_reader(reader()).unwrap();
    assert_eq!(media.read_batch_reader(None).unwrap().count(), 1);
    assert_eq!(media.schema().unwrap(), schema());
}

#[test]
fn batches_round_trip_through_every_coding() {
    for name in ["t.arrows", "t.arrows.gz", "t.arrows.zst"] {
        let mut media = Ipc::new(handle(name)).with_schema(schema());
        media
            .write_batch_reader(reader())
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = media
            .read_batch_reader(None)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), 1, "{name}");
        assert_eq!(actual[0].num_rows(), 2, "{name}");
    }
}

#[test]
fn a_compressed_handle_really_is_compressed() {
    let mut plain = Ipc::new(handle("plain.arrows")).with_schema(schema());
    let mut gzipped = Ipc::new(handle("coded.arrows.gz")).with_schema(schema());

    plain.write_batch_reader(reader()).unwrap();
    gzipped.write_batch_reader(reader()).unwrap();

    assert_eq!(&gzipped.handle().as_slice()[..2], &[0x1F, 0x8B]);
    assert_ne!(plain.handle().size(), gzipped.handle().size());
}

#[test]
fn an_omitted_schema_is_inferred_from_the_stream() {
    let mut writer = Ipc::new(handle("inferred.arrows")).with_schema(schema());
    writer.write_batch_reader(reader()).unwrap();

    // A reader with no declared schema recovers it from the bytes.
    let reader = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    assert_eq!(reader.schema().unwrap(), schema());
    assert_eq!(reader.read_batch_reader(None).unwrap().count(), 1);
}

#[test]
fn open_caches_the_schema_and_close_releases_it() {
    let mut writer = Ipc::new(handle("cached.arrows")).with_schema(schema());
    writer.write_batch_reader(reader()).unwrap();

    let mut reader = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    assert!(!reader.is_open());

    reader.open().unwrap();
    assert!(reader.is_open());
    // The cached schema answers without re-deriving it.
    assert_eq!(reader.schema().unwrap(), schema());

    reader.close().unwrap();
    assert!(!reader.is_open());
    // Still usable after closing; it simply re-derives.
    assert_eq!(reader.schema().unwrap(), schema());
}

#[test]
fn a_closed_stream_fetches_fresh_and_an_open_one_holds_what_it_cached() {
    let renamed = DataType::from_fields([DataType::Int64.required_field("code")])
        .unwrap()
        .required_field("row");
    let mut first = Ipc::new(handle("first.arrows")).with_schema(schema());
    first.write_batch_reader(reader()).unwrap();
    let mut second = Ipc::new(handle("second.arrows")).with_schema(renamed.clone());
    second
        .write_batch_reader(empty_reader_for(&renamed))
        .unwrap();

    // A closed stream reads its schema fresh every time, so a change made
    // underneath the wrapper - here, swapping the bytes directly - is seen
    // immediately. A cache nobody opened would have answered stale.
    let mut probe = Ipc::new(Buffer::from_bytes(first.handle().as_slice().to_vec()));
    assert_eq!(probe.schema().unwrap(), schema());
    assert!(!probe.is_open());
    probe
        .handle_mut()
        .write_all_bytes(second.handle().as_slice())
        .unwrap();
    assert_eq!(probe.schema().unwrap(), renamed);

    // Opening is the opt-in to retention: the cache answers until close,
    // even after the bytes change underneath again.
    probe.open().unwrap();
    assert_eq!(probe.schema().unwrap(), renamed);
    probe
        .handle_mut()
        .write_all_bytes(first.handle().as_slice())
        .unwrap();
    assert_eq!(probe.schema().unwrap(), renamed);
    probe.close().unwrap();
    assert_eq!(probe.schema().unwrap(), schema());
}

#[test]
fn opening_an_empty_stream_caches_nothing_and_reads_no_batches() {
    let mut media = Ipc::new(handle("empty.arrows"));
    media.open().unwrap();

    assert!(!media.is_open());
    assert_eq!(media.read_batch_reader(None).unwrap().count(), 0);
}

#[test]
fn writing_no_batches_still_writes_the_schema() {
    let mut media = Ipc::new(handle("empty.arrows")).with_schema(schema());
    media.write_batch_reader(empty_reader()).unwrap();

    assert!(!media.handle().is_empty());
    assert_eq!(media.read_batch_reader(None).unwrap().count(), 0);
    // The stream carries its schema even with no rows in it.
    assert_eq!(media.schema().unwrap(), schema());
}

#[test]
fn a_missing_stream_reads_as_empty_rather_than_failing() {
    // An untouched buffer stands in for a resource that does not exist yet.
    let media = Ipc::new(Buffer::new()).with_schema(schema());
    assert_eq!(media.read_batch_reader(None).unwrap().count(), 0);
}

/// A schema naming fewer columns becomes an Arrow IPC projection, so the
/// columns it leaves out are never built into arrays.
mod pushdown {
    use arrow_array::RecordBatchReader;

    use super::{Ipc, handle, reader, schema};
    use crate::DataType;
    use crate::Field;

    /// One of the two stored columns.
    fn narrow() -> Field {
        DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row")
    }

    #[test]
    fn a_subset_schema_narrows_the_reader_and_its_batches() {
        let mut media = Ipc::new(handle("pushdown.arrows"));
        media.write_batch_reader(reader()).unwrap();

        // The stream still carries both columns.
        assert_eq!(media.schema().unwrap().field_len(), 2);

        let projected = media.read_batch_reader(Some(&narrow())).unwrap();
        // A projected StreamReader reports the whole stream's schema, so this
        // proves the reported schema is the projected one, not Arrow's.
        assert_eq!(projected.schema().fields().len(), 1);
        assert_eq!(projected.schema().field(0).name(), "id");

        let batches = projected
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_columns(), 1);
        assert_eq!(batches[0].num_rows(), 2);
    }

    #[test]
    fn a_schema_naming_every_column_reads_the_stream_whole() {
        let mut media = Ipc::new(handle("unprojected.arrows"));
        media.write_batch_reader(reader()).unwrap();

        let whole = media.read_batch_reader(Some(&schema())).unwrap();
        assert_eq!(whole.schema().fields().len(), 2);
    }
}
