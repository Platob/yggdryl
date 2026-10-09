//! A namespace: a container of namespaces and tables, the enum that says
//! which implementation lists it, and the collection views every level of
//! the hierarchy answers with.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};

use smol_str::{SmolStr, format_smolstr};

use super::object::path_text;
use super::{FolderNamespace, MemoryNamespace, Object, ObjectValue, Objects, Properties, Table};
use crate::arrow::BatchReader;
use crate::media::RecordOptions;
use crate::{Error, Field, IOBase, IOKind, IOMedia, Result, Url};

/// A container of namespaces and tables.
pub trait NamespaceValue: ObjectValue {
    /// Its children, lazily, in the store's order, each carrying its
    /// effective properties.
    fn children(&self) -> Objects;

    /// The child called `name`, one level down.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing is called
    /// `name`, [`Error::Conflict`] when several entries are, and the store's
    /// own listing failure.
    fn get(&self, name: &str) -> Result<Object>;

    /// The child called `name` as the step of a path that goes on below
    /// it: `get` by default, and for a store whose child proves itself -
    /// an Amazon S3 Tables namespace, which the table under it names by its
    /// own request - a description costing no request, the absence
    /// surfacing at the step below.
    ///
    /// # Errors
    ///
    /// As `get`, where the store is asked.
    fn descend(&self, name: &str) -> Result<Object> {
        self.get(name)
    }

    /// Create the namespace `name` under it. The default refuses by name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] where the namespace creates nothing,
    /// [`Error::Conflict`] when the name is taken, else the store's failure.
    fn create_namespace(&self, name: &str, properties: &Properties) -> Result<super::Namespace> {
        let _ = (name, properties);
        Err(Error::unsupported(
            "creating a namespace",
            super::object::implementation_name(self),
        ))
    }

    /// Create the table `name` under it, `field` its row schema. The default
    /// refuses by name.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] where the namespace creates nothing,
    /// [`Error::Conflict`] when the name is taken, else the store's failure.
    fn create_table(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        let _ = (name, field, properties);
        Err(Error::unsupported(
            "creating a table",
            super::object::implementation_name(self),
        ))
    }
}

/// A namespace implemented outside the core's memory and folder
/// namespaces, as [`Namespace::Registered`] holds it: the namespace
/// contract, the name a refusal calls the implementation by, what a trait
/// object owes the derive-heavy enum holding it - a copy, equality, a hash
/// and the downcast [`Namespace::downcast_ref`] reads - and the two
/// consuming updates every namespace answers.
pub trait RegisteredNamespace: NamespaceValue + Send + Sync + fmt::Debug {
    /// The implementation's own name, as a refusal names it:
    /// `IcebergNamespace`.
    fn implementation_name(&self) -> &'static str;

    /// A boxed copy.
    fn clone_box(&self) -> Box<dyn RegisteredNamespace>;

    /// Equality across the trait object: the same implementation holding an
    /// equal namespace.
    fn dyn_eq(&self, other: &dyn RegisteredNamespace) -> bool;

    /// The implementation's own hash, into any hasher.
    fn dyn_hash(&self, state: &mut dyn Hasher);

    /// The namespace as `Any`, for [`Namespace::downcast_ref`].
    fn as_any(&self) -> &dyn Any;

    /// The namespace as `Any`, mutably, for [`Namespace::downcast_mut`].
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// The namespace with stated properties, which its storage and every
    /// object under it open with.
    fn with_properties(self: Box<Self>, properties: Properties) -> Box<dyn RegisteredNamespace>;

    /// The namespace with its parent's effective properties pushed into it.
    fn inheriting(self: Box<Self>, parent: &Properties) -> Box<dyn RegisteredNamespace>;
}

impl Clone for Box<dyn RegisteredNamespace> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for dyn RegisteredNamespace {
    fn eq(&self, other: &Self) -> bool {
        self.dyn_eq(other)
    }
}

