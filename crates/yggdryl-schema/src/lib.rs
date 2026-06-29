//! # yggdryl-schema
//!
//! The Arrow-compatible schema layer for yggdryl. [`DataType`] is the base trait
//! every data type implements — it knows its [`name`](DataType::name) and
//! [`DataTypeId`], from which the physical / logical / nested category follows.
//! [`Field`] pairs a name with a `DataType`, a nullability flag and byte-keyed
//! [`Metadata`], and offers the functional `copy` / `with_*` updates.
//!
//! Conversion to and from Apache Arrow's `arrow-schema` (gated behind the `arrow`
//! feature) and the concrete data-type / field structs land here next, one module
//! per concern, following the rules in `CLAUDE.md`. Add a crate-local `log_event!`
//! macro to this file once a module performs a loggable action.

mod data_type;
mod data_type_id;
mod field;

pub use data_type::DataType;
pub use data_type_id::DataTypeId;
pub use field::{Field, Metadata};
