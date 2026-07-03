# yggdryl (Node.js)

The Node.js extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed under its own JS namespace, mirroring the crate tree:

- `yggdryl.core` — the foundations (`yggdryl-core`): byte/bit buffers, cursors,
  slices.
- `yggdryl.data` — the Arrow data-model layer (`yggdryl-data`): every integer data
  type with its field, scalar and null-or-value optional scalar, the binary type
  (`Buffer` in and out, `toIo()` handing back a `yggdryl.core` `ByteBuffer`), plus
  the null and union types (64-bit values are `BigInt`, narrower ones `number`).

```js
const yggdryl = require('yggdryl')

console.log(yggdryl.core.version())

const scalar = new yggdryl.data.Int64Scalar(42n)
console.assert(scalar.asI8() === 42) // exact conversion, or a thrown Error
console.assert(yggdryl.data.OptionalInt64Scalar.null().isNull())
console.assert(new yggdryl.data.Int64().optional().arrowFormat() === '+us:0,1')
```

> **Project reset.** A thin scaffold over the Arrow-centralized Rust core. See
> `CLAUDE.md` at the repository root for contributor rules.