impl Eq for dyn RegisteredNamespace {}

impl Hash for dyn RegisteredNamespace {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dyn_hash(state);
    }
}

/// The implementation a namespace is listed by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Namespace {
    /// Registered objects, in order, with no storage.
    Memory(MemoryNamespace),
    /// A folder read as namespaces and tables.
    ///
    /// Boxed: a located namespace carries its location and two property
    /// bags, several times the size of a memory one.
    Folder(Box<FolderNamespace>),
    /// A namespace an implementation outside the core answers - a folder of
    /// an Iceberg warehouse, a namespace of an Amazon S3 Tables table
    /// bucket - held through the contract every such namespace answers.
    Registered(Box<dyn RegisteredNamespace>),
}

impl Namespace {
    /// Borrow the implementation through the contract every namespace
    /// answers.
    pub fn as_namespace(&self) -> &dyn NamespaceValue {
        match self {
            Self::Memory(namespace) => namespace,
            Self::Folder(namespace) => &**namespace,
            Self::Registered(namespace) => &**namespace,
        }
    }

    /// The implementation as the type it is, when it is a `T`: a
    /// [`MemoryNamespace`], a [`FolderNamespace`], or a registered
    /// namespace's own type.
    #[must_use]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any: &dyn Any = match self {
            Self::Memory(namespace) => namespace,
            Self::Folder(namespace) => &**namespace,
            Self::Registered(namespace) => namespace.as_any(),
        };
        any.downcast_ref()
    }

    /// The implementation as the type it is, mutably, when it is a `T`.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        let any: &mut dyn Any = match self {
            Self::Memory(namespace) => namespace,
            Self::Folder(namespace) => &mut **namespace,
            Self::Registered(namespace) => namespace.as_any_mut(),
        };
        any.downcast_mut()
    }

    /// The object a path of parts below this namespace names, descending
    /// through [`NamespaceValue::get`] one part at a time.
    ///
    /// The empty path is this namespace itself.
    ///
    /// # Errors
    ///
    /// Returns what the level that fails answers: absence naming the path to
    /// it, a conflict, or the store's own failure; a table met before the
    /// last part is absence of the namespace asked for below it.
    pub fn resolve(&self, path: impl super::IntoObjectPath) -> Result<Object> {
        resolve_under(self, path.into_object_path()?)
    }

    /// The namespaces one level down, as a lazy view.
    #[must_use]
    pub fn namespaces(&self) -> Namespaces<'_> {
        Namespaces::of(self)
    }

    /// The tables one level down, as a lazy view.
    #[must_use]
    pub fn tables(&self) -> Tables<'_> {
        Tables::of(self)
    }

    /// The implementation's own name: `MemoryNamespace`, `FolderNamespace`,
    /// or what a registered namespace calls itself.
    pub(crate) fn implementation_name(&self) -> &'static str {
        match self {
            Self::Memory(_) => "MemoryNamespace",
            Self::Folder(_) => "FolderNamespace",
            Self::Registered(namespace) => namespace.implementation_name(),
        }
    }

    /// The namespace with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(self, parent: &Properties) -> Self {
        match self {
            Self::Memory(namespace) => Self::Memory(namespace.inheriting(parent)),
            Self::Folder(namespace) => Self::Folder(Box::new(namespace.inheriting(parent))),
            Self::Registered(namespace) => Self::Registered(namespace.inheriting(parent)),
        }
    }
}

