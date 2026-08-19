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

/// A read of one file under one set of options, counting the rows it decoded.
fn decoded(media: &Parquet<crate::io::Buffer>, filter: &str) -> usize {
    let options = media.options().clone().with_filter(filter).unwrap();
    crate::parquet::read_batch_reader(media, None, &options)
        .unwrap()
        .map(|batch| batch.unwrap().num_rows())
        .sum()
}

#[test]
fn a_filter_skips_the_row_groups_its_statistics_refute() {
    let field = root();
    let ids: Vec<i64> = (0..2_048).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let mut media = Parquet::new(handle("prune.parquet"))
        .with_options(ParquetOptions::new().with_max_row_group_size(512));
    media
        .write_batch_reader(reader(&field, [batch(&field, ids, symbols)]))
        .unwrap();

    // Four groups of 512 ids, so `id >= 1600` can only be satisfied by the
    // last one: the other three are never located, decompressed, or decoded.
    // A row group is the unit of *skipping*, not of selection, so the rows the
    // surviving group holds below the bound are still decoded here - the row
    // filter above this level is what drops them, and the next assertion is
    // that the two together are exactly right.
    assert_eq!(decoded(&media, "id >= 1600"), 512);

    // A predicate no group can satisfy decodes nothing at all.
    assert_eq!(decoded(&media, "id > 10000"), 0);

    // And a predicate every group can satisfy skips nothing, because `Maybe`
    // is what an unsettled group answers.
    assert_eq!(decoded(&media, "id >= 0"), 2_048);
}

#[test]
fn the_pruned_read_returns_exactly_what_an_unpruned_one_would() {
    use crate::io::IOBase;

    let field = root();
    let ids: Vec<i64> = (0..2_048).collect();
    let symbols: Vec<Option<&str>> = ids
        .iter()
        .map(|index| (index % 3 == 0).then_some("AAPL"))
        .collect();
    let mut media = Parquet::new(handle("prune-equivalence.parquet"))
        .with_options(ParquetOptions::new().with_max_row_group_size(512));
    media
        .write_batch_reader(reader(&field, [batch(&field, ids.clone(), symbols)]))
        .unwrap();

    // The whole ladder: groups skipped on their footer statistics, then one
    // mask per surviving batch. What comes out has to equal what a read with
    // no pushdown at all returns and then filters, for every predicate.
    for (text, expected) in [
        ("id >= 1600", (1_600..2_048).collect::<Vec<i64>>()),
        ("id < 3", vec![0, 1, 2]),
        ("id BETWEEN 500 AND 503", vec![500, 501, 502, 503]),
        ("id IN (1, 700, 2000)", vec![1, 700, 2_000]),
        ("id > 10000", Vec::new()),
        (
            "symbol IS NULL",
            ids.iter().copied().filter(|id| id % 3 != 0).collect(),
        ),
    ] {
        let options = crate::generic::RecordOptions::Parquet(
            media.options().clone().with_filter(text).unwrap(),
        );
        let read: Vec<i64> = media
            .read_arrow_batch_reader(&options)
            .unwrap()
            .flat_map(|batch| {
                let batch = batch.unwrap();
                let column = batch
                    .column_by_name("id")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .clone();
                (0..batch.num_rows())
                    .map(move |row| column.value(row))
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(read, expected, "pushdown changed the answer for {text:?}");
    }
}

#[test]
fn a_filter_on_a_column_the_file_does_not_carry_leaves_every_group() {
    let field = root();
    let mut media = Parquet::new(handle("absent-filter.parquet"))
        .with_options(ParquetOptions::new().with_max_row_group_size(512));
    media
        .write_batch_reader(reader(
            &field,
            [batch(
                &field,
                (0..1_024).collect(),
                vec![Some("AAPL"); 1_024],
            )],
        ))
        .unwrap();

    // A partition column lives in the path rather than the file, so a filter
    // naming one must not prune the file away - whatever restores it answers.
    assert_eq!(decoded(&media, "venue = 'XNAS'"), 1_024);
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
