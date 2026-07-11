# yggdryl

A Rust-core library with Python and Node.js extensions, built around an
**Apache Arrow-centralized** data model. All logic lives in the Rust core; the
bindings are thin wrappers, so the three languages behave identically. The `codec`,
`compression`, `io`, `buffer`, and wide-integer layers are in place, guided by the
contributor rules in `CLAUDE.md`.

📖 **Documentation: <https://platob.github.io/yggdryl/>**

## Usage

A few of the main examples; see the [documentation](https://platob.github.io/yggdryl/)
for the full surface. The Python and Node bindings mirror the Rust core
method-for-method.

### Compression — gzip / zstd

```python
# Python
from yggdryl import compression

gzip = compression.Gzip()                    # level 6 by default; Zstd() is level 3
packed = gzip.encode_byte_array(b"the quick brown fox" * 8)
assert gzip.decode_byte_array(packed) == b"the quick brown fox" * 8
```

```js
// Node
const { compression } = require('yggdryl')

const gzip = new compression.Gzip()          // level 6 by default
const packed = gzip.encodeByteArray(Buffer.from('the quick brown fox'.repeat(8)))
console.assert(gzip.decodeByteArray(packed).equals(Buffer.from('the quick brown fox'.repeat(8))))
```

```rust
// Rust
use yggdryl_core::{Decoder, Encoder, Gzip};

let gzip = Gzip::default();                   // level 6 by default
let data = b"the quick brown fox".repeat(8);
let packed = gzip.encode_byte_array(&data).unwrap();
assert_eq!(gzip.decode_byte_array(&packed).unwrap(), data);
```

### Typed buffers

```python
# Python
from yggdryl.buffer import I32Buffer

buf = I32Buffer([10, 20, 30])
assert buf.get(1) == 20 and len(buf) == 3
assert I32Buffer.deserialize_bytes(buf.serialize_bytes()) == buf   # byte round-trip
```

```js
// Node
const { I32Buffer } = require('yggdryl').buffer

const buf = new I32Buffer([10, 20, 30])
console.assert(buf.get(1) === 20 && Number(buf.length) === 3)
console.assert(I32Buffer.deserializeBytes(buf.serializeBytes()).equals(buf))
```

```rust
// Rust
use yggdryl_core::I32Buffer;

let buf = I32Buffer::from_slice(&[10, 20, 30]);
assert_eq!(buf.get(1), Some(20));
assert_eq!(I32Buffer::deserialize_bytes(&buf.serialize_bytes()).unwrap(), buf);
```

### Positioned byte IO

```python
# Python
from yggdryl.io import ByteBuffer, Whence

cursor = ByteBuffer(b"hello world").byte_cursor()
assert cursor.pread_byte_array(5) == b"hello"                 # reads at 0, advances to 5
assert cursor.pread_byte_array(6, Whence.Current) == b" world"
```

```js
// Node
const { ByteBuffer, Whence } = require('yggdryl').io

const cursor = new ByteBuffer(Buffer.from('hello world')).byteCursor()
console.assert(cursor.preadByteArray(5).equals(Buffer.from('hello')))
console.assert(cursor.preadByteArray(6, Whence.Current).equals(Buffer.from(' world')))
```

```rust
// Rust
use yggdryl_core::{ByteBuffer, IOBase, Whence};

let mut cursor = ByteBuffer::from_bytes(b"hello world").byte_cursor();
assert_eq!(cursor.pread_byte_array(5, Whence::Start).unwrap(), b"hello");
assert_eq!(cursor.pread_byte_array(6, Whence::Current).unwrap(), b" world");
```

## Layout

- `crates/yggdryl-core` — the Rust core foundations (the `codec`, `compression`, `io`,
  `buffer`, and wide-integer layers so far).
- `bindings/python` — the Python extension (PyO3 / maturin).
- `bindings/node` — the Node.js extension (napi-rs).

See [`CLAUDE.md`](CLAUDE.md) for contributor and agent instructions.

## Develop

### Prerequisites

- **Rust** (stable) via [rustup](https://rustup.rs) — pinned by
  [`rust-toolchain.toml`](rust-toolchain.toml).
- **[uv](https://docs.astral.sh/uv/)** — the Python toolchain for this project
  (venv, installs, building the extension, tests, docs). `uv venv && uv pip install
  maturin`.
- **Node.js ≥ 16** with `npm` — the `@napi-rs/cli` dev-dependency builds the addon.
- **CMake + Ninja + a C compiler** for the bindings — they build the fast
  `gzip-zlib-ng` and `zstd` backends. Install CMake/Ninja with
  `uv run python scripts/setup-build-deps.py`. The pure-Rust core (default `cargo`
  commands, `--no-default-features` for none) needs no C toolchain.

### Rust core

`cargo` with no arguments only touches the pure-Rust core, so no Python or Node
toolchain is needed for the common loop:

```bash
cargo build                 # build the core
cargo test                  # unit, integration, and doctests
cargo bench --workspace     # run the throughput benchmarks (per-crate benches)
```

The core is a library (no binary yet); exercise it through `cargo test`, the
doctests, or the benchmark above.

### Python extension

Built and tested through **[uv](https://docs.astral.sh/uv/)**:

```bash
cd bindings/python
uv run maturin develop      # compile and install into the uv env
uv run python -c "from yggdryl import compression; print(compression.Gzip().name)"
uv run pytest               # run the binding tests
```

### Node extension

```bash
cd bindings/node
npm install                 # once, to fetch @napi-rs/cli
npm run build               # napi build --release -> yggdryl.<triple>.node
node -e "const {compression}=require('.'); console.log(new compression.Gzip().name)"
npm test                    # run the binding tests
```

### Benchmarks

Each surface has a throughput benchmark; the Python and Node scripts compare
`yggdryl` against the platform's native gzip. Build the extensions in **release**
first — a debug build is ~20× slower.

```bash
cargo bench -p yggdryl-compression --bench compression        # Rust core (gzip/zstd)
(cd bindings/python && uv run maturin develop --release) && \
  uv run python bindings/python/benchmarks/bench_compression.py # Python vs stdlib gzip
(cd bindings/node && npm run build) && npm --prefix bindings/node run bench  # Node vs zlib
```

### Before committing

Run the full gate (formatting, lints, tests across all three surfaces, and — when
`docs/` changed — a strict docs build). The authoritative list lives in
[`CLAUDE.md`](CLAUDE.md#required-checks-before-committing):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && uv run maturin develop && uv run pytest)
(cd bindings/node && npm run build && npm test)
uv run mkdocs build --strict  # when docs/ or mkdocs.yml changed
```

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
