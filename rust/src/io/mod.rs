//! Byte-oriented random-access I/O shared by every reader and writer.
//!
//! [`IOBase`] is the one storage abstraction in Yggdryl. It is deliberately
//! positional rather than cursor-based: `pread`/`pwrite` take an explicit
//! offset, so a footer-first container such as Parquet reads its index without
//! seeking a shared cursor, and two readers can share one handle without
//! coordinating. Everything else - streaming `Read`/`Write` adapters, whole-
//! value reads, compression - is derived from those two primitives.
//!
//! An implementation also carries the two things a caller needs to interpret
//! the bytes: an optional [`Url`] naming where they live, and a [`MediaType`]
//! naming what they are.
//!
//! The core ships [`Buffer`], an auto-scaling in-memory implementation, and
//! [`crate::local`], whose [`File`](crate::local::File) is an auto-resizing
//! memory-mapped local file. Two wrapping handles sit over any of them and are
//! handles themselves: [`Coded`] presents the decoded bytes of a compressed
//! resource, and [`crate::buffered::Buffered`] serves reads from a page cache
//! whose header and footer pages are pinned. Anything else - an object store,
//! an Arrow filesystem - implements the same trait outside the core.
//!
//! ```
//! use yggdryl::io::{Buffer, IOBase};
//!
//! # fn main() -> yggdryl::Result<()> {
//! let mut buffer = Buffer::new();
//! buffer.pwrite(0, b"symbol,price\n")?;
//! buffer.pwrite(13, b"AAPL,1\n")?;
//!
//! assert_eq!(buffer.size(), 20);
//!
//! // Positional reads never move a shared cursor.
//! let mut head = [0_u8; 6];
//! buffer.pread(0, &mut head)?;
//! assert_eq!(&head, b"symbol");
//! # Ok(())
//! # }
//! ```

use std::io::{Read, Write};

use crate::{Codec, Level};
use crate::{Error, IOKind, MediaType, MimeType, Result, Url};

mod buffer;
mod coding;
mod cursor;
mod listing;
mod media;
mod stream;
/// How a directory name spells a value that is not there.
///
/// A path cannot distinguish the absence of a value from the four letters, so
/// the convention has to pick one spelling and say what it costs: reading such a
/// partition back yields the text `null` unless a declared schema types the
/// column as something a cast turns back into a null.
///
/// It lives here rather than in [`partition`] because the expression layer's
/// scalar tier reads it and that tier compiles with no Arrow at all, while the
/// partition projection is Arrow's own. `partition` re-exports it, so the
/// spelling every caller already uses keeps working.
pub const NULL_PARTITION: &str = "null";

// The table formats join on a match key through exactly this implementation:
// one merge, whether the rows live in one leaf or in a snapshot's data files.
#[cfg(feature = "arrow")]
pub(crate) mod merge;
#[cfg(feature = "arrow")]
pub mod partition;
mod roles;

pub use buffer::Buffer;
pub use coding::Coded;
pub use cursor::{Cursor, IOCursor};
pub use listing::Listing;
pub use media::IOMedia;
pub use roles::{IOFile, IOFolder, IOPath};
pub use stream::ByteStream;

use crate::generic::Holder;
#[cfg(feature = "arrow")]
use crate::generic::RecordOptions;

/// Default byte-stream batch size used by core readers and language bindings.
pub const DEFAULT_STREAM_BATCH_SIZE: usize = 64 * 1024;

/// Bytes copied per step when moving between two handles.
const TRANSFER_CHUNK: usize = DEFAULT_STREAM_BATCH_SIZE;

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
/// use yggdryl::io::{Buffer, IOBase, IOMedia};
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
/// use yggdryl::io::{Buffer, IOBase, IOMedia};
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
/// use yggdryl::io::{Buffer, IOBase, IOMedia};
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
            $crate::io::IOBase::pread(&self.$handle, offset, buffer)
        }
    };

    (@method $handle:ident, pstream_bytes) => {
        fn pstream_bytes(
            &self,
            position: u64,
            batch_size: usize,
        ) -> $crate::Result<$crate::io::ByteStream<'_>> {
            $crate::io::IOBase::pstream_bytes(&self.$handle, position, batch_size)
        }
    };

    (@method $handle:ident, pwrite) => {
        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> $crate::Result<usize> {
            $crate::io::IOBase::pwrite(&mut self.$handle, offset, bytes)
        }
    };

    (@method $handle:ident, size) => {
        fn size(&self) -> u64 {
            $crate::io::IOBase::size(&self.$handle)
        }
    };

    (@method $handle:ident, capacity) => {
        fn capacity(&self) -> u64 {
            $crate::io::IOBase::capacity(&self.$handle)
        }
    };

    (@method $handle:ident, reserve) => {
        fn reserve(&mut self, capacity: u64) -> $crate::Result<()> {
            $crate::io::IOBase::reserve(&mut self.$handle, capacity)
        }
    };

    (@method $handle:ident, truncate) => {
        fn truncate(&mut self, size: u64) -> $crate::Result<()> {
            $crate::io::IOBase::truncate(&mut self.$handle, size)
        }
    };

    (@method $handle:ident, url) => {
        fn url(&self) -> Option<&$crate::Url> {
            $crate::io::IOBase::url(&self.$handle)
        }
    };

    (@method $handle:ident, media_type) => {
        fn media_type(&self) -> &$crate::MediaType {
            $crate::io::IOBase::media_type(&self.$handle)
        }
    };

    (@method $handle:ident, set_media_type) => {
        fn set_media_type(&mut self, media_type: $crate::MediaType) {
            $crate::io::IOBase::set_media_type(&mut self.$handle, media_type);
        }
    };

    (@method $handle:ident, flush) => {
        fn flush(&mut self) -> $crate::Result<()> {
            $crate::io::IOBase::flush(&mut self.$handle)
        }
    };

    (@method $handle:ident, open) => {
        fn open(&mut self) -> $crate::Result<()> {
            $crate::io::IOBase::open(&mut self.$handle)
        }
    };

    (@method $handle:ident, opened) => {
        fn opened(&self) -> bool {
            $crate::io::IOBase::opened(&self.$handle)
        }
    };

    (@method $handle:ident, close) => {
        fn close(&mut self) -> $crate::Result<()> {
            $crate::io::IOBase::close(&mut self.$handle)
        }
    };

    (@method $handle:ident, parent) => {
        fn parent(&self) -> Option<$crate::generic::Holder> {
            $crate::io::IOBase::parent(&self.$handle)
        }
    };

    (@method $handle:ident, child_by_path) => {
        fn child_by_path(&self, name: &str) -> $crate::Result<$crate::generic::Holder> {
            $crate::io::IOBase::child_by_path(&self.$handle, name)
        }
    };

    (@method $handle:ident, ls) => {
        fn ls(&self, recursive: bool, include_private: bool) -> $crate::io::Listing {
            $crate::io::IOBase::ls(&self.$handle, recursive, include_private)
        }
    };

    (@method $handle:ident, kind) => {
        fn kind(&self) -> $crate::IOKind {
            $crate::io::IOBase::kind(&self.$handle)
        }
    };

    (@method $handle:ident, clear) => {
        fn clear(&mut self) -> $crate::Result<()> {
            $crate::io::IOBase::clear(&mut self.$handle)
        }
    };

    (@method $handle:ident, remove) => {
        fn remove(&mut self, recursive: bool) -> $crate::Result<()> {
            $crate::io::IOBase::remove(&mut self.$handle, recursive)
        }
    };

    (@method $handle:ident, is_atomic) => {
        fn is_atomic(&self) -> bool {
            $crate::io::IOBase::is_atomic(&self.$handle)
        }
    };

    (@method $handle:ident, is_tabular) => {
        fn is_tabular(&self) -> bool {
            $crate::io::IOBase::is_tabular(&self.$handle)
        }
    };

    (@method $handle:ident, is_io) => {
        fn is_io(&self) -> bool {
            $crate::io::IOBase::is_io(&self.$handle)
        }
    };
}

/// Treat a backend's own not-found answer as a completed removal.
///
/// This is the shape [`IOBase::clear`] and [`IOBase::remove`] require: issue the
/// delete, and let the store say whether there was anything there. A backend
/// with a different absence signal - a store's no-such-key, an HTTP 404 - maps
/// its own answer the same way; everything else stays the typed failure it is,
/// because a permission or network error is not an absence.
///
/// ```
/// use yggdryl::io::skip_absent;
///
/// # fn main() -> yggdryl::Result<()> {
/// let absent = std::io::Error::from(std::io::ErrorKind::NotFound);
/// skip_absent(Err::<(), _>(absent))?;
///
/// let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
/// assert!(skip_absent(Err::<(), _>(denied)).is_err());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns every failure that is not [`std::io::ErrorKind::NotFound`].
pub fn skip_absent<T: Default>(result: std::io::Result<T>) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(Error::Io(error)),
    }
}

/// Refuse to delete a container that still holds children.
///
/// `remove(false)` on a populated container is an error naming the location and
/// the fact that it has children - never a silent success and never a silent
/// recursion.
pub fn not_empty(url: &Url) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::DirectoryNotEmpty,
        format!(
            "expected an empty container to remove, got {url}, which still has children; pass \
             recursive to delete them too"
        ),
    ))
}

/// Resolve a chain of fixed names below `base`, without touching anything.
///
/// Returns `None` for an empty chain, so a caller can tell "descend nowhere"
/// from "descend to here".
fn descend(base: &(impl IOBase + ?Sized), names: &[&str]) -> Result<Option<Holder>> {
    let Some((first, rest)) = names.split_first() else {
        return Ok(None);
    };
    let mut holder = base.child_by_path(first)?;
    for name in rest {
        holder = holder.child_by_path(name)?;
    }
    Ok(Some(holder))
}

/// Answer whether a container reads as one table, without listing the tree.
///
/// A folder reads as the table beneath it, so its leaves decide - and one leaf
/// is enough, because a partitioned tree is one table in one encoding. The walk
/// therefore descends towards the first leaf it can reach: every entry a level
/// already listed is checked before anything deeper is listed at all, so a lake
/// answers from the first partition that holds a file. Nothing is capped or
/// sampled; a container holding no tabular leaf anywhere is walked exactly as a
/// recursive listing would walk it, and answers `false` at the end of it.
///
/// A listing failure answers `false` rather than propagating: this is a
/// predicate, and a container nobody can list holds no rows anyone can read.
pub(crate) fn container_is_tabular(handle: &(impl IOBase + ?Sized)) -> bool {
    #[cfg(feature = "iceberg")]
    // A folder holding a table format is one tabular value however its files
    // are named, and asking costs one lookup of the metadata directory.
    if matches!(crate::iceberg::located(handle), Ok(Some(_))) {
        return true;
    }
    let mut level = handle.ls(false, false);
    // The frontier: the containers a level named and this walk has not opened
    // yet. It is bounded by the tree's width at the levels already listed, and
    // the walk stops at the first tabular leaf, so it is never the result.
    let mut deeper: Vec<Holder> = Vec::new();
    loop {
        for entry in level {
            let Ok(entry) = entry else {
                return false;
            };
            // The media type answers first because it is free, and no
            // container reports a tabular one - asking whether an entry is a
            // container is what costs a call into the backing store.
            if entry.media_type().is_tabular() {
                return true;
            }
            if entry.is_container() {
                deeper.push(entry);
            }
        }
        let Some(next) = deeper.pop() else {
            return false;
        };
        level = next.ls(false, false);
    }
}

