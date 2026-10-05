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
//! let folder = LocalFolder::new(LocalFolder::temporary()?.path()?.join("trades"))?;
//! let spec = PartitionSpec::identity(0, &schema, &["venue"])?;
//! let table = IcebergTable::create(folder, FormatVersion::V2, schema, spec)?;
//!
//! // A table with no snapshot yet reads as no rows, never as a failure.
//! assert!(table.current_snapshot()?.is_none());
//! assert_eq!(table.scan(None)?.count(), 0);
//! # Ok(())
//! # }
//! ```
//!
//! # What a commit costs
//!
//! Committing means writing a new metadata document, so an append writes at
//! least one Parquet file per partition - more when a partition's rows exceed
//! [`IcebergTable::target_file_size_bytes`] - one manifest, one manifest list, and one
//! metadata JSON. Nothing is mutated in place, which is what makes the previous
//! snapshot still readable afterwards.
//!
//! Every data file holds one partition, with its rows in the table's default
//! sort order - the partition's source columns unless the creator declared
//! another with [`IcebergTable::create_sorted`] - so the bounds a file records on
//! those columns are tight and a filter on them prunes files rather than
//! reading them. Partition groups are written on up to
//! [`IcebergOptions::write_parallelism`] threads, and the manifest lists
//! their files in group order whatever order the threads finished in.
//!
//! # Partition keys are the primary keys
//!
//! A merge joins on the identity partition columns first and the caller's
//! match key after them, so a row can only update a row of its own partition,
//! and a merge into one partition of a thousand reads that partition's files
//! and no other. A merge that names no key at all is keyed by the partition
//! alone: every partition the incoming rows fall in is replaced by them, and
//! every other partition is carried untouched.
//!
//! # Concurrent writers
//!
//! Commits are optimistic. Before publishing version N+1 a commit re-checks
//! the current version with the same lookup [`IcebergTable::open`] uses; a commit
//! that finds itself beaten *rebases* when that is safe - it reloads the
//! winner's document and re-applies its own intent on top, with exponential
//! jittered backoff between attempts, bounded by
//! [`IcebergOptions::commit_retries`] and
//! [`IcebergOptions::commit_total_timeout_ms`] - and otherwise reports a
//! [`CommitConflict`] naming both versions. An append and a metadata-only
//! change rebase; [`IcebergTable::commit_overwrite_where`], [`IcebergTable::commit_merge_where`], and
//! [`IcebergTable::compact`] cannot, because they planned against files a concurrent
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
//! A table opened or created through a [`MetadataPointer`]
//! ([`IcebergTable::open_pointed`], [`IcebergTable::create_pointed`]) has
//! that compare-and-swap: the pointer names its current document and
//! publishes the next one on condition it still stands where the last
//! reading left it, so nothing is listed, no hint is written, and a commit
//! that lost is told rather than overwritten. Nothing a pointed commit wrote
//! is removed when it fails: its store need not take a delete.
//!
//! # Branches and tags
//!
//! [`IcebergTable::create_branch`], [`IcebergTable::create_tag`], [`IcebergTable::remove_snapshot_ref`],
//! [`IcebergTable::fast_forward_branch`], and [`IcebergTable::expire_snapshots`] are thin wrappers
//! over [`TableMetadata`]'s ref vocabulary, each committed through the same
//! retrying [`IcebergTable::commit_metadata_changes`]. Writing *to* a branch other than
//! `main` remains future work, because a commit's parent is currently always
//! the table's current snapshot.

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasherDefault, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use arrow_array::{Array, ArrayRef, RecordBatch, StructArray, UInt32Array};
use arrow_row::{Row, RowConverter, SortField};
use smol_str::{SmolStr, format_smolstr};

use super::manifest::{
    DataFile, FieldSummary, ManifestContent, ManifestEntry, ManifestFile, is_iceberg_mime_type,
    read_manifest_list, read_v1_direct_manifest_file, write_manifest, write_manifest_list,
};
use super::metadata::{FormatVersion, SortOrder, TableMetadata, now_ms, uuid};
use super::options::{CommitSettings, IcebergOptions, WriteSettings, WriteStaging};
use super::partition::PartitionSpec;
use super::pointer::{MetadataPointer, PointerState};
use super::scan::{ScanPart, ScanPlan, ScanTask, identity_column};
use super::snapshot::{Snapshot, SnapshotRef};
use super::staging::{Staging, container, leaf, sized};
use super::value::{compare_single, is_portable, single_value};
use crate::arrow::BatchReader;
use crate::cast::{ArrowCastOptions, ArrowCastPlan, Deferred, PlanCache};
use crate::expression::Projection;
use crate::holder::Holder;
use crate::media::{Cadence, IORecordOptions, RecordOptions};
use crate::{ChunkedSerie, IOBase, IOMedia, Serie, SpillOptions};
use crate::{
    DataType, Error, Field, Filter, IOKind, MimeType, Result, Scalar, Selector, StructType, Term,
};
use crate::{Handle, IntoObjectPath, ObjectValue, Properties, TableValue};

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
/// data files - is a child of it. The relationship runs both ways: a `IcebergTable`
/// is itself an [`IOBase`], so the generic record surface works on the value
/// directly - see the trait implementation for what each method answers.
#[derive(Debug)]
pub struct IcebergTable<H: IOBase> {
    /// The folder the table lives in.
    root: H,
    /// Its warehouse path: the catalog's name down to its own, or its
    /// folder's name alone for a table opened by its location.
    path: Vec<SmolStr>,
    /// What was stated for it at construction or registration.
    stated: Properties,
    /// Its parent's effective properties.
    inherited: Properties,
    /// What its store says it is, when it says anything.
    description: Option<String>,
    /// The current metadata document, read once on first use.
    opened: OnceLock<Opened>,
    /// An explicit options override the resolvers consult before properties.
    options: Option<IcebergOptions>,
    /// What names the current document, when the folder does not: `None`
    /// is the folder contract - the hint, else a listing of `metadata/`.
    /// Shared by every clone, and never part of the table's identity.
    pointer: Option<Arc<dyn MetadataPointer>>,
}

/// The current metadata document of a table, as read: what every reader
/// and every commit works from.
#[derive(Clone, Debug)]
struct Opened {
    /// The parsed current metadata document.
    metadata: TableMetadata,
    /// The version number of the metadata document that was last written.
    version: u32,
    /// The exact discovered metadata filename, including UUID and compression.
    metadata_file_name: SmolStr,
    /// Where the pointer stood when the document was read or published, for
    /// a pointed table; `None` under the folder contract. A pointed table not
    /// yet published states no location.
    pointed: Option<PointerState>,
}

impl IcebergTable<Handle> {
    /// Open the table a location names, under `properties`.
    ///
    /// `location` is a [`Url`](crate::Url) or any identifier that locates
    /// one ([`Uri::locator`](crate::Uri::locator)). The table is rooted on
    /// the handle [`Holder::from_url`] opens for it under `properties`,
    /// resolved on first use and again by every clone, and its current
    /// document is read as [`Self::open`] reads it. The table states
    /// `properties` less the ones the store's own reader took - who signs,
    /// where the store is, how it is addressed - so a credential reaches the
    /// storage alone and is neither listed by [`ObjectValue::properties`]
    /// nor printed by `Debug`; a local path takes none.
    ///
    /// Under the `s3tables` feature an `s3tables://<bucket>/<namespace>/<table>`
    /// location, or a table's ARN, is the table that bucket keeps: its
    /// document named by the service, every commit published through the
    /// control plane. Who signs, the region and the endpoint are read off
    /// `properties` as the bucket's catalog reads them
    /// ([`Catalog::from_url`](crate::Catalog::from_url)) and kept by its
    /// session alone, so such a table states none and inherits the others. It
    /// costs one `GetTableMetadataLocation` (one `GetTable` for an ARN), plus
    /// the one `ListTableBuckets` that finds the bucket where neither the
    /// location nor a property states its ARN or its account.
    ///
    /// ```
    /// use yggdryl::iceberg::IcebergTable;
    /// use yggdryl::{DataType, Properties, StructType, Url};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let folder = std::env::temp_dir().join(format!("yggdryl-from-url-doc-{}", std::process::id()));
    /// let location = Url::from_path(&folder)?;
    /// let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    ///     .required_field("row");
    /// IcebergTable::create_from_url(&location, &Properties::new(), None, schema, None)?;
    ///
    /// let table = IcebergTable::from_url(&location, &Properties::new())?;
    /// assert_eq!(table.schema()?.field_len(), 1);
    /// assert!(table.current_snapshot()?.is_none());
    /// # std::fs::remove_dir_all(&folder)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the identifier's own refusal when it names no location, the
    /// [`Self::open`] failures, and for a table bucket's location
    /// [`Error::Absent`] when the bucket keeps no such table,
    /// [`Error::InvalidRecord`] at `$.url` when the location spells more than
    /// a namespace and a table below its bucket or names the bucket or a
    /// namespace rather than a table, and the service's own refusal.
    pub fn from_url(location: impl AsRef<crate::Uri>, properties: &Properties) -> Result<Self> {
        let location = location.as_ref();
        #[cfg(feature = "s3tables")]
        if location.names_s3_tables() {
            return match crate::s3tables::locate(location, properties)? {
                crate::Object::Table(crate::Table::Iceberg(table)) => Ok(*table),
                other => Err(crate::s3tables::not_a_table(
                    location,
                    ObjectValue::kind(&other),
                )),
            };
        }
        let root = rooted_at(location, properties)?;
        Ok(Self::open(root)?.stating(properties))
    }

    /// Create the table a location names, under `properties`, writing its
    /// first metadata document.
    ///
    /// The location and the properties are read as [`Self::from_url`] reads
    /// them, and the schema is stored as Iceberg expresses it - a layout the
    /// format does not state rewritten to the one it does - numbered above
    /// its highest identifier. A `spec` left out is the schema's own
    /// ([`PartitionSpec::from_schema`]); a `version` left out is the
    /// `format-version` property, else 3 for a nanosecond timestamp, a
    /// variant or an unknown column and 2 otherwise. A hand-built `spec`
    /// names its columns by identifier, so its schema is numbered first
    /// ([`super::assign_field_ids`]).
    ///
    /// Under the `s3tables` feature an `s3tables://<bucket>/<namespace>/<table>`
    /// location registers the table in its table bucket and costs
    /// `CreateTable` (with no schema), `GetTableMetadataLocation`, one
    /// `PutObject` and `UpdateTableMetadataLocation`; a namespace the bucket
    /// lacks is created on the way (the refused `CreateTable`, one
    /// `CreateNamespace`, the creation again), which a catalog's own
    /// `create_table` never does. A table's ARN names a table that exists, and
    /// is refused.
    ///
    /// ```
    /// use yggdryl::iceberg::{FormatVersion, IcebergTable};
    /// use yggdryl::{DataType, Properties, StructType, Url};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let folder = std::env::temp_dir().join(format!("yggdryl-create-from-url-doc-{}", std::process::id()));
    /// let schema = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::utf8().nullable_field("venue").with_partition(true),
    /// ])?)
    /// .required_field("row");
    ///
    /// // Neither a version nor a spec stated: the schema's own.
    /// let table = IcebergTable::create_from_url(
    ///     Url::from_path(&folder)?,
    ///     &Properties::new(),
    ///     None,
    ///     schema,
    ///     None,
    /// )?;
    /// assert_eq!(table.metadata()?.format_version(), FormatVersion::V2);
    /// assert_eq!(table.metadata()?.default_spec()?.fields.len(), 1);
    /// # std::fs::remove_dir_all(&folder)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the [`Self::create`] failures - a conflict where a table is
    /// already there - the schema's refusal of a column no Iceberg type
    /// holds, [`Error::InvalidRecord`] at `$.with.format-version` for a
    /// version no format has and at `$.url` for a table bucket's location
    /// that does not name a table by its namespace and its name, and the
    /// service's own refusal.
    pub fn create_from_url(
        location: impl AsRef<crate::Uri>,
        properties: &Properties,
        version: Option<FormatVersion>,
        schema: Field,
        spec: Option<PartitionSpec>,
    ) -> Result<Self> {
        let location = location.as_ref();
        #[cfg(feature = "s3tables")]
        if location.names_s3_tables() {
            return crate::s3tables::create(location, properties, version, &schema, spec);
        }
        Self::create_rooted(
            rooted_at(location, properties)?,
            properties,
            version,
            &schema,
            spec,
        )
    }

    /// Open the table a location names if it is there, creating it
    /// otherwise: [`Self::from_url`], and on absence
    /// [`Self::create_from_url`]. `version`, `schema` and `spec` describe
    /// only the table a create would make.
    ///
    /// The location is resolved once for both halves, so a miss costs the
    /// open's one refused request and nothing a second time. A table's ARN
    /// is never created: one the bucket has no table of is
    /// [`Error::Absent`], as [`Self::from_url`] answers it.
    ///
    /// ```
    /// use yggdryl::iceberg::IcebergTable;
    /// use yggdryl::{DataType, Properties, StructType, Url};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let folder = std::env::temp_dir().join(format!("yggdryl-open-or-create-doc-{}", std::process::id()));
    /// let location = Url::from_path(&folder)?;
    /// let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    ///     .required_field("row");
    ///
    /// let created =
    ///     IcebergTable::open_or_create_from_url(&location, &Properties::new(), None, schema.clone(), None)?;
    /// let opened =
    ///     IcebergTable::open_or_create_from_url(&location, &Properties::new(), None, schema, None)?;
    /// assert_eq!(opened.metadata_file_name()?, created.metadata_file_name()?);
    /// # std::fs::remove_dir_all(&folder)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create_from_url(
        location: impl AsRef<crate::Uri>,
        properties: &Properties,
        version: Option<FormatVersion>,
        schema: Field,
        spec: Option<PartitionSpec>,
    ) -> Result<Self> {
        let location = location.as_ref();
        #[cfg(feature = "s3tables")]
        if location.names_s3_tables() {
            return crate::s3tables::open_or_create(location, properties, version, &schema, spec);
        }
        // One locate is the whole existence question: the handle comes back
        // from the miss, so the create resolves nothing a second time.
        match Self::locate_keeping(rooted_at(location, properties)?)? {
            Ok(table) => Ok(table.stating(properties)),
            Err(root) => Self::create_rooted(root, properties, version, &schema, spec),
        }
    }

    /// Create the table on `root`, laid out by [`super::create_layout`],
    /// stating `properties`.
    fn create_rooted(
        root: Handle,
        properties: &Properties,
        version: Option<FormatVersion>,
        schema: &Field,
        spec: Option<PartitionSpec>,
    ) -> Result<Self> {
        let (schema, spec, version) = super::create_layout(schema, properties, version, spec)?;
        Ok(Self::create(root, version, schema, spec)?.stating(properties))
    }

    /// The table stating `properties` less the ones its root's store reads
    /// for itself ([`Holder::is_backend_property`]), which reach the storage
    /// alone. The root's own bag is left as rooted, unlike
    /// [`with_properties`](Self::with_properties).
    fn stating(mut self, properties: &Properties) -> Self {
        self.stated = match self.root.url() {
            Some(url) => properties
                .iter()
                .filter(|(name, _)| !Holder::is_backend_property(url, name))
                .collect(),
            None => properties.clone(),
        };
        self
    }

    /// Return this table with stated properties, which its storage opens
    /// with and [`ObjectValue::properties`] answers over the stored ones.
    ///
    /// A table a warehouse holds states properties, and one opened by its
    /// location states them less what its store read ([`Self::from_url`]);
    /// one opened over a handle in hand was opened by whoever handed the
    /// handle over, and its settings are its [`options`](Self::options).
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self.root
            .set_properties(self.stated.inherit(&self.inherited));
        self
    }

    /// The table with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(mut self, parent: &Properties) -> Self {
        self.inherited = parent.clone();
        self.root
            .set_properties(self.stated.inherit(&self.inherited));
        self
    }

    /// Whether anything is at the table's location now: for a pointed
    /// table, whether its pointer answers - a store that takes no listing
    /// is never asked whether its folder holds anything.
    pub(crate) fn exists(&self) -> bool {
        match &self.pointer {
            Some(pointer) => self.opened.get().is_some() || pointer.current().is_ok(),
            None => self.root.exists(),
        }
    }
}

impl<H: IOBase + Clone> Clone for IcebergTable<H> {
    /// A clone starts with nothing read: the root's own clone, and the
    /// current metadata document read again on its first use.
    fn clone(&self) -> Self {
        Self {
            root: self.root.clone(),
            path: self.path.clone(),
            stated: self.stated.clone(),
            inherited: self.inherited.clone(),
            description: self.description.clone(),
            opened: OnceLock::new(),
            options: self.options.clone(),
            pointer: self.pointer.clone(),
        }
    }
}

impl<H: IOBase> PartialEq for IcebergTable<H> {
    /// Equal by description - the path, the root's location, what was stated
    /// and inherited, the description and the options - never by what was
    /// read.
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
            && self.root.url() == other.root.url()
            && self.stated == other.stated
            && self.inherited == other.inherited
            && self.description == other.description
            && self.options == other.options
    }
}

impl<H: IOBase> Eq for IcebergTable<H> {}

impl<H: IOBase> Hash for IcebergTable<H> {
    fn hash<S: Hasher>(&self, state: &mut S) {
        self.path.hash(state);
        self.root.url().hash(state);
        self.stated.hash(state);
        self.inherited.hash(state);
        self.description.hash(state);
        self.options.hash(state);
    }
}

/// The path a table opened by its location alone stands under: its folder's
/// name, `table` for an unlocated folder.
fn path_of<H: IOBase>(root: &H) -> Vec<SmolStr> {
    named_after(root.url())
}

/// The path a table at `url` stands under when nothing else names it: the
/// location's last segment, `table` where it has none.
fn named_after(url: Option<&crate::Url>) -> Vec<SmolStr> {
    vec![SmolStr::new(
        url.and_then(crate::Url::file_name)
            .filter(|name| !name.is_empty())
            .unwrap_or("table"),
    )]
}

/// The handle a table opened by its location is rooted on: the one
/// [`Holder::from_url`] opens for the location under `properties`, resolved
/// on the first verb that needs it and again by every clone.
fn rooted_at(location: &crate::Uri, properties: &Properties) -> Result<Handle> {
    let url = location.locator()?;
    let path = named_after(Some(&url));
    Ok(Handle::at(
        crate::warehouse::Site::Url(url),
        false,
        &path,
        properties.clone(),
    ))
}

/// The default sort order a created table keeps: the one the schema's
/// `SORT:by` declares, else the spec's source columns.
fn default_order(schema: &mut Field, spec: &PartitionSpec) -> Result<SortOrder> {
    if schema.as_sort().declares_order() {
        // An order names its columns by identifier, so the schema is
        // numbered here exactly as the metadata document numbers it.
        let start = super::last_column_id(schema)?.saturating_add(1);
        super::assign_field_ids(schema, start)?;
        SortOrder::from_schema(1, schema)
    } else {
        Ok(SortOrder::for_spec(spec))
    }
}

