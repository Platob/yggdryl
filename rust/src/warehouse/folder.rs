//! A container read as a catalog: [`FolderCatalog`] and the
//! [`FolderNamespace`]s and tables it lists, and [`FolderLayout`], how a
//! table it lists holds its rows.
//!
//! A folder is listed when it is asked, so a table written a moment ago is
//! found on the next ask, and one listing is what an ask costs; nothing is
//! cached but the container handle. The rules, parametrized by how many
//! namespace `levels` sit under the catalog (one by default, so a path reads
//! `catalog.schema.table`):
//!
//! - a folder at a depth under the levels is a namespace; deeper, it is a
//!   table read as the rows beneath it;
//! - a folder laid out as a table format - the store answers
//!   [`IOKind::Table`], or one listing of its `metadata/` shows a version
//!   hint or a metadata document, never a read - is a table at any depth;
//! - a leaf is a table when a record medium of this build reads its media
//!   type, which its name states with no read; any other leaf is skipped,
//!   and so is a private entry;
//! - a table is named by its file name less every extension a media type
//!   claims, a folder by its name, URI escapes decoded exactly once and a
//!   ZIP member by its last segment;
//! - two entries of one name at one level are both listed, and asking for
//!   the name is a conflict.

use std::fmt;
use std::str::FromStr;

use smol_str::{SmolStr, format_smolstr};

use super::handle::{Handle, Site};
use super::object::{extended, path_text};
use super::{
    CatalogValue, IntoObjectPath, MediaTable, Namespace, NamespaceValue, Object, ObjectValue,
    Objects, Properties, Table,
};
use crate::holder::Holder;
use crate::media::RecordOptions;
use crate::{Error, IOBase, IOKind, MimeType, Result, Url};

/// How a table a folder catalog lists holds its rows, decided once when the
/// folder is listed and kept on the table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FolderLayout {
    /// A leaf a record medium reads.
    Leaf,
    /// A folder read as the table beneath it, leaf by leaf.
    Folder,
    /// A folder laid out as a table format - Iceberg - whether the store
    /// says [`IOKind::Table`] or its `metadata/` does.
    Format,
}

impl FolderLayout {
    /// The canonical lowercase name: `leaf`, `folder`, `format`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Leaf => "leaf",
            Self::Folder => "folder",
            Self::Format => "format",
        }
    }
}

impl fmt::Display for FolderLayout {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for FolderLayout {
    type Err = Error;

    /// Read a layout by its canonical name, ASCII case ignored and the
    /// surrounding blanks not part of it: the one reader of a layout a
    /// binding's argument names.
    fn from_str(value: &str) -> Result<Self> {
        [Self::Leaf, Self::Folder, Self::Format]
            .into_iter()
            .find(|layout| value.trim().eq_ignore_ascii_case(layout.as_str()))
            .ok_or_else(|| Error::Parse {
                target: "folder layout",
                position: 0,
                reason: format_smolstr!(
                    "expected a layout of `leaf`, `folder` or `format`, got {value:?}"
                ),
            })
    }
}

/// A container read as a catalog of namespaces and tables.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FolderCatalog {
    path: Vec<SmolStr>,
    handle: Handle,
    description: Option<String>,
    stated: Properties,
    levels: usize,
}

impl FolderCatalog {
    /// The default number of namespace levels: `catalog.schema.table`.
    pub const DEFAULT_LEVELS: usize = 1;

    /// The catalog `name` over the container `url` names, touching nothing.
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
            levels: Self::DEFAULT_LEVELS,
        })
    }

    /// The catalog `name` over a container handle already in hand.
    pub fn bound(name: impl Into<SmolStr>, holder: Holder) -> Self {
        let path = vec![name.into()];
        Self {
            handle: Handle::bound(holder, false, &path, Properties::new()),
            path,
            description: None,
            stated: Properties::new(),
            levels: Self::DEFAULT_LEVELS,
        }
    }

    /// Return this catalog with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this catalog with stated properties, which its container and
    /// every object under it open with.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self.handle.set_properties(self.stated.clone());
        self
    }

    /// Return this catalog reading `levels` namespace levels under it: a
    /// folder at a shallower depth is a namespace, a deeper one a table.
    #[must_use]
    pub const fn with_levels(mut self, levels: usize) -> Self {
        self.levels = levels;
        self
    }

    /// How many namespace levels sit under the catalog.
    #[must_use]
    pub const fn levels(&self) -> usize {
        self.levels
    }

    /// The container handle, opened on the first call.
    fn handle(&self) -> Result<&Holder> {
        self.handle.get()
    }
}

impl ObjectValue for FolderCatalog {
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
        Ok(self.stated.clone())
    }
}

impl NamespaceValue for FolderCatalog {
    fn children(&self) -> Objects {
        match self.handle() {
            Ok(handle) => list(handle, &self.path, self.levels, self.stated.clone()),
            Err(error) => Objects::failing(error),
        }
    }

    fn get(&self, name: &str) -> Result<Object> {
        get(self.handle()?, &self.path, self.levels, &self.stated, name)
    }
}

