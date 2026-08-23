//! Footer statistics recovered from a Parquet file.
//!
//! These are the values a query planner uses to skip row groups and an
//! Iceberg manifest writer records per data file: row counts, byte sizes,
//! null counts, and encoded value bounds.

use parquet::file::metadata::ParquetMetaData;
use parquet::file::statistics::Statistics;

use super::GeospatialStatistics;
use crate::Scalar;

/// Bounds and counts for one column chunk within one row group.
///
/// Floating-point geospatial bounds use the same total NaN and signed-zero
/// identity as [`crate::Float64`].
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ColumnStatistics {
    /// Dotted path of the leaf column, such as `address.zip`.
    pub path: String,
    /// Compressed size of the column chunk in bytes.
    pub compressed_size: i64,
    /// Uncompressed size of the column chunk in bytes.
    pub uncompressed_size: i64,
    /// Number of null values, when the writer recorded one.
    pub null_count: Option<u64>,
    /// Encoded minimum value, when the writer recorded one.
    ///
    /// Geospatial columns never record one - their sort order is undefined -
    /// and one found in a foreign file is ignored rather than surfaced.
    pub min_bytes: Option<Vec<u8>>,
    /// Encoded maximum value, when the writer recorded one.
    ///
    /// Absent for geospatial columns, exactly like [`Self::min_bytes`].
    pub max_bytes: Option<Vec<u8>>,
    /// Bounding box and geometry types, when the writer recorded them.
    ///
    /// This is the statistic a geospatial column carries *instead of* value
    /// bounds: Parquet's own `GeospatialStatistics` footer field, projected
    /// into the shared WKB vocabulary.
    pub geospatial: Option<GeospatialStatistics>,
}

/// Counts and per-column statistics for one row group.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RowGroupStatistics {
    /// Rows stored in this group.
    pub num_rows: i64,
    /// Total compressed bytes across every column chunk.
    pub compressed_size: i64,
    /// Byte offset of this group within the file, when the writer recorded one.
    ///
    /// Iceberg records these as a data file's `split_offsets`.
    pub file_offset: Option<i64>,
    /// One entry per leaf column, in schema order.
    pub columns: Vec<ColumnStatistics>,
}

/// Whole-file counts, footer metadata, and per-row-group statistics.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileStatistics {
    /// Total rows across every row group.
    pub num_rows: i64,
    /// The writer that produced the file, when recorded.
    pub created_by: Option<String>,
    /// File-level key/value metadata from the footer.
    pub key_value_metadata: Vec<(String, String)>,
    /// One entry per row group, in file order.
    pub row_groups: Vec<RowGroupStatistics>,
}

impl FileStatistics {
    /// Project Parquet footer metadata into the shared statistics value.
    pub(super) fn from_metadata(metadata: &ParquetMetaData) -> Self {
        let file = metadata.file_metadata();
        let key_value_metadata = file
            .key_value_metadata()
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| (entry.key.clone(), entry.value.clone().unwrap_or_default()))
                    .collect()
            })
            .unwrap_or_default();

        let row_groups = metadata
            .row_groups()
            .iter()
            .map(|group| RowGroupStatistics {
                num_rows: group.num_rows(),
                compressed_size: group.compressed_size(),
                file_offset: group.file_offset(),
                columns: group
                    .columns()
                    .iter()
                    .map(|column| {
                        // A geospatial column's sort order is undefined, so a
                        // min/max a foreign writer recorded anyway is ignored
                        // rather than surfaced as a usable bound.
                        let geospatial_column = matches!(
                            column.column_descr().logical_type_ref(),
                            Some(
                                parquet::basic::LogicalType::Geometry(_)
                                    | parquet::basic::LogicalType::Geography(_)
                            )
                        );
                        ColumnStatistics {
                            path: column.column_path().string(),
                            compressed_size: column.compressed_size(),
                            uncompressed_size: column.uncompressed_size(),
                            null_count: column.statistics().and_then(Statistics::null_count_opt),
                            min_bytes: (!geospatial_column)
                                .then(|| column.statistics().and_then(min_bytes))
                                .flatten(),
                            max_bytes: (!geospatial_column)
                                .then(|| column.statistics().and_then(max_bytes))
                                .flatten(),
                            geospatial: column.geo_statistics().map(super::geospatial::from_footer),
                        }
                    })
                    .collect(),
            })
            .collect();

        Self {
            num_rows: file.num_rows(),
            created_by: file.created_by().map(ToOwned::to_owned),
            key_value_metadata,
            row_groups,
        }
    }

    /// Return the byte offset of every row group that recorded one.
    ///
    /// Iceberg writes exactly this sequence as a data file's `split_offsets`.
    pub fn split_offsets(&self) -> Vec<i64> {
        self.row_groups
            .iter()
            .filter_map(|group| group.file_offset)
            .collect()
    }

    /// Sum the null counts recorded for one leaf column path.
    ///
    /// Returns `None` when no row group recorded a null count for it.
    pub fn null_count(&self, path: &str) -> Option<u64> {
        let mut total = None;
        for group in &self.row_groups {
            for column in &group.columns {
                if column.path == path {
                    if let Some(count) = column.null_count {
                        total = Some(total.unwrap_or(0) + count);
                    }
                }
            }
        }
        total
    }
}

