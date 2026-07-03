# yggdryl (Python)

The Python extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed as a submodule of the `yggdryl` package, mirroring the
crate tree:

- `yggdryl.core` — the foundations (`yggdryl-core`): byte/bit buffers, cursors,
  slices.
- `yggdryl.dtype` — the data types (`yggdryl-dtype`): every integer type and its
  logical optional, the binary type, plus the null and union types, each with the
  native byte codec and defaults.
- `yggdryl.field` — the fields (`yggdryl-field`): a name paired with a
  `yggdryl.dtype` data type and a nullability flag.
- `yggdryl.scalar` — the scalars (`yggdryl-scalar`): single, possibly-null values
  with exact-or-raise `as_*` accessors; the binary scalar hands its bytes back as
  a `yggdryl.core` `ByteBuffer` through `to_io()`.

The three data submodules share one bare name per type — `dtype.Int64` describes
the type, `field.Int64` names a column of it, `scalar.Int64` holds one value of
it.

```python
import yggdryl
from yggdryl import dtype, field, scalar

print(yggdryl.core.version())

value = scalar.Int64(42)
assert value.as_i8() == 42           # exact conversion, or ValueError
assert scalar.OptionalInt64.null().is_null()
assert dtype.Int64().optional().arrow_format() == "+us:0,1"
assert field.Int64("id", False).data_type().name() == "int64"
```

> **Project reset.** A thin scaffold over the Arrow-centralized Rust core. See
> `CLAUDE.md` at the repository root for contributor rules.
