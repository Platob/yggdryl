//! An Iceberg table as a folder handle and nothing else.
//!
//! A table *is* a directory: `metadata/` holds the JSON documents and the Avro
//! manifests, `data/` holds the Parquet files, and every one of them is reached
//! with [`IOBase::child_by`] against the handle the table was constructed from.
//! There is no path opening and no file-system call anywhere below here, which
//! is what makes the same code work over a local folder today and over an
//! object store the moment a backend for one exists.
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
//! let folder = Folder::new(std::env::temp_dir().join("trades"))?;
//! let spec = PartitionSpec::identity(0, &schema, &["venue"])?;
//! let table = Table::create(folder, FormatVersion::V2, schema, spec)?;
//!
//! // A table with no snapshot yet reads as no rows, never as a failure.
//! assert!(table.current_snapshot().is_none());
//! assert_eq!(table.scan(None)?.count(), 0);
//! # Ok(())
//! # }
//! ```
//!
//! # What a commit costs
//!
//! Committing means writing a new metadata document, so an append writes at
//! least one Parquet file per partition - more when a partition's rows exceed
//! [`Table::target_file_size`] - one manifest, one manifest list, and one
//! metadata JSON. Nothing is mutated in place, which is what makes the previous
//! snapshot still readable afterwards.
//!
//! # Concurrent writers
//!
//! Commits are optimistic. Before publishing version N+1 a commit re-checks
//! the current version with the same lookup [`Table::open`] uses; a commit
//! that finds itself beaten *rebases* when that is safe - it reloads the
//! winner's document and re-applies its own intent on top, with exponential
//! jittered backoff between attempts, bounded by
//! [`IcebergOptions::commit_retries`] - and otherwise reports a
//! [`CommitConflict`] naming both versions. An append and a metadata-only
//! change rebase; [`Table::overwrite_where`], [`Table::merge_where`], and
//! [`Table::compact`] cannot, because they planned against files a concurrent
//! commit may have replaced and their input readers are already consumed, so
//! they conflict instead. Readers are never blocked, and a failed commit
//! leaves no visible change - at worst it orphans data files no snapshot
//! names.
//!
//! **The version check is racy on plain storage.** [`IOBase`] has no
//! compare-and-swap, so two writers can still observe the same version and
//! publish the same document number, one silently over the other. Retries
//! shrink that window; they cannot close it. Serialized writers - a catalog,
//! a lock, one writer per table - are what closes it.
//!
//! # Branches and tags
//!
//! [`Table::create_branch`], [`Table::create_tag`], [`Table::remove_ref`],
//! [`Table::fast_forward`], and [`Table::expire_snapshots`] are thin wrappers
//! over [`TableMetadata`]'s ref vocabulary, each committed through the same
//! retrying [`Table::commit_changes`]. Writing *to* a branch other than
//! `main` remains future work, because a commit's parent is currently always
//! the table's current snapshot.

use std::collections::HashMap;

use arrow_array::{Array, ArrayRef, RecordBatch, UInt32Array};
use arrow_schema::SortOptions;
use smol_str::{SmolStr, format_smolstr};

use super::manifest::{
    DataFile, FieldSummary, FileFormat, ManifestContent, ManifestEntry, ManifestFile,
    read_manifest_list, write_manifest, write_manifest_list,
};
use super::metadata::{FormatVersion, TableMetadata, now_ms, uuid};
use super::options::IcebergOptions;
use super::partition::PartitionSpec;
use super::scan::{Filter, ScanPart, ScanPlan, ScanTask};
use super::snapshot::{Snapshot, SnapshotRef};
use super::value::{compare_single, is_portable, single_value};
use crate::arrow::BatchReader;
use crate::field::cast::ArrowCast;
use crate::generic::{Holder, IORecordOptions, RecordOptions};
use crate::io::IOBase;
use crate::{DataType, Error, Field, Result, Value};

/// The directory a table keeps its metadata documents and manifests in.
const METADATA_DIR: &str = "metadata";

/// The directory a table keeps its data files in.
const DATA_DIR: &str = "data";

/// The file naming the current metadata version, as `HadoopTables` writes it.
const VERSION_HINT: &str = "version-hint.text";

/// An Iceberg table reached entirely through one container handle.
///
/// The handle is whatever [`IOBase`] implementation addresses the table's
/// folder. Everything below - metadata documents, manifest lists, manifests,
/// data files - is a child of it. The relationship runs both ways: a `Table`
/// is itself an [`IOBase`], so the generic record surface works on the value
/// directly - see the trait implementation for what each method answers.
#[derive(Debug)]
pub struct Table<H: IOBase> {
    /// The folder the table lives in.
    root: H,
    /// The parsed current metadata document.
    metadata: TableMetadata,
    /// The version number of the metadata document that was last written.
    version: u32,
    /// An explicit options override the resolvers consult before properties.
    options: Option<IcebergOptions>,
}

