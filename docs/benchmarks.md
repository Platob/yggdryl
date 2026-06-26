# Benchmarks

yggdryl ships two reproducible harnesses, both in the repo: the **Rust core**
(`cargo bench` — the library's true ceiling, no FFI in the path) and the
**bindings** (`benchmarks/compare.py`, `benchmarks/compare.mjs` — the *same
high-level code* run through yggdryl and through the host-language stalwarts on the
same in-process server and in-memory payload). The page below is organised by
theme: [HTTP](#http), [Compression](#compression), and the
[core byte-IO](#core-byte-io).

All figures are from one developer machine (localhost, no real network) — treat
them as ratios, not absolutes, and re-run them yourself.

!!! note "Honesty first"
    yggdryl is not faster at *everything*. The wins below are real and reproduced
    here; gzip *decompress* (where `zlib` is a hand-tuned C decoder) is roughly a
    wash. We show that case too rather than hide it.

## HTTP

The HTTP client streams the response body straight off the socket into a
Rust-backed buffer; the binding hands that buffer back without a native copy, so a
download is one Rust call rather than a chunk-collecting loop in the host language.
The bindings benchmark replays the `requests` / `node:http`-equivalent GET against
the *same* in-process server (both sides set `TCP_NODELAY` for a fair fight).

**Node binding vs `node:http`** (same in-process server):

| workload | yggdryl | `node:http` | speedup |
| --- | --- | --- | --- |
| GET small body (latency) | — | — | ≈1.4× |
| GET 8 MiB body (throughput) | 1093 MiB/s | 770 MiB/s | **1.42×** |

**Python binding vs `requests`** (same in-process server):

| workload | yggdryl | `requests` | speedup |
| --- | --- | --- | --- |
| GET small body (latency) | 0.53 ms | 0.83 ms | **1.6×** |
| GET 8 MiB body (throughput) | 912 MiB/s | 530 MiB/s | **1.7×** |

The Rust core goes further: the windowed `HttpStream` reads to end at full memory
speed, a remote footer is one `Range` request (no full download), and concurrent
`send_many` beats a sequential loop by roughly the concurrency factor.

| workload | result |
| --- | --- |
| `HttpStream` windowed `read_to_end` (8 MiB) | 1.35 GiB/s |
| footer via `pread` (one Range, no full download) | 0.44 ms |
| `send_many` (concurrency 8) vs sequential | ≈6× |

```bash
# Rust core — in-process server, no network
cargo bench -p yggdryl-http --all-features

# Same code vs the host-language stalwarts
(cd bindings/node && npm run build) && node benchmarks/compare.mjs
(cd bindings/python && maturin develop) && pip install requests && python3 benchmarks/compare.py
```

## Compression

The compression codecs stream over the [`Io`](core/io.md) abstraction, so a
whole-buffer compress is a single Rust call from the bindings — no per-chunk FFI.
The gzip backend is **pure-Rust [zlib-rs](https://github.com/trifectatechfoundation/zlib-rs)**
(no C `zlib` linkage), and it is what lets the *compress* path beat `node:zlib`
outright. The benchmark uses a semi-compressible CSV payload (the shape of real
columnar/log data).

**Node binding vs `node:zlib`** (CSV payload):

| workload | yggdryl | `node:zlib` | speedup |
| --- | --- | --- | --- |
| gzip compress | 67 MiB/s | 31 MiB/s | **2.2×** |
| gzip decompress | 491 MiB/s | 450 MiB/s | **1.09×** |

**Python binding vs stdlib `gzip`**:

| workload | yggdryl | stdlib | speedup |
| --- | --- | --- | --- |
| gzip compress | 14 MiB/s | 9 MiB/s | **1.5×** |

yggdryl also exposes codecs the standard libraries lack — `zstd`, `snappy`, and
`brotli` — through the *same* `Compression` API, so there is nothing to compare
against in `node:zlib` / Python `gzip`:

| codec | available in yggdryl | `node:zlib` / stdlib |
| --- | --- | --- |
| zstd | yes | *(not in stdlib)* |
| snappy | yes | *(not in stdlib)* |
| brotli | yes | built-in (Node), *(not in stdlib gzip)* |

The Rust core benchmark reports one-shot and streamed (`Io`) throughput per codec;
the streamed path matches the one-shot path (no per-iteration buffer copy).

```bash
# Rust core — one-shot + Io-stream throughput per codec
cargo bench -p yggdryl-core --bench compression --all-features

# Same code vs the host-language stalwarts
(cd bindings/node && npm run build) && node benchmarks/compare.mjs
(cd bindings/python && maturin develop) && python3 benchmarks/compare.py
```

!!! tip "A codec whose feature is off is skipped"
    Each codec is an optional cargo feature (all on by `default`). Run with
    `--all-features` to bench every backend; an unavailable one is reported as
    `(feature off)` rather than failing.

## Core byte-IO

Every byte access funnels through the [`Io`](core/io.md) trait, which is where the
zero-copy wins live: a positional `pread` over memory is a borrow, and a
`BytesIO → BytesIO` copy is a single `memcpy`. These are the library's true
ceiling — pure Rust, no FFI, no server.

| workload | result |
| --- | --- |
| `Io::pread` (memory, positional) | 1.3 ns |
| `copy` BytesIO → BytesIO (zero-copy) | 8.4 GiB/s |
| `read_to_end` chunked | 13.7 GiB/s |
| compression `Io`-stream decompress | = one-shot (no overhead) |

```bash
cargo bench -p yggdryl-core --bench io
```

See the repo's [`benchmarks/`](https://github.com/Platob/yggdryl/tree/main/benchmarks)
folder for the full tables and the measurement methodology, and
[Getting started](getting-started.md) for the APIs these numbers exercise.
