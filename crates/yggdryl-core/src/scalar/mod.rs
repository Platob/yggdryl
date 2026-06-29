//! Scalar values.
//!
//! The crate's one scalar today is [`Binary`] — a growable, in-memory binary
//! buffer that holds its payload in a shared allocation (O(1) clone, borrowed and
//! zero-copy access) and implements [`Io`](crate::Io). Every scalar round-trips
//! through JSON (the base trait's [`to_json`](Scalar::to_json) /
//! [`from_json`](Scalar::from_json)) and, per type, through a compact binary frame
//! and a component map.

mod binary;

pub use binary::Binary;

use crate::datatype::AnyType;
#[cfg(feature = "json")]
use crate::error::ScalarError;

/// Behaviour shared by every scalar value.
pub trait Scalar {
    /// The scalar's data type.
    fn data_type(&self) -> AnyType;

    /// The JSON form.
    #[cfg(feature = "json")]
    fn to_json(&self) -> String
    where
        Self: Sized + serde::Serialize,
    {
        serde_json::to_string(self).expect("scalar serializes to JSON")
    }

    /// Parses the JSON form produced by [`to_json`](Scalar::to_json).
    #[cfg(feature = "json")]
    fn from_json(value: &str) -> Result<Self, ScalarError>
    where
        Self: Sized + serde::de::DeserializeOwned,
    {
        serde_json::from_str(value).map_err(|err| ScalarError::InvalidEncoding(err.to_string()))
    }
}
