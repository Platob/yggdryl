//! Tests for the [`IoSlice`] bounded window.

use yggdryl_core::{Io, IoError, IoSlice, Whence};

#[test]
fn window_rebases_positions_and_reports_its_own_len() {
    let io = IoSlice::new(vec![10u8, 20, 30, 40, 50], 1, 3); // window [20, 30, 40]
    assert_eq!(io.len().unwrap(), 3);
    assert_eq!(io.offset(), 1);
    assert_eq!(io.pread_one(0, Whence::Start).unwrap(), 20);
    assert_eq!(io.pread_one(2, Whence::Start).unwrap(), 40);
    // End is measured from the window's end.
    assert_eq!(io.pread_one(1, Whence::End).unwrap(), 40);
}

#[test]
fn reads_clamp_to_the_window_and_never_spill_over() {
    let io = IoSlice::new(vec![10u8, 20, 30, 40, 50], 1, 3);
    // Asking for more than the window holds returns only the window.
    assert_eq!(
        io.pread_array(0, Whence::Start, 10).unwrap(),
        vec![20, 30, 40]
    );
    // Reading at the window end yields nothing / errors for a single element.
    assert!(io.pread_array(3, Whence::Start, 2).unwrap().is_empty());
    assert_eq!(io.pread_one(3, Whence::Start), Err(IoError::OutOfBounds));
    // Past the window errors.
    assert_eq!(io.pread_one(4, Whence::Start), Err(IoError::OutOfBounds));
}

#[test]
fn writes_stay_within_the_window() {
    let mut io = IoSlice::new(vec![10u8, 20, 30, 40, 50], 1, 3);
    io.pwrite_one(0, Whence::Start, 21).unwrap();
    io.pwrite_array(1, Whence::Start, &[31, 41]).unwrap();
    assert_eq!(
        io.pread_array(0, Whence::Start, 3).unwrap(),
        vec![21, 31, 41]
    );
    // Writing a single element past the window end errors.
    assert_eq!(
        io.pwrite_one(3, Whence::Start, 9),
        Err(IoError::OutOfBounds)
    );
    // A bulk write is clamped to what fits in the window.
    assert_eq!(io.pwrite_array(2, Whence::Start, &[42, 99, 99]).unwrap(), 1);
    // The elements outside the window are untouched.
    assert_eq!(io.into_inner(), vec![10, 21, 31, 42, 50]);
}

#[test]
fn resize_is_a_no_op_on_a_fixed_window() {
    let mut io = IoSlice::new(vec![1u8, 2, 3, 4], 0, 2);
    io.resize(5).unwrap(); // ignored — a slice is a fixed window
    assert_eq!(io.len().unwrap(), 2);
    assert_eq!(io.into_inner(), vec![1, 2, 3, 4]);
}
