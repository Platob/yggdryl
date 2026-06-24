# yggdryl

A small, polyglot library built around a **Rust core** with **Python** and
**Node.js** extensions that wrap that same core. The name nods to *Yggdrasil*,
the world tree — fittingly, the library is a hierarchical, path-addressed tree.

One implementation, three languages: the Python and Node packages are thin FFI
shims over the `yggdryl` core crate, so behaviour is identical everywhere.

## Layout

```
yggdryl/
├── Cargo.toml                  # Cargo workspace
├── crates/
│   └── yggdryl/                # pure-Rust core (Apache Arrow is the one dep)
└── bindings/
    ├── python/                 # PyO3 + maturin  → `import yggdryl`
    └── node/                   # napi-rs         → `require('yggdryl')`
```

## The core API

`Tree` is a hierarchical map from `/`-separated paths to `f64` values:

| Method | Description |
| --- | --- |
| `insert(path, value)` | Insert a value, creating branches as needed; returns the previous value. |
| `get(path)` | Value at `path`, or `None`/`null`. |
| `contains(path)` | Whether a node exists at `path`. |
| `remove(path)` | Delete a node and its subtree. |
| `count()` | Number of nodes. |
| `depth()` | Longest root-to-leaf chain. |
| `sum()` | Sum of all values. |
| `leaves()` | All `(path, value)` leaves, sorted by path. |

## Building & testing

### Rust core

```bash
cargo test            # runs the core crate's tests (default workspace member)
cargo clippy --workspace --all-targets
cargo fmt --all --check
```

### Python extension

```bash
cd bindings/python
pip install maturin
maturin develop                 # build + install into the active venv
pytest                          # run bindings/python/tests
```

### Node extension

```bash
cd bindings/node
npm install
npm run build                   # napi build --platform --release
npm test                        # node --test
```

## Why this shape?

- **Rust core** holds all logic and the test suite of record.
- **Bindings stay thin** — they only translate types and errors, so the three
  languages can never drift apart.
- `default-members` in the workspace keeps `cargo build`/`cargo test` limited to
  the pure-Rust core, so the common path never needs Python or Node headers; the
  extensions are built through their own toolchains (maturin / napi).

## License

[Apache-2.0](LICENSE).
