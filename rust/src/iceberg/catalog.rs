//! A warehouse folder of namespaces of Iceberg tables, reached through one
//! handle.
//!
//! A catalog here is storage and nothing else, the way `HadoopCatalog` is: the
//! warehouse is one container handle, a namespace is a folder under it, a
//! table is a folder [`Table::locate`] recognizes, and a dotted name like
//! `"nyc.taxis"` is the folder `nyc/taxis` spelled the way a catalog spells
//! it. Every lookup runs through [`IOBase::child_by_path`] and [`IOBase::ls`]
//! against that one handle - no path is opened, no network is reached - and a
//! [`Catalog`] is only a description of where tables live, so constructing one
//! touches nothing at all.
//!
//! # The object model
//!
//! Each type owns one job, and access chains through two map-oriented views:
//!
//! - [`Catalog::namespaces`] is the collection of namespaces, a [`Namespaces`]
//!   view; indexing it with [`Namespaces::get`] answers a [`Namespace`].
//! - [`Namespace::tables`] is that namespace's collection of tables, a
//!   [`Tables`] view; indexing it with [`Tables::get`] answers a [`Table`].
//! - [`Namespace::namespaces`] is the same [`Namespaces`] shape one level
//!   down, so a nested namespace is reached through its parent's view.
//!
//! The views are cheap handles, not caches: constructing one performs no I/O,
//! and membership, iteration, and length all consult storage at the moment
//! they are asked, so two views over the same catalog observe each other's
//! writes and a view stays valid across creation and deletion. The catalog
//! keeps its dotted-name conveniences - [`Catalog::create_table`] and
//! friends - as one-line delegates to the views, so there is exactly one
//! implementation of every operation.
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
//!
//! // The same table through the chained views.
//! let nyc = catalog.namespaces().get("nyc")?;
//! assert!(nyc.tables().contains("taxis")?);
//! assert_eq!(nyc.tables().names()?, ["taxis"]);
//! assert_eq!(catalog.namespaces().names()?, ["nyc"]);
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

