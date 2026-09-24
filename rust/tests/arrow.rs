//! Arrow runtime integration tests.

use yggdryl::{DataType, Field, StructType};

/// The non-null Struct root a record's rows live under.
///
/// Every module below builds one, so it is spelled once here rather than three
/// times with three different failure messages.
fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    StructType::from_fields(fields)
        .map(DataType::from)
        .expect("the root datatype is valid")
        .required_field("row")
}

#[path = "arrow/mod_.rs"]
mod mod_;
#[path = "arrow/rows.rs"]
mod rows;
