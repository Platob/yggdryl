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

use std::io::{Read as _, Write as _};

use crate::{ByteStream, Cursor, Error, IOKind, IOMedia, Listing, MediaType, Result, Url};
use crate::{Codec, Level};

use crate::generic::Holder;

/// Default byte-stream batch size used by core readers and language bindings.
pub const DEFAULT_STREAM_BATCH_SIZE: usize = 64 * 1024;

/// Bytes copied per step when moving between two handles.
const TRANSFER_CHUNK: usize = DEFAULT_STREAM_BATCH_SIZE;

mod bytes;
mod hierarchy;
mod lifecycle;
#[cfg(feature = "arrow")]
mod transfer;

pub use bytes::{Reader, Writer};
pub(crate) use hierarchy::container_is_tabular;
use hierarchy::{descend, no_children};
pub(crate) use lifecycle::{coding_mime, oversized};
pub use lifecycle::{not_empty, skip_absent};
#[cfg(feature = "arrow")]
pub use transfer::{ArrowWriteSession, overwrite_arrow_reader_default};
#[cfg(feature = "arrow")]
pub(crate) use transfer::{
    append_arrow_reader_default, leaf_field, leaf_reader, leaf_row_size, leaf_writer,
    merge_arrow_reader_default, non_empty_arrow_reader, overwrite_arrow_reader_default_with_field,
    prepare_arrow_write_onto, select_reader, stored_field,
};
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
    /// use yggdryl::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(Folder::temporary()?.path()?.join("lake"))?;
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
    /// use yggdryl::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(Folder::temporary()?.path()?.join("lake"))?;
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
    /// use yggdryl::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(Folder::temporary()?.path()?.join("lake"))?;
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
    /// use yggdryl::IOBase;
    /// use yggdryl::local::Folder;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let lake = Folder::new(Folder::temporary()?.path()?.join("lake"))?;
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
    /// handle.write_scalar(&value)?;
    ///
    /// let field = Field::from_str("trade: struct<quantity: int64 not null> not null")?;
    /// assert_eq!(handle.read_scalar(None)?, value);
    /// assert_eq!(
    ///     handle.read_scalar(Some(&field))?,
    ///     Scalar::from_sequence([Scalar::I64(2)])
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a read, decompression, format, parse, or field-cast failure.
    fn read_scalar(&self, field: Option<&crate::Field>) -> Result<crate::Scalar> {
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
    fn write_scalar(&mut self, value: &crate::Scalar) -> Result<()> {
        crate::text::into_io(value, self)
    }

    /// Read `length` bytes starting at `offset`.
    ///
    /// The ranged half of the byte-read family [`Self::read_all_bytes`] reads
    /// whole; it names `bytes` for the same reason, because the answer is the
    /// bytes rather than the rows
    /// [`read_arrow_reader`](IOMedia::read_arrow_reader) answers.
    ///
    /// The result is short only when the value ends first.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        let available = self.size().saturating_sub(offset);
        let length = length.min(usize::try_from(available).unwrap_or(usize::MAX));
        let mut bytes = vec![0_u8; length];
        self.pread_exact(offset, &mut bytes)?;
        Ok(bytes)
    }

    /// Digest the complete value without holding it in memory.
    ///
    /// The read streams through [`Self::pstream_bytes`] and retains one
    /// bounded chunk, so memory is flat in the object's size: a multi-gigabyte
    /// file costs one window rather than a copy. Every wrapper inherits this -
    /// a coding wrapper digests the decoded payload while the handle it wraps
    /// digests the compressed form, which is how a caller asks the two
    /// questions apart.
    ///
    /// A missing resource digests as empty. Construction is lazy everywhere in
    /// this trait, so absence is emptiness here too: the answer is the
    /// algorithm's empty-input value, never an error.
    ///
    /// ```
    /// use yggdryl::io::{Buffer, IOBase};
    /// use yggdryl::{DigestAlgorithm, xxhash};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut handle = Buffer::new();
    /// handle.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;
    ///
    /// assert_eq!(
    ///     handle.read_digest(DigestAlgorithm::Xxh3_64)?,
    ///     DigestAlgorithm::Xxh3_64.digest(&handle.read_all_bytes()?),
    /// );
    /// // Nothing written yet is the digest of no bytes.
    /// assert_eq!(
    ///     Buffer::new().read_digest(DigestAlgorithm::Xxh3_64)?.as_u64(),
    ///     Some(xxhash::xxh3_64(b"")),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure, or [`Error::NotAtomic`]
    /// naming the kind when this handle is a container. A folder holds no
    /// bytes of its own, and which files a folder digest would cover in what
    /// order is a convention no format states.
    fn read_digest(&self, algorithm: crate::DigestAlgorithm) -> Result<crate::Digest> {
        crate::xxhash::stream::read_digest(self, algorithm)
    }

    /// Digest `length` bytes starting at `offset`, streaming the window.
    ///
    /// The range is clamped exactly as [`Self::read_range_bytes`] clamps it: a
    /// window that runs past the end digests only the bytes that are there,
    /// and a window wholly past the end digests nothing.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure, or [`Error::NotAtomic`]
    /// naming the kind when this handle is a container.
    fn read_range_digest(
        &self,
        offset: u64,
        length: usize,
        algorithm: crate::DigestAlgorithm,
    ) -> Result<crate::Digest> {
        crate::xxhash::stream::read_range_digest(self, offset, length, algorithm)
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
    /// The byte member of the append family, beside
    /// [`append_arrow_reader`](IOMedia::append_arrow_reader), so the offset
    /// this one reports is unambiguously a byte offset.
    ///
    /// # Errors
    ///
    /// Returns the backing store's write failure.
    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
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
    /// assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
    ///
    /// // The second read of the same bytes reaches no further than memory.
    /// assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
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

    /// Consume this handle into plain-text record media.
    ///
    /// The wrapper is lazy and adds no line-only API. Its retained
    /// [`TextOptions`](crate::text::TextOptions) become the defaults for the
    /// ordinary [`IOMedia`] record methods.
    fn into_text(self) -> crate::text::Text<Self>
    where
        Self: Sized,
    {
        crate::text::Text::new(self)
    }

    /// Consume this handle into plain-text record media with explicit options.
    fn into_text_with(self, options: crate::text::TextOptions) -> crate::text::Text<Self>
    where
        Self: Sized,
    {
        crate::text::Text::new(self).with_options(options)
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
}

#[cfg(test)]
#[path = "io/tests.rs"]
mod tests;
