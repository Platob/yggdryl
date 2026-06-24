# yggdryl (Rust core)

The pure-Rust core of [**yggdryl**](https://github.com/Platob/yggdryl).

It implements a hierarchical, path-addressed tree (`Tree`) where each node may
carry an optional numeric value and any number of named children. The tree can
also be exchanged as an [Apache Arrow](https://arrow.apache.org/) `RecordBatch`
or IPC stream. The Python and Node extensions in the wider project are thin
wrappers around this crate, so the behaviour is identical in every language.

```rust
use yggdryl::Tree;

let mut tree = Tree::new();
tree.insert("roots/urdr", 1.0);
tree.insert("roots/verdandi", 2.0);
tree.insert("roots/skuld", 3.0);

assert_eq!(tree.get("roots/urdr"), Some(1.0));
assert_eq!(tree.sum(), 6.0);
assert_eq!(tree.count(), 4); // 3 leaves + the `roots` branch

// Columnar Arrow interchange:
let batch = tree.to_record_batch();
assert_eq!(batch.num_rows(), 3);
```
