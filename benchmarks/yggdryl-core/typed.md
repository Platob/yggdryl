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

## Encodings — build strategy, width, and the kernel vs. allocation

`from_values` builds by element width, plus the streaming / nullable / bit / decimal paths:

| encode | Melem/s | allocs/op | note |
|---|--:|--:|---|
| `from_values` i8 (1 B) | ~14 300 | 1 | memcpy-bound |
| `from_values` i32 (4 B) | ~2 990 | 1 | memcpy-bound |
| `from_values` i64 (8 B) | ~1 320 | 1 | memcpy-bound |
| `from_values` i128 / u128 (16 B) | ~80–120 | 1 | **allocation-bound** (see below) |
| `from_values` `Bit` (bool) | **~630–930** | 2 | bit-packed (was ~167) |
| `from_values` `Decimal128` | ~65–90 | 1 | i128 array, allocation-bound |
| `from_values` `Decimal256` (`I256`) | ~27 | 1 | 32 B/element |
| `push` loop i64 (streaming) | ~45 | 1 | per-element call overhead |
| **`extend`** i64 (batch) | **~560** | 1 | one bulk write (was `push` ~45) |
| `from_options` i64 (nullable) | **~74** | **4** | bulk data + packed validity (was ~27, 12 allocs) |
| `FixedScalar::of` i64 (single) | ~8 | 1/scalar | a standalone scalar owns a `Heap` |
| **encode kernel** i64 → reused buffer | **~2 500** | **0** | isolates the kernel from the alloc |
| **encode kernel** i128 → reused buffer | **~1 040** | **0** | ~16.6 GB/s — memcpy speed |

### Found optimizations

- **Untag the raw data buffer (4 → 1 alloc/build, 3.9×).** The first cut set the element `DataTypeId`
  on the column's raw data-buffer `Headers` in every constructor — three allocations for identity that
  already lives at the compile-time `T` + the `field()` metadata. Removing it dropped a build **4 → 1
  allocation** and **330 → 1286 Melem/s**.
- **i128 / u128 whole-slice memcpy (kernel ~12×).** `Heap`'s bulk write did a per-element
  `copy_from_slice(&value.to_le_bytes())` — which LLVM vectorizes for the ≤8-byte widths but *not* for
  the 16-byte ones (≈1.4 GB/s). On little-endian the element bytes **are** the wire bytes, so the whole
  slice is one `memcpy` behind `#[cfg(target_endian = "little")]`: the isolated i128 kernel rises to
  **~1 040 Melem/s (~16.6 GB/s)**. It also lifts `Decimal128` and every wide-int decode.
- **Bit-packed `Bit` encode (~3.8×).** `from_values::<Bit>` packed bit-by-bit; the byte-aligned fast
  path now packs 8 bits/byte and does **one** `pwrite_byte_array` (per-bit tail preserved) — **167 →
  ~630–930 Melem/s**.
- **Bulk `from_options` (~2.8×, 12 → 4 allocs).** Was a per-element `push` that reallocated the growing
  validity buffer 12×; now one vectorized data write (nulls → default) + one pre-sized, packed
  validity write.
- **`extend` for batch append (~11× over `push`).** A new bulk counterpart of `push` — one
  `encode_slice` for a whole slice instead of a call per element.

**The large-buffer builds are allocation-bound, not kernel-bound.** A fresh `i128`/decimal column
allocates a 1–2 MB data buffer, which the allocator serves via `mmap`; the encode then first-touches
those pages, so the `from_values` rows for the wide types measure `mmap` + page-fault cost, not the
encode. The **reused-buffer** rows above prove it: the same i64/i128 `encode_slice` into a pre-grown
`Heap` runs at **2 500 / 1 040 Melem/s with zero allocations** — memcpy speed. Real code that builds a
column once and reads it many times pays the allocation once.

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
- **The seams that remain.** `from_options` now builds the validity bitmap packed in one write, but
  `push_null` (incremental) still sets one bit at a time; the `mask_filter`-based `filter` compaction
  is still element-by-element — marked in the source for the null-aware SIMD path.
