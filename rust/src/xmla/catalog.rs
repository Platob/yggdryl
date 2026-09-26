//! A catalog: a folder whose contents are the tables a provider serves.
//!
//! A [`Catalog`] is a named [`Holder`] over a container, read two levels
//! deep. Directly under the root, a leaf whose media type is a record
//! encoding this build implements is a table, a folder that is a table format
//! (an Iceberg table - one whose `metadata/` holds a version hint or a
//! metadata document - or any folder a store answers [`IOKind::Table`] for)
//! is a table, and every other folder is a *schema*. Under a schema, every such leaf is a table and every folder
//! is one too, read as the table beneath it: a partitioned tree, a table
//! format. So `catalog.schema.table` spells a path two levels under the root
//! and `catalog.table` one level, and no listing has to guess whether a
//! folder of files is one table or several, because where it sits says. A
//! table is named by its file name with the extensions a media type claims
//! taken off, so `trades.parquet` and `trades.arrows.gz` are both `trades`,
//! and a folder keeps its name.
//!
//! Nothing here is cached: a catalog is listed when it is asked, so a table
//! written a moment ago is served on the next request, and one listing is
//! what a request costs.

use smol_str::{SmolStr, format_smolstr};

use crate::holder::Holder;
use crate::media::RecordOptions;
use crate::{Error, Field, IOBase, IOKind, IOMedia, MimeType, Result, Url};

/// One catalog: a name and the container its tables live under.
#[derive(Debug)]
pub struct Catalog {
    name: SmolStr,
    holder: Holder,
    description: Option<String>,
}

impl Catalog {
    /// The catalog `name` over `holder`, a container.
    pub fn new(name: impl Into<SmolStr>, holder: Holder) -> Self {
        Self {
            name: name.into(),
            holder,
            description: None,
        }
    }

    /// Return this catalog with a description, what `DBSCHEMA_CATALOGS`
    /// states.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The container.
    #[must_use]
    pub const fn holder(&self) -> &Holder {
        &self.holder
    }

    /// The description, when one was given.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The container's location, when it has one.
    #[must_use]
    pub fn url(&self) -> Option<&Url> {
        self.holder.url()
    }

    /// When the container was last modified, nanoseconds since the Unix
    /// epoch, when the store keeps that fact.
    #[must_use]
    pub fn modified(&self) -> Option<i64> {
        self.holder.mtime()
    }

    /// Every table of this catalog: the tabular children of the root, then
    /// the tabular children of each schema folder, in listing order.
    ///
    /// # Errors
    ///
    /// Returns the store's listing failure.
    pub fn tables(&self) -> Result<Vec<Table>> {
        let mut tables = Vec::new();
        let mut schemas = Vec::new();
        for child in self.holder.ls(false, false) {
            let child = child?;
            match Entry::under_root(&child) {
                Entry::Table => tables.push(Table::new(&self.name, None, child)),
                Entry::Schema => schemas.push(child),
                Entry::Other => {}
            }
        }
        for schema in schemas {
            let Some(schema_name) = schema.url().and_then(entry_name) else {
                continue;
            };
            for child in schema.ls(false, false) {
                let child = child?;
                if Entry::under_schema(&child) == Entry::Table {
                    tables.push(Table::new(&self.name, Some(schema_name.clone()), child));
                }
            }
        }
        Ok(tables)
    }

    /// Every schema of this catalog: the folders under the root that are not
    /// themselves tables, by name, in listing order.
    ///
    /// # Errors
    ///
    /// Returns the store's listing failure.
    pub fn schemas(&self) -> Result<Vec<SmolStr>> {
        let mut schemas = Vec::new();
        for child in self.holder.ls(false, false) {
            let child = child?;
            if Entry::under_root(&child) == Entry::Schema {
                if let Some(name) = child.url().and_then(entry_name) {
                    schemas.push(name);
                }
            }
        }
        Ok(schemas)
    }

    /// The table `name` under `schema`, or directly under the root.
    ///
    /// # Errors
    ///
    /// Returns the store's listing failure, [`Error::Absent`] when no table
    /// has the name, and [`Error::Conflict`] when two do - two leaves whose
    /// names differ only in their extension.
    pub fn table(&self, schema: Option<&str>, name: &str) -> Result<Table> {
        let mut found: Vec<Table> = self
            .tables()?
            .into_iter()
            .filter(|table| table.name() == name && table.schema() == schema)
            .collect();
        match found.len() {
            1 => Ok(found.remove(0)),
            0 => Err(Error::absent("table", self.path(schema, name))),
            _ => Err(Error::conflict("one table", "several leaves of that name", self.path(schema, name))),
        }
    }

    fn path(&self, schema: Option<&str>, name: &str) -> String {
        match schema {
            Some(schema) => format!("{}.{schema}.{name}", self.name),
            None => format!("{}.{name}", self.name),
        }
    }
}

/// What a child of a catalog folder is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    /// A leaf a record medium reads, or a folder read as the table beneath it.
    Table,
    /// A folder directly under the root, holding tables.
    Schema,
    /// A leaf no record medium reads.
    Other,
}

impl Entry {
    /// A child of the root: a table format's folder is a table, any other
    /// folder a schema, a leaf a table when its encoding is one this build
    /// reads.
    fn under_root(child: &Holder) -> Self {
        if child.is_container() {
            return if child.kind() == IOKind::Table || is_table_format(child) {
                Self::Table
            } else {
                Self::Schema
            };
        }
        Self::leaf(child)
    }

