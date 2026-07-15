//! Edge-case tests for the byte-I/O family — [`Bytes`] against the [`IOBase`] / [`IOCursor`]
//! / [`IOSlice`] contracts: positioned vs cursor read/write, short reads and EOF, seeking
//! with every [`Whence`] (including past the end and before the start), write-grow with
//! zero-filled gaps, and the zero-copy-slice / copy-on-write invariant that a write to one
//! side never disturbs a slice sharing its allocation.

use yggdryl_core::io::{Bytes, IOBase, IOCursor, IOSlice, IoError, Whence};

// -------------------------------------------------------------------------------------
// IOBase — positioned read
// -------------------------------------------------------------------------------------

#[test]
fn pread_full_partial_and_past_end() {
    let data = Bytes::from_slice(b"hello world");
    assert_eq!(data.len(), 11);
    assert!(!data.is_empty());

    let mut buf = [0u8; 5];
    assert_eq!(data.pread(0, &mut buf), 5);
    assert_eq!(&buf, b"hello");

    // A read that runs into the end returns a short count.
    let mut tail = [0u8; 10];
    assert_eq!(data.pread(6, &mut tail), 5); // only "world" remains
    assert_eq!(&tail[..5], b"world");

    // At or past the end: nothing.
    assert_eq!(data.pread(11, &mut buf), 0);
    assert_eq!(data.pread(999, &mut buf), 0);
    // An empty destination reads nothing.
    assert_eq!(data.pread(0, &mut []), 0);
}

#[test]
fn pread_exact_fills_or_errors_with_shortfall() {
    let data = Bytes::from_slice(b"hello");
    let mut buf = [0u8; 3];
    data.pread_exact(1, &mut buf).unwrap();
    assert_eq!(&buf, b"ell");

    // Only 2 bytes remain from offset 3 but 5 were requested.
    let err = data.pread_exact(3, &mut [0u8; 5]).unwrap_err();
    assert_eq!(
        err,
        IoError::UnexpectedEof {
            offset: 5,
            requested: 5,
            available: 2
        }
    );
    assert!(err.to_string().contains("offset 5"));
    assert!(err.to_string().contains("only 2 remain"));
}

#[test]
fn pread_vec_clamps_to_available() {
    let data = Bytes::from_slice(b"hello world");
    assert_eq!(data.pread_vec(6, 100), b"world");
    assert!(data.pread_vec(11, 10).is_empty());
}

// -------------------------------------------------------------------------------------
// IOBase — positioned write (grow + zero-fill gaps)
// -------------------------------------------------------------------------------------

#[test]
fn pwrite_overwrites_in_place() {
    let mut data = Bytes::from_slice(b"hello world");
    assert_eq!(data.pwrite(6, b"earth"), 5);
    assert_eq!(data.as_slice(), b"hello earth");
    assert_eq!(data.len(), 11); // no growth — same length
}

#[test]
fn pwrite_extends_and_zero_fills_the_gap() {
    let mut data = Bytes::from_slice(b"abc");
    // Write past the end: the gap [3, 5) is zero-filled.
    assert_eq!(data.pwrite(5, b"Z"), 1);
    assert_eq!(data.as_slice(), b"abc\0\0Z");
    assert_eq!(data.len(), 6);

    // A straddling write that partly overlaps and partly extends.
    assert_eq!(data.pwrite(4, b"XYW"), 3);
    assert_eq!(data.as_slice(), b"abc\0XYW");
}

#[test]
fn pwrite_empty_is_a_no_op() {
    let mut data = Bytes::from_slice(b"abc");
    assert_eq!(data.pwrite(1, b""), 0);
    assert_eq!(data.as_slice(), b"abc");
    assert_eq!(data.pwrite(99, b""), 0); // even far past the end
    assert_eq!(data.len(), 3);
}

// -------------------------------------------------------------------------------------
// IOCursor — read/write advance, seek with whence
// -------------------------------------------------------------------------------------

