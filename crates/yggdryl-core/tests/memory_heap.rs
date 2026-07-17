//! Functional tests for the in-heap [`Heap`](yggdryl_core::memory::Heap) source and the byte
//! I/O trait surface it implements — the positioned primitives and typed accessors on
//! [`IOBase`](yggdryl_core::memory::IOBase), the cursor stream on
//! [`IOCursor`](yggdryl_core::memory::IOCursor), bounded [`IOSlice`](yggdryl_core::memory::IOSlice)
//! windows, and [`Whence`](yggdryl_core::memory::Whence) seeks. Doctests cover the happy paths;
//! this file hammers the edges (EOF, bit addressing, capacity reuse, content equality).

use yggdryl_core::memory::{Heap, IOBase, IOCursor, IOSlice, IoError, Whence};
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
fn uri_default_empty_and_settable() {
    assert_eq!(Heap::new().uri(), Uri::default());
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
