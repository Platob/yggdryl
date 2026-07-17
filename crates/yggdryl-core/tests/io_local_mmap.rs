//! Functional tests for the memory-mapped [`Mmap`](yggdryl_core::io::local::Mmap) source:
//! uri-addressed open/create, the shared `IOBase` surface (typed + bulk + utf8 access, the
//! cursor stream, wrappers), **auto-resizing** writes with amortized growth, persistence
//! (capacity padding truncated on drop), read-only mappings, and the guided file errors.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use yggdryl_core::io::local::Mmap;
use yggdryl_core::io::memory::{IOBase, IoError};
use yggdryl_core::io::{IOKind, IOMode};
use yggdryl_core::uri::Uri;

/// A unique temp file per test (process id + counter), removed on drop.
struct TempPath(PathBuf);

impl TempPath {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        Self(std::env::temp_dir().join(format!("yggdryl_mmap_{tag}_{pid}_{n}.bin")))
    }

    fn uri(&self) -> Uri {
        Uri::from_path(&self.0.to_string_lossy())
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        std::fs::remove_file(&self.0).ok();
    }
}

// -------------------------------------------------------------------------------------
// Open / create / errors
// -------------------------------------------------------------------------------------

#[test]
fn create_open_and_kind_mode_uri() {
    let tmp = TempPath::new("create");
    let map = Mmap::create_uri(&tmp.uri()).unwrap();
    assert_eq!(map.byte_size(), 0);
    assert!(map.is_empty());
    assert_eq!(map.kind(), IOKind::File);
    assert_eq!(map.mode(), IOMode::ReadWrite);
    // The mapping reports its own file address back.
    assert_eq!(map.uri().name(), tmp.uri().name());
}

#[test]
fn open_missing_file_is_guided_error() {
    let tmp = TempPath::new("missing");
    let err = Mmap::open_uri(&tmp.uri()).unwrap_err();
    assert!(matches!(err, IoError::FileIo { op: "open", .. }));
    assert!(err.to_string().contains("check that the path exists"));
}

#[test]
fn non_file_scheme_is_rejected() {
    let uri = Uri::parse_str("https://example.com/data.bin").unwrap();
    let err = Mmap::open_uri(&uri).unwrap_err();
    assert!(err.to_string().contains("unsupported scheme"));
}

#[test]
fn mmap_is_a_leaf_node_of_the_graph_that_really_unlinks() {
    let tmp = TempPath::new("graph");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();
    map.pwrite_utf8(0, "hi");

    // A raw file is a LEAF: it streams no children and has no parent.
    assert_eq!(map.ls().unwrap().count(), 0);
    assert_eq!(map.ls_recursive().unwrap().count(), 0);
    assert!(map.parent().is_none());
    assert_eq!(map.name(), tmp.0.file_name().unwrap().to_string_lossy());

    // Unlike an in-memory leaf, a file has a removable backing — but as a file it refuses
    // rmdir with the guided fix.
    assert!(map.rmdir().unwrap_err().to_string().contains("use rmfile"));
    // Drop the mapping before removal (Windows cannot delete a mapped file).
    drop(map);
    let handle = Mmap::open_uri(&tmp.uri()).unwrap();
    drop(handle); // no live view over the (now non-empty) file
    let fresh = Mmap::open_uri(&tmp.uri()).unwrap();
    fresh.rmfile().unwrap();
    assert!(!tmp.0.exists());
    fresh.rmfile().unwrap(); // idempotent on missing
}

// -------------------------------------------------------------------------------------
// The shared IOBase surface over a file
// -------------------------------------------------------------------------------------

