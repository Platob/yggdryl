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
//! memory-mapped local file. Anything else - an object store, an Arrow
//! filesystem - implements the same trait outside the core.
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
// The table formats join on a match key through exactly this implementation:
// one merge, whether the rows live in one leaf or in a snapshot's data files.
#[cfg(feature = "arrow")]
pub(crate) mod merge;
#[cfg(feature = "arrow")]
pub mod partition;
mod roles;

pub use buffer::Buffer;
pub use coding::Coded;
pub use roles::{IOFile, IOFolder, IOPath};

use crate::generic::Holder;
#[cfg(feature = "arrow")]
use crate::generic::RecordOptions;

/// Bytes copied per step when moving between two handles.
const TRANSFER_CHUNK: usize = 64 * 1024;

/// Implement every [`IOBase`] byte method by forwarding to an inner handle.
///
/// A type that wraps a handle - a media reader, a test double - mirrors that
/// handle's bytes rather than owning bytes of its own. The macro expands to the
/// forwarding bodies inside an `impl IOBase for` block, so anything the wrapper
/// wants to override (typically [`IOBase::open`] and [`IOBase::close`], which
/// usually also manage a cache) is simply written after the invocation.
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
/// assert_eq!(wrapper.read_all()?, b"AAPL");
/// # Ok(())
/// # }
/// ```
#[macro_export]
macro_rules! delegate_iobase {
    ($handle:ident) => {
        fn pread(&self, offset: u64, buffer: &mut [u8]) -> $crate::Result<usize> {
            $crate::io::IOBase::pread(&self.$handle, offset, buffer)
        }

        fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> $crate::Result<usize> {
            $crate::io::IOBase::pwrite(&mut self.$handle, offset, bytes)
        }

        fn size(&self) -> u64 {
            $crate::io::IOBase::size(&self.$handle)
        }

        fn capacity(&self) -> u64 {
            $crate::io::IOBase::capacity(&self.$handle)
        }

        fn reserve(&mut self, capacity: u64) -> $crate::Result<()> {
            $crate::io::IOBase::reserve(&mut self.$handle, capacity)
        }

        fn truncate(&mut self, size: u64) -> $crate::Result<()> {
            $crate::io::IOBase::truncate(&mut self.$handle, size)
        }

        fn url(&self) -> Option<&$crate::Url> {
            $crate::io::IOBase::url(&self.$handle)
        }

        fn media_type(&self) -> &$crate::MediaType {
            $crate::io::IOBase::media_type(&self.$handle)
        }

        fn set_media_type(&mut self, media_type: $crate::MediaType) {
            $crate::io::IOBase::set_media_type(&mut self.$handle, media_type);
        }

        fn flush(&mut self) -> $crate::Result<()> {
            $crate::io::IOBase::flush(&mut self.$handle)
        }

        fn parent(&self) -> Option<$crate::generic::Holder> {
            $crate::io::IOBase::parent(&self.$handle)
        }

        fn child_by(&self, name: &str) -> $crate::Result<$crate::generic::Holder> {
            $crate::io::IOBase::child_by(&self.$handle, name)
        }

        fn ls(
            &self,
            recursive: bool,
            include_private: bool,
        ) -> $crate::Result<Vec<$crate::generic::Holder>> {
            $crate::io::IOBase::ls(&self.$handle, recursive, include_private)
        }

        fn kind(&self) -> $crate::IOKind {
            $crate::io::IOBase::kind(&self.$handle)
        }
    };
}

