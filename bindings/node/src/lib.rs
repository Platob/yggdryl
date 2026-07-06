//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate is exposed under its own JS namespace — currently just
//! `yggdryl.core` (the foundations, mirroring `yggdryl-core`) — each item placed in
//! its namespace by napi's `#[napi(namespace = "…")]` attribute, and the generated
//! `index.js` / `index.d.ts` namespace map is the package entry directly. The
//! wrappers are thin: all logic lives in the Rust crates, so the Node and Python
//! bindings behave identically.

pub mod core;
