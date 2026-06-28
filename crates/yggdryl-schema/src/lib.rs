//! # yggdryl-schema
//!
//! A compact schema layer for yggdryl, centred on two types:
//!
//! - [`DataType`] — a value's logical type, split into three [categories](TypeCategory):
//!   [primitive](PrimitiveType) (null, booleans, integers, floats, string, bytes),
//!   [logical](LogicalType) (decimal, the temporal family, JSON/BSON) and
//!   [nested](NestedType) (list, struct, map, union, …). Every type carries a stable
//!   [`type_id`](DataType::type_id) — a [`DataTypeId`] (`u8`) — and a
//!   [`name`](DataType::name).
//! - [`Field`] — a named [`DataType`] with optional byte-keyed metadata and the
//!   reserved [`comment`](Field::comment) / [`index_name`](Field::index_name) /
//!   [`index_level`](Field::index_level) accessors.
//!
//! ```
//! use yggdryl_schema::{DataType, Field, TypeCategory};
//!
//! let mut id = Field::new("id", DataType::int64());
//! id.set_comment(Some("primary key"));
//! let schema = DataType::struct_(vec![id, Field::new("name", DataType::utf8())]);
//! assert_eq!(schema.category(), TypeCategory::Nested);
//! assert_eq!(schema.fields().len(), 2);
//! ```

mod datatype;
mod field;

pub use datatype::{
    DataType, DataTypeId, IntervalUnit, LogicalType, NestedType, PrimitiveType, TypeCategory,
};
pub use field::{Field, Metadata};

// Re-export the shared temporal vocabulary the logical types build on, so dependents
// resolve `yggdryl_schema::{TimeUnit, Timezone}` without a separate core import.
pub use yggdryl_core::{TimeUnit, Timezone};
