//! `rust/src/iomedia.rs`: the record surface derived from the byte trait - what
//! a read publishes, what a write takes, and the shape a handle answers.

use super::counting;

#[test]
fn retained_text_options_override_structured_content_inference() {
    use arrow_array::StringArray;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::text::TextOptions;
    use yggdryl::{IOMedia as _, MimeType};

    let bytes = Buffer::from_bytes(b"[INFO] first\n[WARN] second\n".to_vec())
        .with_media_type(MimeType::JSON.into());
    let options = TextOptions::new()
        .try_with_rowheader(r"^\[(?<level>[A-Z]+)\] ")
        .unwrap()
        .with_max_row_size(1);
    let source = Holder::from(bytes).into_text_with(options);
    let options = source.record_options().unwrap();
    let batches = source
        .read_arrow_reader(&options)
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].num_rows(), 1);
    let level = batches[0]
        .column_by_name("level")
        .unwrap()
        .as_any()
        .downcast_ref::<StringArray>()
        .unwrap();
    assert_eq!(level.value(0), "INFO");
}

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
    fn an_xml_handle_reads_and_writes_the_document_its_root_names() {
        // XML proves text and nothing else, so the untyped round trip holds
        // for text; a field types the same document below.
        let expected = Scalar::from_struct([(
            "trade",
            Scalar::from_struct([
                ("quantity", Scalar::from("2")),
                ("symbol", Scalar::from("AAPL")),
            ])
            .unwrap(),
        )])
        .unwrap();
        for name in ["trade.xml", "trade.xml.gz"] {
            let media = Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type();
            let mut handle = Buffer::new().with_media_type(media);
            handle
                .write_scalar(&expected)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(handle.read_scalar(None).unwrap(), expected, "{name}");
        }

        let media = Url::from_str("file:///trade.xml").unwrap().media_type();
        let source = Buffer::from_bytes(b"<trade><quantity>2</quantity></trade>".to_vec())
            .with_media_type(media);
        let field = Field::from_str("trade: struct<quantity: int32 not null> not null").unwrap();
        assert_eq!(
            source.read_scalar(Some(&field)).unwrap(),
            Scalar::from_sequence([Scalar::from(2)])
        );
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
            // A cadence counts whole batches, so the row adapter cuts one
            // row a batch for every row to be its own publication.
            .with_batch_row_size(1)
            .with_commit_batch_num(1);
        reader_handle
            .write_arrow_reader(
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1]), rows_batch(&[2])],
                ),
                IOMode::Overwrite,
                &options,
            )
            .unwrap();
        assert_eq!(reader_handle.publications.load(Ordering::SeqCst), 2);

        // A held batch is one batch, and a cadence never cuts one.
        let mut batch_handle =
            PublicationProbe::new("generic-batch-commits.arrows", Arc::clone(&pulls));
        batch_handle
            .write_arrow_batch(batch(), IOMode::Overwrite, &options)
            .unwrap();
        assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 1);

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

    /// Every Arrow shape redirects to the same serie intent; row adapters
    /// retain their native intake before that boundary.
    struct TypedDispatchProbe {
        handle: Buffer,
        serie_calls: [usize; 3],
        record_calls: [usize; 3],
    }

    impl TypedDispatchProbe {
        fn new() -> Self {
            Self {
                handle: handle("typed-dispatch-probe.arrows"),
                serie_calls: [0; 3],
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

        fn overwrite_serie(
            &mut self,
            _value: yggdryl::Serie,
            _options: Option<&RecordOptions>,
        ) -> yggdryl::Result<yggdryl::IOResult> {
            self.serie_calls[0] += 1;
            Ok(yggdryl::IOResult::default())
        }

        fn append_serie(
            &mut self,
            _value: yggdryl::Serie,
            _options: Option<&RecordOptions>,
        ) -> yggdryl::Result<yggdryl::IOResult> {
            self.serie_calls[1] += 1;
            Ok(yggdryl::IOResult::default())
        }

        fn merge_serie(
            &mut self,
            _value: yggdryl::Serie,
            _options: Option<&RecordOptions>,
        ) -> yggdryl::Result<yggdryl::IOResult> {
            self.serie_calls[2] += 1;
            Ok(yggdryl::IOResult::default())
        }

        fn overwrite_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> yggdryl::Result<yggdryl::IOResult>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[0] += 1;
            Ok(yggdryl::IOResult::default())
        }

        fn append_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> yggdryl::Result<yggdryl::IOResult>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[1] += 1;
            Ok(yggdryl::IOResult::default())
        }

        fn merge_records<I, R>(
            &mut self,
            _records: I,
            _options: &RecordOptions,
        ) -> yggdryl::Result<yggdryl::IOResult>
        where
            Self: Sized,
            I: IntoIterator<Item = R>,
            I::IntoIter: Send + 'static,
            R: TryInto<Scalar>,
            R::Error: Into<Error>,
        {
            self.record_calls[2] += 1;
            Ok(yggdryl::IOResult::default())
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

        assert_eq!(probe.serie_calls, [2, 2, 2]);
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

        assert_eq!(probe.serie_calls, [1, 1, 0]);
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

    fn create_bytes(&mut self, bytes: &[u8]) -> yggdryl::Result<()> {
        self.handle.create_bytes(bytes)
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

    fn uri(&self) -> Option<&yggdryl::Uri> {
        self.handle.uri()
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

    /// A leaf's schema is its encoding's own header or footer typed by the
    /// plan its options state - the filter and the selection in the phases a
    /// read runs them in - and it is the schema that leaf's read publishes,
    /// whatever the encoding: a `where` over a column the `select` builds is
    /// typed after it, and one that does not bind is refused.
    #[test]
    fn a_leafs_schema_is_the_shape_its_read_publishes() {
        let mut names = vec!["typed.arrows", "typed.avro", "typed.csv"];
        if cfg!(feature = "parquet") {
            names.push("typed.parquet");
        }
        for name in names {
            let handle = stored(name);
            let plain = handle.record_options().unwrap();
            for options in [
                plain.clone(),
                plain.clone().with_select("price, id").unwrap(),
                plain
                    .clone()
                    .with_select("id, price * 2 as doubled")
                    .unwrap()
                    .with_filter("doubled > 3")
                    .unwrap(),
                plain
                    .clone()
                    .with_filter("symbol = 'AAPL'")
                    .unwrap()
                    .with_select("id")
                    .unwrap(),
            ] {
                let field = handle
                    .read_arrow_field(&options)
                    .unwrap_or_else(|error| panic!("{name}: {error}"));
                let read = handle.read_arrow_reader(&options).unwrap().schema();
                assert_eq!(
                    field,
                    Field::from_arrow_schema(options.name(), read.as_ref()).unwrap(),
                    "{name}: {}",
                    options.plan()
                );
            }
            let unbound = plain.with_filter("nowhere > 1").unwrap();
            assert!(handle.read_arrow_field(&unbound).is_err(), "{name}");
        }
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
    fn a_named_row_leaving_out_a_declared_column_writes_it_absent() {
        let mut handle = handle("named-rows.arrows");
        let options = handle.record_options().unwrap().with_field(schema());

        // A nullable column the row leaves out is null, not its default.
        handle
            .overwrite_records(
                [Scalar::from_struct([("id", Scalar::from(1_i64))]).unwrap()],
                &options,
            )
            .unwrap();
        let batch = handle
            .read_arrow_reader(&options)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(batch.column(1).null_count(), 1);

        // A required one is refused by path rather than stored as `0`, as
        // the bindings' record writers refuse it; the value door still fills
        // the canonical default.
        let row = Scalar::from_struct([("symbol", Scalar::from("AAPL"))]).unwrap();
        let refused = handle
            .overwrite_records([row.clone()], &options)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("id"), "{refused}");
        assert!(refused.contains("null"), "{refused}");
        assert_eq!(rows(&handle, &options), 1);
        assert_eq!(
            schema().scalar(row).unwrap(),
            Scalar::from_sequence([Scalar::from(0_i64), Scalar::from("AAPL")])
        );

        // A name the root does not declare is still refused.
        let extra =
            Scalar::from_struct([("id", Scalar::from(2_i64)), ("venue", Scalar::from("XNAS"))])
                .unwrap();
        let refused = handle
            .append_records([extra], &options)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("venue"), "{refused}");
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
            // One row a batch, one batch a commit: row one publishes before
            // row two converts.
            let committed = plain
                .clone()
                .with_batch_row_size(1)
                .with_commit_batch_num(1);
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
    fn native_rows_publish_the_whole_batches_a_cadence_counts() {
        // Two rows a batch, two batches a commit: the second batch closes on
        // the failing fourth row, so the three rows before it publish as
        // one cadence after that row's conversion, and a cadence never
        // cuts a batch to publish sooner.
        let conversions = Arc::new(AtomicUsize::new(0));
        let mut handle =
            PublicationProbe::new("native-non-divisible.arrows", Arc::clone(&conversions));
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_batch_row_size(2)
            .with_commit_batch_num(2);
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
            [4],
            "the second batch closes on row four, then the two batches publish"
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
        let message = handle("t.orc").record_options().unwrap_err().to_string();
        assert!(message.contains("application/vnd.apache.orc"), "{message}");
    }

    /// A Parquet file's key-value pairs and a declared root's own metadata
    /// ride its footer, and its reader lands the rows under a root stating
    /// none of them: the schema a leaf answers is that root, never a richer
    /// one the rows then contradict.
    #[cfg(feature = "parquet")]
    #[test]
    fn a_parquet_leaf_answers_the_root_its_rows_land_under() {
        let mut handle = handle("t.parquet");
        let RecordOptions::Parquet(parquet) = handle.record_options().unwrap() else {
            panic!("a .parquet name reads as Parquet");
        };
        let mut declared = schema();
        declared.set_metadata([("comment", "trades")]).unwrap();
        let options = RecordOptions::Parquet(parquet.with_key_value("writer", "rust"))
            .with_field(declared)
            .with_safe(true);
        handle.overwrite_arrow_reader(reader(), &options).unwrap();

        let plain = handle.record_options().unwrap();
        let field = handle.read_arrow_field(&plain).unwrap();
        assert_eq!(field, schema());
        assert_eq!(
            Some(&field),
            handle.read_serie(Some(&plain)).unwrap().field()
        );
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
    fn commit_batch_num_controls_exact_publication_counts() {
        // Three batches of two, one and one rows: a cadence counts the
        // batches, and an unset one publishes a leaf once.
        for (label, cadence, expected) in [
            ("unset", None, 1),
            ("one", Some(1), 3),
            ("two", Some(2), 2),
            ("larger-than-stream", Some(10), 1),
        ] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle = PublicationProbe::new(
                &format!("commit-publications-{label}.arrows"),
                Arc::clone(&pulls),
            );
            let mut options = handle.record_options().unwrap().with_field(schema());
            options.set_commit_batch_num(cadence);
            let source = yggdryl::arrow::batch_reader(
                schema().into_arrow_schema().unwrap(),
                [rows_batch(&[1, 2]), rows_batch(&[3]), rows_batch(&[4])],
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

            let options = plain.clone().with_commit_batch_num(1);
            let incoming = yggdryl::arrow::batch_reader(
                schema().into_arrow_schema().unwrap(),
                [rows_batch(&[1, 3]), rows_batch(&[4, 5])],
            );
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(incoming, &options).unwrap(),
                "append" => handle.append_arrow_reader(incoming, &options).unwrap(),
                "merge" => handle
                    .merge_arrow_reader(incoming, &options.with_merge_by(["id"]).unwrap())
                    .unwrap(),
                _ => unreachable!(),
            };

            // Every cadence's rows are in the one answer.
            assert_eq!(result, yggdryl::IOResult::new(4, 4), "{intent}");
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
            .with_batch_row_size(1)
            .with_commit_batch_num(1);
        // A held batch is one batch, published once whatever its rows.
        batch_handle
            .overwrite_arrow_batch(rows_batch(&[1, 2]), &options)
            .unwrap();
        assert_eq!(batch_handle.publications.load(Ordering::SeqCst), 1);

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
    fn zero_commit_batch_num_is_rejected_before_any_input_pull() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle =
                PublicationProbe::new(&format!("zero-commit-{intent}.arrows"), Arc::clone(&pulls));
            let options = handle
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_commit_batch_num(0);
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(source, &options),
                "append" => handle.append_arrow_reader(source, &options),
                "merge" => handle
                    .merge_arrow_reader(source, &options.clone().with_merge_by(["id"]).unwrap()),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("commit_batch_num"), "{intent}: {message}");
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
            .with_commit_batch_num(0);
        let message = handle
            .overwrite_records(records, &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("commit_batch_num"), "{message}");
        assert_eq!(pulls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn zero_num_threads_is_rejected_before_any_input_pull() {
        for intent in ["overwrite", "append", "merge"] {
            let pulls = Arc::new(AtomicUsize::new(0));
            let mut handle =
                PublicationProbe::new(&format!("zero-threads-{intent}.arrows"), Arc::clone(&pulls));
            let options = handle
                .record_options()
                .unwrap()
                .with_field(schema())
                .with_num_threads(0);
            let source = counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1]))]);
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(source, &options),
                "append" => handle.append_arrow_reader(source, &options),
                "merge" => handle
                    .merge_arrow_reader(source, &options.clone().with_merge_by(["id"]).unwrap()),
                _ => unreachable!(),
            };

            let message = result.unwrap_err().to_string();
            assert!(message.contains("$.num_threads"), "{intent}: {message}");
            assert!(
                message.contains("non-zero thread count"),
                "{intent}: {message}"
            );
            assert_eq!(pulls.load(Ordering::SeqCst), 0, "{intent}");
            assert_eq!(handle.publications.load(Ordering::SeqCst), 0, "{intent}");
        }

        // The row doors refuse it before the first row is pulled too.
        let pulls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&pulls);
        let records = std::iter::from_fn(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Some(NativeRow {
                id: 1,
                symbol: None,
            })
        });
        let mut handle = handle("zero-threads-native.arrows");
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            .with_num_threads(0);
        let message = handle
            .overwrite_records(records, &options)
            .unwrap_err()
            .to_string();
        assert!(message.contains("$.num_threads"), "{message}");
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
            let options = plain.clone().with_commit_batch_num(2);
            let source = counted_source(
                Arc::clone(&pulls),
                [
                    Ok(rows_batch(&[2])),
                    Ok(rows_batch(&[3])),
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
                [2],
                "{intent}: the third batch must not be pulled before commit one publishes"
            );
            assert_eq!(
                pulls.load(Ordering::SeqCst),
                4,
                "{intent}: the one-batch second cadence is discarded when its next pull fails"
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
            .with_commit_batch_num(1);
        let source = yggdryl::arrow::batch_reader(
            schema().into_arrow_schema().unwrap(),
            [rows_batch(&[1, 2]), rows_batch(&[3, 4])],
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
    fn a_session_without_a_cadence_publishes_by_bytes_so_small_chunks_wait_for_finish() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-default-cadence.arrows", pulls);
        let options = handle.record_options().unwrap().with_field(schema());
        assert_eq!(options.commit_batch_num(), None);
        let mut session = ArrowWriteSession::overwrite(&options).unwrap();

        for ids in [&[1_i64, 2][..], &[3], &[4]] {
            assert!(
                session
                    .push(
                        &mut handle,
                        yggdryl::arrow::batch_reader(
                            schema().into_arrow_schema().unwrap(),
                            [rows_batch(ids)],
                        ),
                    )
                    .unwrap()
            );
        }
        // Three chunks are far under `DEFAULT_COMMIT_BYTE_SIZE`, so nothing
        // publishes until the input ends - and a session with no cadence
        // is no longer refused, because it publishes by bytes.
        assert_eq!(handle.publications.load(Ordering::SeqCst), 0);
        session.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
        assert_eq!(rows(&handle, &options), 4);
    }

    #[test]
    fn resumed_write_publishes_complete_cadences_and_abort_drops_only_the_remainder() {
        let pulls = Arc::new(AtomicUsize::new(0));
        let mut handle = PublicationProbe::new("resumed-write.arrows", pulls);
        let options = handle
            .record_options()
            .unwrap()
            .with_field(schema())
            // Two batches a commit: the first two chunks publish together.
            .with_commit_batch_num(2);
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
            .with_commit_batch_num(1)
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
            .with_commit_batch_num(2)
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
        // One row a batch, one batch a commit.
        let append_options = plain.clone().with_commit_batch_num(1);
        let mut append = ArrowWriteSession::append(&append_options).unwrap();
        append
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[3]), rows_batch(&[4])],
                ),
            )
            .unwrap();
        append.finish(&mut handle).unwrap();
        assert_eq!(handle.publications.load(Ordering::SeqCst), 2);
        assert_eq!(rows(&handle, &plain), 4);

        handle.reset_publications();
        let merge_options = plain
            .clone()
            .with_commit_batch_num(1)
            .with_merge_by(["id"])
            .unwrap();
        let mut merge = ArrowWriteSession::merge(&merge_options).unwrap();
        merge
            .push(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[2]), rows_batch(&[5])],
                ),
            )
            .unwrap();
        merge.finish(&mut handle).unwrap();
        // The first cadence replays row 2 exactly as it is stored, which a
        // merge leaves unwritten - an append would have published it twice
        // over - and the second merges in the new key 5.
        assert_eq!(handle.publications.load(Ordering::SeqCst), 1);
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
            .with_commit_batch_num(10);
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
            .with_commit_batch_num(1);
        let mut exact_session = ArrowWriteSession::overwrite(&exact_options).unwrap();
        exact_session
            .push(
                &mut exact,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1, 2]), rows_batch(&[3, 4])],
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
            .with_commit_batch_num(2);
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
            .with_commit_batch_num(1);
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
            .with_commit_batch_num(1);
        let mut publication_session = ArrowWriteSession::overwrite(&publication_options).unwrap();
        let error = publication_session
            .push(
                &mut publication,
                yggdryl::arrow::batch_reader(
                    schema().into_arrow_schema().unwrap(),
                    [rows_batch(&[1]), rows_batch(&[2])],
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
            .with_commit_batch_num(1);
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
        let bounded = plain.clone().with_commit_batch_num(2);
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

mod results {
    //! What every write door answers: the rows it read off its source, the
    //! rows that reached the destination, and the difference.

    use super::*;
    use yggdryl::{IOResult, Serie};

    fn options(handle: &Buffer) -> RecordOptions {
        handle.record_options().unwrap().with_field(schema())
    }

    fn batches(ids: &[&[i64]]) -> BatchReader {
        yggdryl::arrow::batch_reader(
            schema().into_arrow_schema().unwrap(),
            ids.iter().map(|ids| rows_batch(ids)).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn an_empty_source_answers_the_empty_result() {
        let mut handle = handle("empty-result.arrows");
        let options = options(&handle);

        for intent in ["overwrite", "append", "merge"] {
            let result = match intent {
                "overwrite" => handle.overwrite_arrow_reader(batches(&[]), &options),
                "append" => handle.append_arrow_reader(batches(&[]), &options),
                _ => handle.merge_arrow_reader(
                    batches(&[]),
                    &options.clone().with_merge_by(["id"]).unwrap(),
                ),
            }
            .unwrap();
            assert_eq!(result, IOResult::default(), "{intent}");
            assert!(result.is_empty(), "{intent}");
        }
    }

    #[test]
    fn every_intent_answers_the_rows_it_read_and_wrote() {
        let mut handle = handle("intents.arrows");
        let options = options(&handle);

        let overwritten = handle
            .overwrite_arrow_reader(batches(&[&[1, 2], &[3]]), &options)
            .unwrap();
        assert_eq!(overwritten, IOResult::new(3, 3));
        assert_eq!(rows(&handle, &options), 3);

        let appended = handle
            .append_arrow_reader(batches(&[&[4, 5]]), &options)
            .unwrap();
        assert_eq!(appended, IOResult::new(2, 2));
        assert_eq!(rows(&handle, &options), 5);

        // A merge writes every incoming row - the ones that update a stored
        // row and the ones that add one - so the stored count says which.
        let merged = handle
            .merge_arrow_reader(
                batches(&[&[5, 6]]),
                &options.clone().with_merge_by(["id"]).unwrap(),
            )
            .unwrap();
        assert_eq!(merged, IOResult::new(2, 2));
        assert_eq!(rows(&handle, &options), 6);
    }

    #[test]
    fn every_shape_answers_the_same_result() {
        let field = schema();
        let incoming = || rows_batch(&[1, 2, 3]);
        for shape in ["reader", "batch", "records", "serie", "mode"] {
            let mut handle = handle(&format!("shape-{shape}.arrows"));
            let options = options(&handle);
            let result = match shape {
                "reader" => handle.overwrite_arrow_reader(batches(&[&[1, 2, 3]]), &options),
                "batch" => handle.overwrite_arrow_batch(incoming(), &options),
                "records" => handle.overwrite_records(
                    [1_i64, 2, 3]
                        .map(|id| Scalar::from_sequence([Scalar::from(id), Scalar::from("S")])),
                    &options,
                ),
                "serie" => handle.overwrite_serie(
                    Serie::from_arrow_batch(Some(&field), &incoming(), Default::default()).unwrap(),
                    Some(&options),
                ),
                _ => handle.write_arrow_reader(batches(&[&[1, 2, 3]]), IOMode::Overwrite, &options),
            }
            .unwrap_or_else(|error| panic!("{shape}: {error}"));
            assert_eq!(result, IOResult::new(3, 3), "{shape}");
            assert_eq!(rows(&handle, &options), 3, "{shape}");
        }
    }

    #[test]
    fn a_where_keeps_rows_out_and_they_are_skipped() {
        let mut handle = handle("filtered.arrows");
        let plain = options(&handle);
        let filtered = plain.clone().with_filter("id > 2").unwrap();

        let result = handle
            .overwrite_arrow_reader(batches(&[&[1, 2], &[3, 4, 5]]), &filtered)
            .unwrap();

        assert_eq!(
            (result.read_rows, result.written_rows, result.skipped_rows),
            (5, 3, 2)
        );
        assert_eq!(rows(&handle, &plain), 3);

        // Every row kept out is a source that was read and a write of none.
        let none = handle
            .append_arrow_reader(
                batches(&[&[1, 2]]),
                &plain.clone().with_filter("id > 9").unwrap(),
            )
            .unwrap();
        assert_eq!(
            (none.read_rows, none.written_rows, none.skipped_rows),
            (2, 0, 2)
        );
        assert!(!none.is_empty());
        assert_eq!(rows(&handle, &plain), 3);
    }

    #[test]
    fn a_row_bound_cuts_the_batch_it_falls_in_and_pulls_no_further() {
        let mut handle = handle("bounded.arrows");
        let plain = options(&handle);
        let bounded = plain.clone().with_max_row_size(3);

        // The bound falls inside the second batch: its last row was read and
        // not written, and the third batch was never pulled.
        let result = handle
            .append_arrow_reader(batches(&[&[1, 2], &[3, 4], &[5, 6]]), &bounded)
            .unwrap();

        assert_eq!(
            (result.read_rows, result.written_rows, result.skipped_rows),
            (4, 3, 1)
        );
        assert_eq!(rows(&handle, &plain), 3);
    }

    #[test]
    fn a_session_answers_the_rows_of_every_chunk_it_was_pushed() {
        let mut handle = handle("session-result.arrows");
        let options = options(&handle)
            .with_filter("id > 1")
            .unwrap()
            .with_commit_batch_num(1);
        let plain = handle.record_options().unwrap();

        let mut session = ArrowWriteSession::append(&options).unwrap();
        assert!(session.push(&mut handle, batches(&[&[1, 2]])).unwrap());
        assert!(session.push(&mut handle, batches(&[&[3], &[4]])).unwrap());
        let result = session.finish(&mut handle).unwrap();

        assert_eq!(
            (result.read_rows, result.written_rows, result.skipped_rows),
            (4, 3, 1)
        );
        assert_eq!(rows(&handle, &plain), 3);
    }

    #[test]
    fn a_document_answers_the_rows_it_rendered() {
        let field = schema();
        for name in ["rows.json", "rows.jsonl", "rows.yaml"] {
            let mut handle = handle(name);
            let rows =
                Serie::from_arrow_batch(Some(&field), &rows_batch(&[1, 2, 3]), Default::default())
                    .unwrap();

            let result = handle
                .overwrite_serie(rows, None)
                .unwrap_or_else(|error| panic!("{name}: {error}"));

            assert_eq!(result, IOResult::new(3, 3), "{name}");
        }
    }
}

mod record_columns {
    //! `read_serie` and `write_serie` over the record encodings: a stream of
    //! record columns in, and the same rows back out, under the stored
    //! schema or a declared root.

    use super::handle;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{
        ArrowCastOptions, DataType, Field, IOMedia, IOMode, MediaType, MimeType, Scalar, Serie,
        StreamChunkedSerie, StructType,
    };

    #[test]
    fn the_overwrite_serie_primitive_replaces_a_structured_document() {
        let root = DataType::from(
            StructType::from_fields([DataType::Int64.required_field("qty")]).unwrap(),
        )
        .required_field("row");
        let value =
            Serie::from_scalars(root, [Scalar::from_sequence([Scalar::from(2_i64)])]).unwrap();
        let mut output = handle("serie-primitive.json");
        assert_eq!(
            output
                .overwrite_serie(value.clone(), None)
                .unwrap()
                .written_rows,
            1
        );
        assert_eq!(
            Serie::from(
                yggdryl::StreamChunkedSerie::from_serie(output.read_serie(None).unwrap())
                    .expect("native record stream")
            ),
            value
        );
    }

    #[test]
    fn a_coding_reads_its_structured_document_through_the_serie_primitive() {
        use yggdryl::IOBase;
        let root = DataType::from(
            StructType::from_fields([DataType::Int64.required_field("qty")]).unwrap(),
        )
        .required_field("row");
        let value =
            Serie::from_scalars(root, [Scalar::from_sequence([Scalar::from(2_i64)])]).unwrap();
        let mut output = yggdryl::coding::Coding::new(handle("coded.json"), yggdryl::Codec::Gzip);
        output.overwrite_serie(value.clone(), None).unwrap();
        output.flush().unwrap();
        assert_eq!(
            Serie::from(
                yggdryl::StreamChunkedSerie::from_serie(output.read_serie(None).unwrap())
                    .expect("native record stream")
            ),
            value
        );
    }

    #[test]
    fn an_arrow_overwrite_reaches_a_leaf_with_the_source_arrays_untouched() {
        use arrow_array::{ArrayRef, Int64Array};
        use std::sync::Arc;
        use yggdryl::holder::Buffer;
        use yggdryl::{IOBase, IOResult};
        struct Leaf {
            handle: Buffer,
            column: ArrayRef,
        }
        impl IOMedia for Leaf {
            yggdryl::impl_default_iomedia!();
            fn overwrite_prepared_serie(
                &mut self,
                value: StreamChunkedSerie,
                _options: &RecordOptions,
            ) -> yggdryl::Result<()> {
                let mut batches = value.into_arrow_reader();
                let batch = batches.next().unwrap()?;
                assert!(Arc::ptr_eq(batch.column(0), &self.column));
                assert!(batches.next().is_none());
                Ok(())
            }
        }
        impl IOBase for Leaf {
            yggdryl::delegate_iobase!(handle);
        }
        let column: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
        let batch =
            arrow_array::RecordBatch::try_from_iter([("qty", Arc::clone(&column))]).unwrap();
        let mut leaf = Leaf {
            handle: handle("transport.arrows"),
            column,
        };
        let options = leaf.record_options().unwrap();
        let batches = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        assert_eq!(
            leaf.overwrite_arrow_reader(batches, &options).unwrap(),
            IOResult::new(2, 2)
        );
    }

    /// The non-null record root the rows land under.
    fn record(fields: impl IntoIterator<Item = Field>) -> Field {
        StructType::from_fields(fields)
            .map(DataType::from)
            .expect("the root datatype is valid")
            .required_field("row")
    }

    fn quote_root() -> Field {
        record([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("size"),
        ])
    }

    fn quote_rows() -> Vec<Scalar> {
        vec![
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
        ]
    }

    /// The stream of one record column a write takes, over `rows`.
    fn stream_of(root: &Field, rows: Vec<Scalar>) -> StreamChunkedSerie {
        let column = Serie::from_scalars(root.clone(), rows).expect("the rows materialize");
        StreamChunkedSerie::from_serie(column).expect("a record column is one stream")
    }

    fn quotes() -> StreamChunkedSerie {
        stream_of(&quote_root(), quote_rows())
    }

    /// Record options carrying the declared root a read lands under.
    fn declaring(field: &Field) -> RecordOptions {
        let mut options = RecordOptions::for_media_type(&MediaType::new(MimeType::ARROW_STREAM))
            .expect("the IPC encoding is built in");
        options.set_field(field.clone());
        options
    }

    /// Every row a stream of record columns yields, as one sequence.
    fn drained(reader: StreamChunkedSerie) -> Scalar {
        let columns = reader
            .into_chunks()
            .collect::<Result<Vec<Serie>, _>>()
            .expect("every batch lands");
        Scalar::from_sequence(columns.iter().flat_map(|column| column.rows().into_owned()))
    }

    /// The rows `name` holds after `quotes()` was written to it.
    fn stored(name: &str) -> StreamChunkedSerie {
        let mut target = handle(name);
        target
            .write_serie(Serie::from(quotes()), IOMode::Overwrite, None)
            .unwrap_or_else(|error| panic!("{name} writes: {error}"));
        StreamChunkedSerie::from_serie(
            target
                .read_serie(None)
                .unwrap_or_else(|error| panic!("{name} reads: {error}")),
        )
        .expect("native record stream")
    }

    fn nested_root() -> Field {
        record([
            StructType::from_fields([
                DataType::utf8().required_field("mic"),
                DataType::Int64.required_field("rank"),
            ])
            .map(DataType::from)
            .expect("the child datatype is valid")
            .required_field("venue"),
            DataType::serie(DataType::Int64.required_field("item")).required_field("sizes"),
            DataType::utf8().nullable_field("note"),
            DataType::Decimal128 {
                precision: 12,
                scale: 2,
            }
            .required_field("price"),
            DataType::date32().required_field("day"),
        ])
    }

    fn nested_rows() -> Vec<Scalar> {
        vec![
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from("XPAR"), Scalar::from(1_i64)]),
                Scalar::from_sequence([Scalar::from(100_i64), Scalar::from(250_i64)]),
                Scalar::from("lit"),
                Scalar::decimal128(12_550, 2),
                Scalar::date32(19_876),
            ]),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from("XNAS"), Scalar::from(2_i64)]),
                Scalar::from_sequence([]),
                Scalar::Null,
                Scalar::decimal128(1, 2),
                Scalar::date32(0),
            ]),
        ]
    }

    #[test]
    fn every_unconditional_record_encoding_round_trips_under_its_stored_schema() {
        for name in ["quotes.arrows", "quotes.avro"] {
            let read = stored(name);
            // The root is the stored schema, stated before a batch is pulled.
            assert_eq!(read.field(), &quote_root(), "{name}");
            assert_eq!(drained(read), Scalar::from_sequence(quote_rows()), "{name}");
        }
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn parquet_round_trips_the_same_rows_under_the_same_root() {
        let read = stored("quotes.parquet");
        assert_eq!(read.field(), &quote_root());
        assert_eq!(drained(read), Scalar::from_sequence(quote_rows()));
    }

    #[test]
    fn an_append_keeps_the_rows_a_record_encoding_already_holds() {
        let mut target = handle("quotes.arrows");
        target
            .write_serie(Serie::from(quotes()), IOMode::Overwrite, None)
            .expect("the rows write");
        target
            .write_serie(Serie::from(quotes()), IOMode::Append, None)
            .expect("the rows append");

        let read = yggdryl::StreamChunkedSerie::from_serie(
            target.read_serie(None).expect("the rows read"),
        )
        .expect("native record stream");
        assert_eq!(
            drained(read),
            Scalar::from_sequence([quote_rows(), quote_rows()].concat())
        );
    }

    #[test]
    fn a_declared_root_casts_the_rows_a_record_encoding_stored() {
        let mut target = handle("quotes.arrows");
        target
            .write_serie(Serie::from(quotes()), IOMode::Overwrite, None)
            .expect("the rows write");

        let declared = record([
            DataType::utf8().required_field("symbol"),
            DataType::Decimal128 {
                precision: 12,
                scale: 4,
            }
            .required_field("size"),
        ]);
        let read = yggdryl::StreamChunkedSerie::from_serie(
            target
                .read_serie(Some(&declaring(&declared)))
                .expect("the declared root casts the stored int64 column"),
        )
        .expect("native record stream");

        assert_eq!(read.field(), &declared);
        assert_eq!(
            drained(read),
            Scalar::from_sequence([
                Scalar::from_sequence([Scalar::from("AAPL"), Scalar::decimal128(1_000_000, 4)]),
                Scalar::from_sequence([Scalar::from("MSFT"), Scalar::decimal128(2_500_000, 4)]),
            ])
        );
    }

    #[test]
    fn one_plan_lands_every_batch_a_stored_stream_yields_under_the_declared_root() {
        let batch = Serie::from_scalars(quote_root(), quote_rows())
            .expect("the rows materialize")
            .into_arrow_batch()
            .expect("a record column is one table");
        let stream = StreamChunkedSerie::from_arrow_reader(
            None,
            yggdryl::arrow::batch_reader(batch.schema(), vec![batch; 3]),
            ArrowCastOptions::default(),
        )
        .expect("the reader names its root");
        let mut target = handle("stream.arrows");
        target
            .write_serie(Serie::from(stream), IOMode::Overwrite, None)
            .expect("the stream writes");

        let declared = record([
            DataType::utf8().required_field("symbol"),
            DataType::Float64.required_field("size"),
        ]);
        let columns = yggdryl::StreamChunkedSerie::from_serie(
            target
                .read_serie(Some(&declaring(&declared)))
                .expect("the plan compiles from the stored schema"),
        )
        .expect("native record stream")
        .into_chunks()
        .collect::<Result<Vec<Serie>, _>>()
        .expect("every batch lands");

        // The plan is compiled once, so every batch - not only the first -
        // lands under the declared root.
        assert_eq!(columns.len(), 3);
        for column in columns {
            assert_eq!(column.field(), Some(&declared));
            assert_eq!(
                Scalar::from(column),
                Scalar::from_sequence([
                    Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100.0_f64)]),
                    Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250.0_f64)]),
                ])
            );
        }
    }

    #[test]
    fn an_empty_table_round_trips_and_keeps_the_columns_it_declared() {
        let empty = Serie::from_scalars(quote_root(), []).expect("no rows still materialize");
        assert_eq!(empty.len(), 0);

        let mut target = handle("empty.arrows");
        target
            .write_serie(
                Serie::from(
                    StreamChunkedSerie::from_serie(empty).expect("a record column is one stream"),
                ),
                IOMode::Overwrite,
                None,
            )
            .expect("the rows write");

        let read = yggdryl::StreamChunkedSerie::from_serie(
            target.read_serie(None).expect("the rows read"),
        )
        .expect("native record stream");
        // The schema is what an empty table carries, so it is the whole claim.
        assert_eq!(read.field(), &quote_root());
        assert_eq!(drained(read), Scalar::from_sequence([]));
    }

    #[test]
    fn a_thousand_rows_survive_the_round_trip_in_order() {
        let rows: Vec<Scalar> = (0..1_000_i64)
            .map(|index| {
                Scalar::from_sequence([Scalar::from(format!("S{index}")), Scalar::from(index)])
            })
            .collect();

        let mut target = handle("wide.arrows");
        target
            .write_serie(
                Serie::from(stream_of(&quote_root(), rows.clone())),
                IOMode::Overwrite,
                None,
            )
            .expect("the rows write");

        let read = yggdryl::StreamChunkedSerie::from_serie(
            target.read_serie(None).expect("the rows read"),
        )
        .expect("native record stream");
        assert_eq!(drained(read), Scalar::from_sequence(rows));
    }

    #[test]
    fn nested_children_keep_their_values_in_a_record_encoding() {
        let mut target = handle("nested.arrows");
        target
            .write_serie(
                Serie::from(stream_of(&nested_root(), nested_rows())),
                IOMode::Overwrite,
                None,
            )
            .expect("the rows write");

        let read = yggdryl::StreamChunkedSerie::from_serie(
            target
                .read_serie(Some(&declaring(&nested_root())))
                .expect("the rows read"),
        )
        .expect("native record stream");
        assert_eq!(drained(read), Scalar::from_sequence(nested_rows()));
    }

    #[test]
    fn a_record_encoding_names_every_nested_child_it_stored() {
        let mut target = handle("nested.arrows");
        target
            .write_serie(
                Serie::from(stream_of(&nested_root(), nested_rows())),
                IOMode::Overwrite,
                None,
            )
            .expect("the rows write");

        // Nothing is declared on the read: the struct child, the serie item,
        // the nullable column, the decimal, and the temporal all come back
        // named and parameterized by the schema the write stored.
        let read = yggdryl::StreamChunkedSerie::from_serie(
            target
                .read_serie(None)
                .expect("the stored schema names the columns"),
        )
        .expect("native record stream");
        assert_eq!(read.field(), &nested_root());
    }
}

