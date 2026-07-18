# `compression` — codec throughput, the zero-copy IOBase path & vs. language-native

Time **and** memory for the [`Compression`](../../crates/yggdryl-core/src/compression.rs) codecs
(the Gzip / Zlib / Zstd / Lzma cores behind the `compression` feature) and their zero-copy
[`IOBase`] integration. Two wins are measured here:

1. **Gzip/zlib run on flate2's `zlib-rs` backend** — a pure-Rust port of the SIMD-tuned zlib-ng,
   selected in `crates/yggdryl-core/Cargo.toml` (`default-features = false, features =
   ["zlib-rs"]`). It needs **no C toolchain** (stays cross-platform, no cmake) yet **out-compresses
   the C `zlib`** that Python's `gzip`/`zlib` and Node's `node:zlib` link — see the *vs.
   language-native* section. (zstd/xz remain the native `libzstd` / `liblzma` C cores.)
2. **The pipeline around the codec is leaner** — `decompressed_with` hands the codec a source's
   **borrowed bytes**, one fewer allocation than a naive copy-then-decode.

## Run

```bash
cargo bench -p yggdryl-core --features compression --bench compression
cargo test  -p yggdryl-core --features compression --test compression
# vs. language-native (build the binding in RELEASE first):
python bindings/python/benchmarks/bench_compression_native.py
node   bindings/node/benchmark/compression_native.bench.js
```

## Rust core (release, counting global allocator, ~1.4 MiB semi-repetitive corpus)

| op | MiB/s | allocs/op | bytes-alloc/op |
|---|--:|--:|--:|
| gzip compress (ratio 14.3×) | 103 | 4 | 1.12 M |
| gzip decompress | 574 | 6 | 4.52 M |
| zlib compress (14.3×) | 106 | 3 | 1.12 M |
| zlib decompress | 524 | 6 | 4.52 M |
| **zstd** compress (27.7×) | **291** | 3 | 0.13 M |
| zstd decompress | 354 | 10 | 4.32 M |
| xz compress (64.4×) | 1.1 | 2 | 0.74 M |
| xz decompress | 137 | 7 | 5.54 M |

Switching gzip/zlib from the default `miniz_oxide` to `zlib-rs` raised compress from ~39 to
~103 MiB/s (**~2.6×**) and cut its allocations (9 → 4), at no cost to portability.

### The zero-copy read path (gzip decompress, base = compressed input)

| op | MiB/s | allocs/op | bytes-alloc/op |
|---|--:|--:|--:|
| **yggdryl `decompressed_with` (zero-copy)** | **34.9** | **6** | 4.518 M |
| naive copy-then-decompress | 31.8 | **7** | 4.617 M |

## vs. language-native (through the bindings, ~4.7 MiB corpus, matched level 6/3)

The binding boundary must be **zero-copy** for this to be a fair fight: a byte input is taken as a
borrowed buffer (Python `PyBackedBytes`, Node `Buffer` → `&[u8]`), never re-extracted element by
element into an owned `Vec<u8>` (which alone cost **~5× on compress** before it was fixed).

| codec / op | vs CPython `zlib` | vs `node:zlib` |
|---|--:|--:|
| gzip compress | **1.42× faster** | **2.04× faster** |
| gzip decompress | 0.83× | **1.96× faster** |
| zlib compress | **1.26× faster** | **1.97× faster** |
| zlib decompress | 0.60× | **1.59× faster** |
| zstd compress | **1.18× faster** | 0.89× |
| zstd decompress | 0.87× | **1.18× faster** |

## What the numbers show

- **Gzip/zlib compress beats the C `zlib`** both runtimes ship — `zlib-rs`'s deflate is simply
  faster, and the win survives the binding once its inputs are zero-copy. Against CPython's very
  thin `zlib` wrapper the pure-Rust *inflate* still trails on decompress (0.6–0.83×); against
  Node's heavier synchronous `zlib` yggdryl wins **both** directions (~1.6–2.0×). Reported
  honestly — the decompress deficit vs CPython is real, not hidden.
- **Pick the codec for the job.** zstd is the balanced default — high throughput at a 27.7× ratio;
  xz squeezes hardest (64×) but compresses slowly; gzip/zlib sit between and decompress fastest.
  All four round-trip losslessly (including empty input) and return a **guided error** on corrupt
  input, tested in `tests/compression.rs`. The streams are interchange-compatible with the native
  codecs (the benchmarks decompress each side's output on the other).
- **The zero-copy read wins the pipeline.** `decompressed_with` hands the codec the source's
  **borrowed bytes** ([`as_bytes`](../../crates/yggdryl-core/src/io/memory/base.rs), overridden by
  `Heap` / `Mmap` / a mapped `LocalIO`), so it makes **one fewer allocation**. On a memory-mapped
  source the "read" is a view into OS pages, so nothing is buffered before the codec at all.
- **Recursive magic inference is bounded.** `infer_media_type` peels compression layers with a
  **truncation-tolerant** `decompress_prefix` (a bounded streaming read of the head), so inferring
  `gzip → pdf` never decodes the whole file — it reads only the head it needs.
