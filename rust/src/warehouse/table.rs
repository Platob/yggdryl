//! A table: an object whose rows any record read and write reaches, and the
//! enum that says which implementation holds them.

use std::any::Any;
use std::fmt;
use std::hash::{Hash, Hasher};

use smol_str::SmolStr;

use super::{MediaTable, ObjectValue, Properties};
use crate::media::RecordOptions;
use crate::{Field, IOBase, IOKind, IOMedia, MediaType, Result, Uri, Url};

/// A table: an object whose rows any record read and write reaches.
pub trait TableValue: ObjectValue + IOBase {
    /// Its row schema, with no row read: the declared field, else the stored
    /// one.
    ///
    /// # Errors
    ///
    /// Returns the read, decoding or schema failure of learning the stored
    /// schema.
    fn field(&self) -> Result<Field>;

    /// What holds its rows, as a listing describes it: a leaf's media type,
    /// `directory` for a folder read as the rows beneath it, `table` for a
    /// table format.
    fn storage(&self) -> String;
}

/// A table implemented outside the core's [`MediaTable`], as
/// [`Table::Registered`] holds it: the table contract and the byte and
/// record surface every table answers, the name a refusal calls the
/// implementation by, what a trait object owes the derive-heavy enum
/// holding it - a copy, equality, a hash and the downcast
/// [`Table::downcast_ref`] reads - and the consuming updates and the
/// presence question every table answers.
///
/// [`IOMedia::as_any`] is the medium's state a record read downcasts, and
/// [`RegisteredTable::as_any`] the table itself: a caller holding a
/// `dyn RegisteredTable` names the one it means.
pub trait RegisteredTable: TableValue + IOBase + Send + Sync + fmt::Debug {
    /// The implementation's own name, as a refusal names it:
    /// `IcebergTable`.
    fn implementation_name(&self) -> &'static str;

    /// A boxed copy.
    fn clone_box(&self) -> Box<dyn RegisteredTable>;

    /// Equality across the trait object: the same implementation holding an
    /// equal table.
    fn dyn_eq(&self, other: &dyn RegisteredTable) -> bool;

    /// The implementation's own hash, into any hasher.
    fn dyn_hash(&self, state: &mut dyn Hasher);

    /// The table as `Any`, for [`Table::downcast_ref`].
    fn as_any(&self) -> &dyn Any;

    /// The table as `Any`, mutably, for [`Table::downcast_mut`].
    fn as_any_mut(&mut self) -> &mut dyn Any;

    /// The table with properties stated on it, which its storage opens
    /// with.
    fn with_properties(self: Box<Self>, properties: Properties) -> Box<dyn RegisteredTable>;

    /// The table with its parent's effective properties pushed into it.
    fn inheriting(self: Box<Self>, parent: &Properties) -> Box<dyn RegisteredTable>;

    /// Whether anything is at the table's location now.
    fn exists(&self) -> bool;
}

impl Clone for Box<dyn RegisteredTable> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl PartialEq for dyn RegisteredTable {
    fn eq(&self, other: &Self) -> bool {
        self.dyn_eq(other)
    }
}

impl Eq for dyn RegisteredTable {}

impl Hash for dyn RegisteredTable {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.dyn_hash(state);
    }
}

/// The implementation a table is held by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Table {
    /// A table over any location a record medium reads.
    ///
    /// Boxed: a media table carries its identifier, its location, two
    /// property bags and a declared field.
    Media(Box<MediaTable>),
    /// A table an implementation outside the core answers - an Iceberg
    /// table rooted on the handle its catalog keeps - held through the
    /// contract every such table answers.
    Registered(Box<dyn RegisteredTable>),
}

impl Table {
    /// Borrow the implementation through the contract every table answers.
    pub fn as_table(&self) -> &dyn TableValue {
        match self {
            Self::Media(table) => &**table,
            Self::Registered(table) => &**table,
        }
    }