/// Random-access byte storage addressed by explicit offsets.
///
/// # Laziness
///
/// **Constructing a handle must not touch the underlying resource.** A handle
/// is a description of where bytes would live, not proof that they do. Opening
/// a file, allocating a mapping, contacting a store, or reading metadata all
/// wait until an operation actually needs them, so building a handle never
/// fails for a resource that does not exist yet and never pays for one that is
/// never used.
///
/// Non-existence is therefore resolved at the operation, not at construction:
///
/// - **Reads skip.** [`Self::pread`] on a resource that does not exist yields
///   `0` bytes, exactly as reading past the end does. [`Self::size`] reports
///   `0`. Absence is emptiness, not an error, so a caller can probe a location
///   without a separate existence check.
/// - **Writes create.** [`Self::pwrite`], [`Self::truncate`], and
///   [`Self::reserve`] create the resource, and any parent it needs, on first
///   use.
///
/// Metadata follows the same rule. [`Self::media_type`] is computed on demand -
/// inferred from content or from a location - rather than eagerly at
/// construction, and re-derived after the bytes change.
///
/// # Invariants
///
/// - [`Self::pread`] returns a short count only at the end of the value; a
///   read entirely past [`Self::size`] returns `0`.
/// - [`Self::pwrite`] grows the value when the write extends past the end, and
///   zero-fills any gap an offset beyond the current size creates.
/// - [`Self::size`] never exceeds [`Self::capacity`].
pub trait IOBase: Send + IOMedia {
    /// Read into `buffer` starting at `offset`, returning the bytes read.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

