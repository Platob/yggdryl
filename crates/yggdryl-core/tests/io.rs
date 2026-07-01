//! Tests for the [`Io`] trait and its in-memory [`Vec`] leaf implementation.

use yggdryl_core::{Io, IoError, Whence};

#[test]
fn pread_reads_one_element_from_each_whence_origin() {
    let io = vec![10u8, 20, 30, 40, 50];
    // Start counts forward from the front.
    assert_eq!(io.pread(1, Whence::Start).unwrap(), 20);
    // End counts back from the end (offset `1` → the last element).
    assert_eq!(io.pread(1, Whence::End).unwrap(), 50);
    // Current anchors at the cursor, which is `0` for a cursorless Vec.
    assert_eq!(io.pread(1, Whence::Current).unwrap(), 20);
}

#[test]
fn pread_errors_at_and_past_the_end() {
    let io = vec![1u8, 2, 3];
    // Reading exactly at the end — where no element lives — errors.
    assert_eq!(io.pread(3, Whence::Start), Err(IoError::OutOfBounds));
    assert_eq!(io.pread(0, Whence::End), Err(IoError::OutOfBounds));
    // Reading past the end errors.
    assert_eq!(io.pread(4, Whence::Start), Err(IoError::OutOfBounds));
    // An End offset larger than the source errors.
    assert_eq!(io.pread(4, Whence::End), Err(IoError::OutOfBounds));
}

#[test]
fn pwrite_overwrites_and_appends() {
    let mut io = vec![1u8, 2, 3];
    // Overwrite in place.
    io.pwrite(1, Whence::Start, 20).unwrap();
    assert_eq!(io, vec![1, 20, 3]);
    // Appending at the very end via `Start == len`.
    io.pwrite(3, Whence::Start, 4).unwrap();
    assert_eq!(io, vec![1, 20, 3, 4]);
    // An End offset of `0` appends at the very end.
    io.pwrite(0, Whence::End, 5).unwrap();
    assert_eq!(io, vec![1, 20, 3, 4, 5]);
    // An End offset of `1` overwrites the last element.
    io.pwrite(1, Whence::End, 50).unwrap();
    assert_eq!(io, vec![1, 20, 3, 4, 50]);
    // Writing past the end errors, leaving the source untouched.
    assert_eq!(io.pwrite(9, Whence::Start, 7), Err(IoError::OutOfBounds));
    assert_eq!(io, vec![1, 20, 3, 4, 50]);
}

#[test]
fn seek_resolves_the_target_from_each_whence_origin() {
    let mut io = vec![1u8, 2, 3, 4, 5];
    // Start / Current count forward; the cursor is `0` for a cursorless Vec.
    assert_eq!(io.seek(2, Whence::Start).unwrap(), 2);
    assert_eq!(io.seek(2, Whence::Current).unwrap(), 2);
    // End counts back; offset `0` lands on the end (past the last element).
    assert_eq!(io.seek(1, Whence::End).unwrap(), 4);
    assert_eq!(io.seek(0, Whence::End).unwrap(), 5);
    // A cursorless Vec does not retain the move.
    assert_eq!(Io::position(&io).unwrap(), 0);
    // Seeking past the end errors.
    assert_eq!(io.seek(6, Whence::Start), Err(IoError::OutOfBounds));
    assert_eq!(io.seek(6, Whence::End), Err(IoError::OutOfBounds));
}

#[test]
fn len_reports_the_element_count() {
    let io = vec![0u16; 4];
    assert_eq!(Io::len(&io).unwrap(), 4);
}
