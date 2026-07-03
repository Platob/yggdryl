# yggdryl (Node.js)

The Node.js extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed under its own JS namespace, mirroring the crate tree:

- `yggdryl.core` — the foundations (`yggdryl-core`): byte/bit buffers, cursors,
  slices.
- `yggdryl.dtype` — the data types (`yggdryl-dtype`): every integer type and its
  logical optional, the binary type, plus the null and union types, each with the
  native byte codec and defaults.
- `yggdryl.field` — the fields (`yggdryl-field`): a name paired with a
  `yggdryl.dtype` data type and a nullability flag.
- `yggdryl.scalar` — the scalars (`yggdryl-scalar`): single, possibly-null values
  with exact-or-throw `as*` accessors; the binary scalar hands its bytes back as
  a `yggdryl.core` `ByteBuffer` through `toIo()` (64-bit values are `BigInt`,
  narrower ones `number`).

The three data namespaces share one bare name per type — `dtype.Int64` describes
the type, `field.Int64` names a column of it, `scalar.Int64` holds one value of
it. (napi registers class constructors by JS class name in one addon-global
registry, so the native classes carry a unique prefix — `DtypeInt64`,
`FieldInt64`, `ScalarInt64` — and the package entry `yggdryl.js` / `yggdryl.d.ts`
strips it into the namespaces.)

```js
const yggdryl = require('yggdryl')

console.log(yggdryl.core.version())

const value = new yggdryl.scalar.Int64(42n)
console.assert(value.asI8() === 42) // exact conversion, or a thrown Error
console.assert(yggdryl.scalar.OptionalInt64.null().isNull())
console.assert(new yggdryl.dtype.Int64().optional().arrowFormat() === '+us:0,1')
console.assert(new yggdryl.field.Int64('id', false).dataType().name() === 'int64')
```

> **Project reset.** A thin scaffold over the Arrow-centralized Rust core. See
> `CLAUDE.md` at the repository root for contributor rules.
