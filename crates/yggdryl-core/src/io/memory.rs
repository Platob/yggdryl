//! [`MemoryIo`] — a growable, in-memory [`Io`] backed by a shared byte store.

use std::sync::Arc;

use crate::buffer::Buffer;
use crate::error::IoError;
use crate::io::{Io, Whence};
use crate::scalar::BinaryScalar;

/// An in-memory, auto-growable [`Io`] whose reads are zero-copy
/// [`BinaryScalar`](crate::BinaryScalar) views of the shared store.
///
/// Writes are copy-on-write: while any read view is still outstanding (the store
/// is shared) a write clones the store first, so existing zero-copy views stay
/// valid and immutable. The whole stream can be taken as a zero-copy
/// [`BinaryScalar`] buffer view via [`to_scalar`](MemoryIo::to_scalar), or
/// serialized through that scalar — there is no separate buffer type.
///
/// `MemoryIo` is an IO handle (it owns a cursor), so it is not itself hashable or
/// serializable; take a [`to_scalar`](MemoryIo::to_scalar) snapshot for that.
///
/// ```
/// use yggdryl_core::{Io, MemoryIo, Scalar, Whence};
///
/// let mut io = MemoryIo::new();
/// io.write(b"hello ").unwrap();
/// io.write(b"world").unwrap();
/// assert_eq!(io.size(), 11);
///
/// io.seek(0, Whence::Start).unwrap();
/// let head = io.read(5).unwrap(); // zero-copy view
/// assert_eq!(head.as_bytes(), Some(b"hello".as_slice()));
/// assert_eq!(io.to_scalar().as_bytes(), Some(b"hello world".as_slice()));
/// ```
#[derive(Clone, Debug)]
pub struct MemoryIo {
    store: Arc<[u8]>,
    size: usize,
    pos: u64,
}

impl Default for MemoryIo {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryIo {
    /// An empty stream.
    pub fn new() -> Self {
        Self {
            store: Arc::from(&[] as &[u8]),
            size: 0,
            pos: 0,
        }
    }

    /// An empty stream with at least `capacity` bytes preallocated.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            store: Arc::from(vec![0u8; capacity]),
            size: 0,
            pos: 0,
        }
    }

    /// A stream initialised with a copy of `bytes` (cursor at the start).
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            store: Arc::from(bytes),
            size: bytes.len(),
            pos: 0,
        }
    }

    /// A stream initialised from a binary scalar's bytes (null is treated as
    /// empty), cursor at the start.
    pub fn from_scalar(scalar: &BinaryScalar) -> Self {
        Self::from_bytes(scalar.as_bytes().unwrap_or(&[]))
    }

    /// The valid bytes, borrowed without copying.
    pub fn as_slice(&self) -> &[u8] {
        &self.store[..self.size]
    }

    /// The whole stream as a zero-copy [`Buffer`] view.
    pub fn to_buffer(&self) -> Buffer {
        Buffer::from_shared(Arc::clone(&self.store), 0, self.size)
    }

    /// The whole stream as a zero-copy [`BinaryScalar`] view.
    pub fn to_scalar(&self) -> BinaryScalar {
        BinaryScalar::from_buffer(self.to_buffer())
    }

    /// Ensures the store holds at least `needed` bytes and is uniquely owned,
    /// returning a mutable view of the whole allocation. Clones (copy-on-write)
    /// when the store is shared, so outstanding read views stay valid.
    fn reserve_mut(&mut self, needed: usize) -> &mut [u8] {
        let capacity = self.store.len();
        let grow = capacity < needed;
        let shared = Arc::get_mut(&mut self.store).is_none();
        if grow || shared {
            let new_capacity = if grow {
                needed.max(capacity.saturating_mul(2)).max(8)
            } else {
                capacity
            };
            let mut grown = vec![0u8; new_capacity];
            grown[..self.size].copy_from_slice(&self.store[..self.size]);
            self.store = Arc::from(grown);
        }
        Arc::get_mut(&mut self.store).expect("store is uniquely owned after reserve")
    }

    /// Converts a read offset to an index, erroring if it is past the end.
    fn read_offset(&self, offset: u64) -> Result<usize, IoError> {
        if offset > self.size as u64 {
            return Err(IoError::OutOfBounds {
                offset,
                size: self.size as u64,
            });
        }
        usize::try_from(offset).map_err(|_| IoError::OutOfBounds {
            offset,
            size: self.size as u64,
        })
    }
}