#[test]
fn cursor_read_write_advance_the_position() {
    let mut data = Bytes::new();
    assert_eq!(data.position(), 0);
    assert_eq!(data.write(b"hello"), 5);
    assert_eq!(data.write(b" world"), 6);
    assert_eq!(data.position(), 11);
    assert_eq!(data.as_slice(), b"hello world");

    data.rewind();
    assert_eq!(data.position(), 0);
    let mut buf = [0u8; 5];
    assert_eq!(data.read(&mut buf), 5);
    assert_eq!(&buf, b"hello");
    assert_eq!(data.position(), 5);
}

#[test]
fn seek_from_every_whence() {
    let mut data = Bytes::from_slice(b"hello world"); // len 11
    assert_eq!(data.seek(Whence::Start, 6).unwrap(), 6);
    assert_eq!(data.seek(Whence::Current, 2).unwrap(), 8);
    assert_eq!(data.seek(Whence::Current, -3).unwrap(), 5);
    assert_eq!(data.seek(Whence::End, 0).unwrap(), 11);
    assert_eq!(data.seek(Whence::End, -5).unwrap(), 6);
    assert_eq!(data.read_to_end(), b"world");
}

#[test]
fn seek_before_the_start_is_a_guided_error() {
    let mut data = Bytes::from_slice(b"hello");
    let err = data.seek(Whence::Start, -1).unwrap_err();
    assert!(matches!(err, IoError::InvalidSeek { .. }));
    assert!(err.to_string().contains("before the start"));
    // Current and End can also underflow.
    assert!(data.seek(Whence::Current, -100).is_err());
    assert!(data.seek(Whence::End, -6).is_err());
    // A failed seek leaves the cursor where it was.
    assert_eq!(data.position(), 0);
}

#[test]
fn seek_past_the_end_is_allowed_then_reads_eof_and_writes_fill() {
    let mut data = Bytes::from_slice(b"abc");
    assert_eq!(data.seek(Whence::End, 3).unwrap(), 6); // past the end
    assert_eq!(data.position(), 6);
    // Reading past the end yields nothing.
    let mut buf = [0u8; 4];
    assert_eq!(data.read(&mut buf), 0);
    // Writing at the past-end cursor zero-fills the gap.
    assert_eq!(data.write(b"Z"), 1);
    assert_eq!(data.as_slice(), b"abc\0\0\0Z");
    assert_eq!(data.position(), 7);
}

#[test]
fn read_exact_errors_leave_the_cursor_put() {
    let mut data = Bytes::from_slice(b"hello");
    data.seek(Whence::Start, 3).unwrap();
    // Wants 5 but only 2 remain: error, cursor unchanged at 3.
    assert!(data.read_exact(&mut [0u8; 5]).is_err());
    assert_eq!(data.position(), 3);
    // A fitting read_exact advances.
    let mut buf = [0u8; 2];
    data.read_exact(&mut buf).unwrap();
    assert_eq!(&buf, b"lo");
    assert_eq!(data.position(), 5);
}

#[test]
fn write_all_advances_the_cursor() {
    let mut data = Bytes::new();
    data.write_all(b"hello").unwrap();
    data.write_all(b" world").unwrap();
    assert_eq!(data.position(), 11);
    assert_eq!(data.as_slice(), b"hello world");
}

#[test]
fn read_to_end_from_the_middle() {
    let mut data = Bytes::from_slice(b"hello world");
    data.seek(Whence::Start, 6).unwrap();
    assert_eq!(data.read_to_end(), b"world");
    assert_eq!(data.position(), 11);
    assert!(data.read_to_end().is_empty()); // already at the end
}

// -------------------------------------------------------------------------------------
// IOSlice — bounded windows, bounds errors, re-slicing
// -------------------------------------------------------------------------------------

