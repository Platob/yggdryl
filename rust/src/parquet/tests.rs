//! Parquet round trips, field-id preservation, and footer statistics.

use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use parquet::basic::Compression;

use super::{Parquet, ParquetOptions};
use crate::arrow::schema_from_field;
use crate::generic::IORecordOptions;
use crate::io::{Buffer, IOBase};
use crate::{DataType, Field, Url};

/// A root carrying explicit Iceberg-style field identifiers.
fn root() -> Field {
    DataType::from_fields([
        DataType::Int64
            .required_field("id")
            .with_parquet_field_id(1),
        DataType::Utf8
            .nullable_field("symbol")
            .with_parquet_field_id(2),
    ])
    .unwrap()
    .required_field("row")
}

fn batch(field: &Field, ids: Vec<i64>, symbols: Vec<Option<&str>>) -> RecordBatch {
    let schema = schema_from_field(field).unwrap();
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
fn reader<I>(field: &Field, batches: I) -> crate::arrow::BatchReader
where
    I: IntoIterator<Item = RecordBatch>,
    I::IntoIter: Send + 'static,
{
    crate::arrow::batch_reader(schema_from_field(field).unwrap(), batches)
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

    media
        .write_batch_reader(reader(&field, [expected.clone()]))
        .unwrap();

    let actual = media
        .read_batch_reader(None)
        .unwrap()
        .map(std::result::Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), 1);
    assert_eq!(actual[0], expected);
    assert_eq!(actual[0].num_rows(), 3);
}

#[test]
fn field_identifiers_survive_the_round_trip() {
    let field = root();
    let mut media = Parquet::new(handle("ids.parquet"));
    media
        .write_batch_reader(reader(&field, [batch(&field, vec![1], vec![Some("AAPL")])]))
        .unwrap();

    // Ids are what an Iceberg reader resolves columns by, so they must not be
    // positional after a round trip.
    let schema = media.read_schema().unwrap();
    assert_eq!(
        schema.field(0).metadata().get("PARQUET:field_id"),
        Some(&"1".to_owned())
    );
    assert_eq!(
        schema.field(1).metadata().get("PARQUET:field_id"),
        Some(&"2".to_owned())
    );

    let recovered = media.read_field().unwrap();
    let fields = recovered.data_type().as_fields().unwrap();
    assert_eq!(fields[0].parquet_field_id().unwrap(), Some(1));
    assert_eq!(fields[1].parquet_field_id().unwrap(), Some(2));
}

#[test]
fn an_empty_write_still_publishes_a_readable_file() {
    let field = root();
    let mut media = Parquet::new(handle("empty.parquet"));

    media.write_batch_reader(reader(&field, [])).unwrap();

    assert!(!media.handle().is_empty());
    assert!(
        media
            .read_batch_reader(None)
            .unwrap()
            .map(std::result::Result::unwrap)
            .collect::<Vec<_>>()
            .is_empty()
    );
    assert_eq!(media.read_schema().unwrap().fields().len(), 2);
    assert_eq!(media.read_statistics().unwrap().num_rows, 0);
}

