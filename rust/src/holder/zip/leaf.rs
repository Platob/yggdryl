//! One member of an archive, as a byte leaf.

use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::holder::Holder;
use crate::{ByteStream, Codec, Error, IOBase, IOFile, IOKind, MediaType, Result, Url};

use super::{Archive, Entry, Node, name};

/// One archive member's bytes, addressed positionally.
///
/// A member is a byte leaf like any other: it reads at an offset, writes at an
/// offset, and reports a size. What makes it worth its own implementation is
/// how little that has to cost.
///
/// # Reading
///
/// A **stored** member is the archive's own bytes over a range, so a
/// positional read is one positional read of the archive at the member's data
/// offset. Nothing is decompressed, nothing is copied beyond the caller's
/// buffer, and nothing is retained between calls - a hundred small reads
/// across a gigabyte member cost a hundred small reads.
///
/// A **compressed** member has no decoded seek, so a closed positional read
/// decodes from the member's first byte and discards what precedes the offset
/// through one bounded scratch buffer. Only the requested bytes are retained.
/// A caller doing many positional reads over one compressed member calls
/// [`IOBase::open`], which decodes it once and answers every later read out of
/// that value until [`IOBase::close`].
///
/// # Writing
///
/// A ZIP member is one compressed unit, so a positional write materializes the
/// decoded member, applies the write, and republishes the whole member on
/// [`IOBase::flush`] - the same shape a content coding has. The republished
/// member is appended and the archive's directory rewritten, so writing never
/// rewrites members it did not touch.
///
/// ```
/// use yggdryl::holder::{Buffer, Holder, zip::Archive};
/// use yggdryl::IOBase;
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
/// let mut member = root.child_by_path("trades/eu.csv")?;
/// member.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;
///
/// // Positional reads address the decoded member.
/// assert_eq!(member.read_range_bytes(7, 5)?, b"price");
///
/// // And positional writes replace part of it.
/// member.pwrite(0, b"ticker")?;
/// member.flush()?;
/// assert_eq!(member.read_range_bytes(0, 6)?, b"ticker");
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Leaf {
    archive: Arc<Archive>,
    /// The member's canonical path inside the archive.
    name: SmolStr,
    /// The member's location, which is the archive's with the name under it.
    url: Url,
    /// An explicit representation supplied by the caller.
    declared: Option<MediaType>,
    /// Inference from the member's compound name, computed on demand.
    inferred: OnceLock<MediaType>,
    /// An explicit coding a write stores this member under.
    codec: Option<Codec>,
    /// The decoded member, materialized by an open or a positional write.
    plain: Option<Vec<u8>>,
    /// Whether `plain` holds changes the archive has not seen.
    dirty: bool,
    /// What the archive said about the member when this handle decoded it.
    ///
    /// A staged value describes the bytes it was decoded from, so publishing
    /// it over a member another handle has since replaced would drop that
    /// write silently. The two are compared instead, and the conflict named.
    decoded_from: Decoded,
}

/// What the archive said about a member when a handle decoded it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Decoded {
    /// Nothing was decoded, so nothing constrains what a publish replaces.
    Nothing,
    /// The archive held no such member.
    Absent,
    /// The member's record began at this offset.
    Record(u64),
}

impl Decoded {
    /// What the archive says about the member now.
    fn of(entry: Option<&Entry>) -> Self {
        entry.map_or(Self::Absent, |entry| Self::Record(entry.header_offset()))
    }
}

impl Leaf {
    /// Address one member of `archive` without touching it.
    pub fn new(archive: Arc<Archive>, name: SmolStr) -> Self {
        Self {
            url: archive.member_url(&name),
            archive,
            name,
            declared: None,
            inferred: OnceLock::new(),
            codec: None,
            plain: None,
            dirty: false,
            decoded_from: Decoded::Nothing,
        }
    }

    /// Return this member with an explicit coding for what is written to it.
    ///
    /// Without one a write reuses the coding the member is already stored
    /// under, stores a representation that already carries a content coding,
    /// and otherwise uses the archive's default.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] naming a coding no ZIP compression
    /// method spells.
    pub fn try_with_codec(mut self, codec: Codec) -> Result<Self> {
        super::format::method_of(codec)?;
        self.codec = Some(codec);
        Ok(self)
    }

    /// Borrow the archive this member lives in.
    pub const fn archive(&self) -> &Arc<Archive> {
        &self.archive
    }

    /// The member's canonical path inside the archive.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the member's location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// What the archive's directory says about this member, when it holds one.
    ///
    /// # Errors
    ///
    /// Returns the read or format failure the index parse hit.
    pub fn get_entry(&self) -> Result<Option<Entry>> {
        self.archive.entry(&self.name)
    }

    /// Borrow the decoded member when this handle is holding one.
    const fn materialized(&self) -> Option<&Vec<u8>> {
        self.plain.as_ref()
    }