impl CatalogValue for FolderCatalog {
    fn namespace_levels(&self) -> Option<usize> {
        Some(self.levels)
    }
}

/// A folder under a folder catalog, read as a namespace of tables and, while
/// levels remain under it, of namespaces.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FolderNamespace {
    path: Vec<SmolStr>,
    handle: Handle,
    description: Option<String>,
    stated: Properties,
    inherited: Properties,
    levels: usize,
}

impl FolderNamespace {
    /// The namespace at `path` over the container `url` names, touching
    /// nothing, with no namespace level under it.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the path has fewer than two
    /// parts, and the identifier's own refusal when it names no location.
    pub fn new(path: impl IntoObjectPath, url: impl Into<crate::Uri>) -> Result<Self> {
        let path = namespace_path(path)?;
        let url = url.into().locator()?;
        Ok(Self {
            handle: Handle::at(Site::Url(url), false, &path, Properties::new()),
            path,
            description: None,
            stated: Properties::new(),
            inherited: Properties::new(),
            levels: 0,
        })
    }

    /// The namespace at `path` over a container handle already in hand.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] when the path has fewer than two
    /// parts.
    pub fn bound(path: impl IntoObjectPath, holder: Holder) -> Result<Self> {
        let path = namespace_path(path)?;
        Ok(Self {
            handle: Handle::bound(holder, false, &path, Properties::new()),
            path,
            description: None,
            stated: Properties::new(),
            inherited: Properties::new(),
            levels: 0,
        })
    }

    /// Return this namespace with a description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Return this namespace with stated properties.
    #[must_use]
    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.stated = properties;
        self.handle.set_properties(self.effective());
        self
    }

    /// Return this namespace with `levels` namespace levels under it.
    #[must_use]
    pub const fn with_levels(mut self, levels: usize) -> Self {
        self.levels = levels;
        self
    }

    /// How many namespace levels sit under this namespace.
    #[must_use]
    pub const fn levels(&self) -> usize {
        self.levels
    }

    fn effective(&self) -> Properties {
        self.stated.inherit(&self.inherited)
    }

    fn handle(&self) -> Result<&Holder> {
        self.handle.get()
    }

    pub(crate) fn inheriting(mut self, parent: &Properties) -> Self {
        self.inherited = parent.clone();
        self.handle.set_properties(self.effective());
        self
    }
}

/// A path of at least two parts, a catalog's name first.
fn namespace_path(path: impl IntoObjectPath) -> Result<Vec<SmolStr>> {
    let path = path.into_object_path()?;
    if path.len() < 2 {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.{}", path_text(&path)),
            reason: format_smolstr!(
                "expected a namespace path of at least two parts, a catalog's name first, got \
                 {} part(s)",
                path.len()
            ),
        });
    }
    Ok(path)
}

impl ObjectValue for FolderNamespace {
    fn name(&self) -> &str {
        self.path.last().map_or("", SmolStr::as_str)
    }

    fn path(&self) -> &[SmolStr] {
        &self.path
    }

    fn kind(&self) -> IOKind {
        IOKind::Namespace
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
        Ok(self.effective())
    }
}

impl NamespaceValue for FolderNamespace {
    fn children(&self) -> Objects {
        match self.handle() {
            Ok(handle) => list(handle, &self.path, self.levels, self.effective()),
            Err(error) => Objects::failing(error),
        }
    }

    fn get(&self, name: &str) -> Result<Object> {
        get(
            self.handle()?,
            &self.path,
            self.levels,
            &self.effective(),
            name,
        )
    }
}

/// What a child of a listed folder is.
enum Entry {
    /// A folder with namespace levels left under it.
    Namespace,
    /// A leaf a record medium reads, or a folder read as the table beneath
    /// it, or a table format's folder.
    Table(FolderLayout),
    /// A leaf no record medium reads.
    Other,
}

impl Entry {
    /// Classify `child` - a `container` or a leaf, asked once - under a
    /// folder with `levels` namespace levels left.
    fn of(child: &Holder, container: bool, levels: usize) -> Self {
        if container {
            if is_format(child) {
                return Self::Table(FolderLayout::Format);
            }
            return if levels > 0 {
                Self::Namespace
            } else {
                Self::Table(FolderLayout::Folder)
            };
        }
        match RecordOptions::for_media_type(child.media_type()) {
            Ok(_) => Self::Table(FolderLayout::Leaf),
            Err(_) => Self::Other,
        }
    }
}

