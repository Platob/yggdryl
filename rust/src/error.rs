use std::fmt;

use smol_str::SmolStr;

/// An error produced by schema, resource-identifier, Arrow, or byte-codec operations.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Metadata keys must not be empty.
    EmptyMetadataKey,
    /// A bulk metadata input contained a duplicate key.
    DuplicateMetadataKey(SmolStr),
    /// A reserved metadata value violates its typed contract.
    InvalidMetadataValue {
        /// The reserved metadata key.
        key: SmolStr,
        /// A concise validation failure.
        reason: SmolStr,
    },
    /// A scalar datatype name was not recognized.
    UnknownDataType(SmolStr),
    /// A datatype parameter violates the Arrow logical-type contract.
    InvalidDataType {
        /// The datatype being constructed.
        kind: &'static str,
        /// A concise validation failure.
        reason: SmolStr,
    },
    /// A schema-bound record value violates its Arrow field contract.
    InvalidRecord {
        /// Dot/bracket path to the failing value.
        path: SmolStr,
        /// A concise validation failure.
        reason: SmolStr,
    },
    /// Numeric operands do not define the requested checked operation.
    InvalidArithmetic {
        /// The operator name, such as `addition` or `negation`.
        operation: &'static str,
        /// The left or unary operand kind.
        left: &'static str,
        /// The right operand kind for a binary operation.
        right: Option<&'static str>,
        /// Why otherwise numeric operands could not be combined.
        reason: SmolStr,
    },
    /// A checked arithmetic result does not fit its promoted native kind.
    ArithmeticOverflow {
        /// The operator name.
        operation: &'static str,
        /// The promoted result kind.
        kind: &'static str,
    },
    /// Division or remainder was requested with a numeric zero divisor.
    DivisionByZero {
        /// `division` or `remainder`.
        operation: &'static str,
    },
    /// Exact decimal division cannot represent the quotient at its result scale.
    InexactArithmetic {
        /// The operator name.
        operation: &'static str,
        /// The promoted result kind.
        kind: &'static str,
    },
    /// A textual schema expression could not be parsed completely.
    Parse {
        /// The value being parsed, such as `datatype` or `field`.
        target: &'static str,
        /// Byte offset at which parsing stopped.
        position: usize,
        /// A concise description of the expected syntax.
        reason: SmolStr,
    },
    /// JSON serialization or deserialization failed.
    Json(serde_json::Error),
    /// A byte-oriented data codec rejected input or output.
    Codec {
        /// The codec being used, such as `json` or `yaml`.
        format: &'static str,
        /// Byte offset at or immediately after the failure.
        position: usize,
        /// A concise description of the failure.
        reason: SmolStr,
    },
    /// A resource an operation addressed is not there.
    ///
    /// This is the typed absence the existence contract branches on: act
    /// first, then read the failure. A backend that spells absence its own
    /// way (`NotFound`, `NoSuchKey`, a 404) is normalized into this at its own
    /// boundary, once, so no caller matches on a message.
    Absent {
        /// What the operation expected to find, such as `table` or `file`.
        expected: &'static str,
        /// The location addressed, rendered canonically.
        path: SmolStr,
    },
    /// A resource an operation meant to create is already there.
    ///
    /// `create` reports this from the attempt, never from a probe;
    /// `open_or_create` is that same attempt with this absorbed.
    Conflict {
        /// What the operation meant to create, such as `table`.
        expected: &'static str,
        /// What is already at that location.
        actual: &'static str,
        /// The location addressed, rendered canonically.
        path: SmolStr,
    },
    /// Reading or writing codec bytes failed.
    Io(std::io::Error),
    /// An Arrow schema value could not be converted.
    Arrow(arrow_schema::ArrowError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMetadataKey => formatter.write_str("metadata keys must not be empty"),
            Self::DuplicateMetadataKey(key) => {
                write!(formatter, "duplicate metadata key {key:?}")
            }
            Self::InvalidMetadataValue { key, reason } => {
                write!(formatter, "invalid metadata value for {key:?}: {reason}")
            }
            Self::UnknownDataType(name) => write!(formatter, "unknown datatype {name:?}"),
            Self::InvalidDataType { kind, reason } => {
                write!(formatter, "invalid {kind} datatype: {reason}")
            }
            Self::InvalidRecord { path, reason } => {
                write!(formatter, "invalid record value at {path}: {reason}")
            }
            Self::InvalidArithmetic {
                operation,
                left,
                right,
                reason,
            } => match right {
                Some(right) => write!(
                    formatter,
                    "invalid {operation} for {left} and {right}: {reason}"
                ),
                None => write!(formatter, "invalid {operation} for {left}: {reason}"),
            },
            Self::ArithmeticOverflow { operation, kind } => {
                write!(formatter, "{operation} overflows {kind}")
            }
            Self::DivisionByZero { operation } => {
                write!(formatter, "{operation} by zero")
            }
            Self::InexactArithmetic { operation, kind } => {
                write!(formatter, "{operation} has no exact {kind} result")
            }
            Self::Parse {
                target,
                position,
                reason,
            } => write!(
                formatter,
                "invalid {target} expression at byte {position}: {reason}"
            ),
            Self::Json(error) => write!(formatter, "invalid schema JSON: {error}"),
            Self::Codec {
                format,
                position,
                reason,
            } => write!(
                formatter,
                "invalid {format} data at byte {position}: {reason}"
            ),
            Self::Absent { expected, path } => {
                write!(formatter, "expected a {expected} at {path:?}, got nothing")
            }
            Self::Conflict {
                expected,
                actual,
                path,
            } => write!(
                formatter,
                "expected to create a {expected} at {path:?}, got an existing {actual}"
            ),
            Self::Io(error) => write!(formatter, "codec I/O error: {error}"),
            Self::Arrow(error) => write!(formatter, "Arrow schema error: {error}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Arrow(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<arrow_schema::ArrowError> for Error {
    fn from(value: arrow_schema::ArrowError) -> Self {
        Self::Arrow(value)
    }
}

impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<std::convert::Infallible> for Error {
    fn from(value: std::convert::Infallible) -> Self {
        match value {}
    }
}

impl Error {
    /// Report that nothing is at `path` where a `expected` was addressed.
    pub fn absent(expected: &'static str, path: impl fmt::Display) -> Self {
        Self::Absent {
            expected,
            path: SmolStr::new(path.to_string()),
        }
    }

    /// Report that `actual` is already at `path` where an `expected` was created.
    pub fn conflict(expected: &'static str, actual: &'static str, path: impl fmt::Display) -> Self {
        Self::Conflict {
            expected,
            actual,
            path: SmolStr::new(path.to_string()),
        }
    }

    /// Normalize a backend's own absence and conflict spellings into the typed
    /// variants, at that backend's boundary.
    ///
    /// Everything else keeps the failure it already is: a permission or network
    /// error is neither an absence nor a conflict, and widening it into one
    /// would make a caller repair something that was never missing.
    pub fn from_io_at(
        error: std::io::Error,
        expected: &'static str,
        path: impl fmt::Display,
    ) -> Self {
        match error.kind() {
            std::io::ErrorKind::NotFound => Self::absent(expected, path),
            std::io::ErrorKind::AlreadyExists => Self::conflict(expected, expected, path),
            _ => Self::Io(error),
        }
    }

    /// Return whether this failure says the addressed resource is not there.
    ///
    /// [`Self::Absent`] is what backends normalize into; an [`Self::Io`] error
    /// still spelling [`std::io::ErrorKind::NotFound`] answers the same, so a
    /// backend whose boundary has not been normalized yet can never make a
    /// caller branch the wrong way.
    #[must_use]
    pub fn is_absent(&self) -> bool {
        match self {
            Self::Absent { .. } => true,
            Self::Io(error) => error.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }

    /// Return whether this failure says the addressed resource is already there.
    ///
    /// Reads [`Self::Io`]'s [`std::io::ErrorKind::AlreadyExists`] for the same
    /// reason [`Self::is_absent`] reads `NotFound`.
    #[must_use]
    pub fn is_conflict(&self) -> bool {
        match self {
            Self::Conflict { .. } => true,
            Self::Io(error) => error.kind() == std::io::ErrorKind::AlreadyExists,
            _ => false,
        }
    }

    /// Return whether checked division or remainder received a zero divisor.
    #[must_use]
    pub const fn is_division_by_zero(&self) -> bool {
        matches!(self, Self::DivisionByZero { .. })
    }
}

/// The result type returned by Yggdryl core operations.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::Error;

    #[test]
    fn an_absence_names_what_was_expected_and_where() {
        let error = Error::absent("table", "warehouse/sales/orders");
        assert_eq!(
            error.to_string(),
            "expected a table at \"warehouse/sales/orders\", got nothing"
        );
        assert!(error.is_absent());
        assert!(!error.is_conflict());
    }

    #[test]
    fn a_conflict_names_both_sides_and_where() {
        let error = Error::conflict("table", "namespace", "warehouse/sales");
        assert_eq!(
            error.to_string(),
            "expected to create a table at \"warehouse/sales\", got an existing namespace"
        );
        assert!(error.is_conflict());
        assert!(!error.is_absent());
    }

    #[test]
    fn a_backend_spelling_normalizes_into_the_typed_variant() {
        let absent = Error::from_io_at(
            std::io::Error::from(std::io::ErrorKind::NotFound),
            "file",
            "/tmp/missing",
        );
        assert!(matches!(absent, Error::Absent { .. }));

        let conflict = Error::from_io_at(
            std::io::Error::from(std::io::ErrorKind::AlreadyExists),
            "file",
            "/tmp/taken",
        );
        assert!(matches!(conflict, Error::Conflict { .. }));
    }

    #[test]
    fn a_permission_failure_is_neither_an_absence_nor_a_conflict() {
        let denied = Error::from_io_at(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            "file",
            "/tmp/locked",
        );
        assert!(matches!(denied, Error::Io(_)));
        assert!(!denied.is_absent());
        assert!(!denied.is_conflict());
    }

    #[test]
    fn an_unnormalized_backend_answer_still_branches_the_same_way() {
        // A boundary that has not been normalized yet must not make a caller
        // repair something that was never missing, nor miss a real absence.
        let absent = Error::Io(std::io::Error::from(std::io::ErrorKind::NotFound));
        assert!(absent.is_absent());
        let conflict = Error::Io(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
        assert!(conflict.is_conflict());
    }
}