    /// Stream byte arrays from `position`, reading at most `batch_size` bytes
    /// at a time.
    ///
    /// Construction performs no read and never asks for the value's size. Each
    /// item is a [`Result<Vec<u8>>`], so a backend failure arrives after every
    /// prefix already yielded and then the iterator stays fused. The final
    /// array may be short; an empty array is never yielded.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::ErrorKind::InvalidInput`] when `batch_size` is zero.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        ByteStream::from_handle(self, position, batch_size)
    }

    /// Write `bytes` at `offset`, returning the bytes written.
    ///
    /// Writing past the end grows the value; an offset beyond the current size
    /// zero-fills the gap.
    ///
    /// # Errors
    ///
    /// Returns the backing store's write or growth failure.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize>;

    /// Return the current byte length.
    fn size(&self) -> u64;

    /// Return the allocated capacity, which is never less than [`Self::size`].
    fn capacity(&self) -> u64;

    /// Grow the allocation so at least `capacity` bytes fit without another
    /// growth. Never shrinks and never changes [`Self::size`].
    ///
    /// # Errors
    ///
    /// Returns the backing store's allocation failure.
    fn reserve(&mut self, capacity: u64) -> Result<()>;

    /// Set the byte length, discarding beyond it or zero-filling up to it.
    ///
    /// # Errors
    ///
    /// Returns the backing store's resize failure.
    fn truncate(&mut self, size: u64) -> Result<()>;

    /// Return the canonical location, when the bytes have one.
    fn url(&self) -> Option<&Url>;

    /// Return the representation and content codings of the bytes.
    fn media_type(&self) -> &MediaType;

    /// Replace the declared representation and content codings.
    fn set_media_type(&mut self, media_type: MediaType);

    /// Commit buffered bytes to the backing store.
    ///
    /// # Errors
    ///
    /// Returns the backing store's flush failure.
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    /// Materialize the resource and cache the metadata reads depend on.
    ///
    /// A handle works without this: every operation materializes what it needs.
    /// Calling it explicitly moves that cost to a known point and keeps the
    /// cached state alive across many small operations instead of re-deriving
    /// it. It is the operation a scoped context binds to, so Python's
    /// `__enter__` and JavaScript's `using` map onto it directly.
    ///
    /// Opening an already-open handle is a no-op, and opening a resource that
    /// does not exist yet succeeds without creating it - creation still waits
    /// for the first write.
    ///
    /// # Errors
    ///
    /// Returns the backing store's open failure.
    fn open(&mut self) -> Result<()> {
        Ok(())
    }

    /// Return whether cached state is currently held.
    ///
    /// This reports a state of the *handle*, not a property of the resource -
    /// which is why it reads `opened` rather than `is_open`, matching the
    /// past-participle form the [`open`](Self::open)/[`close`](Self::close)
    /// pair already establishes. [`Self::is_container`] and [`Self::is_empty`]
    /// keep the `is_` prefix because they genuinely are predicates over the
    /// resource.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::new();
    /// // A plain buffer caches nothing, so it is never open.
    /// assert!(!handle.opened());
    /// assert!(handle.closed());
    /// handle.open()?;
    /// assert_eq!(handle.opened(), !handle.closed());
    /// # Ok(())
    /// # }
    /// ```
    fn opened(&self) -> bool {
        false
    }

    /// Return whether no cached state is currently held.
    ///
    /// Exactly `!`[`Self::opened`], so the pair can never disagree: this is a
    /// derived reading, never something an implementation answers
    /// independently.
    fn closed(&self) -> bool {
        !self.opened()
    }

    /// Flush and release everything [`Self::open`] cached.
    ///
    /// The handle stays usable; a later operation simply re-materializes. This
    /// is what `__exit__` binds to.
    ///
    /// # Errors
    ///
    /// Returns the backing store's flush or release failure.
    fn close(&mut self) -> Result<()> {
        self.flush()
    }

    /// Return the containing resource, when this one has a parent.
    ///
    /// A handle with no location, such as an in-memory buffer, has no parent.
    fn parent(&self) -> Option<Holder> {
        None
    }

    /// Return the descendant that `path` names, resolved against this resource.
    ///
    /// `path` is a *relative path*, not a single name: one segment reaches an
    /// immediate child, `sales/eu/orders` reaches three levels down, and `.`
    /// and `..` resolve the way they do in [`crate::UriPath::joinpath`]. The
    /// resource need not exist - per the laziness contract, reading a missing
    /// one is empty and writing one creates it and its parents.
    ///
    /// # Errors
    ///
    /// Returns an error when this resource cannot have children or `path` does
    /// not form a valid location.
    fn child_by_path(&self, path: &str) -> Result<Holder> {
        Err(no_children(self.url(), path))
    }

    /// List the resources contained by this one, one entry at a time.
    ///
    /// `recursive` descends into every container beneath this one. A resource
    /// that cannot contain others lists nothing rather than failing, so a
    /// caller can walk a tree without testing each node first.
    ///
    /// The listing is lazy: nothing is touched until the first
    /// [`next`](Iterator::next), and a caller that takes three entries from a
    /// folder of a hundred thousand pays for three. A failure arrives *as* an
    /// entry - the item is a [`Result`] - and the iterator is fused after it.
    /// A caller who wants a vector writes `.collect::<Result<Vec<_>>>()`; this
    /// never decides that for them.
    ///
    /// ```no_run
    /// use yggdryl::io::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(std::env::temp_dir().join("lake"))?;
    ///
    /// for entry in lake.ls(true, false).take(3) {
    ///     let _ = entry?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        let _ = (recursive, include_private);
        Listing::empty()
    }

    /// Return the locations beneath this one that match a glob `pattern`.
    ///
    /// The pattern is relative to this resource and anchored to it: `*` and `?`
    /// stay inside one name, a character class picks one character, and `**`
    /// spans any number of levels. The walk decomposes the pattern first, so a
    /// fixed prefix is *descended* rather than listed and filtered - matching
    /// `year=2024/**/*.parquet` under a lake reads one directory, not all of
    /// them.
    ///
    /// ```no_run
    /// use yggdryl::io::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(std::env::temp_dir().join("lake"))?;
    ///
    /// for part in lake.glob("year=2024/**/*.parquet", false)? {
    ///     println!("{}", part?.url().expect("a located child"));
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// A pattern whose fixed prefix loses therefore touches nothing beneath it:
    /// the first `next` descends the prefix and finds nothing to list.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the pattern cannot be decomposed, or when a fixed
    /// prefix segment cannot be resolved. Everything the walk itself hits
    /// arrives as a failing entry instead.
    fn glob(&self, pattern: &str, include_private: bool) -> Result<Listing> {
        let parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
        let Some(fixed) = parts.iter().position(|part| Url::is_pattern(part)) else {
            // Nothing to expand: the pattern names one location, which counts
            // only if something is actually there.
            let child = descend(self, &parts)?;
            return Ok(match child {
                Some(child) if child.kind() != IOKind::Unknown => {
                    Listing::new(std::iter::once(Ok(child)))
                }
                _ => Listing::empty(),
            });
        };
        if fixed > 0 {
            // Descend the fixed prefix so the listing starts as deep as it can.
            let Some(child) = descend(self, &parts[..fixed])? else {
                return Ok(Listing::empty());
            };
            return child.glob(&parts[fixed..].join("/"), include_private);
        }

        let Some(root) = self.url().cloned() else {
            return Ok(Listing::empty());
        };
        // One plain segment is answered by the immediate children; anything
        // deeper, or a `**`, needs the whole subtree.
        let recursive = parts.len() > 1 || parts[0] == "**";
        let pattern = pattern.to_owned();
        Ok(self.ls(recursive, include_private).keeping(move |entry| {
            entry
                .url()
                .is_some_and(|url| url.matches_glob_under(&root, &pattern))
        }))
    }

    /// Return the Hive partition pairs this resource's own location spells out.
    ///
    /// A Hive layout writes one directory per partition column, so a handle
    /// deep in a lake already knows the column values its rows share.
    fn partitions(&self) -> Vec<(String, String)> {
        self.url().map(Url::hive_partitions).unwrap_or_default()
    }

    /// Iterate the entries beneath this one a predicate does not rule out.
    ///
    /// The predicate is asked of the *holder*, not of the rows: `&holder.name`,
    /// `&holder.partition['year']`, `&holder.size`, and anything built from
    /// them. A conjunct that reads a row column is not answerable from a
    /// listing, so it is dropped rather than guessed at - which means this can
    /// keep a file the rows will later discard, and can never discard a file
    /// the rows would have kept.
    ///
    /// Cost drives the order. [`bind`](crate::Expression::bind) puts the free
    /// attributes - the ones a URL answers - in front of the ones that cost a
    /// stat, and evaluation stops at the first `false`, so a listing filtered
    /// by path alone performs no call into the backing store at all.
    ///
    /// ```no_run
    /// use yggdryl::io::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(std::env::temp_dir().join("lake"))?;
    ///
    /// let filter = "&holder.partition['year'] = '2024' and &holder.extension = 'parquet'"
    ///     .parse()?;
    /// for part in lake.children_matching(&filter, false)? {
    ///     let _ = part;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a bind failure when the predicate names something a holder
    /// cannot answer, or the backing store's listing failure.
    fn children_matching(
        &self,
        filter: &crate::Expression,
        include_private: bool,
    ) -> Result<Listing> {
        // Only the conjuncts a listing can settle are kept. Dropping a conjunct
        // from a conjunction only ever widens what is kept, which is the whole
        // reason this is sound.
        let answerable = crate::Expression::all(
            filter
                .conjuncts()
                .into_iter()
                .filter(|conjunct| conjunct.columns().is_empty()),
        );
        let bound = answerable.bind(&crate::DataType::from_fields([])?.required_field("holder"))?;
        // The predicate is asked of each entry as it arrives, so a losing entry
        // is dropped before the next one is fetched and nothing accumulates.
        Ok(Listing::new(
            self.ls(true, include_private)
                .map(move |entry| {
                    let entry = entry?;
                    Ok((
                        bound.matches_holder(&crate::expression::Handle(&entry))?,
                        entry,
                    ))
                })
                .filter_map(|matched| match matched {
                    Ok((true, entry)) => Some(Ok(entry)),
                    Ok((false, _)) => None,
                    Err(error) => Some(Err(error)),
                }),
        ))
    }

    /// Iterate the leaves beneath this one that carry every given partition.
    ///
    /// This is the handle a partitioned write reaches for: select the parts of
    /// a lake that hold one partition, then overwrite or upsert them directly
    /// instead of rewriting the table. Containers are not yielded - only the
    /// resources that hold bytes - and an empty filter yields every leaf.
    ///
    /// The pairs are sugar: each one builds
    /// `&holder.partition['column'] is not null and`
    /// `&holder.partition['column'] = 'value'` and the whole thing is answered
    /// by [`children_matching`](Self::children_matching). There is no second
    /// filter behind them. The null test is what makes this *select* rather
    /// than *prune*: a leaf whose path never names the column is not one of
    /// the leaves that carry it.
    ///
    /// ```no_run
    /// use yggdryl::io::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(std::env::temp_dir().join("lake"))?;
    ///
    /// for part in lake.children_where(&[("year", "2024")], false)? {
    ///     part?.clear()?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing failure.
    fn children_where(&self, filters: &[(&str, &str)], include_private: bool) -> Result<Listing> {
        let filter = crate::Expression::all_holder_partitions_carried(filters.iter().copied());
        Ok(self
            .children_matching(&filter, include_private)?
            .keeping(|entry| !entry.is_container()))
    }

    /// Return what kind of resource this handle addresses.
    ///
    /// A generic handle reads this to pick the specialized implementation to
    /// work through; everything else uses it to tell a container from a leaf
    /// without probing the backend twice.
    fn kind(&self) -> IOKind {
        IOKind::File
    }

    /// Return whether this resource can contain others.
    fn is_container(&self) -> bool {
        self.kind().is_container()
    }

    /// Return whether this resource is one whole byte value.
    ///
    /// *Atomic* names the byte surface: the value [`Self::read_all_bytes`]
    /// reads whole and [`Self::write_all_bytes`] replaces whole. It is the
    /// complement of [`Self::is_tabular`] wherever bytes are held - a resource
    /// is read as bytes or as rows, never as both - and both answer `false` for
    /// the containers that hold neither, a directory of unrelated files as much
    /// as a namespace or a catalog.
    ///
    /// The cheapest evidence answers first, so a name that already spells a
    /// record encoding costs no call into the backing store at all. A location
    /// nothing has decided yet answers from its name exactly as its media type
    /// does: under the laziness contract absence is emptiness, not a third
    /// shape.
    ///
    /// ```
    /// use yggdryl::MimeType;
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// let mut notes = Buffer::new();
    /// notes.set_media_type(MimeType::PLAIN_TEXT.into());
    /// assert!(notes.is_atomic());
    /// assert!(!notes.is_tabular());
    ///
    /// let mut trades = Buffer::new();
    /// trades.set_media_type(MimeType::PARQUET.into());
    /// assert!(trades.is_tabular());
    /// assert!(!trades.is_atomic());
    /// ```
    fn is_atomic(&self) -> bool {
        let media = self.media_type();
        !media.is_tabular() && !media.base().is_directory() && !self.kind().is_container()
    }

    /// Return whether this resource holds rows and columns.
    ///
    /// *Tabular* names the record surface: what
    /// [`read_arrow_reader`](IOMedia::read_arrow_reader) decodes and
    /// what its two writing siblings encode. The question is about the
    /// representation rather than about this build - a `.parquet` leaf is
    /// tabular whether or not the `parquet` feature is compiled in, and
    /// [`record_options`](IOMedia::record_options) is the call that reports an
    /// encoding this build cannot decode, naming it.
    ///
    /// Cost drives the order, so the cheapest evidence answers first:
    ///
    /// - the media type, which a name already spells, settles every leaf and
    ///   every location nothing has decided yet, with no call into the backing
    ///   store;
    /// - [`IOKind::Table`] settles a table format's folder outright, because it
    ///   is one tabular value however its files happen to be named, and
    ///   [`IOKind::Namespace`] and [`IOKind::Catalog`] settle the containers
    ///   that hold only containers;
    /// - only a plain [`IOKind::Directory`] is probed, and the probe stops at
    ///   the first leaf that settles the question rather than listing the tree,
    ///   because a folder reads as the table beneath it and a partitioned tree
    ///   is one table in one encoding.
    ///
    /// ```
    /// use yggdryl::MimeType;
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// let mut trades = Buffer::new();
    /// trades.set_media_type(MimeType::ARROW_FILE.into());
    /// assert!(trades.is_tabular());
    /// ```
    fn is_tabular(&self) -> bool {
        // A representation that spells rows and columns is the answer whatever
        // the backing store would say, and it is free: no container reports a
        // tabular media type, so nothing here can mistake one for a leaf.
        if self.media_type().is_tabular() {
            return true;
        }
        match self.kind() {
            IOKind::Table => true,
            IOKind::Directory => container_is_tabular(self),
            _ => false,
        }
    }

    /// Return whether this handle exposes bytes or rows.
    ///
    /// A container holding neither surface answers `false`; byte leaves and
    /// tabular values answer `true` through their existing shape methods.
    fn is_io(&self) -> bool {
        self.is_atomic() || self.is_tabular()
    }

    /// Return whether the value holds no bytes.
    fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Return the content coding the media type declares.
    fn codec(&self) -> Codec {
        Codec::from_media_type(self.media_type())
    }

    /// Fill `buffer` completely from `offset`.
    ///
    /// # Errors
    ///
    /// Returns an error naming the shortfall when the value ends first.
    fn pread_exact(&self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        let mut filled = 0;
        while filled < buffer.len() {
            let read = self.pread(offset + filled as u64, &mut buffer[filled..])?;
            if read == 0 {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!(
                        "expected {} bytes at offset {offset}, got {filled}",
                        buffer.len()
                    ),
                )));
            }
            filled += read;
        }
        Ok(())
    }

    /// Read the complete value into memory.
    ///
    /// This is the reading half of the whole-value byte pair whose writing half
    /// is [`Self::write_all_bytes`]; both name `bytes` because both are about
    /// the bytes rather than the rows, which
    /// [`read_arrow_reader`](IOMedia::read_arrow_reader) and its
    /// siblings answer. [`Self::is_atomic`] is how a caller asks which of the
    /// two surfaces a handle is for.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        let size = usize::try_from(self.size()).map_err(|_| oversized(self.size()))?;
        let mut bytes = vec![0_u8; size];
        self.pread_exact(0, &mut bytes)?;
        Ok(bytes)
    }

    /// Decode one structured [`Scalar`](crate::Scalar) from this handle.
    ///
    /// The media type selects JSON, YAML, or TOML and its content coding. A
    /// `field` directs parsing and casting; without one the value is inferred.
    /// Reading stays streamed through [`Self::pstream_bytes`], including for a
    /// compressed handle.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    /// use yggdryl::{Field, Url, Scalar};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let media = Url::from_str("file:///trade.json.gz")?.media_type();
    /// let mut handle = Buffer::new().with_media_type(media);
    /// let value = Scalar::from_record([("quantity", Scalar::I64(2))])?;
    /// handle.write_value(&value)?;
    ///
    /// let field = Field::from_str("trade: struct<quantity: int64 not null> not null")?;
    /// assert_eq!(handle.read_value(None)?, value);
    /// assert_eq!(
    ///     handle.read_value(Some(&field))?,
    ///     Scalar::from_sequence([Scalar::I64(2)])
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a read, decompression, format, parse, or field-cast failure.
    fn read_value(&self, field: Option<&crate::Field>) -> Result<crate::Scalar> {
        match field {
            Some(field) => crate::text::from_io_with_field(self, field),
            None => crate::text::from_io(self),
        }
    }

    /// Encode one structured [`Scalar`](crate::Scalar), replacing this handle.
    ///
    /// The media type selects JSON, YAML, or TOML and its content coding.
    ///
    /// # Errors
    ///
    /// Returns a format, representation, compression, or write failure.
    fn write_value(&mut self, value: &crate::Scalar) -> Result<()> {
        crate::text::into_io(value, self)
    }

    /// Read `length` bytes starting at `offset`.
    ///
    /// The result is short only when the value ends first.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn read_range(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        let available = self.size().saturating_sub(offset);
        let length = length.min(usize::try_from(available).unwrap_or(usize::MAX));
        let mut bytes = vec![0_u8; length];
        self.pread_exact(offset, &mut bytes)?;
        Ok(bytes)
    }

    /// Write every byte at `offset`.
    ///
    /// # Errors
    ///
    /// Returns the backing store's write failure.
    fn pwrite_all(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        let mut written = 0;
        while written < bytes.len() {
            let count = self.pwrite(offset + written as u64, &bytes[written..])?;
            if count == 0 {
                return Err(Error::Io(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    format!(
                        "expected to write {} bytes at offset {offset}, stalled after {written}",
                        bytes.len()
                    ),
                )));
            }
            written += count;
        }
        Ok(())
    }

    /// Append bytes after the current end, returning the offset they start at.
    ///
    /// # Errors
    ///
    /// Returns the backing store's write failure.
    fn append(&mut self, bytes: &[u8]) -> Result<u64> {
        let offset = self.size();
        self.pwrite_all(offset, bytes)?;
        Ok(offset)
    }

    /// Replace the complete value with `bytes`.
    ///
    /// The writing half of the pair [`Self::read_all_bytes`] reads.
    ///
    /// A whole-value write is a *complete* operation, so it ends with
    /// [`Self::flush`]: a handle that over-allocates - the memory-mapped
    /// [`local::File`](crate::local::File) grows geometrically so appending
    /// does not remap on every write - must not leave that slack visible to a
    /// second handle on the same location, which would read the padding as
    /// content. Positional [`Self::pwrite`] deliberately does not publish;
    /// it is the primitive a larger operation is built from, and the operation
    /// publishes when it finishes.
    ///
    /// # Errors
    ///
    /// Returns the backing store's resize or write failure.
    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.truncate(0)?;
        self.pwrite_all(0, bytes)?;
        self.flush()
    }

    /// Empty the resource's contents, keeping the resource itself.
    ///
    /// The meaning follows [`Self::kind`], and it is *stated* here rather than
    /// implied by a byte operation:
    ///
    /// - A leaf ([`IOKind::File`], [`IOKind::Memory`]) discards every byte.
    ///   The resource still exists afterwards, with [`Self::size`] `0`.
    /// - A container ([`IOKind::Directory`]) removes every child, recursively.
    ///   The container itself still exists afterwards, and is empty.
    /// - A resource that does not exist succeeds, having done nothing. It is
    ///   *not* created: clearing is not a write.
    ///
    /// Any cache [`Self::open`] filled - a schema, a footer, compiled options -
    /// is invalidated as part of the call, so a later read cannot serve an
    /// answer describing bytes that are gone.
    ///
    /// See [`Self::remove`] for the no-pre-call rule both lifecycle methods
    /// follow.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::from_bytes(b"AAPL,1".to_vec());
    /// handle.clear()?;
    ///
    /// // Emptied, not deleted.
    /// assert_eq!(handle.size(), 0);
    /// assert!(handle.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the backing store's failure to empty the resource. Absence is
    /// never one of them.
    fn clear(&mut self) -> Result<()> {
        self.truncate(0)
    }

    /// Delete the resource completely.
    ///
    /// `remove` is not "delete a file". It is the one *total* removal action
    /// on this abstraction: after it returns, nothing of the resource this
    /// handle addresses remains, whatever that resource is for this
    /// implementation. The signature is generic and each implementation adapts
    /// the mechanics, but no implementation removes only part of what it
    /// addresses - a wrapping handle removes what it *wraps*, not merely its
    /// own view of it, and any pending write, staged buffer, or live mapping is
    /// dropped as part of the removal so a later flush cannot resurrect what
    /// was deleted.
    ///
    /// - On a leaf, the resource is deleted. `recursive` is irrelevant and
    ///   ignored.
    /// - On a container with `recursive`, everything below it and the
    ///   container itself are deleted.
    /// - On a container without `recursive`, the container is deleted only if
    ///   it is already empty; a non-empty one is an error naming the location
    ///   and the fact that it has children. It never silently succeeds and
    ///   never silently recurses.
    /// - On a resource that does not exist, it succeeds, having done nothing.
    ///
    /// Afterwards the handle stays usable and lazy: writing through it
    /// recreates the resource exactly as a never-touched handle would, and any
    /// cache [`Self::open`] filled is gone rather than waiting to be refreshed.
    ///
    /// # Absence is a no-op success, reached without a probe
    ///
    /// This is the requirement that shapes every implementation. An
    /// implementation **issues the delete and treats the backend's own
    /// not-found answer as success**. It must not call [`Self::kind`],
    /// [`Self::size`], an exists-style check, [`Self::ls`], or any other probe
    /// first to decide whether to proceed: on a remote backend every such probe
    /// is a second round trip on the hot path, and a recursive delete over a
    /// large tree turns into a flood of them. Where a backend needs a different
    /// call for a leaf than for a container, the handle's own static role
    /// answers which - [`local::File`](crate::local::File) is a file,
    /// [`local::Folder`](crate::local::Folder) is a directory - so the dispatch
    /// is on the type, not on a probe. The one documented exception is a
    /// generic path handle such as [`local::Path`](crate::local::Path), whose
    /// whole job is to report [`IOKind`] from what is actually there: it routes
    /// on the kind it *already* resolves, and adds no second probe for the
    /// delete.
    ///
    /// Only the not-found failure maps to `Ok(())` -
    /// [`std::io::ErrorKind::NotFound`] locally, and the backend equivalents (a
    /// store's no-such-key, an HTTP 404) elsewhere. A permission, network, or
    /// busy failure surfaces as the typed error it is; a blanket
    /// `let _ = ...` is not an implementation of this rule.
    ///
    /// # Absence and successful removal are indistinguishable
    ///
    /// The return is `Result<()>`, never a bool or a count of what was deleted.
    /// Reporting "did something exist" would force exactly the probe this
    /// design refuses, so it is not reported. That is the contract, not an
    /// omission.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::from_bytes(b"AAPL,1".to_vec());
    /// handle.remove(false)?;
    /// assert_eq!(handle.size(), 0);
    ///
    /// // Removing again succeeds, having done nothing: absence is not an error.
    /// handle.remove(false)?;
    ///
    /// // The handle stays usable and lazy - a write recreates the resource.
    /// handle.write_all_bytes(b"MSFT,2")?;
    /// assert_eq!(handle.read_all_bytes()?, b"MSFT,2");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure, or a refusal naming a
    /// non-empty container when `recursive` is not set. Absence is never one of
    /// them.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        // A handle owning nothing beyond its own bytes removes exactly those.
        // Every implementation addressing an external resource overrides this;
        // the trait cannot delete what it has no primitive for.
        let _ = recursive;
        self.clear()
    }

    /// Copy this value's bytes into `target`, replacing its contents.
    ///
    /// Returns the number of bytes copied. Transfer is chunked, so neither
    /// side is buffered whole.
    ///
    /// # Errors
    ///
    /// Returns the first read or write failure.
    fn copy_into(&self, target: &mut dyn IOBase) -> Result<u64> {
        target.truncate(0)?;
        let mut source = self.pstream_bytes(0, TRANSFER_CHUNK)?;
        let mut chunk = vec![0_u8; TRANSFER_CHUNK];
        let mut offset = 0_u64;
        loop {
            let read = source.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            target.pwrite_all(offset, &chunk[..read])?;
            offset = offset.checked_add(read as u64).ok_or_else(|| {
                Error::Io(std::io::Error::other("copied byte stream exceeds u64::MAX"))
            })?;
        }
        target.set_media_type(self.media_type().clone());
        target.flush()?;
        Ok(offset)
    }

    /// Encode this value into `target` with `codec`, replacing its contents.
    ///
    /// The target's media type records the added coding, so a later
    /// [`Self::decompress_into`] needs no out-of-band knowledge.
    ///
    /// # Errors
    ///
    /// Returns the first read, encode, or write failure.
    fn compress_into(&self, target: &mut dyn IOBase, codec: Codec) -> Result<u64> {
        self.compress_into_with_level(target, codec, Level::DEFAULT)
    }

    /// Encode into `target` at an explicit level.
    ///
    /// # Errors
    ///
    /// Returns the first read, encode, or write failure.
    fn compress_into_with_level(
        &self,
        target: &mut dyn IOBase,
        codec: Codec,
        level: Level,
    ) -> Result<u64> {
        target.truncate(0)?;
        {
            let writer = Writer { target, offset: 0 };
            let mut encoder = codec.writer_with_level(writer, level);
            let mut source = self.pstream_bytes(0, TRANSFER_CHUNK)?;
            let mut chunk = vec![0_u8; TRANSFER_CHUNK];
            loop {
                let read = source.read(&mut chunk)?;
                if read == 0 {
                    break;
                }
                encoder.write_all(&chunk[..read])?;
            }
            encoder.finish()?;
        }
        target.flush()?;
        let media_type = self
            .media_type()
            .clone()
            .try_with_encodings(coding_mime(codec))?;
        target.set_media_type(media_type);
        Ok(target.size())
    }

    /// Decode this value into `target`, replacing its contents.
    ///
    /// The coding comes from this value's media type; use
    /// [`Self::decompress_into_with`] to override it.
    ///
    /// # Errors
    ///
    /// Returns the first read, decode, or write failure.
    fn decompress_into(&self, target: &mut dyn IOBase) -> Result<u64> {
        self.decompress_into_with(target, self.codec())
    }

    /// Decode into `target` with an explicit coding.
    ///
    /// # Errors
    ///
    /// Returns the first read, decode, or write failure.
    fn decompress_into_with(&self, target: &mut dyn IOBase, codec: Codec) -> Result<u64> {
        target.truncate(0)?;
        let source = self.pstream_bytes(0, TRANSFER_CHUNK)?;
        let mut decoder = codec.reader(source);
        let mut decoded = vec![0_u8; TRANSFER_CHUNK];
        let mut offset = 0_u64;
        loop {
            let read = decoder.read(&mut decoded)?;
            if read == 0 {
                break;
            }
            target.pwrite_all(offset, &decoded[..read])?;
            offset = offset.checked_add(read as u64).ok_or_else(|| {
                Error::Io(std::io::Error::other(
                    "decoded byte stream exceeds u64::MAX",
                ))
            })?;
        }
        target.flush()?;
        // The decoded bytes carry the base representation with the coding
        // removed, which is exactly the media type minus its last encoding.
        let mut media_type = self.media_type().clone();
        let mut encodings = media_type.encodings().to_vec();
        encodings.pop();
        media_type.set_encodings(encodings)?;
        target.set_media_type(media_type);
        Ok(offset)
    }

    /// Borrow a streaming reader positioned at `offset`.
    fn reader_at(&self, offset: u64) -> Reader<'_>
    where
        Self: Sized,
    {
        Reader {
            source: self,
            offset,
        }
    }

    /// Consume this handle into a [`Cursor`] positioned at the start.
    ///
    /// The cursor owns one explicit position - `tell`, `seek`, and reads and
    /// writes that advance it - and stays a full handle over the same bytes.
    fn cursor(self) -> Cursor<Self>
    where
        Self: Sized,
    {
        Cursor::new(self)
    }

    /// Consume this handle into a [`Cursor`] positioned at `position`.
    fn cursor_at(self, position: u64) -> Cursor<Self>
    where
        Self: Sized,
    {
        Cursor::at(self, position)
    }

    /// Consume this handle into a page-cached [`Buffered`](crate::buffered::Buffered) one.
    ///
    /// Reads are served from fixed-size pages held under a byte budget, with
    /// the first page and the page holding the last byte pinned so a
    /// header-and-footer access pattern never re-reads either end. Everything
    /// else answers exactly as this handle does.
    ///
    /// [`Buffered`](crate::buffered::Buffered) shadows this with an inherent
    /// method of the same name, so
    /// buffering an already-buffered handle re-wraps the handle it holds
    /// rather than stacking a second cache.
    ///
    /// ```
    /// use yggdryl::buffered::BufferedOptions;
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::from_bytes(b"symbol,price\n".to_vec())
    ///     .buffered(BufferedOptions::default());
    /// assert_eq!(handle.read_range(0, 6)?, b"symbol");
    ///
    /// // The second read of the same bytes reaches no further than memory.
    /// assert_eq!(handle.read_range(0, 6)?, b"symbol");
    /// assert_eq!(handle.cached_pages(), 1);
    /// # Ok(())
    /// # }
    /// ```
    fn buffered(self, options: crate::buffered::BufferedOptions) -> crate::buffered::Buffered<Self>
    where
        Self: Sized,
    {
        crate::buffered::Buffered::new(self, options)
    }

    /// Borrow a streaming writer positioned at `offset`.
    fn writer_at(&mut self, offset: u64) -> Writer<'_>
    where
        Self: Sized,
    {
        Writer {
            target: self,
            offset,
        }
    }

    /// Wrap this handle in the text-line handler.
    ///
    /// The entry point to [`text::line`](crate::text::line): every line read and
    /// write goes through the handler this returns.
    /// [`Text::new`](crate::text::Text::new) is lazy exactly as
    /// [`Coded::new`](Coded) is - the resource is not opened, listed, or probed
    /// here.
    ///
    /// **Idempotent.** A handle that is already a [`Text`](crate::text::Text)
    /// returns itself unchanged, because `Text` carries an inherent
    /// `into_text` and inherent methods win method resolution over trait ones.
    /// So `handle.into_text().into_text()` is `handle.into_text()`, and a
    /// caller who does not know whether they hold a raw handle or an
    /// already-wrapped one can call it unconditionally.
    ///
    /// One edge that resolution rule leaves open: an explicit
    /// `IOBase::into_text(text_handle)` still wraps, producing a
    /// `Text<Text<H>>`. That composition behaves correctly through delegation -
    /// it is wasteful, not broken - so call the method normally.
    ///
    /// It composes with the coding handles: `gzip_handle.into_text()` is a
    /// `Text<Gzip<_>>`, and the codings are peeled as streams underneath.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::new().into_text();
    /// handle.write_lines(["one", "two"])?;
    ///
    /// // A constructor and one method call: no format strings, no mode flags.
    /// let mut lines = handle.read_lines()?;
    /// assert_eq!(lines.next().transpose()?.map(|line| line.bytes()), Some(&b"one"[..]));
    /// # Ok(())
    /// # }
    /// ```
    fn into_text(self) -> crate::text::Text<Self>
    where
        Self: Sized,
    {
        crate::text::Text::new(self)
    }

    /// [`Self::into_text`] with an extractor already in hand.
    ///
    /// The one-call form, for a caller who built the options elsewhere - a
    /// configuration document, a shared constant.
    fn into_text_with(self, options: crate::text::TextLineOptions) -> crate::text::Text<Self>
    where
        Self: Sized,
    {
        crate::text::Text::with_options(self, options)
    }

    /// Iterate the resource's text records, one in memory at a time.
    ///
    /// A thin shim over [`Self::into_text`], so there is exactly one
    /// implementation of line splitting in the tree. Bytes stream through a
    /// bounded window and any content codings the media type declares are
    /// peeled as streaming decoders - a `trades.jsonl.gz` reads its records
    /// without ever holding the decompressed value. A resource that does not
    /// exist yields no records, exactly as it reads zero bytes.
    ///
    /// # Errors
    ///
    /// Construction itself cannot fail; each yielded item carries the read or
    /// decode failure of its record.
    fn read_lines(&self) -> Result<crate::text::TextLines<Box<dyn Read + '_>>>
    where
        Self: Sized,
    {
        Ok(crate::text::line::borrowed_lines(
            self,
            std::sync::Arc::new(crate::text::TextLineOptions::new()),
        ))
    }

    /// Group the resource's records by a pattern, one in memory at a time.
    ///
    /// A thin shim over [`Self::into_text`] with a record-opening pattern. A
    /// record starts at a line `pattern` matches and carries every following
    /// line until the next match, which is how a log whose entries open with a
    /// timestamp reads whole: an entry, its stack trace, and its wrapped lines
    /// arrive as one record. Lines before the first match form the first
    /// record.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse; each yielded item
    /// carries the read or decode failure of its record.
    fn read_lines_matching(
        &self,
        pattern: &str,
    ) -> Result<crate::text::TextLines<Box<dyn Read + '_>>>
    where
        Self: Sized,
    {
        let options = crate::text::TextLineOptions::with_pattern(pattern)?;
        Ok(crate::text::line::borrowed_lines(
            self,
            std::sync::Arc::new(options),
        ))
    }

    /// [`Self::read_lines`], consuming the handle so the records own it.
    ///
    /// # Errors
    ///
    /// Construction itself cannot fail; each yielded item carries the read or
    /// decode failure of its record.
    fn into_read_lines(self) -> Result<crate::text::TextLines<Box<dyn Read + Send + 'static>>>
    where
        Self: Sized + 'static,
    {
        self.into_text().into_read_lines()
    }

    /// [`Self::read_lines_matching`], consuming the handle so the records own it.
    ///
    /// # Errors
    ///
    /// Returns an error when the pattern does not parse.
    fn into_read_lines_matching(
        self,
        pattern: &str,
    ) -> Result<crate::text::TextLines<Box<dyn Read + Send + 'static>>>
    where
        Self: Sized + 'static,
    {
        let options = crate::text::TextLineOptions::with_pattern(pattern)?;
        self.into_text_with(options).into_read_lines()
    }

    /// Project the resource's text records into Arrow batches.
    ///
    /// The extractor-first spelling of the record surface: text is a record
    /// encoding like any other, and [`IOMedia::read_arrow_reader`] under
    /// [`RecordOptions::Text`] reaches
    /// this very method - **not a fourth record method**. Decoding views
    /// (a [`Coded`] handle, [`Gzip`](crate::gzip::Gzip) and its siblings)
    /// override it to own the encoded location behind one streaming decoder;
    /// an opened or dirty view snapshots only the presented value it already
    /// holds. The
    /// columns are described on [`TextLineOptions`](crate::text::TextLineOptions).
    ///
    /// # Errors
    ///
    /// Returns a listing or reopen failure; each yielded batch carries the
    /// read, decode, or parse failure of its rows.
    #[cfg(feature = "arrow")]
    fn read_arrow_lines(
        &self,
        options: &crate::text::TextLineOptions,
    ) -> Result<crate::arrow::BatchReader> {
        crate::text::line::arrow::read_arrow_lines(self, options)
    }

    /// [`Self::read_arrow_lines`], consuming the handle so the reader owns it.
    ///
    /// This is the shape the bindings hand across FFI, and the one a Rust
    /// caller needs when the batches outlive the scope that built the handle.
    ///
    /// # Errors
    ///
    /// Returns a listing failure; each yielded batch carries the read, decode,
    /// or parse failure of its rows.
    #[cfg(feature = "arrow")]
    fn into_arrow_lines(
        self,
        options: &crate::text::TextLineOptions,
    ) -> Result<crate::arrow::BatchReader>
    where
        Self: Sized + 'static,
    {
        crate::text::line::arrow::into_arrow_lines(self, options)
    }
}

