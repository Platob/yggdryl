# yggdryl (Node.js)

The Node.js extension for **yggdryl**, backed by the Rust core.

The package mirrors the `yggdryl-core` module tree as JS namespaces: `yggdryl.core`
(`version` / `hello`), `yggdryl.compression` (the gzip / zstd codecs), `yggdryl.io`
(positioned byte buffers), and `yggdryl.buffer` (typed native-type buffers).

```js
const yggdryl = require('yggdryl')

console.log(yggdryl.core.version()) // the crate version
yggdryl.core.hello()                // -> Hello, world!
```

See `CLAUDE.md` at the repository root for contributor rules.