    /// Borrow the implementation as a byte handle.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Media(table) => &**table,
            Self::Registered(table) => &**table,
        }
    }

    /// Borrow the implementation mutably as a byte handle.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Media(table) => &mut **table,
            Self::Registered(table) => &mut **table,
        }
    }

    /// The implementation as the type it is, when it is a `T`: a
    /// [`MediaTable`], or a registered table's own type.
    #[must_use]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        let any: &dyn Any = match self {
            Self::Media(table) => &**table,
            Self::Registered(table) => RegisteredTable::as_any(&**table),
        };
        any.downcast_ref()
    }

    /// The implementation as the type it is, mutably, when it is a `T`.
    pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T> {
        let any: &mut dyn Any = match self {
            Self::Media(table) => &mut **table,
            Self::Registered(table) => RegisteredTable::as_any_mut(&mut **table),
        };
        any.downcast_mut()
    }

    /// Return this table with properties stated on it, which its handle
    /// opens with: what a target's `with (...)` clause states for the table
    /// it names.
    #[must_use]
    pub fn with_properties(self, properties: Properties) -> Self {
        match self {
            Self::Media(table) => Self::Media(Box::new(table.with_properties(properties))),
            Self::Registered(table) => Self::Registered(table.with_properties(properties)),
        }
    }

    fn as_media(&self) -> &dyn IOMedia {
        match self {
            Self::Media(table) => &**table,
            Self::Registered(table) => &**table,
        }
    }

    fn as_media_mut(&mut self) -> &mut dyn IOMedia {
        match self {
            Self::Media(table) => &mut **table,
            Self::Registered(table) => &mut **table,
        }
    }

    /// The implementation's own name: `MediaTable`, or what a registered
    /// table calls itself.
    pub(crate) fn implementation_name(&self) -> &'static str {
        match self {
            Self::Media(_) => "MediaTable",
            Self::Registered(table) => table.implementation_name(),
        }
    }

    /// Whether anything is at the table's location now.
    pub(crate) fn exists(&self) -> bool {
        match self {
            Self::Media(table) => table.exists(),
            Self::Registered(table) => table.exists(),
        }
    }

    /// The table with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(self, parent: &Properties) -> Self {
        match self {
            Self::Media(table) => Self::Media(Box::new(table.inheriting(parent))),
            Self::Registered(table) => Self::Registered(table.inheriting(parent)),
        }
    }
}

impl ObjectValue for Table {
    fn name(&self) -> &str {
        self.as_table().name()
    }

    fn path(&self) -> &[SmolStr] {
        self.as_table().path()
    }

    fn kind(&self) -> IOKind {
        ObjectValue::kind(self.as_table())
    }

    fn description(&self) -> Option<&str> {
        self.as_table().description()
    }

    fn url(&self) -> Option<&Url> {
        ObjectValue::url(self.as_table())
    }

    fn modified(&self) -> Option<i64> {
        self.as_table().modified()
    }

    fn properties(&self) -> Result<Properties> {
        self.as_table().properties()
    }

    fn update_properties(&self, updates: &Properties, removes: &[SmolStr]) -> Result<()> {
        self.as_table().update_properties(updates, removes)
    }
}

impl TableValue for Table {
    fn field(&self) -> Result<Field> {
        self.as_table().field()
    }

    fn storage(&self) -> String {
        self.as_table().storage()
    }
}

impl fmt::Display for Table {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        super::object::write_path(formatter, self.path())
    }
}

impl From<MediaTable> for Table {
    fn from(table: MediaTable) -> Self {
        Self::Media(Box::new(table))
    }
}

