//! Apache Iceberg tables, read and written through one [`IOBase`] handle.
//!
//! **An Iceberg table is a folder.** `metadata/` holds the JSON documents and
//! the Avro manifests, `data/` holds the Parquet files, and this module reaches
//! every one of them with [`IOBase::child_by_path`] and [`IOBase::ls`] against the
//! handle a [`Table`] was constructed from. Nothing here opens a path or calls
//! the file system, so the same code works over a local directory today and
//! over an object store the moment a backend for one exists. The relationship
//! runs both ways: [`Table`] itself implements [`IOBase`], so the generic
//! record surface ([`IOBase::read_arrow_batch_reader`],
//! [`IOBase::write_arrow_batch_reader`], [`IOBase::append_arrow_batch_reader`])
//! works on the table value directly, answered from the metadata it already
//! holds rather than by probing the location again.
//!
//! The vocabulary is the crate's own. A schema is a non-null struct
//! [`Field`](crate::Field) whose children carry `PARQUET:field_id`, a metadata
//! document is a [`Value`](crate::Value) read by [`crate::json`], a data file
//! is whatever
//! [`crate::parquet`] wrote plus the statistics it reported, and a scan is a
//! [`BatchReader`](crate::arrow::BatchReader) with the same column pushdown
//! every other read in the crate gets. No dependency is added for the table
//! format itself: even the Avro container the manifests live in is implemented
//! here, because it is a header and some blocks.
//!
//! ```no_run
//! use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
//! use yggdryl::local::Folder;
//! use yggdryl::{DataType, Field};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut schema = DataType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::Utf8.nullable_field("venue"),
//! ])?
//! .required_field("row");
//! assign_field_ids(&mut schema, 1)?;
//!
//! let folder = Folder::new(std::env::temp_dir().join("yggdryl-trades"))?;
//! let spec = PartitionSpec::identity(0, &schema, &["venue"])?;
//! let mut table = Table::create(folder, FormatVersion::V2, schema.clone(), spec)?;
//!
//! // A table that has never been written to has no current snapshot.
//! assert!(table.current_snapshot().is_none());
//!
//! let rows = yggdryl::arrow::batch_reader(schema.to_arrow_schema()?, []);
//! table.append(rows)?;
//! assert!(table.current_snapshot().is_some());
//! # Ok(())
//! # }
//! ```
//!
//! # Schema conversion alone
//!
//! [`schema_from_json`] and [`schema_to_json`] convert an Iceberg schema
//! document to and from a root [`Field`](crate::Field) without touching a
//! table, which is
//! what a caller integrating with someone else's catalog needs.
//!
//! ```
//! use yggdryl::iceberg::{schema_from_json, schema_to_json};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let document = yggdryl::json::from_str(
//!     r#"{"type":"struct","schema-id":0,"fields":[
//!         {"id":1,"name":"id","required":true,"type":"long"},
//!         {"id":2,"name":"symbol","required":false,"type":"string"}
//!     ]}"#,
//! )?;
//!
//! let schema = schema_from_json("row", &document)?;
//! assert_eq!(schema.field_len(), 2);
//! assert_eq!(schema.fields()[0].parquet_field_id()?, Some(1));
//! assert!(!schema.fields()[0].is_nullable());
//!
//! // And writes back to the same document.
//! assert_eq!(schema_to_json(&schema)?, document);
//! # Ok(())
//! # }
//! ```
//!
//! # What this module is not
//!
//! It holds no catalog client, no network code, and no transaction protocol.
//! Committing a snapshot means writing metadata somewhere, and *where* is the
//! [`IOBase`] handle the caller supplies, so the format and the transport stay
//! separate. A table found by [`Table::open`] is located the way `HadoopTables`
//! locates one - `metadata/version-hint.text`, falling back to the
//! highest-numbered document - because that is the only way to find a table
//! without a catalog.
//!
//! Two transforms can place a row: `identity` and `void`. A write against a
//! spec using `bucket`, `truncate`, or a calendar transform is refused by name
//! rather than silently writing rows into the wrong partition; reading such a
//! table is unaffected, because a manifest already records which partition each
//! file belongs to.

mod catalog;
mod evolve;
mod inspect;
mod manifest;
mod metadata;
mod options;
mod partition;
mod scan;
mod schema;
mod snapshot;
mod statistics;
mod table;
mod types;
mod value;

pub use catalog::{Catalog, Catalogs, Names, Namespace, Namespaces, Tables};
pub use evolve::{SchemaUpdate, can_promote};
pub use manifest::{
    DataFile, DataFileContent, EntryStatus, FieldSummary, FileFormat, ManifestContent,
    ManifestEntry, ManifestFile, read_manifest, read_manifest_for_plan, read_manifest_list,
    read_manifest_spec, write_manifest, write_manifest_list,
};
pub use metadata::{FormatVersion, SortField, SortOrder, TableMetadata};
pub use options::IcebergOptions;
pub use partition::{FIRST_PARTITION_ID, PartitionField, PartitionSpec, Transform};
pub use scan::{ScanPlan, ScanTask};
pub use schema::{assign_field_ids, last_field_id, schema_from_json, schema_to_json};
pub use snapshot::{MAIN_BRANCH, Snapshot, SnapshotRef};
pub use table::{CommitConflict, Compaction, Table};
pub use types::PrimitiveType;

