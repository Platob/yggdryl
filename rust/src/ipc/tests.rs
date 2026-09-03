//! IPC round trips through one owning media handle.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::ArrowError;

use super::Ipc;
use crate::buffered::{Buffered, BufferedOptions};
use crate::generic::{IORecordOptions, RecordOptions};
use crate::io::{Buffer, IOBase, IOMedia};
use crate::{Codec, DataType, Field, Url};

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
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
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
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        [batch()],
    )
}

/// A valid stream with enough incompressible body bytes to expose read-ahead.
fn large_reader() -> crate::arrow::BatchReader {
    const ROWS: usize = 128 * 1024;
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let ids = (0..ROWS)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as i64
        })
        .collect::<Vec<_>>();
    let batch = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from_iter(std::iter::repeat_n(
                None::<&str>,
                ROWS,
            ))),
        ],
    )
    .unwrap();
    crate::arrow::batch_reader(batch.schema(), [batch])
}

/// Three rows split across two IPC record-batch messages.
fn multi_batch_reader() -> crate::arrow::BatchReader {
    let first = batch();
    let second = batch().slice(0, 1);
    crate::arrow::batch_reader(first.schema(), [first, second])
}

/// A reader that declares the schema and yields nothing.
fn empty_reader() -> crate::arrow::BatchReader {
    empty_reader_for(&schema())
}

/// A reader that declares `field` and yields nothing.
fn empty_reader_for(field: &Field) -> crate::arrow::BatchReader {
    crate::arrow::batch_reader(crate::arrow::arrow_schema_from_field(field).unwrap(), [])
}

/// A handle whose media type comes from a name, so codings are declared.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

/// A byte handle that makes metadata-only body skipping observable.
#[derive(Debug)]
struct Counting {
    handle: Buffer,
    bytes_read: AtomicUsize,
    reads: AtomicUsize,
    first_request: AtomicUsize,
}

impl Counting {
    fn new(handle: Buffer) -> Self {
        Self {
            handle,
            bytes_read: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            first_request: AtomicUsize::new(0),
        }
    }

    fn bytes_read(&self) -> usize {
        self.bytes_read.load(Ordering::Relaxed)
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    fn first_request(&self) -> usize {
        self.first_request.load(Ordering::Relaxed)
    }
}

impl IOMedia for Counting {
    crate::impl_default_iomedia!();
}

impl IOBase for Counting {
    crate::delegate_iobase!(handle: pwrite, size, capacity, reserve, truncate, url, media_type,
        set_media_type, flush, parent, child_by_path, ls, kind, clear, remove, is_atomic,
        is_tabular, is_io);

    fn pread(&self, offset: u64, bytes: &mut [u8]) -> crate::Result<usize> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        let _ = self.first_request.compare_exchange(
            0,
            bytes.len(),
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
        let read = self.handle.pread(offset, bytes)?;
        self.bytes_read.fetch_add(read, Ordering::Relaxed);
        Ok(read)
    }
}

#[test]
fn one_instance_owns_the_handle_and_the_configuration() {
    let mut media = Ipc::new(handle("t.arrows")).with_field(schema());
    let options = media.record_options().unwrap();

    media.overwrite_arrow_reader(reader(), &options).unwrap();
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 1);
    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
}

#[test]
fn the_wrapper_owns_ipc_options_over_an_unnamed_buffer() {
    let media = Ipc::new(Buffer::new()).with_field(schema());
    let options = media.record_options().unwrap();

    assert!(matches!(options, RecordOptions::Ipc(_)));
    assert_eq!(options.field(), Some(schema()));
}

#[test]
fn mismatched_options_are_rejected_before_any_write_pulls_input() {
    for operation in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut media = Ipc::new(Buffer::new()).with_field(schema());
        let mut options = RecordOptions::Avro(crate::avro::AvroOptions::new());
        if operation == "merge" {
            options.set_merge_by_names(vec!["id".into()]);
        }
        let result = match operation {
            "overwrite" => crate::io::IOMedia::overwrite_arrow_reader(
                &mut media,
                counted_reader(Arc::clone(&pulls)),
                &options,
            ),
            "append" => crate::io::IOMedia::append_arrow_reader(
                &mut media,
                counted_reader(Arc::clone(&pulls)),
                &options,
            ),
            "merge" => crate::io::IOMedia::merge_arrow_reader(
                &mut media,
                counted_reader(Arc::clone(&pulls)),
                &options,
            ),
            _ => unreachable!(),
        };

        let message = result.unwrap_err().to_string();
        assert!(message.contains("IPC"), "{operation}: {message}");
        assert_eq!(pulls.load(Ordering::Relaxed), 0, "{operation}");
        assert!(media.handle().is_empty(), "{operation}");
    }
}

