//! Iceberg catalogs over folders: a warehouse folder read as a catalog of
//! namespaces of tables, laid out the way `HadoopCatalog` lays one out.
//!
//! A catalog here is storage and nothing else: a warehouse is one container
//! handle, a namespace is a folder under it, a table is a folder laid out as
//! an Iceberg table, and `lake.nyc.taxis` is the folder `nyc/taxis` under the
//! warehouse registered as `lake`. Every value is a description of where
//! things live - constructing one touches nothing - and every verb runs
//! against the handle at the moment it is asked, so two catalogs over one
//! folder see the same tables.
//!
//! [`IcebergCatalog`] and [`IcebergNamespace`] answer the warehouse traits
//! ([`CatalogValue`], [`NamespaceValue`], [`ObjectValue`]); the collection
//! views, the dotted descent and registration are the generic ones -
//! [`Namespaces`](crate::Namespaces), [`Tables`](crate::Tables),
//! [`Catalog::resolve`](crate::Catalog::resolve), [`Warehouse`](crate::Warehouse).
//! Namespaces nest to any depth, so a catalog states no
//! [`namespace_levels`](CatalogValue::namespace_levels).
//!
//! A listing classifies each folder entry with one listing of its `metadata/`
//! and no read: a folder holding a `version-hint.text` or a `*.metadata.json`
//! is a table, every other folder a namespace, and the reserved `metadata`
//! name - where each level keeps its own document - is skipped. A table
//! answered is an [`IcebergTable`] described at its folder, its current
//! document read on the first verb that needs it.
//!
//! ```no_run
//! use yggdryl::iceberg::IcebergCatalog;
//! use yggdryl::local::LocalFolder;
//! use yggdryl::{DataType, NamespaceValue, StructType, Warehouse};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let folder = LocalFolder::new(LocalFolder::temporary()?.path()?.join("warehouse"))?;
//! let mut warehouse = Warehouse::new();
//! warehouse.register(IcebergCatalog::bound("lake", folder.into()))?;
//!
//! let schema = DataType::from(StructType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::utf8().nullable_field("venue").with_partition(true),
//! ])?)
//! .required_field("row");
//!
//! // A create descends through existing namespaces only, so the namespace is
//! // made before what goes under it; nothing is probed first either way.
//! let nyc = warehouse.catalog("lake")?.namespaces().open_or_create("nyc", &Default::default())?;
//! let taxis = nyc.tables().create("taxis", &schema, &Default::default())?;
//! assert_eq!(taxis.to_string(), "lake.nyc.taxis");
//! assert_eq!(warehouse.table("lake.nyc.taxis")?.path(), taxis.path());
//! # Ok(())
//! # }
//! ```
//!
//! # What is not here
//!
//! - No `drop_table` and no namespace removal: a catalog's folders hold data
//!   files, and a recursive delete is the operation that turns one wrong URL
//!   into a lost warehouse. [`IOBase::remove`] removes a *leaf* or an empty
//!   container; emptying a table's history is [`IcebergTable`] maintenance
//!   work.
//! - No `rename_table`: the storage contract has no move primitive, and a
//!   rename that re-copied every data file would be a copy wearing a rename's
//!   name.
//! - No catalog service here: a REST catalog is a network client, and it is
//!   its own implementation.

mod namespace;

pub use namespace::IcebergNamespace;

use smol_str::{SmolStr, format_smolstr};

use super::IcebergTable;
use super::metadata::FormatVersion;
use super::partition::PartitionSpec;
use crate::holder::Holder;
use crate::warehouse::{Handle, Site, entry_name, extended, path_text, table_layout};
use crate::{
    CatalogValue, Error, Field, IOBase, IOKind, Namespace, NamespaceValue, Object, ObjectValue,
    Objects, Properties, Result, Table, Url,
};

