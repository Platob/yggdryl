//! The [`Binary`] scalar — a growable, in-memory binary buffer that implements
//! [`Io`](crate::Io).

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::buffer::Buffer;
use crate::datatype::{AnyType, BinaryType, DataType};
use crate::error::{IoError, ScalarError};
use crate::io::{Io, Whence};
use crate::mapping::{decode_hex, encode_hex};
use crate::scalar::{AnyScalar, Scalar, Utf8};

/// A growable, in-memory binary value that doubles as an [`Io`](crate::Io) handle.
///
/// `Binary` owns a reference-counted byte store, so cloning is O(1) and reads hand
/// back zero-copy `Binary` views that share the allocation. Writes are
/// copy-on-write: while any view is still outstanding the store is cloned first,
/// so the view stays valid and immutable. Equality, ordering, hashing and
/// serialization use the **content and type only** — the IO cursor and any spare
/// capacity are not part of a value's identity.
///
/// ```
/// use yggdryl_core::{Binary, Io, Whence};
///
/// let mut buf = Binary::new();
/// buf.write(b"hello ").unwrap();
/// buf.write(b"world").unwrap();
/// assert_eq!(buf.size(), 11);
///
/// buf.seek(0, Whence::Start).unwrap();
/// let head = buf.read(5).unwrap(); // zero-copy view
/// assert_eq!(head.as_slice(), b"hello");
/// assert_eq!(buf.as_slice(), b"hello world");
/// ```
#[derive(Clone)]
pub struct Binary {
    data_type: BinaryType,
    store: Arc<[u8]>,
    start: usize,
    size: usize,
    pos: u64,
}

impl Binary {
    /// An empty `binary` buffer.
    pub fn new() -> Self {
        Self {
            data_type: BinaryType::new(),
            store: Arc::from(&[] as &[u8]),
            start: 0,
            size: 0,
            pos: 0,
        }
    }

