//! Parquet round trips, field-id preservation, and footer statistics.

use std::hash::Hash;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use arrow_array::{Int64Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::ArrowError;
use parquet::basic::Compression;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::parquet::{Parquet, ParquetOptions};
use yggdryl::{DataType, Field, MediaType, StructType, Url};
use yggdryl::{IOBase, IOMedia};

#[test]
fn statistics_snapshots_have_total_value_traits() {
    fn assert_traits<T: Clone + Eq + Hash + Ord>() {}
    assert_traits::<yggdryl::parquet::GeospatialStatistics>();
    assert_traits::<yggdryl::parquet::ColumnStatistics>();
    assert_traits::<yggdryl::parquet::RowGroupStatistics>();
    assert_traits::<yggdryl::parquet::FileStatistics>();
    assert_traits::<ParquetOptions>();

    let mut gzip = ParquetOptions::new();
    gzip.set_compression_name("gzip(2)").unwrap();
    let mut other_level = ParquetOptions::new();
    other_level.set_compression_name("gzip(3)").unwrap();
    assert_ne!(gzip, other_level);
    assert!(gzip < other_level);
}

/// Two independent handles over one in-memory byte value, used to exercise
/// opened-session freshness without involving filesystem mappings.
#[derive(Clone, Debug)]
struct Shared {
    handle: Arc<Mutex<Buffer>>,
    media_type: MediaType,
}

impl Shared {
    fn new(handle: Buffer) -> Self {
        let media_type = handle.media_type().clone();
        Self {
            handle: Arc::new(Mutex::new(handle)),
            media_type,
        }
    }
}

impl yggdryl::IOMedia for Shared {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for Shared {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
        self.handle.lock().unwrap().pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> yggdryl::Result<usize> {
        self.handle.lock().unwrap().pwrite(offset, bytes)
    }

    fn size(&self) -> u64 {
        self.handle.lock().unwrap().size()
    }

    fn capacity(&self) -> u64 {
        self.handle.lock().unwrap().capacity()
    }

    fn reserve(&mut self, capacity: u64) -> yggdryl::Result<()> {
        self.handle.lock().unwrap().reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> yggdryl::Result<()> {
        self.handle.lock().unwrap().truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        None
    }

    fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.handle
            .lock()
            .unwrap()
            .set_media_type(media_type.clone());
        self.media_type = media_type;
    }
}

/// A root carrying explicit Iceberg-style field identifiers.
fn root() -> Field {
    StructType::from_fields([
        DataType::Int64
            .required_field("id")
            .with_parquet_field_id(1),
        DataType::utf8()
            .nullable_field("symbol")
            .with_parquet_field_id(2),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

fn batch(field: &Field, ids: Vec<i64>, symbols: Vec<Option<&str>>) -> RecordBatch {
    let schema = field.clone().into_arrow_schema().unwrap();
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
    .unwrap()
}

/// The batches a write takes: one reader over the batches given.
fn reader<I>(field: &Field, batches: I) -> yggdryl::arrow::BatchReader
where
    I: IntoIterator<Item = RecordBatch>,
    I::IntoIter: Send + 'static,
{
    yggdryl::arrow::batch_reader(field.clone().into_arrow_schema().unwrap(), batches)
}

/// A one-batch reader that reports whether a write pulled its input.
fn counted_reader(field: &Field, pulls: Arc<AtomicUsize>) -> yggdryl::arrow::BatchReader {
    let batch = batch(field, vec![1], vec![Some("AAPL")]);
    let batches = std::iter::once(batch).inspect(move |_| {
        pulls.fetch_add(1, Ordering::Relaxed);
    });
    reader(field, batches)
}

fn reader_then_error(field: &Field, first: RecordBatch) -> yggdryl::arrow::BatchReader {
    Box::new(RecordBatchIterator::new(
        [
            Ok(first),
            Err(ArrowError::ComputeError(
                "later Parquet source failure".into(),
            )),
        ],
        field.clone().into_arrow_schema().unwrap(),
    ))
}

/// A handle whose media type comes from the name, so codings are declared.
fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

#[test]
fn batches_round_trip_through_storage() {
    let field = root();
    let mut media = Parquet::new(handle("trades.parquet"));
    let expected = batch(
        &field,
        vec![1, 2, 3],
        vec![Some("AAPL"), None, Some("MSFT")],
    );
    let options = media.record_options().unwrap();

    media
        .overwrite_arrow_reader(reader(&field, [expected.clone()]), &options)
        .unwrap();

    let actual = media
        .read_arrow_reader(&options)
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0], expected);
    assert_eq!(actual[0].num_rows(), 3);
}

#[test]
fn dimensions_describe_all_batches_and_ignore_read_options() {
    let field = root();
    let mut media = Parquet::new(handle("dimensions.parquet"));
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(
                &field,
                [
                    batch(&field, vec![1, 2], vec![Some("AAPL"), None]),
                    batch(
                        &field,
                        vec![3, 4, 5],
                        vec![Some("MSFT"), Some("NVDA"), None],
                    ),
                ],
            ),
            &options,
        )
        .unwrap();
    media.options_mut().set_max_row_size(Some(1));
    media.options_mut().set_select("id".parse().unwrap());
    media
        .options_mut()
        .set_filter("id = '999'".parse().unwrap());

    assert_eq!(media.row_size().unwrap(), 5);
    assert_eq!(media.column_size().unwrap(), 2);
}

#[test]
fn an_empty_open_parquet_file_has_explicit_lifecycle_and_dimensions() {
    let field = root();
    let mut media = Parquet::new(handle("empty-open.parquet")).with_field(field.clone());

    media.open().unwrap();
    assert!(media.opened());
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), field.field_len());

    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(
                &field,
                [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
            ),
            &options,
        )
        .unwrap();
    assert!(media.opened());
    assert_eq!(media.row_size().unwrap(), 2);

    media.clear().unwrap();
    assert!(media.opened());
    assert_eq!(media.row_size().unwrap(), 0);
    assert_eq!(media.column_size().unwrap(), field.field_len());

    let narrowed = StructType::from_fields([DataType::Int64.required_field("id")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    media.options_mut().set_field(narrowed);
    assert_eq!(
        media.column_size().unwrap(),
        1,
        "an option mutation invalidates the opened width cache"
    );

    media.remove(false).unwrap();
    assert!(!media.opened(), "removal ends the opened session");
}

#[test]
fn an_open_parquet_cache_is_stable_until_close_then_reads_fresh() {
    let field = root();
    let encoded = |ids: Vec<i64>| {
        let symbols = vec![None; ids.len()];
        let mut media = Parquet::new(handle("shared.parquet"));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(reader(&field, [batch(&field, ids, symbols)]), &options)
            .unwrap();
        media.into_handle()
    };
    let first = encoded(vec![1]);
    let replacement = encoded(vec![1, 2, 3, 4]);
    let shared = Shared::new(first);
    let mut external = shared.clone();
    let mut media = Parquet::new(shared);

    media.open().unwrap();
    assert_eq!(media.row_size().unwrap(), 1);
    external.write_all_bytes(replacement.as_slice()).unwrap();
    assert_eq!(media.row_size().unwrap(), 1, "the open footer is stable");

    media.close().unwrap();
    assert!(!media.opened());
    assert_eq!(media.row_size().unwrap(), 4, "closed reads are fresh");
}

#[test]
fn the_wrapper_owns_parquet_options_over_an_unnamed_buffer() {
    let field = root();
    let media = Parquet::new(Buffer::new()).with_field(field.clone());
    let options = media.record_options().unwrap();

    assert!(matches!(options, RecordOptions::Parquet(_)));
    assert_eq!(options.field(), Some(field));
}

#[test]
fn mismatched_options_are_rejected_before_any_write_pulls_input() {
    let field = root();
    for operation in ["overwrite", "append", "merge"] {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut media = Parquet::new(Buffer::new()).with_field(field.clone());
        let mut options = RecordOptions::Ipc(yggdryl::ipc::IpcOptions::new());
        if operation == "merge" {
            options.set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
        }
        let result = match operation {
            "overwrite" => yggdryl::IOMedia::overwrite_arrow_reader(
                &mut media,
                counted_reader(&field, Arc::clone(&pulls)),
                &options,
            ),
            "append" => yggdryl::IOMedia::append_arrow_reader(
                &mut media,
                counted_reader(&field, Arc::clone(&pulls)),
                &options,
            ),
            "merge" => yggdryl::IOMedia::merge_arrow_reader(
                &mut media,
                counted_reader(&field, Arc::clone(&pulls)),
                &options,
            ),
            _ => unreachable!(),
        };

        let message = result.unwrap_err().to_string();
        assert!(message.contains("Parquet"), "{operation}: {message}");
        assert_eq!(pulls.load(Ordering::Relaxed), 0, "{operation}");
        assert!(media.handle().is_empty(), "{operation}");
    }
}

#[test]
fn an_open_footer_tracks_selection_and_completion_on_overwrite() {
    let field = root();
    let mut media = Parquet::new(Buffer::new());
    let initial_options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(&field, [batch(&field, vec![1], vec![Some("AAPL")])]),
            &initial_options,
        )
        .unwrap();
    media.open().unwrap();

    let options = media.record_options().unwrap().with_select("id").unwrap();
    yggdryl::IOMedia::overwrite_arrow_reader(
        &mut media,
        reader(
            &field,
            [batch(&field, vec![2, 3], vec![Some("MSFT"), Some("NVDA")])],
        ),
        &options,
    )
    .unwrap();

    assert!(media.opened());
    assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 2);
    assert_eq!(media.read_statistics().unwrap().num_rows, 2);
    let read_options = media.record_options().unwrap();
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
fn an_open_footer_is_refreshed_by_every_successful_write_mode() {
    let field = root();
    let mut media = Parquet::new(Buffer::new()).with_field(field.clone());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(
                &field,
                [batch(&field, vec![1, 2], vec![Some("AAPL"), Some("MSFT")])],
            ),
            &options,
        )
        .unwrap();
    assert!(!media.opened(), "a closed write must not start a cache");

    media.open().unwrap();
    assert!(media.opened());
    assert_eq!(media.read_statistics().unwrap().num_rows, 2);
    media.options_mut().set_commit_row_size(Some(1));

    let overwrite_options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(&field, [batch(&field, vec![3], vec![Some("NVDA")])]),
            &overwrite_options,
        )
        .unwrap();
    assert!(media.opened());
    assert_eq!(media.read_statistics().unwrap().num_rows, 1);

    let append_options = media.record_options().unwrap();
    media
        .append_arrow_reader(
            reader(&field, [batch(&field, vec![4, 5], vec![Some("AMD"), None])]),
            &append_options,
        )
        .unwrap();
    assert!(media.opened());
    assert_eq!(media.read_statistics().unwrap().num_rows, 3);

    media
        .options_mut()
        .set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
    let merge_options = media.record_options().unwrap();
    media
        .merge_arrow_reader(
            reader(
                &field,
                [batch(&field, vec![4, 6], vec![Some("INTC"), Some("ARM")])],
            ),
            &merge_options,
        )
        .unwrap();
    assert!(media.opened());
    assert_eq!(media.read_statistics().unwrap().num_rows, 4);

    media.close().unwrap();
    assert!(!media.opened());
    assert_eq!(media.read_statistics().unwrap().num_rows, 4);
}

