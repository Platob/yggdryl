//! [`Bytes`] — the **in-heap** concrete backing for the memory-access traits: an owned byte
//! `Vec` with a read/write cursor. It is the reference implementor of [`IOBase`] / [`IOCursor`]
//! / [`IOSlice`] — the "memory" side of the layer; a memory-mapped backing plugs in against the
//! same traits.

use super::{IOBase, IOCursor, IOSlice, IoError};

/// An in-heap byte buffer with a read/write cursor — the concrete in-memory implementor of the
/// [`IOBase`] / [`IOCursor`] / [`IOSlice`] contracts.
///
/// ```
/// use yggdryl_core::memory::{Bytes, IOCursor};
///
/// let mut b = Bytes::new();
/// b.write_all(b"hello ").unwrap();
/// b.write_all(b"world").unwrap();
/// assert_eq!(b.as_slice(), b"hello world");
///
/// b.rewind();
/// let mut head = [0u8; 5];
/// b.read_exact(&mut head).unwrap();
/// assert_eq!(&head, b"hello");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bytes {
    data: Vec<u8>,
    /// The cursor — bytes from the start; may sit past the end after a seek.
    position: u64,
}

impl Bytes {
    /// An empty buffer with the cursor at `0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// A buffer owning a **copy** of `data`, cursor at `0`.
    pub fn from_slice(data: &[u8]) -> Self {
        Self {
            data: data.to_vec(),
            position: 0,
        }
    }

    /// A buffer taking ownership of `data` **without copying**, cursor at `0`.
    pub fn from_vec(data: Vec<u8>) -> Self {
        Self { data, position: 0 }
    }

    /// The stored bytes as a slice — zero-copy.
    pub fn as_slice(&self) -> &[u8] {
        &self.data
    }

    /// The owned byte vector (consuming the buffer).
    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }
}

impl IOBase for Bytes {
    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    fn pread(&self, offset: u64, buf: &mut [u8]) -> usize {
        let start = offset as usize;
        if start >= self.data.len() {
            return 0;
        }
        let n = buf.len().min(self.data.len() - start);
        buf[..n].copy_from_slice(&self.data[start..start + n]);
        n
    }

    fn pwrite(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }
        let start = offset as usize;
        let end = start + data.len();
        if end > self.data.len() {
            self.data.resize(end, 0); // grow, zero-filling any gap
        }
        self.data[start..end].copy_from_slice(data);
        data.len()
    }
}

impl IOCursor for Bytes {
    fn position(&self) -> u64 {
        self.position
    }

    fn set_position(&mut self, position: u64) {
        self.position = position;
    }
}

impl IOSlice for Bytes {
    fn slice(&self, offset: u64, len: u64) -> Result<Self, IoError> {
        let available = self.data.len() as u64;
        let end = offset.checked_add(len).filter(|&e| e <= available).ok_or(
            IoError::SliceOutOfBounds {
                offset,
                len,
                available,
            },
        )?;
        Ok(Self::from_slice(&self.data[offset as usize..end as usize]))
    }
}