impl<H: IOBase> Table<H> {
    /// Create a table, writing its first metadata document.
    ///
    /// The table has a schema and a partition spec but no snapshot, which is
    /// exactly what a newly created Iceberg table is. Unnumbered schema
    /// columns are numbered automatically - a schema that already carries
    /// field identifiers keeps every one of them - so a schema projected from
    /// Arrow needs no ceremony first. A caller building a [`PartitionSpec`]
    /// by hand still numbers first with [`super::assign_field_ids`], because
    /// a spec names its source columns by identifier.
    ///
    /// # Errors
    ///
    /// Returns an error when the handle is not a container, when the schema is
    /// not a non-null struct root, or when the metadata document cannot be
    /// written.
    pub fn create(
        root: H,
        format_version: FormatVersion,
        schema: Field,
        spec: PartitionSpec,
    ) -> Result<Self> {
        let location = root.url().map(ToString::to_string).ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a located container to create a table in, got a handle with no URL",
            ))
        })?;
        let metadata = TableMetadata::new(format_version, location, schema, spec)?;
        let mut table = Self {
            root,
            metadata,
            version: 0,
            options: None,
        };
        table.commit_metadata()?;
        Ok(table)
    }

    /// Open the table a container handle addresses.
    ///
    /// The current document is the one `metadata/version-hint.text` names; a
    /// table written by something that keeps no hint falls back to the
    /// highest-numbered `*.metadata.json` in the metadata directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the folder holds no metadata document, or when
    /// the document is not table metadata.
    pub fn open(root: H) -> Result<Self> {
        let metadata_dir = root.child_by(METADATA_DIR)?;
        match find_metadata(&metadata_dir)? {
            Some((version, document)) => Ok(Self {
                root,
                metadata: TableMetadata::from_json(&document)?,
                version,
                options: None,
            }),
            None => Err(missing_metadata(&metadata_dir)),
        }
    }

    /// Open the table a handle addresses, or say plainly that it is not one.
    ///
    /// This is the question [`IOBase`]'s record methods ask of every container
    /// they are handed: a folder holding a metadata document is read through its
    /// snapshots, and a folder that is not one is read as the leaves beneath it.
    /// A folder that *is* a table but whose current document is malformed is an
    /// error rather than a `None`, because that is a broken table and not an
    /// ordinary directory.
    ///
    /// # Errors
    ///
    /// Returns an error when a metadata document is found but is not table
    /// metadata.
    pub fn locate(root: H) -> Result<Option<Self>> {
        let metadata_dir = root.child_by(METADATA_DIR)?;
        let Some((version, document)) = find_metadata(&metadata_dir)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            root,
            metadata: TableMetadata::from_json(&document)?,
            version,
            options: None,
        }))
    }

    /// Open the table if it exists, creating it otherwise.
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create(
        root: H,
        format_version: FormatVersion,
        schema: Field,
        spec: PartitionSpec,
    ) -> Result<Self> {
        let metadata_dir = root.child_by(METADATA_DIR)?;
        if find_metadata(&metadata_dir)?.is_some() {
            return Self::open(root);
        }
        Self::create(root, format_version, schema, spec)
    }

    /// Borrow the container the table lives in.
    pub const fn root(&self) -> &H {
        &self.root
    }

    /// Borrow the current table metadata.
    pub const fn metadata(&self) -> &TableMetadata {
        &self.metadata
    }

    /// Return the version number of the current metadata document.
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Return the name of the current metadata document.
    pub fn metadata_file_name(&self) -> String {
        format!("v{}.metadata.json", self.version)
    }

    /// Return the location of the current metadata document, as a URI.
    ///
    /// # Errors
    ///
    /// Returns an error when the metadata child has no URL.
    pub fn metadata_location(&self) -> Result<String> {
        Ok(format!(
            "{}/{METADATA_DIR}/{}",
            self.metadata.location.trim_end_matches('/'),
            self.metadata_file_name()
        ))
    }

    /// Borrow the schema new data is written against.
    ///
    /// # Errors
    ///
    /// Returns an error when no schema carries the current schema identifier.
    pub fn schema(&self) -> Result<&Field> {
        self.metadata.current_schema()
    }

    /// Borrow the snapshot a reader sees, when the table has one.
    pub fn current_snapshot(&self) -> Option<&Snapshot> {
        self.metadata.current_snapshot()
    }

    /// Return the size a data file aims for, in bytes.
    ///
    /// The one resolver is [`IcebergOptions`]: an explicit option stored with
    /// [`Self::set_options`] wins, then the table property
    /// [`IcebergOptions::TARGET_FILE_SIZE_KEY`], then the schema root's
    /// `iceberg:write.target-file-size-bytes` protocol property, then
    /// Iceberg's own default of 512 MiB.
    ///
    /// What a write measures against this target is the Arrow in-memory size
    /// of the accumulated batches ([`RecordBatch::get_array_memory_size`]),
    /// estimated *before* encoding. Parquet compresses what it writes, so data
    /// files land under the target rather than at it.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key and the value when either property
    /// is present but does not spell a positive byte count; a configured
    /// target is never silently replaced by the default.
    pub fn target_file_size(&self) -> Result<u64> {
        IcebergOptions::target_size(self.options.as_ref(), &self.metadata)
    }

    /// Store an explicit options override the resolvers consult first.
    ///
    /// A field the override sets shadows the table property of the same name -
    /// even one that does not parse, which is what lets a caller repair it -
    /// and a field it leaves unset still resolves property-then-default. The
    /// override lives on this handle alone; it is never written to the table.
    pub fn set_options(&mut self, options: IcebergOptions) {
        self.options = Some(options);
    }

    /// Resolve this table's effective options, field by field.
    ///
    /// Each field takes the nearest of three layers: the explicit override
    /// stored with [`Self::set_options`], then the table property of the same
    /// name (falling back to the schema root's `iceberg:` spelling), and the
    /// getters answer the documented default for whatever remains unset.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key and the value when a property no
    /// explicit option shadows is present but does not parse.
    pub fn options(&self) -> Result<IcebergOptions> {
        IcebergOptions::resolved(self.options.as_ref(), &self.metadata)
    }

    /// Return every manifest the current snapshot points at.
    ///
    /// A table with no current snapshot has no manifests, which is not a
    /// failure: an empty table simply reads as nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest list cannot be reached or decoded.
    pub fn manifests(&self) -> Result<Vec<ManifestFile>> {
        match self.current_snapshot() {
            Some(snapshot) => self.manifests_at(snapshot),
            None => Ok(Vec::new()),
        }
    }

    /// Return every manifest one retained snapshot points at.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest list cannot be reached or decoded.
    pub fn manifests_at(&self, snapshot: &Snapshot) -> Result<Vec<ManifestFile>> {
        if snapshot.manifest_list.is_empty() {
            return Ok(Vec::new());
        }
        let handle = self.child_at(&snapshot.manifest_list)?;
        read_manifest_list(&handle)
    }

    /// Return the retained snapshot a branch or tag names.
    ///
    /// # Errors
    ///
    /// Returns an error naming the refs the table does have when `name` is not
    /// one of them, or when the ref points at a snapshot that is not retained.
    pub fn snapshot_by_ref(&self, name: &str) -> Result<&Snapshot> {
        let reference = self
            .metadata
            .refs
            .iter()
            .find_map(|(candidate, reference)| (candidate == name).then_some(reference))
            .ok_or_else(|| {
                let known: Vec<&str> = self
                    .metadata
                    .refs
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();
                invalid(format_smolstr!(
                    "expected a branch or tag this table has, got {name:?}; it has [{}]",
                    known.join(", ")
                ))
            })?;
        self.metadata
            .snapshot_by_id(reference.snapshot_id)
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected the ref {name:?} to point at a retained snapshot, got {}",
                    reference.snapshot_id
                ))
            })
    }

    /// Plan a scan: decide which data files the metadata says have to be read.
    ///
    /// `filters` is a list of `(column, value)` pairs, the same vocabulary
    /// [`IOBase::children_where`] filters a lake with. Nothing here lists a
    /// directory: the snapshot names a manifest list, whose summaries skip
    /// whole manifests, whose entries carry the partition tuples and column
    /// statistics that skip individual files. The plan reports what it skipped,
    /// so "a filtered read touches only the files the metadata says it must" is
    /// a number a caller can check.
    ///
    /// # Errors
    ///
    /// Returns an error when a filter names a column the schema does not
    /// declare, or when a manifest that had to be read cannot be reached or
    /// decoded.
    pub fn plan(&self, filters: &[(&str, &str)]) -> Result<ScanPlan> {
        let resolved = super::scan::filters(self.schema()?, filters)?;
        self.planned(&resolved, false)
    }

    /// Plan a scan of one retained snapshot rather than the current one.
    ///
    /// This is the planning half of time travel: the snapshot's manifest list
    /// is walked with the same three-level pruning a current-snapshot plan
    /// uses, so a filtered read of history skips exactly what a filtered read
    /// of the present skips.
    ///
    /// # Errors
    ///
    /// Returns an error when no retained snapshot carries `snapshot_id`, when
    /// a filter names a column the snapshot's schema does not declare, or when
    /// a manifest cannot be reached or decoded.
    pub fn plan_at(&self, snapshot_id: i64, filters: &[(&str, &str)]) -> Result<ScanPlan> {
        let snapshot = self.require_snapshot(snapshot_id)?;
        let schema = self.schema_of(snapshot)?;
        let resolved = super::scan::filters(schema, filters)?;
        let manifests = self.manifests_at(snapshot)?;
        self.plan_manifests(&manifests, &resolved, false)
    }

    /// Read one retained snapshot's rows: time travel as an ordinary scan.
    ///
    /// The rows are read as the schema that was current when the snapshot was
    /// written, so a column added later does not appear and a column dropped
    /// later still does. `filters` and `field` mean exactly what they mean on
    /// [`Self::scan_where`].
    ///
    /// # Errors
    ///
    /// Returns an error when no retained snapshot carries `snapshot_id`, when
    /// a filter names a column that schema does not declare, or when a
    /// manifest cannot be read.
    pub fn scan_at(
        &self,
        snapshot_id: i64,
        filters: &[(&str, &str)],
        field: Option<&Field>,
    ) -> Result<BatchReader> {
        let snapshot = self.require_snapshot(snapshot_id)?;
        let stored = self.schema_of(snapshot)?.clone();
        let resolved = super::scan::filters(&stored, filters)?;
        let manifests = self.manifests_at(snapshot)?;
        let plan = self.plan_manifests(&manifests, &resolved, true)?;
        self.reader(plan.tasks, &stored, field, resolved)
    }

    /// Return one retained snapshot, or say which ids are retained.
    fn require_snapshot(&self, snapshot_id: i64) -> Result<&Snapshot> {
        self.metadata.snapshot_by_id(snapshot_id).ok_or_else(|| {
            let retained: Vec<String> = self
                .metadata
                .snapshots
                .iter()
                .map(|snapshot| snapshot.snapshot_id.to_string())
                .collect();
            invalid(format_smolstr!(
                "expected a retained snapshot id, got {snapshot_id}; the table retains [{}]",
                retained.join(", ")
            ))
        })
    }

    /// Return the schema one snapshot was written under, or the current one.
    fn schema_of(&self, snapshot: &Snapshot) -> Result<&Field> {
        match snapshot.schema_id {
            Some(schema_id) => self.metadata.schema_by_id(schema_id).ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected the snapshot's schema {schema_id} among the table's schemas, got none"
                ))
            }),
            None => self.schema(),
        }
    }

    /// Plan a scan from filters that are already resolved.
    ///
    /// `for_read` marks a plan whose entries only feed a read: those decode
    /// through the manifest planning fast path, which skips the statistics a
    /// scan never consults. A plan whose entries may be carried into a
    /// rewritten manifest - an overwrite, a merge, a compaction - must pass
    /// `false` so every carried entry keeps its statistics whole.
    fn planned(&self, filters: &[Filter], for_read: bool) -> Result<ScanPlan> {
        let manifests = self.manifests()?;
        self.plan_manifests(&manifests, filters, for_read)
    }

    /// Plan one set of manifests under one set of resolved filters.
    fn plan_manifests(
        &self,
        manifests: &[ManifestFile],
        filters: &[Filter],
        for_read: bool,
    ) -> Result<ScanPlan> {
        super::scan::plan(
            manifests,
            &|spec_id| {
                self.metadata
                    .spec_by_id(spec_id)
                    .cloned()
                    .unwrap_or_else(PartitionSpec::unpartitioned)
            },
            &|location| self.child_at(location),
            filters,
            for_read,
        )
    }

    /// Return every live data file of the current snapshot, with its spec.
    ///
    /// # Errors
    ///
    /// Returns an error when a manifest cannot be reached or decoded. A
    /// manifest naming a file that is not there is *not* an error here: a scan
    /// reports that, because a missing file is a read failure and not a
    /// metadata failure.
    pub fn data_files(&self) -> Result<Vec<(DataFile, PartitionSpec)>> {
        Ok(self
            .plan(&[])?
            .tasks
            .into_iter()
            .map(|task| (task.entry.data_file, task.spec))
            .collect())
    }

    /// Commit a metadata-only change as the next table version.
    ///
    /// `change` receives the metadata to mutate - table properties, a new
    /// schema from [`TableMetadata::add_schema`], a snapshot ref - and the
    /// result is written as one new metadata document, exactly as a data
    /// commit writes one. An error from the change, or from the write, leaves
    /// the table's in-memory state exactly as it was: a failed commit is a
    /// commit that never happened.
    ///
    /// A commit that finds another writer already published the version it
    /// meant to write *rebases*: it reloads the winner's document and runs
    /// `change` again on it - which is why the closure is `FnMut` - retrying
    /// with jittered exponential backoff up to
    /// [`IcebergOptions::commit_retries`] times, and reporting a
    /// [`CommitConflict`] when the retries run out. The check is best-effort
    /// on plain storage - [`IOBase`] has no compare-and-swap, so retries
    /// shrink the undetected-race window without closing it.
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let folder = yggdryl::local::Folder::new(std::env::temp_dir().join("t"))?;
    /// # let mut table = yggdryl::iceberg::Table::open(folder)?;
    /// table.commit_changes(|metadata| {
    ///     metadata.set_property("commit.retry.num-retries", "4")?;
    ///     Ok(())
    /// })?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the change's own failure, the write failure of the new
    /// document, or a [`CommitConflict`] when concurrent writers exhausted the
    /// retries.
    pub fn commit_changes(
        &mut self,
        mut change: impl FnMut(&mut TableMetadata) -> Result<()>,
    ) -> Result<()> {
        self.commit_document(OnConflict::Rebase, move |table| {
            // The change runs on a copy, so a rejected change costs nothing.
            let mut updated = table.metadata.clone();
            change(&mut updated)?;
            updated.last_updated_ms = now_ms();
            Ok(updated)
        })
    }

    /// Write one prepared document as the next version, retrying when beaten.
    ///
    /// This is the one gate every commit goes through. Each attempt re-checks
    /// the current version with [`find_metadata`]; a newer version than this
    /// handle's counts as being beaten once. What happens next is
    /// `on_conflict`'s: [`OnConflict::Rebase`] adopts the winner's document so
    /// `apply` re-runs on it, while [`OnConflict::Fail`] only waits and looks
    /// again, because version numbers never move backwards and the caller said
    /// re-applying is unsafe - its attempts exist to bound the wait and to
    /// count an honest report. Being beaten more than
    /// [`IcebergOptions::commit_retries`] times restores the in-memory state
    /// and returns a [`CommitConflict`].
    ///
    /// The check-then-write pair is not atomic - [`IOBase`] has no
    /// compare-and-swap - so a writer landing between the two still goes
    /// undetected; the module docs say so plainly.
    fn commit_document(
        &mut self,
        on_conflict: OnConflict,
        mut apply: impl FnMut(&Self) -> Result<TableMetadata>,
    ) -> Result<()> {
        let settings = IcebergOptions::commit_settings(self.options.as_ref(), &self.metadata)?;
        let saved_metadata = self.metadata.clone();
        let saved_version = self.version;
        let restore = |table: &mut Self, error: Error| {
            table.metadata = saved_metadata.clone();
            table.version = saved_version;
            Err(error)
        };

        let metadata_dir = self.root.child_by(METADATA_DIR)?;
        let mut beaten: u32 = 0;
        loop {
            match find_metadata(&metadata_dir) {
                Ok(Some((version, document))) if version > self.version => {
                    beaten += 1;
                    if beaten > settings.retries {
                        let conflict = CommitConflict {
                            expected_version: saved_version + 1,
                            beaten,
                            last_seen_version: version,
                        };
                        return restore(self, conflict.into());
                    }
                    if on_conflict == OnConflict::Rebase {
                        match TableMetadata::from_json(&document) {
                            Ok(fresh) => {
                                self.metadata = fresh;
                                self.version = version;
                            }
                            Err(error) => return restore(self, error),
                        }
                    }
                    let wait =
                        backoff_ms(beaten - 1, settings.min_backoff_ms, settings.max_backoff_ms);
                    if wait > 0 {
                        std::thread::sleep(std::time::Duration::from_millis(wait));
                    }
                    continue;
                }
                Ok(_) => {}
                Err(error) => return restore(self, error),
            }

            let updated = match apply(self) {
                Ok(updated) => updated,
                Err(error) => return restore(self, error),
            };
            self.metadata = updated;
            if let Err(error) = self.commit_metadata() {
                return restore(self, error);
            }
            return Ok(());
        }
    }

    /// Render when each snapshot became current, oldest first.
    ///
    /// The columns are `made_current_at`, `snapshot_id`, `parent_id`, and
    /// `is_current_ancestor`, the names PyIceberg's `history` table uses.
    ///
    /// # Errors
    ///
    /// Returns an error only when the batch cannot be assembled.
    pub fn inspect_history(&self) -> Result<BatchReader> {
        super::inspect::history(&self.metadata)
    }

    /// Render every retained snapshot with its operation and summary.
    ///
    /// The columns are `committed_at`, `snapshot_id`, `parent_id`,
    /// `operation`, `manifest_list`, and the free-form `summary` map.
    ///
    /// # Errors
    ///
    /// Returns an error only when the batch cannot be assembled.
    pub fn inspect_snapshots(&self) -> Result<BatchReader> {
        super::inspect::snapshots(&self.metadata)
    }

    /// Render the live data files of the current snapshot.
    ///
    /// The columns are `file_path`, `file_format`, `spec_id`, the rendered
    /// `partition` chain, `record_count`, and `file_size_in_bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error when a manifest cannot be reached or decoded.
    pub fn inspect_files(&self) -> Result<BatchReader> {
        let entries = self.data_files()?;
        super::inspect::files(&entries)
    }

    /// Read every row of the current snapshot, keeping the columns `field` names.
    ///
    /// # Errors
    ///
    /// Returns an error when a manifest cannot be read or the scan root cannot
    /// be projected.
    pub fn scan(&self, field: Option<&Field>) -> Result<BatchReader> {
        self.scan_where(&[], field)
    }

    /// Read the rows matching `filters`, keeping the columns `field` names.
    ///
    /// Each data file is read through [`IOBase::read_arrow_batch_reader`] with
    /// the scan root as its declared schema, so a projected scan skips the
    /// column chunks it does not want rather than reading and discarding them.
    /// What each file yields is then cast to the scan's own root, which is what
    /// makes a table whose schema evolved readable as one shape: a file written
    /// before a column existed contributes null for it.
    ///
    /// A partition column the data file does not store is restored from the
    /// manifest's partition tuple, typed as the schema declares it. The
    /// manifest is the authority rather than the directory name, because a null
    /// partition value is spelled `null` in a path and a path cannot say
    /// whether that is the string or the absence.
    ///
    /// A filter on a partition column is answered by [`Self::plan`] alone -
    /// every row of a file whose tuple matches holds that value - and a filter
    /// on any other column is applied to the rows the surviving files hold,
    /// because statistics bound a file rather than select a row.
    ///
    /// # Errors
    ///
    /// Returns an error when a filter names a column the schema does not
    /// declare, when a manifest cannot be read, or when the scan root cannot be
    /// projected.
    pub fn scan_where(
        &self,
        filters: &[(&str, &str)],
        field: Option<&Field>,
    ) -> Result<BatchReader> {
        let stored = self.schema()?.clone();
        let resolved = super::scan::filters(&stored, filters)?;
        let plan = self.planned(&resolved, true)?;
        self.reader(plan.tasks, &stored, field, resolved)
    }

    /// Build the reader over one set of planned files.
    fn reader(
        &self,
        tasks: Vec<ScanTask>,
        stored: &Field,
        field: Option<&Field>,
        filters: Vec<Filter>,
    ) -> Result<BatchReader> {
        let root = field.map_or_else(|| stored.clone(), Clone::clone);
        let read_root = super::scan::read_root(&root, stored, &filters)?;

        let mut parts = Vec::new();
        for task in tasks {
            let handle = self.child_at(&task.entry.data_file.file_path)?;
            parts.push(ScanPart {
                handle,
                size: task.entry.data_file.file_size_in_bytes,
                partition: super::scan::partition_columns(
                    &task.spec,
                    stored,
                    &task.entry.data_file,
                )?,
                residual: task.residual,
            });
        }
        let parallel = IcebergOptions::read_settings(self.options.as_ref(), &self.metadata)?;
        super::scan::reader(parts, root, read_root, field.cloned(), filters, &parallel)
    }

    /// Append `batches` as a new snapshot, keeping everything already stored.
    ///
    /// An append beaten by a concurrent commit *rebases*: the data files are
    /// already written, so only the manifest list and the document are rebuilt
    /// on the winner's metadata - fresh parent, fresh sequence number - with
    /// backoff between attempts, up to [`IcebergOptions::commit_retries`]
    /// times. The version check is best-effort on plain storage: [`IOBase`]
    /// has no compare-and-swap, so retries shrink the undetected-race window
    /// without closing it, and serialized writers are what closes it.
    ///
    /// # Errors
    ///
    /// Returns an error when the partition spec cannot place a row, when a
    /// batch cannot be cast to the table schema, when any write fails, or a
    /// [`CommitConflict`] when concurrent writers exhausted the retries.
    pub fn append(&mut self, batches: BatchReader) -> Result<()> {
        self.commit(batches, "append", Retained::All)?;
        Ok(())
    }

    /// Replace every row with `batches` as a new snapshot.
    ///
    /// The previous snapshot is retained and still readable; only the current
    /// pointer moves, which is what makes an overwrite reversible.
    ///
    /// # Errors
    ///
    /// Returns an error when the partition spec cannot place a row, when a
    /// batch cannot be cast to the table schema, or when any write fails.
    pub fn overwrite(&mut self, batches: BatchReader) -> Result<()> {
        self.overwrite_where(&[], batches)
    }

    /// Replace only the rows `filters` selects, keeping every other file.
    ///
    /// A file the filters exclude is carried into the new snapshot exactly as
    /// it is - the same location, the same statistics, the commit order it was
    /// written with - so overwriting one partition of a thousand rewrites one
    /// partition. A manifest the summaries excluded outright is not even
    /// rewritten: it stays in the manifest list as it was.
    ///
    /// An overwrite beaten by a concurrent commit cannot rebase: what it keeps
    /// was planned against a snapshot the winner may have replaced, and the
    /// incoming reader is already consumed, so re-planning could lose the
    /// winner's rows or double this write's. It reports a [`CommitConflict`]
    /// naming both versions instead, after the bounded waits of
    /// [`IcebergOptions::commit_retries`]; the caller re-reads and retries
    /// with fresh input.
    ///
    /// # Errors
    ///
    /// Returns an error when a filter names a column the schema does not
    /// declare, when the partition spec cannot place a row, when any read or
    /// write fails, or a [`CommitConflict`] when a concurrent commit won.
    pub fn overwrite_where(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
    ) -> Result<()> {
        let plan = self.plan(filters)?;
        self.commit(
            batches,
            "overwrite",
            Retained::Only {
                manifests: plan.skipped,
                entries: plan.excluded,
            },
        )?;
        Ok(())
    }

    /// Merge `batches` into the stored rows, matching on the `merge_by_names` columns.
    ///
    /// # Errors
    ///
    /// Returns the failure of the read, the join, or the commit.
    pub fn merge(
        &mut self,
        batches: BatchReader,
        merge_by_names: &[String],
        safe: bool,
    ) -> Result<()> {
        self.merge_where(&[], batches, merge_by_names, safe)
    }

    /// Merge `batches` into the rows `filters` selects, on the `merge_by_names` columns.
    ///
    /// This is the one place the *column statistics* decide what is read. A row
    /// can only update a file whose recorded bounds for every match-key column
    /// contain one of the incoming keys, so the files whose bounds cannot are
    /// neither read nor rewritten: they are carried into the new snapshot
    /// untouched. That makes an upsert cost the files it can actually change
    /// rather than the whole table, and it stays correct however coarse the
    /// statistics are, because a file that is not read keeps every row it had.
    ///
    /// Like [`Self::overwrite_where`], a merge beaten by a concurrent commit
    /// reports a [`CommitConflict`] rather than rebasing, because the files it
    /// selected and the reader it consumed cannot be re-planned safely.
    ///
    /// # Errors
    ///
    /// Returns an error when `merge_by_names` names a column the schema does not
    /// declare, and the failure of any read, join, or write otherwise,
    /// including a [`CommitConflict`] when a concurrent commit won.
    pub fn merge_where(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        merge_by_names: &[String],
        safe: bool,
    ) -> Result<()> {
        if merge_by_names.is_empty() {
            return self.overwrite_where(filters, batches);
        }
        let schema = self.schema()?.clone();

        // The incoming side is held, and this is why: the files a merge has to
        // read are the ones whose statistics say they can hold an incoming key,
        // and a key range cannot be taken from a reader that has not been read.
        // The stored side is what that buys - only the files that can actually
        // change are decoded - so the whole table is never in memory even when
        // the write is not streamed.
        let mut incoming = Vec::new();
        for batch in batches {
            let batch = schema.cast_arrow_batch(batch.map_err(Error::Arrow)?, safe)?;
            if batch.num_rows() > 0 {
                incoming.push(batch);
            }
        }
        let bounds = KeyBounds::of(&incoming, &schema, merge_by_names)?;

        let plan = self.plan(filters)?;
        let mut selected = Vec::new();
        let mut carried = plan.excluded;
        for task in plan.tasks {
            if bounds.may_hold(&task.entry.data_file) {
                selected.push(task);
            } else {
                carried.push(task);
            }
        }

        let stored = self.reader(selected, &schema, None, Vec::new())?;
        let arrow_schema = crate::arrow::schema_from_field(&schema)?;
        let merged = crate::io::merge::merged(
            stored,
            crate::arrow::batch_reader(arrow_schema, incoming),
            &schema,
            merge_by_names,
            safe,
        )?;
        self.commit(
            merged,
            "overwrite",
            Retained::Only {
                manifests: plan.skipped,
                entries: carried,
            },
        )?;
        Ok(())
    }

    /// Merge the current snapshot's undersized data files, one partition at a time.
    ///
    /// The live files are grouped by spec and partition tuple - a data file
    /// belongs to exactly one partition, so files of different partitions are
    /// never merged into one - and a group is rewritten when it holds at least
    /// two files and at least one of them is smaller than
    /// [`Self::target_file_size`]. The rewritten rows go through the same
    /// rolling writer an append uses, so a compacted partition lands in files
    /// of roughly the target size, and every file of every other group is
    /// carried into the new snapshot untouched: same location, same
    /// statistics, same commit order.
    ///
    /// The commit is one `replace` snapshot, so the pre-compaction snapshot
    /// stays retained and [`Self::scan_at`] still reads exactly the rows it
    /// always read. A table with nothing to compact is left exactly as it is:
    /// no snapshot is committed and the returned `Compaction` is all zeros.
    ///
    /// # Errors
    ///
    /// Returns an error when the target size is configured but unparseable,
    /// when a manifest cannot be read, or when any read or write of the
    /// rewrite fails.
    pub fn compact(&mut self) -> Result<Compaction> {
        let target = i64::try_from(self.target_file_size()?).unwrap_or(i64::MAX);
        let plan = self.plan(&[])?;

        // Group the live files by (spec, partition tuple), in plan order.
        let mut groups: Vec<(i32, Vec<Value>, Vec<ScanTask>)> = Vec::new();
        for task in plan.tasks {
            match groups.iter_mut().find(|(spec_id, partition, _)| {
                *spec_id == task.spec.spec_id && *partition == task.entry.data_file.partition
            }) {
                Some((_, _, tasks)) => tasks.push(task),
                None => {
                    let partition = task.entry.data_file.partition.clone();
                    groups.push((task.spec.spec_id, partition, vec![task]));
                }
            }
        }

        let mut selected: Vec<ScanTask> = Vec::new();
        let mut carried = plan.excluded;
        for (_, _, tasks) in groups {
            let undersized = tasks
                .iter()
                .any(|task| task.entry.data_file.file_size_in_bytes < target);
            if tasks.len() >= 2 && undersized {
                selected.extend(tasks);
            } else {
                carried.extend(tasks);
            }
        }

        // Nothing qualifies, so nothing is committed: a snapshot that changes
        // no file would still cost a manifest, a list, and a document.
        if selected.is_empty() {
            return Ok(Compaction::default());
        }

        let files_before = selected.len();
        let bytes_rewritten: i64 = selected
            .iter()
            .map(|task| task.entry.data_file.file_size_in_bytes)
            .sum();

        let schema = self.schema()?.clone();
        let rows = self.reader(selected, &schema, None, Vec::new())?;
        let files_after = self.commit(
            rows,
            "replace",
            Retained::Only {
                manifests: plan.skipped,
                entries: carried,
            },
        )?;
        Ok(Compaction {
            files_before,
            files_after,
            bytes_rewritten,
        })
    }

    /// Add a schema and make it current, then write a new metadata document.
    ///
    /// Returns the new schema's identifier. Data written under the previous
    /// schema stays readable: [`Self::scan`] casts every file to the scan root,
    /// so a column added here reads as null in the files that predate it.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema is not a non-null struct root or the
    /// metadata document cannot be written.
    pub fn evolve_schema(&mut self, schema: Field) -> Result<i32> {
        // Committing through the one retrying gate means a beaten evolution
        // renumbers itself against the winner's `last-column-id`.
        let mut schema_id = 0;
        self.commit_changes(|metadata| {
            schema_id = metadata.add_schema(schema.clone())?;
            metadata.current_schema_id = schema_id;
            Ok(())
        })?;
        Ok(schema_id)
    }

    /// Create a branch at one retained snapshot, as one metadata commit.
    ///
    /// This is [`TableMetadata::create_branch`] committed through the
    /// retrying [`Self::commit_changes`]. Writing *to* a branch other than
    /// `main` remains future work - a commit's parent is currently always the
    /// current snapshot - so a branch is read with [`Self::scan_ref`] and
    /// moved with [`Self::fast_forward`].
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken or reserved, when the snapshot
    /// is not retained, or when the commit fails.
    pub fn create_branch(&mut self, name: &str, snapshot_id: i64) -> Result<()> {
        self.commit_changes(|metadata| metadata.create_branch(SmolStr::new(name), snapshot_id))
    }

    /// Create a tag at one retained snapshot, as one metadata commit.
    ///
    /// This is [`TableMetadata::create_tag`] committed through the retrying
    /// [`Self::commit_changes`].
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken or reserved, when the snapshot
    /// is not retained, or when the commit fails.
    pub fn create_tag(&mut self, name: &str, snapshot_id: i64) -> Result<()> {
        self.commit_changes(|metadata| metadata.create_tag(SmolStr::new(name), snapshot_id))
    }

    /// Remove one branch or tag, as one metadata commit.
    ///
    /// Returns the reference that was removed. This is
    /// [`TableMetadata::remove_snapshot_ref`] committed through the retrying
    /// [`Self::commit_changes`]; a name the table does not have is an error
    /// rather than an empty commit.
    ///
    /// # Errors
    ///
    /// Returns an error naming the refs the table does have when `name` is
    /// not one of them, or when the commit fails.
    pub fn remove_ref(&mut self, name: &str) -> Result<SnapshotRef> {
        let mut removed = None;
        self.commit_changes(|metadata| match metadata.remove_snapshot_ref(name) {
            Some(reference) => {
                removed = Some(reference);
                Ok(())
            }
            None => {
                let known: Vec<&str> = metadata
                    .refs
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();
                Err(invalid(format_smolstr!(
                    "expected a branch or tag this table has, got {name:?}; it has [{}]",
                    known.join(", ")
                )))
            }
        })?;
        removed.ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected the committed removal to record the ref, got none",
            ))
        })
    }

    /// Move a branch forward to a descendant snapshot, as one metadata commit.
    ///
    /// This is [`TableMetadata::fast_forward_branch`] committed through the
    /// retrying [`Self::commit_changes`]: the target must be retained and must
    /// reach the branch's head by walking parent ids, so a fast-forward can
    /// never lose history.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not a branch, the target is not
    /// retained or not a descendant, or the commit fails.
    pub fn fast_forward(&mut self, name: &str, snapshot_id: i64) -> Result<()> {
        self.commit_changes(|metadata| metadata.fast_forward_branch(name, snapshot_id))
    }

    /// Expire the snapshots retention no longer keeps, as one metadata commit.
    ///
    /// `older_than_ms` is the default age cutoff
    /// [`TableMetadata::expire_snapshots_older_than`] applies; every ref's own
    /// retention fields are honored first. Returns the expired snapshot ids,
    /// sorted. A table with nothing old commits nothing - the check runs on a
    /// copy first, so an empty expiry costs no version.
    ///
    /// # Errors
    ///
    /// Returns the expiry's own failure, or the commit failure.
    pub fn expire_snapshots(&mut self, older_than_ms: i64) -> Result<Vec<i64>> {
        let mut probe = self.metadata.clone();
        if probe.expire_snapshots_older_than(older_than_ms)?.is_empty() {
            return Ok(Vec::new());
        }
        let mut expired = Vec::new();
        self.commit_changes(|metadata| {
            expired = metadata.expire_snapshots_older_than(older_than_ms)?;
            Ok(())
        })?;
        Ok(expired)
    }

    /// Read the rows a branch or tag names: [`Self::snapshot_by_ref`] plus
    /// [`Self::scan_at`], with the same `filters` and `field` meanings.
    ///
    /// # Errors
    ///
    /// Returns an error naming the refs the table does have when `name` is
    /// not one of them, and any [`Self::scan_at`] failure otherwise.
    pub fn scan_ref(
        &self,
        name: &str,
        filters: &[(&str, &str)],
        field: Option<&Field>,
    ) -> Result<BatchReader> {
        let snapshot_id = self.snapshot_by_ref(name)?.snapshot_id;
        self.scan_at(snapshot_id, filters, field)
    }

    /// Resolve one recorded location into a child of the table's folder.
    ///
    /// Everything a table names is inside it, so a location is turned back into
    /// a relative name and resolved with [`IOBase::child_by`]. That is what
    /// keeps this module free of path handling: the backend decides what a
    /// child is, and a table written on one storage system moves to another by
    /// rewriting its locations rather than its code.
    pub(super) fn child_at(&self, location: &str) -> Result<Holder> {
        let relative = relative_location(&self.metadata.location, location)?;
        self.root.child_by(&relative)
    }

    /// Write the current metadata as the next numbered document.
    fn commit_metadata(&mut self) -> Result<()> {
        // A bad in-memory state is refused before a document exists, so a
        // broken table can only be read, never written.
        self.metadata.validate()?;
        let previous = (self.version > 0)
            .then(|| self.metadata_location())
            .transpose()?;
        self.version += 1;
        if let Some(previous) = previous {
            self.metadata
                .metadata_log
                .push((self.metadata.last_updated_ms, SmolStr::new(previous)));
        }

        let document = self.metadata.to_json()?;
        let encoded = crate::json::to_vec(&document)?;
        let name = self.metadata_file_name();
        let mut handle = self.root.child_by(&format!("{METADATA_DIR}/{name}"))?;
        handle.write_all_bytes(&encoded)?;

        // The hint is how a catalog-free reader finds the current document.
        let mut hint = self
            .root
            .child_by(&format!("{METADATA_DIR}/{VERSION_HINT}"))?;
        hint.write_all_bytes(self.version.to_string().as_bytes())
    }

    /// Write the data files, the manifest, the manifest list, and the metadata.
    ///
    /// Returns how many data files the commit wrote. Each partition group's
    /// rows are rolled into files of roughly [`Self::target_file_size`] bytes,
    /// and one running index numbers every file of the commit.
    ///
    /// The expensive half - decoding the consumed reader into data files and
    /// their one manifest - happens exactly once. Only the manifest list and
    /// the document are rebuilt per retry attempt, because they are what carry
    /// the parent snapshot and the sequence number a rebase changes. A commit
    /// keeping [`Retained::All`] rebases; one keeping [`Retained::Only`]
    /// conflicts instead, because what it keeps was planned against a snapshot
    /// a concurrent commit may have replaced.
    fn commit(
        &mut self,
        batches: BatchReader,
        operation: &str,
        retained: Retained,
    ) -> Result<usize> {
        let schema = self.schema()?.clone();
        let spec = self.metadata.default_spec()?.clone();
        spec.require_writable()?;
        let target = self.target_file_size()?;

        let snapshot_id = snapshot_id();
        let partition = spec.partition_field(&schema)?;
        let sources = spec.source_names(&schema)?;

        let mut written = Vec::new();
        for (values, group) in grouped_batches(batches, &schema, &spec, &sources, &partition)? {
            for file in rolled(group, target) {
                written.push(self.write_data_file(
                    written.len(),
                    snapshot_id,
                    &schema,
                    &spec,
                    &values,
                    file,
                )?);
            }
        }
        let files_written = written.len();

        let added_records: i64 = written.iter().map(|file| file.record_count).sum();
        let added_size: i64 = written.iter().map(|file| file.file_size_in_bytes).sum();
        let added_files = i32::try_from(written.len()).unwrap_or(i32::MAX);

        // The new manifest holds only `added` entries, whose snapshot and
        // sequence numbers are inherited from the manifest list row, so its
        // bytes are attempt-invariant and it is written once; the row's
        // numbers are filled in per attempt below.
        let new_manifest = if written.is_empty() {
            None
        } else {
            let entries: Vec<ManifestEntry> = written
                .into_iter()
                .map(|file| ManifestEntry::added(snapshot_id, file))
                .collect();
            let mut manifest = self.write_manifest_file(
                &format!("{snapshot_id}-m0.avro"),
                &schema,
                &spec,
                &entries,
                snapshot_id,
                self.metadata.last_sequence_number + 1,
            )?;
            manifest.added_files_count = added_files;
            manifest.added_rows_count = added_records;
            Some(manifest)
        };

        // What the commit keeps. `All` re-reads the live manifests per
        // attempt, because a rebase changes what "all" means; `Only` was
        // planned against this exact snapshot, so a conflict is final.
        let (on_conflict, kept) = match retained {
            Retained::All => (OnConflict::Rebase, None),
            Retained::Only { manifests, entries } => {
                let mut kept = manifests;
                kept.extend(self.carried_manifests(
                    &entries,
                    &schema,
                    snapshot_id,
                    self.metadata.last_sequence_number + 1,
                )?);
                (OnConflict::Fail, Some(kept))
            }
        };

        let operation = SmolStr::new(operation);
        let compacting = operation == "replace";
        self.commit_document(on_conflict, move |table| {
            let sequence_number = table.metadata.last_sequence_number + 1;
            let mut manifests = match &kept {
                Some(kept) => kept.clone(),
                None => table.manifests()?,
            };
            if let Some(manifest) = &new_manifest {
                let mut row = manifest.clone();
                row.sequence_number = sequence_number;
                // Every entry in it is `added`, so the floor is this commit's.
                row.min_sequence_number = sequence_number;
                manifests.push(row);
            }

            let list_name = format!("snap-{snapshot_id}-1-{}.avro", uuid());
            let mut list = table
                .root
                .child_by(&format!("{METADATA_DIR}/{list_name}"))?;
            write_manifest_list(
                &mut list,
                table.metadata.format_version,
                snapshot_id,
                table.metadata.current_snapshot_id,
                sequence_number,
                &manifests,
            )?;

            let total_records: i64 = manifests
                .iter()
                .map(|manifest| manifest.added_rows_count + manifest.existing_rows_count)
                .sum();
            let total_files: i32 = manifests
                .iter()
                .map(|manifest| manifest.added_files_count + manifest.existing_files_count)
                .sum();

            let snapshot = Snapshot {
                snapshot_id,
                parent_snapshot_id: table.metadata.current_snapshot_id,
                sequence_number: (table.metadata.format_version >= FormatVersion::V2)
                    .then_some(sequence_number),
                timestamp_ms: now_ms(),
                manifest_list: SmolStr::new(table.location_of(METADATA_DIR, &list_name)),
                summary: vec![
                    (
                        SmolStr::new_static("operation"),
                        SmolStr::new(operation.clone()),
                    ),
                    (
                        SmolStr::new_static("added-data-files"),
                        format_smolstr!("{added_files}"),
                    ),
                    (
                        SmolStr::new_static("added-records"),
                        format_smolstr!("{added_records}"),
                    ),
                    (
                        SmolStr::new_static("added-files-size"),
                        format_smolstr!("{added_size}"),
                    ),
                    (
                        SmolStr::new_static("total-data-files"),
                        format_smolstr!("{total_files}"),
                    ),
                    (
                        SmolStr::new_static("total-records"),
                        format_smolstr!("{total_records}"),
                    ),
                ],
                schema_id: Some(table.metadata.current_schema_id),
                first_row_id: (table.metadata.format_version >= FormatVersion::V3)
                    .then(|| table.metadata.next_row_id.unwrap_or_default()),
                added_rows: (table.metadata.format_version >= FormatVersion::V3)
                    .then_some(added_records),
            };

            let mut updated = table.metadata.clone();
            if updated.format_version >= FormatVersion::V3 {
                updated.next_row_id = Some(updated.next_row_id.unwrap_or_default() + added_records);
            }
            updated.set_current_snapshot(snapshot);
            Ok(updated)
        })?;
        self.maybe_auto_compact(compacting)?;
        Ok(files_written)
    }

    /// Run the configured compaction cadence after a data commit.
    ///
    /// [`IcebergOptions::compact_after_commits`] paces this: after every `n`
    /// data commits the undersized files fold together, so no single commit
    /// pays for a full rewrite and no scan pays for hundreds of small files.
    /// A compaction itself commits `replace`, which is what the count runs
    /// from, so the cadence cannot recurse. A beaten compaction is ignored -
    /// a concurrent writer's success is not this commit's failure, and the
    /// next cadence point retries what this one left - while any other
    /// failure surfaces, because the data commit already stands either way.
    fn maybe_auto_compact(&mut self, compacting: bool) -> Result<()> {
        if compacting {
            return Ok(());
        }
        let Some(cadence) = IcebergOptions::resolved(self.options.as_ref(), &self.metadata)?
            .compact_after_commits()
        else {
            return Ok(());
        };
        let mut since_replace: u32 = 0;
        for snapshot in self.metadata.snapshots.iter().rev() {
            if snapshot.operation() == "replace" {
                break;
            }
            since_replace = since_replace.saturating_add(1);
        }
        if since_replace < cadence {
            return Ok(());
        }
        match self.compact() {
            Ok(_) => Ok(()),
            // A CommitConflict reaches `Error` through exactly one From impl,
            // so its display is the marker; a beaten compaction retries at
            // the next cadence point rather than failing the data commit.
            Err(error) if error.to_string().contains("got beaten") => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Write one partition's rows as a Parquet data file and describe it.
    fn write_data_file(
        &self,
        index: usize,
        snapshot_id: i64,
        schema: &Field,
        spec: &PartitionSpec,
        values: &[Value],
        batches: Vec<RecordBatch>,
    ) -> Result<DataFile> {
        let directory = spec.partition_path(values)?;
        let name = format!("{index:05}-{snapshot_id}-{}.parquet", uuid());
        let relative = if directory.is_empty() {
            format!("{DATA_DIR}/{name}")
        } else {
            format!("{DATA_DIR}/{directory}/{name}")
        };

        let mut handle = self.root.child_by(&relative)?;
        let options = handle
            .record_options()?
            .with_safe(false)
            .with_schema(schema.clone());
        let arrow_schema = crate::arrow::schema_from_field(schema)?;
        handle.write_arrow_batch_reader(
            crate::arrow::batch_reader(arrow_schema, batches),
            &options,
        )?;
        handle.flush()?;

        let statistics = crate::parquet::read_statistics(&handle)?;
        let mut file = super::statistics::data_file(schema, &statistics)?;
        file.file_path = SmolStr::new(self.location_of(DATA_DIR, &{
            if directory.is_empty() {
                name.clone()
            } else {
                format!("{directory}/{name}")
            }
        }));
        file.file_format = FileFormat::Parquet;
        file.file_size_in_bytes = i64::try_from(handle.size()).unwrap_or_default();
        file.partition = values.to_vec();
        Ok(file)
    }

    /// Write one manifest and describe it as a manifest list row.
    ///
    /// The counts a caller cares about are filled in afterwards, because what
    /// makes an entry added or existing is the commit's business rather than
    /// this write's.
    fn write_manifest_file(
        &self,
        name: &str,
        schema: &Field,
        spec: &PartitionSpec,
        entries: &[ManifestEntry],
        snapshot_id: i64,
        sequence_number: i64,
    ) -> Result<ManifestFile> {
        let mut handle = self.root.child_by(&format!("{METADATA_DIR}/{name}"))?;
        write_manifest(
            &mut handle,
            self.metadata.format_version,
            schema,
            spec,
            entries,
        )?;
        handle.flush()?;
        Ok(ManifestFile {
            manifest_path: SmolStr::new(self.location_of(METADATA_DIR, name)),
            manifest_length: i64::try_from(handle.size()).unwrap_or_default(),
            partition_spec_id: spec.spec_id,
            content: ManifestContent::Data,
            sequence_number,
            // A carried entry keeps the order it was written with, so the
            // manifest's floor is the oldest entry in it rather than this
            // commit's own number.
            min_sequence_number: entries
                .iter()
                .filter_map(|entry| entry.sequence_number)
                .min()
                .unwrap_or(sequence_number)
                .min(sequence_number),
            added_snapshot_id: snapshot_id,
            added_files_count: 0,
            existing_files_count: 0,
            deleted_files_count: 0,
            added_rows_count: 0,
            existing_rows_count: 0,
            deleted_rows_count: 0,
            partitions: summaries(spec, schema, entries)?,
            first_row_id: None,
        })
    }

    /// Rewrite the files a commit keeps as existing entries, one manifest per spec.
    ///
    /// A manifest's `partition` column has the shape of one spec, so files
    /// written under two specs cannot share a manifest however few of them there
    /// are.
    fn carried_manifests(
        &self,
        tasks: &[ScanTask],
        schema: &Field,
        snapshot_id: i64,
        sequence_number: i64,
    ) -> Result<Vec<ManifestFile>> {
        let mut grouped: Vec<(PartitionSpec, Vec<ManifestEntry>)> = Vec::new();
        for task in tasks {
            match grouped
                .iter_mut()
                .find(|(spec, _)| spec.spec_id == task.spec.spec_id)
            {
                Some((_, entries)) => entries.push(task.entry.existing()),
                None => grouped.push((task.spec.clone(), vec![task.entry.existing()])),
            }
        }

        let mut manifests = Vec::with_capacity(grouped.len());
        for (index, (spec, entries)) in grouped.into_iter().enumerate() {
            let name = format!("{snapshot_id}-m{}.avro", index + 1);
            let mut manifest = self.write_manifest_file(
                &name,
                schema,
                &spec,
                &entries,
                snapshot_id,
                sequence_number,
            )?;
            manifest.existing_files_count = i32::try_from(entries.len()).unwrap_or(i32::MAX);
            manifest.existing_rows_count = entries
                .iter()
                .map(|entry| entry.data_file.record_count)
                .sum();
            manifests.push(manifest);
        }
        Ok(manifests)
    }

    /// Build the URI of one child of a table directory.
    fn location_of(&self, directory: &str, name: &str) -> String {
        format!(
            "{}/{directory}/{name}",
            self.metadata.location.trim_end_matches('/')
        )
    }
}

/// The table is itself a handle: the byte surface is the folder it lives in,
/// and the record surface is answered from the parsed metadata this value
/// already holds.
///
/// A plain container handle addressing the table's folder answers the same
/// contract - the three record methods, one commit per write - by probing the
/// location for a table on every call. Holding the [`Table`] skips the probe:
/// no metadata document is re-read, [`IOBase::read_arrow_field`] is
/// [`Table::schema`] with its field identifiers and protocol metadata rather
/// than a shape lifted off decoded batches, and a
/// [`filter_partitions`](IORecordOptions::filter_partitions) pair prunes data
/// files through [`Table::plan`] instead of filtering rows after they were
/// decoded. The in-memory metadata stays current across commits, so
/// [`Table::current_snapshot`] and [`Table::version`] reflect a write made
/// through this surface without reopening anything.
///
/// One deliberate difference from the folder route: a filter naming a column
/// the schema does not declare is an error here, exactly as
/// [`Table::scan_where`] reports it, where a folder of leaves ignores a column
/// its batches do not carry. A table's schema is authoritative, so a filter it
/// cannot answer is a mistake worth naming rather than a row set worth
/// guessing.
impl<H: IOBase> IOBase for Table<H> {
    crate::delegate_iobase!(root);

    /// The encoding of this table's data files, from metadata alone.
    ///
    /// A table that has never been written to holds no data file to read a
    /// media type off, and its encoding is still not a guess: this module
    /// writes Parquet, so that is what an Iceberg table's rows are.
    fn record_options(&self) -> Result<RecordOptions> {
        Ok(RecordOptions::Parquet(crate::parquet::ParquetOptions::new()))
    }

    /// The stored schema as the metadata declares it, no data file opened.
    ///
    /// A declared schema is returned as it stands, as on every handle.
    /// Otherwise the answer is [`Table::schema`] renamed to the options' root
    /// name - field identifiers and protocol metadata included - where the
    /// base implementation would build a reader and take the shape off its
    /// batches.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        if let Some(schema) = options.schema() {
            return Ok(schema.clone());
        }
        Ok(self.schema()?.clone().with_name(options.root_name()))
    }

    /// Scan the current snapshot, the options' filters answered by the plan.
    fn read_arrow_batch_reader(&self, options: &RecordOptions) -> Result<BatchReader> {
        let filters = options.filter_partitions();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        let reader = self.scan_where(&pairs, options.schema())?;
        crate::io::select_reader(reader, options)
    }

    /// One commit: an overwrite without a match key, a merge with one, scoped
    /// to the partitions the options' filters select.
    fn write_arrow_batch_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        let batches = crate::io::select_reader(batches, options)?;
        let filters: Vec<(String, String)> = options.filter_partitions().to_vec();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.merge_where(&pairs, batches, options.merge_by_names(), options.safe())
    }

    /// One `append` snapshot, keeping every manifest the last one had.
    fn append_arrow_batch_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        let batches = crate::io::select_reader(batches, options)?;
        self.append(batches)
    }
}