#[test]
fn a_partial_commit_refreshes_the_open_parquet_footer() {
    let field = root();
    let mut media = Parquet::new(Buffer::new()).with_field(field.clone());
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(
                &field,
                [batch(&field, vec![1, 2], vec![Some("AAPL"), Some("MSFT")])],
            ),
            &options,
        )
        .unwrap();
    media.open().unwrap();
    media.options_mut().set_commit_row_size(Some(1));

    let options = media.record_options().unwrap();
    let message = media
        .overwrite_arrow_reader(
            reader_then_error(&field, batch(&field, vec![7], vec![Some("NVDA")])),
            &options,
        )
        .unwrap_err()
        .to_string();

    assert!(
        message.contains("later Parquet source failure"),
        "{message}"
    );
    assert!(media.opened());
    assert_eq!(media.read_statistics().unwrap().num_rows, 1);
    assert_eq!(media.row_size().unwrap(), 1);
    assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 2);
}

#[test]
fn field_identifiers_survive_the_round_trip() {
    let field = root();
    let mut media = Parquet::new(handle("ids.parquet"));
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(
            reader(&field, [batch(&field, vec![1], vec![Some("AAPL")])]),
            &options,
        )
        .unwrap();

    // Ids are what an Iceberg reader resolves columns by, so they must not be
    // positional after a round trip.
    let schema = media.read_arrow_schema().unwrap();
    assert_eq!(
        schema.field(0).metadata().get("PARQUET:field_id"),
        Some(&"1".to_owned())
    );
    assert_eq!(
        schema.field(1).metadata().get("PARQUET:field_id"),
        Some(&"2".to_owned())
    );

    let recovered = media.read_arrow_field(&options).unwrap();
    let fields = recovered.dtype().as_fields().unwrap();
    assert_eq!(fields[0].parquet_field_id().unwrap(), Some(1));
    assert_eq!(fields[1].parquet_field_id().unwrap(), Some(2));
}

