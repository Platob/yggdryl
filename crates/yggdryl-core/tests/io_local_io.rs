//! Functional tests for [`LocalIO`](yggdryl_core::io::local::LocalIO) — the local family's
//! single access point: laziness, auto-creating self-optimizing writes, streamed graph
//! discovery, shape-checked CRUD, plus the `IOBase` predicates (`is_file` / `is_dir` /
//! `exists`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use yggdryl_core::io::local::LocalIO;
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::io::{IOKind, Path};

/// A unique temp directory per test, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        Self(std::env::temp_dir().join(format!("yggdryl_localio_{tag}_{pid}_{n}")))
    }

    fn root(&self) -> LocalIO {
        LocalIO::from_path(&self.0)
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

    let dir = tmp.root().join_str("d");
    dir.mkdir().unwrap();
    assert!(dir.is_dir() && !dir.is_file() && dir.exists());

    // A live heap exists although it is neither file nor directory.
    let heap = Heap::new();
    assert!(heap.exists() && !heap.is_file() && !heap.is_dir());
}

// -------------------------------------------------------------------------------------
// Lazy handle + auto-creating, self-optimizing writes
// -------------------------------------------------------------------------------------

#[test]
fn lazy_then_write_auto_creates_and_self_optimizes() {
    let tmp = TempDir::new("lazy");
    let mut note = tmp.root().join_str("deep/nested/dirs/note.txt");
    // Lazy: constructing + probing + reading touches nothing, and no mapping exists.
    assert!(!note.exists());
    assert!(!note.is_mapped());
    assert_eq!(note.byte_size(), 0);
    assert_eq!(note.pread_vec(0, 16), b""); // reads on a missing node are empty

    // The first write brings the ancestry + file into being AND keeps the mapped backing.
    assert_eq!(note.pwrite_utf8(0, "hello"), 5);
    assert!(note.is_file());
    assert!(note.is_mapped()); // self-optimized: later access runs at memory speed
    assert!(tmp.root().join_str("deep/nested/dirs").is_dir());
    assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello");

    // Typed + bulk + capacity all work through the same handle.
    note.pwrite_i32(8, -7).unwrap();
    assert_eq!(note.pread_i32(8).unwrap(), -7);
    note.pwrite_i64_array(16, &[1, 2, 3]).unwrap();
    let mut back = [0i64; 3];
    note.pread_i64_array(16, &mut back).unwrap();
    assert_eq!(back, [1, 2, 3]);
    note.try_reserve(4096).unwrap();
    assert!(note.capacity() >= 4096);
    note.flush().unwrap();

    // close() releases the mapping (truncating) but the handle stays usable, back to lazy.
    note.close();
    assert!(!note.is_mapped());
    assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello"); // ad-hoc read path
    assert_eq!(std::fs::metadata(note.as_std_path()).unwrap().len(), 40);
}

#[test]
fn reads_before_any_write_are_ad_hoc() {
    let tmp = TempDir::new("adhoc");
    // Produce a file through one handle…
    let mut writer = tmp.root().join_str("data.bin");
    writer.pwrite_i64(0, 1 << 40).unwrap();
    writer.close();
    // …and read it through a fresh, never-written handle: served ad hoc, no mapping.
    let reader = tmp.root().join_str("data.bin");
    assert_eq!(reader.pread_i64(0).unwrap(), 1 << 40);
    assert!(!reader.is_mapped());
    // The cursor stream works on the lazy handle too (reads only).
    let mut cur = tmp.root().join_str("data.bin");
    assert_eq!(cur.read_i64().unwrap(), 1 << 40);
}

#[test]
fn clone_is_a_fresh_lazy_handle() {
    let tmp = TempDir::new("clone");
    let mut a = tmp.root().join_str("x.bin");
    a.pwrite_byte(0, 7).unwrap();
    assert!(a.is_mapped());
    let b = a.clone();
    assert_eq!(a, b); // same path
    assert!(!b.is_mapped()); // but its own lazy state
    a.close();
    assert_eq!(b.pread_byte(0).unwrap(), 7);
}

// -------------------------------------------------------------------------------------
// Navigation + streamed discovery
// -------------------------------------------------------------------------------------

#[test]
fn navigation_name_parent_join() {
    let tmp = TempDir::new("nav");
    let node = tmp.root().join_str("a/b/c.txt");
    assert_eq!(node.name(), "c.txt");
    let parent = node.parent().unwrap();
    assert_eq!(parent.name(), "b");
    assert_eq!(parent.parent().unwrap().name(), "a");
    assert_eq!(parent.join_str("d/e.bin").name(), "e.bin");
    assert!(node.uri().to_string().ends_with("c.txt"));
}

#[test]
fn ls_streams_children_and_walks_recursively() {
    let tmp = TempDir::new("ls");
    let root = tmp.root();
    root.join_str("one.txt").pwrite_utf8(0, "1");
    root.join_str("sub/two.txt").pwrite_utf8(0, "2");
    root.join_str("sub/deeper/three.txt").pwrite_utf8(0, "3");

    let mut direct: Vec<String> = root.ls().unwrap().map(|e| e.unwrap().name()).collect();
    direct.sort();
    assert_eq!(direct, vec!["one.txt", "sub"]);
    assert_eq!(root.children().unwrap().len(), 2);

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
// Folders + byte-stream refusal
// -------------------------------------------------------------------------------------

#[test]
fn mkdir_and_directory_write_refusal() {
    let tmp = TempDir::new("dir");
    let dir = tmp.root().join_str("a/b/c");
    dir.mkdir().unwrap(); // mkdir -p
    assert!(dir.is_dir());

    // A directory refuses a byte stream with a guided fix.
    let mut as_writer = dir.clone();
    let err = as_writer.pwrite_all(0, b"x").unwrap_err();
    assert!(err.to_string().contains("join_str a file name"));
    // The primitive writes nothing.
    assert_eq!(as_writer.pwrite_byte_array(0, b"x"), 0);
    // Reads on a directory are empty.
    assert_eq!(dir.pread_vec(0, 8), b"");
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
    f.close(); // release the mapping so Windows can delete
    let d = root.join_str("d");
    d.mkdir().unwrap();

    assert!(d.rmfile().unwrap_err().to_string().contains("use rmdir"));
    assert!(f.rmdir().unwrap_err().to_string().contains("use rmfile"));

    f.rmfile().unwrap();
    assert!(!f.exists());
    f.rmfile().unwrap(); // idempotent on missing
    d.rmdir().unwrap();
    assert!(!d.exists());

    // rm removes whatever exists (file or whole tree).
    root.join_str("g.txt").pwrite_utf8(0, "y");
    root.join_str("h/i.txt").pwrite_utf8(0, "z");
    root.join_str("g.txt").rm().unwrap();
    root.join_str("h").rm().unwrap();
    assert_eq!(root.children().unwrap().len(), 0);
}

// -------------------------------------------------------------------------------------
// Persistence across handles
// -------------------------------------------------------------------------------------

#[test]
fn persistence_across_handles() {
    let tmp = TempDir::new("persist");
    {
        let mut w = tmp.root().join_str("keep.bin");
        w.pwrite_i64(0, 1 << 40).unwrap();
    } // handle dropped: mapping released, file truncated to logical length
    let fresh = tmp.root().join_str("keep.bin");
    assert_eq!(fresh.byte_size(), 8);
    assert_eq!(fresh.pread_i64(0).unwrap(), 1 << 40);
    assert_eq!(std::fs::metadata(fresh.as_std_path()).unwrap().len(), 8);
}