/// Read the document `pointer` names under `root`: one answer of the
/// pointer and one read of the document, nothing listed. A pointer naming
/// no document yet is `None`.
fn read_pointed<H: IOBase>(root: &H, pointer: &dyn MetadataPointer) -> Result<Option<Opened>> {
    let state = pointer.current()?;
    let Some(location) = state.location() else {
        return Ok(None);
    };
    let base = root.url().ok_or_else(|| {
        invalid(SmolStr::new_static(
            "expected a located container to read a pointed table from, got a handle with no URL",
        ))
    })?;
    let named = location.to_string();
    let relative = relative_location(&base.to_string(), &named)?;
    let bytes = leaf(root.child_by_path(&relative)?)?.read_all_bytes()?;
    if bytes.is_empty() {
        return Err(invalid(format_smolstr!(
            "expected the Iceberg metadata document the table's pointer names at {named}, \
             got nothing there"
        )));
    }
    let metadata = TableMetadata::from_json(&parse_metadata_bytes(&bytes)?)?;
    let metadata_file_name = SmolStr::new(location.file_name().unwrap_or_default());
    // The number Iceberg's catalogs write a document under; a name that
    // states none is read as the first.
    let version = metadata_version_from_name(&metadata_file_name).unwrap_or(0);
    log::debug!(
        "opened iceberg table {} at the pointed metadata version {version}",
        metadata.location(),
    );
    Ok(Some(Opened {
        metadata,
        version,
        metadata_file_name,
        pointed: Some(state),
    }))
}

/// The version the next document of a pointed table is written under: one
/// above the document held, `0` when the pointer names none yet.
fn next_pointed_version(held: &Opened) -> Result<u32> {
    if held
        .pointed
        .as_ref()
        .and_then(PointerState::location)
        .is_none()
    {
        return Ok(0);
    }
    held.version.checked_add(1).ok_or_else(|| {
        invalid(format_smolstr!(
            "cannot commit metadata after version {}: the version overflows u32",
            held.version
        ))
    })
}

/// Refuse `operation` on the folder of a table a [`MetadataPointer`] names:
/// the catalog that keeps the pointer keeps the table, and its warehouse
/// location need take neither a listing nor a delete.
fn kept_by_catalog<H: IOBase>(root: &H, operation: &'static str) -> Error {
    Error::unsupported(operation, location_text(root))
}

/// Report a pointer that names no document for the table at `root`.
fn missing_pointed<H: IOBase>(root: &H) -> Error {
    invalid(format_smolstr!(
        "expected the metadata pointer of the table at {} to name a document, got none",
        location_text(root)
    ))
}

/// A handle's location as a refusal names it: its URL, else
/// `an unlocated folder`.
fn location_text<H: IOBase + ?Sized>(handle: &H) -> String {
    handle
        .url()
        .map_or_else(|| "an unlocated folder".to_owned(), ToString::to_string)
}

/// Read the current metadata document under `root`, when there is one:
/// the one `metadata/version-hint.text` names, else the highest-numbered
/// `*.metadata.json`.
fn read_current<H: IOBase>(root: &H) -> Result<Option<Opened>> {
    let metadata_dir = container(root.child_by_path(METADATA_DIR)?)?;
    let Some((version, metadata_file_name, document)) = find_metadata_document(&metadata_dir)?
    else {
        return Ok(None);
    };
    let metadata = TableMetadata::from_json(&document)?;
    log::debug!(
        "opened iceberg table {} at metadata version {version}",
        metadata.location(),
    );
    Ok(Some(Opened {
        metadata,
        version,
        metadata_file_name,
        pointed: None,
    }))
}

impl<H: IOBase> IcebergTable<H> {
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
    /// The table's default sort order is the one the schema declares as its
    /// `SORT:by` ([`SortOrder::from_schema`]), else [`SortOrder::for_spec`]:
    /// the spec's source columns ascending, nulls first, and the unsorted
    /// order zero when the spec partitions nothing. [`Self::create_sorted`]
    /// takes another.
    ///
    /// # Errors
    ///
    /// Returns a conflict when the handle already contains a table, or an
    /// error when the handle is not a container, the schema is not a non-null
    /// struct root, a declared order names what no sort field holds, or the
    /// metadata document cannot be written.
    pub fn create(
        root: H,
        format_version: FormatVersion,
        mut schema: Field,
        spec: PartitionSpec,
    ) -> Result<Self> {
        let order = default_order(&mut schema, &spec)?;
        Self::create_sorted(root, format_version, schema, spec, order)
    }

    /// Create a table whose current document `pointer` names, writing and
    /// publishing its first document.
    ///
    /// [`Self::create`] for a table a catalog service keeps: the pointer is
    /// read once and must name no document yet, and the first document -
    /// `metadata/00000-{uuid}.metadata.json`, version 0 - is published under
    /// the token it answered. Nothing is listed and no hint is written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the pointer already names a
    /// document or moved before the first one was published, the pointer's
    /// own failure, and the [`Self::create`] failures.
    pub fn create_pointed(
        root: H,
        format_version: FormatVersion,
        mut schema: Field,
        spec: PartitionSpec,
        pointer: Arc<dyn MetadataPointer>,
    ) -> Result<Self> {
        let order = default_order(&mut schema, &spec)?;
        Self::create_with(root, format_version, schema, spec, order, Some(pointer))
    }

    /// Create a table whose data files keep `order`, writing its first document.
    ///
    /// This is [`Self::create`] with the default sort order chosen rather
    /// than derived: every commit sorts each partition group's rows by the
    /// order's source columns before cutting them into files, and the order
    /// is recorded as the table's default so another writer keeps it too.
    /// [`SortOrder::unsorted`] declares that files carry rows as they arrive.
    ///
    /// # Errors
    ///
    /// Returns the [`Self::create`] failures, or an error when the order
    /// names a column the schema does not have.
    pub fn create_sorted(
        root: H,
        format_version: FormatVersion,
        schema: Field,
        spec: PartitionSpec,
        order: SortOrder,
    ) -> Result<Self> {
        Self::create_with(root, format_version, schema, spec, order, None)
    }

