//! Encoders and decoders bridging native Rust values — the values an Arrow scalar
//! holds — and the raw bytes a yggdryl [`Scalar`](crate::Scalar) stores.
//!
//! Arrow's array values *are* native Rust types (`Vec<u8>`, `String`, …), so an
//! "Arrow scalar" round-trips through these traits: [`Encode`] writes a value into
//! a scalar's bytes, [`Decode`] reads it back.

use crate::error::ScalarError;

/// Encodes a native Rust value (an Arrow scalar value) into the bytes a scalar
/// stores.
pub trait Encode {
    /// The value's byte form.
    fn encode(&self) -> Vec<u8>;
}

/// Decodes the bytes a scalar stores into a native Rust value (an Arrow scalar
/// value).
pub trait Decode: Sized {
    /// Reads a value from `bytes`.
    fn decode(bytes: &[u8]) -> Result<Self, ScalarError>;
}

impl Encode for [u8] {
    fn encode(&self) -> Vec<u8> {
        self.to_vec()
    }
}

impl Encode for Vec<u8> {
    fn encode(&self) -> Vec<u8> {
        self.clone()
    }
}

impl Encode for str {
    fn encode(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl Encode for String {
    fn encode(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl Decode for Vec<u8> {
    fn decode(bytes: &[u8]) -> Result<Self, ScalarError> {
        Ok(bytes.to_vec())
    }
}

impl Decode for String {
    fn decode(bytes: &[u8]) -> Result<Self, ScalarError> {
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| ScalarError::NonUtf8)
    }
}
