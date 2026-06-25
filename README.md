# yggdryl

**One streaming byte-IO core — `Io`, compression and HTTP — for Rust, Python and
Node.** Write the same high-level code in any of the three languages and get
near-Rust throughput, because all the work happens in a dependency-light Rust core
and the bindings are thin wrappers that **never copy your bytes through the host
language**.

📖 **[Documentation → platob.github.io/yggdryl](https://platob.github.io/yggdryl/)**
 · 📊 **[Benchmarks](benchmarks/)** · one implementation, three packages
([crates.io](https://crates.io) / [PyPI](https://pypi.org) / [npm](https://www.npmjs.com)).

```python
# Python — looks like requests, runs in Rust
import yggdryl
data = yggdryl.HttpSession().get("https://example.com/data.csv.gz").content
```
```javascript
// Node — looks like fetch/axios, runs in Rust
const { HttpSession } = require("yggdryl");
const data = (await new HttpSession().get("https://example.com/data.csv.gz")).content;
```
```rust
// Rust — the core
let data = yggdryl_http::HttpSession::new().get("https://example.com/data.csv.gz")?.bytes()?;
```

## Why

A reader (Arrow / Parquet / CSV / JSON) should not care **where** its bytes live —
memory, a memory-mapped file, or a remote HTTP object — nor whether they arrive
**random** (a footer, a column chunk) or **streamed** (scan record batches).
yggdryl unifies all of it behind **one trait, `Io`**, and builds compression and a
`requests`-like HTTP client on top, each handle composing with the next with **at
most one copy** of the data.

## Performance — same code, real gains

Measured on one developer machine (localhost, no real network) — ratios, not
absolutes; reproduce with [`benchmarks/`](benchmarks/) and `cargo bench`.

| workload | yggdryl | host-language baseline | speedup |
| --- | --- | --- | --- |
| HTTP GET, small body + latency (Python) | 0.53 ms | `requests` 0.83 ms | **1.6×** |
| HTTP GET, 8 MiB throughput (Python) | 912 MiB/s | `requests` 530 MiB/s | **1.7×** |
| gzip compress (Python) | 14 MiB/s | stdlib `gzip` 9 MiB/s | **1.5×** |
| `zstd` / `snappy` codecs | ✅ built in | ❌ not in stdlib | — |
| `copy` BytesIO → BytesIO (Rust core) | **8.4 GiB/s** | — | zero-copy |
| `HttpStream` windowed read (Rust core) | **1.35 GiB/s** | — | streamed |
| footer via `pread` (one Range request) | **0.44 ms** | full download | no download |
| `send_many` vs sequential (Rust core) | **≈6×** | — | concurrent |

See **[benchmarks/README.md](benchmarks/README.md)** for the full tables, the Node
comparison, memory figures, and the one spot the C `zlib` decoder still leads.

## What's inside

Two crates: **`yggdryl-core`** (all the data types + byte IO + compression) and
**`yggdryl-http`** (the network client), each split into one-file-per-type modules.

| `yggdryl-core` module | what it is |
| --- | --- |
| **`io`** | the one byte-IO trait `Io` — read/write/seek/`pread`/`pwrite` over memory (`BytesIO`), local mmap (`LocalPath`) or cloud; codecs; the `from_str`/`from_url` factory |
| **`compression`** | streamed gzip / Zstd / Snappy (on by default) — encoders/decoders are themselves `Io` handles |
| **`url`** | `Uri` / `Url` (RFC 3986) with query CRUD and inferred media types |
| **`media`** | `MimeType` / `MediaType` from extension or magic bytes |
| **`version`** | a standalone `Version` type |
| `encoding` / `mapping` / `output` | dependency-free foundations (`ToOutput`, `Mapping`/`Params`, percent-encoding) |

**`yggdryl-http`** — a `requests`-like blocking client built on `yggdryl-core`:
pooling, retries with resume-on-drop, a **seekable** response body, `send_many`,
cookies, redirects.

Bindings live under `bindings/python` (PyO3 + maturin → `import yggdryl`) and
`bindings/node` (napi-rs → `require('yggdryl')`). Every type is built with
`from_str(value)` (or `from_parts` / `from_mapping`), validates on parse, and
exposes read-only accessors; the naming is identical across all three languages
(JS uses camelCase) — see [`CLAUDE.md`](CLAUDE.md) and the
[docs](https://platob.github.io/yggdryl/).

```rust
use yggdryl::{MediaType, MimeType, Uri, Url, Version};

let url = Url::from_str("https://example.com/data/sales.csv.gz?a=1&a=2")?;
assert_eq!(url.host(), "example.com");
// A layered media type, inferred from the path's extensions.
assert_eq!(url.media_type().unwrap().types(), [MimeType::Csv, MimeType::Gzip]);
assert!(Version::from_str("1.4.2")? < Version::from_str("1.10.0")?);
# Ok::<(), yggdryl::UrlError>(())
```

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

Lightweight, dependency-free timing benchmarks (`harness = false` binaries, no
framework) plus same-code comparisons against the host-language stalwarts — see
the [`benchmarks/`](benchmarks/) folder for the methodology and full tables.

```bash
cargo bench -p yggdryl-core --bench io                    # Io: cursor, pread, copy, codecs
cargo bench -p yggdryl-core --bench compression --all-features   # gzip/zstd/snappy, one-shot vs Io-stream
cargo bench -p yggdryl-http --all-features   # download, footer pread, send_many
cargo bench -p yggdryl-core --bench url                   # Uri/Url/MediaType parsing, encoding
cargo bench -p yggdryl-core --bench version               # Version parsing / rendering

# Same high-level code, yggdryl vs requests / stdlib gzip / Node http+zlib
python3 benchmarks/compare.py
node benchmarks/compare.mjs
```

### Documentation site

The docs site is built with MkDocs (Material) and published to
[platob.github.io/yggdryl](https://platob.github.io/yggdryl/) on every push to
`main`:

```bash
pip install mkdocs-material && mkdocs serve   # preview at http://127.0.0.1:8000
```

### Logging (optional)

`yggdryl-core` has an optional `log` feature (off by default, so the crate stays
dependency-free). Enable it to emit `log` events — parse traces, global
MIME-registry changes, scheme registration — through any `log` backend:

```toml
yggdryl-core = { version = "0.1", features = ["log"] }
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
cargo publish -p yggdryl-http

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
