//! The mounted archive: one index over one byte handle.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, MutexGuard};

use smol_str::{SmolStr, format_smolstr};

use crate::holder::Holder;
use crate::{Codec, Error, IOBase, Level, Result, Url};

use super::{Entry, Node, format, name};

/// What separates one archive's member from the archive inside it.
///
/// A canonical member path never holds an empty segment - `name::resolve`
/// drops them - so a doubled separator is a spelling no member can claim, and
/// a fragment that holds one is naming a member of a member.
pub(super) const NESTED: &str = "//";

/// The bytes of evidence a restart point is proven by, on either side of it.
///
/// Both spellings are four bytes - the empty stored block a DEFLATE full flush
/// ends with, and the magic a Zstandard frame begins with - so a window of
/// four on each side of a point holds whichever one applies.
const RESTART_EVIDENCE: u64 = 4;

/// The most bytes an end-of-central-directory record can be from the end.
///
/// The record is fixed at 22 bytes plus a comment the format bounds at 65535,
/// so a tail this long always contains it wherever the comment ends.
const END_SEARCH_LEN: u64 = format::END_LEN as u64 + u16::MAX as u64;

/// Bytes moved per step while compacting, matching the shared stream batch.
const COMPACT_CHUNK: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// Decoded bytes between the restart points a compressed member is written
/// with.
///
/// One stream batch, which is also the page a [`Buffered`] handle fetches, so
/// a page of a member begins exactly on a point and its fill decodes nothing
/// it does not return. Restarting costs size - measured at about 3% on text
/// that deflates well - which is the price of a member that can be read at an
/// offset rather than only from its first byte.
///
/// [`Buffered`]: crate::holder::buffered::Buffered
pub const DEFAULT_RESTART_STRIDE: u64 = crate::DEFAULT_STREAM_BATCH_SIZE as u64;

/// A ZIP archive mounted over one byte handle.
///
/// The archive owns two things and nothing else: the handle its bytes live in,
/// and the central directory that says where each member is inside it. Every
/// member view - [`Node`](super::Node), [`Leaf`](super::Leaf),
/// [`Path`](super::Path) - is a name plus a shared reference to this, so
/// opening a member allocates nothing but its name, and two handles on one
/// member always agree about what is there.
///
/// # Laziness
///
/// Construction touches nothing. The index is parsed on the first operation
/// that needs it, an archive that does not exist yet indexes as empty, and the
/// resource is created by the first write, exactly as [`IOBase`] requires
/// everywhere else.
///
/// # Publishing
///
/// A member write appends its record after the last member and updates the
/// in-memory index; [`Self::flush`] writes the central directory after it.
/// Writing `n` members therefore costs `n` appends and one directory, not `n`
/// directories. Until that flush the stored archive still reads as its
/// previous state, because the directory an unfinished write has not reached
/// is still the one that indexes it.
///
/// Replacing a member leaves its previous bytes behind as dead space, which is
/// what makes the replacement an append rather than a rewrite. Removing one
/// compacts the archive on the next flush, so removal is the operation that
/// also reclaims what every earlier replacement left.
///
/// ```
/// use yggdryl::holder::{Buffer, Holder, zip::Archive};
/// use yggdryl::IOBase;
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
/// let archive = root.archive();
/// archive.write_member("trades/eu.csv", b"symbol,price\nAAPL,187.23\n")?;
/// archive.write_member("trades/us.csv", b"symbol,price\nMSFT,412.10\n")?;
/// archive.flush()?;
///
/// assert_eq!(archive.entries()?.len(), 2);
/// assert_eq!(
///     archive.read_member("trades/us.csv")?,
///     b"symbol,price\nMSFT,412.10\n",
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Archive {
    inner: Mutex<Inner>,
    /// The archive's own location, which every member URL is built under.
    url: Url,
    /// The coding a member write uses when nothing more specific applies.
    codec: Codec,
    /// The compression level that coding runs at.
    level: Level,
    /// Decoded bytes between the restart points a compressed write states.
    restart_stride: u64,
}

/// The archive's mutable state, held under one lock.
///
/// Every call this backend makes into the handle beneath it goes through the
/// counted methods below, so the cost model is a number a test can assert
/// rather than a claim in a comment.
#[derive(Debug)]
struct Inner {
    handle: Holder,
    /// The parsed index, materialized on first use.
    index: Option<Index>,
    /// Questions asked of the handle.
    reads: u64,
    /// Operations that changed the handle.
    writes: u64,
}

impl Inner {
    /// Read into `buffer` at `offset`.
    fn pread(&mut self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.reads += 1;
        self.handle.pread(offset, buffer)
    }

    /// Fill `buffer` at `offset`.
    fn pread_exact(&mut self, offset: u64, buffer: &mut [u8]) -> Result<()> {
        self.reads += 1;
        self.handle.pread_exact(offset, buffer)
    }

    /// Read `length` bytes at `offset`.
    fn read_range(&mut self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.reads += 1;
        self.handle.read_range_bytes(offset, length)
    }

    /// Ask the handle its byte length.
    fn size(&mut self) -> u64 {
        self.reads += 1;
        self.handle.size()
    }

    /// Write every byte at `offset`.
    fn pwrite_all(&mut self, offset: u64, bytes: &[u8]) -> Result<()> {
        self.writes += 1;
        self.handle.pwrite_all(offset, bytes)
    }

    /// Set the handle's byte length.
    fn truncate(&mut self, size: u64) -> Result<()> {
        self.writes += 1;
        self.handle.truncate(size)
    }

    /// Publish the handle's buffered bytes.
    fn flush(&mut self) -> Result<()> {
        self.writes += 1;
        self.handle.flush()
    }
}

/// The central directory, as this crate holds it between operations.
#[derive(Debug, Default)]
struct Index {
    /// Members by their canonical name; a directory keeps its trailing `/`.
    entries: BTreeMap<SmolStr, Entry>,
    /// Where the central directory starts, which is where an append goes.
    directory_offset: u64,
    /// The archive comment, preserved across every rewrite.
    comment: Vec<u8>,
    /// Whether the stored directory no longer matches this index.
    dirty: bool,
    /// Whether a removal left dead space the next flush must reclaim.
    compact: bool,
    /// Bytes before the first record, which belong to something else.
    ///
    /// A self-extracting archive carries a program there. It is not the
    /// archive's to move, so compaction packs the records behind it rather
    /// than over it.
    prologue: u64,
    /// Where each member's bytes start, once a local header has said so.
    ///
    /// A member's data offset is only knowable from its own local header, so
    /// it is read once and shared: every handle on that member, and every
    /// resolution of a location naming it, answers from here instead of
    /// reading the header again.
    data: BTreeMap<SmolStr, u64>,
    /// Members whose restart map has been proven against the member's bytes.
    ///
    /// One name per member that a positional read has actually seeked in, so
    /// the eight-byte proof is read once rather than once a seek.
    proven: BTreeSet<SmolStr>,
    /// The byte length the stored archive had when this index last matched it.
    ///
    /// A publish only has to shorten the archive when the trailer it writes
    /// ends before the previous one did, which is what this answers without
    /// asking the handle its size.
    stored_end: u64,
}

impl Archive {
    /// Mount `handle` as an archive without touching it.
    ///
    /// The archive's location is the handle's own, fragment included: an
    /// archive mounted over a member of another archive is that member, and
    /// its own members are named below it.
    pub fn new(handle: Holder) -> Self {
        let url = handle.url().cloned().unwrap_or_else(|| unlocated().clone());
        Self {
            url,
            inner: Mutex::new(Inner {
                handle,
                index: None,
                reads: 0,
                writes: 0,
            }),
            codec: Codec::Deflate,
            level: Level::DEFAULT,
            restart_stride: DEFAULT_RESTART_STRIDE,
        }
    }

