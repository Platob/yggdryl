//! One test file per file under `rust/src/iceberg/` that answers for the
//! kinds `yggdryl-market` claims, behind the `iceberg` feature the crate
//! forwards. A file that pins what a caller cannot reach carries the
//! `internals` cfg beside it and reaches the core through
//! `yggdryl::internals`.

#![cfg(feature = "iceberg")]

#[path = "support/install.rs"]
mod install;
#[path = "iceberg/mod_.rs"]
mod mod_;
