//! An Iceberg table as a folder handle and nothing else.
//!
//! A table *is* a directory: `metadata/` holds the JSON documents and the Avro
//! manifests, `data/` holds the Parquet files, and every one of them is reached
//! with [`IOBase::child_by_path`] against the handle the table was constructed from.
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
//! [`Table::target_file_size_bytes`] - one manifest, one manifest list, and one
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
//! [`IcebergOptions::commit_retries`] and
//! [`IcebergOptions::commit_total_timeout_ms`] - and otherwise reports a
//! [`CommitConflict`] naming both versions. An append and a metadata-only
//! change rebase; [`Table::commit_overwrite_where`], [`Table::commit_merge_where`], and
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
//! [`Table::create_branch`], [`Table::create_tag`], [`Table::remove_snapshot_ref`],
//! [`Table::fast_forward_branch`], and [`Table::expire_snapshots`] are thin wrappers
//! over [`TableMetadata`]'s ref vocabulary, each committed through the same
//! retrying [`Table::commit_metadata_changes`]. Writing *to* a branch other than
//! `main` remains future work, because a commit's parent is currently always
//! the table's current snapshot.

use std::collections::HashMap;

use arrow_array::{Array, ArrayRef, RecordBatch, StructArray, UInt32Array};
use arrow_schema::SortOptions;
use smol_str::{SmolStr, format_smolstr};

