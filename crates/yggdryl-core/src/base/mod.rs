//! The [`Base`] trait: JSON and byte serialization for every value type.

use serde::de::DeserializeOwned;
use serde::Serialize;

mod error;
pub use error::BaseError;

/// The foundational trait every yggdryl value type implements.
///
/// The JSON string form is free from `serde`: [`serialize_json`](Base::serialize_json)
/// / [`deserialize_json`](Base::deserialize_json) have default implementations, so a
/// value type gets them by deriving [`Serialize`](serde::Serialize) and
/// [`Deserialize`](serde::Deserialize). The canonical byte form —
/// [`serialize_bytes`](Base::serialize_bytes) /
/// [`deserialize_bytes`](Base::deserialize_bytes) — is a compact binary layout each
/// type defines itself (never JSON); `deserialize_bytes` validates its input fully
/// and is the exact inverse of `serialize_bytes`.
///
/// ```
/// use serde::{Deserialize, Serialize};
/// use yggdryl_core::{Base, BaseError};
///
/// #[derive(Debug, PartialEq, Serialize, Deserialize)]
/// struct Point {
///     x: i32,
///     y: i32,
/// }
///
/// impl Base for Point {
///     fn serialize_bytes(&self) -> Result<Vec<u8>, BaseError> {
///         let mut out = Vec::with_capacity(8);
///         out.extend_from_slice(&self.x.to_le_bytes());
///         out.extend_from_slice(&self.y.to_le_bytes());
///         Ok(out)
///     }
///
///     fn deserialize_bytes(bytes: &[u8]) -> Result<Self, BaseError> {
///         let a: [u8; 8] = bytes.try_into().map_err(|_| BaseError::InvalidBytes {
///             reason: format!("expected 8 bytes, got {}", bytes.len()),
///         })?;
///         Ok(Point {
///             x: i32::from_le_bytes([a[0], a[1], a[2], a[3]]),
///             y: i32::from_le_bytes([a[4], a[5], a[6], a[7]]),
///         })
///     }
/// }
///
/// let p = Point { x: 1, y: 2 };
///
/// // Content JSON is free from serde.
/// assert_eq!(p.serialize_json()?, r#"{"x":1,"y":2}"#);
/// assert_eq!(Point::deserialize_json(&p.serialize_json()?)?, p);
///
/// // The byte form is the type's own compact binary layout — not JSON.
/// assert_eq!(p.serialize_bytes()?, vec![1, 0, 0, 0, 2, 0, 0, 0]);
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

    /// Serialize to the canonical byte form: a compact binary layout of the type's
    /// choosing.
    fn serialize_bytes(&self) -> Result<Vec<u8>, BaseError>;

    /// Deserialize from the canonical byte form, validating the input fully.
    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, BaseError>;
}