/// The default append implementation after an encoding-specific boundary has
/// validated its option variant.
///
/// Stateful media override the trait method only to perform that validation
/// before the source reader is pulled, then call this shared implementation.
#[cfg(feature = "arrow")]
pub(crate) fn append_arrow_reader_default(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    options.require_write_mode(crate::IOMode::Append)?;
    let commit_row_size = options.require_commit_row_size()?;
    options.require_write_limits()?;
    if options.write_limit_is_zero() {
        return Ok(());
    }
    // An empty append is a true no-op: discover it before asking the handle
    // whether it is a table or folder, so no location probe or listing runs.
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    let container = handle.is_container();
    if container {
        #[cfg(feature = "iceberg")]
        if let Some(mut table) = crate::iceberg::located(handle)? {
            return table.append_arrow_reader(batches, options);
        }
        return append_arrow_reader_folder(handle, batches, options, commit_row_size);
    }
    let (batches, delegated, target) = prepare_leaf_arrow_write(handle, batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            match &target {
                Some(target) => append_leaf_onto(handle, commit?, &delegated, target)?,
                None => append_leaf(handle, commit?, &delegated)?,
            }
        }
        return Ok(());
    }
    match target {
        Some(target) => append_leaf_onto(handle, batches, &delegated, &target),
        None => append_leaf(handle, batches, &delegated),
    }
}

