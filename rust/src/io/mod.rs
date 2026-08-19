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
pub use roles::{IOFile, IOFolder, IOPath};

use crate::generic::Holder;
#[cfg(feature = "arrow")]
use crate::generic::RecordOptions;

/// Bytes copied per step when moving between two handles.
const TRANSFER_CHUNK: usize = 64 * 1024;

/// Implement [`IOBase`] methods by forwarding them to an inner handle.
///
/// A type that wraps a handle - a media reader, a page cache, a test double -
/// mirrors that handle rather than owning bytes of its own. The macro expands
/// to the forwarding bodies inside an `impl IOBase for` block, so anything the
/// wrapper wants to override (typically [`IOBase::open`] and [`IOBase::close`],
/// which usually also manage a cache) is simply written after the invocation.
///
/// [`IOBase::clear`] and [`IOBase::remove`] are delegated too, so a wrapper
/// empties and deletes the resource it wraps - not merely its own view of it -
/// without thinking about it. A wrapper holding a cache of its own must
/// invalidate it as part of those calls, and a macro-provided body cannot be
/// overridden, so it invokes the second form and writes the pair itself:
///
/// ```
/// use yggdryl::io::{Buffer, IOBase};
///
/// struct Cached {
///     handle: Buffer,
///     schema: Option<String>,
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
/// use yggdryl::io::{Buffer, IOBase};
///
/// struct Wrapper {
///     handle: Buffer,
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
/// use yggdryl::io::{Buffer, IOBase};
///
/// /// A handle that counts the reads reaching the one it wraps.
/// struct Counted {
///     handle: Buffer,
///     reads: AtomicUsize,
/// }
///
/// impl IOBase for Counted {
///     yggdryl::delegate_iobase!(handle: pwrite, size, capacity, reserve,
///         truncate, url, media_type, set_media_type, flush, parent, child_by_path,
///         ls, kind, clear, remove, is_atomic, is_tabular);
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
        $crate::delegate_iobase!($handle: pread, pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, parent, child_by_path,
            ls, kind, clear, remove, is_atomic, is_tabular);
    };

    // Everything but [`IOBase::clear`] and [`IOBase::remove`], which a wrapper
    // holding a cache of its own writes itself so the cache is invalidated as
    // part of the call rather than left to go stale, and but for the two surface
    // questions, which a record encoding answers as constants rather than
    // mirroring the bytes underneath. The same list, named once instead of at
    // five call sites.
    ($handle:ident, except_lifecycle) => {
        $crate::delegate_iobase!($handle: pread, pwrite, size, capacity, reserve,
            truncate, url, media_type, set_media_type, flush, parent, child_by_path,
            ls, kind);
    };

    ($handle:ident: $($method:ident),+ $(,)?) => {
        $($crate::delegate_iobase!(@method $handle, $method);)+
    };

    (@method $handle:ident, pread) => {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> $crate::Result<usize> {
            $crate::io::IOBase::pread(&self.$handle, offset, buffer)
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
pub trait IOBase: Send {
    /// Read into `buffer` starting at `offset`, returning the bytes read.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize>;

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
    /// [`read_arrow_batch_reader`](Self::read_arrow_batch_reader) decodes and
    /// what its two writing siblings encode. The question is about the
    /// representation rather than about this build - a `.parquet` leaf is
    /// tabular whether or not the `parquet` feature is compiled in, and
    /// [`record_options`](Self::record_options) is the call that reports an
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
    /// [`read_arrow_batch_reader`](Self::read_arrow_batch_reader) and its
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
        target.reserve(self.size())?;
        let mut chunk = vec![0_u8; TRANSFER_CHUNK];
        let mut offset = 0;
        loop {
            let read = self.pread(offset, &mut chunk)?;
            if read == 0 {
                break;
            }
            target.pwrite_all(offset, &chunk[..read])?;
            offset += read as u64;
        }
        target.set_media_type(self.media_type().clone());
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
        let encoded = codec.dump_with_level(&self.read_all_bytes()?, level)?;
        target.pwrite_all(0, &encoded)?;
        let media_type = self
            .media_type()
            .clone()
            .try_with_encodings(coding_mime(codec))?;
        target.set_media_type(media_type);
        Ok(encoded.len() as u64)
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
        let decoded = codec.load(&self.read_all_bytes()?)?;
        target.pwrite_all(0, &decoded)?;
        // The decoded bytes carry the base representation with the coding
        // removed, which is exactly the media type minus its last encoding.
        let mut media_type = self.media_type().clone();
        let mut encodings = media_type.encodings().to_vec();
        encodings.pop();
        media_type.set_encodings(encodings)?;
        target.set_media_type(media_type);
        Ok(decoded.len() as u64)
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
    /// A thin shim over [`Self::into_text`]. This is a *text-line* projection
    /// like [`Self::read_lines`], **not a fourth record method**: the record
    /// surface stays exactly [`Self::read_arrow_batch_reader`],
    /// [`Self::write_arrow_batch_reader`], and
    /// [`Self::append_arrow_batch_reader`], and this never touches how a record
    /// encoding decodes rows. The columns are described on
    /// [`TextLineOptions`](crate::text::TextLineOptions).
    ///
    /// # Errors
    ///
    /// Returns a listing or reopen failure; each yielded batch carries the
    /// read, decode, or parse failure of its rows.
    #[cfg(feature = "arrow")]
    fn read_arrow_lines(
        &self,
        options: &crate::text::TextLineOptions,
    ) -> Result<crate::arrow::BatchReader>
    where
        Self: Sized,
    {
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

    /// Return the record options this resource's encoding names.
    ///
    /// This is what a caller supplies when they have no options of their own,
    /// so the encoding is never guessed: it is whatever the handle already says
    /// it holds. A container has no bytes and therefore no media type of its
    /// own, so it answers with the encoding of the leaves beneath it - a
    /// partitioned tree is one table in one encoding - and a container that is
    /// an Iceberg table answers with the encoding its data files are written
    /// in, which its metadata knows before a single file exists.
    ///
    /// # Errors
    ///
    /// Returns an error when no record encoding in this build covers the
    /// handle's media type, or the media type of anything below it.
    #[cfg(feature = "arrow")]
    fn record_options(&self) -> Result<RecordOptions> {
        if self.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(table) = crate::iceberg::located(self)? {
                return table.record_options();
            }
            // The first leaf that names an encoding answers, and the listing
            // is lazy, so a lake of a million files costs the walk to its
            // first leaf and stops there.
            for child in self.children_where(&[], false)? {
                if let Ok(options) = RecordOptions::for_media_type(child?.media_type()) {
                    return Ok(options);
                }
            }
        }
        RecordOptions::for_media_type(self.media_type())
    }

    /// Read the canonical non-null Struct root Field of this resource.
    ///
    /// A declared schema is returned as it stands; otherwise this is the shape
    /// [`Self::read_arrow_batch_reader`] reports, so the schema a caller reads
    /// and the batches a caller gets can never disagree.
    ///
    /// # Errors
    ///
    /// Returns a read, decoding, or schema-projection failure.
    #[cfg(feature = "arrow")]
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<crate::Field> {
        use crate::generic::IORecordOptions;

        if let Some(schema) = options.schema() {
            return Ok(schema.clone());
        }
        let schema = self.read_arrow_batch_reader(options)?.schema();
        Ok(crate::arrow::record_schema_from_arrow(
            options.root_name(),
            schema.as_ref(),
        )?)
    }

    /// Read this resource's rows as one [`BatchReader`](crate::arrow::BatchReader).
    ///
    /// This is the one read path, so an encoding is decoded in exactly one
    /// place, and the result streams: one batch at a time, never a materialized
    /// vector.
    ///
    /// **A declared schema selects and casts during the read.** The columns it
    /// names that the resource stores become the encoding's own projection - a
    /// Parquet projection mask, an Arrow IPC projection - so the rest are
    /// skipped rather than read and discarded, and what comes back is then cast
    /// to the declared shape as each batch arrives. Ordering, conversion, and a
    /// column the resource does not hold are the cast's business, because a
    /// projection can only drop columns, never reorder or invent them. Say
    /// plainly what each encoding's projection saves: Parquet skips locating and
    /// decoding a column chunk, while an Arrow IPC record batch is one
    /// contiguous message, so its projection saves the decode and the
    /// allocation but not the bytes. With no declared schema the stored shape is
    /// preserved exactly.
    ///
    /// **A folder reads as the table beneath it.** When this handle addresses a
    /// container, every leaf holding this encoding is read in turn, the columns
    /// its `column=value` directories spell out are restored, and each batch is
    /// cast to one root - so a caller never has to know whether they addressed
    /// one file or a partitioned tree. A container holding a *table format*
    /// reads through that format instead: an Iceberg table's current snapshot
    /// says which data files are live and which of them a filtered read can
    /// skip, so the folder is never listed and a file an overwrite replaced is
    /// never read back.
    ///
    /// Per the laziness contract, a resource that does not exist yet holds no
    /// batches rather than failing.
    ///
    /// The shaping order is fixed: declared schema, then selection, then
    /// completion cast, then partition filter, then
    /// [`max_row_size`](crate::generic::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::generic::IORecordOptions::max_byte_size)
    /// last - so a limit counts result rows, and a limit of ten with a filter
    /// means the first ten matching rows. A satisfied limit stops pulling, so
    /// the rest of the resource is never decoded.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, decoding, or cast failure.
    #[cfg(feature = "arrow")]
    fn read_arrow_batch_reader(
        &self,
        options: &RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        use crate::generic::IORecordOptions;

        let reader = if self.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(table) = crate::iceberg::located(self)? {
                let filtered = partition::filtered_reader(table.read(options)?, options)?;
                return options.limit_arrow_reader(select_reader(filtered, options)?);
            }
            partition::folder_reader(self, options)?
        } else {
            leaf_reader(self, options)?
        };
        let reader = partition::filtered_reader(reader, options)?;
        options.limit_arrow_reader(select_reader(reader, options)?)
    }

    /// Replace or merge this resource's rows with every batch `batches` yields.
    ///
    /// This is the one write path, so an encoding is encoded in exactly one
    /// place. Which of the two it is comes from
    /// [`IORecordOptions::merge_by_names`](crate::generic::IORecordOptions::merge_by_names):
    ///
    /// - **An empty match key overwrites.** A declared schema is applied to the
    ///   incoming rows first; the result is then cast to the schema the resource
    ///   already stores, if it stores one, so an overwrite replaces rows rather
    ///   than redefining columns. A resource that holds nothing yet takes the
    ///   incoming shape as it stands, and a caller who really means to change a
    ///   stored schema clears the handle first.
    /// - **A non-empty match key merges.** The stored rows are read, the
    ///   incoming reader is joined against them one batch at a time, a row whose
    ///   key is already stored updates it, a row whose key is not appends, and
    ///   the merged contents are rewritten. The join is streamed over the
    ///   incoming side: one batch is pulled, matched, folded in, and dropped
    ///   before the next is pulled. What has to be held is the stored side,
    ///   because updating a row means finding it by key and a reader cannot be
    ///   rewound to a row it has already yielded.
    ///
    /// A folder routes each row to the leaf its partition values name, creating
    /// the `column=value` directory when the layout has one and no leaf holds
    /// that value yet. A folder holding a table format commits instead: an
    /// Iceberg table writes one snapshot, whose merge reads only the data files
    /// whose statistics say they can hold an incoming key and carries the rest
    /// forward untouched.
    ///
    /// A limited write truncates data the caller offered:
    /// [`max_row_size`](crate::generic::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::generic::IORecordOptions::max_byte_size) bound
    /// the incoming reader exactly as they bound a read, and what they cut off
    /// is never pulled from it. A limit combined with a non-empty match key is
    /// refused naming both settings, because a truncated merge would update
    /// some matched keys and silently drop the rest.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, schema, cast, encoding, or write failure.
    #[cfg(feature = "arrow")]
    fn write_arrow_batch_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        use crate::generic::IORecordOptions;

        // The limit sits on the incoming side, before anything else pulls, so
        // a satisfied write stops consuming the caller's reader; the same call
        // refuses a limit combined with a match key.
        let batches = options.limit_arrow_reader(batches)?;
        // The selection narrows what a write is about before any encoding or
        // matching sees the rows, so the columns it drops can never land.
        let batches = select_reader(batches, options)?;
        if self.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(mut table) = crate::iceberg::located(self)? {
                return table.write(batches, options);
            }
            return partition::write_folder(self, batches, options, false);
        }
        if options.merge_by_names().is_empty() {
            return overwrite_leaf(self, batches, options);
        }
        merge_leaf(self, batches, options)
    }

    /// Add every batch `batches` yields after the rows this resource holds.
    ///
    /// The encodings here are whole-value containers - an Arrow IPC stream and a
    /// Parquet file each carry one schema and one footer - so appending means
    /// reading what is there, adding to it, and rewriting. The current rows are
    /// read as the declared schema when there is one and as the stored schema
    /// otherwise, a resource holding nothing is skipped rather than decoded, and
    /// the incoming batches are cast to that same shape, so a caller may append
    /// data whose schema merely *fits*. Both sides stream: the stored batches
    /// are chained ahead of the incoming ones and encoded as they arrive, so
    /// neither is collected.
    ///
    /// A folder appends into each partition the incoming rows name, leaving
    /// every other partition untouched. A folder holding a table format appends
    /// the way that format does: an Iceberg table writes new data files and
    /// commits a snapshot that keeps every manifest the last one had, so nothing
    /// already stored is read, rewritten, or even listed.
    ///
    /// A limited write truncates data the caller offered: an append is a
    /// write, so
    /// [`max_row_size`](crate::generic::IORecordOptions::max_row_size) and
    /// [`max_byte_size`](crate::generic::IORecordOptions::max_byte_size) bound
    /// the incoming reader here exactly as they do on
    /// [`write_arrow_batch_reader`](Self::write_arrow_batch_reader), and a
    /// limit combined with a non-empty match key is refused the same way.
    ///
    /// # Errors
    ///
    /// Returns a listing, read, cast, encoding, or write failure. A failure
    /// leaves the resource unchanged, because nothing is written until the new
    /// contents are complete.
    #[cfg(feature = "arrow")]
    fn append_arrow_batch_reader(
        &mut self,
        batches: crate::arrow::BatchReader,
        options: &RecordOptions,
    ) -> Result<()> {
        use crate::generic::IORecordOptions;

        // The limit sits on the incoming side for the same reason it does on
        // a write: a satisfied append stops consuming the caller's reader.
        let batches = options.limit_arrow_reader(batches)?;
        // The same narrowing a write applies: an append is a write that keeps.
        let batches = select_reader(batches, options)?;
        if self.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(mut table) = crate::iceberg::located(self)? {
                return table.append(batches);
            }
            return partition::write_folder(self, batches, options, true);
        }
        // A merge key means "update the row that already has this key" - the
        // folder path above has always honoured it, so a leaf that ignored it
        // made one option mean two things depending on what the handle
        // happened to address, and stored a second row under a key the caller
        // said identifies one. Merging already keeps every stored row the
        // incoming keys do not name, which is what makes it the append.
        if options.merge_by_names().is_empty() {
            return append_leaf(self, batches, options);
        }
        merge_leaf(self, batches, options)
    }
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
        crate::arrow::record_schema_from_arrow(options.root_name(), reader.schema().as_ref())?;
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

    let declared = options.schema();
    let reader = match options {
        RecordOptions::Ipc(ipc) => crate::ipc::read_batch_reader(handle, declared, ipc)?,
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => {
            crate::parquet::read_batch_reader(handle, declared, parquet)?
        }
        RecordOptions::Avro(avro) => crate::avro::read_batch_reader(handle, declared, avro)?,
    };
    match declared {
        Some(field) => Ok(crate::arrow::cast_reader(reader, field, options.safe())?),
        None => Ok(reader),
    }
}

