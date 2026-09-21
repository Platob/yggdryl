//! One test file per file under `rust/src/iceberg/catalog/`.
//!
//! The hierarchy is one shape per level - catalogs of namespaces of tables -
//! and every level of it is `yggdryl::iceberg` API, so the whole suite reaches
//! the crate the way a caller does.

#[path = "catalog/mod_.rs"]
mod mod_;
