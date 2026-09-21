//! `rust/src/parquet/metadata.rs`: footer statistics as one generic value.
//!
//! Every door here is public: the statistics a footer carries are what an
//! Iceberg manifest writer records, so the whole file reaches the crate
//! through `yggdryl::`.

use yggdryl::Scalar;
use yggdryl::parquet::{
    ColumnStatistics, FileStatistics, GeospatialStatistics, RowGroupStatistics,
};
use yggdryl::wkb::BoundingBox;

#[test]
fn statistics_project_into_one_lossless_generic_value_shape() {
    let statistics = FileStatistics {
        num_rows: 2,
        created_by: Some("writer".to_owned()),
        // Repeated keys are legal and must stay repeated.
        key_value_metadata: vec![
            ("tag".to_owned(), "one".to_owned()),
            ("tag".to_owned(), "two".to_owned()),
        ],
        row_groups: vec![RowGroupStatistics {
            num_rows: 2,
            compressed_size: 12,
            file_offset: Some(4),
            columns: vec![ColumnStatistics {
                path: "shape".to_owned(),
                compressed_size: 12,
                uncompressed_size: 24,
                null_count: Some(1),
                min_bytes: None,
                max_bytes: None,
                geospatial: Some(GeospatialStatistics {
                    bounding_box: Some(BoundingBox {
                        xmin: -3.0,
                        xmax: 1.0,
                        ymin: 2.0,
                        ymax: 7.0,
                        zmin: None,
                        zmax: None,
                        mmin: None,
                        mmax: None,
                    }),
                    geometry_types: vec![1],
                }),
            }],
        }],
    };

    let value = Scalar::from(statistics);
    assert_eq!(
        value.get_key_str("num_rows").and_then(Scalar::as_i64),
        Some(2)
    );
    let metadata = value
        .get_key_str("key_value_metadata")
        .and_then(Scalar::as_sequence)
        .unwrap();
    assert_eq!(metadata.len(), 2);
    assert_eq!(
        metadata[0].get_key_str("key").and_then(Scalar::as_str),
        Some("tag")
    );
    let geospatial = value
        .get_key_str("row_groups")
        .and_then(Scalar::as_sequence)
        .and_then(|groups| groups[0].get_key_str("columns"))
        .and_then(Scalar::as_sequence)
        .and_then(|columns| columns[0].get_key_str("geospatial"))
        .unwrap();
    assert_eq!(
        geospatial
            .get_key_str("geometry_types")
            .and_then(Scalar::as_sequence)
            .and_then(|types| types[0].as_i64()),
        Some(1)
    );
}

mod parquet {
    use std::hash::Hash;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, Field, StructType, Url};

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

    /// A handle whose media type comes from the name, so codings are declared.
    fn handle(name: &str) -> Buffer {
        Buffer::new().with_media_type(
            Url::from_str(&format!("file:///{name}"))
                .unwrap()
                .media_type(),
        )
    }

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
}
