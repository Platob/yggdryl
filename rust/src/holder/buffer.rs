//! An auto-scaling in-memory [`IOBase`].

use std::sync::OnceLock;

use crate::iobase::oversized;
use crate::{IOBase, MediaType, MimeType, Result, Url};

/// A growable byte array addressed by offset.
///
/// The allocation doubles rather than growing exactly, so appending in many
/// small writes stays amortized constant. [`IOBase::reserve`] pre-sizes it when
/// the final length is known.
///
/// A buffer has no persisted location. [`IOBase::url`] reports a synthetic
/// `mem:` identity naming this process and the allocation's address, which is
/// enough to tell two live buffers apart in a log or an error without
/// pretending the bytes live anywhere.
///
/// [`IOBase::media_type`] is lazy: unless one is set explicitly, it is inferred
/// from the stored bytes' leading signature the first time it is asked for, and
/// re-inferred after the bytes change.
///
/// ```
/// use yggdryl::{IOBase, holder::Buffer};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut buffer = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
///
/// // The representation comes from the content, not from a filename.
/// assert_eq!(buffer.media_type().base(), &yggdryl::MimeType::JSON);
///
/// // Writing past the end grows the value and zero-fills the gap.
/// buffer.pwrite(20, b"!")?;
/// assert_eq!(buffer.size(), 21);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Default)]
pub struct Buffer {
    bytes: Vec<u8>,
    /// An explicit media type overrides inference when present.
    declared: Option<MediaType>,
    /// Inference result, discarded whenever the bytes change.
    inferred: OnceLock<MediaType>,
    /// Synthetic `mem:` identity, assigned on first use.
    identity: OnceLock<Url>,
}

impl Buffer {
    /// Create an empty buffer with no allocation.
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            declared: None,
            inferred: OnceLock::new(),
            identity: OnceLock::new(),
        }
    }

    /// Create an empty buffer that can hold `capacity` bytes without growing.
    pub fn with_capacity(capacity: usize) -> Self {
        let mut buffer = Self::new();
        buffer.bytes.reserve_exact(capacity);
        buffer
    }

    /// Take ownership of existing bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut buffer = Self::new();
        buffer.bytes = bytes;
        buffer
    }

    /// Return this buffer with an explicit media type, disabling inference.
    ///
    /// Use this when the bytes are a format content cannot identify, such as
    /// CSV or a specific structured-text flavor. `Url::media_type` turns a
    /// location into the value to pass here.
    pub fn with_media_type(mut self, media_type: MediaType) -> Self {
        self.declared = Some(media_type);
        self
    }

    /// Borrow the stored bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    /// Borrow the stored bytes mutably.
    ///
    /// Any inferred media type is discarded, because the caller may change the
    /// content's identity through this handle.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.invalidate();
        &mut self.bytes
    }

    /// Consume the buffer and return its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Discard a cached inference after the bytes change.
    fn invalidate(&mut self) {
        self.inferred.take();
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        // The clone is a distinct allocation, so it gets its own identity and
        // re-infers its own media type.
        Self {
            bytes: self.bytes.clone(),
            declared: self.declared.clone(),
            inferred: OnceLock::new(),
            identity: OnceLock::new(),
        }
    }
}

impl crate::IOMedia for Buffer {
    crate::impl_default_iomedia!();
}

impl IOBase for Buffer {
    fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize> {
        let Ok(offset) = usize::try_from(offset) else {
            // An offset past the address space is simply past the end.
            return Ok(0);
        };
        if offset >= self.bytes.len() {
            return Ok(0);
        }
        let available = &self.bytes[offset..];
        let count = available.len().min(buffer.len());
        buffer[..count].copy_from_slice(&available[..count]);
        Ok(count)
    }

    fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize> {
        let offset = usize::try_from(offset).map_err(|_| oversized(offset))?;
        let end = offset
            .checked_add(bytes.len())
            .ok_or_else(|| oversized(u64::MAX))?;
        if end > self.bytes.len() {
            // Growing zero-fills any gap the offset created.
            self.bytes.resize(end, 0);
        }
        self.bytes[offset..end].copy_from_slice(bytes);
        self.invalidate();
        Ok(bytes.len())
    }

    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn capacity(&self) -> u64 {
        self.bytes.capacity() as u64
    }

    fn reserve(&mut self, capacity: u64) -> Result<()> {
        let capacity = usize::try_from(capacity).map_err(|_| oversized(capacity))?;
        if capacity > self.bytes.capacity() {
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|error| {
                    crate::Error::Io(std::io::Error::other(format!(
                        "unable to reserve {capacity} buffer bytes: {error}"
                    )))
                })?;
        }
        Ok(())
    }

    fn truncate(&mut self, size: u64) -> Result<()> {
        let size = usize::try_from(size).map_err(|_| oversized(size))?;
        if size <= self.bytes.len() {
            self.bytes.truncate(size);
        } else {
            self.bytes.resize(size, 0);
        }
        self.invalidate();
        Ok(())
    }

    fn url(&self) -> Option<&Url> {
        // A buffer is not stored anywhere, so this is an identity rather than
        // a location. `Url` requires a host for non-`file:` schemes, and the
        // process id is the scope that makes an address unique.
        Some(self.identity.get_or_init(|| {
            let text = format!("mem://{}/{:p}", std::process::id(), self.bytes.as_ptr());
            Url::from_str(&text).unwrap_or_else(|_| {
                // Unreachable for a generated identity, but a panic here would
                // turn a diagnostic accessor into a crash.
                Url::from_str("mem://0/0x0").expect("the fallback identity is valid")
            })
        }))
    }

    fn media_type(&self) -> &MediaType {
        if let Some(declared) = &self.declared {
            return declared;
        }
        self.inferred.get_or_init(|| {
            MediaType::from_magic_bytes(&self.bytes)
                .unwrap_or_else(|| MediaType::from(MimeType::OCTET_STREAM))
        })
    }

    fn kind(&self) -> crate::IOKind {
        crate::IOKind::Memory
    }

    fn set_media_type(&mut self, media_type: MediaType) {
        self.declared = Some(media_type);
    }

    /// Discard every byte, keeping the allocation for the next write.
    ///
    /// The buffer still exists afterwards, empty - which for an in-memory
    /// resource is exactly what a cleared leaf is.
    fn clear(&mut self) -> Result<()> {
        self.bytes.clear();
        self.invalidate();
        Ok(())
    }

    /// Delete the buffer completely: the bytes *and* the allocation.
    ///
    /// Complete removal for an in-memory resource means giving the memory back,
    /// not merely forgetting the length - a cleared buffer still holds its
    /// capacity, a removed one holds nothing. `recursive` is irrelevant: a
    /// buffer contains no other resources. The handle stays usable and lazy
    /// afterwards, so writing through it allocates again exactly as a fresh
    /// buffer would.
    fn remove(&mut self, recursive: bool) -> Result<()> {
        let _ = recursive;
        self.bytes = Vec::new();
        self.invalidate();
        Ok(())
    }
}

impl From<Vec<u8>> for Buffer {
    fn from(value: Vec<u8>) -> Self {
        Self::from_bytes(value)
    }
}

impl From<&[u8]> for Buffer {
    fn from(value: &[u8]) -> Self {
        Self::from_bytes(value.to_vec())
    }
}

impl AsRef<[u8]> for Buffer {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}