impl Io for MemoryIo {
    fn size(&self) -> u64 {
        self.size as u64
    }

    fn capacity(&self) -> u64 {
        self.store.len() as u64
    }

    fn tell(&self) -> u64 {
        self.pos
    }

    fn seek(&mut self, offset: i64, whence: Whence) -> Result<u64, IoError> {
        let base = match whence {
            Whence::Start => 0i128,
            Whence::Current => self.pos as i128,
            Whence::End => self.size as i128,
        };
        let target = base + offset as i128;
        if target < 0 {
            return Err(IoError::InvalidSeek(format!(
                "resolved to a negative position ({target})"
            )));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }

    fn pread_into(&self, offset: u64, dst: &mut [u8]) -> Result<usize, IoError> {
        let start = self.read_offset(offset)?;
        let n = dst.len().min(self.size - start);
        dst[..n].copy_from_slice(&self.store[start..start + n]);
        Ok(n)
    }

    fn pread(&self, offset: u64, len: usize) -> Result<BinaryScalar, IoError> {
        let start = self.read_offset(offset)?;
        let n = len.min(self.size - start);
        Ok(BinaryScalar::from_buffer(Buffer::from_shared(
            Arc::clone(&self.store),
            start,
            start + n,
        )))
    }

    fn pwrite(&mut self, offset: u64, src: &[u8]) -> Result<usize, IoError> {
        let start = usize::try_from(offset).map_err(|_| IoError::OutOfBounds {
            offset,
            size: self.size as u64,
        })?;
        let end = start.checked_add(src.len()).ok_or(IoError::OutOfBounds {
            offset,
            size: self.size as u64,
        })?;
        let size = self.size;
        let store = self.reserve_mut(end);
        if start > size {
            store[size..start].fill(0); // zero the gap when writing past the end
        }
        store[start..end].copy_from_slice(src);
        if end > self.size {
            self.size = end;
        }
        Ok(src.len())
    }

    fn set_capacity(&mut self, capacity: u64) -> Result<(), IoError> {
        let capacity = usize::try_from(capacity).map_err(|_| IoError::OutOfBounds {
            offset: capacity,
            size: self.size as u64,
        })?;
        let keep = self.size.min(capacity);
        let mut grown = vec![0u8; capacity];
        grown[..keep].copy_from_slice(&self.store[..keep]);
        self.store = Arc::from(grown);
        self.size = keep;
        Ok(())
    }

    fn resize(&mut self, new_size: u64, fill: u8) -> Result<(), IoError> {
        let new_size = usize::try_from(new_size).map_err(|_| IoError::OutOfBounds {
            offset: new_size,
            size: self.size as u64,
        })?;
        let old = self.size;
        if new_size > old {
            let store = self.reserve_mut(new_size);
            store[old..new_size].fill(fill);
        }
        self.size = new_size;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_zero_copy_views_of_the_store() {
        let mut io = MemoryIo::from_bytes(b"hello world");
        let view = io.pread(0, 5).unwrap();
        // The read shares the store's allocation rather than copying it out.
        assert_eq!(Arc::strong_count(&io.store), 2);
        assert_eq!(view.as_bytes(), Some(b"hello".as_slice()));

        // A write while the view is outstanding copies-on-write, leaving the view
        // valid and unchanged.
        io.pwrite(0, b"HELLO").unwrap();
        assert_eq!(view.as_bytes(), Some(b"hello".as_slice()));
        assert_eq!(io.as_slice(), b"HELLO world");
    }

    #[test]
    fn grows_and_tracks_capacity() {
        let mut io = MemoryIo::new();
        assert_eq!(io.write(b"abc").unwrap(), 3);
        assert_eq!(io.size(), 3);
        assert!(io.capacity() >= 3);

        io.resize(5, b'.').unwrap();
        assert_eq!(io.as_slice(), b"abc..");
        io.resize(2, 0).unwrap();
        assert_eq!(io.as_slice(), b"ab");
    }

    #[test]
    fn seek_and_cursor_reads() {
        let mut io = MemoryIo::from_bytes(b"0123456789");
        assert_eq!(io.seek(3, Whence::Start).unwrap(), 3);
        assert_eq!(io.read(2).unwrap().as_bytes(), Some(b"34".as_slice()));
        assert_eq!(io.tell(), 5);
        assert_eq!(io.seek(-1, Whence::End).unwrap(), 9);
        assert!(io.seek(-100, Whence::Start).is_err());
    }
}
