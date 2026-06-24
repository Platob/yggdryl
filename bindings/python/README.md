# yggdryl (Python)

Python bindings for [**yggdryl**](https://github.com/Platob/yggdryl), backed by
the Rust `yggdryl` core crate.

## Install

```bash
pip install maturin
maturin develop          # build & install into the current virtualenv
# or build a wheel:
maturin build --release
```

## Usage

```python
import yggdryl

tree = yggdryl.Tree()
tree.insert("roots/urdr", 1.0)
tree.insert("roots/verdandi", 2.0)
tree.insert("roots/skuld", 3.0)

print(tree.get("roots/urdr"))  # 1.0
print(tree.sum())              # 6.0
print(len(tree))               # 4
print(tree.leaves())           # [('roots/skuld', 3.0), ('roots/urdr', 1.0), ('roots/verdandi', 2.0)]
```
