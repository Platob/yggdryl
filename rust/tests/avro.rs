//! One test file per file under `rust/src/avro/`, under `tests/avro/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and this target is the harness for the Avro codec's
//! own files. A test reaches the crate through `yggdryl::`; where what it pins
//! is not reachable that way, it reaches `yggdryl::internals`, which exists
//! only under the `internals` feature, and the file that reaches it is
//! declared behind that feature here.

#[path = "avro/arrow.rs"]
mod arrow;
#[path = "avro/batch.rs"]
mod batch;
#[path = "avro/container.rs"]
mod container;
#[path = "avro/datum.rs"]
mod datum;
#[path = "avro/mod_.rs"]
mod mod_;
#[path = "avro/resolve.rs"]
mod resolve;
#[path = "avro/schema.rs"]
mod schema;
#[path = "avro/single.rs"]
mod single;
