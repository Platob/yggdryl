//! [`NodeRef`] — a **transient**, Rust-only cursor over a nested [`AnySerie`](crate::io::AnySerie)
//! tree. It pairs the *root* column, the *current* node, and the [`NodePath`] that reached it, so a
//! caller can drill down ([`child_at`](NodeRef::child_at) / [`child_by`](NodeRef::child_by)) and walk
//! back up ([`parent`](NodeRef::parent), re-resolved from the root in `O(depth)`), all while carrying
//! the path it is at.
//!
//! DESIGN: this is the **only** tolerated lifetime on the child-access surface. It is
//! `pub(crate)` + `#[doc(hidden)]` and never stored on a column or crossed to the bindings — it is a
//! borrow tied to `&self`, used internally; a binding that needs a resolved node clones it out. A
//! public, storable cursor would need a lifetime on a public type, which the crate rules forbid.

use super::any_serie::resolve_serie;
use super::{AnySerie, NodePath, PathSegment};

/// A transient cursor at one node of a nested [`AnySerie`](crate::io::AnySerie) tree: the root, the
/// current node, and the [`NodePath`] that reached it. Build one with [`NodeRef::new`] (or
/// `dyn AnySerie::root_ref`).
// DESIGN: `allow(dead_code)` — this is a deliberate crate-internal API (a transient graph cursor)
// exercised by its own unit tests and reserved for later phases (the bindings clone a resolved node
// out of it); nothing in non-test library code consumes it yet.
#[doc(hidden)]
#[allow(dead_code)]
pub(crate) struct NodeRef<'a> {
    // `+ 'static` (not the default `+ 'a`) to match the erased children the child accessors hand out
    // (`Option<&(dyn AnySerie + 'static)>`), so drilling and re-resolution unify.
    root: &'a (dyn AnySerie + 'static),
    node: &'a (dyn AnySerie + 'static),
    path: NodePath,
}

#[allow(dead_code)]
impl<'a> NodeRef<'a> {
    /// A cursor rooted at `root`, positioned at the root (empty path).
    pub(crate) fn new(root: &'a (dyn AnySerie + 'static)) -> Self {
        Self {
            root,
            node: root,
            path: NodePath::new(),
        }
    }

    /// The cursor at the current node's `index`-th child, or `None` if there is none. Drills **down**
    /// from the current node directly (no re-resolution), extending the path by an `[index]` segment.
    pub(crate) fn child_at(&self, index: usize) -> Option<NodeRef<'a>> {
        let child = self.node.child_serie_at(index)?;
        Some(NodeRef {
            root: self.root,
            node: child,
            path: self.path.clone().child(PathSegment::index(index)),
        })
    }

    /// The cursor at the current node's child named `name`, or `None` if there is none. Drills
    /// **down** directly, extending the path by a `name` segment.
    pub(crate) fn child_by(&self, name: &str) -> Option<NodeRef<'a>> {
        let child = self.node.child_serie_by(name)?;
        Some(NodeRef {
            root: self.root,
            node: child,
            path: self.path.clone().child(PathSegment::name(name)),
        })
    }

    /// The cursor at the current node's parent, or `None` at the root. The parent path is a pure
    /// [`NodePath::parent`] (no graph reference), then **re-resolved from the root** in `O(depth)` —
    /// a node does not hold a back-pointer, so up-navigation replays the path.
    pub(crate) fn parent(&self) -> Option<NodeRef<'a>> {
        let parent_path = self.path.parent()?;
        // A parent of an already-resolved node always resolves, so `.ok()` never drops a real node.
        let node = resolve_serie(self.root, &parent_path).ok()?;
        Some(NodeRef {
            root: self.root,
            node,
            path: parent_path,
        })
    }

    /// The current node.
    pub(crate) fn serie(&self) -> &'a (dyn AnySerie + 'static) {
        self.node
    }

    /// The path that reached the current node.
    pub(crate) fn path(&self) -> &NodePath {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use crate::io::fixed::Serie;
    use crate::io::nested::{ListSerie, StructSerie};
    use crate::io::{boxed, AnySerie};

    /// Builds `struct<a: list<struct<{b:i32}>>>` with the inner `b` column `[10, 20, 30]` and list
    /// rows `[[e0, e1], [e2]]`.
    fn tree() -> StructSerie {
        let b = Serie::from_values(&[10i32, 20, 30]).named("b");
        let inner = boxed(StructSerie::from_series(vec![b]).unwrap());
        let list = ListSerie::from_values(inner, &[0, 2, 3], None).unwrap();
        StructSerie::from_named(vec![("a", boxed(list))]).unwrap()
    }

    #[test]
    fn drill_down_tracks_the_path_and_reaches_the_leaf() {
        let root = tree();
        let cursor = (&root as &dyn AnySerie).root_ref();
        assert!(cursor.path().is_empty());

        // root -> "a" (list) -> [0] (the item child = the flattened inner struct) -> "b" (i32 column)
        let leaf = cursor
            .child_by("a")
            .unwrap()
            .child_at(0)
            .unwrap()
            .child_by("b")
            .unwrap();
        assert_eq!(leaf.path().to_string(), "a[0].b");
        assert_eq!(leaf.serie().len(), 3);
        assert_eq!(leaf.serie().name(), "b");
    }

    #[test]
    fn parent_re_resolves_from_the_root() {
        let root = tree();
        let cursor = (&root as &dyn AnySerie).root_ref();
        let leaf = cursor
            .child_by("a")
            .unwrap()
            .child_at(0)
            .unwrap()
            .child_by("b")
            .unwrap();

        let up1 = leaf.parent().unwrap();
        assert_eq!(up1.path().to_string(), "a[0]");
        let up2 = up1.parent().unwrap();
        assert_eq!(up2.path().to_string(), "a");
        assert!(up2.serie().type_id().is_list());
        let up3 = up2.parent().unwrap();
        assert!(up3.path().is_empty());
        assert!(up3.serie().type_id().is_struct());
        assert!(up3.parent().is_none());
    }

    #[test]
    fn missing_children_yield_none() {
        let root = tree();
        let cursor = (&root as &dyn AnySerie).root_ref();
        assert!(cursor.child_by("nope").is_none());
        assert!(cursor.child_at(9).is_none());
        // A leaf column has no children.
        let list = cursor.child_by("a").unwrap();
        let leaf = list.child_at(0).unwrap().child_by("b").unwrap();
        assert!(leaf.child_at(0).is_none());
        assert!(leaf.child_by("x").is_none());
    }
}