/// Descend from `parent` through `path`: `descend` for every part another
/// follows and `get` for the last, so what the path names is proven where it
/// stands and a level above it is asked for only where its store must.
pub(crate) fn resolve_under(parent: &dyn NamespaceValue, path: Vec<SmolStr>) -> Result<Object> {
    let mut parts = path.into_iter().peekable();
    let Some(first) = parts.next() else {
        return Err(Error::InvalidRecord {
            path: format_smolstr!("$.{}", path_text(parent.path())),
            reason: SmolStr::new_static("expected at least one part below the namespace, got none"),
        });
    };
    let mut current = if parts.peek().is_some() {
        parent.descend(&first)?
    } else {
        parent.get(&first)?
    };
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        current = match current {
            Object::Catalog(catalog) if last => catalog.get(&part)?,
            Object::Catalog(catalog) => catalog.descend(&part)?,
            Object::Namespace(namespace) if last => namespace.get(&part)?,
            Object::Namespace(namespace) => namespace.descend(&part)?,
            Object::Table(table) => {
                return Err(Error::absent("namespace", path_text(table.path())));
            }
        };
    }
    Ok(current)
}

impl ObjectValue for Namespace {
    fn name(&self) -> &str {
        self.as_namespace().name()
    }

    fn path(&self) -> &[SmolStr] {
        self.as_namespace().path()
    }

    fn kind(&self) -> IOKind {
        self.as_namespace().kind()
    }

    fn description(&self) -> Option<&str> {
        self.as_namespace().description()
    }

    fn url(&self) -> Option<&Url> {
        self.as_namespace().url()
    }

    fn modified(&self) -> Option<i64> {
        self.as_namespace().modified()
    }

    fn properties(&self) -> Result<Properties> {
        self.as_namespace().properties()
    }

    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        self.as_namespace().update_properties(updates, removes)
    }
}

impl NamespaceValue for Namespace {
    fn children(&self) -> Objects {
        self.as_namespace().children()
    }

    fn get(&self, name: &str) -> Result<Object> {
        self.as_namespace().get(name)
    }

    fn descend(&self, name: &str) -> Result<Object> {
        self.as_namespace().descend(name)
    }

    fn create_namespace(&self, name: &str, properties: &Properties) -> Result<Self> {
        self.as_namespace().create_namespace(name, properties)
    }

    fn create_table(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        self.as_namespace().create_table(name, field, properties)
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::object::write_path(formatter, self.path())
    }
}

impl From<MemoryNamespace> for Namespace {
    fn from(namespace: MemoryNamespace) -> Self {
        Self::Memory(namespace)
    }
}

impl From<FolderNamespace> for Namespace {
    fn from(namespace: FolderNamespace) -> Self {
        Self::Folder(Box::new(namespace))
    }
}