    /// The one creation: the first document under the folder contract, or
    /// published through `pointer`.
    fn create_with(
        root: H,
        format_version: FormatVersion,
        schema: Field,
        spec: PartitionSpec,
        order: SortOrder,
        pointer: Option<Arc<dyn MetadataPointer>>,
    ) -> Result<Self> {
        let location = root.url().map(ToString::to_string).ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a located container to create a table in, got a handle with no URL",
            ))
        })?;
        let metadata = TableMetadata::new_sorted(format_version, location, schema, spec, order)?;
        let mut table = Self::at(path_of(&root), root);
        match pointer {
            None => {
                table.adopt(Opened {
                    metadata,
                    version: 0,
                    metadata_file_name: SmolStr::new_static(""),
                    pointed: None,
                });
                table.create_metadata()?;
            }
            Some(pointer) => {
                // The pointer is read once: a table it already names a
                // document for was created by someone else, and is never
                // replaced by a second first document.
                let state = pointer.current()?;
                if let Some(named) = state.location() {
                    return Err(Error::conflict("Iceberg table", "Iceberg table", named));
                }
                table.adopt(Opened {
                    metadata,
                    version: 0,
                    metadata_file_name: SmolStr::new_static(""),
                    pointed: Some(state),
                });
                table.publish_pointed(pointer.as_ref(), None)?;
                table.pointer = Some(pointer);
            }
        }
        log::info!(
            "created iceberg table at {} (format v{}, {} columns)",
            table.opened()?.metadata.location(),
            format_version as u8,
            table.schema().map_or(0, |schema| schema.fields().len()),
        );
        Ok(table)
    }

    /// Describe the table `root` holds under `path`, touching nothing: the
    /// current metadata document is read on the first verb that needs it.
    ///
    /// This is the table a catalog lists, and the one [`Self::open`] answers
    /// once it has read the document; a folder that holds no table says so
    /// on that first use, naming its `metadata/` folder.
    pub fn at(path: impl IntoObjectPath, root: H) -> Self {
        let path = match path.into_object_path() {
            Ok(path) if !path.is_empty() => path,
            _ => path_of(&root),
        };
        Self {
            root,
            path,
            stated: Properties::new(),
            inherited: Properties::new(),
            description: None,
            opened: OnceLock::new(),
            options: None,
            pointer: None,
        }
    }

    /// Return this table with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The table under `path`: what a catalog that created it names it.
    pub(crate) fn placed(mut self, path: Vec<SmolStr>) -> Self {
        self.path = path;
        self
    }

    /// The table whose current document `pointer` names, touching nothing:
    /// what a catalog service's listing answers, read on first use.
    pub(crate) fn pointed(mut self, pointer: Arc<dyn MetadataPointer>) -> Self {
        self.pointer = Some(pointer);
        self
    }

    /// The table over `f` of its root, everything else kept - the document
    /// it has read included: how a table created over a handle in hand is
    /// re-rooted on the handle a catalog keeps.
    pub(crate) fn map_root<G: IOBase>(self, f: impl FnOnce(H) -> G) -> IcebergTable<G> {
        IcebergTable {
            root: f(self.root),
            path: self.path,
            stated: self.stated,
            inherited: self.inherited,
            description: self.description,
            opened: self.opened,
            options: self.options,
            pointer: self.pointer,
        }
    }

    /// The current metadata document, read on the first call.
    fn opened(&self) -> Result<&Opened> {
        if let Some(opened) = self.opened.get() {
            return Ok(opened);
        }
        let opened = match &self.pointer {
            Some(pointer) => match read_pointed(&self.root, pointer.as_ref())? {
                Some(opened) => opened,
                None => return Err(missing_pointed(&self.root)),
            },
            None => match read_current(&self.root)? {
                Some(opened) => opened,
                None => {
                    return Err(missing_metadata(&container(
                        self.root.child_by_path(METADATA_DIR)?,
                    )?));
                }
            },
        };
        Ok(self.opened.get_or_init(|| opened))
    }

    /// The current metadata document, mutably, read on the first call.
    fn opened_mut(&mut self) -> Result<&mut Opened> {
        self.opened()?;
        self.opened.get_mut().ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected the table's metadata document to be held once read",
            ))
        })
    }

    /// Hold `opened` as the current document, read or committed.
    fn adopt(&mut self, opened: Opened) {
        match self.opened.get_mut() {
            Some(held) => *held = opened,
            None => {
                let _ = self.opened.set(opened);
            }
        }
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
        match Self::locate_keeping(root)? {
            Ok(table) => Ok(table),
            Err(root) => Err(missing_metadata(&root.child_by_path(METADATA_DIR)?)),
        }
    }

    /// Open the table whose current document `pointer` names, under `root`.
    ///
    /// One answer of the pointer and one read of the document it names -
    /// its location taken relative to the root's - and nothing else: no
    /// hint is read and nothing is listed. Every commit through the table
    /// publishes through the pointer ([`MetadataPointer`]).
    ///
    /// # Errors
    ///
    /// Returns the pointer's own failure, an error when it names no
    /// document or one outside the root, and the read or parse failure of
    /// the document.
    pub fn open_pointed(root: H, pointer: Arc<dyn MetadataPointer>) -> Result<Self> {
        let table = Self::at(path_of(&root), root).pointed(pointer);
        table.opened()?;
        Ok(table)
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
        let Some(opened) = read_current(&root)? else {
            return Ok(Err(root));
        };
        let mut table = Self::at(path_of(&root), root);
        table.adopt(opened);
        Ok(Ok(table))
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

    /// Borrow the current table metadata, read on the first call.
    ///
    /// # Errors
    ///
    /// Returns an error when the folder holds no metadata document, or when
    /// the document is not table metadata.
    pub fn metadata(&self) -> Result<&TableMetadata> {
        Ok(&self.opened()?.metadata)
    }

    /// Return the version number of the current metadata document: the
    /// number its name states - `v3.metadata.json` and
    /// `00003-{uuid}.metadata.json` are both version 3. A pointed table's
    /// first document is version 0.
    ///
    /// # Errors
    ///
    /// As [`Self::metadata`].
    pub fn metadata_version(&self) -> Result<u32> {
        Ok(self.opened()?.version)
    }

    /// Return the name of the current metadata document.
    ///
    /// # Errors
    ///
    /// As [`Self::metadata`].
    pub fn metadata_file_name(&self) -> Result<String> {
        Ok(self.opened()?.metadata_file_name.to_string())
    }

    /// Return the location of the current metadata document, as a URI.
    ///
    /// # Errors
    ///
    /// As [`Self::metadata`].
    pub fn metadata_location(&self) -> Result<String> {
        let opened = self.opened()?;
        // A pointed table's document is wherever the pointer names it.
        if let Some(named) = opened.pointed.as_ref().and_then(PointerState::location) {
            return Ok(named.to_string());
        }
        Ok(format!(
            "{}/{METADATA_DIR}/{}",
            opened.metadata.location.trim_end_matches('/'),
            opened.metadata_file_name
        ))
    }

    /// Borrow the schema new data is written against.
    ///
    /// # Errors
    ///
    /// Returns an error when no schema carries the current schema identifier.
    pub fn schema(&self) -> Result<&Field> {
        self.opened()?.metadata.current_schema()
    }

    /// Borrow the snapshot a reader sees, when the table has one.
    ///
    /// # Errors
    ///
    /// As [`Self::metadata`].
    pub fn current_snapshot(&self) -> Result<Option<&Snapshot>> {
        Ok(self.opened()?.metadata.current_snapshot())
    }

    /// Return the size a data file aims for, in bytes.
    ///
    /// The one resolver is [`IcebergOptions`]: an explicit option stored with
    /// [`Self::set_options`] wins, then the table property
    /// [`IcebergOptions::TARGET_FILE_SIZE_KEY`], then the schema root's
    /// `ICEBERG:write.target-file-size-bytes` protocol property, then
    /// Iceberg's own default of 512 MiB.
    ///
    /// What a write measures against this target is the Arrow in-memory size
    /// of the accumulated rows as [`memory_size`](crate::arrow::memory_size)
    /// counts it - a zero-copy slice counts its own extent, not its parent's
    /// buffers - estimated *before* encoding. Parquet compresses what it
    /// writes, so data files land under the target rather than at it. The
    /// target cuts files, never commits: a streamed write through
    /// [`IOMedia`] commits once when its source ends unless its options
    /// state a [`commit_batch_num`](crate::media::IORecordOptions::commit_batch_num),
    /// and holds every partition's rows under the process spill bound until
    /// then (`write_cadenced`).
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key and the value when either property
    /// is present but does not spell a positive byte count; a configured
    /// target is never silently replaced by the default.
    pub fn target_file_size_bytes(&self) -> Result<u64> {
        IcebergOptions::target_size(self.options.as_ref(), &self.opened()?.metadata)
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
    /// the defaults are not consulted - so a caller can build a per-call
    /// override on top of it; [`Self::with_call_options`] is what shadows the
    /// override for one call and puts it back.
    pub fn explicit_options(&self) -> Option<&IcebergOptions> {
        self.options.as_ref()
    }

    /// Remove the explicit options override, returning what was stored.
    ///
    /// Every field then resolves property-then-default again.
    pub fn clear_options(&mut self) -> Option<IcebergOptions> {
        self.options.take()
    }

    /// Run `operation` on this table under `options` as its explicit
    /// override, then put back the override it had - whatever the
    /// operation answered - so per-call options never leak into the
    /// handle's own configuration. `None` runs the operation as the table
    /// stands.
    ///
    /// ```
    /// use yggdryl::iceberg::{IcebergOptions, IcebergTable};
    /// use yggdryl::{DataType, Properties, StructType, Url};
    ///
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let folder = std::env::temp_dir().join(format!("yggdryl-call-options-doc-{}", std::process::id()));
    /// let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    ///     .required_field("row");
    /// let location = Url::from_path(&folder)?;
    /// let mut table = IcebergTable::create_from_url(&location, &Properties::new(), None, schema, None)?;
    ///
    /// let once = Some(IcebergOptions::new().with_commit_retries(7));
    /// let retries = table.with_call_options(once, |table| table.options().map(|options| options.commit_retries()))?;
    /// assert_eq!(retries, 7);
    /// assert!(table.explicit_options().is_none());
    /// # std::fs::remove_dir_all(&folder)?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_call_options<R>(
        &mut self,
        options: Option<IcebergOptions>,
        operation: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let Some(options) = options else {
            return operation(self);
        };
        let saved = self.options.replace(options);
        let answer = operation(self);
        self.options = saved;
        answer
    }

    /// Resolve this table's effective options, field by field.
    ///
    /// Each field takes the nearest of three layers: the explicit override
    /// stored with [`Self::set_options`], then the table property of the same
    /// name (falling back to the schema root's `ICEBERG:` spelling), and the
    /// getters answer the documented default for whatever remains unset.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key and the value when a property no
    /// explicit option shadows is present but does not parse.
    pub fn options(&self) -> Result<IcebergOptions> {
        IcebergOptions::resolved(self.options.as_ref(), &self.opened()?.metadata)
    }

    /// Resolve where this table's commits stage their files.
    ///
    /// The explicit option, then the `write.staging` property, then the
    /// root's own default: the platform temporary folder when the root is
    /// remote - an object store, a foreign filesystem - and off when it is
    /// local. See [`WriteStaging`] for what a staged commit does and what a
    /// failed one leaves behind.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key when the property is present but
    /// does not spell `off` or a local folder.
    pub fn write_staging(&self) -> Result<WriteStaging> {
        let settings =
            IcebergOptions::write_settings(self.options.as_ref(), &self.opened()?.metadata)?;
        Ok(match settings.staging {
            Some(staging) => staging,
            None if self.is_remote() => {
                WriteStaging::Folder(crate::local::LocalFolder::temporary()?.url().clone())
            }
            None => WriteStaging::Off,
        })
    }

    /// Whether the table's folder is somewhere a local staging file is not.
    fn is_remote(&self) -> bool {
        self.root.url().is_some_and(|url| !url.is_local())
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
        match self.current_snapshot()? {
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
        let metadata = &self.opened()?.metadata;
        let reference = metadata
            .refs
            .iter()
            .find_map(|(candidate, reference)| (candidate == name).then_some(reference))
            .ok_or_else(|| {
                let known: Vec<&str> = metadata
                    .refs
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect();
                invalid(format_smolstr!(
                    "expected a branch or tag this table has, got {name:?}; it has [{}]",
                    known.join(", ")
                ))
            })?;
        metadata
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
    /// table here, a Parquet file's row groups, and a batch through
    /// [`Bound::filter`](crate::expression::Bound::filter).
    /// Each level of the metadata chain answers it from the statistics it
    /// carries: a manifest-list summary, then a manifest entry's partition
    /// tuple and column bounds. What none of them settles is left for the rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the predicate is text that does not parse, names
    /// a column the schema does not declare, or when a manifest that had to be
    /// read cannot be reached or decoded.
    pub fn plan_matching(&self, filter: impl crate::expression::IntoFilter) -> Result<ScanPlan> {
        let filter = filter.into_filter()?;
        let schema = self.schema()?;
        let conjuncts = super::scan::conjuncts(schema, &filter)?;
        self.planned(&conjuncts, schema, false)
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
        self.plan_manifests(&manifests, &conjuncts, &schema, false, None)
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
        let plan = self.plan_manifests(&manifests, &conjuncts, &stored, true, None)?;
        self.reader(plan.tasks, &stored, field, &filter)
    }

    /// Return one retained snapshot, or say which ids are retained.
    fn require_snapshot(&self, snapshot_id: i64) -> Result<&Snapshot> {
        let metadata = &self.opened()?.metadata;
        metadata.snapshot_by_id(snapshot_id).ok_or_else(|| {
            let retained: Vec<String> = metadata
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
            Some(schema_id) => self.opened()?.metadata.schema_by_id(schema_id).ok_or_else(|| {
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
        self.plan_manifests(&manifests, conjuncts, schema, for_read, None)
    }

    /// Plan one set of manifests under one set of resolved filters,
    /// keeping the bounds of the column `ordered` names on a read-only plan
    /// (see [`super::scan::plan`]).
    fn plan_manifests(
        &self,
        manifests: &[ManifestFile],
        conjuncts: &[crate::expression::Bound],
        schema: &Field,
        for_read: bool,
        ordered: Option<i32>,
    ) -> Result<ScanPlan> {
        let location = self.opened()?.metadata.location();
        log::debug!(
            "planning iceberg scan of {location} over {} manifests",
            manifests.len()
        );
        let plan = super::scan::plan(
            manifests,
            &|spec_id| {
                self.opened()?.metadata
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
            ordered,
        )?;
        // How much the filters removed is the read signal worth watching: a
        // plan that opens every file is a plan whose predicate bought nothing.
        log::info!(
            "planned iceberg scan of {location}: {} data files to open, {} excluded by filters, \
             {} manifests read and {} skipped on their summary",
            plan.tasks.len(),
            plan.excluded.len(),
            plan.manifests_read,
            plan.skipped.len(),
        );
        Ok(plan)
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
    /// # let folder = yggdryl::local::LocalFolder::new(yggdryl::local::LocalFolder::temporary()?.path()?.join("t"))?;
    /// # let mut table = yggdryl::iceberg::IcebergTable::open(folder)?;
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
        self.commit_document(
            OnConflict::Rebase,
            move |table| {
                // The change runs on a copy, so a rejected change costs nothing.
                let mut updated = table.opened()?.metadata.clone();
                change(&mut updated)?;
                Ok(updated)
            },
            None,
        )
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
    ///
    /// `staging` is the data commit's, when there is one: it is committed
    /// the moment the versioned document is durable, so a failure after
    /// that - the hint write, say - leaves every file the document names.
    fn commit_document(
        &mut self,
        on_conflict: OnConflict,
        mut apply: impl FnMut(&Self) -> Result<TableMetadata>,
        staging: Option<&Staging>,
    ) -> Result<()> {
        if let Some(pointer) = self.pointer.clone() {
            return self.commit_pointed(pointer.as_ref(), on_conflict, apply, staging);
        }
        let settings =
            IcebergOptions::commit_settings(self.options.as_ref(), &self.opened()?.metadata)?;
        let saved = self.opened()?.clone();
        let saved_version = saved.version;
        let expected_version = saved_version.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "cannot commit metadata after version {saved_version}: the version overflows u32"
            ))
        })?;
        let metadata_dir = container(self.root.child_by_path(METADATA_DIR)?)?;
        let restore = |table: &mut Self, error: Error| {
            table.adopt(saved.clone());
            Err(error)
        };
        let reconcile_visible = |table: &mut Self, error: Error| {
            // A backend may publish the metadata document and hint, then
            // report the hint write as failed. Re-read through the same
            // discovery path a new handle uses and adopt that visible version
            // when it is sound. A failed reload must never mask `error`; the
            // saved state is the only conservative in-memory answer when
            // visibility itself is uncertain.
            match find_metadata_document(&metadata_dir).and_then(|visible| {
                visible
                    .map(|(version, metadata_file_name, document)| {
                        TableMetadata::from_json(&document)
                            .map(|metadata| (version, metadata_file_name, metadata))
                    })
                    .transpose()
            }) {
                Ok(Some((version, metadata_file_name, metadata))) => {
                    table.adopt(Opened {
                        metadata,
                        version,
                        metadata_file_name,
                        pointed: None,
                    });
                }
                Ok(None) | Err(_) => table.adopt(saved.clone()),
            }
            Err(error)
        };

        let mut beaten: u32 = 0;
        let mut backoff_spent_ms = 0_u64;
        loop {
            // The version this handle holds is the version it re-checks, so
            // a hint naming it settles the check without reading the
            // document again: the document is read only when it is newer.
            let held_version = self.opened()?.version;
            match find_metadata(&metadata_dir, Some(held_version)) {
                Ok(Some((version, metadata_file_name, document))) if version > held_version => {
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
                        let fresh = document
                            .ok_or_else(|| {
                                invalid(format_smolstr!(
                                    "expected the newer metadata document {metadata_file_name} to be read, got none"
                                ))
                            })
                            .and_then(|document| TableMetadata::from_json(&document));
                        match fresh {
                            Ok(fresh) => self.adopt(Opened {
                                metadata: fresh,
                                version,
                                metadata_file_name,
                                pointed: None,
                            }),
                            Err(error) => return restore(self, error),
                        }
                    }
                    log::debug!(
                        "iceberg commit of {} found version {version} already published; \
                         retry {beaten}, waiting {wait} ms",
                        self.opened()?.metadata.location(),
                    );
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
            self.opened_mut()?.metadata = updated;
            if let Err(error) = self.commit_metadata(staging) {
                if !error.is_conflict() {
                    return reconcile_visible(self, error);
                }
                let winner = find_metadata_document(&metadata_dir).and_then(|visible| {
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
                if version <= self.opened()?.version {
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
                    self.adopt(Opened {
                        metadata,
                        version,
                        metadata_file_name,
                        pointed: None,
                    });
                }
                log::debug!(
                    "iceberg commit of {} was beaten to version {version} on write; \
                     retry {beaten}, waiting {wait} ms",
                    self.opened()?.metadata.location(),
                );
                if wait > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(wait));
                }
                continue;
            }
            let committed = self.opened()?;
            log::info!(
                "committed iceberg metadata version {} of {}{}",
                committed.version,
                committed.metadata.location(),
                match committed.metadata.current_snapshot() {
                    Some(snapshot) => format!(", snapshot {}", snapshot.snapshot_id),
                    None => String::new(),
                },
            );
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
        super::inspect::history(&self.opened()?.metadata)
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
        super::inspect::snapshots(&self.opened()?.metadata)
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
    /// Each data file is read through [`crate::IOMedia::read_arrow_reader`] with
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
    /// than equality pairs: ranges, null tests, `in` lists, and nested paths.
    /// Planning prunes with [`Self::plan_matching`], and only the conjuncts the
    /// metadata could not settle are tested against the rows a surviving file
    /// holds.
    ///
    /// ```no_run
    /// use yggdryl::iceberg::IcebergTable;
    /// use yggdryl::local::LocalFolder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let table = IcebergTable::open(LocalFolder::new("/lake/trades")?)?;
    /// let reader = table.scan_matching(
    ///     "ccy = 'EUR' and price > 100 and year = 2024",
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
        filter: impl crate::expression::IntoFilter,
        field: Option<&Field>,
    ) -> Result<BatchReader> {
        let filter = filter.into_filter()?;
        let stored = self.schema()?.clone();
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
        filter: &crate::Filter,
    ) -> Result<BatchReader> {
        // The files are read in plan order, so the rows state no order
        // across them: the root drops the `SORT:by` the writers keep.
        let root = field.map_or_else(
            || stored.clone().with_metadata_removed("SORT:by"),
            Clone::clone,
        );
        let read_root = super::scan::read_root(&root, stored, filter)?;
        // The residual conjuncts run against the read root, which carries the
        // predicate's own columns even when the caller projected them away.
        let predicates = super::scan::conjuncts(&read_root, filter)?;
        let parts = self.scan_parts(tasks, stored, &read_root)?;
        let parallel =
            IcebergOptions::read_settings(self.options.as_ref(), &self.opened()?.metadata)?;
        super::scan::reader(
            parts,
            root,
            read_root,
            field.cloned(),
            predicates,
            &parallel,
            columns_renamed(&self.opened()?.metadata),
        )
    }

    /// Resolve planned files into the handles a scan opens.
    ///
    /// A partition column is restored from the manifest only when the read
    /// root asks for it: one it leaves out would be built for every row and
    /// dropped again by the final cast.
    fn scan_parts(
        &self,
        tasks: Vec<ScanTask>,
        stored: &Field,
        read_root: &Field,
    ) -> Result<Vec<ScanPart>> {
        let mut parts = Vec::with_capacity(tasks.len());
        for task in tasks {
            // The manifest recorded the file's length, so the handle knows it
            // before the read asks - unless it recorded none, which is not
            // believed: the file answers for itself then.
            let mut handle = sized(
                self.child_at(&task.entry.data_file.file_path)?,
                u64::try_from(task.entry.data_file.file_size_in_bytes).unwrap_or_default(),
            );
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
                )?
                .into_iter()
                .filter(|(column, _)| {
                    read_root
                        .fields()
                        .iter()
                        .any(|wanted| wanted.name().eq_ignore_ascii_case(column.name()))
                })
                .collect(),
                residual: task.residual,
            });
        }
        Ok(parts)
    }

    /// Read the rows the record options ask for, under one more predicate:
    /// [`Self::read_rows`] as transport, the selector and the limit wrapping
    /// it.
    ///
    /// This is the one door every options-driven read takes: the `where`
    /// clause and `scope` - the partition a folder handle addresses - are
    /// pushed into the scan plan whole, so a range, an `in` list or a null test
    /// prunes manifests and files exactly as an equality does; the scan reads only the columns the clauses need; and
    /// the selector and the limit wrap what comes back. A `where` that names
    /// a column only the `select` publishes cannot prune - the scan does not
    /// know the name - so it runs after the projection, as DuckDB lets a
    /// `where` read an alias.
    pub(crate) fn read_scoped(
        &self,
        scope: Filter,
        options: &RecordOptions,
    ) -> Result<BatchReader> {
        let (rows, late) = self.read_rows(scope, options)?;
        let reader = rows.into_arrow_reader()?;
        if late {
            return options.limit_arrow_reader(options.apply_arrow_expressions(reader)?);
        }
        // The limit wraps last, as on every handle, so it counts result rows
        // and a satisfied read opens no partition past the one that
        // satisfied it.
        options.limit_arrow_reader(options.select().apply_arrow_reader(reader)?)
    }

    /// The rows an options-driven read yields before the `select`, a late
    /// `where` and the row bounds wrap them, and whether that `where` runs
    /// after the `select` (`true`) or was pushed into the plan with `scope`.
    ///
    /// Partitions arrive in ascending tuple order
    /// ([`super::scan::partition_groups`]), each decoded once the one before
    /// it was yielded, its rows in the table's default sort order: where the
    /// order names a key the group does not hold constant, the group is held
    /// under the process spill bound and sorted unless it already keeps the
    /// order, at most one partition at once. A key the root does not read, and
    /// any after it, is not sorted on. A table with no such key - unsorted, or
    /// sorted only by identity partition columns - streams its files and holds
    /// nothing. A plan holding a file of another partition spec reads in plan
    /// order and proves nothing. The root declares what the stream proves
    /// ([`Self::read_order`]), as [`IOMedia::read_arrow_field`] does.
    fn read_rows(
        &self,
        scope: Filter,
        options: &RecordOptions,
    ) -> Result<(super::scan::Partitions, bool)> {
        let stored = self.schema()?.clone();
        let filter = options.filter();
        let late = crate::expression::filter_after_select(
            filter,
            options.select(),
            stored.fields().iter().map(Field::name),
        );
        let (landing, given) = self.read_landing(options)?;
        let pushed = if late {
            scope
        } else if scope.is_always_true() {
            filter.clone()
        } else {
            Filter::all([scope, filter.clone()])
        };
        let metadata = &self.opened()?.metadata;
        let spec = metadata.default_spec()?;
        let order = metadata.default_sort_order()?;
        let conjuncts = super::scan::conjuncts(&stored, &pushed)?;
        let plan = self.plan_manifests(
            &self.manifests()?,
            &conjuncts,
            &stored,
            true,
            super::scan::leading_key_id(order, spec, &stored),
        )?;
        let read_root = super::scan::read_root(&landing, &stored, &pushed)?;
        // The residual conjuncts run against the read root, which carries the
        // predicate's own columns even when the caller projected them away.
        let predicates = super::scan::conjuncts(&read_root, &pushed)?;
        let (groups, sorting, proven) =
            match super::scan::partition_groups(plan.tasks, spec, order, &stored) {
                Ok(groups) => {
                    let sorting = self.read_sorting(&landing)?;
                    let proven = self.read_order(&landing, options.select(), sorting.len())?;
                    (groups, sorting, proven)
                }
                Err(tasks) => {
                    log::debug!(
                        "reading iceberg table {} in plan order: a planned file belongs to a \
                         partition spec other than the default {}, whose tuples do not compare",
                        metadata.location(),
                        spec.spec_id
                    );
                    (vec![tasks], Vec::new(), Vec::new())
                }
            };
        let groups = groups
            .into_iter()
            .map(|tasks| self.scan_parts(tasks, &stored, &read_root))
            .collect::<Result<Vec<_>>>()?;
        let mut parallel = IcebergOptions::read_settings(self.options.as_ref(), metadata)?;
        if options.max_row_size().is_some() {
            // A limited read decodes one file at a time, on one thread, so it
            // stops where its limit does rather than decoding ahead of it.
            parallel.parallelism = 1;
        }
        let root = super::scan::declaring(landing.clone(), proven)?;
        let declared_read_root = super::scan::read_root(&root, &stored, &pushed)?;
        let partitions = super::scan::Partitions::new(
            groups,
            super::scan::GroupScan {
                target: given.then(|| landing.clone()),
                root: landing,
                read_root,
                declared_read_root,
                predicates,
                parallel,
                renamed: columns_renamed(metadata),
            },
            sorting,
            std::sync::Arc::new(root),
        );
        Ok((partitions, late))
    }

    /// The root an options-driven read lands its rows under, declaring no
    /// order, and whether the caller gave it: the declared field, else the
    /// stored schema narrowed to the columns the clauses read and named as
    /// the options name the root.
    fn read_landing(&self, options: &RecordOptions) -> Result<(Field, bool)> {
        if let Some(field) = options.field() {
            return Ok((field.with_metadata_removed("SORT:by"), true));
        }
        let stored = self.schema()?;
        let projected = options
            .apply_columns()
            .and_then(|columns| projected_root(stored, &columns));
        let given = projected.is_some();
        let root = projected
            .unwrap_or_else(|| stored.clone())
            .with_name(options.name());
        Ok((root.with_metadata_removed("SORT:by"), given))
    }

    /// The keys a record read sorts each partition group by: the table's
    /// default order without the columns a group holds constant, while
    /// `landing` reads them. An order naming a column the schema no longer
    /// has sorts nothing - the rows are still read, in tuple order.
    fn read_sorting(&self, landing: &Field) -> Result<Vec<crate::expression::Ordering>> {
        let metadata = &self.opened()?.metadata;
        let Ok((keys, _)) = sort_orderings(
            metadata.default_sort_order()?,
            metadata.default_spec()?,
            self.schema()?,
        ) else {
            return Ok(Vec::new());
        };
        Ok(keys
            .into_iter()
            .take_while(|key| key.term().bind(landing).is_ok())
            .collect())
    }

    /// The order a record read of rows landing under `landing`, groups
    /// sorted on `sorted` keys, proves once `select` has run
    /// ([`super::scan::proven_order`]) - and none for a table whose
    /// metadata holds a partition spec besides the default, so the root a
    /// schema read answers from metadata alone declares exactly what the
    /// stream does, whichever files a filter plans.
    fn read_order(
        &self,
        landing: &Field,
        select: &Selector,
        sorted: usize,
    ) -> Result<Vec<crate::expression::Ordering>> {
        let metadata = &self.opened()?.metadata;
        if metadata.partition_specs().len() != 1 {
            return Ok(Vec::new());
        }
        Ok(super::scan::proven_order(
            metadata.default_spec()?,
            metadata.default_sort_order()?,
            self.schema()?,
            landing,
            select,
            sorted,
        ))
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
        let batches = self.derived(batches)?;
        self.commit_append_on(batches, None)
    }

    /// The rows of `batches` with every column the schema derives
    /// (`TRANSFORM:expression`) computed before anything is cast or grouped,
    /// replacing whatever the rows carry under that name, so the table
    /// stores what its schema says. A schema deriving nothing hands the
    /// reader back. The public commit and record doors derive; the crate's
    /// `_on` and cadence forms take rows already derived.
    fn derived(&self, batches: BatchReader) -> Result<BatchReader> {
        match crate::expression::Derivation::owning(self.schema()?)? {
            Some(derivation) => derivation.apply_arrow_reader(batches),
            None => Ok(batches),
        }
    }

    /// [`Self::commit_append`] with its partition groups written on
    /// `threads` at once where one is stated, else on the table's own
    /// parallelism.
    pub(crate) fn commit_append_on(
        &mut self,
        batches: BatchReader,
        threads: Option<usize>,
    ) -> Result<()> {
        let writes = self.partition_writes(batches, false)?;
        self.commit(writes, None, "append", Retained::All, threads)?;
        Ok(())
    }

    /// Replace the partitions `batches` falls in as a new snapshot.
    ///
    /// Every live file of a partition the rows reach is dropped and every
    /// other file carried as it is - location, statistics, row lineage - so
    /// no stored row is read or rewritten and this holds on every format
    /// version. A source with no row reaches no partition and commits
    /// nothing; an unpartitioned table is one partition, replaced whole and
    /// emptied by a source with no row. [`Self::commit_overwrite_where`]
    /// replaces a stated scope instead.
    ///
    /// The previous snapshot is retained and still readable; only the current
    /// pointer moves, which is what makes an overwrite reversible.
    ///
    /// # Errors
    ///
    /// Returns an error when the partition spec cannot place a row, when a
    /// batch cannot be cast to the table schema, when a live file written
    /// under another partition spec could hold rows of a partition the
    /// source reaches - rewrite it into the current spec first - when any
    /// write fails, or a [`CommitConflict`] when a concurrent commit won.
    pub fn commit_overwrite(&mut self, batches: BatchReader) -> Result<()> {
        let batches = self.derived(batches)?;
        self.commit_overwrite_cadence(&[], batches, &mut ReplacedPartitions::default(), None)
    }

    /// One commit of a streamed overwrite.
    ///
    /// With no `filters` on a partitioned table the partitions the rows fall
    /// in are replaced, and `replaced` carries the ones this write's earlier
    /// commits replaced: the first commit that reaches a partition replaces
    /// it, every later one appends beside what the write put there. With
    /// `filters`, or on an unpartitioned table, the write's first commit
    /// replaces the rows the filters select - every row with none - and
    /// every later one appends.
    pub(crate) fn commit_overwrite_cadence(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        replaced: &mut ReplacedPartitions,
        threads: Option<usize>,
    ) -> Result<()> {
        if filters.is_empty() && !self.opened()?.metadata.default_spec()?.is_unpartitioned() {
            return self.commit_partitions(
                filters,
                batches,
                Selector::new(Vec::new()),
                false,
                replaced,
                threads,
            );
        }
        if replaced.scope {
            return self.commit_append_on(batches, threads);
        }
        self.commit_overwrite_where_on(filters, batches, threads)?;
        replaced.scope = true;
        Ok(())
    }

    /// Replace only the rows `filters` selects, keeping every other file.
    ///
    /// No filter selects every row, which is the whole table replaced in
    /// one snapshot whatever its partitions.
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
        let batches = self.derived(batches)?;
        self.commit_overwrite_where_on(filters, batches, None)
    }

    /// [`Self::commit_overwrite_where`] with its partition groups written
    /// on `threads` at once where one is stated, else on the table's own
    /// parallelism.
    pub(crate) fn commit_overwrite_where_on(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        threads: Option<usize>,
    ) -> Result<()> {
        let plan = self.plan(filters)?;
        let writes = self.partition_writes(batches, false)?;
        self.commit(
            writes,
            None,
            "overwrite",
            Retained::Only {
                manifests: plan.skipped,
                entries: plan.excluded,
            },
            threads,
        )?;
        Ok(())
    }

    /// Merge `batches` into the stored rows, matching on the `merge_by` columns.
    ///
    /// The partition columns are the primary keys: see
    /// [`Self::commit_merge_where`], which this is with no scope.
    ///
    /// # Errors
    ///
    /// Returns the failure of the read, the join, or the commit.
    pub fn commit_merge(
        &mut self,
        batches: BatchReader,
        merge_by: &crate::Selector,
        safe: bool,
    ) -> Result<()> {
        self.commit_merge_where(&[], batches, merge_by, safe)
    }

    /// Merge `batches` into the rows `filters` selects, on the `merge_by` columns.
    ///
    /// **Partition keys are the primary keys.** The match key is the identity
    /// partition columns followed by `merge_by`, each named once, so a row
    /// can only update a stored row of its own partition. The incoming rows
    /// are grouped by partition tuple first - the same grouping an append
    /// lays files out by - and the plan opens the manifests and files of
    /// those partitions alone; every other file is carried into the new
    /// snapshot untouched, same location, same statistics. Within a
    /// partition the *column statistics* narrow further: a row can only
    /// update a file whose recorded bounds for every non-partition key
    /// column contain one of the incoming keys, and a file they cannot is
    /// carried too. Each group then joins with its own files, keeps the last
    /// of the rows that arrive with one key, and writes that partition's new
    /// files; what is in memory at once is one group's rows and the stored
    /// files it selected, never the whole table.
    ///
    /// A merge that names no key beyond the partition columns replaces the
    /// partitions the rows fall in - the partition *is* the row's identity -
    /// and needs a partitioned table: with no partition and no key there is
    /// nothing to match on, which is refused by name rather than read as an
    /// overwrite.
    ///
    /// Like [`Self::commit_overwrite_where`], a merge beaten by a concurrent commit
    /// reports a [`CommitConflict`] rather than rebasing, because the files it
    /// selected and the reader it consumed cannot be re-planned safely.
    ///
    /// # Errors
    ///
    /// Returns an error for a keyed merge on format v3, whose existing row IDs
    /// this writer cannot yet preserve, when `merge_by` names a column the schema
    /// does not declare, when the table has neither a partition nor a key,
    /// when a live file written under another partition spec could hold an
    /// incoming key - it belongs to no partition of the current spec, so
    /// rewrite it first - or for any read, join, or write failure, including
    /// a [`CommitConflict`] when a concurrent commit won.
    pub fn commit_merge_where(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        merge_by: &crate::Selector,
        safe: bool,
    ) -> Result<()> {
        let batches = self.derived(batches)?;
        self.commit_merge_cadence(
            filters,
            batches,
            merge_by,
            safe,
            &mut ReplacedPartitions::default(),
            None,
        )
    }

    /// One commit of a streamed merge: [`Self::commit_merge_where`] over
    /// the partitions the write's earlier commits replaced.
    ///
    /// A merge keyed by the partition alone replaces the partitions its
    /// rows fall in, and a stream cut into commits by its cadence must not
    /// replace one per commit - that would keep only the last commit's rows
    /// of each. So `replaced` carries the partitions this write has already
    /// replaced: the first commit that reaches a partition replaces it and
    /// records it, every later one carries its files and appends beside
    /// them. A keyed merge joins by key on every commit and records nothing.
    pub(crate) fn commit_merge_cadence(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        merge_by: &crate::Selector,
        safe: bool,
        replaced: &mut ReplacedPartitions,
        threads: Option<usize>,
    ) -> Result<()> {
        let schema = self.schema()?.clone();
        let spec = self.opened()?.metadata.default_spec()?.clone();
        let (keys, row_keys) = merge_keys(&schema, &spec, merge_by);
        if !row_keys.is_empty() {
            // A keyed merge rewrites the stored rows it keeps, under fresh
            // row IDs; a partition replaced whole retains none of them.
            self.require_row_id_preserving_rewrite("merge")?;
        }
        if keys.is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.merge_by"),
                reason: SmolStr::new_static(
                    "expected at least one column to merge on, got an empty match key on an \
                     unpartitioned table",
                ),
            });
        }
        // Every key column is checked against the schema before a file is
        // read, computed keys included, so a bad key costs nothing.
        keys.bind(&schema)?;
        self.commit_partitions(filters, batches, row_keys, safe, replaced, threads)
    }

    /// One commit over the partitions `batches` falls in: each joined with
    /// its stored rows by `row_keys`, or - with no key beyond the partition -
    /// replaced.
    ///
    /// What [`Self::commit_merge_cadence`] and
    /// [`Self::commit_overwrite_cadence`] share: the incoming rows grouped
    /// by partition tuple, the plan opened over those partitions alone, and
    /// every file of every other partition carried untouched. A source with
    /// no row commits nothing.
    fn commit_partitions(
        &mut self,
        filters: &[(&str, &str)],
        batches: BatchReader,
        row_keys: Selector,
        safe: bool,
        replaced: &mut ReplacedPartitions,
        threads: Option<usize>,
    ) -> Result<()> {
        let schema = self.schema()?.clone();
        let spec = self.opened()?.metadata.default_spec()?.clone();

        // The incoming side is held, grouped by partition, and this is why:
        // the files a merge has to read are the ones of the partitions the
        // rows fall in whose statistics say they can hold an incoming key,
        // and neither is known from a reader that has not been read.
        let mut writes = self.partition_writes(batches, safe)?;
        if writes.is_empty() {
            return Ok(());
        }
        let tuples: Vec<Vec<Scalar>> = writes.iter().map(|write| write.values.clone()).collect();
        let scope = Filter::all([
            pairs_predicate(&schema, filters),
            partition_tuples_filter(&spec, &schema, &tuples),
        ]);
        let conjuncts = super::scan::conjuncts(&schema, &scope)?;
        let plan = self.planned(&conjuncts, &schema, false)?;

        let keyed = !row_keys.is_empty();
        let mut carried = plan.excluded;
        let mut own: Vec<Vec<ScanTask>> = (0..writes.len()).map(|_| Vec::new()).collect();
        let mut foreign: Vec<ScanTask> = Vec::new();
        for task in plan.tasks {
            if task.spec.spec_id != spec.spec_id {
                foreign.push(task);
                continue;
            }
            match tuples
                .iter()
                .position(|values| *values == task.entry.data_file.partition)
            {
                Some(position) => own[position].push(task),
                // A partition no incoming row falls in: the filter kept the
                // file on bounds a transformed field cannot settle, and the
                // tuple settles it now.
                None => carried.push(task),
            }
        }
        if !foreign.is_empty() {
            // A file of another spec belongs to no partition of this one, so
            // no group can own it; it is carried only when the statistics
            // prove no incoming key can be in it.
            let mut all: Vec<RecordBatch> = Vec::new();
            for write in &writes {
                all.extend(hold_batches(&write.hold)?);
            }
            let bounds = KeyBounds::of(&all, &schema, &row_keys)?;
            for task in foreign {
                if keyed && !bounds.may_hold(&task.entry.data_file) {
                    carried.push(task);
                    continue;
                }
                return Err(invalid(format_smolstr!(
                    "expected every live file this write could replace to belong to partition \
                     spec {}, got {:?} under spec {}; rewrite it into the current spec first",
                    spec.spec_id,
                    task.entry.data_file.file_path,
                    task.spec.spec_id
                )));
            }
        }
        let mut replacing: Vec<Vec<Scalar>> = Vec::new();
        for (write, tasks) in writes.iter_mut().zip(own) {
            if !keyed {
                // The partition is the key: its files are replaced, not
                // read, by the first commit of the write that reaches it,
                // and carried - this commit appends beside them - by every
                // later commit of the same write.
                if replaced.contains(&write.values) {
                    carried.extend(tasks);
                } else {
                    replacing.push(write.values.clone());
                }
                continue;
            }
            let incoming = hold_batches(&write.hold)?;
            let bounds = KeyBounds::of(&incoming, &schema, &row_keys)?;
            let mut selected = Vec::new();
            for task in tasks {
                if bounds.may_hold(&task.entry.data_file) {
                    selected.push(task);
                } else {
                    carried.push(task);
                }
            }
            write.stored = self.scan_parts(selected, &schema, &schema)?;
        }
        let join = Join {
            keys: row_keys,
            safe,
        };
        self.commit(
            writes,
            keyed.then_some(&join),
            "overwrite",
            Retained::Only {
                manifests: plan.skipped,
                entries: carried,
            },
            threads,
        )?;
        // Recorded once the commit is published: a commit that failed
        // replaced nothing, and the write it belonged to is over.
        replaced.tuples.extend(replacing);
        Ok(())
    }

    /// Group an incoming reader by partition tuple, each group a write.
    fn partition_writes(&self, batches: BatchReader, safe: bool) -> Result<Vec<PartitionWrite>> {
        let schema = self.schema()?;
        let spec = self.opened()?.metadata.default_spec()?;
        let partition = spec.partition_field(schema)?;
        Ok(grouped_holds(batches, schema, spec, &partition, safe)?
            .into_iter()
            .map(|(values, hold)| PartitionWrite {
                values,
                hold,
                stored: Vec::new(),
            })
            .collect())
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
            log::info!(
                "compaction of {} found nothing to rewrite; no snapshot committed",
                self.opened()?.metadata.location(),
            );
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

        log::debug!(
            "compacting {}: rewriting {files_before} files of {bytes_rewritten} bytes",
            self.opened()?.metadata.location(),
        );
        let schema = self.schema()?.clone();
        let rows = self.reader(selected, &schema, None, &crate::Filter::always_true())?;
        let writes = self.partition_writes(rows, false)?;
        let files_after = self.commit(
            writes,
            None,
            "replace",
            Retained::Only {
                manifests: plan.skipped,
                entries: carried,
            },
            None,
        )?;
        log::info!(
            "compacted {}: {files_before} files of {bytes_rewritten} bytes rewritten as {files_after}",
            self.opened()?.metadata.location(),
        );
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
    /// so a column added here reads as its v3 `initial-default` in the files
    /// that predate it, or as null where it declares none.
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
        log::info!(
            "evolved iceberg schema of {} to id {schema_id}",
            self.opened()?.metadata.location(),
        );
        Ok(schema_id)
    }

    /// Commit a recorded [`SchemaUpdate`](crate::iceberg::SchemaUpdate) and
    /// return the schema identifier it made current.
    ///
    /// The operations replay onto the metadata each attempt reads, so a
    /// commit beaten by another writer rebases onto the winner's schema
    /// rather than overwriting it. An update that recorded nothing commits
    /// nothing and answers the current schema's identifier.
    ///
    /// # Errors
    ///
    /// Returns an operation's failure against the current schema - a path
    /// that does not resolve, a name collision, an illegal promotion - or a
    /// commit failure.
    pub fn update_schema(&mut self, update: &crate::iceberg::SchemaUpdate) -> Result<i32> {
        if update.is_empty() {
            return Ok(self.opened()?.metadata.current_schema_id());
        }
        let mut schema_id = 0;
        self.commit_metadata_changes(|metadata| {
            schema_id = metadata.add_schema(update.replay(metadata)?)?;
            metadata.set_current_schema(schema_id)?;
            Ok(())
        })?;
        log::info!(
            "evolved iceberg schema of {} to id {schema_id}",
            self.opened()?.metadata.location(),
        );
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
        let mut probe = self.opened()?.metadata.clone();
        probe.expire_snapshots(older_than_ms, retain_last, snapshot_ids)?;
        if probe == self.opened()?.metadata {
            return Ok(Vec::new());
        }
        let mut expired = Vec::new();
        self.commit_metadata_changes(|metadata| {
            expired = metadata.expire_snapshots(older_than_ms, retain_last, snapshot_ids)?;
            Ok(())
        })?;
        log::info!(
            "expired {} iceberg snapshots of {}",
            expired.len(),
            self.opened()?.metadata.location(),
        );
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
        let relative = relative_location(&self.opened()?.metadata.location, location)?;
        leaf(self.root.child_by_path(&relative)?)
    }

    /// Write the first document, trying again while every racing creator
    /// yielded.
    ///
    /// A creator that finds another's attempt at version one removes its own
    /// and yields, so creators writing side by side can all yield and leave no
    /// table. Storage has no compare-and-swap to break that tie; but each
    /// removes its own attempt before looking again, so the last to look finds
    /// the version free and tries once more after a jittered wait, under the
    /// commit retry budget. A creator that finds a document there - published
    /// or still in flight - was beaten and reports the typed conflict.
    fn create_metadata(&mut self) -> Result<()> {
        let settings =
            IcebergOptions::commit_settings(self.options.as_ref(), &self.opened()?.metadata)?;
        let metadata_dir = container(self.root.child_by_path(METADATA_DIR)?)?;
        let mut beaten = 0_u32;
        let mut backoff_spent_ms = 0_u64;
        loop {
            let conflict = match self.commit_metadata(None) {
                Err(error) if error.is_conflict() => error,
                done => return done,
            };
            if !metadata_names_at_version(&metadata_dir, 1)?.is_empty() {
                return Err(conflict);
            }
            let Ok(wait) = retry_wait_ms(&settings, &mut beaten, &mut backoff_spent_ms, 1, 0)
            else {
                return Err(conflict);
            };
            log::debug!(
                "iceberg create of {} yielded to a racing creator; retry {beaten}, waiting {wait} ms",
                self.opened()?.metadata.location(),
            );
            if wait > 0 {
                std::thread::sleep(std::time::Duration::from_millis(wait));
            }
        }
    }

    /// Write the current metadata as the next numbered document.
    fn commit_metadata(&mut self, staging: Option<&Staging>) -> Result<()> {
        // A bad in-memory state is refused before a document exists, so a
        // broken table can only be read, never written.
        let held = self.opened()?;
        held.metadata.validate()?;
        let previous = (held.version > 0)
            .then(|| self.metadata_location())
            .transpose()?;
        let next_version = held.version.checked_add(1).ok_or_else(|| {
            invalid(format_smolstr!(
                "cannot commit metadata after version {}: the version overflows u32",
                held.version
            ))
        })?;
        let mut metadata = held.metadata.clone();
        metadata.finalize_official(previous)?;
        let (suffix, encoded) = encoded_document(&metadata)?;
        let attempt = format_smolstr!("{next_version:05}-{}{suffix}.metadata.json", uuid());
        let metadata_dir = container(self.root.child_by_path(METADATA_DIR)?)?;
        let mut handle = leaf(
            self.root
                .child_by_path(&format!("{METADATA_DIR}/{attempt}"))?,
        )?;
        handle.write_all_bytes(&encoded)?;

        // UUID filenames make the write itself the create/commit attempt.
        // Another document at this version means a table or concurrent writer
        // already won; remove only our unpublished candidate and report it.
        // A listing that fails removes it too: the attempt names files a
        // data commit rolls back, and left behind it would claim the version
        // against every later writer.
        let competitors = match metadata_names_at_version(&metadata_dir, next_version) {
            Ok(names) => names
                .into_iter()
                .filter(|candidate| candidate != &attempt)
                .collect::<Vec<_>>(),
            Err(error) => {
                drop(handle.remove(false));
                return Err(error);
            }
        };
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
        let mut document = leaf(self.root.child_by_path(&format!("{METADATA_DIR}/{name}"))?)?;
        if let Err(error) = document.write_all_bytes(&encoded) {
            // Nothing durable names the commit's files yet, and the attempt
            // - which does - goes with them rather than staying to claim the
            // version.
            drop(handle.remove(false));
            return Err(error);
        }
        // The versioned document is durable and names every file the commit
        // published: this is the point of no return. A fresh handle resolves
        // the version to this document whatever the hint write reports next,
        // so from here nothing is rolled back.
        if let Some(staging) = staging {
            staging.commit();
        }

        // The hint is how a catalog-free reader finds the current document.
        let mut hint = leaf(
            self.root
                .child_by_path(&format!("{METADATA_DIR}/{VERSION_HINT}"))?,
        )?;
        hint.write_all_bytes(next_version.to_string().as_bytes())?;

        // The attempt has served its whole purpose. Its removal is the commit's
        // last act rather than a step of it: the published document and the hint
        // are already durable, so a backend that refuses leaves an unreferenced
        // duplicate rather than an unfinished commit.
        drop(handle.remove(false));
        self.adopt(Opened {
            metadata,
            version: next_version,
            metadata_file_name: name,
            pointed: None,
        });
        Ok(())
    }

    /// [`Self::commit_document`] for a pointed table: the pointer is the
    /// version check and the publication in one.
    ///
    /// Nothing is read before an attempt publishes: the pointer's condition
    /// is the check. A refused publication reads the pointer again - one
    /// answer, one document: a commit that may rebase applies again onto
    /// that document under the retry budget; one that may not is a
    /// [`CommitConflict`] at once, unless the pointer still names the
    /// document it planned against (a token moved by a change that wrote no
    /// document), which it publishes again. Nothing an attempt wrote is
    /// removed, and a failure restores the document and token held before.
    fn commit_pointed(
        &mut self,
        pointer: &dyn MetadataPointer,
        on_conflict: OnConflict,
        mut apply: impl FnMut(&Self) -> Result<TableMetadata>,
        staging: Option<&Staging>,
    ) -> Result<()> {
        let settings =
            IcebergOptions::commit_settings(self.options.as_ref(), &self.opened()?.metadata)?;
        let saved = self.opened()?.clone();
        let planned = saved
            .pointed
            .as_ref()
            .and_then(PointerState::location)
            .cloned();
        let expected_version = next_pointed_version(&saved)?;
        let restore = |table: &mut Self, error: Error| {
            table.adopt(saved.clone());
            Err(error)
        };
        let mut beaten: u32 = 0;
        let mut backoff_spent_ms = 0_u64;
        loop {
            let updated = match apply(self) {
                Ok(updated) => updated,
                Err(error) => return restore(self, error),
            };
            self.opened_mut()?.metadata = updated;
            let refused = match self.publish_pointed(pointer, staging) {
                Ok(()) => {
                    let committed = self.opened()?;
                    log::info!(
                        "published iceberg metadata version {} of {}{}",
                        committed.version,
                        committed.metadata.location(),
                        match committed.metadata.current_snapshot() {
                            Some(snapshot) => format!(", snapshot {}", snapshot.snapshot_id),
                            None => String::new(),
                        },
                    );
                    return Ok(());
                }
                Err(error) if error.is_conflict() => error,
                Err(error) => return restore(self, error),
            };
            // The pointer moved: where it stands now decides.
            let fresh = match read_pointed(&self.root, pointer) {
                Ok(Some(fresh)) => fresh,
                Ok(None) => return restore(self, refused),
                Err(error) => return restore(self, error),
            };
            let moved = fresh.pointed.as_ref().and_then(PointerState::location) != planned.as_ref();
            if on_conflict == OnConflict::Fail && moved {
                let conflict = CommitConflict {
                    expected_version,
                    beaten: beaten.saturating_add(1),
                    last_seen_version: fresh.version,
                };
                return restore(self, conflict.into());
            }
            let wait = match retry_wait_ms(
                &settings,
                &mut beaten,
                &mut backoff_spent_ms,
                expected_version,
                fresh.version,
            ) {
                Ok(wait) => wait,
                Err(error) => return restore(self, error),
            };
            log::debug!(
                "iceberg commit of {} found its pointer at version {}; retry {beaten}, \
                 waiting {wait} ms",
                fresh.metadata.location(),
                fresh.version,
            );
            self.adopt(fresh);
            if wait > 0 {
                std::thread::sleep(std::time::Duration::from_millis(wait));
            }
        }
    }

    /// Write the held metadata as the next document of a pointed table and
    /// publish it under the token held: one write and one publication.
    ///
    /// The document is `metadata/{version:05}-{uuid}.metadata.json`, the
    /// name Iceberg's catalogs write, its version one above the document
    /// held and `00000` for the first. A publication refused because the
    /// pointer moved is that conflict; one that fails otherwise is in doubt,
    /// since the store may have taken it, so the pointer is read once more,
    /// and a pointer naming this document is the commit made.
    fn publish_pointed(
        &mut self,
        pointer: &dyn MetadataPointer,
        staging: Option<&Staging>,
    ) -> Result<()> {
        let held = self.opened()?;
        held.metadata.validate()?;
        let state = held.pointed.clone().ok_or_else(|| {
            invalid(SmolStr::new_static(
                "expected a pointed table to hold where its pointer stands, got nothing",
            ))
        })?;
        let next_version = next_pointed_version(held)?;
        let previous = state.location().map(ToString::to_string);
        let mut metadata = held.metadata.clone();
        metadata.finalize_official(previous)?;
        let (suffix, encoded) = encoded_document(&metadata)?;
        let name = format_smolstr!("{next_version:05}-{}{suffix}.metadata.json", uuid());
        let mut document = leaf(self.root.child_by_path(&format!("{METADATA_DIR}/{name}"))?)?;
        document.write_all_bytes(&encoded)?;
        let location = crate::Url::from_str(&self.location_of(METADATA_DIR, &name)?)?;
        let published = match pointer.publish(state.token(), &location) {
            Ok(published) => published,
            Err(error) if error.is_conflict() => return Err(error),
            Err(error) => match pointer.current() {
                // The store took the publication and its answer was lost.
                Ok(current) if current.location() == Some(&location) => current,
                _ => return Err(error),
            },
        };
        // The pointer names the document: every file it names is the
        // table's now.
        if let Some(staging) = staging {
            staging.commit();
        }
        let published = match published.location() {
            Some(_) => published,
            None => PointerState::new(Some(location), published.token()),
        };
        self.adopt(Opened {
            metadata,
            version: next_version,
            metadata_file_name: name,
            pointed: Some(published),
        });
        Ok(())
    }

    /// Write the data files, the manifest, the manifest list, and the metadata.
    ///
    /// Returns how many data files the commit wrote. Each partition group's
    /// rows - joined with the group's stored files first when `join` says the
    /// commit is a merge - are sorted by the table's default order and cut
    /// into files of roughly [`Self::target_file_size_bytes`] bytes, numbered
    /// within their group. Groups are written on up to the resolved
    /// [`IcebergOptions::write_parallelism`] threads; the manifest lists their
    /// files in group order, so its bytes do not depend on scheduling, and a
    /// group that fails fails the commit before any metadata is written.
    ///
    /// The expensive half - writing the data files and their one manifest -
    /// happens exactly once. Only the manifest list and the document are
    /// rebuilt per retry attempt, because they are what carry the parent
    /// snapshot and the sequence number a rebase changes. A commit keeping
    /// [`Retained::All`] rebases; one keeping [`Retained::Only`] conflicts
    /// instead, because what it keeps was planned against a snapshot a
    /// concurrent commit may have replaced.
    fn commit(
        &mut self,
        writes: Vec<PartitionWrite>,
        join: Option<&Join>,
        operation: &str,
        retained: Retained,
        threads: Option<usize>,
    ) -> Result<usize> {
        let schema = self.schema()?.clone();
        let spec = self.opened()?.metadata.default_spec()?.clone();
        spec.require_writable()?;
        // The format is resolved and checked against the build before a row
        // is written, so a format this build cannot encode fails up front
        // rather than after data files were written.
        let mut settings =
            IcebergOptions::write_settings(self.options.as_ref(), &self.opened()?.metadata)?;
        if let Some(threads) = threads {
            // The thread count the write's options state is the explicit
            // layer over the table's `write.parallelism` property.
            settings.parallelism = threads.max(1);
        }
        require_encodable(&settings.mime_type)?;
        let (sort, sort_order_id) = sort_orderings(
            self.opened()?.metadata.default_sort_order()?,
            &spec,
            &schema,
        )?;
        let initial_sequence = next_sequence_number(&self.opened()?.metadata)?;
        let snapshot_id = snapshot_id();
        let location = self
            .opened()?
            .metadata
            .location()
            .trim_end_matches('/')
            .to_owned();
        // Every file below goes through the one staging, so a failure
        // anywhere before the document is published rolls all of them back.
        let staging = Staging::begin(settings.staging.as_ref(), self.is_remote(), snapshot_id)?;
        // A pointed table's store need not take a delete: what a failed
        // commit wrote stays, unreferenced, rather than being removed.
        let staging = if self.pointer.is_some() {
            staging.keeping()
        } else {
            staging
        };

        let write = CommitWrite {
            snapshot_id,
            location: &location,
            schema: &schema,
            settings: &settings,
            sort: &sort,
            sort_order_id,
            join,
            staging: &staging,
            file_threads: (settings.parallelism / settings.parallelism.min(writes.len()).max(1))
                .max(1),
        };
        log::debug!(
            "writing an iceberg {operation} snapshot {snapshot_id} to {} in {} partition groups",
            self.opened()?.metadata.location(),
            writes.len(),
        );
        let mut jobs = Vec::with_capacity(writes.len());
        for write in writes {
            let directory = spec.partition_path(&write.values)?;
            // Each group gets its own handle on the table folder: the file
            // writes resolve their full relative path against it, exactly as
            // a single-threaded write does, and the table's own root handle
            // never crosses a thread.
            let root = self.root.child_by_path(".")?;
            jobs.push(PartitionJob {
                write,
                directory,
                root,
            });
        }
        let written = write_partitions(jobs, &write)?;
        let files_written = written.len();

        let added_records = checked_file_sum(&written, |file| file.record_count, "record count")?;
        let added_size = checked_file_sum(&written, |file| file.file_size_in_bytes, "file size")?;
        log::info!(
            "wrote {added_records} rows as {files_written} iceberg data files \
             ({added_size} bytes) for the {operation} snapshot {snapshot_id} of {}",
            self.opened()?.metadata.location(),
        );
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
                &spec,
                &entries,
                initial_sequence,
                &write,
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
                kept.extend(self.carried_manifests(&entries, initial_sequence, &write)?);
                (OnConflict::Fail, Some(kept))
            }
        };

        let operation = SmolStr::new(operation);
        // The live manifests of one snapshot never change, so an attempt
        // beaten on write rather than on the version check re-uses the list
        // it already read; only a rebase onto a newer snapshot reads again.
        let mut listed: Option<(Option<i64>, Vec<ManifestFile>)> = None;
        // The manifest list of the attempt before, when this one is a retry:
        // it names nothing a document will ever name, so it is withdrawn.
        let mut previous_list: Option<String> = None;
        let staged = &staging;
        let apply = move |table: &Self| {
            let sequence_number = next_sequence_number(&table.opened()?.metadata)?;
            let mut manifests = match &kept {
                Some(kept) => kept.clone(),
                None => {
                    let current = table.opened()?.metadata.current_snapshot_id;
                    match &listed {
                        Some((snapshot, manifests)) if *snapshot == current => manifests.clone(),
                        _ => {
                            let manifests = table.manifests()?;
                            listed = Some((current, manifests.clone()));
                            manifests
                        }
                    }
                }
            };
            if let Some(manifest) = &new_manifest {
                let mut row = manifest.clone();
                row.sequence_number = sequence_number;
                // Every entry in it is `added`, so the floor is this commit's.
                row.min_sequence_number = sequence_number;
                manifests.push(row);
            }

            let list_name = format!("snap-{snapshot_id}-1-{}.avro", uuid());
            let first_row_id =
                if table.opened()?.metadata.format_version >= FormatVersion::V3 {
                    Some(table.opened()?.metadata.next_row_id.ok_or_else(|| {
                        Error::InvalidRecord {
                            path: "$.iceberg.next-row-id".into(),
                            reason: SmolStr::new_static("expected a v3 next-row-id, got none"),
                        }
                    })?)
                } else {
                    None
                };
            let format_version = table.opened()?.metadata.format_version;
            let parent_snapshot_id = table.opened()?.metadata.current_snapshot_id;
            if let Some(previous) = previous_list.take() {
                // The attempt this one replaces lost: its list goes now, one
                // removal, rather than staying as the orphan a successful
                // commit would otherwise keep.
                staged.withdraw(&previous)?;
            }
            let list_path = format!("{METADATA_DIR}/{list_name}");
            let (next_row_id, _) = staged.publish(
                &table.root,
                &list_path,
                &crate::MediaType::new(MimeType::AVRO),
                |list| {
                    write_manifest_list(
                        list,
                        format_version,
                        snapshot_id,
                        parent_snapshot_id,
                        sequence_number,
                        first_row_id,
                        &manifests,
                    )
                },
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
                parent_snapshot_id: table.opened()?.metadata.current_snapshot_id,
                sequence_number: (table.opened()?.metadata.format_version >= FormatVersion::V2)
                    .then_some(sequence_number),
                timestamp_ms: now_ms(),
                manifest_list: SmolStr::new(table.location_of(METADATA_DIR, &list_name)?),
                manifests: None,
                summary,
                schema_id: Some(table.opened()?.metadata.current_schema_id),
                encryption_key_id: None,
                first_row_id,
                added_rows: assigned_rows,
            };

            let mut updated = table.opened()?.metadata.clone();
            updated.set_current_snapshot(snapshot)?;
            previous_list = Some(list_path);
            Ok(updated)
        };
        // The staging is committed inside, the moment the versioned document
        // is durable; what is left of it when it drops is the directory.
        self.commit_document(on_conflict, apply, Some(&staging))?;
        Ok(files_written)
    }

    /// Reject rewrites that would assign fresh row IDs to retained v3 rows.
    fn require_row_id_preserving_rewrite(&self, operation: &'static str) -> Result<()> {
        if self.opened()?.metadata.format_version >= FormatVersion::V3 {
            return Err(Error::iceberg(format_smolstr!(
                "{operation} is not supported for Iceberg format v3: rewritten rows cannot yet preserve their existing row IDs"
            )));
        }
        Ok(())
    }

    /// Write one manifest and describe it as a manifest list row.
    ///
    /// The counts a caller cares about are filled in afterwards, because what
    /// makes an entry added or existing is the commit's business rather than
    /// this write's.
    fn write_manifest_file(
        &self,
        name: &str,
        spec: &PartitionSpec,
        entries: &[ManifestEntry],
        sequence_number: i64,
        write: &CommitWrite<'_>,
    ) -> Result<ManifestFile> {
        let schema = write.schema;
        let snapshot_id = write.snapshot_id;
        let staging = write.staging;
        let version = self.opened()?.metadata.format_version;
        let ((), length) = staging.publish(
            &self.root,
            &format!("{METADATA_DIR}/{name}"),
            &crate::MediaType::new(MimeType::AVRO),
            |handle| write_manifest(handle, version, schema, spec, entries),
        )?;
        Ok(ManifestFile {
            manifest_path: SmolStr::new(self.location_of(METADATA_DIR, name)?),
            manifest_length: i64::try_from(length).map_err(|_| {
                invalid(format_smolstr!(
                    "expected a manifest size fitting i64, got {length}"
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
        sequence_number: i64,
        write: &CommitWrite<'_>,
    ) -> Result<Vec<ManifestFile>> {
        let snapshot_id = write.snapshot_id;
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
            let mut manifest =
                self.write_manifest_file(&name, &spec, &entries, sequence_number, write)?;
            manifest.existing_files_count = Some(existing_files_count);
            manifest.existing_rows_count = Some(existing_rows_count);
            manifests.push(manifest);
        }
        Ok(manifests)
    }

    /// Build the URI of one child of a table directory.
    fn location_of(&self, directory: &str, name: &str) -> Result<String> {
        Ok(format!(
            "{}/{directory}/{name}",
            self.opened()?.metadata.location.trim_end_matches('/')
        ))
    }
}

/// The table is itself a handle: the byte surface is the folder it lives in,
/// and the record surface is answered from the parsed metadata this value
/// already holds.
///
/// A plain container handle addressing the table's folder answers the same
/// contract - the three record methods, a commit per cadence of a write -
/// by probing the location for a table on every call. Holding the
/// [`IcebergTable`] skips the probe: no metadata document is re-read,
/// [`crate::IOMedia::read_arrow_field`] is
/// [`IcebergTable::schema`] with its field identifiers and protocol metadata rather
/// than a shape lifted off decoded batches, and a
/// [`partition_pairs`](IORecordOptions::partition_pairs) pair prunes data
/// files through [`IcebergTable::plan`] instead of filtering rows after they were
/// decoded. The in-memory metadata stays current across commits, so
/// [`IcebergTable::current_snapshot`] and [`IcebergTable::metadata_version`] reflect a write made
/// through this surface without reopening anything.
///
/// One deliberate difference from the folder route: a filter naming a column
/// the schema does not declare is an error here, exactly as
/// [`IcebergTable::scan_where`] reports it, where a folder of leaves ignores a column
/// its batches do not carry. A table's schema is authoritative, so a filter it
/// cannot answer is a mistake worth naming rather than a row set worth
/// guessing.
impl<H: IOBase> IOBase for IcebergTable<H> {
    // `kind` is answered below: storage sees a folder, and this handle is
    // the table that folder holds.
    crate::delegate_iobase!(root: pread, pwrite, size, capacity, reserve,
        truncate, uri, url, bound_location, mtime, media_type, set_media_type, flush, parent,
        child_by_path);

    /// The files below the table: its folder's listing, or for a table a
    /// [`MetadataPointer`] names one refusal, touching nothing - its
    /// catalog's warehouse location need not take a listing.
    fn ls(&self, recursive: bool, include_private: bool) -> crate::Listing {
        if self.pointer.is_some() {
            return crate::Listing::failing(kept_by_catalog(
                &self.root,
                "listing the files of a table its catalog keeps",
            ));
        }
        self.root.ls(recursive, include_private)
    }

    /// Stream nothing: the files below a table - its metadata, its manifests,
    /// its data - are its storage, not its bytes, so the root folder's stream
    /// of them is not this handle's. The table is read as rows.
    fn pstream_bytes(&self, _position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        crate::ByteStream::from_reader(std::io::empty(), batch_size)
    }

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
        if self.current_snapshot()?.is_none() {
            // Nothing has ever been written, so there is nothing to replace.
            return Ok(());
        }
        let schema = crate::arrow::arrow_schema_from_field(self.schema()?)?;
        self.commit_overwrite_where(&[], crate::arrow::batch_reader(schema, []))
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
    /// The in-memory [`IcebergTable`] value describes a table that no longer exists
    /// once this returns, so it must not be committed to afterwards; reopen the
    /// location instead. The handle itself stays usable and lazy, per the
    /// contract.
    ///
    /// A table a [`MetadataPointer`] names is dropped by the catalog that
    /// keeps it - [`MetadataPointer::remove`], one `DeleteTable` for an
    /// Amazon S3 Tables table - whatever `recursive` says, and its folder is
    /// asked for nothing. The document this value read is forgotten, so what
    /// it asks next is asked of the pointer again and a later verb fails as
    /// absent rather than commit to a table that is gone.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure, a refusal naming the
    /// location when it still has children and `recursive` is not set, and
    /// for a table a pointer names the pointer's own failure -
    /// [`Error::Unsupported`] naming the table's location for a pointer that
    /// drops nothing.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        let Some(pointer) = &self.pointer else {
            return self.root.remove(recursive);
        };
        match pointer.remove() {
            Ok(()) => {
                self.opened = OnceLock::new();
                Ok(())
            }
            // A pointer that drops nothing says so by its own name; the
            // refusal a caller reads names the table, as the listing's does.
            // Any other failure of the drop - the catalog's own refusal, the
            // transport's - is the caller's as it came.
            Err(Error::Unsupported { operation, .. })
                if operation == super::pointer::DROPS_NOTHING =>
            {
                Err(kept_by_catalog(
                    &self.root,
                    "dropping a table through its pointer, which drops none",
                ))
            }
            Err(error) => Err(error),
        }
    }
}

impl<H: IOBase + Sync + std::fmt::Debug> ObjectValue for IcebergTable<H> {
    fn name(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Table
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn url(&self) -> Option<&crate::Url> {
        self.root.url()
    }

    fn modified(&self) -> Option<i64> {
        self.root.mtime()
    }

    /// The parent's effective properties, then the table properties the
    /// current metadata document keeps, then what was stated for it.
    fn properties(&self) -> Result<Properties> {
        let mut stored = Properties::new();
        for (key, value) in &self.opened()?.metadata.properties {
            stored.set(key.clone(), value.clone());
        }
        Ok(self.stated.inherit(&stored.inherit(&self.inherited)))
    }

    /// A table's stored properties live in its metadata document, and
    /// writing one is a commit, which needs the table held mutably:
    /// [`IcebergTable::commit_metadata_changes`] is that door.
    fn update_properties(&self, _updates: &Properties, _removes: &[SmolStr]) -> Result<()> {
        Err(Error::unsupported(
            "updating the properties it keeps through a shared table; commit them \
             through IcebergTable::commit_metadata_changes",
            "IcebergTable",
        ))
    }
}

impl<H: IOBase + Sync + std::fmt::Debug> TableValue for IcebergTable<H> {
    fn field(&self) -> Result<Field> {
        self.schema().cloned()
    }

    fn storage(&self) -> String {
        IOKind::Table.as_str().to_owned()
    }
}

impl<H: IOBase> crate::IOMedia for IcebergTable<H> {
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
        let Some(snapshot) = self.current_snapshot()? else {
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

    /// The stored schema as the metadata declares it, no data file opened:
    /// a declared schema as it stands, else [`IcebergTable::schema`] renamed
    /// to the options' root name, field identifiers and protocol metadata
    /// included. Either way its `SORT:by` is the order a
    /// record read of these options proves - none where it proves none -
    /// rather than the order the table's writers keep.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        let (landing, _) = self.read_landing(options)?;
        let sorted = self.read_sorting(&landing)?.len();
        let proven = self.read_order(&landing, options.select(), sorted)?;
        let root = match options.field() {
            Some(field) => field,
            None => self.schema()?.clone().with_name(options.name()),
        };
        super::scan::declaring(root.with_metadata_removed("SORT:by"), proven)
    }

    /// Scan the current snapshot, the whole `where` clause answered by the plan.
    ///
    /// The clause is pushed into the scan as [`IcebergTable::scan_matching`] takes
    /// it, so every spelling the expression language has prunes: `venue in
    /// ('XNAS', 'XLON')` and `ts between ... and ...` skip the same manifests
    /// and files an equality does. The read decodes only the columns the
    /// `select` and the `where` name. The rows arrive as [`Self::read_serie`]
    /// yields them - partition after partition, each in the table's sort
    /// order - as transport.
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<BatchReader> {
        self.read_scoped(Filter::always_true(), options)
    }

    /// The table's rows partition after partition, in ascending partition
    /// tuple order, each partition's rows in the table's default sort order,
    /// the root declaring the order the stream proves.
    ///
    /// One partition is held at a time, spilled under the process bound,
    /// and decoded only once the one before it has been yielded. A read the
    /// options shape - a `select`, a `where` after it, a row bound - is
    /// [`Self::read_arrow_reader`]'s stream landed once, the order it
    /// declares carried as proven rather than read batch by batch again.
    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::SerieReader> {
        let owned;
        let options = match options {
            Some(options) => options,
            None => {
                owned = self.record_options()?;
                &owned
            }
        };
        let bounded = options.max_row_size().is_some()
            || options.max_byte_size().is_some()
            || options.row_offset().is_some_and(|rows| rows != 0);
        // Under `*` the whole `where` clause was pushed into the plan, so the
        // rows need nothing past what the read itself yields.
        if options.select().is_all() && !bounded {
            return Ok(self
                .read_rows(Filter::always_true(), options)?
                .0
                .into_serie_reader()?);
        }
        // The selector, a `where` after it and the bounds are Arrow's, so
        // what they shape lands once, under its root without the order, and
        // each record is relabelled under the root declaring it. Lazy, so no
        // record is read against that order again: it was proven before
        // them - its keys are the ones the selector publishes unchanged
        // (`proven_order`) - and a `where` keeps its rows in their order, a
        // bound a run of them.
        let shaped = self.read_scoped(Filter::always_true(), options)?;
        let declared = crate::arrow::field_from_arrow_schema(
            crate::media::DEFAULT_ROOT_NAME,
            shaped.schema().as_ref(),
        )?;
        let plain = declared.clone().with_metadata_removed("SORT:by");
        let landed = crate::SerieReader::from_arrow_reader(
            Some(&plain),
            shaped,
            crate::ArrowCastOptions::default(),
        )?;
        if !declared.as_sort().declares_order() {
            return Ok(landed);
        }
        let root = std::sync::Arc::new(declared);
        let relabel = std::sync::Arc::clone(&root);
        Ok(crate::SerieReader::from_landed_iter(
            root,
            landed.map(move |record| {
                record.map(|record| record.into_relabeled(std::sync::Arc::clone(&relabel)))
            }),
        )?)
    }

    /// Replace the partitions the rows fall in - the partitions the
    /// options' `where` addresses where it states any, an unpartitioned
    /// table whole: `write_cadenced` under
    /// [`IOMode::Overwrite`](crate::IOMode::Overwrite). No other partition
    /// is touched, and a source with no row replaces none.
    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        let filters: Vec<(String, String)> = options.partition_pairs();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.write_cadenced(batches, crate::IOMode::Overwrite, options, &pairs)
    }

    /// Add the rows: `write_cadenced` under
    /// [`IOMode::Append`](crate::IOMode::Append), an `append` snapshot per
    /// commit, each keeping every manifest the last one had.
    fn append_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        self.write_cadenced(batches, crate::IOMode::Append, options, &[])
    }

    /// Merge into the selected partitions: `write_cadenced` under
    /// [`IOMode::Merge`](crate::IOMode::Merge). The partition columns lead
    /// the match key, so an empty [`merge_by`](IORecordOptions::merge_by)
    /// on a partitioned table replaces the partitions the rows fall in; see
    /// [`IcebergTable::commit_merge_where`].
    fn merge_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        let filters: Vec<(String, String)> = options.partition_pairs();
        let pairs: Vec<(&str, &str)> = filters
            .iter()
            .map(|(column, value)| (column.as_str(), value.as_str()))
            .collect();
        self.write_cadenced(batches, crate::IOMode::Merge, options, &pairs)
    }
}

