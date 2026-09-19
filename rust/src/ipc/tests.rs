//! The stream cache an integration test cannot reach.
//!
//! Reaching the handle field directly models a different storage client: it
//! changes the bytes underneath without the wrapper's own mutation
//! invalidation, which is the only way to tell a closed handle's fresh fetch
//! from an open one's retained answer. `handle_mut` invalidates, so it cannot
//! stand in. Everything a caller can observe lives in `tests/media/ipc.rs`.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};

use crate::arrow::BatchReader;
use crate::holder::Buffer;
use crate::ipc::Ipc;
use crate::{DataType, Field, IOBase, IOMedia, Url};

/// A handle whose media type comes from a name, so codings are declared.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .unwrap()
    .required_field("row")
}

/// The batches a write takes: one reader over one two-row batch.
fn reader() -> crate::arrow::BatchReader {
    crate::arrow::batch_reader(schema().into_arrow_schema().unwrap(), [batch()])
}

/// The two-row batch every write in this module takes.
fn batch() -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )
    .unwrap()
}

/// A reader carrying a schema and no rows.
fn empty_reader_for(field: &Field) -> BatchReader {
    crate::arrow::batch_reader(field.clone().into_arrow_schema().unwrap(), [])
}

#[test]
fn a_closed_stream_fetches_fresh_and_an_open_one_holds_what_it_cached() {
    let renamed = DataType::from_fields([DataType::Int64.required_field("code")])
        .unwrap()
        .required_field("row");
    let mut first = Ipc::new(handle("first.arrows")).with_field(schema());
    let first_options = first.record_options().unwrap();
    first
        .overwrite_arrow_reader(reader(), &first_options)
        .unwrap();
    let mut second = Ipc::new(handle("second.arrows")).with_field(renamed.clone());
    let second_options = second.record_options().unwrap();
    second
        .overwrite_arrow_reader(empty_reader_for(&renamed), &second_options)
        .unwrap();

    // A closed stream reads its schema fresh every time, so a change made
    // underneath the wrapper - here, swapping the bytes directly - is seen
    // immediately. A cache nobody opened would have answered stale.
    let mut probe = Ipc::new(Buffer::from_bytes(first.handle().as_slice().to_vec()));
    let options = probe.record_options().unwrap();
    assert_eq!(probe.read_arrow_field(&options).unwrap(), schema());
    assert!(!probe.opened());
    // Reaching the private handle directly models a different storage client:
    // it deliberately bypasses this wrapper's mutation invalidation.
    probe
        .handle
        .write_all_bytes(second.handle().as_slice())
        .unwrap();
    assert_eq!(probe.read_arrow_field(&options).unwrap(), renamed);

    // Opening is the opt-in to retention: the cache answers until close,
    // even after the bytes change underneath again.
    probe.open().unwrap();
    assert_eq!(probe.read_arrow_field(&options).unwrap(), renamed);
    assert_eq!(probe.row_size().unwrap(), 0);
    assert_eq!(probe.column_size().unwrap(), 1);
    probe
        .handle
        .write_all_bytes(first.handle().as_slice())
        .unwrap();
    assert_eq!(probe.read_arrow_field(&options).unwrap(), renamed);
    assert_eq!(probe.row_size().unwrap(), 0);
    assert_eq!(probe.column_size().unwrap(), 1);
    probe.close().unwrap();
    assert_eq!(probe.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(probe.row_size().unwrap(), 2);
    assert_eq!(probe.column_size().unwrap(), 2);
}