use super::manifest::{
    DataFile, FieldSummary, ManifestContent, ManifestEntry, ManifestFile, is_iceberg_mime_type,
    read_manifest_list, read_v1_direct_manifest_file, write_manifest, write_manifest_list,
};
use super::metadata::{FormatVersion, TableMetadata, now_ms, uuid};
use super::options::{CommitSettings, IcebergOptions};
use super::partition::PartitionSpec;
use super::scan::{ScanPart, ScanPlan, ScanTask};
use super::snapshot::{Snapshot, SnapshotRef};
use super::value::{compare_single, is_portable, single_value};
use crate::arrow::BatchReader;
use crate::field::cast::ArrowCast;
use crate::generic::{Holder, IORecordOptions, RecordOptions};
use crate::io::{IOBase, IOMedia};
use crate::{DataType, Error, Field, IOKind, MimeType, Result, Scalar};

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
    /// The exact discovered metadata filename, including UUID and compression.
    metadata_file_name: SmolStr,
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
    /// Returns a conflict when the handle already contains a table, or an
    /// error when the handle is not a container, the schema is not a non-null
    /// struct root, or the metadata document cannot be written.
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
            metadata_file_name: SmolStr::new_static(""),
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
        let metadata_dir = root.child_by_path(METADATA_DIR)?;
        match find_metadata(&metadata_dir)? {
            Some((version, metadata_file_name, document)) => Ok(Self {
                root,
                metadata: TableMetadata::from_json(&document)?,
                version,
                metadata_file_name,
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
        Ok(Self::locate_keeping(root)?.ok())
    }

    /// [`Self::locate`], handing the handle back when nothing is there.
    ///
    /// A handle is not always cheap to rebuild - and per the existence
    /// contract the locate *is* the existence question, so a caller that
    /// creates on absence must not have to resolve the folder a second time
    /// to do it. `Ok(Err(root))` is that answer: no table, and here is the
    /// handle you gave, untouched.
    pub(crate) fn locate_keeping(root: H) -> Result<std::result::Result<Self, H>> {
        let metadata_dir = root.child_by_path(METADATA_DIR)?;
        let Some((version, metadata_file_name, document)) = find_metadata(&metadata_dir)? else {
            return Ok(Err(root));
        };
        Ok(Ok(Self {
            root,
            metadata: TableMetadata::from_json(&document)?,
            version,
            metadata_file_name,
            options: None,
        }))
    }

    /// Open the table if it exists, creating it otherwise.
    ///
    /// One locate is the whole existence question, per the existence
    /// contract: the handle comes back from the miss, so the create needs no
    /// second resolution.
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
        match Self::locate_keeping(root)? {
            Ok(table) => Ok(table),
            Err(root) => Self::create(root, format_version, schema, spec),
        }
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
    pub const fn metadata_version(&self) -> u32 {
        self.version
    }

    /// Return the name of the current metadata document.
    pub fn metadata_file_name(&self) -> String {
        self.metadata_file_name.to_string()
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
    pub fn target_file_size_bytes(&self) -> Result<u64> {
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

    /// Borrow the explicit options override, when one is stored.
    ///
    /// This is only what [`Self::set_options`] stored - the property layer and
    /// the defaults are not consulted - so a binding can save and restore the
    /// override around a call that shadows it.
    pub fn explicit_options(&self) -> Option<&IcebergOptions> {
        self.options.as_ref()
    }

    /// Remove the explicit options override, returning what was stored.
    ///
    /// Every field then resolves property-then-default again.
    pub fn clear_options(&mut self) -> Option<IcebergOptions> {
        self.options.take()
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
    /// Returns an error when a manifest or manifest list cannot be reached or decoded.
    pub fn manifests_at(&self, snapshot: &Snapshot) -> Result<Vec<ManifestFile>> {
        if let Some(paths) = &snapshot.manifests {
            return paths
                .iter()
                .map(|path| {
                    let handle = self.child_at(path)?;
                    read_v1_direct_manifest_file(&handle, path, snapshot.snapshot_id)
                })
                .collect();
        }
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
        self.plan_matching(pairs_predicate(self.schema()?, filters))
    }

    /// Plan a scan under one predicate, opening only what the metadata allows.
    ///
    /// The predicate is the crate's one filter type, so the same text prunes a
    /// table here, a lake through
    /// [`IOBase::children_matching`](crate::io::IOBase::children_matching), and
    /// a batch through [`Bound::filter`](crate::expression::Bound::filter).
    /// Each level of the metadata chain answers it from the statistics it
    /// carries: a manifest-list summary, then a manifest entry's partition
    /// tuple and column bounds. What none of them settles is left for the rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the predicate is text that does not parse, names
    /// a column the schema does not declare, or when a manifest that had to be
    /// read cannot be reached or decoded.
    pub fn plan_matching(
        &self,
        filter: impl crate::expression::IntoExpression,
    ) -> Result<ScanPlan> {
        let filter = filter.into_expression()?;
        let conjuncts = super::scan::conjuncts(self.schema()?, &filter)?;
        let schema = self.schema()?.clone();
        self.planned(&conjuncts, &schema, false)
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
        let filter = pairs_predicate(schema, filters);
        let conjuncts = super::scan::conjuncts(schema, &filter)?;
        let schema = schema.clone();
        let manifests = self.manifests_at(snapshot)?;
        self.plan_manifests(&manifests, &conjuncts, &schema, false)
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
        let filter = pairs_predicate(&stored, filters);
        let conjuncts = super::scan::conjuncts(&stored, &filter)?;
        let manifests = self.manifests_at(snapshot)?;
        let plan = self.plan_manifests(&manifests, &conjuncts, &stored, true)?;
        self.reader(plan.tasks, &stored, field, &filter)
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
    fn planned(
        &self,
        conjuncts: &[crate::expression::Bound],
        schema: &Field,
        for_read: bool,
    ) -> Result<ScanPlan> {
        let manifests = self.manifests()?;
        self.plan_manifests(&manifests, conjuncts, schema, for_read)
    }

    /// Plan one set of manifests under one set of resolved filters.
    fn plan_manifests(
        &self,
        manifests: &[ManifestFile],
        conjuncts: &[crate::expression::Bound],
        schema: &Field,
        for_read: bool,
    ) -> Result<ScanPlan> {
        super::scan::plan(
            manifests,
            &|spec_id| {
                self.metadata
                    .spec_by_id(spec_id)
                    .cloned()
                    .ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected partition spec id {spec_id} among the table's partition specs, got none"
                        ))
                    })
            },
            &|location| self.child_at(location),
            conjuncts,
            schema,
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
    /// [`IcebergOptions::commit_retries`] times within
    /// [`IcebergOptions::commit_total_timeout_ms`], and reporting a
    /// [`CommitConflict`] when the retries run out. The check is best-effort
    /// on plain storage - [`IOBase`] has no compare-and-swap, so retries
    /// shrink the undetected-race window without closing it.
    ///
    /// ```no_run
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// # let folder = yggdryl::local::Folder::new(std::env::temp_dir().join("t"))?;
    /// # let mut table = yggdryl::iceberg::Table::open(folder)?;
    /// table.commit_metadata_changes(|metadata| {
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
    pub fn commit_metadata_changes(
        &mut self,
        mut change: impl FnMut(&mut TableMetadata) -> Result<()>,
    ) -> Result<()> {
        self.commit_document(OnConflict::Rebase, move |table| {
            // The change runs on a copy, so a rejected change costs nothing.
            let mut updated = table.metadata.clone();
            change(&mut updated)?;
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
    /// [`IcebergOptions::commit_retries`] times or reserving more cumulative
    /// backoff than [`IcebergOptions::commit_total_timeout_ms`] restores the
    /// in-memory state and returns a [`CommitConflict`].
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
        let saved_metadata_file_name = self.metadata_file_name.clone();
        let expected_version = saved_version.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "cannot commit metadata after version {saved_version}: the version overflows u32"
            ))
        })?;
        let metadata_dir = self.root.child_by_path(METADATA_DIR)?;
        let restore = |table: &mut Self, error: Error| {
            table.metadata = saved_metadata.clone();
            table.version = saved_version;
            table.metadata_file_name = saved_metadata_file_name.clone();
            Err(error)
        };
        let reconcile_visible = |table: &mut Self, error: Error| {
            // A backend may publish the metadata document and hint, then
            // report the hint write as failed. Re-read through the same
            // discovery path a new handle uses and adopt that visible version
            // when it is sound. A failed reload must never mask `error`; the
            // saved state is the only conservative in-memory answer when
            // visibility itself is uncertain.
            match find_metadata(&metadata_dir).and_then(|visible| {
                visible
                    .map(|(version, metadata_file_name, document)| {
                        TableMetadata::from_json(&document)
                            .map(|metadata| (version, metadata_file_name, metadata))
                    })
                    .transpose()
            }) {
                Ok(Some((version, metadata_file_name, metadata))) => {
                    table.metadata = metadata;
                    table.version = version;
                    table.metadata_file_name = metadata_file_name;
                }
                Ok(None) | Err(_) => {
                    table.metadata = saved_metadata.clone();
                    table.version = saved_version;
                    table.metadata_file_name = saved_metadata_file_name.clone();
                }
            }
            Err(error)
        };

        let mut beaten: u32 = 0;
        let mut backoff_spent_ms = 0_u64;
        loop {
            match find_metadata(&metadata_dir) {
                Ok(Some((version, metadata_file_name, document))) if version > self.version => {
                    let wait = match retry_wait_ms(
                        &settings,
                        &mut beaten,
                        &mut backoff_spent_ms,
                        expected_version,
                        version,
                    ) {
                        Ok(wait) => wait,
                        Err(error) => return restore(self, error),
                    };
                    if on_conflict == OnConflict::Rebase {
                        match TableMetadata::from_json(&document) {
                            Ok(fresh) => {
                                self.metadata = fresh;
                                self.version = version;
                                self.metadata_file_name = metadata_file_name;
                            }
                            Err(error) => return restore(self, error),
                        }
                    }
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
                if !error.is_conflict() {
                    return reconcile_visible(self, error);
                }
                let winner = find_metadata(&metadata_dir).and_then(|visible| {
                    visible
                        .map(|(version, metadata_file_name, document)| {
                            TableMetadata::from_json(&document)
                                .map(|metadata| (version, metadata_file_name, metadata))
                        })
                        .transpose()
                });
                let Ok(Some((version, metadata_file_name, metadata))) = winner else {
                    return reconcile_visible(self, error);
                };
                if version <= self.version {
                    return reconcile_visible(self, error);
                }
                let wait = match retry_wait_ms(
                    &settings,
                    &mut beaten,
                    &mut backoff_spent_ms,
                    expected_version,
                    version,
                ) {
                    Ok(wait) => wait,
                    Err(error) => return restore(self, error),
                };
                if on_conflict == OnConflict::Rebase {
                    self.metadata = metadata;
                    self.version = version;
                    self.metadata_file_name = metadata_file_name;
                }
                if wait > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(wait));
                }
                continue;
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
    /// Each data file is read through [`crate::io::IOMedia::read_arrow_reader`] with
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
        self.scan_matching(pairs_predicate(&stored, filters), field)
    }

    /// Read the rows matching one predicate, keeping the columns `field` names.
    ///
    /// This is [`Self::scan_where`] with the whole expression language rather
    /// than equality pairs: ranges, null tests, `in` lists, nested paths, and
    /// `&holder.*` attributes about the files themselves. Planning prunes with
    /// [`Self::plan_matching`], and only the conjuncts the metadata could not
    /// settle are tested against the rows a surviving file holds.
    ///
    /// ```no_run
    /// use yggdryl::iceberg::Table;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let table = Table::open(Folder::new("/lake/trades")?)?;
    /// let reader = table.scan_matching(
    ///     "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'",
    ///     None,
    /// )?;
    /// # let _ = reader;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the predicate does not parse or bind, when a
    /// manifest cannot be read, or when the scan root cannot be projected.
    pub fn scan_matching(
        &self,
        filter: impl crate::expression::IntoExpression,
        field: Option<&Field>,
    ) -> Result<BatchReader> {
        let stored = self.schema()?.clone();
        let filter = filter.into_expression()?;
        let conjuncts = super::scan::conjuncts(&stored, &filter)?;
        let plan = self.planned(&conjuncts, &stored, true)?;
        self.reader(plan.tasks, &stored, field, &filter)
    }

    /// Build the reader over one set of planned files.
    fn reader(
        &self,
        tasks: Vec<ScanTask>,
        stored: &Field,
        field: Option<&Field>,
        filter: &crate::Expression,
    ) -> Result<BatchReader> {
        let root = field.map_or_else(|| stored.clone(), Clone::clone);
        let read_root = super::scan::read_root(&root, stored, filter)?;
        // The residual conjuncts run against the read root, which carries the
        // predicate's own columns even when the caller projected them away.
        let predicates = super::scan::conjuncts(&read_root, filter)?;

        let mut parts = Vec::new();
        for task in tasks {
            let mut handle = self.child_at(&task.entry.data_file.file_path)?;
            // The manifest is the authority on a file's format, not its name:
            // a table whose files mix formats - or name them without an
            // extension - still decodes each file as the entry records it.
            handle.set_media_type(crate::MediaType::new(
                task.entry.data_file.mime_type.clone(),
            ));
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
        super::scan::reader(
            parts,
            root,
            read_root,
            field.cloned(),
            predicates,
            &parallel,
        )
    }

    /// Append `batches` as a new snapshot, keeping everything already stored.
    ///
    /// An append beaten by a concurrent commit *rebases*: the data files are
    /// already written, so only the manifest list and the document are rebuilt
    /// on the winner's metadata - fresh parent, fresh sequence number - with
    /// backoff between attempts, bounded by [`IcebergOptions::commit_retries`]
    /// and [`IcebergOptions::commit_total_timeout_ms`]. The version check is
    /// best-effort on plain storage: [`IOBase`] has no compare-and-swap, so
    /// retries shrink the undetected-race window without closing it, and
    /// serialized writers are what closes it.
    ///
    /// # Errors
    ///
    /// Returns an error when the partition spec cannot place a row, when a
    /// batch cannot be cast to the table schema, when any write fails, or a
    /// [`CommitConflict`] when concurrent writers exhausted the retries.
    pub fn commit_append(&mut self, batches: BatchReader) -> Result<()> {
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
    pub fn commit_overwrite(&mut self, batches: BatchReader) -> Result<()> {
        self.commit_overwrite_where(&[], batches)
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
    /// naming both versions instead, after the waits bounded by
    /// [`IcebergOptions::commit_retries`] and
    /// [`IcebergOptions::commit_total_timeout_ms`]; the caller re-reads and
    /// retries with fresh input.
    ///
    /// # Errors
    ///
    /// Returns an error when a filter names a column the schema does not
    /// declare, when the partition spec cannot place a row, when any read or
    /// write fails, or a [`CommitConflict`] when a concurrent commit won.
    pub fn commit_overwrite_where(
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
    pub fn commit_merge(
        &mut self,
        batches: BatchReader,
        merge_by_names: &[String],
        safe: bool,
    ) -> Result<()> {
        self.commit_merge_where(&[], batches, merge_by_names, safe)
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
    /// Like [`Self::commit_overwrite_where`], a merge beaten by a concurrent commit
    /// reports a [`CommitConflict`] rather than rebasing, because the files it
    /// selected and the reader it consumed cannot be re-planned safely.
    ///
    /// # Errors
    ///
    /// Returns an error for a keyed merge on format v3, whose existing row IDs
    /// this writer cannot yet preserve, when `merge_by_names` names a column
    /// the schema does not declare, or for any read, join, or write failure,
    /// including a [`CommitConflict`] when a concurrent commit won.
    pub fn commit_merge_where(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        merge_by_names: &[String],
        safe: bool,
    ) -> Result<()> {
        if merge_by_names.is_empty() {
            return self.commit_overwrite_where(filters, batches);
        }
        self.require_row_id_preserving_rewrite("merge")?;
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

        let stored = self.reader(selected, &schema, None, &crate::Expression::always_true())?;
        let arrow_schema = crate::arrow::arrow_schema_from_field(&schema)?;
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
    /// [`Self::target_file_size_bytes`]. The rewritten rows go through the same
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
    /// Returns an error for format v3, whose existing row IDs this writer
    /// cannot yet preserve, when the target size is configured but
    /// unparseable, or when any read or write of the rewrite fails.
    pub fn compact(&mut self) -> Result<Compaction> {
        self.require_row_id_preserving_rewrite("compaction")?;
        let target = i64::try_from(self.target_file_size_bytes()?).unwrap_or(i64::MAX);
        let plan = self.plan(&[])?;

        // Group the live files by (spec, partition tuple), in plan order.
        let mut groups: Vec<(i32, Vec<Scalar>, Vec<ScanTask>)> = Vec::new();
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
        let bytes_rewritten = selected.iter().try_fold(0_i64, |total, task| {
            total
                .checked_add(task.entry.data_file.file_size_in_bytes)
                .ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected compacted byte size fitting i64, overflowed at {:?}",
                        task.entry.data_file.file_path
                    ))
                })
        })?;

        let schema = self.schema()?.clone();
        let rows = self.reader(selected, &schema, None, &crate::Expression::always_true())?;
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
        self.commit_metadata_changes(|metadata| {
            schema_id = metadata.add_schema(schema.clone())?;
            metadata.set_current_schema(schema_id)?;
            Ok(())
        })?;
        Ok(schema_id)
    }

    /// Create a branch at one retained snapshot, as one metadata commit.
    ///
    /// This is [`TableMetadata::create_branch`] committed through the
    /// retrying [`Self::commit_metadata_changes`]. Writing *to* a branch other than
    /// `main` remains future work - a commit's parent is currently always the
    /// current snapshot - so a branch is read with [`Self::scan_ref`] and
    /// moved with [`Self::fast_forward_branch`].
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken or reserved, when the snapshot
    /// is not retained, or when the commit fails.
    pub fn create_branch(&mut self, name: &str, snapshot_id: i64) -> Result<()> {
        self.commit_metadata_changes(|metadata| {
            metadata.create_branch(SmolStr::new(name), snapshot_id)
        })
    }

    /// Create a tag at one retained snapshot, as one metadata commit.
    ///
    /// This is [`TableMetadata::create_tag`] committed through the retrying
    /// [`Self::commit_metadata_changes`].
    ///
    /// # Errors
    ///
    /// Returns an error when the name is taken or reserved, when the snapshot
    /// is not retained, or when the commit fails.
    pub fn create_tag(&mut self, name: &str, snapshot_id: i64) -> Result<()> {
        self.commit_metadata_changes(|metadata| {
            metadata.create_tag(SmolStr::new(name), snapshot_id)
        })
    }

    /// Remove one branch or tag, as one metadata commit.
    ///
    /// Returns the reference that was removed. This is
    /// [`TableMetadata::remove_snapshot_ref`] committed through the retrying
    /// [`Self::commit_metadata_changes`]; a name the table does not have is an error
    /// rather than an empty commit.
    ///
    /// # Errors
    ///
    /// Returns an error naming the refs the table does have when `name` is
    /// not one of them, or when the commit fails.
    pub fn remove_snapshot_ref(&mut self, name: &str) -> Result<SnapshotRef> {
        let mut removed = None;
        self.commit_metadata_changes(|metadata| match metadata.remove_snapshot_ref(name)? {
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
    /// retrying [`Self::commit_metadata_changes`]: the target must be retained and must
    /// reach the branch's head by walking parent ids, so a fast-forward can
    /// never lose history.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not a branch, the target is not
    /// retained or not a descendant, or the commit fails.
    pub fn fast_forward_branch(&mut self, name: &str, snapshot_id: i64) -> Result<()> {
        self.commit_metadata_changes(|metadata| metadata.fast_forward_branch(name, snapshot_id))
    }

    /// Expire the snapshots retention no longer keeps, as one metadata commit.
    ///
    /// Optional cutoffs, retain counts, and explicit ids have the same union
    /// and precedence as [`TableMetadata::expire_snapshots`]. Returns sorted
    /// expired ids and commits nothing when neither a snapshot nor stale ref
    /// changes.
    ///
    /// # Errors
    ///
    /// Returns the expiry's own failure, or the commit failure.
    pub fn expire_snapshots(
        &mut self,
        older_than_ms: Option<i64>,
        retain_last: Option<usize>,
        snapshot_ids: &[i64],
    ) -> Result<Vec<i64>> {
        let mut probe = self.metadata.clone();
        probe.expire_snapshots(older_than_ms, retain_last, snapshot_ids)?;
        if probe == self.metadata {
            return Ok(Vec::new());
        }
        let mut expired = Vec::new();
        self.commit_metadata_changes(|metadata| {
            expired = metadata.expire_snapshots(older_than_ms, retain_last, snapshot_ids)?;
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
    /// a relative name and resolved with [`IOBase::child_by_path`]. That is what
    /// keeps this module free of path handling: the backend decides what a
    /// child is, and a table written on one storage system moves to another by
    /// rewriting its locations rather than its code.
    pub(super) fn child_at(&self, location: &str) -> Result<Holder> {
        let relative = relative_location(&self.metadata.location, location)?;
        self.root.child_by_path(&relative)
    }

    /// Write the current metadata as the next numbered document.
    fn commit_metadata(&mut self) -> Result<()> {
        // A bad in-memory state is refused before a document exists, so a
        // broken table can only be read, never written.
        self.metadata.validate()?;
        let previous = (self.version > 0)
            .then(|| self.metadata_location())
            .transpose()?;
        let next_version = self.version.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "cannot commit metadata after version {}: the version overflows u32",
                self.version
            ))
        })?;
        let mut metadata = self.metadata.clone();
        metadata.finalize_official(previous)?;
        let compression = metadata.metadata_compression_codec()?;
        let document = metadata.clone().into_json()?;
        let encoded = crate::json::into_bytes(&document)?;
        let (suffix, encoded) = match compression {
            iceberg_official::compression::CompressionCodec::None => ("", encoded),
            iceberg_official::compression::CompressionCodec::Gzip(_) => {
                (".gz", crate::gzip::dump(&encoded)?)
            }
            other => {
                return Err(invalid(format_smolstr!(
                    "unsupported Iceberg metadata compression codec {other}; expected none or gzip"
                )));
            }
        };
        let attempt = format_smolstr!("{next_version:05}-{}{suffix}.metadata.json", uuid());
        let metadata_dir = self.root.child_by_path(METADATA_DIR)?;
        let mut handle = self
            .root
            .child_by_path(&format!("{METADATA_DIR}/{attempt}"))?;
        handle.write_all_bytes(&encoded)?;

        // UUID filenames make the write itself the create/commit attempt.
        // Another document at this version means a table or concurrent writer
        // already won; remove only our unpublished candidate and report it.
        let competitors = metadata_names_at_version(&metadata_dir, next_version)?
            .into_iter()
            .filter(|candidate| candidate != &attempt)
            .collect::<Vec<_>>();
        if !competitors.is_empty() {
            handle.remove(false)?;
            return Err(Error::conflict(
                "Iceberg metadata version",
                "Iceberg metadata version",
                format!("{next_version}: {}", competitors.join(", ")),
            ));
        }

        // Winning the attempt publishes the document under the name a hint
        // names: `v{version}`. A catalog stores the exact UUID filename and can
        // afford any spelling, but this surface has only the hint, and every
        // reader of a catalog-free table - this module's own included - resolves
        // it that way. The attempt stays in place across the publish so that a
        // racing writer never sees the version free: `metadata_names_at_version`
        // counts both spellings, and one of them is always there.
        let name = format_smolstr!("v{next_version}{suffix}.metadata.json");
        let mut document = self.root.child_by_path(&format!("{METADATA_DIR}/{name}"))?;
        document.write_all_bytes(&encoded)?;

        // The hint is how a catalog-free reader finds the current document.
        let mut hint = self
            .root
            .child_by_path(&format!("{METADATA_DIR}/{VERSION_HINT}"))?;
        hint.write_all_bytes(next_version.to_string().as_bytes())?;

        // The attempt has served its whole purpose. Its removal is the commit's
        // last act rather than a step of it: the published document and the hint
        // are already durable, so a backend that refuses leaves an unreferenced
        // duplicate rather than an unfinished commit.
        drop(handle.remove(false));
        self.metadata = metadata;
        self.version = next_version;
        self.metadata_file_name = name;
        Ok(())
    }

    /// Write the data files, the manifest, the manifest list, and the metadata.
    ///
    /// Returns how many data files the commit wrote. Each partition group's
    /// rows are rolled into files of roughly [`Self::target_file_size_bytes`] bytes,
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
        let target = self.target_file_size_bytes()?;
        // The format is resolved and checked against the build before the
        // incoming reader is consumed, so a format this build cannot encode
        // fails up front rather than after data files were written.
        let mime_type = IcebergOptions::write_mime_type(self.options.as_ref(), &self.metadata)?;
        require_encodable(&mime_type)?;
        let initial_sequence = next_sequence_number(&self.metadata)?;

        let snapshot_id = snapshot_id();
        let partition = spec.partition_field(&schema)?;

        let write = FileWrite {
            snapshot_id,
            schema: &schema,
            spec: &spec,
            mime_type,
        };
        let mut written = Vec::new();
        for (values, group) in grouped_batches(batches, &schema, &spec, &partition)? {
            for file in rolled(group, target) {
                written.push(self.write_data_file(&write, written.len(), &values, file)?);
            }
        }
        let files_written = written.len();

        let added_records = checked_file_sum(&written, |file| file.record_count, "record count")?;
        let added_size = checked_file_sum(&written, |file| file.file_size_in_bytes, "file size")?;
        let added_files = i32::try_from(written.len()).map_err(|_| {
            invalid(format_smolstr!(
                "expected fewer than {} files in one commit, got {}",
                i32::MAX,
                written.len()
            ))
        })?;

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
                initial_sequence,
            )?;
            manifest.added_files_count = Some(added_files);
            manifest.added_rows_count = Some(added_records);
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
                    initial_sequence,
                )?);
                (OnConflict::Fail, Some(kept))
            }
        };

        let operation = SmolStr::new(operation);
        let compacting = operation == "replace";
        self.commit_document(on_conflict, move |table| {
            let sequence_number = next_sequence_number(&table.metadata)?;
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
                .child_by_path(&format!("{METADATA_DIR}/{list_name}"))?;
            let first_row_id = if table.metadata.format_version >= FormatVersion::V3 {
                Some(
                    table
                        .metadata
                        .next_row_id
                        .ok_or_else(|| Error::InvalidRecord {
                            path: "$.iceberg.next-row-id".into(),
                            reason: SmolStr::new_static("expected a v3 next-row-id, got none"),
                        })?,
                )
            } else {
                None
            };
            let next_row_id = write_manifest_list(
                &mut list,
                table.metadata.format_version,
                snapshot_id,
                table.metadata.current_snapshot_id,
                sequence_number,
                first_row_id,
                &manifests,
            )?;

            let total_records = checked_manifest_total_i64(
                &manifests,
                |manifest| (manifest.added_rows_count, manifest.existing_rows_count),
                "row count",
            )?;
            let total_files = checked_manifest_total_i32(
                &manifests,
                |manifest| (manifest.added_files_count, manifest.existing_files_count),
                "file count",
            )?;
            let mut summary = vec![
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
            ];
            if let Some(total) = total_files {
                summary.push((
                    SmolStr::new_static("total-data-files"),
                    format_smolstr!("{total}"),
                ));
            }
            if let Some(total) = total_records {
                summary.push((
                    SmolStr::new_static("total-records"),
                    format_smolstr!("{total}"),
                ));
            }

            let assigned_rows = match (first_row_id, next_row_id) {
                (Some(first), Some(next)) => Some(next.checked_sub(first).ok_or_else(|| {
                    invalid(format_smolstr!(
                        "expected manifest-list row ids to advance from {first}, got {next}"
                    ))
                })?),
                (None, None) => None,
                (first, next) => {
                    return Err(invalid(format_smolstr!(
                        "expected matching snapshot and manifest-list row-id state, got {first:?} and {next:?}"
                    )));
                }
            };
            let snapshot = Snapshot {
                snapshot_id,
                parent_snapshot_id: table.metadata.current_snapshot_id,
                sequence_number: (table.metadata.format_version >= FormatVersion::V2)
                    .then_some(sequence_number),
                timestamp_ms: now_ms(),
                manifest_list: SmolStr::new(table.location_of(METADATA_DIR, &list_name)),
                manifests: None,
                summary,
                schema_id: Some(table.metadata.current_schema_id),
                encryption_key_id: None,
                first_row_id,
                added_rows: assigned_rows,
            };

            let mut updated = table.metadata.clone();
            updated.set_current_snapshot(snapshot)?;
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
        // Automatic compaction is optional. Skipping it on v3 keeps a
        // successful data commit successful without rewriting retained rows
        // under fresh row IDs.
        if compacting || self.metadata.format_version >= FormatVersion::V3 {
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

    /// Reject rewrites that would assign fresh row IDs to retained v3 rows.
    fn require_row_id_preserving_rewrite(&self, operation: &'static str) -> Result<()> {
        if self.metadata.format_version >= FormatVersion::V3 {
            return Err(Error::iceberg(format_smolstr!(
                "{operation} is not supported for Iceberg format v3: rewritten rows cannot yet preserve their existing row IDs"
            )));
        }
        Ok(())
    }

    /// Write one partition's rows with the configured MIME type and describe it.
    ///
    /// The file's name carries the MIME type's own extension, so the handle's
    /// media type - which is what selects the encoder - agrees with the
    /// `file_format` the manifest will record. A Parquet file's statistics
    /// are read back from the footer that was just written; any other format
    /// has no footer this crate reads, so its statistics are measured from
    /// the batches before they are encoded.
    fn write_data_file(
        &self,
        write: &FileWrite<'_>,
        index: usize,
        values: &[Scalar],
        batches: Vec<RecordBatch>,
    ) -> Result<DataFile> {
        let snapshot_id = write.snapshot_id;
        let schema = write.schema;
        let spec = write.spec;
        let mime_type = &write.mime_type;
        let directory = spec.partition_path(values)?;
        let extension = mime_type
            .extension()
            .ok_or_else(|| not_encodable(mime_type))?;
        let name = format!("{index:05}-{snapshot_id}-{}.{extension}", uuid());
        let relative = if directory.is_empty() {
            format!("{DATA_DIR}/{name}")
        } else {
            format!("{DATA_DIR}/{directory}/{name}")
        };

        let mut handle = self.root.child_by_path(&relative)?;
        handle.set_media_type(crate::MediaType::new(mime_type.clone()));
        let options = handle
            .record_options()?
            .with_safe(false)
            .with_field(schema.clone());
        let mut file = if mime_type == &MimeType::PARQUET {
            let arrow_schema = crate::arrow::arrow_schema_from_field(schema)?;
            handle.overwrite_arrow_reader(
                crate::arrow::batch_reader(arrow_schema, batches),
                &options,
            )?;
            handle.flush()?;
            let statistics = crate::parquet::read_statistics(&handle)?;
            super::statistics::data_file(schema, &statistics)?
        } else {
            // The batches are measured before they are consumed by the write,
            // because this format's file carries no footer to read them from.
            let file = super::statistics::data_file_from_batches(schema, &batches)?;
            let arrow_schema = crate::arrow::arrow_schema_from_field(schema)?;
            handle.overwrite_arrow_reader(
                crate::arrow::batch_reader(arrow_schema, batches),
                &options,
            )?;
            handle.flush()?;
            file
        };
        file.file_path = SmolStr::new(self.location_of(DATA_DIR, &{
            if directory.is_empty() {
                name.clone()
            } else {
                format!("{directory}/{name}")
            }
        }));
        file.mime_type = mime_type.clone();
        file.file_size_in_bytes = i64::try_from(handle.size()).map_err(|_| {
            invalid(format_smolstr!(
                "expected a data file size fitting i64, got {}",
                handle.size()
            ))
        })?;
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
        let mut handle = self.root.child_by_path(&format!("{METADATA_DIR}/{name}"))?;
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
            manifest_length: i64::try_from(handle.size()).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a manifest size fitting i64, got {}",
                    handle.size()
                ))
            })?,
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
            added_files_count: Some(0),
            existing_files_count: Some(0),
            deleted_files_count: Some(0),
            added_rows_count: Some(0),
            existing_rows_count: Some(0),
            deleted_rows_count: Some(0),
            partitions: summaries(spec, schema, entries)?,
            key_metadata: None,
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
            let suffix = index.checked_add(1).ok_or_else(|| {
                invalid(SmolStr::new_static(
                    "cannot number more than usize::MAX carried manifests",
                ))
            })?;
            let existing_files_count = i32::try_from(entries.len()).map_err(|_| {
                invalid(format_smolstr!(
                    "expected fewer than {} existing files in a manifest, got {}",
                    i32::MAX,
                    entries.len()
                ))
            })?;
            let existing_rows_count = entries.iter().try_fold(0_i64, |total, entry| {
                total
                    .checked_add(entry.data_file.record_count)
                    .ok_or_else(|| {
                        invalid(format_smolstr!(
                            "expected existing row count fitting i64, overflowed at {:?}",
                            entry.data_file.file_path
                        ))
                    })
            })?;
            let name = format!("{snapshot_id}-m{suffix}.avro");
            let mut manifest = self.write_manifest_file(
                &name,
                schema,
                &spec,
                &entries,
                snapshot_id,
                sequence_number,
            )?;
            manifest.existing_files_count = Some(existing_files_count);
            manifest.existing_rows_count = Some(existing_rows_count);
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
/// no metadata document is re-read, [`crate::io::IOMedia::read_arrow_field`] is
/// [`Table::schema`] with its field identifiers and protocol metadata rather
/// than a shape lifted off decoded batches, and a
/// [`filter_partitions`](IORecordOptions::filter_partitions) pair prunes data
/// files through [`Table::plan`] instead of filtering rows after they were
/// decoded. The in-memory metadata stays current across commits, so
/// [`Table::current_snapshot`] and [`Table::metadata_version`] reflect a write made
/// through this surface without reopening anything.
///
/// One deliberate difference from the folder route: a filter naming a column
/// the schema does not declare is an error here, exactly as
/// [`Table::scan_where`] reports it, where a folder of leaves ignores a column
/// its batches do not carry. A table's schema is authoritative, so a filter it
/// cannot answer is a mistake worth naming rather than a row set worth
/// guessing.
impl<H: IOBase> IOBase for Table<H> {
    // `kind` is answered below: storage sees a folder, and this handle is
    // the table that folder holds.
    crate::delegate_iobase!(root: pread, pstream_bytes, pwrite, size, capacity, reserve,
        truncate, url, media_type, set_media_type, flush, parent, child_by_path,
        ls);

    /// A table folder is a container of its own kind: [`IOKind::Table`].
    ///
    /// The root folder underneath would answer [`IOKind::Directory`], which is
    /// true of the bytes and wrong about the value: the files below a table are
    /// its storage, not its contents, and a caller reads it through the record
    /// surface rather than by listing it. Saying so costs nothing - this handle
    /// is the table - where a plain folder handle has to find the metadata
    /// document before it can know.
    fn kind(&self) -> IOKind {
        IOKind::Table
    }

    /// A table is rows and columns, answered without touching storage.
    ///
    /// The folder route reaches the same answer by probing the location for a
    /// metadata document; holding the table skips the probe, exactly as it
    /// skips it for [`Self::read_arrow_field`] and the record methods.
    fn is_tabular(&self) -> bool {
        true
    }

    /// A table is never one whole byte value.
    fn is_atomic(&self) -> bool {
        false
    }

    /// Empty the table's *rows*, keeping the table.
    ///
    /// A table is a folder holding a metadata tree, so the two lifecycle
    /// methods mean genuinely different things here and neither is inherited
    /// silently from the folder route.
    ///
    /// Emptying a table cannot mean deleting files: the manifests would still
    /// name them, which is exactly the partial teardown a complete operation
    /// must not leave behind. So `clear` commits one snapshot that carries no
    /// data files - the table still exists afterwards, with its schema, its
    /// properties, and its whole history intact, and holding zero rows. That is
    /// what "empty the contents, keep the resource" is for a table format.
    ///
    /// A table with no snapshot is already empty and commits nothing, which is
    /// the same no-op success absence gets everywhere else on this pair.
    ///
    /// # Errors
    ///
    /// Returns the commit's failure, including a
    /// [`CommitConflict`](crate::Error) when concurrent writers exhaust the
    /// retries - an overwrite never rebases.
    fn clear(&mut self) -> Result<()> {
        if self.current_snapshot().is_none() {
            // Nothing has ever been written, so there is nothing to replace.
            return Ok(());
        }
        let schema = crate::arrow::arrow_schema_from_field(self.schema()?)?;
        self.commit_overwrite(crate::arrow::batch_reader(schema, []))
    }

    /// Delete the table completely: metadata, manifests, and data files.
    ///
    /// Complete removal of a table is removal of its whole location, exactly as
    /// for the folder it is - the metadata documents, the manifest lists, the
    /// manifests, and the data files all go, with nothing orphaned behind. This
    /// is deliberately *not* what [`Self::clear`] does: dropping a table is not
    /// emptying it, and the two must not be reachable by accident from each
    /// other.
    ///
    /// `recursive` behaves as it does on any container: without it, a table
    /// root that still holds a `metadata/` tree is refused naming the location,
    /// because a populated container is never silently recursed into.
    ///
    /// The in-memory [`Table`] value describes a table that no longer exists
    /// once this returns, so it must not be committed to afterwards; reopen the
    /// location instead. The handle itself stays usable and lazy, per the
    /// contract.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure, or a refusal naming the
    /// location when it still has children and `recursive` is not set.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.root.remove(recursive)
    }
}

impl<H: IOBase> crate::io::IOMedia for Table<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    /// Return the current snapshot's row count from table metadata.
    ///
    /// Tables written by this crate carry Iceberg's `total-records` summary,
    /// so the ordinary path does not open a manifest or data file. Imported
    /// snapshots that omit the optional summary fall back to manifest record
    /// counts; rows are still never decoded.
    fn row_size(&self) -> Result<u64> {
        let Some(snapshot) = self.current_snapshot() else {
            return Ok(0);
        };
        if let Some(total) = snapshot.summary_value("total-records") {
            return total.parse::<u64>().map_err(|_| {
                invalid(format_smolstr!(
                    "expected snapshot {} summary total-records to be a non-negative integer, got {total:?}",
                    snapshot.snapshot_id
                ))
            });
        }
        u64::try_from(self.plan(&[])?.record_count()?).map_err(|_| {
            invalid(format_smolstr!(
                "expected snapshot {} manifests to carry a non-negative record count",
                snapshot.snapshot_id
            ))
        })
    }

    /// Return the current table schema's width from the parsed metadata.
    fn column_size(&self) -> Result<usize> {
        Ok(self.schema()?.fields().len())
    }

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
        if let Some(field) = options.field() {
            return Ok(field.clone());
        }
        Ok(self.schema()?.clone().with_name(options.root_name()))
    }

    /// Scan the current snapshot, the options' filters answered by the plan.
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<BatchReader> {
        let filters = options.filter_partitions();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        let reader = self.scan_where(&pairs, options.field())?;
        // The limit wraps last, as on every handle, so it counts result rows
        // and a satisfied scan stops decoding data files.
        options.limit_arrow_reader(crate::io::select_reader(reader, options)?)
    }

    /// One overwrite commit scoped to the selected partitions.
    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        options.require_write_mode(crate::IOMode::Overwrite)?;
        let commit_row_size = options.require_commit_row_size()?;
        let stored = self.schema()?.clone();
        let (batches, _, _) = crate::io::prepare_arrow_write_onto(batches, options, Some(&stored))?;
        let filters: Vec<(String, String)> = options.filter_partitions().to_vec();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        if commit_row_size.is_none() {
            return self.commit_merge_where(&pairs, batches, &[], options.safe());
        }
        let schema = batches.schema();
        let mut commits = options.commit_arrow_readers(batches)?;
        let Some(first) = commits.next() else {
            return self.commit_merge_where(
                &pairs,
                crate::arrow::batch_reader(schema, []),
                &[],
                options.safe(),
            );
        };
        self.commit_merge_where(&pairs, first?, &[], options.safe())?;
        for commit in commits {
            self.commit_append(commit?)?;
        }
        Ok(())
    }

    /// One `append` snapshot, keeping every manifest the last one had.
    ///
    /// A limited write truncates data the caller offered here too: an append
    /// is a write.
    fn append_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
        options.require_write_mode(crate::IOMode::Append)?;
        let commit_row_size = options.require_commit_row_size()?;
        options.require_write_limits()?;
        if options.write_limit_is_zero() {
            return Ok(());
        }
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        let stored = self.schema()?.clone();
        let (batches, _, _) = crate::io::prepare_arrow_write_onto(batches, options, Some(&stored))?;
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        if commit_row_size.is_none() {
            return self.commit_append(batches);
        }
        for commit in options.commit_arrow_readers(batches)? {
            self.commit_append(commit?)?;
        }
        Ok(())
    }

    /// One keyed merge commit scoped to the selected partitions.
    fn merge_arrow_reader(&mut self, batches: BatchReader, options: &RecordOptions) -> Result<()> {
        options.require_write_mode(crate::IOMode::Merge)?;
        let commit_row_size = options.require_commit_row_size()?;
        options.require_write_limits()?;
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        let stored = self.schema()?.clone();
        let (batches, _, _) = crate::io::prepare_arrow_write_onto(batches, options, Some(&stored))?;
        let Some(batches) = crate::io::non_empty_arrow_reader(batches)? else {
            return Ok(());
        };
        let filters: Vec<(String, String)> = options.filter_partitions().to_vec();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        if commit_row_size.is_none() {
            return self.commit_merge_where(
                &pairs,
                batches,
                options.merge_by_names(),
                options.safe(),
            );
        }
        for commit in options.commit_arrow_readers(batches)? {
            self.commit_merge_where(&pairs, commit?, options.merge_by_names(), options.safe())?;
        }
        Ok(())
    }
}