impl<H: IOBase> IcebergTable<H> {
    /// One streamed write under `mode`, over the partitions `pairs` address.
    ///
    /// The rows are shaped onto the stored schema once - the columns it
    /// derives computed between the options' declared field and their
    /// clauses - and cut into cadences: every
    /// [`commit_batch_num`](IORecordOptions::commit_batch_num)
    /// batches where one is stated, else one commit when the source ends -
    /// the rows of every partition held under the process spill bound until
    /// then, so a stream of any length is one atomic snapshot, and
    /// `commit_batch_num` paces a stream whose rows would outgrow the spill
    /// folder. Each cadence is one commit: its partition groups written on
    /// [`num_threads`](IORecordOptions::num_threads) threads at once where
    /// stated, else the table's `write.parallelism`, each group's rows in
    /// the table's sort order and cut into files at
    /// [`Self::target_file_size_bytes`]. An overwrite of a partitioned
    /// table addressed whole replaces each partition its rows fall in on the
    /// first commit that reaches it and appends to it on every later one,
    /// touching no other partition; an overwrite of stated `pairs`, or of
    /// an unpartitioned table, replaces that scope on its first commit and
    /// appends after. An append appends, and every commit of a merge merges
    /// by its key - a merge keyed by the partition alone replacing
    /// partitions as an overwrite does. The commits before a failure stay
    /// published.
    ///
    /// # Errors
    ///
    /// Returns the options' refusal of the mode, a zero cadence or thread
    /// count, a limit on a merge, and a metadata, manifest, read, cast or
    /// write failure.
    pub(crate) fn write_cadenced(
        &mut self,
        batches: BatchReader,
        mode: crate::IOMode,
        options: &RecordOptions,
        pairs: &[(&str, &str)],
    ) -> Result<crate::IOResult> {
        match mode {
            crate::IOMode::Overwrite => options.require_write_mode(mode)?,
            crate::IOMode::Append => {
                options.require_write_mode(mode)?;
                options.require_write_limits()?;
                if options.write_limit_is_zero() {
                    return Ok(crate::IOResult::default());
                }
            }
            crate::IOMode::Merge => {
                // The generic rule - a merge names a key - is met by the
                // partition columns of a partitioned table, so only an
                // unpartitioned one has to be told what to match on.
                if self.opened()?.metadata.default_spec()?.is_unpartitioned()
                    || !options.merge_by().is_empty()
                {
                    options.require_write_mode(mode)?;
                }
                options.require_write_limits()?;
            }
            crate::IOMode::ReadOnly | crate::IOMode::Random => {
                return Err(Error::InvalidRecord {
                    path: SmolStr::new_static("$.mode"),
                    reason: SmolStr::new_static(
                        "write mode readonly or random is not supported for an iceberg table",
                    ),
                });
            }
        }
        let threads = options.require_num_threads()?;
        let cadence = options.commit_cadence(Cadence::Once)?;
        let stored = self.schema()?.clone();
        let overwrite = mode == crate::IOMode::Overwrite;
        let batches = if overwrite {
            Some(batches)
        } else {
            crate::iobase::non_empty_arrow_reader(batches)?
        };
        let Some(batches) = batches else {
            return Ok(crate::IOResult::default());
        };
        let (batches, count) =
            crate::iobase::prepare_arrow_write_deriving(batches, options, &stored)?;
        let batches = if overwrite {
            Some(batches)
        } else {
            crate::iobase::non_empty_arrow_reader(batches)?
        };
        let Some(batches) = batches else {
            return Ok(count.result());
        };
        let schema = batches.schema();
        let mut commits = options.commit_arrow_readers(batches, cadence)?;
        match mode {
            crate::IOMode::Overwrite => {
                let mut replaced = ReplacedPartitions::default();
                let Some(first) = commits.next() else {
                    self.commit_overwrite_cadence(
                        pairs,
                        crate::arrow::batch_reader(schema, []),
                        &mut replaced,
                        threads,
                    )?;
                    return Ok(count.result());
                };
                self.commit_overwrite_cadence(pairs, first?, &mut replaced, threads)?;
                for commit in commits {
                    self.commit_overwrite_cadence(pairs, commit?, &mut replaced, threads)?;
                }
            }
            crate::IOMode::Append => {
                for commit in commits {
                    self.commit_append_on(commit?, threads)?;
                }
            }
            crate::IOMode::Merge => {
                let mut replaced = ReplacedPartitions::default();
                for commit in commits {
                    self.commit_merge_cadence(
                        pairs,
                        commit?,
                        options.merge_by(),
                        options.safe(),
                        &mut replaced,
                        threads,
                    )?;
                }
            }
            crate::IOMode::ReadOnly | crate::IOMode::Random => unreachable!("refused above"),
        }
        Ok(count.result())
    }
}

