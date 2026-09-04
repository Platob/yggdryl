//! The row and byte limits, exercised through the one shaping seam.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use arrow_schema::{ArrowError, SchemaRef};

use crate::IOMedia;
use crate::arrow::BatchReader;
use crate::generic::{IORecordOptions, RecordOptions};
use crate::io::Buffer;
use crate::ipc::IpcOptions;
use crate::{DataType, Field, Url};

/// A struct field is the schema of the batches it describes.
fn schema() -> Field {
    DataType::from_fields([DataType::Int64.required_field("id")])
        .unwrap()
        .required_field("row")
}

/// One batch holding `ids` as its only column.
fn batch(ids: std::ops::Range<i64>) -> RecordBatch {
    RecordBatch::try_new(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        vec![Arc::new(Int64Array::from_iter_values(ids))],
    )
    .unwrap()
}

/// A reader over `count` batches of `per_batch` rows each.
fn reader(count: i64, per_batch: i64) -> BatchReader {
    let batches: Vec<RecordBatch> = (0..count)
        .map(|index| batch(index * per_batch..(index + 1) * per_batch))
        .collect();
    crate::arrow::batch_reader(
        crate::arrow::arrow_schema_from_field(&schema()).unwrap(),
        batches,
    )
}

/// The total rows a limited reader yields.
fn rows(reader: BatchReader) -> usize {
    reader.map(|batch| batch.unwrap().num_rows()).sum()
}

#[test]
fn the_declared_field_is_built_from_its_three_parts() {
    let declared = schema();
    let mut options = IpcOptions::new();

    // Nothing declared: no field, the default root name, no metadata.
    assert!(options.field().is_none());
    assert_eq!(options.name(), crate::generic::DEFAULT_ROOT_NAME);
    assert_eq!(options.dtype(), None);
    assert!(options.metadata().is_empty());
    let message = options.require_field().unwrap_err().to_string();
    assert!(message.contains("with_field"), "{message}");
    assert!(message.contains("with_dtype"), "{message}");

    // A field declares all three parts at once and builds back the same.
    options.set_field(declared.clone());
    assert_eq!(options.name(), "row");
    assert_eq!(options.dtype(), Some(declared.dtype()));
    assert!(options.metadata().is_empty());
    assert_eq!(options.field(), Some(declared.clone()));
    assert_eq!(options.require_field().unwrap(), declared);

    // Declaring the datatype alone spells the same declaration: one stored
    // form, so the two compare and hash equal.
    let by_dtype = IpcOptions::new().with_dtype(declared.dtype().clone());
    assert_eq!(options, by_dtype);
    assert_eq!(
        RecordOptions::Ipc(options.clone()).stable_hash(),
        RecordOptions::Ipc(by_dtype).stable_hash()
    );

    // Each part mutates alone and the next build reflects it.
    options.set_name("trade".into());
    assert_eq!(options.name(), "trade");
    assert_eq!(options.field().unwrap().name(), "trade");
    assert_eq!(options.field().unwrap().dtype(), declared.dtype());

    let metadata = crate::Metadata::from_entries([("source", "test")]).unwrap();
    options.set_metadata(metadata.clone());
    assert_eq!(options.metadata(), &metadata);
    assert_eq!(
        options.field().unwrap().get_metadata("source"),
        Some("test")
    );
    assert!(!options.field().unwrap().is_nullable());

    let widened = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])
    .unwrap();
    options.set_dtype(Some(widened.clone()));
    let built = options.field().unwrap();
    assert_eq!(built.name(), "trade");
    assert_eq!(built.dtype(), &widened);
    assert_eq!(built.get_metadata("source"), Some("test"));

    // A declared field's nullability is not part of the declaration: the
    // build is the non-null row root.
    let nullable = IpcOptions::new().with_field(declared.clone().with_nullable(true));
    assert!(!nullable.field().unwrap().is_nullable());
    assert_eq!(nullable, IpcOptions::new().with_field(declared.clone()));

    // Taking the field clears the datatype and metadata; the name still names
    // the root a delegated write infers.
    let mut taken = options.clone();
    assert_eq!(taken.take_field(), Some(built.clone()));
    assert!(taken.field().is_none());
    assert_eq!(taken.dtype(), None);
    assert!(taken.metadata().is_empty());
    assert_eq!(taken.name(), "trade");
    assert_eq!(taken.take_field(), None);

    // Without a datatype there is nothing to build, whatever else is set, and
    // taking still clears the metadata that would otherwise resurface.
    let mut named = IpcOptions::new().with_name("trade").with_metadata(metadata);
    assert!(named.field().is_none());
    assert_eq!(named.take_field(), None);
    assert!(named.metadata().is_empty());

    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let options = RecordOptions::for_media_type(&media_type)
        .unwrap()
        .with_field(declared.clone());
    assert_eq!(options.field(), Some(declared.clone()));

    let RecordOptions::Ipc(inner) = options else {
        panic!("an arrows handle names the IPC encoding");
    };
    assert_eq!(inner.name, "row");
    assert_eq!(inner.dtype.as_ref(), Some(declared.dtype()));
    assert!(inner.metadata.is_empty());
}

