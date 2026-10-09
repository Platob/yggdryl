//! One test file per file under `rust/src/xxhash/` that answers for what
//! `yggdryl-market` owns, in `tests/xxhash/`. A test reaches this crate
//! through `yggdryl_market::` and the core through `yggdryl::`.

#[path = "xxhash/arrow.rs"]
mod arrow;
#[path = "support/install.rs"]
mod install;
#[path = "xxhash/scalar.rs"]
mod scalar;