/// What one [`IcebergTable::compact`] call did, in numbers a caller can assert on.
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
        crate::hashing::stable_hash_of(self)
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
        crate::hashing::stable_hash_of(self)
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

/// What one streamed write has replaced, carried across its commits.
///
/// An overwrite of a partitioned table, and a merge keyed by the partition
/// columns alone, replace the partitions their rows fall in; cut into
/// several commits, such a write replaces a partition on the first commit
/// that reaches it and appends to it on every later one
/// ([`IcebergTable::commit_overwrite_cadence`],
/// [`IcebergTable::commit_merge_cadence`]). An overwrite of a stated scope,
/// or of an unpartitioned table, replaces on its first commit alone, which
/// `scope` records. One value per write; a keyed merge records nothing.
///
/// Bounded by the partitions the write's rows fall in: one tuple each,
/// held until the write ends, because a later commit may reach any of them.
#[derive(Debug, Default)]
pub(crate) struct ReplacedPartitions {
    /// The partition tuples replaced so far, in spec order.
    tuples: HashSet<Vec<Scalar>>,
    /// Whether an overwrite of a stated scope, or of an unpartitioned
    /// table, has replaced it: every later commit of the write appends.
    scope: bool,
}

impl ReplacedPartitions {
    /// Whether a commit of this write already replaced the partition.
    fn contains(&self, tuple: &[Scalar]) -> bool {
        self.tuples.contains(tuple)
    }
}

