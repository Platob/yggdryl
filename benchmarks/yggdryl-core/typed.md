# `typed` — the serialization layer adds no overhead

Time **and** memory for the [`typed`](../../crates/yggdryl-core/src/typed) layer's hot paths — the
`Encoder`/`Decoder` bulk round-trip and the `Reduce` aggregations over a
[`FixedSerie`](../../crates/yggdryl-core/src/typed/serie.rs). The point: a **typed column is a
zero-overhead view** on the byte layer — build/decode/reduce forward straight to the same vectorized
`IOBase` bulk + `Aggregate` kernels, so the typed API costs only what the *result* owns.

## Run

```bash
cargo bench -p yggdryl-core --bench typed
cargo test  -p yggdryl-core --test typed
```

## Release, counting global allocator, 65 536 elements, 2000 iters

| op | Melem/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `FixedSerie::from_values` (build `i64`) | **1286** | **1** | 524 288 |
| `Serie::values` (decode `i64`) | 1105 | 1 | 524 288 |
| `Serie::sum` (reduce `i64`) | 937 | **0** | 0 |
| `Serie::min` (reduce `i64`) | 1051 | **0** | 0 |
| `Serie::mean` (reduce `f64`) | 487 | **0** | 0 |
| `Serie::get` (scalar decode `i64`) | 270 | **0** | 0 |

## Found optimization — untag the raw data buffer (4 → 1 alloc/build, 3.9×)

The first cut set the element `DataTypeId` on the column's raw data-buffer `Headers` in every
constructor — three allocations (a `Headers` entry: the name box, the value box, the vec push) on a
path that should own only its data buffer. But the type identity already lives at the **compile-time
`T`** and the **`field()`** metadata, so tagging the raw bytes is redundant. Removing it dropped a
build from **4 → 1 allocation** (just the data `Heap`) and lifted throughput **330 → 1286 Melem/s
(~3.9×)** — the typed build is now indistinguishable from a bare `pwrite_i64_array`.

## What the numbers show

- **Build and decode own exactly one buffer.** `from_values` makes the single data-`Heap`
  allocation the column holds; `values()` makes the single `Vec` the caller receives (524 288 B =
  65 536 × 8, no slack). Both run at the byte layer's bulk-write / bulk-read speed.
- **Reductions allocate nothing.** `sum` / `min` / `mean` forward to the source's `Aggregate`
  kernels — the same allocation-free, LLVM-vectorized, NaN-safe loops a bare `Heap` runs — so a typed
  reduction has **zero** per-op allocation and ~1 Gelem/s throughput. `mean` over `f64` is ~2× slower
  per element than `i64` (8-byte elements, half the elements per cache line).
- **Scalar random access is allocation-free.** `get(i)` decodes one element (a one-element bulk
  read) with no heap traffic — the null-aware indexed path costs a bounds + validity check over the
  raw decode.
- **The seams that remain.** `from_options` / `push_null` build the validity bitmap element-by-element
  (a bit write each); the vectorized bit-pack and the `mask_filter`-based `filter` are marked in the
  source — they trade the current simplicity for SIMD once the null-aware bitmap compaction lands.
