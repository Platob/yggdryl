//! One member location, whatever it turns out to be.

use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::holder::Holder;
use crate::{IOBase, IOKind, IOPath, Listing, MediaType, MimeType, Result, Url};

use super::{Archive, File, Folder, name};

/// A member location that resolves to the role it turns out to need.
///
/// [`Folder::child_by_path`](crate::IOBase::child_by_path) answers this,
/// because a name inside an archive says nothing about whether it holds bytes
/// or holds other members until the index is asked. Resolution follows the
/// laziness contract: construction touches nothing, reading a member that is
/// not there yields nothing, and writing creates it - a byte write is what
/// makes a name a member rather than a prefix.
///
/// ```
/// use yggdryl::holder::{Buffer, Holder, zip::Archive};
/// use yggdryl::{IOBase, IOKind};
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
///
/// // Nothing is there, so nothing has decided.
/// let member = root.child_by_path("trades/eu.csv")?;
/// assert_eq!(member.kind(), IOKind::Unknown);
/// assert!(member.read_all_bytes()?.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Path {
    archive: Arc<Archive>,
    /// The member's canonical path inside the archive.
    name: SmolStr,
    /// The location, which is the archive's with the name under it.
    url: Url,
    /// An explicit representation supplied by the caller.
    declared: Option<MediaType>,
    /// Inference from the member's compound name, computed on demand.
    inferred: OnceLock<MediaType>,
}

impl Path {
    /// Address one member location without touching the archive.
    pub fn new(archive: Arc<Archive>, name: SmolStr) -> Self {
        Self {
            url: archive.member_url(&name),
            archive,
            name,
            declared: None,
            inferred: OnceLock::new(),
        }
    }

    /// Borrow the archive this location is inside.
    pub const fn archive(&self) -> &Arc<Archive> {
        &self.archive
    }

    /// The member's canonical path inside the archive.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Borrow the location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// Treat this location as a directory, whether or not it is one yet.
    pub fn as_directory(&self) -> Folder {
        Folder::new(Arc::clone(&self.archive), self.name.clone())
    }

    /// Treat this location as a byte member, whether or not it is one yet.
    pub fn as_file(&self) -> File {
        let mut file = File::new(Arc::clone(&self.archive), self.name.clone());
        if let Some(media_type) = &self.declared {
            file.set_media_type(media_type.clone());
        }
        file
    }
}

impl IOPath for Path {
    fn path_url(&self) -> &Url {
        &self.url
    }

    /// Whether a record names this prefix, or a member continues it.
    fn is_folder(&self) -> bool {
        self.archive.holds_prefix(&self.name).unwrap_or(false)
    }

    /// Whether the archive holds a byte member under exactly this name.
    fn is_file(&self) -> bool {
        self.archive
            .entry(&self.name)
            .ok()
            .flatten()
            .is_some_and(|entry| !entry.is_directory())
    }
}

impl crate::IOMedia for Path {
    crate::impl_default_iomedia!();
}

impl IOBase for Path {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        self.as_file().pread(offset, buffer)
    }

    /// Stream the member this location resolves to.
    ///
    /// The reader owns the archive it reads through, so the stream outlives
    /// the resolution that built it.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        let Some(entry) = self.archive.entry(&self.name)? else {
            return crate::ByteStream::from_reader(std::io::empty(), batch_size);
        };
        crate::ByteStream::from_reader(self.archive.entry_reader(&entry, position)?, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        self.as_file().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        self.as_file().read_range_bytes(offset, length)
    }

    /// Write through a member handle, publishing what this call staged.
    ///
    /// A resolving location owns no staged member of its own, so the write is
    /// published here rather than left for a flush the caller cannot reach.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let mut file = self.as_file();
        let written = file.pwrite(offset, bytes)?;
        file.flush()?;
        Ok(written)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_file().write_all_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        let mut file = self.as_file();
        let offset = file.append_bytes(bytes)?;
        file.flush()?;
        Ok(offset)
    }

    fn size(&self) -> u64 {
        self.as_file().size()
    }

    fn capacity(&self) -> u64 {
        self.size()
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        if self.is_folder() {
            return self.as_directory().truncate(size);
        }
        let mut file = self.as_file();
        file.truncate(size)?;
        file.flush()
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        static DIRECTORY: std::sync::LazyLock<MediaType> =
            std::sync::LazyLock::new(|| MediaType::from(MimeType::DIRECTORY));
        if self.is_folder() {
            return &DIRECTORY;
        }
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
        self.archive.flush()
    }

    fn kind(&self) -> IOKind {
        self.path_kind()
    }

    fn parent(&self) -> Option<Holder> {
        if self.name.is_empty() {
            return self.archive.archive_parent();
        }
        Some(Holder::ZipFolder(Folder::new(
            Arc::clone(&self.archive),
            SmolStr::new(name::parent(&self.name).unwrap_or_default()),
        )))
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.as_directory().child_by_path(path)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.as_directory().ls(recursive, include_private)
    }

    fn glob(&self, pattern: &str, include_private: bool) -> Result<Listing> {
        self.as_directory().glob(pattern, include_private)
    }

    fn partitions(&self) -> Vec<(String, String)> {
        super::member_partitions(&self.archive, &self.name)
    }

    /// Empty whichever role is there, creating neither.
    fn clear(&mut self) -> Result<()> {
        if self.is_folder() {
            return self.as_directory().clear();
        }
        self.as_file().clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        if self.is_folder() {
            return self.as_directory().remove(recursive);
        }
        self.as_file().remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        self.path_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.path_is_tabular()
    }
}
