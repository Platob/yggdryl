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
//! `data/` holds record files. [`IcebergTable`] reaches both through its supplied
//! handle and implements [`IOBase`], so [`crate::IOMedia`] operations use
//! the same storage path.
//!
//! ```no_run
//! use yggdryl::iceberg::{FormatVersion, PartitionSpec, IcebergTable, assign_field_ids};
//! use yggdryl::local::LocalFolder;
//! use yggdryl::{DataType, Field, StructType};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let mut schema = DataType::from(StructType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::utf8().nullable_field("venue"),
//! ])?)
//! .required_field("row");
//! assign_field_ids(&mut schema, 1)?;
//!
//! let folder = LocalFolder::new(LocalFolder::temporary()?.path()?.join("yggdryl-trades"))?;
//! let spec = PartitionSpec::identity(0, &schema, &["venue"])?;
//! let mut table = IcebergTable::create(folder, FormatVersion::V2, schema.clone(), spec)?;
//!
//! // A table that has never been written to has no current snapshot.
//! assert!(table.current_snapshot()?.is_none());
//!
//! let rows = yggdryl::arrow::batch_reader(schema.into_arrow_schema()?, []);
//! table.commit_append(rows)?;
//! assert!(table.current_snapshot()?.is_some());
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
//! Yggdryl supplies storage and publication. [`IcebergTable::open`] starts
//! from `metadata/version-hint.text`, else the highest-numbered metadata
//! document, and reads on to every newer version's document there is; a
//! commit claims its version by creating that document exclusively
//! ([`IOBase::create_bytes`]). A table whose current document a catalog
//! service names is opened through a [`MetadataPointer`] instead
//! ([`IcebergTable::open_pointed`]), which is how the Amazon S3 Tables
//! catalog commits. [`IcebergTable::from_url`],
//! [`IcebergTable::create_from_url`] and
//! [`IcebergTable::open_or_create_from_url`] reach a table by its location
//! alone - a folder any backend holds or, under the `s3tables` feature, a
//! table an Amazon S3 Tables table bucket keeps - with nothing built first.
//!
//! Writes support `bucket`, `truncate`, `year`, `month`, `day`, `hour`,
//! `identity`, and `void` through the official scalar transform contract.
//! Unknown transforms remain readable metadata but are rejected for writes.

mod catalog;
mod evolve;
mod field;
mod inspect;
pub(crate) mod manifest;
pub(crate) mod metadata;
mod official;
pub(crate) mod options;
pub(crate) mod partition;
mod pointer;
pub(crate) mod scan;
mod schema;
pub(crate) mod snapshot;
pub(crate) mod staging;
pub(crate) mod statistics;
pub(crate) mod table;
mod types;
pub(crate) mod value;

pub(crate) use catalog::create_layout;
pub use catalog::{HADOOP_FACTORY, HadoopFactory, IcebergCatalog, IcebergNamespace};
pub use evolve::{SchemaUpdate, can_promote};
pub use manifest::{
    DataFile, EntryStatus, FieldSummary, ManifestContent, ManifestEntry, ManifestFile,
    read_manifest, read_manifest_for_plan, read_manifest_list, read_manifest_spec, write_manifest,
    write_manifest_list,
};
pub use metadata::{FormatVersion, SortField, SortOrder, TableMetadata};
pub use options::{IcebergOptions, WriteStaging};
pub use partition::{FIRST_PARTITION_ID, PartitionField, PartitionSpec, Transform};
pub use pointer::{MetadataPointer, PointerState};
pub use scan::{ScanPlan, ScanTask};
pub use schema::{assign_field_ids, last_column_id, schema_from_json, schema_into_json};
pub use snapshot::{MAIN_BRANCH, Snapshot, SnapshotRef};
pub use table::{CommitConflict, Compaction, IcebergTable};
pub use types::PrimitiveType;

use table::ReplacedPartitions;

use crate::holder::Holder;
use crate::media::{LocatedTable, RecordOptions, TableFormat};
use crate::{Error, Result};
use crate::{IOBase, IOMedia};
use crate::{Properties, Table};

/// The directory an Iceberg table keeps its data files under.
const DATA_DIR: &str = "data";