    /// Mount the local archive `path` names, without touching it.
    ///
    /// # Errors
    ///
    /// Returns an error only when the path cannot be expressed as a canonical
    /// `file:` URL.
    pub fn from_path(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Holder::file(path).map(Self::new)
    }

    /// Return this archive with a different default member coding.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] naming a coding no ZIP compression
    /// method spells, which is gzip and zlib: their framing wraps a whole
    /// resource, while a member carries the raw DEFLATE stream inside it.
    pub fn try_with_codec(mut self, codec: Codec) -> Result<Self> {
        format::method_of(codec)?;
        self.codec = codec;
        Ok(self)
    }

    /// Return this archive with a different compression level.
    #[must_use]
    pub const fn with_level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// The coding a member write uses when nothing more specific applies.
    pub const fn codec(&self) -> Codec {
        self.codec
    }

    /// The compression level member writes run at.
    pub const fn level(&self) -> Level {
        self.level
    }

    /// Return this archive with a different restart stride for its writes.
    ///
    /// A compressed member is written as units this far apart in decoded
    /// bytes, and its record states where each begins, so a positional read
    /// decodes one unit rather than everything before the offset. Zero writes
    /// solid members, which are smaller and readable only from their first
    /// byte.
    ///
    /// ```
    /// use yggdryl::holder::{Buffer, Holder, zip::Archive};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let root = Archive::new(Holder::buffer(Buffer::new()))
    ///     .with_restart_stride(4_096)
    ///     .mount();
    /// root.archive()
    ///     .write_member("trades.csv", &b"symbol,price\nAAPL,187.23\n".repeat(512))?;
    ///
    /// let entry = root.archive().get_entry("trades.csv")?.expect("the member");
    /// assert_eq!(entry.restarts().stride(), 4_096);
    /// assert!(!entry.restarts().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub const fn with_restart_stride(mut self, stride: u64) -> Self {
        self.restart_stride = stride;
        self
    }

    /// Decoded bytes between the restart points a compressed write states.
    pub const fn restart_stride(&self) -> u64 {
        self.restart_stride
    }

    /// The archive's own location, which every member URL is built under.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// Hold this archive so its members can be addressed as resources.
    ///
    /// The answer is the archive root: a container whose children are the
    /// members, resolved by [`IOBase::child_by_path`] and walked by
    /// [`IOBase::ls`] exactly as any other container's are.
    ///
    /// ```
    /// use yggdryl::holder::{Buffer, Holder, zip::Archive};
    /// use yggdryl::IOBase;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
    ///
    /// let mut leaf = root.child_by_path("trades/eu.csv")?;
    /// leaf.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;
    ///
    /// // The member and the directory its name implies.
    /// assert_eq!(root.ls(true, false).count(), 2);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn mount(self) -> Node {
        Node::new(Arc::new(self), SmolStr::default())
    }

    /// Consume the archive, publishing the index and answering its handle.
    ///
    /// # Errors
    ///
    /// Returns the write failure a pending directory hit, or a poisoned-lock
    /// failure when another thread panicked while holding this archive.
    pub fn into_handle(self) -> Result<Holder> {
        self.flush()?;
        let inner = self
            .inner
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Ok(inner.handle)
    }

    /// Every member, in canonical name order.
    ///
    /// The result is the index itself, which the archive already holds: it is
    /// bounded by the number of members rather than by their contents, and no
    /// member byte is read to build it.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn entries(&self) -> Result<Vec<Entry>> {
        let mut guard = self.locked();
        Ok(Self::index(&mut guard)?.entries.values().cloned().collect())
    }

    /// The member `path` names, when the archive holds one.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn get_entry(&self, path: &str) -> Result<Option<Entry>> {
        let name = name::resolve("", path)?;
        self.entry(&name)
    }

    /// The archive comment, which is empty unless the archive states one.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn comment(&self) -> Result<Vec<u8>> {
        let mut guard = self.locked();
        Ok(Self::index(&mut guard)?.comment.clone())
    }

    /// Replace the archive comment, published by the next flush.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit, or a refusal
    /// when the comment is longer than the 65535 bytes the record can state.
    pub fn set_comment(&self, comment: &[u8]) -> Result<()> {
        if comment.len() > usize::from(u16::MAX) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "expected an archive comment of at most {} bytes, got {}",
                    u16::MAX,
                    comment.len()
                ),
            )));
        }
        let mut guard = self.locked();
        let index = Self::index(&mut guard)?;
        index.comment = comment.to_vec();
        index.dirty = true;
        Ok(())
    }

    /// Read one member whole, verifying the digest its record states.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Absent`] naming the member when it is not there, the
    /// decode failure, or an [`Error::Codec`] refusal when the decoded bytes
    /// do not match the record's CRC-32.
    pub fn read_member(self: &Arc<Self>, path: &str) -> Result<Vec<u8>> {
        let name = name::resolve("", path)?;
        let Some(entry) = self.entry(&name)? else {
            return Err(Error::absent("zip member", self.member_url(&name)));
        };
        self.read_entry(&entry)
    }

    /// Write one member under the archive's default coding.
    ///
    /// # Errors
    ///
    /// Returns the encode or write failure, or a refusal when `path` resolves
    /// to the archive root rather than to a member.
    pub fn write_member(&self, path: &str, bytes: &[u8]) -> Result<Entry> {
        self.write_member_with(path, bytes, self.codec)
    }

    /// Write one member under an explicit coding.
    ///
    /// # Errors
    ///
    /// Returns the encode or write failure, a refusal when `path` resolves to
    /// the archive root, or [`Error::Unsupported`] naming a coding no ZIP
    /// compression method spells.
    pub fn write_member_with(&self, path: &str, bytes: &[u8], codec: Codec) -> Result<Entry> {
        self.write_member_from(path, bytes, codec)
    }

    /// Write one member by streaming `source` through `codec` into the handle.
    ///
    /// Nothing is held whole: the source is read one batch at a time, encoded
    /// into one window, and the window written out when it fills, so a member
    /// larger than memory costs a window rather than its own size. The sizes
    /// and the digest are only known when the last byte is encoded, so the
    /// header reserves room for them and is settled afterwards - which is one
    /// extra write, and none at all for a member whose whole encoded form fit
    /// the first window.
    ///
    /// ```
    /// use yggdryl::holder::{Buffer, Holder, zip::Archive};
    /// use yggdryl::{Codec, IOBase};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let source = std::io::Cursor::new(b"symbol,price\nAAPL,187.23\n");
    /// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
    /// root.archive()
    ///     .write_member_from("trades/eu.csv", source, Codec::Deflate)?;
    /// root.archive().flush()?;
    ///
    /// assert_eq!(
    ///     root.archive().read_member("trades/eu.csv")?,
    ///     b"symbol,price\nAAPL,187.23\n",
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The archive is held for the whole write, because the record is placed
    /// where the directory currently begins and nothing may move that until
    /// it is there. So the source must not read through *this* archive: a
    /// member copied inside one archive goes through its value, while one
    /// copied between two archives streams.
    ///
    /// # Errors
    ///
    /// Returns the source's read failure, the encode or write failure, a
    /// refusal when `path` resolves to the archive root, or
    /// [`Error::Unsupported`] naming a coding no ZIP compression method
    /// spells.
    pub fn write_member_from(
        &self,
        path: &str,
        source: impl Read,
        codec: Codec,
    ) -> Result<Entry> {
        let name = name::resolve("", path)?;
        if name.is_empty() {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::IsADirectory,
                format!(
                    "expected a member to write into, got the archive root {}",
                    self.member_url_text("")
                ),
            )));
        }
        let method = format::method_of(codec)?;
        if name.len() > usize::from(format::ZIP64_MARK_16) {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "expected a member name of at most {} bytes, got {}",
                    format::ZIP64_MARK_16,
                    name.len()
                ),
            )));
        }
        let mut guard = self.locked();
        let inner = &mut *guard;
        let index = Self::index_of(inner)?;
        let offset = index.directory_offset;
        let previous = index.entries.get(name.as_str()).cloned();
        let mut entry = Entry::new(name, method, now_nanos()).with_header_offset(offset);
        // Replacing a member's bytes says nothing about the member: what the
        // record already stated about it is carried rather than reinvented.
        if let Some(previous) = &previous {
            entry = entry.with_facts_of(previous);
        }
        let stride = if codec.has_restarts() && !codec.is_identity() {
            self.restart_stride
        } else {
            0
        };
        self.write_record(inner, entry, source, codec, stride)
    }

    /// Encode `source` into one record and index what it wrote.
    fn write_record(
        &self,
        inner: &mut Inner,
        entry: Entry,
        mut source: impl Read,
        codec: Codec,
        stride: u64,
    ) -> Result<Entry> {
        let produced = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let mut sink = MemberSink {
            offset: entry.header_offset(),
            entry: entry.clone(),
            header: None,
            reserved: false,
            window: Vec::new(),
            flushed: 0,
            produced: Arc::clone(&produced),
            inner,
        };
        let mut crc = flate2::Crc::new();
        let mut log = RestartLog::new(stride);
        let mut size = 0_u64;
        {
            let mut encoder = codec.writer_with_level(&mut sink, self.level);
            let mut batch = vec![0_u8; crate::DEFAULT_STREAM_BATCH_SIZE];
            loop {
                let read = source.read(&mut batch)?;
                if read == 0 {
                    break;
                }
                let mut chunk = &batch[..read];
                while !chunk.is_empty() {
                    // A point lands exactly on its stride, so the map states
                    // where a unit begins without also stating how long it is.
                    let head = log.until(size).min(chunk.len());
                    if head == 0 {
                        encoder.restart()?;
                        log.record(produced.load(std::sync::atomic::Ordering::Relaxed));
                        continue;
                    }
                    encoder.write_all(&chunk[..head])?;
                    crc.update(&chunk[..head]);
                    size += head as u64;
                    chunk = &chunk[head..];
                }
            }
            encoder.finish()?;
        }
        let entry = entry
            .with_content(
                crc.sum(),
                produced.load(std::sync::atomic::Ordering::Relaxed),
                size,
            )
            .with_restarts(log.into_restarts());
        let data = sink.finish(&entry)?;
        Self::index_record(inner, &entry, data)?;
        Ok(entry)
    }

    /// Record a directory member, which a ZIP names with a trailing `/`.
    ///
    /// A member's parents are implied by its own name, so this is only needed
    /// for a directory that holds nothing yet - and recording one twice is
    /// success, because the record is already what it would be written as.
    ///
    /// # Errors
    ///
    /// Returns the write failure.
    pub fn create_directory(&self, path: &str) -> Result<()> {
        let base = name::resolve("", path)?;
        {
            let mut guard = self.locked();
            let inner = &mut *guard;
            if base.is_empty() {
                // The root is the archive itself; an empty one is a bare
                // trailer, which the flush below writes.
                Self::index_of(inner)?.dirty = true;
            } else {
                let directory = name::directory_name(&base);
                let index = Self::index_of(inner)?;
                if index.entries.contains_key(&directory) {
                    return Ok(());
                }
                let offset = index.directory_offset;
                let entry = Entry::new(directory, format::method_of(Codec::Identity)?, now_nanos())
                    .with_header_offset(offset);
                self.write_record(inner, entry, std::io::empty(), Codec::Identity, 0)?;
            }
        }
        self.flush()
    }

    /// Remove one member, answering whether it was there.
    ///
    /// Removing marks the archive for compaction, so the next flush rewrites
    /// it without the removed member's bytes and without any dead space an
    /// earlier replacement left.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn remove_member(&self, path: &str) -> Result<bool> {
        let name = name::resolve("", path)?;
        let directory = name::directory_name(&name);
        let mut guard = self.locked();
        let index = Self::index(&mut guard)?;
        let removed =
            index.entries.remove(&name).is_some() | index.entries.remove(&directory).is_some();
        if removed {
            index.data.remove(&name);
            index.data.remove(&directory);
            index.dirty = true;
            index.compact = true;
        }
        Ok(removed)
    }

    /// Remove every member, keeping the archive itself.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn clear_members(&self) -> Result<()> {
        let mut guard = self.locked();
        let index = Self::index(&mut guard)?;
        // Clearing is not a write: an archive that holds nothing is not
        // brought into being by being emptied.
        if !index.entries.is_empty() {
            index.entries.clear();
            index.data.clear();
            index.compact = true;
            index.dirty = true;
        }
        Ok(())
    }

    /// Publish the central directory, and with it every pending member write.
    ///
    /// # Errors
    ///
    /// Returns the compaction, write, or flush failure.
    pub fn flush(&self) -> Result<()> {
        let mut guard = self.locked();
        Self::publish(&mut guard)
    }

    /// Parse the central directory once and hold it for this scope.
    ///
    /// Every operation materializes what it needs, so this only moves that
    /// cost to a known point and keeps the index alive across many small
    /// operations instead of re-deriving it.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn open(&self) -> Result<()> {
        let mut guard = self.locked();
        Self::index(&mut guard)?;
        Ok(())
    }

    /// Whether the central directory is currently held in memory.
    pub fn is_indexed(&self) -> bool {
        self.locked().index.is_some()
    }

    /// Publish anything pending and release the index.
    ///
    /// # Errors
    ///
    /// Returns the write or flush failure.
    pub fn close(&self) -> Result<()> {
        let mut guard = self.locked();
        Self::publish(&mut guard)?;
        guard.index = None;
        Ok(())
    }

    /// Remove every member under the prefix `path`, answering how many.
    ///
    /// The directory record naming the prefix is kept, because emptying a
    /// container preserves the container.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn remove_under(&self, path: &str) -> Result<usize> {
        let base = name::resolve("", path)?;
        let mut guard = self.locked();
        let index = Self::index(&mut guard)?;
        let kept = name::directory_name(&base);
        let doomed: Vec<SmolStr> = index
            .entries
            .keys()
            .filter(|key| {
                *key != &kept
                    && name::under(&base, key.trim_end_matches('/'))
                        .is_some_and(|relative| !relative.is_empty())
            })
            .cloned()
            .collect();
        for key in &doomed {
            index.entries.remove(key);
        }
        if !doomed.is_empty() {
            index.dirty = true;
            index.compact = true;
        }
        Ok(doomed.len())
    }

    /// Whether the index differs from the directory the handle holds.
    pub fn is_pending(&self) -> bool {
        self.locked()
            .index
            .as_ref()
            .is_some_and(|index| index.dirty)
    }

    /// Delete the archive and everything in it.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure.
    pub fn remove(&self) -> Result<()> {
        let mut guard = self.locked();
        // The index goes first, so a later flush cannot recreate what is gone.
        guard.index = None;
        guard.writes += 1;
        guard.handle.remove(false)
    }

    /// The archive's own bytes, published first.
    ///
    /// # Errors
    ///
    /// Returns the publish or read failure.
    #[cfg(test)]
    pub(super) fn image(&self) -> Result<Vec<u8>> {
        self.flush()?;
        let mut guard = self.locked();
        guard.reads += 1;
        guard.handle.read_all_bytes()
    }

    /// The byte length of the archive itself.
    ///
    /// A parsed index already knows it - it is where the next record goes,
    /// plus the trailer behind it - so this asks the handle only while the
    /// archive is still unopened, exactly as any other cached metadata does.
    pub fn size(&self) -> u64 {
        let mut guard = self.locked();
        match guard.index.as_ref() {
            Some(index) => index.stored_end,
            None => guard.size(),
        }
    }

    /// How many questions this archive has asked the handle beneath it.
    ///
    /// The backend's cost model is stated in these terms, so it can be
    /// asserted in them: mounting an archive whose directory lies in its last
    /// 64 KiB is two reads, a warm positional read of a stored member is one,
    /// and a listing of any size is none.
    pub fn handle_reads(&self) -> u64 {
        self.locked().reads
    }

    /// How many operations this archive has run that changed that handle.
    ///
    /// Publishing `n` members is `n` record writes plus one trailer, one
    /// shortening only when the trailer ends earlier than the last one did,
    /// and one flush.
    pub fn handle_writes(&self) -> u64 {
        self.locked().writes
    }

    /// The URL a member of this archive is addressed by.
    ///
    /// The member's path is the URL *fragment*, so a member of
    /// `file:///lake/day.zip` is `file:///lake/day.zip#trades/eu.csv`. That
    /// keeps both facts in one location - which archive, and which member of
    /// it - where joining the member onto the path would say only that
    /// something lives below a name that happens to end in `.zip`, and would
    /// read identically to a real directory of that name.
    ///
    /// An archive inside an archive continues the same fragment, one level per
    /// [`NESTED`] separator: `day.zip#inner.zip//trades/eu.csv` is that member
    /// of that inner archive. The separator cannot collide with a name,
    /// because a canonical member path never holds an empty segment.
    ///
    /// It also makes the location a round trip: [`zip::from_url`](super::from_url)
    /// mounts the archive the base names, descends every level the fragment
    /// spells, and resolves the member at the end, so a member URL that was
    /// written down opens the member again however deeply it was nested.
    pub(super) fn member_url(&self, member: &str) -> Url {
        let member = member.trim_end_matches('/');
        let held = match self.url.fragment(true) {
            Ok(held) => held.filter(|held| !held.is_empty()),
            Err(_) => return self.url.clone(),
        };
        let fragment = match held {
            // A member-backed archive continues the chain past the marker,
            // and its own root is the marker with nothing after it - which is
            // what tells the archive apart from the member holding its bytes.
            Some(outer) => format_smolstr!("{outer}{NESTED}{member}"),
            None if member.is_empty() => return self.url.clone(),
            None => SmolStr::new(member),
        };
        let mut url = self.url.clone();
        // A member path is not URI text, so the fragment carries it encoded.
        // A name no fragment can state leaves the archive as the location.
        if url.set_fragment(Some(&fragment)).is_err() {
            return self.url.clone();
        }
        url
    }

    /// The member location an error names.
    pub(super) fn member_url_text(&self, member: &str) -> SmolStr {
        format_smolstr!("{}", self.member_url(member))
    }

    /// The parent of the archive itself, which is where its bytes live.
    pub(super) fn archive_parent(&self) -> Option<Holder> {
        self.locked().handle.parent()
    }

    /// The member `name` names, when the archive holds one.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub(super) fn entry(&self, name: &str) -> Result<Option<Entry>> {
        let mut guard = self.locked();
        Ok(Self::index(&mut guard)?.entries.get(name).cloned())
    }

    /// Whether the archive holds any member under the prefix `base`.
    ///
    /// The root prefix is the archive, which is always a container. Any other
    /// prefix is one exactly when a directory record names it or some member
    /// continues it, which a range over the ordered index answers without a
    /// walk.
    pub(super) fn holds_prefix(&self, base: &str) -> Result<bool> {
        if base.is_empty() {
            return Ok(true);
        }
        let directory = name::directory_name(base);
        let mut guard = self.locked();
        let index = Self::index(&mut guard)?;
        Ok(index
            .entries
            .range(directory.clone()..)
            .next()
            .is_some_and(|(key, _)| key.starts_with(directory.as_str())))
    }

    /// The member names one level of, or the whole tree under, `base`.
    ///
    /// The answer pairs each name with whether it is a container, so a caller
    /// builds the right role without asking the index twice. Names come from
    /// the index, which is the archive's own directory and already in memory,
    /// so a listing reads no member byte and holds no more than the level - or
    /// the subtree - it was asked for.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub(super) fn names_under(
        &self,
        base: &str,
        recursive: bool,
        include_private: bool,
    ) -> Result<Vec<(SmolStr, bool)>> {
        let mut guard = self.locked();
        let index = Self::index(&mut guard)?;
        // A directory is anything a record marks as one plus anything a
        // deeper member implies, so both are collected before either answers.
        let mut folders: BTreeSet<SmolStr> = BTreeSet::new();
        let mut files: BTreeSet<SmolStr> = BTreeSet::new();
        for key in index.entries.keys() {
            let Some(relative) = name::under(base, key.trim_end_matches('/')) else {
                continue;
            };
            if relative.is_empty() || (!include_private && name::is_private(relative)) {
                continue;
            }
            let (head, deeper) = name::head(relative);
            let child = join(base, head);
            if deeper || (!recursive && key.ends_with('/') && relative == head) {
                folders.insert(child.clone());
            }
            if !recursive {
                if !deeper && !key.ends_with('/') {
                    files.insert(child);
                }
                continue;
            }
            if key.ends_with('/') {
                folders.insert(join(base, relative));
            } else {
                files.insert(join(base, relative));
                // Every level between this member and the prefix is a
                // container the walk must name, whether or not a record does.
                let mut ancestor = relative;
                while let Some(parent) = name::parent(ancestor) {
                    folders.insert(join(base, parent));
                    ancestor = parent;
                }
            }
        }
        let mut listed: BTreeMap<SmolStr, bool> =
            files.into_iter().map(|name| (name, false)).collect();
        for folder in folders {
            listed.insert(folder, true);
        }
        Ok(listed.into_iter().collect())
    }

    /// Read raw archive bytes, which is what a stored member reads through.
    ///
    /// # Errors
    ///
    /// Returns the backing store's read failure.
    pub(super) fn pread_raw(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.locked().pread(offset, buffer)
    }

    /// Where a member's bytes start, read from its own local file header.
    ///
    /// The local header repeats the name and can carry different extra fields
    /// from the central record, so the answer is only knowable from the header
    /// itself. It is therefore read once per member and held in the index: a
    /// second handle on that member, and every resolution of a location naming
    /// it, answer from there. A member this archive wrote never costs the read
    /// at all, because the write already knew where it put the bytes.
    ///
    /// # Errors
    ///
    /// Returns the read failure, or [`Error::Codec`] when the local header is
    /// not one.
    pub(super) fn data_offset(&self, entry: &Entry) -> Result<u64> {
        let mut guard = self.locked();
        Self::member_data(&mut guard, entry)
    }

    /// Where a member's bytes start, from state already borrowed.
    fn member_data(inner: &mut Inner, entry: &Entry) -> Result<u64> {
        if let Some(data) = Self::index_of(inner)?.data.get(entry.name()).copied() {
            return Ok(data);
        }
        let mut header = [0_u8; format::LOCAL_LEN];
        inner.pread_exact(entry.header_offset(), &mut header)?;
        let data = format::local_data_offset(&header, entry.header_offset())?;
        Self::index_of(inner)?
            .data
            .insert(SmolStr::new(entry.name()), data);
        Ok(data)
    }

    /// The decoded length of one member, or zero when there is none.
    ///
    /// The index already holds it, so this answers without copying the record
    /// out of the map - which matters because every ranged read asks it first.
    pub(super) fn member_size(&self, name: &str) -> u64 {
        let mut guard = self.locked();
        Self::index_of(&mut guard)
            .ok()
            .and_then(|index| index.entries.get(name))
            .map_or(0, Entry::size)
    }

    /// Read from one member, when its bytes can pass straight through.
    ///
    /// A stored member is the archive's own bytes over a range, so the whole
    /// answer - which member, where its bytes start, and the read itself - is
    /// one lock and one call into the handle, with nothing copied and nothing
    /// allocated. `None` says the member needs a decode instead, and says it
    /// without having asked the handle anything.
    ///
    /// # Errors
    ///
    /// Returns the read failure, or [`Error::Codec`] when the local header the
    /// data offset comes from is not one.
    pub(super) fn pread_member(
        &self,
        name: &str,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<Option<usize>> {
        let mut guard = self.locked();
        let inner = &mut *guard;
        let index = Self::index_of(inner)?;
        let Some(entry) = index.entries.get(name) else {
            return Ok(None);
        };
        if entry.is_encrypted() || !matches!(entry.codec(), Ok(Codec::Identity)) {
            return Ok(None);
        }
        // A stored member decodes to itself, so the two sizes agree - and
        // where a malformed record says otherwise, the shorter one bounds the
        // read rather than letting it run into the next member.
        let stored = entry.size().min(entry.compressed_size());
        let header_offset = entry.header_offset();
        let cached = index.data.get(name).copied();

        let Some(available) = stored.checked_sub(offset) else {
            return Ok(Some(0));
        };
        let length = usize::try_from(available)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        if length == 0 {
            return Ok(Some(0));
        }
        let data = match cached {
            Some(data) => data,
            None => {
                let mut header = [0_u8; format::LOCAL_LEN];
                inner.pread_exact(header_offset, &mut header)?;
                let data = format::local_data_offset(&header, header_offset)?;
                Self::index_of(inner)?.data.insert(SmolStr::new(name), data);
                data
            }
        };
        inner.pread(data + offset, &mut buffer[..length]).map(Some)
    }

    /// Decode one member whole, verifying the digest its record states.
    ///
    /// # Errors
    ///
    /// Returns the read or decode failure, or [`Error::Codec`] naming both
    /// digests when the decoded bytes do not match the record's.
    pub(super) fn read_entry(self: &Arc<Self>, entry: &Entry) -> Result<Vec<u8>> {
        // A stored member is already the bytes it decodes to, so reading one
        // whole is one ranged read of the archive rather than a stream over
        // it - and the offset it reads at comes from the same lock.
        if !entry.is_encrypted() && entry.codec()?.is_identity() {
            let length = usize::try_from(entry.size().min(entry.compressed_size()))
                .map_err(|_| crate::iobase::oversized(entry.size()))?;
            let bytes = {
                let mut guard = self.locked();
                let inner = &mut *guard;
                let data = Self::member_data(inner, entry)?;
                inner.read_range(data, length)?
            };
            // Nothing streamed, so nothing hashed on the way past.
            verify_crc(entry, &bytes)?;
            return Ok(bytes);
        }
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        self.entry_reader(entry, 0)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Open a decoded reader over one member, retaining one bounded window.
    ///
    /// A stored member decodes to itself, so the reader is the archive's own
    /// bytes over the member's range: `position` is applied to the range and
    /// nothing before it is touched.
    ///
    /// A compressed member has no decoded seek, so the decode begins at the
    /// restart point its record maps at or before `position` and the rest of
    /// that unit is discarded through one bounded scratch buffer. What a read
    /// decodes is therefore bounded by the member's stride rather than by the
    /// offset - and for a member whose record maps nothing, which is what
    /// another writer's compressed member is, the point is the member's first
    /// byte and the whole prefix is discarded as it always was.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for an encrypted member or a compression
    /// method this build cannot decode, or the local header read failure.
    pub(super) fn entry_reader(
        self: &Arc<Self>,
        entry: &Entry,
        position: u64,
    ) -> Result<Box<dyn Read + Send>> {
        if entry.is_encrypted() {
            return Err(Error::unsupported(
                "reading an encrypted zip member",
                self.member_url_text(entry.name()),
            ));
        }
        let codec = entry.codec()?;
        let start = self.data_offset(entry)?;
        if codec.is_identity() {
            let skipped = position.min(entry.compressed_size());
            return Ok(Box::new(RangeReader {
                archive: Arc::clone(self),
                position: start + skipped,
                remaining: entry.compressed_size() - skipped,
            }));
        }

        let (decoded_at, encoded_at) = self.restart_before(entry, codec, position)?;
        let range = RangeReader {
            archive: Arc::clone(self),
            position: start + encoded_at,
            remaining: entry.compressed_size() - encoded_at,
        };
        let decoded = codec.reader_send(std::io::BufReader::with_capacity(
            crate::DEFAULT_STREAM_BATCH_SIZE,
            range,
        ));
        if position == decoded_at {
            // A reader that begins at the member's first byte can hash what
            // it hands out, so a stream read to its end proves the digest the
            // record states. One that begins anywhere else cannot: it never
            // sees the bytes before it.
            if position == 0 {
                return Ok(Box::new(Verified::new(decoded, entry)));
            }
            return Ok(decoded);
        }
        Ok(Box::new(Skip {
            reader: decoded,
            remaining: position - decoded_at,
        }))
    }

    /// The restart point a read at `position` decodes from.
    ///
    /// A record states its map, and a map states bytes: another tool that
    /// rewrote the member while keeping the record's extra fields would leave
    /// one describing bytes that are gone. So a point is proven against the
    /// coding's own evidence before it is trusted - the marker a full flush
    /// ends with, the magic a frame begins with - and a map that fails is
    /// dropped from the index rather than believed. That costs one read of
    /// eight bytes, once per member however many seeks follow, because what
    /// it proves is the map rather than the point.
    fn restart_before(
        self: &Arc<Self>,
        entry: &Entry,
        codec: Codec,
        position: u64,
    ) -> Result<(u64, u64)> {
        let (decoded_at, encoded_at) = entry.restarts().before(position);
        if encoded_at == 0 {
            return Ok((0, 0));
        }
        {
            let mut guard = self.locked();
            if Self::index_of(&mut guard)?.proven.contains(entry.name()) {
                return Ok((decoded_at, encoded_at));
            }
        }
        if self.restart_proven(entry, codec, encoded_at)? {
            let mut guard = self.locked();
            Self::index_of(&mut guard)?
                .proven
                .insert(SmolStr::new(entry.name()));
            return Ok((decoded_at, encoded_at));
        }
        // The map does not describe these bytes, so nothing may use it again.
        let mut guard = self.locked();
        let index = Self::index_of(&mut guard)?;
        if let Some(held) = index.entries.get_mut(entry.name()) {
            *held = held.clone().with_restarts(crate::Restarts::default());
        }
        index.proven.insert(SmolStr::new(entry.name()));
        Ok((0, 0))
    }

    /// Whether the coding's own evidence of a restart sits at `encoded_at`.
    ///
    /// The window spans both spellings a restart has - the four bytes a full
    /// flush ends with, and the four a frame begins with - so the scan the
    /// coding owns answers for either without this knowing which.
    fn restart_proven(&self, entry: &Entry, codec: Codec, encoded_at: u64) -> Result<bool> {
        let start = self.data_offset(entry)?;
        let Some(before) = encoded_at.checked_sub(RESTART_EVIDENCE) else {
            return Ok(false);
        };
        if encoded_at + RESTART_EVIDENCE > entry.compressed_size() {
            return Ok(false);
        }
        let window = self
            .locked()
            .read_range(start + before, 2 * RESTART_EVIDENCE as usize)?;
        let mut found = Vec::new();
        codec.restart_scan().push(&window, &mut found);
        Ok(found.contains(&RESTART_EVIDENCE))
    }


    /// Lock the archive, taking the state a panicking thread left behind.
    ///
    /// Every operation here either completes or leaves the index untouched,
    /// so a panic elsewhere in the process has nothing to have corrupted -
    /// and reporting the poison instead would answer "empty" from every
    /// accessor that cannot carry an error, which reads as an archive that is
    /// not there and invites a caller to write over one that is.
    fn locked(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Borrow the index, parsing the stored directory on first use.
    fn index<'guard>(guard: &'guard mut MutexGuard<'_, Inner>) -> Result<&'guard mut Index> {
        Self::index_of(&mut *guard)
    }

    /// Borrow the index of already-borrowed state.
    fn index_of(inner: &mut Inner) -> Result<&mut Index> {
        if inner.index.is_none() {
            inner.index = Some(parse(inner)?);
        }
        inner
            .index
            .as_mut()
            .ok_or_else(|| Error::Io(std::io::Error::other("the archive index was lost")))
    }

    /// Index the record a write just placed, which is the last thing it does.
    ///
    /// The index only learns about a member once its bytes are in the handle,
    /// so a failed write leaves an archive that never claimed them.
    fn index_record(inner: &mut Inner, entry: &Entry, data: u64) -> Result<()> {
        let name = SmolStr::new(entry.name());
        let index = Self::index_of(inner)?;
        index.directory_offset = data + entry.compressed_size();
        index.stored_end = index.stored_end.max(index.directory_offset);
        index.entries.insert(name.clone(), entry.clone());
        // The write knows where it put the bytes, so reading them back never
        // costs the local header read that would otherwise find out.
        index.data.insert(name.clone(), data);
        // Whatever was proven about the member that used to be here describes
        // bytes this write replaced.
        index.proven.remove(&name);
        index.dirty = true;
        Ok(())
    }

    /// Write the central directory and its trailer, compacting first when a
    /// removal asked for it.
    fn publish(guard: &mut MutexGuard<'_, Inner>) -> Result<()> {
        match guard.index.as_ref() {
            Some(index) if index.dirty => {}
            // Nothing has been parsed or nothing has changed, and a flush is
            // not a write: an untouched archive is not created by one.
            _ => return Ok(()),
        }
        if guard.index.as_ref().is_some_and(|index| index.compact) {
            Self::compact(guard)?;
        }
        let inner = &mut **guard;
        let index = Self::index_of(inner)?;
        let mut trailer = Vec::new();
        for entry in index.entries.values() {
            format::write_central(entry, &mut trailer);
        }
        let directory_size = trailer.len() as u64;
        let offset = index.directory_offset;
        format::write_end(
            format::End {
                directory_offset: offset,
                directory_size,
                entries: index.entries.len() as u64,
            },
            &index.comment,
            &mut trailer,
        );
        let end = offset + trailer.len() as u64;
        let shrinks = end < index.stored_end;
        inner.pwrite_all(offset, &trailer)?;
        // Shortening is only needed when this trailer ends before the last one
        // did; an archive that only grew has nothing beyond it to discard.
        if shrinks {
            inner.truncate(end)?;
        }
        inner.flush()?;
        // Only a published directory makes the index match the archive, so a
        // failed write leaves the flush still owed rather than forgotten.
        let index = Self::index_of(inner)?;
        index.dirty = false;
        index.stored_end = end;
        Ok(())
    }

    /// Move every surviving member to the front, reclaiming dead space.
    ///
    /// Records move in ascending offset order and only ever move *earlier*, so
    /// a forward copy always writes behind what it has already read. The local
    /// header travels verbatim rather than being rebuilt, which is what keeps
    /// each record exactly as long as it was and makes that guarantee hold.
    fn compact(guard: &mut MutexGuard<'_, Inner>) -> Result<()> {
        let inner = &mut **guard;
        let index = Self::index_of(inner)?;
        let mut ordered: Vec<Entry> = index.entries.values().cloned().collect();
        let mut cursor = index.prologue;
        ordered.sort_by_key(Entry::header_offset);

        let mut moved = Vec::with_capacity(ordered.len());
        let mut buffer = vec![0_u8; COMPACT_CHUNK];
        for entry in ordered {
            // Only the fixed head of the header is rewritten, so only it is
            // read: the name and extra fields behind it are unchanged, and
            // travel with the member's bytes in the copy below. A record whose
            // sizes need 64 bits is the exception - its ZIP64 extra carries
            // them, so that one is read and settled too.
            let mut settled = vec![0_u8; format::LOCAL_LEN];
            inner.pread_exact(entry.header_offset(), &mut settled)?;
            let data = format::local_data_offset(&settled, entry.header_offset())?;
            let header_len = usize::try_from(data - entry.header_offset()).map_err(|_| {
                format::malformed(
                    clamp(entry.header_offset()),
                    "the local header is longer than this platform can address",
                )
            })?;
            if format::settles_extra(&entry) {
                settled.resize(header_len, 0);
                inner.pread_exact(entry.header_offset(), &mut settled)?;
            }
            format::settle_local(&mut settled, &entry);
            inner.pwrite_all(cursor, &settled)?;

            // Whatever the head did not cover moves verbatim, in one pass with
            // the member's bytes rather than in a pass of its own.
            let copied = settled.len() as u64;
            let remaining = header_len as u64 - copied + entry.compressed_size();
            let mut done = 0_u64;
            while done < remaining {
                let step = usize::try_from(remaining - done)
                    .unwrap_or(COMPACT_CHUNK)
                    .min(COMPACT_CHUNK);
                let window = &mut buffer[..step];
                inner.pread_exact(entry.header_offset() + copied + done, window)?;
                inner.pwrite_all(cursor + copied + done, window)?;
                done += step as u64;
            }
            let record_len = header_len as u64 + entry.compressed_size();
            moved.push(entry.settled().with_header_offset(cursor));
            cursor += record_len;
        }

        let index = Self::index_of(inner)?;
        index.entries = moved
            .into_iter()
            .map(|entry| (SmolStr::new(entry.name()), entry))
            .collect();
        index.directory_offset = cursor;
        index.compact = false;
        // Every member moved, so no data offset the index held still holds.
        index.data.clear();
        Ok(())
    }
}

/// Join a member prefix and a relative name.
fn join(base: &str, relative: &str) -> SmolStr {
    if base.is_empty() {
        return SmolStr::new(relative);
    }
    format_smolstr!("{base}/{relative}")
}

/// Encoded bytes a member write holds before they reach the handle.
///
/// One transport window, so a member whose encoded form fits it is one write -
/// header and bytes together, because they are one record - and a longer one
/// is one write per window plus the write that settles its header.
const MEMBER_WINDOW: usize = crate::DEFAULT_FETCH_BYTE_SIZE;

/// Where a member write puts the encoded bytes an encoder hands it.
///
/// The header travels with the first window, because a member that fits one
/// window is one record and one write. A member that does not have its header
/// written with room reserved for the sizes nobody knows yet, and settled once
/// the last byte is encoded.
struct MemberSink<'inner> {
    inner: &'inner mut Inner,
    /// Where the record begins in the archive.
    offset: u64,
    /// The member being written, which the header spells.
    entry: Entry,
    /// The header's byte length, once one has been written.
    header: Option<u64>,
    /// Whether that header reserved room for 64-bit sizes.
    reserved: bool,
    /// Encoded bytes held back from the handle.
    window: Vec<u8>,
    /// Encoded bytes already written.
    flushed: u64,
    /// Encoded bytes handed in, which the restart log reads as it goes.
    produced: Arc<std::sync::atomic::AtomicU64>,
}

impl MemberSink<'_> {
    /// Write what the window holds, spelling the header on the first call.
    fn spill(&mut self, last: bool) -> Result<()> {
        match self.header {
            // A tail with nothing in it is not a write; the header is, even
            // for a member whose encoded form is empty.
            Some(_) if self.window.is_empty() => return Ok(()),
            Some(header) => {
                let at = self.offset + header + self.flushed;
                self.inner.pwrite_all(at, &self.window)?;
            }
            None => {
                // A member that ends inside its first window states its own
                // sizes; one that does not reserves the room to state them.
                self.reserved = !last;
                let mut record = Vec::with_capacity(
                    format::LOCAL_LEN + self.entry.name().len() + self.window.len(),
                );
                format::write_local_with(&self.entry, self.reserved, &mut record);
                self.header = Some(record.len() as u64);
                record.extend_from_slice(&self.window);
                self.inner.pwrite_all(self.offset, &record)?;
            }
        }
        self.flushed += self.window.len() as u64;
        self.window.clear();
        Ok(())
    }

    /// Write the tail, settle the header, and answer where the bytes start.
    fn finish(&mut self, entry: &Entry) -> Result<u64> {
        self.entry = entry.clone();
        self.spill(true)?;
        if self.reserved {
            let mut header = Vec::new();
            format::write_local_with(&self.entry, true, &mut header);
            self.inner.pwrite_all(self.offset, &header)?;
        }
        Ok(self.offset + self.header.unwrap_or(0))
    }
}

