//! One member location, whatever it turns out to be.

use std::sync::{Arc, OnceLock};

use smol_str::SmolStr;

use crate::holder::Holder;
use crate::{IOBase, IOKind, IOPath, Listing, MediaType, MimeType, Result, Url};

use super::{Archive, Leaf, Node, name};

/// A member location that resolves to the role it turns out to need.
///
/// [`Node::child_by_path`](crate::IOBase::child_by_path) answers this,
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
    /// The byte role, built once and then reused.
    ///
    /// Every byte verb here is that role's, and building one costs the
    /// member's location - a URL clone and a percent-encode. Holding it is
    /// also what lets a positional write stage rather than publish, which is
    /// what [`IOBase::pwrite`] means everywhere else.
    leaf: OnceLock<Leaf>,
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
            leaf: OnceLock::new(),
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
    pub fn as_node(&self) -> Node {
        Node::new(Arc::clone(&self.archive), self.name.clone())
    }

    /// Treat this location as a byte member, whether or not it is one yet.
    pub fn as_leaf(&self) -> &Leaf {
        self.leaf.get_or_init(|| {
            let mut leaf = Leaf::new(Arc::clone(&self.archive), self.name.clone());
            if let Some(media_type) = &self.declared {
                leaf.set_media_type(media_type.clone());
            }
            leaf
        })
    }

    /// The byte role, borrowed for a call that changes it.
    fn as_leaf_mut(&mut self) -> &mut Leaf {
        let _ = self.as_leaf();
        self.leaf
            .get_mut()
            .expect("the role was built by the borrow above")
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
    /// Read the bytes of whichever role is there.
    ///
    /// A name a record marks as a directory and a member of the same name can
    /// both be in one archive, and what the location *is* decides every verb
    /// here: answering a member's bytes from a location that lists as a
    /// container would make [`IOBase::kind`] and this call disagree.
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        if self.is_folder() {
            return self.as_node().pread(offset, buffer);
        }
        self.as_leaf().pread(offset, buffer)
    }

    /// Stream the member this location resolves to.
    ///
    /// The reader owns the archive it reads through, so the stream outlives
    /// the resolution that built it.
    fn pstream_bytes(&self, position: u64, batch_size: usize) -> Result<crate::ByteStream<'_>> {
        if self.is_folder() {
            // A container holds no bytes, and reads as the empty one it is.
            return crate::ByteStream::from_reader(std::io::empty(), batch_size);
        }
        self.as_leaf().pstream_bytes(position, batch_size)
    }

    fn read_all_bytes(&self) -> Result<Vec<u8>> {
        if self.is_folder() {
            return self.as_node().read_all_bytes();
        }
        self.as_leaf().read_all_bytes()
    }

    fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>> {
        if self.is_folder() {
            return self.as_node().read_range_bytes(offset, length);
        }
        self.as_leaf().read_range_bytes(offset, length)
    }

    /// Stage a write into the member this location resolves to.
    ///
    /// The role is held, so the write stages exactly as it does on a member
    /// handle and [`IOBase::flush`] publishes it. Publishing per call would
    /// append a whole record per call.
    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        self.as_leaf_mut().pwrite(offset, bytes)
    }

    fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.as_leaf_mut().write_all_bytes(bytes)
    }

    fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64> {
        self.as_leaf_mut().append_bytes(bytes)
    }

    fn size(&self) -> u64 {
        if self.is_folder() {
            return self.as_node().size();
        }
        self.as_leaf().size()
    }

    fn capacity(&self) -> u64 {
        self.size()
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        if self.is_folder() {
            return self.as_node().truncate(size);
        }
        self.as_leaf_mut().truncate(size)
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

    /// Publish what this location staged, and the directory that indexes it.
    fn flush(&mut self) -> Result<()> {
        if self.leaf.get().is_some() {
            return self.as_leaf_mut().flush();
        }
        self.archive.flush()
    }

    fn kind(&self) -> IOKind {
        self.path_kind()
    }

    fn parent(&self) -> Option<Holder> {
        if self.name.is_empty() {
            return self.archive.archive_parent();
        }
        Some(Holder::ZipNode(Node::new(
            Arc::clone(&self.archive),
            SmolStr::new(name::parent(&self.name).unwrap_or_default()),
        )))
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        self.as_node().child_by_path(path)
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.as_node().ls(recursive, include_private)
    }

    fn glob(&self, pattern: &str, include_private: bool) -> Result<Listing> {
        self.as_node().glob(pattern, include_private)
    }

    fn partitions(&self) -> Vec<(String, String)> {
        super::member_partitions(&self.archive, &self.name)
    }

    /// Empty whichever role is there, creating neither.
    fn clear(&mut self) -> Result<()> {
        if self.is_folder() {
            return self.as_node().clear();
        }
        self.as_leaf_mut().clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        if self.is_folder() {
            return self.as_node().remove(recursive);
        }
        self.as_leaf_mut().remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        self.path_is_atomic()
    }

    fn is_tabular(&self) -> bool {
        self.path_is_tabular()
    }
}
