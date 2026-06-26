# yggdryl — benchmarks

Numbers for the **yggdryl** byte-IO / compression / HTTP stack, measured in
**speed and memory**, three ways:

1. **The Rust core** (`cargo bench`) — the library's true ceiling, with no FFI in
   the path. This is what you get from Rust, and what the bindings stream through.
2. **From Python** (`python3 benchmarks/compare.py`) — the *same high-level code*
   run through yggdryl and through the Python stalwarts (`requests`, the stdlib
   `gzip`), on the same in-process server / in-memory payload, plus peak-heap
   (`tracemalloc`) memory figures.
3. **From Node** (`node benchmarks/compare.mjs`) — the same against Node's built-in
   `http` client and `zlib`.

All figures are from one developer machine (localhost, no real network); treat
them as ratios, not absolutes, and re-run them yourself — every harness is in this
folder and in each crate's `benches/`.

> Honesty first: yggdryl is not faster at *everything*. The wins below are real
> and reproduced here; the one place the Python stdlib still leads (gzip
> *decompress*, where `zlib` is a hand-tuned C library) is shown too.

---

## 1. The Rust core (`cargo bench`)

### Byte IO — the unified `Io` trait (`cargo bench -p yggdryl-core --bench io`)

| operation | result |
| --- | --- |
| `BytesIO::seek` | **0.9 ns** |
| `Io::pread` (memory, positional) | **1.3 ns** |
| `Io::read` (4 KiB streamed) | **1.2 ns** |
| `copy` BytesIO → BytesIO (zero-copy) | **8.4 GiB/s** |
| `read_to_end` BytesIO → Vec (chunked) | **13.7 GiB/s** |
| `Frames::write` (256 B frame) | 34 ns |

The single `Io` trait carries reads, writes and the cursor; a memory-resident
backend serves `read`/`pread` straight off `as_slice`, so positional access is a
slice copy and a transfer is one `memcpy`.

### Compression — streamed codecs over `Io` (`cargo bench -p yggdryl-core --bench compression`)

| codec | ratio | compress | decompress | `Io`-stream decompress |
| --- | --- | --- | --- | --- |
| gzip | 6.4× | 35 MiB/s | 489 MiB/s | 496 MiB/s |
| zstd | 6.5× | 356 MiB/s | 1200 MiB/s | 1225 MiB/s |
| snappy | 3.7× | 760 MiB/s | 1290 MiB/s | 1340 MiB/s |
| brotli | **7.5×** | 27 MiB/s | 442 MiB/s | 453 MiB/s |

Brotli gives the **best ratio** of the four (it compresses smallest) at a higher
CPU cost — a good default for a body downloaded far more often than it is built.

The **`Io`-stream column equals the one-shot column** — wrapping any handle in a
streaming `Encoder`/`Decoder` (themselves `Io` handles) adds no measurable
overhead, so you can compress/decompress a file or an HTTP body a chunk at a time
without buffering the whole payload.

### HTTP — a `requests`-like client streaming over `Io` (`cargo bench -p yggdryl-http`)

| workload | result |
| --- | --- |
| 8 MiB download → `BytesIO` | **1.0 GiB/s** |
| 8 MiB windowed `HttpStream::read_to_end` | **1.35 GiB/s** |
| 16-byte footer via `pread` (one `Range`, no full download) | **0.28 ms** |
| 64 small requests, 5 ms latency — sequential loop | 344 ms |
| 64 small requests, 5 ms latency — `send_many` (concurrency 8) | **56 ms (≈6.1× faster)** |
| 200 tiny requests — pooled keep-alive vs reconnect-each | 23 ms vs 40 ms |

`HttpStream` streams straight off the held connection; a footer read is one
`Range` request, and `send_many` fans a request iterator across the pool.

---

## 2. From Python — same code, two backends (`benchmarks/compare.py`)

The thin binding wins where bulk work runs in Rust in a single FFI call (a
download, a whole-buffer compress); for tiny per-call operations the FFI crossing
dominates, so reach for the bulk / streaming methods.

### HTTP — `yggdryl.HttpSession` vs `requests` / `httpx`

