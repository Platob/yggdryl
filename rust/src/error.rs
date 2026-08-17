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

/// The result type returned by Yggdryl core operations.
pub type Result<T> = std::result::Result<T, Error>;
