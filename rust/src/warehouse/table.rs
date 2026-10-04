//! A table: an object whose rows any record read and write reaches, and the
//! enum that says which implementation holds them.

use std::fmt;

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

/// The implementation a table is held by.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Table {
    /// A table over any location a record medium reads.
    ///
    /// Boxed: a media table carries its identifier, its location, two
    /// property bags and a declared field.
    Media(Box<MediaTable>),
    /// An Iceberg table, rooted on the handle its catalog keeps.
    ///
    /// Boxed: the table carries its description and, once it has read one,
    /// the current metadata document.
    #[cfg(feature = "iceberg")]
    Iceberg(Box<crate::iceberg::IcebergTable<super::Handle>>),
}

impl Table {
    /// Borrow the implementation through the contract every table answers.
    pub fn as_table(&self) -> &dyn TableValue {
        match self {
            Self::Media(table) => table.as_ref(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => table.as_ref(),
        }
    }

    /// Borrow the implementation as a byte handle.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Media(table) => table.as_ref(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => table.as_ref(),
        }
    }

    /// Borrow the implementation mutably as a byte handle.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Media(table) => table.as_mut(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => table.as_mut(),
        }
    }

    /// Return this table with properties stated on it, which its handle
    /// opens with: what a target's `with (...)` clause states for the table
    /// it names.
    #[must_use]
    pub fn with_properties(self, properties: Properties) -> Self {
        match self {
            Self::Media(table) => Self::Media(Box::new(table.with_properties(properties))),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => Self::Iceberg(Box::new(table.with_properties(properties))),
        }
    }

    fn as_media(&self) -> &dyn IOMedia {
        match self {
            Self::Media(table) => table.as_ref(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => table.as_ref(),
        }
    }

    fn as_media_mut(&mut self) -> &mut dyn IOMedia {
        match self {
            Self::Media(table) => table.as_mut(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => table.as_mut(),
        }
    }

    /// The implementation's own name: `MediaTable` or `IcebergTable`.
    pub(crate) const fn implementation_name(&self) -> &'static str {
        match self {
            Self::Media(_) => "MediaTable",
            #[cfg(feature = "iceberg")]
            Self::Iceberg(_) => "IcebergTable",
        }
    }

    /// Whether anything is at the table's location now.
    pub(crate) fn exists(&self) -> bool {
        match self {
            Self::Media(table) => table.exists(),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => table.exists(),
        }
    }

    /// The table with its parent's effective properties pushed into it.
    pub(crate) fn inheriting(self, parent: &Properties) -> Self {
        match self {
            Self::Media(table) => Self::Media(Box::new(table.inheriting(parent))),
            #[cfg(feature = "iceberg")]
            Self::Iceberg(table) => Self::Iceberg(Box::new(table.inheriting(parent))),
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

#[cfg(feature = "iceberg")]
impl From<crate::iceberg::IcebergTable<super::Handle>> for Table {
    fn from(table: crate::iceberg::IcebergTable<super::Handle>) -> Self {
        Self::Iceberg(Box::new(table))
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

    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::parquet::FileStatistics> {
        IOMedia::read_parquet_statistics(self.as_media())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::parquet::GeospatialStatistics> {
        IOMedia::read_parquet_geospatial_statistics(self.as_media(), column)
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field> {
        IOMedia::read_arrow_field(self.as_media(), options)
    }

    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        IOMedia::read_arrow_reader(self.as_media(), options)
    }

    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::SerieReader> {
        IOMedia::read_serie(self.as_media(), options)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::overwrite_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_prepared_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::overwrite_arrow_batch(self.as_media_mut(), batch, options)
    }

    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::append_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::append_arrow_batch(self.as_media_mut(), batch, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::merge_arrow_reader(self.as_media_mut(), batches, options)
    }

    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::merge_arrow_batch(self.as_media_mut(), batch, options)
    }
}
