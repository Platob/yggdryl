//! Functional tests for the in-heap [`Heap`](yggdryl_core::io::memory::Heap) source and the byte
//! I/O trait surface it implements — the positioned primitives and typed accessors on
//! [`IOBase`](yggdryl_core::io::memory::IOBase), the cursor stream on
//! [`IOCursor`](yggdryl_core::io::memory::IOCursor), bounded [`IOSlice`](yggdryl_core::io::memory::IOSlice)
//! windows, and [`Whence`](yggdryl_core::io::memory::Whence) seeks. Doctests cover the happy paths;
//! this file hammers the edges (EOF, bit addressing, capacity reuse, content equality).

use yggdryl_core::io::memory::{
    Heap, HeapCursor, HeapSlice, IOBase, IOCursor, IOSlice, IoError, Whence,
};
use yggdryl_core::uri::Uri;

// -------------------------------------------------------------------------------------
// Size + capacity
// -------------------------------------------------------------------------------------

#[test]
fn byte_and_bit_size() {
    let h = Heap::from_slice(b"abcd");
    assert_eq!(h.byte_size(), 4);
    assert_eq!(h.bit_size(), 32);
    assert!(!h.is_empty());
    assert!(Heap::new().is_empty());
    assert_eq!(Heap::new().bit_size(), 0);
}

#[test]
fn with_capacity_and_reserve() {
    let mut h = Heap::with_capacity(64);
    assert!(h.is_empty());
    assert!(h.capacity() >= 64);

    // Writing within capacity keeps the same allocation.
    let cap = h.capacity();
    h.pwrite_byte_array(0, &[1, 2, 3, 4]);
    assert_eq!(h.byte_size(), 4);
    assert_eq!(h.capacity(), cap);

    // reserve grows capacity but not size.
    h.reserve(1000);
    assert!(h.capacity() >= 1004);
    assert_eq!(h.byte_size(), 4);
}

// -------------------------------------------------------------------------------------
// Byte-array primitives
// -------------------------------------------------------------------------------------

#[test]
fn pread_byte_array_short_and_empty() {
    let h = Heap::from_slice(b"hello");
    let mut buf = [0u8; 8];
    assert_eq!(h.pread_byte_array(2, &mut buf), 3); // only 3 remain from offset 2
    assert_eq!(&buf[..3], b"llo");
    assert_eq!(h.pread_byte_array(5, &mut buf), 0); // at the end
    assert_eq!(h.pread_byte_array(99, &mut buf), 0); // past the end
}

#[test]
fn pwrite_byte_array_grows_and_zero_fills() {
    let mut h = Heap::from_slice(b"abc");
    assert_eq!(h.pwrite_byte_array(5, b"Z"), 1);
    assert_eq!(h.as_slice(), b"abc\0\0Z");
    // Overwriting in place does not grow.
    assert_eq!(h.pwrite_byte_array(0, b"XY"), 2);
    assert_eq!(h.as_slice(), b"XYc\0\0Z");
    // Empty write is a no-op.
    assert_eq!(h.pwrite_byte_array(0, b""), 0);
}

#[test]
fn pread_exact_reports_shortfall() {
    let h = Heap::from_slice(b"abc");
    let mut buf = [0u8; 5];
    let err = h.pread_exact(1, &mut buf).unwrap_err();
    assert_eq!(
        err,
        IoError::UnexpectedEof {
            offset: 3, // ran out after reading 2 (offset 1 + 2)
            requested: 5,
            available: 2,
        }
    );
}

// -------------------------------------------------------------------------------------
// pread_into — allocation-reusing transfer
// -------------------------------------------------------------------------------------

#[test]
fn pread_into_reuses_buffer() {
    let src = Heap::from_slice(b"hello world");
    let mut scratch = Vec::new();
    assert_eq!(src.pread_into(0, 5, &mut scratch), 5);
    assert_eq!(&scratch, b"hello");
    let cap = scratch.capacity();
    assert_eq!(src.pread_into(6, 5, &mut scratch), 5);
    assert_eq!(&scratch, b"world");
    assert_eq!(
        scratch.capacity(),
        cap,
        "buffer must be reused, not reallocated"
    );
    // Short near the end: reads what remains.
    assert_eq!(src.pread_into(9, 100, &mut scratch), 2);
    assert_eq!(&scratch, b"ld");
}

// -------------------------------------------------------------------------------------
// Typed positioned accessors: byte / bit / i32 / i64
// -------------------------------------------------------------------------------------