/// The reserved folder every level keeps its own document in.
///
/// A table already holds `metadata/` for its versioned documents; a namespace
/// holds `metadata/namespace.json` and a warehouse `metadata/catalog.json`.
/// The name is therefore reserved at every level: a namespace or table named
/// `metadata` would collide with the level's own folder, so it is refused as
/// a name and skipped as an entry.
const METADATA_DIR: &str = "metadata";

/// The document that makes a namespace durable and carries its properties.
const NAMESPACE_DOCUMENT: &str = "metadata/namespace.json";

/// The document that makes a warehouse a catalog and carries its properties.
const CATALOG_DOCUMENT: &str = "metadata/catalog.json";

/// The property prefix the format reserves for itself.
///
/// Protocol metadata is inert `<scheme>:<property>` strings, and `ICEBERG:` is
/// the format's own scheme - a caller writing under it could silently change
/// what the format later reads, so the update path refuses it by name.
const RESERVED_PREFIX: &str = "ICEBERG:";

/// A warehouse folder read as a catalog of namespaces of Iceberg tables.
///
/// The catalog is a description of where tables live, not proof that any do:
/// constructing one touches nothing, its folder opens on the first verb that
/// needs it under its stated properties, and every verb resolves against the
/// folder at the moment it runs. Its stored properties are the
/// `metadata/catalog.json` document, read as [`ObjectValue::properties`]
/// beneath what was stated and written by
/// [`ObjectValue::update_properties`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IcebergCatalog {
    path: Vec<SmolStr>,
    handle: Handle,
    description: Option<String>,
    stated: Properties,
}

impl IcebergCatalog {
    /// The catalog `name` over the warehouse folder `url` names, touching
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns the identifier's own refusal when it names no location.
    pub fn new(name: impl Into<SmolStr>, url: impl Into<crate::Uri>) -> Result<Self> {
        let url = url.into().locator()?;
        let path = vec![name.into()];
        Ok(Self {
            handle: Handle::at(Site::Url(url), false, &path, Properties::new()),
            path,
            description: None,
            stated: Properties::new(),
        })
    }

    /// The catalog `name` over a warehouse folder already in hand.
    pub fn bound(name: impl Into<SmolStr>, warehouse: Holder) -> Self {
        let path = vec![name.into()];
        Self {
            handle: Handle::bound(warehouse, false, &path, Properties::new()),
            path,
            description: None,
            stated: Properties::new(),
        }
    }

    /// Create the catalog `name` in `warehouse`, writing its
    /// `metadata/catalog.json`.
    ///
    /// The write is what creates the folder and its ancestry - nothing walks
    /// or prepares anything first.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the folder is already there - a
    /// catalog, a table or a file - and the write failure otherwise.
    pub fn create(name: impl Into<SmolStr>, warehouse: Holder) -> Result<Self> {
        let name = name.into();
        match classify(warehouse)? {
            Occupant::Nothing(folder) => {
                write_document(&folder, CATALOG_DOCUMENT, &Properties::new())?;
                Ok(Self::bound(name, folder))
            }
            Occupant::Namespace(_) => Err(Error::conflict("catalog", "catalog", name)),
            Occupant::Table(_) => Err(Error::conflict("catalog", "table", name)),
            Occupant::File => Err(Error::conflict("catalog", "file", name)),
        }
    }

    /// The catalog `name` over `warehouse`, created when the folder is not
    /// there yet: [`Self::create`] with the conflict absorbed.
    ///
    /// # Errors
    ///
    /// Returns a refusal when a table or a file occupies the folder.
    pub fn open_or_create(name: impl Into<SmolStr>, warehouse: Holder) -> Result<Self> {
        let name = name.into();
        match classify(warehouse)? {
            Occupant::Namespace(folder) => Ok(Self::bound(name, folder)),
            Occupant::Nothing(folder) => {
                write_document(&folder, CATALOG_DOCUMENT, &Properties::new())?;
                Ok(Self::bound(name, folder))
            }
            Occupant::Table(_) => Err(invalid(format_smolstr!(
                "expected a catalog folder at {name:?}, got a table"
            ))),
            Occupant::File => Err(invalid(format_smolstr!(
                "expected a catalog folder at {name:?}, got a file"
            ))),
        }
    }

