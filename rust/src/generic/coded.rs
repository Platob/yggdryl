//! One value naming every transparent content-coding handle.

use crate::gzip::Gzip;
use crate::io::IOBase;
use crate::zlib::Zlib;
use crate::zstd::Zstd;
use crate::{Error, Level, MediaType, Result, Url};

/// A handle wrapped in the content coding its media type names.
///
/// [`crate::Codec`] says *which* coding a payload uses; this enum is that coding
/// applied to a handle. Reading through one decompresses and writing through
/// one compresses, so everything downstream - a codec, a record encoding,
/// another handle - sees plain bytes.
///
/// ```
/// use yggdryl::generic::Coded;
/// use yggdryl::io::{Buffer, IOBase};
/// use yggdryl::Url;
///
/// # fn main() -> yggdryl::Result<()> {
/// // The coding comes from the name, so nothing else in the call changes.
/// let named = Url::from_str("file:///trades.csv.gz")?;
/// let mut handle = Coded::wrap(Buffer::new(), yggdryl::Codec::from_url(&named));
///
/// handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
/// handle.flush()?;
/// assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub enum Coded<H: IOBase> {
    /// The bytes pass through unchanged.
    Identity(H),
    /// RFC 1952 gzip framing over DEFLATE.
    Gzip(Gzip<H>),
    /// RFC 1950 zlib framing over DEFLATE.
    Zlib(Zlib<H>),
    /// RFC 8878 Zstandard.
    Zstd(Zstd<H>),
}

impl<H: IOBase> Coded<H> {
    /// Wrap a handle in one coding.
    ///
    /// [`crate::Codec::Deflate`] has no transparent handle of its own - raw DEFLATE
    /// carries no framing to detect - so it wraps as [`Self::Zlib`], which is
    /// the framed form of the same algorithm.
    pub fn wrap(handle: H, codec: crate::Codec) -> Self {
        match codec {
            crate::Codec::Identity => Self::Identity(handle),
            crate::Codec::Gzip => Self::Gzip(Gzip::new(handle)),
            crate::Codec::Zlib | crate::Codec::Deflate => Self::Zlib(Zlib::new(handle)),
            crate::Codec::Zstd => Self::Zstd(Zstd::new(handle)),
        }
    }

    /// Wrap a handle in the coding its own media type declares.
    pub fn infer(handle: H) -> Self {
        let codec = handle.codec();
        Self::wrap(handle, codec)
    }

    /// Return the coding applied to the wrapped handle.
    pub const fn codec(&self) -> crate::Codec {
        match self {
            Self::Identity(_) => crate::Codec::Identity,
            Self::Gzip(_) => crate::Codec::Gzip,
            Self::Zlib(_) => crate::Codec::Zlib,
            Self::Zstd(_) => crate::Codec::Zstd,
        }
    }

    /// Return this handle with a different compression level.
    #[must_use]
    pub fn with_level(self, level: Level) -> Self {
        match self {
            Self::Identity(handle) => Self::Identity(handle),
            Self::Gzip(handle) => Self::Gzip(handle.with_level(level)),
            Self::Zlib(handle) => Self::Zlib(handle.with_level(level)),
            Self::Zstd(handle) => Self::Zstd(handle.with_level(level)),
        }
    }

    /// Borrow the compressed handle underneath.
    pub const fn handle(&self) -> &H {
        match self {
            Self::Identity(handle) => handle,
            Self::Gzip(handle) => handle.handle(),
            Self::Zlib(handle) => handle.handle(),
            Self::Zstd(handle) => handle.handle(),
        }
    }

    /// Consume this handle, publishing any pending write first.
    ///
    /// # Errors
    ///
    /// Returns the encode or write failure.
    pub fn into_handle(self) -> Result<H> {
        match self {
            Self::Identity(handle) => Ok(handle),
            Self::Gzip(handle) => handle.into_handle(),
            Self::Zlib(handle) => handle.into_handle(),
            Self::Zstd(handle) => handle.into_handle(),
        }
    }

    /// Borrow the coded handle as a byte handle.
    pub fn as_io(&self) -> &dyn IOBase {
        match self {
            Self::Identity(handle) => handle,
            Self::Gzip(handle) => handle,
            Self::Zlib(handle) => handle,
            Self::Zstd(handle) => handle,
        }
    }

    /// Borrow the coded handle as a mutable byte handle.
    pub fn as_io_mut(&mut self) -> &mut dyn IOBase {
        match self {
            Self::Identity(handle) => handle,
            Self::Gzip(handle) => handle,
            Self::Zlib(handle) => handle,
            Self::Zstd(handle) => handle,
        }
    }
}

impl<H: IOBase> crate::io::IOMedia for Coded<H> {
    fn as_io_base(&self) -> &dyn IOBase {
        self.as_io()
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self.as_io_mut()
    }

    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        crate::io::IOMedia::row_size(self.as_io())
    }

    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        crate::io::IOMedia::column_size(self.as_io())
    }

    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<crate::generic::RecordOptions> {
        crate::io::IOMedia::record_options(self.as_io())
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &crate::generic::RecordOptions) -> Result<crate::Field> {
        crate::io::IOMedia::read_arrow_field(self.as_io(), options)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_reader(
        &self,
        options: &crate::generic::RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        crate::io::IOMedia::read_arrow_reader(self.as_io(), options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::overwrite_arrow_reader(self.as_io_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::overwrite_prepared_arrow_reader(self.as_io_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::overwrite_arrow_batch(self.as_io_mut(), batch, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::append_arrow_reader(self.as_io_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::append_arrow_batch(self.as_io_mut(), batch, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::merge_arrow_reader(self.as_io_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &crate::generic::RecordOptions,
    ) -> Result<()> {
        crate::io::IOMedia::merge_arrow_batch(self.as_io_mut(), batch, options)
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::parquet::FileStatistics> {
        crate::io::IOMedia::read_parquet_statistics(self.as_io())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::parquet::GeospatialStatistics> {
        crate::io::IOMedia::read_parquet_geospatial_statistics(self.as_io(), column)
    }
}

/// A `Compression` is the decoded view of the handle it wraps.
impl<H: IOBase> IOBase for Coded<H> {
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

    fn kind(&self) -> crate::IOKind {
        self.as_io().kind()
    }

    fn is_atomic(&self) -> bool {
        self.as_io().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_io().is_tabular()
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

    fn parent(&self) -> Option<super::Holder> {
        self.as_io().parent()
    }

    fn child_by_path(&self, name: &str) -> Result<super::Holder> {
        self.as_io().child_by_path(name)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> crate::io::Listing {
        self.as_io().ls(recursive, include_private)
    }
}

impl<H: IOBase> TryFrom<Coded<H>> for Gzip<H> {
    type Error = Error;

    fn try_from(value: Coded<H>) -> Result<Self> {
        match value {
            Coded::Gzip(handle) => Ok(handle),
            other => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("expected a gzip handle, got {}", other.codec()),
            ))),
        }
    }
}

#[cfg(test)]
mod tests;