#[test]
fn an_empty_write_still_publishes_a_readable_file() {
    let field = root();
    let mut media = Parquet::new(handle("empty.parquet"));
    let options = media.record_options().unwrap();

    media
        .overwrite_arrow_reader(reader(&field, []), &options)
        .unwrap();

    assert!(!media.handle().is_empty());
    assert!(
        media
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>()
            .is_empty()
    );
    assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 2);
    assert_eq!(media.read_statistics().unwrap().num_rows, 0);
}

#[test]
fn a_coded_location_is_rejected_with_the_reason() {
    let field = root();

    for name in ["trades.parquet.gz", "trades.parquet.zst"] {
        let mut media = Parquet::new(handle(name));
        let options = media.record_options().unwrap();
        let message = media
            .overwrite_arrow_reader(reader(&field, []), &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("compresses"), "{name}: {message}");
        assert!(
            message.contains("ParquetOptions::compression"),
            "{name}: {message}"
        );
        // Nothing was published.
        assert!(media.handle().is_empty(), "{name}");
    }
}

#[test]
fn a_mismatched_batch_reports_which_index_disagreed() {
    let field = root();
    let other = StructType::from_fields([DataType::utf8().required_field("unrelated")])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
    let mut media = Parquet::new(handle("mismatch.parquet"));
    let options = media.record_options().unwrap();

    let good = batch(&field, vec![1], vec![Some("AAPL")]);
    let bad = RecordBatch::try_new(
        other.into_arrow_schema().unwrap(),
        vec![Arc::new(StringArray::from(vec!["x"]))],
    )
    .unwrap();

    let message = media
        .overwrite_arrow_reader(reader(&field, [good, bad]), &options)
        .unwrap_err()
        .to_string();
    assert!(message.contains("index 1"), "{message}");
    assert!(media.handle().is_empty());
}

