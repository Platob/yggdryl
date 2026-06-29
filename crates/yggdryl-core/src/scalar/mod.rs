//! Scalar values — a single, typed cell of data.
//!
//! Every scalar carries its [`AnyType`] and may be *null*. The byte-backed
//! scalars ([`BinaryScalar`], [`StringScalar`]) hold their payload in a
//! [`Buffer`](crate::Buffer), so cloning is O(1) and the bytes are borrowed, not
//! copied, by their accessors. Each scalar round-trips through JSON (the base
//! trait's [`to_json`](Scalar::to_json)/[`from_json`](Scalar::from_json)) and
//! through a compact binary frame and component map (per-type `to_bytes` /
//! `to_mapping`).

mod binary;
mod string;

pub use binary::BinaryScalar;
pub use string::StringScalar;

use crate::datatype::AnyType;
#[cfg(feature = "json")]
use crate::error::ScalarError;

/// Behaviour shared by every scalar value.
pub trait Scalar {
    /// The scalar's data type.
    fn data_type(&self) -> AnyType;

    /// Whether the scalar holds the null value.
    fn is_null(&self) -> bool;

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