/// A table reached through a handle, and the partition its location addresses:
/// what [`IcebergFormat`] answers for a container laid out as an Iceberg
/// table, and what the record doors then drive through [`LocatedTable`].
///
/// A handle addressing the table's folder holds the whole table; a handle
/// addressing one of its `column=value` directories holds the same table plus
/// the filters that directory spells out, so reading and upserting one
/// partition of a table is the same call as reading and upserting one
/// partition of a plain folder. One value lives as long as the write session
/// that located it, and holds what that write's cadences replaced so far.
#[derive(Debug)]
pub(crate) struct Located {
    /// The table itself, opened from whichever ancestor holds its metadata.
    table: IcebergTable<Holder>,
    /// The `column=value` pairs the addressed location spells below the table.
    filters: Vec<(String, String)>,
    /// What the write's earlier cadences replaced: an addressed partition is
    /// replaced by the first cadence and appended to after, and a table
    /// addressed whole has each partition its rows reach replaced once.
    replaced: ReplacedPartitions,
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
        // A pattern names leaves, never a table's folder: nothing is asked
        // of a store for one, where the climb would read a hint under the
        // pattern and list everything beneath its fixed prefix.
        if url.is_glob() {
            return Ok(None);
        }
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
            if let Some(table) = IcebergTable::locate(handle.child_by_path(&relative)?)? {
                filters.reverse();
                return Ok(Some(Self {
                    table,
                    filters,
                    replaced: ReplacedPartitions::default(),
                }));
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

    /// `IcebergTable::write_cadenced` over the partitions this location
    /// addresses.
    fn write_cadenced(
        &mut self,
        batches: crate::arrow::BatchReader,
        mode: crate::IOMode,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        use crate::media::IORecordOptions as _;

        // The directories this location was reached through and the
        // equalities the options' `where` spells name the scope together,
        // as the table's own doors read the `where` alone: a column the
        // path already pins is not pinned twice.
        let mut filters = self.filters.clone();
        if mode != crate::IOMode::Append {
            for (column, value) in options.partition_pairs() {
                if !filters.iter().any(|(held, _)| *held == column) {
                    filters.push((column, value));
                }
            }
        }
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.table.write_cadenced(batches, mode, options, &pairs)
    }

    /// Borrow the located filters as the pairs a scan takes.
    fn pairs(&self) -> Vec<(&str, &str)> {
        self.filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect()
    }
}

impl LocatedTable for Located {
    /// Whether the location addresses the table whole rather than one of
    /// its partitions.
    fn is_whole(&self) -> bool {
        self.filters.is_empty()
    }

    /// Replace every row of the table with `batches` in one snapshot,
    /// whatever its partitions ([`IcebergTable::commit_overwrite_where`]
    /// with no filter).
    fn overwrite_whole(&mut self, batches: crate::arrow::BatchReader) -> Result<()> {
        self.table.commit_overwrite_where(&[], batches)
    }

    /// Empty the table in one snapshot that keeps it a table
    /// ([`IOBase::clear`] on it).
    fn clear(&mut self) -> Result<()> {
        self.table.clear()
    }

    /// Return the table field used to shape every chunk of one resumed write.
    fn stored_field(&self) -> Result<crate::Field> {
        self.table.schema().cloned()
    }

    /// The table's stored schema, as the table answers its origin: from its
    /// metadata, with no scan planned.
    fn read_origin_field(&self) -> Result<Option<crate::Field>> {
        crate::IOMedia::read_origin_field(&self.table)
    }

    /// The table's schema under the options' root name, as the table
    /// answers it: from its metadata, with no scan planned.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        crate::IOMedia::read_arrow_field(&self.table, options)
    }

    /// Read the table's rows the options ask for, within the addressed
    /// partition.
    ///
    /// The directory's `column=value` pairs and the options' own `where`
    /// clause are both pushed into the scan plan, the `select` and the limit
    /// wrap the result: the returned reader is complete.
    fn read(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        let scope = crate::Filter::all_partitions_equal(
            self.table.schema()?,
            self.filters.iter().map(|(column, value)| (column, value)),
        );
        self.table.read_scoped(scope, options)
    }