#[test]
fn batches_round_trip_through_every_coding() {
    for name in ["t.arrows", "t.arrows.gz", "t.arrows.zst"] {
        let mut media = Ipc::new(handle(name)).with_field(schema());
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(reader(), &options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let actual = media
            .read_arrow_reader(&options)
            .unwrap_or_else(|error| panic!("{name}: {error}"))
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), 1, "{name}");
        assert_eq!(actual[0].num_rows(), 2, "{name}");
    }
}

#[test]
fn a_compressed_handle_really_is_compressed() {
    let mut plain = Ipc::new(handle("plain.arrows")).with_field(schema());
    let mut gzipped = Ipc::new(handle("coded.arrows.gz")).with_field(schema());

    let plain_options = plain.record_options().unwrap();
    plain
        .overwrite_arrow_reader(reader(), &plain_options)
        .unwrap();
    let gzipped_options = gzipped.record_options().unwrap();
    gzipped
        .overwrite_arrow_reader(reader(), &gzipped_options)
        .unwrap();

    assert_eq!(&gzipped.handle().as_slice()[..2], &[0x1F, 0x8B]);
    assert_ne!(plain.handle().size(), gzipped.handle().size());
}

#[test]
fn an_omitted_schema_is_inferred_from_the_stream() {
    let mut writer = Ipc::new(handle("inferred.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();

    // A reader with no declared schema recovers it from the bytes.
    let reader = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    let options = reader.record_options().unwrap();
    assert_eq!(reader.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(reader.read_arrow_reader(&options).unwrap().count(), 1);
}

#[test]
fn open_caches_the_schema_and_close_releases_it() {
    let mut writer = Ipc::new(handle("cached.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();

    let mut reader = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    let options = reader.record_options().unwrap();
    assert!(!reader.opened());

    reader.open().unwrap();
    assert!(reader.opened());
    // The cached schema answers without re-deriving it.
    assert_eq!(reader.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(reader.row_size().unwrap(), 2);
    assert_eq!(reader.column_size().unwrap(), 2);

    reader.close().unwrap();
    assert!(!reader.opened());
    // Still usable after closing; it simply re-derives.
    assert_eq!(reader.read_arrow_field(&options).unwrap(), schema());
}

/// A one-batch reader that reports whether a write pulled its input.
fn counted_reader(pulls: Arc<AtomicUsize>) -> crate::arrow::BatchReader {
    let batch = batch();
    let schema = batch.schema();
    let batches = std::iter::once(batch).inspect(move |_| {
        pulls.fetch_add(1, Ordering::Relaxed);
    });
    crate::arrow::batch_reader(schema, batches)
}

fn reader_then_error(first: RecordBatch) -> crate::arrow::BatchReader {
    let schema = first.schema();
    Box::new(RecordBatchIterator::new(
        [
            Ok(first),
            Err(ArrowError::ComputeError("later IPC source failure".into())),
        ],
        schema,
    ))
}

#[test]
fn an_open_cache_tracks_the_published_field_after_a_fitting_overwrite() {
    let mut writer = Ipc::new(handle("cache-source.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();
    let mut media = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    let options = media.record_options().unwrap();
    media.open().unwrap();
    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());

    // The input is shaped differently but fits the stored field: `id` is
    // text and `symbol` is absent. The overwrite completion keeps the stored
    // Int64 + nullable Utf8 shape, so an open cache must not retain this
    // reader's pre-cast schema.
    let loose = DataType::from_fields([DataType::Utf8.required_field("id")])
        .unwrap()
        .required_field("row");
    let incoming = RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&loose).unwrap(),
        vec![Arc::new(StringArray::from(vec!["7"]))],
    )
    .unwrap();
    media
        .overwrite_arrow_reader(
            crate::arrow::batch_reader(incoming.schema(), [incoming]),
            &options,
        )
        .unwrap();

    assert!(media.opened());
    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(
        media.read_arrow_reader(&options).unwrap().schema(),
        crate::arrow::arrow_schema_from_field(&schema()).unwrap()
    );
}

#[test]
fn an_open_cache_tracks_selection_and_completion_on_overwrite() {
    let mut writer = Ipc::new(handle("selected-source.arrows")).with_field(schema());
    let write_options = writer.record_options().unwrap();
    writer
        .overwrite_arrow_reader(reader(), &write_options)
        .unwrap();
    let mut media = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    media.open().unwrap();

    let options = media.record_options().unwrap().with_select_by_names(["id"]);
    crate::io::IOMedia::overwrite_arrow_reader(&mut media, reader(), &options).unwrap();

    // Selection narrows the incoming stream first, then completion onto the
    // stored shape restores `symbol`; the open cache must describe that final
    // publication, not either intermediate schema.
    assert!(media.opened());
    let read_options = media.record_options().unwrap();
    assert_eq!(media.read_arrow_field(&read_options).unwrap(), schema());
    let written = media
        .read_arrow_reader(&read_options)
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    assert_eq!(written.num_columns(), 2);
    assert_eq!(written.column(1).null_count(), written.num_rows());
}

#[test]
fn append_and_merge_keep_an_open_cache_coherent_until_close() {
    let mut media = Ipc::new(Buffer::new()).with_field(schema());
    let options = media.record_options().unwrap();
    media.overwrite_arrow_reader(reader(), &options).unwrap();
    assert!(!media.opened());
    media.open().unwrap();
    media.options_mut().set_commit_row_size(Some(1));

    let append_options = media.record_options().unwrap();
    media
        .append_arrow_reader(reader(), &append_options)
        .unwrap();
    assert!(media.opened());
    assert_eq!(media.read_arrow_field(&append_options).unwrap(), schema());
    assert_eq!(
        media
            .read_arrow_reader(&append_options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        4
    );

    media.options_mut().set_merge_by_names(vec!["id".into()]);
    let merge_options = media.record_options().unwrap();
    media.merge_arrow_reader(reader(), &merge_options).unwrap();
    assert!(media.opened());
    assert_eq!(media.read_arrow_field(&merge_options).unwrap(), schema());
    assert_eq!(
        media
            .read_arrow_reader(&merge_options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        4
    );

    media.close().unwrap();
    assert!(!media.opened());
    assert_eq!(media.read_arrow_field(&merge_options).unwrap(), schema());
}

#[test]
fn a_partial_commit_keeps_an_open_ipc_cache_coherent() {
    let mut media = Ipc::new(Buffer::new()).with_field(schema());
    let options = media.record_options().unwrap();
    media.overwrite_arrow_reader(reader(), &options).unwrap();
    media.open().unwrap();
    media.options_mut().set_commit_row_size(Some(1));

    let options = media.record_options().unwrap();
    let message = media
        .overwrite_arrow_reader(reader_then_error(batch().slice(0, 1)), &options)
        .unwrap_err()
        .to_string();

    assert!(message.contains("later IPC source failure"), "{message}");
    assert!(media.opened());
    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(
        media
            .read_arrow_reader(&options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum::<usize>(),
        1
    );
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

#[test]
fn opening_an_empty_stream_caches_zero_dimensions_and_reads_no_batches() {
    let mut media = Ipc::new(handle("empty.arrows"));
    media.open().unwrap();

    assert!(media.opened());
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 0);
    let options = media.record_options().unwrap();
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 0);
    media.close().unwrap();
    assert!(!media.opened());
}

#[test]
fn dimensions_count_message_metadata_and_ignore_transient_read_shaping() {
    let mut media = Ipc::new(handle("dimensions.arrows")).with_field(schema());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(multi_batch_reader(), &options)
        .unwrap();

    media.options_mut().set_select_by_names(vec!["id".into()]);
    media.options_mut().set_max_row_size(Some(1));
    media.options_mut().set_max_byte_size(Some(1));

    assert_eq!(super::row_size(media.handle(), media.options()).unwrap(), 3);
    assert_eq!(media.row_size().unwrap(), 3);
    assert_eq!(media.column_size().unwrap(), 2);
}

#[test]
fn same_handle_byte_and_option_mutations_invalidate_open_dimensions() {
    let renamed = DataType::from_fields([DataType::Int64.required_field("code")])
        .unwrap()
        .required_field("row");
    let mut replacement = Ipc::new(handle("replacement.arrows")).with_field(renamed.clone());
    let replacement_options = replacement.record_options().unwrap();
    replacement
        .overwrite_arrow_reader(empty_reader_for(&renamed), &replacement_options)
        .unwrap();

    let mut media = Ipc::new(handle("mutable.arrows")).with_field(schema());
    let options = media.record_options().unwrap();
    media.overwrite_arrow_reader(reader(), &options).unwrap();
    media.open().unwrap();
    assert_eq!(media.row_size().unwrap(), 2);
    assert_eq!(media.column_size().unwrap(), 2);

    media
        .handle_mut()
        .write_all_bytes(replacement.handle().as_slice())
        .unwrap();
    assert_eq!(media.row_size().unwrap(), 0);
    // The declared field remains authoritative until the options change.
    assert_eq!(media.column_size().unwrap(), 2);

    media.options_mut().set_field(renamed);
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), 1);
    assert!(media.opened());
}

#[test]
fn row_size_skips_record_batch_bodies_without_constructing_arrays() {
    let mut writer = Ipc::new(handle("metadata-only.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();
    let mut bytes = writer.handle().as_slice().to_vec();

    corrupt_first_record_body(&mut bytes);
    let media = Ipc::new(Buffer::from_bytes(bytes));

    assert_eq!(media.row_size().unwrap(), 2);
    let options = media.record_options().unwrap();
    let error = media
        .read_arrow_reader(&options)
        .unwrap()
        .next()
        .expect("one encoded record batch")
        .unwrap_err();
    assert!(error.to_string().contains("offset"), "{error}");
}

#[test]
fn uncoded_row_size_does_not_read_record_batch_body_ranges() {
    let mut writer = Ipc::new(handle("metadata-ranges.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();
    let total = writer.handle().size() as usize;
    let media = Ipc::new(Counting::new(Buffer::from_bytes(
        writer.handle().as_slice().to_vec(),
    )));

    assert_eq!(media.row_size().unwrap(), 2);
    assert!(
        media.handle().bytes_read() < total,
        "metadata read {} of {total} encoded bytes",
        media.handle().bytes_read()
    );
}

#[test]
fn schema_reads_bypass_buffer_pages_and_stop_before_the_batch_body() {
    let mut writer = Ipc::new(handle("schema-stream.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();
    let total = writer.handle().size() as usize;
    let buffered = Buffered::new(
        Counting::new(Buffer::from_bytes(writer.handle().as_slice().to_vec())),
        BufferedOptions::default(),
    );
    let media = Ipc::new(buffered);
    let options = media.record_options().unwrap();

    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
    assert_eq!(media.handle().cached_pages(), 0);
    assert!(
        media.handle().handle().bytes_read() < total,
        "schema read {} of {total} IPC bytes",
        media.handle().handle().bytes_read()
    );

    // Projection setup also needs the stored schema. It must keep using the
    // sequential path instead of warming a random-access page as a side effect.
    let narrow = DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row");
    let projected_options = options.with_field(narrow);
    let projected = media.read_arrow_reader(&projected_options).unwrap();
    assert_eq!(projected.schema().fields().len(), 1);
    assert_eq!(media.handle().cached_pages(), 0);
}

#[test]
fn compressed_schema_reads_stop_before_the_encoded_batch_body() {
    let mut writer = Ipc::new(handle("large.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer
        .overwrite_arrow_reader(large_reader(), &options)
        .unwrap();
    let encoded = Codec::Gzip.dump(writer.handle().as_slice()).unwrap();
    let total = encoded.len();
    assert!(total > crate::io::DEFAULT_STREAM_BATCH_SIZE);

    let source = Buffer::from_bytes(encoded).with_media_type(
        Url::from_str("file:///large.arrows.gz")
            .unwrap()
            .media_type(),
    );
    let media = Ipc::new(Counting::new(source));
    let options = media.record_options().unwrap();

    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
    assert!(
        media.handle().bytes_read() < total,
        "compressed schema read {} of {total} encoded bytes",
        media.handle().bytes_read()
    );
}

/// Replace the first record-batch body while retaining valid IPC framing and
/// FlatBuffer metadata. A metadata-only counter must never inspect these bytes.
fn corrupt_first_record_body(bytes: &mut [u8]) {
    let mut offset = 0_usize;
    loop {
        let mut prefix = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        offset += 4;
        if prefix == u32::MAX {
            prefix = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
            offset += 4;
        }
        assert_ne!(prefix, 0, "a record batch precedes IPC EOS");
        let metadata_end = offset + prefix as usize;
        let message = arrow_ipc::root_as_message(&bytes[offset..metadata_end]).unwrap();
        offset = metadata_end;
        let body_end = offset + usize::try_from(message.bodyLength()).unwrap();
        if message.header_type() == arrow_ipc::MessageHeader::RecordBatch {
            bytes[offset..body_end].fill(0xFF);
            return;
        }
        offset = body_end;
    }
}

#[test]
fn writing_no_batches_still_writes_the_schema() {
    let mut media = Ipc::new(handle("empty.arrows")).with_field(schema());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(empty_reader(), &options)
        .unwrap();

    assert!(!media.handle().is_empty());
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 0);
    // The stream carries its schema even with no rows in it.
    assert_eq!(media.read_arrow_field(&options).unwrap(), schema());
}

#[test]
fn a_missing_stream_reads_as_empty_rather_than_failing() {
    // An untouched buffer stands in for a resource that does not exist yet.
    let media = Ipc::new(Buffer::new()).with_field(schema());
    let options = media.record_options().unwrap();
    assert_eq!(media.read_arrow_reader(&options).unwrap().count(), 0);
}

#[test]
fn an_absent_compressed_stream_keeps_its_declared_empty_schema() {
    let media = Ipc::new(handle("missing.arrows.gz")).with_field(schema());
    let options = media.record_options().unwrap();
    let mut batches = media.read_arrow_reader(&options).unwrap();

    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(
        batches.schema(),
        crate::arrow::arrow_schema_from_field(&schema()).unwrap()
    );
    assert!(batches.next().is_none());
}

#[test]
fn an_unprojected_reader_uses_one_normal_transport_stream() {
    let mut writer = Ipc::new(handle("one-pass.arrows")).with_field(schema());
    let options = writer.record_options().unwrap();
    writer.overwrite_arrow_reader(reader(), &options).unwrap();
    let counted = Counting::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
    let media = Ipc::new(counted);

    let options = media.record_options().unwrap();
    let batches = media.read_arrow_reader(&options).unwrap();
    assert_eq!(batches.schema().fields().len(), 2);
    assert_eq!(
        media.handle().first_request(),
        crate::io::DEFAULT_STREAM_BATCH_SIZE,
        "absence detection must not issue a one-byte positional read"
    );
    assert_eq!(
        media.handle().reads(),
        2,
        "one data read plus the iterator's EOF read"
    );
}

/// A schema naming fewer columns becomes an Arrow IPC projection, so the
/// columns it leaves out are never built into arrays.
mod pushdown {
    use arrow_array::RecordBatchReader;

    use super::{Ipc, handle, reader, schema};
    use crate::DataType;
    use crate::Field;
    use crate::generic::IORecordOptions;
    use crate::io::IOMedia;

    /// One of the two stored columns.
    fn narrow() -> Field {
        DataType::from_fields([DataType::Int64.required_field("id")])
            .unwrap()
            .required_field("row")
    }

    #[test]
    fn a_subset_schema_narrows_the_reader_and_its_batches() {
        let mut media = Ipc::new(handle("pushdown.arrows"));
        let options = media.record_options().unwrap();
        media.overwrite_arrow_reader(reader(), &options).unwrap();

        // The stream still carries both columns.
        assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 2);

        let projected_options = options.with_field(narrow());
        let projected = media.read_arrow_reader(&projected_options).unwrap();
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
        let options = media.record_options().unwrap();
        media.overwrite_arrow_reader(reader(), &options).unwrap();

        let options = options.with_field(schema());
        let whole = media.read_arrow_reader(&options).unwrap();
        assert_eq!(whole.schema().fields().len(), 2);
    }
}

mod limits {
    use std::sync::Arc;

    use arrow_array::RecordBatchReader;
    use arrow_array::cast::AsArray;
    use arrow_array::types::Int64Type;

    use super::{Buffer, handle, reader, schema};
    use crate::DataType;
    use crate::generic::IORecordOptions;
    use crate::io::IOMedia;

    /// Four stored rows whose symbols alternate, so a filter has rows to skip.
    fn stored() -> Buffer {
        let mut handle = handle("limited.arrows");
        let batch = arrow_array::RecordBatch::try_new(
            crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
            vec![
                Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3, 4])),
                Arc::new(arrow_array::StringArray::from(vec![
                    Some("MSFT"),
                    Some("AAPL"),
                    Some("MSFT"),
                    Some("AAPL"),
                ])),
            ],
        )
        .unwrap();
        let options = handle.record_options().unwrap();
        handle
            .overwrite_arrow_reader(
                crate::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        handle
    }

    /// The total rows a handle yields under `options`.
    fn rows(handle: &Buffer, options: &crate::generic::RecordOptions) -> usize {
        handle
            .read_arrow_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
    }

    #[test]
    fn a_zero_limit_reads_the_declared_schema_and_no_batches() {
        let handle = stored();
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_max_row_size(0);

        let mut limited = handle.read_arrow_reader(&options).unwrap();
        // The schema is asserted, not only the emptiness: `Some(0)` is a
        // valid ask that still says what the rows would have been.
        assert_eq!(
            limited.schema(),
            crate::arrow::arrow_schema_from_field(&schema()).unwrap()
        );
        assert!(limited.next().is_none());
    }

    #[test]
    fn a_zero_limit_without_a_declared_schema_reads_the_stored_one() {
        let handle = stored();
        let options = handle.record_options().unwrap().with_max_row_size(0);

        let mut limited = handle.read_arrow_reader(&options).unwrap();
        assert_eq!(
            limited.schema(),
            crate::arrow::arrow_schema_from_field(&schema()).unwrap()
        );
        assert!(limited.next().is_none());
    }

    #[test]
    fn a_zero_limit_flows_through_the_three_record_methods() {
        let mut handle = handle("zero.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        let zero = options.clone().with_max_row_size(0);

        // A limited write truncates data the caller offered - here to
        // nothing, so only the schema lands.
        handle.overwrite_arrow_reader(reader(), &zero).unwrap();
        assert_eq!(rows(&handle, &options), 0);

        handle.overwrite_arrow_reader(reader(), &options).unwrap();
        handle.append_arrow_reader(reader(), &zero).unwrap();
        assert_eq!(rows(&handle, &options), 2);
    }

    #[test]
    fn a_limited_write_truncates_what_the_caller_offered() {
        let mut handle = handle("truncated.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        handle
            .overwrite_arrow_reader(reader(), &options.clone().with_max_row_size(1))
            .unwrap();
        assert_eq!(rows(&handle, &options), 1);
    }

    #[test]
    fn a_limit_counts_result_rows_after_projection_and_cast() {
        let handle = stored();
        // The declared root both projects the stream down to `id` and casts
        // it to text, so what the limit counts is the shaped result.
        let declared = DataType::from_fields([DataType::Utf8.required_field("id")])
            .unwrap()
            .required_field("row");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(declared)
            .with_max_row_size(2);

        let batches: Vec<arrow_array::RecordBatch> = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].num_columns(), 1);
        assert_eq!(batches[0].num_rows(), 2);
        assert_eq!(batches[0].column(0).as_string::<i32>().value(1), "2");
    }

    #[test]
    fn a_limit_with_a_filter_means_the_first_matching_rows() {
        let handle = stored();
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_filter_partitions([("symbol", "AAPL")])
            .with_max_row_size(1);

        let batches: Vec<arrow_array::RecordBatch> = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect();
        // The first stored row does not match, so a limit of one means the
        // first *matching* row - id 2 - never simply the first row.
        let total: usize = batches.iter().map(arrow_array::RecordBatch::num_rows).sum();
        assert_eq!(total, 1);
        let first = batches
            .iter()
            .find(|batch| batch.num_rows() > 0)
            .expect("one matching row");
        assert_eq!(first.column(0).as_primitive::<Int64Type>().value(0), 2);
    }
}
