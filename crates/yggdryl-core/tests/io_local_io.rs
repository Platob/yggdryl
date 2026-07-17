//! Functional tests for [`LocalIO`](yggdryl_core::io::local::LocalIO) — the local family's
//! single access point: laziness, auto-creating self-optimizing writes, streamed graph
//! discovery, shape-checked CRUD, plus the `IOBase` predicates (`is_file` / `is_dir` /
//! `exists`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use yggdryl_core::io::local::LocalIO;
use yggdryl_core::io::memory::{Heap, IOBase};
use yggdryl_core::io::IOKind;

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
fn uri_round_trips_a_path_with_spaces() {
    let tmp = TempDir::new("uri_space");
    let mut written = tmp.root().join_str("with space/no te.bin");
    written.pwrite_utf8(0, "hi");
    written.close();

    // uri() percent-encodes the space; from_uri must decode it back to the same file.
    let uri = written.uri();
    assert!(uri.to_string().contains("%20"));
    let back = LocalIO::from_uri(&uri).unwrap();
    assert_eq!(back, written);
    assert!(back.is_file());
    assert_eq!(back.pread_utf8(0, 2).unwrap(), "hi");
}

#[test]
fn read_only_mode_gates_every_write_shaped_call() {
    let tmp = TempDir::new("ro");
    let mut h = tmp.root().join_str("never.bin");
    h.set_mode(yggdryl_core::io::IOMode::Read);

    // The checked reservations refuse with the same guided text as pwrite_all — and,
    // crucially, nothing is created on disk.
    let err = h.try_reserve(100).unwrap_err().to_string();
    assert!(err.contains("read-only") && err.contains("set_mode"));
    let err = h.try_reserve_exact(100).unwrap_err().to_string();
    assert!(err.contains("read-only") && err.contains("set_mode"));
    h.reserve(100);
    h.reserve_exact(100);
    assert_eq!(h.pwrite_byte_array(0, b"x"), 0);
    assert!(!h.exists());
    assert!(!tmp.root().exists()); // not even the parent folder appeared
}

#[test]
fn reserve_exact_materializes_real_capacity() {
    let tmp = TempDir::new("resx");
    let mut h = tmp.root().join_str("cap.bin");
    h.reserve_exact(4096);
    assert!(h.capacity() >= 4096); // not the trait's silent no-op
    assert!(h.is_mapped());
    h.close();
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

    // An EMPTY directory (no blocks) refuses a byte stream with a guided, binding-neutral fix.
    let mut as_writer = dir.clone();
    let err = as_writer.pwrite_all(0, b"x").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("join a file name") && !msg.contains("join_str"));
    // The primitive writes nothing into an empty tree.
    assert_eq!(as_writer.pwrite_byte_array(0, b"x"), 0);
    // Reads on an empty directory are empty.
    assert_eq!(dir.pread_vec(0, 8), b"");
}

#[test]
fn empty_write_never_touches_a_missing_file() {
    let tmp = TempDir::new("touch");
    let mut node = tmp.root().join_str("ghost.bin");
    // A zero-length pwrite_all is a no-op — it must not auto-create the file.
    node.pwrite_all(0, b"").unwrap();
    assert!(!node.exists());
    assert!(!node.is_mapped());
}

// -------------------------------------------------------------------------------------
// A directory is a memory tree
// -------------------------------------------------------------------------------------

#[test]
fn directory_reads_as_one_memory_tree() {
    let tmp = TempDir::new("tree");
    let root = tmp.root();
    root.join_str("a.bin").pwrite_utf8(0, "AAAA");
    root.join_str("b.bin").pwrite_utf8(0, "BB");
    root.join_str("sub/c.bin").pwrite_utf8(0, "CCC");

    // byte_size is the lazy streamed sum of the subtree — recomputed live, nothing cached.
    assert_eq!(root.byte_size(), 9);
    // Blocks are name-sorted (a.bin | b.bin | sub) — one contiguous region.
    assert_eq!(root.pread_utf8(0, 9).unwrap(), "AAAABBCCC");
    // A read across block boundaries stitches transparently.
    assert_eq!(root.pread_utf8(3, 4).unwrap(), "ABBC");
    // The cursor stream works on the tree too.
    let mut cur = tmp.root();
    assert_eq!(cur.read_utf8(6).unwrap(), "AAAABB");
    // Growth is visible immediately (full laziness): add a file, the tree grows.
    root.join_str("d.bin").pwrite_utf8(0, "!");
    assert_eq!(root.byte_size(), 10);
    assert_eq!(root.pread_utf8(0, 10).unwrap(), "AAAABB!CCC"); // d.bin sorts before sub
}

#[test]
fn directory_writes_route_across_blocks() {
    let tmp = TempDir::new("treew");
    let mut root = tmp.root();
    root.join_str("a.bin").pwrite_utf8(0, "AAAA");
    root.join_str("b.bin").pwrite_utf8(0, "BB");

    // A write inside one block stays inside it.
    root.pwrite_all(1, b"XX").unwrap();
    assert_eq!(root.join_str("a.bin").pread_utf8(0, 4).unwrap(), "AXXA");
    // A write across the boundary splits between blocks — the middle block never grows.
    root.pwrite_all(3, b"12").unwrap();
    assert_eq!(root.join_str("a.bin").pread_utf8(0, 4).unwrap(), "AXX1");
    assert_eq!(root.join_str("b.bin").pread_utf8(0, 2).unwrap(), "2B");
    // Bytes past the end grow the LAST block.
    root.pwrite_all(6, b"ZZ").unwrap();
    assert_eq!(root.join_str("b.bin").pread_utf8(0, 4).unwrap(), "2BZZ");
    assert_eq!(root.byte_size(), 8);
}

/// Creates a directory symlink, returning `false` when the OS refuses (unprivileged
/// Windows) so the test can skip rather than fail.
#[cfg(unix)]
fn symlink_dir(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(windows)]
fn symlink_dir(target: &std::path::Path, link: &std::path::Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}

#[test]
fn memory_tree_does_not_recurse_into_directory_symlinks() {
    let tmp = TempDir::new("cycle");
    let root = tmp.root();
    root.join_str("a.bin").pwrite_utf8(0, "AAAA");
    // A directory symlink pointing back at the root would make a naive tree recurse forever.
    if !symlink_dir(&tmp.0, tmp.0.join("loop").as_path()) {
        return; // unprivileged environment — the guard is still compiled and covered elsewhere
    }
    // The symlinked directory is excluded from the block layout, so this terminates and the
    // size is just the real file's.
    assert_eq!(root.byte_size(), 4);
    assert_eq!(root.pread_utf8(0, 4).unwrap(), "AAAA");
    // Discovery still lists the symlink (only the tree layout skips it).
    let names: Vec<String> = root.ls().unwrap().map(|e| e.unwrap().name()).collect();
    assert!(names.contains(&"loop".to_string()));
}

#[test]
fn wrapper_over_a_spaced_path_names_it_decoded() {
    // The default IOBase::name() percent-decodes the address segment, so a cursor over a
    // LocalIO whose path contains a space reports "my file.txt", never "my%20file.txt".
    let tmp = TempDir::new("nm");
    let mut node = tmp.root().join_str("my file.txt");
    node.pwrite_utf8(0, "x");
    node.close();
    assert!(node.uri().to_string().contains("%20"));
    let cur = tmp.root().join_str("my file.txt").cursor();
    assert_eq!(cur.name(), "my file.txt");
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
