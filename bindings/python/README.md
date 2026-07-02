# yggdryl (Python)

The Python extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed as a submodule of the `yggdryl` package, mirroring the
crate tree:

- `yggdryl.core` — the foundations (`yggdryl-core`): byte/bit buffers, cursors,
  slices.
- `yggdryl.data` — the Arrow data-model layer (`yggdryl-data`): every integer data
  type with its field, scalar and null-or-value optional scalar, plus the null and
  union types.

```python
import yggdryl
from yggdryl import data

print(yggdryl.core.version())

scalar = data.Int64Scalar(42)
assert scalar.as_i8() == 42          # exact conversion, or None
assert data.OptionalInt64Scalar.null().is_null()
assert data.Int64().optional().arrow_format() == "+us:0,1"
```

> **Project reset.** A thin scaffold over the Arrow-centralized Rust core. See
> `CLAUDE.md` at the repository root for contributor rules.