mod shape {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    use super::counting::Counting;
    use yggdryl::coding::Coding;
    use yggdryl::holder::Holder;
    use yggdryl::{Codec, DataType, IOKind, MediaType, MimeType};

    /// A writable temporary root of this test's own.
    fn root(label: &str) -> std::path::PathBuf {
        let mut path = yggdryl::local::LocalFolder::temporary()
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
        let missing = yggdryl::local::LocalPath::new(path.join("trades.parquet")).unwrap();
        assert_eq!(missing.kind(), IOKind::Unknown);
        assert_eq!(missing.media_type().base(), &MimeType::PARQUET);
        assert!(missing.is_tabular());
        assert!(!missing.is_atomic());

        let notes = yggdryl::local::LocalPath::new(path.join("notes.txt")).unwrap();
        assert_eq!(notes.kind(), IOKind::Unknown);
        assert!(notes.is_atomic());
        assert!(!notes.is_tabular());

        // The leaf implementation answers the same, existing or not.
        let leaf = yggdryl::local::LocalFile::new(path.join("trades.arrows")).unwrap();
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

        let folder = yggdryl::local::LocalFolder::new(&lake).unwrap();
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
        let folder = yggdryl::local::LocalFolder::new(&logs).unwrap();
        assert!(!folder.is_tabular());
        assert!(!folder.is_atomic());
        assert!(!folder.is_io());

        // So is an empty one, and so is a folder that does not exist yet.
        let empty = yggdryl::local::LocalFolder::new(path.join("empty")).unwrap();
        assert!(!empty.is_tabular());
        assert!(!empty.is_atomic());
        assert!(!empty.is_io());

        // A location resolving to that lake answers exactly as the folder did.
        let located = yggdryl::local::LocalPath::new(&lake).unwrap();
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
            let mut leaf = yggdryl::local::LocalPath::new(lake.join(name)).expect("a lazy leaf");
            let batch = rows(&values);
            let options = leaf.record_options().expect("IPC options");
            leaf.overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .expect("a published IPC leaf");
        }
        yggdryl::local::LocalPath::new(lake.join("notes.txt"))
            .expect("a text leaf")
            .write_all_bytes(b"not a table row")
            .expect("a published unrelated leaf");

