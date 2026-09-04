//! Apache Iceberg tables over one [`IOBase`] handle.
//!
//! The optional `iceberg` feature delegates metadata and schema builders and
//! manifest/list readers to official Iceberg 0.10.1. That dependency requires
//! Rust 1.94 and keeps its Arrow 58 types internal. Yggdryl owns the public
//! [`Field`](crate::Field), Arrow 59 record boundary, [`IOBase`] storage and
//! publication, data-file writes, deterministic manifest/list writers,
//! planning, and scans. The local writers remain because the official 0.10.1
//! writers materialize unbounded output, produce random or order-dependent
//! Avro bytes, and encode Iceberg UUID partitions as Avro strings instead of
//! `fixed[16]`.
//!
//! A table is one container: `metadata/` holds metadata and manifests, and
//! `data/` holds record files. [`Table`] reaches both through its supplied
//! handle and implements [`IOBase`], so [`crate::io::IOMedia`] operations use
//! the same storage path.
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
//! let folder = Folder::new(Folder::temporary()?.path()?.join("yggdryl-trades"))?;
//! let spec = PartitionSpec::identity(0, &schema, &["venue"])?;
//! let mut table = Table::create(folder, FormatVersion::V2, schema.clone(), spec)?;
//!
//! // A table that has never been written to has no current snapshot.
//! assert!(table.current_snapshot().is_none());
//!
//! let rows = yggdryl::arrow::batch_reader(schema.into_arrow_schema()?, []);
//! table.commit_append(rows)?;
//! assert!(table.current_snapshot().is_some());
//! # Ok(())
//! # }
//! ```
//!
//! # Schema conversion alone
//!
//! [`schema_from_json`] and [`schema_into_json`] convert an Iceberg schema
//! document to and from a root [`Field`](crate::Field) without touching a
//! table, which is
//! what a caller integrating with someone else's catalog needs.
//!
//! ```
//! use yggdryl::iceberg::{schema_from_json, schema_into_json};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let document = yggdryl::json::from_utf8(
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
//! assert_eq!(schema_into_json(&schema)?, document);
//! # Ok(())
//! # }
//! ```
//!
//! # Scope
//!
//! Yggdryl supplies storage and publication, not a remote catalog client.
//! [`Table::open`] resolves `metadata/version-hint.text`, then falls back to the
//! highest-numbered metadata document.
//!
//! Writes support `bucket`, `truncate`, `year`, `month`, `day`, `hour`,
//! `identity`, and `void` through the official scalar transform contract.
//! Unknown transforms remain readable metadata but are rejected for writes.

mod catalog;
mod evolve;
mod field;
mod inspect;
mod manifest;
mod metadata;
mod official;
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
    DataFile, EntryStatus, FieldSummary, ManifestContent, ManifestEntry, ManifestFile,
    read_manifest, read_manifest_for_plan, read_manifest_list, read_manifest_spec, write_manifest,
    write_manifest_list,
};
pub use metadata::{FormatVersion, SortField, SortOrder, TableMetadata};
pub use options::IcebergOptions;
pub use partition::{FIRST_PARTITION_ID, PartitionField, PartitionSpec, Transform};
pub use scan::{ScanPlan, ScanTask};
pub use schema::{assign_field_ids, last_column_id, schema_from_json, schema_into_json};
pub use snapshot::{MAIN_BRANCH, Snapshot, SnapshotRef};
pub use table::{CommitConflict, Compaction, Table};
pub use types::PrimitiveType;

