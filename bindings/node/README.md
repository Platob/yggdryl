# yggdryl (Node.js)

The Node.js extension for **yggdryl**, backed by the Rust core.

Each Rust crate is exposed under its own JS namespace, mirroring the crate tree —
currently just `yggdryl.core` (the foundations, `yggdryl-core`).

```js
const yggdryl = require('yggdryl')

console.log(yggdryl.core.version()) // the crate version
yggdryl.core.hello()                // -> Hello, world!
```

> **Project reset.** A thin hello-world scaffold over the Arrow-centralized Rust
> core, rebuilt from scratch. See `CLAUDE.md` at the repository root for contributor
> rules.
