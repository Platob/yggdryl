//! [`Buffer`] — the zero-copy byte backbone shared by every scalar value.

use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use std::ops::{Bound, RangeBounds};
use std::sync::Arc;

/// An immutable, reference-counted byte buffer with O(1) clone and zero-copy
/// slicing.
///
/// Cloning only bumps an [`Arc`] refcount and [`slice`](Buffer::slice) only moves
/// a pair of offsets, so neither touches the underlying bytes; the data is copied
/// exactly once, when the buffer is first built (from either a borrowed slice or
/// an owned `Vec` — an `Arc<[u8]>` stores its refcount inline and so cannot reuse
/// a `Vec`'s allocation). Equality, ordering and hashing all run over the live
/// byte range, so a sliced buffer compares equal to a fresh buffer holding the
/// same bytes.
///
/// ```
/// use yggdryl_core::Buffer;
///
/// let buf = Buffer::from_slice(b"hello world");
/// let world = buf.slice(6..); // no copy — shares the same allocation
/// assert_eq!(world.as_slice(), b"world");
/// assert_eq!(buf.as_slice(), b"hello world");
/// assert_eq!(world, Buffer::from_slice(b"world"));
/// ```
#[derive(Clone)]
pub struct Buffer {
    data: Arc<[u8]>,
    start: usize,
    end: usize,
}

impl Buffer {
    /// Builds a buffer by copying `bytes` into a fresh allocation.
    pub fn from_slice(bytes: &[u8]) -> Self {
        Self::from_arc(Arc::from(bytes))
    }

    /// Builds a buffer from an owned `Vec`, copying its bytes into the `Arc`
    /// allocation (an `Arc<[u8]>` cannot reuse a `Vec`'s allocation).
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        Self::from_arc(Arc::from(bytes))
    }

    fn from_arc(data: Arc<[u8]>) -> Self {
        let end = data.len();
        Self {
            data,
            start: 0,
            end,
        }
    }

    /// Builds a buffer over an existing shared allocation and byte range, without
    /// copying. Used by in-memory IO to hand out zero-copy views of its store.
    pub(crate) fn from_shared(data: Arc<[u8]>, start: usize, end: usize) -> Self {
        debug_assert!(start <= end && end <= data.len());
        Self { data, start, end }
    }

    /// The buffer's shared allocation and live byte range, for zero-copy reuse.
    pub(crate) fn as_parts(&self) -> (&Arc<[u8]>, usize, usize) {
        (&self.data, self.start, self.end)
    }

    /// The buffer's bytes, borrowed without copying.
    pub fn as_slice(&self) -> &[u8] {
        &self.data[self.start..self.end]
    }

    /// The number of live bytes in the buffer.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Whether the buffer holds no bytes.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// A zero-copy sub-range of this buffer, sharing the same allocation.
    ///
    /// Panics if the range falls outside `0..self.len()`.
    ///
    /// ```
    /// use yggdryl_core::Buffer;
    /// let buf = Buffer::from_slice(b"yggdryl");
    /// assert_eq!(buf.slice(0..3).as_slice(), b"ygg");
    /// ```
    pub fn slice(&self, range: impl RangeBounds<usize>) -> Self {
        let len = self.len();
        let start = match range.start_bound() {
            Bound::Included(&n) => n,
            Bound::Excluded(&n) => n.saturating_add(1),
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&n) => n.saturating_add(1),
            Bound::Excluded(&n) => n,
            Bound::Unbounded => len,
        };
        assert!(
            start <= end && end <= len,
            "Buffer::slice range {start}..{end} out of bounds for length {len}"
        );
        crate::log_event!(trace, "Buffer::slice {}..{} of {}", start, end, len);
        Self {
            data: Arc::clone(&self.data),
            start: self.start + start,
            end: self.start + end,
        }
    }

    /// Copies the live bytes into an owned `Vec`.
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<&[u8]> for Buffer {
    fn from(bytes: &[u8]) -> Self {
        Buffer::from_slice(bytes)
    }
}

impl From<Vec<u8>> for Buffer {
    fn from(bytes: Vec<u8>) -> Self {
        Buffer::from_vec(bytes)
    }
}

impl From<&str> for Buffer {
    fn from(text: &str) -> Self {
        Buffer::from_slice(text.as_bytes())
    }
}

impl From<String> for Buffer {
    fn from(text: String) -> Self {
        Buffer::from_vec(text.into_bytes())
    }
}

impl std::fmt::Debug for Buffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Avoid dumping the full contents; report the length only.
        f.debug_struct("Buffer").field("len", &self.len()).finish()
    }
}

impl PartialEq for Buffer {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for Buffer {}

impl PartialOrd for Buffer {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Buffer {
    fn cmp(&self, other: &Self) -> Ordering {
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
        serializer.serialize_bytes(self.as_slice())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Buffer {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BufferVisitor;

        impl<'de> serde::de::Visitor<'de> for BufferVisitor {
            type Value = Buffer;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a byte buffer")
            }

            fn visit_bytes<E: serde::de::Error>(self, v: &[u8]) -> Result<Buffer, E> {
                Ok(Buffer::from_slice(v))
            }

            fn visit_byte_buf<E: serde::de::Error>(self, v: Vec<u8>) -> Result<Buffer, E> {
                Ok(Buffer::from_vec(v))
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Buffer, A::Error> {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(byte) = seq.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(Buffer::from_vec(bytes))
            }
        }

        deserializer.deserialize_byte_buf(BufferVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_and_slice_share_the_allocation() {
        let buf = Buffer::from_slice(b"hello world");
        assert_eq!(Arc::strong_count(&buf.data), 1);
        let clone = buf.clone();
        let world = buf.slice(6..);
        // Clone and slice both share the original allocation, no byte copies.
        assert_eq!(Arc::strong_count(&buf.data), 3);
        assert_eq!(world.as_slice(), b"world");
        assert_eq!(clone.as_slice(), b"hello world");
    }

    #[test]
    fn slice_compares_equal_to_a_fresh_buffer() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::Hash;

        let world = Buffer::from_slice(b"hello world").slice(6..);
        let fresh = Buffer::from_slice(b"world");
        assert_eq!(world, fresh);

        let hash = |b: &Buffer| {
            let mut h = DefaultHasher::new();
            b.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&world), hash(&fresh));
    }
}