        let folder = yggdryl::local::LocalFolder::new(&lake).expect("the lake folder");
        assert_eq!(folder.row_size().expect("metadata row count"), 3);
        assert_eq!(folder.column_size().expect("metadata field width"), 1);

        let _ = std::fs::remove_dir_all(&path);
    }

    /// A lake of two Arrow stream leaves - two rows, then one - and a note
    /// beside them that is no part of the table.
    fn ipc_lake(path: &std::path::Path) -> std::path::PathBuf {
        use std::sync::Arc;

        use arrow_array::{Int64Array, RecordBatch};
        use yggdryl::IOMedia as _;

        let lake = path.join("lake");
        for (name, values) in [("a.arrows", vec![1_i64, 2]), ("b.arrows", vec![3])] {
            let schema = Arc::new(arrow_schema::Schema::new(vec![arrow_schema::Field::new(
                "id",
                arrow_schema::DataType::Int64,
                false,
            )]));
            let batch = RecordBatch::try_new(schema, vec![Arc::new(Int64Array::from(values))])
                .expect("an IPC fixture");
            let mut leaf = yggdryl::local::LocalPath::new(lake.join(name)).expect("a lazy leaf");
            let options = leaf.record_options().expect("IPC options");
            leaf.overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .expect("a published IPC leaf");
        }
        std::fs::write(lake.join("notes.txt"), b"not a table row").unwrap();
        lake
    }

    #[test]
    fn an_ipc_wrapper_over_a_folder_answers_for_the_leaves_beneath_it() {
        use yggdryl::IOMedia as _;

        let path = root("ipc-wrapper");
        let lake = ipc_lake(&path);

        // Each leaf is its own stream: read as one run of bytes, the first
        // stream's end would end the table at two rows.
        let folder = yggdryl::local::LocalFolder::new(&lake).unwrap();
        let ipc = yggdryl::ipc::Ipc::new(yggdryl::local::LocalFolder::new(&lake).unwrap());
        assert_eq!(ipc.row_size().unwrap(), 3);
        assert_eq!(ipc.column_size().unwrap(), 1);
        let options = ipc.record_options().unwrap();
        assert_eq!(
            ipc.read_arrow_field(&options).unwrap(),
            folder.read_arrow_field(&options).unwrap()
        );
        assert_eq!(ipc.row_size().unwrap(), folder.row_size().unwrap());

        let _ = std::fs::remove_dir_all(&path);
    }

    /// One Arrow batch of `columns` Int64 columns and `rows` rows.
    fn int64_batch(columns: usize, rows: usize) -> arrow_array::RecordBatch {
        use std::sync::Arc;

        let fields: Vec<_> = (0..columns)
            .map(|column| {
                arrow_schema::Field::new(format!("c{column}"), arrow_schema::DataType::Int64, false)
            })
            .collect();
        let values: Vec<arrow_array::ArrayRef> = (0..columns)
            .map(|_| {
                Arc::new(arrow_array::Int64Array::from_iter_values(0..rows as i64))
                    as arrow_array::ArrayRef
            })
            .collect();
        arrow_array::RecordBatch::try_new(Arc::new(arrow_schema::Schema::new(fields)), values)
            .expect("an Int64 fixture")
    }

    #[test]
    fn an_opened_ipc_session_over_a_folder_never_caches_its_width() {
        use yggdryl::IOMedia as _;

        let path = root("ipc-session");
        let lake = path.join("lake");
        std::fs::create_dir_all(&lake).unwrap();
        let mut ipc = yggdryl::ipc::Ipc::new(yggdryl::local::LocalFolder::new(&lake).unwrap());
        ipc.open().unwrap();
        assert_eq!(
            ipc.column_size().unwrap(),
            0,
            "an empty folder has no columns"
        );

        // A leaf written beneath the folder since is what its width is now.
        let batch = int64_batch(2, 3);
        let mut leaf = yggdryl::local::LocalPath::new(lake.join("a.arrows")).unwrap();
        let options = leaf.record_options().unwrap();
        leaf.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
        assert_eq!(ipc.column_size().unwrap(), 2);
        assert_eq!(ipc.row_size().unwrap(), 3);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn an_opened_avro_session_over_a_folder_answers_from_its_leaves_after_a_write() {
        use yggdryl::IOMedia as _;

        let path = root("avro-session");
        let lake = path.join("lake");
        std::fs::create_dir_all(&lake).unwrap();
        let mut avro = yggdryl::avro::Avro::new(yggdryl::local::LocalFolder::new(&lake).unwrap());
        avro.open().unwrap();

        // The write lands a leaf beneath the folder; the session it refreshes
        // holds no leaf's dimensions, so the leaves still answer.
        let batch = int64_batch(2, 3);
        let options = avro.record_options().unwrap();
        avro.overwrite_arrow_reader(
            yggdryl::arrow::batch_reader(batch.schema(), [batch]),
            &options,
        )
        .unwrap();
        assert_eq!(avro.row_size().unwrap(), 3);
        assert_eq!(avro.column_size().unwrap(), 2);
        assert_eq!(avro.read_arrow_field(&options).unwrap().field_len(), 2);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[cfg(feature = "parquet")]
    #[test]
    fn an_opened_parquet_session_over_a_folder_answers_from_its_leaves_after_a_write() {
        use yggdryl::IOMedia as _;

        let path = root("parquet-session");
        let lake = path.join("lake");
        std::fs::create_dir_all(&lake).unwrap();
        let mut parquet =
            yggdryl::parquet::Parquet::new(yggdryl::local::LocalFolder::new(&lake).unwrap());
        parquet.open().unwrap();

        let batch = int64_batch(2, 3);
        let options = parquet.record_options().unwrap();
        parquet
            .overwrite_arrow_reader(
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &options,
            )
            .unwrap();
        assert_eq!(parquet.row_size().unwrap(), 3);
        assert_eq!(parquet.column_size().unwrap(), 2);
        assert_eq!(parquet.read_arrow_field(&options).unwrap().field_len(), 2);

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_csv_wrapper_over_a_folder_answers_for_the_leaves_beneath_it() {
        use yggdryl::IOMedia as _;
        use yggdryl::csv::Csv;

        let path = root("csv-wrapper");
        let lake = path.join("lake");
        std::fs::create_dir_all(&lake).unwrap();
        std::fs::write(lake.join("a.csv"), b"id,symbol\n1,AAPL\n2,MSFT\n").unwrap();
        std::fs::write(lake.join("b.csv"), b"id,symbol\n3,IBM\n").unwrap();
        std::fs::write(lake.join("notes.txt"), b"not a table row").unwrap();

        // Each leaf is its own document, header and all: read as one run of
        // bytes, the second header would be a fourth record and would make
        // `id` text.
        let folder = yggdryl::local::LocalFolder::new(&lake).unwrap();
        let csv = Csv::new(yggdryl::local::LocalFolder::new(&lake).unwrap());
        assert_eq!(csv.row_size().unwrap(), 3);
        assert_eq!(csv.column_size().unwrap(), 2);
        let options = csv.record_options().unwrap();
        let field = csv.read_arrow_field(&options).unwrap();
        assert_eq!(field, folder.read_arrow_field(&options).unwrap());
        assert_eq!(
            field.get_field_by_path("id").map(|id| id.dtype().clone()),
            Some(DataType::Int64),
            "{field}"
        );
        assert_eq!(csv.row_size().unwrap(), folder.row_size().unwrap());

        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn an_undeclared_read_of_a_folder_reads_the_table_beneath_it() {
        use yggdryl::IOMedia as _;

        let path = root("read-folder");
        let lake = ipc_lake(&path);

        // The folder declares a directory; what it holds is found beneath it.
        let folder = yggdryl::local::LocalFolder::new(&lake).unwrap();
        let rows: usize = yggdryl::StreamChunkedSerie::from_serie(folder.read_serie(None).unwrap())
            .expect("native record stream")
            .into_chunks()
            .map(|serie| serie.unwrap().len())
            .sum();
        assert_eq!(rows, 3);

        let _ = std::fs::remove_dir_all(&path);
    }
}

mod serie_verbs {
    //! `write_serie`, `overwrite_serie`, `append_serie` and `merge_serie`
    //! over a [`Serie`]: every shape rows are held in reaches the one
    //! publication path, and the refusals land before the destination is
    //! touched.

    use super::handle;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{
        ChunkedSerie, DataType, Field, IOBase, IOMedia, IOMode, Scalar, Serie, StreamChunkedSerie,
        StructType,
    };

    fn trade_root() -> Field {
        StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Int64.required_field("size"),
        ])
        .map(DataType::from)
        .expect("a record datatype")
        .required_field("trade")
    }

    fn trade(id: i64, size: i64) -> Scalar {
        Scalar::from_sequence([Scalar::from(id), Scalar::from(size)])
    }

    fn trades(ids: impl IntoIterator<Item = i64>) -> Serie {
        Serie::from_scalars(trade_root(), ids.into_iter().map(|id| trade(id, id * 10)))
            .expect("the rows land")
    }

    /// Every row `handle` holds, in order.
    fn stored(handle: &impl IOMedia) -> Vec<Scalar> {
        yggdryl::StreamChunkedSerie::from_serie(handle.read_serie(None).expect("the rows read"))
            .expect("native record stream")
            .into_chunks()
            .collect::<Result<Vec<Serie>, _>>()
            .expect("every batch lands")
            .iter()
            .flat_map(|column| column.rows().into_owned())
            .collect()
    }

    /// The same rows in the three shapes: held, cut into a chunk per row,
    /// streamed.
    fn sources(rows: impl IntoIterator<Item = i64> + Clone) -> [Serie; 3] {
        let held = trades(rows.clone());
        let chunks: Vec<Serie> = (0..held.len())
            .map(|row| held.slice(row, 1).expect("one row"))
            .collect();
        [
            held,
            Serie::from(
                ChunkedSerie::from_series(Some(&trade_root()), chunks, Default::default())
                    .expect("a chunk per row"),
            ),
            Serie::from(StreamChunkedSerie::from_serie(trades(rows)).expect("a stream")),
        ]
    }

    /// The record encodings this build writes a leaf in.
    fn encodings() -> Vec<&'static str> {
        let mut names = vec!["trades.arrows", "trades.avro"];
        if cfg!(feature = "parquet") {
            names.push("trades.parquet");
        }
        names
    }

    #[test]
    fn every_shape_overwrites_appends_and_merges_through_the_one_path() {
        for name in encodings() {
            for (shape, source) in sources([1, 2]).into_iter().enumerate() {
                let mut target = handle(name);
                target
                    .overwrite_serie(source, None)
                    .unwrap_or_else(|error| panic!("{name} shape {shape} overwrites: {error}"));
                assert_eq!(
                    stored(&target),
                    [trade(1, 10), trade(2, 20)],
                    "{name} shape {shape}"
                );

                let [_, _, appended] = sources([3, 4]);
                target
                    .append_serie(appended, None)
                    .unwrap_or_else(|error| panic!("{name} shape {shape} appends: {error}"));
                assert_eq!(
                    stored(&target),
                    [trade(1, 10), trade(2, 20), trade(3, 30), trade(4, 40)],
                    "{name} shape {shape}"
                );

                // A merge updates the matched row and appends the miss.
                let mut options =
                    RecordOptions::for_media_type(target.media_type()).expect("an encoding");
                options.set_merge_by("id".parse().expect("a selector"));
                let incoming = Serie::from_scalars(trade_root(), [trade(2, 99), trade(5, 50)])
                    .expect("the rows");
                target
                    .merge_serie(incoming, Some(&options))
                    .unwrap_or_else(|error| panic!("{name} shape {shape} merges: {error}"));
                let rows = stored(&target);
                assert_eq!(rows.len(), 5, "{name} shape {shape}: {rows:?}");
                assert!(
                    rows.contains(&trade(2, 99)) && !rows.contains(&trade(2, 20)),
                    "{name}: {rows:?}"
                );
                assert!(rows.contains(&trade(5, 50)), "{name}: {rows:?}");

                // The generic verb under an explicit mode is the same path.
                let [held, _, _] = sources([7]);
                target
                    .write_serie(held, IOMode::Overwrite, None)
                    .expect("the generic write overwrites");
                assert_eq!(stored(&target), [trade(7, 70)]);
            }
        }
    }

    #[test]
    fn a_plain_column_is_written_as_the_one_child_of_a_row_record() {
        let mut target = handle("sizes.arrows");
        let sizes = Serie::from_scalars(
            DataType::Int64.required_field("size"),
            [Scalar::from(1_i64), Scalar::from(2_i64)],
        )
        .expect("a column");
        target
            .overwrite_serie(sizes, None)
            .expect("the column writes");
        let read = yggdryl::StreamChunkedSerie::from_serie(
            target.read_serie(None).expect("the rows read"),
        )
        .expect("native record stream");
        assert_eq!(read.field().name(), "row");
        assert_eq!(
            read.field()
                .fields()
                .iter()
                .map(Field::name)
                .collect::<Vec<_>>(),
            ["size"]
        );
        assert_eq!(
            stored(&target),
            [
                Scalar::from_sequence([Scalar::from(1_i64)]),
                Scalar::from_sequence([Scalar::from(2_i64)])
            ]
        );
    }

    #[test]
    fn a_run_and_an_absent_row_are_refused_before_the_destination_is_touched() {
        let mut target = handle("trades.arrows");
        target
            .overwrite_serie(trades([1]), None)
            .expect("the rows write");

        let run = Serie::new(vec![Scalar::from(1_i64)]);
        let error = target
            .append_serie(run, None)
            .expect_err("a run names no layout");
        assert!(
            error.to_string().contains("run") || error.to_string().contains("field"),
            "{error}"
        );
        assert_eq!(stored(&target), [trade(1, 10)], "the refusal wrote nothing");

        let absent = Serie::from_scalars(
            trade_root().with_nullable(true),
            [trade(2, 20), Scalar::Null],
        )
        .expect("a nullable record holds an absent row");
        let error = target
            .append_serie(absent, None)
            .expect_err("a table states no absent row");
        assert!(
            error.to_string().contains("null") || error.to_string().contains("absent"),
            "{error}"
        );
        assert_eq!(stored(&target), [trade(1, 10)], "the refusal wrote nothing");
    }

    #[test]
    fn a_structured_document_takes_an_overwrite_alone() {
        let mut target = handle("trades.jsonl");
        target
            .overwrite_serie(trades([1, 2]), None)
            .expect("a document is replaced whole");
        assert_eq!(stored(&target), [trade(1, 10), trade(2, 20)]);
        for (verb, mode) in [("append", IOMode::Append), ("merge", IOMode::Merge)] {
            let error = target
                .write_serie(trades([3]), mode, None)
                .expect_err("a document is written whole");
            assert!(
                error.to_string().contains("expected overwrite"),
                "{verb}: {error}"
            );
            assert_eq!(
                stored(&target),
                [trade(1, 10), trade(2, 20)],
                "{verb} wrote nothing"
            );
        }
    }

    #[test]
    fn absent_options_are_the_handle_s_own_so_a_declared_field_shapes_nothing_unasked() {
        // Under the handle's own options, the stored schema is the rows' own.
        let mut target = handle("trades.arrows");
        target
            .append_serie(trades([1]), None)
            .expect("an append creates the leaf");
        target
            .append_serie(trades([2]), None)
            .expect("a second append keeps the first");
        assert_eq!(stored(&target), [trade(1, 10), trade(2, 20)]);
        // An encoding stores the columns, so the stored root carries the
        // options' name and the rows' own children.
        let stored_field = target
            .read_arrow_field(&target.record_options().expect("the stored encoding"))
            .expect("the stored field");
        assert_eq!(
            stored_field
                .fields()
                .iter()
                .map(Field::name)
                .collect::<Vec<_>>(),
            ["id", "size"]
        );

        // A declared field on the options casts the rows once, onto it.
        let wide = StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("size"),
        ])
        .map(DataType::from)
        .expect("a record datatype")
        .required_field("trade");
        let mut options = RecordOptions::for_media_type(target.media_type()).expect("an encoding");
        options.set_field(wide.clone());
        let mut declared = handle("wide.arrows");
        declared
            .overwrite_serie(trades([3]), Some(&options))
            .expect("the rows cast onto the declared field");
        assert_eq!(
            stored(&declared),
            [Scalar::from_sequence([
                Scalar::from(3_i64),
                Scalar::from(30.0_f64)
            ])]
        );
    }
}