/// What one [`Table::compact`] call did, in numbers a caller can assert on.
///
/// A compaction with nothing to do reports zeros, because it commits nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compaction {
    /// How many live data files were read and replaced.
    pub files_before: usize,
    /// How many data files the rewrite produced in their place.
    pub files_after: usize,
    /// The recorded size of the replaced files, in bytes.
    pub bytes_rewritten: i64,
}

/// What a beaten commit may do about the writer that got there first.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OnConflict {
    /// Reload the winner's document and re-apply this commit on top of it.
    Rebase,
    /// Only wait and look again; re-applying was declared unsafe, so an
    /// advanced version can end in nothing but a [`CommitConflict`].
    Fail,
}

/// A commit that lost the race to concurrent writers and ran out of retries.
///
/// The crate's error enum is closed, so this crosses the [`Result`] boundary
/// as the module's `iceberg` codec error carrying exactly this value's
/// [`Display`](std::fmt::Display) - which names both versions, so a caller
/// can see how far the table moved while this writer was being beaten.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitConflict {
    /// The document version this writer expected to publish.
    pub expected_version: u32,
    /// How many observations found another writer's version instead.
    pub beaten: u32,
    /// The newest version observed before giving up.
    pub last_seen_version: u32,
}

impl std::fmt::Display for CommitConflict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "expected to commit version {}, got beaten {} times; last saw version {}",
            self.expected_version, self.beaten, self.last_seen_version
        )
    }
}