| workload | yggdryl | requests | httpx | vs requests |
| --- | --- | --- | --- | --- |
| GET small body (latency) | 0.20 ms | 0.74 ms | 0.61 ms | **3.6×** |
| GET 8 MiB body (throughput) | 1353 MiB/s | 590 MiB/s | 748 MiB/s | **2.3×** |

…and yggdryl additionally gives you a **seekable** response body (`pread` a footer
without downloading the whole object), **resume-on-drop** streaming, transparent
`gzip`/`zstd`/`snappy`/`brotli` decoding, and `send_many` concurrency — none of
which `requests` or the sync `httpx` client offers.

### Compression — `yggdryl.Compression` vs stdlib `gzip`

| workload | yggdryl | stdlib `gzip` | speedup |
| --- | --- | --- | --- |
| gzip compress | 18 MiB/s | 11 MiB/s | **1.6×** |
| gzip decompress | 209 MiB/s | 497 MiB/s | 0.42× |

gzip *decompress* is the one spot the stdlib leads — CPython's `zlib` is a
hand-tuned C decoder, and the binding pays an extra FFI copy. yggdryl's edge here
is breadth, not raw gzip-decode speed:

| codec (no stdlib equivalent) | compress | decompress | ratio |
| --- | --- | --- | --- |
| zstd | 77 MiB/s | 361 MiB/s | 8.2× |
| snappy | 88 MiB/s | 158 MiB/s | 2.0× |
| brotli | 12 MiB/s | 230 MiB/s | **10.0×** |

`zstd` compresses to the same size as gzip **~10× faster**, and `brotli` packs the
**smallest** of all (10× ratio) — all four ship with the default wheel, no extra
`pip install`.

### Memory — peak host-heap for the same result

`tracemalloc` peak heap held while producing the identical output. yggdryl runs
the bulk work in Rust and hands one buffer across the FFI, so the host heap stays
flatter than the pure-Python path with its intermediate objects:

| workload | yggdryl | Python stdlib |
| --- | --- | --- |
| gzip compress (peak heap) | ~0.1 MiB | ~5 MiB |
| gzip decompress (peak heap) | ~7 MiB | ~12 MiB |

The deeper win is **streaming**: in the Rust core an `HttpStream` reads a
multi-gigabyte object in a bounded 4 MiB window and `pread`s a footer with one
`Range` request — never holding the whole body in memory at all.

---

## 3. From Node — same code, two backends (`benchmarks/compare.mjs`)

The same workloads against Node's built-in `http` client and `zlib`.

| workload | yggdryl | node built-in | speedup |
| --- | --- | --- | --- |
| HTTP GET small body (latency) | 0.63 ms | `node:http` 0.23 ms | 0.36× |
| HTTP GET 8 MiB (throughput) | 622 MiB/s | `node:http` 875 MiB/s | 0.71× |
| gzip compress | 22 MiB/s | `node:zlib` 31 MiB/s | 0.71× |
| `zstd` / `snappy` / `brotli` | 263 / 408 / 14 MiB/s | ❌ no built-in `zstd`/`snappy` | — |

This is **the one place a host runtime leads**: Node's `http` and `zlib` are
highly-tuned C++/C, and the napi binding pays a `Promise` + FFI crossing per call,
so for localhost round-trips and gzip the built-ins win. yggdryl's value in Node is
**breadth and shape** — `zstd`/`snappy`/`brotli` (no `node:zlib` equivalent for the
first two), a **seekable** body with `pread`, resume-on-drop streaming, and
`send_many` concurrency — not raw localhost throughput. (Brotli `compress` is slow
by design; it earns the smallest body — see the ratios above.)

---

## Reproduce

```bash
# Rust core (true ceiling, no FFI)
cargo bench -p yggdryl-core --bench io
cargo bench -p yggdryl-core --bench compression --all-features
cargo bench -p yggdryl-http --all-features

# Python — same code vs requests / httpx / gzip (+ memory)
(cd bindings/python && maturin develop --release) && pip install requests httpx
python3 benchmarks/compare.py

# Node — same code vs node:http / node:zlib
(cd bindings/node && npm run build)
node benchmarks/compare.mjs
```
