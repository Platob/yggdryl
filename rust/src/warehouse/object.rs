//! What every object a warehouse path reaches answers, and the one enum that
//! says which implementation answers it.
//!
//! An object is a description - its path, what it states, and where its
//! storage or its client is - never a view borrowed from its parent, so any
//! object sits in an enum, is registered, and crosses a binding boundary. A
//! resolved handle is kept beside the description and a clone starts
//! unresolved, as a [`Uri`](crate::Uri) does.

use std::fmt;

use smol_str::{SmolStr, format_smolstr};

use super::{Catalog, Namespace, Properties, Table};
use crate::expression::{Location, Target};
use crate::holder::Holder;
use crate::{Error, IOKind, Result, Url};

/// What every object a warehouse path reaches answers.
pub trait ObjectValue: fmt::Debug + Send + Sync {
    /// The last part of its path.
    fn name(&self) -> &str;

    /// Its parts, from its catalog's name down to its own.
    fn path(&self) -> &[SmolStr];

    /// [`IOKind::Catalog`], [`IOKind::Namespace`] or [`IOKind::Table`].
    fn kind(&self) -> IOKind;

    /// What its store says it is, when it says anything.
    fn description(&self) -> Option<&str> {
        None
    }

    /// Where its storage is, when it has a location.
    fn url(&self) -> Option<&Url> {
        None
    }

    /// When it last changed, in UTC nanoseconds since the epoch, when the
    /// store keeps that fact.
    fn modified(&self) -> Option<i64> {
        None
    }

    /// Its effective properties: its parent's, then what its store keeps for
    /// it, then what was stated for it, a later entry replacing an earlier
    /// one by name.
    ///
    /// # Errors
    ///
    /// Returns the store's failure to read what it keeps.
    fn properties(&self) -> Result<Properties>;

    /// Persist updates and removals of what its store keeps for it.
    ///
    /// Stated properties - credentials included - are never written into a
    /// document. The default refuses, naming the implementation.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] where the object keeps nothing, else the
    /// store's own refusal.
    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        let _ = (updates, removes);
        Err(Error::unsupported(
            "updating the properties it keeps",
            implementation_name(self),
        ))
    }
}

/// The last segment of an implementation's type name: `MemoryCatalog`.
pub(crate) fn implementation_name<T: ?Sized>(value: &T) -> &'static str {
    let name = std::any::type_name_of_val(value);
    name.rsplit("::").next().unwrap_or(name)
}

/// Anything a warehouse path reaches: a catalog, a namespace or a table.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Object {
    /// The first namespace layer, registered by name.
    Catalog(Catalog),
    /// A container of namespaces and tables.
    Namespace(Namespace),
    /// A table: rows any record read and write reaches.
    Table(Table),
}

impl Object {
    /// Borrow the implementation through the contract every object answers.
    pub fn as_object(&self) -> &dyn ObjectValue {
        match self {
            Self::Catalog(catalog) => catalog,
            Self::Namespace(namespace) => namespace,
            Self::Table(table) => table,
        }
    }

    /// The catalog this is, when it is one.
    #[must_use]
    pub fn as_catalog(&self) -> Option<&Catalog> {
        match self {
            Self::Catalog(catalog) => Some(catalog),
            _ => None,
        }
    }

    /// The namespace this is, when it is one.
    #[must_use]
    pub fn as_namespace(&self) -> Option<&Namespace> {
        match self {
            Self::Namespace(namespace) => Some(namespace),
            _ => None,
        }
    }

    /// The table this is, when it is one.
    #[must_use]
    pub fn as_table(&self) -> Option<&Table> {
        match self {
            Self::Table(table) => Some(table),
            _ => None,
        }
    }

    /// This object as the table it is, or the absence of one at its path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when a catalog or a
    /// namespace sits there: asking for a table where a namespace sits is
    /// absence.
    pub fn into_table(self) -> Result<Table> {
        match self {
            Self::Table(table) => Ok(table),
            other => Err(Error::absent("table", path_text(other.path()))),
        }
    }

    /// This object as the namespace it is, or the absence of one at its path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when a catalog or a table
    /// sits there.
    pub fn into_namespace(self) -> Result<Namespace> {
        match self {
            Self::Namespace(namespace) => Ok(namespace),
            other => Err(Error::absent("namespace", path_text(other.path()))),
        }
    }