impl std::error::Error for CommitConflict {}

impl From<CommitConflict> for Error {
    fn from(value: CommitConflict) -> Self {
        invalid(format_smolstr!("{value}"))
    }
}

/// The wait before one retry attempt, exponential with full jitter.
///
/// The window doubles from `min` per attempt and is capped at `max` - a floor
/// configured above the ceiling waits the ceiling - and the wait is drawn
/// uniformly from floor..=window with the same per-process hashing randomness
/// [`snapshot_id`] uses, so beaten writers spread out rather than colliding
/// again in step.
fn backoff_ms(attempt: u32, min: u64, max: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};

    let floor = min.min(max);
    let window = floor
        .saturating_mul(1_u64.checked_shl(attempt).unwrap_or(u64::MAX))
        .clamp(floor, max);
    if window <= floor {
        return floor;
    }
    let state = std::collections::hash_map::RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_i64(now_ms());
    hasher.write_u32(attempt);
    floor + hasher.finish() % (window - floor + 1)
}

/// Split one partition group's batches into files of roughly `target` bytes.
///
/// The estimate is the Arrow in-memory size of each batch
/// ([`RecordBatch::get_array_memory_size`]), taken *before* encoding: Parquet
/// compresses what it writes, so the files land under the target rather than
/// at it. A file closes at the first batch boundary at or past the target and
/// a batch is never split, so one batch larger than the target is one file.
fn rolled(batches: Vec<RecordBatch>, target: u64) -> Vec<Vec<RecordBatch>> {
    let mut files: Vec<Vec<RecordBatch>> = Vec::new();
    let mut current: Vec<RecordBatch> = Vec::new();
    let mut held: u64 = 0;
    for batch in batches {
        let size = u64::try_from(batch.get_array_memory_size()).unwrap_or(u64::MAX);
        held = held.saturating_add(size);
        current.push(batch);
        if held >= target {
            files.push(std::mem::take(&mut current));
            held = 0;
        }
    }
    if !current.is_empty() {
        files.push(current);
    }
    files
}