#[test]
fn record_options_have_complete_value_traits_and_stable_hashes() {
    fn assert_traits<T: Clone + Eq + Ord + std::hash::Hash>(_: &T) {}

    let text = crate::text::TextOptions::new()
        .try_with_rowheader(r"^(?<id>\d+)")
        .unwrap();
    let options = RecordOptions::from(text.clone());
    let equal = options.clone();
    let changed = RecordOptions::from(text.clone().with_batch_row_size(3));

    assert_traits(&text);
    assert_traits(&options);
    assert_eq!(options, equal);
    assert_eq!(options.stable_hash(), equal.stable_hash());
    assert_ne!(options, changed);
    assert_ne!(options.stable_hash(), changed.stable_hash());
}

#[test]
fn a_zero_row_limit_reads_the_schema_and_no_batches() {
    let options = IpcOptions::new().with_max_row_size(0);
    let mut limited = options.limit_arrow_reader(reader(3, 2)).unwrap();

    // The schema still answers - `Some(0)` is a valid ask, not an error.
    assert_eq!(
        limited.schema(),
        crate::arrow::arrow_schema_from_field(&schema()).unwrap()
    );
    assert!(limited.next().is_none());
}

#[test]
fn a_zero_byte_limit_reads_the_schema_and_no_batches() {
    let options = IpcOptions::new().with_max_byte_size(0);
    let mut limited = options.limit_arrow_reader(reader(3, 2)).unwrap();

    assert_eq!(
        limited.schema(),
        crate::arrow::arrow_schema_from_field(&schema()).unwrap()
    );
    assert!(limited.next().is_none());
}

#[test]
fn a_row_limit_at_exactly_the_stored_count_keeps_every_row() {
    let options = IpcOptions::new().with_max_row_size(6);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

#[test]
fn a_row_limit_one_below_the_stored_count_slices_the_last_batch() {
    let options = IpcOptions::new().with_max_row_size(5);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(reader(3, 2))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // The bound is exact: the last batch is cut mid-way, not dropped whole.
    assert_eq!(
        batches
            .iter()
            .map(RecordBatch::num_rows)
            .collect::<Vec<_>>(),
        [2, 2, 1]
    );
}

#[test]
fn a_row_limit_one_above_the_stored_count_changes_nothing() {
    let options = IpcOptions::new().with_max_row_size(7);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}

#[test]
fn a_byte_limit_landing_mid_batch_slices_a_view_of_the_last_batch() {
    let source = batch(0..8);
    let size = u64::try_from(source.get_array_memory_size()).unwrap();
    // One byte short of the whole batch: the last row no longer fits, and the
    // seven that do are handed out as a slice.
    let options = IpcOptions::new().with_max_byte_size(size - 1);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(crate::arrow::batch_reader(
            source.schema(),
            [source.clone()],
        ))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 7);
    // A slice is a view: the cut batch's column still points at the source's
    // buffer rather than at a copy of it.
    assert_eq!(
        batches[0].column(0).to_data().buffers()[0].as_ptr(),
        source.column(0).to_data().buffers()[0].as_ptr()
    );
}

