//! [`Path`] — the uniform **filesystem-graph** contract every path-like source implements.

use super::IoError;
use crate::io::memory::IOBase;

/// A node in a filesystem graph — the **uniform cross-filesystem abstraction** layered over
/// [`IOBase`]: the same navigation (parent / children), streamed discovery (`ls` /
/// `ls_recursive`), and CRUD (`rm` / `rmfile` / `rmdir`) surface, whatever the backing —
/// today the local filesystem ([`LocalPath`](crate::io::local::LocalPath) /
/// [`LocalFile`](crate::io::local::LocalFile) / [`LocalFolder`](crate::io::local::LocalFolder)
/// / [`Mmap`](crate::io::local::Mmap)); an object store or archive family implements the same
/// trait and every caller works unchanged.
///
/// # The model
///
/// - **One node type per family.** [`Node`](Path::Node) is the family's uniform path type
///   (`LocalPath` for the local family): `parent()` and every discovered child is a `Node`,
///   so graphs stay homogeneous whatever concrete type you started from.
/// - **Discovery is streamed.** [`ls`](Path::ls) (one level) and
///   [`ls_recursive`](Path::ls_recursive) (the whole subtree) return **iterators** — children
///   are produced lazily as the caller pulls, never pre-collected. The collected convenience
///   is [`children`](Path::children).
/// - **Byte access is [`IOBase`].** A path *is* a byte source: reads on a missing node are
///   empty, and — per the auto-create rule — a **write** creates the missing parent folders
///   and the file itself, so callers never pre-flight `mkdir`/`touch`.
/// - **Existence is a probe, not a state.** [`kind`](IOBase::kind) /
///   [`is_file`](IOBase::is_file) / [`is_dir`](IOBase::is_dir) / [`exists`](IOBase::exists)
///   ask the backing each call.
///
/// ```
/// use yggdryl_core::io::local::LocalPath;
/// use yggdryl_core::io::memory::IOBase;
/// use yggdryl_core::io::Path;
///
/// let root = LocalPath::from_path(std::env::temp_dir().join("yggdryl_path_doc"));
/// let mut note = root.join_str("a/b/note.txt"); // lazy: nothing touched yet
/// assert!(!note.exists());
///
/// note.pwrite_utf8(0, "hi"); // auto-creates a/ and a/b/ and the file
/// assert!(note.is_file());
/// assert_eq!(note.parent().unwrap().name(), "b");
///
/// // Streamed discovery from the root.
/// let names: Vec<String> = root
///     .ls_recursive()
///     .unwrap()
///     .map(|entry| entry.unwrap().name())
///     .collect();
/// assert!(names.contains(&"note.txt".to_string()));
///
/// root.rmdir().unwrap(); // recursive delete of the whole tree
/// assert!(!root.exists());
/// ```
pub trait Path: IOBase {
    /// The family's uniform node type — what navigation and discovery produce.
    type Node: Path;

    /// The streamed one-level child iterator ([`ls`](Path::ls)).
    type Children: Iterator<Item = Result<Self::Node, IoError>>;

    /// The streamed recursive walker ([`ls_recursive`](Path::ls_recursive)).
    type Walk: Iterator<Item = Result<Self::Node, IoError>>;

    /// The last path segment — the node's own name (empty for a root).
    fn name(&self) -> String;

    /// The parent node, or `None` at a root.
    fn parent(&self) -> Option<Self::Node>;

    /// The child node at `segment` (which may be a multi-segment relative path like
    /// `"a/b/c.txt"`) — **lazy**: nothing is touched or created.
    fn join_str(&self, segment: &str) -> Self::Node;

    /// Streams this node's **direct children**, lazily — each item is produced as the caller
    /// pulls. A file (or a missing node) streams nothing. Errors with a guided
    /// [`IoError::FileIo`] when the backing cannot be listed.
    fn ls(&self) -> Result<Self::Children, IoError>;

    /// Streams the node's **entire subtree** (depth-first), lazily. The recursive counterpart
    /// of [`ls`](Path::ls) — the bindings expose both through one generic
    /// `ls(recursive=…)` entry point.
    fn ls_recursive(&self) -> Result<Self::Walk, IoError>;

    /// The direct children, collected — the convenience over the streamed [`ls`](Path::ls).
    fn children(&self) -> Result<Vec<Self::Node>, IoError> {
        self.ls()?.collect()
    }

    /// Removes **whatever exists** at this node — a file is unlinked, a directory is removed
    /// with its whole subtree; a missing node is a no-op. The generic form of
    /// [`rmfile`](Path::rmfile) / [`rmdir`](Path::rmdir).
    fn rm(&self) -> Result<(), IoError>;

    /// Removes this node **as a file** — errors with a guided [`IoError::FileIo`] when the
    /// node is a directory (use [`rmdir`](Path::rmdir)) and is a no-op when missing.
    fn rmfile(&self) -> Result<(), IoError>;

    /// Removes this node **as a directory**, recursively — errors with a guided
    /// [`IoError::FileIo`] when the node is a file (use [`rmfile`](Path::rmfile)) and is a
    /// no-op when missing.
    fn rmdir(&self) -> Result<(), IoError>;
}