/// What one [`Table::compact`] call did, in numbers a caller can assert on.
///
/// A compaction with nothing to do reports zeros, because it commits nothing.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Compaction {
    /// How many live data files were read and replaced.
    pub files_before: usize,
    /// How many data files the rewrite produced in their place.
    pub files_after: usize,
    /// The recorded size of the replaced files, in bytes.
    pub bytes_rewritten: i64,
}

impl Compaction {
    /// Return a deterministic hash of this complete compaction report.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
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
/// This crosses the [`Result`] boundary as [`Error::Conflict`], retaining this
/// value's [`Display`](std::fmt::Display) in the reported location.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct CommitConflict {
    /// The document version this writer expected to publish.
    pub expected_version: u32,
    /// How many observations found another writer's version instead.
    pub beaten: u32,
    /// The newest version observed before giving up.
    pub last_seen_version: u32,
}

impl CommitConflict {
    /// Return a deterministic hash of this complete conflict report.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }
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
        Error::conflict(
            "Iceberg metadata commit",
            "concurrent Iceberg metadata commit",
            value,
        )
    }
}

/// Count one conflict and reserve its randomized wait from the official total
/// retry-delay budget.
fn retry_wait_ms(
    settings: &CommitSettings,
    beaten: &mut u32,
    backoff_spent_ms: &mut u64,
    expected_version: u32,
    last_seen_version: u32,
) -> Result<u64> {
    *beaten = beaten.checked_add(1).ok_or_else(|| {
        invalid(SmolStr::new_static(
            "cannot count more than u32::MAX metadata commit conflicts",
        ))
    })?;
    let conflict = || {
        CommitConflict {
            expected_version,
            beaten: *beaten,
            last_seen_version,
        }
        .into()
    };
    if *beaten > settings.retries {
        return Err(conflict());
    }

    let wait = backoff_ms(
        *beaten - 1,
        settings.min_backoff_ms,
        settings.max_backoff_ms,
    );
    if !reserve_retry_backoff(backoff_spent_ms, wait, settings.total_timeout_ms) {
        return Err(conflict());
    }
    Ok(wait)
}