/// The byte and record surface of a container object: a catalog or a
/// namespace holds no bytes and is no table, so every byte verb is refused
/// as [`Error::NotAtomic`] and every record verb by name, while the
/// hierarchy verbs answer the children as handles.
macro_rules! container_object_io {
    ($type:ty) => {
        impl IOBase for $type {
            fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
                Err(crate::iobase::not_atomic(self, "pread"))
            }

            fn pwrite(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize> {
                Err(crate::iobase::not_atomic(self, "pwrite"))
            }

            fn pstream_bytes(
                &self,
                _position: u64,
                _batch_size: usize,
            ) -> Result<crate::ByteStream<'_>> {
                Err(crate::iobase::not_atomic(self, "pstream_bytes"))
            }

            fn read_all_bytes(&self) -> Result<Vec<u8>> {
                Err(crate::iobase::not_atomic(self, "read_all_bytes"))
            }

            fn read_range_bytes(&self, _offset: u64, _length: usize) -> Result<Vec<u8>> {
                Err(crate::iobase::not_atomic(self, "read_range_bytes"))
            }

            fn read_digest(&self, _algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
                Err(crate::iobase::not_atomic(self, "read_digest"))
            }

            fn read_range_digest(
                &self,
                _offset: u64,
                _length: usize,
                _algorithm: crate::DigestAlgorithm,
            ) -> Result<crate::Digest> {
                Err(crate::iobase::not_atomic(self, "read_range_digest"))
            }

            fn write_all_bytes(&mut self, _bytes: &[u8]) -> Result<()> {
                Err(crate::iobase::not_atomic(self, "write_all_bytes"))
            }

            fn create_bytes(&mut self, _bytes: &[u8]) -> Result<()> {
                Err(crate::iobase::not_atomic(self, "create_bytes"))
            }

            fn append_bytes(&mut self, _bytes: &[u8]) -> Result<u64> {
                Err(crate::iobase::not_atomic(self, "append_bytes"))
            }

            fn size(&self) -> u64 {
                0
            }

            fn capacity(&self) -> u64 {
                0
            }

            fn reserve(&mut self, _capacity: u64) -> Result<()> {
                Err(crate::iobase::not_atomic(self, "reserve"))
            }

            fn truncate(&mut self, _size: u64) -> Result<()> {
                Err(crate::iobase::not_atomic(self, "truncate"))
            }

            fn uri(&self) -> Option<&crate::Uri> {
                ObjectValue::url(self).map(AsRef::as_ref)
            }

            fn url(&self) -> Option<&Url> {
                ObjectValue::url(self)
            }

            fn mtime(&self) -> Option<i64> {
                ObjectValue::modified(self)
            }

            fn media_type(&self) -> &crate::MediaType {
                &super::namespace::CONTAINER_MEDIA_TYPE
            }

            fn set_media_type(&mut self, _media_type: crate::MediaType) {
                // A namespace is a namespace; it has no content type to declare.
            }

            fn flush(&mut self) -> Result<()> {
                Ok(())
            }

            fn open(&mut self) -> Result<()> {
                Ok(())
            }

            fn opened(&self) -> bool {
                false
            }

            fn close(&mut self) -> Result<()> {
                Ok(())
            }

            fn clear(&mut self) -> Result<()> {
                Err(Error::unsupported(
                    "clearing a container object",
                    ObjectValue::kind(self),
                ))
            }

            fn remove(&mut self, _recursive: bool) -> Result<()> {
                Err(Error::unsupported(
                    "removing a container object",
                    ObjectValue::kind(self),
                ))
            }

            fn child_by_path(&self, path: &str) -> Result<crate::holder::Holder> {
                let parts: Vec<SmolStr> = path
                    .split('/')
                    .filter(|part| !part.is_empty() && *part != ".")
                    .map(SmolStr::new)
                    .collect();
                if parts.is_empty() {
                    return Ok(Object::from(self.clone()).into_holder());
                }
                Ok(super::namespace::resolve_under(self, parts)?.into_holder())
            }

            fn ls(&self, recursive: bool, _include_private: bool) -> crate::Listing {
                self.children().into_listing(recursive)
            }

            fn kind(&self) -> IOKind {
                ObjectValue::kind(self)
            }

            fn is_container(&self) -> bool {
                true
            }

            fn is_atomic(&self) -> bool {
                false
            }

            fn is_tabular(&self) -> bool {
                false
            }
        }

        impl IOMedia for $type {
            fn as_io_base(&self) -> &dyn IOBase {
                self
            }

            fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
                self
            }

            fn row_size(&self) -> Result<u64> {
                Err(super::namespace::no_table(self))
            }

            fn column_size(&self) -> Result<usize> {
                Err(super::namespace::no_table(self))
            }

            fn record_options(&self) -> Result<RecordOptions> {
                Err(super::namespace::no_table(self))
            }

            fn read_origin_field(&self) -> Result<Option<Field>> {
                Err(super::namespace::no_table(self))
            }

            fn read_arrow_field(&self, _options: &RecordOptions) -> Result<Field> {
                Err(super::namespace::no_table(self))
            }

            fn read_serie(&self, _options: Option<&RecordOptions>) -> Result<crate::Serie> {
                Err(super::namespace::no_table(self))
            }

            fn overwrite_serie(
                &mut self,
                _value: crate::Serie,
                _options: Option<&RecordOptions>,
            ) -> Result<crate::IOResult> {
                Err(super::namespace::no_table(self))
            }

            fn append_serie(
                &mut self,
                _value: crate::Serie,
                _options: Option<&RecordOptions>,
            ) -> Result<crate::IOResult> {
                Err(super::namespace::no_table(self))
            }

            fn merge_serie(
                &mut self,
                _value: crate::Serie,
                _options: Option<&RecordOptions>,
            ) -> Result<crate::IOResult> {
                Err(super::namespace::no_table(self))
            }
        }
    };
}

