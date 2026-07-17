//! Functional tests for the local [`Path`](yggdryl_core::io::Path) family —
//! [`LocalPath`](yggdryl_core::io::local::LocalPath) (lazy + auto-create),
//! [`LocalFile`](yggdryl_core::io::local::LocalFile) (mapped),
//! [`LocalFolder`](yggdryl_core::io::local::LocalFolder) — plus the new `IOBase`
//! predicates (`is_file` / `is_dir` / `exists`) and streamed graph discovery.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use yggdryl_core::io::local::{LocalFile, LocalFolder, LocalPath};
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::io::{IOKind, Path};

/// A unique temp directory per test, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        Self(std::env::temp_dir().join(format!("yggdryl_path_{tag}_{pid}_{n}")))
    }

    fn root(&self) -> LocalPath {
        LocalPath::from_path(&self.0)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

// -------------------------------------------------------------------------------------
// IOBase predicates
// -------------------------------------------------------------------------------------

#[test]
fn predicates_derive_from_kind() {
    let tmp = TempDir::new("pred");
    let missing = tmp.root().join_str("nothing.bin");
    assert_eq!(missing.kind(), IOKind::Missing);
    assert!(!missing.is_file() && !missing.is_dir() && !missing.exists());

    let mut file = tmp.root().join_str("a.bin");
    file.pwrite_byte(0, 1).unwrap();
    assert!(file.is_file() && !file.is_dir() && file.exists());

    let dir = tmp.root().folder().unwrap();
    assert!(dir.is_dir() && !dir.is_file() && dir.exists());

    // A live heap exists although it is neither file nor directory.
    let heap = Heap::new();
    assert!(heap.exists() && !heap.is_file() && !heap.is_dir());
}

// -------------------------------------------------------------------------------------
// Lazy LocalPath + auto-create writes
// -------------------------------------------------------------------------------------

#[test]
fn lazy_path_auto_creates_parents_and_file_on_write() {
    let tmp = TempDir::new("lazy");
    let mut note = tmp.root().join_str("deep/nested/dirs/note.txt");
    // Lazy: constructing + probing + reading touches nothing.
    assert!(!note.exists());
    assert_eq!(note.byte_size(), 0);
    assert_eq!(note.pread_vec(0, 16), b""); // reads on a missing node are empty

    // A write brings the whole ancestry + file into being.
    assert_eq!(note.pwrite_utf8(0, "hello"), 5);
    assert!(note.is_file());
    assert!(tmp.root().join_str("deep/nested/dirs").is_dir());
    assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello");
    assert_eq!(note.byte_size(), 5);

    // Typed + bulk access works through the lazy handle too.
    note.pwrite_i32(8, -7).unwrap();
    assert_eq!(note.pread_i32(8).unwrap(), -7);
    note.pwrite_i64_array(16, &[1, 2, 3]).unwrap();
    let mut back = [0i64; 3];
    note.pread_i64_array(16, &mut back).unwrap();
    assert_eq!(back, [1, 2, 3]);
}

#[test]
fn path_navigation_name_parent_join() {
    let tmp = TempDir::new("nav");
    let node = tmp.root().join_str("a/b/c.txt");
    assert_eq!(node.name(), "c.txt");
    let parent = node.parent().unwrap();
    assert_eq!(parent.name(), "b");
    assert_eq!(parent.parent().unwrap().name(), "a");
    // join is lazy and composes.
    assert_eq!(parent.join_str("d/e.bin").name(), "e.bin");
    // uri reports the address.
    assert!(node.uri().to_string().ends_with("c.txt"));
}

// -------------------------------------------------------------------------------------
// Streamed discovery
// -------------------------------------------------------------------------------------

#[test]
fn ls_streams_children_and_walks_recursively() {
    let tmp = TempDir::new("ls");
    let root = tmp.root();
    root.join_str("one.txt").pwrite_utf8(0, "1");
    root.join_str("sub/two.txt").pwrite_utf8(0, "2");
    root.join_str("sub/deeper/three.txt").pwrite_utf8(0, "3");

    // One level, streamed.
    let mut direct: Vec<String> = root.ls().unwrap().map(|e| e.unwrap().name()).collect();
    direct.sort();
    assert_eq!(direct, vec!["one.txt", "sub"]);

    // Collected convenience matches.
    assert_eq!(root.children().unwrap().len(), 2);

    // Recursive walk reaches every node.
    let mut all: Vec<String> = root
        .ls_recursive()
        .unwrap()
        .map(|e| e.unwrap().name())
        .collect();
    all.sort();
    assert_eq!(
        all,
        vec!["deeper", "one.txt", "sub", "three.txt", "two.txt"]
    );

    // A file (and a missing node) streams nothing.
    assert_eq!(root.join_str("one.txt").children().unwrap().len(), 0);
    assert_eq!(root.join_str("ghost").children().unwrap().len(), 0);
}

// -------------------------------------------------------------------------------------
// CRUD: rm / rmfile / rmdir
// -------------------------------------------------------------------------------------

#[test]
fn rm_family_with_guided_mismatch_errors() {
    let tmp = TempDir::new("rm");
    let root = tmp.root();
    let mut f = root.join_str("f.txt");
    f.pwrite_utf8(0, "x");
    let d = root.join_str("d");
    d.folder().unwrap();

    // rmfile refuses a directory; rmdir refuses a file — both with guided text.
    assert!(d.rmfile().unwrap_err().to_string().contains("use rmdir"));
    assert!(f.rmdir().unwrap_err().to_string().contains("use rmfile"));

    // The right-shaped removals work; removing a missing node is idempotent.
    f.rmfile().unwrap();
    assert!(!f.exists());
    f.rmfile().unwrap(); // no-op
    d.rmdir().unwrap();
    assert!(!d.exists());

    // rm removes whatever exists.
    root.join_str("g.txt").pwrite_utf8(0, "y");
    root.join_str("h/i.txt").pwrite_utf8(0, "z");
    root.join_str("g.txt").rm().unwrap();
    root.join_str("h").rm().unwrap(); // recursive
    assert_eq!(root.children().unwrap().len(), 0);
}

// -------------------------------------------------------------------------------------
// Sub-instantiation: LocalFile (mapped) + LocalFolder
// -------------------------------------------------------------------------------------

#[test]
fn localfile_auto_creates_and_maps() {
    let tmp = TempDir::new("file");
    // Sub-instantiate from a lazy path whose parents don't exist yet.
    let mut file = tmp.root().join_str("x/y/data.bin").file().unwrap();
    assert!(file.is_file());
    assert_eq!(file.kind(), IOKind::File);

    // The full mapped surface: cursor stream + typed + capacity family.
    file.write_utf8("mapped");
    file.write_i32(7).unwrap();
    file.rewind();
    assert_eq!(file.read_utf8(6).unwrap(), "mapped");
    assert_eq!(file.read_i32().unwrap(), 7);
    file.try_reserve(4096).unwrap();
    assert!(file.capacity() >= 4096);
    file.flush().unwrap();

    // Path surface on the file: name/parent; ls streams nothing.
    assert_eq!(file.name(), "data.bin");
    assert_eq!(file.parent().unwrap().name(), "y");
    assert_eq!(file.children().unwrap().len(), 0);
}

#[test]
fn localfolder_auto_creates_tree_and_rejects_bytes() {
    let tmp = TempDir::new("folder");
    let folder = tmp.root().join_str("a/b/c").folder().unwrap();
    assert!(folder.is_dir());
    assert_eq!(folder.byte_size(), 0);

    // No byte stream: reads empty, primitive writes nothing, full write is guided.
    assert_eq!(folder.pread_vec(0, 8), b"");
    let err = folder.clone().pwrite_all(0, b"x").unwrap_err();
    assert!(err.to_string().contains("join_str a file name"));

    // Graph work: create a child file, discover it, clean up.
    folder.join_str("inner.txt").pwrite_utf8(0, "v");
    assert_eq!(folder.children().unwrap().len(), 1);
    folder.rmdir().unwrap();
    assert!(!folder.exists());
}

#[test]
fn persistence_across_handles() {
    let tmp = TempDir::new("persist");
    {
        let mut file = tmp.root().join_str("keep.bin").file().unwrap();
        file.pwrite_i64(0, 1 << 40).unwrap();
    } // mapping dropped: truncated to logical length
      // A fresh lazy handle sees the same bytes.
    let lazy = tmp.root().join_str("keep.bin");
    assert_eq!(lazy.byte_size(), 8);
    assert_eq!(lazy.pread_i64(0).unwrap(), 1 << 40);
    // And an existing-file open works without auto-create.
    let reopened = LocalFile::open_path(lazy.as_std_path()).unwrap();
    assert_eq!(reopened.pread_i64(0).unwrap(), 1 << 40);
    assert!(LocalFile::open_path(tmp.0.join("missing.bin")).is_err());
    // Direct folder creation is also exposed.
    assert!(LocalFolder::create_path(tmp.0.join("direct"))
        .unwrap()
        .is_dir());
}
