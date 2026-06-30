//! # yggdryl-schema
//!
//! The Arrow-compatible schema layer for yggdryl. [`DataType`] is the base trait
//! every data type implements — it knows its [`name`](DataType::name) and
//! [`DataTypeId`], from which the physical / logical / nested category follows;
//! each concrete type also carries the matching marker ([`PhysicalType`],
//! [`LogicalType`] or [`NestedType`]). The binary types ([`BinaryType`],
//! [`LargeBinaryType`], [`BinaryViewType`], [`LargeBinaryViewType`]) are the
//! concrete physical types; the string types ([`StringType`], [`LargeStringType`],
//! [`StringViewType`], [`LargeStringViewType`]) are logical types backed by them,
//! carrying a [`Charset`]. Both families carry an optional `byte_size` cap.
//! [`Field`] pairs a name with a `DataType`, a nullability flag and byte-keyed
//! [`Metadata`], and offers the functional `copy` / `with_*` updates.
//!
//! Conversion to and from Apache Arrow's `arrow-schema` is gated behind the
//! `arrow` feature; because Arrow's type system is narrower, the [`metadata`]
//! strategy stashes what Arrow drops so the exact type round-trips. New types land
//! here one module per concern, following the rules in `CLAUDE.md`.

mod binary;
mod charset;
mod data_type;
mod data_type_id;
#[cfg(feature = "arrow")]
mod error;
mod field;
pub mod metadata;
mod string;

pub use binary::{BinaryType, BinaryViewType, LargeBinaryType, LargeBinaryViewType};
pub use charset::Charset;
pub use data_type::{DataType, LogicalType, NestedType, PhysicalType};
pub use data_type_id::DataTypeId;
#[cfg(feature = "arrow")]
pub use error::SchemaError;
pub use field::Field;
pub use metadata::Metadata;
pub use string::{LargeStringType, LargeStringViewType, StringType, StringViewType};
