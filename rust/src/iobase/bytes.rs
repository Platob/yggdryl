//! Forwarding and boxed-handle implementations for [`IOBase`](super::IOBase).

use std::io::{Read, Write};

use super::IOBase;
use crate::holder::Holder;
use crate::media::RecordOptions;
use crate::{ByteStream, IOKind, IOMedia, Listing, MediaType, Result, Uri, Url};

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
/// [`IOBase::create_bytes`] has no default at all - only the store beneath
/// can make a create exclusive - so a list names it or the wrapper writes it.
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
///     yggdryl::delegate_iobase!(handle: pwrite, create_bytes, size, capacity, reserve,
///         truncate, uri, url, media_type, set_media_type, flush, parent, child_by_path,
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
    // The whole contract, lifecycle included: the wrapper changes nothing. The
    // whole-value reads and writes and the two digests are in the list because
    // leaving them out is not neutral - the trait's default answers them with
    // `size` plus positional calls, which on a decoding handle is a second
    // pass over the whole value and on a remote one hides the handle's own
    // request plan.
    ($handle:ident) => {
        $crate::delegate_iobase!(@methods $handle: pread, read_all_bytes, read_range_bytes,
            read_digest, read_range_digest, write_all_bytes, create_bytes, append_bytes,
            applied_codec, pstream_bytes, pwrite, size, capacity, reserve,
            truncate, uri, url, bound_location, mtime, media_type, set_media_type, flush, open, opened, close, parent, child_by_path,
            ls, kind, is_container, clear, remove, is_atomic, is_tabular, is_io);
    };

    // Everything but [`IOBase::clear`] and [`IOBase::remove`], which a wrapper
    // holding a cache of its own writes itself so the cache is invalidated as
    // part of the call rather than left to go stale, and but for the two surface
    // questions, which a record encoding answers as constants rather than
    // mirroring the bytes underneath. The same list, named once instead of at
    // five call sites.
    ($handle:ident, except_lifecycle) => {
        $crate::delegate_iobase!(@methods $handle: pread, pstream_bytes, pwrite, create_bytes, size, capacity, reserve,
            truncate, uri, url, bound_location, mtime, media_type, set_media_type, applied_codec, flush, open, opened, close,
            parent, child_by_path, ls, kind, is_container);
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

    (@method $handle:ident, read_all_bytes) => {
        fn read_all_bytes(&self) -> $crate::Result<Vec<u8>> {
            $crate::IOBase::read_all_bytes(&self.$handle)
        }
    };

    (@method $handle:ident, read_range_bytes) => {
        fn read_range_bytes(&self, offset: u64, length: usize) -> $crate::Result<Vec<u8>> {
            $crate::IOBase::read_range_bytes(&self.$handle, offset, length)
        }
    };

    (@method $handle:ident, read_digest) => {
        fn read_digest(
            &self,
            algorithm: $crate::DigestAlgorithm,
        ) -> $crate::Result<$crate::Digest> {
            $crate::IOBase::read_digest(&self.$handle, algorithm)
        }
    };

    (@method $handle:ident, read_range_digest) => {
        fn read_range_digest(
            &self,
            offset: u64,
            length: usize,
            algorithm: $crate::DigestAlgorithm,
        ) -> $crate::Result<$crate::Digest> {
            $crate::IOBase::read_range_digest(&self.$handle, offset, length, algorithm)
        }
    };

    (@method $handle:ident, write_all_bytes) => {
        fn write_all_bytes(&mut self, bytes: &[u8]) -> $crate::Result<()> {
            $crate::IOBase::write_all_bytes(&mut self.$handle, bytes)
        }
    };

    (@method $handle:ident, create_bytes) => {
        fn create_bytes(&mut self, bytes: &[u8]) -> $crate::Result<()> {
            $crate::IOBase::create_bytes(&mut self.$handle, bytes)
        }
    };

    (@method $handle:ident, append_bytes) => {
        fn append_bytes(&mut self, bytes: &[u8]) -> $crate::Result<u64> {
            $crate::IOBase::append_bytes(&mut self.$handle, bytes)
        }
    };

    (@method $handle:ident, applied_codec) => {
        fn applied_codec(&self) -> $crate::Codec {
            $crate::IOBase::applied_codec(&self.$handle)
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

    (@method $handle:ident, uri) => {
        fn uri(&self) -> Option<&$crate::Uri> {
            $crate::IOBase::uri(&self.$handle)
        }
    };

    (@method $handle:ident, url) => {
        fn url(&self) -> Option<&$crate::Url> {
            $crate::IOBase::url(&self.$handle)
        }
    };

    (@method $handle:ident, bound_location) => {
        fn bound_location(&self) -> Option<&$crate::fs::BoundLocation> {
            $crate::IOBase::bound_location(&self.$handle)
        }
    };

    (@method $handle:ident, mtime) => {
        fn mtime(&self) -> Option<i64> {
            $crate::IOBase::mtime(&self.$handle)
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

    (@method $handle:ident, is_container) => {
        fn is_container(&self) -> bool {
            $crate::IOBase::is_container(&self.$handle)
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

/// What a handle that resolves to nothing declares: no representation.
pub(crate) static UNRESOLVED_MEDIA_TYPE: std::sync::LazyLock<crate::MediaType> =
    std::sync::LazyLock::new(crate::MediaType::default);

/// Every [`IOBase`] verb but `uri` and `url`, forwarded to a handle resolved
/// on the first call that needs one: `$get` and `$get_mut` answer the
/// resolved [`Holder`](crate::holder::Holder) as a `Result`, and `$held` is
/// the `OnceLock` that keeps it. A verb that returns a `Result` carries the
/// resolution's failure; an accessor that cannot answers the empty value;
/// `opened` and `close` read the lock and resolve nothing. What
/// [`Uri`](crate::Uri) and the warehouse's [`Handle`](crate::Handle) share.
#[doc(hidden)]
#[macro_export]
macro_rules! __delegate_resolved_iobase {
    ($get:ident, $get_mut:ident, $held:ident) => {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> $crate::Result<usize> {
            $crate::IOBase::pread(self.$get()?, offset, buffer)
        }

        fn pstream_bytes(
            &self,
            position: u64,
            batch_size: usize,
        ) -> $crate::Result<$crate::ByteStream<'_>> {
            $crate::IOBase::pstream_bytes(self.$get()?, position, batch_size)
        }

        fn read_all_bytes(&self) -> $crate::Result<Vec<u8>> {
            $crate::IOBase::read_all_bytes(self.$get()?)
        }

        fn read_range_bytes(&self, offset: u64, length: usize) -> $crate::Result<Vec<u8>> {
            $crate::IOBase::read_range_bytes(self.$get()?, offset, length)
        }

        fn read_digest(
            &self,
            algorithm: $crate::DigestAlgorithm,
        ) -> $crate::Result<$crate::Digest> {
            $crate::IOBase::read_digest(self.$get()?, algorithm)
        }

        fn read_range_digest(
            &self,
            offset: u64,
            length: usize,
            algorithm: $crate::DigestAlgorithm,
        ) -> $crate::Result<$crate::Digest> {
            $crate::IOBase::read_range_digest(self.$get()?, offset, length, algorithm)
        }

        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> $crate::Result<usize> {
            $crate::IOBase::pwrite(self.$get_mut()?, offset, bytes)
        }

        fn write_all_bytes(&mut self, bytes: &[u8]) -> $crate::Result<()> {
            $crate::IOBase::write_all_bytes(self.$get_mut()?, bytes)
        }

        fn create_bytes(&mut self, bytes: &[u8]) -> $crate::Result<()> {
            $crate::IOBase::create_bytes(self.$get_mut()?, bytes)
        }

        fn append_bytes(&mut self, bytes: &[u8]) -> $crate::Result<u64> {
            $crate::IOBase::append_bytes(self.$get_mut()?, bytes)
        }

        fn size(&self) -> u64 {
            self.$get().map_or(0, $crate::IOBase::size)
        }

        fn capacity(&self) -> u64 {
            self.$get().map_or(0, $crate::IOBase::capacity)
        }

        fn reserve(&mut self, capacity: u64) -> $crate::Result<()> {
            $crate::IOBase::reserve(self.$get_mut()?, capacity)
        }

        fn truncate(&mut self, size: u64) -> $crate::Result<()> {
            $crate::IOBase::truncate(self.$get_mut()?, size)
        }

        fn bound_location(&self) -> Option<&$crate::fs::BoundLocation> {
            $crate::IOBase::bound_location(self.$get().ok()?)
        }

        fn mtime(&self) -> Option<i64> {
            $crate::IOBase::mtime(self.$get().ok()?)
        }

        fn media_type(&self) -> &$crate::MediaType {
            self.$get().map_or(
                &$crate::iobase::UNRESOLVED_MEDIA_TYPE,
                $crate::IOBase::media_type,
            )
        }

        fn applied_codec(&self) -> $crate::Codec {
            self.$get()
                .map_or($crate::Codec::Identity, $crate::IOBase::applied_codec)
        }

        /// Declare the resolved handle's representation. A handle that
        /// resolves to nothing has nothing to declare it on, and every
        /// operation that could read it names why.
        fn set_media_type(&mut self, media_type: $crate::MediaType) {
            if let Ok(held) = self.$get_mut() {
                $crate::IOBase::set_media_type(held, media_type);
            }
        }

        fn flush(&mut self) -> $crate::Result<()> {
            $crate::IOBase::flush(self.$get_mut()?)
        }

        fn open(&mut self) -> $crate::Result<()> {
            $crate::IOBase::open(self.$get_mut()?)
        }

        fn opened(&self) -> bool {
            self.$held
                .get()
                .is_some_and(|held| $crate::IOBase::opened(held.as_ref()))
        }

        fn close(&mut self) -> $crate::Result<()> {
            match self.$held.get_mut() {
                Some(held) => $crate::IOBase::close(held.as_mut()),
                // Nothing was resolved, so nothing was opened to close.
                None => Ok(()),
            }
        }

        fn clear(&mut self) -> $crate::Result<()> {
            $crate::IOBase::clear(self.$get_mut()?)
        }

        fn remove(&mut self, recursive: bool) -> $crate::Result<()> {
            $crate::IOBase::remove(self.$get_mut()?, recursive)
        }

        fn parent(&self) -> Option<$crate::holder::Holder> {
            $crate::IOBase::parent(self.$get().ok()?)
        }

        fn child_by_path(&self, path: &str) -> $crate::Result<$crate::holder::Holder> {
            $crate::IOBase::child_by_path(self.$get()?, path)
        }

        fn ls(&self, recursive: bool, include_private: bool) -> $crate::Listing {
            match self.$get() {
                Ok(held) => $crate::IOBase::ls(held, recursive, include_private),
                Err(error) => $crate::Listing::failing(error),
            }
        }

        fn glob(&self, pattern: &str, include_private: bool) -> $crate::Result<$crate::Listing> {
            $crate::IOBase::glob(self.$get()?, pattern, include_private)
        }

        fn partitions(&self) -> Vec<(String, String)> {
            self.$get()
                .map($crate::IOBase::partitions)
                .unwrap_or_default()
        }

        fn kind(&self) -> $crate::IOKind {
            self.$get()
                .map_or($crate::IOKind::Unknown, $crate::IOBase::kind)
        }

        fn is_container(&self) -> bool {
            self.$get().is_ok_and($crate::IOBase::is_container)
        }

        fn is_atomic(&self) -> bool {
            self.$get().is_ok_and($crate::IOBase::is_atomic)
        }

        fn is_tabular(&self) -> bool {
            self.$get().is_ok_and($crate::IOBase::is_tabular)
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

    /// Take the whole remainder in one call rather than in a ladder of them.
    ///
    /// [`Read::read_to_end`] grows its buffer by doubling and asks for what
    /// fits each time, so draining a value costs a call per doubling - on a
    /// store, a round trip per doubling, which for a four-megabyte object is
    /// seventeen of them. The handle answers the remainder in one.
    fn read_to_end(&mut self, into: &mut Vec<u8>) -> std::io::Result<usize> {
        let rest = rest_of(self.source, self.offset)?;
        self.offset += rest.len() as u64;
        let read = rest.len();
        into.extend_from_slice(&rest);
        Ok(read)
    }
}

/// Everything `source` holds from `offset`, in one call to it.
///
/// The remainder a ladder of [`IOBase::pread`] calls would read, so a
/// container - whose positional reads are empty while its whole reads stream
/// its leaves - answers nothing, and a [`Read`] never says end of stream to
/// `read` and hands `read_to_end` bytes.
pub(crate) fn rest_of(source: &dyn IOBase, offset: u64) -> std::io::Result<Vec<u8>> {
    if source.is_container() {
        return Ok(Vec::new());
    }
    let rest = if offset == 0 {
        source.read_all_bytes()
    } else {
        // Every implementation clamps a window to what is there, so asking
        // for the largest one asks for the remainder.
        source.read_range_bytes(offset, usize::MAX)
    };
    rest.map_err(std::io::Error::other)
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

    fn row_size(&self) -> Result<u64> {
        IOMedia::row_size(self.as_ref())
    }

    fn column_size(&self) -> Result<usize> {
        IOMedia::column_size(self.as_ref())
    }

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

    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        IOMedia::read_arrow_field(self.as_ref(), options)
    }

    // Forwarded, because a handle can answer its rows other than through its
    // bytes - an HTTP request walks the pages of a paginated document.
    fn read_serie(&self, options: Option<&RecordOptions>) -> Result<crate::SerieReader> {
        IOMedia::read_serie(&**self, options)
    }

    fn read_arrow_reader(&self, options: &RecordOptions) -> Result<crate::arrow::BatchReader> {
        IOMedia::read_arrow_reader(self.as_ref(), options)
    }

    fn overwrite_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::overwrite_arrow_reader(self.as_mut(), batches, options)
    }

    fn overwrite_prepared_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_prepared_arrow_reader(self.as_mut(), batches, options)
    }

    fn overwrite_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::overwrite_arrow_batch(self.as_mut(), batch, options)
    }

    fn append_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::append_arrow_reader(self.as_mut(), batches, options)
    }

    fn append_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::append_arrow_batch(self.as_mut(), batch, options)
    }

    fn merge_arrow_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
        IOMedia::merge_arrow_reader(self.as_mut(), batches, options)
    }

    fn merge_arrow_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<crate::IOResult> {
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

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_mut().write_all_bytes(bytes)
    }

    fn create_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_mut().create_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.as_mut().append_bytes(bytes)
    }

    fn applied_codec(&self) -> crate::Codec {
        self.as_ref().applied_codec()
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

    fn uri(&self) -> Option<&Uri> {
        self.as_ref().uri()
    }

    fn url(&self) -> Option<&Url> {
        self.as_ref().url()
    }

    fn bound_location(&self) -> Option<&crate::fs::BoundLocation> {
        self.as_ref().bound_location()
    }

    fn mtime(&self) -> Option<i64> {
        self.as_ref().mtime()
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