/// The default merge implementation after an encoding-specific boundary has
/// validated its option variant.
#[cfg(feature = "arrow")]
pub(crate) fn merge_arrow_reader_default(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    options.require_write_mode(crate::IOMode::Merge)?;
    let commit_row_size = options.require_commit_row_size()?;
    options.require_write_limits()?;
    // Key and limit intent is deterministic and has already been validated;
    // only then may an empty merge end without touching its destination.
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    let container = handle.is_container();
    if container {
        #[cfg(feature = "iceberg")]
        if let Some(mut table) = crate::iceberg::located(handle)? {
            return table.merge_arrow_reader(batches, options);
        }
        return merge_arrow_reader_folder(handle, batches, options, commit_row_size);
    }
    let (batches, delegated, target) = prepare_leaf_arrow_write(handle, batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            match &target {
                Some(target) => merge_leaf_onto(
                    handle,
                    commit?,
                    &delegated,
                    options.merge_by_names(),
                    target,
                )?,
                None => merge_leaf(handle, commit?, &delegated, options.merge_by_names())?,
            }
        }
        return Ok(());
    }
    match target {
        Some(target) => merge_leaf_onto(
            handle,
            batches,
            &delegated,
            options.merge_by_names(),
            &target,
        ),
        None => merge_leaf(handle, batches, &delegated, options.merge_by_names()),
    }
}

/// The common overwrite implementation for byte and folder handles.
///
/// [`crate::io::IOMedia::overwrite_arrow_reader`] is required so a media or table format
/// can make publication one native operation. Implementations whose only
/// publication primitive is the byte surface call this function; it performs
/// all generic shaping and reaches exactly one encoding writer.
///
/// # Errors
///
/// Returns a field, cast, listing, encoding, or write failure. A non-empty
/// match key is refused because overwrite never guesses merge intent.
#[cfg(feature = "arrow")]
#[doc(hidden)]
pub fn overwrite_arrow_reader_default(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    overwrite_arrow_reader_default_with_field(handle, batches, options).map(|_| ())
}