    /// Return this catalog with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this catalog with stated properties, which its folder and
    /// every object under it open with.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self.handle.set_properties(self.stated.clone());
        self
    }

    /// The warehouse folder, opened on the first call.
    fn handle(&self) -> Result<&Holder> {
        self.handle.get()
    }

    /// The effective properties: what the document keeps, then what was
    /// stated.
    fn effective(&self) -> Result<Properties> {
        let stored = read_document(self.handle()?, CATALOG_DOCUMENT)?;
        Ok(self.stated.inherit(&stored))
    }
}

impl ObjectValue for IcebergCatalog {
    fn name(&self) -> &str {
        &self.path[0]
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Catalog
    }

    fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    fn url(&self) -> Option<&Url> {
        self.handle.url()
    }

    fn modified(&self) -> Option<i64> {
        self.handle().ok()?.mtime()
    }

    fn properties(&self) -> Result<Properties> {
        self.effective()
    }

    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        update_document(self.handle()?, CATALOG_DOCUMENT, updates, removes)
    }
}

impl NamespaceValue for IcebergCatalog {
    fn children(&self) -> Objects {
        match self
            .handle()
            .and_then(|handle| Ok((handle, self.effective()?)))
        {
            Ok((handle, effective)) => level(handle, &self.path, effective),
            Err(error) => Objects::failing(error),
        }
    }

    fn get(&self, name: &str) -> Result<Object> {
        child(self.handle()?, &self.path, &self.effective()?, name)
    }

    fn create_namespace(&self, name: &str, properties: &Properties) -> Result<Namespace> {
        create_namespace(
            self.handle()?,
            &self.path,
            &self.effective()?,
            name,
            properties,
        )
    }

    fn create_table(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        create_table(
            self.handle()?,
            &self.path,
            &self.effective()?,
            name,
            field,
            properties,
        )
    }
}

impl CatalogValue for IcebergCatalog {
    fn namespace_levels(&self) -> Option<usize> {
        None
    }
}

/// What occupies a resolved folder, classified in one pass and without a
/// read: presence costs one call, and only a present folder pays for the
/// listing of its `metadata/` that tells a table from a namespace.
enum Occupant {
    /// A folder laid out as an Iceberg table, handed back.
    Table(Holder),
    /// A folder that exists and is not a table, handed back.
    Namespace(Holder),
    /// Nothing at all, handed back so a create needs no second resolution.
    Nothing(Holder),
    /// An actual file occupies the location, which can be neither.
    File,
}

/// Classify what occupies `folder`, cheapest evidence first.
fn classify(folder: Holder) -> Result<Occupant> {
    if !folder_present(&folder)? {
        return Ok(Occupant::Nothing(folder));
    }
    match table_layout(&folder) {
        Ok(true) => Ok(Occupant::Table(folder)),
        Ok(false) => Ok(Occupant::Namespace(folder)),
        Err(error) if is_not_a_directory(&error) => Ok(Occupant::File),
        Err(error) => Err(error),
    }
}

/// Return whether the folder is there at all - an empty one included.
///
/// This is the existence question the classification is *answering*, not a
/// probe on the way to another act: `get` must raise absence, `create` must
/// raise the conflict, and both branch on this one shared answer. The folder
/// roles answer it in one backend call; anything else settles for its first
/// listing entry, which cannot see an empty-but-present folder and says so
/// here rather than pretending.
fn folder_present(folder: &Holder) -> Result<bool> {
    match folder {
        Holder::LocalFolder(folder) => Ok(folder.exists()),
        Holder::FsFolder(folder) => {
            Ok(folder.filesystem().file_info(folder.path())?.kind != IOKind::Unknown)
        }
        other => match other.ls(false, true).next() {
            None => Ok(false),
            Some(Ok(_)) => Ok(true),
            Some(Err(error)) if is_not_a_directory(&error) => Ok(false),
            Some(Err(error)) => Err(error),
        },
    }
}

