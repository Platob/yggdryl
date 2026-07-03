//! Tests for the `RawIOBase` (bytes/bits) and typed `IOBase` positioned-I/O traits.

use yggdryl_core::{IOBase, IOError, RawIOBase, Whence};

/// A cursorless byte store; bits are addressed MSB-first within each byte.
#[derive(Default)]
struct Mem {
    data: Vec<u8>,
}

impl Mem {
    fn byte_offset(&self, position: usize, whence: Whence) -> usize {
        match whence {
            Whence::End => self.data.len() + position,
            _ => position, // Start, and cursorless Current, from 0
        }
    }

    fn bit_offset(&self, position: usize, whence: Whence) -> usize {
        match whence {
            Whence::End => self.data.len() * 8 + position,
            _ => position,
        }
    }
}

impl RawIOBase for Mem {
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
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.byte_offset(position, whence);
        let end = start + size;
        if end > self.data.len() {
            return Err(IOError::OutOfBounds {
                offset: end,
                len: self.data.len(),
            });
        }
        Ok(self.data[start..end].to_vec())
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        let start = self.byte_offset(position, whence);
        let end = start + values.len();
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[start..end].copy_from_slice(values);
        Ok(())
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        let start = self.bit_offset(position, whence);
        let total = self.data.len() * 8;
        if start + size > total {
            return Err(IOError::OutOfBounds {
                offset: start + size,
                len: total,
            });
        }
        Ok((0..size)
            .map(|i| {
                let idx = start + i;
                (self.data[idx / 8] >> (7 - idx % 8)) & 1 == 1
            })
            .collect())
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        let start = self.bit_offset(position, whence);
        let needed = (start + values.len()).div_ceil(8);
        if needed > self.data.len() {
            self.data.resize(needed, 0);
        }
        for (i, &bit) in values.iter().enumerate() {
            let idx = start + i;
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

#[test]
fn byte_array_round_trip_and_append() {
    let mut mem = Mem::default();
    mem.pwrite_byte_array(0, Whence::Start, &[1, 2, 3]).unwrap();
    mem.pwrite_byte_array(0, Whence::End, &[4, 5]).unwrap(); // append at the end
    assert_eq!(
        mem.pread_byte_array(0, Whence::Start, 5).unwrap(),
        vec![1, 2, 3, 4, 5]
    );
}

#[test]
fn byte_one_defaults_delegate_to_arrays() {
    let mut mem = Mem::default();
    mem.pwrite_byte_array(0, Whence::Start, &[0; 3]).unwrap();
    mem.pwrite_byte_one(1, Whence::Start, 9).unwrap();
    assert_eq!(mem.pread_byte_one(1, Whence::Start).unwrap(), 9);
}

#[test]
fn bit_array_round_trips_msb_first() {
    let mut mem = Mem::default();
    mem.pwrite_bit_array(0, Whence::Start, &[true, false, true, true])
        .unwrap();
    assert_eq!(
        mem.pread_bit_array(0, Whence::Start, 4).unwrap(),
        vec![true, false, true, true]
    );
    // MSB-first: bits 0,2,3 set => 0b1011_0000.
    assert_eq!(mem.pread_byte_one(0, Whence::Start).unwrap(), 0b1011_0000);
}

#[test]
fn bit_one_defaults_are_msb_first() {
    let mut mem = Mem::default();
    mem.pwrite_byte_array(0, Whence::Start, &[0b1000_0000])
        .unwrap();
    assert!(mem.pread_bit_one(0, Whence::Start).unwrap());
    assert!(!mem.pread_bit_one(1, Whence::Start).unwrap());
    mem.pwrite_bit_one(3, Whence::Start, true).unwrap();
    assert_eq!(mem.pread_byte_one(0, Whence::Start).unwrap(), 0b1001_0000);
}

#[test]
fn out_of_bounds_byte_read_errors() {
    let mem = Mem::default();
    let error = mem.pread_byte_one(0, Whence::Start).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
}

// The typed layer: a `u32` is serialized little-endian, then written as raw bytes.
impl IOBase<u32> for Mem {
    fn value_to_bytes(&self, value: &u32) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    fn size(&self) -> usize {
        self.byte_size() / 4
    }

    fn element_width(&self) -> usize {
        4
    }

    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        self.resize_bytes(size * 4)
    }
}

#[test]
fn sizes_report_bytes_bits_and_items() {
    let mut mem = Mem::default();
    mem.pwrite_byte_array(0, Whence::Start, &[0; 8]).unwrap();
    assert_eq!(mem.byte_size(), 8);
    assert_eq!(mem.bit_size(), 64); // default: byte_size * 8
    assert_eq!(mem.size(), 2); // 8 bytes / 4 bytes per u32
}

#[test]
fn typed_writes_go_through_value_to_bytes() {
    let mut mem = Mem::default();
    mem.pwrite_one(0, Whence::Start, &0x0403_0201).unwrap();
    mem.pwrite_array(4, Whence::Start, &[0x0807_0605, 0x0c0b_0a09])
        .unwrap();
    assert_eq!(
        mem.pread_byte_array(0, Whence::Start, 12).unwrap(),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
}

#[test]
fn default_capacities_mirror_sizes() {
    let mut mem = Mem::default();
    mem.pwrite_byte_array(0, Whence::Start, &[0; 8]).unwrap();
    assert_eq!(mem.byte_capacity(), 8); // default: capacity == size
    assert_eq!(mem.bit_capacity(), 64);
    assert_eq!(mem.capacity(), 2); // typed default: capacity == size
                                   // The default capacity resize is a hint that leaves the allocation unchanged.
    assert_eq!(mem.resize_byte_capacity(100).unwrap(), 8);
    assert_eq!(IOBase::<u32>::resize_capacity(&mut mem, 100).unwrap(), 2);
}

#[test]
fn resize_bytes_and_bits() {
    let mut mem = Mem::default();
    mem.resize_bytes(4).unwrap();
    assert_eq!(mem.byte_size(), 4);
    // The default bit resize rounds up to whole bytes.
    mem.resize_bits(20).unwrap();
    assert_eq!(mem.byte_size(), 3);
}

#[test]
fn typed_resize_counts_items() {
    let mut mem = Mem::default();
    IOBase::<u32>::resize(&mut mem, 3).unwrap();
    assert_eq!(mem.byte_size(), 12);
    assert_eq!(mem.size(), 3);
}

#[test]
fn stream_copy_between_ios() {
    let mut source = Mem::default();
    source
        .pwrite_byte_array(0, Whence::Start, &[1, 2, 3, 4, 5, 6, 7, 8])
        .unwrap();

    // Read a slice of `source` into `sink`.
    let mut sink = Mem::default();
    source
        .pread_raw_io(2, Whence::Start, 4, &mut sink, 0, Whence::Start)
        .unwrap();
    assert_eq!(sink.data, vec![3, 4, 5, 6]);

    // And append from `source` into `sink` via End, resolved once against its size.
    sink.pwrite_raw_io(0, Whence::End, &source, 0, Whence::Start, 2)
        .unwrap();
    assert_eq!(sink.data, vec![3, 4, 5, 6, 1, 2]);
}

#[test]
fn typed_stream_copies_items_element_aligned() {
    let mut source = Mem::default();
    source
        .pwrite_array(
            0,
            Whence::Start,
            &[0x0403_0201u32, 0x0807_0605, 0x0c0b_0a09],
        )
        .unwrap(); // three u32s, 12 bytes

    // Read two items (item positions/counts) into a fresh sink.
    let mut sink = Mem::default();
    source
        .pread_typed_io(1, Whence::Start, 2, &mut sink, 0, Whence::Start)
        .unwrap();
    assert_eq!(sink.data, vec![5, 6, 7, 8, 9, 10, 11, 12]); // items 1 and 2

    // Append one item into the typed sink via End (item units).
    sink.pwrite_typed_io(0, Whence::End, &source, 0, Whence::Start, 1)
        .unwrap();
    assert_eq!(sink.data, vec![5, 6, 7, 8, 9, 10, 11, 12, 1, 2, 3, 4]);
    assert_eq!(sink.size(), 3);
}

#[test]
fn typed_stream_into_empty_sink_needs_a_width() {
    // A cursorless Mem overrides element_width, so a fresh sink still knows its width.
    let mut source = Mem::default();
    source.pwrite_one(0, Whence::Start, &7u32).unwrap();
    let mut sink = Mem::default();
    sink.pwrite_typed_io(0, Whence::Start, &source, 0, Whence::Start, 1)
        .unwrap();
    assert_eq!(sink.size(), 1);
    assert_eq!(sink.data, vec![7, 0, 0, 0]);
}

#[test]
fn primitive_helpers_read_and_write_little_endian() {
    use yggdryl_core::{ByteBuffer, RawIOBase, Whence};

    // Every numeric primitive round-trips through its little-endian bytes.
    let mut buf = ByteBuffer::new();
    buf.pwrite_i64(0, Whence::Start, -2).unwrap();
    buf.pwrite_u16(8, Whence::Start, 0xBEEF).unwrap();
    buf.pwrite_f64(10, Whence::Start, 1.5).unwrap();
    assert_eq!(buf.pread_i64(0, Whence::Start).unwrap(), -2);
    assert_eq!(buf.pread_u16(8, Whence::Start).unwrap(), 0xBEEF);
    assert_eq!(buf.pread_f64(10, Whence::Start).unwrap(), 1.5);

    // The bytes really are little-endian: the low byte comes first.
    buf.pwrite_u32(0, Whence::Start, 1).unwrap();
    assert_eq!(buf.pread_byte_one(0, Whence::Start).unwrap(), 1);
    assert_eq!(buf.pread_u8(0, Whence::Start).unwrap(), 1);
    assert_eq!(buf.pread_i8(0, Whence::Start).unwrap(), 1);
    assert_eq!(buf.pread_i16(0, Whence::Start).unwrap(), 1);
    assert_eq!(buf.pread_i32(0, Whence::Start).unwrap(), 1);
    assert_eq!(buf.pread_u64(8, Whence::Start).unwrap(), 0xBEEF);
    assert_eq!(
        buf.pread_f32(0, Whence::Start).unwrap(),
        f32::from_le_bytes([1, 0, 0, 0])
    );

    // Reads past the end fail like any positioned byte read.
    assert!(buf.pread_i64(usize::MAX, Whence::Start).is_err());
}
