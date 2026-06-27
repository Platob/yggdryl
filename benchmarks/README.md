# yggdryl — benchmarks

Numbers for the **yggdryl** byte-IO / compression / HTTP stack, organised **by
theme**. Within each theme the same workload is measured three ways:

- **Rust core** (`cargo bench`) — the library's true ceiling, no FFI in the path.
- **From Python** (`python3 benchmarks/compare.py`) — the *same high-level code*
  through yggdryl and through the Python stalwarts (`requests` / `httpx`, the stdlib
  `gzip`), on one in-process server / in-memory payload, plus peak-heap memory.
- **From Node** (`node benchmarks/compare.mjs`) — the same against Node's built-in
  `http` client and `zlib`.

All figures are from one developer machine (localhost, no real network); treat them
as ratios, not absolutes, and re-run them yourself — see [Reproduce](#reproduce).

> Honesty first: yggdryl is not faster at *everything*. The wins are real and
> reproduced here; the one place a host stdlib still leads (gzip *decompress* from
> Python, where CPython's `zlib` is hand-tuned C and the binding pays an FFI copy)
> is shown too.

---

## HTTP

A `requests`-like client streaming over the `Io` abstraction — pooled connections,
retries with resume-on-drop, a **seekable** body, and `send_many` concurrency.

### Rust core — `cargo bench -p yggdryl-http`

| workload | result |
| --- | --- |
| 8 MiB download → `BytesIO` | **1.0 GiB/s** |
| 8 MiB windowed `HttpStream::read_to_end` | **1.35 GiB/s** |
| 16-byte footer via `pread` (one `Range`, no full download) | **0.28 ms** |
| 64 small requests (5 ms latency) — sequential vs `send_many` (8) | 344 ms → **56 ms (≈6.1×)** |
| 200 tiny requests — pooled keep-alive vs reconnect-each | 23 ms vs 40 ms |

### From Python — vs `requests` / `httpx`

| workload | yggdryl | requests | httpx | vs requests |
| --- | --- | --- | --- | --- |
| GET small body (latency) | 0.21 ms | 0.75 ms | 0.62 ms | **3.6×** |
| GET 8 MiB body (throughput) | 1573 MiB/s | 723 MiB/s | 697 MiB/s | **2.2×** |

### From Node — vs `node:http`

| workload | yggdryl | node:http | speedup |
| --- | --- | --- | --- |
| GET 8 MiB body (throughput) | 1093 MiB/s | 770 MiB/s | **1.42×** |
| GET small body (latency) | 0.60 ms | 0.24 ms | 0.39× |

The 8 MiB throughput win comes from backing the response body with a single
ref-counted `Buffer` (no redundant copy). Tiny-body latency is bound by the
`Promise` + FFI crossing — reach for the bulk / streaming methods. yggdryl also
adds a **seekable** body (`pread` a footer without downloading the object),
resume-on-drop streaming, transparent decompression and `send_many`, which none of
the baselines offer.

---

## Compression

Streamed codecs that are themselves `Io` handles. gzip/deflate run on `flate2`'s
**pure-Rust `zlib-rs` backend** (no C/cmake), so the wheels and npm packages stay
pure-Rust while matching C-zlib throughput.

### Rust core — `cargo bench -p yggdryl-core --bench compression`

| codec | ratio | compress | decompress | `Io`-stream decompress |
| --- | --- | --- | --- | --- |
| gzip / deflate¹ | 6.2× | 106 MiB/s | 693 MiB/s | 710 MiB/s |
| zstd | 6.5× | 356 MiB/s | 1207 MiB/s | 1220 MiB/s |
| snappy | 3.7× | 867 MiB/s | 1317 MiB/s | 1351 MiB/s |
| brotli | **7.5×** | 28 MiB/s | 438 MiB/s | 441 MiB/s |

¹ `deflate` is the zlib format (HTTP `Content-Encoding: deflate`); it shares the
gzip backend, so its throughput tracks gzip's. The **`Io`-stream column equals the
one-shot column** — wrapping a handle in a streaming `Encoder`/`Decoder` adds no
measurable overhead, so a file or HTTP body compresses/decompresses a chunk at a
time without buffering the whole payload.

### From Python — vs stdlib `gzip`

| workload | yggdryl | stdlib `gzip` | speedup |
| --- | --- | --- | --- |
| gzip compress | 40 MiB/s | 11 MiB/s | **3.6×** |
| gzip decompress | 227 MiB/s | 498 MiB/s | 0.45× |

gzip *decompress* is the one spot the stdlib leads (CPython's C `zlib` decoder plus
an FFI copy on our side). The breadth wins are the codecs the stdlib has no
equivalent for — all shipped in the default wheel:

| codec | compress | decompress | ratio |
| --- | --- | --- | --- |
| zstd | 74 MiB/s | 368 MiB/s | 8.2× |
| snappy | 84 MiB/s | 157 MiB/s | 2.0× |
| brotli | 12 MiB/s | 226 MiB/s | **10.0×** |

Peak host-heap for the same result (`tracemalloc`): gzip compress **0.9 MiB** vs
2.2 MiB; gzip decompress **3.1 MiB** vs 9.4 MiB — the bulk work runs in Rust and
hands one buffer across the FFI.

### From Node — vs `node:zlib`

| workload | yggdryl | node:zlib | speedup |
| --- | --- | --- | --- |
| gzip compress | 67 MiB/s | 31 MiB/s | **2.2×** |
| gzip decompress | 491 MiB/s | 450 MiB/s | **1.09×** |
| zstd / snappy / brotli (compress) | 255 / 413 / 14 MiB/s | ❌ no built-in zstd/snappy | — |

With the `zlib-rs` backend yggdryl now **beats `node:zlib` on both gzip compress and
decompress**, and adds zstd/snappy/brotli with no `node:zlib` equivalent for the
first two.

---

## Byte IO — the unified `Io` trait

`cargo bench -p yggdryl-core --bench io`. One trait carries reads, writes and the
cursor; a memory-resident backend serves `read`/`pread` straight off `as_slice`, so
positional access is a slice copy and a transfer is one `memcpy`. The bindings
stream through this same core.

| operation | result |
| --- | --- |
| `BytesIO::seek` | **0.9 ns** |
| `Io::pread` (memory, positional) | **1.3 ns** |
| `Io::read` (4 KiB streamed) | **1.2 ns** |
| `copy` BytesIO → BytesIO (zero-copy) | **8.4 GiB/s** |
| `read_to_end` BytesIO → Vec (chunked) | **13.7 GiB/s** |
| `Frames::write` (256 B frame) | 34 ns |

---

## Schema & time

The `yggdryl-schema` `DataType` / `Field` layer and the core calendar/time module.
The fast type checks are the point: routing a value by its type or reading its
physical width is sub-nanosecond, so a batch/column metadata pass is essentially
free.

### Rust core — `cargo bench -p yggdryl-schema --bench schema --features arrow`

| workload | result |
| --- | --- |
| `DataType::is_numeric` / `category` / `bit_size` (fast checks) | **0.8–1.2 ns** |
| `DataType::can_cast_to` | **5.9 ns** |
| `DataType::common_type` (int promotion) | **9.8 ns** |
| `DataType::from_str` (`int64`) | 32 ns |
| `DataType::from_str` (`timestamp[us, UTC]`) | 138 ns |
| `DataType::from_str` (nested struct, 3 fields) | 0.93 µs |
| `DataType::merge` (two 8-field structs, promote) | 1.1 µs |
| `Field::to_arrow_schema` / `from_arrow_schema` (8 fields) | 1.3 µs / 0.61 µs |

The conversion to/from `arrow-schema` is a cheap structural walk; the type checks
and category lookup are branch-only (no allocation), so the metadata operations a
dataframe runs per batch — type unification, cast feasibility, schema merge — stay
in the nanosecond-to-microsecond range. The timezone engine resolves a DST offset
from an embedded POSIX rule with no I/O or tz-database lookup.

### From Python — temporal vs `datetime` / `zoneinfo`

Unlike the bulk HTTP/compression paths, these are *tiny per-call* operations, so the
FFI crossing dominates and the C-implemented stdlib `datetime` wins the raw timing.
The value yggdryl adds is **coverage and safety**, not microseconds:

| workload | yggdryl | datetime | vs datetime |
| --- | --- | --- | --- |
| parse ISO datetime | 6.7 µs | 0.14 µs | 0.02× |
| format datetime | 1.1 µs | 0.80 µs | 0.70× |
| convert UTC→New York (DST-aware) | 3.6 µs | 0.37 µs | 0.10× |

| capability | yggdryl | stdlib datetime |
| --- | --- | --- |
| parse a duration string (`1h30m`, `PT15M`) | ✓ | ✗ (no parser) |
| sub-microsecond (nanosecond) precision | ✓ | ✗ (µs only) |
| DST conversion with no OS tz database | ✓ (embedded) | ✗ (needs tzdata) |
| flexible parse (`2024/07/01` slash form) | ✓ | ✗ (dashes only) |
| reject an invalid calendar date | ✓ raises | ✓ raises |

### From Node — temporal vs `Date` / `Intl`

Node's `Date` is the starkest case for "more complete and safer": it has no duration
parser, only millisecond precision, no per-zone offset API, and silently rolls an
invalid date over. yggdryl's formatting is already on par; the gap is the capability
table.

| workload | yggdryl | Date/Intl | vs Date |
| --- | --- | --- | --- |
| parse ISO datetime | 0.98 µs | 0.24 µs | 0.24× |
| format datetime | 0.50 µs | 0.51 µs | 1.03× |
| convert UTC→New York (DST-aware) | 3.0 µs | 0.71 µs | 0.24× |

| capability | yggdryl | JS Date |
| --- | --- | --- |
| parse a duration string (`1h30m`, `PT15M`) | ✓ | ✗ (no parser) |
| sub-millisecond (nanosecond) precision | ✓ | ✗ (ms only) |
| DST offset for an arbitrary IANA zone | ✓ (`offsetSeconds`) | ~ (Intl format only) |
| reject an invalid date (`2023-02-29`) | ✓ throws | ✗ rolls to Mar 1 |
| flexible parse (`20240701`, `2024/07/01`) | ✓ | ~ (impl-defined) |

(Numbers from `python3 benchmarks/compare.py` and `node benchmarks/compare.mjs`;
hardware-dependent — the takeaway is the capability columns, not the microseconds.)

---

## Serie — the columnar layer

The Arrow-backed `Serie` (a named, typed column) sits on the schema layer. As with
schema, the **metadata pass is the point**: reading a column's shape and type is
branch-only, so a per-batch / per-column scan over many columns is essentially free.

### Rust core — `cargo bench -p yggdryl-serie --bench serie`

| workload | result |
| --- | --- |
| metadata / fast checks — `num_rows` / `null_count` / `category` / `data_type` | **1.6–1.9 ns** |
| typed value read — `Int32Serie::value` | **0.9 ns** |
| lazy `RangeSerie::value_at` (computed, no storage) | **1.5 ns** |
| type-erased `Serie::value_at` → `Scalar` (dynamic dispatch) | 57 ns |
| factory dispatch — `from_array` (4096-row int32 / utf8) | 127 / 145 ns |
| zero-copy `slice` (re-wrap a slice as a new column) | 226 ns |
| nested `child_by_name` / `select` (node path) | 34 / 96 ns |
| `resize` grow + fill (16 → 4096, nulls) | 1.1 µs |
| `cast` int32 → int64 / float64 (4096 rows) | 1.4 / 1.2 µs |
| dictionary decode — `CategoricalSerie::materialize` | 19 µs |
| dictionary encode — `CategoricalSerie::from_serie` (4096 rows, 8 distinct) | 59 µs |

Value access has **two tiers**: the typed `value` / `get` is sub-nanosecond — use it in
hot loops — while the type-erased `value_at → Scalar` pays a dynamic match + downcast
(~57 ns) for the convenience of not knowing the column's type ahead of time. A **lazy**
column computes a value without touching memory, so a `RangeSerie` read matches a typed
array read. `slice` is O(1) on the Arrow buffers; its cost is re-wrapping the slice as a
new boxed column, not copying data. `cast` and `resize` are bulk Arrow-kernel passes
(microseconds for 4 K rows). **Dictionary encoding** (`CategoricalSerie::from_serie`)
builds a hash of the distinct values — the one genuinely heavy op, worth it only when a
column actually repeats; decoding back to a flat column is ~3× cheaper.

---

## Reproduce

```bash
# Rust core (true ceiling, no FFI) — one bench file per theme
cargo bench -p yggdryl-core --bench io
cargo bench -p yggdryl-schema --bench schema --features arrow
cargo bench -p yggdryl-serie --bench serie
cargo bench -p yggdryl-core --bench compression --all-features
cargo bench -p yggdryl-http --all-features

# From Python — same code vs requests / httpx / gzip (+ peak-heap memory)
(cd bindings/python && maturin develop --release) && pip install requests httpx
python3 benchmarks/compare.py

# From Node — same code vs node:http / node:zlib
(cd bindings/node && npm run build)
node benchmarks/compare.mjs
```
