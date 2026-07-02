//! The [`Base`] trait: content-based JSON serialization for every value type.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::charset::{Charset, Utf8};

mod error;
pub use error::BaseError;

/// The foundational trait every yggdryl value type implements: content-based
/// serialization to and from JSON — as a string and as the canonical byte form.
///
/// Every method has a default implementation built on `serde` and `serde_json`,
/// so a value type opts in with an empty `impl Base for MyType {}` once it derives
/// [`Serialize`](serde::Serialize) and [`Deserialize`](serde::Deserialize):
///
/// - [`serialize_json`](Base::serialize_json) /
///   [`deserialize_json`](Base::deserialize_json) — a JSON string.
/// - [`serialize_bytes`](Base::serialize_bytes) /
///   [`deserialize_bytes`](Base::deserialize_bytes) — the canonical byte form:
///   the JSON string encoded as UTF-8 with the [`Utf8`] charset.
///
/// ```
/// use serde::{Deserialize, Serialize};
/// use yggdryl_core::Base;
///
/// #[derive(Debug, PartialEq, Serialize, Deserialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
/// impl Base for Point {}
///
/// let p = Point { x: 1, y: 2 };
///
/// assert_eq!(p.serialize_json()?, r#"{"x":1,"y":2}"#);
/// assert_eq!(Point::deserialize_json(&p.serialize_json()?)?, p);
///
/// // The canonical byte form is compact UTF-8 JSON.
/// assert_eq!(p.serialize_bytes()?, br#"{"x":1,"y":2}"#.to_vec());
/// assert_eq!(Point::deserialize_bytes(&p.serialize_bytes()?)?, p);
/// # Ok::<(), yggdryl_core::BaseError>(())
/// ```
pub trait Base: Serialize + DeserializeOwned {
    /// Serialize to a compact JSON string.
    fn serialize_json(&self) -> Result<String, BaseError> {
        crate::log_event!(trace, "Base::serialize_json");
        Ok(serde_json::to_string(self)?)
    }

    /// Deserialize from a JSON string.
    fn deserialize_json(json: &str) -> Result<Self, BaseError> {
        crate::log_event!(trace, "Base::deserialize_json");
        Ok(serde_json::from_str(json)?)
    }

    /// Serialize to the canonical byte form: the compact JSON string encoded as
    /// UTF-8.
    fn serialize_bytes(&self) -> Result<Vec<u8>, BaseError> {
        crate::log_event!(trace, "Base::serialize_bytes");
        Ok(Utf8.encode_bytes(&self.serialize_json()?)?)
    }

    /// Deserialize from the canonical byte form: UTF-8-encoded compact JSON.
    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, BaseError> {
        crate::log_event!(trace, "Base::deserialize_bytes");
        Self::deserialize_json(&Utf8.decode_bytes(bytes)?)
    }
}
