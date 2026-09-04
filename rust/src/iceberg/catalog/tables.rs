//! The bottom of the cascade: the tables of one namespace, or of the root.

use smol_str::{SmolStr, format_smolstr};

use super::super::{
    FormatVersion, IcebergOptions, PartitionSpec, Table, assign_field_ids, last_column_id,
};
use super::catalogs::{Catalog, level_names};
use super::{Names, Occupant, classify, invalid, resolve};
use crate::IOBase;
use crate::arrow::{BatchReader, field_from_arrow_schema};
use crate::generic::Holder;
use crate::{Field, Result};

/// The root name a schema inferred from an incoming reader is given.
const ROOT_NAME: &str = crate::generic::DEFAULT_ROOT_NAME;

/// The tables of one namespace - or of the warehouse root - as a lazy view.
///
/// The same shape as [`super::Namespaces`], one level down the chain:
/// constructing the view performs no I/O, every question consults storage
/// when asked, and [`Self::get`] indexes to a [`Table`]. Names may be dotted
/// (`tables.get("sales.eu.orders")` descends), so the resolution rule lives
/// here. The write conveniences that take a name, [`Self::append_arrow_reader`] and
/// [`Self::overwrite_arrow_reader`], create the table on first write, from the incoming
/// rows' own schema.
#[derive(Debug)]
pub struct Tables<'catalog, H: IOBase> {
    /// The catalog every question is asked of.
    catalog: &'catalog Catalog<H>,
    /// The owning namespace's dotted name; `None` is the warehouse root.
    namespace: Option<SmolStr>,
}

impl<'catalog, H: IOBase> Tables<'catalog, H> {
    /// The view at the warehouse root, where names may be fully dotted.
    pub(super) const fn at_root(catalog: &'catalog Catalog<H>) -> Self {
        Self {
            catalog,
            namespace: None,
        }
    }

    /// The view inside one namespace.
    pub(super) const fn under(catalog: &'catalog Catalog<H>, namespace: SmolStr) -> Self {
        Self {
            catalog,
            namespace: Some(namespace),
        }
    }

    /// Spell one table's full dotted name under this namespace.
    fn dotted(&self, table: &str) -> SmolStr {
        match &self.namespace {
            Some(namespace) => format_smolstr!("{namespace}.{table}"),
            None => SmolStr::new(table),
        }
    }