/// What a commit keeps of the files the current snapshot already names.
enum Retained {
    /// Every live file, left in the manifests that already list it.
    All,
    /// These manifests untouched, plus these files rewritten as existing entries.
    Only {
        /// Manifests the plan never opened, kept in the list exactly as they are.
        manifests: Vec<ManifestFile>,
        /// Files that survive the write, carried into one new manifest per spec.
        entries: Vec<ScanTask>,
    },
}

/// Summarize the partition values a manifest's entries hold, in spec order.
///
/// This is what lets the *next* plan skip the manifest without reading it, so a
/// commit pays one fold over the tuples it already has in hand.
fn summaries(
    spec: &PartitionSpec,
    schema: &Field,
    entries: &[ManifestEntry],
) -> Result<Vec<FieldSummary>> {
    if spec.is_unpartitioned() {
        return Ok(Vec::new());
    }
    let partition = spec.partition_field(schema)?;
    let mut summaries = vec![FieldSummary::default(); partition.field_len()];
    for entry in entries {
        for (index, child) in partition.fields().iter().enumerate() {
            let Some(value) = entry.data_file.partition.get(index) else {
                continue;
            };
            let Some(summary) = summaries.get_mut(index) else {
                continue;
            };
            if matches!(value, Value::Null) {
                summary.contains_null = true;
                continue;
            }
            let Some(encoded) = single_value(value, child.data_type()) else {
                continue;
            };
            fold(&mut summary.lower_bound, &encoded, child.data_type(), true);
            fold(&mut summary.upper_bound, &encoded, child.data_type(), false);
        }
    }
    Ok(summaries)
}