/// Run the default overwrite and return the logical field actually published.
///
/// Stateful media use this to refresh an already-open metadata cache without
/// rereading the encoded value. The field is resolved by the same shaping pass
/// that consumes `batches`: declared-field casting and selection happen once,
/// then an existing stored field completes the result. `None` is reserved for
/// a table-format redirection whose own commit owns its metadata cache.
#[cfg(feature = "arrow")]
pub(crate) fn overwrite_arrow_reader_default_with_field(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<Option<crate::Field>> {
    use crate::generic::IORecordOptions;

    options.require_write_mode(crate::IOMode::Overwrite)?;
    let commit_row_size = options.require_commit_row_size()?;
    let container = handle.is_container();
    if container {
        #[cfg(feature = "iceberg")]
        if let Some(mut table) = crate::iceberg::located(handle)? {
            table.overwrite_arrow_reader(batches, options)?;
            return Ok(None);
        }
        return overwrite_arrow_reader_folder(handle, batches, options, commit_row_size).map(Some);
    }
    let (batches, delegated, target) = prepare_leaf_arrow_write(handle, batches, options)?;
    let schema = batches.schema();
    let published = target
        .clone()
        .or(Some(crate::arrow::field_from_arrow_schema(
            delegated.root_name(),
            schema.as_ref(),
        )?));
    if commit_row_size.is_some() {
        let mut commits = options.commit_arrow_readers(batches)?;
        let Some(first) = commits.next() else {
            // Overwrite is the one intent for which an empty input still
            // publishes its shaped schema and clears the prior rows.
            handle.overwrite_prepared_arrow_reader(
                crate::arrow::batch_reader(schema, []),
                &delegated,
            )?;
            return Ok(published);
        };
        handle.overwrite_prepared_arrow_reader(first?, &delegated)?;
        // Replacing every cadence would retain only the last one. Once the
        // first prefix is visible, later overwrite cadences are appends.
        for commit in commits {
            match &target {
                Some(target) => append_leaf_onto(handle, commit?, &delegated, target)?,
                None => append_leaf(handle, commit?, &delegated)?,
            }
        }
        return Ok(published);
    }
    handle.overwrite_prepared_arrow_reader(batches, &delegated)?;
    Ok(published)
}

/// Append through one folder routing plan shared by every publication cadence.
#[cfg(feature = "arrow")]
fn append_arrow_reader_folder(
    folder: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    commit_row_size: Option<usize>,
) -> Result<()> {
    let mut writer = partition::FolderWriter::new(folder, options)?;
    let (batches, delegated, declared) = prepare_arrow_write(batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    writer.set_options(routing_options(delegated, declared))?;
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            writer.append(folder, commit?)?;
        }
        return Ok(());
    }
    writer.append(folder, batches)
}

/// Merge through one folder routing plan shared by every publication cadence.
#[cfg(feature = "arrow")]
fn merge_arrow_reader_folder(
    folder: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    commit_row_size: Option<usize>,
) -> Result<()> {
    // Layout resolves before shaping or mutation because it decides whether
    // at least one merge key remains inside each leaf. The top-level no-op
    // peek has retained the first row-bearing batch without advancing past it.
    let mut writer = partition::FolderWriter::new(folder, options)?;
    let (batches, delegated, declared) = prepare_arrow_write(batches, options)?;
    let Some(batches) = non_empty_arrow_reader(batches)? else {
        return Ok(());
    };
    writer.set_options(routing_options(delegated, declared))?;
    if commit_row_size.is_some() {
        for commit in options.commit_arrow_readers(batches)? {
            writer.merge(folder, commit?)?;
        }
        return Ok(());
    }
    writer.merge(folder, batches)
}

/// Overwrite through one folder routing plan shared by every publication cadence.
#[cfg(feature = "arrow")]
fn overwrite_arrow_reader_folder(
    folder: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    commit_row_size: Option<usize>,
) -> Result<crate::Field> {
    use crate::generic::IORecordOptions;

    let mut writer = partition::FolderWriter::new(folder, options)?;
    let (batches, delegated, declared) = prepare_arrow_write(batches, options)?;
    let schema = batches.schema();
    let published = crate::arrow::field_from_arrow_schema(delegated.root_name(), schema.as_ref())?;
    writer.set_options(routing_options(delegated, declared))?;
    if commit_row_size.is_none() {
        writer.overwrite(folder, batches)?;
        return Ok(published);
    }

    let mut commits = options.commit_arrow_readers(batches)?;
    let Some(first) = commits.next() else {
        writer.overwrite(folder, crate::arrow::batch_reader(schema, []))?;
        return Ok(published);
    };
    writer.overwrite(folder, first?)?;
    // Only the first cadence replaces the addressed tree. Every later prefix
    // extends the same top-level overwrite using the routing plan above.
    for commit in commits {
        writer.append(folder, commit?)?;
    }
    Ok(published)
}

