//! The zero-copy [`Buffer`].

use std::hash::{Hash, Hasher};
use std::ops::{Deref, Range};
use std::sync::Arc;

/// An `Arc`-backed byte buffer.
///
/// Cloning shares the underlying allocation (O(1), no copy) and
/// [`slice`](Buffer::slice) returns a sub-view that shares it too — the zero-copy
/// foundation the view-backed scalar types rely on. Equality, ordering and hashing
/// are by byte content, so a `Buffer` keys a map the same as its bytes would.
///
/// ```
/// use yggdryl_core::Buffer;
///
/// let buf = Buffer::from_vec(b"hello world".to_vec());
/// assert_eq!(buf.as_slice(), b"hello world");
///
/// let world = buf.slice(6..11);
/// assert_eq!(world.as_slice(), b"world");
/// // The slice shares the original allocation — same address, no copy.
/// assert_eq!(world.as_slice().as_ptr(), buf.as_slice()[6..].as_ptr());
///
/// // The default is an empty buffer.
/// assert!(Buffer::default().is_empty());
/// ```
#[derive(Clone, Default)]
pub struct Buffer {
    storage: Arc<Vec<u8>>,
    offset: usize,
    len: usize,
}

impl Buffer {
    /// Wraps `bytes`, taking ownership without copying.
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        let len = bytes.len();
        Self {
            storage: Arc::new(bytes),
            offset: 0,
            len,
        }
    }

    /// The buffer's bytes, borrowed (zero-copy).
    pub fn as_slice(&self) -> &[u8] {
        &self.storage[self.offset..self.offset + self.len]
    }

    /// The number of bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// A zero-copy sub-view over `range`, sharing the same allocation.
    ///
    /// # Panics
    ///
    /// Panics if `range` is out of bounds.
    pub fn slice(&self, range: Range<usize>) -> Self {
        assert!(
            range.start <= range.end && range.end <= self.len,
            "buffer slice {range:?} out of bounds for length {}",
            self.len
        );
        Self {
            storage: Arc::clone(&self.storage),
            offset: self.offset + range.start,
            len: range.end - range.start,
        }
    }
}

impl From<Vec<u8>> for Buffer {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from_vec(bytes)
    }
}

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Buffer").field(&self.as_slice()).finish()
    }
}

impl PartialEq for Buffer {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Buffer {}

impl PartialOrd for Buffer {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Buffer {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl Hash for Buffer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Buffer {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Buffer {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(Buffer::from_vec)
    }
}
