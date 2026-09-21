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