/// One partition's rows to write, and the stored files they merge with.
struct PartitionWrite {
    /// The partition tuple every row computes to, in spec order.
    values: Vec<Scalar>,
    /// The rows, cast to the table schema and landed under it: one chunk per
    /// piece an incoming batch gave this partition - a batch falling whole
    /// in it the batch itself, a run of its rows a slice, interleaved rows
    /// one take - settled under the process spill bound as the pieces
    /// arrive, so a commit of any size holds that bound in memory.
    hold: ChunkedSerie,
    /// The stored files of this partition a keyed merge joins with, resolved
    /// to handles; empty for an append, an overwrite, or a partition replace.
    stored: Vec<ScanPart>,
}

/// Every batch a hold's rows export as, in row order: zero-copy over the
/// hold's buffers, mapped or not.
fn hold_batches(hold: &ChunkedSerie) -> Result<Vec<RecordBatch>> {
    hold.into_arrow_reader()?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::Arrow)
}

/// One partition group handed to a writer thread, with where it lands.
struct PartitionJob {
    /// The group's rows and stored files.
    write: PartitionWrite,
    /// The Hive directory chain the partition tuple spells, empty when
    /// unpartitioned.
    directory: String,
    /// The thread's own handle on the table folder, resolved before it
    /// starts so a writer never touches the table's root handle.
    root: Holder,
}

/// The match key a merge joins on within one partition group.
struct Join {
    /// The key columns beyond the partition, which is constant in a group.
    keys: Selector,
    /// Whether an incoming value that cannot cast to its column is an error.
    safe: bool,
}

/// What every partition group of one commit is written with.
///
/// Shared by every writer thread by reference, so it holds nothing a thread
/// could own: the table's root handle never crosses.
struct CommitWrite<'a> {
    /// The snapshot the files belong to, which names them.
    snapshot_id: i64,
    /// The table's location, which every recorded file path starts with.
    location: &'a str,
    /// The schema every batch was cast to.
    schema: &'a Field,
    /// The resolved format, target size, parallelism, and read settings.
    settings: &'a WriteSettings,
    /// The keys every file's rows are ordered by, most significant first.
    sort: &'a [crate::expression::Ordering],
    /// The order recorded on each file, when the table declares one.
    sort_order_id: Option<i32>,
    /// The match key, when the commit is a keyed merge.
    join: Option<&'a Join>,
    /// The staging every file of the commit is published through.
    staging: &'a Staging,
    /// The threads one file's columns encode on: the write parallelism's
    /// share left over by the partition groups written side by side.
    file_threads: usize,
}

/// Resolve the table's default sort order into the `order by` keys files
/// sort by.
///
/// A sort field is honoured through its source column: an identity, a
/// truncation, and every calendar transform order exactly as their source
/// does, and a bucket orders by its source value rather than its hash. The
/// unsorted order resolves to no keys, and a file written under it records
/// no order id.
///
/// A column `spec` partitions by identity is left out of the sort: every row
/// of one partition group holds the same value of it, so ordering by it
/// moves nothing, and the order derived from a spec is exactly those
/// columns. A floating source is the exception, because one group can hold
/// both zeros, which the order tells apart. The file still records the
/// order id, because its rows are in that order.
fn sort_orderings(
    order: &SortOrder,
    spec: &PartitionSpec,
    schema: &Field,
) -> Result<(Vec<crate::expression::Ordering>, Option<i32>)> {
    let mut keys = Vec::with_capacity(order.fields.len());
    for field in &order.fields {
        let (path, source) = super::partition::source_path(schema, field.source_id)?;
        let constant = !source.dtype().id().is_floating()
            && spec.fields.iter().any(|partition| {
                partition.source_id == field.source_id
                    && partition.transform == super::partition::Transform::Identity
            });
        if constant {
            continue;
        }
        let (first, nested) = path
            .split_first()
            .ok_or_else(|| invalid(SmolStr::new_static("expected a non-empty sort column path")))?;
        let mut term = Term::column(first.clone());
        if !nested.is_empty() {
            term = term.path(
                nested
                    .iter()
                    .map(|name| crate::expression::FieldSegment::field(name.clone())),
            )?;
        }
        let options = if field.direction == "desc" {
            crate::SortOptions::descending()
        } else {
            crate::SortOptions::ascending()
        }
        .with_nulls_first(field.null_order == "nulls-first");
        keys.push(crate::expression::Ordering::new(term, options));
    }
    let order_id = (!order.fields.is_empty())
        .then(|| i32::try_from(order.order_id).ok())
        .flatten();
    Ok((keys, order_id))
}

/// Write every partition group, on up to the resolved parallelism threads.
///
/// The files come back in group order whatever order the groups finished
/// in, so the manifest a commit writes does not depend on scheduling. The
/// first failing group fails the whole commit: the others stop at their
/// next group rather than writing files no manifest will name.
fn write_partitions(jobs: Vec<PartitionJob>, write: &CommitWrite<'_>) -> Result<Vec<DataFile>> {
    let parallelism = write.settings.parallelism.min(jobs.len()).max(1);
    if parallelism == 1 {
        let mut written = Vec::new();
        for job in jobs {
            written.extend(write_partition(job, write)?);
        }
        return Ok(written);
    }
    let total = jobs.len();
    let queue: Mutex<VecDeque<(usize, PartitionJob)>> =
        Mutex::new(jobs.into_iter().enumerate().collect());
    let outcomes: Mutex<Vec<Option<Result<Vec<DataFile>>>>> =
        Mutex::new((0..total).map(|_| None).collect());
    let failed = AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..parallelism {
            scope.spawn(|| {
                loop {
                    if failed.load(Ordering::Acquire) {
                        return;
                    }
                    let next = match queue.lock() {
                        Ok(mut queue) => queue.pop_front(),
                        Err(_) => return,
                    };
                    let Some((index, job)) = next else {
                        return;
                    };
                    let outcome = write_partition(job, write);
                    if outcome.is_err() {
                        failed.store(true, Ordering::Release);
                    }
                    if let Ok(mut outcomes) = outcomes.lock() {
                        outcomes[index] = Some(outcome);
                    }
                }
            });
        }
    });
    let outcomes = outcomes.into_inner().map_err(|_| {
        invalid(SmolStr::new_static(
            "expected every partition writer to finish, got a poisoned result",
        ))
    })?;
    let mut written = Vec::new();
    for outcome in outcomes {
        match outcome {
            Some(Ok(files)) => written.extend(files),
            Some(Err(error)) => return Err(error),
            None => {
                return Err(invalid(SmolStr::new_static(
                    "expected every partition group to be written, got one no writer took",
                )));
            }
        }
    }
    Ok(written)
}

/// Write one partition group's files: join, sort, cut, encode.
///
/// A keyed merge reads the group's selected stored files here, on the
/// writer's own thread, and joins them with the group's rows - the last of
/// the rows arriving with one key wins - so a group's stored files are in
/// memory only while its files are being written.
fn write_partition(job: PartitionJob, write: &CommitWrite<'_>) -> Result<Vec<DataFile>> {
    let PartitionJob {
        write: group,
        directory,
        root,
    } = job;
    let hold = match write.join {
        Some(join) => {
            let arrow_schema = crate::arrow::arrow_schema_from_field(write.schema)?;
            let stored = if group.stored.is_empty() {
                crate::arrow::batch_reader(arrow_schema, [])
            } else {
                super::scan::reader(
                    group.stored,
                    write.schema.clone(),
                    write.schema.clone(),
                    None,
                    Vec::new(),
                    &write.settings.read,
                    false,
                )?
            };
            let merged = crate::media::merge::merged(
                stored,
                group.hold.into_arrow_reader()?,
                write.schema,
                &join.keys,
                join.safe,
            )?;
            // The merge answers transport batches, so its rows land under
            // the schema again - without the order it declares, which is
            // sorted below - each batch settled as it arrives.
            let landing = write.schema.clone().with_metadata_removed("SORT:by");
            ChunkedSerie::from_arrow_reader(Some(&landing), merged, ArrowCastOptions::new())?
        }
        None => group.hold,
    };
    if hold.is_empty() {
        return Ok(Vec::new());
    }
    // A group already in order - by its root's proven declaration, or read
    // once chunk by chunk and edge by edge - goes to the encoder as the
    // chunks it arrived in; one out of order is sorted out of core, each
    // chunk on its own and the chunks merged, the output settled.
    let hold = if write.sort.is_empty() || hold.keeps_order(write.sort)? {
        hold
    } else {
        hold.into_sort_by(write.sort)?
    };
    let rows: Vec<RecordBatch> = hold_batches(&hold)?
        .into_iter()
        .filter(|batch| batch.num_rows() > 0)
        .collect();
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let mut written = Vec::new();
    for slice in sliced(rows, write.settings.target_file_size_bytes) {
        // Deliberately no record per file: a commit is the unit worth
        // watching, and a wide partition write is thousands of files.
        written.push(write_data_file(
            &root,
            &directory,
            write,
            written.len(),
            &group.values,
            slice,
        )?);
    }
    Ok(written)
}

