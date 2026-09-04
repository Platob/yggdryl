//! A hierarchy of catalogs, namespaces, and tables, one shape at every level.
//!
//! A catalog here is storage and nothing else, the way `HadoopCatalog` is: a
//! warehouse is one container handle, a namespace is a folder under it, a
//! table is a folder [`Table::locate`] recognizes, and a dotted name like
//! `"nyc.taxis"` is the folder `nyc/taxis` spelled the way a catalog spells
//! it. Every lookup runs through [`IOBase::child_by_path`] and [`IOBase::ls`]
//! against that one handle - no path is opened, no network is reached - and
//! every value in this module is only a description of where things live, so
//! constructing one touches nothing at all.
//!
//! # Three levels, one shape
//!
//! A *collection* is a lazy map-oriented view whose construction touches
//! nothing, and it has exactly this vocabulary at every level - no level
//! invents a verb:
//!
//! ```text
//! collection.get(name)                 // open, absence raised
//! collection.create(name, ...)        // create, conflict raised
//! collection.open_or_create(name, ...) // both absorbed - same attempt, one path
//! collection.contains(name)           // the answer a caller asked for
//! collection.iter()                   // the names, one at a time
//! collection.len() / .is_empty()      // drain the iterator; they cost the listing
//! ```
//!
//! A *resource* is one addressed thing, and it has exactly this one:
//!
//! ```text
//! resource.name()          // its dotted identity
//! resource.kind()          // the role it plays
//! resource.properties()    // its metadata document, absent means empty
//! resource.update_properties(updates, removes)
//! ```
//!
//! So the cascade reads the same at every depth, and a namespace nests:
//!
//! ```no_run
//! use yggdryl::DataType;
//! use yggdryl::iceberg::Catalog;
//! use yggdryl::local::Folder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = Catalog::new(Folder::new(Folder::temporary()?.path()?.join("warehouse"))?);
//!
//! let schema = DataType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::Utf8.nullable_field("venue").with_partition(true),
//! ])?
//! .required_field("row");
//!
//! // The namespaces come into being because the metadata document was
//! // written, not because anything checked for them first.
//! let table = catalog.tables().create("nyc.taxis", schema)?;
//! assert!(table.current_snapshot().is_none());
//!
//! // The same table through the cascade, and through the dotted entry point.
//! let nyc = catalog.namespaces().get("nyc")?;
//! assert!(nyc.tables().contains("taxis")?);
//! let _same = catalog.table("nyc.taxis")?;
//! # Ok(())
//! # }
//! ```
//!
//! Dotted names are resolved in one place: a collection's [`Tables::get`],
//! [`Tables::create`], and their namespace siblings accept a dotted
//! identifier and descend - `namespaces.get("sales.eu")`,
//! `tables.get("sales.eu.orders")` - so the resolution rule lives in the
//! collection and not in five call sites.
//!
//! [`get`](Tables::get) returns `Result` and nothing implements `Index`:
//! panic-on-missing is normal for an in-memory child lookup and is not normal
//! for a storage lookup. The bindings give Python and JavaScript the map
//! spelling their readers expect instead.
//!
//! # What is not here
//!
//! - No `drop_table` and no namespace removal: a catalog's folders hold data
//!   files, and a recursive delete is the operation that turns one wrong URL
//!   into a lost warehouse. [`IOBase::remove`] removes a *leaf* or an empty
//!   container; emptying a table's history is [`Table`] maintenance work.
//! - No `rename_table`: the storage contract has no move primitive, and a
//!   rename that re-copied every data file would be a copy wearing a rename's
//!   name.
//! - No catalog service client: a REST or Hive catalog is a network client
//!   and this module holds no network code, so a table is found from its own
//!   metadata documents, the way [`Table::locate`] finds one.

mod catalogs;
mod namespaces;
mod tables;

