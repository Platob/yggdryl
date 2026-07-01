//! Tests for the [`Io`] trait and its in-memory [`Vec`] leaf implementation.

use yggdryl_core::{Io, IoError, Whence};

#[test]
fn pread_reads_one_element_from_each_whence_origin() {
    let io = vec![10u8, 20, 30, 40, 50];
    // Start counts forward from the front.
    assert_eq!(io.pread_one(1, Whence::Start).unwrap(), 20);
    // End counts back from the end (offset `1` → the last element).
    assert_eq!(io.pread_one(1, Whence::End).unwrap(), 50);
    // Current anchors at the cursor, which is `0` for a cursorless Vec.
    assert_eq!(io.pread_one(1, Whence::Current).unwrap(), 20);
}

#[test]
fn pread_errors_at_and_past_the_end() {
    let io = vec![1u8, 2, 3];
    // Reading exactly at the end — where no element lives — errors.
    assert_eq!(io.pread_one(3, Whence::Start), Err(IoError::OutOfBounds));
    assert_eq!(io.pread_one(0, Whence::End), Err(IoError::OutOfBounds));
    // Reading past the end errors.
    assert_eq!(io.pread_one(4, Whence::Start), Err(IoError::OutOfBounds));
    // An End offset larger than the source errors.
    assert_eq!(io.pread_one(4, Whence::End), Err(IoError::OutOfBounds));
}

#[test]
fn pwrite_overwrites_and_appends() {
    let mut io = vec![1u8, 2, 3];
    // Overwrite in place.
    io.pwrite_one(1, Whence::Start, 20).unwrap();
    assert_eq!(io, vec![1, 20, 3]);
    // Appending at the very end via `Start == len`.
    io.pwrite_one(3, Whence::Start, 4).unwrap();
    assert_eq!(io, vec![1, 20, 3, 4]);
    // An End offset of `0` appends at the very end.
    io.pwrite_one(0, Whence::End, 5).unwrap();
    assert_eq!(io, vec![1, 20, 3, 4, 5]);
    // An End offset of `1` overwrites the last element.
    io.pwrite_one(1, Whence::End, 50).unwrap();
    assert_eq!(io, vec![1, 20, 3, 4, 50]);
    // Writing past the end errors, leaving the source untouched.
    assert_eq!(
        io.pwrite_one(9, Whence::Start, 7),
        Err(IoError::OutOfBounds)
    );
    assert_eq!(io, vec![1, 20, 3, 4, 50]);
}

#[test]
fn pread_array_reads_a_window_and_clamps_at_the_end() {
    let io = vec![10u8, 20, 30, 40, 50];
    // A window from each origin.
    assert_eq!(io.pread_array(1, Whence::Start, 2).unwrap(), vec![20, 30]);
    assert_eq!(io.pread_array(2, Whence::End, 5).unwrap(), vec![40, 50]);
    // More than is available returns only what's there.
    assert_eq!(io.pread_array(3, Whence::Start, 10).unwrap(), vec![40, 50]);
    // Reading exactly at the end yields nothing.
    assert!(io.pread_array(5, Whence::Start, 3).unwrap().is_empty());
    // Starting past the end errors.
    assert_eq!(
        io.pread_array(6, Whence::Start, 1),
        Err(IoError::OutOfBounds)
    );
}

#[test]
fn pwrite_array_overwrites_and_extends() {
    let mut io = vec![1u8, 2, 3];
    // Overwrite in place.
    assert_eq!(io.pwrite_array(1, Whence::Start, &[20, 30]).unwrap(), 2);
    assert_eq!(io, vec![1, 20, 30]);
    // Overwrite the tail and extend past the end in one write.
    assert_eq!(io.pwrite_array(2, Whence::Start, &[31, 4, 5]).unwrap(), 3);
    assert_eq!(io, vec![1, 20, 31, 4, 5]);
    // An End offset of `0` appends at the very end.
    assert_eq!(io.pwrite_array(0, Whence::End, &[6, 7]).unwrap(), 2);
    assert_eq!(io, vec![1, 20, 31, 4, 5, 6, 7]);
    // Starting past the end errors, leaving the source untouched.
    assert_eq!(
        io.pwrite_array(9, Whence::Start, &[8]),
        Err(IoError::OutOfBounds)
    );
    assert_eq!(io, vec![1, 20, 31, 4, 5, 6, 7]);
}