/// Cut one partition group's ordered rows into files of roughly `target` bytes.
///
/// The estimate is the group's Arrow in-memory size - each slice's own
/// extent, not the buffers it shares - spread evenly over its rows, taken
/// *before* encoding: Parquet compresses what it writes, so the files
/// land under the target rather than at it. A file holds at least one row,
/// so a target below one row's size is one file per row. The cuts fall at
/// the same row offsets whether the rows arrive as one batch or several; a
/// file is the zero-copy slices of the batches its rows span.
fn sliced(batches: Vec<RecordBatch>, target: u64) -> Vec<Vec<RecordBatch>> {
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    let bytes = batches.iter().fold(0_u64, |total, batch| {
        total.saturating_add(u64::try_from(crate::arrow::memory_size(batch)).unwrap_or(u64::MAX))
    });
    if rows == 0 || bytes <= target {
        return vec![batches];
    }
    let per_file = (u128::from(target) * rows as u128 / u128::from(bytes)).max(1);
    let per_file = usize::try_from(per_file).unwrap_or(rows).max(1);
    let mut files = Vec::with_capacity(rows.div_ceil(per_file));
    let mut file = Vec::new();
    let mut filled = 0;
    for batch in batches {
        let mut offset = 0;
        while offset < batch.num_rows() {
            let length = (per_file - filled).min(batch.num_rows() - offset);
            file.push(batch.slice(offset, length));
            filled += length;
            offset += length;
            if filled == per_file {
                files.push(std::mem::take(&mut file));
                filled = 0;
            }
        }
    }
    if !file.is_empty() {
        files.push(file);
    }
    files
}

/// Write one file of one partition group and describe it.
///
/// The file's name carries the MIME type's own extension, so the handle's
/// media type - which is what selects the encoder - agrees with the
/// `file_format` the manifest will record. A Parquet file's statistics are
/// read back from the footer that was just written; any other format has no
/// footer this crate reads, so its statistics are measured from the batches
/// before they are encoded. An `unknown` column is left out of the file, as
/// the spec requires; the scan restores it as null.
fn write_data_file(
    root: &Holder,
    directory_path: &str,
    write: &CommitWrite<'_>,
    index: usize,
    values: &[Scalar],
    batches: Vec<RecordBatch>,
) -> Result<DataFile> {
    let snapshot_id = write.snapshot_id;
    let schema = write.schema;
    let mime_type = &write.settings.mime_type;
    let extension = mime_type
        .extension()
        .ok_or_else(|| not_encodable(mime_type))?;
    let name = format!("{index:05}-{snapshot_id}-{}.{extension}", uuid());
    let (stored, batches) = stored_columns(schema, batches)?;
    let relative = if directory_path.is_empty() {
        format!("{DATA_DIR}/{name}")
    } else {
        format!("{DATA_DIR}/{directory_path}/{name}")
    };

    // The statistics are read back from the file the writer just wrote -
    // the staged copy when the commit stages, so the footer read costs the
    // store nothing - before the file reaches the table.
    let arrow_schema = crate::arrow::arrow_schema_from_field(&stored)?;
    let parquet = mime_type == &MimeType::PARQUET;
    let nans = super::statistics::nan_value_counts(schema, &batches)?;
    let (mut file, size) = write.staging.publish(
        root,
        &relative,
        &crate::MediaType::new(mime_type.clone()),
        |handle| {
            let mut options = handle
                .record_options()?
                .with_safe(false)
                .with_field(stored.clone());
            options.set_file_threads(write.file_threads);
            if parquet {
                handle.overwrite_arrow_reader(
                    crate::arrow::batch_reader(arrow_schema, batches),
                    &options,
                )?;
                handle.flush()?;
                let statistics = crate::parquet::read_statistics(handle)?;
                super::statistics::data_file(schema, &statistics)
            } else {
                // The batches are measured before the write consumes them,
                // because this format's file carries no footer to read them
                // from.
                let file = super::statistics::data_file_from_batches(schema, &batches)?;
                handle.overwrite_arrow_reader(
                    crate::arrow::batch_reader(arrow_schema, batches),
                    &options,
                )?;
                handle.flush()?;
                Ok(file)
            }
        },
    )?;
    file.file_path = SmolStr::new(if directory_path.is_empty() {
        format!("{}/{DATA_DIR}/{name}", write.location)
    } else {
        format!("{}/{DATA_DIR}/{directory_path}/{name}", write.location)
    });
    file.mime_type = mime_type.clone();
    file.file_size_in_bytes = i64::try_from(size).map_err(|_| {
        invalid(format_smolstr!(
            "expected a data file size fitting i64, got {size}"
        ))
    })?;
    file.partition = values.to_vec();
    file.sort_order_id = write.sort_order_id;
    file.nan_value_counts = nans;
    Ok(file)
}

/// Store each `unknown` column as the spec keeps one out of data files.
///
/// A top-level `unknown` column is left out of the file, and the scan restores
/// it from the schema as null. One inside a struct, a list or a map has a
/// slot its parent's layout keeps, and stores Arrow's null column there -
/// never a variant group, which a promoted column could not read back. Each
/// slot is read first, because an `unknown` column holds no value and one
/// written into it would be lost here rather than stored: a cell is absent or
/// the variant null, or the write is refused naming the column and the value.
/// A schema with none hands the batches back untouched, unread.
fn stored_columns(schema: &Field, batches: Vec<RecordBatch>) -> Result<(Field, Vec<RecordBatch>)> {
    if !schema.fields().iter().any(holds_unknown) {
        return Ok((schema.clone(), batches));
    }
    let mut slots = Vec::new();
    let mut kept = Vec::with_capacity(schema.field_len());
    for field in schema.fields() {
        unknown_slots(field, field.name().to_owned(), &mut slots);
        if !field.as_iceberg().is_unknown() {
            kept.push(stored_slots(field)?);
        }
    }
    let mut stored = schema.clone();
    stored.set_dtype(DataType::from(StructType::from_fields(kept)?))?;
    let arrow = crate::arrow::arrow_schema_from_field(&stored)?;

    // One pass per batch lays out its stored columns and gathers each slot's
    // arrays, so every slot is read through one plan across the batches.
    let mut held: Vec<Vec<ArrayRef>> = vec![Vec::with_capacity(batches.len()); slots.len()];
    let mut stored_batches = Vec::with_capacity(batches.len());
    for batch in &batches {
        let mut gathered = held.iter_mut();
        let mut columns = Vec::with_capacity(arrow.fields().len());
        for field in schema.fields() {
            let column = batch.column_by_name(field.name()).ok_or_else(|| {
                invalid(format_smolstr!(
                    "expected the table column {:?} in a batch cast to the table schema",
                    field.name()
                ))
            })?;
            let target = arrow.field_with_name(field.name()).ok();
            let data = stored_data(
                field,
                target.map(|target| target.data_type()),
                column.to_data(),
                &mut gathered,
            )?;
            if target.is_some() {
                columns.push(arrow_array::make_array(data));
            }
        }
        stored_batches.push(RecordBatch::try_new(
            std::sync::Arc::clone(&arrow),
            columns,
        )?);
    }
    for ((path, field), arrays) in slots.iter().zip(held) {
        let column = crate::ChunkedSerie::from_arrow_arrays(
            Some(field),
            arrays,
            ArrowCastOptions::default(),
        )?;
        for chunk in column.chunks() {
            require_no_value(chunk, path)?;
        }
    }
    Ok((stored, stored_batches))
}

/// Whether a column is `unknown` or holds one at any depth.
fn holds_unknown(field: &Field) -> bool {
    field.as_iceberg().is_unknown()
        || (0..field.dtype().field_len())
            .filter_map(|index| field.dtype().get_field_at(index))
            .any(holds_unknown)
}

/// Every `unknown` slot of one column, depth first, with the path that names
/// it: the order [`stored_data`] meets them in.
fn unknown_slots<'field>(
    field: &'field Field,
    path: String,
    out: &mut Vec<(String, &'field Field)>,
) {
    if field.as_iceberg().is_unknown() {
        out.push((path, field));
        return;
    }
    for child in
        (0..field.dtype().field_len()).filter_map(|index| field.dtype().get_field_at(index))
    {
        if holds_unknown(child) {
            unknown_slots(child, format!("{path}.{}", child.name()), out);
        }
    }
}

/// One column as its data file declares it: each `unknown` slot inside it
/// is Arrow's null column.
fn stored_slots(field: &Field) -> Result<Field> {
    if !holds_unknown(field) {
        return Ok(field.clone());
    }
    let mut stored = field.clone();
    if field.as_iceberg().is_unknown() {
        stored.set_dtype(DataType::Null)?;
        stored.as_iceberg_mut().set_unknown(false)?;
        return Ok(stored);
    }
    let mut dtype = field.dtype().clone();
    for index in 0..field.dtype().field_len() {
        if let Some(child) = field.dtype().get_field_at(index)
            && holds_unknown(child)
        {
            dtype.set_field_at(index, stored_slots(child)?)?;
        }
    }
    stored.set_dtype(dtype)?;
    Ok(stored)
}

/// One column's data in the layout [`stored_slots`] declares, `target` its
/// Arrow datatype - `None` for a column the file leaves out.
///
/// Each `unknown` slot's array is handed to the next of `gathered` and stored
/// as the null array of its length; every other buffer is shared.
fn stored_data(
    field: &Field,
    target: Option<&arrow_schema::DataType>,
    data: arrow_data::ArrayData,
    gathered: &mut std::slice::IterMut<'_, Vec<ArrayRef>>,
) -> Result<arrow_data::ArrayData> {
    if field.as_iceberg().is_unknown() {
        let len = data.len();
        if let Some(arrays) = gathered.next() {
            arrays.push(arrow_array::make_array(data));
        }
        return Ok(arrow_data::ArrayData::new_null(
            &arrow_schema::DataType::Null,
            len,
        ));
    }
    if !holds_unknown(field) {
        return Ok(data);
    }
    let mut children = Vec::with_capacity(data.child_data().len());
    for (index, child) in data.child_data().iter().enumerate() {
        let child_target = target.and_then(|target| arrow_child_type(target, index));
        children.push(match field.dtype().get_field_at(index) {
            Some(child_field) => stored_data(child_field, child_target, child.clone(), gathered)?,
            None => child.clone(),
        });
    }
    let Some(target) = target else {
        return Ok(data);
    };
    data.into_builder()
        .data_type(target.clone())
        .child_data(children)
        .build()
        .map_err(Error::Arrow)
}

/// The Arrow datatype of child `index` of a nested Arrow datatype, in the
/// order its array data holds the children.
fn arrow_child_type(
    dtype: &arrow_schema::DataType,
    index: usize,
) -> Option<&arrow_schema::DataType> {
    use arrow_schema::DataType as Arrow;
    match dtype {
        Arrow::Struct(fields) => fields.get(index).map(|field| field.data_type()),
        Arrow::List(item)
        | Arrow::LargeList(item)
        | Arrow::ListView(item)
        | Arrow::LargeListView(item)
        | Arrow::FixedSizeList(item, _)
        | Arrow::Map(item, _) => (index == 0).then(|| item.data_type()),
        Arrow::Union(fields, _) => fields.iter().nth(index).map(|(_, field)| field.data_type()),
        Arrow::RunEndEncoded(run_ends, values) => match index {
            0 => Some(run_ends.data_type()),
            1 => Some(values.data_type()),
            _ => None,
        },
        _ => None,
    }
}

/// Refuse a value in one landed `unknown` slot.
///
/// `path` names the column the way its schema nests it, for the refusal.
fn require_no_value(column: &crate::Serie, path: &str) -> Result<()> {
    let Some(leaf) = column.as_variant() else {
        return Ok(());
    };
    for row in 0..column.len() {
        let (Some(metadata), Some(value)) = (leaf.metadata(row), leaf.value(row)) else {
            continue;
        };
        if crate::variant::is_null_value(value) {
            continue;
        }
        // The value is named as JSON spells it, bounded; one that does not
        // decode is still a value.
        let held = crate::Variant::new(metadata.to_vec(), value.to_vec())
            .and_then(|variant| variant.scalar())
            .and_then(|scalar| crate::json::into_utf8(&scalar))
            .map_or_else(
                |_| SmolStr::new_static("a value"),
                |text| format_smolstr!("{}", crate::text::elide_to(&text, 64)),
            );
        return Err(invalid(format_smolstr!(
            "expected only nulls in the unknown column {path:?} (an unknown column holds no \
             value; promote it to a type first), got {held}"
        )));
    }
    Ok(())
}

/// The columns every top-level identity partition field reads, in spec order.
fn partition_columns_of<'schema>(
    spec: &PartitionSpec,
    schema: &'schema Field,
) -> Vec<&'schema Field> {
    let mut columns: Vec<&Field> = Vec::new();
    for position in 0..spec.fields.len() {
        let Some(column) = identity_column(spec, position, schema) else {
            continue;
        };
        if !columns
            .iter()
            .any(|held| held.name().eq_ignore_ascii_case(column.name()))
        {
            columns.push(column);
        }
    }
    columns
}

/// The match key a merge joins on, and the part of it beyond the partition.
///
/// The identity partition columns lead, each named once, and `merge_by`
/// follows with any projection that repeats one of them dropped. The second
/// selector is `merge_by` without the partition columns: within a partition
/// group they are constant, so the join reads only these, and an empty one
/// says the partition alone is the key.
fn merge_keys(schema: &Field, spec: &PartitionSpec, merge_by: &Selector) -> (Selector, Selector) {
    let partitions: Vec<&Field> = partition_columns_of(spec, schema);
    let mut keys: Vec<Projection> = partitions
        .iter()
        .map(|column| Projection::column(column.name()))
        .collect();
    let mut row_keys: Vec<Projection> = Vec::new();
    for projection in merge_by.projections() {
        let repeated = projection.term().as_column().is_some_and(|name| {
            partitions
                .iter()
                .any(|column| column.name().eq_ignore_ascii_case(name))
        });
        if repeated {
            continue;
        }
        keys.push(projection.clone());
        row_keys.push(projection.clone());
    }
    (Selector::new(keys), Selector::new(row_keys))
}

/// The predicate that keeps exactly the partitions a set of tuples names.
///
/// Only an identity field is a column of the rows, so only those spell a
/// predicate the metadata chain can prune with: one column is an `in` list
/// (plus `is null` when a tuple holds one), several are a disjunction of the
/// tuples. A spec with no identity field answers always-true and the
/// file-level tuple comparison does the selecting.
fn partition_tuples_filter(spec: &PartitionSpec, schema: &Field, tuples: &[Vec<Scalar>]) -> Filter {
    let columns: Vec<(usize, &Field)> = (0..spec.fields.len())
        .filter_map(|position| {
            identity_column(spec, position, schema).map(|column| (position, column))
        })
        .collect();
    let value_of = |tuple: &Vec<Scalar>, position: usize| -> Scalar {
        tuple.get(position).cloned().unwrap_or(Scalar::Null)
    };
    match columns.as_slice() {
        [] => Filter::always_true(),
        [(position, column)] => {
            let mut values: Vec<Scalar> = Vec::new();
            let mut has_null = false;
            for tuple in tuples {
                match value_of(tuple, *position) {
                    Scalar::Null => has_null = true,
                    value => {
                        if !values.contains(&value) {
                            values.push(value);
                        }
                    }
                }
            }
            let mut parts = Vec::new();
            if !values.is_empty() {
                parts.push(Filter::new(
                    Term::column(column.name()).is_in(values.into_iter().map(Term::literal)),
                ));
            }
            if has_null {
                parts.push(Filter::new(Term::column(column.name()).is_null()));
            }
            Filter::any(parts)
        }
        columns => {
            Filter::any(tuples.iter().map(|tuple| {
                Filter::all(columns.iter().map(|(position, column)| {
                    match value_of(tuple, *position) {
                        Scalar::Null => Filter::new(Term::column(column.name()).is_null()),
                        value => Filter::new(Term::column(column.name()).eq(Term::literal(value))),
                    }
                }))
            }))
        }
    }
}

