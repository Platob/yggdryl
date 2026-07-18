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
fn join_composes_addresses_and_reads_writes_the_child() {
    let tmp = TempDir::new("join");
    let root = tmp.root();

    // `join` composes the child's address by joining onto the parent's URI (Uri::joinpath),
    // and is lazy — nothing exists until we write.
    let mut child = root.join("logs/day1.bin").unwrap();
    assert_eq!(child.uri(), root.uri().joinpath("logs/day1.bin"));
    assert!(!child.exists());

    // Writing the joined child auto-creates its parents + file, and reads back exactly.
    child.pwrite_utf8(0, "hello join");
    assert!(child.is_file());
    assert_eq!(child.pread_utf8(0, 10).unwrap(), "hello join");
    child.close();

    // The child's `parent()` addresses the joined directory again (the inverse of join).
    let dir = child.parent().unwrap();
    assert_eq!(dir.uri(), root.join("logs").unwrap().uri());
    assert!(dir.is_dir());

    // A second child under the same directory; the directory (memory tree) sees both.
    let mut sibling = root.join("logs").unwrap().join("day2.bin").unwrap();
    sibling.pwrite_utf8(0, "two");
    sibling.close();
    let mut names: Vec<String> = dir.ls().unwrap().map(|e| e.unwrap().name()).collect();
    names.sort();
    assert_eq!(names, vec!["day1.bin", "day2.bin"]);

    // Multi-segment and re-reading through a freshly joined handle (no shared state).
    let mut deep = root.join("a/b/c/note.txt").unwrap();
    deep.pwrite_utf8(0, "deep");
    deep.close();
    let reread = root.join("a").unwrap().join("b/c/note.txt").unwrap();
    assert_eq!(reread.pread_utf8(0, 4).unwrap(), "deep");

    // `join_str` is the infallible inherent form of the same operation, and it agrees with
    // `join` down to path identity (both resolve through the URI, so they compare equal).
    assert_eq!(root.join_str("logs/day1.bin").uri(), child.uri());
    assert_eq!(
        root.join_str("logs/day1.bin"),
        root.join("logs/day1.bin").unwrap()
    );
}

