//! One member of a ZIP archive, as its central directory record states it.

use smol_str::SmolStr;

use crate::{Codec, Restarts, Result};

use super::format;

/// What the central directory says about one member.
///
/// An entry is metadata, not content: it says where a member's bytes are, how
/// long they are encoded and decoded, how they are encoded, and what they hash
/// to. Reading one costs no member byte, which is what lets an archive answer
/// a listing, a size, or a digest check without decompressing anything.
///
/// ```
/// use yggdryl::holder::{Buffer, Holder, zip::Archive};
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
/// root.archive()
///     .write_member("trades/eu.csv", b"symbol,price\nAAPL,187.23\n")?;
///
/// let entry = root.archive().get_entry("trades/eu.csv")?.expect("the member");
/// assert_eq!(entry.name(), "trades/eu.csv");
/// assert_eq!(entry.size(), 25);
/// assert!(!entry.is_directory());
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Entry {
    name: SmolStr,
    made_by: u16,
    flags: u16,
    method: u16,
    modified: i64,
    crc32: u32,
    compressed_size: u64,
    size: u64,
    header_offset: u64,
    external_attributes: u32,
    comment: SmolStr,
    restarts: Restarts,
    spelling: Option<Box<[u8]>>,
}

impl Entry {
    /// Assemble an entry from the exact fields a record carries.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn from_parts(
        name: SmolStr,
        made_by: u16,
        flags: u16,
        method: u16,
        modified: i64,
        crc32: u32,
        compressed_size: u64,
        size: u64,
        header_offset: u64,
        external_attributes: u32,
        comment: SmolStr,
    ) -> Self {
        Self {
            name,
            made_by,
            flags,
            method,
            modified,
            crc32,
            compressed_size,
            size,
            header_offset,
            external_attributes,
            comment,
            restarts: Restarts::default(),
            spelling: None,
        }
    }

    /// Describe a member this archive is about to write.
    pub(super) fn new(name: SmolStr, method: u16, modified: i64) -> Self {
        let directory = name.ends_with('/');
        Self {
            made_by: format::MADE_BY_THIS,
            flags: format::FLAG_UTF8,
            method,
            modified,
            crc32: 0,
            compressed_size: 0,
            size: 0,
            header_offset: 0,
            external_attributes: format::external_attributes(directory),
            comment: SmolStr::default(),
            restarts: Restarts::default(),
            spelling: None,
            name,
        }
    }

    /// The member's path inside the archive, always `/` separated.
    ///
    /// A directory entry keeps the trailing `/` the format writes it with, so
    /// the name is exactly what the record holds.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The coding the member's bytes are stored under.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`](crate::Error::Unsupported) naming the
    /// compression method when this build cannot decode it.
    pub fn codec(&self) -> Result<Codec> {
        format::codec_of(self.method)
    }

    /// The member's decoded byte length.
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// The member's encoded byte length, as it occupies the archive.
    pub const fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    /// The CRC-32 of the decoded bytes, which every read verifies.
    pub const fn crc32(&self) -> u32 {
        self.crc32
    }

    /// The modification time, in UTC nanoseconds since the Unix epoch.
    ///
    /// An Info-ZIP extended timestamp answers when the archive carries one;
    /// otherwise this is the MS-DOS pair, whose resolution is two seconds.
    pub const fn modified(&self) -> i64 {
        self.modified
    }

    /// Where the member's local file header starts in the archive.
    pub const fn header_offset(&self) -> u64 {
        self.header_offset
    }

    /// The member's comment, which is empty unless the archive states one.
    pub fn comment(&self) -> &str {
        &self.comment
    }

    /// Where the member's decode may begin, when its record maps that.
    ///
    /// A member this crate compressed carries the map beside its record, so a
    /// positional read decodes one stride rather than everything before the
    /// offset. A stored member needs none - every offset is addressable - and
    /// a member another writer compressed states none, which reads honestly
    /// as an empty map.
    ///
    /// Offsets are relative to the member's first data byte, so compaction
    /// moves a record without touching what it says.
    pub const fn restarts(&self) -> &Restarts {
        &self.restarts
    }

    /// Whether the record names a directory rather than a byte member.
    pub fn is_directory(&self) -> bool {
        self.name.ends_with('/')
    }

    /// Whether the member's bytes are encrypted, which this crate refuses.
    pub const fn is_encrypted(&self) -> bool {
        self.flags & format::FLAG_ENCRYPTED != 0
    }

    /// Whether the sizes follow the member's bytes instead of preceding them.
    ///
    /// The central directory states the true sizes either way, so this only
    /// describes how the local header was written.
    pub const fn has_data_descriptor(&self) -> bool {
        self.flags & format::FLAG_DATA_DESCRIPTOR != 0
    }

    /// The general purpose bit flags the record carries.
    pub(super) const fn flags(&self) -> u16 {
        self.flags
    }

    /// The compression method number the record carries.
    pub(super) const fn method(&self) -> u16 {
        self.method
    }

    /// The external file attributes the record carries.
    pub(super) const fn external_attributes(&self) -> u32 {
        self.external_attributes
    }

    /// The host system and specification version the record was made by.
    ///
    /// The high byte is what says how to read the external attributes, so a
    /// record rewritten under a different one changes what its mode means.
    pub(super) const fn made_by(&self) -> u16 {
        self.made_by
    }

    /// Restate this entry under what another record already said about it.
    ///
    /// A rewrite replaces a member's bytes, not the facts around them: the
    /// host that made the record, the mode it carries, and the comment beside
    /// it all describe the member rather than its content.
    pub(super) fn with_facts_of(mut self, previous: &Self) -> Self {
        self.made_by = previous.made_by;
        self.external_attributes = previous.external_attributes;
        self.comment = previous.comment.clone();
        self
    }

    /// Restate this entry with the sizes and digest a published member has.
    pub(super) fn with_content(mut self, crc32: u32, compressed_size: u64, size: u64) -> Self {
        self.crc32 = crc32;
        self.compressed_size = compressed_size;
        self.size = size;
        // The map describes bytes that are being replaced, so it goes with
        // them; the writer states the new one beside the new content.
        self.restarts = Restarts::default();
        self
    }

    /// The bytes a record spells this member's name with.
    ///
    /// A name is UTF-8 in every record this crate writes, and a record it did
    /// not write may spell one in a code page or under an extra field. The
    /// recorded bytes are kept so republishing a directory leaves an untouched
    /// member exactly as it was - a name re-encoded in the central record
    /// alone would no longer match the local header that introduces it.
    pub(super) fn spelling(&self) -> &[u8] {
        self.spelling
            .as_deref()
            .unwrap_or_else(|| self.name.as_bytes())
    }

    /// Restate this entry with the bytes its record spells the name with.
    pub(super) fn with_spelling(mut self, spelling: &[u8]) -> Self {
        self.spelling = (spelling != self.name.as_bytes()).then(|| spelling.into());
        self
    }

    /// Restate this entry with the restart map its record carries.
    pub(super) fn with_restarts(mut self, restarts: Restarts) -> Self {
        self.restarts = restarts;
        self
    }

    /// The host byte a record was made by, for a test that asserts it.
    #[cfg(test)]
    pub(super) const fn made_by_for_test(&self) -> u16 {
        self.made_by
    }

    /// The external attributes a record carries, for a test that asserts them.
    #[cfg(test)]
    pub(super) const fn external_attributes_for_test(&self) -> u32 {
        self.external_attributes
    }

    /// Restate this entry with the general purpose bit flags a record has.
    ///
    /// The writer only ever sets the UTF-8 bit, so this exists for the records
    /// it does not produce - an encrypted member, a streamed one - which the
    /// refusal and compatibility cases have to assemble themselves.
    #[cfg(test)]
    pub(super) const fn with_flags(mut self, flags: u16) -> Self {
        self.flags = flags;
        self
    }

    /// Restate this entry with the data-descriptor bit cleared.
    ///
    /// Compaction settles a moved record's sizes into its local header, so the
    /// bit that said they were coming later no longer describes it.
    pub(super) const fn settled(mut self) -> Self {
        self.flags &= !format::FLAG_DATA_DESCRIPTOR;
        self
    }

    /// Restate this entry under the canonical spelling of its name.
    ///
    /// A record written as `./a//b.txt` addresses the member `a/b.txt`, and
    /// the index holds one spelling of it, so the entry states that one too.
    pub(super) fn with_name(mut self, name: SmolStr) -> Self {
        self.name = name;
        self
    }

    /// Restate this entry at the local header offset it was written to.
    pub(super) const fn with_header_offset(mut self, header_offset: u64) -> Self {
        self.header_offset = header_offset;
        self
    }
}