#[test]
fn typed_bulk_and_utf8_roundtrip() {
    let tmp = TempPath::new("typed");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();

    map.pwrite_byte(0, 0x7F).unwrap();
    map.pwrite_i32(1, -42).unwrap();
    map.pwrite_i64(5, 1 << 40).unwrap();
    map.pwrite_bit(104, true).unwrap(); // bit 0 of byte 13
    assert_eq!(map.pread_byte(0).unwrap(), 0x7F);
    assert_eq!(map.pread_i32(1).unwrap(), -42);
    assert_eq!(map.pread_i64(5).unwrap(), 1 << 40);
    assert!(map.pread_bit(104).unwrap());

    // Bulk arrays + repeats through the trait defaults, over the mapping.
    let values: Vec<i32> = (0..1000).collect();
    map.pwrite_i32_array(16, &values).unwrap();
    let mut back = vec![0i32; 1000];
    map.pread_i32_array(16, &mut back).unwrap();
    assert_eq!(back, values);
    map.pwrite_i64_repeat(4016, -1, 100).unwrap();
    let mut wide = vec![0i64; 100];
    map.pread_i64_array(4016, &mut wide).unwrap();
    assert!(wide.iter().all(|&v| v == -1));

    // UTF-8 + EOF errors behave exactly like Heap's.
    assert_eq!(map.pwrite_utf8(4816, "héllo"), 6);
    assert_eq!(map.pread_utf8(4816, 6).unwrap(), "héllo");
    assert!(matches!(
        map.pread_i32(map.byte_size()).unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
}

#[test]
fn direct_bulk_overrides_match_the_staged_semantics() {
    // The `Mmap` bulk methods are direct contiguous conversions off the mapping (not the
    // stack-staged default); this pins their edge behavior to the trait contract.
    let tmp = TempPath::new("bulk");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();

    // A bulk write grows the mapping; the readback matches exactly, at an offset.
    let values: Vec<i32> = (-500..500).collect();
    map.pwrite_i32_array(40, &values).unwrap();
    let mut back = vec![0i32; 1000];
    map.pread_i32_array(40, &mut back).unwrap();
    assert_eq!(back, values);
    assert_eq!(map.byte_size(), 40 + 4000);

    // i64 + the repeats take the same direct path.
    map.pwrite_i64_array(0, &[1, -2, 3, -4, 5]).unwrap();
    let mut wide = [0i64; 5];
    map.pread_i64_array(0, &mut wide).unwrap();
    assert_eq!(wide, [1, -2, 3, -4, 5]);
    map.pwrite_i32_repeat(0, -7, 8).unwrap();
    let mut sevens = [0i32; 8];
    map.pread_i32_array(0, &mut sevens).unwrap();
    assert!(sevens.iter().all(|&v| v == -7));

    // A bulk read past the end is a guided short-read, not a panic or partial fill.
    let mut over = vec![0i32; map.byte_size() as usize]; // far more than exists
    let err = map.pread_i32_array(0, &mut over).unwrap_err();
    assert!(matches!(err, IoError::UnexpectedEof { .. }));

    // A read-only mapping refuses every bulk write with the guided fix (writes nothing).
    drop(map);
    let mut ro = Mmap::open_uri_readonly(&tmp.uri()).unwrap();
    let err = ro.pwrite_i32_array(0, &[1, 2, 3]).unwrap_err();
    assert!(err.to_string().contains("read-only"));
    assert!(ro
        .pwrite_byte_repeat(0, 0, 4)
        .unwrap_err()
        .to_string()
        .contains("read-only"));
}

#[test]
fn many_threads_read_one_shared_mapping() {
    // A mapping is Send + Sync for concurrent READS (`&self`): N threads hammer one shared
    // Arc-wrapped mapping and every read is correct, no data race, no lock.
    use std::sync::Arc;

    let tmp = TempPath::new("shared");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();
    let values: Vec<i32> = (0..1000).collect();
    map.pwrite_i32_array(0, &values).unwrap();
    let shared = Arc::new(map);

    let sum: i64 = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let m = Arc::clone(&shared);
                s.spawn(move || {
                    let mut acc = 0i64;
                    for _ in 0..10_000 {
                        for k in 0..1000u64 {
                            acc += m.pread_i32(k * 4).unwrap() as i64;
                        }
                    }
                    acc
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).sum()
    });
    // Each thread summed 0..1000 (= 499_500) ten thousand times; eight threads.
    assert_eq!(sum, 499_500 * 10_000 * 8);
}

#[test]
fn cursor_stream_and_wrappers() {
    let tmp = TempPath::new("cursor");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();

    // The built-in cursor stream (same macro surface as Heap).
    map.write_utf8("stream");
    map.write_i32(7).unwrap();
    map.rewind();
    assert_eq!(map.read_utf8(6).unwrap(), "stream");
    assert_eq!(map.read_i32().unwrap(), 7);

    // The generic wrappers compose over a mapping too.
    let win = map.window(0, 6).unwrap();
    assert_eq!(win.pread_utf8(0, 6).unwrap(), "stream");
    assert_eq!(win.kind(), IOKind::File);
    let mut cur = win.cursor();
    assert_eq!(cur.read_utf8(6).unwrap(), "stream");
}

// -------------------------------------------------------------------------------------
// Auto-resizing + persistence
// -------------------------------------------------------------------------------------

#[test]
fn auto_resizing_appends_amortize_and_truncate_on_drop() {
    let tmp = TempPath::new("grow");
    {
        let mut map = Mmap::create_uri(&tmp.uri()).unwrap();
        let chunk = [0xA5u8; 1024];
        for i in 0..64u64 {
            let end = map.byte_size();
            assert_eq!(map.pwrite_byte_array(end, &chunk), 1024);
            assert_eq!(map.byte_size(), (i + 1) * 1024);
        }
        // Amortized doubling: capacity >= size, and the on-disk file is the capacity while
        // open (its padding is reclaimed on drop below).
        assert!(map.capacity() >= 64 * 1024);
        assert!(map.spare_capacity() == map.capacity() - 64 * 1024);
    }
    // Dropped: the file is truncated back to the logical length…
    assert_eq!(std::fs::metadata(&tmp.0).unwrap().len(), 64 * 1024);
    // …and reopening sees exactly the written bytes.
    let reopened = Mmap::open_uri(&tmp.uri()).unwrap();
    assert_eq!(reopened.byte_size(), 64 * 1024);
    assert_eq!(reopened.pread_vec(0, 8), vec![0xA5; 8]);
}

#[test]
fn gap_writes_zero_fill_and_flush_persists() {
    let tmp = TempPath::new("gap");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();
    map.pwrite_byte_array(8, b"tail");
    assert_eq!(map.byte_size(), 12);
    assert_eq!(map.pread_vec(0, 8), vec![0u8; 8]); // the gap is zero-filled
    map.flush().unwrap();
}

#[test]
fn capacity_family_over_a_file() {
    let tmp = TempPath::new("cap");
    let mut map = Mmap::create_uri(&tmp.uri()).unwrap();
    map.try_reserve(8192).unwrap();
    assert!(map.capacity() >= 8192);
    map.pwrite_byte_array(0, &[1; 100]);
    map.shrink_to_fit();
    assert_eq!(map.capacity(), 100);
    assert_eq!(map.pread_vec(0, 100), vec![1u8; 100]); // contents survive the remap
    map.ensure_capacity(4096);
    assert!(map.capacity() >= 4096);
}

// -------------------------------------------------------------------------------------
// Read-only mappings
// -------------------------------------------------------------------------------------

#[test]
fn readonly_reads_but_never_writes() {
    let tmp = TempPath::new("ro");
    {
        let mut map = Mmap::create_uri(&tmp.uri()).unwrap();
        map.pwrite_utf8(0, "locked");
    }
    let mut ro = Mmap::open_uri_readonly(&tmp.uri()).unwrap();
    assert_eq!(ro.mode(), IOMode::Read);
    assert_eq!(ro.pread_utf8(0, 6).unwrap(), "locked");
    // The primitive writes nothing; the full write names the fix.
    assert_eq!(ro.pwrite_byte_array(0, b"x"), 0);
    let err = ro.pwrite_all(0, b"x").unwrap_err();
    assert!(err.to_string().contains("read-only"));
    assert!(ro.try_reserve(1024).is_err());
    assert_eq!(ro.pread_utf8(0, 6).unwrap(), "locked"); // untouched
}
