//! Node.js extension for **yggdryl**.
//!
//! Each Rust crate is exposed under its own JS namespace — `yggdryl.core` (the
//! foundations), `yggdryl.dtype` (the data types), `yggdryl.field` (the fields)
//! and `yggdryl.scalar` (the scalars) — mirroring the crate tree, each class
//! placed in its namespace by napi's `#[napi(namespace = "…")]` attribute. The
//! classes carry globally-unique names (the `…Type` / `…Field` / `…Scalar`
//! suffixes keep the three concerns distinct in napi's addon-global registry), so
//! the generated `index.js` / `index.d.ts` namespace map is the package entry
//! directly. The wrappers are thin: all logic lives in the Rust crates, so the
//! Node and Python bindings behave identically.

use napi::bindgen_prelude::{BigInt, Error, Result};

pub mod core;
pub mod dtype;
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
