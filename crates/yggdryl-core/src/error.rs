//! Error types for the data-type, scalar and field layers.
//!
//! Each concern carries its own `enum` implementing [`Display`](std::fmt::Display)
//! and [`std::error::Error`], with `From` conversions so a lower-level failure
//! bubbles up unchanged. Messages are actionable: they name the offending value
//! and, where the fix is knowable, the expected input.

use std::fmt;

/// Failure while constructing or parsing an Arrow [`DataType`](crate::DataType).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TypeError {
    /// The canonical type name is not one this crate knows.
    UnknownType(String),
    /// A component mapping was missing a key or held an unusable value.
    InvalidMapping(String),
    /// Byte input could not be read as a UTF-8 type name.
    InvalidUtf8,
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TypeError::UnknownType(name) => write!(
                f,
                "unknown data type {name:?}; expected one of: \
                 binary, large_binary, string, large_string"
            ),
            TypeError::InvalidMapping(msg) => write!(f, "invalid type mapping: {msg}"),
            TypeError::InvalidUtf8 => write!(f, "type name was not valid UTF-8"),
        }
    }
}

impl std::error::Error for TypeError {}

/// Failure while constructing a [`Scalar`](crate::Scalar) value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScalarError {
    /// Bytes handed to a string scalar were not valid UTF-8.
    InvalidUtf8,
    /// A scalar's serialized bytes could not be decoded back into a value.
    InvalidEncoding(String),
    /// The scalar's data type could not be resolved.
    Type(TypeError),
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScalarError::InvalidUtf8 => {
                write!(f, "string scalar bytes were not valid UTF-8")
            }
            ScalarError::InvalidEncoding(msg) => write!(f, "invalid scalar encoding: {msg}"),
            ScalarError::Type(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ScalarError {}

impl From<TypeError> for ScalarError {
    fn from(err: TypeError) -> Self {
        ScalarError::Type(err)
    }
}

/// Failure while constructing a [`Field`](crate::Field) or decoding its mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldError {
    /// A required key (`name` or `type`) was absent from the mapping.
    MissingKey(&'static str),
    /// A mapping value was malformed (e.g. a non-boolean `nullable`).
    InvalidMapping(String),
    /// The field's data type failed to parse.
    Type(TypeError),
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldError::MissingKey(key) => {
                write!(f, "field mapping is missing required key {key:?}")
            }
            FieldError::InvalidMapping(msg) => write!(f, "invalid field mapping: {msg}"),
            FieldError::Type(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for FieldError {}

impl From<TypeError> for FieldError {
    fn from(err: TypeError) -> Self {
        FieldError::Type(err)
    }
}

/// Failure while reading from or writing to an [`Io`](crate::Io) handle.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IoError {
    /// A positional access fell outside the handle's valid range.
    OutOfBounds {
        /// The requested absolute offset.
        offset: u64,
        /// The handle's current size.
        size: u64,
    },
    /// A seek resolved to a negative position.
    InvalidSeek(String),
    /// The handle does not support the requested operation (e.g. a read-only
    /// source asked to resize).
    Unsupported(&'static str),
}

impl fmt::Display for IoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoError::OutOfBounds { offset, size } => write!(
                f,
                "offset {offset} is out of bounds for an IO of size {size}"
            ),
            IoError::InvalidSeek(msg) => write!(f, "invalid seek: {msg}"),
            IoError::Unsupported(op) => {
                write!(f, "this IO handle does not support {op}")
            }
        }
    }
}

impl std::error::Error for IoError {}