#[test]
fn every_compression_round_trips_and_changes_the_bytes() {
    let field = root();
    // A payload with structure so compression has something to remove.
    let ids: Vec<i64> = (0..4_000).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let source = batch(&field, ids, symbols);

    let mut sizes = Vec::new();
    for (name, compression) in [
        ("none.parquet", Compression::UNCOMPRESSED),
        ("snappy.parquet", Compression::SNAPPY),
        ("zstd.parquet", Compression::ZSTD(Default::default())),
    ] {
        // Read the whole file as one batch so the comparison is not split by
        // the reader's default batch size.
        let mut media = Parquet::new(handle(name)).with_options(
            ParquetOptions::new()
                .with_compression(compression)
                .with_batch_row_size(source.num_rows()),
        );
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(reader(&field, [source.clone()]), &options)
            .unwrap_or_else(|error| panic!("{name}: {error}"));

        let actual = media
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), 1, "{name}");
        assert_eq!(actual[0], source, "{name}");
        sizes.push(media.handle().size() as usize);
    }

    // Compression is recovered from the footer, so every file reads back the
    // same rows while the uncompressed one is the largest.
    assert!(sizes[0] > sizes[1], "{sizes:?}");
    assert!(sizes[0] > sizes[2], "{sizes:?}");
}