mod own_key {
    //! A merge whose options name no key takes the destination's own
    //! ([`IOMedia::merge_by`]), resolved once by [`IOMedia::write_options`]
    //! before a source is pulled; a leaf states none, so it is refused there.

    use std::borrow::Cow;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use yggdryl::media::IORecordOptions;
    use yggdryl::{ArrowCastOptions, IOBase, IOMedia, IOMode, Serie, StreamChunkedSerie};

    use super::{counted_source, handle, rows_batch};

    #[test]
    fn a_leaf_states_no_key_so_a_keyless_merge_is_refused_before_its_source_is_pulled() {
        let mut target = handle("trades.arrows");
        assert!(IOMedia::merge_by(&target).unwrap().is_empty());
        let options = target.record_options().unwrap();
        let error = target
            .write_options(IOMode::Merge, &options)
            .expect_err("a leaf states no key of its own");
        assert!(error.to_string().contains("$.merge_by"), "{error}");

        let pulls = Arc::new(AtomicUsize::new(0));
        let error = target
            .write_arrow_reader(
                counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[1, 2]))]),
                IOMode::Merge,
                &options,
            )
            .expect_err("the generic door refuses the empty key");
        assert!(
            error
                .to_string()
                .contains("requires at least one merge_by column"),
            "{error}"
        );
        assert_eq!(
            pulls.load(Ordering::SeqCst),
            0,
            "the source is never pulled"
        );

        let stream = StreamChunkedSerie::from_arrow_reader(
            None,
            counted_source(Arc::clone(&pulls), [Ok(rows_batch(&[3]))]),
            ArrowCastOptions::default(),
        )
        .unwrap();
        let error = target
            .merge_serie(Serie::from(stream), None)
            .expect_err("the serie door refuses the empty key");
        assert!(error.to_string().contains("$.merge_by"), "{error}");
        assert_eq!(
            pulls.load(Ordering::SeqCst),
            0,
            "the stream is never pulled"
        );
        assert_eq!(IOBase::size(&target), 0, "nothing was written");
    }

    #[test]
    fn write_options_borrows_the_options_it_does_not_rekey() {
        let target = handle("trades.arrows");
        let options = target.record_options().unwrap();
        for mode in [IOMode::Overwrite, IOMode::Append] {
            assert!(
                matches!(target.write_options(mode, &options), Ok(Cow::Borrowed(_))),
                "{mode}"
            );
        }
        let keyed = options.clone().with_merge_by(["id"]).unwrap();
        let resolved = target.write_options(IOMode::Merge, &keyed).unwrap();
        assert!(matches!(resolved, Cow::Borrowed(_)));
        assert_eq!(resolved.merge_by().to_string(), "id");
        let error = target
            .write_options(IOMode::Overwrite, &keyed)
            .expect_err("an overwrite refuses a key");
        assert!(
            error.to_string().contains("does not accept merge_by"),
            "{error}"
        );
    }
}
