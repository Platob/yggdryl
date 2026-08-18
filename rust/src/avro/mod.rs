//! Apache Avro object containers over any byte handle.
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
//! carries the branch value directly rather than a wrapper, because that is
//! how an optional field is meant to read.
//!
//! [`read_container`] reads a container whole, which is the right shape for
//! the small self-describing files Avro is usually asked for - an Iceberg
//! manifest describes files, not rows, so it is small by construction. The
//! streaming and columnar shapes this module grows are additions beside that
//! path, never a replacement for it.

mod container;
mod datum;
mod schema;

pub use container::{Container, read_container, write_container};

#[cfg(test)]
mod tests;