/// Return whether a failure says a file sat where a folder was addressed.
fn is_not_a_directory(error: &Error) -> bool {
    matches!(error, Error::Io(error) if error.kind() == std::io::ErrorKind::NotADirectory)
}

/// Re-describe a resolved child in the folder role, touching nothing.
///
/// A backend's `child_by_path` answers with what it believes is there, and
/// for a location nothing occupies yet that is a leaf-to-be. A catalog knows
/// better - its names address containers - so the leaf spellings are re-cast
/// as the same backend's folder, keeping every bound-location fact.
fn folder_role(child: Holder) -> Result<Holder> {
    match child {
        Holder::LocalFolder(_) | Holder::FsFolder(_) => Ok(child),
        Holder::LocalPath(path) => Ok(Holder::LocalFolder(crate::local::LocalFolder::from_url(
            path.url().clone(),
        )?)),
        Holder::LocalFile(file) => match file.url() {
            Some(url) => Ok(Holder::LocalFolder(crate::local::LocalFolder::from_url(
                url.clone(),
            )?)),
            None => Err(invalid(SmolStr::new_static(
                "expected a located folder for a catalog name, got a handle with no URL",
            ))),
        },
        Holder::FsPath(path) => Ok(Holder::FsFolder(crate::fs::FsFolder::new(
            path.bound().clone(),
        ))),
        Holder::FsFile(file) => Ok(Holder::FsFolder(crate::fs::FsFolder::new(
            file.bound().clone(),
        ))),
        other => {
            let described = other
                .url()
                .map_or_else(|| "<memory>".to_owned(), ToString::to_string);
            Err(invalid(format_smolstr!(
                "expected a backend that can hold a folder for a catalog name, got {described}"
            )))
        }
    }
}

/// Check one part of a path as a folder name under a catalog.
///
/// A part must be usable as one folder name, so an empty part, a path
/// separator, a `column=value` spelling and the reserved `metadata` name are
/// refused by name rather than resolved into a layout they would collide
/// with.
fn segment(parent: &[SmolStr], name: &str) -> Result<()> {
    let refusal =
        |reason: SmolStr| invalid(format_smolstr!("{reason} under {}", path_text(parent)));
    if name.is_empty() {
        return Err(refusal(SmolStr::new_static(
            "expected a non-empty name, got an empty one",
        )));
    }
    if name.contains('/') {
        return Err(refusal(format_smolstr!(
            "expected a name without '/', got {name:?}; a namespace nests as a path's parts, \
             not with separators"
        )));
    }
    if name.contains('=') {
        return Err(refusal(format_smolstr!(
            "expected a name without '=', got {name:?}; a column=value folder is a partition \
             directory, not a name"
        )));
    }
    if name == METADATA_DIR {
        return Err(refusal(format_smolstr!(
            "expected a name other than {METADATA_DIR:?}; that folder is where each level keeps \
             its own metadata document"
        )));
    }
    Ok(())
}

/// The folder `name` resolves to under `folder`, touching nothing.
fn resolve(folder: &Holder, parent: &[SmolStr], name: &str) -> Result<Holder> {
    segment(parent, name)?;
    folder_role(folder.child_by_path(name)?)
}

/// The object a present folder is, at `path`, carrying `effective`.
fn object_of(folder: Holder, table: bool, path: Vec<SmolStr>, effective: &Properties) -> Object {
    if table {
        let root = Handle::bound(folder, false, &path, Properties::new());
        Object::Table(Table::Iceberg(Box::new(
            IcebergTable::at(path, root).inheriting(effective),
        )))
    } else {
        Object::Namespace(Namespace::Iceberg(Box::new(IcebergNamespace::listed(
            path,
            folder,
            effective.clone(),
        ))))
    }
}