/// Narrow a stored root to the columns one plan reads, in stored order.
///
/// A name the schema does not declare is left for the selector to report;
/// a plan that reads none of the stored columns reads the whole root, so a
/// projection never asks a data file for zero columns.
fn projected_root(stored: &Field, columns: &[String]) -> Option<Field> {
    let kept: Vec<Field> = stored
        .fields()
        .iter()
        .filter(|field| {
            columns
                .iter()
                .any(|column| column.eq_ignore_ascii_case(field.name()))
        })
        .cloned()
        .collect();
    if kept.is_empty() || kept.len() == stored.field_len() {
        return None;
    }
    Field::from_parts(
        stored.name(),
        StructType::from_fields(kept).map(DataType::from).ok()?,
        stored.is_nullable(),
        stored.metadata_iter(),
    )
    .ok()
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
    // A float field says whether any value is NaN, and its bounds leave NaN
    // out, as the specification asks: a planner trusts them only when none is.
    for (summary, child) in summaries.iter_mut().zip(partition.fields()) {
        if child.dtype().id().is_floating() {
            summary.contains_nan = Some(false);
        }
    }
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
            if value.as_f64().is_some_and(f64::is_nan) {
                summary.contains_nan = Some(true);
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
    ///
    /// A key that is one stored column is measured against that column's
    /// statistics. A computed key has no statistics to prune on, so it keeps
    /// every file a candidate; it is still typed, so a key that cannot be
    /// computed over the table is refused before anything is read.
    fn of(batches: &[RecordBatch], schema: &Field, merge_by: &crate::Selector) -> Result<Self> {
        let mut columns = Vec::with_capacity(merge_by.len());
        for projection in merge_by.projections() {
            let Some(name) = projection.term().as_column() else {
                let computed = projection.field(schema)?;
                columns.push(KeyBound {
                    id: 0,
                    dtype: computed.dtype().clone(),
                    unbounded: true,
                    has_null: false,
                    lower: None,
                    upper: None,
                });
                continue;
            };
            let field = schema.dtype().get_field_by_name(name).ok_or_else(|| {
                let stored = schema
                    .fields()
                    .iter()
                    .map(|field| field.name())
                    .collect::<Vec<_>>()
                    .join(", ");
                invalid(crate::text::expected_got(
                    format_args!(
                        "a merge_by column the table schema declares, got {name:?}; it has"
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
    let options = arrow_schema::SortOptions {
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
    // The one row lands under the field, read once through its leaf.
    let scalar = crate::serie::land(
        std::sync::Arc::new(field.clone().with_nullable(true)),
        column.slice(row as usize, 1),
        &crate::serie::Proof::Unproven,
    )
    .and_then(|extreme| Ok(extreme.scalar(0)?))
    .map_err(|error| invalid(format_smolstr!("{error}")))?;
    Ok(single_value(&scalar, field.dtype()))
}

/// Land every incoming batch under the table schema and cut it into one
/// hold per partition tuple.
///
/// Every batch lands once, through one plan per layout the batches arrive
/// in; the pieces a partition takes out of it share its buffers - a
/// batch falling whole in one partition is the batch, a run of its rows a
/// slice, interleaved rows one take. A partitioned spec computes each
/// batch's tuples once ([`row_groups`]); an unpartitioned one holds every
/// batch under the one empty tuple. Each hold settles as it is pushed to,
/// and the holds together are settled against the process bound after each
/// batch, heaviest hold first, so a commit of any size keeps that bound in
/// memory.
fn grouped_holds(
    batches: BatchReader,
    schema: &Field,
    spec: &PartitionSpec,
    partition: &Field,
    safe: bool,
) -> Result<Vec<(Vec<Scalar>, ChunkedSerie)>> {
    let mut groups: Vec<(Vec<Scalar>, ChunkedSerie)> = Vec::new();
    // `Scalar`'s hash reads canonical content only, never the
    // interior-mutable caches a datatype holds, so the key is stable.
    #[allow(clippy::mutable_key_type)]
    let mut index: HashMap<Vec<Scalar>, usize> = HashMap::new();
    let transforms = spec.write_transforms(schema, partition)?;
    // The landing root is the schema without the order it declares: a
    // declaring root would refuse rows out of that order where they land,
    // and ordering them is the writer's work. One plan per layout the
    // batches arrive in, compiled once.
    let root = std::sync::Arc::new(schema.clone().with_metadata_removed("SORT:by"));
    let mut plans = PlanCache::new();
    for batch in batches {
        let batch = batch.map_err(Error::Arrow)?;
        let record = plans
            .get_or_compile(batch.schema_ref().fields(), || {
                ArrowCastPlan::compile_schema(
                    batch.schema_ref(),
                    &root,
                    ArrowCastOptions::new().with_safe(safe),
                    Deferred::default(),
                )
            })?
            .cast_batch(batch)?;
        if record.is_empty() {
            continue;
        }
        if spec.is_unpartitioned() {
            if groups.is_empty() {
                groups.push((
                    Vec::new(),
                    ChunkedSerie::from_landed(std::sync::Arc::clone(&root), Vec::new()),
                ));
            }
            groups[0].1.push_landed(record)?;
            settle_holds(&mut groups)?;
            continue;
        }

        let batch = record.into_arrow_batch()?;
        let found = row_groups(&batch, &transforms)?;
        let whole = found.len() == 1;
        for (values, rows) in found {
            let position = match index.get(&values) {
                Some(position) => *position,
                None => {
                    groups.push((
                        values.clone(),
                        ChunkedSerie::from_landed(std::sync::Arc::clone(&root), Vec::new()),
                    ));
                    index.insert(values, groups.len() - 1);
                    groups.len() - 1
                }
            };
            let piece = if whole {
                record.clone()
            } else {
                piece_of(&record, &rows)?
            };
            groups[position].1.push_landed(piece)?;
        }
        settle_holds(&mut groups)?;
    }
    Ok(groups)
}

/// The rows of `record` at `rows`, in that order: one zero-copy slice where
/// they are one run of the batch - every partition of a stream written in
/// the order its partitions advance - and one take where they interleave
/// with another partition's.
fn piece_of(record: &Serie, rows: &[u32]) -> Result<Serie> {
    match (rows.first(), rows.last()) {
        // Indices arrive in batch order, so a run is first..=last.
        (Some(&first), Some(&last)) if (last - first) as usize + 1 == rows.len() => {
            Ok(record.slice(first as usize, rows.len())?)
        }
        _ => {
            let indices = Serie::from_arrow_array(
                None,
                std::sync::Arc::new(UInt32Array::from(rows.to_vec())),
                ArrowCastOptions::new(),
            )?;
            Ok(record.into_taken(&indices)?)
        }
    }
}

/// Spill the heaviest holds whole until what every hold of one commit keeps
/// resident is under the process bound. Each hold alone settles as it is
/// pushed to; this is the bound across them.
fn settle_holds(groups: &mut [(Vec<Scalar>, ChunkedSerie)]) -> Result<()> {
    let options = SpillOptions::from_env()?;
    if options.is_never() {
        return Ok(());
    }
    let bound = options.byte_size();
    let resident = |groups: &[(Vec<Scalar>, ChunkedSerie)]| {
        groups
            .iter()
            .map(|(_, hold)| u64::try_from(hold.resident_size()).unwrap_or(u64::MAX))
            .fold(0_u64, u64::saturating_add)
    };
    if resident(groups) <= bound {
        return Ok(());
    }
    let mut order: Vec<(usize, usize)> = groups
        .iter()
        .enumerate()
        .map(|(index, (_, hold))| (index, hold.resident_size()))
        .collect();
    order.sort_by_key(|(_, bytes)| std::cmp::Reverse(*bytes));
    let whole = options.clone().with_byte_size(0);
    for (index, _) in order {
        if resident(groups) <= bound {
            break;
        }
        groups[index].1.spill(&whole)?;
    }
    Ok(())
}

/// Group row indices by their computed, typed partition tuple.
///
/// The tuple is computed once per distinct grouping key rather than once per
/// row: each transform's key column is resolved for the whole batch
/// ([`PartitionTransform::grouping_key`](super::partition::PartitionTransform::grouping_key)),
/// the keys are row-encoded together, and a row whose encoded key was seen
/// before joins that key's group without a scalar being built. Only the first
/// row of a new key reaches the transform, and two keys computing one tuple
/// share its group. Groups keep the order their first row arrived in, and
/// each group's rows stay in batch order.
fn row_groups(
    batch: &RecordBatch,
    transforms: &[super::partition::PartitionTransform],
) -> Result<Vec<(Vec<Scalar>, Vec<u32>)>> {
    let rows = u32::try_from(batch.num_rows()).map_err(|_| {
        invalid(format_smolstr!(
            "expected at most {} rows in one partitioning batch, got {}",
            u32::MAX,
            batch.num_rows()
        ))
    })?;
    let mut keys = Vec::with_capacity(transforms.len());
    for transform in transforms {
        if let Some(key) = transform.grouping_key(&source_column(batch, transform)?)? {
            keys.push(key);
        }
    }
    if keys.is_empty() {
        // Nothing varies by row, so every row computes the first row's tuple.
        return Ok(vec![(tuple_at(batch, transforms, 0)?, (0..rows).collect())]);
    }
    let converter = RowConverter::new(
        keys.iter()
            .map(|key| SortField::new(key.data_type().clone()))
            .collect(),
    )
    .map_err(Error::Arrow)?;
    let encoded = converter.convert_columns(&keys).map_err(Error::Arrow)?;

    let mut order: Vec<(Vec<Scalar>, Vec<u32>)> = Vec::new();
    // `Scalar`'s hash reads canonical content only, never the
    // interior-mutable caches a datatype holds, so the key is stable.
    #[allow(clippy::mutable_key_type)]
    let mut tuples: HashMap<Vec<Scalar>, usize> = HashMap::new();
    let mut groups: HashMap<Row<'_>, usize, BuildHasherDefault<RowKeyHasher>> = HashMap::default();
    // Rows of one partition usually arrive together, so the previous row's
    // key is checked before the map is.
    let mut previous: Option<(Row<'_>, usize)> = None;
    for row in 0..rows {
        let key = encoded.row(row as usize);
        let position = match previous {
            Some((last, position)) if last == key => position,
            _ => match groups.get(&key) {
                Some(position) => *position,
                None => {
                    let values = tuple_at(batch, transforms, row)?;
                    let position = match tuples.get(&values) {
                        Some(position) => *position,
                        None => {
                            tuples.insert(values.clone(), order.len());
                            order.push((values, Vec::new()));
                            order.len() - 1
                        }
                    };
                    groups.insert(key, position);
                    position
                }
            },
        };
        previous = Some((key, position));
        order[position].1.push(row);
    }
    Ok(order)
}

/// Hashes one row-encoded partition key in a single pass over its bytes.
///
/// The row format is already a canonical encoding of every key value, so one
/// XXH3 over those bytes is the whole hash; this is the per-row lookup of a
/// partitioned write, where SipHash's rounds are most of the cost. The keys
/// are the writer's own rows, so there is no one to flood the map but the
/// writer.
#[derive(Default)]
struct RowKeyHasher(u64);

impl Hasher for RowKeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0 = self.0.rotate_left(23) ^ twox_hash::XxHash3_64::oneshot(bytes);
    }

    fn write_usize(&mut self, value: usize) {
        self.0 = self.0.rotate_left(23) ^ value as u64;
    }
}

/// Resolve one transform's source column for a whole batch.
///
/// A column below a struct is null wherever an enclosing struct is, as
/// [`source_value`] reads it, so the parents' nulls are folded into what the
/// grouping keys see.
fn source_column(
    batch: &RecordBatch,
    transform: &super::partition::PartitionTransform,
) -> Result<ArrayRef> {
    let (first, nested) = transform.path().split_first().ok_or_else(|| {
        invalid(SmolStr::new_static(
            "expected a non-empty partition source path",
        ))
    })?;
    let mut column = std::sync::Arc::clone(batch.column_by_name(first).ok_or_else(|| {
        invalid(format_smolstr!(
            "expected a partition source column {first:?} in the batch, got none"
        ))
    })?);
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
        let child = parent.column_by_name(name).ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a nested partition source column {name:?}, got none"
            ))
        })?;
        column = match parent.nulls() {
            Some(nulls) => {
                let hidden = arrow_array::BooleanArray::new(!nulls.inner(), None);
                arrow_select::nullif::nullif(child.as_ref(), &hidden).map_err(Error::Arrow)?
            }
            None => std::sync::Arc::clone(child),
        };
    }
    Ok(column)
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
    // The one row lands under the source field, read once through its leaf.
    crate::serie::land(
        std::sync::Arc::new(transform.source().clone().with_nullable(true)),
        column.slice(row, 1),
        &crate::serie::Proof::Unproven,
    )
    .and_then(|source| Ok(source.scalar(0)?))
    .map_err(|error| invalid(format_smolstr!("{error}")))
}

/// The bytes of one metadata document as its compression codec writes them,
/// and the suffix its name carries before `.metadata.json` for it.
fn encoded_document(metadata: &TableMetadata) -> Result<(&'static str, Vec<u8>)> {
    let compression = metadata.metadata_compression_codec()?;
    let document = metadata.clone().into_json()?;
    let encoded = crate::json::into_bytes(&document)?;
    match compression {
        iceberg_official::compression::CompressionCodec::None => Ok(("", encoded)),
        iceberg_official::compression::CompressionCodec::Gzip(_) => {
            Ok((".gz", crate::gzip::dump(&encoded)?))
        }
        other => Err(invalid(format_smolstr!(
            "unsupported Iceberg metadata compression codec {other}; expected none or gzip"
        ))),
    }
}

/// Return the current metadata document with its exact name and number.
///
/// [`find_metadata`] with the document always read.
fn find_metadata_document(metadata_dir: &Holder) -> Result<Option<(u32, SmolStr, Scalar)>> {
    match find_metadata(metadata_dir, None)? {
        Some((version, name, Some(document))) => Ok(Some((version, name, document))),
        Some((version, name, None)) => Err(invalid(format_smolstr!(
            "expected the metadata document {name} of version {version} to be read, got none"
        ))),
        None => Ok(None),
    }
}

/// Return the metadata document with the highest version, exact name, and number.
///
/// A folder that holds none is `None` rather than an error, because that is the
/// question "is this a table" and the answer "no" is not a failure.
///
/// `known` is the version the caller already holds: when the hint names
/// exactly it, the document is not read again and the answer carries `None`
/// in its place - the version is what the caller asked, and a document it
/// already has is not worth a round trip. Every other answer carries the
/// document.
fn find_metadata(
    metadata_dir: &Holder,
    known: Option<u32>,
) -> Result<Option<(u32, SmolStr, Option<Scalar>)>> {
    // A folder that is not a table has no metadata directory at all, and the
    // laziness contract makes that a handle that simply is not a container.
    if !metadata_dir.is_container() {
        return Ok(None);
    }

    let hint = leaf(metadata_dir.child_by_path(VERSION_HINT)?)?;
    let hinted_version = String::from_utf8_lossy(&hint.read_all_bytes()?)
        .trim()
        .parse::<u32>()
        .ok();

    // Prefer the conventional Hadoop filename named by a usable hint.
    if let Some(version) = hinted_version {
        if known == Some(version) {
            return Ok(Some((
                version,
                format_smolstr!("v{version}.metadata.json"),
                None,
            )));
        }
        for name in [
            format!("v{version}.metadata.json"),
            format!("v{version}.gz.metadata.json"),
        ] {
            let document = leaf(metadata_dir.child_by_path(&name)?)?;
            let bytes = document.read_all_bytes()?;
            if !bytes.is_empty() {
                return Ok(Some((
                    version,
                    SmolStr::new(name),
                    Some(parse_metadata_bytes(&bytes)?),
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
    Ok(Some((version, name, Some(parse_metadata_bytes(&bytes)?))))
}

/// Whether any schema the table ever had spells a column of the current
/// schema under another name.
///
/// A data file stores its columns under the names of the schema it was
/// written with, so a table whose schemas all agree on every name holds no
/// file a scan has to open the footer of just to learn what it calls a
/// column; only a table that renamed one pays that read per file.
fn columns_renamed(metadata: &TableMetadata) -> bool {
    let Ok(current) = metadata.current_schema() else {
        return true;
    };
    metadata.schemas().iter().any(|schema| {
        schema.fields().iter().any(|field| {
            let Ok(Some(id)) = field.parquet_field_id() else {
                return false;
            };
            current.fields().iter().any(|now| {
                matches!(now.parquet_field_id(), Ok(Some(now_id)) if now_id == id)
                    && now.name() != field.name()
            })
        })
    })
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

    // `<version>-<uuid>`, exactly what the official `MetadataLocation`
    // accepts - a non-negative `i32` and a UUID - checked here because its
    // parser builds every error it might return before it knows whether it
    // will, so a listing of good names captured a backtrace per file.
    let (version, id) = stem.split_once('-')?;
    uuid::Uuid::parse_str(id).ok()?;
    if version.is_empty() || !version.bytes().all(|byte| byte.is_ascii_digit()) {
        None
    } else {
        version
            .parse::<i32>()
            .ok()
            .and_then(|version| u32::try_from(version).ok())
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
        location_text(metadata_dir)
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
fn pairs_predicate(schema: &Field, pairs: &[(&str, &str)]) -> crate::Filter {
    crate::Filter::all(pairs.iter().map(|(column, value)| {
        schema.dtype().get_field_by_name(column).map_or_else(
            || crate::Filter::new(crate::Term::column(*column).eq(crate::Term::literal(*value))),
            |field| crate::Filter::partition_equals(column, value, field.dtype()),
        )
    }))
}

#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/iceberg/table.rs` pins and a caller cannot reach.
    //!
    //! A commit is one call to a caller, so the decisions it makes on the way
    //! - the retry ladder's arithmetic, the merge pruning a malformed bound
    //! must not win, the partition summary a NaN must not enter, the metadata
    //! names a directory listing accepts, the relative location two writers
    //! spell differently - are each forwarded here to be pinned on their own.
    //! [`CommitSettings`], [`KeyBound`] and [`KeyBounds`] are forwarding
    //! wrappers holding the real value, so nothing in `iceberg::table` or
    //! `iceberg::options` changes visibility.

    use arrow_array::RecordBatch;

    use crate::iceberg::{DataFile, FieldSummary, ManifestEntry, PartitionSpec};
    use crate::{DataType, Field, Result, Selector};

    /// One commit's resolved retry ladder, forwarding to the real one.
    ///
    /// A caller states retries and waits as table properties; what the ladder
    /// runs with is what those resolve to, which is only nameable here.
    pub struct CommitSettings(crate::iceberg::options::CommitSettings);

    impl CommitSettings {
        /// The resolved ladder, spelled out attempt budget first.
        pub fn new(
            retries: u32,
            min_backoff_ms: u64,
            max_backoff_ms: u64,
            total_timeout_ms: u64,
        ) -> Self {
            Self(crate::iceberg::options::CommitSettings {
                retries,
                min_backoff_ms,
                max_backoff_ms,
                total_timeout_ms,
            })
        }
    }

    /// One match-key column's incoming range, forwarding to the real one.
    pub struct KeyBound(super::KeyBound);

    impl KeyBound {
        /// One column's incoming range, as a merge holds it before pruning.
        pub fn new(
            id: i32,
            dtype: DataType,
            unbounded: bool,
            has_null: bool,
            lower: Option<Vec<u8>>,
            upper: Option<Vec<u8>>,
        ) -> Self {
            Self(super::KeyBound {
                id,
                dtype,
                unbounded,
                has_null,
                lower,
                upper,
            })
        }

        /// Return whether one file's statistics leave room for these keys.
        pub fn may_hold(&self, file: &DataFile) -> bool {
            self.0.may_hold(file)
        }
    }

    /// Every match-key column's incoming range, forwarding to the real ones.
    pub struct KeyBounds(super::KeyBounds);

    impl KeyBounds {
        /// Measure the incoming rows' range for every match-key column.
        pub fn of(batches: &[RecordBatch], schema: &Field, merge_by: &Selector) -> Result<Self> {
            super::KeyBounds::of(batches, schema, merge_by).map(Self)
        }

        /// Whether the bound at one position can exclude nothing.
        pub fn column_is_unbounded(&self, index: usize) -> bool {
            self.0.columns[index].unbounded
        }
    }

    /// The wait before one retry attempt, exponential with full jitter.
    pub fn backoff_ms(attempt: u32, min: u64, max: u64) -> u64 {
        super::backoff_ms(attempt, min, max)
    }

    /// Reserve one randomized wait from the total retry-delay budget.
    pub fn reserve_retry_backoff(spent_ms: &mut u64, wait_ms: u64, limit_ms: u64) -> bool {
        super::reserve_retry_backoff(spent_ms, wait_ms, limit_ms)
    }

    /// Count one conflict and answer the wait it earns, or give up.
    pub fn retry_wait_ms(
        settings: &CommitSettings,
        beaten: &mut u32,
        backoff_spent_ms: &mut u64,
        expected_version: u32,
        last_seen_version: u32,
    ) -> Result<u64> {
        super::retry_wait_ms(
            &settings.0,
            beaten,
            backoff_spent_ms,
            expected_version,
            last_seen_version,
        )
    }

    /// Summarize the partition values a manifest's entries hold, in spec order.
    pub fn summaries(
        spec: &PartitionSpec,
        schema: &Field,
        entries: &[ManifestEntry],
    ) -> Result<Vec<FieldSummary>> {
        super::summaries(spec, schema, entries)
    }

    /// Parse the version prefix of a Hadoop or official metadata file name.
    pub fn metadata_version_from_name(name: &str) -> Option<u32> {
        super::metadata_version_from_name(name)
    }

    /// Turn one absolute location into a name relative to the table's folder.
    pub fn relative_location(base: &str, location: &str) -> Result<String> {
        super::relative_location(base, location)
    }

    /// Resolve one recorded location into a child of the table's folder.
    ///
    /// A caller reads rows, not files; a suite that has to open the file a
    /// manifest named resolves it the way the table itself does.
    pub fn child_at<H: crate::IOBase>(
        table: &crate::iceberg::IcebergTable<H>,
        location: &str,
    ) -> Result<crate::holder::Holder> {
        table.child_at(location)
    }
}