fn reserve_retry_backoff(spent_ms: &mut u64, wait_ms: u64, limit_ms: u64) -> bool {
    match spent_ms.checked_add(wait_ms) {
        Some(total) if total <= limit_ms => {
            *spent_ms = total;
            true
        }
        _ => false,
    }
}

/// The wait before one retry attempt, exponential with full jitter.
///
/// The window doubles from `min` per attempt and is capped at `max`; full
/// jitter draws uniformly from zero through that window so beaten writers do
/// not collide again in step.
fn backoff_ms(attempt: u32, min: u64, max: u64) -> u64 {
    use std::hash::{BuildHasher, Hasher};

    let window = min
        .saturating_mul(1_u64.checked_shl(attempt).unwrap_or(u64::MAX))
        .min(max);
    if window == 0 {
        return 0;
    }
    let state = std::collections::hash_map::RandomState::new();
    let mut hasher = state.build_hasher();
    hasher.write_i64(now_ms());
    hasher.write_u32(attempt);
    window
        .checked_add(1)
        .map_or_else(|| hasher.finish(), |width| hasher.finish() % width)
}

#[cfg(test)]
mod retry_tests {
    use super::{CommitSettings, backoff_ms, reserve_retry_backoff, retry_wait_ms};

    #[test]
    fn retry_budget_includes_its_exact_boundary_and_exhaustion_is_a_conflict() {
        let mut spent = 4;
        assert!(reserve_retry_backoff(&mut spent, 1, 5));
        assert_eq!(spent, 5);
        assert!(!reserve_retry_backoff(&mut spent, 1, 5));
        assert_eq!(spent, 5, "a refused wait does not consume budget");

        let settings = CommitSettings {
            retries: 1,
            min_backoff_ms: 0,
            max_backoff_ms: 0,
            total_timeout_ms: 0,
        };
        let mut beaten = 0;
        let mut spent = 0;
        assert_eq!(
            retry_wait_ms(&settings, &mut beaten, &mut spent, 2, 2).unwrap(),
            0
        );
        let error = retry_wait_ms(&settings, &mut beaten, &mut spent, 2, 2).unwrap_err();
        assert!(error.is_conflict(), "{error}");
    }