#[test]
fn footer_statistics_expose_row_groups_bounds_and_split_offsets() {
    let field = root();
    let ids: Vec<i64> = (0..2_048).collect();
    let symbols: Vec<Option<&str>> = ids
        .iter()
        .map(|index| (index % 2 == 0).then_some("AAPL"))
        .collect();

    let mut media = Parquet::new(handle("stats.parquet")).with_options(
        // Force several row groups so the statistics have something to say.
        ParquetOptions::new()
            .with_max_row_group_size(512)
            .with_key_value("iceberg.schema-id", "7"),
    );
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(&field, [batch(&field, ids, symbols)]), &options)
        .unwrap();

    let statistics = media.read_statistics().unwrap();
    assert_eq!(statistics.num_rows, 2_048);
    assert_eq!(statistics.row_groups.len(), 4);
    assert!(statistics.created_by.is_some());
    assert!(
        statistics
            .key_value_metadata
            .iter()
            .any(|(key, value)| key == "iceberg.schema-id" && value == "7"),
        "{:?}",
        statistics.key_value_metadata
    );

    // Half the symbols are null across the whole file.
    assert_eq!(statistics.null_count("symbol"), Some(1_024));
    assert_eq!(statistics.null_count("id"), Some(0));
    assert_eq!(statistics.null_count("absent"), None);

    // Bounds are recorded per column chunk.
    let first = &statistics.row_groups[0];
    assert_eq!(first.num_rows, 512);
    assert!(first.compressed_size > 0);
    assert!(first.columns.iter().any(|column| column.path == "id"));
    assert!(
        first
            .columns
            .iter()
            .any(|column| column.min_bytes.is_some() && column.max_bytes.is_some())
    );

    // Split offsets are what an Iceberg manifest records per data file.
    let offsets = statistics.split_offsets();
    assert_eq!(offsets.len(), statistics.row_groups.len());
    assert!(
        offsets.windows(2).all(|pair| pair[0] < pair[1]),
        "{offsets:?}"
    );
}

#[test]
fn generic_statistics_redirect_validates_the_handle_encoding() {
    let field = root();
    let mut parquet = handle("redirect.parquet");
    let options = parquet.record_options().unwrap().with_field(field.clone());
    parquet
        .overwrite_arrow_reader(
            reader(&field, [batch(&field, vec![1], vec![Some("AAPL")])]),
            &options,
        )
        .unwrap();

    assert_eq!(parquet.read_parquet_statistics().unwrap().num_rows, 1);

    let ipc = handle("redirect.arrows");
    let error = ipc.read_parquet_statistics().unwrap_err();
    assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
    let message = error.to_string();
    assert!(message.contains("expected Parquet media"), "{message}");
    assert!(
        message.contains("application/vnd.apache.arrow.stream"),
        "{message}"
    );

    let error = ipc.read_parquet_geospatial_statistics("shape").unwrap_err();
    assert!(matches!(error, yggdryl::Error::InvalidRecord { .. }));
    assert!(error.to_string().contains("expected Parquet media"));
}

#[test]
fn a_bounded_batch_row_size_splits_the_read() {
    let field = root();
    let mut media = Parquet::new(handle("batched.parquet"))
        .with_options(ParquetOptions::new().with_batch_row_size(256));
    let ids: Vec<i64> = (0..1_000).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let options = media.record_options().unwrap();
    media
        .overwrite_arrow_reader(reader(&field, [batch(&field, ids, symbols)]), &options)
        .unwrap();

    let batches = media
        .read_arrow_reader(&options)
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect::<Vec<_>>();
    assert!(batches.len() >= 4, "{}", batches.len());
    assert_eq!(
        batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
        1_000
    );
}

/// Column pushdown: a schema naming fewer columns becomes a projection mask,
/// which is the format's own way of not reading a column chunk.
mod pushdown {

    use std::sync::Arc;
    use yggdryl::StructType;

    use arrow_array::{
        Array, Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray,
    };

