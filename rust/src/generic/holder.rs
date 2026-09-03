//! One concrete value for any [`IOBase`] implementation.

use crate::buffered::{Buffered, BufferedOptions};
use crate::{MediaType, Result, Url};

use crate::io::{Buffer, IOBase};
use crate::local::Folder;

use crate::local::File;

/// A concrete, sized value holding any core [`IOBase`] implementation.
///
/// Hierarchy accessors such as [`IOBase::parent`], [`IOBase::child_by_path`], and
/// [`IOBase::ls`] have to return *some* handle without knowing which kind it
/// will be, and `Box<dyn IOBase>` would erase the concrete type a caller needs
/// to match on. `Holder` is that return type: an enum over the implementations
/// the core ships, which itself implements [`IOBase`] by delegation.
///
/// A local directory therefore yields [`Holder::Folder`] for its
/// subdirectories and [`Holder::File`] for its files, and a caller can walk the
/// tree through one type.
///
/// ```
/// use yggdryl::generic::Holder;
/// use yggdryl::io::IOBase;
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Holder::folder(std::env::temp_dir())?;
/// assert!(root.is_container());
///
/// // A leaf is a mapped file, and it need not exist yet.
/// let leaf = root.child_by_path("yggdryl-holder-doc.bin")?;
/// assert!(!leaf.is_container());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub enum Holder {
    /// An in-memory byte array.
    Buffer(Buffer),
    /// A local directory.
    Folder(Folder),
    /// A local location that resolves to whatever it turns out to be.
    Path(crate::local::Path),
    /// A memory-mapped local file.
    File(File),
    /// A directory on a foreign Arrow filesystem.
    ArrowFolder(crate::arrowfs::Folder),
    /// A foreign-filesystem location that resolves to whatever it turns out
    /// to be.
    ArrowPath(crate::arrowfs::Path),
    /// A staged whole-value file on a foreign Arrow filesystem.
    ArrowFile(crate::arrowfs::File),
    /// Any of the others, read through a page cache.
    ///
    /// The box is what keeps the enum a fixed size: this variant holds a
    /// handle of the very type it belongs to.
    Buffered(Box<Buffered<Self>>),
    /// Any of the others, retained behind its inferred record encoding.
    ///
    /// The box breaks the recursive shape: a [`Media`](crate::generic::Media)
    /// owns a `Holder` as its byte handle, while this variant lets a binding
    /// keep that media wrapper (and its opened-session metadata cache) without
    /// changing from the one `Holder` surface.
    #[cfg(feature = "arrow")]
    Media(Box<crate::generic::Media>),
}

impl Holder {
    /// Hold an in-memory buffer.
    pub const fn buffer(buffer: Buffer) -> Self {
        Self::Buffer(buffer)
    }