use super::{FormatVersion, IcebergOptions, PartitionSpec, Table, assign_field_ids, last_field_id};
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
/// tables. Its collection is [`Self::namespaces`]; the flat methods here are
/// one-line delegates over that view, kept so a dotted name stays one call.
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

    /// The role this value plays: [`IOKind::Catalog`].
    ///
    /// The warehouse handle underneath answers [`IOKind::Directory`], because a
    /// warehouse is a folder and storage has no way to tell that one folder is
    /// a catalog while the next is not. Framing it as a catalog is what this
    /// value adds, so it is this value that says so - the same way
    /// [`Namespace::kind`] and [`Table`]'s own [`IOBase::kind`] do one level
    /// down each.
    pub const fn kind(&self) -> IOKind {
        IOKind::Catalog
    }

    /// The catalog's namespaces, as a lazy map-oriented view.
    ///
    /// Constructing the view performs no I/O; every question it answers is
    /// asked of storage when it is asked of the view.
    pub const fn namespaces(&self) -> Namespaces<'_, H> {
        Namespaces {
            catalog: self,
            parent: None,
        }
    }

    /// Describe one namespace as a view, touching nothing.
    ///
    /// A namespace is a folder name, so the view exists whether or not the
    /// folder does - exactly as a handle describes a location without proof.
    /// [`Namespaces::get`] is the indexing spelling, which *does* check.
    pub fn namespace(&self, name: &str) -> Namespace<'_, H> {
        Namespace {
            catalog: self,
            name: SmolStr::new(name),
        }
    }

    /// Create the table a dotted name addresses: [`Tables::create`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::create`] returns.
    pub fn create_table(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        let (parent, table) = split_dotted(name);
        self.tables_under(parent).create(table, schema)
    }

    /// Open the table a dotted name addresses: [`Tables::get`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::get`] returns.
    pub fn table(&self, name: &str) -> Result<Table<Holder>> {
        let (parent, table) = split_dotted(name);
        self.tables_under(parent).get(table)
    }

    /// Return whether the dotted name addresses a table: [`Tables::contains`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::contains`] returns.
    pub fn has_table(&self, name: &str) -> Result<bool> {
        let (parent, table) = split_dotted(name);
        self.tables_under(parent).contains(table)
    }

    /// Open or create the table a dotted name addresses:
    /// [`Tables::open_or_create`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::open_or_create`] returns.
    pub fn open_or_create_table(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        let (parent, table) = split_dotted(name);
        self.tables_under(parent).open_or_create(table, schema)
    }

    /// Append to the table a dotted name addresses: [`Tables::append`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::append`] returns.
    pub fn append(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        self.append_with(name, batches, None)
    }

    /// [`Self::append`] with per-call options: [`Tables::append_with`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::append_with`] returns.
    pub fn append_with(
        &self,
        name: &str,
        batches: BatchReader,
        options: Option<IcebergOptions>,
    ) -> Result<Table<Holder>> {
        let (parent, table) = split_dotted(name);
        self.tables_under(parent)
            .append_with(table, batches, options)
    }

    /// Overwrite the table a dotted name addresses: [`Tables::overwrite`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::overwrite`] returns.
    pub fn overwrite(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        self.overwrite_with(name, batches, None)
    }

    /// [`Self::overwrite`] with per-call options: [`Tables::overwrite_with`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::overwrite_with`] returns.
    pub fn overwrite_with(
        &self,
        name: &str,
        batches: BatchReader,
        options: Option<IcebergOptions>,
    ) -> Result<Table<Holder>> {
        let (parent, table) = split_dotted(name);
        self.tables_under(parent)
            .overwrite_with(table, batches, options)
    }

    /// List the namespaces one level below `parent`, as dotted names:
    /// [`Namespaces::names`] with the parent prefix restored.
    ///
    /// # Errors
    ///
    /// Returns what [`Namespaces::names`] returns.
    pub fn list_namespaces(&self, parent: Option<&str>) -> Result<Vec<String>> {
        let view = Namespaces {
            catalog: self,
            parent: parent.map(SmolStr::new),
        };
        Ok(view
            .names()?
            .into_iter()
            .map(|name| match parent {
                Some(parent) => format!("{parent}.{name}"),
                None => name,
            })
            .collect())
    }

    /// List the tables in a namespace, as sorted dotted names:
    /// [`Tables::names`] with the namespace prefix restored.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::names`] returns.
    pub fn list_tables(&self, namespace: &str) -> Result<Vec<String>> {
        Ok(self
            .tables_under(Some(SmolStr::new(namespace)))
            .names()?
            .into_iter()
            .map(|name| format!("{namespace}.{name}"))
            .collect())
    }

    /// The tables view for one namespace, or for the warehouse root.
    const fn tables_under(&self, namespace: Option<SmolStr>) -> Tables<'_, H> {
        Tables {
            catalog: self,
            namespace,
        }
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
        let child = self.warehouse.child_by_path(&segments(name)?.join("/"))?;
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

    /// List one level of child folders under `parent`, keeping tables or
    /// namespaces, as sorted bare names.
    ///
    /// A child that holds a metadata document is a table; a plain folder -
    /// even one somebody else made - is a namespace, because that is all a
    /// namespace is. A parent that does not exist lists nothing rather than
    /// failing.
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
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }
}

/// The namespaces one level below a catalog or a namespace, as a lazy view.
///
/// The view holds the catalog and the parent's dotted name and nothing else:
/// constructing one performs no I/O, membership and listing consult storage
/// when asked, and [`Self::get`] indexes to a [`Namespace`]. Two views over
/// the same catalog therefore observe each other's writes, and a view stays
/// valid across creation and deletion because every answer is storage's.
#[derive(Debug)]
pub struct Namespaces<'catalog, H: IOBase> {
    /// The catalog every question is asked of.
    catalog: &'catalog Catalog<H>,
    /// The parent namespace's dotted name; `None` is the warehouse root.
    parent: Option<SmolStr>,
}

