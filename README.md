# yggdryl

**One abstract byte-access contract. Every source. Three languages.**

yggdryl is a Rust library — with first-class **Python** (PyO3/maturin) and **Node** (napi-rs)
extensions — built around a single idea: *everything reads and writes through one contract*. Write a
new source once (in-heap, memory-mapped file, device memory, compressed, network…) and it works
everywhere the contract does, at native speed, with the **fewest copies and the least reallocation**
the operation allows.

From that foundation the library grows **upward** into typed data serialization — a precise element
type system, columns, and Arrow-style interop over the same bytes — so ingestion is broad at the
edge, the representation is exact underneath, and everything downstream is fast.

> 📖 **Docs:** <https://platob.github.io/yggdryl/> · 📊 **Benchmarks:**
> [`benchmarks/`](benchmarks/yggdryl-core) · 📦 crates.io · PyPI · npm

---

## Why yggdryl

- **`IOBase` — one contract, bytes + address + graph.** Positioned typed reads/writes
  (`pread_i32` / `pwrite_f64_array` / `pread_utf8`), a moving **cursor**, bounded **slices**,
  capacity control, *and* the filesystem-style graph surface (`ls` / `parent` / `rm`, a directory
  that serves the byte contract too) — all on one trait. A new source implements a few required
  methods and inherits the rest.
- **Zero-copy where it counts.** Bulk typed operations are **vectorized** dense loops the compiler
  auto-vectorizes on stable Rust (no SIMD dependency); every bulk op ships an allocation-free
  *fill-into* / *read-into* twin; a mapped file is read as a view into OS pages. Performance claims
  come with a **time _and_ memory** benchmark and a deterministic allocation test.
- **Sources behind the one contract** — an in-heap [`Heap`](docs/io/memory.md), a self-optimizing
  local-filesystem [`LocalIO`](docs/io/local.md) (lazy, auto-creating, memory-maps on first write),
  and an AMD Radeon device-memory [`AmdHeap`](docs/io/amd.md) with live hardware detection and a
  GPU-vs-CPU compute dispatch. Each gets its own named cursor/slice, all sharing one zero-copy
  optimization.
- **Element-typed bytes + vectorized stats.** A byte region carries a
  [`DataTypeId`](crates/yggdryl-core/src/datatype_id.rs); `resize_dtype` widens/shrinks between
  widths, `mask_filter` compacts by a bit buffer, and the `Aggregate` trait reduces
  `sum`/`min`/`max`/`mean`/`std`/`first`/`last`/`count_ge` over **every** source — allocation-free,
  NaN-safe, 2–3 Gelem/s on `i32`.
- **Compression that beats the language-native codec.** Gzip/Zlib run on flate2's pure-Rust
  `zlib-rs` backend (no C toolchain) and, with a zero-copy binding boundary, **out-compress** the C
  `zlib` that Python's `gzip` and Node's `zlib` link — 1.4× (CPython) / 2.0× (Node) on gzip compress.
  Zstd and Lzma round it out. See [the report](benchmarks/yggdryl-core/compression.md).
- **Addressing done right.** A full RFC 3986 [`Uri` / `Url` / `Authority`](docs/uri.md) family, one
  metadata map ([`Headers`](docs/headers.md) — ordered, case-insensitive, multi-value), and a
  media-type layer ([`MimeType` / `MediaType`](docs/mediatype.md)) with magic-byte inference.
- **The same shape everywhere.** The module tree is mirrored across the Rust core, the two bindings,
  the tests, the benchmarks, and the docs — a reader finds the same structure in every language, and
  every public method has a matching test on all three surfaces.

## Install

```toml
# Rust
[dependencies]
yggdryl-core = "0.1"
```

```bash
# Python                         # Node
pip install yggdryl             npm install yggdryl
```

## A taste

**Rust**

```rust
use yggdryl_core::io::memory::{Heap, IOBase, Aggregate};

let mut h = Heap::new();
h.pwrite_i64_array(0, &[4, 8, 15, 16, 23, 42]).unwrap(); // vectorized bulk write
h.set_dtype(yggdryl_core::datatype_id::DataTypeId::I64);
assert_eq!(h.element_count(), 6);
assert_eq!(h.sum_i64(0, 6).unwrap(), 108);               // allocation-free reduce
assert_eq!(h.max_i64(0, 6).unwrap(), Some(42));
```

```python
# Python
from yggdryl.memory import Heap

h = Heap()
h.pwrite_i64_array(0, [4, 8, 15, 16, 23, 42])
assert h.sum_i64(0, 6) == 108
assert h.max_i64(0, 6) == 42
```

```javascript
// Node
const { Heap } = require('yggdryl').memory

const h = new Heap()
h.pwriteI64Array(0, [4n, 8n, 15n, 16n, 23n, 42n])
console.assert(h.sumI64(0, 6) === 108n)
```

## Cargo features

The core is **dependency-free by default** — the `IOBase` contract, the URI/media-type layers, and
magic inference are all `std`. Opt in to native extras:

| feature | adds |
|---|---|
| `compression` | Gzip / Zlib / Zstd / Lzma codecs (flate2 `zlib-rs`, zstd, xz2) |
| `amd` | the AMD Radeon device-memory family (`io::amd`), live detection, GPU-vs-CPU dispatch |

## Documentation & benchmarks

- **Guide (three-language tabs):** <https://platob.github.io/yggdryl/> — [memory](docs/io/memory.md),
  [local](docs/io/local.md), [amd](docs/io/amd.md), [uri](docs/uri.md), [headers](docs/headers.md),
  [media types](docs/mediatype.md), [compression](docs/compression.md).
- **Benchmark reports** (time + memory, with allocation checks):
  [`benchmarks/yggdryl-core/`](benchmarks/yggdryl-core) — including
  [compression vs native](benchmarks/yggdryl-core/compression.md) and
  [CPU-vs-GPU compute](benchmarks/yggdryl-core/io/amd.md).

## Building from source

```bash
cargo test                                             # core (default features)
cargo clippy --workspace --all-targets -- -D warnings
(cd bindings/python && uv run maturin develop && uv run pytest)
(cd bindings/node && npm run build && npm test)
```

## License

See [`Cargo.toml`](Cargo.toml) for the workspace license.