impl std::io::Write for MemberSink<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.window.extend_from_slice(buffer);
        self.produced
            .fetch_add(buffer.len() as u64, std::sync::atomic::Ordering::Relaxed);
        if self.window.len() >= MEMBER_WINDOW {
            self.spill(false).map_err(std::io::Error::other)?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The restart points a member write states, and the stride they are apart.
///
/// The map rides the central directory, so it is bounded rather than allowed
/// to grow: at the ceiling the stride doubles and every second point is
/// dropped, which is exactly the map the wider stride would have produced. The
/// points already written stay where they are - they cost size, not
/// correctness.
struct RestartLog {
    stride: u64,
    /// The decoded offset the next point belongs at.
    next: u64,
    points: Vec<u64>,
}

impl RestartLog {
    /// Log points `stride` decoded bytes apart, or none at all for zero.
    const fn new(stride: u64) -> Self {
        Self {
            stride,
            next: stride,
            points: Vec::new(),
        }
    }

    /// Decoded bytes still to write before the next point is due.
    fn until(&self, decoded: u64) -> usize {
        if self.stride == 0 {
            return usize::MAX;
        }
        usize::try_from(self.next.saturating_sub(decoded)).unwrap_or(usize::MAX)
    }

    /// Record a point that has just been written at encoded offset `at`.
    fn record(&mut self, at: u64) {
        self.points.push(at);
        self.next += self.stride;
        if self.points.len() >= format::MAX_RESTARTS {
            let mut kept = Vec::with_capacity(self.points.len() / 2);
            let mut index = 1;
            while index < self.points.len() {
                kept.push(self.points[index]);
                index += 2;
            }
            self.points = kept;
            self.stride *= 2;
            self.next = (self.points.len() as u64 + 1) * self.stride;
        }
    }

    /// The map the log built.
    fn into_restarts(self) -> crate::Restarts {
        crate::Restarts::new(self.stride, self.points)
    }
}

/// A bounded positional reader over the archive's own bytes.
///
/// One member's encoded range, read through the shared handle. Nothing is
/// retained between calls beyond the caller's buffer, so a decoder wrapping
/// this holds its own window and nothing else.
struct RangeReader {
    archive: Arc<Archive>,
    position: u64,
    remaining: u64,
}

impl Read for RangeReader {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if self.remaining == 0 || target.is_empty() {
            return Ok(0);
        }
        let length = usize::try_from(self.remaining)
            .unwrap_or(usize::MAX)
            .min(target.len());
        let read = self
            .archive
            .pread_raw(self.position, &mut target[..length])
            .map_err(std::io::Error::other)?;
        self.position += read as u64;
        self.remaining -= read as u64;
        Ok(read)
    }
}