use crate::generic::{Holder, RecordOptions};
use crate::io::{IOBase, IOMedia};
use crate::{Error, Result};

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
    /// Return the table field used to shape every chunk of one resumed write.
    pub(crate) fn stored_field(&self) -> Result<crate::Field> {
        self.table.schema().cloned()
    }

    /// Publish one already-shaped overwrite cadence.
    pub(crate) fn overwrite_prepared(
        &mut self,
        batches: crate::arrow::BatchReader,
        safe: bool,
    ) -> Result<()> {
        let filters = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.table.commit_merge_where(&pairs, batches, &[], safe)
    }

    /// Publish one already-shaped append cadence.
    pub(crate) fn append_prepared(&mut self, batches: crate::arrow::BatchReader) -> Result<()> {
        self.table.commit_append(batches)
    }

    /// Publish one already-shaped merge cadence.
    pub(crate) fn merge_prepared(
        &mut self,
        batches: crate::arrow::BatchReader,
        merge_by_names: &[String],
        safe: bool,
    ) -> Result<()> {
        let filters = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.table
            .commit_merge_where(&pairs, batches, merge_by_names, safe)
    }

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

        self.table
            .scan_where(&self.pairs(), options.field().as_ref())
    }

    /// Replace the addressed table partition in one commit.
    ///
    /// # Errors
    ///
    /// Returns a metadata, manifest, read, or write failure.
    pub(crate) fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        use crate::generic::IORecordOptions;

        options.require_write_mode(crate::IOMode::Overwrite)?;
        let commit_row_size = options.require_commit_row_size()?;
        let stored = self.table.schema()?.clone();
        let (batches, _, _) = crate::io::prepare_arrow_write_onto(batches, options, Some(&stored))?;
        let filters: Vec<(String, String)> = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        if commit_row_size.is_none() {
            return self
                .table
                .commit_merge_where(&pairs, batches, &[], options.safe());
        }
        let schema = batches.schema();
        let mut commits = options.commit_arrow_readers(batches)?;
        let Some(first) = commits.next() else {
            return self.table.commit_merge_where(
                &pairs,
                crate::arrow::batch_reader(schema, []),
                &[],
                options.safe(),
            );
        };
        self.table
            .commit_merge_where(&pairs, first?, &[], options.safe())?;
        for commit in commits {
            self.table.commit_append(commit?)?;
        }
        Ok(())
    }

    /// Add the rows as a new snapshot, keeping every stored file.
    ///
    /// # Errors
    ///
    /// Returns a metadata, manifest, or write failure.
    pub(crate) fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        use crate::generic::IORecordOptions;

        options.require_write_mode(crate::IOMode::Append)?;
        let commit_row_size = options.require_commit_row_size()?;
        options.require_write_limits()?;
        if options.write_limit_is_zero() {
            return Ok(());
        }
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        let stored = self.table.schema()?.clone();
        let (batches, _, _) = crate::io::prepare_arrow_write_onto(batches, options, Some(&stored))?;
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        if commit_row_size.is_none() {
            return self.table.commit_append(batches);
        }
        for commit in options.commit_arrow_readers(batches)? {
            self.table.commit_append(commit?)?;
        }
        Ok(())
    }

    /// Merge rows into the addressed table partition in one commit.
    ///
    /// # Errors
    ///
    /// Returns a metadata, manifest, read, merge, or write failure.
    pub(crate) fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        use crate::generic::IORecordOptions;

        options.require_write_mode(crate::IOMode::Merge)?;
        let commit_row_size = options.require_commit_row_size()?;
        options.require_write_limits()?;
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        let stored = self.table.schema()?.clone();
        let (batches, _, _) = crate::io::prepare_arrow_write_onto(batches, options, Some(&stored))?;
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        let filters: Vec<(String, String)> = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        if commit_row_size.is_none() {
            return self.table.commit_merge_where(
                &pairs,
                batches,
                options.merge_by_names(),
                options.safe(),
            );
        }
        for commit in options.commit_arrow_readers(batches)? {
            self.table.commit_merge_where(
                &pairs,
                commit?,
                options.merge_by_names(),
                options.safe(),
            )?;
        }
        Ok(())
    }

    /// Return the rows at the addressed table location.
    ///
    /// Partition predicates settled by manifest metadata need no data-file
    /// read. A predicate left residual by the plan is counted from the scan,
    /// because a file statistic can bound a value but cannot count matches.
    pub(crate) fn row_size(&self) -> Result<u64> {
        if self.filters.is_empty() {
            return self.table.row_size();
        }
        let pairs = self.pairs();
        let plan = self.table.plan(&pairs)?;
        if plan.tasks.iter().all(|task| task.residual.is_empty()) {
            return u64::try_from(plan.record_count()?).map_err(|_| Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::SmolStr::new_static(
                    "expected Iceberg manifests to carry non-negative record counts",
                ),
            });
        }
        let mut rows = 0_u64;
        for batch in self.table.scan_where(&pairs, None)? {
            let batch = batch.map_err(crate::arrow::from_reader_error)?;
            rows = rows
                .checked_add(
                    u64::try_from(batch.num_rows()).map_err(|_| Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$"),
                        reason: smol_str::SmolStr::new_static(
                            "logical row count does not fit in u64",
                        ),
                    })?,
                )
                .ok_or_else(|| Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: smol_str::SmolStr::new_static("logical row count exceeds u64::MAX"),
                })?;
        }
        Ok(rows)
    }

    /// Return the addressed table's current schema width from metadata.
    pub(crate) fn column_size(&self) -> Result<usize> {
        self.table.column_size()
    }

    /// Return the encoding this table's data files are written in.
    ///
    /// Answered by the table's own [`crate::io::IOMedia::record_options`], so the one
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