#[test]
fn a_nonzero_byte_limit_smaller_than_one_row_still_yields_one_row() {
    let options = IpcOptions::new().with_max_byte_size(1);
    let batches: Vec<RecordBatch> = options
        .limit_arrow_reader(reader(2, 4))
        .unwrap()
        .map(Result::unwrap)
        .collect();

    // A bounded read must never be a silent total loss: only `Some(0)` yields
    // nothing.
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
}

#[test]
fn whichever_limit_binds_first_wins() {
    // The byte budget admits everything, so the row bound is what cuts.
    let generous = IpcOptions::new()
        .with_max_row_size(3)
        .with_max_byte_size(u64::MAX);
    assert_eq!(rows(generous.limit_arrow_reader(reader(3, 2)).unwrap()), 3);

    // One byte admits one row, so the byte bound cuts before the row bound.
    let tight = IpcOptions::new().with_max_row_size(3).with_max_byte_size(1);
    assert_eq!(rows(tight.limit_arrow_reader(reader(3, 2)).unwrap()), 1);
}

/// A reader counting how often its source is pulled.
struct Counting {
    inner: BatchReader,
    pulls: Arc<std::sync::atomic::AtomicUsize>,
}

impl Iterator for Counting {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.pulls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.next()
    }
}

impl RecordBatchReader for Counting {
    fn schema(&self) -> SchemaRef {
        self.inner.schema()
    }
}

#[test]
fn a_satisfied_limit_stops_pulling_the_inner_reader() {
    let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Box::new(Counting {
        inner: reader(20, 2),
        pulls: Arc::clone(&pulls),
    });
    let options = IpcOptions::new().with_max_row_size(10);
    let mut limited = options.limit_arrow_reader(counted).unwrap();

    // At most one batch is retained: every yield costs exactly one pull, so
    // nothing is prefetched or buffered behind the caller's back.
    for served in 1..=5 {
        assert_eq!(limited.next().unwrap().unwrap().num_rows(), 2);
        assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), served);
    }

    // The tenth row satisfied the limit, so the source is never touched
    // again - fifteen batches of the file go undecoded.
    assert!(limited.next().is_none());
    assert!(limited.next().is_none());
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 5);
}

#[test]
fn a_limit_with_a_match_key_is_refused_naming_both_settings() {
    let options = IpcOptions::new()
        .with_max_row_size(10)
        .with_merge_by_names(["id"]);
    let Err(error) = options.limit_arrow_reader(reader(1, 2)) else {
        panic!("a limited merge must be refused");
    };
    let message = error.to_string();

    assert!(message.contains("max_row_size = 10"), "{message}");
    assert!(message.contains("merge_by_names [\"id\"]"), "{message}");
}

#[test]
fn an_empty_batch_passes_through_without_ending_the_stream() {
    let empty = RecordBatch::new_empty(crate::arrow::arrow_schema_from_field(&schema()).unwrap());
    let source = crate::arrow::batch_reader(empty.schema(), [empty, batch(0..2)]);
    let options = IpcOptions::new().with_max_row_size(2);
    assert_eq!(rows(options.limit_arrow_reader(source).unwrap()), 2);
}

#[test]
fn the_enum_mirrors_the_limits_of_the_encoding_it_holds() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let options = RecordOptions::for_media_type(&media_type)
        .unwrap()
        .with_max_row_size(7)
        .with_max_byte_size(1024);

    assert_eq!(options.max_row_size(), Some(7));
    assert_eq!(options.max_byte_size(), Some(1024));
    let RecordOptions::Ipc(inner) = options else {
        panic!("an arrows handle names the IPC encoding");
    };
    assert_eq!(inner.max_row_size, Some(7));
    assert_eq!(inner.max_byte_size, Some(1024));
}

