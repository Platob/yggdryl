//! Crate-internal helpers for the length-prefixed byte encodings.
//!
//! Length prefixes are 8-byte little-endian `u64`s so encoding is total on
//! every platform; strings are UTF-8 payloads behind a length prefix.

use core::fmt;
use core::str;

/// Appends a `u64` little-endian length prefix.
pub(crate) fn put_len(out: &mut Vec<u8>, len: usize) {
    out.extend_from_slice(&(len as u64).to_le_bytes());
}

/// Appends a length-prefixed UTF-8 string.
pub(crate) fn put_str(out: &mut Vec<u8>, value: &str) {
    put_len(out, value.len());
    out.extend_from_slice(value.as_bytes());
}

/// A cursor over an encoded byte slice; every `take_*` validates before
/// advancing.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    /// Starts reading at the beginning of `bytes`.
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    /// Takes the next `len` bytes.
    pub(crate) fn take(&mut self, len: usize) -> Result<&'a [u8], BytesError> {
        if self.bytes.len() < len {
            return Err(BytesError::Truncated {
                needed: len,
                remaining: self.bytes.len(),
            });
        }
        let (taken, rest) = self.bytes.split_at(len);
        self.bytes = rest;
        Ok(taken)
    }

    /// Takes a single byte.
    pub(crate) fn take_u8(&mut self) -> Result<u8, BytesError> {
        Ok(self.take(1)?[0])
    }

    /// Takes a `u64` little-endian length prefix.
    pub(crate) fn take_len(&mut self) -> Result<usize, BytesError> {
        let len = u64::from_le_bytes(self.take(8)?.try_into().expect("take(8) yields 8 bytes"));
        usize::try_from(len).map_err(|_| BytesError::Oversize { length: len })
    }

    /// Takes a length-prefixed byte payload.
    pub(crate) fn take_len_prefixed(&mut self) -> Result<&'a [u8], BytesError> {
        let len = self.take_len()?;
        self.take(len)
    }

    /// Takes a length-prefixed UTF-8 string.
    pub(crate) fn take_str(&mut self) -> Result<&'a str, BytesError> {
        str::from_utf8(self.take_len_prefixed()?).map_err(|_| BytesError::InvalidUtf8)
    }

    /// Ends the read, rejecting trailing bytes.
    pub(crate) fn finish(self) -> Result<(), BytesError> {
        if self.bytes.is_empty() {
            Ok(())
        } else {
            Err(BytesError::Trailing {
                trailing: self.bytes.len(),
            })
        }
    }
}

/// Why a byte slice failed to decode; rendered into the public error types'
/// messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BytesError {
    /// The input ended before a required part.
    Truncated { needed: usize, remaining: usize },
    /// Bytes remained after the last part.
    Trailing { trailing: usize },
    /// A length prefix exceeding this platform's `usize`.
    Oversize { length: u64 },
    /// A string payload that is not valid UTF-8.
    InvalidUtf8,
}

impl fmt::Display for BytesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, remaining } => write!(
                f,
                "truncated input: needed {needed} more bytes, {remaining} remaining — \
                 re-encode the value with to_bytes"
            ),
            Self::Trailing { trailing } => {
                write!(f, "{trailing} trailing bytes after the encoded value")
            }
            Self::Oversize { length } => {
                write!(
                    f,
                    "length prefix {length} does not fit this platform's usize"
                )
            }
            Self::InvalidUtf8 => f.write_str("string payload is not valid UTF-8"),
        }
    }
}