    #[test]
    fn full_jitter_never_exceeds_its_exponential_window() {
        for attempt in 0..8 {
            let cap = 10_u64.saturating_mul(1_u64 << attempt).min(100);
            for _ in 0..32 {
                assert!(backoff_ms(attempt, 10, 100) <= cap);
            }
        }
        assert_eq!(backoff_ms(4, 0, 100), 0);
    }
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

/// What every data file of one commit is written with.
#[derive(Clone)]
struct FileWrite<'a> {
    /// The snapshot the files belong to, which names them.
    snapshot_id: i64,
    /// The schema every batch was cast to.
    schema: &'a Field,
    /// The spec the partition tuples are written against.
    spec: &'a PartitionSpec,
    /// The resolved MIME type the files are encoded with.
    mime_type: MimeType,
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
            if matches!(value, Scalar::Null) {
                summary.contains_null = true;
                continue;
            }
            let Some(encoded) = single_value(value, child.dtype()) else {
                continue;
            };
            fold(&mut summary.lower_bound, &encoded, child.dtype(), true);
            fold(&mut summary.upper_bound, &encoded, child.dtype(), false);
        }
    }
    Ok(summaries)
}

/// Keep the smaller or larger of a running bound and one candidate.
fn fold(current: &mut Option<Vec<u8>>, candidate: &[u8], dtype: &DataType, minimum: bool) {
    match current {
        None => *current = Some(candidate.to_vec()),
        Some(held) => {
            let replace = compare_single(candidate, held, dtype).is_some_and(|ordering| {
                (minimum && ordering.is_lt()) || (!minimum && ordering.is_gt())
            });
            if replace {
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
    dtype: DataType,
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
                dtype: field.dtype().clone(),
                unbounded: id.is_none() || !is_portable(field.dtype()),
                has_null: false,
                lower: None,
                upper: None,
            };
            let mut has_non_null = false;
            for batch in batches {
                let Some(column) = batch.column_by_name(name) else {
                    continue;
                };
                bound.has_null = bound.has_null || column.null_count() > 0;
                has_non_null = has_non_null || column.null_count() < column.len();
                if bound.unbounded {
                    continue;
                }
                if let Some(encoded) = extreme(column, field, false)? {
                    fold(&mut bound.lower, &encoded, &bound.dtype, true);
                }
                if let Some(encoded) = extreme(column, field, true)? {
                    fold(&mut bound.upper, &encoded, &bound.dtype, false);
                }
            }
            // A non-null NaN is valid as a merge key but forbidden as an
            // Iceberg bound. If either extreme was therefore omitted, retain
            // every candidate file rather than treating the key as null-only.
            if has_non_null && (bound.lower.is_none() || bound.upper.is_none()) {
                bound.unbounded = true;
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
        let before = compare_single(upper, file_lower, &self.dtype);
        let after = compare_single(lower, file_upper, &self.dtype);
        match (before, after) {
            (Some(before), Some(after)) => !(before.is_lt() || after.is_gt()),
            // A malformed external bound cannot prove a file disjoint.
            _ => true,
        }
    }
}

#[cfg(test)]
mod key_bound_tests {
    use std::sync::Arc;

    use arrow_array::Float64Array;

    use super::*;

    #[test]
    fn malformed_external_bounds_cannot_exclude_a_merge_file() {
        let incoming = 37_i64.to_le_bytes().to_vec();
        let bound = KeyBound {
            id: 1,
            dtype: DataType::Int64,
            unbounded: false,
            has_null: false,
            lower: Some(incoming.clone()),
            upper: Some(incoming),
        };
        let file = DataFile {
            record_count: 1,
            lower_bounds: vec![(1, vec![0; 3])],
            upper_bounds: vec![(1, vec![0; 9])],
            null_value_counts: vec![(1, 0)],
            ..DataFile::default()
        };
        assert!(bound.may_hold(&file));

        let incoming = 1.5_f64.to_le_bytes().to_vec();
        let float_bound = KeyBound {
            id: 1,
            dtype: DataType::Float64,
            unbounded: false,
            has_null: false,
            lower: Some(incoming.clone()),
            upper: Some(incoming),
        };
        let nan = f64::NAN.to_le_bytes().to_vec();
        let file = DataFile {
            record_count: 1,
            lower_bounds: vec![(1, nan.clone())],
            upper_bounds: vec![(1, nan)],
            null_value_counts: vec![(1, 0)],
            ..DataFile::default()
        };
        assert!(float_bound.may_hold(&file));
    }

    #[test]
    fn generated_nan_merge_bounds_are_conservatively_unbounded() {
        let mut schema = DataType::from_fields([DataType::Float64.required_field("ratio")])
            .unwrap()
            .required_field("row");
        crate::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let arrow_schema = crate::arrow::arrow_schema_from_field(&schema).unwrap();
        let batch = RecordBatch::try_new(
            arrow_schema,
            vec![Arc::new(Float64Array::from(vec![1.5, f64::NAN]))],
        )
        .unwrap();

        let bounds = KeyBounds::of(&[batch], &schema, &["ratio".to_owned()]).unwrap();
        assert!(bounds.columns[0].unbounded);
    }

    #[test]
    fn partition_summaries_omit_nan_bounds() {
        let mut schema = DataType::from_fields([DataType::Float64.required_field("ratio")])
            .unwrap()
            .required_field("row");
        crate::iceberg::assign_field_ids(&mut schema, 1).unwrap();
        let spec = PartitionSpec::identity(0, &schema, &["ratio"]).unwrap();
        let entry = |value| {
            ManifestEntry::added(
                1,
                DataFile {
                    record_count: 1,
                    partition: vec![value],
                    ..DataFile::default()
                },
            )
        };

        let only_nan = summaries(&spec, &schema, &[entry(Scalar::from(f64::NAN))]).unwrap();
        assert!(only_nan[0].lower_bound.is_none());
        assert!(only_nan[0].upper_bound.is_none());

        let finite = Scalar::from(1.5_f64);
        let encoded = single_value(&finite, &DataType::Float64).unwrap();
        let mixed = summaries(
            &spec,
            &schema,
            &[entry(Scalar::from(f64::NAN)), entry(finite)],
        )
        .unwrap();
        assert_eq!(mixed[0].lower_bound.as_deref(), Some(encoded.as_slice()));
        assert_eq!(mixed[0].upper_bound.as_deref(), Some(encoded.as_slice()));
    }
}

/// Encode the smallest or largest value one column holds.
///
/// The extreme is found by a bounded sort rather than a scan of decoded values:
/// one index is all a bound needs, and asking Arrow for it keeps the work in the
/// kernel instead of in a per-row conversion.
pub(super) fn extreme(
    column: &ArrayRef,
    field: &Field,
    descending: bool,
) -> Result<Option<Vec<u8>>> {
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
    let slice = column.slice(row as usize, 1);
    let scalar = crate::arrow::scalar_value(&field.clone().with_nullable(true), slice.as_ref())
        .map_err(|error| invalid(format_smolstr!("{error}")))?;
    Ok(single_value(&scalar, field.dtype()))
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
    partition: &Field,
) -> Result<Vec<(Vec<Scalar>, Vec<RecordBatch>)>> {
    let mut groups: Vec<(Vec<Scalar>, Vec<RecordBatch>)> = Vec::new();
    let mut index: HashMap<Vec<Scalar>, usize> = HashMap::new();
    let transforms = spec.write_transforms(schema, partition)?;

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

        for (values, rows) in row_groups(&batch, &transforms)? {
            let position = match index.get(&values) {
                Some(position) => *position,
                None => {
                    groups.push((values.clone(), Vec::new()));
                    index.insert(values, groups.len() - 1);
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

/// Group row indices by their computed, typed partition tuple.
fn row_groups(
    batch: &RecordBatch,
    transforms: &[super::partition::PartitionTransform],
) -> Result<Vec<(Vec<Scalar>, Vec<u32>)>> {
    let mut order: Vec<(Vec<Scalar>, Vec<u32>)> = Vec::new();
    let mut seen: HashMap<Vec<Scalar>, usize> = HashMap::new();
    for row in 0..batch.num_rows() {
        let row = u32::try_from(row).map_err(|_| {
            invalid(format_smolstr!(
                "expected at most {} rows in one partitioning batch, got {}",
                u32::MAX,
                batch.num_rows()
            ))
        })?;
        let values = tuple_at(batch, transforms, row)?;
        match seen.get(&values) {
            Some(position) => order[*position].1.push(row),
            None => {
                seen.insert(values.clone(), order.len());
                order.push((values, vec![row]));
            }
        }
    }
    Ok(order)
}

/// Read one row's partition tuple out of a batch.
fn tuple_at(
    batch: &RecordBatch,
    transforms: &[super::partition::PartitionTransform],
    row: u32,
) -> Result<Vec<Scalar>> {
    let mut values = Vec::with_capacity(transforms.len());
    for transform in transforms {
        let value = source_value(batch, transform, row as usize)?;
        values.push(transform.partition_value(value)?);
    }
    Ok(values)
}

/// Read one primitive source through top-level and nested Struct arrays.
fn source_value(
    batch: &RecordBatch,
    transform: &super::partition::PartitionTransform,
    row: usize,
) -> Result<Scalar> {
    let (first, nested) = transform.path().split_first().ok_or_else(|| {
        invalid(SmolStr::new_static(
            "expected a non-empty partition source path",
        ))
    })?;
    let mut column = batch.column_by_name(first).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected a partition source column {first:?} in the batch, got none"
        ))
    })?;
    if column.is_null(row) {
        return Ok(Scalar::Null);
    }
    for name in nested {
        let parent = column
            .as_any()
            .downcast_ref::<StructArray>()
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected a struct on the partition source path before {name:?}, got {}",
                    column.data_type()
                ))
            })?;
        column = parent.column_by_name(name).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a nested partition source column {name:?}, got none"
            ))
        })?;
        if column.is_null(row) {
            return Ok(Scalar::Null);
        }
    }
    let slice = column.slice(row, 1);
    crate::arrow::scalar_value(
        &transform.source().clone().with_nullable(true),
        slice.as_ref(),
    )
    .map_err(|error| invalid(format_smolstr!("{error}")))
}