/// Keep the smaller or larger of a running bound and one candidate.
fn fold(current: &mut Option<Vec<u8>>, candidate: &[u8], data_type: &DataType, minimum: bool) {
    match current {
        None => *current = Some(candidate.to_vec()),
        Some(held) => {
            let ordering = compare_single(candidate, held, data_type);
            if (minimum && ordering.is_lt()) || (!minimum && ordering.is_gt()) {
                *current = Some(candidate.to_vec());
            }
        }
    }
}

/// The range of key values one write brings, per match-key column.
///
/// A file can only be changed by a merge if every match-key column's recorded
/// range overlaps the incoming one, so this is what turns a set of statistics
/// into a list of files worth reading.
struct KeyBounds {
    /// One bound per match-key column, in the order the caller named them.
    columns: Vec<KeyBound>,
}

/// One match-key column's incoming range.
struct KeyBound {
    /// The column's field identifier, which statistics are keyed by.
    id: i32,
    /// The column's datatype, which says how a bound compares.
    data_type: DataType,
    /// Whether nothing about this column can exclude a file.
    unbounded: bool,
    /// Whether an incoming key holds no value for this column.
    has_null: bool,
    /// The smallest incoming value, encoded.
    lower: Option<Vec<u8>>,
    /// The largest incoming value, encoded.
    upper: Option<Vec<u8>>,
}

