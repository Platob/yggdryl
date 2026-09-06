//! The mounted archive: one index over one byte handle.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::sync::{Arc, Mutex, MutexGuard};

use smol_str::{SmolStr, format_smolstr};

use crate::holder::Holder;
use crate::{Codec, Error, IOBase, Level, Result, Url};

use super::{Entry, Folder, format, name};

/// The most bytes an end-of-central-directory record can be from the end.
///
/// The record is fixed at 22 bytes plus a comment the format bounds at 65535,
/// so a tail this long always contains it wherever the comment ends.
const END_SEARCH_LEN: u64 = format::END_LEN as u64 + u16::MAX as u64;

/// Bytes moved per step while compacting, matching the shared stream batch.
const COMPACT_CHUNK: usize = crate::DEFAULT_STREAM_BATCH_SIZE;

/// A ZIP archive mounted over one byte handle.
///
/// The archive owns two things and nothing else: the handle its bytes live in,
/// and the central directory that says where each member is inside it. Every
/// member view - [`Folder`](super::Folder), [`File`](super::File),
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
}

/// The archive's mutable state, held under one lock.
#[derive(Debug)]
struct Inner {
    handle: Holder,
    /// The parsed index, materialized on first use.
    index: Option<Index>,
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
}

impl Archive {
    /// Mount `handle` as an archive without touching it.
    pub fn new(handle: Holder) -> Self {
        let mut url = handle.url().cloned().unwrap_or_else(|| unlocated().clone());
        // The archive is the whole resource; a fragment addresses one member
        // of it, so an archive mounted from a member URL is still the archive.
        let _ = url.set_fragment(None);
        Self {
            url,
            inner: Mutex::new(Inner {
                handle,
                index: None,
            }),
            codec: Codec::Deflate,
            level: Level::DEFAULT,
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
    pub fn mount(self) -> Folder {
        Folder::new(Arc::new(self), SmolStr::default())
    }

    /// Consume the archive, publishing the index and answering its handle.
    ///
    /// # Errors
    ///
    /// Returns the write failure a pending directory hit, or a poisoned-lock
    /// failure when another thread panicked while holding this archive.
    pub fn into_handle(self) -> Result<Holder> {
        self.flush()?;
        let inner = self.inner.into_inner().map_err(|_| poisoned())?;
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
        let mut guard = self.locked()?;
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
        let mut guard = self.locked()?;
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
        let mut guard = self.locked()?;
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
        let bytes = self.read_entry(&entry)?;
        verify_crc(&entry, &bytes)?;
        Ok(bytes)
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
        let encoded = codec.dump_with_level(bytes, self.level)?;
        let mut crc = flate2::Crc::new();
        crc.update(bytes);
        let mut guard = self.locked()?;
        let inner = &mut *guard;
        let offset = Self::index_of(inner)?.directory_offset;
        let entry = Entry::new(name, method, now_nanos())
            .with_content(crc.sum(), encoded.len() as u64, bytes.len() as u64)
            .with_header_offset(offset);
        Self::append_record(inner, &entry, &encoded)?;
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
            let mut guard = self.locked()?;
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
                Self::append_record(inner, &entry, &[])?;
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
        let mut guard = self.locked()?;
        let index = Self::index(&mut guard)?;
        let removed =
            index.entries.remove(&name).is_some() | index.entries.remove(&directory).is_some();
        if removed {
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
        let mut guard = self.locked()?;
        let index = Self::index(&mut guard)?;
        if !index.entries.is_empty() {
            index.entries.clear();
            index.compact = true;
        }
        index.dirty = true;
        Ok(())
    }

    /// Publish the central directory, and with it every pending member write.
    ///
    /// # Errors
    ///
    /// Returns the compaction, write, or flush failure.
    pub fn flush(&self) -> Result<()> {
        let mut guard = self.locked()?;
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
        let mut guard = self.locked()?;
        Self::index(&mut guard)?;
        Ok(())
    }

    /// Whether the central directory is currently held in memory.
    pub fn is_indexed(&self) -> bool {
        self.locked().is_ok_and(|guard| guard.index.is_some())
    }

    /// Publish anything pending and release the index.
    ///
    /// # Errors
    ///
    /// Returns the write or flush failure.
    pub fn close(&self) -> Result<()> {
        let mut guard = self.locked()?;
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
        let mut guard = self.locked()?;
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
            .is_ok_and(|guard| guard.index.as_ref().is_some_and(|index| index.dirty))
    }

    /// Delete the archive and everything in it.
    ///
    /// # Errors
    ///
    /// Returns the backing store's delete failure.
    pub fn remove(&self) -> Result<()> {
        let mut guard = self.locked()?;
        // The index goes first, so a later flush cannot recreate what is gone.
        guard.index = None;
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
        self.locked()?.handle.read_all_bytes()
    }

    /// The byte length of the archive itself.
    pub fn size(&self) -> u64 {
        self.locked().map_or(0, |guard| guard.handle.size())
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
    /// It also makes the location a round trip: [`zip::from_url`](super::from_url)
    /// mounts the archive the base names and resolves the member the fragment
    /// names, so a member URL that was written down opens the member again.
    pub(super) fn member_url(&self, member: &str) -> Url {
        let member = member.trim_end_matches('/');
        if member.is_empty() {
            return self.url.clone();
        }
        let mut url = self.url.clone();
        // A member path is not URI text, so the fragment carries it encoded.
        // A name no fragment can state leaves the archive as the location.
        if url.set_fragment(Some(member)).is_err() {
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
        self.locked().ok()?.handle.parent()
    }

    /// The member `name` names, when the archive holds one.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub(super) fn entry(&self, name: &str) -> Result<Option<Entry>> {
        let mut guard = self.locked()?;
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
        let mut guard = self.locked()?;
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
        let mut guard = self.locked()?;
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
        self.locked()?.handle.pread(offset, buffer)
    }

    /// Where a member's bytes start, read from its own local file header.
    ///
    /// The local header repeats the name and can carry different extra fields
    /// from the central record, so this is the one read a member's first byte
    /// costs. A caller that reads a member more than once caches the answer.
    ///
    /// # Errors
    ///
    /// Returns the read failure, or [`Error::Codec`] when the local header is
    /// not one.
    pub(super) fn data_offset(&self, entry: &Entry) -> Result<u64> {
        let mut header = [0_u8; format::LOCAL_LEN];
        let guard = self.locked()?;
        guard
            .handle
            .pread_exact(entry.header_offset(), &mut header)?;
        format::local_data_offset(&header, entry.header_offset())
    }

    /// Decode one member whole, without verifying its digest.
    ///
    /// # Errors
    ///
    /// Returns the read or decode failure.
    pub(super) fn read_entry(self: &Arc<Self>, entry: &Entry) -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        self.entry_reader(entry, 0)?.read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Open a decoded reader over one member, retaining one bounded window.
    ///
    /// A stored member decodes to itself, so the reader is the archive's own
    /// bytes over the member's range: `position` is applied to the range and
    /// nothing before it is touched. A compressed member has no decoded seek,
    /// so the same range runs through the coding its record names and the
    /// decoded prefix is discarded through one bounded scratch buffer.
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
        let range = RangeReader {
            archive: Arc::clone(self),
            position: start,
            remaining: entry.compressed_size(),
        };
        let decoded = codec.reader_send(std::io::BufReader::with_capacity(
            crate::DEFAULT_STREAM_BATCH_SIZE,
            range,
        ));
        if position == 0 {
            return Ok(decoded);
        }
        Ok(Box::new(Skip {
            reader: decoded,
            remaining: position,
        }))
    }

    /// Lock the archive, naming a poisoned lock rather than panicking on it.
    fn locked(&self) -> Result<MutexGuard<'_, Inner>> {
        self.inner.lock().map_err(|_| poisoned())
    }

    /// Borrow the index, parsing the stored directory on first use.
    fn index<'guard>(guard: &'guard mut MutexGuard<'_, Inner>) -> Result<&'guard mut Index> {
        Self::index_of(&mut *guard)
    }

    /// Borrow the index of already-borrowed state.
    fn index_of(inner: &mut Inner) -> Result<&mut Index> {
        if inner.index.is_none() {
            inner.index = Some(parse(&inner.handle)?);
        }
        inner
            .index
            .as_mut()
            .ok_or_else(|| Error::Io(std::io::Error::other("the archive index was lost")))
    }

    /// Append one member record after the last member.
    fn append_record(inner: &mut Inner, entry: &Entry, encoded: &[u8]) -> Result<()> {
        let offset = entry.header_offset();
        let mut header = Vec::with_capacity(format::LOCAL_LEN + entry.name().len());
        format::write_local(entry, &mut header);
        let index = Self::index_of(inner)?;
        index.directory_offset = offset + header.len() as u64 + encoded.len() as u64;
        index
            .entries
            .insert(SmolStr::new(entry.name()), entry.clone());
        index.dirty = true;
        inner.handle.pwrite_all(offset, &header)?;
        inner
            .handle
            .pwrite_all(offset + header.len() as u64, encoded)
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
        index.dirty = false;
        inner.handle.pwrite_all(offset, &trailer)?;
        inner.handle.truncate(offset + trailer.len() as u64)?;
        inner.handle.flush()
    }

    /// Move every surviving member to the front, reclaiming dead space.
    ///
    /// Records move in ascending offset order and only ever move *earlier*, so
    /// a forward copy always writes behind what it has already read. The local
    /// header travels verbatim rather than being rebuilt, which is what keeps
    /// each record exactly as long as it was and makes that guarantee hold.
    fn compact(guard: &mut MutexGuard<'_, Inner>) -> Result<()> {
        let inner = &mut **guard;
        let mut ordered: Vec<Entry> = Self::index_of(inner)?.entries.values().cloned().collect();
        ordered.sort_by_key(Entry::header_offset);

        let mut cursor = 0_u64;
        let mut moved = Vec::with_capacity(ordered.len());
        let mut buffer = vec![0_u8; COMPACT_CHUNK];
        for entry in ordered {
            let mut fixed = [0_u8; format::LOCAL_LEN];
            inner
                .handle
                .pread_exact(entry.header_offset(), &mut fixed)?;
            let data = format::local_data_offset(&fixed, entry.header_offset())?;
            let header_len = usize::try_from(data - entry.header_offset()).map_err(|_| {
                format::malformed(
                    clamp(entry.header_offset()),
                    "the local header is longer than this platform can address",
                )
            })?;

            // The header is rewritten rather than copied, so a record a
            // streaming writer left incomplete states its own sizes once it
            // has moved away from the trailer that carried them. Its length
            // never changes, which is what keeps the copy below moving only
            // backwards.
            let mut header = vec![0_u8; header_len];
            inner
                .handle
                .pread_exact(entry.header_offset(), &mut header)?;
            format::settle_local(&mut header, &entry);
            inner.handle.pwrite_all(cursor, &header)?;

            let mut done = 0_u64;
            while done < entry.compressed_size() {
                let step = usize::try_from(entry.compressed_size() - done)
                    .unwrap_or(COMPACT_CHUNK)
                    .min(COMPACT_CHUNK);
                let window = &mut buffer[..step];
                inner.handle.pread_exact(data + done, window)?;
                inner
                    .handle
                    .pwrite_all(cursor + header_len as u64 + done, window)?;
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

/// Discard a decoded prefix before serving the position a caller asked for.
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
    Err(Error::Codec {
        format: "zip",
        position: usize::try_from(entry.header_offset()).unwrap_or(usize::MAX),
        reason: format_smolstr!(
            "expected the member {:?} to hash to {:#010x}, got {:#010x}",
            entry.name(),
            entry.crc32(),
            crc.sum()
        ),
    })
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

/// Report a lock another thread panicked while holding.
fn poisoned() -> Error {
    Error::Io(std::io::Error::other(
        "the zip archive lock was poisoned by a panic in another thread",
    ))
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
fn parse(handle: &Holder) -> Result<Index> {
    let size = handle.size();
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
    let tail = handle.read_range_bytes(start, usize::try_from(window).unwrap_or(usize::MAX))?;
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
        let locator = handle.read_range_bytes(locator_at, format::ZIP64_LOCATOR_LEN)?;
        if let Some(record_at) = format::read_zip64_locator(&locator, clamp(locator_at))? {
            let record = handle.read_range_bytes(record_at, format::ZIP64_END_LEN)?;
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

    let records = handle.read_range_bytes(directory_offset, directory_size)?;
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