pub(crate) use container_object_io;

container_object_io!(Namespace);

/// What a container object declares as its representation: a directory.
pub(crate) static CONTAINER_MEDIA_TYPE: std::sync::LazyLock<crate::MediaType> =
    std::sync::LazyLock::new(|| crate::MediaType::from(crate::MimeType::DIRECTORY));

/// The refusal of a record verb on a container object: a namespace is no
/// table, so name a table under it.
pub(crate) fn no_table(object: &dyn ObjectValue) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{}", path_text(object.path())),
        reason: format_smolstr!(
            "expected a table, got the {} `{}`; name a table under it",
            object.kind(),
            path_text(object.path())
        ),
    }
}

/// The names of one collection level, yielded one at a time.
///
/// The walk runs as the iterator is drained, so taking three names from a
/// level of a hundred thousand costs three entries. The item is a
/// [`Result`], so a listing fails *at* the failing entry and is fused
/// afterwards. Order is the store's own.
pub struct Names {
    /// The walk still running. `None` once the listing is spent.
    entries: Option<Box<dyn Iterator<Item = Result<SmolStr>> + Send + Sync>>,
}

impl Names {
    /// A listing of nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(std::iter::empty())
    }

    /// Wrap a walk that is already lazy.
    pub fn new(entries: impl Iterator<Item = Result<SmolStr>> + Send + Sync + 'static) -> Self {
        Self {
            entries: Some(Box::new(entries)),
        }
    }

    /// A listing that reports one failure and then ends.
    #[must_use]
    pub fn failing(error: Error) -> Self {
        Self::new(std::iter::once(Err(error)))
    }

    /// The names of the children of `kind` a namespace lists.
    fn of_kind(children: Objects, kind: IOKind) -> Self {
        Self::new(children.filter_map(move |child| match child {
            Ok(child) if child.kind() == kind => Some(Ok(SmolStr::new(child.name()))),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        }))
    }
}

impl Iterator for Names {
    type Item = Result<SmolStr>;

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

impl fmt::Debug for Names {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Names")
            .field("spent", &self.entries.is_none())
            .finish()
    }
}

/// The namespaces one level below a catalog or a namespace, as a lazy view.
///
/// Constructing the view touches nothing; every question is asked of the
/// store when it is asked of the view. Names may be dotted:
/// `namespaces.get("sales.eu")` descends.
#[derive(Debug, Clone, Copy)]
pub struct Namespaces<'a> {
    parent: &'a dyn NamespaceValue,
}

impl<'a> Namespaces<'a> {
    pub(crate) const fn of(parent: &'a dyn NamespaceValue) -> Self {
        Self { parent }
    }

    /// Open the named namespace, dotted names descending.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing - or a table -
    /// is there, and the store's own failure.
    pub fn get(&self, name: &str) -> Result<Namespace> {
        let path = name.into_object_path_checked()?;
        resolve_under(self.parent, path)?.into_namespace()
    }

    /// Create the named namespace, dotted names descending to the parent it
    /// is created under.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the name is taken, the refusal of a
    /// namespace that creates nothing, and the store's own failure.
    pub fn create(&self, name: &str, properties: &Properties) -> Result<Namespace> {
        let (parent, last) = self.split(name)?;
        match parent {
            Some(parent) => parent.create_namespace(&last, properties),
            None => self.parent.create_namespace(&last, properties),
        }
    }

