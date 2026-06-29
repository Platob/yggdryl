//! The schema layer's error type.

/// Errors raised when converting schema types to or from Apache Arrow.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SchemaError {
    /// An Arrow data type with no yggdryl equivalent.
    UnsupportedArrowType(arrow_schema::DataType),
    /// Field metadata that is not valid UTF-8. Arrow field metadata is
    /// string-keyed, so byte metadata must decode as UTF-8 to convert.
    NonUtf8Metadata,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::UnsupportedArrowType(dtype) => {
                write!(f, "no yggdryl data type matches the Arrow type {dtype:?}")
            }
            SchemaError::NonUtf8Metadata => f.write_str(
                "field metadata keys and values must be valid UTF-8 to convert to Arrow",
            ),
        }
    }
}

impl std::error::Error for SchemaError {}