/// Return the metadata document with the highest version, exact name, and number.
///
/// A folder that holds none is `None` rather than an error, because that is the
/// question "is this a table" and the answer "no" is not a failure.
fn find_metadata(metadata_dir: &Holder) -> Result<Option<(u32, SmolStr, Scalar)>> {
    // A folder that is not a table has no metadata directory at all, and the
    // laziness contract makes that a handle that simply is not a container.
    if !metadata_dir.is_container() {
        return Ok(None);
    }

    let hint = metadata_dir.child_by_path(VERSION_HINT)?;
    let hinted_version = String::from_utf8_lossy(&hint.read_all_bytes()?)
        .trim()
        .parse::<u32>()
        .ok();

    // Prefer the conventional Hadoop filename named by a usable hint.
    if let Some(version) = hinted_version {
        for name in [
            format!("v{version}.metadata.json"),
            format!("v{version}.gz.metadata.json"),
        ] {
            let document = metadata_dir.child_by_path(&name)?;
            let bytes = document.read_all_bytes()?;
            if !bytes.is_empty() {
                return Ok(Some((
                    version,
                    SmolStr::new(name),
                    parse_metadata_bytes(&bytes)?,
                )));
            }
        }
    }

    // No conventional hint target: inspect exact filenames. A UUID filename
    // is never rewritten into a synthetic `vN` path.
    let mut candidates: Vec<(u32, SmolStr, Holder)> = Vec::new();
    for entry in metadata_dir.ls(false, false) {
        let entry = entry?;
        let Some(name) = entry
            .url()
            .and_then(|url| url.file_name().map(ToOwned::to_owned))
        else {
            continue;
        };
        let Some(version) = metadata_version_from_name(&name) else {
            continue;
        };
        candidates.push((version, SmolStr::new(name), entry));
    }

    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_by(|left, right| (left.0, &left.1).cmp(&(right.0, &right.1)));
    let highest_version = candidates
        .iter()
        .map(|candidate| candidate.0)
        .max()
        .ok_or_else(|| invalid(SmolStr::new_static("expected a metadata candidate")))?;
    let chosen_version = hinted_version
        .filter(|hint| candidates.iter().any(|candidate| candidate.0 == *hint))
        .unwrap_or(highest_version);
    // A catalog normally stores the exact UUID filename. This catalog-free
    // surface has only a numeric hint, so concurrent same-version candidates
    // resolve by exact filename order rather than backend listing order.
    let chosen = candidates
        .iter()
        .rposition(|candidate| candidate.0 == chosen_version)
        .ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a matching metadata candidate",
            ))
        })?;
    let (version, name, document) = candidates.swap_remove(chosen);
    let bytes = document.read_all_bytes()?;
    Ok(Some((version, name, parse_metadata_bytes(&bytes)?)))
}

