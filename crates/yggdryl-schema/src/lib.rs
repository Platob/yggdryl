//! # yggdryl-schema
//!
//! The Arrow-compatible schema layer for yggdryl: the `DataType` / `Field` and
//! schema types, plus fast conversion to and from Apache Arrow's `arrow-schema`
//! (gated behind the `arrow` feature). The `arrow-schema` SDK is a dependency of
//! this crate only, so the rest of the workspace stays free of the Arrow runtime.
//!
//! This is the buildable scaffold left after the project reset. Reintroduce the
//! schema types here — one module per concern, each re-exported at the crate root,
//! with a crate-local `log_event!` macro in this file — following the rules in
//! `CLAUDE.md`.
