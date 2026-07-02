//! Tests for the `IOBase` positioned-I/O trait.

use yggdryl_core::{IOBase, IOError, Whence};

/// A tiny in-memory byte store implementing only the two array primitives.
#[derive(Default)]
struct Mem {
    data: Vec<u8>,
}

impl Mem {
    fn offset(&self, position: usize, whence: Whence) -> usize {
        match whence {
            Whence::End => self.data.len() + position,
            _ => position,
        }
    }
}

impl IOBase<u8> for Mem {
    fn pread_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        let start = self.offset(position, whence);
        let end = start + size;
        if end > self.data.len() {
            return Err(IOError::OutOfBounds {
                offset: end,
                len: self.data.len(),
            });
        }
        Ok(self.data[start..end].to_vec())
    }

    fn pwrite_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        let start = self.offset(position, whence);
        let end = start + values.len();
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
        self.data[start..end].copy_from_slice(values);
        Ok(())
    }
}

#[test]
fn array_round_trip_and_append() {
    let mut mem = Mem::default();
    mem.pwrite_array(0, Whence::Start, &[1, 2, 3]).unwrap();
    mem.pwrite_array(0, Whence::End, &[4, 5]).unwrap(); // append at the end
    assert_eq!(
        mem.pread_array(0, Whence::Start, 5).unwrap(),
        vec![1, 2, 3, 4, 5]
    );
}

#[test]
fn single_element_defaults_delegate_to_arrays() {
    let mut mem = Mem::default();
    mem.pwrite_array(0, Whence::Start, &[0; 4]).unwrap();
    mem.pwrite_one(1, Whence::Start, 9).unwrap();
    assert_eq!(mem.pread_one(1, Whence::Start).unwrap(), 9);
    assert_eq!(
        mem.pread_array(0, Whence::Start, 4).unwrap(),
        vec![0, 9, 0, 0]
    );
}

#[test]
fn out_of_bounds_read_errors() {
    let mem = Mem::default();
    let error = mem.pread_one(0, Whence::Start).unwrap_err();
    assert!(matches!(error, IOError::OutOfBounds { .. }));
}