impl KeyBounds {
    /// Measure the incoming rows' range for every match-key column.
    fn of(batches: &[RecordBatch], schema: &Field, merge_by_names: &[String]) -> Result<Self> {
        let mut columns = Vec::with_capacity(merge_by_names.len());
        for name in merge_by_names {
            let field = schema.get_field_by_name(name).ok_or_else(|| {
                let stored = schema
                    .fields()
                    .iter()
                    .map(|field| field.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                invalid(crate::text::expected_got(
                    format_args!(
                        "a merge_by_names column the table schema declares, got {name:?}; it has"
                    ),
                    crate::text::elide_display(&stored),
                ))
            })?;
            let id = field.parquet_field_id()?;
            let mut bound = KeyBound {
                id: id.unwrap_or_default(),
                data_type: field.data_type().clone(),
                unbounded: id.is_none() || !is_portable(field.data_type()),
                has_null: false,
                lower: None,
                upper: None,
            };
            for batch in batches {
                let Some(column) = batch.column_by_name(name) else {
                    continue;
                };
                bound.has_null = bound.has_null || column.null_count() > 0;
                if bound.unbounded {
                    continue;
                }
                if let Some(encoded) = extreme(column, field, false)? {
                    fold(&mut bound.lower, &encoded, &bound.data_type, true);
                }
                if let Some(encoded) = extreme(column, field, true)? {
                    fold(&mut bound.upper, &encoded, &bound.data_type, false);
                }
            }
            columns.push(bound);
        }
        Ok(Self { columns })
    }

    /// Return whether a data file can hold a row one of these keys matches.
    fn may_hold(&self, file: &DataFile) -> bool {
        self.columns.iter().all(|column| column.may_hold(file))
    }
}

impl KeyBound {
    /// Return whether one file's statistics leave room for this column's keys.
    fn may_hold(&self, file: &DataFile) -> bool {
        if self.unbounded {
            return true;
        }
        let nulls = file
            .null_value_counts
            .iter()
            .find_map(|(id, count)| (*id == self.id).then_some(*count));
        // A file with no recorded null count may still hold one, so only a
        // recorded zero rules a null key out.
        if self.has_null && nulls != Some(0) {
            return true;
        }
        let (Some(lower), Some(upper)) = (self.lower.as_deref(), self.upper.as_deref()) else {
            // Nothing but nulls arrived for this column, and the null case above
            // already decided what that can match.
            return false;
        };
        let file_lower = file
            .lower_bounds
            .iter()
            .find_map(|(id, bytes)| (*id == self.id).then_some(bytes.as_slice()));
        let file_upper = file
            .upper_bounds
            .iter()
            .find_map(|(id, bytes)| (*id == self.id).then_some(bytes.as_slice()));
        let (Some(file_lower), Some(file_upper)) = (file_lower, file_upper) else {
            // A file that records no range for the key has to be read.
            return true;
        };
        !(compare_single(upper, file_lower, &self.data_type).is_lt()
            || compare_single(lower, file_upper, &self.data_type).is_gt())
    }
}

/// Encode the smallest or largest value one column holds.
///
/// The extreme is found by a bounded sort rather than a scan of decoded values:
/// one index is all a bound needs, and asking Arrow for it keeps the work in the
/// kernel instead of in a per-row conversion.
fn extreme(column: &ArrayRef, field: &Field, descending: bool) -> Result<Option<Vec<u8>>> {
    if column.null_count() == column.len() {
        return Ok(None);
    }
    let options = SortOptions {
        descending,
        // Nulls last, so the first index is the extreme value rather than an
        // absent one, whichever direction the sort runs in.
        nulls_first: false,
    };
    let indices = arrow_ord::sort::sort_to_indices(column.as_ref(), Some(options), Some(1))
        .map_err(Error::Arrow)?;
    let Some(row) = indices.values().first().copied() else {
        return Ok(None);
    };
    let slice = column.slice(usize::try_from(row).unwrap_or_default(), 1);
    let scalar = crate::arrow::ArrowScalar::from_parts(field.clone().with_nullable(true), slice)
        .and_then(|scalar| scalar.to_value())
        .map_err(|error| invalid(format_smolstr!("{error}")))?;
    Ok(single_value(&scalar, field.data_type()))
}

/// Split every incoming batch into one group per partition tuple.
///
/// A data file belongs to exactly one partition, so a partitioned write has to
/// group its rows before it can write anything; an unpartitioned one does not
/// and passes straight through as a single group.
fn grouped_batches(
    batches: BatchReader,
    schema: &Field,
    spec: &PartitionSpec,
    sources: &[SmolStr],
    partition: &Field,
) -> Result<Vec<(Vec<Value>, Vec<RecordBatch>)>> {
    let mut groups: Vec<(Vec<Value>, Vec<RecordBatch>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for batch in batches {
        let batch = schema.cast_arrow_batch(batch.map_err(Error::Arrow)?, false)?;
        if batch.num_rows() == 0 {
            continue;
        }
        if spec.is_unpartitioned() {
            match groups.first_mut() {
                Some(group) => group.1.push(batch),
                None => groups.push((Vec::new(), vec![batch])),
            }
            continue;
        }

        for (key, rows) in row_groups(&batch, sources)? {
            let position = match index.get(&key) {
                Some(position) => *position,
                None => {
                    let first = *rows.first().unwrap_or(&0);
                    let values = tuple_at(&batch, sources, partition, first)?;
                    groups.push((values, Vec::new()));
                    index.insert(key, groups.len() - 1);
                    groups.len() - 1
                }
            };
            let indices = UInt32Array::from(rows);
            let taken = arrow_select::take::take_record_batch(&batch, &indices)?;
            groups[position].1.push(taken);
        }
    }
    Ok(groups)
}

/// Group a batch's row indices by the text of their partition source values.
fn row_groups(batch: &RecordBatch, sources: &[SmolStr]) -> Result<Vec<(String, Vec<u32>)>> {
    let mut formatters = Vec::with_capacity(sources.len());
    for source in sources {
        let column = batch.column_by_name(source).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a partition source column {source:?} in the batch, got none"
            ))
        })?;
        formatters.push(arrow_cast::display::ArrayFormatter::try_new(
            column.as_ref(),
            &arrow_cast::display::FormatOptions::default(),
        )?);
    }

    let mut order: Vec<(String, Vec<u32>)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    for row in 0..batch.num_rows() {
        let mut key = String::new();
        for (offset, formatter) in formatters.iter().enumerate() {
            if offset > 0 {
                key.push('\u{1}');
            }
            // A null is not the text a formatter prints for it, so it gets a
            // marker no formatted value can collide with.
            let column = batch.column(
                batch
                    .schema()
                    .index_of(&sources[offset])
                    .map_err(Error::Arrow)?,
            );
            if column.is_null(row) {
                key.push('\u{0}');
            } else {
                key.push_str(&formatter.value(row).to_string());
            }
        }
        match seen.get(&key) {
            Some(position) => order[*position]
                .1
                .push(u32::try_from(row).unwrap_or_default()),
            None => {
                seen.insert(key.clone(), order.len());
                order.push((key, vec![u32::try_from(row).unwrap_or_default()]));
            }
        }
    }
    Ok(order)
}

