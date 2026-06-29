//! Python extension for **yggdryl**.
//!
//! Blank slate. The PyO3 wrappers were removed in a project reset; the bindings
//! will be reimplemented as thin wrappers over the Arrow-centralized Rust core.
//! All logic lives in the shared core so the Python and Node bindings stay in
//! lockstep — see `CLAUDE.md` for the contributor rules.

use pyo3::prelude::*;

/// The `yggdryl` extension module. Empty until the Arrow-centralized bindings
/// are reimplemented.
#[pymodule]
fn yggdryl(_py: Python<'_>, _module: &Bound<'_, PyModule>) -> PyResult<()> {
    Ok(())
}
