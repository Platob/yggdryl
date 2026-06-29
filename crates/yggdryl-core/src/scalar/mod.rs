//! Scalar values.
//!
//! The crate's scalars are [`Binary`] (a growable, in-memory binary buffer that
//! also implements [`Io`](crate::Io)) and [`Utf8`] (a validated UTF-8 string
//! value). Both hold their payload in a shared allocation (O(1) clone, borrowed
//! access), expose a data-type accessor/mutator and a [`cast`](Scalar::cast) to
//! another type, and round-trip through JSON, a binary/text form and a component
//! map. The type-erased result of a cast is an [`AnyScalar`].

mod any;
mod binary;
mod string;

pub use any::AnyScalar;
pub use binary::Binary;
pub use string::Utf8;

use crate::datatype::{AnyType, DataType};
use crate::error::ScalarError;

/// Behaviour shared by every scalar value.
pub trait Scalar {
    /// The scalar's data type (accessor).
    fn data_type(&self) -> AnyType;

    /// Sets the scalar's data type **in place**, keeping the payload. Errors if
    /// the new type is a different family (e.g. a string type on a binary scalar)
    /// — use [`cast`](Scalar::cast) to convert across families.
    fn set_data_type(&mut self, data_type: &dyn DataType) -> Result<(), ScalarError>;

    /// Casts the value to `data_type`, returning a new [`AnyScalar`]. A
    /// same-family cast only re-labels the variant; a cross-family cast converts
    /// the payload (and may fail, e.g. binary → string on non-UTF-8 bytes).
    fn cast(&self, data_type: &dyn DataType) -> Result<AnyScalar, ScalarError>;

    /// The JSON form.
    #[cfg(feature = "json")]
    fn to_json(&self) -> String
    where
        Self: Sized + serde::Serialize,
    {
        crate::json::render(self)
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