/// Check a member's digest against what a stream of it actually decoded to.
///
/// The check happens at the end of the member and only there: a stream a
/// caller stops reading has proven nothing, and says so by not answering.
struct Verified {
    reader: Box<dyn Read + Send>,
    crc: flate2::Crc,
    entry: Entry,
    done: bool,
}

impl Verified {
    /// Hash what `reader` hands out, against what `entry` states.
    fn new(reader: Box<dyn Read + Send>, entry: &Entry) -> Self {
        Self {
            reader,
            crc: flate2::Crc::new(),
            entry: entry.clone(),
            done: false,
        }
    }
}

impl Read for Verified {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        let read = self.reader.read(target)?;
        if read > 0 {
            self.crc.update(&target[..read]);
            return Ok(read);
        }
        if !self.done {
            self.done = true;
            if self.crc.sum() != self.entry.crc32() {
                return Err(std::io::Error::other(digest_failure(
                    &self.entry,
                    self.crc.sum(),
                )));
            }
        }
        Ok(0)
    }
}

/// Discard a decoded prefix before serving the position a caller asked for.
///
/// What it discards is the distance from a restart point to the position, so
/// its bound is the member's stride rather than the position itself. A member
/// whose record maps no restart point is the one case where the two are the
/// same.
struct Skip {
    reader: Box<dyn Read + Send>,
    remaining: u64,
}

