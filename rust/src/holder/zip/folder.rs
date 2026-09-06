//! One directory inside an archive, including the archive root itself.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::holder::Holder;
use crate::{Error, IOBase, IOFolder, IOKind, Listing, MediaType, Result, Url};

use super::{Archive, File, Path, name};

/// A directory of archive members, addressed by the prefix its members share.
///
/// A ZIP has no directory tree: it has a flat list of members whose names
/// happen to contain separators, and a directory record is optional metadata
/// beside them. This is that tree, derived from the names: the root prefix is
/// the archive itself, and any other prefix is a directory exactly when a
/// record names it or some member continues it.
///
/// Listing therefore reads no member byte. The archive's directory is already
/// the index, so one level of a listing is a range over it and the whole tree
/// is a walk of it.
///
/// ```
/// use yggdryl::holder::{Buffer, Holder, zip::Archive};
/// use yggdryl::{IOBase, IOKind};
///
/// # fn main() -> yggdryl::Result<()> {
/// let root = Archive::new(Holder::buffer(Buffer::new())).mount();
/// root.child_by_path("trades/eu.csv")?
///     .write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;
///
/// // The name implied a directory, which lists as one.
/// let trades = root.child_by_path("trades")?;
/// assert_eq!(trades.kind(), IOKind::Directory);
/// assert_eq!(trades.ls(false, false).count(), 1);
/// # Ok(())
/// # }
/// ```
#[derive(Debug)]
pub struct Folder {
    archive: Arc<Archive>,
    /// The prefix this directory holds, empty at the archive root.
    base: SmolStr,
    /// The directory's location, which is the archive's with the prefix under
    /// it.
    url: Url,
}

impl Folder {
    /// Address one directory of `archive` without touching it.
    pub fn new(archive: Arc<Archive>, base: SmolStr) -> Self {
        Self {
            url: archive.member_url(&base),
            archive,
            base,
        }
    }

    /// Borrow the archive this directory lives in.
    pub const fn archive(&self) -> &Arc<Archive> {
        &self.archive
    }

    /// The prefix this directory holds, empty at the archive root.
    pub fn name(&self) -> &str {
        &self.base
    }

    /// Borrow the directory's location.
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// Whether this handle addresses the archive itself.
    pub fn is_root(&self) -> bool {
        self.base.is_empty()
    }

    /// Address one member of this directory as a byte leaf.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the path climbs above the archive root.
    pub fn as_file(&self, path: &str) -> Result<File> {
        Ok(File::new(
            Arc::clone(&self.archive),
            name::resolve(&self.base, path)?,
        ))
    }

    /// Address one directory below this one.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Parse`] when the path climbs above the archive root.
    pub fn as_directory(&self, path: &str) -> Result<Self> {
        Ok(Self::new(
            Arc::clone(&self.archive),
            name::resolve(&self.base, path)?,
        ))
    }

    /// The names themselves, read when the listing is first polled.
    fn level(archive: Arc<Archive>, base: &str, recursive: bool, include_private: bool) -> Listing {
        let names = match archive.names_under(base, recursive, include_private) {
            Ok(names) => names,
            Err(error) => return Listing::failing(error),
        };
        Listing::new(names.into_iter().map(move |(name, is_folder)| {
            Ok(if is_folder {
                Holder::ZipFolder(Self::new(Arc::clone(&archive), name))
            } else {
                Holder::ZipFile(File::new(Arc::clone(&archive), name))
            })
        }))
    }
}

impl IOFolder for Folder {
    fn folder_url(&self) -> &Url {
        &self.url
    }

    /// Whether the directory is there.
    ///
    /// The archive root exists exactly when the archive has bytes; every other
    /// prefix exists when a record names it or a member continues it.
    fn folder_exists(&self) -> bool {
        if self.is_root() {
            return self.archive.size() > 0;
        }
        self.archive.holds_prefix(&self.base).unwrap_or(false)
    }

    fn create_folder(&self) -> Result<()> {
        self.archive.create_directory(&self.base)
    }

    /// List one level, or the whole subtree, from the archive's own index.
    ///
    /// The index is what an archive already holds - it is the directory the
    /// format writes - so a listing snapshots names out of it rather than
    /// re-reading anything. What it holds is therefore bounded by the level,
    /// or by the subtree, that was asked for and never by member contents.
    fn list_folder(&self, recursive: bool, include_private: bool) -> Listing {
        // Deferred into the first `next`, so constructing a listing parses no
        // index: a glob whose fixed prefix loses never reads one.
        let archive = Arc::clone(&self.archive);
        let base = self.base.clone();
        Listing::new(std::iter::once(()).flat_map(move |()| {
            Self::level(Arc::clone(&archive), &base, recursive, include_private)
        }))
    }