/// The children of `folder`, lazily: one listing, and one listing of
/// `metadata/` per folder entry as its turn comes.
fn level(folder: &Holder, path: &[SmolStr], effective: Properties) -> Objects {
    let path = path.to_vec();
    Objects::new(folder.ls(false, false).filter_map(move |entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error)),
        };
        if !entry.is_container() {
            return None;
        }
        let name = entry_name(&entry)?;
        if name == METADATA_DIR || name.starts_with('.') {
            return None;
        }
        let table = match table_layout(&entry) {
            Ok(table) => table,
            Err(error) => return Some(Err(error)),
        };
        Some(Ok(object_of(
            entry,
            table,
            extended(&path, &name),
            &effective,
        )))
    }))
}

/// The child of `folder` called `name`: one resolution and one
/// classification.
fn child(folder: &Holder, path: &[SmolStr], effective: &Properties, name: &str) -> Result<Object> {
    let below = extended(path, name);
    match classify(resolve(folder, path, name)?)? {
        Occupant::Table(child) => Ok(object_of(child, true, below, effective)),
        Occupant::Namespace(child) => Ok(object_of(child, false, below, effective)),
        Occupant::Nothing(_) => Err(Error::absent("table", path_text(&below))),
        Occupant::File => Err(invalid(format_smolstr!(
            "expected a namespace or table folder at {}, got a file",
            path_text(&below)
        ))),
    }
}

/// Create the namespace `name` under `folder`, writing its
/// `metadata/namespace.json` with `properties`.
fn create_namespace(
    folder: &Holder,
    path: &[SmolStr],
    effective: &Properties,
    name: &str,
    properties: &Properties,
) -> Result<Namespace> {
    let below = extended(path, name);
    match classify(resolve(folder, path, name)?)? {
        Occupant::Nothing(child) => {
            write_document(&child, NAMESPACE_DOCUMENT, properties)?;
            Ok(Namespace::Iceberg(Box::new(
                IcebergNamespace::listed(below, child, effective.clone())
                    .with_properties(properties.clone()),
            )))
        }
        Occupant::Namespace(_) => Err(Error::conflict("namespace", "namespace", path_text(&below))),
        Occupant::Table(_) => Err(Error::conflict("namespace", "table", path_text(&below))),
        Occupant::File => Err(Error::conflict("namespace", "file", path_text(&below))),
    }
}

/// Create the table `name` under `folder`, writing its first metadata
/// document: the schema numbered above the highest identifier it carries,
/// the partition spec derived from the columns it marks, format version 2.
///
/// Writing the first metadata document is what creates every missing
/// ancestor folder - nothing checks for them and nothing makes them in
/// advance. Storage has no compare-and-swap, so two creators of one table
/// converge on one document or one of them gets the typed conflict; neither
/// replaces the other's table.
fn create_table(
    folder: &Holder,
    path: &[SmolStr],
    effective: &Properties,
    name: &str,
    field: &Field,
    properties: &Properties,
) -> Result<Table> {
    let below = extended(path, name);
    match classify(resolve(folder, path, name)?)? {
        Occupant::Nothing(child) => {
            // The schema as Iceberg expresses it: a layout the format does
            // not state - a dictionary, a view string - is rewritten to the
            // one it does, and a column no type of its holds is refused by
            // path. The rows an append brings are cast to it once.
            let mut schema = field.clone().into_scheme_compat(&crate::Scheme::ICEBERG)?;
            let start = super::last_column_id(&schema)?.saturating_add(1);
            super::assign_field_ids(&mut schema, start)?;
            let spec = PartitionSpec::from_schema(0, &schema)?;
            let table = IcebergTable::create(child, FormatVersion::V2, schema, spec)?
                .map_root(|root| Handle::bound(root, false, &below, Properties::new()))
                .placed(below)
                .with_properties(properties.clone())
                .inheriting(effective);
            Ok(Table::Iceberg(Box::new(table)))
        }
        Occupant::Namespace(_) => Err(Error::conflict("table", "namespace", path_text(&below))),
        Occupant::Table(_) => Err(Error::conflict("table", "table", path_text(&below))),
        Occupant::File => Err(Error::conflict("table", "file", path_text(&below))),
    }
}

