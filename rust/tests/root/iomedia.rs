//! `rust/src/iomedia.rs`: the record surface derived from the byte trait - what
//! a read publishes, what a write takes, and the shape a handle answers.

use super::counting;

mod positional {

    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Field, Scalar, Url};

    #[test]
    fn structured_values_follow_the_declared_format_and_content_coding() {
        let expected = Scalar::from_struct([
            ("quantity", Scalar::from(2)),
            ("symbol", Scalar::from("AAPL")),
        ])
        .unwrap();

        for name in [
            "trade.json",
            "trade.json.gz",
            "trade.json.zz",
            "trade.json.zst",
            "trade.yaml",
            "trade.toml",
        ] {
            let media = Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type();
            let mut handle = Buffer::new().with_media_type(media);
            handle
                .write_scalar(&expected)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            let actual = handle
                .read_scalar(None)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(actual, expected, "{name}");
        }
    }

    #[test]
    fn structured_value_fields_direct_parsing_and_casting() {
        let media = Url::from_str("file:///trade.json").unwrap().media_type();
        let source = Buffer::from_bytes(br#"{"quantity":2}"#.to_vec()).with_media_type(media);
        let field = Field::from_str("trade: struct<quantity: int32 not null> not null").unwrap();
        let expected = Scalar::from_sequence([Scalar::from(2)]);

        assert_eq!(source.read_scalar(Some(&field)).unwrap(), expected);

        let invalid = Buffer::from_bytes(br#"{"quantity":"many"}"#.to_vec())
            .with_media_type(Url::from_str("file:///trade.json").unwrap().media_type());
        let message = invalid.read_scalar(Some(&field)).unwrap_err().to_string();
        assert!(message.contains("quantity"), "{message}");
        assert!(message.contains("int32"), "{message}");
    }
}

mod dispatch {
    use super::*;

    #[test]
    fn generic_write_entry_points_compose_the_three_typed_shapes() {
        let mut handle = handle("generic-write-mode.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_row_size(1);

        handle
            .write_arrow_reader(reader(), IOMode::Overwrite, &options)
            .unwrap();
        handle
            .write_arrow_batch(rows_batch(&[3]), IOMode::Append, &options)
            .unwrap();
        handle
            .write_records(
                [
                    NativeRow {
                        id: 2,
                        symbol: Some("updated"),
                    },
                    NativeRow {
                        id: 4,
                        symbol: Some("AMD"),
                    },
                ],
                IOMode::Merge,
                &options.clone().with_merge_by(["id"]).unwrap(),
            )
            .unwrap();

        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn generic_write_entry_points_preserve_commit_cadence() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut reader_handle =
            PublicationProbe::new("generic-reader-commits.arrows", Arc::clone(&pulls));
        let options = reader_handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        reader_handle
            .write_arrow_reader(reader(), IOMode::Overwrite, &options)
            .unwrap();
        assert_eq!(reader_handle.publications.load(Ordering::SeqCst), 2);

        let mut batch_handle =
            PublicationProbe::new("generic-batch-commits.arrows", Arc::clone(&pulls));
        batch_handle
            .write_arrow_batch(batch(), IOMode::Overwrite, &options)
            .unwrap();
        assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 2);

        let mut record_handle = PublicationProbe::new("generic-row-commits.arrows", pulls);
        record_handle
            .write_records(
                [
                    NativeRow {
                        id: 1,
                        symbol: Some("AAPL"),
                    },
                    NativeRow {
                        id: 2,
                        symbol: Some("MSFT"),
                    },
                ],
                IOMode::Overwrite,
                &options,
            )
            .unwrap();
        assert_eq!(record_handle.publications.load(Ordering::SeqCst), 2);
    }

    /// A media may preserve a same-shape optimization while still converging
    /// on the reader primitives. The generic entry points must select that
    /// authoritative adapter rather than rebuilding its input themselves.
    struct TypedDispatchProbe {
        handle: Buffer,
        reader_calls: [usize; 3],
        batch_calls: [usize; 3],
        record_calls: [usize; 3],
    }

    impl TypedDispatchProbe {
        fn new() -> Self {
            Self {
                handle: handle("typed-dispatch-probe.arrows"),
                reader_calls: [0; 3],
                batch_calls: [0; 3],
                record_calls: [0; 3],
            }
        }
    }

    impl IOMedia for TypedDispatchProbe {
        fn as_io_base(&self) -> &dyn IOBase {
            self
        }

        fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
            self
        }

        fn overwrite_arrow_reader(
            &mut self,
            _batches: BatchReader,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()> {
            self.reader_calls[0] += 1;
            Ok(())
        }

        fn append_arrow_reader(
            &mut self,
            _batches: BatchReader,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()> {
            self.reader_calls[1] += 1;
            Ok(())
        }

        fn merge_arrow_reader(
            &mut self,
            _batches: BatchReader,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()> {
            self.reader_calls[2] += 1;
            Ok(())
        }

        fn overwrite_arrow_batch(
            &mut self,
            _batch: RecordBatch,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()> {
            self.batch_calls[0] += 1;
            Ok(())
        }

        fn append_arrow_batch(
            &mut self,
            _batch: RecordBatch,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()> {
            self.batch_calls[1] += 1;
            Ok(())
        }

        fn merge_arrow_batch(
            &mut self,
            _batch: RecordBatch,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()> {
            self.batch_calls[2] += 1;
            Ok(())
        }

        fn overwrite_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[0] += 1;
            Ok(())
        }

        fn append_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[1] += 1;
            Ok(())
        }

        fn merge_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> yggdryl::Result<()>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[2] += 1;
            Ok(())
        }
    }

    impl IOBase for TypedDispatchProbe {
        yggdryl::delegate_iobase!(handle);
    }

    #[test]
    fn generic_writes_select_the_same_shape_for_every_mode() {
        let mut probe = TypedDispatchProbe::new();
        let plain = probe.record_options().unwrap().with_field(schema());

        for mode in IOMode::WRITE {
            let options = if mode == IOMode::Merge {
                plain.clone().with_merge_by(["id"]).unwrap()
            } else {
                plain.clone()
            };
            probe.write_arrow_reader(reader(), mode, &options).unwrap();
            probe.write_arrow_batch(batch(), mode, &options).unwrap();
            probe
                .write_records(std::iter::empty::<NativeRow>(), mode, &options)
                .unwrap();
        }

        assert_eq!(probe.reader_calls, [1, 1, 1]);
        assert_eq!(probe.batch_calls, [1, 1, 1]);
        assert_eq!(probe.record_calls, [1, 1, 1]);
    }

    #[test]
    fn generic_arrow_writes_remain_object_safe() {
        let mut probe = TypedDispatchProbe::new();
        let options = probe.record_options().unwrap().with_field(schema());
        let media: &mut dyn IOMedia = &mut probe;

        media
            .write_arrow_reader(reader(), IOMode::Overwrite, &options)
            .unwrap();
        media
            .write_arrow_batch(batch(), IOMode::Append, &options)
            .unwrap();

        assert_eq!(probe.reader_calls, [1, 0, 0]);
        assert_eq!(probe.batch_calls, [0, 1, 0]);
    }

    #[test]
    fn generic_writes_validate_mode_before_touching_input() {
        let mut handle = handle("generic-write-mode-validation.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        // The required mode is validated before a one-shot source is pulled;
        // a match key never silently turns overwrite into merge.
        let pulls = Arc::new(AtomicUsize::new(0));
        let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[5]))]);
        let error = handle
            .write_arrow_reader(
                source,
                IOMode::Overwrite,
                &options.clone().with_merge_by(["id"]).unwrap(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("write mode overwrite"));
        assert_eq!(pulls.load(Ordering::SeqCst), 0);

        // A held batch is already materialized, but invalid intent still does
        // not reach the destination or silently select merge.
        let before = handle.as_slice().to_vec();
        let error = handle
            .write_arrow_batch(
                rows_batch(&[6]),
                IOMode::Overwrite,
                &options.clone().with_merge_by(["id"]).unwrap(),
            )
            .unwrap_err();
        assert!(error.to_string().contains("write mode overwrite"));
        assert_eq!(handle.as_slice(), before.as_slice());

        struct CountedIntoRows(Arc<AtomicUsize>);

        impl IntoIterator for CountedIntoRows {
            type Item = NativeRow;
            type IntoIter = std::iter::Empty<NativeRow>;

            fn into_iter(self) -> Self::IntoIter {
                self.0.fetch_add(1, Ordering::SeqCst);
                std::iter::empty()
            }
        }

        // Mode validation also wins over the missing-field error and happens
        // before even constructing a native row iterator.
        let into_iters = Arc::new(AtomicUsize::new(0));
        let untyped = handle
            .record_options()
            .unwrap()
            .with_merge_by(["id"])
            .unwrap();
        let error = handle
            .write_records(
                CountedIntoRows(Arc::clone(&into_iters)),
                IOMode::Overwrite,
                &untyped,
            )
            .unwrap_err();
        assert!(error.to_string().contains("write mode overwrite"));
        assert_eq!(into_iters.load(Ordering::SeqCst), 0);
    }
}

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
use arrow_schema::{ArrowError, SchemaRef};

use yggdryl::arrow::BatchReader;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{ArrowWriteSession, IOBase, IOMedia, StructType};
use yggdryl::{DataType, Error, Field, IOMode, MimeType, Scalar, Url};

fn schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

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

/// The batches a write takes: one reader over one two-row batch.
fn reader() -> BatchReader {
    yggdryl::arrow::batch_reader(schema().into_arrow_schema().unwrap(), [batch()])
}

fn handle(name: &str) -> Buffer {
    Buffer::new().with_media_type(
        Url::from_str(&format!("file:///{name}"))
            .unwrap()
            .media_type(),
    )
}

/// The total row count a handle currently holds.
fn rows(handle: &impl IOBase, options: &RecordOptions) -> usize {
    handle
        .read_arrow_reader(options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum()
}

fn rows_batch(ids: &[i64]) -> RecordBatch {
    RecordBatch::try_new(
        schema().into_arrow_schema().unwrap(),
        vec![
            Arc::new(Int64Array::from(ids.to_vec())),
            Arc::new(StringArray::from(
                ids.iter().map(|_| Some("S")).collect::<Vec<_>>(),
            )),
        ],
    )
    .unwrap()
}

/// A byte handle that observes each complete encoded publication.
struct PublicationProbe {
    handle: Buffer,
    publications: Arc<AtomicUsize>,
    source_pulls: Arc<AtomicUsize>,
    pulls_when_published: Arc<Mutex<Vec<usize>>>,
    destination_touches: Arc<AtomicUsize>,
    fail_publication: Option<usize>,
}

impl PublicationProbe {
    fn new(name: &str, source_pulls: Arc<AtomicUsize>) -> Self {
        Self {
            handle: handle(name),
            publications: Arc::new(AtomicUsize::new(0)),
            source_pulls,
            pulls_when_published: Arc::new(Mutex::new(Vec::new())),
            destination_touches: Arc::new(AtomicUsize::new(0)),
            fail_publication: None,
        }
    }

    fn reset_publications(&self) {
        self.publications.store(0, Ordering::SeqCst);
        self.pulls_when_published.lock().unwrap().clear();
    }

    fn fail_on_publication(&mut self, publication: usize) {
        self.fail_publication = Some(publication);
    }
}

impl yggdryl::IOMedia for PublicationProbe {
    yggdryl::impl_default_iomedia!();
}

impl IOBase for PublicationProbe {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
        self.handle.pread(offset, buffer)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> yggdryl::Result<usize> {
        self.handle.pwrite(offset, bytes)
    }

    fn size(&self) -> u64 {
        self.handle.size()
    }

    fn capacity(&self) -> u64 {
        self.handle.capacity()
    }

    fn reserve(&mut self, capacity: u64) -> yggdryl::Result<()> {
        self.handle.reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> yggdryl::Result<()> {
        self.handle.truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn media_type(&self) -> &yggdryl::MediaType {
        self.handle.media_type()
    }

    fn set_media_type(&mut self, media_type: yggdryl::MediaType) {
        self.handle.set_media_type(media_type);
    }

    fn kind(&self) -> yggdryl::IOKind {
        self.destination_touches.fetch_add(1, Ordering::SeqCst);
        yggdryl::IOKind::Memory
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> yggdryl::Result<()> {
        let publication = self.publications.fetch_add(1, Ordering::SeqCst) + 1;
        self.pulls_when_published
            .lock()
            .unwrap()
            .push(self.source_pulls.load(Ordering::SeqCst));
        if self.fail_publication == Some(publication) {
            return Err(yggdryl::Error::Io(std::io::Error::other(format!(
                "publication {publication} refused"
            ))));
        }
        self.handle.write_all_bytes(bytes)
    }
}

/// A fallible source whose exact pull frontier is observable.
struct CountedSource {
    schema: SchemaRef,
    batches: std::collections::VecDeque<std::result::Result<RecordBatch, ArrowError>>,
    pulls: Arc<AtomicUsize>,
}

impl Iterator for CountedSource {
    type Item = std::result::Result<RecordBatch, ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.batches.pop_front()?;
        self.pulls.fetch_add(1, Ordering::SeqCst);
        Some(item)
    }
}

impl RecordBatchReader for CountedSource {
    fn schema(&self) -> SchemaRef {
        Arc::clone(&self.schema)
    }
}

fn counted_source(
    pulls: Arc<AtomicUsize>,
    batches: impl IntoIterator<Item = std::result::Result<RecordBatch, ArrowError>>,
) -> BatchReader {
    Box::new(CountedSource {
        schema: schema().into_arrow_schema().unwrap(),
        batches: batches.into_iter().collect(),
        pulls,
    })
}

#[derive(Clone)]
struct NativeRow {
    id: i64,
    symbol: Option<&'static str>,
}

impl From<NativeRow> for Scalar {
    fn from(row: NativeRow) -> Self {
        Scalar::from_sequence([
            Scalar::from(row.id),
            row.symbol.map_or(Scalar::Null, Scalar::from),
        ])
    }
}

mod pushdown {
    use arrow_array::RecordBatchReader;
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, Field, StructType};

    use super::handle;

    use std::sync::Arc;

    use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};

    use yggdryl::IOMedia;

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

    fn stored(name: &str) -> Buffer {
        let mut handle = handle(name);
        let options = handle.record_options().unwrap();
        let batch = RecordBatch::try_new(
            wide().into_arrow_schema().unwrap(),
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
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
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
        let invented = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("nowhere"),
        ])
        .map(DataType::from)
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
        let reversed = StructType::from_fields([
            DataType::Float64.required_field("price"),
            DataType::Int64.required_field("id"),
        ])
        .map(DataType::from)
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
}

mod rows {
    use super::*;
    use yggdryl::StructType;

    #[test]
    fn native_struct_row_adapters_route_all_three_intents() {
        let mut handle = handle("native-row-adapters.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_row_size(1);

        handle
            .overwrite_records(
                [
                    NativeRow {
                        id: 1,
                        symbol: Some("AAPL"),
                    },
                    NativeRow {
                        id: 2,
                        symbol: None,
                    },
                ],
                &options,
            )
            .unwrap();
        handle
            .append_records(
                [NativeRow {
                    id: 3,
                    symbol: Some("MSFT"),
                }],
                &options,
            )
            .unwrap();
        handle
            .merge_records(
                [
                    NativeRow {
                        id: 2,
                        symbol: Some("updated"),
                    },
                    NativeRow {
                        id: 4,
                        symbol: Some("AMD"),
                    },
                ],
                &options.clone().with_merge_by(["id"]).unwrap(),
            )
            .unwrap();

        assert_eq!(rows(&handle, &options), 4);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn native_row_methods_require_a_field_before_pulling() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountedRows(Arc<AtomicUsize>);

        impl Iterator for CountedRows {
            type Item = NativeRow;

            fn next(&mut self) -> Option<Self::Item> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Some(NativeRow {
                    id: 1,
                    symbol: None,
                })
            }
        }

        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = handle(&format!("native-row-no-field-{intent}.arrows"));
            let options = handle.record_options().unwrap();
            let result = match intent {
                "overwrite" => handle.overwrite_records(CountedRows(Arc::clone(&pulls)), &options),
                "append" => handle.append_records(CountedRows(Arc::clone(&pulls)), &options),
                "merge" => handle.merge_records(
                    CountedRows(Arc::clone(&pulls)),
                    &options.with_merge_by(["id"]).unwrap(),
                ),
                _ => unreachable!(),
            };
            let message = result.unwrap_err().to_string();
            assert!(message.contains("with_field"), "{intent}: {message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert!(handle.is_empty(), "{intent}");
        }
    }

    #[test]
    fn native_row_methods_validate_intent_before_building_or_pulling_the_iterator() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&pulls);
            let records = std::iter::from_fn(move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Some(NativeRow {
                    id: 1,
                    symbol: None,
                })
            });
            let mut handle = handle(&format!("native-row-invalid-intent-{intent}.arrows"));
            let plain = handle.record_options().unwrap().with_field(schema());
            let result =
                match intent {
                    "overwrite" => handle
                        .overwrite_records(records, &plain.clone().with_merge_by(["id"]).unwrap()),
                    "append" => handle
                        .append_records(records, &plain.clone().with_merge_by(["id"]).unwrap()),
                    "merge" => handle.merge_records(records, &plain),
                    _ => unreachable!(),
                };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("merge_by"), "{intent}: {message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert!(handle.is_empty(), "{intent}");
        }
    }

    struct FallibleRow(std::result::Result<Scalar, Error>);

    impl TryFrom<FallibleRow> for Scalar {
        type Error = Error;

        fn try_from(row: FallibleRow) -> std::result::Result<Self, Self::Error> {
            row.0
        }
    }

    struct CountedFallibleRow {
        value: std::result::Result<Scalar, Error>,
        conversions: Arc<AtomicUsize>,
    }

    impl TryFrom<CountedFallibleRow> for Scalar {
        type Error = Error;

        fn try_from(row: CountedFallibleRow) -> std::result::Result<Self, Self::Error> {
            row.conversions.fetch_add(1, Ordering::SeqCst);
            row.value
        }
    }

    #[test]
    fn native_row_conversion_failure_is_typed_and_does_not_publish() {
        let mut handle = handle("native-row-failure.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_row_size(1);
        handle
            .overwrite_records(
                [NativeRow {
                    id: 9,
                    symbol: Some("kept"),
                }],
                &options,
            )
            .unwrap();
        let before = handle.as_slice().to_vec();

        let error = handle
            .append_records(
                [
                    FallibleRow(Ok(Scalar::from_sequence([
                        Scalar::from(10_i64),
                        Scalar::from("not-published"),
                    ]))),
                    FallibleRow(Err(Error::InvalidRecord {
                        path: "$.row".into(),
                        reason: "conversion refused".into(),
                    })),
                ],
                &options,
            )
            .unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }));
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn native_row_conversion_stops_at_each_commit_for_all_intents() {
        for intent in ["overwrite", "append", "merge"] {
            let conversions = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("native-partial-{intent}.arrows"),
                Arc::clone(&conversions),
            );
            let plain = handle.record_options().unwrap().with_field(schema());
            if intent != "overwrite" {
                handle.overwrite_arrow_reader(reader(), &plain).unwrap();
                handle.reset_publications();
            }
            let records = [
                CountedFallibleRow {
                    value: Ok(Scalar::from_sequence([
                        Scalar::from(3_i64),
                        Scalar::from("committed"),
                    ])),
                    conversions: Arc::clone(&conversions),
                },
                CountedFallibleRow {
                    value: Err(Error::InvalidRecord {
                        path: "$.row[1]".into(),
                        reason: "later native conversion failure".into(),
                    }),
                    conversions: Arc::clone(&conversions),
                },
            ];
            let committed = plain.clone().with_commit_row_size(1);
            let result = match intent {
                "overwrite" => handle.overwrite_records(records, &committed),
                "append" => handle.append_records(records, &committed),
                "merge" => {
                    handle.merge_records(records, &committed.clone().with_merge_by(["id"]).unwrap())
                }
                _ => unreachable!(),
            };

            let error = result.unwrap_err();
            assert!(matches!(error, Error::InvalidRecord { .. }), "{intent}");
            assert_eq!(handle.publications.load(Ordering::SeqCst), 1, "{intent}");
            assert_eq!(
                handle.pulls_when_published.lock().unwrap().as_slice(),
                [1],
                "{intent}: row two must not convert before row one publishes"
            );
            assert_eq!(conversions.load(Ordering::SeqCst), 2, "{intent}");
            assert_eq!(
                rows(&handle, &plain),
                if intent == "overwrite" { 1 } else { 3 },
                "{intent}"
            );
        }
    }

    #[test]
    fn native_rows_align_a_non_divisible_batch_before_the_next_conversion() {
        let conversions = Arc::new(AtomicUsize::new(0));
        let mut handle =
            PublicationProbe::new("native-non-divisible.arrows", Arc::clone(&conversions));
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_row_size(2)
            .with_commit_row_size(3);
        let mut records = Vec::new();
        for id in 1..=3_i64 {
            records.push(CountedFallibleRow {
                value: Ok(Scalar::from_sequence([
                    Scalar::from(id),
                    Scalar::from("committed"),
                ])),
                conversions: Arc::clone(&conversions),
            });
        }
        records.push(CountedFallibleRow {
            value: Err(Error::InvalidRecord {
                path: "$.row[3]".into(),
                reason: "conversion after a non-divisible cadence".into(),
            }),
            conversions: Arc::clone(&conversions),
        });

        let error = handle.overwrite_records(records, &options).unwrap_err();
        assert!(matches!(error, Error::InvalidRecord { .. }));
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.pulls_when_published.lock().unwrap().as_slice(),
            [3],
            "row four must not convert before the three-row cadence publishes"
        );
        assert_eq!(conversions.load(Ordering::SeqCst), 4);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn native_rows_stop_at_the_global_row_limit_without_one_extra_pull() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&pulls);
        let records = std::iter::from_fn(move || {
            let id = counted.fetch_add(1, Ordering::SeqCst) as i64;
            Some(NativeRow {
                id,
                symbol: Some("bounded"),
            })
        });
        let mut handle = handle("native-global-row-limit.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_row_size(2)
            .with_max_row_size(3);

        handle.overwrite_records(records, &options).unwrap();

        assert_eq!(pulls.load(Ordering::SeqCst), 3);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn empty_native_row_intents_keep_overwrite_schema_and_make_append_merge_no_ops() {
        let mut missing = handle("empty-native-row-append.arrows");
        let options = missing.record_options().unwrap().with_field(schema());
        missing
            .append_records(std::iter::empty::<NativeRow>(), &options)
            .unwrap();
        assert!(missing.is_empty());

        let mut handle = handle("empty-native-row-overwrite.arrows");
        handle
            .overwrite_records(std::iter::empty::<NativeRow>(), &options)
            .unwrap();
        assert_eq!(rows(&handle, &options), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        let before = handle.as_slice().to_vec();

        handle
            .append_records(
                std::iter::empty::<NativeRow>(),
                &options.clone().with_select("absent").unwrap(),
            )
            .unwrap();
        handle
            .merge_records(
                std::iter::empty::<NativeRow>(),
                &options
                    .clone()
                    .with_merge_by(["id"])
                    .unwrap()
                    .with_select("absent")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn an_empty_record_batch_overwrite_keeps_its_field_and_no_rows() {
        let mut handle = handle("empty-record-batch.arrows");
        let options = handle.record_options().unwrap();
        let empty = RecordBatch::new_empty(schema().into_arrow_schema().unwrap());

        handle.overwrite_arrow_batch(empty, &options).unwrap();

        assert_eq!(rows(&handle, &options), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn empty_append_and_merge_are_byte_for_byte_no_ops() {
        let mut missing = handle("empty-no-op.arrows");
        let options = missing.record_options().unwrap().with_field(schema());
        missing
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(schema().into_arrow_schema().unwrap(), []),
                &options.clone().with_select("absent").unwrap(),
            )
            .unwrap();
        assert!(missing.is_empty(), "an empty append must not create bytes");

        missing.overwrite_arrow_reader(reader(), &options).unwrap();
        let before = missing.as_slice().to_vec();
        let zero = RecordBatch::new_empty(schema().into_arrow_schema().unwrap());
        missing
            .merge_arrow_reader(
                yggdryl::arrow::batch_reader(zero.schema(), [zero]),
                &options
                    .clone()
                    .with_merge_by(["id"])
                    .unwrap()
                    .with_select("absent")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(missing.as_slice(), before.as_slice());
    }

    #[test]
    fn appending_casts_incoming_batches_to_the_target_shape() {
        let mut handle = handle("cast-append.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        // The incoming batch merely fits: `id` is narrower and the columns are
        // the other way round.
        let loose = StructType::from_fields([
            DataType::utf8().nullable_field("symbol"),
            DataType::Int32.required_field("id"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("row");
        let incoming = RecordBatch::try_new(
            loose.clone().into_arrow_schema().unwrap(),
            vec![
                Arc::new(StringArray::from(vec![Some("MSFT")])),
                Arc::new(arrow_array::Int32Array::from(vec![3])),
            ],
        )
        .unwrap();

        handle
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap();

        let batches = handle
            .read_arrow_reader(&options)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[1].schema(), batches[0].schema());
        assert_eq!(batches[1].num_rows(), 1);
    }

    #[test]
    fn a_cast_that_cannot_be_planned_leaves_the_resource_alone() {
        let mut handle = handle("failed-append.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &options).unwrap();
        let before = handle.as_slice().to_vec();

        // Text that is not a number cannot become the declared Int64, and this
        // write is strict, so the append fails while the batches are being
        // encoded - before anything is published.
        let hostile = StructType::from_fields([DataType::utf8().required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let incoming = RecordBatch::try_new(
            hostile.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["not a number"]))],
        )
        .unwrap();

        let message = handle
            .append_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap_err()
            .to_string();

        // The core failure is reported as itself rather than as the Arrow
        // envelope it had to travel through the reader inside.
        assert!(!message.contains("External error"), "{message}");
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn a_row_limit_bounds_a_read_at_below_and_above_the_stored_count() {
        let mut handle = handle("row-limited.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        // The bound is exact: one below slices, the count itself keeps
        // everything, and one above changes nothing.
        assert_eq!(rows(&handle, &options.clone().with_max_row_size(1)), 1);
        assert_eq!(rows(&handle, &options.clone().with_max_row_size(2)), 2);
        assert_eq!(rows(&handle, &options.clone().with_max_row_size(3)), 2);
    }

    #[test]
    fn overwrite_refuses_a_match_key_and_a_limited_merge_names_both_settings() {
        let mut handle = handle("limited-merge.arrows");
        let keyed = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_merge_by(["id"])
            .unwrap();
        let before = handle.as_slice().to_vec();

        // The operation carries intent: overwrite never silently becomes a
        // merge because its options happen to carry keys.
        let message = handle
            .overwrite_arrow_reader(reader(), &keyed)
            .unwrap_err()
            .to_string();
        assert!(message.contains("write mode overwrite"), "{message}");
        assert!(message.contains("merge_by"), "{message}");
        assert_eq!(handle.as_slice(), before.as_slice());

        // A truncated merge would update the matched keys it kept and
        // silently drop the rest, so the combination is refused by name
        // before a single row moves.
        let limited = keyed.with_max_row_size(1);
        let message = handle
            .merge_arrow_reader(reader(), &limited)
            .unwrap_err()
            .to_string();
        assert!(message.contains("max_row_size = 1"), "{message}");
        assert!(message.contains("merge_by `id`"), "{message}");
        assert_eq!(handle.as_slice(), before.as_slice());
    }

    #[test]
    fn merge_refuses_an_empty_match_key_before_pulling_the_reader() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Counting {
            schema: arrow_schema::SchemaRef,
            pulls: Arc<AtomicUsize>,
        }

        impl Iterator for Counting {
            type Item = std::result::Result<RecordBatch, arrow_schema::ArrowError>;

            fn next(&mut self) -> Option<Self::Item> {
                self.pulls.fetch_add(1, Ordering::SeqCst);
                Some(Ok(batch()))
            }
        }

        impl RecordBatchReader for Counting {
            fn schema(&self) -> arrow_schema::SchemaRef {
                Arc::clone(&self.schema)
            }
        }

        let pulls = Arc::new(AtomicUsize::new(0));
        let reader: BatchReader = Box::new(Counting {
            schema: schema().into_arrow_schema().unwrap(),
            pulls: Arc::clone(&pulls),
        });
        let mut handle = handle("missing-merge-key.arrows");
        let options = handle.record_options().unwrap().with_field(schema());
        let message = handle
            .merge_arrow_reader(reader, &options)
            .unwrap_err()
            .to_string();

        assert!(message.contains("requires at least one"), "{message}");
        assert!(message.contains("merge_by"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
        assert!(handle.is_empty());
    }
}

mod write {
    use super::*;
    use yggdryl::StructType;

    #[test]
    fn the_handles_media_type_picks_the_record_encoding() {
        assert!(matches!(
            handle("t.arrows").record_options().unwrap(),
            RecordOptions::Ipc(_)
        ));
        #[cfg(feature = "parquet")]
        assert!(matches!(
            handle("t.parquet").record_options().unwrap(),
            RecordOptions::Parquet(_)
        ));

        // An encoding with no implementation is named rather than guessed.
        let message = handle("t.csv").record_options().unwrap_err().to_string();
        assert!(message.contains("text/csv"), "{message}");
    }

    #[test]
    fn batches_round_trip_through_a_bare_handle() {
        let mut names = vec!["t.arrows", "t.arrows.zst"];
        if cfg!(feature = "parquet") {
            names.push("t.parquet");
        }

        for name in names {
            let mut handle = handle(name);
            let options = handle
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_safe(true);

            handle
                .overwrite_arrow_reader(reader(), &options)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(rows(&handle, &options), 2, "{name}");
            assert_eq!(
                handle.read_arrow_field(&options).unwrap(),
                schema(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_write_stores_the_schema_its_reader_declares() {
        let mut handle = handle("declared.arrows");
        let options = handle.record_options().unwrap();

        // The write path takes a reader and nothing else, so with nothing
        // declared and nothing stored the reader's own schema is what the
        // resource ends up holding.
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(
            handle
                .read_arrow_reader(&options)
                .unwrap()
                .schema()
                .fields()
                .len(),
            2
        );
    }

    #[test]
    fn an_overwrite_keeps_the_schema_the_resource_already_stores() {
        let mut handle = handle("stable.arrows");
        let options = handle.record_options().unwrap();
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        // The incoming rows declare `id` as text and drop `symbol` entirely. An
        // overwrite replaces rows, so the stored columns survive it and the
        // text is cast back into the stored Int64.
        let loose = StructType::from_fields([DataType::utf8().required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let incoming = RecordBatch::try_new(
            loose.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["7"]))],
        )
        .unwrap();
        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                &options,
            )
            .unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(rows(&handle, &options), 1);
    }

    #[test]
    fn a_missing_resource_reads_as_empty_rather_than_failing() {
        let handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
        let options = handle.record_options().unwrap().with_field(schema());

        assert_eq!(handle.read_arrow_reader(&options).unwrap().count(), 0);
        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
    }

    #[test]
    fn appending_reads_adds_and_rewrites() {
        let mut handle = handle("append.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        // Appending to nothing simply writes.
        handle.append_arrow_reader(reader(), &options).unwrap();
        assert_eq!(rows(&handle, &options), 2);

        handle.append_arrow_reader(reader(), &options).unwrap();
        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn commit_row_size_controls_exact_publication_counts() {
        for (label, cadence, expected) in [
            ("unset", None, 1),
            ("one", Some(1), 4),
            ("across-batches", Some(3), 2),
            ("larger-than-stream", Some(10), 1),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("commit-publications-{label}.arrows"),
                Arc::clone(&pulls),
            );
            let mut options = handle.record_options().unwrap().with_field(schema());
            options.set_commit_row_size(cadence);
            let source = yggdryl::arrow::batch_reader(
                schema().into_arrow_schema().unwrap(),
                [rows_batch(&[1, 2]), rows_batch(&[3, 4])],
            );

            handle.overwrite_arrow_reader(source, &options).unwrap();

            assert_eq!(
                handle.publications.load(Ordering::SeqCst),
                expected,
                "{label}"
            );
            assert_eq!(rows(&handle, &options), 4, "{label}");
        }
    }

    #[test]
    fn every_write_intent_retains_its_intent_for_each_commit() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("commit-intent-{intent}.arrows"),
                Arc::clone(&pulls),
            );
            let plain = handle.record_options().unwrap().with_field(schema());
            handle
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(
                        schema().into_arrow_schema().unwrap(),
                        [rows_batch(&[1, 2])],
                    ),
                    &plain,
                )
                .unwrap();
            handle.reset_publications();

            let options = plain.clone().with_commit_row_size(2);
            let incoming = yggdryl::arrow::batch_reader(
                schema().into_arrow_schema().unwrap(),
                [rows_batch(&[1, 3, 4, 5])],
            );
            match intent {
                "overwrite" => handle.overwrite_arrow_reader(incoming, &options).unwrap(),
                "append" => handle.append_arrow_reader(incoming, &options).unwrap(),
                "merge" => handle
                    .merge_arrow_reader(incoming, &options.with_merge_by(["id"]).unwrap())
                    .unwrap(),
                _ => unreachable!(),
            }

            assert_eq!(handle.publications.load(Ordering::SeqCst), 2, "{intent}");
            let expected_rows = match intent {
                "overwrite" => 4,
                "append" => 6,
                "merge" => 5,
                _ => unreachable!(),
            };
            assert_eq!(rows(&handle, &plain), expected_rows, "{intent}");
        }
    }

    #[test]
    fn held_batch_and_native_row_adapters_inherit_commit_boundaries() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut batch_handle = PublicationProbe::new("commit-batch.arrows", Arc::clone(&pulls));
        let options = batch_handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        batch_handle
            .overwrite_arrow_batch(rows_batch(&[1, 2]), &options)
            .unwrap();
        assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 2);

        let mut row_handle = PublicationProbe::new("commit-rows.arrows", pulls);
        row_handle
            .overwrite_records(
                [
                    NativeRow {
                        id: 1,
                        symbol: Some("AAPL"),
                    },
                    NativeRow {
                        id: 2,
                        symbol: Some("MSFT"),
                    },
                ],
                &options,
            )
            .unwrap();
        assert_eq!(row_handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&row_handle, &options), 2);
    }

    #[test]
    fn zero_commit_row_size_is_rejected_before_any_input_pull() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle =
                PublicationProbe::new(&format!("zero-commit-{intent}.arrows"), Arc::clone(&pulls));
            let options = handle
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_commit_row_size(0);
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(source, &options),
                "append" => handle.append_arrow_reader(source, &options),
                "merge" => handle
                    .merge_arrow_reader(source, &options.clone().with_merge_by(["id"]).unwrap()),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("commit_row_size"), "{intent}: {message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert_eq!(handle.publications.load(Ordering::SeqCst), 0, "{intent}");
        }

        let pulls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&pulls);
        let records = std::iter::from_fn(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Some(NativeRow {
                id: 1,
                symbol: None,
            })
        });
        let mut handle = handle("zero-commit-native.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(0);
        let message = handle
            .overwrite_records(records, &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("commit_row_size"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn empty_append_and_merge_do_not_touch_the_destination() {
        let source_pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("empty-no-touch.arrows", source_pulls);
        let options = handle.record_options().unwrap().with_field(schema());
        let touches = Arc::clone(&handle.destination_touches);
        // Option discovery is outside the write; count only destination work
        // performed after the empty source crosses the primitive boundary.
        touches.store(0, Ordering::SeqCst);
        let empty = || yggdryl::arrow::batch_reader(schema().into_arrow_schema().unwrap(), []);

        handle.append_arrow_reader(empty(), &options).unwrap();
        handle
            .merge_arrow_reader(empty(), &options.with_merge_by(["id"]).unwrap())
            .unwrap();

        assert_eq!(touches.load(Ordering::SeqCst), 0);
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_append_limits_and_invalid_merge_limits_do_not_pull() {
        for options in [
            handle("limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_max_row_size(0),
            handle("limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_max_byte_size(0),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let mut destination = handle("zero-limit-append.arrows");
            destination.append_arrow_reader(source, &options).unwrap();
            assert_eq!(pulls.load(Ordering::SeqCst), 0);
        }

        for limited in [
            handle("merge-limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_merge_by(["id"])
                .unwrap()
                .with_max_row_size(1),
            handle("merge-limit-options.arrows")
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_merge_by(["id"])
                .unwrap()
                .with_max_byte_size(1),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let mut destination = handle("invalid-limit-merge.arrows");
            let message = destination
                .merge_arrow_reader(source, &limited)
                .unwrap_err()
                .to_string();
            assert!(message.contains("merge_by"), "{message}");
            assert_eq!(pulls.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn a_later_source_failure_leaves_each_successful_prefix_visible() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("partial-commit-{intent}.arrows"),
                Arc::clone(&pulls),
            );
            let plain = handle.record_options().unwrap().with_field(schema());
            if intent != "overwrite" {
                handle
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(
                            schema().into_arrow_schema().unwrap(),
                            [rows_batch(&[1, 2])],
                        ),
                        &plain,
                    )
                    .unwrap();
                handle.reset_publications();
            }
            let options = plain.clone().with_commit_row_size(2);
            let source = counted_source(
                Arc::clone(&pulls),
                [
                    Ok(rows_batch(&[2, 3])),
                    Ok(rows_batch(&[99])),
                    Err(ArrowError::ComputeError("later source failure".into())),
                ],
            );
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(source, &options),
                "append" => handle.append_arrow_reader(source, &options),
                "merge" => handle
                    .merge_arrow_reader(source, &options.clone().with_merge_by(["id"]).unwrap()),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(
                message.contains("later source failure"),
                "{intent}: {message}"
            );
            assert_eq!(handle.publications.load(Ordering::SeqCst), 1, "{intent}");
            assert_eq!(
                handle.pulls_when_published.lock().unwrap().as_slice(),
                [1],
                "{intent}: the second batch must not be pulled before commit one publishes"
            );
            assert_eq!(
                pulls.load(Ordering::SeqCst),
                3,
                "{intent}: the one-row second cadence is discarded when its next pull fails"
            );
            let expected_rows = match intent {
                "overwrite" => 2,
                "append" => 4,
                "merge" => 3,
                _ => unreachable!(),
            };
            assert_eq!(rows(&handle, &plain), expected_rows, "{intent}");
        }
    }

    #[test]
    fn a_second_publication_failure_keeps_the_first_commit_visible() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("second-publication-failure.arrows", pulls);
        handle.fail_on_publication(2);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let source = yggdryl::arrow::batch_reader(
            schema().into_arrow_schema().unwrap(),
            [rows_batch(&[1, 2, 3, 4])],
        );

        let message = handle
            .overwrite_arrow_reader(source, &options)
            .unwrap_err()
            .to_string();

        assert!(message.contains("publication 2 refused"), "{message}");
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &options), 2);
    }

    #[test]
    fn resumed_write_publishes_complete_cadences_and_abort_drops_only_the_remainder() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-write.arrows", pulls);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(3);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();

        assert!(
            session
                .push(
                    &mut handle,
                    yggdryl::arrow::batch_reader(
                        schema().into_arrow_schema().unwrap(),
                        [rows_batch(&[1, 2])],
                    ),
                )
                .unwrap()
        );
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);

        assert!(
            session
                .push(
                    &mut handle,
                    yggdryl::arrow::batch_reader(
                        schema().into_arrow_schema().unwrap(),
                        [rows_batch(&[3])],
                    ),
                )
                .unwrap()
        );
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &options), 3);

        session
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[4])],
                ),
            )
            .unwrap();
        session.abort();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn resumed_write_keeps_global_limits_and_stops_before_another_chunk_pull() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-limit.arrows", Arc::clone(&pulls));
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2)
            .with_max_row_size(3);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();

        assert!(
            session
                .push(
                    &mut handle,
                    counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1, 2]))]),
                )
                .unwrap()
        );
        let second = counted_source(
            Arc::clone(&pulls),
            [Ok(rows_batch(&[3, 4])), Ok(rows_batch(&[99]))],
        );
        assert!(!session.push(&mut handle, second).unwrap());
        session.finish(&mut handle).unwrap();

        assert_eq!(pulls.load(Ordering::SeqCst), 2);
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &options), 3);
    }

    #[test]
    fn resumed_zero_limits_need_no_source_and_only_overwrite_publishes() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-zero.arrows", pulls);
        let base = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2)
            .with_max_row_size(0);
        handle.destination_touches.store(0, Ordering::SeqCst);

        let mut append = ArrowWriteSession::append(&base).unwrap();
        append.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
        assert_eq!(handle.destination_touches.load(Ordering::SeqCst), 0);

        let mut overwrite = ArrowWriteSession::overwrite(&base).unwrap();
        overwrite.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &base), 0);
        assert_eq!(handle.read_arrow_field(&base).unwrap(), schema());
    }

    #[test]
    fn resumed_sessions_keep_append_and_merge_intent_for_every_cadence() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-intents.arrows", pulls);
        let plain = handle.record_options().unwrap().with_field(schema());
        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1, 2])],
                ),
                &plain,
            )
            .unwrap();

        handle.reset_publications();
        let append_options = plain.clone().with_commit_row_size(1);
        let mut append = ArrowWriteSession::append(&append_options).unwrap();
        append
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[3, 4])],
                ),
            )
            .unwrap();
        append.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &plain), 4);

        handle.reset_publications();
        let merge_options = plain
            .clone()
            .with_commit_row_size(1)
            .with_merge_by(["id"])
            .unwrap();
        let mut merge = ArrowWriteSession::merge(&merge_options).unwrap();
        merge
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[2, 5])],
                ),
            )
            .unwrap();
        merge.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &plain), 5);
    }

    #[test]
    fn resumed_session_covers_large_cadence_multiple_commits_and_terminal_reuse() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut large = PublicationProbe::new("resumed-large-cadence.arrows", Arc::clone(&pulls));
        let large_options = large
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(10);
        let mut session = ArrowWriteSession::overwrite(&large_options).unwrap();
        session
            .push(
                &mut large,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1, 2])],
                ),
            )
            .unwrap();
        assert_eq!(large.publications.load(Ordering::SeqCst), 0);
        session.finish(&mut large).unwrap();
        assert_eq!(large.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&large, &large_options), 2);
        let message = session
            .push(
                &mut large,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[3])],
                ),
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("cannot be reused"), "{message}");

        let mut exact = PublicationProbe::new("resumed-multiple.arrows", pulls);
        let exact_options = exact
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let mut exact_session = ArrowWriteSession::overwrite(&exact_options).unwrap();
        exact_session
            .push(
                &mut exact,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1, 2, 3, 4])],
                ),
            )
            .unwrap();
        exact_session.finish(&mut exact).unwrap();
        assert_eq!(exact.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&exact, &exact_options), 4);
    }

    #[test]
    fn resumed_session_fuses_on_schema_source_and_publication_failures() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut mismatch = PublicationProbe::new("resumed-schema.arrows", Arc::clone(&pulls));
        let options = mismatch
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();
        session
            .push(
                &mut mismatch,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1])],
                ),
            )
            .unwrap();
        let other = StructType::from_fields([DataType::utf8().required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
        let other_batch = RecordBatch::try_new(
            other.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["2"]))],
        )
        .unwrap();
        let message = session
            .push(
                &mut mismatch,
                yggdryl::arrow::batch_reader(other_batch.schema(), [other_batch]),
            )
            .unwrap_err()
            .to_string();
        assert!(message.contains("later chunk schema"), "{message}");
        assert_eq!(mismatch.publications.load(Ordering::SeqCst), 0);

        let mut source_failure =
            PublicationProbe::new("resumed-source-error.arrows", Arc::clone(&pulls));
        let source_options = source_failure
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(2);
        let mut source_session = ArrowWriteSession::overwrite(&source_options).unwrap();
        let error = source_session
            .push(
                &mut source_failure,
                counted_source(
                    Arc::clone(&pulls),
                    [
                        Ok(rows_batch(&[1, 2])),
                        Err(ArrowError::ComputeError("resumed source failed".into())),
                    ],
                ),
            )
            .unwrap_err();
        assert!(error.to_string().contains("resumed source failed"));
        assert_eq!(source_failure.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&source_failure, &source_options), 2);

        let mut publication = PublicationProbe::new("resumed-publication-error.arrows", pulls);
        publication.fail_on_publication(2);
        let publication_options = publication
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        let mut publication_session = ArrowWriteSession::overwrite(&publication_options).unwrap();
        let error = publication_session
            .push(
                &mut publication,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1, 2])],
                ),
            )
            .unwrap_err();
        assert!(error.to_string().contains("publication 2 refused"));
        assert_eq!(publication.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&publication, &publication_options), 1);
    }

    #[test]
    fn resumed_leaf_keeps_the_target_captured_before_an_external_replacement() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-stable-target.arrows", pulls);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_commit_row_size(1);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();
        session
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1])],
                ),
            )
            .unwrap();

        handle.clear().unwrap();
        let loose = StructType::from_fields([DataType::utf8().required_field("id")])
            .map(DataType::from)
            .unwrap()
            .required_field("other");
        let loose_batch = RecordBatch::try_new(
            loose.clone().into_arrow_schema().unwrap(),
            vec![Arc::new(StringArray::from(vec!["9"]))],
        )
        .unwrap();
        let external = handle.record_options().unwrap();
        handle
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(loose_batch.schema(), [loose_batch]),
                &external,
            )
            .unwrap();
        let replaced = handle.read_arrow_field(&external).unwrap();
        assert_eq!(replaced.dtype(), loose.dtype());
        assert_ne!(replaced, schema());

        session
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[2])],
                ),
            )
            .unwrap();
        session.finish(&mut handle).unwrap();

        assert_eq!(handle.read_arrow_field(&options).unwrap(), schema());
        assert_eq!(rows(&handle, &options), 2);
    }

    #[test]
    fn bounded_empty_intents_publish_only_overwrite() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("bounded-empty.arrows", pulls);
        let plain = handle.record_options().unwrap().with_field(schema());
        handle.overwrite_arrow_reader(reader(), &plain).unwrap();
        handle.reset_publications();
        let bounded = plain.clone().with_commit_row_size(2);
        let empty = || yggdryl::arrow::batch_reader(schema().into_arrow_schema().unwrap(), []);

        handle.append_arrow_reader(empty(), &bounded).unwrap();
        handle
            .merge_arrow_reader(empty(), &bounded.clone().with_merge_by(["id"]).unwrap())
            .unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
        assert_eq!(rows(&handle, &plain), 2);

        handle.overwrite_arrow_reader(empty(), &bounded).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &plain), 0);
        assert_eq!(handle.read_arrow_field(&plain).unwrap(), schema());
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn the_three_methods_behave_the_same_way_on_parquet() {
        let mut handle = handle("three.parquet");
        let options = handle.record_options().unwrap().with_field(schema());

        handle.append_arrow_reader(reader(), &options).unwrap();
        handle.overwrite_arrow_reader(reader(), &options).unwrap();
        handle.append_arrow_reader(reader(), &options).unwrap();

        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn record_batch_adapters_route_to_each_explicit_reader_primitive() {
        let mut handle = handle("record-batch-adapters.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        handle.overwrite_arrow_batch(batch(), &options).unwrap();
        handle.append_arrow_batch(batch(), &options).unwrap();
        assert_eq!(rows(&handle, &options), 4);

        let merging = options.clone().with_merge_by(["id"]).unwrap();
        handle.merge_arrow_batch(batch(), &merging).unwrap();
        // Both stored copies of each key update in place; merge does not turn
        // either incoming row into a third copy.
        assert_eq!(rows(&handle, &options), 4);
    }
}

mod shape {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    use super::counting::Counting;
    use yggdryl::coding::Coding;
    use yggdryl::holder::Holder;
    use yggdryl::{Codec, IOKind, MediaType, MimeType};

    /// A writable temporary root of this test's own.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = yggdryl::local::Folder::temporary()
            .expect("the temporary directory")
            .path()
            .expect("a platform path");
        path.push(format!("yggdryl-shape-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a writable temporary root");
        path
    }

    #[test]
    fn a_leaf_answers_from_its_representation_and_the_two_are_complements() {
        for (mime, tabular) in [
            (MimeType::PLAIN_TEXT, false),
            (MimeType::JSON, false),
            (MimeType::PARQUET, true),
            (MimeType::ARROW_FILE, true),
            (MimeType::CSV, true),
        ] {
            let mut handle = Buffer::new();
            handle.set_media_type(MediaType::from(mime.clone()));
            assert_eq!(handle.is_tabular(), tabular, "{mime}");
            // Exactly one of the two, because a leaf is read one way or the
            // other and never both.
            assert_eq!(handle.is_atomic(), !tabular, "{mime}");
            assert!(handle.is_io(), "{mime}");
        }
    }

    #[test]
    fn a_default_buffer_is_one_whole_byte_value() {
        let handle = Buffer::from_bytes(b"AAPL".to_vec());
        assert_eq!(handle.kind(), IOKind::Memory);
        assert!(handle.is_atomic());
        assert!(!handle.is_tabular());
    }

    #[test]
    fn a_content_coding_answers_for_the_representation_underneath_it() {
        // `trades.arrows.gz` is an Arrow file that happens to be compressed,
        // so the coding never changes which surface reads it.
        let media = MediaType::from_file_name("trades.arrows.gz");
        assert_eq!(media.base(), &MimeType::ARROW_STREAM);
        assert_eq!(media.encodings(), [MimeType::GZIP]);
        let mut handle = Buffer::new();
        handle.set_media_type(media);
        assert!(handle.is_tabular());
        assert!(!handle.is_atomic());

        let coded = Coding::new(handle, Codec::Gzip);
        assert!(coded.is_tabular());
        assert!(!coded.is_atomic());
    }

    #[test]
    fn a_named_location_answers_before_anything_exists() {
        let path = root("named");

        // Nothing has been written, so the kind is undecided - and the name
        // still says which surface reads it, exactly as the media type does.
        let missing = yggdryl::local::Path::new(path.join("trades.parquet")).unwrap();
        assert_eq!(missing.kind(), IOKind::Unknown);
        assert_eq!(missing.media_type().base(), &MimeType::PARQUET);
        assert!(missing.is_tabular());
        assert!(!missing.is_atomic());

        let notes = yggdryl::local::Path::new(path.join("notes.txt")).unwrap();
        assert_eq!(notes.kind(), IOKind::Unknown);
        assert!(notes.is_atomic());
        assert!(!notes.is_tabular());

        // The leaf implementation answers the same, existing or not.
        let leaf = yggdryl::local::File::new(path.join("trades.arrows")).unwrap();
        assert!(leaf.is_tabular());
        assert!(!leaf.is_atomic());

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_folder_reads_as_the_table_beneath_it() {
        let path = root("folder");
        let lake = path.join("lake");
        std::fs::create_dir_all(lake.join("year=2024/month=01")).unwrap();
        std::fs::write(lake.join("year=2024/month=01/part-0.parquet"), b"PAR1").unwrap();

        let folder = yggdryl::local::Folder::new(&lake).unwrap();
        assert_eq!(folder.kind(), IOKind::Directory);
        assert!(folder.is_container());
        // The probe descends to the first leaf; a folder is never one whole
        // byte value whatever is under it.
        assert!(folder.is_tabular());
        assert!(!folder.is_atomic());

        // A container of plain files is neither: no rows to read, and no one
        // byte value to read whole.
        let logs = path.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("run.txt"), b"started").unwrap();
        let folder = yggdryl::local::Folder::new(&logs).unwrap();
        assert!(!folder.is_tabular());
        assert!(!folder.is_atomic());
        assert!(!folder.is_io());

        // So is an empty one, and so is a folder that does not exist yet.
        let empty = yggdryl::local::Folder::new(path.join("empty")).unwrap();
        assert!(!empty.is_tabular());
        assert!(!empty.is_atomic());
        assert!(!empty.is_io());

        // A location resolving to that lake answers exactly as the folder did.
        let located = yggdryl::local::Path::new(&lake).unwrap();
        assert_eq!(located.kind(), IOKind::Directory);
        assert!(located.is_tabular());
        assert!(!located.is_atomic());

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_record_encoding_handle_answers_without_touching_its_bytes() {
        // The buffer underneath carries no media type at all, so nothing but
        // the encoding itself can be answering here.
        let plain = Buffer::new();
        assert!(plain.is_atomic());

        let ipc = yggdryl::ipc::Ipc::new(Buffer::new());
        assert!(ipc.is_tabular());
        assert!(!ipc.is_atomic());

        #[cfg(feature = "parquet")]
        {
            let parquet = yggdryl::parquet::Parquet::new(Buffer::new());
            assert!(parquet.is_tabular());
            assert!(!parquet.is_atomic());
        }

        let avro = yggdryl::avro::Avro::new(Buffer::new());
        assert!(avro.is_tabular());
        assert!(!avro.is_atomic());
    }

    #[test]
    fn asking_the_shape_of_a_leaf_reads_nothing() {
        // The counting double is the measuring instrument the page cache uses:
        // it reports every `pread` and every `size` that reaches the bytes.
        let mut handle = Counting::from_bytes(b"PAR1".to_vec());
        handle.set_media_type(MediaType::from(MimeType::PARQUET));

        assert!(handle.is_tabular());
        assert!(!handle.is_atomic());

        // Both answers came from the representation, so nothing was read and
        // nothing was even measured.
        assert_eq!(handle.reads(), 0);
        assert_eq!(handle.sizes(), 0);
    }

    #[test]
    fn wrapping_a_handle_keeps_the_shape_it_wraps() {
        let mut handle = Buffer::new();
        handle.set_media_type(MediaType::from(MimeType::PARQUET));

        // A page cache is invisible: it answers exactly what it wraps.
        let cached = handle.buffered(yggdryl::holder::buffered::BufferedOptions::default());
        assert!(cached.is_tabular());
        assert!(!cached.is_atomic());

        // So is the generic enum every listing hands back.
        let held = Holder::from(Buffer::from_bytes(b"AAPL".to_vec()));
        assert!(held.is_atomic());
        assert!(!held.is_tabular());
    }

    #[test]
    fn folder_dimensions_sum_only_the_selected_record_encoding() {
        use std::sync::Arc;

        use arrow_array::{Int64Array, RecordBatch};

        use yggdryl::IOMedia as _;

        fn rows(values: &[i64]) -> RecordBatch {
            let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int64,
                false,
            )]));
            RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values.to_vec()))])
                .expect("a dimension fixture")
        }

        let path = root("dimensions");
        let lake = path.join("lake");
        for (name, values) in [("a.arrows", vec![1, 2]), ("b.arrows", vec![3])] {
            let mut leaf = yggdryl::local::Path::new(lake.join(name)).expect("a lazy leaf");
            let batch = rows(&values);
            let options = leaf.record_options().expect("IPC options");
            leaf.overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .expect("a published IPC leaf");
        }
        yggdryl::local::Path::new(lake.join("notes.txt"))
            .expect("a text leaf")
            .write_all_bytes(b"not a table row")
            .expect("a published unrelated leaf");

        let folder = yggdryl::local::Folder::new(&lake).expect("the lake folder");
        assert_eq!(folder.row_size().expect("metadata row count"), 3);
        assert_eq!(folder.column_size().expect("metadata field width"), 1);

        let _ = std::fs::remove_dir_all(&path);
    }
}
