//! One test file per file under `rust/src/xxhash/`, in `tests/xxhash/`.
//!
//! `rust/tests/` mirrors `rust/src/`: a source file has exactly one test file
//! at the matching path, and `mod_.rs` is the folder's own `mod.rs`. A test
//! reaches the crate through `yggdryl::`; where what it pins is not reachable
//! that way, it reaches `yggdryl::internals`, which exists only under the
//! `internals` feature, and the file that reaches it is declared behind that
//! feature here.

#[path = "xxhash/arrow.rs"]
mod arrow;
#[path = "xxhash/field.rs"]
mod field;
#[path = "xxhash/handle.rs"]
mod handle;
#[path = "xxhash/mod_.rs"]
mod mod_;
#[path = "xxhash/scalar.rs"]
mod scalar;
#[path = "xxhash/secret.rs"]
mod secret;
#[path = "xxhash/state.rs"]
mod state;
#[path = "xxhash/stream.rs"]
mod stream;