    /// Open the named table, raised as a typed absence when nothing is there.
    ///
    /// The locate *is* the open: a `get` that finds the table has already
    /// parsed its current metadata document, so nothing is asked twice.
    ///
    /// # Errors
    ///
    /// Returns a typed absence naming the dotted name when nothing or only a
    /// namespace is there, and the metadata failure when the current document
    /// cannot be read.
    pub fn get(&self, name: &str) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Table(table) => Ok(*table),
            Occupant::Namespace(_) => Err(invalid(format_smolstr!(
                "expected a table at {dotted:?}, got a namespace"
            ))),
            Occupant::Nothing(_) => Err(crate::Error::absent("table", dotted)),
            Occupant::File => Err(invalid(format_smolstr!(
                "expected a table folder at {dotted:?}, got a file"
            ))),
        }
    }

    /// Create the named table, writing its first metadata document.
    ///
    /// Unnumbered schema fields are numbered above the highest identifier the
    /// schema already carries, and the partition spec is derived from the
    /// columns the schema itself marks - a schema that marks none produces an
    /// unpartitioned table. The table is written as format version 2.
    ///
    /// Writing the first metadata document is what creates every missing
    /// ancestor namespace folder - nothing checks for them and nothing makes
    /// them in advance. The conflict comes from the same one classification
    /// the open paths use, never from a separate probe. Storage has no
    /// compare-and-swap, so callers that require concurrent-create exclusion
    /// must serialize publication through their catalog. Sequential creation
    /// still returns a typed conflict and never replaces the table.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict when a table is already there, a refusal when
    /// the name is bad or the schema is not a non-null struct root, and the
    /// write failure otherwise.
    pub fn create(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Nothing(folder) => Self::create_at(folder, schema),
            // An occupied non-table folder is a namespace, and a table
            // sharing a namespace's exact dotted name would collide with it.
            Occupant::Namespace(_) => Err(crate::Error::conflict("table", "namespace", dotted)),
            Occupant::Table(_) => Err(crate::Error::conflict("table", "table", dotted)),
            Occupant::File => Err(crate::Error::conflict("table", "file", dotted)),
        }
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is - `schema` describes only the
    /// table this call would create. One classification, one code path.
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create(&self, name: &str, schema: Field) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Table(table) => Ok(*table),
            Occupant::Nothing(folder) => Self::create_at(folder, schema),
            Occupant::Namespace(_) => Err(invalid(format_smolstr!(
                "expected a table at {dotted:?}, got a namespace"
            ))),
            Occupant::File => Err(invalid(format_smolstr!(
                "expected a table folder at {dotted:?}, got a file"
            ))),
        }
    }

    /// Open the named table, creating it from the reader's schema when absent.
    ///
    /// This is the first half of [`Self::append_arrow_reader`] and [`Self::overwrite_arrow_reader`],
    /// public so a caller can settle per-call [`IcebergOptions`] on the table
    /// before handing it the rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the reader's schema
    /// cannot describe a table, or when the create fails.
    pub fn open_or_create_from_arrow_reader(
        &self,
        name: &str,
        batches: &BatchReader,
    ) -> Result<Table<Holder>> {
        let dotted = self.dotted(name);
        match classify(resolve(self.catalog.warehouse(), &dotted)?)? {
            Occupant::Table(table) => Ok(*table),
            Occupant::Nothing(folder) => Self::create_at(
                folder,
                field_from_arrow_schema(ROOT_NAME, &batches.schema())?,
            ),
            Occupant::Namespace(_) => Err(invalid(format_smolstr!(
                "expected a table at {dotted:?}, got a namespace"
            ))),
            Occupant::File => Err(invalid(format_smolstr!(
                "expected a table folder at {dotted:?}, got a file"
            ))),
        }
    }

    /// Return whether the named table exists, asked of storage now.
    ///
    /// This is an answer for a caller who asked; nothing in this module calls
    /// it on the way to doing something else.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, or when a metadata document
    /// is found but is not table metadata.
    pub fn contains(&self, name: &str) -> Result<bool> {
        Ok(matches!(
            classify(resolve(self.catalog.warehouse(), &self.dotted(name))?)?,
            Occupant::Table(_)
        ))
    }

    /// The table names one level down, one at a time, sorted.
    ///
    /// One classification per entry, lazily: taking three names classifies
    /// three, and a namespace that does not exist lists nothing rather than
    /// failing.
    pub fn iter(&self) -> Names {
        let level = match &self.namespace {
            Some(namespace) => match resolve(self.catalog.warehouse(), namespace) {
                Ok(folder) => folder.ls(false, false),
                Err(error) => return Names::failing(error),
            },
            None => self.catalog.warehouse().ls(false, false),
        };
        level_names(level, true)
    }

    /// Return how many tables the namespace holds, which drains the listing.
    ///
    /// # Errors
    ///
    /// Returns the first listing failure.
    pub fn len(&self) -> Result<usize> {
        let mut count = 0;
        for name in self.iter() {
            name?;
            count += 1;
        }
        Ok(count)
    }

    /// Return whether the namespace holds no table, which costs the listing
    /// up to its first table.
    ///
    /// # Errors
    ///
    /// Returns the first listing failure.
    pub fn is_empty(&self) -> Result<bool> {
        match self.iter().next() {
            Some(entry) => entry.map(|_| false),
            None => Ok(true),
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
    pub fn append_arrow_reader(&self, name: &str, batches: BatchReader) -> Result<Table<Holder>> {
        self.append_arrow_reader_with_options(name, batches, None)
    }

    /// [`Self::append_arrow_reader`] with per-call [`IcebergOptions`] settled first.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::append_arrow_reader`] returns.
    pub fn append_arrow_reader_with_options(
        &self,
        name: &str,
        batches: BatchReader,
        options: Option<IcebergOptions>,
    ) -> Result<Table<Holder>> {
        let mut table = self.open_or_create_from_arrow_reader(name, &batches)?;
        if let Some(options) = options {
            table.set_options(options);
        }
        table.commit_append(batches)?;
        Ok(table)
    }

    /// Replace the named table's rows with `batches`, creating it on first
    /// write.
    ///
    /// The same shape as [`Self::append_arrow_reader`] with [`Table::commit_overwrite`] as the
    /// write: an absent table is created from the reader's schema, an
    /// existing one keeps its previous snapshot readable. Returns the table
    /// so the caller can keep going.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is refused, when the reader's schema
    /// cannot describe a table, or when the create or the overwrite fails.
    pub fn overwrite_arrow_reader(
        &self,
        name: &str,
        batches: BatchReader,
    ) -> Result<Table<Holder>> {
        self.overwrite_arrow_reader_with_options(name, batches, None)
    }

    /// [`Self::overwrite_arrow_reader`] with per-call [`IcebergOptions`] settled first.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::overwrite_arrow_reader`] returns.
    pub fn overwrite_arrow_reader_with_options(
        &self,
        name: &str,
        batches: BatchReader,
        options: Option<IcebergOptions>,
    ) -> Result<Table<Holder>> {
        let mut table = self.open_or_create_from_arrow_reader(name, &batches)?;
        if let Some(options) = options {
            table.set_options(options);
        }
        table.commit_overwrite(batches)?;
        Ok(table)
    }

    /// Create a table in `folder` from a numbered schema and its own marks.
    ///
    /// This is the create half every path above shares: the classification
    /// already ran, the folder is already resolved, and writing the first
    /// metadata document is what brings the folder and its ancestry into
    /// being.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema is not a non-null struct root, or
    /// when the metadata document cannot be written.
    fn create_at(folder: Holder, mut schema: Field) -> Result<Table<Holder>> {
        // Numbering continues above the highest identifier already assigned,
        // so a partially numbered schema keeps every id it came with.
        let start = last_column_id(&schema)?.saturating_add(1);
        assign_field_ids(&mut schema, start)?;
        let spec = PartitionSpec::from_schema(0, &schema)?;
        Table::create(folder, FormatVersion::V2, schema, spec)
    }
}