    /// Open the named namespace, creating it when absent.
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create(&self, name: &str, properties: &Properties) -> Result<Namespace> {
        match self.get(name) {
            Ok(namespace) => Ok(namespace),
            Err(error) if error.is_absent() => self.create(name, properties),
            Err(error) => Err(error),
        }
    }

    /// Whether the named namespace exists, asked of the store now.
    ///
    /// # Errors
    ///
    /// Returns the store's own failure.
    pub fn contains(&self, name: &str) -> Result<bool> {
        match self.get(name) {
            Ok(_) => Ok(true),
            Err(error) if error.is_absent() => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The namespace names one level down, one at a time.
    #[must_use]
    pub fn iter(&self) -> Names {
        Names::of_kind(self.parent.children(), IOKind::Namespace)
    }

    /// How many namespaces are one level down, which drains the listing.
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

    /// Whether no namespace is one level down, which costs the listing up
    /// to its first namespace.
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

    /// The namespace a dotted name's parent parts name, and the last part.
    fn split(&self, name: &str) -> Result<(Option<Namespace>, SmolStr)> {
        let mut path = name.into_object_path_checked()?;
        let Some(last) = path.pop() else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", path_text(self.parent.path())),
                reason: SmolStr::new_static("expected a name to create, got the empty path"),
            });
        };
        if path.is_empty() {
            return Ok((None, last));
        }
        Ok((
            Some(resolve_under(self.parent, path)?.into_namespace()?),
            last,
        ))
    }
}

/// The tables one level below a catalog or a namespace, as a lazy view.
///
/// The same shape as [`Namespaces`]: constructing it touches nothing, names
/// may be dotted, and the write helpers open or create the table through
/// [`NamespaceValue::create_table`] and then write through the table's own
/// record surface.
#[derive(Debug, Clone, Copy)]
pub struct Tables<'a> {
    parent: &'a dyn NamespaceValue,
}

impl<'a> Tables<'a> {
    pub(crate) const fn of(parent: &'a dyn NamespaceValue) -> Self {
        Self { parent }
    }

    /// Open the named table, dotted names descending.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the path when nothing - or a
    /// namespace - is there, [`Error::Conflict`] when several entries are,
    /// and the store's own failure.
    pub fn get(&self, name: &str) -> Result<Table> {
        let path = name.into_object_path_checked()?;
        resolve_under(self.parent, path)?.into_table()
    }

    /// Create the named table with `field` as its row schema, dotted names
    /// descending to the namespace it is created under.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Conflict`] when the name is taken, the refusal of a
    /// namespace that creates nothing, and the store's own failure.
    pub fn create(&self, name: &str, field: &Field, properties: &Properties) -> Result<Table> {
        let (parent, last) = self.split(name)?;
        match parent {
            Some(parent) => parent.create_table(&last, field, properties),
            None => self.parent.create_table(&last, field, properties),
        }
    }

    /// Open the named table if it exists, creating it otherwise.
    ///
    /// An existing table is opened as it is: `field` describes only the
    /// table this call would create.
    ///
    /// # Errors
    ///
    /// Returns the failure of whichever operation ran.
    pub fn open_or_create(
        &self,
        name: &str,
        field: &Field,
        properties: &Properties,
    ) -> Result<Table> {
        match self.get(name) {
            Ok(table) => Ok(table),
            Err(error) if error.is_absent() => self.create(name, field, properties),
            Err(error) => Err(error),
        }
    }

    /// Open the named table, creating it from the reader's schema when
    /// absent: the first half of the write helpers, public so a caller can
    /// settle options on the table before handing it the rows.
    ///
    /// # Errors
    ///
    /// Returns an error when the reader's schema cannot describe a table, or
    /// when the open or the create fails.
    pub fn open_or_create_from_arrow_reader(
        &self,
        name: &str,
        batches: &BatchReader,
        properties: &Properties,
    ) -> Result<Table> {
        match self.get(name) {
            Ok(table) => Ok(table),
            Err(error) if error.is_absent() => {
                let field = crate::arrow::field_from_arrow_schema(
                    crate::media::DEFAULT_ROOT_NAME,
                    &batches.schema(),
                )?;
                self.create(name, &field, properties)
            }
            Err(error) => Err(error),
        }
    }