/// Parse the version prefix of Hadoop (`v3`) and official UUID (`00003-id`) names.
fn metadata_version_from_name(name: &str) -> Option<u32> {
    let stem = name.strip_suffix(".metadata.json")?;
    let stem = stem.strip_suffix(".gz").unwrap_or(stem);
    if let Some(version) = stem.strip_prefix('v') {
        return if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
            None
        } else {
            version.parse().ok()
        };
    }

    format!("/metadata/{name}")
        .parse::<iceberg_official::MetadataLocation>()
        .ok()?;
    let (version, _) = stem.split_once('-')?;
    if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
        None
    } else {
        version.parse().ok()
    }
}

#[cfg(test)]
mod metadata_name_tests {
    use super::metadata_version_from_name;

    #[test]
    fn accepts_exact_hadoop_and_official_metadata_names() {
        for (name, version) in [
            ("v3.metadata.json", 3),
            ("v00003.gz.metadata.json", 3),
            (
                "00003-2cd22b57-5127-4198-92ba-e4e67c79821b.metadata.json",
                3,
            ),
            ("9-2cd22b57-5127-4198-92ba-e4e67c79821b.gz.metadata.json", 9),
        ] {
            assert_eq!(metadata_version_from_name(name), Some(version), "{name}");
        }
    }