    /// Hold a local directory, without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn folder(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self::Folder(Folder::new(path)?))
    }

    /// Hold a memory-mapped local file, without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self::File(File::new(path)?))
    }

    /// Hold the local resource a path names.
    ///
    /// The returned [`Self::Path`] resolves only when an operation needs to
    /// know what is there. A caller that already knows the role can select
    /// [`Self::folder`] or [`Self::file`] explicitly.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn local(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Ok(Self::Path(crate::local::Path::new(path)?))
    }

    /// Hold this resource behind a page cache.
    ///
    /// A holder that is already buffered is re-wrapped with the new options
    /// rather than nested, so there is never a second cache layer. This is the
    /// inherent spelling of [`IOBase::buffered`], and it wins method
    /// resolution over it.
    #[must_use]
    pub fn buffered(self, options: BufferedOptions) -> Self {
        let held = match self {
            Self::Buffered(buffered) => buffered.into_handle(),
            other => other,
        };
        Self::Buffered(Box::new(Buffered::new(held, options)))
    }

    /// Retain the record implementation inferred from this handle's media type.
    ///
    /// The conversion is lazy: it only adds the stateful wrapper and reads no
    /// bytes. IPC, Parquet (when enabled), and Avro are held through
    /// [`Self::Media`]. Plain text needs no wrapper: the ordinary record-media
    /// methods select its decoder through [`RecordOptions`](super::RecordOptions).
    /// A page cache remains the outermost wrapper, so promotion followed by
    /// repeated buffering cannot stack caches.
    ///
    /// A directory, JSON document, or any other ordinary byte representation
    /// is deliberately returned unchanged. This is a best-fitting view, not a
    /// request that the handle must be tabular, so unsupported media is never
    /// an error.
    #[must_use]
    pub fn into_media(self) -> Self {
        #[cfg(not(feature = "arrow"))]
        {
            self
        }

        #[cfg(feature = "arrow")]
        {
            if self.has_media_surface() {
                return self;
            }

            let base = self.media_type().base().clone();
            let supported = base == crate::MimeType::ARROW_STREAM
                || base == crate::MimeType::ARROW_FILE
                || base == crate::MimeType::AVRO
                || cfg!(feature = "parquet") && base == crate::MimeType::PARQUET;
            if !supported {
                return self;
            }

            // Keep an existing page cache outside the media wrapper. Besides
            // preserving the cache's one-layer invariant, this lets its
            // IOMedia delegation reach the retained encoding override.
            if let Self::Buffered(buffered) = self {
                let options = *buffered.options();
                let held = buffered.into_handle().into_media();
                return Self::Buffered(Box::new(Buffered::new(held, options)));
            }

            if base == crate::MimeType::ARROW_STREAM || base == crate::MimeType::ARROW_FILE {
                return Self::Media(Box::new(crate::generic::Media::ipc(self)));
            }
            #[cfg(feature = "parquet")]
            if base == crate::MimeType::PARQUET {
                return Self::Media(Box::new(crate::generic::Media::parquet(self)));
            }
            debug_assert_eq!(base, crate::MimeType::AVRO);
            Self::Media(Box::new(crate::generic::Media::avro(self)))
        }
    }

    /// Materialize this holder and retain any record metadata the inferred
    /// media implementation can cache.
    ///
    /// This inherent method intentionally shadows [`IOBase::open`] for a
    /// concrete `Holder`. Stateful media wrappers call `open` through their
    /// generic `H: IOBase`, so their inner holder reaches the trait method and
    /// cannot recursively promote itself into the same encoding.
    ///
    /// # Errors
    ///
    /// Returns the selected handle or media implementation's open failure.
    pub fn open(&mut self) -> Result<()> {
        let held = std::mem::replace(self, Self::Buffer(Buffer::new()));
        *self = held.into_media();
        IOBase::open(self)
    }

    /// Return whether this holder already retains a media implementation.
    #[cfg(feature = "arrow")]
    fn has_media_surface(&self) -> bool {
        match self {
            Self::Media(_) => true,
            Self::Buffered(buffered) => buffered.handle().has_media_surface(),
            _ => false,
        }
    }

    /// Borrow the held implementation as a trait object.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::ArrowFolder(inner) => inner,
            Self::ArrowPath(inner) => inner,
            Self::ArrowFile(inner) => inner,
            Self::Buffered(inner) => inner.as_ref(),
            #[cfg(feature = "arrow")]
            Self::Media(inner) => inner.as_ref(),
        }
    }

    /// Borrow the held implementation mutably as a trait object.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::ArrowFolder(inner) => inner,
            Self::ArrowPath(inner) => inner,
            Self::ArrowFile(inner) => inner,
            Self::Buffered(inner) => inner.as_mut(),
            #[cfg(feature = "arrow")]
            Self::Media(inner) => inner.as_mut(),
        }
    }

    /// Borrow the held implementation through its media contract.
    #[cfg(feature = "arrow")]
    fn as_media(&self) -> &dyn crate::io::IOMedia {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::ArrowFolder(inner) => inner,
            Self::ArrowPath(inner) => inner,
            Self::ArrowFile(inner) => inner,
            Self::Buffered(inner) => inner.as_ref(),
            #[cfg(feature = "arrow")]
            Self::Media(inner) => inner.as_ref(),
        }
    }

    /// Mutably borrow the held implementation through its media contract.
    #[cfg(feature = "arrow")]
    fn as_media_mut(&mut self) -> &mut dyn crate::io::IOMedia {
        match self {
            Self::Buffer(inner) => inner,
            Self::Folder(inner) => inner,
            Self::Path(inner) => inner,
            Self::File(inner) => inner,
            Self::ArrowFolder(inner) => inner,
            Self::ArrowPath(inner) => inner,
            Self::ArrowFile(inner) => inner,
            Self::Buffered(inner) => inner.as_mut(),
            #[cfg(feature = "arrow")]
            Self::Media(inner) => inner.as_mut(),
        }
    }
}

