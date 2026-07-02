//! Tests for the `RawIOSlice` / `IOSlice` window adapters and the `cursor` / `slice`
//! factory methods on `RawIOBase` and `IOBase`.

mod common;
use common::Store;
use yggdryl_core::{
    ByteBuffer, IOBase, IOCursor, IOError, IOSlice, RawIOBase, RawIOCursor, RawIOSlice, Whence,
};

#[test]
fn window_reads_are_relative_and_bounded() {
    let slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40, 50]), 1, 4);
    assert_eq!(slice.byte_size(), 3); // bytes [1, 4)
    assert_eq!(slice.start(), 1);
    assert_eq!(slice.end(), 4);
    assert_eq!(
        slice.pread_byte_array(0, Whence::Start, 3).unwrap(),
        vec![20, 30, 40]
    );
    // Reading past the window fails even though the inner has more bytes.
    let error = slice.pread_byte_array(0, Whence::Start, 4).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
}

#[test]
fn window_end_is_the_backed_append_point() {
    let slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40, 50]), 1, 4);
    // End, 0 is the append point (one past the last window byte): reading there fails.
    assert!(slice.pread_byte_one(0, Whence::End).is_err());
    // But a zero-length read at the append point is a valid no-op.
    assert_eq!(
        slice.pread_byte_array(0, Whence::End, 0).unwrap(),
        Vec::<u8>::new()
    );
}

#[test]
fn writes_stay_within_the_window_and_reach_the_inner() {
    let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40, 50]), 1, 4);
    slice
        .pwrite_byte_array(0, Whence::Start, &[97, 98])
        .unwrap();
    assert_eq!(slice.get_ref().as_bytes(), &[10, 97, 98, 40, 50]);
    // A write that would spill past the window end fails; the inner is untouched.
    let error = slice
        .pwrite_byte_array(2, Whence::Start, &[1, 2])
        .unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
    assert_eq!(slice.get_ref().as_bytes(), &[10, 97, 98, 40, 50]);
}

#[test]
fn append_via_end_grows_the_inner_within_the_window() {
    // Window [2, 5) over a 2-byte buffer: only bytes [2, 5) may be written.
    let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![1, 2]), 2, 5);
    assert_eq!(slice.byte_size(), 0); // nothing backed yet within the window
    slice.pwrite_byte_array(0, Whence::End, &[3, 4, 5]).unwrap();
    assert_eq!(slice.byte_size(), 3);
    assert_eq!(slice.get_ref().as_bytes(), &[1, 2, 3, 4, 5]);
    // The window is full; another End write fails.
    assert!(slice.pwrite_byte_one(0, Whence::End, 6).is_err());
}

#[test]
fn resize_moves_the_end_and_never_truncates_outside_data() {
    let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![1, 2, 3, 4, 5]), 1, 3);
    assert_eq!(slice.byte_size(), 2);
    // Grow the window: end moves out, backing bytes appear from the inner.
    slice.resize_bytes(4).unwrap();
    assert_eq!(slice.byte_size(), 4); // window is now [1, 5)
    assert_eq!(
        slice.pread_byte_array(0, Whence::Start, 4).unwrap(),
        vec![2, 3, 4, 5]
    );
    // Shrink the window: the inner keeps all of its data (nothing truncated).
    slice.resize_bytes(1).unwrap();
    assert_eq!(slice.byte_size(), 1);
    assert_eq!(slice.get_ref().as_bytes(), &[1, 2, 3, 4, 5]);
}

#[test]
fn resize_grows_the_inner_to_back_the_window() {
    let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![1, 2, 3]), 1, 3);
    // Grow beyond the inner: the inner is zero-filled to back the window.
    slice.resize_bytes(5).unwrap();
    assert_eq!(slice.byte_size(), 5); // window [1, 6)
    assert_eq!(slice.get_ref().as_bytes(), &[1, 2, 3, 0, 0, 0]);
}

#[test]
fn bit_access_is_offset_by_the_window_start() {
    let slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![0x00, 0b1010_0000, 0x00]), 1, 2);
    assert_eq!(slice.bit_size(), 8); // one byte in the window
    assert!(slice.pread_bit_one(0, Whence::Start).unwrap()); // MSB of byte 1
    assert!(!slice.pread_bit_one(1, Whence::Start).unwrap());
    // Reading past the window's bits fails.
    assert!(slice.pread_bit_array(0, Whence::Start, 9).is_err());
}

#[test]
fn overflow_from_a_huge_window_end_errors() {
    let slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![1, 2, 3]), 1, usize::MAX);
    // End base (3) + usize::MAX would wrap; guarded as OutOfBounds.
    let error = slice.pread_byte_one(usize::MAX, Whence::End).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
}

#[test]
fn a_window_starting_past_the_inner_cannot_escape_below_its_start() {
    // Regression: the End base was min(inner, end), which could fall below start and
    // let an End-relative write mutate inner data before the window.
    let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![1, 2, 3]), 5, 8);
    assert_eq!(slice.byte_size(), 0);
    // An End write appends at the window start (5), zero-filling the gap [3, 5).
    slice.pwrite_byte_array(0, Whence::End, &[9, 9]).unwrap();
    assert_eq!(slice.byte_size(), 2);
    // The original bytes are untouched, the gap is zeroed, and the write lands in-window.
    assert_eq!(slice.get_ref().as_bytes(), &[1, 2, 3, 0, 0, 9, 9]);
    assert_eq!(
        slice.pread_byte_array(0, Whence::Start, 2).unwrap(),
        vec![9, 9]
    );
}