impl<'catalog, H: IOBase> Namespaces<'catalog, H> {
    /// Spell one child namespace's full dotted name.
    fn dotted(&self, name: &str) -> SmolStr {
        match &self.parent {
            Some(parent) => format_smolstr!("{parent}.{name}"),
            None => SmolStr::new(name),
        }
    }

    /// List the namespaces one level down, as sorted bare names.
    ///
    /// # Errors
    ///
    /// Returns an error when the parent name is refused or the listing fails.
    pub fn names(&self) -> Result<Vec<String>> {
        self.catalog.list_children(self.parent.as_deref(), false)
    }

    /// Return how many namespaces are one level down, right now.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::names`] returns.
    pub fn len(&self) -> Result<usize> {
        Ok(self.names()?.len())
    }

    /// Return whether no namespace is one level down, right now.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::names`] returns.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.names()?.is_empty())
    }

    /// Return whether the named namespace exists, asked of storage now.
    ///
    /// A namespace is a folder that is not a table, so a table's name answers
    /// `false` here, and so does a location nothing occupies yet.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused or the probe fails.
    pub fn contains(&self, name: &str) -> Result<bool> {
        let dotted = self.dotted(name);
        let child = self
            .catalog
            .warehouse
            .child_by_path(&segments(&dotted)?.join("/"))?;
        if !child.is_container() {
            return Ok(false);
        }
        Ok(Table::locate(child)?.is_none())
    }

    /// Index one namespace: the view of it, checked to exist.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the namespace when nothing is there, or
    /// when the name addresses a table instead.
    pub fn get(&self, name: &str) -> Result<Namespace<'catalog, H>> {
        let dotted = self.dotted(name);
        if !self.contains(name)? {
            return Err(invalid(format_smolstr!(
                "expected a namespace at {dotted:?}, got none; create it with create"
            )));
        }
        Ok(Namespace {
            catalog: self.catalog,
            name: dotted,
        })
    }

    /// Create the named namespace, as the folder it is.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the namespace when one - or a table - is
    /// already there, and the storage failure otherwise.
    pub fn create(&self, name: &str) -> Result<Namespace<'catalog, H>> {
        let dotted = self.dotted(name);
        let child = self
            .catalog
            .warehouse
            .child_by_path(&segments(&dotted)?.join("/"))?;
        if child.is_container() {
            let what = if Table::locate(child)?.is_some() {
                "a table"
            } else {
                "one"
            };
            return Err(invalid(format_smolstr!(
                "expected no namespace at {dotted:?}, got {what}; open it with get"
            )));
        }
        self.open_or_create(name)
    }

    /// Open the named namespace, creating its folder when absent.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, addresses a table, or the
    /// folder cannot be created.
    pub fn open_or_create(&self, name: &str) -> Result<Namespace<'catalog, H>> {
        let dotted = self.dotted(name);
        let child = self
            .catalog
            .warehouse
            .child_by_path(&segments(&dotted)?.join("/"))?;
        if child.is_container() {
            if Table::locate(child)?.is_some() {
                return Err(invalid(format_smolstr!(
                    "expected a namespace at {dotted:?}, got a table"
                )));
            }
        } else {
            // An empty truncate is the storage spelling of `mkdir -p`: it
            // brings the folder into being without writing a byte into it.
            let mut folder = self.catalog.resolve(&dotted)?;
            folder.truncate(0)?;
        }
        Ok(Namespace {
            catalog: self.catalog,
            name: dotted,
        })
    }
}

/// One namespace of a catalog: identity, plus its two collection views.
///
/// The view borrows the catalog and holds only the namespace's dotted name;
/// its tables are [`Self::tables`] and its child namespaces are
/// [`Self::namespaces`], each resolving against the warehouse at the moment
/// an operation runs, so the namespace is as lazy as the catalog itself.
#[derive(Debug)]
pub struct Namespace<'catalog, H: IOBase> {
    catalog: &'catalog Catalog<H>,
    name: SmolStr,
}

