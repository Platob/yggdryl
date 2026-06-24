//! Node.js extension for **yggdryl**.
//!
//! A thin napi-rs wrapper around [`yggdryl_core::Tree`]; all behaviour lives in
//! the shared Rust core so the Node and Python bindings stay in lockstep.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use yggdryl_core::{Tree as CoreTree, TreeError};

/// Maps a core [`TreeError`] onto a JavaScript-friendly napi error.
fn to_napi_err(err: TreeError) -> Error {
    Error::from_reason(err.to_string())
}

/// A single leaf returned by [`Tree::leaves`].
#[napi(object)]
pub struct Leaf {
    pub path: String,
    pub value: f64,
}

/// A hierarchical, path-addressed tree of numeric values.
///
/// Paths are `/`-separated, e.g. `"roots/urdr"`.
#[napi]
pub struct Tree {
    inner: CoreTree,
}

#[napi]
impl Tree {
    #[napi(constructor)]
    pub fn new() -> Self {
        Tree {
            inner: CoreTree::new(),
        }
    }

    /// Insert `value` at `path`, returning the previous value if any.
    #[napi]
    pub fn insert(&mut self, path: String, value: f64) -> Result<Option<f64>> {
        self.inner.insert(&path, value).map_err(to_napi_err)
    }

    /// Return the value stored at `path`, or `null` if absent.
    #[napi]
    pub fn get(&self, path: String) -> Option<f64> {
        self.inner.get(&path)
    }

    /// Return `true` if a node exists at `path`.
    #[napi]
    pub fn contains(&self, path: String) -> bool {
        self.inner.contains(&path)
    }

    /// Remove the node at `path` and its subtree, returning its value if any.
    #[napi]
    pub fn remove(&mut self, path: String) -> Result<Option<f64>> {
        self.inner.remove(&path).map_err(to_napi_err)
    }

    /// Total number of nodes in the tree.
    #[napi]
    pub fn count(&self) -> u32 {
        self.inner.count() as u32
    }

    /// Depth of the longest root-to-leaf chain.
    #[napi]
    pub fn depth(&self) -> u32 {
        self.inner.depth() as u32
    }

    /// Sum of every value stored in the tree.
    #[napi]
    pub fn sum(&self) -> f64 {
        self.inner.sum()
    }

    /// `true` when the tree holds no nodes.
    #[napi(js_name = "isEmpty")]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Return every leaf as a `{ path, value }` object, sorted by path.
    #[napi]
    pub fn leaves(&self) -> Vec<Leaf> {
        self.inner
            .leaves()
            .into_iter()
            .map(|(path, value)| Leaf { path, value })
            .collect()
    }
}

impl Default for Tree {
    fn default() -> Self {
        Tree::new()
    }
}