use crate::Result;
use crate::generic::{Holder, RecordOptions};
use crate::io::IOBase;

/// The directory an Iceberg table keeps its data files under.
const DATA_DIR: &str = "data";

/// A table reached through a handle, and the partition its location addresses.
///
/// This is what [`IOBase`]'s three record methods hand a container to. A handle
/// addressing the table's folder holds the whole table; a handle addressing one
/// of its `column=value` directories holds the same table plus the filters that
/// directory spells out, so reading and upserting one partition of a table is
/// the same call as reading and upserting one partition of a plain folder.
pub(crate) struct Located {
    /// The table itself, opened from whichever ancestor holds its metadata.
    table: Table<Holder>,
    /// The `column=value` pairs the addressed location spells below the table.
    filters: Vec<(String, String)>,
}

impl Located {
    /// Return the table a container handle addresses, if it addresses one.
    ///
    /// The probe is one child lookup per level and it stops immediately: a
    /// folder that is not a table and whose name is not a partition directory
    /// is answered after a single `metadata/` lookup. Only the segments a Hive
    /// layout could have produced - `column=value` and the table's own `data` -
    /// are climbed, so this can never walk a caller's whole filesystem looking
    /// for a table.
    ///
    /// # Errors
    ///
    /// Returns an error when a metadata document is found but cannot be read.
    pub(crate) fn of(handle: &(impl IOBase + ?Sized)) -> Result<Option<Self>> {
        let Some(url) = handle.url() else {
            return Ok(None);
        };
        let segments: Vec<&str> = url.path_segments().collect();
        let mut filters: Vec<(String, String)> = Vec::new();
        let mut climbed = 0_usize;
        let mut passed_data = false;
        loop {
            let relative = if climbed == 0 {
                ".".to_owned()
            } else {
                vec![".."; climbed].join("/")
            };
            if let Some(table) = Table::locate(handle.child_by_path(&relative)?)? {
                filters.reverse();
                return Ok(Some(Self { table, filters }));
            }
            let Some(name) = segments
                .len()
                .checked_sub(climbed + 1)
                .and_then(|index| segments.get(index))
            else {
                return Ok(None);
            };
            match name.split_once('=') {
                Some((column, value)) => filters.push((column.to_owned(), value.to_owned())),
                // A table's `data` directory is the one segment that is not a
                // partition between the table and its files, and there is
                // exactly one of it.
                None if *name == DATA_DIR && !passed_data => passed_data = true,
                // Anything else is an ordinary folder, so this is not a table
                // and the climb stops rather than walking a whole filesystem.
                _ => return Ok(None),
            }
            climbed += 1;
        }
    }

    /// Read the table's rows, filtered by whatever the location addressed.
    ///
    /// # Errors
    ///
    /// Returns a metadata, manifest, or read failure.
    pub(crate) fn read(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        use crate::generic::IORecordOptions;

        self.table.scan_where(&self.pairs(), options.schema())
    }

    /// Replace or merge the table's rows, per the options' match key.
    ///
    /// # Errors
    ///
    /// Returns a metadata, manifest, read, or write failure.
    pub(crate) fn write(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        use crate::generic::IORecordOptions;

        let filters: Vec<(String, String)> = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.table
            .merge_where(&pairs, batches, options.merge_by_names(), options.safe())
    }

    /// Add the rows as a new snapshot, keeping every stored file.
    ///
    /// # Errors
    ///
    /// Returns a metadata, manifest, or write failure.
    pub(crate) fn append(&mut self, batches: crate::arrow::BatchReader) -> Result<()> {
        self.table.append(batches)
    }

    /// Return the encoding this table's data files are written in.
    ///
    /// Answered by the table's own [`IOBase::record_options`], so the one
    /// place that knows what an Iceberg table's rows are is the table.
    pub(crate) fn record_options(&self) -> Result<RecordOptions> {
        self.table.record_options()
    }

    /// Borrow the located filters as the pairs a scan takes.
    fn pairs(&self) -> Vec<(&str, &str)> {
        self.filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect()
    }
}

/// Return the Iceberg table a container handle addresses, if it addresses one.
///
/// This is the one question [`IOBase`]'s record methods ask before they treat a
/// container as a folder of leaves. `None` means an ordinary folder, which is
/// what keeps the folder half unchanged for everything that is not a table.
///
/// # Errors
///
/// Returns an error when a metadata document is found but cannot be read.
pub(crate) fn located(handle: &(impl IOBase + ?Sized)) -> Result<Option<Located>> {
    Located::of(handle)
}

#[cfg(test)]
mod tests;