    /// Materialize the decoded member, decoding the archive's bytes once.
    fn decoded(&mut self) -> Result<&mut Vec<u8>> {
        if self.plain.is_none() {
            self.decoded_from = Decoded::of(self.get_entry()?.as_ref());
            self.plain = Some(self.read_whole()?);
        }
        self.plain
            .as_mut()
            .ok_or_else(|| Error::Io(std::io::Error::other("the decoded member was lost")))
    }

    /// Read the member whole, answering nothing for one that is not there.
    fn read_whole(&self) -> Result<Vec<u8>> {
        let Some(entry) = self.get_entry()? else {
            return Ok(Vec::new());
        };
        self.archive.read_entry(&entry)
    }

    /// The coding a write stores this member under.
    fn write_codec(&self) -> Result<Codec> {
        if let Some(codec) = self.codec {
            return Ok(codec);
        }
        if let Some(entry) = self.get_entry()? {
            if let Ok(codec) = entry.codec() {
                return Ok(codec);
            }
        }
        // A representation that is already compressed is stored as it is:
        // recoding a `.csv.gz` member costs time and grows it, and an archive
        // member stored whole is one whose own members stay addressable.
        let media_type = self.media_type();
        if !Codec::from_media_type(media_type).is_identity() || media_type.base().is_archive() {
            return Ok(Codec::Identity);
        }
        Ok(self.archive.codec())
    }

    /// Write the decoded member back into the archive.
    fn publish(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if self.decoded_from != Decoded::Nothing
            && Decoded::of(self.get_entry()?.as_ref()) != self.decoded_from
        {
            return Err(Error::conflict(
                "the member this value was decoded from",
                "a member another handle has since written",
                &self.url,
            ));
        }
        let codec = self.write_codec()?;
        let plain = self.plain.take().unwrap_or_default();
        let entry = self
            .archive
            .write_member_from(&self.name, &plain[..], codec)?;
        self.decoded_from = Decoded::Record(entry.header_offset());
        self.plain = Some(plain);
        self.dirty = false;
        self.archive.flush()
    }

    /// Copy `buffer.len()` bytes of `plain` from `offset`, returning what fit.
    fn copy_range(plain: &[u8], offset: u64, buffer: &mut [u8]) -> usize {
        let Ok(offset) = usize::try_from(offset) else {
            return 0;
        };
        let Some(available) = plain.len().checked_sub(offset) else {
            return 0;
        };
        let read = available.min(buffer.len());
        buffer[..read].copy_from_slice(&plain[offset..offset + read]);
        read
    }
}

impl IOFile for Leaf {
    fn file_url(&self) -> &Url {
        &self.url
    }

    fn file_exists(&self) -> bool {
        self.get_entry().ok().flatten().is_some()
    }

    /// Empty the member without bringing an absent one into being.
    fn clear_file(&mut self) -> Result<()> {
        self.plain = None;
        self.dirty = false;
        self.decoded_from = Decoded::Nothing;
        if !self.file_exists() {
            return Ok(());
        }
        let codec = self.write_codec()?;
        self.archive
            .write_member_from(&self.name, std::io::empty(), codec)?;
        self.archive.flush()
    }

    /// Delete the member, dropping any pending write with it.
    fn delete_file(&mut self) -> Result<()> {
        self.plain = None;
        self.dirty = false;
        self.decoded_from = Decoded::Nothing;
        if self.archive.remove_member(&self.name)? {
            return self.archive.flush();
        }
        Ok(())
    }
}

impl crate::IOMedia for Leaf {
    crate::impl_default_iomedia!();
}

