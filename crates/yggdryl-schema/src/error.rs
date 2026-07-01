//! The [`SchemaError`].

/// An error raised by the schema types.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemaError {
    /// [`child_field`](crate::NestedFields::child_field) was called with neither an
    /// index nor a name.
    NoChildSelector,
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::NoChildSelector => {
                f.write_str("child_field needs an index or a name — both were omitted")
            }
        }
    }
}

impl std::error::Error for SchemaError {}