    /// Whether the named table exists, asked of the store now.
    ///
    /// # Errors
    ///
    /// Returns the store's own failure.
    pub fn contains(&self, name: &str) -> Result<bool> {
        match self.get(name) {
            Ok(_) => Ok(true),
            Err(error) if error.is_absent() => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The table names one level down, one at a time.
    #[must_use]
    pub fn iter(&self) -> Names {
        Names::of_kind(self.parent.children(), IOKind::Table)
    }

    /// How many tables are one level down, which drains the listing.
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

    /// Whether no table is one level down, which costs the listing up to
    /// its first table.
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

    /// Append `batches` to the named table, creating it on first write from
    /// the reader's own schema, and answer the table.
    ///
    /// # Errors
    ///
    /// Returns an error when the open, the create or the append fails.
    pub fn append_arrow_reader(&self, name: &str, batches: BatchReader) -> Result<Table> {
        let mut table =
            self.open_or_create_from_arrow_reader(name, &batches, &Properties::new())?;
        let options = table.record_options()?;
        table.append_arrow_reader(batches, &options)?;
        Ok(table)
    }

    /// [`Self::append_arrow_reader`] under explicit record options.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::append_arrow_reader`] returns.
    pub fn append_arrow_reader_with_options(
        &self,
        name: &str,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<Table> {
        let mut table =
            self.open_or_create_from_arrow_reader(name, &batches, &Properties::new())?;
        table.append_arrow_reader(batches, options)?;
        Ok(table)
    }

    /// Replace the named table's rows with `batches`, creating it on first
    /// write from the reader's own schema, and answer the table.
    ///
    /// # Errors
    ///
    /// Returns an error when the open, the create or the overwrite fails.
    pub fn overwrite_arrow_reader(&self, name: &str, batches: BatchReader) -> Result<Table> {
        let mut table =
            self.open_or_create_from_arrow_reader(name, &batches, &Properties::new())?;
        let options = table.record_options()?;
        table.overwrite_arrow_reader(batches, &options)?;
        Ok(table)
    }

    /// [`Self::overwrite_arrow_reader`] under explicit record options.
    ///
    /// # Errors
    ///
    /// Returns what [`Self::overwrite_arrow_reader`] returns.
    pub fn overwrite_arrow_reader_with_options(
        &self,
        name: &str,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<Table> {
        let mut table =
            self.open_or_create_from_arrow_reader(name, &batches, &Properties::new())?;
        table.overwrite_arrow_reader(batches, options)?;
        Ok(table)
    }

    /// The namespace a dotted name's parent parts name, and the last part.
    fn split(&self, name: &str) -> Result<(Option<Namespace>, SmolStr)> {
        let mut path = name.into_object_path_checked()?;
        let Some(last) = path.pop() else {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", path_text(self.parent.path())),
                reason: SmolStr::new_static("expected a name to create, got the empty path"),
            });
        };
        if path.is_empty() {
            return Ok((None, last));
        }
        Ok((
            Some(resolve_under(self.parent, path)?.into_namespace()?),
            last,
        ))
    }
}

/// A name read as a path below a view's parent: the grammar's quoting, and
/// never the empty path, which names the parent itself.
trait IntoCheckedPath {
    fn into_object_path_checked(self) -> Result<Vec<SmolStr>>;
}

impl IntoCheckedPath for &str {
    fn into_object_path_checked(self) -> Result<Vec<SmolStr>> {
        use super::IntoObjectPath;
        let path = self.into_object_path()?;
        if path.is_empty() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new_static("$.path"),
                reason: format_smolstr!("expected a name, got the empty path {self:?}"),
            });
        }
        Ok(path)
    }
}
