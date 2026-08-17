//! A warehouse folder of namespaces of Iceberg tables, reached through one
//! handle.
//!
//! A catalog here is storage and nothing else, the way `HadoopCatalog` is: the
//! warehouse is one container handle, a namespace is a folder under it, a
//! table is a folder [`Table::locate`] recognizes, and a dotted name like
//! `"nyc.taxis"` is the folder `nyc/taxis` spelled the way a catalog spells
//! it. Every lookup runs through [`IOBase::child_by`] and [`IOBase::ls`]
//! against that one handle - no path is opened, no network is reached - and a
//! [`Catalog`] is only a description of where tables live, so constructing one
//! touches nothing at all.
//!
//! ```no_run
//! use yggdryl::DataType;
//! use yggdryl::iceberg::Catalog;
//! use yggdryl::local::Folder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let catalog = Catalog::new(Folder::new(std::env::temp_dir().join("warehouse"))?);
//!
//! // The schema marks its own partition columns, so the table's spec is
//! // derived rather than declared twice; unnumbered fields are numbered.
//! let schema = DataType::from_fields([
//!     DataType::Int64.required_field("id"),
//!     DataType::Utf8.nullable_field("venue").with_partition(true),
//! ])?
//! .required_field("row");
//!
//! let table = catalog.create_table("nyc.taxis", schema)?;
//! assert!(table.current_snapshot().is_none());
//! assert!(catalog.has_table("nyc.taxis")?);
//! assert_eq!(catalog.list_namespaces(None)?, ["nyc"]);
//! assert_eq!(catalog.list_tables("nyc")?, ["nyc.taxis"]);
//! # Ok(())
//! # }
//! ```
//!
//! # What is not here
//!
//! - No `drop_table`: the storage contract has no delete primitive, so a
//!   catalog reached only through [`IOBase`] has nothing to remove a table's
//!   folder with.
//! - No `rename_table`: the storage contract has no move primitive either, and
//!   a rename that re-copied every data file would be a copy wearing a
//!   rename's name.
//! - No catalog service client: a REST or Hive catalog is a network client and
//!   this module holds no network code, so a table is found from its own
//!   metadata documents, the way [`Table::locate`] finds one.

use smol_str::{SmolStr, format_smolstr};

use super::{FormatVersion, PartitionSpec, Table, assign_field_ids, last_field_id};
use crate::arrow::{BatchReader, record_schema_from_arrow};
use crate::generic::Holder;
use crate::io::IOBase;
use crate::local::Folder;
use crate::{Error, Field, IOKind, Result};

/// The root name a schema inferred from an incoming reader is given.
const ROOT_NAME: &str = "row";

/// A warehouse folder of namespaces of Iceberg tables.
///
/// The catalog is a description of where tables live, not proof that any do:
/// constructing one touches nothing, and every operation resolves its dotted
/// name against the warehouse handle at the moment it runs. There is no
/// service in between, so two catalogs over the same folder see the same
/// tables.
#[derive(Debug)]
pub struct Catalog<H: IOBase> {
    /// The folder every namespace and table lives under.
    warehouse: H,
}

impl<H: IOBase> Catalog<H> {
    /// Describe a catalog over a warehouse folder, touching nothing.
    pub const fn new(warehouse: H) -> Self {
        Self { warehouse }
    }

    /// Borrow the warehouse folder the catalog resolves names against.
    pub const fn warehouse(&self) -> &H {
        &self.warehouse
    }

    /// Create the named table, writing its first metadata document.
    ///
    /// Unnumbered schema fields are numbered above the highest identifier the
    /// schema already carries, and the partition spec is derived from the
    /// columns the schema itself marks - a schema that marks none produces an
    /// unpartitioned table. The table is written as format version 2.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when a table is already
    /// there, when the schema is not a non-null struct root, or when the
    /// metadata document cannot be written.
    pub fn create_table(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        if self.has_table(name)? {
            return Err(invalid(format_smolstr!(
                "expected no table at {name:?}, got one; open it with table"
            )));
        }
        self.create_at(name, schema)
    }