impl<'catalog, H: IOBase> Namespace<'catalog, H> {
    /// The namespace's dotted name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The role this view plays: [`IOKind::Namespace`].
    ///
    /// A namespace is a folder that is not a table, which is all storage can
    /// see; that it is a namespace rather than any other folder is what the
    /// catalog framing adds, so the view is what answers it.
    pub const fn kind(&self) -> IOKind {
        IOKind::Namespace
    }

    /// This namespace's tables, as a lazy map-oriented view.
    pub fn tables(&self) -> Tables<'catalog, H> {
        Tables {
            catalog: self.catalog,
            namespace: Some(self.name.clone()),
        }
    }

    /// The namespaces one level below this one, as the same view shape the
    /// catalog itself answers - the cascade that reaches a nested namespace.
    pub fn namespaces(&self) -> Namespaces<'catalog, H> {
        Namespaces {
            catalog: self.catalog,
            parent: Some(self.name.clone()),
        }
    }

    /// Open the named table: [`Tables::get`], one line.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::get`] returns.
    pub fn table(&self, name: &str) -> Result<Table<Holder>> {
        self.tables().get(name)
    }

    /// Return whether the named table exists here: [`Tables::contains`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::contains`] returns.
    pub fn has_table(&self, name: &str) -> Result<bool> {
        self.tables().contains(name)
    }

    /// Create the named table: [`Tables::create`], one line.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::create`] returns.
    pub fn create_table(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        self.tables().create(name, schema)
    }

    /// Open or create the named table: [`Tables::open_or_create`], one line.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::open_or_create`] returns.
    pub fn open_or_create_table(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        self.tables().open_or_create(name, schema)
    }

    /// Append to the named table: [`Tables::append`], one line.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::append`] returns.
    pub fn append(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        self.tables().append(name, batches)
    }

    /// Overwrite the named table: [`Tables::overwrite`], one line.
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::overwrite`] returns.
    pub fn overwrite(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        self.tables().overwrite(name, batches)
    }

    /// List this namespace's tables, as bare names: [`Tables::names`].
    ///
    /// # Errors
    ///
    /// Returns what [`Tables::names`] returns.
    pub fn list_tables(&self) -> Result<Vec<String>> {
        self.tables().names()
    }

    /// List the child namespaces, as bare names: [`Namespaces::names`].
    ///
    /// # Errors
    ///
    /// Returns what [`Namespaces::names`] returns.
    pub fn list_namespaces(&self) -> Result<Vec<String>> {
        self.namespaces().names()
    }
}

/// The tables of one namespace - or of the warehouse root - as a lazy view.
///
/// The same shape as [`Namespaces`], one level down the chain: constructing
/// the view performs no I/O, every question consults storage when asked, and
/// [`Self::get`] indexes to a [`Table`]. The write conveniences that take a
/// name - [`Self::append`] and [`Self::overwrite`] - create the table on
/// first write, from the incoming rows' own schema.
#[derive(Debug)]
pub struct Tables<'catalog, H: IOBase> {
    /// The catalog every question is asked of.
    catalog: &'catalog Catalog<H>,
    /// The owning namespace's dotted name; `None` is the warehouse root.
    namespace: Option<SmolStr>,
}

impl<H: IOBase> Tables<'_, H> {
    /// Spell one table's full dotted name under this namespace.
    fn dotted(&self, table: &str) -> SmolStr {
        match &self.namespace {
            Some(namespace) => format_smolstr!("{namespace}.{table}"),
            None => SmolStr::new(table),
        }
    }

    /// List this namespace's tables, as sorted bare names.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace name is refused, the listing
    /// fails, or a child's metadata document is not table metadata.
    pub fn names(&self) -> Result<Vec<String>> {
        self.catalog.list_children(self.namespace.as_deref(), true)
    }

    /// Return how many tables the namespace holds, right now.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::names`] returns.
    pub fn len(&self) -> Result<usize> {
        Ok(self.names()?.len())
    }

    /// Return whether the namespace holds no table, right now.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::names`] returns.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.names()?.is_empty())
    }

    /// Return whether the named table exists, asked of storage now.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, or when a metadata document
    /// is found but is not table metadata.
    pub fn contains(&self, name: &str) -> Result<bool> {
        Ok(Table::locate(self.catalog.resolve(&self.dotted(name))?)?.is_some())
    }

    /// Index one table: open it, checked to exist.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the table when no table is there, and the
    /// metadata failure when its current document cannot be read.
    pub fn get(&self, name: &str) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        Table::locate(self.catalog.resolve(&dotted)?)?.ok_or_else(|| {
            invalid(format_smolstr!(
                "expected a table at {dotted:?}, got none; create it with create_table"
            ))
        })
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
    pub fn create(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        if self.contains(name)? {
            return Err(invalid(format_smolstr!(
                "expected no table at {dotted:?}, got one; open it with table"
            )));
        }
        self.create_at(&dotted, schema)
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is - `schema` describes only the
    /// table this call would create.
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        match Table::locate(self.catalog.resolve(&self.dotted(name))?)? {
            Some(table) => Ok(table),
            None => self.create_at(&self.dotted(name), schema),
        }
    }

    /// Open the named table, creating it from the reader's schema when absent.
    ///
    /// This is the first half of [`Self::append`] and [`Self::overwrite`],
    /// public so a caller can settle per-call [`IcebergOptions`] on the table
    /// before handing it the rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the reader's schema
    /// cannot describe a table, or when the create fails.
    pub fn open_or_create_from(&self, name: &str, batches: &BatchReader) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        if let Some(table) = Table::locate(self.catalog.resolve(&dotted)?)? {
            return Ok(table);
        }
        self.create_at(
            &dotted,
            record_schema_from_arrow(ROOT_NAME, &batches.schema())?,
        )
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
        self.append_with(name, batches, None)
    }

    /// [`Self::append`] with per-call [`IcebergOptions`] settled first.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::append`] returns.
    pub fn append_with(
        &self,
        name: &str,
        batches: BatchReader,
        options: Option<IcebergOptions>,
    ) -> Result<Table<Holder>> {
        let mut table = self.open_or_create_from(name, &batches)?;
        if let Some(options) = options {
            table.set_options(options);
        }
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
        self.overwrite_with(name, batches, None)
    }

    /// [`Self::overwrite`] with per-call [`IcebergOptions`] settled first.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::overwrite`] returns.
    pub fn overwrite_with(
        &self,
        name: &str,
        batches: BatchReader,
        options: Option<IcebergOptions>,
    ) -> Result<Table<Holder>> {
        let mut table = self.open_or_create_from(name, &batches)?;
        if let Some(options) = options {
            table.set_options(options);
        }
        table.overwrite(batches)?;
        Ok(table)
    }

    /// Create the named table from a numbered schema and its own marks.
    ///
    /// This is [`Self::create`] without the existence check, shared with the
    /// paths that have already looked.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the schema is not a
    /// non-null struct root, or when the metadata document cannot be written.
    fn create_at(&self, dotted: &str, mut schema: Field) -> Result<Table<Holder>> {
        // Numbering continues above the highest identifier already assigned,
        // so a partially numbered schema keeps every id it came with.
        let start = last_field_id(&schema)?.saturating_add(1);
        assign_field_ids(&mut schema, start)?;
        let spec = PartitionSpec::from_schema(0, &schema)?;
        Table::create(
            self.catalog.resolve(dotted)?,
            FormatVersion::V2,
            schema,
            spec,
        )
    }
}

/// Split a dotted name into its namespace prefix and its final segment.
///
/// `"nyc.yellow.taxis"` is the table `taxis` of the namespace `nyc.yellow`;
/// a name with no dot addresses the warehouse root directly.
fn split_dotted(name: &str) -> (Option<SmolStr>, &str) {
    match name.rsplit_once('.') {
        Some((parent, leaf)) if !parent.is_empty() && !leaf.is_empty() => {
            (Some(SmolStr::new(parent)), leaf)
        }
        // An empty half is a malformed name; hand it through whole so
        // `segments` refuses it with the message that names the problem.
        _ => (None, name),
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