pub use catalogs::{Catalog, Catalogs};
pub use namespaces::{Namespace, Namespaces};
pub use tables::Tables;

use smol_str::{SmolStr, format_smolstr};

use super::Table;
use crate::generic::Holder;
use crate::io::IOBase;
use crate::metadata::Metadata;
use crate::{Error, Result, Scalar};

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
/// Protocol metadata is inert `<scheme>:<property>` strings, and `iceberg:` is
/// the format's own scheme - a caller writing under it could silently change
/// what the format later reads, so the update path refuses it by name.
const RESERVED_PREFIX: &str = "iceberg:";

/// The names of one collection level, yielded one at a time.
///
/// The walk runs as the iterator is drained - listing a warehouse of a
/// hundred thousand namespaces and taking three costs three entries' worth of
/// backend calls. The item is a [`Result`], so a listing fails *at* the
/// failing entry, naming it, and the iterator is fused afterwards. Order is
/// the storage listing's, which is sorted, so the same collection over the
/// same state yields the same sequence twice.
pub struct Names {
    /// The walk still running. `None` once the listing is spent.
    entries: Option<Box<dyn Iterator<Item = Result<String>> + Send + Sync>>,
}

impl Names {
    /// A listing of nothing, which a missing parent answers with.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    /// Wrap a walk that is already lazy.
    fn new(entries: impl Iterator<Item = Result<String>> + Send + Sync + 'static) -> Self {
        Self {
            entries: Some(Box::new(entries)),
        }
    }

    /// A listing that reports one failure and then ends.
    fn failing(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }
}

impl Iterator for Names {
    type Item = Result<String>;

    fn next(&mut self) -> Option<Self::Item> {
        let entries = self.entries.as_mut()?;
        match entries.next() {
            Some(Ok(entry)) => Some(Ok(entry)),
            Some(Err(error)) => {
                self.entries = None;
                Some(Err(error))
            }
            None => {
                self.entries = None;
                None
            }
        }
    }
}

impl std::iter::FusedIterator for Names {}

impl std::fmt::Debug for Names {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Names")
            .field("spent", &self.entries.is_none())
            .finish()
    }
}

/// What one classification found at a resolved location.
///
/// The classification *is* the act, per the existence contract: one listing
/// probe answers emptiness, and one [`Table::locate`] both distinguishes a
/// table from a namespace and opens the table when it is one - so a `get`
/// that finds a table has already paid for opening it, and nothing is asked
/// twice.
enum Occupant {
    /// A folder holding a current table metadata document, opened.
    Table(Box<Table<Holder>>),
    /// A folder that exists and is not a table, handed back.
    Namespace(Holder),
    /// Nothing at all, handed back so a create needs no second resolution.
    Nothing(Holder),
    /// An actual file occupies the location, which can be neither.
    File,
}