/// Project whole-file statistics into the shared cross-language value tree.
///
/// Footer key/value entries stay an ordered sequence rather than becoming a
/// mapping: Parquet permits repeated keys, and a binding must not silently
/// discard one while adapting the native value to a language object.
impl From<FileStatistics> for Scalar {
    fn from(statistics: FileStatistics) -> Self {
        statistics_record([
            ("num_rows", Scalar::I64(statistics.num_rows)),
            (
                "created_by",
                statistics.created_by.map_or(Scalar::Null, Scalar::from),
            ),
            (
                "key_value_metadata",
                Scalar::from_sequence(statistics.key_value_metadata.into_iter().map(
                    |(key, value)| {
                        statistics_record([
                            ("key", Scalar::from(key)),
                            ("value", Scalar::from(value)),
                        ])
                    },
                )),
            ),
            (
                "row_groups",
                Scalar::from_sequence(statistics.row_groups.into_iter().map(Scalar::from)),
            ),
        ])
    }
}

impl From<RowGroupStatistics> for Scalar {
    fn from(statistics: RowGroupStatistics) -> Self {
        statistics_record([
            ("num_rows", Scalar::I64(statistics.num_rows)),
            ("compressed_size", Scalar::I64(statistics.compressed_size)),
            (
                "file_offset",
                statistics.file_offset.map_or(Scalar::Null, Scalar::I64),
            ),
            (
                "columns",
                Scalar::from_sequence(statistics.columns.into_iter().map(Scalar::from)),
            ),
        ])
    }
}

impl From<ColumnStatistics> for Scalar {
    fn from(statistics: ColumnStatistics) -> Self {
        statistics_record([
            ("path", Scalar::from(statistics.path)),
            ("compressed_size", Scalar::I64(statistics.compressed_size)),
            (
                "uncompressed_size",
                Scalar::I64(statistics.uncompressed_size),
            ),
            (
                "null_count",
                statistics.null_count.map_or(Scalar::Null, Scalar::U64),
            ),
            (
                "min_bytes",
                statistics.min_bytes.map_or(Scalar::Null, Scalar::from),
            ),
            (
                "max_bytes",
                statistics.max_bytes.map_or(Scalar::Null, Scalar::from),
            ),
            (
                "geospatial",
                statistics.geospatial.map_or(Scalar::Null, Scalar::from),
            ),
        ])
    }
}

impl From<GeospatialStatistics> for Scalar {
    fn from(statistics: GeospatialStatistics) -> Self {
        statistics_record([
            (
                "bounding_box",
                statistics.bounding_box.map_or(Scalar::Null, |bounds| {
                    statistics_record([
                        ("xmin", Scalar::from(bounds.xmin)),
                        ("xmax", Scalar::from(bounds.xmax)),
                        ("ymin", Scalar::from(bounds.ymin)),
                        ("ymax", Scalar::from(bounds.ymax)),
                        ("zmin", bounds.zmin.map_or(Scalar::Null, Scalar::from)),
                        ("zmax", bounds.zmax.map_or(Scalar::Null, Scalar::from)),
                        ("mmin", bounds.mmin.map_or(Scalar::Null, Scalar::from)),
                        ("mmax", bounds.mmax.map_or(Scalar::Null, Scalar::from)),
                    ])
                }),
            ),
            (
                "geometry_types",
                Scalar::from_sequence(statistics.geometry_types.into_iter().map(Scalar::I32)),
            ),
        ])
    }
}

/// Build a record whose field names are fixed, distinct literals above.
fn statistics_record<const N: usize>(entries: [(&'static str, Scalar); N]) -> Scalar {
    // Every call above uses distinct literals. Keeping construction here makes
    // that one auditable invariant and prevents either binding from growing a
    // separate Parquet DTO renderer.
    Scalar::from_record(entries).expect("Parquet statistics field names are distinct")
}

/// Borrow a statistic's encoded minimum, when the writer recorded one.
fn min_bytes(statistics: &Statistics) -> Option<Vec<u8>> {
    statistics.min_bytes_opt().map(<[u8]>::to_vec)
}

/// Borrow a statistic's encoded maximum, when the writer recorded one.
fn max_bytes(statistics: &Statistics) -> Option<Vec<u8>> {
    statistics.max_bytes_opt().map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::{ColumnStatistics, FileStatistics, GeospatialStatistics, RowGroupStatistics};
    use crate::Scalar;
    use crate::generic::wkb::BoundingBox;

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
            metadata[0].get_key_str("key").and_then(Scalar::as_utf8),
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
}