#[test]
fn a_coded_location_is_rejected_with_the_reason() {
    let field = root();

    for name in ["trades.parquet.gz", "trades.parquet.zst"] {
        let mut media = Parquet::new(handle(name));
        let message = media
            .write_batch_reader(reader(&field, []))
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
    let other = DataType::from_fields([DataType::Utf8.required_field("unrelated")])
        .unwrap()
        .required_field("row");
    let mut media = Parquet::new(handle("mismatch.parquet"));

    let good = batch(&field, vec![1], vec![Some("AAPL")]);
    let bad = RecordBatch::try_new(
        other.to_arrow_schema().unwrap(),
        vec![Arc::new(StringArray::from(vec!["x"]))],
    )
    .unwrap();

    let message = media
        .write_batch_reader(reader(&field, [good, bad]))
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
                .with_batch_size(source.num_rows()),
        );
        media
            .write_batch_reader(reader(&field, [source.clone()]))
            .unwrap_or_else(|error| panic!("{name}: {error}"));

        let actual = media
            .read_batch_reader(None)
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
    media
        .write_batch_reader(reader(&field, [batch(&field, ids, symbols)]))
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
fn a_bounded_batch_size_splits_the_read() {
    let field = root();
    let mut media = Parquet::new(handle("batched.parquet"))
        .with_options(ParquetOptions::new().with_batch_size(256));
    let ids: Vec<i64> = (0..1_000).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    media
        .write_batch_reader(reader(&field, [batch(&field, ids, symbols)]))
        .unwrap();

    let batches = media
        .read_batch_reader(None)
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

    use arrow_array::{
        Array, Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray,
    };

    use super::{Parquet, handle};
    use crate::arrow::schema_from_field;
    use crate::{DataType, Field};

    /// Four columns, so a two-column read is a genuine subset.
    fn wide() -> Field {
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
            DataType::Float64.required_field("price"),
            DataType::Utf8.nullable_field("venue"),
        ])
        .unwrap()
        .required_field("row")
    }

    /// The two columns a caller actually wants.
    fn narrow() -> Field {
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Float64.required_field("price"),
        ])
        .unwrap()
        .required_field("row")
    }

    /// A file wide enough that skipping two columns is measurable.
    fn stored() -> Parquet<crate::io::Buffer> {
        let rows = 4_096;
        let ids: Vec<i64> = (0..rows).collect();
        let batch = RecordBatch::try_new(
            schema_from_field(&wide()).unwrap(),
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
        media
            .write_batch_reader(crate::arrow::batch_reader(batch.schema(), [batch]))
            .unwrap();
        media
    }

    /// Total bytes every array in a read occupies, which is the data the read
    /// actually moved into Arrow memory.
    fn materialized(reader: crate::arrow::BatchReader) -> usize {
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

        // The file stores four columns; nothing about it changed.
        assert_eq!(media.read_schema().unwrap().fields().len(), 4);
        assert_eq!(media.read_field().unwrap().field_len(), 4);

        let reader = media.read_batch_reader(Some(&narrow())).unwrap();
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

        let whole = materialized(media.read_batch_reader(None).unwrap());
        let subset = materialized(media.read_batch_reader(Some(&narrow())).unwrap());

        // The two string columns are the bulk of this file, and a pushed-down
        // read never builds them.
        assert!(subset * 2 < whole, "subset {subset} bytes, whole {whole}");
    }

    #[test]
    fn a_column_the_file_does_not_store_leaves_the_read_whole() {
        let media = stored();

        // A mask can only drop columns, so a schema asking for one that is not
        // there reads everything and leaves the gap to a later cast.
        let invented = DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.required_field("nowhere"),
        ])
        .unwrap()
        .required_field("row");

        let reader = media.read_batch_reader(Some(&invented)).unwrap();
        assert_eq!(reader.schema().fields().len(), 4);
    }
}

mod limits {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use arrow_array::RecordBatchReader;
    use parquet::basic::Compression;

    use super::{Parquet, ParquetOptions, batch, handle, reader, root};
    use crate::generic::{IORecordOptions, RecordOptions};
    use crate::io::{Buffer, IOBase};

    /// The total rows a handle yields under `options`.
    fn rows<H: IOBase + ?Sized>(handle: &H, options: &RecordOptions) -> usize {
        handle
            .read_arrow_batch_reader(options)
            .unwrap()
            .map(|batch| batch.unwrap().num_rows())
            .sum()
    }

    #[test]
    fn a_zero_limit_reads_the_declared_schema_and_no_batches() {
        let field = root();
        let mut handle = handle("limited.parquet");
        let options = handle.record_options().unwrap().with_schema(field.clone());
        handle
            .write_arrow_batch_reader(
                reader(
                    &field,
                    [batch(&field, vec![1, 2], vec![Some("AAPL"), None])],
                ),
                &options,
            )
            .unwrap();

        let mut limited = handle
            .read_arrow_batch_reader(&options.with_max_row_size(0))
            .unwrap();
        // The schema is asserted, not only the emptiness: `Some(0)` is a
        // valid ask that still says what the rows would have been.
        assert_eq!(
            limited.schema(),
            crate::arrow::schema_from_field(&field).unwrap()
        );
        assert!(limited.next().is_none());
    }