#[test]
fn record_options_preflight_each_write_intent_without_an_input_reader() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let plain = RecordOptions::for_media_type(&media_type).unwrap();

    plain.require_write_mode(crate::IOMode::Overwrite).unwrap();
    plain.require_write_mode(crate::IOMode::Append).unwrap();
    let merge = plain
        .require_write_mode(crate::IOMode::Merge)
        .unwrap_err()
        .to_string();
    assert!(merge.contains("write mode merge requires"), "{merge}");
    assert!(merge.contains("$.merge_by_names"), "{merge}");

    let keyed = plain.with_merge_by_names(["id"]);
    keyed.require_write_mode(crate::IOMode::Merge).unwrap();
    for refused in [
        keyed.require_write_mode(crate::IOMode::Overwrite),
        keyed.require_write_mode(crate::IOMode::Append),
    ] {
        let message = refused.unwrap_err().to_string();
        assert!(
            message.contains("does not accept merge_by_names"),
            "{message}"
        );
        assert!(message.contains("use merge mode"), "{message}");
    }
}

#[test]
fn every_concrete_options_type_carries_the_same_commit_cadence() {
    fn assert_cadence(mut options: impl IORecordOptions) {
        assert_eq!(options.commit_row_size(), None);
        options.set_commit_row_size(Some(17));
        assert_eq!(options.commit_row_size(), Some(17));
    }

    assert_cadence(IpcOptions::new());
    assert_cadence(crate::avro::AvroOptions::new());
    assert_cadence(crate::text::TextOptions::new());
    #[cfg(feature = "parquet")]
    assert_cadence(crate::parquet::ParquetOptions::new());

    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(5);
    assert_eq!(options.commit_row_size(), Some(5));
    let RecordOptions::Ipc(inner) = options else {
        unreachable!()
    };
    assert_eq!(inner.commit_row_size, Some(5));
}

#[test]
fn avro_only_options_validate_codec_and_sync_marker_in_the_core() {
    let media_type = Url::from_str("file:///t.avro").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(options.avro_block_codec(), Some("deflate"));
    assert_eq!(options.avro_sync_marker(), None);

    options.set_avro_block_codec("null").unwrap();
    assert_eq!(options.avro_block_codec(), Some("null"));
    let marker = *b"0123456789abcdef";
    options.set_avro_sync_marker(Some(&marker)).unwrap();
    assert_eq!(options.avro_sync_marker(), Some(&marker));

    let mut handle = Buffer::new().with_media_type(media_type);
    handle.overwrite_arrow_batch(batch(0..2), &options).unwrap();
    assert!(handle.as_slice().ends_with(&marker));
    assert_eq!(rows(handle.read_arrow_reader(&options).unwrap()), 2);

    options.set_avro_sync_marker(None).unwrap();
    assert_eq!(options.avro_sync_marker(), None);

    let codec = options.set_avro_block_codec("brotli").unwrap_err();
    assert!(matches!(codec, crate::Error::Codec { format: "avro", .. }));
    assert!(codec.to_string().contains("brotli"));

    let length = options.set_avro_sync_marker(Some(b"short")).unwrap_err();
    assert!(matches!(length, crate::Error::InvalidRecord { .. }));
    let message = length.to_string();
    assert!(message.contains("$.sync_marker"), "{message}");
    assert!(message.contains("16 bytes"), "{message}");
    assert!(message.contains("got 5"), "{message}");
}

#[test]
fn avro_only_setters_reject_another_inferred_encoding() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(options.avro_block_codec(), None);
    assert_eq!(options.avro_sync_marker(), None);
    for error in [
        options.set_avro_block_codec("null").unwrap_err(),
        options.set_avro_sync_marker(None).unwrap_err(),
    ] {
        assert!(matches!(error, crate::Error::InvalidRecord { .. }));
        let message = error.to_string();
        assert!(message.contains("Avro"), "{message}");
        assert!(message.contains("arrow.stream"), "{message}");
    }
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_only_options_are_owned_by_the_generic_core_variant() {
    let media_type = Url::from_str("file:///t.parquet").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert_eq!(
        options.parquet_compression_name().as_deref(),
        Some("zstd(1)")
    );
    options.set_parquet_compression_name("gzip(4)").unwrap();
    assert_eq!(
        options.parquet_compression_name().as_deref(),
        Some("gzip(4)")
    );

    options.set_parquet_max_row_group_size(17).unwrap();
    assert_eq!(options.parquet_max_row_group_size(), Some(17));
    options
        .set_parquet_key_value_metadata(vec![("source".into(), "test".into())])
        .unwrap();
    options.push_parquet_key_value("version", "1").unwrap();
    assert_eq!(
        options.parquet_key_value_metadata().unwrap(),
        [
            ("source".into(), "test".into()),
            ("version".into(), "1".into())
        ]
    );
}

