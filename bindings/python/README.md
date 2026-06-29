# yggdryl (Python)

The Python extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed as a submodule of the `yggdryl` package, mirroring the
crate tree:

- `yggdryl.core` — the foundations (`yggdryl-core`).
- `yggdryl.schema` — the Arrow-compatible schema layer (`yggdryl-schema`).

```python
import yggdryl
from yggdryl.schema import DataTypeId

print(yggdryl.core.version())
assert DataTypeId.Int32 != DataTypeId.Utf8
```

> **Project reset.** A thin scaffold over the Arrow-centralized Rust core. See
> `CLAUDE.md` at the repository root for contributor rules.