impl Read for Skip {
    fn read(&mut self, target: &mut [u8]) -> std::io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        let mut discarded = [0_u8; 8 * 1024];
        while self.remaining > 0 {
            let length = usize::try_from(self.remaining)
                .unwrap_or(usize::MAX)
                .min(discarded.len());
            let read = self.reader.read(&mut discarded[..length])?;
            if read == 0 {
                self.remaining = 0;
                return Ok(0);
            }
            self.remaining -= read as u64;
        }
        self.reader.read(target)
    }
}

/// Check decoded bytes against the digest their record states.
///
/// # Errors
///
/// Returns [`Error::Codec`] naming both digests when they disagree.
pub(super) fn verify_crc(entry: &Entry, bytes: &[u8]) -> Result<()> {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    if crc.sum() == entry.crc32() {
        return Ok(());
    }
    Err(digest_failure(entry, crc.sum()))
}

/// Report a member whose bytes do not hash to what its record states.
fn digest_failure(entry: &Entry, digest: u32) -> Error {
    Error::Codec {
        format: "zip",
        position: usize::try_from(entry.header_offset()).unwrap_or(usize::MAX),
        reason: format_smolstr!(
            "expected the member {:?} to hash to {:#010x}, got {digest:#010x}",
            entry.name(),
            entry.crc32(),
        ),
    }
}