#[test]
fn join_edge_cases_read_write_spaced_paths() {
    let tmp = TempDir::new("joinedge");
    let root = tmp.root();

    // An empty segment addresses the same node.
    assert_eq!(root.join("").unwrap().uri(), root.uri());

    // A spaced multi-segment child writes and reads back exactly (percent round-trip).
    let mut spaced = root.join("my dir/my file.bin").unwrap();
    assert!(spaced.uri().to_string().contains("%20"));
    spaced.pwrite_utf8(0, "spaced write");
    spaced.close();
    let reread = root.join("my dir").unwrap().join("my file.bin").unwrap();
    assert_eq!(reread.pread_utf8(0, 12).unwrap(), "spaced write");
    assert_eq!(reread.name(), "my file.bin");

    // A chain of joins builds a deep path; the leaf reads back through a from-scratch handle.
    let mut deep = root
        .join("a")
        .unwrap()
        .join("b")
        .unwrap()
        .join("c.bin")
        .unwrap();
    deep.pwrite_i64(0, 1 << 40).unwrap();
    deep.close();
    assert_eq!(
        LocalIO::from_path(tmp.0.join("a/b/c.bin"))
            .pread_i64(0)
            .unwrap(),
        1 << 40
    );
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
fn bulk_typed_access_delegates_to_the_map_and_stays_correct() {
    let tmp = TempDir::new("bulk");
    let values: Vec<i32> = (-512..512).collect();

    // A bulk write to a fresh (unmapped, missing) file self-optimizes: it maps then writes
    // directly, and the readback through the mapped handle matches exactly.
    let mut node = tmp.root().join_str("data.bin");
    node.pwrite_i32_array(0, &values).unwrap();
    assert!(node.is_mapped());
    let mut back = vec![0i32; 1024];
    node.pread_i32_array(0, &mut back).unwrap();
    assert_eq!(back, values);
    node.pwrite_i64_repeat(4096, -1, 64).unwrap();
    let mut wide = [0i64; 64];
    node.pread_i64_array(4096, &mut wide).unwrap();
    assert!(wide.iter().all(|&v| v == -1));
    node.close();

    // A bulk read through a fresh, never-written handle stages over the ad-hoc read path.
    let reader = tmp.root().join_str("data.bin");
    assert!(!reader.is_mapped());
    let mut adhoc = vec![0i32; 1024];
    reader.pread_i32_array(0, &mut adhoc).unwrap();
    assert_eq!(adhoc, values);

    // A read-only handle refuses a bulk write with the guided fix, mapping nothing.
    let mut ro = tmp.root().join_str("nope.bin");
    ro.set_mode(yggdryl_core::io::IOMode::Read);
    assert!(ro
        .pwrite_i32_array(0, &[1, 2, 3])
        .unwrap_err()
        .to_string()
        .contains("read-only"));
    assert!(!ro.exists());
}

#[test]
fn bulk_typed_write_on_a_directory_routes_through_the_tree() {
    let tmp = TempDir::new("bulktree");
    let mut root = tmp.root();
    // Two equal blocks so a bulk i32 write lands across the boundary.
    root.join_str("a.bin").pwrite_byte_repeat(0, 0, 8).unwrap();
    root.join_str("b.bin").pwrite_byte_repeat(0, 0, 8).unwrap();

    // A bulk typed write on the directory routes through the memory tree (not ensure_map).
    root.pwrite_i32_array(0, &[1, 2, 3, 4]).unwrap();
    assert!(!root.is_mapped()); // a directory never maps
    let mut back = [0i32; 4];
    root.pread_i32_array(0, &mut back).unwrap();
    assert_eq!(back, [1, 2, 3, 4]);
    // The bytes really landed in the two child blocks (a.bin holds the first two i32s).
    let mut a = [0i32; 2];
    root.join_str("a.bin").pread_i32_array(0, &mut a).unwrap();
    assert_eq!(a, [1, 2]);
}

#[test]
fn many_threads_write_disjoint_files() {
    // Independent LocalIO handles to disjoint files write concurrently with no contention —
    // each mapping is its own exclusive resource.
    let tmp = TempDir::new("conc");
    let root_path = tmp.0.clone();
    std::fs::create_dir_all(&root_path).unwrap();

    std::thread::scope(|s| {
        for t in 0..8u32 {
            let path = root_path.join(format!("t{t}.bin"));
            s.spawn(move || {
                let mut node = LocalIO::from_path(&path);
                let values: Vec<i32> = (0..1000).map(|k| k * 10 + t as i32).collect();
                node.pwrite_i32_array(0, &values).unwrap();
                node.close();
            });
        }
    });

    // Every file has exactly its thread's data.
    for t in 0..8u32 {
        let node = LocalIO::from_path(root_path.join(format!("t{t}.bin")));
        let mut back = vec![0i32; 1000];
        node.pread_i32_array(0, &mut back).unwrap();
        let expected: Vec<i32> = (0..1000).map(|k| k * 10 + t as i32).collect();
        assert_eq!(back, expected);
    }
}

#[test]
fn uri_is_a_file_url_over_an_absolute_path() {
    let tmp = TempDir::new("fileuri");
    let node = tmp.root().join_str("sub/data.bin");
    let uri = node.uri();
    // A local node reports a file:// URL.
    assert_eq!(uri.scheme(), Some("file"));
    assert!(uri.to_string().starts_with("file:///"));
    assert!(uri.to_string().ends_with("data.bin"));
    // The mime type is inferred from the path (the node need not exist).
    assert_eq!(node.uri().name().unwrap(), "data.bin");

    // A relative path is made absolute at construction, so the handle carries a full path.
    let rel = LocalIO::from_path("relative/scratch.bin");
    assert!(rel.as_std_path().is_absolute());
    assert!(rel.uri().to_string().starts_with("file:///"));
    assert!(rel.uri().to_string().ends_with("relative/scratch.bin"));

    // The file:// URL round-trips back to the same handle.
    let back = LocalIO::from_uri(&node.uri()).unwrap();
    assert_eq!(back, node);
}

#[test]
fn tmpfile_and_tmpfolder_builders() {
    // A named tmpfile is lazy (nothing on disk), created on first write, and reads back.
    let mut f = LocalIO::tmpfile(Some("yggdryl_test_tmpfile.bin"));
    assert!(f.as_std_path().starts_with(std::env::temp_dir()));
    f.rmfile(true).ok(); // clean any leftover from a previous run
    assert!(!f.exists());
    f.pwrite_utf8(0, "scratch");
    assert!(f.is_file());
    assert_eq!(f.pread_utf8(0, 7).unwrap(), "scratch");
    f.close();
    f.rmfile(true).unwrap();

    // Two unnamed tmpfiles get distinct (unique) paths.
    let a = LocalIO::tmpfile(None);
    let b = LocalIO::tmpfile(None);
    assert_ne!(a.as_std_path(), b.as_std_path());
    assert!(a.name().ends_with(".tmp"));

    // A tmpfolder is lazy; writing a child auto-creates it, then rmdir cleans up.
    let work = LocalIO::tmpfolder(Some("yggdryl_test_tmpfolder"));
    work.rmdir(true).ok();
    assert!(!work.exists());
    let mut child = work.join_str("out.bin");
    child.pwrite_byte_array(0, b"x");
    assert!(work.is_dir());
    child.close();
    work.rmdir(true).unwrap();
    assert!(!work.exists());
}

#[test]
fn parents_iterates_the_ancestor_chain() {
    let tmp = TempDir::new("parents");
    let leaf = tmp.root().join("a/b/c/leaf.bin").unwrap();
    // parents() yields the ancestor nodes nearest-first; each is the next parent().
    let names: Vec<String> = leaf.parents().map(|p| p.name()).take(3).collect();
    assert_eq!(names, vec!["c", "b", "a"]);
    // It agrees with a manual parent() walk.
    assert_eq!(
        leaf.parents().next().unwrap().uri(),
        leaf.parent().unwrap().uri()
    );
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

    assert!(d
        .rmfile(true)
        .unwrap_err()
        .to_string()
        .contains("use rmdir"));
    assert!(f
        .rmdir(true)
        .unwrap_err()
        .to_string()
        .contains("use rmfile"));

    f.rmfile(true).unwrap();
    assert!(!f.exists());
    f.rmfile(true).unwrap(); // idempotent on missing (exist_ok = true, the default)
                             // With exist_ok = false, removing a missing node is a guided error naming the fix.
    let err = f.rmfile(false).unwrap_err().to_string();
    assert!(err.contains("nothing exists here") && err.contains("exist_ok=true"));
    assert!(root.join_str("nope").rm(false).is_err());
    d.rmdir(true).unwrap();
    assert!(!d.exists());
    assert!(d.rmdir(false).is_err()); // now missing → raises

    // rm removes whatever exists (file or whole tree).
    root.join_str("g.txt").pwrite_utf8(0, "y");
    root.join_str("h/i.txt").pwrite_utf8(0, "z");
    root.join_str("g.txt").rm(true).unwrap();
    root.join_str("h").rm(true).unwrap();
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

// -------------------------------------------------------------------------------------
// mmap() builder + tmpdir alias + truncate
// -------------------------------------------------------------------------------------

#[test]
fn mmap_builder_reuses_path_mode_and_headers() {
    let tmp = TempDir::new("mmapb");
    let mut node = tmp.root().join_str("mapped/data.bin");
    node.headers_mut()
        .set_content_type("application/octet-stream");

    // Read-write handle: mmap() auto-creates parents + file and copies the handle's headers.
    let mut map = node.mmap().unwrap();
    assert_eq!(
        map.headers().content_type(),
        Some("application/octet-stream")
    );
    map.pwrite_utf8(0, "mapped bytes");
    assert_eq!(map.pread_utf8(0, 12).unwrap(), "mapped bytes");
    map.flush().unwrap();
    drop(map); // releasing the mapping writes back + lets Windows re-open the file

    // A read-only handle maps read-only (writes are inert), and requires the file to exist.
    let mut ro = tmp.root().join_str("mapped/data.bin");
    ro.set_mode(yggdryl_core::io::IOMode::Read);
    let ro_map = ro.mmap().unwrap();
    assert_eq!(ro_map.pread_utf8(0, 12).unwrap(), "mapped bytes");
}

#[test]
fn tmpdir_is_tmpfolder_and_truncate_resizes() {
    // tmpdir aliases tmpfolder — a lazy temp folder handle, nothing on disk yet.
    let dir = LocalIO::tmpdir(Some("yggdryl_tmpdir_alias_test"));
    dir.rmdir(true).ok(); // clean any leftover from an earlier run of this fixed-name test
    assert!(!dir.exists());
    let mut file = dir.join_str("t.bin");
    file.pwrite_byte_array(0, b"0123456789");
    assert!(dir.is_dir());

    // truncate shrinks then grows (zero-filling) the mapped file.
    file.truncate(4).unwrap();
    assert_eq!(file.byte_size(), 4);
    assert_eq!(file.pread_vec(0, 8), b"0123");
    file.truncate(6).unwrap();
    assert_eq!(file.pread_vec(0, 6), b"0123\0\0");
    file.close();
    dir.rmdir(true).unwrap();
}

// -------------------------------------------------------------------------------------
// move_into — streamed relocation that deletes the source file
// -------------------------------------------------------------------------------------

#[test]
fn move_into_relocates_file_and_deletes_source() {
    let tmp = TempDir::new("move");
    let mut src = tmp.root().join_str("src.bin");
    src.pwrite_utf8(0, "move these bytes");
    src.close();

    let mut dst = tmp.root().join_str("moved/dst.bin");
    let moved = src.move_into(&mut dst).unwrap();
    dst.close();
    assert_eq!(moved, 16);
    assert!(!src.exists()); // the source file is gone after the move
    assert_eq!(dst.pread_utf8(0, 16).unwrap(), "move these bytes");

    // Moving a file onto its OWN path is a no-op — it does not delete the file.
    let mut self_move = tmp.root().join_str("keep.bin");
    self_move.pwrite_utf8(0, "stay");
    self_move.close();
    let mut same = tmp.root().join_str("keep.bin");
    assert_eq!(self_move.move_into(&mut same).unwrap(), 4);
    assert!(self_move.exists()); // still there — same address is skipped
    assert_eq!(self_move.pread_utf8(0, 4).unwrap(), "stay");
}

// -------------------------------------------------------------------------------------
// load() — eager mmap for read-heavy / concurrent access
// -------------------------------------------------------------------------------------

#[test]
fn load_maps_for_memory_speed_reads_and_is_concurrent() {
    use std::sync::Arc;

    let tmp = TempDir::new("load");
    let mut w = tmp.root().join_str("shared.bin");
    w.pwrite_i64_array(0, &(0..1024i64).collect::<Vec<_>>())
        .unwrap();
    w.close();

    // A fresh read-only handle: load() maps it once; later reads run from the mapping.
    let mut r = tmp.root().join_str("shared.bin");
    r.set_mode(yggdryl_core::io::IOMode::Read);
    assert!(!r.is_mapped());
    r.load().unwrap();
    assert!(r.is_mapped());

    // Shared across threads (Mmap is Send + Sync): concurrent readers see the same bytes.
    let shared = Arc::new(r);
    let mut handles = Vec::new();
    for t in 0..4u64 {
        let reader = Arc::clone(&shared);
        handles.push(std::thread::spawn(move || {
            let idx = (t * 100) as usize;
            reader.pread_i64(idx as u64 * 8).unwrap()
        }));
    }
    for (t, h) in handles.into_iter().enumerate() {
        assert_eq!(h.join().unwrap(), (t * 100) as i64);
    }

    // load() on a missing node is a lazy no-op (nothing to map, reads stay empty).
    let mut missing = tmp.root().join_str("nope.bin");
    missing.load().unwrap();
    assert!(!missing.is_mapped());
    assert_eq!(missing.pread_vec(0, 8), b"");
}
