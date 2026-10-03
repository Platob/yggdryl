//! A [`Uri`] as a handle: the storage it names, resolved on first use.
//!
//! Every operation forwards to the one [`Holder`] the identifier resolves to
//! through [`Uri::locator`] and [`Holder::from_url`], the dispatcher every
//! location reaches its backend through, so a URL, a URN and an ARN open
//! exactly what the same location opens anywhere else. The resolved handle is
//! kept in the value, so a staged write, an open scope and a cached answer
//! live as long as the `Uri` does; a clone starts unresolved, as its
//! rendering does. A resolution that fails is stored nowhere: it is the error
//! of every operation that returns one, and the empty answer of every
//! accessor that cannot.

use std::sync::{LazyLock, OnceLock};

use crate::holder::Holder;
use crate::media::RecordOptions;
use crate::{Codec, IOBase, IOKind, IOMedia, Listing, MediaType, Result, Scheme, Url};

use super::Uri;

/// What a handle that resolves to nothing declares: no representation.
static UNRESOLVED: LazyLock<MediaType> = LazyLock::new(MediaType::default);

impl Uri {
    /// The handle this identifier names, resolved on the first call.
    pub(crate) fn held(&self) -> Result<&Holder> {
        if let Some(held) = self.held.get() {
            return Ok(held.as_ref());
        }
        let resolved = self.resolve()?;
        Ok(self.held.get_or_init(|| Box::new(resolved)).as_ref())
    }

    /// The handle this identifier names, mutably, resolved on the first call.
    fn held_mut(&mut self) -> Result<&mut Holder> {
        if self.held.get().is_none() {
            self.held = OnceLock::from(Box::new(self.resolve()?));
        }
        match self.held.get_mut() {
            Some(held) => Ok(held.as_mut()),
            None => unreachable!("a handle was resolved just above"),
        }
    }

    /// Open nothing: name the backend the location selects, as every other
    /// location does. [`Holder::from_url`] never answers [`Holder::Uri`], so
    /// resolving cannot recurse.
    fn resolve(&self) -> Result<Holder> {
        Holder::from_url(&self.locator()?, std::iter::empty::<(&str, &str)>())
    }

    /// Whether this identifier is a name - a URN or an ARN - rather than the
    /// location it resolves to.
    fn is_name(&self) -> bool {
        self.scheme() == &Scheme::URN || self.scheme() == &Scheme::ARN
    }

    /// The resolved handle, taken out of this value: what a reader holding
    /// many handles in turn streams from without borrowing this one.
    pub(crate) fn into_held(mut self) -> Result<Holder> {
        match self.held.take() {
            Some(held) => Ok(*held),
            None => self.resolve(),
        }
    }
}

impl IOBase for Uri {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.held()?.pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        self.held()?.pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.held()?.read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.held()?.read_range_bytes(offset, length)
    }

    fn read_digest(&self, algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
        self.held()?.read_digest(algorithm)
    }

    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        self.held()?.read_range_digest(offset, length, algorithm)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.held_mut()?.pwrite(offset, bytes)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.held_mut()?.write_all_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.held_mut()?.append_bytes(bytes)
    }

    fn size(&self) -> u64 {
        self.held().map_or(0, IOBase::size)
    }

    fn capacity(&self) -> u64 {
        self.held().map_or(0, IOBase::capacity)
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.held_mut()?.reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.held_mut()?.truncate(size)
    }

    /// This identifier itself, with nothing resolved.
    fn uri(&self) -> Option<&Uri> {
        Some(self)
    }

    /// The location this handle opens; none for a name, which has no URL of
    /// its own to lend - [`uri`](Self::uri) is its address.
    fn url(&self) -> Option<&Url> {
        if self.is_name() {
            return None;
        }
        self.held().ok()?.url()
    }

    fn bound_location(&self) -> Option<&crate::fs::BoundLocation> {
        self.held().ok()?.bound_location()
    }

    fn mtime(&self) -> Option<i64> {
        self.held().ok()?.mtime()
    }

    fn media_type(&self) -> &MediaType {
        self.held().map_or(&UNRESOLVED, IOBase::media_type)
    }

    fn applied_codec(&self) -> Codec {
        self.held().map_or(Codec::Identity, IOBase::applied_codec)
    }

    /// Declare the resolved handle's representation. An identifier that
    /// resolves to nothing has nothing to declare it on, and every operation
    /// that could read it names why.
    fn set_media_type(&mut self, media_type: MediaType) {
        if let Ok(held) = self.held_mut() {
            held.set_media_type(media_type);
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.held_mut()?.flush()
    }

    fn open(&mut self) -> Result<()> {
        self.held_mut()?.open()
    }

    fn opened(&self) -> bool {
        self.held.get().is_some_and(|held| held.opened())
    }

    fn close(&mut self) -> Result<()> {
        match self.held.get_mut() {
            Some(held) => held.close(),
            // Nothing was resolved, so nothing was opened to close.
            None => Ok(()),
        }
    }

    fn clear(&mut self) -> Result<()> {
        self.held_mut()?.clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.held_mut()?.remove(recursive)
    }

    fn parent(&self) -> Option<Holder> {
        self.held().ok()?.parent()
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.held()?.child_by_path(path)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        match self.held() {
            Ok(held) => held.ls(recursive, include_private),
            Err(error) => Listing::failing(error),
        }
    }

    fn glob(&self, pattern: &str, include_private: bool) -> Result<Listing> {
        self.held()?.glob(pattern, include_private)
    }

    fn partitions(&self) -> Vec<(String, String)> {
        self.held().map(IOBase::partitions).unwrap_or_default()
    }

    fn kind(&self) -> IOKind {
        self.held().map_or(IOKind::Unknown, IOBase::kind)
    }

    fn is_container(&self) -> bool {
        self.held().is_ok_and(IOBase::is_container)
    }

    fn is_atomic(&self) -> bool {
        self.held().is_ok_and(IOBase::is_atomic)
    }

    fn is_tabular(&self) -> bool {
        self.held().is_ok_and(IOBase::is_tabular)
    }
}

/// Every record operation is the resolved handle's, so an HTTP location's
/// paginated rows, a table folder and the media a name infers answer as they
/// do on that handle.
impl IOMedia for Uri {
    fn as_io_base(&self) -> &dyn IOBase {
        self
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self
    }

    fn row_size(&self) -> Result<u64> {
        IOMedia::row_size(self.held()?)
    }

    fn column_size(&self) -> Result<usize> {
        IOMedia::column_size(self.held()?)
    }

    fn record_options(&self) -> Result<RecordOptions> {
        IOMedia::record_options(self.held()?)
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::parquet::FileStatistics> {
        IOMedia::read_parquet_statistics(self.held()?)
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::parquet::GeospatialStatistics> {
        IOMedia::read_parquet_geospatial_statistics(self.held()?, column)
    }

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        IOMedia::read_arrow_field(self.held()?, options)
    }

    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        IOMedia::read_arrow_reader(self.held()?, options)
    }

    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::SerieReader> {
        IOMedia::read_serie(self.held()?, options)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_arrow_reader(self.held_mut()?, batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_prepared_arrow_reader(self.held_mut()?, batches, options)
    }

    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_arrow_batch(self.held_mut()?, batch, options)
    }

    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::append_arrow_reader(self.held_mut()?, batches, options)
    }

    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::append_arrow_batch(self.held_mut()?, batch, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::merge_arrow_reader(self.held_mut()?, batches, options)
    }

    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::merge_arrow_batch(self.held_mut()?, batch, options)
    }
}
