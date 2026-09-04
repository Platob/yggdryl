//! Forwarding and boxed-handle implementations for [`IOBase`](super::IOBase).

use std::io::{Read, Write};

use super::IOBase;
#[cfg(feature = "arrow")]
use crate::generic::RecordOptions;
use crate::holder::Holder;
use crate::{ByteStream, IOKind, IOMedia, Listing, MediaType, Result, Url};

/// Implement [`IOBase`] methods by forwarding them to an inner handle.
///
/// A type that wraps a handle - a media reader, a page cache, a test double -
/// mirrors that handle rather than owning bytes of its own. The macro expands
/// to the forwarding bodies inside an `impl IOBase for` block. A wrapper that
/// changes one method uses the list form below and leaves that method out.
///
/// [`IOBase::clear`] and [`IOBase::remove`] are delegated too, so a wrapper
/// empties and deletes the resource it wraps - not merely its own view of it -
/// without thinking about it. A wrapper holding a cache of its own must
/// invalidate it as part of those calls, and a macro-provided body cannot be
/// overridden, so it invokes the second form and writes the pair itself:
///
/// ```
/// use yggdryl::{IOBase, IOMedia, holder::Buffer};
///
/// struct Cached {
///     handle: Buffer,
///     schema: Option<String>,
/// }
///
/// impl IOMedia for Cached {
///     yggdryl::delegate_iomedia!(handle);
/// }
///
/// impl IOBase for Cached {
///     yggdryl::delegate_iobase!(handle, except_lifecycle);
///
///     fn clear(&mut self) -> yggdryl::Result<()> {
///         self.schema = None;
///         self.handle.clear()
///     }
///
///     fn remove(&mut self, recursive: bool) -> yggdryl::Result<()> {
///         self.schema = None;
///         self.handle.remove(recursive)
///     }
/// }
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut cached = Cached {
///     handle: Buffer::from_bytes(b"AAPL".to_vec()),
///     schema: Some("row".to_owned()),
/// };
/// cached.remove(false)?;
/// assert!(cached.schema.is_none());
/// assert_eq!(cached.size(), 0);
/// # Ok(())
/// # }
/// ```
///
/// ```
/// use yggdryl::{IOBase, IOMedia, holder::Buffer};
///
/// struct Wrapper {
///     handle: Buffer,
/// }
///
/// impl IOMedia for Wrapper {
///     yggdryl::delegate_iomedia!(handle);
/// }
///
/// impl IOBase for Wrapper {
///     yggdryl::delegate_iobase!(handle);
/// }
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut wrapper = Wrapper {
///     handle: Buffer::new(),
/// };
/// wrapper.write_all_bytes(b"AAPL")?;
/// assert_eq!(wrapper.read_all_bytes()?, b"AAPL");
/// # Ok(())
/// # }
/// ```
///
/// A wrapper that *changes* one of the methods above names the ones it still
/// mirrors instead, because a method cannot be both expanded here and written
/// out below. The list form is that spelling - it is exactly the same bodies,
/// only chosen - and what it leaves out is what the wrapper owns:
///
/// Leave a name out and the wrapper takes the trait's own default for it,
/// which for `clear` and `remove` means truncating rather than reaching the
/// resource. That is why the list below still names them: a wrapper drops them
/// from the list only when it writes the pair itself, as `Cached` does above.
///
/// ```
/// use std::sync::atomic::{AtomicUsize, Ordering};
///
/// use yggdryl::{IOBase, IOMedia, holder::Buffer};
///
/// /// A handle that counts the reads reaching the one it wraps.
/// struct Counted {
///     handle: Buffer,
///     reads: AtomicUsize,
/// }
///
/// impl IOMedia for Counted {
///     yggdryl::delegate_iomedia!(handle);
/// }
///
/// impl IOBase for Counted {
///     yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve,
///         truncate, url, media_type, set_media_type, flush, parent, child_by_path,
///         ls, kind, clear, remove, is_atomic, is_tabular, is_io);
///
///     // `pread` takes `&self`, so the counter is atomic rather than a cell:
///     // the trait is `Send`, and a double is held across threads like any
///     // other handle.
///     fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
///         self.reads.fetch_add(1, Ordering::Relaxed);
///         self.handle.pread(offset, buffer)
///     }
/// }
///
/// # fn main() -> yggdryl::Result<()> {
/// let counted = Counted {
///     handle: Buffer::from_bytes(b"AAPL".to_vec()),
///     reads: AtomicUsize::new(0),
/// };
/// assert_eq!(counted.read_all_bytes()?, b"AAPL");
/// assert!(counted.reads.load(Ordering::Relaxed) > 0);
/// # Ok(())
/// # }
/// ```
#[macro_export]
macro_rules! delegate_iobase {
    // The whole contract, lifecycle included: the wrapper changes nothing.
    ($handle:ident) => {
        $crate::delegate_iobase!(@methods $handle: pread, pstream_bytes, pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
            ls, kind, clear, remove, is_atomic, is_tabular, is_io);
    };

    // Everything but [`IOBase::clear`] and [`IOBase::remove`], which a wrapper
    // holding a cache of its own writes itself so the cache is invalidated as
    // part of the call rather than left to go stale, and but for the two surface
    // questions, which a record encoding answers as constants rather than
    // mirroring the bytes underneath. The same list, named once instead of at
    // five call sites.
    ($handle:ident, except_lifecycle) => {
        $crate::delegate_iobase!(@methods $handle: pread, pstream_bytes, pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
            ls, kind);
    };

    ($handle:ident: $($method:ident),+ $(,)?) => {
        $crate::delegate_iobase!(@methods $handle: $($method),+);
    };

    (@methods $handle:ident: $($method:ident),+ $(,)?) => {
        $($crate::delegate_iobase!(@method $handle, $method);)+
    };

    (@method $handle:ident, pread) => {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> $crate::Result<usize> {
            $crate::IOBase::pread(&self.$handle, offset, buffer)
        }
    };

    (@method $handle:ident, pstream_bytes) => {
        fn pstream_bytes(
            &self,
            position: u64,
            batch_size: usize,
        ) -> $crate::Result<$crate::ByteStream<'_>> {
            $crate::IOBase::pstream_bytes(&self.$handle, position, batch_size)
        }
    };

    (@method $handle:ident, pwrite) => {
        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> $crate::Result<usize> {
            $crate::IOBase::pwrite(&mut self.$handle, offset, bytes)
        }
    };

    (@method $handle:ident, size) => {
        fn size(&self) -> u64 {
            $crate::IOBase::size(&self.$handle)
        }
    };

    (@method $handle:ident, capacity) => {
        fn capacity(&self) -> u64 {
            $crate::IOBase::capacity(&self.$handle)
        }
    };

    (@method $handle:ident, reserve) => {
        fn reserve(&mut self, capacity: u64) -> $crate::Result<()> {
            $crate::IOBase::reserve(&mut self.$handle, capacity)
        }
    };

    (@method $handle:ident, truncate) => {
        fn truncate(&mut self, size: u64) -> $crate::Result<()> {
            $crate::IOBase::truncate(&mut self.$handle, size)
        }
    };

    (@method $handle:ident, url) => {
        fn url(&self) -> Option<&$crate::Url> {
            $crate::IOBase::url(&self.$handle)
        }
    };

    (@method $handle:ident, media_type) => {
        fn media_type(&self) -> &$crate::MediaType {
            $crate::IOBase::media_type(&self.$handle)
        }
    };

    (@method $handle:ident, set_media_type) => {
        fn set_media_type(&mut self, media_type: $crate::MediaType) {
            $crate::IOBase::set_media_type(&mut self.$handle, media_type);
        }
    };

    (@method $handle:ident, flush) => {
        fn flush(&mut self) -> $crate::Result<()> {
            $crate::IOBase::flush(&mut self.$handle)
        }
    };

    (@method $handle:ident, open) => {
        fn open(&mut self) -> $crate::Result<()> {
            $crate::IOBase::open(&mut self.$handle)
        }
    };

    (@method $handle:ident, opened) => {
        fn opened(&self) -> bool {
            $crate::IOBase::opened(&self.$handle)
        }
    };

    (@method $handle:ident, close) => {
        fn close(&mut self) -> $crate::Result<()> {
            $crate::IOBase::close(&mut self.$handle)
        }
    };

    (@method $handle:ident, parent) => {
        fn parent(&self) -> Option<$crate::holder::Holder> {
            $crate::IOBase::parent(&self.$handle)
        }
    };

    (@method $handle:ident, child_by_path) => {
        fn child_by_path(&self, name: &str) -> $crate::Result<$crate::holder::Holder> {
            $crate::IOBase::child_by_path(&self.$handle, name)
        }
    };

    (@method $handle:ident, ls) => {
        fn ls(&self, recursive: bool, include_private: bool) -> $crate::Listing {
            $crate::IOBase::ls(&self.$handle, recursive, include_private)
        }
    };

    (@method $handle:ident, kind) => {
        fn kind(&self) -> $crate::IOKind {
            $crate::IOBase::kind(&self.$handle)
        }
    };

    (@method $handle:ident, clear) => {
        fn clear(&mut self) -> $crate::Result<()> {
            $crate::IOBase::clear(&mut self.$handle)
        }
    };

    (@method $handle:ident, remove) => {
        fn remove(&mut self, recursive: bool) -> $crate::Result<()> {
            $crate::IOBase::remove(&mut self.$handle, recursive)
        }
    };

    (@method $handle:ident, is_atomic) => {
        fn is_atomic(&self) -> bool {
            $crate::IOBase::is_atomic(&self.$handle)
        }
    };

    (@method $handle:ident, is_tabular) => {
        fn is_tabular(&self) -> bool {
            $crate::IOBase::is_tabular(&self.$handle)
        }
    };

    (@method $handle:ident, is_io) => {
        fn is_io(&self) -> bool {
            $crate::IOBase::is_io(&self.$handle)
        }
    };
}

/// A streaming reader over an [`IOBase`], advancing its own offset.
pub struct Reader<'source> {
    pub(super) source: &'source dyn IOBase,
    pub(super) offset: u64,
}