// ---- typed IOSlice ----

#[test]
fn typed_slice_counts_items_in_the_window() {
    let mut store = Store::default();
    store
        .pwrite_array(0, Whence::Start, &[1u32, 2, 3, 4])
        .unwrap(); // 4 u32s, 16 bytes
    let slice = IOSlice::new(store, 4, 12); // middle two u32s
    assert_eq!(slice.byte_size(), 8);
    assert_eq!(slice.size(), 2);
    assert_eq!(
        slice.pread_byte_array(0, Whence::Start, 8).unwrap(),
        vec![2, 0, 0, 0, 3, 0, 0, 0]
    );
}

#[test]
fn typed_slice_resize_counts_items() {
    let mut store = Store::default();
    store
        .pwrite_array(0, Whence::Start, &[1u32, 2, 3, 4])
        .unwrap();
    let mut slice = IOSlice::new(store, 4, 8); // one u32 window
    assert_eq!(slice.size(), 1);
    // Resize to two items -> the window's end bound moves out by one u32.
    IOBase::<u32>::resize(&mut slice, 2).unwrap();
    assert_eq!(slice.size(), 2);
    assert_eq!(slice.byte_size(), 8);
}

#[test]
fn typed_slice_over_an_empty_resource_sizes_via_element_width() {
    // Store overrides element_width, so a window over an empty store can still be
    // grown by item count (regression: this used to silently no-op).
    let mut slice = IOSlice::new(Store::default(), 0, 0);
    assert_eq!(slice.size(), 0);
    IOBase::<u32>::resize(&mut slice, 3).unwrap();
    assert_eq!(slice.size(), 3);
    assert_eq!(slice.byte_size(), 12);
}

/// A typed resource that does not override `element_width`, so the default
/// (`byte_size / size`) applies — `0` when empty.
#[derive(Default)]
struct DefaultWidth {
    inner: ByteBuffer,
}

impl RawIOBase for DefaultWidth {
    fn byte_size(&self) -> usize {
        self.inner.byte_size()
    }
    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.inner.resize_bytes(size)
    }
    fn pread_byte_array(&self, p: usize, w: Whence, s: usize) -> Result<Vec<u8>, IOError> {
        self.inner.pread_byte_array(p, w, s)
    }
    fn pwrite_byte_array(&mut self, p: usize, w: Whence, v: &[u8]) -> Result<(), IOError> {
        self.inner.pwrite_byte_array(p, w, v)
    }
    fn pread_bit_array(&self, p: usize, w: Whence, s: usize) -> Result<Vec<bool>, IOError> {
        self.inner.pread_bit_array(p, w, s)
    }
    fn pwrite_bit_array(&mut self, p: usize, w: Whence, v: &[bool]) -> Result<(), IOError> {
        self.inner.pwrite_bit_array(p, w, v)
    }
}

impl IOBase<u8> for DefaultWidth {
    fn value_to_bytes(&self, value: &u8) -> Vec<u8> {
        vec![*value]
    }
    fn size(&self) -> usize {
        self.byte_size()
    }
    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        self.resize_bytes(size)
    }
}

#[test]
fn typed_slice_resize_errors_when_the_width_is_indeterminate() {
    // An empty resource with the default element_width cannot convert items to bytes.
    let mut slice = IOSlice::new(DefaultWidth::default(), 0, 0);
    assert_eq!(IOBase::<u8>::size(&slice), 0);
    let error = IOBase::<u8>::resize(&mut slice, 4).unwrap_err();
    assert!(matches!(error, IOError::IndeterminateElementWidth));
    // A resize to zero still collapses cleanly.
    IOBase::<u8>::resize(&mut slice, 0).unwrap();
}

#[test]
fn typed_stream_errors_when_the_width_is_indeterminate() {
    // Streaming items into an empty resource with the default element_width can't
    // convert the item count to bytes.
    let source = ByteBuffer::from_bytes(vec![1, 2, 3]);
    let mut sink = DefaultWidth::default();
    let error = sink
        .pwrite_typed_io(0, Whence::Start, &source, 0, Whence::Start, 1)
        .unwrap_err();
    assert!(matches!(error, IOError::IndeterminateElementWidth));
}

// ---- cursor / slice factory methods ----

#[test]
fn raw_factory_methods_build_the_adapters() {
    let cursor: RawIOCursor<ByteBuffer> = ByteBuffer::from_bytes(vec![1, 2, 3]).cursor();
    assert_eq!(cursor.byte_size(), 3);

    let slice: RawIOSlice<ByteBuffer> = ByteBuffer::from_bytes(vec![1, 2, 3, 4]).slice(1, 3);
    assert_eq!(
        slice.pread_byte_array(0, Whence::Start, 2).unwrap(),
        vec![2, 3]
    );
}

#[test]
fn typed_factory_methods_build_the_typed_adapters() {
    let mut store = Store::default();
    store
        .pwrite_array(0, Whence::Start, &[10u32, 20, 30])
        .unwrap();

    // Disambiguate the typed factory from the raw one with fully-qualified syntax.
    let cursor: IOCursor<Store> = IOBase::<u32>::cursor(store);
    assert_eq!(cursor.size(), 3);

    let mut store = Store::default();
    store
        .pwrite_array(0, Whence::Start, &[10u32, 20, 30])
        .unwrap();
    let slice: IOSlice<Store> = IOBase::<u32>::slice(store, 4, 12);
    assert_eq!(slice.size(), 2);
}
