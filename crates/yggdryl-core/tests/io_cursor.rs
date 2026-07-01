//! Tests for the [`IoCursor`] stateful cursor.

use yggdryl_core::{Io, IoCursor, IoError, Whence};

#[test]
fn cursor_starts_at_zero_and_delegates_len() {
    let io = IoCursor::new(vec![1u8, 2, 3, 4]);
    assert_eq!(io.position().unwrap(), 0);
    assert_eq!(io.len().unwrap(), 4);
}

#[test]
fn seek_retains_the_move_and_current_addresses_it() {
    let mut io = IoCursor::new(vec![10u8, 20, 30, 40, 50]);
    assert_eq!(io.seek(2, Whence::Start).unwrap(), 2);
    assert_eq!(io.position().unwrap(), 2);
    // A read with Current is anchored at the cursor...
    assert_eq!(io.pread_one(0, Whence::Current).unwrap(), 30);
    assert_eq!(io.pread_one(1, Whence::Current).unwrap(), 40);
    // ...but does not move it (reads are positional).
    assert_eq!(io.position().unwrap(), 2);
    // Seeking relative to Current advances from the cursor.
    assert_eq!(io.seek(1, Whence::Current).unwrap(), 3);
    assert_eq!(io.pread_one(0, Whence::Current).unwrap(), 40);
}

#[test]
fn writes_and_reads_delegate_to_the_inner() {
    let mut io = IoCursor::new(vec![1u8, 2, 3]);
    io.seek(1, Whence::Start).unwrap();
    io.pwrite_one(0, Whence::Current, 20).unwrap();
    io.pwrite_array(1, Whence::Current, &[30, 4]).unwrap(); // overwrite + extend
    assert_eq!(
        io.pread_array(0, Whence::Start, 4).unwrap(),
        vec![1, 20, 30, 4]
    );
    // The inner Vec carries the changes.
    assert_eq!(io.into_inner(), vec![1, 20, 30, 4]);
}

#[test]
fn resize_delegates_to_the_inner() {
    let mut io = IoCursor::new(vec![1u8, 2, 3]);
    io.resize(5).unwrap();
    assert_eq!(io.into_inner(), vec![1, 2, 3, 0, 0]);
}

#[test]
fn seek_past_the_end_errors_and_leaves_the_cursor() {
    let mut io = IoCursor::new(vec![1u8, 2, 3]);
    io.seek(2, Whence::Start).unwrap();
    assert_eq!(io.seek(9, Whence::Start), Err(IoError::OutOfBounds));
    assert_eq!(io.position().unwrap(), 2);
}
