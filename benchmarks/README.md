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
| gzip | 6.4× | 28 MiB/s | 481 MiB/s | 471 MiB/s |
| zstd | 6.5× | 288 MiB/s | 1075 MiB/s | 1084 MiB/s |
| snappy | 3.7× | 755 MiB/s | 1276 MiB/s | 1336 MiB/s |

The **`Io`-stream column equals the one-shot column** — wrapping any handle in a
streaming `Encoder`/`Decoder` (themselves `Io` handles) adds no measurable
overhead, so you can compress/decompress a file or an HTTP body a chunk at a time
without buffering the whole payload.

### HTTP — a `requests`-like client streaming over `Io` (`cargo bench -p yggdryl-http`)

| workload | result |
| --- | --- |
| 8 MiB download → `BytesIO` | **1.0 GiB/s** |
| 8 MiB windowed `HttpStream::read_to_end` | **1.35 GiB/s** |
| 16-byte footer via `pread` (one `Range`, no full download) | **0.44 ms** |
| 64 small requests, 5 ms latency — sequential loop | 349 ms |
| 64 small requests, 5 ms latency — `send_many` (concurrency 8) | **57 ms (≈6.1× faster)** |
| 200 tiny requests — pooled keep-alive vs reconnect-each | 33 ms vs 37 ms |

`HttpStream` streams straight off the held connection; a footer read is one
`Range` request, and `send_many` fans a request iterator across the pool.

---

## 2. From Python — same code, two backends (`benchmarks/compare.py`)

The thin binding wins where bulk work runs in Rust in a single FFI call (a
download, a whole-buffer compress); for tiny per-call operations the FFI crossing
dominates, so reach for the bulk / streaming methods.

### HTTP — `yggdryl.HttpSession` vs `requests`

| workload | yggdryl | requests | speedup |
| --- | --- | --- | --- |
| GET small body (latency) | 0.53 ms | 0.83 ms | **1.56×** |
| GET 8 MiB body (throughput) | 912 MiB/s | 530 MiB/s | **1.72×** |

…and yggdryl additionally gives you a **seekable** response body (`pread` a footer
without downloading the whole object), **resume-on-drop** streaming, and
`send_many` concurrency — none of which `requests` offers.

### Compression — `yggdryl.Compression` vs stdlib `gzip`

| workload | yggdryl | stdlib `gzip` | speedup |
| --- | --- | --- | --- |
| gzip compress | 14 MiB/s | 9 MiB/s | **1.54×** |
| gzip decompress | 181 MiB/s | 438 MiB/s | 0.41× |

gzip *decompress* is the one spot the stdlib leads — CPython's `zlib` is a
hand-tuned C decoder, and the binding pays an extra FFI copy. yggdryl's edge here
is breadth, not raw gzip-decode speed:

| codec (no stdlib equivalent) | compress | decompress | ratio |
| --- | --- | --- | --- |
| zstd | 78 MiB/s | 334 MiB/s | 8.2× |
| snappy | 93 MiB/s | 168 MiB/s | 2.0× |

`zstd` compresses to the same size as gzip **~10× faster**, and both ship with the
default wheel — no extra `pip install`.

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
| HTTP GET small body (latency) | *run it* | `node:http` | — |
| HTTP GET 8 MiB (throughput) | *run it* | `node:http` | — |
| gzip compress | *run it* | `node:zlib` | — |
| `zstd` / `snappy` | ✅ built in | ❌ no `node:zlib` equivalent | — |

Run `node benchmarks/compare.mjs` to fill the table on your machine (Node's HTTP
client returns the body in chunks you `Buffer.concat`; yggdryl returns it from
Rust in one call, and additionally offers `zstd`/`snappy`, a seekable body and
`send_many`).

---

## Reproduce

```bash
# Rust core (true ceiling, no FFI)
cargo bench -p yggdryl-core --bench io
cargo bench -p yggdryl-core --bench compression --all-features
cargo bench -p yggdryl-http --all-features

# Python — same code vs requests / gzip (+ memory)
(cd bindings/python && maturin develop) && pip install requests
python3 benchmarks/compare.py

# Node — same code vs node:http / node:zlib
(cd bindings/node && npm run build)
node benchmarks/compare.mjs
```