    /// This object as the handle it is.
    #[must_use]
    pub fn into_holder(self) -> Holder {
        match self {
            Self::Catalog(catalog) => Holder::Catalog(Box::new(catalog)),
            Self::Namespace(namespace) => Holder::Namespace(Box::new(namespace)),
            Self::Table(table) => Holder::Table(Box::new(table)),
        }
    }

    /// The implementation's own name: `MemoryCatalog`, `MediaTable`.
    pub(crate) const fn implementation_name(&self) -> &'static str {
        match self {
            Self::Catalog(catalog) => catalog.implementation_name(),
            Self::Namespace(namespace) => namespace.implementation_name(),
            Self::Table(table) => table.implementation_name(),
        }
    }

    /// The object with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(self, parent: &Properties) -> Self {
        match self {
            Self::Catalog(catalog) => Self::Catalog(catalog),
            Self::Namespace(namespace) => Self::Namespace(namespace.inheriting(parent)),
            Self::Table(table) => Self::Table(table.inheriting(parent)),
        }
    }
}

impl ObjectValue for Object {
    fn name(&self) -> &str {
        self.as_object().name()
    }

    fn path(&self) -> &[SmolStr] {
        self.as_object().path()
    }

    fn kind(&self) -> IOKind {
        self.as_object().kind()
    }

    fn description(&self) -> Option<&str> {
        self.as_object().description()
    }

    fn url(&self) -> Option<&Url> {
        self.as_object().url()
    }

    fn modified(&self) -> Option<i64> {
        self.as_object().modified()
    }

    fn properties(&self) -> Result<Properties> {
        self.as_object().properties()
    }

    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        self.as_object().update_properties(updates, removes)
    }
}

impl fmt::Display for Object {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_path(formatter, self.path())
    }
}

impl From<Catalog> for Object {
    fn from(catalog: Catalog) -> Self {
        Self::Catalog(catalog)
    }
}

impl From<Namespace> for Object {
    fn from(namespace: Namespace) -> Self {
        Self::Namespace(namespace)
    }
}

impl From<Table> for Object {
    fn from(table: Table) -> Self {
        Self::Table(table)
    }
}

impl From<super::MemoryCatalog> for Object {
    fn from(catalog: super::MemoryCatalog) -> Self {
        Self::Catalog(Catalog::from(catalog))
    }
}

impl From<super::FolderCatalog> for Object {
    fn from(catalog: super::FolderCatalog) -> Self {
        Self::Catalog(Catalog::from(catalog))
    }
}

impl From<super::MemoryNamespace> for Object {
    fn from(namespace: super::MemoryNamespace) -> Self {
        Self::Namespace(Namespace::from(namespace))
    }
}

impl From<super::FolderNamespace> for Object {
    fn from(namespace: super::FolderNamespace) -> Self {
        Self::Namespace(Namespace::from(namespace))
    }
}

impl From<super::MediaTable> for Object {
    fn from(table: super::MediaTable) -> Self {
        Self::Table(Table::from(table))
    }
}

/// Write a path as the plan grammar spells it: parts joined by `.`, each
/// quoted only where the grammar needs it.
pub(crate) fn write_path(formatter: &mut fmt::Formatter<'_>, parts: &[SmolStr]) -> fmt::Result {
    for (index, part) in parts.iter().enumerate() {
        if index != 0 {
            formatter.write_str(".")?;
        }
        crate::expression::write_identifier(formatter, part)?;
    }
    Ok(())
}

/// A path rendered as the plan grammar spells it, which is how every error
/// names one.
pub(crate) fn path_text(parts: &[SmolStr]) -> String {
    struct Rendered<'a>(&'a [SmolStr]);

    impl fmt::Display for Rendered<'_> {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write_path(formatter, self.0)
        }
    }

    Rendered(parts).to_string()
}

/// The path with one more part below it.
pub(crate) fn extended(parts: &[SmolStr], name: &str) -> Vec<SmolStr> {
    let mut path = Vec::with_capacity(parts.len() + 1);
    path.extend_from_slice(parts);
    path.push(SmolStr::new(name));
    path
}

