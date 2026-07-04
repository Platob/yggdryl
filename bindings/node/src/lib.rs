//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate is exposed under its own JS namespace — `yggdryl.core` (the
//! foundations), `yggdryl.dtype` (the data types), `yggdryl.field` (the fields)
//! and `yggdryl.scalar` (the scalars) — mirroring the crate tree, each class
//! placed in its namespace by napi's `#[napi(namespace = "…")]` attribute. The
//! classes carry globally-unique names (the `…Type` / `…Field` / `…Scalar`
//! suffixes keep the three concerns distinct in napi's addon-global registry), so
//! the generated `index.js` / `index.d.ts` namespace map is the package entry
//! directly. A convenience `factory` namespace adds the type-inference factory
//! (`scalar` / `dtype` / `field`), building the matching object from a native value
//! without naming its type. The wrappers are thin: all logic lives in the Rust
//! crates, so the Node and Python bindings behave identically.

// Every class exposes a `to_string` inherent method that napi maps to JS
// `toString()` (its pretty `display()` form); the wrappers hold no `Display` impl,
// so clippy's `inherent_to_string` would fire on each — the mapping is deliberate.
#![allow(clippy::inherent_to_string)]

use napi::bindgen_prelude::{BigInt, Error, Result};

pub mod core;
pub mod dtype;
pub mod factory;
pub mod field;
pub mod scalar;

/// Wraps a data-layer error so napi throws it as a JS `Error` — the shared error
/// conversion for the `dtype`, `field` and `scalar` namespaces.
pub(crate) fn data_error(error: yggdryl_dtype::DataError) -> Error {
    Error::from_reason(error.to_string())
}

/// A JS `number` (as `i64`) narrowed to the native type, or an actionable error.
pub(crate) fn wire_to_native<T: TryFrom<i64>>(value: i64, name: &str) -> Result<T> {
    T::try_from(value)
        .map_err(|_| Error::from_reason(format!("expected {value} to be in the {name} range")))
}

/// A JS `number` index as a `usize`, or an actionable error when negative —
/// taking the index as `i64` keeps every JS integer exact, where a `u32`
/// parameter would silently wrap values past 2^32 back into range.
pub(crate) fn index_to_usize(index: i64) -> Result<usize> {
    usize::try_from(index)
        .map_err(|_| Error::from_reason(format!("expected a non-negative index, got {index}")))
}

/// A `BigInt` as an `i64`, or an actionable error when out of range.
pub(crate) fn bigint_to_i64(value: BigInt) -> Result<i64> {
    let (value, lossless) = value.get_i64();
    if lossless {
        Ok(value)
    } else {
        Err(Error::from_reason(
            "expected an int64 in -(2**63)..=2**63-1",
        ))
    }
}

/// A `BigInt` as a `u64`, or an actionable error when negative or out of range.
pub(crate) fn bigint_to_u64(value: BigInt) -> Result<u64> {
    let (sign, value, lossless) = value.get_u64();
    if !sign && lossless {
        Ok(value)
    } else {
        Err(Error::from_reason("expected a uint64 in 0..=2**64-1"))
    }
}

/// Bridges a native float and the JS `number` (always an `f64`): narrows a
/// `number` to the native width (`f32` rounds to nearest; `f64` is exact) and
/// widens the native value back to a `number`. The one place the float wire
/// conversion lives, shared by the `scalar` and `dtype` namespaces (mirroring
/// [`wire_to_native`] for the integer families).
pub(crate) trait WireFloat: Copy {
    /// The native value from a JS `number`.
    fn from_wire(value: f64) -> Self;
    /// The native value as a JS `number`.
    fn to_wire(self) -> f64;
}

impl WireFloat for f32 {
    fn from_wire(value: f64) -> Self {
        value as f32
    }
    fn to_wire(self) -> f64 {
        self as f64
    }
}

impl WireFloat for yggdryl_scalar::half::f16 {
    // `f16` is not a Rust primitive, so it narrows / widens through `half`'s
    // conversions rather than `as` casts (a JS `number` is lossily narrowed to f16).
    fn from_wire(value: f64) -> Self {
        yggdryl_scalar::half::f16::from_f64(value)
    }
    fn to_wire(self) -> f64 {
        self.to_f64()
    }
}

impl WireFloat for f64 {
    fn from_wire(value: f64) -> Self {
        value
    }
    fn to_wire(self) -> f64 {
        self
    }
}
