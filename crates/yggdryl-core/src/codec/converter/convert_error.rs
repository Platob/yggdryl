//! [`ConvertError`] — the failure modes of a [`Converter`](crate::Converter).

use core::fmt;

/// An error raised while converting between representations.
///
/// Every [`Converter`](crate::Converter) / [`TypedConverter`](crate::TypedConverter)
/// reports failures through this one enum, so callers handle conversion errors
/// uniformly regardless of the concrete converter. Each message names the remedy —
/// the accepted formats, the expected width, or the offending input — so the fix is
/// knowable from the error alone. In the bindings it surfaces as a Python
/// `ValueError` / a thrown `Error`.
///
/// ```
/// use yggdryl_core::ConvertError;
///
/// let err = ConvertError::InvalidByteLength { len: 6, width: 4 };
/// assert!(err.to_string().contains("multiple of 4"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConvertError {
    /// A byte array whose length is not a whole number of source elements. Pass a
    /// length that is a multiple of `width`.
    InvalidByteLength {
        /// The offending byte length.
        len: usize,
        /// The source element width the length must be a multiple of.
        width: usize,
    },
    /// A string that no accepted format could parse into the target type. The
    /// `expected` field lists the formats that would have worked.
    ParseFailed {
        /// The offending input (truncated to a reasonable length for the message).
        input: String,
        /// The target type name, e.g. `"i32"`.
        target: &'static str,
        /// A short description of the accepted formats.
        expected: &'static str,
    },
    /// A value whose format was valid but which did not fit the target type's range
    /// (e.g. `"99999999999"` into `i32`). Pass a value within `min..=max`.
    OutOfRange {
        /// The offending value (truncated to a reasonable length for the message).
        input: String,
        /// The target type name, e.g. `"i32"`.
        target: &'static str,
        /// The lowest value the target accepts.
        min: String,
        /// The highest value the target accepts.
        max: String,
    },
    /// A byte array that is not valid UTF-8, so it cannot decode to a string.
    InvalidUtf8 {
        /// The byte offset at which decoding failed.
        valid_up_to: usize,
    },
    /// A dtype name that does not name a known primitive. Pass one of `expected`.
    UnknownType {
        /// The offending name.
        name: String,
        /// The accepted dtype names.
        expected: &'static str,
    },
}

impl fmt::Display for ConvertError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByteLength { len, width } => write!(
                f,
                "byte length {len} is not a multiple of {width}; \
                 pass a whole number of {width}-byte source elements"
            ),
            Self::ParseFailed {
                input,
                target,
                expected,
            } => write!(f, "cannot parse {input:?} as {target}; expected {expected}"),
            Self::OutOfRange {
                input,
                target,
                min,
                max,
            } => write!(
                f,
                "value {input:?} is out of range for {target}; expected {min}..={max}"
            ),
            Self::InvalidUtf8 { valid_up_to } => write!(
                f,
                "invalid UTF-8 at byte {valid_up_to}; pass valid UTF-8 bytes"
            ),
            Self::UnknownType { name, expected } => {
                write!(f, "unknown dtype {name:?}; expected one of {expected}")
            }
        }
    }
}

impl std::error::Error for ConvertError {}
