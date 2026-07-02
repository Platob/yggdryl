//! Tests for the `RawIOCursor` and typed `IOCursor` positioned-stream adapters.

use yggdryl_core::{
    BitBuffer, ByteBuffer, IOBase, IOCursor, IOError, RawIOBase, RawIOCursor, Seekable, Whence,
};

#[test]
fn sequential_byte_reads_advance_the_cursor() {
    let cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40]));
    assert_eq!(cursor.tell(), 0);
    assert_eq!(
        cursor.pread_byte_array(0, Whence::Current, 2).unwrap(),
        vec![10, 20]
    );
    assert_eq!(cursor.tell(), 2);
    assert_eq!(
        cursor.pread_byte_array(0, Whence::Current, 2).unwrap(),
        vec![30, 40]
    );
    assert_eq!(cursor.tell(), 4);
}

#[test]
fn sequential_byte_writes_advance_the_cursor() {
    let mut cursor = RawIOCursor::new(ByteBuffer::new());
    cursor
        .pwrite_byte_array(0, Whence::Current, &[1, 2, 3])
        .unwrap();
    assert_eq!(cursor.tell(), 3);
    cursor.pwrite_byte_one(0, Whence::Current, 4).unwrap();
    assert_eq!(cursor.tell(), 4);
    assert_eq!(cursor.get_ref().as_bytes(), &[1, 2, 3, 4]);
}

#[test]
fn seek_and_tell_move_the_cursor_without_touching_data() {
    let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40]));
    assert_eq!(cursor.seek(2, Whence::Start).unwrap(), 2);
    assert_eq!(cursor.tell(), 2);
    // Current + 1 == absolute byte 3 == 40; reading advances the cursor to 4.
    assert_eq!(cursor.pread_byte_one(1, Whence::Current).unwrap(), 40);
    assert_eq!(cursor.tell(), 4);
    assert_eq!(cursor.seek(1, Whence::Current).unwrap(), 5);
    assert_eq!(cursor.seek(0, Whence::End).unwrap(), 4);
    assert_eq!(cursor.get_ref().as_bytes(), &[10, 20, 30, 40]); // data untouched
}

#[test]
fn start_and_end_relative_access_ignore_the_cursor_but_still_advance_it() {
    let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40]));
    cursor.seek(1, Whence::Start).unwrap(); // cursor at byte 1
                                            // An absolute Start read ignores the cursor base, then advances to end-of-read.
    assert_eq!(cursor.pread_byte_one(3, Whence::Start).unwrap(), 40);
    assert_eq!(cursor.tell(), 4);
    // An End-relative write appends (ignoring the cursor) and advances past it.
    cursor.pwrite_byte_one(0, Whence::End, 50).unwrap();
    assert_eq!(cursor.tell(), 5);
    assert_eq!(cursor.get_ref().as_bytes(), &[10, 20, 30, 40, 50]);
}

#[test]
fn bit_access_advances_the_cursor_in_bits() {
    let mut cursor = RawIOCursor::new(BitBuffer::new());
    cursor
        .pwrite_bit_array(0, Whence::Current, &[true, false, true])
        .unwrap();
    // Three bits written: the byte-granular tell floors to byte 0.
    assert_eq!(cursor.tell(), 0);
    cursor
        .pwrite_bit_array(0, Whence::Current, &[true])
        .unwrap();
    // Four bits total, still within the first byte.
    assert_eq!(cursor.get_ref().bit_size(), 4);
    cursor.seek(0, Whence::Start).unwrap();
    assert_eq!(
        cursor.pread_bit_array(0, Whence::Current, 4).unwrap(),
        vec![true, false, true, true]
    );
    // Reading four bits from bit 0 lands the cursor at bit 4 -> byte 0.
    assert_eq!(cursor.tell(), 0);
}

#[test]
fn empty_read_and_write_leave_the_cursor_untouched() {
    let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![1, 2, 3]));
    cursor.seek(2, Whence::Start).unwrap();
    cursor.pwrite_byte_array(0, Whence::Current, &[]).unwrap();
    assert_eq!(cursor.tell(), 2); // empty write is a no-op
    assert_eq!(
        cursor.pread_byte_array(0, Whence::Current, 0).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(cursor.tell(), 2); // empty read is a no-op
    assert_eq!(cursor.get_ref().byte_size(), 3);
}