/// The identity a member of an unlocated handle is addressed by.
///
/// Every handle the core ships answers a location - a buffer answers the
/// synthetic identity it reports for itself - so this only covers an outside
/// implementation that answers none at all.
fn unlocated() -> &'static Url {
    static UNLOCATED: std::sync::LazyLock<Url> = std::sync::LazyLock::new(|| {
        Url::from_str("mem://0/0x0").expect("the fallback identity is valid")
    });
    &UNLOCATED
}

/// The current instant, in UTC nanoseconds since the Unix epoch.
fn now_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|since| i64::try_from(since.as_nanos()).ok())
        .unwrap_or(0)
}

/// Parse the central directory of `handle`.
///
/// An archive with no bytes indexes as empty rather than failing: under the
/// laziness contract absence is emptiness, and the first write is what brings
/// the archive into being.
fn parse(inner: &mut Inner) -> Result<Index> {
    let size = inner.size();
    if size == 0 {
        return Ok(Index::default());
    }
    if size < format::END_LEN as u64 {
        return Err(format::malformed(
            0,
            format_smolstr!(
                "expected at least {} bytes of zip archive, got {size}",
                format::END_LEN
            ),
        ));
    }
    let window = size.min(END_SEARCH_LEN);
    let start = size - window;
    let tail = inner.read_range(start, usize::try_from(window).unwrap_or(usize::MAX))?;
    // Everything the mount still needs - the ZIP64 pair, the directory itself
    // - usually lies inside the tail already read, so it is sliced out of it
    // rather than asked for again. Only an archive whose directory is larger
    // than the search window costs a second read.
    let held = |at: u64, len: usize| -> Option<&[u8]> {
        let from = usize::try_from(at.checked_sub(start)?).ok()?;
        tail.get(from..from.checked_add(len)?)
    };
    let end_at = find_end(&tail).ok_or_else(|| {
        format::malformed(
            clamp(start),
            "expected an end of central directory record in the archive's last 64 KiB",
        )
    })?;
    let end_offset = start + end_at as u64;
    let (mut end, comment) = format::read_end(&tail[end_at..], clamp(end_offset))?;

    // The 32-bit record saturates rather than overflowing, so a saturated
    // field is the signal to read the 64-bit pair that sits before it.
    let mut trailer_offset = end_offset;
    if (end.entries == u64::from(format::ZIP64_MARK_16)
        || end.directory_offset == u64::from(format::ZIP64_MARK_32)
        || end.directory_size == u64::from(format::ZIP64_MARK_32))
        && end_offset >= format::ZIP64_LOCATOR_LEN as u64
    {
        let locator_at = end_offset - format::ZIP64_LOCATOR_LEN as u64;
        let locator = match held(locator_at, format::ZIP64_LOCATOR_LEN) {
            Some(held) => Cow::Borrowed(held),
            None => Cow::Owned(inner.read_range(locator_at, format::ZIP64_LOCATOR_LEN)?),
        };
        if let Some(record_at) = format::read_zip64_locator(&locator, clamp(locator_at))? {
            let record = match held(record_at, format::ZIP64_END_LEN) {
                Some(held) => Cow::Borrowed(held),
                None => Cow::Owned(inner.read_range(record_at, format::ZIP64_END_LEN)?),
            };
            end = format::read_zip64_end(&record, clamp(record_at))?;
            trailer_offset = record_at;
        }
    }

    // A self-extracting archive carries a program before its members, and the
    // offsets it records are relative to where the archive was assembled
    // rather than to where it now starts. The trailer says where the directory
    // really ends, so the difference is the shift every recorded offset needs.
    let declared_end = end
        .directory_offset
        .checked_add(end.directory_size)
        .ok_or_else(|| {
            format::malformed(
                clamp(trailer_offset),
                "the central directory extends past the address space",
            )
        })?;
    let shift = trailer_offset as i64 - declared_end as i64;
    let directory_offset = shift_offset(end.directory_offset, shift, trailer_offset)?;
    let directory_size = usize::try_from(end.directory_size).map_err(|_| {
        format::malformed(
            clamp(trailer_offset),
            "the central directory is larger than this platform can address",
        )
    })?;

    let records = match held(directory_offset, directory_size) {
        Some(held) => Cow::Borrowed(held),
        None => Cow::Owned(inner.read_range(directory_offset, directory_size)?),
    };
    let mut scan = format::Scan::new(&records, clamp(directory_offset));
    let mut entries = BTreeMap::new();
    for _ in 0..end.entries {
        if scan.remaining() < format::CENTRAL_LEN {
            break;
        }
        let entry = format::read_central(&mut scan)?;
        let header_offset = shift_offset(entry.header_offset(), shift, trailer_offset)?;
        let canonical = canonical_name(entry.name())?;
        let entry = entry
            .with_header_offset(header_offset)
            .with_name(canonical.clone());
        entries.insert(canonical, entry);
    }

    Ok(Index {
        entries,
        directory_offset,
        comment,
        dirty: false,
        compact: false,
        prologue: u64::try_from(shift).unwrap_or(0),
        data: BTreeMap::new(),
        proven: BTreeSet::new(),
        stored_end: size,
    })
}

