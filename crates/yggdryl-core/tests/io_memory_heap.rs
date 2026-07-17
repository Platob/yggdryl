//! Functional tests for the in-heap [`Heap`](yggdryl_core::io::memory::Heap) source and the byte
//! I/O trait surface it implements — the positioned primitives and typed accessors on
//! [`IOBase`](yggdryl_core::io::memory::IOBase), the cursor stream on
//! [`IOCursor`](yggdryl_core::io::memory::IOCursor), bounded [`IOSlice`](yggdryl_core::io::memory::IOSlice)
//! windows, and [`Whence`](yggdryl_core::io::memory::Whence) seeks. Doctests cover the happy paths;
//! this file hammers the edges (EOF, bit addressing, capacity reuse, content equality).

use yggdryl_core::io::memory::{Heap, IOBase, IOCursor, IOSlice, IoError, Whence};
use yggdryl_core::io::uri::Uri;

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
fn uri_default_is_mem_heap_and_settable() {
    // An unaddressed heap reports the stable synthetic mem:// address (deterministic).
    assert_eq!(Heap::new().uri().to_string(), "mem://heap");
    assert_eq!(Heap::new().uri().scheme(), Some("mem"));
    assert_eq!(Heap::new().uri().host(), Some("heap"));
    let named = Heap::from_slice(b"x").with_uri(Uri::parse_str("mem://scratch/a").unwrap());
    assert_eq!(named.uri().host(), Some("scratch"));

    let mut h = Heap::new();
    h.set_uri(Uri::parse_str("mem://b/1").unwrap());
    assert_eq!(h.uri().host(), Some("b"));

    // The address is metadata, not part of value equality (like the cursor).
    assert_eq!(named, Heap::from_slice(b"x"));
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
    let cur = Heap::from_slice(b"x")
        .with_uri(Uri::parse_str("mem://h/1").unwrap())
        .cursor();
    assert_eq!(cur.uri().host(), Some("h"));
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
    let built = Heap::new().with_headers(yggdryl_core::io::Headers::new().with("k", "v"));
    assert_eq!(built.headers().get("k"), Some("v"));

    // Wrappers delegate to the inner source's map.
    let mut cur = built.cursor();
    cur.headers_mut().insert("k", "v2");
    assert_eq!(cur.headers().get("k"), Some("v2"));
    let win = Heap::from_slice(b"abcd")
        .with_headers(yggdryl_core::io::Headers::new().with("w", "1"))
        .window(1, 2)
        .unwrap();
    assert_eq!(win.headers().get("w"), Some("1"));

    // Metadata is not part of value equality.
    assert_eq!(
        Heap::from_slice(b"x").with_headers(yggdryl_core::io::Headers::new().with("a", "1")),
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
    let h = Heap::from_slice(b"payload").with_uri(Uri::parse_str("mem://x/1").unwrap());
    let bytes = Serializable::serialize_bytes(&h);
    assert_eq!(bytes, b"payload");
    let back = <Heap as Serializable>::deserialize_bytes(&bytes).unwrap();
    assert_eq!(back, h); // equality is content-only, so the round-trip is exact
    assert_eq!(back.uri().to_string(), "mem://heap"); // metadata is not serialized
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