    /// An empty buffer with at least `capacity` bytes preallocated.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            data_type: BinaryType::new(),
            store: Arc::from(vec![0u8; capacity]),
            start: 0,
            size: 0,
            pos: 0,
        }
    }

    /// A `binary` buffer holding a copy of `bytes` (the inverse of
    /// [`to_bytes`](Binary::to_bytes)).
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self::from_arc(Arc::from(bytes), 0, bytes.len())
    }

    /// A buffer taking ownership of `bytes` (copied into the `Arc` allocation).
    pub fn from_vec(bytes: Vec<u8>) -> Self {
        let size = bytes.len();
        Self::from_arc(Arc::from(bytes), 0, size)
    }

    /// A buffer sharing `buffer`'s bytes without copying.
    pub fn from_buffer(buffer: Buffer) -> Self {
        let (store, start, end) = buffer.as_parts();
        Self::from_arc(Arc::clone(store), start, end - start)
    }

    fn from_arc(store: Arc<[u8]>, start: usize, size: usize) -> Self {
        Self {
            data_type: BinaryType::new(),
            store,
            start,
            size,
            pos: 0,
        }
    }

    /// The buffer's bytes, borrowed without copying.
    pub fn as_slice(&self) -> &[u8] {
        &self.store[self.start..self.start + self.size]
    }

    /// The buffer's bytes as a zero-copy [`Buffer`] view.
    pub fn to_buffer(&self) -> Buffer {
        Buffer::from_shared(Arc::clone(&self.store), self.start, self.start + self.size)
    }

    /// The number of bytes held.
    pub fn len(&self) -> usize {
        self.size
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// The buffer's concrete [`BinaryType`] (`binary` vs `large_binary`).
    pub fn binary_type(&self) -> BinaryType {
        self.data_type
    }

    /// Returns a copy carrying the given `binary` type variant; the payload is
    /// shared, not copied.
    pub fn with_data_type(&self, data_type: BinaryType) -> Self {
        Self {
            data_type,
            ..self.clone()
        }
    }

    /// The buffer's raw bytes as an owned `Vec`. This is the content only; the
    /// `binary` vs `large_binary` variant round-trips through
    /// [`to_mapping`](Binary::to_mapping) / [`to_json`](Scalar::to_json), not the
    /// raw bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }

    /// The component map (`type` plus `value` as hex).
    pub fn to_mapping(&self) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        map.insert("type".to_string(), self.data_type.type_name().to_string());
        map.insert("value".to_string(), encode_hex(self.as_slice()));
        map
    }

    /// Reconstructs a buffer from the component map produced by
    /// [`to_mapping`](Binary::to_mapping).
    pub fn from_mapping(map: &BTreeMap<String, String>) -> Result<Self, ScalarError> {
        let type_name = map
            .get("type")
            .ok_or_else(|| ScalarError::InvalidEncoding("missing \"type\" key".to_string()))?;
        let data_type = BinaryType::from_str(type_name)?;
        let value = match map.get("value") {
            Some(hex) => decode_hex(hex).map_err(ScalarError::InvalidEncoding)?,
            None => Vec::new(),
        };
        let size = value.len();
        Ok(Self {
            data_type,
            store: Arc::from(value),
            start: 0,
            size,
            pos: 0,
        })
    }

    /// Ensures the store holds at least `needed` bytes from offset 0 and is
    /// uniquely owned, returning a mutable view. Clones (copy-on-write) when the
    /// store is shared or offset, so outstanding read views stay valid.
    fn reserve_mut(&mut self, needed: usize) -> &mut [u8] {
        let usable = self.store.len().saturating_sub(self.start);
        let needs_realloc =
            self.start != 0 || usable < needed || Arc::get_mut(&mut self.store).is_none();
        if needs_realloc {
            let new_capacity = needed.max(self.size.saturating_mul(2)).max(8);
            let mut grown = vec![0u8; new_capacity];
            grown[..self.size].copy_from_slice(&self.store[self.start..self.start + self.size]);
            self.store = Arc::from(grown);
            self.start = 0;
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

impl Default for Binary {
    fn default() -> Self {
        Self::new()
    }
}

impl Scalar for Binary {
    fn data_type(&self) -> AnyType {
        self.data_type.to_any()
    }

    fn set_data_type(&mut self, data_type: &dyn DataType) -> Result<(), ScalarError> {
        match data_type.to_any() {
            AnyType::Binary(binary) => {
                self.data_type = binary;
                Ok(())
            }
            other => Err(ScalarError::IncompatibleType(format!(
                "cannot set type \"{}\" on a binary scalar; use cast",
                other.to_str()
            ))),
        }
    }

    fn cast(&self, data_type: &dyn DataType) -> Result<AnyScalar, ScalarError> {
        match data_type.to_any() {
            AnyType::Binary(binary) => Ok(AnyScalar::Binary(self.with_data_type(binary))),
            AnyType::Utf8(utf8) => Ok(AnyScalar::Utf8(
                Utf8::from_bytes(self.as_slice())?.with_data_type(utf8),
            )),
        }
    }
}

impl Io for Binary {
    fn size(&self) -> u64 {
        self.size as u64
    }

    fn capacity(&self) -> u64 {
        self.store.len().saturating_sub(self.start) as u64
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
        let off = self.read_offset(offset)?;
        let n = dst.len().min(self.size - off);
        let from = self.start + off;
        dst[..n].copy_from_slice(&self.store[from..from + n]);
        Ok(n)
    }

    fn pread(&self, offset: u64, len: usize) -> Result<Binary, IoError> {
        let off = self.read_offset(offset)?;
        let n = len.min(self.size - off);
        Ok(Self {
            data_type: self.data_type,
            store: Arc::clone(&self.store),
            start: self.start + off,
            size: n,
            pos: 0,
        })
    }

    fn pwrite(&mut self, offset: u64, src: &[u8]) -> Result<usize, IoError> {
        let off = usize::try_from(offset).map_err(|_| IoError::OutOfBounds {
            offset,
            size: self.size as u64,
        })?;
        let end = off.checked_add(src.len()).ok_or(IoError::OutOfBounds {
            offset,
            size: self.size as u64,
        })?;
        let size = self.size;
        let store = self.reserve_mut(end);
        if off > size {
            store[size..off].fill(0); // zero the gap when writing past the end
        }
        store[off..end].copy_from_slice(src);
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
        grown[..keep].copy_from_slice(&self.store[self.start..self.start + keep]);
        self.store = Arc::from(grown);
        self.start = 0;
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

impl std::fmt::Debug for Binary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Binary")
            .field("type", &self.data_type)
            .field("len", &self.size)
            .finish()
    }
}

impl PartialEq for Binary {
    fn eq(&self, other: &Self) -> bool {
        self.data_type == other.data_type && self.as_slice() == other.as_slice()
    }
}

impl Eq for Binary {}

impl PartialOrd for Binary {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Binary {
    fn cmp(&self, other: &Self) -> Ordering {
        self.data_type
            .cmp(&other.data_type)
            .then_with(|| self.as_slice().cmp(other.as_slice()))
    }
}

impl Hash for Binary {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data_type.hash(state);
        self.as_slice().hash(state);
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Binary {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Binary", 2)?;
        state.serialize_field("type", &self.data_type)?;
        state.serialize_field("value", self.as_slice())?;
        state.end()
    }
}

/// Deserialization shim: rebuilds the buffer from its `type` and byte `value`.
#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct BinaryRepr {
    #[serde(rename = "type")]
    data_type: BinaryType,
    value: Vec<u8>,
}

#[cfg(feature = "serde")]
impl From<BinaryRepr> for Binary {
    fn from(repr: BinaryRepr) -> Self {
        let size = repr.value.len();
        Self {
            data_type: repr.data_type,
            store: Arc::from(repr.value),
            start: 0,
            size,
            pos: 0,
        }
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Binary {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        BinaryRepr::deserialize(deserializer).map(Binary::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_zero_copy_views_with_cow_writes() {
        let mut buf = Binary::from_bytes(b"hello world");
        let view = buf.pread(0, 5).unwrap();
        assert_eq!(Arc::strong_count(&buf.store), 2); // shared, not copied
        assert_eq!(view.as_slice(), b"hello");

        buf.pwrite(0, b"HELLO").unwrap(); // copy-on-write
        assert_eq!(view.as_slice(), b"hello");
        assert_eq!(buf.as_slice(), b"HELLO world");
    }

    #[test]
    fn grows_resizes_and_tracks_capacity() {
        let mut buf = Binary::new();
        assert_eq!(buf.write(b"abc").unwrap(), 3);
        assert!(buf.capacity() >= 3);
        buf.resize(5, b'.').unwrap();
        assert_eq!(buf.as_slice(), b"abc..");
        buf.resize(2, 0).unwrap();
        assert_eq!(buf.as_slice(), b"ab");
    }
}
