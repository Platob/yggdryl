# yggdryl

A small, polyglot library built **Rust-first**: a dependency-free Rust core
defines the types, and thin **Python** and **Node.js** wrappers expose that same
core. One implementation, three published packages
([crates.io](https://crates.io) / [PyPI](https://pypi.org) /
[npm](https://www.npmjs.com)), so behaviour is identical everywhere.

The core provides three value types:

- **`Uri`** — the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
  shape: `scheme:[//authority]path[?query][#fragment]`.
- **`Url`** — the common subset that always has an authority, decomposed into
  `username`, `password`, `host` and `port`.
- **`Version`** — a generic `major.minor.patch` version that parses, renders and
  orders numerically.

## Layout

```
yggdryl/
├── Cargo.toml                  # Cargo workspace
├── crates/
│   ├── yggdryl-core/           # dependency-free foundations (FromInput/ToOutput, encoding)
│   ├── yggdryl-version/        # standalone Version type
│   └── yggdryl-url/            # Uri/Url, built on (and re-exporting) yggdryl-core
└── bindings/
    ├── python/                 # PyO3 + maturin  → `import yggdryl`
    └── node/                   # napi-rs         → `require('yggdryl')`
```

## The core API

Each type is built with `from_str(value, safe)` (or `from_parts` / `from_mapping`)
and exposes its components as read-only accessors. `safe = true` validates fully;
`false` is a faster, lenient parse. Rendering takes an `encode` flag —
`to_string(encode=true)` (the default, also what `str()` / `toString()` use)
percent-encodes for transport, `encode=false` decodes for display; both are
cached. Functional `copy(...)` / `with_*` / `without_*` builders and the
multi-valued `params` / `with_params` / `add_param` query CRUD all return new
values without mutating the original. The naming is identical across all three
languages (JS uses camelCase) — see [`CLAUDE.md`](CLAUDE.md).

```rust
use yggdryl::{FromInput, Uri, Url, Version};

let uri = Uri::from_str("urn:isbn:0451450523", true)?;
assert_eq!(uri.scheme(), "urn");

let url = Url::from_str("https://user:pw@example.com:8443/api?a=1&a=2", true)?;
assert_eq!(url.host(), "example.com");
assert_eq!(url.port(), Some(8443));
assert_eq!(url.params(true).get("a"), Some(&vec!["1".into(), "2".into()]));

assert!(Version::from_str("1.4.2", true)? < Version::from_str("1.10.0", true)?);
# Ok::<(), yggdryl::UrlError>(())
```

```python
import yggdryl
url = yggdryl.Url("https://example.com/api").copy(port=8443).add_param("q", ["a b"])
print(url.host, url.port, url.params())   # example.com 8443 {'q': ['a b']}
```

```javascript
const { Url } = require('yggdryl')
const url = new Url('https://example.com/api').copy(null, null, null, null, 8443)
console.log(url.host, url.port)           // example.com 8443
```

| `Uri` | `Url` (is-a `Uri` via `to_uri()`) |
| --- | --- |
| `scheme` | `scheme` |
| `authority` | `username` / `password` / `host` / `port` / `authority` |
| `path` | `path` |
| `query` / `params()` | `query` / `params()` |
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

Publishing is automated by `.github/workflows/release.yml`. Pushing a version
tag builds the per-OS artifacts and publishes to all three registries:

```bash
git tag v0.1.0
git push origin v0.1.0
```

(`workflow_dispatch` runs the build only, so you can dry-run the artifacts
without publishing.) It needs three repository secrets — Settings → Secrets and
variables → Actions:

| Secret | Registry |
| --- | --- |
| `CARGO_REGISTRY_TOKEN` | crates.io — the `yggdryl` core crate |
| `PYPI_API_TOKEN` | PyPI — Linux + Windows wheels (+ sdist) |
| `NPM_TOKEN` | npm — the `yggdryl` addon (both `.node` files bundled) |

The PyPI step uses `pypa/gh-action-pypi-publish` — `maturin upload`/`maturin
publish` are deprecated, so don't reintroduce them.

### Publishing manually

If you ever publish outside CI, use the non-deprecated tools:

```bash
# Rust
cargo publish -p yggdryl

# Python — build wheel + sdist, then upload with twine (NOT `maturin upload`)
maturin build --release -m bindings/python/Cargo.toml --out dist
maturin sdist           -m bindings/python/Cargo.toml --out dist
twine upload dist/*

# Node
cd bindings/node && npm run build && npm publish --access public
```

`twine` reads `TWINE_USERNAME=__token__` / `TWINE_PASSWORD=<pypi-token>` from the
environment, so no credential is written to disk or a keyring.

## License

[Apache-2.0](LICENSE).
