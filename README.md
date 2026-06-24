# yggdryl

A small, polyglot library built **Rust-first**: a dependency-free Rust core
defines the types, and thin **Python** and **Node.js** wrappers expose that same
core. One implementation, three published packages
([crates.io](https://crates.io) / [PyPI](https://pypi.org) /
[npm](https://www.npmjs.com)), so behaviour is identical everywhere.

The core provides two URI value types:

- **`Uri`** — the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
  shape: `scheme:[//authority]path[?query][#fragment]`.
- **`Url`** — the common subset that always has an authority, decomposed into
  `username`, `password`, `host` and `port`.

## Layout

```
yggdryl/
├── Cargo.toml                  # Cargo workspace
├── crates/
│   └── yggdryl/                # pure-Rust core (no dependencies)
└── bindings/
    ├── python/                 # PyO3 + maturin  → `import yggdryl`
    └── node/                   # napi-rs         → `require('yggdryl')`
```

## The core API

Both types parse from a string and expose their components as read-only
accessors; `to_string()` / `str()` / `toString()` reconstructs the original.

```rust
use yggdryl::{Uri, Url};

let uri = Uri::parse("urn:isbn:0451450523")?;
assert_eq!(uri.scheme(), "urn");

let url = Url::parse("https://user:pw@example.com:8443/api?v=1#top")?;
assert_eq!(url.host(), "example.com");
assert_eq!(url.port(), Some(8443));
# Ok::<(), yggdryl::UrlError>(())
```

```python
import yggdryl
url = yggdryl.Url("https://example.com:8443/api")
print(url.host, url.port)        # example.com 8443
```

```javascript
const { Url } = require('yggdryl')
const url = new Url('https://example.com:8443/api')
console.log(url.host, url.port)  // example.com 8443
```

| `Uri` | `Url` |
| --- | --- |
| `scheme` | `scheme` |
| `authority` | `username` / `password` / `host` / `port` / `authority` |
| `path` | `path` |
| `query` | `query` |
| `fragment` | `fragment` |

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

- **Rust-first**: the core holds all parsing logic and the test suite of record.
- **Bindings stay thin** — they only translate types and errors, so the three
  languages can never drift apart.
- `default-members` in the workspace keeps `cargo build`/`cargo test` limited to
  the pure-Rust core, so the common path never needs Python or Node headers; the
  extensions are built through their own toolchains (maturin / napi).

## Platforms

CI builds and tests every layer on **Linux** and **Windows**
(`x86_64-unknown-linux-gnu` and `x86_64-pc-windows-msvc`). The pure-Rust crate is
portable source; the Python wheels and Node addons are built per-OS by the
`Build artifacts` workflow (`.github/workflows/release.yml`).

## Publishing

Each language ships from its own manifest:

- **Rust** → `cargo publish -p yggdryl` (crates.io) — one portable source crate.
- **Python** → wheels built per-OS with maturin, then `maturin publish` /
  `twine upload` (PyPI).
- **Node** → `napi build` per-OS, then `npm publish` from `bindings/node` (npm).

## License

[Apache-2.0](LICENSE).