impl IOBase for Leaf {
    /// Read the range out of the decoded member.
    ///
    /// A stored member reads straight out of the archive at its data offset;
    /// nothing is decompressed and nothing is retained. A compressed one
    /// decodes only far enough to fill this call unless the handle is open, in
    /// which case it answers from the value it already holds.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if let Some(plain) = self.materialized() {
            return Ok(Self::copy_range(plain, offset, buffer));
        }
        if buffer.is_empty() {
            return Ok(0);
        }
        // The stored answer is one lock and one read, so it is asked for
        // first; `None` says this member needs a decode instead.
        if let Some(read) = self.archive.pread_member(&self.name, offset, buffer)? {
            return Ok(read);
        }
        let Some(entry) = self.get_entry()? else {
            return Ok(0);
        };
        // A decode step answers whatever one step produced, which is short of
        // the buffer far more often than the member is short of the offset,
        // and `pread` is short only at the end of a value.
        let mut reader = self.archive.entry_reader(&entry, offset)?;
        let mut filled = 0;
        while filled < buffer.len() {
            let read = std::io::Read::read(&mut reader, &mut buffer[filled..])?;
            if read == 0 {
                break;
            }
            filled += read;
        }
        Ok(filled)
    }

    /// Stream the decoded member without materializing it.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<ByteStream<'_>> {
        if let Some(plain) = self.materialized() {
            let plain = usize::try_from(position)
                .ok()
                .and_then(|position| plain.get(position..))
                .unwrap_or_default();
            return ByteStream::from_reader(std::io::Cursor::new(plain), batch_size);
        }
        let Some(entry) = self.get_entry()? else {
            return ByteStream::from_reader(std::io::empty(), batch_size);
        };
        ByteStream::from_reader(self.archive.entry_reader(&entry, position)?, batch_size)
    }

    /// Replace the member, streaming the value into the archive.
    ///
    /// The member that was there is not decoded to be discarded, and the
    /// value is not staged: it goes through the coding into the archive as it
    /// is read, and the directory publishes it.
    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let codec = self.write_codec()?;
        self.archive.write_member_from(&self.name, bytes, codec)?;
        self.plain = None;
        self.dirty = false;
        self.decoded_from = Decoded::Nothing;
        self.archive.flush()
    }

    /// Read the member whole, verifying the digest its record states.
    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        if let Some(plain) = self.materialized() {
            return Ok(plain.clone());
        }
        self.read_whole()
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let offset = usize::try_from(offset).map_err(|_| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("expected an offset within a decoded member, got {offset}"),
            ))
        })?;
        let plain = self.decoded()?;
        let end = offset.checked_add(bytes.len()).ok_or_else(|| {
            Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "the written member range exceeds addressable memory",
            ))
        })?;
        if plain.len() < end {
            // Writing past the end grows the member and zero-fills any gap.
            plain.resize(end, 0);
        }
        plain[offset..end].copy_from_slice(bytes);
        self.dirty = true;
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        if let Some(plain) = self.materialized() {
            return plain.len() as u64;
        }
        self.archive.member_size(&self.name)
    }

    fn mtime(&self) -> Option<i64> {
        self.archive.member_mtime(&self.name)
    }

    fn capacity(&self) -> u64 {
        self.plain
            .as_ref()
            .map_or_else(|| self.size(), |plain| plain.capacity() as u64)
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let capacity = usize::try_from(capacity).unwrap_or(usize::MAX);
        let plain = self.decoded()?;
        if capacity > plain.len() {
            plain.reserve(capacity - plain.len());
        }
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        let size = usize::try_from(size).unwrap_or(usize::MAX);
        if size == 0 {
            // Emptying a member keeps nothing of it, so nothing is decoded to
            // find that out.
            self.plain = Some(Vec::new());
            self.dirty = true;
            return Ok(());
        }
        let plain = self.decoded()?;
        if size < plain.len() {
            plain.truncate(size);
        } else {
            plain.resize(size, 0);
        }
        self.dirty = true;
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    /// The representation the member's own name spells.
    ///
    /// The location's path names the archive, not the member, so inference
    /// reads the member's name: `trades.csv.gz` inside `day.zip` is a gzipped
    /// CSV, where the URL alone would say every member is a ZIP.
    fn media_type(&self) -> &MediaType {
        if let Some(media_type) = &self.declared {
            return media_type;
        }
        self.inferred
            .get_or_init(|| MediaType::from_file_name(name::base_name(&self.name)))
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    fn flush(&mut self) -> Result<()> {
        self.publish()
    }

    /// Decode the member once and answer every read from it until close.
    fn open(&mut self) -> Result<()> {
        self.decoded()?;
        Ok(())
    }

    fn opened(&self) -> bool {
        self.plain.is_some()
    }

    fn close(&mut self) -> Result<()> {
        self.publish()?;
        self.plain = None;
        self.decoded_from = Decoded::Nothing;
        Ok(())
    }

    /// The archive directory this member lives in.
    ///
    /// A member at the archive root answers the archive root itself, which is
    /// the container it is in - the archive's own parent is one level further
    /// out and belongs to the archive, not to its members.
    fn parent(&self) -> Option<Holder> {
        let base = name::parent(&self.name).unwrap_or_default();
        Some(Holder::ZipNode(Node::new(
            Arc::clone(&self.archive),
            SmolStr::new(base),
        )))
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.file_child_by_path(path)
    }

    /// The Hive partitions the archive's location and the member name spell.
    ///
    /// Both halves count: a lake can partition the archives themselves and
    /// partition again inside one, and a member carries whichever it is under.
    fn partitions(&self) -> Vec<(String, String)> {
        super::member_partitions(&self.archive, &self.name)
    }

    fn ls(&self, _recursive: bool, _include_private: bool) -> crate::Listing {
        self.file_ls()
    }

    fn kind(&self) -> IOKind {
        self.file_kind()
    }

    fn clear(&mut self) -> Result<()> {
        self.clear_file()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.file_remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        self.file_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.file_is_tabular()
    }
}