    /// Return the rows at the addressed table location.
    ///
    /// Partition predicates settled by manifest metadata need no data-file
    /// read. A predicate left residual by the plan is counted from the scan,
    /// because a file statistic can bound a value but cannot count matches.
    fn row_size(&self) -> Result<u64> {
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
    fn column_size(&self) -> Result<usize> {
        self.table.column_size()
    }

    /// Return the encoding this table's data files are written in.
    ///
    /// Answered by the table's own [`crate::IOMedia::record_options`], so the one
    /// place that knows what an Iceberg table's rows are is the table.
    fn record_options(&self) -> Result<RecordOptions> {
        self.table.record_options()
    }

    /// Replace the addressed table partition - or, addressing the table
    /// whole, the partitions the rows fall in: `IcebergTable::write_cadenced`
    /// under [`IOMode::Overwrite`](crate::IOMode::Overwrite).
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.write_cadenced(batches, crate::IOMode::Overwrite, options)
    }

    /// Add the rows: `IcebergTable::write_cadenced` under
    /// [`IOMode::Append`](crate::IOMode::Append).
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.write_cadenced(batches, crate::IOMode::Append, options)
    }

    /// Merge rows into the addressed table partition:
    /// `IcebergTable::write_cadenced` under [`IOMode::Merge`](crate::IOMode::Merge).
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.write_cadenced(batches, crate::IOMode::Merge, options)
    }

    /// Publish one already-shaped overwrite cadence.
    ///
    /// The rows were cast when they were shaped, so nothing here is safe or
    /// unsafe. What the write's earlier cadences replaced is this value's
    /// own: an addressed partition is replaced by the first cadence and
    /// appended to after, and a table addressed whole has each partition its
    /// rows reach replaced once; see
    /// [`IcebergTable::commit_overwrite_cadence`].
    fn overwrite_prepared(
        &mut self,
        batches: crate::arrow::BatchReader,
        threads: Option<usize>,
    ) -> Result<()> {
        let filters = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.table
            .commit_overwrite_cadence(&pairs, batches, &mut self.replaced, threads, &[])
    }

    /// Publish one already-shaped append cadence: the rows it left out, a
    /// key the table already held (see [`IcebergTable::commit_append`]).
    fn append_prepared(
        &mut self,
        batches: crate::arrow::BatchReader,
        threads: Option<usize>,
    ) -> Result<u64> {
        self.table.commit_append_on(batches, threads, &[])
    }

    /// Publish one already-shaped merge cadence.
    ///
    /// What the write's earlier cadences replaced is this value's own, so a
    /// merge keyed by the partition alone replaces each partition once and
    /// appends to it after; see [`IcebergTable::commit_merge_cadence`].
    fn merge_prepared(
        &mut self,
        batches: crate::arrow::BatchReader,
        merge_by: &crate::Selector,
        safe: bool,
        threads: Option<usize>,
    ) -> Result<()> {
        let filters = self.filters.clone();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.table.commit_merge_cadence(
            &pairs,
            batches,
            merge_by,
            safe,
            &mut self.replaced,
            threads,
            &[],
        )
    }
}

/// The Iceberg table format: a container whose folder - or an ancestor of
/// it, through `column=value` directories and the table's own `data` -
/// holds an Iceberg table's metadata.
///
/// The core claims it as `iceberg` on the register
/// [`crate::media::format`] reads, so every record door reads and writes such
/// a container as the table it is rather than as a folder of leaves.
#[derive(Debug)]
pub struct IcebergFormat;

/// The one [`IcebergFormat`], the value the core claims.
pub static ICEBERG_FORMAT: IcebergFormat = IcebergFormat;

impl TableFormat for IcebergFormat {
    fn name(&self) -> &'static str {
        "iceberg"
    }

    /// The table `handle` addresses, by `Located`'s walk: one child lookup
    /// per level, climbing only the segments a Hive layout could have
    /// produced, so it never walks a caller's whole filesystem.
    fn locate(&self, handle: &dyn IOBase) -> Result<Option<Box<dyn LocatedTable>>> {
        let Some(located) = Located::of(handle)? else {
            return Ok(None);
        };
        Ok(Some(Box::new(located)))
    }

    /// An [`IcebergTable`] rooted on the folder, as a folder catalog lists
    /// one: its root eager and its document read on the first verb that
    /// needs it.
    fn table(
        &self,
        path: Vec<smol_str::SmolStr>,
        root: Holder,
        inherited: &Properties,
    ) -> Result<Table> {
        let root = crate::Handle::bound(root, false, &path, Properties::new());
        Ok(Table::from(
            IcebergTable::at(path, root).inheriting(inherited),
        ))
    }
}