/// Resolve a chain of fixed names below `base`, without touching anything.
///
/// Returns `None` for an empty chain, so a caller can tell "descend nowhere"
/// from "descend to here".
fn descend(base: &(impl IOBase + ?Sized), names: &[&str]) -> Result<Option<Holder>> {
    let Some((first, rest)) = names.split_first() else {
        return Ok(None);
    };
    let mut holder = base.child_by(first)?;
    for name in rest {
        holder = holder.child_by(name)?;
    }
    Ok(Some(holder))
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
    fn is_open(&self) -> bool {
        false
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

    /// Return the child named `name`, resolved against this resource.
    ///
    /// `name` may be a single segment or a relative path; `.` and `..` resolve
    /// the way they do in [`crate::UriPath::joinpath`]. The child need not
    /// exist - per the laziness contract, reading a missing child is empty and
    /// writing one creates it.
    ///
    /// # Errors
    ///
    /// Returns an error when this resource cannot have children or `name` does
    /// not form a valid location.
    fn child_by(&self, name: &str) -> Result<Holder> {
        Err(no_children(self.url(), name))
    }

    /// List the resources contained by this one.
    ///
    /// `recursive` descends into every container beneath this one. A resource
    /// that cannot contain others lists nothing rather than failing, so a
    /// caller can walk a tree without testing each node first.
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing failure.
    fn ls(&self, recursive: bool, include_private: bool) -> Result<Vec<Holder>> {
        let _ = (recursive, include_private);
        Ok(Vec::new())
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
    ///     println!("{}", part.url().expect("a located child"));
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing failure.
    fn glob(&self, pattern: &str, include_private: bool) -> Result<Vec<Holder>> {
        let parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
        let Some(fixed) = parts.iter().position(|part| Url::is_pattern(part)) else {
            // Nothing to expand: the pattern names one location, which counts
            // only if something is actually there.
            let child = descend(self, &parts)?;
            return Ok(match child {
                Some(child) if child.kind() != IOKind::Unknown => vec![child],
                _ => Vec::new(),
            });
        };
        if fixed > 0 {
            // Descend the fixed prefix so the listing starts as deep as it can.
            let Some(child) = descend(self, &parts[..fixed])? else {
                return Ok(Vec::new());
            };
            return child.glob(&parts[fixed..].join("/"), include_private);
        }

        let Some(root) = self.url().cloned() else {
            return Ok(Vec::new());
        };
        // One plain segment is answered by the immediate children; anything
        // deeper, or a `**`, needs the whole subtree.
        let recursive = parts.len() > 1 || parts[0] == "**";
        let mut matched: Vec<Holder> = self
            .ls(recursive, include_private)?
            .into_iter()
            .filter(|entry| {
                entry
                    .url()
                    .is_some_and(|url| url.matches_glob_under(&root, pattern))
            })
            .collect();
        matched.sort_by(|left, right| {
            left.url()
                .map(ToString::to_string)
                .cmp(&right.url().map(ToString::to_string))
        });
        Ok(matched)
    }

    /// Return the Hive partition pairs this resource's own location spells out.
    ///
    /// A Hive layout writes one directory per partition column, so a handle
    /// deep in a lake already knows the column values its rows share.
    fn partitions(&self) -> Vec<(String, String)> {
        self.url().map(Url::hive_partitions).unwrap_or_default()
    }

    /// Iterate the leaves beneath this one that carry every given partition.
    ///
    /// This is the handle a partitioned write reaches for: select the parts of
    /// a lake that hold one partition, then overwrite or upsert them directly
    /// instead of rewriting the table. Containers are not yielded - only the
    /// resources that hold bytes - and an empty filter yields every leaf.
    ///
    /// ```no_run
    /// use yggdryl::io::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(std::env::temp_dir().join("lake"))?;
    ///
    /// for mut part in lake.children_where(&[("year", "2024")], false)? {
    ///     part.clear()?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the backing store's listing failure.
    fn children_where(
        &self,
        filters: &[(&str, &str)],
        include_private: bool,
    ) -> Result<std::vec::IntoIter<Holder>> {
        let mut matched: Vec<Holder> = self
            .ls(true, include_private)?
            .into_iter()
            .filter(|entry| !entry.is_container())
            .filter(|entry| {
                let partitions = entry.partitions();
                filters.iter().all(|(column, value)| {
                    partitions
                        .iter()
                        .any(|(key, held)| key == column && held == value)
                })
            })
            .collect();
        matched.sort_by(|left, right| {
            left.url()
                .map(ToString::to_string)
                .cmp(&right.url().map(ToString::to_string))
        });
        Ok(matched.into_iter())
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
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn read_all(&self) -> Result<Vec<u8>> {
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
    /// # Errors
    ///
    /// Returns the backing store's resize or write failure.
    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.truncate(0)?;
        self.pwrite_all(0, bytes)
    }

    /// Discard every byte, keeping the allocation.
    ///
    /// # Errors
    ///
    /// Returns the backing store's resize failure.
    fn clear(&mut self) -> Result<()> {
        self.truncate(0)
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
        let encoded = codec.dump_with_level(&self.read_all()?, level)?;
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
        let decoded = codec.load(&self.read_all()?)?;
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
                return Ok(table.record_options());
            }
            for child in self.children_where(&[], false)? {
                if let Ok(options) = RecordOptions::for_media_type(child.media_type()) {
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
    /// # Errors
    ///
    /// Returns a listing, read, decoding, or cast failure.
    #[cfg(feature = "arrow")]
    fn read_arrow_batch_reader(
        &self,
        options: &RecordOptions,
    ) -> Result<crate::arrow::BatchReader> {
        let reader = if self.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(table) = crate::iceberg::located(self)? {
                return select_reader(table.read(options)?, options);
            }
            partition::folder_reader(self, options)?
        } else {
            leaf_reader(self, options)?
        };
        select_reader(reader, options)
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
        // The same narrowing a write applies: an append is a write that keeps.
        let batches = select_reader(batches, options)?;
        if self.is_container() {
            #[cfg(feature = "iceberg")]
            if let Some(mut table) = crate::iceberg::located(self)? {
                return table.append(batches);
            }
            return partition::write_folder(self, batches, options, true);
        }
        append_leaf(self, batches, options)
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
    let mut selected = Vec::with_capacity(names.len());
    for name in names {
        let child = root
            .fields()
            .iter()
            .find(|child| child.name().eq_ignore_ascii_case(name))
            .ok_or_else(|| Error::InvalidRecord {
                path: smol_str::format_smolstr!("$.{name}"),
                reason: smol_str::format_smolstr!(
                    "expected a column named {name:?} to select, got columns {:?}",
                    root.fields()
                        .iter()
                        .map(crate::Field::name)
                        .collect::<Vec<_>>()
                ),
            })?;
        selected.push(child.clone());
    }
    let target = crate::DataType::from_fields(selected)?.required_field(options.root_name());
    Ok(crate::arrow::cast_reader(reader, &target, options.safe())?)
}

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

    // The declared schema says what the incoming rows are meant to be, so it is
    // applied before anything else looks at them.
    let batches = match options.schema() {
        Some(field) => crate::arrow::cast_reader(batches, field, options.safe())?,
        None => batches,
    };
    // A resource that already has a shape keeps it. An overwrite replaces rows,
    // so a value that will not convert into a stored column becomes null rather
    // than quietly redefining that column for every reader of the resource.
    let batches = match stored_field(handle, options)? {
        Some(stored) => crate::arrow::cast_reader(batches, &stored, true)?,
        None => batches,
    };
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
}

#[cfg(test)]
mod tests;
