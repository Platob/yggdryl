//! Arrow runtime integration tests.

use yggdryl::{DataType, Field};

/// The non-null Struct root a record's rows live under.
///
/// Every module below builds one, so it is spelled once here rather than three
/// times with three different failure messages.
fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    DataType::from_fields(fields)
        .expect("the root datatype is valid")
        .required_field("row")
}

#[path = "arrow/arrow_value.rs"]
mod arrow_value;
#[path = "arrow/cast_coverage.rs"]
mod cast_coverage;
#[path = "arrow/cast_plan.rs"]
mod cast_plan;
#[path = "arrow/combined.rs"]
mod combined;
#[path = "arrow/row_value.rs"]
mod row_value;