/// The one intake for a path into a warehouse.
///
/// Dotted text is read through the plan's location grammar, so quoting is
/// the grammar's - `lake."eu west".fills`, backticks or `[...]` - the empty
/// text is the root, and parts arrive as they are. A URL or a `with (...)`
/// clause where a path is expected is refused by name at `$.path`, and
/// nothing past this intake splits a path on `.`.
pub trait IntoObjectPath {
    /// The parts, from the catalog's name down.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.path` when the text is not a
    /// path of parts.
    fn into_object_path(self) -> Result<Vec<SmolStr>>;
}

impl IntoObjectPath for &str {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        if self.trim().is_empty() {
            return Ok(Vec::new());
        }
        let target = Target::parse(self).map_err(|error| Error::InvalidRecord {
            path: SmolStr::new_static("$.path"),
            reason: format_smolstr!("expected a path of parts, got {self:?}: {error}"),
        })?;
        if !target.properties().is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: format_smolstr!(
                    "expected a path of parts, got a `with (...)` clause on `{}`",
                    target.location()
                ),
            });
        }
        target.location().into_object_path()
    }
}

impl IntoObjectPath for String {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        self.as_str().into_object_path()
    }
}

impl IntoObjectPath for &String {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        self.as_str().into_object_path()
    }
}

impl IntoObjectPath for &Location {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        match self {
            Location::Parts(parts) => Ok(parts.clone()),
            Location::Url(url) => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: format_smolstr!("expected a path of parts, got the URL {url}"),
            }),
        }
    }
}

impl IntoObjectPath for Location {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        match self {
            Location::Parts(parts) => Ok(parts),
            Location::Url(url) => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: format_smolstr!("expected a path of parts, got the URL {url}"),
            }),
        }
    }
}

impl IntoObjectPath for Vec<SmolStr> {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        Ok(self)
    }
}

impl IntoObjectPath for &[SmolStr] {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        Ok(self.to_vec())
    }
}

impl IntoObjectPath for &Vec<SmolStr> {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        Ok(self.clone())
    }
}

impl IntoObjectPath for &[&str] {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        Ok(self.iter().map(SmolStr::new).collect())
    }
}

impl IntoObjectPath for Vec<&str> {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        self.as_slice().into_object_path()
    }
}

impl<const N: usize> IntoObjectPath for [&str; N] {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        self.as_slice().into_object_path()
    }
}

impl<const N: usize> IntoObjectPath for &[&str; N] {
    fn into_object_path(self) -> Result<Vec<SmolStr>> {
        self.as_slice().into_object_path()
    }
}

/// The children of a namespace, yielded one at a time.
///
/// The walk runs as the iterator is drained, so a caller that takes three
/// children from a store of a hundred thousand pays for three. The item is a
/// [`Result`], so a listing fails *at* the failing child, naming it, and the
/// iterator is fused afterwards. Order is the store's own.
pub struct Objects {
    /// The walk still running. `None` once the listing is spent.
    entries: Option<Box<dyn Iterator<Item = Result<Object>> + Send + Sync>>,
}

impl Objects {
    /// A listing of nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    /// Wrap a walk that is already lazy.
    pub fn new(entries: impl Iterator<Item = Result<Object>> + Send + Sync + 'static) -> Self {
        Self {
            entries: Some(Box::new(entries)),
        }
    }

    /// A listing that reports one failure and then ends.
    #[must_use]
    pub fn failing(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }

    /// The children as handles, lazily, descending every namespace when
    /// `recursive`.
    #[must_use]
    pub fn into_listing(self, recursive: bool) -> crate::Listing {
        let listing = crate::Listing::new(self.map(|entry| entry.map(Object::into_holder)));
        if recursive {
            listing.descending(false)
        } else {
            listing
        }
    }
}

impl Iterator for Objects {
    type Item = Result<Object>;

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

impl std::iter::FusedIterator for Objects {}

impl fmt::Debug for Objects {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Objects")
            .field("spent", &self.entries.is_none())
            .finish()
    }
}

impl Default for Objects {
    fn default() -> Self {
        Self::empty()
    }
}

impl FromIterator<Result<Object>> for Objects {
    fn from_iter<I: IntoIterator<Item = Result<Object>>>(entries: I) -> Self {
        Self::new(entries.into_iter().collect::<Vec<_>>().into_iter())
    }
}