#[test]
fn typed_byte_roundtrip_and_eof() {
    let mut h = Heap::new();
    h.pwrite_byte(3, 0xAB).unwrap(); // grows to 4, zero-filling 0..3
    assert_eq!(h.as_slice(), &[0, 0, 0, 0xAB]);
    assert_eq!(h.pread_byte(3).unwrap(), 0xAB);
    assert_eq!(h.pread_byte(0).unwrap(), 0);
    assert!(matches!(
        h.pread_byte(4).unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
}

#[test]
fn typed_bit_lsb_first() {
    let h = Heap::from_slice(&[0b0000_0101, 0b1000_0000]);
    assert!(h.pread_bit(0).unwrap()); // byte 0, bit 0
    assert!(!h.pread_bit(1).unwrap());
    assert!(h.pread_bit(2).unwrap());
    assert!(h.pread_bit(15).unwrap()); // byte 1, bit 7 (MSB)
    assert!(!h.pread_bit(8).unwrap());
    // Reading a bit past the end is EOF.
    assert!(h.pread_bit(16).is_err());
}

#[test]
fn typed_bit_write_grows_and_sets() {
    let mut h = Heap::new();
    h.pwrite_bit(10, true).unwrap(); // byte 1, bit 2 — grows to 2 bytes
    assert_eq!(h.as_slice(), &[0b0000_0000, 0b0000_0100]);
    assert!(h.pread_bit(10).unwrap());
    // Clearing an already-set bit, read-modify-write.
    h.pwrite_bit(10, false).unwrap();
    assert_eq!(h.as_slice(), &[0, 0]);
    // Setting a second bit in the same byte preserves the first.
    h.pwrite_bit(1, true).unwrap();
    h.pwrite_bit(3, true).unwrap();
    assert_eq!(h.as_slice()[0], 0b0000_1010);
}

#[test]
fn typed_i32_i64_little_endian_and_eof() {
    let mut h = Heap::new();
    h.pwrite_i32(0, -42).unwrap();
    h.pwrite_i64(4, 1234567890123).unwrap();
    assert_eq!(&h.as_slice()[..4], &(-42i32).to_le_bytes());
    assert_eq!(h.pread_i32(0).unwrap(), -42);
    assert_eq!(h.pread_i64(4).unwrap(), 1234567890123);
    // i32 needing 4 bytes with only 3 available -> EOF.
    let small = Heap::from_slice(b"abc");
    assert!(matches!(
        small.pread_i32(0).unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
    assert!(small.pread_i64(0).is_err());
}

// -------------------------------------------------------------------------------------
// Cursor stream
// -------------------------------------------------------------------------------------

#[test]
fn cursor_read_write_advances() {
    let mut h = Heap::new();
    assert_eq!(h.write(b"hello"), 5);
    assert_eq!(h.position(), 5);
    h.rewind();
    assert_eq!(h.position(), 0);
    let mut buf = [0u8; 5];
    assert_eq!(h.read(&mut buf), 5);
    assert_eq!(&buf, b"hello");
    assert_eq!(h.position(), 5);
}

#[test]
fn cursor_typed_roundtrip() {
    let mut h = Heap::new();
    h.write_byte(0x7F).unwrap();
    h.write_i32(-7).unwrap();
    h.write_i64(1 << 40).unwrap();
    assert_eq!(h.position(), 1 + 4 + 8);
    h.rewind();
    assert_eq!(h.read_byte().unwrap(), 0x7F);
    assert_eq!(h.read_i32().unwrap(), -7);
    assert_eq!(h.read_i64().unwrap(), 1 << 40);
    // Reading past the end leaves the cursor put and errors.
    let pos = h.position();
    assert!(h.read_byte().is_err());
    assert_eq!(h.position(), pos);
}

#[test]
fn cursor_read_exact_leaves_cursor_on_eof() {
    let mut h = Heap::from_slice(b"ab");
    h.set_position(0);
    let mut buf = [0u8; 4];
    assert!(h.read_exact(&mut buf).is_err());
    assert_eq!(h.position(), 0, "a failed read_exact must not advance");
}

#[test]
fn cursor_bulk_readers() {
    let mut h = Heap::from_slice(b"hello world");
    assert_eq!(h.read_vec(5), b"hello");
    assert_eq!(h.position(), 5);
    assert_eq!(h.read_to_end(), b" world");
    assert_eq!(h.position(), 11);

    h.rewind();
    assert_eq!(h.read_exact_vec(5).unwrap(), b"hello");
    // read_exact_vec of more than remains errors (bounded, no giant alloc).
    assert!(h.read_exact_vec(1000).is_err());
}

// -------------------------------------------------------------------------------------
// Seek / Whence
// -------------------------------------------------------------------------------------

#[test]
fn seek_from_all_anchors() {
    let mut h = Heap::from_slice(b"hello world");
    assert_eq!(h.seek(Whence::Start, 6).unwrap(), 6);
    assert_eq!(h.seek(Whence::Current, -1).unwrap(), 5);
    assert_eq!(h.seek(Whence::End, -5).unwrap(), 6);
    // Past the end is allowed.
    assert_eq!(h.seek(Whence::End, 10).unwrap(), 21);
    // Before the start is not.
    assert!(matches!(
        h.seek(Whence::Start, -1).unwrap_err(),
        IoError::InvalidSeek { .. }
    ));
}

#[test]
fn write_past_seeked_end_zero_fills() {
    let mut h = Heap::new();
    h.seek(Whence::Start, 4).unwrap();
    h.write(b"Z");
    assert_eq!(h.as_slice(), &[0, 0, 0, 0, b'Z']);
}

// -------------------------------------------------------------------------------------
// Slice
// -------------------------------------------------------------------------------------

#[test]
fn slice_window_and_bounds() {
    let h = Heap::from_slice(b"hello world");
    let world = h.slice(6, 5).unwrap();
    assert_eq!(world.byte_size(), 5);
    assert_eq!(world.as_slice(), b"world");
    // A window addresses from its own 0 and can be sliced again.
    assert_eq!(world.slice(0, 2).unwrap().as_slice(), b"wo");
    // Out of bounds names the fix.
    assert_eq!(
        h.slice(6, 6).unwrap_err(),
        IoError::SliceOutOfBounds {
            offset: 6,
            len: 6,
            available: 11,
        }
    );
}

// -------------------------------------------------------------------------------------
// Value semantics
// -------------------------------------------------------------------------------------

#[test]
fn equality_ignores_cursor() {
    let mut a = Heap::from_slice(b"same");
    let b = Heap::from_slice(b"same");
    a.set_position(3); // different cursor
    assert_eq!(a, b, "equality is over the bytes, not the cursor");
    assert_ne!(Heap::from_slice(b"same"), Heap::from_slice(b"diff"));
}

#[test]
fn from_vec_is_zero_copy_into_vec_roundtrips() {
    let v = vec![1u8, 2, 3];
    let h = Heap::from_vec(v);
    assert_eq!(h.as_slice(), &[1, 2, 3]);
    assert_eq!(h.into_vec(), vec![1, 2, 3]);
}

// -------------------------------------------------------------------------------------
// Addressing URI
// -------------------------------------------------------------------------------------

#[test]
fn uri_is_always_the_synthetic_mem_heap() {
    // A heap stores no address: every heap reports the stable synthetic mem:// address
    // (deterministic), regardless of contents or state.
    assert_eq!(Heap::new().uri().to_string(), "mem://heap");
    assert_eq!(Heap::new().uri().scheme(), "mem");
    assert_eq!(Heap::new().uri().host(), Some("heap"));
    assert_eq!(Heap::from_slice(b"x").uri().to_string(), "mem://heap");
    // It parses as a real Uri.
    assert_eq!(
        Uri::parse_str("mem://heap").unwrap().to_string(),
        Heap::new().uri().to_string()
    );
}

// -------------------------------------------------------------------------------------
// IOCursor<T> wrapper — a cursor over any source
// -------------------------------------------------------------------------------------

#[test]
fn cursor_wrapper_over_a_source() {
    let mut cur: IOCursor<Heap> = Heap::new().cursor();
    cur.write_byte(0x7F).unwrap();
    cur.write_i32(-7).unwrap();
    cur.write_i64(1 << 40).unwrap();
    assert_eq!(cur.byte_size(), 13); // IOBase delegates to the wrapped source
    cur.rewind();
    assert_eq!(cur.read_byte().unwrap(), 0x7F);
    assert_eq!(cur.read_i32().unwrap(), -7);
    assert_eq!(cur.read_i64().unwrap(), 1 << 40);

    // The wrapper owns its source; you can get it back.
    let inner: &Heap = cur.inner();
    assert_eq!(inner.byte_size(), 13);
    let heap = cur.into_inner();
    assert_eq!(heap.byte_size(), 13);
}

#[test]
fn cursor_wrapper_delegates_uri() {
    // The wrapper reports its inner source's address — for a heap, the synthetic default.
    let cur = Heap::from_slice(b"x").cursor();
    assert_eq!(cur.uri().to_string(), "mem://heap");
}

#[test]
fn wrappers_forward_the_existence_predicates() {
    // A live heap exists (its own override), and the wrappers forward that notion instead
    // of re-deriving it from `kind` alone.
    let cur = Heap::new().cursor();
    assert!(cur.exists() && !cur.is_file() && !cur.is_dir());
    let win = Heap::from_slice(b"hello").window(1, 3).unwrap();
    assert!(win.exists() && !win.is_file() && !win.is_dir());
}

#[test]
fn heap_join_composes_addresses_over_an_in_memory_buffer() {
    // The uniform graph `join`/`parent` work over an in-memory heap as address algebra: the
    // child is an independent buffer, but its address composes through the URI (joinpath),
    // and `parent()` is the inverse.
    let root = Heap::new();
    assert_eq!(root.uri().to_string(), "mem://heap");
    assert!(root.parent().is_none()); // the root has no parent

    let mut child = root.join("logs/app.bin").unwrap();
    assert_eq!(child.uri().to_string(), "mem://heap/logs/app.bin");
    assert_eq!(child.name(), "app.bin");

    // The child is a real, independent buffer — writing and reading it works.
    child.pwrite_utf8(0, "entry");
    assert_eq!(child.pread_utf8(0, 5).unwrap(), "entry");
    assert_eq!(child.byte_size(), 5);
    // The parent heap is a *different* buffer (addresses compose; bytes do not).
    assert_eq!(root.byte_size(), 0);

    // parent() navigates back up the URI, the exact inverse of join.
    assert_eq!(child.parent().unwrap().uri().to_string(), "mem://heap/logs");
    assert_eq!(
        child.parent().unwrap().parent().unwrap().uri().to_string(),
        "mem://heap"
    );
    assert!(child.parent().unwrap().parent().unwrap().parent().is_none());

    // A percent-encoding round-trip: a spaced segment encodes on join and the retained
    // portion is not double-encoded on parent.
    let spaced = root.join("my dir/my file").unwrap();
    assert_eq!(spaced.uri().to_string(), "mem://heap/my%20dir/my%20file");
    assert_eq!(
        spaced.parent().unwrap().uri().to_string(),
        "mem://heap/my%20dir"
    );

    // An untouched heap still allocates nothing for its address (None → the static default).
    assert_eq!(Heap::new().uri().to_string(), "mem://heap");
}

#[test]
fn join_edge_cases_and_identities() {
    use yggdryl_core::io::Serializable;

    let root = Heap::new();

    // An empty segment is a no-op: the child addresses the same place.
    assert_eq!(root.join("").unwrap().uri(), root.uri());
    // An absolute segment re-roots (the URI join algebra), keeping the scheme/host.
    let child = root.join("a/b").unwrap();
    assert_eq!(
        child.join("/reset").unwrap().uri().to_string(),
        "mem://heap/reset"
    );
    // A trailing slash on the base is not doubled.
    assert_eq!(
        root.join("dir/")
            .unwrap()
            .join("f")
            .unwrap()
            .uri()
            .to_string(),
        "mem://heap/dir/f"
    );

    // A joined heap is equal-by-bytes regardless of address (address is transient metadata),
    // and its serialized form is its bytes only.
    let mut a = root.join("x").unwrap();
    let mut b = root.join("y").unwrap();
    a.pwrite_utf8(0, "same");
    b.pwrite_utf8(0, "same");
    assert_eq!(a, b); // different addresses, same bytes -> equal
    assert_eq!(a.serialize_bytes(), b"same");

    // Clone preserves the address.
    let clone = a.clone();
    assert_eq!(clone.uri(), a.uri());

    // A wrapper (cursor) has no child path space: join is a guided error.
    let cur = Heap::from_slice(b"z").cursor();
    let err = cur.join("child").unwrap_err().to_string();
    assert!(err.contains("no child path space") && err.contains("LocalIO"));
}

#[test]
fn media_type_inference_headers_then_uri_then_octet_stream() {
    // A bare heap: no headers, no address extension -> the octet-stream fallback (never None).
    let mut h = Heap::new();
    assert!(h.mime_type().is_octet_stream());
    assert_eq!(h.media_type().essences(), vec!["application/octet-stream"]);

    // A heap addressed by a name with an extension infers from the address.
    let logs = Heap::new().join("data/records.json").unwrap();
    assert_eq!(logs.mime_type().essence(), "application/json");

    // Declared headers win over the address.
    let mut named = Heap::new().join("thing.json").unwrap();
    named.headers_mut().set_content_type("text/csv");
    assert_eq!(named.mime_type().essence(), "text/csv");

    // ensure_content_type memoizes the inference into the headers (only when absent).
    let inferred = h.ensure_content_type();
    assert!(inferred.is_octet_stream());
    assert_eq!(h.headers().content_type(), Some("application/octet-stream")); // stored
                                                                              // A second call reads the stored header (and does not overwrite a set value).
    h.headers_mut().set_content_type("application/json");
    assert_eq!(h.ensure_content_type().essence(), "application/json");
}

#[test]
fn infer_mime_type_reads_magic_without_seeking() {
    // A PNG header: magic inference works even with no headers and no address, and a
    // positioned head read must NOT move the cursor.
    let mut png = Heap::from_slice(b"\x89PNG\r\n\x1a\nrest of the file...");
    png.set_position(3);
    assert_eq!(png.infer_mime_type().essence(), "image/png"); // from magic
    assert_eq!(png.position(), 3, "infer must not seek the cursor");

    // Recursive media inference of a non-compressed magic type is just that type.
    assert_eq!(png.infer_media_type().essences(), vec!["image/png"]);

    // No magic + no headers + no address extension -> the octet-stream fallback.
    let plain = Heap::from_slice(b"just some text");
    assert!(plain.infer_mime_type().is_octet_stream());
}

#[test]
fn leaf_sources_carry_the_graph_surface() {
    // IOBase is the central access path: every source is a node of the IO graph. A heap
    // (and the wrapper views) are LEAVES — they stream no children and have no parent.
    let heap = Heap::from_slice(b"x");
    assert_eq!(heap.ls().unwrap().count(), 0);
    assert_eq!(heap.ls_recursive().unwrap().count(), 0);
    assert!(heap.children().unwrap().is_empty());
    assert!(heap.parent().is_none());
    assert_eq!(heap.name(), ""); // mem://heap has no path segment to name
    assert_eq!(heap.tree_byte_size(), 0); // a leaf's tree is empty

    // Removal has no backing here — a guided refusal names the fix.
    let err = heap.rm(true).unwrap_err().to_string();
    assert!(err.contains("removable backing") && err.contains("LocalIO"));
    assert!(heap
        .rmfile(true)
        .unwrap_err()
        .to_string()
        .contains("rmfile"));
    assert!(heap.rmdir(true).unwrap_err().to_string().contains("rmdir"));

    // The wrappers are leaf byte views too.
    let cur = Heap::new().cursor();
    assert_eq!(cur.ls().unwrap().count(), 0);
    let win = Heap::from_slice(b"hello").window(1, 3).unwrap();
    assert_eq!(win.ls().unwrap().count(), 0);
}

// -------------------------------------------------------------------------------------
// IOSlice<T> wrapper — a bounded window over any source
// -------------------------------------------------------------------------------------

#[test]
fn window_wrapper_view_and_bounds() {
    let win: IOSlice<Heap> = Heap::from_slice(b"hello world").window(6, 5).unwrap();
    assert_eq!(win.byte_size(), 5);
    assert_eq!(win.offset(), 6);
    assert_eq!(win.pread_vec(0, 5), b"world"); // addressed from its own 0
    assert_eq!(win.pread_byte(0).unwrap(), b'w');
    // A read past the window end returns nothing.
    assert_eq!(win.pread_byte_array(5, &mut [0u8; 4]), 0);
    // Out of bounds names the fix.
    assert_eq!(
        Heap::from_slice(b"hello world").window(6, 6).unwrap_err(),
        IoError::SliceOutOfBounds {
            offset: 6,
            len: 6,
            available: 11,
        }
    );
}

#[test]
fn window_write_is_clamped_to_the_window() {
    let mut win = Heap::from_slice(b"hello world").window(6, 5).unwrap();
    // Writing more than the window holds is clamped (the window can't grow the source).
    assert_eq!(win.pwrite_byte_array(3, b"ABCDEF"), 2); // only 2 bytes fit (offsets 3,4)
    assert_eq!(win.pread_vec(0, 5), b"worAB");
    // A write starting past the window end writes nothing.
    assert_eq!(win.pwrite_byte_array(5, b"Z"), 0);
}

#[test]
fn window_is_composable() {
    // A window of a window, and a cursor over a window.
    let outer = Heap::from_slice(b"abcdefgh").window(2, 5).unwrap(); // "cdefg"
    let inner = outer.window(1, 3).unwrap(); // "def"
    assert_eq!(inner.pread_vec(0, 3), b"def");

    let mut cur = Heap::from_slice(b"abcdefgh").window(2, 4).unwrap().cursor();
    assert_eq!(cur.read_vec(4), b"cdef");
}

// -------------------------------------------------------------------------------------
// Bulk typed arrays + repeated-value fills (vectorized, zero-heap-alloc staging)
// -------------------------------------------------------------------------------------

#[test]
fn bulk_i32_array_roundtrip_and_eof() {
    let mut h = Heap::new();
    // Longer than one 256-element staging chunk, to cross the chunk boundary.
    let values: Vec<i32> = (0..1000).map(|i| i * 3 - 1500).collect();
    h.pwrite_i32_array(0, &values).unwrap();
    assert_eq!(h.byte_size(), 4000);

    let mut back = vec![0i32; 1000];
    h.pread_i32_array(0, &mut back).unwrap();
    assert_eq!(back, values);

    // Misaligned offset still works (byte-addressed), and running past the end errors.
    assert_eq!(h.pread_i32(4).unwrap(), values[1]);
    let mut too_many = vec![0i32; 1001];
    assert!(matches!(
        h.pread_i32_array(0, &mut too_many).unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
}

#[test]
fn bulk_i64_array_roundtrip() {
    let mut h = Heap::new();
    let values: Vec<i64> = (0..300).map(|i| (i as i64) << 33).collect();
    h.pwrite_i64_array(0, &values).unwrap();
    let mut back = vec![0i64; 300];
    h.pread_i64_array(0, &mut back).unwrap();
    assert_eq!(back, values);
}

#[test]
fn repeat_fills_without_building_arrays() {
    let mut h = Heap::new();
    h.pwrite_byte_repeat(0, 0xAB, 3000).unwrap(); // > one staging chunk
    assert_eq!(h.byte_size(), 3000);
    assert!(h.as_slice().iter().all(|&b| b == 0xAB));

    let mut h32 = Heap::new();
    h32.pwrite_i32_repeat(0, -7, 700).unwrap();
    let mut back = vec![0i32; 700];
    h32.pread_i32_array(0, &mut back).unwrap();
    assert!(back.iter().all(|&v| v == -7));

    let mut h64 = Heap::new();
    h64.pwrite_i64_repeat(4, 1 << 40, 300).unwrap(); // offset keeps a zero-filled gap
    assert_eq!(h64.pread_i32(0).unwrap(), 0);
    let mut wide = vec![0i64; 300];
    h64.pread_i64_array(4, &mut wide).unwrap();
    assert!(wide.iter().all(|&v| v == 1 << 40));
}

// -------------------------------------------------------------------------------------
// UTF-8 read/write over the byte layer
// -------------------------------------------------------------------------------------

#[test]
fn utf8_positioned_roundtrip_and_errors() {
    let mut h = Heap::new();
    assert_eq!(h.pwrite_utf8(0, "héllo wörld"), 13); // 2 two-byte chars
    assert_eq!(h.pread_utf8(0, 13).unwrap(), "héllo wörld");
    // Clamped near the end like pread_vec.
    assert_eq!(h.pread_utf8(7, 100).unwrap(), "wörld");
    // Cutting a multi-byte character mid-sequence is a guided error.
    let err = h.pread_utf8(0, 2).unwrap_err();
    assert!(matches!(err, IoError::InvalidUtf8 { .. }));
    assert!(err.to_string().contains("invalid UTF-8"));
    // Non-text bytes error too.
    let bin = Heap::from_slice(&[0xff, 0xfe]);
    assert!(bin.pread_utf8(0, 2).is_err());
}

#[test]
fn utf8_cursor_roundtrip() {
    let mut h = Heap::new();
    assert_eq!(h.write_utf8("ab"), 2);
    assert_eq!(h.write_utf8("cé"), 3);
    assert_eq!(h.position(), 5);
    h.rewind();
    assert_eq!(h.read_utf8(2).unwrap(), "ab");
    assert_eq!(h.read_utf8(3).unwrap(), "cé");
    assert_eq!(h.position(), 5);
    // A failed decode leaves the cursor put.
    h.rewind();
    assert!(h.read_utf8(4).is_err()); // cuts é
    assert_eq!(h.position(), 0);

    // The cursor wrapper inherits the same methods.
    let mut cur = Heap::new().cursor();
    cur.write_utf8("xyz");
    cur.rewind();
    assert_eq!(cur.read_utf8(3).unwrap(), "xyz");
}

// -------------------------------------------------------------------------------------
// Trait-level with_capacity + headers/mode/kind + Serializable
// -------------------------------------------------------------------------------------

#[test]
fn trait_with_capacity_preallocates_any_source() {
    let heap = <Heap as IOBase>::with_capacity(4096);
    assert!(heap.is_empty());
    assert!(heap.capacity() >= 4096);
    // Works for the wrappers too (Default + reserve delegation).
    let cur = <IOCursor<Heap> as IOBase>::with_capacity(128);
    assert!(cur.capacity() >= 128);
}

#[test]
fn headers_metadata_lives_on_every_source() {
    let mut h = Heap::new();
    assert!(h.headers().is_empty());
    h.headers_mut()
        .insert("Content-Type", "application/octet-stream");
    assert_eq!(h.headers().content_type(), Some("application/octet-stream"));

    // The builder trio.
    let built = Heap::new().with_headers(yggdryl_core::headers::Headers::new().with("k", "v"));
    assert_eq!(built.headers().get("k"), Some("v"));

    // Wrappers delegate to the inner source's map.
    let mut cur = built.cursor();
    cur.headers_mut().insert("k", "v2");
    assert_eq!(cur.headers().get("k"), Some("v2"));
    let win = Heap::from_slice(b"abcd")
        .with_headers(yggdryl_core::headers::Headers::new().with("w", "1"))
        .window(1, 2)
        .unwrap();
    assert_eq!(win.headers().get("w"), Some("1"));

    // Metadata is not part of value equality.
    assert_eq!(
        Heap::from_slice(b"x").with_headers(yggdryl_core::headers::Headers::new().with("a", "1")),
        Heap::from_slice(b"x")
    );
}

#[test]
fn mode_and_kind_accessors() {
    use yggdryl_core::io::{IOKind, IOMode};
    let h = Heap::new();
    assert_eq!(h.mode(), IOMode::ReadWrite); // in-memory default
    assert_eq!(h.kind(), IOKind::Heap);

    let read_only = Heap::new().with_mode(IOMode::Read);
    assert_eq!(read_only.mode(), IOMode::Read);
    let mut m = Heap::new();
    m.set_mode(IOMode::Append);
    assert_eq!(m.mode(), IOMode::Append);

    // Wrappers delegate both.
    let cur = read_only.cursor();
    assert_eq!(cur.mode(), IOMode::Read);
    assert_eq!(cur.kind(), IOKind::Heap);
    let win = Heap::from_slice(b"ab")
        .with_mode(IOMode::Read)
        .window(0, 1)
        .unwrap();
    assert_eq!(win.mode(), IOMode::Read);
    assert_eq!(win.kind(), IOKind::Heap);
}

#[test]
fn heap_serializable_is_its_bytes() {
    use yggdryl_core::io::Serializable;
    let h = Heap::from_slice(b"payload").with_mode(yggdryl_core::io::IOMode::Read);
    let bytes = Serializable::serialize_bytes(&h);
    assert_eq!(bytes, b"payload");
    let back = <Heap as Serializable>::deserialize_bytes(&bytes).unwrap();
    assert_eq!(back, h); // equality is content-only, so the round-trip is exact
    assert_eq!(back.mode(), yggdryl_core::io::IOMode::ReadWrite); // metadata is not serialized
}

// -------------------------------------------------------------------------------------
// Review regressions: clamped-window write errors, overflow guard, hostile lengths
// -------------------------------------------------------------------------------------

#[test]
fn window_full_and_typed_writes_error_at_the_edge() {
    // The raw primitive clamps (documented); the FULL and TYPED writes must report the
    // shortfall instead of silently succeeding.
    let mut win = Heap::from_slice(b"hello world").window(6, 5).unwrap();
    assert!(matches!(
        win.pwrite_all(3, b"ABCDEF").unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
    assert!(win.pwrite_i32(3, -1).is_err()); // 4 bytes into 2 of room
    assert!(win.pwrite_i64(0, 1).is_err()); // 8 bytes into a 5-byte window
                                            // Within the window they still succeed.
    win.pwrite_all(0, b"WORLD").unwrap();
    assert_eq!(win.pread_vec(0, 5), b"WORLD");
}

#[test]
fn heap_write_overflowing_the_address_space_is_a_guided_error() {
    let mut h = Heap::new();
    // The primitive is a no-op (0 written)…
    assert_eq!(h.pwrite_byte_array(u64::MAX - 1, b"xy"), 0);
    // …and the full write reports the shortfall instead of wrapping or panicking.
    assert!(matches!(
        h.pwrite_all(u64::MAX - 1, b"xy").unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
    assert!(h.is_empty());
}

#[test]
fn hostile_lengths_never_preallocate() {
    // pread_vec / pread_into / pread_utf8 size to what is AVAILABLE, not the raw request, so
    // a hostile length cannot trigger a runaway allocation (usize::MAX would abort if it did).
    let src = Heap::from_slice(b"tiny");
    assert_eq!(src.pread_vec(0, usize::MAX), b"tiny");
    let mut scratch = Vec::new();
    assert_eq!(src.pread_into(1, usize::MAX, &mut scratch), 3);
    assert_eq!(&scratch, b"iny");
    assert_eq!(src.pread_utf8(0, usize::MAX).unwrap(), "tiny");
}

// -------------------------------------------------------------------------------------
// Capacity family: checked reserves, exact reserves, ensure, shrink, spare
// -------------------------------------------------------------------------------------

#[test]
fn spare_capacity_tracks_room_before_reallocation() {
    let mut h = Heap::with_capacity(64);
    let cap = h.capacity();
    assert_eq!(h.spare_capacity(), cap);
    h.pwrite_byte_array(0, &[0; 16]);
    assert_eq!(h.spare_capacity(), cap - 16);
    // Fixed windows have no spare room at all.
    let win = Heap::from_slice(b"abcd").window(0, 4).unwrap();
    assert_eq!(win.spare_capacity(), 0);
}

#[test]
fn try_reserve_is_checked_never_aborts() {
    let mut h = Heap::new();
    h.try_reserve(1024).unwrap();
    assert!(h.capacity() >= 1024);
    // A hostile size is a guided error, not a process abort.
    let err = h.try_reserve(u64::MAX).unwrap_err();
    assert!(matches!(err, IoError::CapacityOverflow { .. }));
    assert!(err.to_string().contains("reserve less"));
    // The heap is still fully usable afterwards.
    h.pwrite_byte_array(0, b"still alive");
    assert_eq!(h.pread_vec(0, 11), b"still alive");
    // The exact twin behaves identically.
    h.try_reserve_exact(2048).unwrap();
    assert!(h.capacity() >= 2048);
    assert!(h.try_reserve_exact(u64::MAX).is_err());
}

#[test]
fn ensure_capacity_targets_a_total_and_never_shrinks() {
    let mut h = Heap::new();
    h.ensure_capacity(4096);
    assert!(h.capacity() >= 4096);
    let cap = h.capacity();
    h.ensure_capacity(16); // already satisfied
    assert_eq!(h.capacity(), cap);
    // Checked form: recoverable on a hostile total.
    assert!(h.try_ensure_capacity(8192).is_ok());
    assert!(h.try_ensure_capacity(u64::MAX).is_err());
}

#[test]
fn shrink_releases_spare_capacity() {
    let mut h = Heap::with_capacity(4096);
    h.pwrite_byte_array(0, &[7; 32]);
    h.shrink_to(64);
    assert!(h.capacity() >= 32 && h.capacity() <= 4096);
    let after_to = h.capacity();
    h.shrink_to_fit();
    assert!(h.capacity() >= 32 && h.capacity() <= after_to);
    assert_eq!(h.pread_vec(0, 32), vec![7u8; 32]); // contents untouched
}

#[test]
fn capacity_family_delegates_through_the_cursor_wrapper() {
    let mut cur = Heap::new().cursor();
    cur.try_reserve(512).unwrap();
    assert!(cur.capacity() >= 512);
    assert!(cur.try_reserve(u64::MAX).is_err()); // the wrapper stays checked
    cur.shrink_to_fit();
    // A fixed window stays inert (deliberately — it may not grow its source).
    let mut win = Heap::from_slice(b"abcd").window(0, 4).unwrap();
    win.try_reserve(1024).unwrap(); // trait default: Ok, reserves nothing
    assert_eq!(win.capacity(), 4);
}

#[test]
fn auto_scaling_appends_amortize_growth() {
    // Appending chunk after chunk with NO reservation still costs only O(log n) reallocations
    // (Vec's amortized doubling through the single-write append path).
    let mut h = Heap::new();
    let chunk = [0xA5u8; 1024];
    for i in 0..64u64 {
        let end = h.byte_size();
        h.pwrite_byte_array(end, &chunk);
        assert_eq!(h.byte_size(), (i + 1) * 1024);
    }
    assert!(h.as_slice().iter().all(|&b| b == 0xA5));
    assert!(h.capacity() >= 64 * 1024);
}

// -------------------------------------------------------------------------------------
// Bulk numeric ops for the unsigned + floating widths (u16 / u32 / u64 / f32 / f64) —
// the Heap direct-contiguous overrides and the trait-default staged kernels must agree.
// -------------------------------------------------------------------------------------

#[test]
fn bulk_u16_u32_u64_array_roundtrip() {
    let mut h = Heap::new();
    h.pwrite_u16_array(0, &[1, 2, 0xFFFF]).unwrap();
    h.pwrite_u32_array(6, &[7, 0xDEAD_BEEF]).unwrap();
    h.pwrite_u64_array(14, &[0x0102_0304_0506_0708]).unwrap();
    let mut a = [0u16; 3];
    let mut b = [0u32; 2];
    let mut c = [0u64; 1];
    h.pread_u16_array(0, &mut a).unwrap();
    h.pread_u32_array(6, &mut b).unwrap();
    h.pread_u64_array(14, &mut c).unwrap();
    assert_eq!(a, [1, 2, 0xFFFF]);
    assert_eq!(b, [7, 0xDEAD_BEEF]);
    assert_eq!(c, [0x0102_0304_0506_0708]);
    // Little-endian on the wire: first u16 is 01 00.
    assert_eq!(&h.as_slice()[..2], &[0x01, 0x00]);
    // A short read is a guided EOF.
    let mut too_many = [0u32; 100];
    assert!(matches!(
        h.pread_u32_array(0, &mut too_many).unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
}

#[test]
fn bulk_float_array_roundtrip_and_repeat() {
    let mut h = Heap::new();
    h.pwrite_f32_array(0, &[1.5, -2.25, f32::INFINITY]).unwrap();
    h.pwrite_f64_array(12, &[core::f64::consts::PI, -0.0])
        .unwrap();
    let mut f = [0f32; 3];
    let mut d = [0f64; 2];
    h.pread_f32_array(0, &mut f).unwrap();
    h.pread_f64_array(12, &mut d).unwrap();
    assert_eq!(f, [1.5, -2.25, f32::INFINITY]);
    assert_eq!(d, [core::f64::consts::PI, -0.0]);

    // Repeat fill never materializes the array — 500 copies crosses the BULK_CHUNK boundary.
    let mut r = Heap::new();
    r.pwrite_f64_repeat(0, 2.5, 500).unwrap();
    let mut back = [0f64; 500];
    r.pread_f64_array(0, &mut back).unwrap();
    assert!(back.iter().all(|&x| x == 2.5));
    assert_eq!(r.byte_size(), 500 * 8);
}

#[test]
fn bulk_numeric_default_kernel_matches_heap_override() {
    // The staged trait default (used by composing sources) and the Heap contiguous override
    // must produce byte-identical output. Drive the default through a LocalIO-style path is
    // overkill here; instead compare a Heap write to a hand-rolled little-endian layout.
    let mut h = Heap::new();
    h.pwrite_u32_array(0, &[0x1122_3344, 0x5566_7788]).unwrap();
    assert_eq!(
        h.as_slice(),
        &[0x44, 0x33, 0x22, 0x11, 0x88, 0x77, 0x66, 0x55]
    );
    // Repeat of a wide value across the chunk boundary stays byte-exact.
    let mut r = Heap::new();
    r.pwrite_u16_repeat(0, 0xABCD, 300).unwrap();
    let mut back = [0u16; 300];
    r.pread_u16_array(0, &mut back).unwrap();
    assert!(back.iter().all(|&x| x == 0xABCD));
}

// -------------------------------------------------------------------------------------
// readline / readlines — cursor advances line by line, blank lines distinct from EOF
// -------------------------------------------------------------------------------------

#[test]
fn readline_strips_terminators_and_is_quote_aware() {
    // CRLF + LF terminators stripped; blank line kept (advances) but EOF does not advance.
    let mut cur = Heap::from_slice(b"alpha\r\nbeta\n\ngamma").cursor();
    assert_eq!(cur.readline().unwrap(), "alpha"); // \r\n stripped
    assert_eq!(cur.readline().unwrap(), "beta");
    assert_eq!(cur.readline().unwrap(), ""); // blank line — advanced
    assert_eq!(cur.readline().unwrap(), "gamma"); // last line, no terminator
    let at_eof = cur.position();
    assert_eq!(cur.readline().unwrap(), ""); // EOF
    assert_eq!(cur.position(), at_eof); // EOF does not advance (distinct from a blank line)

    // A newline inside a quoted field does not split the record; "" is an escaped quote.
    let mut csv = Heap::from_slice(b"a,\"x\ny\",b\n\"esc \"\" quote\"\nlast").cursor();
    assert_eq!(csv.readline().unwrap(), "a,\"x\ny\",b");
    assert_eq!(csv.readline().unwrap(), "\"esc \"\" quote\"");
    assert_eq!(csv.readline().unwrap(), "last");
}

#[test]
fn readlines_collects_every_line() {
    let mut cur = Heap::from_slice(b"a\n\nb\n").cursor();
    assert_eq!(cur.readlines().unwrap(), vec!["a", "", "b"]); // blank line kept, terminators gone
    assert_eq!(cur.position(), 5);
    // From a mid-stream position, readlines only sees the remainder (position 3 = start of "b").
    cur.set_position(3);
    assert_eq!(cur.readlines().unwrap(), vec!["b"]);
    // A line spanning more than one 256-byte scan chunk is still read whole.
    let long = vec![b'x'; 700];
    let mut long_cur = Heap::from_slice(&long).cursor();
    assert_eq!(long_cur.readline().unwrap().len(), 700);
}

// -------------------------------------------------------------------------------------
// content_length + truncate + in-place compression
// -------------------------------------------------------------------------------------

// -------------------------------------------------------------------------------------
// Cursor / Slice specialization over contiguous holders — zero-copy as_bytes + fast forwards
// -------------------------------------------------------------------------------------

#[test]
fn cursor_as_bytes_is_zero_copy_over_heap() {
    let heap = Heap::from_slice(b"contiguous");
    let base = heap.as_slice().as_ptr();
    let cur = heap.cursor(); // IOCursor<Heap>
    let view = cur.as_bytes().expect("a cursor over a Heap is contiguous");
    assert_eq!(view, b"contiguous");
    assert_eq!(
        view.as_ptr(),
        base,
        "cursor must borrow the heap's bytes, not copy"
    );
}

#[test]
fn slice_as_bytes_is_the_zero_copy_window() {
    let heap = Heap::from_slice(b"hello world");
    let base = heap.as_slice().as_ptr();
    let win = heap.window(6, 5).unwrap(); // IOSlice<Heap> over "world"
    let view = win.as_bytes().expect("a window over a Heap is contiguous");
    assert_eq!(view, b"world");
    // The window borrows the source's own bytes at offset 6 — no copy, no re-alloc.
    assert_eq!(view.as_ptr(), unsafe { base.add(6) });
}

#[test]
fn cursor_forwards_bulk_typed_ops_to_inner() {
    // The typed bulk arrays go straight to the wrapped Heap's contiguous override and round-trip.
    let mut cur = Heap::new().cursor();
    cur.pwrite_i32_array(0, &[10, -20, 30]).unwrap();
    cur.pwrite_i64_array(12, &[1 << 40]).unwrap();
    let mut a = [0i32; 3];
    let mut b = [0i64; 1];
    cur.pread_i32_array(0, &mut a).unwrap();
    cur.pread_i64_array(12, &mut b).unwrap();
    assert_eq!(a, [10, -20, 30]);
    assert_eq!(b, [1 << 40]);
    // And the wide unsigned/float arrays inherited from the trait still work through the cursor.
    cur.pwrite_f64_array(20, &[1.25, 2.5]).unwrap();
    let mut f = [0f64; 2];
    cur.pread_f64_array(20, &mut f).unwrap();
    assert_eq!(f, [1.25, 2.5]);
}

#[test]
fn content_length_prefers_cached_header() {
    let mut h = Heap::from_slice(b"abcde");
    assert_eq!(h.content_length(), 5); // no header — byte_size
    h.headers_mut().set_content_length(4096);
    assert_eq!(h.content_length(), 4096); // header short-circuits the probe
}

#[test]
fn truncate_grows_and_shrinks_with_header_sync() {
    let mut h = Heap::from_slice(b"hello world");
    h.headers_mut().set_content_length(11); // opt into size-header sync
    h.truncate(5).unwrap();
    assert_eq!(h.as_slice(), b"hello");
    assert_eq!(h.headers().content_length(), Some(5)); // synced down
    h.truncate(8).unwrap(); // grow zero-fills
    assert_eq!(h.as_slice(), b"hello\0\0\0");
    assert_eq!(h.headers().content_length(), Some(8));
}

// -------------------------------------------------------------------------------------
// All native types — scalar + array + repeat + cursor round-trips
// -------------------------------------------------------------------------------------

#[test]
fn scalar_native_types_round_trip() {
    let mut h = Heap::new();
    h.pwrite_i8(0, -5).unwrap();
    h.pwrite_i16(1, -300).unwrap();
    h.pwrite_u16(3, 60_000).unwrap();
    h.pwrite_u32(5, 4_000_000_000).unwrap();
    h.pwrite_u64(9, u64::MAX).unwrap();
    h.pwrite_i128(17, i128::MIN).unwrap();
    h.pwrite_u128(33, u128::MAX).unwrap();
    h.pwrite_f32(49, 1.5).unwrap();
    h.pwrite_f64(53, core::f64::consts::PI).unwrap();
    assert_eq!(h.pread_i8(0).unwrap(), -5);
    assert_eq!(h.pread_i16(1).unwrap(), -300);
    assert_eq!(h.pread_u16(3).unwrap(), 60_000);
    assert_eq!(h.pread_u32(5).unwrap(), 4_000_000_000);
    assert_eq!(h.pread_u64(9).unwrap(), u64::MAX);
    assert_eq!(h.pread_i128(17).unwrap(), i128::MIN);
    assert_eq!(h.pread_u128(33).unwrap(), u128::MAX);
    assert_eq!(h.pread_f32(49).unwrap(), 1.5);
    assert_eq!(h.pread_f64(53).unwrap(), core::f64::consts::PI);
    // The u8 scalar mirrors the byte accessor.
    h.pwrite_u8(61, 200).unwrap();
    assert_eq!(h.pread_u8(61).unwrap(), 200);
    assert_eq!(h.pread_byte(61).unwrap(), 200);
    // A short source errors with a guided EOF, never a silent partial value.
    assert!(matches!(
        Heap::from_slice(b"ab").pread_i128(0).unwrap_err(),
        IoError::UnexpectedEof { .. }
    ));
}

#[test]
fn native_type_arrays_and_cursor_round_trip() {
    let mut h = Heap::new();
    h.pwrite_i8_array(0, &[-1, 2, -3]).unwrap();
    h.pwrite_i16_array(3, &[-1000, 1000]).unwrap();
    h.pwrite_i128_array(7, &[i128::MIN, i128::MAX]).unwrap();
    h.pwrite_u128_array(39, &[0, u128::MAX]).unwrap();
    let mut a = [0i8; 3];
    let mut b = [0i16; 2];
    let mut c = [0i128; 2];
    let mut d = [0u128; 2];
    h.pread_i8_array(0, &mut a).unwrap();
    h.pread_i16_array(3, &mut b).unwrap();
    h.pread_i128_array(7, &mut c).unwrap();
    h.pread_u128_array(39, &mut d).unwrap();
    assert_eq!(a, [-1, 2, -3]);
    assert_eq!(b, [-1000, 1000]);
    assert_eq!(c, [i128::MIN, i128::MAX]);
    assert_eq!(d, [0, u128::MAX]);

    // Repeat fill of a 16-byte type crosses the chunk boundary without materializing the array.
    let mut r = Heap::new();
    r.pwrite_u128_repeat(0, 42, 100).unwrap();
    let mut back = [0u128; 100];
    r.pread_u128_array(0, &mut back).unwrap();
    assert!(back.iter().all(|&x| x == 42));

    // The cursor streams every native width, advancing by the right byte count.
    let mut cur = Heap::new().cursor();
    cur.write_i8(-7).unwrap();
    cur.write_u16(65_535).unwrap();
    cur.write_i128(i128::MIN).unwrap();
    cur.write_f64(2.5).unwrap();
    assert_eq!(cur.position(), 1 + 2 + 16 + 8);
    cur.rewind();
    assert_eq!(cur.read_i8().unwrap(), -7);
    assert_eq!(cur.read_u16().unwrap(), 65_535);
    assert_eq!(cur.read_i128().unwrap(), i128::MIN);
    assert_eq!(cur.read_f64().unwrap(), 2.5);
}

#[test]
fn move_into_relocates_and_empties_source() {
    let mut src = Heap::from_slice(b"payload to move");
    let mut dst = Heap::from_slice(b"old dst content that is longer");
    assert_eq!(src.move_into(&mut dst).unwrap(), 15);
    assert_eq!(dst.as_slice(), b"payload to move"); // dst replaced, old tail dropped
    assert_eq!(src.byte_size(), 0); // source emptied

    // A large move crosses the 64 KiB chunk boundary and relocates exactly.
    let big: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    let mut s2 = Heap::from_vec(big.clone());
    let mut d2 = Heap::new();
    assert_eq!(s2.move_into(&mut d2).unwrap(), 200_000);
    assert_eq!(d2.as_slice(), &big[..]);
    assert_eq!(s2.byte_size(), 0);

    // Two distinct anonymous heaps share mem://heap yet still move (not treated as same source).
    let mut a = Heap::from_slice(b"abc");
    let mut b = Heap::new();
    assert_eq!(a.move_into(&mut b).unwrap(), 3);
    assert_eq!(b.as_slice(), b"abc");
}

#[test]
fn copy_from_and_pwrite_from_cover_both_branches() {
    // copy_from: a longer source overwrites a shorter dst and drops the old tail (zero-copy path).
    let src = Heap::from_slice(b"the full replacement payload");
    let mut dst = Heap::from_slice(b"short");
    assert_eq!(dst.copy_from(&src).unwrap(), 28);
    assert_eq!(dst.as_slice(), b"the full replacement payload");

    // copy_from from a non-contiguous source (a directory-less LocalIO read) hits the pread_vec
    // fallback — exercised in the local suite; here assert the contiguous count/return.
    let mut small = Heap::from_slice(b"xxxxxxxx");
    assert_eq!(small.copy_from(&Heap::from_slice(b"ab")).unwrap(), 2);
    assert_eq!(small.as_slice(), b"ab");

    // pwrite_from: a positioned mid-offset copy lands at the right place; a request past src's end
    // returns a short transferred count (contiguous branch).
    let mut sink = Heap::new();
    let source = Heap::from_slice(b"0123456789");
    assert_eq!(sink.pwrite_from(4, &source, 2, 3).unwrap(), 3); // copies "234" to offset 4
    assert_eq!(sink.pread_vec(4, 3), b"234");
    assert_eq!(sink.pwrite_from(0, &source, 8, 100).unwrap(), 2); // only "89" remain from offset 8
    assert_eq!(sink.pread_vec(0, 2), b"89");
}

#[test]
fn move_into_of_an_empty_source_empties_the_destination() {
    // An empty move drops the destination's prior content and returns 0 (documented behavior).
    let mut src = Heap::new();
    let mut dst = Heap::from_slice(b"existing content");
    assert_eq!(src.move_into(&mut dst).unwrap(), 0);
    assert_eq!(dst.byte_size(), 0);
    assert_eq!(src.byte_size(), 0);
}

// -------------------------------------------------------------------------------------
// DataTypeId + element dtype + resize_dtype + aggregations (Aggregate over any IOBase)
// -------------------------------------------------------------------------------------

#[test]
fn dtype_and_resize_widen_and_shrink() {
    use yggdryl_core::datatype_id::DataTypeId;
    let mut h = Heap::new();
    h.pwrite_i64_array(0, &[1, -2, 3, 1_000_000]).unwrap();
    h.set_dtype(DataTypeId::I64);
    assert_eq!(h.dtype(), DataTypeId::I64);
    assert_eq!(h.element_count(), 4);

    // The copy form leaves the source untouched.
    let narrowed = h.resize_dtype(DataTypeId::I32).unwrap();
    assert_eq!(narrowed.byte_size(), 16);
    assert_eq!(h.byte_size(), 32); // source unchanged

    // Shrink i64 -> i32 in place preserving in-range values.
    assert_eq!(h.resize_dtype_in_place(DataTypeId::I32).unwrap(), 4);
    assert_eq!(h.byte_size(), 16);
    let mut back = [0i32; 4];
    h.pread_i32_array(0, &mut back).unwrap();
    assert_eq!(back, [1, -2, 3, 1_000_000]);

    // Widen i32 -> f64 in place.
    h.resize_dtype_in_place(DataTypeId::F64).unwrap();
    let mut fs = [0f64; 4];
    h.pread_f64_array(0, &mut fs).unwrap();
    assert_eq!(fs, [1.0, -2.0, 3.0, 1_000_000.0]);

    // A narrowing conversion saturates rather than wrapping.
    let mut big = Heap::new();
    big.pwrite_i64(0, 5_000_000_000).unwrap(); // > i32::MAX
    big.set_dtype(DataTypeId::I64);
    big.resize_dtype_in_place(DataTypeId::I32).unwrap();
    assert_eq!(big.pread_i32(0).unwrap(), i32::MAX); // saturated, not wrapped
}

#[test]
fn aggregations_over_a_heap() {
    use yggdryl_core::io::memory::Aggregate;
    let mut h = Heap::new();
    h.pwrite_i64_array(0, &[4, 8, 15, 16, 23, 42]).unwrap();
    assert_eq!(h.sum_i64(0, 6).unwrap(), 108);
    assert_eq!(h.min_i64(0, 6).unwrap(), Some(4));
    assert_eq!(h.max_i64(0, 6).unwrap(), Some(42));
    assert_eq!(h.mean_i64(0, 6).unwrap(), Some(18.0));
    assert_eq!(h.first_i64(0, 6).unwrap(), Some(4));
    assert_eq!(h.last_i64(0, 6).unwrap(), Some(42));
    assert_eq!(h.count_ge_i64(0, 6, 16).unwrap(), 3);
    // std of [4,8,15,16,23,42]: population std dev sqrt(910/6) ~ 12.315
    let std = h.std_i64(0, 6).unwrap().unwrap();
    assert!((std - 12.315).abs() < 0.01, "std = {std}");

    // Float aggregation across the streaming chunk boundary (> AGG_CHUNK).
    let data: Vec<f64> = (0..5000).map(|i| i as f64).collect();
    let mut f = Heap::new();
    f.pwrite_f64_array(0, &data).unwrap();
    assert_eq!(f.sum_f64(0, 5000).unwrap(), (0..5000).sum::<i64>() as f64);
    assert_eq!(f.max_f64(0, 5000).unwrap(), Some(4999.0));
    assert_eq!(f.mean_f64(0, 5000).unwrap(), Some(2499.5));
    // Empty range -> None / 0.
    assert_eq!(h.min_i64(0, 0).unwrap(), None);
    assert_eq!(h.sum_i64(0, 0).unwrap(), 0);
}

#[test]
fn mask_filter_selects_elements_by_bitmask() {
    use yggdryl_core::datatype_id::DataTypeId;
    let mut data = Heap::new();
    data.pwrite_i64_array(0, &[10, 20, 30, 40, 50]).unwrap();
    data.set_dtype(DataTypeId::I64);
    // Keep elements 0, 2, 4 (bits 0b0001_0101).
    let mask = Heap::from_slice(&[0b0001_0101]);

    // Copy form leaves the source untouched.
    let kept = data.mask_filter(&mask).unwrap();
    assert_eq!(kept.byte_size(), 24); // 3 kept x 8 bytes
    let mut out = [0i64; 3];
    kept.pread_i64_array(0, &mut out).unwrap();
    assert_eq!(out, [10, 30, 50]);
    assert_eq!(data.byte_size(), 40); // source unchanged

    // In-place form compacts the source itself.
    assert_eq!(data.mask_filter_in_place(&mask).unwrap(), 3);
    assert_eq!(data.byte_size(), 24);
    data.pread_i64_array(0, &mut out).unwrap();
    assert_eq!(out, [10, 30, 50]);
    assert_eq!(data.element_count(), 3); // dtype preserved -> count updated

    // No element type -> guided error.
    let mut raw = Heap::from_slice(b"raw");
    assert!(raw.mask_filter_in_place(&mask).is_err());
}

#[test]
fn heap_cursor_and_slice_named_aliases() {
    // HeapCursor / HeapSlice are the named per-type instantiations of the shared cursor/window.
    let mut cur: HeapCursor = HeapCursor::new(Heap::from_slice(b"heap bytes"));
    let mut head = [0u8; 4];
    assert_eq!(cur.read(&mut head), 4);
    assert_eq!(&head, b"heap");
    cur.seek(Whence::Start, 5).unwrap();
    assert_eq!(cur.read(&mut head), 4);
    assert_eq!(&head, b"byte");

    let win: HeapSlice = HeapSlice::new(Heap::from_slice(b"heap bytes"), 5, 5).unwrap();
    assert_eq!(win.byte_size(), 5);
    assert_eq!(win.pread_vec(0, 5), b"bytes");
    // The alias is exactly the generic instantiation.
    let _: IOCursor<Heap> = cur;
    let _: IOSlice<Heap> = win;
}