    #[test]
    fn a_limited_write_truncates_what_the_caller_offered() {
        let field = root();
        let mut handle = handle("truncated.parquet");
        let options = handle.record_options().unwrap().with_schema(field.clone());

        handle
            .write_arrow_batch_reader(
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
            .append_arrow_batch_reader(
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

    impl IOBase for Counting {
        crate::delegate_iobase!(handle: pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, parent, child_by_path,
            ls, kind, clear, remove, is_atomic, is_tabular);

        fn pread(&self, offset: u64, buffer: &mut [u8]) -> crate::Result<usize> {
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
        media
            .write_batch_reader(reader(
                &field,
                [batch(
                    &field,
                    (0..total as i64).collect(),
                    vec![None; total],
                )],
            ))
            .unwrap();
        assert_eq!(media.read_statistics().unwrap().row_groups.len(), 32);

        let counting = Counting::new(media.into_handle());
        let options = counting.record_options().unwrap();

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

/// The geospatial pair and the variant storage struct: logical types in the
/// footer, refused value bounds, and the WKB statistics written instead.
mod geospatial {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_array::{ArrayRef, BinaryArray, Int64Array, RecordBatch};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use parquet::basic::{EdgeInterpolationAlgorithm, LogicalType};

    use super::{Parquet, handle};
    use crate::io::{Buffer, IOBase};

    /// One little-endian ISO WKB point.
    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    }

    /// A nullable field carrying one Arrow extension declaration.
    ///
    /// The metadata is spelled by hand so foreign and malformed documents can
    /// be written too; the well-formed spelling is proven equal to the
    /// `Field` projection's own output by the end-to-end test below.
    fn extension_field(
        name: &str,
        storage: ArrowDataType,
        extension: &str,
        document: Option<&str>,
    ) -> ArrowField {
        let mut metadata =
            HashMap::from([("ARROW:extension:name".to_owned(), extension.to_owned())]);
        if let Some(document) = document {
            metadata.insert("ARROW:extension:metadata".to_owned(), document.to_owned());
        }
        ArrowField::new(name, storage, true).with_metadata(metadata)
    }

    /// The canonical variant storage: a struct of two required binaries.
    fn variant_storage() -> ArrowDataType {
        ArrowDataType::Struct(arrow_schema::Fields::from(vec![
            ArrowField::new("metadata", ArrowDataType::Binary, false),
            ArrowField::new("value", ArrowDataType::Binary, false),
        ]))
    }

    /// Write one file holding the given fields and columns.
    fn written(name: &str, fields: Vec<ArrowField>, columns: Vec<ArrayRef>) -> Parquet<Buffer> {
        let schema = Arc::new(Schema::new(fields));
        let batches = if columns.is_empty() {
            Vec::new()
        } else {
            vec![RecordBatch::try_new(Arc::clone(&schema), columns).unwrap()]
        };
        let mut media = Parquet::new(handle(name));
        media
            .write_batch_reader(crate::arrow::batch_reader(schema, batches))
            .unwrap();
        media
    }

    /// The logical type the footer stores for one leaf column path.
    fn leaf_logical(media: &Parquet<Buffer>, path: &str) -> Option<LogicalType> {
        let builder = crate::parquet::open_builder(media.handle()).unwrap();
        builder
            .parquet_schema()
            .columns()
            .iter()
            .find(|column| column.path().string() == path)
            .and_then(|column| column.logical_type_ref().cloned())
    }

    #[test]
    fn a_geometry_column_writes_the_logical_type_and_wkb_statistics() {
        let media = written(
            "geometry.parquet",
            vec![
                ArrowField::new("id", ArrowDataType::Int64, false),
                extension_field(
                    "shape",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "OGC:CRS84"}"#),
                ),
            ],
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(BinaryArray::from_opt_vec(vec![
                    Some(&wkb_point(1.0, 2.0)),
                    None,
                    Some(&wkb_point(-3.0, 7.0)),
                ])),
            ],
        );

        // The default CRS folds to Parquet's absent spelling.
        assert_eq!(
            leaf_logical(&media, "shape"),
            Some(LogicalType::geometry(None))
        );
        assert_eq!(leaf_logical(&media, "id"), None);

        let statistics = media.read_statistics().unwrap();
        let columns = &statistics.row_groups[0].columns;
        let id = columns.iter().find(|column| column.path == "id").unwrap();
        let shape = columns
            .iter()
            .find(|column| column.path == "shape")
            .unwrap();

        // The sibling still records value bounds; the geometry never does.
        assert!(id.min_bytes.is_some() && id.max_bytes.is_some());
        assert!(shape.min_bytes.is_none() && shape.max_bytes.is_none());
        assert_eq!(shape.null_count, Some(1));

        // What a geometry records instead: the WKB bounds and type codes.
        let geospatial = shape.geospatial.as_ref().unwrap();
        let bounds = geospatial.bounding_box.unwrap();
        assert_eq!(
            (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
            (-3.0, 1.0, 2.0, 7.0)
        );
        assert_eq!(geospatial.geometry_types, vec![1]);
        assert!(id.geospatial.is_none());
    }

    #[test]
    fn a_custom_crs_and_a_bare_declaration_both_survive() {
        let media = written(
            "crs.parquet",
            vec![
                extension_field(
                    "mercator",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "EPSG:3857"}"#),
                ),
                // No metadata document at all: a geometry in the default CRS.
                extension_field("bare", ArrowDataType::Binary, "geoarrow.wkb", None),
            ],
            vec![
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(0.0, 0.0)[..],
                )])),
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(0.0, 0.0)[..],
                )])),
            ],
        );

        assert_eq!(
            leaf_logical(&media, "mercator"),
            Some(LogicalType::geometry(Some("EPSG:3857".to_owned())))
        );
        assert_eq!(
            leaf_logical(&media, "bare"),
            Some(LogicalType::geometry(None))
        );
    }

    #[test]
    fn a_geography_column_carries_its_algorithm_and_writes_no_bounds() {
        let media = written(
            "geography.parquet",
            vec![
                extension_field(
                    "route",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "EPSG:4326", "edges": "vincenty"}"#),
                ),
                // The spherical default folds to Parquet's absent spelling.
                extension_field(
                    "region",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "OGC:CRS84", "edges": "spherical"}"#),
                ),
            ],
            vec![
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(4.0, 5.0)[..],
                )])),
                Arc::new(BinaryArray::from_opt_vec(vec![Some(
                    &wkb_point(6.0, 7.0)[..],
                )])),
            ],
        );

        assert_eq!(
            leaf_logical(&media, "route"),
            Some(LogicalType::geography(
                Some("EPSG:4326".to_owned()),
                Some(EdgeInterpolationAlgorithm::VINCENTY),
            ))
        );
        assert_eq!(
            leaf_logical(&media, "region"),
            Some(LogicalType::geography(None, None))
        );

        // A geography's bounds are edge-algorithm-aware, so a planar fold of
        // the vertices would under-cover them: no value bounds, and no box.
        let statistics = media.read_statistics().unwrap();
        for column in &statistics.row_groups[0].columns {
            assert!(column.min_bytes.is_none() && column.max_bytes.is_none());
            assert!(column.geospatial.is_none(), "{}", column.path);
        }
    }

    #[test]
    fn a_geometry_nested_in_a_struct_still_gets_the_logical_type() {
        let media = written(
            "nested.parquet",
            vec![ArrowField::new(
                "place",
                ArrowDataType::Struct(arrow_schema::Fields::from(vec![
                    ArrowField::new("name", ArrowDataType::Utf8, false),
                    extension_field(
                        "shape",
                        ArrowDataType::Binary,
                        "geoarrow.wkb",
                        Some(r#"{"crs": "OGC:CRS84"}"#),
                    ),
                ])),
                false,
            )],
            Vec::new(),
        );

        assert_eq!(
            leaf_logical(&media, "place.shape"),
            Some(LogicalType::geometry(None))
        );
        assert_eq!(
            leaf_logical(&media, "place.name"),
            Some(LogicalType::String)
        );
    }

    #[test]
    fn the_variant_storage_struct_publishes_the_variant_logical_type() {
        // Schema level only: a variant *value* cannot cross an Arrow array
        // boundary yet - the binary encoding lands with the Iceberg v3 layer.
        let media = written(
            "variant.parquet",
            vec![extension_field(
                "payload",
                variant_storage(),
                "arrow.parquet.variant",
                Some(""),
            )],
            Vec::new(),
        );

        let builder = crate::parquet::open_builder(media.handle()).unwrap();
        let root = builder.parquet_schema().root_schema_ptr();
        let payload = root
            .get_fields()
            .iter()
            .find(|field| field.name() == "payload")
            .unwrap();
        assert_eq!(
            payload.get_basic_info().logical_type_ref(),
            Some(&LogicalType::variant(None))
        );

        // The storage struct itself round-trips as a plain struct.
        let schema = media.read_schema().unwrap();
        let field = schema.field_with_name("payload").unwrap();
        assert_eq!(
            field.metadata().get("ARROW:extension:name"),
            Some(&"arrow.parquet.variant".to_owned())
        );
        let ArrowDataType::Struct(children) = field.data_type() else {
            panic!("expected struct storage, got {}", field.data_type());
        };
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].name(), "metadata");
        assert_eq!(children[1].name(), "value");
    }

    #[test]
    fn reading_back_our_own_file_surfaces_the_extension_metadata() {
        let media = written(
            "roundtrip.parquet",
            vec![extension_field(
                "shape",
                ArrowDataType::Binary,
                "geoarrow.wkb",
                Some(r#"{"crs": "OGC:CRS84"}"#),
            )],
            vec![Arc::new(BinaryArray::from_opt_vec(vec![Some(
                &wkb_point(1.0, 1.0)[..],
            )]))],
        );

        // Our writer embeds the Arrow schema, so the extension identity comes
        // back; a foreign file without that embedding surfaces plain Binary,
        // which is the named read-side limit in the module docs.
        let schema = media.read_schema().unwrap();
        let field = schema.field_with_name("shape").unwrap();
        assert_eq!(field.data_type(), &ArrowDataType::Binary);
        assert_eq!(
            field.metadata().get("ARROW:extension:name"),
            Some(&"geoarrow.wkb".to_owned())
        );
        assert_eq!(
            field.metadata().get("ARROW:extension:metadata"),
            Some(&r#"{"crs": "OGC:CRS84"}"#.to_owned())
        );
    }

    #[test]
    fn scanning_the_stored_wkb_recomputes_the_footer_statistics() {
        let media = written(
            "scan.parquet",
            vec![
                ArrowField::new("id", ArrowDataType::Int64, false),
                extension_field(
                    "shape",
                    ArrowDataType::Binary,
                    "geoarrow.wkb",
                    Some(r#"{"crs": "OGC:CRS84"}"#),
                ),
            ],
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(BinaryArray::from_opt_vec(vec![
                    Some(&wkb_point(10.0, -2.5)),
                    None,
                    Some(&wkb_point(-4.0, 8.0)),
                ])),
            ],
        );

        let scanned = media.read_geospatial_statistics("shape").unwrap();
        let bounds = scanned.bounding_box.unwrap();
        assert_eq!(
            (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
            (-4.0, 10.0, -2.5, 8.0)
        );
        assert_eq!(scanned.geometry_types, vec![1]);

        // The scan and the footer answer with the same statistics.
        let statistics = media.read_statistics().unwrap();
        let footer = statistics.row_groups[0]
            .columns
            .iter()
            .find(|column| column.path == "shape")
            .and_then(|column| column.geospatial.clone())
            .unwrap();
        assert_eq!(footer, scanned);
    }

    #[test]
    fn the_scan_refuses_a_column_that_is_not_wkb_by_name() {
        let media = written(
            "refuse.parquet",
            vec![ArrowField::new("id", ArrowDataType::Int64, false)],
            vec![Arc::new(Int64Array::from(vec![1]))],
        );

        let message = media
            .read_geospatial_statistics("id")
            .unwrap_err()
            .to_string();
        assert!(message.contains("expected WKB binary storage"), "{message}");
        assert!(message.contains("got Int64"), "{message}");
        assert!(message.contains("$.id"), "{message}");

        let message = media
            .read_geospatial_statistics("absent")
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("expected a stored geospatial column"),
            "{message}"
        );
        assert!(message.contains("absent"), "{message}");
    }

    #[test]
    fn a_malformed_geoarrow_document_is_refused_before_any_write() {
        let schema = Arc::new(Schema::new(vec![extension_field(
            "shape",
            ArrowDataType::Binary,
            "geoarrow.wkb",
            Some(r#"{"edges": "diagonal"}"#),
        )]));
        let mut media = Parquet::new(handle("bad-edges.parquet"));
        let message = media
            .write_batch_reader(crate::arrow::batch_reader(schema, []))
            .unwrap_err()
            .to_string();
        assert!(
            message.contains("expected a GeoArrow JSON metadata document"),
            "{message}"
        );
        // The shared parser names the vocabulary inside the refusal.
        assert!(message.contains("expected one of"), "{message}");
        assert!(message.contains("\"diagonal\""), "{message}");
        assert!(message.contains("$.shape"), "{message}");
        // Nothing was published.
        assert!(media.handle().is_empty());
    }

    #[test]
    fn a_field_declared_schema_drives_the_logical_types_end_to_end() {
        // The schema comes from the Field layer's own projection rather than
        // hand-spelled metadata, so the two layers are proven to agree; rows
        // stay out because a variant value cannot cross an Arrow array yet.
        let root = crate::DataType::from_fields([
            crate::DataType::Int64.required_field("id"),
            crate::DataType::geometry(Some("EPSG:3857"))
                .unwrap()
                .nullable_field("shape"),
            crate::DataType::geography(None, Some(crate::enums::EdgeAlgorithm::Vincenty))
                .unwrap()
                .nullable_field("route"),
            crate::DataType::variant().nullable_field("payload"),
        ])
        .unwrap()
        .required_field("row");
        let schema = root.to_arrow_schema().unwrap();

        let mut media = Parquet::new(handle("field-declared.parquet"));
        media
            .write_batch_reader(crate::arrow::batch_reader(schema, []))
            .unwrap();

        assert_eq!(
            leaf_logical(&media, "shape"),
            Some(LogicalType::geometry(Some("EPSG:3857".to_owned())))
        );
        // The default CRS folds to absence; a non-spherical algorithm rides.
        assert_eq!(
            leaf_logical(&media, "route"),
            Some(LogicalType::geography(
                None,
                Some(EdgeInterpolationAlgorithm::VINCENTY)
            ))
        );
        assert_eq!(leaf_logical(&media, "id"), None);
        let builder = crate::parquet::open_builder(media.handle()).unwrap();
        let payload = builder
            .parquet_schema()
            .root_schema_ptr()
            .get_fields()
            .iter()
            .find(|field| field.name() == "payload")
            .cloned()
            .unwrap();
        assert_eq!(
            payload.get_basic_info().logical_type_ref(),
            Some(&LogicalType::variant(None))
        );

        // And the identity survives the read: the reimported root speaks the
        // datatypes the declaration did, extension transport keys stripped.
        let read =
            crate::arrow::record_schema_from_arrow("row", media.read_schema().unwrap().as_ref())
                .unwrap();
        assert_eq!(read, root);
    }
}