/// Encode one leaf's complete contents.
///
/// This is the only place a record write reaches an encoding. Nothing reaches
/// the handle until the last batch has been encoded, so a failure leaves the
/// resource exactly as it was.
#[cfg(feature = "arrow")]
fn leaf_writer(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    match options {
        RecordOptions::Ipc(ipc) => crate::ipc::write_batch_reader(handle, batches, ipc)?,
        #[cfg(feature = "parquet")]
        RecordOptions::Parquet(parquet) => {
            crate::parquet::write_batch_reader(handle, batches, parquet)?;
        }
        RecordOptions::Avro(avro) => crate::avro::write_batch_reader(handle, batches, avro)?,
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
    let mut probe = RecordOptions::for_mime_type(&options.mime_type())?;
    probe.set_root_name(smol_str::SmolStr::new(options.root_name()));
    let schema = leaf_reader(handle, &probe)?.schema();
    Ok(Some(crate::arrow::record_schema_from_arrow(
        probe.root_name(),
        schema.as_ref(),
    )?))
}

/// Replace a leaf's complete contents with `batches`.
#[cfg(feature = "arrow")]
fn overwrite_leaf(
    handle: &mut (impl IOBase + ?Sized),
    batches: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    // One definition of option-driven casting: the declared schema first,
    // then the selection, then completion onto the stored shape, so a value
    // that will not convert into a stored column becomes null rather than
    // quietly redefining that column for every reader of the resource.
    let stored = stored_field(handle, options)?;
    let batches = options.cast_arrow_reader(batches, stored.as_ref())?;
    leaf_writer(handle, batches, options)
}

/// Merge `incoming` into a leaf's rows on the options' match key.
#[cfg(feature = "arrow")]
fn merge_leaf(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    let target = target_field(handle, &incoming, options)?;
    // The stored side is read as the target so both sides of the match agree
    // column for column before a single key is compared.
    let mut rewrite = options.clone();
    rewrite.set_schema(target.clone());
    let stored = leaf_reader(handle, &rewrite)?;
    let merged = merge::merged(
        stored,
        incoming,
        &target,
        options.merge_by_names(),
        options.safe(),
    )?;
    // The merged contents are the whole new value, so what publishes them is a
    // plain write: merging again would match the result against itself.
    rewrite.set_merge_by_names(Vec::new());
    leaf_writer(handle, merged, &rewrite)
}

/// Add `incoming` after a leaf's current rows.
#[cfg(feature = "arrow")]
fn append_leaf(
    handle: &mut (impl IOBase + ?Sized),
    incoming: crate::arrow::BatchReader,
    options: &RecordOptions,
) -> Result<()> {
    use crate::generic::IORecordOptions;

    let target = target_field(handle, &incoming, options)?;
    let mut rewrite = options.clone();
    rewrite.set_schema(target.clone());
    let current = if handle.is_empty() {
        // Per the laziness contract, a resource that holds nothing is skipped
        // rather than decoded.
        crate::arrow::batch_reader(crate::arrow::schema_from_field(&target)?, [])
    } else {
        leaf_reader(handle, &rewrite)?
    };
    let appended = crate::arrow::appended(current, incoming, &target, options.safe())?;
    rewrite.set_merge_by_names(Vec::new());
    leaf_writer(handle, appended, &rewrite)
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

    if let Some(field) = options.schema() {
        return Ok(field.clone());
    }
    if let Some(field) = stored_field(handle, options)? {
        return Ok(field);
    }
    Ok(crate::arrow::record_schema_from_arrow(
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

impl IOBase for Box<dyn IOBase> {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_ref().pread(offset, buffer)
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

    // The shape questions forward rather than deriving from this box's own
    // answers: a boxed folder is a folder, and the default would read the
    // trait's `kind` here rather than the one the value inside answers.
    fn is_atomic(&self) -> bool {
        self.as_ref().is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.as_ref().is_tabular()
    }
}

#[cfg(test)]
mod tests;