/// Narrow an archive offset for a diagnostic that states one.
fn clamp(offset: u64) -> usize {
    usize::try_from(offset).unwrap_or(usize::MAX)
}

/// Apply a self-extracting archive's offset shift to one recorded offset.
fn shift_offset(offset: u64, shift: i64, trailer_offset: u64) -> Result<u64> {
    offset.checked_add_signed(shift).ok_or_else(|| {
        format::malformed(
            clamp(trailer_offset),
            format_smolstr!("a recorded offset of {offset} does not lie in the archive"),
        )
    })
}

/// Normalize one recorded member name into the key every role compares.
///
/// A name written as `./a//b` addresses the same member as `a/b`, so the index
/// stores one spelling of it; the trailing separator that marks a directory is
/// part of the name and is kept.
fn canonical_name(recorded: &str) -> Result<SmolStr> {
    let resolved = name::resolve("", recorded)?;
    if recorded.ends_with('/') && !resolved.is_empty() {
        return Ok(name::directory_name(&resolved));
    }
    Ok(resolved)
}

/// Find the end-of-central-directory record in an archive's tail.
///
/// The scan runs backwards, so the record's signature appearing inside a
/// stored member never wins over the real one. A record whose comment length
/// reaches exactly the end of the archive is taken immediately; otherwise the
/// latest record whose comment fits is used, which is what reads an archive
/// something was appended to.
fn find_end(tail: &[u8]) -> Option<usize> {
    let signature = format::END_SIGNATURE.to_le_bytes();
    let last = tail.len().checked_sub(format::END_LEN)?;
    let mut candidate = None;
    for offset in (0..=last).rev() {
        if tail[offset..offset + 4] != signature {
            continue;
        }
        let comment_len = usize::from(u16::from_le_bytes([tail[offset + 20], tail[offset + 21]]));
        let end = offset + format::END_LEN + comment_len;
        if end == tail.len() {
            return Some(offset);
        }
        if end < tail.len() && candidate.is_none() {
            candidate = Some(offset);
        }
    }
    candidate
}