    /// Delete the directory itself, refusing one that still holds members.
    fn delete_folder(&mut self) -> Result<()> {
        if self.is_root() {
            return self.archive.remove();
        }
        if !self
            .archive
            .names_under(&self.base, false, true)?
            .is_empty()
        {
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::DirectoryNotEmpty,
                self.url.to_string(),
            )));
        }
        self.archive.remove_member(&self.base)?;
        self.archive.flush()
    }

    /// Remove every member under this directory in one pass over the index.
    fn folder_clear(&mut self) -> Result<()> {
        if self.is_root() {
            self.archive.clear_members()?;
        } else {
            self.archive.remove_under(&self.base)?;
        }
        self.archive.flush()
    }
}

impl crate::IOMedia for Folder {
    crate::impl_default_iomedia!();
}

impl IOBase for Folder {
    fn pread(&self, _offset: u64, _buffer: &mut [u8]) -> Result<usize> {
        self.folder_pread()
    }

    fn pwrite(&mut self, _offset: u64, bytes: &[u8]) -> Result<usize> {
        self.folder_pwrite(bytes.len())
    }

    fn size(&self) -> u64 {
        0
    }

    fn capacity(&self) -> u64 {
        0
    }

    fn reserve(&mut self, _capacity: u64) -> Result<()> {
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        self.folder_truncate(size)
    }

    fn url(&self) -> Option<&Url> {
        Some(&self.url)
    }

    fn media_type(&self) -> &MediaType {
        self.folder_media_type()
    }

    fn set_media_type(&mut self, _media_type: MediaType) {}

    /// Publish the archive's directory, and with it every pending member.
    fn flush(&mut self) -> Result<()> {
        self.archive.flush()
    }

    /// Parse the archive's index once and hold it for this scope.
    fn open(&mut self) -> Result<()> {
        self.archive.open()
    }

    fn opened(&self) -> bool {
        self.archive.is_indexed()
    }

    fn close(&mut self) -> Result<()> {
        self.archive.close()
    }

    /// The directory above this one, leaving the archive at its root.
    ///
    /// The root's parent is where the archive's own bytes live, so a walk that
    /// goes up out of an archive lands beside it rather than stopping.
    fn parent(&self) -> Option<Holder> {
        if self.is_root() {
            return self.archive.archive_parent();
        }
        Some(Holder::ZipFolder(Self::new(
            Arc::clone(&self.archive),
            SmolStr::new(name::parent(&self.base).unwrap_or_default()),
        )))
    }

    fn child_by_path(&self, path: &str) -> Result<Holder> {
        Ok(Holder::ZipPath(Path::new(
            Arc::clone(&self.archive),
            name::resolve(&self.base, path)?,
        )))
    }

    fn ls(&self, recursive: bool, include_private: bool) -> Listing {
        self.folder_ls(recursive, include_private)
    }

    /// Match the pattern against member names rather than against locations.
    ///
    /// A member's location says which archive it is in; its name is what the
    /// pattern is about. Matching the name also needs no fixed prefix to be
    /// descended first: the index is one map, so the walk is already as deep
    /// as it can be.
    fn glob(&self, pattern: &str, include_private: bool) -> Result<Listing> {
        let base = self.base.clone();
        let pattern = pattern.to_owned();
        let recursive = pattern.contains('/') || pattern.split('/').any(|part| part == "**");
        Ok(self.ls(recursive, include_private).keeping(move |entry| {
            super::member_name(entry)
                .and_then(|member| name::under(&base, member))
                .is_some_and(|relative| crate::uri::pattern::matches_glob_text(relative, &pattern))
        }))
    }

    fn partitions(&self) -> Vec<(String, String)> {
        super::member_partitions(&self.archive, &self.base)
    }

    fn kind(&self) -> IOKind {
        IOKind::Directory
    }

    fn clear(&mut self) -> Result<()> {
        self.folder_clear()
    }

    fn remove(&mut self, recursive: bool) -> Result<()> {
        self.folder_remove(recursive)
    }

    fn is_atomic(&self) -> bool {
        false
    }

    fn is_tabular(&self) -> bool {
        self.folder_is_tabular()
    }
}
