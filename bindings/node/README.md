# yggdryl (Node.js)

The Node.js extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed under its own JS namespace, mirroring the crate tree:

- `yggdryl.core` — the foundations (`yggdryl-core`).
- `yggdryl.schema` — the Arrow-compatible schema layer (`yggdryl-schema`).

```js
const yggdryl = require('yggdryl')

console.log(yggdryl.core.version())
console.assert(yggdryl.schema.DataTypeId.Int32 !== yggdryl.schema.DataTypeId.Utf8)
```

> **Project reset.** A thin scaffold over the Arrow-centralized Rust core. See
> `CLAUDE.md` at the repository root for contributor rules.