    /// Open the named table.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when no table is there, or
    /// when the table's current metadata document cannot be read.
    pub fn table(&self, name: &str) -> Result<Table<Holder>> {
        Table::locate(self.resolve(name)?)?.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a table at {name:?}, got none; create it with create_table"
            ))
        })
    }

    /// Return whether the named table exists.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, or when a metadata document
    /// is found but is not table metadata.
    pub fn has_table(&self, name: &str) -> Result<bool> {
        Ok(Table::locate(self.resolve(name)?)?.is_some())
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is - `schema` describes only the
    /// table this call would create.
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create_table(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        match Table::locate(self.resolve(name)?)? {
            Some(table) => Ok(table),
            None => self.create_at(name, schema),
        }
    }

    /// Append `batches` to the named table, creating it on first write.
    ///
    /// A table that is not there yet takes its schema from the reader: the
    /// Arrow schema becomes a root [`Field`] named `row`, unnumbered fields
    /// are numbered, and the partition marks that rode the Arrow fields'
    /// metadata become the spec - so a marked schema lays its files out
    /// partitioned from the very first append. Returns the table so the
    /// caller can keep going.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the reader's schema
    /// cannot describe a table, or when the create or the append fails.
    pub fn append(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        let mut table = self.table_for_write(name, &batches)?;
        table.append(batches)?;
        Ok(table)
    }

    /// Replace the named table's rows with `batches`, creating it on first
    /// write.
    ///
    /// The same shape as [`Self::append`] with [`Table::overwrite`] as the
    /// write: an absent table is created from the reader's schema, an existing
    /// one keeps its previous snapshot readable. Returns the table so the
    /// caller can keep going.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the reader's schema
    /// cannot describe a table, or when the create or the overwrite fails.
    pub fn overwrite(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        let mut table = self.table_for_write(name, &batches)?;
        table.overwrite(batches)?;
        Ok(table)
    }

    /// List the namespaces one level below `parent`, as dotted names.
    ///
    /// `None` lists the warehouse's own child folders. A child that holds a
    /// metadata document is a table, not a namespace, and is not listed here;
    /// a plain folder somebody else made is a namespace, because that is all a
    /// namespace is. A parent that does not exist lists nothing rather than
    /// failing.
    ///
    /// # Errors
    ///
    /// Returns an error when the parent name is refused, when the listing
    /// fails, or when a child's metadata document is not table metadata.
    pub fn list_namespaces(&self, parent: Option<&str>) -> Result<Vec<String>> {
        self.list_children(parent, false)
    }

    /// List the tables in a namespace, as sorted dotted names.
    ///
    /// A child folder counts exactly when [`Table::locate`] recognizes it, so
    /// a stray plain folder never shows up as a table. A namespace that does
    /// not exist lists nothing rather than failing.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace name is refused, when the listing
    /// fails, or when a child's metadata document is not table metadata.
    pub fn list_tables(&self, namespace: &str) -> Result<Vec<String>> {
        self.list_children(Some(namespace), true)
    }

    /// Resolve the folder a dotted name addresses, touching nothing.
    ///
    /// A dotted name addresses a container by definition - a namespace is a
    /// folder and a table is a folder - so a location nothing has decided yet
    /// is described as a folder rather than left as the leaf-to-be a plain
    /// child lookup answers with. A location an actual file occupies cannot be
    /// a namespace or a table and is refused by name.
    ///
    /// # Errors
    ///
    /// Returns an error when a name segment is refused, when a file occupies
    /// the location, or when the warehouse cannot resolve children.
    fn resolve(&self, name: &str) -> Result<Holder> {
        let child = self.warehouse.child_by(&segments(name)?.join("/"))?;
        if child.is_container() {
            return Ok(child);
        }
        if child.kind() == IOKind::File {
            return Err(invalid(format_smolstr!(
                "expected a namespace or table folder at {name:?}, got a file"
            )));
        }
        // Nothing is there yet. Re-describe the location in the folder role,
        // per backend, so listing it stays empty and writing into it creates
        // it - the laziness contract, kept for containers.
        match child.url() {
            Some(url) => Ok(Folder::from_url(url.clone())?.into()),
            None => Ok(child),
        }
    }

    /// Create the named table from a numbered schema and its own marks.
    ///
    /// This is [`Self::create_table`] without the existence check, shared with
    /// the paths that have already looked.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the schema is not a
    /// non-null struct root, or when the metadata document cannot be written.
    fn create_at(&self, name: &str, mut schema: Field) -> Result<Table<Holder>> {
        // Numbering continues above the highest identifier already assigned,
        // so a partially numbered schema keeps every id it came with.
        let start = last_field_id(&schema)?.saturating_add(1);
        assign_field_ids(&mut schema, start)?;
        let spec = PartitionSpec::from_schema(0, &schema)?;
        Table::create(self.resolve(name)?, FormatVersion::V2, schema, spec)
    }

    /// Open the named table, creating it from the reader's schema when absent.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the reader's schema
    /// cannot describe a table, or when the create fails.
    fn table_for_write(&self, name: &str, batches: &BatchReader) -> Result<Table<Holder>> {
        if let Some(table) = Table::locate(self.resolve(name)?)? {
            return Ok(table);
        }
        self.create_at(
            name,
            record_schema_from_arrow(ROOT_NAME, &batches.schema())?,
        )
    }

    /// List one level of child folders, keeping tables or namespaces.
    ///
    /// # Errors
    ///
    /// Returns an error when the parent name is refused, when the listing
    /// fails, or when a child's metadata document is not table metadata.
    fn list_children(&self, parent: Option<&str>, tables: bool) -> Result<Vec<String>> {
        let children = match parent {
            Some(parent) => self.resolve(parent)?.ls(false, false)?,
            None => self.warehouse.ls(false, false)?,
        };
        let mut names = Vec::new();
        for child in children {
            if !child.is_container() {
                continue;
            }
            let Some(name) = child
                .url()
                .and_then(|url| url.file_name().map(ToOwned::to_owned))
            else {
                continue;
            };
            if Table::locate(child)?.is_some() == tables {
                names.push(match parent {
                    Some(parent) => format!("{parent}.{name}"),
                    None => name,
                });
            }
        }
        names.sort();
        Ok(names)
    }
}

/// Split a dotted table or namespace name into its folder segments.
///
/// `"nyc.taxis"` names the folder `nyc/taxis`: every dot is one namespace
/// level, at any depth. Each segment must be usable as one folder name, so an
/// empty segment, a path separator, and a `column=value` spelling are refused
/// by name rather than resolved into a layout they would collide with.
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
    }
    Ok(parts)
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