    #[test]
    fn rejects_metadata_lookalikes() {
        for name in [
            "v.metadata.json",
            "v2backup.metadata.json",
            "vv2.metadata.json",
            "00002-not-a-uuid.metadata.json",
            "00002-2cd22b57-5127-4198-92ba-e4e67c79821b.extra.metadata.json",
            "00002-2cd22b57-5127-4198-92ba-e4e67c79821b.zst.metadata.json",
            "2.metadata.json",
            "v2.json",
        ] {
            assert_eq!(metadata_version_from_name(name), None, "{name}");
        }
    }
}

/// List exact metadata filenames at one version, deterministically.
fn metadata_names_at_version(metadata_dir: &Holder, version: u32) -> Result<Vec<SmolStr>> {
    let mut names = Vec::new();
    for entry in metadata_dir.ls(false, false) {
        let entry = entry?;
        let Some(name) = entry.url().and_then(|url| url.file_name()) else {
            continue;
        };
        if metadata_version_from_name(name) == Some(version) {
            names.push(SmolStr::new(name));
        }
    }
    names.sort();
    Ok(names)
}

/// Decode Iceberg metadata exactly as the official reader does: gzip is
/// detected from its magic bytes, independent of the filename.
fn parse_metadata_bytes(bytes: &[u8]) -> Result<Scalar> {
    let decoded = if bytes.starts_with(&[0x1f, 0x8b]) {
        crate::gzip::load(bytes)?
    } else {
        bytes.to_vec()
    };
    crate::json::from_bytes(&decoded)
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

/// Spell one location the single way, so that two spellings of one place match.
///
/// Separators are normalized because an implementation that wrote the table on
/// Windows may have spelled its own location with backslashes. An empty URI
/// authority is spelled out because `file:/warehouse` and `file:///warehouse`
/// name the same place: Java's URI normalizer - so every Hadoop and Spark
/// writer - drops it, while this crate's URLs keep it, and a table written by
/// one and committed into by the other carries both spellings at once.
fn normalized_location(location: &str) -> String {
    let normalized = location.replace('\\', "/");
    let Some((scheme, rest)) = normalized.split_once(':') else {
        return normalized;
    };
    // A one-letter scheme is a Windows drive letter, and `//` already spells an
    // authority - empty or not. Everything else is `scheme:/path`, the form that
    // is missing the empty authority.
    let scheme_shaped = scheme.len() > 1
        && scheme.starts_with(|first: char| first.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'));
    if !scheme_shaped || !rest.starts_with('/') || rest.starts_with("//") {
        return normalized;
    }
    format!("{scheme}://{rest}")
}

/// Turn one absolute location into a name relative to the table's folder.
fn relative_location(base: &str, location: &str) -> Result<String> {
    let normalized_base = normalized_location(base);
    let normalized_base = normalized_base.trim_end_matches('/');
    let normalized = normalized_location(location);
    if normalized == normalized_base {
        return Ok(String::new());
    }
    if let Some(rest) = normalized
        .strip_prefix(normalized_base)
        .and_then(|rest| rest.strip_prefix('/'))
    {
        return Ok(rest.to_owned());
    }
    Err(invalid(format_smolstr!(
        "expected a location inside the table at {normalized_base:?}, got {location:?}"
    )))
}

#[cfg(test)]
mod location_tests {
    use super::relative_location;

    #[test]
    fn an_empty_uri_authority_is_the_same_place_spelled_shorter() {
        // Whichever writer spelled which: the crate writes the table location
        // with the authority, Spark commits its manifest lists without it.
        for (base, location) in [
            (
                "file:///warehouse/db/t",
                "file:/warehouse/db/t/metadata/snap-1.avro",
            ),
            (
                "file:/warehouse/db/t",
                "file:///warehouse/db/t/metadata/snap-1.avro",
            ),
            (
                "file:/warehouse/db/t",
                "file:/warehouse/db/t/metadata/snap-1.avro",
            ),
        ] {
            assert_eq!(
                relative_location(base, location).unwrap(),
                "metadata/snap-1.avro",
                "{base} -> {location}"
            );
        }
    }

    #[test]
    fn a_real_authority_and_a_windows_drive_are_left_alone() {
        assert_eq!(
            relative_location("s3://bucket/db/t", "s3://bucket/db/t/data/0.parquet").unwrap(),
            "data/0.parquet"
        );
        assert_eq!(
            relative_location("C:\\warehouse\\t", "C:\\warehouse\\t\\data\\0.parquet").unwrap(),
            "data/0.parquet"
        );
        // A neighbour is not a child, however either side is spelled.
        relative_location("file:///warehouse/db/t", "file:/warehouse/db/other/x").unwrap_err();
        relative_location("s3://bucket/db/t", "s3://other/db/t/x").unwrap_err();
    }
}

/// Refuse a data-file MIME type this build has no encoder for, by name.
///
/// The error is typed and names both the property key and the format, so a
/// caller who asked for ORC - or for Parquet in a build without the feature -
/// learns which setting to change rather than silently getting Parquet.
fn require_encodable(mime_type: &MimeType) -> Result<()> {
    if !is_iceberg_mime_type(mime_type) {
        return Err(not_encodable(mime_type));
    }
    RecordOptions::for_mime_type(mime_type)
        .map(|_| ())
        .map_err(|_| not_encodable(mime_type))
}

fn next_sequence_number(metadata: &TableMetadata) -> Result<i64> {
    if metadata.format_version == FormatVersion::V1 {
        return Ok(0);
    }
    metadata.last_sequence_number.checked_add(1).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected last-sequence-number below {}, got {}",
            i64::MAX,
            metadata.last_sequence_number
        ))
    })
}

fn checked_file_sum(
    files: &[DataFile],
    value: impl Fn(&DataFile) -> i64,
    name: &str,
) -> Result<i64> {
    files.iter().try_fold(0_i64, |total, file| {
        total.checked_add(value(file)).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected total {name} fitting i64, overflowed at {:?}",
                file.file_path
            ))
        })
    })
}

fn checked_manifest_total_i64(
    manifests: &[ManifestFile],
    values: impl Fn(&ManifestFile) -> (Option<i64>, Option<i64>),
    name: &str,
) -> Result<Option<i64>> {
    let mut total = 0_i64;
    for manifest in manifests {
        let (Some(first), Some(second)) = values(manifest) else {
            return Ok(None);
        };
        total = total
            .checked_add(first)
            .and_then(|value| value.checked_add(second))
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected total {name} fitting i64, overflowed at {:?}",
                    manifest.manifest_path
                ))
            })?;
    }
    Ok(Some(total))
}

fn checked_manifest_total_i32(
    manifests: &[ManifestFile],
    values: impl Fn(&ManifestFile) -> (Option<i32>, Option<i32>),
    name: &str,
) -> Result<Option<i32>> {
    let mut total = 0_i32;
    for manifest in manifests {
        let (Some(first), Some(second)) = values(manifest) else {
            return Ok(None);
        };
        total = total
            .checked_add(first)
            .and_then(|value| value.checked_add(second))
            .ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected total {name} fitting i32, overflowed at {:?}",
                    manifest.manifest_path
                ))
            })?;
    }
    Ok(Some(total))
}

/// The typed refusal [`require_encodable`] reports.
fn not_encodable(mime_type: &MimeType) -> Error {
    Error::InvalidMetadataValue {
        key: SmolStr::new_static(IcebergOptions::DATA_MIME_TYPE_KEY),
        reason: format_smolstr!(
            "expected a data MIME type this build encodes (application/vnd.apache.parquet or application/avro), got {mime_type}"
        ),
    }
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

/// The predicate one set of `(column, value)` pairs spells about a table.
///
/// The pairs are the older, narrower spelling of a filter and they stay: they
/// build an expression here and are answered by the one planner and the one
/// evaluator, so there is no second filter language behind them. A pair naming
/// a column the schema does not declare is left in as written, so binding it
/// reports the column rather than silently ignoring it.
fn pairs_predicate(schema: &Field, pairs: &[(&str, &str)]) -> crate::Expression {
    crate::Expression::all(pairs.iter().map(|(column, value)| {
        schema.get_field_by_name(column).map_or_else(
            || crate::Expression::column(*column).eq(crate::Expression::literal(*value)),
            |field| crate::Expression::partition_equals(column, value, field.dtype()),
        )
    }))
}