    /// A child of a schema: every folder is a table, read as the table
    /// beneath it, and a leaf is one when its encoding is one this build
    /// reads.
    fn under_schema(child: &Holder) -> Self {
        if child.is_container() {
            return Self::Table;
        }
        Self::leaf(child)
    }

    /// A leaf is a table when its media type is a record encoding this build
    /// implements: what the name declares, with no read.
    fn leaf(child: &Holder) -> Self {
        match RecordOptions::for_media_type(child.media_type()) {
            Ok(_) => Self::Table,
            Err(_) => Self::Other,
        }
    }
}

/// One table of a catalog: where it is, and what it is called.
#[derive(Debug)]
pub struct Table {
    catalog: SmolStr,
    schema: Option<SmolStr>,
    name: SmolStr,
    holder: Holder,
}

impl Table {
    fn new(catalog: &SmolStr, schema: Option<SmolStr>, holder: Holder) -> Self {
        let name = holder
            .url()
            .and_then(entry_name)
            .map_or_else(|| SmolStr::new_static("table"), |file| table_name(&file, &holder));
        Self {
            catalog: catalog.clone(),
            schema,
            name,
            holder,
        }
    }

    /// The catalog this table belongs to.
    #[must_use]
    pub fn catalog(&self) -> &str {
        &self.catalog
    }

    /// The schema this table is under, `None` directly under the root.
    #[must_use]
    pub fn schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    /// The table's name: its file name less the extensions its media type
    /// claims.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The leaf or folder holding the rows.
    #[must_use]
    pub const fn holder(&self) -> &Holder {
        &self.holder
    }

    /// The location, when the store has one.
    #[must_use]
    pub fn url(&self) -> Option<&Url> {
        self.holder.url()
    }

    /// When the rows were last modified, nanoseconds since the Unix epoch,
    /// when the store keeps that fact.
    #[must_use]
    pub fn modified(&self) -> Option<i64> {
        self.holder.mtime()
    }

    /// The `TABLE_TYPE` OLE DB states: `TABLE`.
    #[must_use]
    pub const fn table_type(&self) -> &'static str {
        "TABLE"
    }

    /// What holds the rows, as `DBSCHEMA_TABLES` describes it: a leaf's media
    /// type, a folder's kind - `table` for a table format's folder, whether
    /// the store says so or the layout does.
    #[must_use]
    pub fn description(&self) -> String {
        if self.holder.is_container() {
            if self.holder.kind() == IOKind::Table || is_table_format(&self.holder) {
                return IOKind::Table.as_str().to_owned();
            }
            self.holder.kind().as_str().to_owned()
        } else {
            self.holder.media_type().to_string()
        }
    }

    /// The record encoding the rows are held in.
    ///
    /// # Errors
    ///
    /// Returns an error when no encoding in this build covers the table.
    pub fn record_options(&self) -> Result<RecordOptions> {
        self.holder.record_options()
    }

    /// The table's row field: its columns.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding or schema failure.
    pub fn field(&self) -> Result<Field> {
        let options = self.record_options()?;
        self.holder.read_arrow_field(&options)
    }

    /// The dotted path the statement grammar names this table by.
    #[must_use]
    pub fn path(&self) -> String {
        match &self.schema {
            Some(schema) => format!("{}.{schema}.{}", self.catalog, self.name),
            None => format!("{}.{}", self.catalog, self.name),
        }
    }
}

/// Whether a folder is laid out as an Iceberg table: its `metadata/` holds
/// the `version-hint.text` a catalog-less table keeps, or a metadata
/// document. A store that answers [`IOKind::Table`] for such a folder is
/// never asked; a local or object store, which answers a directory, is asked
/// with one listing of `metadata/` and no read. The layout is the fact, so
/// a build without the `iceberg` feature lists the table too and refuses to
/// read it by name.
fn is_table_format(folder: &Holder) -> bool {
    let Ok(metadata) = folder.child_by_path("metadata") else {
        return false;
    };
    metadata.ls(false, false).any(|entry| {
        entry.ok().and_then(|entry| entry.url().and_then(entry_name)).is_some_and(|name| {
            name == "version-hint.text" || name.ends_with(".metadata.json")
        })
    })
}

/// The last segment of a location as the store spells it - the file or
/// folder name, its URI escapes decoded (`order%20book` is `order book`) -
/// which is what a catalog names its schemas and tables by.
fn entry_name(url: &Url) -> Option<SmolStr> {
    let name = url.file_name()?;
    Some(match crate::uri::percent_decode(name, "a catalog entry's name") {
        Ok(decoded) => SmolStr::new(decoded),
        Err(_) => SmolStr::new(name),
    })
}

/// A file name less the extensions a media type claims: `trades.arrows.gz`
/// is `trades`, `2024.report.parquet` is `2024.report`, and a folder's name
/// stands.
fn table_name(file: &str, holder: &Holder) -> SmolStr {
    if holder.is_container() {
        return SmolStr::new(file);
    }
    let mut stem = file;
    while let Some((head, extension)) = stem.rsplit_once('.') {
        if head.is_empty() || MimeType::from_extension(extension).is_err() {
            break;
        }
        stem = head;
    }
    if stem.is_empty() {
        return SmolStr::new(file);
    }
    SmolStr::new(stem)
}

impl std::fmt::Display for Table {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.path())
    }
}

pub(crate) fn no_catalog(name: &str) -> Error {
    Error::absent("catalog", format_smolstr!("{name}"))
}
