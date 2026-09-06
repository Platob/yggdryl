//! ZIP archives as [`IOBase`](crate::IOBase) resources.
//!
//! An archive is a file system that happens to live inside one file, so it is
//! a storage backend like any other and supplies the same three roles:
//!
//! - [`Folder`] is the container: the archive root, or any prefix its members
//!   share. It lists and resolves members out of the archive's own directory,
//!   reading no member byte to do it.
//! - [`File`] is the leaf: one member, addressed positionally. A stored member
//!   reads straight out of the archive at an offset; a compressed one decodes
//!   through one bounded window.
//! - [`Path`] is the generic location, which resolves to whichever of the two
//!   is actually there.
//!
//! [`Archive`] is what the three share: one byte handle plus the central
//! directory that indexes it. [`Entry`] is what that directory says about one
//! member.
//!
//! All three follow the shared [`IOBase`](crate::IOBase) laziness contract:
//! constructing one touches nothing, reading something that is not there
//! yields nothing, and writing creates it - including the archive itself.
//!
//! # Codings
//!
//! A member is stored, raw DEFLATE, or Zstandard, which are exactly
//! [`Codec::Identity`](crate::Codec::Identity),
//! [`Codec::Deflate`](crate::Codec::Deflate), and
//! [`Codec::Zstd`](crate::Codec::Zstd): the archive adds no second coding
//! dispatcher. Any other compression method is reported by number rather than
//! guessed at, and an encrypted member is refused.
//!
//! ```
//! use yggdryl::holder::{Buffer, Holder, zip};
//! use yggdryl::IOBase;
//!
//! # fn main() -> yggdryl::Result<()> {
//! let root = zip::mount(Holder::buffer(Buffer::new()));
//!
//! let mut trades = root.child_by_path("2024/06/trades.csv")?;
//! trades.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;
//!
//! // Members are children, so the ordinary walk reaches them.
//! assert_eq!(root.glob("**/*.csv", false)?.count(), 1);
//! assert_eq!(
//!     root.child_by_path("2024/06/trades.csv")?.read_range_bytes(7, 5)?,
//!     b"price",
//! );
//! # Ok(())
//! # }
//! ```

mod archive;
mod entry;
mod file;
mod folder;
mod format;
mod name;
mod path;

pub use archive::Archive;
pub use entry::Entry;
pub use file::File;
pub use folder::Folder;
pub use path::Path;

use crate::holder::Holder;

/// Hold the archive `handle` addresses, without touching it.
///
/// The answer is the archive root: a container whose children are the members.
/// Nothing is read until an operation needs the index, and an archive that
/// does not exist yet is an empty one that the first write creates.
#[must_use]
pub fn mount(handle: Holder) -> Holder {
    Holder::ZipFolder(Archive::new(handle).mount())
}

/// Resolve the archive and member a location names, without touching either.
///
/// A member's location is the archive's URL with the member path in its
/// fragment, so this is the exact inverse of what a member handle reports:
/// `file:///lake/day.zip#trades/eu.csv` opens that member, and the same URL
/// without a fragment opens the archive root.
///
/// ```
/// use yggdryl::{IOBase, Url, holder::zip};
///
/// # fn main() -> yggdryl::Result<()> {
/// let archive = std::env::temp_dir().join("yggdryl-zip-from-url.zip");
/// let mut root = zip::mount(yggdryl::holder::Holder::file(&archive)?);
/// root.child_by_path("trades/eu.csv")?.write_all_bytes(b"symbol")?;
///
/// // A member reports where it is, and that is enough to open it again.
/// let member = root.child_by_path("trades/eu.csv")?;
/// let located = zip::from_url(member.url().expect("a member url"))?;
/// assert_eq!(located.read_all_bytes()?, b"symbol");
///
/// root.remove(true)?;
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`Error::Unsupported`](crate::Error::Unsupported) naming the scheme
/// when the archive is not a local file - mount any other handle with
/// [`mount`] - or the fragment's decode failure.
pub fn from_url(url: &crate::Url) -> crate::Result<Holder> {
    use crate::IOBase as _;

    let member = url.fragment(true)?.map(std::borrow::Cow::into_owned);
    let mut base = url.clone();
    base.set_fragment(None)?;
    if !base.is_local() {
        return Err(crate::Error::unsupported(
            "mounting a zip archive from a location this backend cannot hold",
            base.scheme().as_str(),
        ));
    }
    let root = Archive::new(Holder::Path(crate::holder::local::Path::from_url(base)?)).mount();
    match member.as_deref() {
        Some(member) if !member.is_empty() => root.child_by_path(member),
        _ => Ok(Holder::ZipFolder(root)),
    }
}

/// The member name a holder inside an archive addresses.
fn member_name(holder: &Holder) -> Option<&str> {
    match holder {
        Holder::ZipFolder(folder) => Some(folder.name()),
        Holder::ZipPath(path) => Some(path.name()),
        Holder::ZipFile(file) => Some(file.name()),
        _ => None,
    }
}

/// The Hive partitions an archive's location and a member name spell out.
///
/// Both halves count: a lake can partition the archives themselves and
/// partition again inside one, and a member carries whichever it is under.
fn member_partitions(archive: &Archive, member: &str) -> Vec<(String, String)> {
    let mut pairs = archive.url().hive_partitions();
    pairs.extend(crate::uri::hive_partitions_of(member.split('/')));
    pairs
}

#[cfg(test)]
mod tests;