/// Classify what occupies `folder`, in one pass, cheapest evidence first.
///
/// Presence costs one backend call, so a miss - the common case on a create
/// path - costs exactly that one call. Only a present folder pays for
/// [`Table::locate`], which is also what *opens* the table when the metadata
/// is there - so a `get` that finds a table has already parsed its document.
/// A location an actual file occupies answers [`Occupant::File`] from the
/// act's own `NotADirectory` failure - nothing probes twice.
fn classify(folder: Holder) -> Result<Occupant> {
    if !folder_present(&folder)? {
        return Ok(Occupant::Nothing(folder));
    }
    match Table::locate_keeping(folder) {
        Ok(Ok(table)) => Ok(Occupant::Table(Box::new(table))),
        Ok(Err(folder)) => Ok(Occupant::Namespace(folder)),
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
        Holder::Folder(folder) => Ok(folder.exists()),
        Holder::ArrowFolder(folder) => Ok(folder.exists()),
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

/// Resolve the folder a dotted name addresses, touching nothing.
///
/// A dotted name addresses a container by definition - a namespace is a
/// folder and a table is a folder - so the location is described in the
/// folder role outright, per backend, without asking what is actually there.
/// The act that follows is what answers: listing a file's path fails with the
/// backend's own `NotADirectory`, which [`classify`] turns into the refusal.
fn resolve(warehouse: &(impl IOBase + ?Sized), name: &str) -> Result<Holder> {
    let path = segments(name)?.join("/");
    folder_role(warehouse.child_by_path(&path)?)
}

/// Re-describe a resolved child in the folder role, touching nothing.
///
/// A backend's `child_by_path` answers with what it believes is there, and
/// for a location nothing occupies yet that is a leaf-to-be. A catalog knows
/// better - its names address containers - so the leaf spellings are re-cast
/// as the same backend's folder, keeping the handle's filesystem and URL.
fn folder_role(child: Holder) -> Result<Holder> {
    match child {
        Holder::Folder(_) | Holder::ArrowFolder(_) => Ok(child),
        Holder::Path(path) => Ok(Holder::Folder(crate::local::Folder::from_url(
            path.url().clone(),
        )?)),
        Holder::File(file) => match file.url() {
            Some(url) => Ok(Holder::Folder(crate::local::Folder::from_url(url.clone())?)),
            None => Err(invalid(SmolStr::new_static(
                "expected a located folder for a catalog name, got a handle with no URL",
            ))),
        },
        Holder::ArrowPath(path) => Ok(Holder::ArrowFolder(crate::arrowfs::Folder::new(
            path.filesystem().clone(),
            path.url().clone(),
        ))),
        Holder::ArrowFile(file) => Ok(Holder::ArrowFolder(crate::arrowfs::Folder::new(
            file.filesystem().clone(),
            file.url().clone(),
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

/// Split a dotted table or namespace name into its folder segments.
///
/// `"nyc.taxis"` names the folder `nyc/taxis`: every dot is one namespace
/// level, at any depth. Each segment must be usable as one folder name, so an
/// empty segment, a path separator, a `column=value` spelling, and the
/// reserved `metadata` name are refused by name rather than resolved into a
/// layout they would collide with.
fn segments(name: &str) -> Result<Vec<&str>> {
    let parts: Vec<&str> = name.split('.').collect();
    for segment in &parts {
        if segment.is_empty() {
            return Err(invalid(format_smolstr!(
                "expected non-empty dot-separated segments in the name {name:?}, got an empty one"
            )));
        }
        if segment.contains('/') {
            return Err(invalid(format_smolstr!(
                "expected a segment without '/' in the name {name:?}, got {segment:?}; \
                 a namespace nests with dots, not path separators"
            )));
        }
        if segment.contains('=') {
            return Err(invalid(format_smolstr!(
                "expected a segment without '=' in the name {name:?}, got {segment:?}; \
                 a column=value folder is a partition directory, not a name"
            )));
        }
        if *segment == METADATA_DIR {
            return Err(invalid(format_smolstr!(
                "expected a segment other than {METADATA_DIR:?} in the name {name:?}; \
                 that folder is where each level keeps its own metadata document"
            )));
        }
    }
    Ok(parts)
}

/// Read the properties document under `folder`, absent meaning empty.
///
/// The read is the act: a document that is not there reads as zero bytes and
/// answers empty properties - never a missing-file failure a caller has to
/// catch. A document that is there but is not the expected shape is an error
/// naming what was found.
fn read_properties(folder: &Holder, document: &str) -> Result<Metadata> {
    read_properties_from(&folder.child_by_path(document)?)
}

/// [`read_properties`], from the document handle itself.
fn read_properties_from(document: &Holder) -> Result<Metadata> {
    let bytes = document.read_all_bytes()?;
    if bytes.is_empty() {
        return Ok(Metadata::new());
    }
    let described = document
        .url()
        .map_or_else(|| "<memory>".to_owned(), ToString::to_string);
    let value = crate::json::from_bytes(&bytes)?;
    let Some(entries) = value.get_key_str("properties") else {
        return Err(invalid(format_smolstr!(
            "expected a {{\"properties\": ...}} document at {described}, got one without the key"
        )));
    };
    let mut pairs = Vec::with_capacity(entries.len());
    if let Some(record) = entries.as_record() {
        for (key, value) in record {
            let Some(value) = value.as_str() else {
                return Err(invalid(format_smolstr!(
                    "expected string property pairs at {described}, got {key:?}: {value:?}"
                )));
            };
            pairs.push((key.clone(), SmolStr::new(value)));
        }
    } else if let Some(mapping) = entries.as_mapping() {
        for (key, value) in mapping {
            let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
                return Err(invalid(format_smolstr!(
                    "expected string property pairs at {described}, got {key:?}: {value:?}"
                )));
            };
            pairs.push((SmolStr::new(key), SmolStr::new(value)));
        }
    } else {
        return Err(invalid(format_smolstr!(
            "expected \"properties\" to hold a mapping at {described} \
             (a record or mapping value), got {entries:?}"
        )));
    }
    Metadata::from_entries(pairs)
}

/// Apply property updates and removals to `folder`'s document, transactionally.
///
/// The whole new document is built and validated before a byte is written, so
/// a failure leaves the stored value unchanged; the write itself is a
/// whole-value replacement, which every backend publishes atomically or not
/// at all. Writing the document is also what creates the folder and its
/// ancestry, which is exactly how an empty namespace becomes durable.
fn update_properties(
    folder: &Holder,
    document: &str,
    updates: impl IntoIterator<Item = (String, String)>,
    removes: impl IntoIterator<Item = String>,
) -> Result<()> {
    update_properties_at(&mut folder.child_by_path(document)?, updates, removes)
}

/// [`update_properties`], on the document handle itself.
fn update_properties_at(
    document: &mut Holder,
    updates: impl IntoIterator<Item = (String, String)>,
    removes: impl IntoIterator<Item = String>,
) -> Result<()> {
    let current = read_properties_from(document)?;
    let mut pairs: Vec<(SmolStr, SmolStr)> = current
        .iter()
        .map(|(key, value)| (SmolStr::new(key), SmolStr::new(value)))
        .collect();
    for (key, value) in updates {
        if key.starts_with(RESERVED_PREFIX) {
            return Err(invalid(format_smolstr!(
                "expected a property key outside the reserved {RESERVED_PREFIX:?} prefix, \
                 got {key:?}"
            )));
        }
        match pairs.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, existing)) => *existing = SmolStr::new(&value),
            None => pairs.push((SmolStr::new(&key), SmolStr::new(&value))),
        }
    }
    for key in removes {
        pairs.retain(|(existing, _)| *existing != key);
    }
    write_properties_at(document, pairs)
}

/// Write `pairs` as the properties document under `folder`.
fn write_properties(folder: &Holder, document: &str, pairs: Vec<(SmolStr, SmolStr)>) -> Result<()> {
    write_properties_at(&mut folder.child_by_path(document)?, pairs)
}

/// [`write_properties`], on the document handle itself.
fn write_properties_at(document: &mut Holder, pairs: Vec<(SmolStr, SmolStr)>) -> Result<()> {
    let properties = Scalar::from_mapping(
        pairs
            .into_iter()
            .map(|(key, value)| (Scalar::from(key.as_str()), Scalar::from(value.as_str()))),
    )?;
    let body = Scalar::from_mapping([(Scalar::from("properties"), properties)])?;
    let bytes = crate::json::into_bytes(&body)?;
    document.write_all_bytes(&bytes)?;
    Ok(())
}

/// Report a name or a folder this catalog cannot accept.
fn invalid(reason: SmolStr) -> Error {
    Error::Codec {
        format: "iceberg",
        position: 0,
        reason,
    }
}

#[cfg(test)]
mod tests;
