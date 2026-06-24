# yggdryl

A small, polyglot library built **Rust-first**: a dependency-free Rust core
defines the types, and thin **Python** and **Node.js** wrappers expose that same
core. One implementation, three published packages
([crates.io](https://crates.io) / [PyPI](https://pypi.org) /
[npm](https://www.npmjs.com)), so behaviour is identical everywhere.

The core provides these value types:

- **`Uri`** — the generic [RFC 3986](https://www.rfc-editor.org/rfc/rfc3986)
  shape: `scheme:[//authority]path[?query][#fragment]`.
- **`Url`** — the common subset that always has an authority, decomposed into
  `username`, `password`, `host` and `port`.
- **`Version`** — a generic `major.minor.patch` version that parses, renders and
  orders numerically.
- **`MimeType`** — an enum of common MIME types, inferred from a file extension
  or from magic bytes (Arrow IPC, Parquet, ZIP, gzip, …). Its extension/magic
  registry is global and can be extended or trimmed at runtime.
- **`MediaType`** — an ordered stack of `MimeType`s for layered files, so
  `data.csv.gz` → `[Csv, Gzip]`. `Uri`/`Url` expose an inferred `media_type()`.

## Layout

```
yggdryl/
├── Cargo.toml                  # Cargo workspace
├── crates/
│   ├── yggdryl-core/           # dependency-free foundations (FromInput/ToOutput, encoding)
│   ├── yggdryl-io/             # dependency-free abstract IO<T> contract (read/write bytes, stream)
│   ├── yggdryl-version/        # standalone Version type
│   ├── yggdryl-media/          # standalone MediaType (MIME) detection
│   └── yggdryl-url/            # Uri/Url, built on (and re-exporting) yggdryl-core + yggdryl-media
└── bindings/
    ├── python/                 # PyO3 + maturin  → `import yggdryl`
    └── node/                   # napi-rs         → `require('yggdryl')`
```

## The core API

Each type is built with `from_str(value)` (or `from_parts` / `from_mapping`)
and exposes its components as read-only accessors. Parsing always validates and
returns an error on malformed input. Rendering takes an `encode` flag —
`to_string(encode=true)` (the default, also what `str()` / `toString()` use)
percent-encodes for transport, `encode=false` decodes for display; both are
cached. Functional `copy(...)` / `with_*` / `without_*` builders and the
multi-valued `params` / `with_params` / `add_param` query CRUD all return new
values without mutating the original. The naming is identical across all three
languages (JS uses camelCase) — see [`CLAUDE.md`](CLAUDE.md).

```rust
use yggdryl::{FromInput, MediaType, MimeType, Uri, Url, Version};

let uri = Uri::from_str("urn:isbn:0451450523")?;
assert_eq!(uri.scheme(), "urn");

let url = Url::from_str("https://example.com/data/sales.csv.gz?a=1&a=2")?;
assert_eq!(url.host(), "example.com");
assert_eq!(url.params(true).get("a"), Some(&vec!["1".into(), "2".into()]));
// A layered media type, inferred from the path's extensions.
assert_eq!(url.media_type().unwrap().types(), [MimeType::Csv, MimeType::Gzip]);
assert_eq!(MimeType::from_magic(b"ARROW1\x00\x00"), Some(MimeType::Arrow));

assert!(Version::from_str("1.4.2")? < Version::from_str("1.10.0")?);
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

### Benchmarks

The hot parsing/rendering paths have lightweight, dependency-free timing
benchmarks (`harness = false` binaries, no benchmark framework):

```bash
cargo bench -p yggdryl-url       # Uri/Url/MediaType parsing, encoding, rendering
cargo bench -p yggdryl-version   # Version parsing / rendering
```

### Logging (optional)

`yggdryl-url` and `yggdryl-media` have an optional `log` feature (off by default,
so the crates stay dependency-free). Enable it to emit `log` events — parse
traces and global MIME-registry changes — through any `log` backend:

```toml
yggdryl-url = { version = "0.1", features = ["log"] }
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
`Release` workflow (`.github/workflows/release.yml`).

## Publishing

Publishing is automated by `.github/workflows/release.yml` and triggered by a
**version bump on `main`** — there is nothing to tag by hand:

1. Bump `version` under `[workspace.package]` in the root `Cargo.toml` (e.g.
   `0.1.1` → `0.1.2`) and merge to `main`.
2. The workflow's `check` job compares that version to the existing
   `v<version>` tags. If it is new, it runs the gate (`fmt` / `clippy` /
   `cargo test`), builds the per-OS wheels and addons, publishes to crates.io,
   PyPI and npm, and finally creates the `v<version>` tag and a GitHub Release
   (with auto-generated notes). If the version is unchanged, only `check` runs.

crates.io and the Python wheels inherit the version (`version.workspace = true`);
the npm `package.json` version is synced from the workspace version at publish
time. `workflow_dispatch` runs the same logic on demand. It needs three
repository secrets — Settings → Secrets and variables → Actions:

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
# Rust — publish in dependency order
cargo publish -p yggdryl-core
cargo publish -p yggdryl-version
cargo publish -p yggdryl-media
cargo publish -p yggdryl-url

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
