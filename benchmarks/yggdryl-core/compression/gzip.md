# `Gzip` — gzip compression

Source: [`crates/yggdryl-core/src/compression/gzip.rs`](../../../crates/yggdryl-core/src/compression/gzip.rs)
· Benches: [`benches/compression.rs`](../../../crates/yggdryl-core/benches/compression.rs)
(one-shot) and [`benches/io.rs`](../../../crates/yggdryl-core/benches/io.rs) (streaming)

Corpus: 1 MiB of English-like text, 200 iterations, `--release`.

## Backends

`Gzip` is `flate2`-backed. Two backends, selected by cargo feature:

- **`gzip`** (default) — pure-Rust `miniz_oxide`; builds anywhere, no C toolchain.
- **`gzip-zlib-ng`** — SIMD-optimised **zlib-ng** (via CMake + Ninja). **The Python
  and Node bindings enable this** so the shipped extensions out-run stock C zlib.

### Core throughput by backend

| Level | | miniz encode | zlib-ng encode | miniz decode | zlib-ng decode |
| --- | --- | --- | --- | --- | --- |
| 1 | | ~1.6 GB/s | ~1.4 GB/s | ~1.6 GB/s | ~1.9 GB/s |
| 6 (default) | | ~0.23 GB/s | **~1.38 GB/s** | ~1.0 GB/s | ~1.77 GB/s |
| 9 | | ~0.23 GB/s | ~0.35 GB/s | ~1.1 GB/s | ~1.18 GB/s |

zlib-ng is **~6× faster at level 6 encode** (the default). It trades a little ratio
for speed (level 6: ~169× vs miniz's ~335× on this corpus), matching stock zlib.

## vs native C zlib (the goal)

Python binding (zlib-ng) vs stdlib `gzip`, from
[`bench_compression.py`](../../../bindings/python/benchmarks/bench_compression.py):

| Level | encode | decode |
| --- | --- | --- |
| 1 | 0.72× | 0.96× |
| **6 (default)** | **5.58×** | 0.92× |
| 9 | 1.28× | 0.94× |

**At the default level 6 the extension is 5.6× faster than Python's stdlib on
encode**, and within noise on decode. Level-1 encode is the one spot behind (stock
zlib's level-1 fast path is hard to beat); decode is at ~parity — the residual gap
is the FFI copy of the output `bytes`, not the codec.

## Optimization: ISIZE decode preallocation

`decode_byte_array` previously grew its output `Vec` blindly (`read_to_end`). The
gzip trailer's **ISIZE** field (last 4 bytes = uncompressed size mod 2³²) gives the
exact size, so we now `Vec::with_capacity` from it (capped at 4096× the compressed
length to bound a hostile ISIZE). This roughly **doubled decode**:

| level-6 decode | before | after |
| --- | --- | --- |
| miniz | ~0.55 GB/s | ~1.0 GB/s |
| zlib-ng | ~0.89 GB/s | ~1.77 GB/s |

## Building the fast backend

zlib-ng needs CMake + Ninja. Run
[`scripts/setup-build-deps.py`](../../../scripts/setup-build-deps.py) (installs both
via `pip`); the workspace pins the Ninja generator in
[`.cargo/config.toml`](../../../.cargo/config.toml). CI installs them automatically.