/// Shape one incoming write stream and return options safe for delegation.
///
/// The declared field and selection are applied before the limits. The field
/// is then *taken* from the clone, and every other consumed shaping option is
/// cleared - including `commit_row_size` - so a default append or merge can
/// publish through an implementor's required overwrite hook without applying
/// an incoming-only transform to the stored rows, splitting recursively, or
/// casting the incoming rows twice.
#[cfg(feature = "arrow")]
pub(crate) fn prepare_arrow_write(
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<(
    crate::arrow::BatchReader,
    RecordOptions,
    Option<crate::Field>,
)> {
    prepare_arrow_write_onto(batches, options, None)
}

/// Shape one incoming stream and safely complete it onto a stored field once.
///
/// Table formats use this seam before splitting publication cadences. Their
/// native commit may defensively inspect the exact shape again, but every
/// declared cast, selection, limit, and safe stored-field completion has
/// already happened here over the one streaming reader.
#[cfg(feature = "arrow")]
pub(crate) fn prepare_arrow_write_onto(
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
    existing: Option<&crate::Field>,
) -> Result<(
    crate::arrow::BatchReader,
    RecordOptions,
    Option<crate::Field>,
)> {
    use crate::generic::IORecordOptions;

    let batches = options.cast_arrow_reader(batches, existing)?;
    let batches = options.limit_arrow_reader(batches)?;
    let mut delegated = options.clone();
    let declared = delegated.take_field();
    delegated.set_select_by_names(Vec::new());
    delegated.set_max_row_size(None);
    delegated.set_max_byte_size(None);
    delegated.set_commit_row_size(None);
    Ok((batches, delegated, declared))
}

/// Shape one leaf stream onto a target resolved exactly once for the write.
///
/// A stored field completes the cast before global limits are applied. A
/// missing leaf takes the shaped reader's field as its target; text remains
/// schema-less and uses its native append implementation. The returned
/// options have every incoming-only transform removed and are safe for the
/// prepared publication hook.
#[cfg(feature = "arrow")]
fn prepare_leaf_arrow_write(
    handle: &(impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<(
    crate::arrow::BatchReader,
    RecordOptions,
    Option<crate::Field>,
)> {
    use crate::generic::IORecordOptions;

    let stored = if matches!(options, RecordOptions::Text(_)) {
        None
    } else {
        stored_field(handle, options)?
    };
    let (batches, delegated, _) = prepare_arrow_write_onto(batches, options, stored.as_ref())?;
    let target = match stored {
        Some(stored) => Some(stored),
        None if matches!(options, RecordOptions::Text(_)) => None,
        None => Some(crate::arrow::field_from_arrow_schema(
            delegated.root_name(),
            batches.schema().as_ref(),
        )?),
    };
    Ok((batches, delegated, target))
}

/// One resumable, cadence-bounded Arrow write used by asynchronous bindings.
///
/// The session owns option shaping, global row/byte counters, the incomplete
/// cadence, and any folder or table routing plan. It deliberately does not
/// borrow the destination: a binding may leave Rust while awaiting its next
/// source chunk, then temporarily pass the same handle back to [`push`](Self::push)
/// or [`finish`](Self::finish). Complete cadences publish synchronously before
/// either method returns; [`abort`](Self::abort) drops only the unpublished
/// remainder.
///
/// This is hidden because it is a narrow runtime bridge, not another write
/// operation. Its mode is the same public [`crate::IOMode`] accepted by the
/// generic media entry points.
#[cfg(feature = "arrow")]
#[doc(hidden)]
pub struct ArrowWriteSession {
    mode: crate::IOMode,
    options: RecordOptions,
    delegated: RecordOptions,
    declared: Option<crate::Field>,
    limit: crate::generic::WriteLimitState,
    commit_row_size: usize,
    input_schema: Option<arrow_schema::SchemaRef>,
    shaped_schema: Option<arrow_schema::SchemaRef>,
    buffer: Option<crate::generic::CommitBuffer>,
    target: Option<ArrowWriteTarget>,
    published: bool,
    input_complete: bool,
    terminal: bool,
}

/// Destination state that must stay stable while an async source is awaited.
#[cfg(feature = "arrow")]
enum ArrowWriteTarget {
    /// A schema-bearing leaf after its one stable target is resolved.
    Leaf { stored: crate::Field },
    /// A missing schema-bearing leaf before the input schema is shaped.
    EmptyLeaf,
    /// Text lines have no stored field; append remains their native operation.
    TextLeaf,
    Folder {
        writer: Box<partition::FolderWriter>,
    },
    #[cfg(feature = "iceberg")]
    Iceberg {
        located: Box<crate::iceberg::Located>,
        stored: crate::Field,
    },
}

#[cfg(feature = "arrow")]
impl ArrowWriteSession {
    /// Start an overwrite session without touching a destination or source.
    pub fn overwrite(options: &RecordOptions) -> Result<Self> {
        Self::new(crate::IOMode::Overwrite, options)
    }

    /// Start an append session without touching a destination or source.
    pub fn append(options: &RecordOptions) -> Result<Self> {
        Self::new(crate::IOMode::Append, options)
    }

    /// Start a merge session without touching a destination or source.
    pub fn merge(options: &RecordOptions) -> Result<Self> {
        Self::new(crate::IOMode::Merge, options)
    }

    /// Start a session for one explicit write mode.
    pub fn new(mode: crate::IOMode, options: &RecordOptions) -> Result<Self> {
        use crate::generic::IORecordOptions;

        options.require_write_mode(mode)?;
        let commit_row_size =
            options
                .require_commit_row_size()?
                .ok_or_else(|| Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$.commit_row_size"),
                    reason: crate::text::expected_got(
                        "a non-zero commit_row_size for a resumable write session",
                        "an unset commit_row_size",
                    ),
                })?;
        options.require_write_limits()?;
        let mut delegated = options.clone();
        let declared = delegated.take_field();
        delegated.set_select_by_names(Vec::new());
        delegated.set_max_row_size(None);
        delegated.set_max_byte_size(None);
        delegated.set_commit_row_size(None);
        Ok(Self {
            mode,
            options: options.clone(),
            delegated,
            declared,
            limit: crate::generic::WriteLimitState::new(
                options.max_row_size(),
                options.max_byte_size(),
            ),
            commit_row_size,
            input_schema: None,
            shaped_schema: None,
            buffer: None,
            target: None,
            published: false,
            input_complete: false,
            terminal: false,
        })
    }

    /// Admit one Arrow chunk, publishing every complete cadence before return.
    ///
    /// The boolean is `true` while the binding should request another chunk.
    /// `false` means a global row or byte limit completed the logical input;
    /// no later source item may be inspected.
    pub fn push(
        &mut self,
        handle: &mut (impl IOBase + ?Sized),
        mut batches: crate::arrow::BatchReader,
    ) -> Result<bool> {
        use crate::generic::IORecordOptions as _;
        use arrow_array::RecordBatchReader as _;

        self.require_live()?;
        if self.input_complete {
            return Ok(false);
        }
        let input_schema = batches.schema();
        if let Some(expected) = &self.input_schema {
            if expected.as_ref() != input_schema.as_ref() {
                let expected = format!("{expected:?}");
                let got = format!("{input_schema:?}");
                self.abort();
                return Err(Error::InvalidRecord {
                    path: smol_str::SmolStr::new_static("$"),
                    reason: crate::text::expected_got(
                        format_args!("the first asynchronous chunk schema {expected}"),
                        format_args!("a later chunk schema {got}"),
                    ),
                });
            }
        } else {
            self.input_schema = Some(std::sync::Arc::clone(&input_schema));
        }
        if let Err(error) = self.ensure_shaped_schema(handle, input_schema) {
            self.abort();
            return Err(error);
        }

        while !self.limit.satisfied() {
            let batch = match batches.next() {
                Some(Ok(batch)) => batch,
                Some(Err(error)) => {
                    self.abort();
                    return Err(crate::arrow::from_reader_error(error).into());
                }
                None => break,
            };
            let batch = match self.options.cast_arrow_batch(batch, self.target_field()) {
                Ok(batch) => batch,
                Err(error) => {
                    self.abort();
                    return Err(error);
                }
            };
            let Some(batch) = self.limit.apply(batch) else {
                break;
            };
            if batch.num_rows() != 0 {
                if let Some(reader) = self
                    .buffer
                    .as_mut()
                    .expect("a shaped session owns a commit buffer")
                    .push(batch)
                {
                    if let Err(error) = self.publish(handle, reader) {
                        self.abort();
                        return Err(error);
                    }
                }
                if let Err(error) = self.publish_ready(handle) {
                    self.abort();
                    return Err(error);
                }
            }
            if self.limit.satisfied() {
                if let Err(error) = self.complete_input(handle) {
                    self.abort();
                    return Err(error);
                }
                return Ok(false);
            }
        }
        if self.limit.satisfied() {
            if let Err(error) = self.complete_input(handle) {
                self.abort();
                return Err(error);
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Publish the final incomplete cadence and complete the session.
    pub fn finish(&mut self, handle: &mut (impl IOBase + ?Sized)) -> Result<()> {
        use crate::generic::IORecordOptions as _;

        self.require_live()?;
        if !self.input_complete {
            if let Err(error) = self.complete_input(handle) {
                self.abort();
                return Err(error);
            }
        }
        if self.mode == crate::IOMode::Overwrite && !self.published {
            if self.shaped_schema.is_none() {
                let field = match self.options.require_field() {
                    Ok(field) => field.clone(),
                    Err(error) => {
                        self.abort();
                        return Err(error);
                    }
                };
                let schema = match field.into_arrow_schema() {
                    Ok(schema) => schema,
                    Err(error) => {
                        self.abort();
                        return Err(error.into());
                    }
                };
                self.input_schema = Some(std::sync::Arc::clone(&schema));
                if let Err(error) = self.ensure_shaped_schema(handle, schema) {
                    self.abort();
                    return Err(error);
                }
            }
            let schema = std::sync::Arc::clone(
                self.shaped_schema
                    .as_ref()
                    .expect("an empty overwrite has a shaped schema"),
            );
            if let Err(error) = self.publish(
                handle,
                crate::arrow::batch_reader(schema, std::iter::empty()),
            ) {
                self.abort();
                return Err(error);
            }
        }
        self.terminal = true;
        Ok(())
    }

    /// Drop the unpublished partial cadence while retaining prior commits.
    pub fn abort(&mut self) {
        if let Some(buffer) = &mut self.buffer {
            buffer.clear();
        }
        self.terminal = true;
    }

    fn require_live(&self) -> Result<()> {
        if self.terminal {
            return Err(Error::InvalidRecord {
                path: smol_str::SmolStr::new_static("$"),
                reason: smol_str::SmolStr::new_static(
                    "an Arrow write session cannot be reused after finish, abort, or failure",
                ),
            });
        }
        Ok(())
    }

    fn ensure_target(&mut self, handle: &(impl IOBase + ?Sized)) -> Result<()> {
        if self.target.is_some() {
            return Ok(());
        }
        if handle.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(located) = crate::iceberg::located(handle)? {
                let stored = located.stored_field()?;
                self.target = Some(ArrowWriteTarget::Iceberg {
                    located: Box::new(located),
                    stored,
                });
                return Ok(());
            }
            let mut writer = partition::FolderWriter::new(handle, &self.options)?;
            writer.set_options(routing_options(
                self.delegated.clone(),
                self.declared.clone(),
            ))?;
            self.target = Some(ArrowWriteTarget::Folder {
                writer: Box::new(writer),
            });
        } else if matches!(self.delegated, RecordOptions::Text(_)) {
            self.target = Some(ArrowWriteTarget::TextLeaf);
        } else {
            self.target = Some(match stored_field(handle, &self.delegated)? {
                Some(stored) => ArrowWriteTarget::Leaf { stored },
                None => ArrowWriteTarget::EmptyLeaf,
            });
        }
        Ok(())
    }

    fn target_field(&self) -> Option<&crate::Field> {
        match self.target.as_ref() {
            Some(ArrowWriteTarget::Leaf { stored }) => Some(stored),
            Some(ArrowWriteTarget::EmptyLeaf)
            | Some(ArrowWriteTarget::TextLeaf)
            | Some(ArrowWriteTarget::Folder { .. })
            | None => None,
            #[cfg(feature = "iceberg")]
            Some(ArrowWriteTarget::Iceberg { stored, .. }) => Some(stored),
        }
    }

    fn ensure_shaped_schema(
        &mut self,
        handle: &(impl IOBase + ?Sized),
        input_schema: arrow_schema::SchemaRef,
    ) -> Result<()> {
        self.ensure_target(handle)?;
        if self.shaped_schema.is_some() {
            return Ok(());
        }
        use crate::generic::IORecordOptions as _;
        let empty = arrow_array::RecordBatch::new_empty(input_schema);
        let shaped = self.options.cast_arrow_batch(empty, self.target_field())?;
        let schema = shaped.schema();
        // A missing leaf acquires the first shaped schema as this session's
        // target. Unlike an ordinary one-shot call, resumed cadences must not
        // re-plan against a resource another handle changed between awaits.
        if matches!(self.target, Some(ArrowWriteTarget::EmptyLeaf)) {
            self.target = Some(ArrowWriteTarget::Leaf {
                stored: crate::arrow::field_from_arrow_schema(
                    self.delegated.root_name(),
                    schema.as_ref(),
                )?,
            });
        }
        self.buffer = Some(crate::generic::CommitBuffer::new(
            std::sync::Arc::clone(&schema),
            self.commit_row_size,
        ));
        self.shaped_schema = Some(schema);
        Ok(())
    }

    fn publish_ready(&mut self, handle: &mut (impl IOBase + ?Sized)) -> Result<()> {
        loop {
            let ready = self
                .buffer
                .as_mut()
                .and_then(crate::generic::CommitBuffer::next_ready);
            let Some(reader) = ready else { return Ok(()) };
            self.publish(handle, reader)?;
        }
    }

    fn complete_input(&mut self, handle: &mut (impl IOBase + ?Sized)) -> Result<()> {
        if self.input_complete {
            return Ok(());
        }
        self.publish_ready(handle)?;
        let remainder = self
            .buffer
            .as_mut()
            .and_then(crate::generic::CommitBuffer::finish);
        if let Some(reader) = remainder {
            self.publish(handle, reader)?;
        }
        self.input_complete = true;
        Ok(())
    }

    fn publish(
        &mut self,
        handle: &mut (impl IOBase + ?Sized),
        batches: crate::arrow::BatchReader,
    ) -> Result<()> {
        use crate::generic::IORecordOptions as _;

        let mode = match (self.mode, self.published) {
            (crate::IOMode::Overwrite, true) => crate::IOMode::Append,
            (mode, _) => mode,
        };
        match self
            .target
            .as_mut()
            .expect("a publishing session has resolved its target")
        {
            ArrowWriteTarget::Leaf { stored } => match mode {
                crate::IOMode::Overwrite => {
                    handle.overwrite_prepared_arrow_reader(batches, &self.delegated)?
                }
                crate::IOMode::Append => {
                    append_leaf_onto(handle, batches, &self.delegated, stored)?
                }
                crate::IOMode::Merge => merge_leaf_onto(
                    handle,
                    batches,
                    &self.delegated,
                    self.delegated.merge_by_names(),
                    stored,
                )?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
            ArrowWriteTarget::TextLeaf => match mode {
                crate::IOMode::Overwrite => {
                    handle.overwrite_prepared_arrow_reader(batches, &self.delegated)?
                }
                crate::IOMode::Append => append_leaf(handle, batches, &self.delegated)?,
                crate::IOMode::Merge => merge_leaf(
                    handle,
                    batches,
                    &self.delegated,
                    self.delegated.merge_by_names(),
                )?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
            ArrowWriteTarget::EmptyLeaf => {
                unreachable!("a shaped session has resolved an empty leaf field")
            }
            ArrowWriteTarget::Folder { writer } => match mode {
                crate::IOMode::Overwrite => writer.overwrite(handle, batches)?,
                crate::IOMode::Append => writer.append(handle, batches)?,
                crate::IOMode::Merge => writer.merge(handle, batches)?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
            #[cfg(feature = "iceberg")]
            ArrowWriteTarget::Iceberg { located, .. } => match mode {
                crate::IOMode::Overwrite => {
                    located.overwrite_prepared(batches, self.delegated.safe())?
                }
                crate::IOMode::Append => located.append_prepared(batches)?,
                crate::IOMode::Merge => located.merge_prepared(
                    batches,
                    self.delegated.merge_by_names(),
                    self.delegated.safe(),
                )?,
                crate::IOMode::ReadOnly | crate::IOMode::Random => {
                    return Err(crate::Error::InvalidRecord {
                        path: smol_str::SmolStr::new_static("$.mode"),
                        reason: smol_str::SmolStr::new_static(
                            "write mode readonly or random is not supported for this operation",
                        ),
                    });
                }
            },
        }
        self.published = true;
        Ok(())
    }
}

/// Peek until the first row-bearing batch without losing it or its schema.
///
/// Append and merge use this before touching a handle. A reader that ends (or
/// yields only zero-row batches) is a true no-op, while a first real batch is
/// returned ahead of the untouched remainder. At most that one batch is held.
#[cfg(feature = "arrow")]
pub(crate) fn non_empty_arrow_reader(
    mut batches: crate::arrow::BatchReader,
) -> Result<Option<crate::arrow::BatchReader>> {
    use arrow_array::RecordBatchReader as _;

    let schema = batches.schema();
    loop {
        let Some(batch) = batches.next() else {
            return Ok(None);
        };
        let batch = batch.map_err(crate::arrow::from_reader_error)?;
        if batch.num_rows() == 0 {
            continue;
        }
        return Ok(Some(Box::new(PrefixedBatchReader {
            schema,
            first: Some(batch),
            rest: batches,
        })));
    }
}

/// One peeked batch followed by the source it came from.
#[cfg(feature = "arrow")]
struct PrefixedBatchReader {
    schema: arrow_schema::SchemaRef,
    first: Option<arrow_array::RecordBatch>,
    rest: crate::arrow::BatchReader,
}

#[cfg(feature = "arrow")]
impl Iterator for PrefixedBatchReader {
    type Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.first.take().map(Ok).or_else(|| self.rest.next())
    }
}

#[cfg(feature = "arrow")]
impl arrow_array::RecordBatchReader for PrefixedBatchReader {
    fn schema(&self) -> arrow_schema::SchemaRef {
        std::sync::Arc::clone(&self.schema)
    }
}

/// Restore a declared field only for partition-layout discovery.
#[cfg(feature = "arrow")]
fn routing_options(mut delegated: RecordOptions, declared: Option<crate::Field>) -> RecordOptions {
    use crate::generic::IORecordOptions;

    if let Some(field) = declared {
        delegated.set_field(field);
    }
    delegated
}

/// Decode one leaf, pushing the declared schema down and casting what returns.
///
/// This is the only place a record read reaches an encoding.
#[cfg(feature = "arrow")]
/// Narrow a reader to the columns the options select, in the order they name.
///
/// An empty selection is the reader as it stands - the common case pays one
/// slice borrow and nothing else. A non-empty one builds a target root holding
/// exactly the named columns of the reader's own schema, resolved ASCII
/// case-insensitively the way every cast matches names, and casts each batch
/// onto it - which is a projection, because the columns keep their datatypes.
/// A name the schema does not have is an error listing what is there, because
/// a selection is a claim about the rows rather than a wish.
#[cfg(feature = "arrow")]
pub(crate) fn select_reader(
    reader: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<crate::arrow::BatchReader> {
    use crate::generic::IORecordOptions;

    let names = options.select_by_names();
    if names.is_empty() {
        return Ok(reader);
    }
    let root =
        crate::arrow::field_from_arrow_schema(options.root_name(), reader.schema().as_ref())?;
    match crate::arrow::selected_root(&root, names, options.root_name())? {
        Some(target) => Ok(crate::arrow::cast_reader(reader, &target, options.safe())?),
        None => Ok(reader),
    }
}

#[cfg(feature = "arrow")]
pub(crate) fn leaf_reader(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<crate::arrow::BatchReader> {
    use crate::generic::IORecordOptions;

    let declared = options.field();
    let reader = match options {
        RecordOptions::Ipc(ipc) => crate::ipc::read_batch_reader(handle, declared, ipc)?,
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => {
            crate::parquet::read_batch_reader(handle, declared, parquet)?
        }
        RecordOptions::Avro(avro) => crate::avro::read_batch_reader(handle, declared, avro)?,
        // Through the trait method rather than the free function, so a
        // decoding view's snapshot override is honored: the record surface
        // must see the value a handle presents, pending writes included.
        RecordOptions::Text(text) => handle.read_arrow_lines(&text.lines)?,
    };
    match declared {
        Some(field) => Ok(crate::arrow::cast_reader(reader, field, options.safe())?),
        None => Ok(reader),
    }
}

/// Count one encoded leaf from format metadata without decoding row arrays.
#[cfg(feature = "arrow")]
pub(crate) fn leaf_row_size(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<u64> {
    match options {
        RecordOptions::Ipc(ipc) => crate::ipc::row_size(handle, ipc),
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => crate::parquet::row_size(handle, parquet),
        RecordOptions::Avro(avro) => crate::avro::row_size(handle, avro),
        RecordOptions::Text(text) => crate::text::line::row_size(handle, &text.lines),
    }
}

/// Read one encoded leaf's canonical Struct field from format metadata.
///
/// Unlike asking a batch reader for its schema, each binary encoding reaches
/// its header or footer directly, so discovering a large Avro container's
/// width never fetches or decodes its block payloads.
#[cfg(feature = "arrow")]
pub(crate) fn leaf_field(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<crate::Field> {
    use crate::generic::IORecordOptions;

    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    match options {
        RecordOptions::Ipc(ipc) => Ok(crate::ipc::read_field(handle, ipc)?),
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => Ok(crate::parquet::read_field(handle, parquet)?),
        RecordOptions::Avro(avro) => Ok(crate::avro::read_field(handle, avro)?),
        RecordOptions::Text(text) => Ok(text.lines().field().clone()),
    }
}

/// Encode one leaf's complete contents.
///
/// This is the only place a record write reaches an encoding. Nothing reaches
/// the handle until the last batch has been encoded, so a failure leaves the
/// resource exactly as it was.
#[cfg(feature = "arrow")]
pub(crate) fn leaf_writer(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    match options {
        RecordOptions::Ipc(ipc) => crate::ipc::overwrite_batch_reader(handle, batches, ipc)?,
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => {
            crate::parquet::overwrite_batch_reader(handle, batches, parquet)?;
        }
        RecordOptions::Avro(avro) => crate::avro::overwrite_batch_reader(handle, batches, avro)?,
        RecordOptions::Text(text) => {
            crate::text::line::arrow::write_arrow_lines(handle, batches, text)?;
        }
    }
    Ok(())
}

/// Read the root Field a leaf's own bytes declare, if it holds any.
///
/// The declared schema is deliberately not consulted: this asks what is stored,
/// which is the only thing that can say whether a write is filling a resource
/// that already has a shape or giving one to a resource that has none.
#[cfg(feature = "arrow")]
pub(crate) fn stored_field(
    handle: &(impl IOBase + ?Sized),
    options: &RecordOptions,
) -> Result<Option<crate::Field>> {
    use crate::generic::IORecordOptions;

    if handle.is_empty() {
        return Ok(None);
    }
    // Text lines store no record shape of their own: any row shape writes,
    // rendered line by line, so there is nothing to complete a cast onto.
    if matches!(options, RecordOptions::Text(_)) {
        return Ok(None);
    }
    let mut probe = RecordOptions::for_mime_type(&options.mime_type())?;
    probe.set_root_name(smol_str::SmolStr::new(options.root_name()));
    Ok(Some(leaf_field(handle, &probe)?))
}

/// Merge `incoming` into a leaf's rows on the options' match key.
#[cfg(feature = "arrow")]
fn merge_leaf(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
    merge_by_names: &[String],
) -> Result<()> {
    // A text line has no row identity: re-parsing the resource yields
    // projection rows, not the rows a caller wrote, so a key match would
    // silently compare against the wrong thing. Refused rather than guessed.
    if matches!(options, RecordOptions::Text(_)) {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.merge_by_names"),
            reason: crate::text::expected_got(
                "a record encoding with row identity to merge by (Arrow IPC, Parquet, Avro)",
                "text lines, which have none - use overwrite or append",
            ),
        });
    }
    let target = target_field(handle, &incoming, options)?;
    merge_leaf_onto(handle, incoming, options, merge_by_names, &target)
}

/// Merge an already-shaped cadence under one target fixed for the operation.
#[cfg(feature = "arrow")]
fn merge_leaf_onto(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
    merge_by_names: &[String],
    target: &crate::Field,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    // The stored side is read as the target so both sides of the match agree
    // column for column before a single key is compared.
    let mut rewrite = options.clone();
    rewrite.set_field(target.clone());
    let stored = leaf_reader(handle, &rewrite)?;
    let merged = merge::merged(stored, incoming, target, merge_by_names, options.safe())?;
    // The merged contents are the whole new value. The cloned options already
    // had its declared field popped by `prepare_arrow_write`; clear the key as
    // well so the required overwrite hook sees exactly one publication and
    // cannot recursively merge the result against itself.
    rewrite.take_field();
    rewrite.set_merge_by_names(Vec::new());
    handle.overwrite_prepared_arrow_reader(merged, &rewrite)
}

/// Add `incoming` after a leaf's current rows.
#[cfg(feature = "arrow")]
fn append_leaf(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    // Text lines append natively: rows render after the current last line,
    // with no reason to re-parse what is already there.
    if let RecordOptions::Text(text) = options {
        return crate::text::line::arrow::append_arrow_lines(handle, incoming, text);
    }
    let target = target_field(handle, &incoming, options)?;
    append_leaf_onto(handle, incoming, options, &target)
}

/// Append an already-shaped cadence under one target fixed for the operation.
#[cfg(feature = "arrow")]
fn append_leaf_onto(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
    target: &crate::Field,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    let mut rewrite = options.clone();
    rewrite.set_field(target.clone());
    let current = if handle.is_empty() {
        // Per the laziness contract, a resource that holds nothing is skipped
        // rather than decoded.
        crate::arrow::batch_reader(crate::arrow::arrow_schema_from_field(target)?, [])
    } else {
        leaf_reader(handle, &rewrite)?
    };
    let appended = crate::arrow::appended(current, incoming, target, options.safe())?;
    rewrite.take_field();
    rewrite.set_merge_by_names(Vec::new());
    handle.overwrite_prepared_arrow_reader(appended, &rewrite)
}

/// Resolve the root Field a merge or an append produces.
///
/// The declared schema wins, then what the resource already stores, then the
/// shape the incoming reader arrived with - which is the only answer left when
/// nothing has been declared and nothing has been stored.
#[cfg(feature = "arrow")]
fn target_field(
    handle: &(impl IOBase + ?Sized),
    incoming: &crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<crate::Field> {
    use crate::generic::IORecordOptions;

    if let Some(field) = options.field() {
        return Ok(field.clone());
    }
    if let Some(field) = stored_field(handle, options)? {
        return Ok(field);
    }
    Ok(crate::arrow::field_from_arrow_schema(
        options.root_name(),
        incoming.schema().as_ref(),
    )?)
}

/// Report a resource that cannot contain children.
fn no_children(url: Option<&Url>, name: &str) -> Error {
    Error::Io(std::io::Error::new(
        std::io::ErrorKind::NotADirectory,
        match url {
            Some(url) => format!("expected a container to resolve {name:?} against, got {url}"),
            None => format!("expected a container to resolve {name:?} against, got a buffer"),
        },
    ))
}

/// Report a value too large for this platform's address space.
pub(crate) fn oversized(size: u64) -> Error {
    Error::Io(std::io::Error::other(format!(
        "expected a value addressable by usize, got {size} bytes"
    )))
}

/// The MIME type naming one content coding, for media-type bookkeeping.
fn coding_mime(codec: Codec) -> Option<MimeType> {
    match codec {
        Codec::Identity => None,
        Codec::Gzip => Some(MimeType::GZIP),
        Codec::Zlib | Codec::Deflate => Some(MimeType::ZLIB),
        Codec::Zstd => Some(MimeType::ZSTD),
    }
}

/// A streaming reader over an [`IOBase`], advancing its own offset.
pub struct Reader<'source> {
    source: &'source dyn IOBase,
    offset: u64,
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
    target: &'target mut dyn IOBase,
    offset: u64,
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
    fn overwrite_arrow_record_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::overwrite_arrow_record_batch(self.as_mut(), batch, options)
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
    fn append_arrow_record_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::append_arrow_record_batch(self.as_mut(), batch, options)
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
    fn merge_arrow_record_batch(
        &mut self,
        batch: arrow_array::RecordBatch,
        options: &RecordOptions,
    ) -> Result<()> {
        IOMedia::merge_arrow_record_batch(self.as_mut(), batch, options)
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

    fn read_range(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_ref().read_range(offset, length)
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

#[cfg(test)]
mod tests;
