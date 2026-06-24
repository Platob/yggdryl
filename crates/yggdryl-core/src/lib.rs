//! # yggdryl-core
//!
//! The pure-Rust core of the **yggdryl** project: a hierarchical, path-addressed
//! tree. Each [`Node`] may hold an optional numeric value and any number of named
//! children. Paths are `/`-separated, e.g. `"roots/urdr"`.
//!
//! This crate has no dependencies and no FFI; the Python and Node extensions in
//! the wider project wrap the types defined here so behaviour is identical across
//! every language binding.

use std::collections::BTreeMap;

/// Error returned by fallible [`Tree`] operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeError {
    /// A path was empty or contained only separators.
    EmptyPath,
    /// No node exists at the requested path.
    NotFound(String),
}

impl std::fmt::Display for TreeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TreeError::EmptyPath => write!(f, "path is empty"),
            TreeError::NotFound(path) => write!(f, "no node at path '{path}'"),
        }
    }
}

impl std::error::Error for TreeError {}

/// A single node in a [`Tree`].
///
/// A node carries an optional numeric `value` and a set of named children. A node
/// with no value is a pure branch; a node with a value may still have children.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Node {
    value: Option<f64>,
    children: BTreeMap<String, Node>,
}

impl Node {
    /// Returns the value stored directly on this node, if any.
    pub fn value(&self) -> Option<f64> {
        self.value
    }

    /// Returns the number of direct children of this node.
    pub fn child_count(&self) -> usize {
        self.children.len()
    }

    /// Returns the names of the direct children, in sorted order.
    pub fn child_names(&self) -> impl Iterator<Item = &str> {
        self.children.keys().map(String::as_str)
    }

    /// Borrows a direct child by name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.get(name)
    }

    /// Total number of nodes in the subtree rooted at `self`, including `self`.
    fn subtree_size(&self) -> usize {
        1 + self
            .children
            .values()
            .map(Node::subtree_size)
            .sum::<usize>()
    }

    /// Sum of every `value` in the subtree rooted at `self`.
    fn subtree_sum(&self) -> f64 {
        self.value.unwrap_or(0.0) + self.children.values().map(Node::subtree_sum).sum::<f64>()
    }

    /// Length of the longest root-to-leaf chain in the subtree rooted at `self`.
    /// A node with no children has depth 1.
    fn subtree_depth(&self) -> usize {
        1 + self
            .children
            .values()
            .map(Node::subtree_depth)
            .max()
            .unwrap_or(0)
    }

    /// Appends every `prefix`-qualified leaf path/value pair into `out`.
    fn collect_leaves(&self, prefix: &str, out: &mut Vec<(String, f64)>) {
        if self.children.is_empty() {
            if let Some(value) = self.value {
                out.push((prefix.to_string(), value));
            }
            return;
        }
        for (name, child) in &self.children {
            let next = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            child.collect_leaves(&next, out);
        }
    }
}

/// A hierarchical, path-addressed tree of [`Node`]s.
///
/// ```
/// use yggdryl_core::Tree;
///
/// let mut tree = Tree::new();
/// tree.insert("roots/urdr", 1.0);
/// tree.insert("roots/verdandi", 2.0);
///
/// assert_eq!(tree.get("roots/urdr"), Some(1.0));
/// assert_eq!(tree.sum(), 3.0);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tree {
    root: Node,
}

/// Splits a `/`-separated path into its non-empty segments.
fn segments(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|s| !s.is_empty())
}

impl Tree {
    /// Creates an empty tree.
    pub fn new() -> Self {
        Tree::default()
    }

    /// Inserts `value` at `path`, creating intermediate branches as needed.
    ///
    /// Returns the previous value at `path`, if one existed. Returns
    /// [`TreeError::EmptyPath`] if `path` has no non-empty segments.
    pub fn insert(&mut self, path: &str, value: f64) -> Result<Option<f64>, TreeError> {
        let mut node = &mut self.root;
        let mut any = false;
        for segment in segments(path) {
            any = true;
            node = node.children.entry(segment.to_string()).or_default();
        }
        if !any {
            return Err(TreeError::EmptyPath);
        }
        Ok(node.value.replace(value))
    }

    /// Returns the value stored at `path`, or `None` if the node is missing or
    /// holds no value.
    pub fn get(&self, path: &str) -> Option<f64> {
        self.node(path).and_then(Node::value)
    }

    /// Returns `true` if a node exists at `path` (with or without a value).
    pub fn contains(&self, path: &str) -> bool {
        self.node(path).is_some()
    }

    /// Borrows the node at `path`, if it exists.
    pub fn node(&self, path: &str) -> Option<&Node> {
        let mut node = &self.root;
        for segment in segments(path) {
            node = node.children.get(segment)?;
        }
        Some(node)
    }

