//! Tests for the [`Io`] trait and its in-memory [`Vec`] leaf implementation.

use yggdryl_core::{Io, IoError, Whence};

#[test]
fn pread_from_each_whence_origin() {
    let io = vec![10u8, 20, 30, 40, 50];
    // Start counts forward from the front.
    assert_eq!(io.pread(1, Whence::Start, 2).unwrap(), vec![20, 30]);
    // End counts back from the end.
    assert_eq!(io.pread(2, Whence::End, 5).unwrap(), vec![40, 50]);
    // Current anchors at the cursor, which is `0` for a cursorless Vec.
    assert_eq!(io.pread(1, Whence::Current, 2).unwrap(), vec![20, 30]);
}

#[test]
fn pread_clamps_at_the_end_and_errors_past_it() {
    let io = vec![1u8, 2, 3];
    // More than is available returns only what's there.
    assert_eq!(io.pread(1, Whence::Start, 10).unwrap(), vec![2, 3]);
    // Reading exactly at the end yields nothing.
    assert!(io.pread(3, Whence::Start, 4).unwrap().is_empty());
    // Starting past the end errors.
    assert_eq!(io.pread(4, Whence::Start, 1), Err(IoError::OutOfBounds));
    // An End offset larger than the source errors.
    assert_eq!(io.pread(4, Whence::End, 1), Err(IoError::OutOfBounds));
}

#[test]
fn pwrite_overwrites_and_extends() {
    let mut io = vec![1u8, 2, 3];
    // Overwrite in place.
    assert_eq!(io.pwrite(1, Whence::Start, &[20, 30]).unwrap(), 2);
    assert_eq!(io, vec![1, 20, 30]);
    // Overwrite the tail and extend past the end in one write.
    assert_eq!(io.pwrite(2, Whence::Start, &[31, 4, 5]).unwrap(), 3);
    assert_eq!(io, vec![1, 20, 31, 4, 5]);
    // An End offset of `0` appends at the very end.
    assert_eq!(io.pwrite(0, Whence::End, &[6]).unwrap(), 1);
    assert_eq!(io, vec![1, 20, 31, 4, 5, 6]);
    // Starting past the end errors, leaving the source untouched.
    assert_eq!(io.pwrite(9, Whence::Start, &[7]), Err(IoError::OutOfBounds));
    assert_eq!(io, vec![1, 20, 31, 4, 5, 6]);
}

#[test]
fn len_reports_the_element_count() {
    let io = vec![0u16; 4];
    assert_eq!(Io::len(&io).unwrap(), 4);
}
