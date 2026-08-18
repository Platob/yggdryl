//! Apache Avro: schemas, datums, object containers, and schema resolution.
//!
//! Avro is the exchange format Iceberg keeps its manifests in and one other
//! systems hand over on its own, so it is implemented here as a first-class
//! codec module - a sibling of [`crate::json`] on the byte side and of
//! [`crate::ipc`] on the record side - with no Avro crate underneath, which is
//! what keeps the table format's promise that no dependency is added for the
//! format itself.
//!
//! Rows cross this boundary as the same [`Value`](crate::Value) the JSON
//! parser produces, so an Avro record and a JSON document are read with one
//! vocabulary: a record is a mapping, an array is a sequence, and a union
//! carries the branch value directly rather than a wrapper. Logical types are
//! modeled because the value model is typed - a `date` decodes as a calendar
//! date and a `decimal` keeps its exact unscaled integer - while an annotation
//! this implementation does not know degrades to its underlying type, as the
//! specification requires.
//!
//! Reading has three shapes, cheapest first: [`read_container`] pulls a small
//! self-describing file whole, which is the manifest fast case;
//! [`read_blocks`] streams a large container block by block over nothing but
//! `pread`; and [`read_container_resolved`] decodes through a [`Resolution`] -
//! the writer/reader schema resolution matrix compiled once per pair - so
//! extra writer fields are skipped without being decoded and missing reader
//! fields fill from defaults. [`Schema::fingerprint`] names a schema for
//! caches and for the single-object framing in [`to_single_object_vec`].

mod container;
mod datum;
mod resolve;
mod schema;
mod single;

pub use container::{
    Block, Blocks, Container, read_blocks, read_blocks_with_limits, read_container,
    read_container_resolved, read_container_resolved_with_limits, read_container_with_limits,
    write_container,
};
pub use resolve::Resolution;
pub use schema::{MAX_SCHEMA_DEPTH, Schema};
pub use single::{
    from_single_object_slice, from_single_object_slice_with_limits, to_single_object_vec,
};

#[cfg(test)]
mod tests;