/// Read the properties document under `folder`, absent meaning empty.
///
/// The read is the act: a document that is not there reads as zero bytes and
/// answers empty properties - never a missing-file failure a caller has to
/// catch. A document that is there but is not the expected shape is an error
/// naming what was found.
fn read_document(folder: &Holder, document: &str) -> Result<Properties> {
    let handle = folder.child_by_path(document)?;
    let bytes = handle.read_all_bytes()?;
    if bytes.is_empty() {
        return Ok(Properties::new());
    }
    let described = handle
        .url()
        .map_or_else(|| "<memory>".to_owned(), ToString::to_string);
    let value = crate::json::from_bytes(&bytes)?;
    let Some(entries) = value.get_key_str("properties") else {
        return Err(invalid(format_smolstr!(
            "expected a {{\"properties\": ...}} document at {described}, got one without the key"
        )));
    };
    let mut properties = Properties::new();
    if let Some(record) = entries.as_struct() {
        for (key, value) in record {
            let Some(value) = value.as_str() else {
                return Err(invalid(format_smolstr!(
                    "expected string property pairs at {described}, got {key:?}: {value:?}"
                )));
            };
            properties.set(key.clone(), value);
        }
    } else if let Some(mapping) = entries.as_mapping() {
        for (key, value) in mapping {
            let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
                return Err(invalid(format_smolstr!(
                    "expected string property pairs at {described}, got {key:?}: {value:?}"
                )));
            };
            properties.set(key, value);
        }
    } else {
        return Err(invalid(format_smolstr!(
            "expected \"properties\" to hold a mapping at {described} \
             (a record or mapping value), got {entries:?}"
        )));
    }
    Ok(properties)
}

/// Write `properties` as the document under `folder`, which is also what
/// creates the folder and its ancestry.
fn write_document(folder: &Holder, document: &str, properties: &Properties) -> Result<()> {
    let entries = crate::Scalar::from_mapping(
        properties
            .iter()
            .map(|(key, value)| (crate::Scalar::from(key), crate::Scalar::from(value))),
    )?;
    let body = crate::Scalar::from_mapping([(crate::Scalar::from("properties"), entries)])?;
    let bytes = crate::json::into_bytes(&body)?;
    folder.child_by_path(document)?.write_all_bytes(&bytes)?;
    Ok(())
}

/// Apply property updates and removals to the document under `folder`,
/// transactionally.
///
/// The whole new document is built and validated before a byte is written,
/// so a failure leaves the stored value unchanged; the write itself is a
/// whole-value replacement, which every backend publishes atomically or not
/// at all. Writing the document is also what creates the folder and its
/// ancestry, which is exactly how an empty namespace becomes durable.
fn update_document(
    folder: &Holder,
    document: &str,
    updates: &Properties,
    removes: &[SmolStr],
) -> Result<()> {
    let mut current = read_document(folder, document)?;
    for (key, value) in updates.iter() {
        if key.starts_with(RESERVED_PREFIX) {
            return Err(invalid(format_smolstr!(
                "expected a property key outside the reserved {RESERVED_PREFIX:?} prefix, \
                 got {key:?}"
            )));
        }
        current.set(key, value);
    }
    for key in removes {
        current.remove(key);
    }
    write_document(folder, document, &current)
}

/// Report a name or a folder this catalog cannot accept.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}