#[test]
fn slice_is_addressed_from_its_own_zero() {
    let data = Bytes::from_slice(b"hello world");
    let world = data.slice(6, 5).unwrap();
    assert_eq!(world.len(), 5);
    assert_eq!(world.as_slice(), b"world");
    let mut buf = [0u8; 3];
    assert_eq!(world.pread(0, &mut buf), 3);
    assert_eq!(&buf, b"wor");

    // The empty window and the whole window are both valid.
    assert_eq!(data.slice(11, 0).unwrap().len(), 0);
    assert_eq!(data.slice(0, 11).unwrap().as_slice(), b"hello world");
}

#[test]
fn slice_out_of_bounds_is_a_guided_error() {
    let data = Bytes::from_slice(b"hello"); // len 5
    assert_eq!(
        data.slice(3, 5).unwrap_err(),
        IoError::SliceOutOfBounds {
            offset: 3,
            len: 5,
            available: 5
        }
    );
    assert!(data.slice(6, 0).is_err()); // offset past the end
    assert!(data
        .slice(3, 5)
        .unwrap_err()
        .to_string()
        .contains("past the end"));
}

#[test]
fn slice_of_a_slice_composes() {
    let data = Bytes::from_slice(b"hello world");
    let world = data.slice(6, 5).unwrap();
    let orl = world.slice(1, 3).unwrap();
    assert_eq!(orl.as_slice(), b"orl");
}

// -------------------------------------------------------------------------------------
// Zero-copy slice + copy-on-write independence (the headline optimization)
// -------------------------------------------------------------------------------------

#[test]
fn writing_to_a_slice_does_not_disturb_the_parent() {
    let parent = Bytes::from_slice(b"hello world");
    let mut window = parent.slice(0, 5).unwrap(); // shares the parent allocation
                                                  // Writing triggers copy-on-write in the window only.
    window.pwrite(0, b"HELLO");
    assert_eq!(window.as_slice(), b"HELLO");
    assert_eq!(parent.as_slice(), b"hello world"); // parent untouched
}

#[test]
fn writing_to_the_parent_does_not_disturb_a_slice() {
    let mut parent = Bytes::from_slice(b"hello world");
    let window = parent.slice(6, 5).unwrap(); // "world", shares the allocation
    parent.pwrite(6, b"earth");
    assert_eq!(parent.as_slice(), b"hello earth");
    assert_eq!(window.as_slice(), b"world"); // slice still sees the old bytes
}

#[test]
fn a_clone_is_an_independent_value_under_writes() {
    let original = Bytes::from_slice(b"hello");
    let mut dup = original.clone(); // shares the Arc until a write forces a copy
    dup.pwrite(0, b"HELLO");
    assert_eq!(dup.as_slice(), b"HELLO");
    assert_eq!(original.as_slice(), b"hello");
}

// -------------------------------------------------------------------------------------
// Value semantics (content equality; cursor is not part of the value)
// -------------------------------------------------------------------------------------

#[test]
fn equality_is_by_content_and_ignores_the_cursor() {
    let a = Bytes::from_slice(b"hello");
    let mut b = Bytes::from_vec(b"hello".to_vec());
    assert_eq!(a, b);
    // Moving b's cursor does not change the value.
    b.seek(Whence::Start, 3).unwrap();
    assert_eq!(a, b);
    assert_ne!(a, Bytes::from_slice(b"world"));
    assert_ne!(a, Bytes::from_slice(b"hell")); // prefix is not equal
}

#[test]
fn from_vec_and_from_slice_and_with_capacity() {
    assert_eq!(Bytes::from_vec(vec![1, 2, 3]).to_vec(), vec![1, 2, 3]);
    assert_eq!(Bytes::from_slice(&[1, 2, 3]).as_slice(), &[1, 2, 3]);

    // with_capacity starts empty but can be filled.
    let mut buf = Bytes::with_capacity(64);
    assert_eq!(buf.len(), 0);
    buf.write(b"data");
    assert_eq!(buf.as_slice(), b"data");
}