impl crate::io::IOMedia for Holder {
    fn as_io_base(&self) -> &dyn IOBase {
        self.as_io()
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self.as_io_mut()
    }

    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        crate::io::IOMedia::row_size(self.as_media())
    }

    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        crate::io::IOMedia::column_size(self.as_media())
    }

    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<crate::generic::RecordOptions> {
        crate::io::IOMedia::record_options(self.as_media())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::parquet::FileStatistics> {
        crate::io::IOMedia::read_parquet_statistics(self.as_media())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::parquet::GeospatialStatistics> {
        crate::io::IOMedia::read_parquet_geospatial_statistics(self.as_media(), column)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &crate::generic::RecordOptions) -> Result<crate::Field> {
        crate::io::IOMedia::read_arrow_field(self.as_media(), options)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_reader(
        &self,
        options: &crate::generic::RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        crate::io::IOMedia::read_arrow_reader(self.as_media(), options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::overwrite_arrow_reader(self.as_media_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::overwrite_prepared_arrow_reader(self.as_media_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::overwrite_arrow_batch(self.as_media_mut(), batch, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::append_arrow_reader(self.as_media_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::append_arrow_batch(self.as_media_mut(), batch, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::merge_arrow_reader(self.as_media_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::merge_arrow_batch(self.as_media_mut(), batch, options)
    }
}

impl IOBase for Holder {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_io().pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::io::ByteStream<'_>> {
        self.as_io().pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.as_io().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_io().read_range_bytes(offset, length)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.as_io_mut().pwrite(offset, bytes)
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

    fn url(&self) -> Option<&Url> {
        self.as_io().url()
    }

    fn media_type(&self) -> &MediaType {
        self.as_io().media_type()
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

    fn parent(&self) -> Option<Self> {
        self.as_io().parent()
    }

    fn child_by_path(&self, name: &str) -> Result<Self> {
        self.as_io().child_by_path(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::io::Listing {
        self.as_io().ls(recursive, include_private)
    }

    fn kind(&self) -> crate::IOKind {
        self.as_io().kind()
    }

    fn is_atomic(&self) -> bool {
        self.as_io().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_io().is_tabular()
    }
}

impl From<Buffer> for Holder {
    fn from(value: Buffer) -> Self {
        Self::Buffer(value)
    }
}

impl From<Folder> for Holder {
    fn from(value: Folder) -> Self {
        Self::Folder(value)
    }
}

impl From<File> for Holder {
    fn from(value: File) -> Self {
        Self::File(value)
    }
}

impl From<crate::arrowfs::Folder> for Holder {
    fn from(value: crate::arrowfs::Folder) -> Self {
        Self::ArrowFolder(value)
    }
}

impl From<crate::arrowfs::Path> for Holder {
    fn from(value: crate::arrowfs::Path) -> Self {
        Self::ArrowPath(value)
    }
}

impl From<crate::arrowfs::File> for Holder {
    fn from(value: crate::arrowfs::File) -> Self {
        Self::ArrowFile(value)
    }
}

impl From<Buffered<Holder>> for Holder {
    fn from(value: Buffered<Self>) -> Self {
        Self::Buffered(Box::new(value))
    }
}

#[cfg(feature = "arrow")]
impl From<crate::generic::Media> for Holder {
    fn from(value: crate::generic::Media) -> Self {
        Self::Media(Box::new(value))
    }
}
