//! One test file per file under `rust/src/txhash/`, in `tests/txhash/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and `mod_.rs` is the folder's own `mod.rs`. A test
//! reaches the crate through `yggdryl::`; where what it pins is not reachable
//! that way, it reaches `yggdryl::internals`, which exists only under the
//! `internals` feature, and the file that reaches it is declared behind that
//! feature here.

#[path = "txhash/arrow.rs"]
mod arrow;
#[path = "txhash/field.rs"]
mod field;
#[path = "txhash/hasher.rs"]
mod hasher;
#[path = "txhash/mod_.rs"]
mod mod_;
#[path = "txhash/scalar.rs"]
mod scalar;
#[path = "txhash/time.rs"]
mod time;
#[path = "txhash/value.rs"]
mod value;