    use super::handle;
    use yggdryl::IOMedia;
    use yggdryl::media::IORecordOptions;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, Field};

    /// Four columns, so a two-column read is a genuine subset.
    fn wide() -> Field {
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::utf8().nullable_field("venue"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    /// The two columns a caller actually wants.
    fn narrow() -> Field {
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("price"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row")
    }

    /// A file wide enough that skipping two columns is measurable.
    fn stored() -> Parquet<yggdryl::holder::Buffer> {
        let rows = 4_096;
        let ids: Vec<i64> = (0..rows).collect();
        let batch = RecordBatch::try_new(
            wide().into_arrow_schema().unwrap(),
            vec![
                Arc::new(Int64Array::from(ids.clone())),
                Arc::new(StringArray::from(
                    ids.iter().map(|id| format!("SYM{id}")).collect::<Vec<_>>(),
                )),
                #[allow(clippy::cast_precision_loss)]
                Arc::new(Float64Array::from(
                    ids.iter().map(|id| *id as f64).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    ids.iter()
                        .map(|id| format!("VENUE{id}"))
                        .collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap();

        let mut media = Parquet::new(handle("pushdown.parquet"));
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        media
    }

    /// Total bytes every array in a read occupies, which is the data the read
    /// actually moved into Arrow memory.
    fn materialized(reader: yggdryl::arrow::BatchReader) -> usize {
        reader
            .map(std::result::Result::unwrap)
            .map(|batch| {
                batch
                    .columns()
                    .iter()
                    .map(|column| column.get_array_memory_size())
                    .sum::<usize>()
            })
            .sum()
    }

    #[test]
    fn a_subset_schema_is_pushed_into_the_file_rather_than_applied_after_it() {
        let media = stored();
        let options = media.record_options().unwrap();

        // The file stores four columns; nothing about it changed.
        assert_eq!(media.read_arrow_schema().unwrap().fields().len(), 4);
        assert_eq!(media.read_arrow_field(&options).unwrap().field_len(), 4);

        let options = options.with_field(narrow());
        let reader = media.read_arrow_reader(&options).unwrap();
        // The projection is known before a single batch is decoded.
        assert_eq!(reader.schema().fields().len(), 2);
        assert_eq!(reader.schema().field(0).name(), "id");
        assert_eq!(reader.schema().field(1).name(), "price");

        let batches = reader.map(std::result::Result::unwrap).collect::<Vec<_>>();
        assert!(!batches.is_empty());
        for batch in &batches {
            assert_eq!(batch.num_columns(), 2);
        }
        assert_eq!(
            batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
            4_096
        );
    }

    #[test]
    fn the_projected_read_materializes_less_than_the_whole_file() {
        let media = stored();
        let options = media.record_options().unwrap();

        let whole = materialized(media.read_arrow_reader(&options).unwrap());
        let subset = materialized(
            media
                .read_arrow_reader(&options.with_field(narrow()))
                .unwrap(),
        );

        // The two string columns are the bulk of this file, and a pushed-down
        // read never builds them.
        assert!(subset * 2 < whole, "subset {subset} bytes, whole {whole}");
    }

    #[test]
    fn a_column_the_file_does_not_store_is_completed_after_pushdown() {
        let media = stored();
        let options = media.record_options().unwrap();

        // A mask can only drop columns, so the encoding reads what is present
        // and the canonical declared-Field cast supplies the absent column.
        let invented = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("nowhere"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");

        let batches = media
            .read_arrow_reader(&options.with_field(invented))
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert!(!batches.is_empty());
        for batch in &batches {
            assert_eq!(batch.num_columns(), 2);
            assert_eq!(batch.schema().field(1).name(), "nowhere");
        }
        assert_eq!(
            batches.iter().map(RecordBatch::num_rows).sum::<usize>(),
            4_096
        );
        assert_eq!(
            batches
                .iter()
                .map(|batch| batch.column(1).null_count())
                .sum::<usize>(),
            4_096
        );
    }
}

mod limits {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::RecordBatchReader;
    use parquet::basic::Compression;

    use super::{batch, handle, reader, root};
    use yggdryl::holder::Buffer;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{IOBase, IOMedia};

    /// The total rows a handle yields under `options`.
    fn rows<H: IOBase + ?Sized>(handle: &H, options: &RecordOptions) -> usize {
        handle
            .read_arrow_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
    }

    #[test]
    fn a_zero_limit_reads_the_declared_schema_and_no_batches() {
        let field = root();
        let mut handle = handle("limited.parquet");
        let options = handle.record_options().unwrap().with_field(field.clone());
        handle
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
                ),
                &options,
            )
            .unwrap();

        let mut limited = handle
            .read_arrow_reader(&options.with_max_row_size(0))
            .unwrap();
        // The schema is asserted, not only the emptiness: `Some(0)` is a
        // valid ask that still says what the rows would have been.
        assert_eq!(limited.schema(), field.clone().into_arrow_schema().unwrap());
        assert!(limited.next().is_none());
    }

    #[test]
    fn a_limited_write_truncates_what_the_caller_offered() {
        let field = root();
        let mut handle = handle("truncated.parquet");
        let options = handle.record_options().unwrap().with_field(field.clone());

        handle
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
                ),
                &options.clone().with_max_row_size(1),
            )
            .unwrap();
        assert_eq!(rows(&handle, &options), 1);

        // An append is a write, so the same bound truncates it the same way.
        handle
            .append_arrow_reader(
                reader(&field, [batch(&field, vec![3, 4], vec![None, None])]),
                &options.clone().with_max_row_size(1),
            )
            .unwrap();
        assert_eq!(rows(&handle, &options), 2);
    }

    /// A handle counting the read calls and bytes that reach the one it
    /// wraps, so a test can say what a read actually fetched.
    struct Counting {
        handle: Buffer,
        reads: AtomicUsize,
        bytes: AtomicUsize,
    }

    impl Counting {
        fn new(handle: Buffer) -> Self {
            Self {
                handle,
                reads: AtomicUsize::new(0),
                bytes: AtomicUsize::new(0),
            }
        }

        /// One measured run: the reads and bytes `operation` costs.
        fn cost(&self, operation: impl FnOnce()) -> (usize, usize) {
            let reads = self.reads.load(Ordering::Relaxed);
            let bytes = self.bytes.load(Ordering::Relaxed);
            operation();
            (
                self.reads.load(Ordering::Relaxed) - reads,
                self.bytes.load(Ordering::Relaxed) - bytes,
            )
        }
    }

    impl yggdryl::IOMedia for Counting {
        yggdryl::impl_default_iomedia!();
    }

    impl IOBase for Counting {
        yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, parent, child_by_path,
            ls, kind, clear, remove, is_atomic, is_tabular);

        fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
            let read = self.handle.pread(offset, buffer)?;
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.bytes.fetch_add(read, Ordering::Relaxed);
            Ok(read)
        }
    }

    #[test]
    fn a_small_row_bound_over_many_row_groups_stops_reading_early() {
        // Thirty-two uncompressed row groups, so one group is a small
        // fraction of the file and the fraction shows up in bytes read.
        let field = root();
        let total = 16_384_usize;
        let mut media = Parquet::new(handle("grouped.parquet")).with_options(
            ParquetOptions::new()
                .with_compression(Compression::UNCOMPRESSED)
                .with_max_row_group_size(512),
        );
        let options = media.record_options().unwrap();
        media
            .overwrite_arrow_reader(
                reader(
                    &field,
                    [batch(
                        &field,
                        (0..total as i64).collect(),
                        vec![None; total],
                    )],
                ),
                &options,
            )
            .unwrap();
        assert_eq!(media.read_statistics().unwrap().row_groups.len(), 32);

        let counting = Counting::new(media.into_handle());
        let options = counting.record_options().unwrap();

        // The logical dimension is a two-range footer read: neither column
        // pages nor row arrays are fetched.
        let (dimension_reads, dimension_bytes) = counting.cost(|| {
            assert_eq!(counting.row_size().unwrap(), total as u64);
        });
        assert_eq!(dimension_reads, 2, "tail and footer metadata");
        assert!(
            dimension_bytes < counting.size() as usize,
            "{dimension_bytes} footer bytes vs {} file bytes",
            counting.size()
        );

        // The full drain fetches the complete value: one whole-value read.
        let (full_reads, full_bytes) = counting.cost(|| {
            assert_eq!(rows(&counting, &options), total);
        });
        assert_eq!(full_reads, 1, "one whole-value read");
        assert_eq!(full_bytes as u64, counting.size());

        // Five rows out of 16,384: the tail, the footer, and one leading
        // row-group prefix - the other thirty-one groups are never read.
        let (limited_reads, limited_bytes) = counting.cost(|| {
            assert_eq!(rows(&counting, &options.clone().with_max_row_size(5)), 5);
        });
        assert_eq!(limited_reads, 3, "tail, footer, one-group prefix");
        assert!(
            limited_bytes * 4 < full_bytes,
            "{limited_bytes} bytes under the bound vs {full_bytes} for the drain"
        );
    }
}
