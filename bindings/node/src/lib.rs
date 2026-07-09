//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate — and, where a crate spans several concerns, each concern — is
//! exposed under its own JS namespace: `yggdryl.core` (the foundations),
//! `yggdryl.compression` (the compression codecs), `yggdryl.io` (the positioned
//! byte-IO resources), and `yggdryl.buffer` (the typed native-type buffers), all
//! mirroring `yggdryl-core`. Each item is placed in its namespace by napi's
//! `#[napi(namespace = "…")]` attribute, and the generated `index.js` / `index.d.ts`
//! namespace map is the package entry directly. The wrappers are thin: all logic
//! lives in the Rust crates, so the Node and Python bindings behave identically.

pub mod buffer;
pub mod compression;
pub mod core;
pub mod io;
