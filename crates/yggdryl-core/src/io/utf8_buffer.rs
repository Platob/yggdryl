//! The [`Utf8Buffer`] in-memory UTF-8 string resource.

use super::{ByteBuffer, IOBase, IOError, RawIOBase, Whence};

/// A growable in-memory UTF-8 string buffer — the string counterpart of
/// [`ByteBuffer`], holding its content as UTF-8 bytes.
///
/// Storing the bytes (rather than a `Vec<char>`) makes it *like* a byte buffer: it
/// carries the same positioned byte- and bit-[`RawIOBase`] surface, and its bytes
/// bridge zero-copy to Arrow's `utf8`. On top of that it adds a typed
/// [`IOBase<char>`] view — writing a `char` appends its UTF-8 encoding, and
/// [`size`](IOBase::size) counts Unicode scalar values (`char`s) rather than bytes.
/// Because UTF-8 is variable-width, [`char_len`](Utf8Buffer::char_len) scans the
/// content and the typed streaming helpers are exact only for single-byte (ASCII)
/// text; raw byte writes may leave the bytes non-UTF-8, so
/// [`as_str`](Utf8Buffer::as_str) validates and returns
/// [`IOError::InvalidUtf8`] rather than assuming.
///
/// ```
/// use yggdryl_core::{IOBase, RawIOBase, Utf8Buffer, Whence};
///
/// let mut text = Utf8Buffer::from("hé");
/// assert_eq!(text.as_str().unwrap(), "hé");
/// assert_eq!(text.byte_size(), 3); // 'h' = 1 byte, 'é' = 2 bytes
/// assert_eq!(IOBase::<char>::size(&text), 2); // ...but two chars
///
/// // The typed char view: append a char as its UTF-8 bytes.
/// text.pwrite_one(text.byte_size(), Whence::Start, &'!')?;
/// assert_eq!(text.as_str().unwrap(), "hé!");
///
/// // The raw byte / bit surface is a ByteBuffer's, over the UTF-8 bytes.
/// assert_eq!(text.pread_byte_one(0, Whence::Start)?, b'h');
/// # Ok::<(), yggdryl_core::IOError>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Utf8Buffer {
    bytes: ByteBuffer,
}

impl Utf8Buffer {
    /// An empty string buffer.
    pub fn new() -> Self {
        Self {
            bytes: ByteBuffer::new(),
        }
    }

    /// A buffer over the UTF-8 bytes of `value`.
    pub fn from_string(value: String) -> Self {
        Self {
            bytes: ByteBuffer::from_bytes(value.into_bytes()),
        }
    }

    /// The buffer's raw UTF-8 bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    /// The content as a `&str`, or [`IOError::InvalidUtf8`] when a raw byte write has
    /// left the bytes non-UTF-8.
    pub fn as_str(&self) -> Result<&str, IOError> {
        std::str::from_utf8(self.as_bytes()).map_err(|error| IOError::InvalidUtf8 {
            offset: error.valid_up_to(),
        })
    }

    /// Consume the buffer, returning its content as an owned `String`, or
    /// [`IOError::InvalidUtf8`] when the bytes are not valid UTF-8.
    pub fn into_string(self) -> Result<String, IOError> {
        String::from_utf8(self.bytes.into_bytes()).map_err(|error| IOError::InvalidUtf8 {
            offset: error.utf8_error().valid_up_to(),
        })
    }

    /// The number of `char`s (Unicode scalar values) in the content, or the byte
    /// length as a fallback when the bytes are not valid UTF-8.
    pub fn char_len(&self) -> usize {
        std::str::from_utf8(self.as_bytes())
            .map(|text| text.chars().count())
            .unwrap_or_else(|_| self.as_bytes().len())
    }

    /// Whether the buffer holds no content.
    pub fn is_empty(&self) -> bool {
        self.as_bytes().is_empty()
    }
}

impl From<String> for Utf8Buffer {
    fn from(value: String) -> Self {
        Self::from_string(value)
    }
}

impl From<&str> for Utf8Buffer {
    fn from(value: &str) -> Self {
        Self::from_string(value.to_string())
    }
}

// The raw surface delegates to the inner byte buffer: a Utf8Buffer is a byte
// buffer over UTF-8 bytes, so it borrows every positioned byte / bit method.
impl RawIOBase for Utf8Buffer {
    fn byte_size(&self) -> usize {
        self.bytes.byte_size()
    }

    fn byte_capacity(&self) -> usize {
        self.bytes.byte_capacity()
    }

    fn resize_byte_capacity(&mut self, capacity: usize) -> Result<usize, IOError> {
        self.bytes.resize_byte_capacity(capacity)
    }

    fn resize_bytes(&mut self, size: usize) -> Result<(), IOError> {
        self.bytes.resize_bytes(size)
    }

    fn pread_byte_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<u8>, IOError> {
        self.bytes.pread_byte_array(position, whence, size)
    }

    fn pwrite_byte_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[u8],
    ) -> Result<(), IOError> {
        self.bytes.pwrite_byte_array(position, whence, values)
    }

    fn pread_bit_array(
        &self,
        position: usize,
        whence: Whence,
        size: usize,
    ) -> Result<Vec<bool>, IOError> {
        self.bytes.pread_bit_array(position, whence, size)
    }

    fn pwrite_bit_array(
        &mut self,
        position: usize,
        whence: Whence,
        values: &[bool],
    ) -> Result<(), IOError> {
        self.bytes.pwrite_bit_array(position, whence, values)
    }
}

// The typed char view: a `char` becomes its UTF-8 bytes, `size` counts chars.
impl IOBase<char> for Utf8Buffer {
    fn value_to_bytes(&self, value: &char) -> Vec<u8> {
        let mut buffer = [0u8; 4];
        value.encode_utf8(&mut buffer).as_bytes().to_vec()
    }

    fn size(&self) -> usize {
        self.char_len()
    }

    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        // Resizing is char-based, so the content must be valid UTF-8: truncate at the
        // byte boundary of the `size`-th char, or pad with NUL chars (one byte each).
        let byte_end = {
            let text = self.as_str()?;
            let current = text.chars().count();
            if size <= current {
                text.char_indices()
                    .nth(size)
                    .map(|(offset, _)| offset)
                    .unwrap_or_else(|| text.len())
            } else {
                text.len() + (size - current)
            }
        };
        self.bytes.resize_bytes(byte_end)
    }
}
