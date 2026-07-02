//! The [`Base`] trait: content-based JSON serialization for every value type.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::charset::Charset;

mod error;
pub use error::BaseError;

/// The foundational trait every yggdryl value type implements: content-based
/// serialization to and from JSON — as a string, as encoded bytes, and as the
/// canonical byte form.
///
/// Every method has a default implementation built on `serde` and `serde_json`,
/// so a value type opts in with an empty `impl Base for MyType {}` once it derives
/// [`Serialize`](serde::Serialize) and [`Deserialize`](serde::Deserialize):
///
/// - [`to_json`](Base::to_json) / [`from_json`](Base::from_json) — a JSON string.
/// - [`to_bson`](Base::to_bson) / [`from_bson`](Base::from_bson) — JSON bytes,
///   optionally indented, encoded with a [`Charset`].
/// - [`to_bytes`](Base::to_bytes) / [`from_bytes`](Base::from_bytes) — the
///   canonical byte form: compact UTF-8 JSON.
///
/// ```
/// use serde::{Deserialize, Serialize};
/// use yggdryl_core::{Base, Charset};
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
/// assert_eq!(p.to_json()?, r#"{"x":1,"y":2}"#);
/// assert_eq!(Point::from_json(&p.to_json()?)?, p);
///
/// // JSON bytes, pretty-printed with a two-space indent, encoded as UTF-8.
/// let bytes = p.to_bson(Some(2), Charset::Utf8)?;
/// assert_eq!(Point::from_bson(&bytes, Charset::Utf8)?, p);
///
/// // The canonical byte form is compact UTF-8 JSON.
/// assert_eq!(p.to_bytes()?, br#"{"x":1,"y":2}"#.to_vec());
/// assert_eq!(Point::from_bytes(&p.to_bytes()?)?, p);
/// # Ok::<(), yggdryl_core::BaseError>(())
/// ```
pub trait Base: Serialize + DeserializeOwned {
    /// Serialize to a compact JSON string.
    fn to_json(&self) -> Result<String, BaseError> {
        crate::log_event!(trace, "Base::to_json");
        Ok(serde_json::to_string(self)?)
    }

    /// Deserialize from a JSON string.
    fn from_json(json: &str) -> Result<Self, BaseError> {
        crate::log_event!(trace, "Base::from_json");
        Ok(serde_json::from_str(json)?)
    }

    /// Serialize to JSON bytes: pretty-printed with `indent` spaces when `indent`
    /// is `Some`, compact when `None`, then encoded with `charset`.
    fn to_bson(&self, indent: Option<usize>, charset: Charset) -> Result<Vec<u8>, BaseError> {
        crate::log_event!(debug, "Base::to_bson indent={indent:?} charset={charset:?}");
        let json = match indent {
            Some(width) => to_pretty_json(self, width)?,
            None => self.to_json()?,
        };
        Ok(charset.encode(&json)?)
    }

    /// Deserialize from JSON bytes decoded with `charset`.
    fn from_bson(bytes: &[u8], charset: Charset) -> Result<Self, BaseError> {
        crate::log_event!(debug, "Base::from_bson charset={charset:?}");
        let json = charset.decode(bytes)?;
        Self::from_json(&json)
    }

    /// Serialize to the canonical byte form: compact UTF-8 JSON.
    fn to_bytes(&self) -> Result<Vec<u8>, BaseError> {
        crate::log_event!(trace, "Base::to_bytes");
        self.to_bson(None, Charset::Utf8)
    }

    /// Deserialize from the canonical byte form: compact UTF-8 JSON.
    fn from_bytes(bytes: &[u8]) -> Result<Self, BaseError> {
        crate::log_event!(trace, "Base::from_bytes");
        Self::from_bson(bytes, Charset::Utf8)
    }
}

/// Serialize `value` to pretty-printed JSON indented with `indent` spaces.
fn to_pretty_json<T: Serialize + ?Sized>(value: &T, indent: usize) -> Result<String, BaseError> {
    let spaces = vec![b' '; indent];
    let formatter = serde_json::ser::PrettyFormatter::with_indent(&spaces);
    let mut buf = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut buf, formatter);
    value.serialize(&mut serializer)?;
    Ok(String::from_utf8(buf).expect("serde_json always emits valid UTF-8"))
}