impl Read for Reader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self
            .source
            .pread(self.offset, buffer)
            .map_err(std::io::Error::other)?;
        self.offset += read as u64;
        Ok(read)
    }
}

/// A streaming writer over an [`IOBase`], advancing its own offset.
pub struct Writer<'target> {
    pub(super) target: &'target mut dyn IOBase,
    pub(super) offset: u64,
}

impl Write for Writer<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self
            .target
            .pwrite(self.offset, bytes)
            .map_err(std::io::Error::other)?;
        self.offset += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        IOBase::flush(self.target).map_err(std::io::Error::other)
    }
}

impl IOMedia for Box<dyn IOBase> {
    fn as_io_base(&self) -> &dyn IOBase {
        self.as_ref()
    }

    fn as_io_base_mut(&mut self) -> &mut dyn IOBase {
        self.as_mut()
    }

    #[cfg(feature = "arrow")]
    fn row_size(&self) -> Result<u64> {
        IOMedia::row_size(self.as_ref())
    }

    #[cfg(feature = "arrow")]
    fn column_size(&self) -> Result<usize> {
        IOMedia::column_size(self.as_ref())
    }

    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<RecordOptions> {
        IOMedia::record_options(self.as_ref())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_statistics(&self) -> Result<crate::parquet::FileStatistics> {
        IOMedia::read_parquet_statistics(self.as_ref())
    }

    #[cfg(feature = "parquet")]
    fn read_parquet_geospatial_statistics(
        &self,
        column: &str,
    ) -> Result<crate::parquet::GeospatialStatistics> {
        IOMedia::read_parquet_geospatial_statistics(self.as_ref(), column)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        IOMedia::read_arrow_field(self.as_ref(), options)
    }

    #[cfg(feature = "arrow")]
    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        IOMedia::read_arrow_reader(self.as_ref(), options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_arrow_reader(self.as_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_prepared_arrow_reader(self.as_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_arrow_batch(self.as_mut(), batch, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::append_arrow_reader(self.as_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::append_arrow_batch(self.as_mut(), batch, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::merge_arrow_reader(self.as_mut(), batches, options)
    }

    #[cfg(feature = "arrow")]
    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::merge_arrow_batch(self.as_mut(), batch, options)
    }
}

impl IOBase for Box<dyn IOBase> {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_ref().pread(offset, buffer)
    }

    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        self.as_ref().pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.as_ref().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_ref().read_range_bytes(offset, length)
    }

    fn read_digest(&self, algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
        self.as_ref().read_digest(algorithm)
    }

    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        self.as_ref().read_range_digest(offset, length, algorithm)
    }

    fn clear(&mut self) -> Result<()> {
        self.as_mut().clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.as_mut().remove(recursive)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.as_mut().pwrite(offset, bytes)
    }

    fn size(&self) -> u64 {
        self.as_ref().size()
    }

    fn capacity(&self) -> u64 {
        self.as_ref().capacity()
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        self.as_mut().reserve(capacity)
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.as_mut().truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        self.as_ref().url()
    }

    fn media_type(&self) -> &MediaType {
        self.as_ref().media_type()
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.as_mut().set_media_type(media_type);
    }

    fn flush(&mut self) -> Result<()> {
        self.as_mut().flush()
    }

    fn open(&mut self) -> Result<()> {
        self.as_mut().open()
    }

    fn opened(&self) -> bool {
        self.as_ref().opened()
    }

    fn close(&mut self) -> Result<()> {
        self.as_mut().close()
    }

    fn parent(&self) -> Option<Holder> {
        self.as_ref().parent()
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.as_ref().child_by_path(path)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.as_ref().ls(recursive, include_private)
    }

    fn kind(&self) -> IOKind {
        self.as_ref().kind()
    }

    // The shape questions forward rather than deriving from this box's own
    // answers: a boxed folder is a folder, and the default would read the
    // trait's `kind` here rather than the one the value inside answers.
    fn is_atomic(&self) -> bool {
        self.as_ref().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_ref().is_tabular()
    }

    fn is_io(&self) -> bool {
        self.as_ref().is_io()
    }
}
