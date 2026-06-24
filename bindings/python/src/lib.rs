//! Python extension for **yggdryl**.
//!
//! A thin PyO3 wrapper around [`yggdryl_core::Tree`]; the heavy lifting lives in
//! the shared Rust core so the Python and Node bindings behave identically.

// The `#[pymethods]` macro injects an `.into()` on returned errors; because our
// fallible methods already return `PyErr`, clippy flags it as a useless
// conversion. The lint fires on macro-generated code, so it must be allowed at
// crate level rather than per method.
#![allow(clippy::useless_conversion)]

use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;
use yggdryl_core::{Tree as CoreTree, TreeError};

/// Translates a core [`TreeError`] into the matching Python exception.
fn to_pyerr(err: TreeError) -> PyErr {
    match err {
        TreeError::EmptyPath => PyValueError::new_err("path is empty"),
        TreeError::NotFound(path) => PyKeyError::new_err(path),
    }
}

/// A hierarchical, path-addressed tree of numeric values.
///
/// Paths are ``/``-separated, e.g. ``"roots/urdr"``.
#[pyclass(name = "Tree", module = "yggdryl")]
#[derive(Clone, Default)]
struct Tree {
    inner: CoreTree,
}

#[pymethods]
impl Tree {
    #[new]
    fn new() -> Self {
        Tree::default()
    }

    /// Insert ``value`` at ``path``, returning the previous value if any.
    fn insert(&mut self, path: &str, value: f64) -> PyResult<Option<f64>> {
        self.inner.insert(path, value).map_err(to_pyerr)
    }

    /// Return the value stored at ``path``, or ``None`` if absent.
    fn get(&self, path: &str) -> Option<f64> {
        self.inner.get(path)
    }

    /// Return ``True`` if a node exists at ``path``.
    fn contains(&self, path: &str) -> bool {
        self.inner.contains(path)
    }

    /// Remove the node at ``path`` and its subtree, returning its value if any.
    fn remove(&mut self, path: &str) -> PyResult<Option<f64>> {
        self.inner.remove(path).map_err(to_pyerr)
    }

    /// Total number of nodes in the tree.
    fn count(&self) -> usize {
        self.inner.count()
    }

    /// Depth of the longest root-to-leaf chain.
    fn depth(&self) -> usize {
        self.inner.depth()
    }

    /// Sum of every value stored in the tree.
    fn sum(&self) -> f64 {
        self.inner.sum()
    }

    /// ``True`` when the tree holds no nodes.
    fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Return every leaf as a ``(path, value)`` tuple, sorted by path.
    fn leaves(&self) -> Vec<(String, f64)> {
        self.inner.leaves()
    }

    fn __len__(&self) -> usize {
        self.inner.count()
    }

    fn __contains__(&self, path: &str) -> bool {
        self.inner.contains(path)
    }

    fn __repr__(&self) -> String {
        format!(
            "Tree(count={}, depth={}, sum={})",
            self.inner.count(),
            self.inner.depth(),
            self.inner.sum()
        )
    }
}

/// The ``yggdryl`` Python module.
#[pymodule]
fn yggdryl(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<Tree>()?;
    Ok(())
}