#[cfg(feature = "parquet")]
#[test]
fn parquet_only_setters_reject_another_inferred_encoding() {
    let media_type = Url::from_str("file:///t.arrows").unwrap().media_type();
    let mut options = RecordOptions::for_media_type(&media_type).unwrap();

    assert!(options.parquet_compression_name().is_none());
    assert!(options.parquet_max_row_group_size().is_none());
    assert!(options.parquet_key_value_metadata().is_none());
    for error in [
        options.set_parquet_compression_name("snappy").unwrap_err(),
        options.set_parquet_max_row_group_size(17).unwrap_err(),
        options
            .set_parquet_key_value_metadata(Vec::new())
            .unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(message.contains("Parquet"), "{message}");
        assert!(message.contains("arrow.stream"), "{message}");
    }
}

#[test]
fn native_batch_writes_stop_at_the_smaller_conversion_or_commit_bound() {
    assert_eq!(crate::generic::DEFAULT_RECORD_BATCH_ROW_SIZE, 65_536);
    assert_eq!(
        crate::arrow::rows::DEFAULT_BATCH_ROW_SIZE,
        crate::generic::DEFAULT_RECORD_BATCH_ROW_SIZE
    );
    let default = IpcOptions::new().with_commit_row_size(usize::MAX);
    assert_eq!(
        default.write_batch_row_size(),
        Some(crate::arrow::rows::DEFAULT_BATCH_ROW_SIZE)
    );
    assert_eq!(
        IpcOptions::new()
            .with_batch_row_size(7)
            .with_commit_row_size(11)
            .write_batch_row_size(),
        Some(7)
    );
    assert_eq!(
        IpcOptions::new()
            .with_batch_row_size(11)
            .with_commit_row_size(7)
            .write_batch_row_size(),
        Some(7)
    );
    assert_eq!(IpcOptions::new().write_batch_row_size(), None);
}

#[test]
fn zero_commit_row_size_is_a_typed_preflight_error() {
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(0);
    let message = options.require_commit_row_size().unwrap_err().to_string();
    assert!(message.contains("$.commit_row_size"), "{message}");
    assert!(message.contains("non-zero"), "{message}");
    assert!(message.contains("got 0"), "{message}");
}

#[test]
fn commit_readers_slice_exact_cadences_across_batch_boundaries() {
    let schema = crate::arrow::arrow_schema_from_field(&schema()).unwrap();
    let source =
        crate::arrow::batch_reader(Arc::clone(&schema), [batch(0..2), batch(2..6), batch(6..7)]);
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(3);
    let commits = options
        .commit_arrow_readers(source)
        .unwrap()
        .map(|commit| rows(commit.unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(commits, [3, 3, 1]);
}

#[test]
fn a_commit_larger_than_the_stream_yields_one_final_remainder() {
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(20);
    let commits = options
        .commit_arrow_readers(reader(3, 2))
        .unwrap()
        .map(|commit| rows(commit.unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(commits, [6]);
}

#[test]
fn a_full_commit_does_not_read_ahead() {
    let pulls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = Box::new(Counting {
        inner: reader(2, 4),
        pulls: Arc::clone(&pulls),
    });
    let options = RecordOptions::Ipc(IpcOptions::new()).with_commit_row_size(2);
    let mut commits = options.commit_arrow_readers(counted).unwrap();

    assert_eq!(rows(commits.next().unwrap().unwrap()), 2);
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 1);
    // The next cadence is the unconsumed slice of that same input batch.
    assert_eq!(rows(commits.next().unwrap().unwrap()), 2);
    assert_eq!(pulls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn a_reader_without_limits_is_returned_as_it_stands() {
    // The default options carry no bounds, so the seam costs nothing when
    // unused: six rows flow untouched, exactly as the source yields them.
    let options = IpcOptions::new();
    assert_eq!(options.max_row_size(), None);
    assert_eq!(options.max_byte_size(), None);
    assert_eq!(rows(options.limit_arrow_reader(reader(3, 2)).unwrap()), 6);
}
