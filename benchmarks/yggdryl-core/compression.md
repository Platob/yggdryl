# `compression` — codec throughput & the zero-copy IOBase path

Time **and** memory for the [`Compression`](../../crates/yggdryl-core/src/compression.rs) codecs
(the native Gzip / Zlib / Zstd / Lzma cores behind the `compression` feature) and their
zero-copy [`IOBase`] integration. The point of the yggdryl path is not to beat the native codec
core — it *is* the native core — but to beat a **naive** pipeline that copies the source into a
`Vec` before decoding.

## Run

```bash
cargo bench -p yggdryl-core --features compression --bench compression
cargo test  -p yggdryl-core --features compression --test compression
```

## Rust core (release, counting global allocator, ~1.4 MiB semi-repetitive corpus)

| op | MiB/s | allocs/op | bytes-alloc/op |
|---|--:|--:|--:|
| gzip compress (ratio 14.6×) | 39 | 9 | 1.06 M |
| gzip decompress | 478 | 6 | 4.44 M |
| zlib compress (14.6×) | 41 | 8 | 1.06 M |
| zlib decompress | 377 | 6 | 4.44 M |
| **zstd** compress (27.7×) | **305** | 3 | 0.13 M |
| zstd decompress | 315 | 10 | 4.32 M |
| xz compress (64.4×) | 1.0 | 2 | 0.74 M |
| xz decompress | 125 | 7 | 5.54 M |

### The zero-copy read path (gzip decompress, base = compressed input)

| op | MiB/s | allocs/op | bytes-alloc/op |
|---|--:|--:|--:|
| **yggdryl `decompressed_with` (zero-copy)** | **26.6** | **6** | 4.438 M |
| naive copy-then-decompress | 25.3 | **7** | 4.535 M |

## What the numbers show

- **Pick the codec for the job.** zstd is the balanced default — ~305 MiB/s compress at a
  27.7× ratio; xz squeezes hardest (64×) but compresses slowly (1 MiB/s); gzip/zlib sit in
  between and decompress fastest. All four round-trip losslessly (including empty input) and
  return a **guided error** on corrupt input, tested in `tests/compression.rs`.
- **The zero-copy read wins the pipeline.** `decompressed_with` hands the codec the source's
  **borrowed bytes** ([`as_bytes`](../../crates/yggdryl-core/src/io/memory/base.rs), overridden
  by `Heap` / `Mmap` / a mapped `LocalIO`), so it makes **one fewer allocation** and moves
  ~97 KB less memory (the compressed-input copy the naive path makes) — and runs measurably
  faster. On a memory-mapped source the "read" is a view into OS pages, so nothing is buffered
  before the codec at all. That is the honest "faster than a naive native use" the design
  targets — the codec core is native; the *pipeline* around it is leaner.
- **Recursive magic inference is bounded.** `infer_media_type` peels compression layers with a
  **truncation-tolerant** `decompress_prefix` (a bounded streaming read of the head), so
  inferring `gzip → pdf` never decodes the whole file — it reads only the head it needs.
