//! Python views over the byte/value digests and the time-coupled digests.

use pyo3::prelude::*;

pub(crate) mod txhash;
pub(crate) mod xxhash;

/// Register both digest families' classes and functions on the native module.
pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    xxhash::register(module)?;
    txhash::register(module)
}
