//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate — and, where a crate spans several concerns, each concern — is
//! exposed under its own JS namespace: `yggdryl.core` (the foundations),
//! `yggdryl.compression` (the compression codecs), `yggdryl.io` (the positioned
//! byte-IO resources), all mirroring `yggdryl-core`; `yggdryl.buffer` (the typed
//! native-type buffers, mirroring `yggdryl-buffer`); `yggdryl.dtype` / `yggdryl.field` /
//! `yggdryl.scalar` (the Arrow primitive data types, fields, and scalars, mirroring
//! `yggdryl-dtype` / `yggdryl-field` / `yggdryl-scalar`); `yggdryl.decimal` (the
//! fixed-width decimals, mirroring `yggdryl-core`'s `decimal` module); plus `yggdryl.infer`
//! (a binding-only convenience that
//! reads a value's runtime type and builds the matching buffer — `CLAUDE.md` rule 13,
//! so it has no core counterpart) and `yggdryl.converter` (a dtype-keyed facade over
//! the core's `codec::converter`, surfaced flat — as `compression` surfaces the core
//! codec — so the `codec` grouping stays Rust-only). Each item is placed by napi's
//! `#[napi(namespace = "…")]` attribute, and the generated `index.js` / `index.d.ts`
//! namespace map is the package entry directly. The wrappers are thin: all logic
//! lives in the Rust crates, so the Node and Python bindings behave identically.

pub mod buffer;
pub mod compression;
pub mod converter;
pub mod core;
pub mod decimal;
pub mod dtype;
pub mod field;
pub mod infer;
pub mod io;
pub mod scalar;