#[test]
fn capacity_and_with_capacity_manage_spare_room() {
    // `Io::capacity` / `Io::resize` are reached through `Io::` on a bare Vec, whose
    // inherent `capacity` / `resize` would otherwise shadow them.
    let mut io = vec![1u8, 2, 3];
    assert!(Io::capacity(&io).unwrap() >= 3);
    // Reserving grows the capacity without touching the length or the elements.
    io.with_capacity(64).unwrap();
    assert!(Io::capacity(&io).unwrap() >= 64);
    assert_eq!(io, vec![1, 2, 3]);
}

#[test]
fn resize_grows_with_the_default_and_shrinks() {
    let mut io = vec![1u8, 2, 3];
    // The fill value is `T::default()`.
    assert_eq!(Io::default(&io), 0u8);
    // Growing fills the new slots with the default element.
    Io::resize(&mut io, 5).unwrap();
    assert_eq!(io, vec![1, 2, 3, 0, 0]);
    // Shrinking truncates.
    Io::resize(&mut io, 2).unwrap();
    assert_eq!(io, vec![1, 2]);
}

#[test]
fn pread_io_is_a_zero_copy_view_that_clamps() {
    let io = vec![10u8, 20, 30, 40, 50];
    // The view borrows `io` and re-bases to its own `0..len`.
    let view = io.pread_io(1, Whence::Start, 3).unwrap();
    assert_eq!(view.len().unwrap(), 3);
    assert_eq!(view.pread_one(0, Whence::Start).unwrap(), 20);
    assert_eq!(
        view.pread_array(0, Whence::Start, 10).unwrap(),
        vec![20, 30, 40]
    );
    // The borrowed source is read-only through the view.
    let mut view = io.pread_io(0, Whence::Start, 5).unwrap();
    assert_eq!(
        view.pwrite_one(0, Whence::Start, 99),
        Err(IoError::ReadOnly)
    );
}

#[test]
fn pwrite_io_streams_another_io_in() {
    let src = vec![7u8, 8, 9];
    let mut dst = vec![1u8, 2, 3, 4, 5];
    // Overwrites in place, returning the count written.
    assert_eq!(dst.pwrite_io(1, Whence::Start, &src).unwrap(), 3);
    assert_eq!(dst, vec![1, 7, 8, 9, 5]);
    // Overwrites the tail and extends past the end in one transfer.
    assert_eq!(dst.pwrite_io(0, Whence::End, &src).unwrap(), 3);
    assert_eq!(dst, vec![1, 7, 8, 9, 5, 7, 8, 9]);
    // A zero-copy view round-trips through pwrite_io.
    let view = dst.pread_io(0, Whence::Start, 3).unwrap();
    let mut copy = vec![0u8; 3];
    assert_eq!(copy.pwrite_io(0, Whence::Start, &view).unwrap(), 3);
    assert_eq!(copy, vec![1, 7, 8]);
}

// A minimal source that implements only the single-element primitives, so the
// trait's default `pread_array` / `pwrite_array` (looping those primitives) are the
// ones under test here — the "array IO for free" path a new source inherits.
struct Bare(Vec<u8>);

impl Io<u8> for Bare {
    fn len(&self) -> Result<usize, IoError> {
        Io::len(&self.0)
    }
    fn pread_one(&self, position: usize, whence: Whence) -> Result<u8, IoError> {
        self.0.pread_one(position, whence)
    }
    fn pwrite_one(&mut self, position: usize, whence: Whence, value: u8) -> Result<(), IoError> {
        self.0.pwrite_one(position, whence, value)
    }
}

#[test]
fn default_array_methods_loop_the_single_element_primitives() {
    let mut io = Bare(vec![1u8, 2, 3]);
    assert_eq!(io.pread_array(0, Whence::Start, 2).unwrap(), vec![1, 2]);
    // Clamps at the end just like the bulk override.
    assert_eq!(io.pread_array(1, Whence::Start, 10).unwrap(), vec![2, 3]);
    // Overwrites then extends contiguously.
    assert_eq!(io.pwrite_array(1, Whence::Start, &[20, 30, 40]).unwrap(), 3);
    assert_eq!(io.0, vec![1, 20, 30, 40]);
}

#[test]
fn default_resize_grows_with_fill_but_cannot_shrink() {
    let mut io = Bare(vec![1u8, 2, 3]);
    // The trait default grows through `pwrite_array`, filling with the default.
    io.resize(5).unwrap();
    assert_eq!(io.0, vec![1, 2, 3, 0, 0]);
    // A shrink is skipped — the base trait cannot truncate.
    io.resize(1).unwrap();
    assert_eq!(io.0, vec![1, 2, 3, 0, 0]);
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
