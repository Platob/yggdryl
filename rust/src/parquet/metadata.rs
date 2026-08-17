//! Footer statistics recovered from a Parquet file.
//!
//! These are the values a query planner uses to skip row groups and an
//! Iceberg manifest writer records per data file: row counts, byte sizes,
//! null counts, and encoded value bounds.

use parquet::file::metadata::ParquetMetaData;
use parquet::file::statistics::Statistics;

/// Bounds and counts for one column chunk within one row group.
#[derive(Clone, Debug, Eq, PartialEq)]
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
    pub min_bytes: Option<Vec<u8>>,
    /// Encoded maximum value, when the writer recorded one.
    pub max_bytes: Option<Vec<u8>>,
}

/// Counts and per-column statistics for one row group.
#[derive(Clone, Debug, Eq, PartialEq)]
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
#[derive(Clone, Debug, Eq, PartialEq)]
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
                    .map(|column| ColumnStatistics {
                        path: column.column_path().string(),
                        compressed_size: column.compressed_size(),
                        uncompressed_size: column.uncompressed_size(),
                        null_count: column.statistics().and_then(Statistics::null_count_opt),
                        min_bytes: column.statistics().and_then(min_bytes),
                        max_bytes: column.statistics().and_then(max_bytes),
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

/// Borrow a statistic's encoded minimum, when the writer recorded one.
fn min_bytes(statistics: &Statistics) -> Option<Vec<u8>> {
    statistics.min_bytes_opt().map(<[u8]>::to_vec)
}

/// Borrow a statistic's encoded maximum, when the writer recorded one.
fn max_bytes(statistics: &Statistics) -> Option<Vec<u8>> {
    statistics.max_bytes_opt().map(<[u8]>::to_vec)
}