impl IOBase for Table {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_io().pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        self.as_io().pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.as_io().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_io().read_range_bytes(offset, length)
    }

    fn read_tail_bytes(&self, length: usize) -> Result<(Vec<u8>, u64)> {
        self.as_io().read_tail_bytes(length)
    }

    fn read_digest(&self, algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
        self.as_io().read_digest(algorithm)
    }

    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        self.as_io().read_range_digest(offset, length, algorithm)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.as_io_mut().pwrite(offset, bytes)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_io_mut().write_all_bytes(bytes)
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_io_mut().create_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.as_io_mut().append_bytes(bytes)
    }

    fn size(&self) -> u64 {
        self.as_io().size()
    }

    fn capacity(&self) -> u64 {
        self.as_io().capacity()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.as_io_mut().reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.as_io_mut().truncate(size)
    }

    fn uri(&self) -> Option<&Uri> {
        self.as_io().uri()
    }

    fn url(&self) -> Option<&Url> {
        self.as_io().url()
    }

    fn bound_location(&self) -> Option<&crate::fs::BoundLocation> {
        self.as_io().bound_location()
    }

    fn mtime(&self) -> Option<i64> {
        self.as_io().mtime()
    }

    fn media_type(&self) -> &MediaType {
        self.as_io().media_type()
    }

    fn applied_codec(&self) -> crate::Codec {
        self.as_io().applied_codec()
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.as_io_mut().set_media_type(media_type);
    }

    fn flush(&mut self) -> Result<()> {
        self.as_io_mut().flush()
    }

    fn open(&mut self) -> Result<()> {
        self.as_io_mut().open()
    }

    fn opened(&self) -> bool {
        self.as_io().opened()
    }

    fn close(&mut self) -> Result<()> {
        self.as_io_mut().close()
    }

    fn clear(&mut self) -> Result<()> {
        self.as_io_mut().clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.as_io_mut().remove(recursive)
    }

    fn parent(&self) -> Option<crate::holder::Holder> {
        self.as_io().parent()
    }

    fn child_by_path(&self, name: &str) -> Result<crate::holder::Holder> {
        self.as_io().child_by_path(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::Listing {
        self.as_io().ls(recursive, include_private)
    }

    fn glob(&self, pattern: &str, include_private: bool) -> Result<crate::Listing> {
        self.as_io().glob(pattern, include_private)
    }

    fn partitions(&self) -> Vec<(String, String)> {
        self.as_io().partitions()
    }

    fn kind(&self) -> IOKind {
        self.as_io().kind()
    }

    fn is_container(&self) -> bool {
        self.as_io().is_container()
    }

    fn is_atomic(&self) -> bool {
        self.as_io().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_io().is_tabular()
    }
}

impl IOMedia for Table {
    fn as_io_base(&self) -> &dyn IOBase {
        self.as_io()
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self.as_io_mut()
    }

    fn row_size(&self) -> Result<u64> {
        IOMedia::row_size(self.as_media())
    }

    fn column_size(&self) -> Result<usize> {
        IOMedia::column_size(self.as_media())
    }

    fn record_options(&self) -> Result<RecordOptions> {
        IOMedia::record_options(self.as_media())
    }

    fn merge_by(&self) -> Result<crate::Selector> {
        IOMedia::merge_by(self.as_media())
    }

    fn as_any(&self) -> Option<&dyn Any> {
        IOMedia::as_any(self.as_media())
    }

    fn read_origin_field(&self) -> Result<Option<Field>> {
        IOMedia::read_origin_field(self.as_media())
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        IOMedia::read_arrow_field(self.as_media(), options)
    }

    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::Serie> {
        IOMedia::read_serie(self.as_media(), options)
    }

    fn overwrite_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        IOMedia::overwrite_serie(self.as_media_mut(), value, options)
    }

    fn overwrite_prepared_serie(
        &mut self,
        value: crate::StreamChunkedSerie,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_prepared_serie(self.as_media_mut(), value, options)
    }

    fn append_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        IOMedia::append_serie(self.as_media_mut(), value, options)
    }

    fn merge_serie(
        &mut self,
        value: crate::Serie,
        options: Option<&RecordOptions>,
    ) -> Result<crate::IOResult> {
        IOMedia::merge_serie(self.as_media_mut(), value, options)
    }
}

crate::media_serie::media_serie!(
    WarehouseTableSerie,
    WarehouseTable,
    as_warehouse_table,
    get_warehouse_table_mut,
    accepts = None
);