/// The object an entry is, at `path`, carrying `effective`.
fn object_of(
    entry: Holder,
    container: bool,
    path: Vec<SmolStr>,
    levels: usize,
    effective: &Properties,
) -> Option<Object> {
    match Entry::of(&entry, container, levels) {
        Entry::Namespace => Some(Object::Namespace(Namespace::Folder(Box::new(
            FolderNamespace {
                handle: Handle::bound(entry, false, &path, effective.clone()),
                path,
                description: None,
                stated: Properties::new(),
                inherited: effective.clone(),
                levels: levels - 1,
            },
        )))),
        #[cfg(feature = "iceberg")]
        Entry::Table(FolderLayout::Format) => {
            let root = Handle::bound(entry, false, &path, Properties::new());
            Some(Object::Table(Table::Iceberg(Box::new(
                crate::iceberg::IcebergTable::at(path, root).inheriting(effective),
            ))))
        }
        Entry::Table(layout) => Some(Object::Table(Table::Media(Box::new(
            MediaTable::listed(path, entry, layout).inheriting(effective),
        )))),
        Entry::Other => None,
    }
}

/// The children of `folder`, lazily: one listing, and one listing of
/// `metadata/` per folder entry as its turn comes.
fn list(folder: &Holder, path: &[SmolStr], levels: usize, effective: Properties) -> Objects {
    let path = path.to_vec();
    Objects::new(folder.ls(false, false).filter_map(move |entry| {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return Some(Err(error)),
        };
        let file = entry_name(&entry)?;
        if file.starts_with('.') {
            return None;
        }
        // Asked once per entry: on a store a listed leaf's role is a request.
        let container = entry.is_container();
        let name = table_name(&file, container);
        object_of(entry, container, extended(&path, &name), levels, &effective).map(Ok)
    }))
}

/// The child of `folder` called `name`: one listing of the level.
fn get(
    folder: &Holder,
    path: &[SmolStr],
    levels: usize,
    effective: &Properties,
    name: &str,
) -> Result<Object> {
    let child = extended(path, name);
    if name.is_empty() || name.starts_with('.') {
        return Err(Error::absent("table", path_text(&child)));
    }
    let mut found = Vec::new();
    for entry in folder.ls(false, false) {
        let entry = entry?;
        let Some(file) = entry_name(&entry) else {
            continue;
        };
        // An entry whose name matches neither as a folder's nor as a leaf's
        // is passed over without asking the store what it is.
        if file != name && without_extensions(&file) != name {
            continue;
        }
        let container = entry.is_container();
        if table_name(&file, container) != name {
            continue;
        }
        if let Some(object) = object_of(entry, container, child.clone(), levels, effective) {
            found.push(object);
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(Error::absent("table", path_text(&child))),
        _ => Err(Error::conflict(
            "table",
            "several leaves of that name",
            path_text(&child),
        )),
    }
}

/// Whether a folder is a table format's: the store says so, or its layout
/// does.
fn is_format(folder: &Holder) -> bool {
    folder.kind() == IOKind::Table || is_table_format(folder)
}

/// Whether a folder is laid out as an Iceberg table, a listing failure read
/// as no: a folder catalog lists what it can read.
fn is_table_format(folder: &Holder) -> bool {
    table_layout(folder).unwrap_or(false)
}

/// Whether a folder is laid out as an Iceberg table: its `metadata/` holds
/// the `version-hint.text` a catalog-less table keeps, or a metadata
/// document - one listing of `metadata/` and no read. The layout is the
/// fact, so a build without the `iceberg` feature lists the table too and
/// refuses to read it by name. A `metadata/` that is not there is no
/// layout; any other listing failure is the store's own.
pub(crate) fn table_layout(folder: &Holder) -> Result<bool> {
    let metadata = folder.child_by_path("metadata")?;
    for entry in metadata.ls(false, false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if error.is_absent() => return Ok(false),
            Err(error) => return Err(error),
        };
        if entry_name(&entry)
            .is_some_and(|name| name == "version-hint.text" || name.ends_with(".metadata.json"))
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The name of an entry as its store spells it - the last segment of a
/// member's name inside an archive, never the archive's file name, else the
/// location's file or folder name with its URI escapes decoded exactly once
/// (`order%20book` is `order book`).
pub(crate) fn entry_name(holder: &Holder) -> Option<SmolStr> {
    if let Some(member) = crate::zip::member_name(holder) {
        let member = member.trim_end_matches('/');
        return Some(SmolStr::new(member.rsplit('/').next().unwrap_or(member)));
    }
    let name = holder.url()?.file_name()?;
    Some(
        match crate::uri::percent_decode(name, "a catalog entry's name") {
            Ok(decoded) => SmolStr::new(decoded),
            Err(_) => SmolStr::new(name),
        },
    )
}

/// A file name less the extensions a media type claims: `trades.arrows.gz`
/// is `trades`, `2024.report.parquet` is `2024.report`, and a folder's name
/// stands.
fn table_name(file: &str, container: bool) -> SmolStr {
    if container {
        return SmolStr::new(file);
    }
    SmolStr::new(without_extensions(file))
}

/// A leaf's name less every extension a media type claims, never emptied.
fn without_extensions(file: &str) -> &str {
    let mut stem = file;
    while let Some((head, extension)) = stem.rsplit_once('.') {
        if head.is_empty() || MimeType::from_extension(extension).is_err() {
            break;
        }
        stem = head;
    }
    if stem.is_empty() { file } else { stem }
}