#[test]
fn overflow_from_a_huge_seek_errors_instead_of_wrapping() {
    let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![1, 2, 3]));
    cursor.seek(usize::MAX, Whence::Start).unwrap(); // unbounded seek
    let error = cursor.pread_byte_one(1, Whence::Current).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
}

#[test]
fn size_and_capacity_forward_to_the_wrapped_resource() {
    let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![1, 2, 3]));
    assert_eq!(cursor.byte_size(), 3);
    assert!(cursor.byte_capacity() >= 3);
    cursor.resize_bytes(5).unwrap();
    assert_eq!(cursor.byte_size(), 5);
    assert!(cursor.resize_byte_capacity(64).unwrap() >= 64);
    assert_eq!(cursor.into_inner().as_bytes(), &[1, 2, 3, 0, 0]);
}

#[test]
fn a_cursor_can_be_a_stream_sink() {
    let source = ByteBuffer::from_bytes(vec![1, 2, 3, 4, 5, 6]);
    let mut sink = RawIOCursor::new(ByteBuffer::new());
    // Streaming writes at absolute Start; it still lands the bytes correctly.
    source
        .pread_io(1, Whence::Start, 4, &mut sink, 0, Whence::Start)
        .unwrap();
    assert_eq!(sink.get_ref().as_bytes(), &[2, 3, 4, 5]);
}

// ---- typed IOCursor ----

/// A minimal cursorless resource holding `u32`s, four little-endian bytes each.
#[derive(Default)]
struct Store {
    data: Vec<u8>,
}

impl RawIOBase for Store {
    fn byte_size(&self) -> usize {
        self.data.len()
    }
    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.data.resize(size, 0);
        Ok(())
    }
    fn pread_byte_array(
        &self,
        position: usize,
        _whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        self.data
            .get(position..position + size)
            .map(<[u8]>::to_vec)
            .ok_or(IOError::OutOfBounds {
                offset: position + size,
                len: self.data.len(),
            })
    }
    fn pwrite_byte_array(
        &mut self,
        position: usize,
        _whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        let end = position + values.len();
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[position..end].copy_from_slice(values);
        Ok(())
    }
    fn pread_bit_array(
        &self,
        position: usize,
        _whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        (0..size)
            .map(|i| {
                let idx = position + i;
                self.data
                    .get(idx / 8)
                    .map(|b| (b >> (7 - idx % 8)) & 1 == 1)
                    .ok_or(IOError::OutOfBounds {
                        offset: idx,
                        len: self.data.len() * 8,
                    })
            })
            .collect()
    }
    fn pwrite_bit_array(
        &mut self,
        position: usize,
        _whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        let needed = (position + values.len()).div_ceil(8);
        if needed > self.data.len() {
            self.data.resize(needed, 0);
        }
        for (i, &bit) in values.iter().enumerate() {
            let idx = position + i;
            let mask = 1u8 << (7 - idx % 8);
            if bit {
                self.data[idx / 8] |= mask;
            } else {
                self.data[idx / 8] &= !mask;
            }
        }
        Ok(())
    }
}

impl IOBase<u32> for Store {
    fn value_to_bytes(&self, value: &u32) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }
    fn size(&self) -> usize {
        self.byte_size() / 4
    }
    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        self.resize_bytes(size * 4)
    }
}

#[test]
fn typed_writes_stream_and_advance_the_cursor() {
    let mut cursor = IOCursor::new(Store::default());
    cursor.pwrite_one(0, Whence::Current, &1).unwrap();
    cursor.pwrite_one(0, Whence::Current, &2).unwrap();
    cursor.pwrite_array(0, Whence::Current, &[3, 4]).unwrap();
    assert_eq!(cursor.tell(), 16); // four u32s, four bytes each
    assert_eq!(cursor.size(), 4);
    assert_eq!(
        cursor.get_ref().data,
        vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 4, 0, 0, 0]
    );
}

#[test]
fn typed_cursor_forwards_size_and_resize() {
    let mut cursor = IOCursor::new(Store::default());
    IOBase::<u32>::resize(&mut cursor, 3).unwrap();
    assert_eq!(cursor.size(), 3);
    assert_eq!(cursor.byte_size(), 12);
    assert_eq!(IOBase::<u32>::capacity(&cursor), 3);
}