    /// Removes the node at `path` and its entire subtree.
    ///
    /// Returns the value that was stored directly at `path`, if any.
    /// Returns [`TreeError::NotFound`] if no node exists at `path`, or
    /// [`TreeError::EmptyPath`] for an empty path (the root cannot be removed).
    pub fn remove(&mut self, path: &str) -> Result<Option<f64>, TreeError> {
        let parts: Vec<&str> = segments(path).collect();
        let (last, parents) = parts.split_last().ok_or(TreeError::EmptyPath)?;

        let mut node = &mut self.root;
        for segment in parents {
            node = node
                .children
                .get_mut(*segment)
                .ok_or_else(|| TreeError::NotFound(path.to_string()))?;
        }
        node.children
            .remove(*last)
            .map(|removed| removed.value)
            .ok_or_else(|| TreeError::NotFound(path.to_string()))
    }

    /// Total number of nodes in the tree, excluding the implicit root.
    pub fn count(&self) -> usize {
        self.root.subtree_size() - 1
    }

    /// `true` when the tree holds no nodes.
    pub fn is_empty(&self) -> bool {
        self.root.children.is_empty()
    }

    /// Sum of every value stored anywhere in the tree.
    pub fn sum(&self) -> f64 {
        self.root.subtree_sum()
    }

    /// Depth of the tree: the longest root-to-leaf chain. An empty tree has
    /// depth 0; a tree with a single node has depth 1.
    pub fn depth(&self) -> usize {
        self.root.subtree_depth() - 1
    }

    /// Returns every leaf as a `(path, value)` pair, sorted by path.
    pub fn leaves(&self) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        self.root.collect_leaves("", &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tree {
        let mut tree = Tree::new();
        tree.insert("roots/urdr", 1.0).unwrap();
        tree.insert("roots/verdandi", 2.0).unwrap();
        tree.insert("roots/skuld", 3.0).unwrap();
        tree
    }

    #[test]
    fn insert_and_get() {
        let tree = sample();
        assert_eq!(tree.get("roots/urdr"), Some(1.0));
        assert_eq!(tree.get("roots/verdandi"), Some(2.0));
        assert_eq!(tree.get("roots/skuld"), Some(3.0));
        assert_eq!(tree.get("roots/missing"), None);
    }

    #[test]
    fn insert_returns_previous_value() {
        let mut tree = Tree::new();
        assert_eq!(tree.insert("a", 1.0), Ok(None));
        assert_eq!(tree.insert("a", 2.0), Ok(Some(1.0)));
        assert_eq!(tree.get("a"), Some(2.0));
    }

    #[test]
    fn empty_path_is_rejected() {
        let mut tree = Tree::new();
        assert_eq!(tree.insert("", 1.0), Err(TreeError::EmptyPath));
        assert_eq!(tree.insert("///", 1.0), Err(TreeError::EmptyPath));
    }

    #[test]
    fn branch_nodes_have_no_value() {
        let tree = sample();
        assert!(tree.contains("roots"));
        assert_eq!(tree.get("roots"), None);
    }

    #[test]
    fn count_includes_branches() {
        let tree = sample();
        // `roots` branch + three leaves.
        assert_eq!(tree.count(), 4);
    }

    #[test]
    fn sum_walks_whole_tree() {
        assert_eq!(sample().sum(), 6.0);
    }

    #[test]
    fn depth_is_longest_chain() {
        let tree = sample();
        assert_eq!(tree.depth(), 2);

        let mut deep = Tree::new();
        deep.insert("a/b/c/d", 1.0).unwrap();
        assert_eq!(deep.depth(), 4);
    }

    #[test]
    fn empty_tree() {
        let tree = Tree::new();
        assert!(tree.is_empty());
        assert_eq!(tree.count(), 0);
        assert_eq!(tree.depth(), 0);
        assert_eq!(tree.sum(), 0.0);
        assert!(tree.leaves().is_empty());
    }

    #[test]
    fn leaves_are_sorted_paths() {
        let tree = sample();
        assert_eq!(
            tree.leaves(),
            vec![
                ("roots/skuld".to_string(), 3.0),
                ("roots/urdr".to_string(), 1.0),
                ("roots/verdandi".to_string(), 2.0),
            ]
        );
    }

    #[test]
    fn remove_subtree() {
        let mut tree = sample();
        assert_eq!(tree.remove("roots/urdr"), Ok(Some(1.0)));
        assert_eq!(tree.get("roots/urdr"), None);
        assert_eq!(tree.count(), 3);

        assert_eq!(tree.remove("roots"), Ok(None));
        assert!(tree.is_empty());
    }

    #[test]
    fn remove_missing_and_empty() {
        let mut tree = sample();
        assert_eq!(
            tree.remove("roots/nowhere"),
            Err(TreeError::NotFound("roots/nowhere".to_string()))
        );
        assert_eq!(tree.remove(""), Err(TreeError::EmptyPath));
    }

    #[test]
    fn leading_and_trailing_separators_are_ignored() {
        let mut tree = Tree::new();
        tree.insert("/a/b/", 5.0).unwrap();
        assert_eq!(tree.get("a/b"), Some(5.0));
        assert_eq!(tree.get("/a/b"), Some(5.0));
    }
}