/// Read one row's partition tuple out of a batch.
fn tuple_at(
    batch: &RecordBatch,
    sources: &[SmolStr],
    partition: &Field,
    row: u32,
) -> Result<Vec<Value>> {
    let mut values = Vec::with_capacity(sources.len());
    for (source, field) in sources.iter().zip(partition.fields()) {
        let column = batch.column_by_name(source).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a partition source column {source:?} in the batch, got none"
            ))
        })?;
        let slice = column.slice(row as usize, 1);
        let scalar = crate::arrow::ArrowScalar::from_parts(field.clone(), slice)
            .map_err(|error| invalid(format_smolstr!("{error}")))?;
        values.push(
            scalar
                .to_value()
                .map_err(|error| invalid(format_smolstr!("{error}")))?,
        );
    }
    Ok(values)
}

/// Return the metadata document with the highest version, and its number.
///
/// A folder that holds none is `None` rather than an error, because that is the
/// question "is this a table" and the answer "no" is not a failure.
fn find_metadata(metadata_dir: &Holder) -> Result<Option<(u32, Value)>> {
    // A folder that is not a table has no metadata directory at all, and the
    // laziness contract makes that a handle that simply is not a container.
    if !metadata_dir.is_container() {
        return Ok(None);
    }

    let hint = metadata_dir.child_by(VERSION_HINT)?;
    if hint.size() > 0 {
        let text = String::from_utf8_lossy(&hint.read_all()?).trim().to_owned();
        if let Ok(version) = text.parse::<u32>() {
            let document = metadata_dir.child_by(&format!("v{version}.metadata.json"))?;
            if document.size() > 0 {
                return Ok(Some((
                    version,
                    crate::json::from_slice(&document.read_all()?)?,
                )));
            }
        }
    }

    // No usable hint: take the highest-numbered document that is actually there.
    let mut best: Option<(u32, Holder)> = None;
    for entry in metadata_dir.ls(false, false)? {
        let Some(name) = entry
            .url()
            .and_then(|url| url.file_name().map(ToOwned::to_owned))
        else {
            continue;
        };
        let Some(stem) = name.strip_suffix(".metadata.json") else {
            continue;
        };
        // Both `v3` and Iceberg's own `00003-<uuid>` numbering start with digits.
        let digits: String = stem
            .trim_start_matches('v')
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        let Ok(version) = digits.parse::<u32>() else {
            continue;
        };
        if best.as_ref().is_none_or(|(highest, _)| version > *highest) {
            best = Some((version, entry));
        }
    }

    let Some((version, document)) = best else {
        return Ok(None);
    };
    Ok(Some((
        version,
        crate::json::from_slice(&document.read_all()?)?,
    )))
}

/// Report a folder that holds no Iceberg metadata document.
fn missing_metadata(metadata_dir: &Holder) -> Error {
    invalid(format_smolstr!(
        "expected an Iceberg metadata document under {}, got none",
        metadata_dir
            .url()
            .map_or_else(|| "an unlocated folder".to_owned(), ToString::to_string)
    ))
}

/// Turn one absolute location into a name relative to the table's folder.
fn relative_location(base: &str, location: &str) -> Result<String> {
    // Separators are normalized because an implementation that wrote the table
    // on Windows may have spelled its own location with backslashes.
    let normalized_base = base.replace('\\', "/");
    let normalized_base = normalized_base.trim_end_matches('/');
    let normalized = location.replace('\\', "/");
    if let Some(rest) = normalized.strip_prefix(normalized_base) {
        return Ok(rest.trim_start_matches('/').to_owned());
    }
    // A table moved after it was written names its own old location; falling
    // back to the last `data/` or `metadata/` segment keeps it readable.
    for directory in [DATA_DIR, METADATA_DIR] {
        if let Some(position) = normalized.rfind(&format!("/{directory}/")) {
            return Ok(normalized[position + 1..].to_owned());
        }
    }
    Err(invalid(format_smolstr!(
        "expected a location inside the table at {normalized_base:?}, got {location:?}"
    )))
}

/// Produce a positive random snapshot identifier.
fn snapshot_id() -> i64 {
    use std::hash::{BuildHasher, Hasher};

    let state = std::collections::hash_map::RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_i64(now_ms());
    // Iceberg identifiers are signed but conventionally positive.
    (hasher.finish() >> 1) as i64
}

/// Report a malformed or unreachable Iceberg table.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
