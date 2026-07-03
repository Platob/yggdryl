//! Shared test fixtures for the positioned-I/O integration tests.

use yggdryl_core::{IOBase, IOError, RawIOBase, Whence};

/// A minimal cursorless resource holding `u32`s, four little-endian bytes each —
/// enough to exercise the `RawIOBase` and `IOBase<u32>` surfaces.
#[derive(Default)]
pub struct Store {
    pub data: Vec<u8>,
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
    fn element_width(&self) -> usize {
        4 // fixed width, so a slice can size items even over an empty store
    }
    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        self.resize_bytes(size * 4)
    }
}
