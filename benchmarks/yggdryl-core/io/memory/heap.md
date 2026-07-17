# `io::memory::Heap` — benchmark & optimization history

Time **and** memory for the in-heap [`Heap`](../../../../crates/yggdryl-core/src/io/memory/heap.rs)
source and the byte I/O trait surface (the byte-array primitives, the typed `byte` / `bit` /
`i32` / `i64` accessors, the bulk vectorized arrays and repeated-value fills, UTF-8 text, the
`pread_into` transfer vs the owning `pread_vec`, cursor streaming, and slicing). The harness is
dependency-free and finishes in **well under a second**, so it doubles as fast performance
validation. Allocation *counts* are build-independent (same in debug and release), which is why
the Rust harness — and the deterministic `io_memory_heap_alloc` test — assert them as a
regression guard; wall-clock is release-only and reported, not asserted.

## Run

```bash
cargo bench -p yggdryl-core --bench io_memory_heap        # Rust: Mops/s + allocs/op + bytes/op
cargo test  -p yggdryl-core --test io_memory_heap_alloc   # deterministic memory budgets (ms)
```

## Rust core (release, counting global allocator)

After the `Heap` fast-path round (defaults measured through a minimal primitives-only source
in the same run — the baseline any new source starts from):

| op | Mops/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `pread_byte` / `pread_bit` | 4000–200000 | 0.00 | 0.0 |
| `pread_i32` / `pread_i64` (direct slice) | ~200000 | 0.00 | 0.0 |
| `pwrite_i32` / `pwrite_i64` (in place, inlined) | **~50000** | 0.00 | 0.0 |
| cursor `write byte+i32+i64` | 145 | 0.00 | 0.0 |
| `pread_into` (4 KiB, reused buffer) | 5.7 | **0.00** | **0.0** |
| `pread_vec` (4 KiB, fresh `Vec`) | 4.6 | 1.00 | 4096.0 |
| **append** 4 KiB (reserved heap, single-write grow) | 7.4 | 1.00 | 4096.0 |
| overwrite 4 KiB (in place) | 17.2 | 0.00 | 0.0 |
| `pwrite_i32_array` (1024) — **Heap override** | **16762** | 0.00 | 0.0 |
| `pwrite_i32_array` (1024) — trait default (min src) | 7126 | 0.00 | 0.0 |
| `pread_i32_array` (1024) — **Heap override** | **17325** | 0.00 | 0.0 |
| `pread_i32_array` (1024) — trait default (min src) | 5915 | 0.00 | 0.0 |
| `pwrite_i64_array` / `pread_i64_array` (1024) | 6900 / 4900 | 0.00 | 0.0 |
| `pwrite_i32_repeat` (1024, doubling fill) | 8526 | 0.00 | 0.0 |
| repeat via full `Vec` (compare) | 4840 | — | 4.0 |
| `pwrite_byte_repeat` (8 KiB, memset) | **63825** | 0.00 | 0.0 |
| `slice` (1 KiB window) | 8.7 | 1.00 | 1024.0 |
| `from_slice` (4 KiB ingest) | 7.3 | 1.00 | 4096.0 |
| `pread_utf8` (short text) | 8.4 | 1.00 | 23.0 |

## What the numbers show

- **Heap overrides beat the trait defaults 2–3× on bulk ops** (measured in the same run
  through a minimal primitives-only source running the defaults): the defaults stage every
  typed/bulk op through a stack chunk + the byte primitives (two copies); `Heap` owns
  contiguous bytes, so its overrides convert **directly off the stored bytes in one pass** —
  `pwrite_i32_array` 16.8 vs 7.1 Gelem/s, `pread_i32_array` 17.3 vs 5.9 Gelem/s. Every
  override keeps the default's exact semantics, including identical error values.
- **The single-write hot path is fully inlined.** `pwrite_i32`/`pwrite_i64` restructured to
  "in-place check first, growth out of line" — ~50 Gops/s (a ~40× jump over routing through
  the growth-capable path; the same restructure recovered the cursor stream too).
- **Appends never write the grown region twice.** The old grow path `resize`-zeroed the region
  and then overwrote it; the append fast-path zero-fills only a *gap* and extends with the
  data in one pass — append throughput up ~1.6× (7.4 vs the pre-fix 4.7 Mpages/s equivalent).
- **Typed accessors are zero-copy.** Reads go straight through `data.get(..)` +
  `from_le_bytes`; the deterministic `io_memory_heap_alloc` test pins each at `0` allocations.
- **Fills are memset-class.** `pwrite_byte_repeat` is a plain `fill` (64 Gops/s ≈ memory
  bandwidth); the typed repeats use a **doubling `copy_within`** fill (log₂(n) bulk copies),
  and no repeat ever materializes the full array.
- **`pread_into` reuses the caller's buffer.** Zero allocations across a transfer loop versus
  `pread_vec`'s per-call allocation — prefer it in hot loops.
- **The owned-copy operations cost exactly one allocation.** `slice`, `from_slice`, and
  `pread_utf8` own their result — a single allocation sized to the payload.
- **The heap itself is lightweight.** No stored address (the lazy static `mem://heap` clone
  costs exactly 2 small string allocations — asserted) and a directly-embedded empty headers
  map (allocation-free until used — asserted).
- **Growth auto-scales and is fully checkable.** Un-reserved chunked appends amortize (64 x
  1 KiB appends = ~0.11 reallocations per chunk, O(log n) — asserted `<= 8` total in the alloc
  test), and the checked `try_reserve` / `try_reserve_exact` / `try_ensure_capacity` twins turn
  an overflowing or refused reservation into a guided error instead of a process abort — a
  failed try_reserve allocates nothing (asserted).
- **`with_capacity` amortizes growth.** Filling a reserved heap stays at one allocation (the
  reservation) — asserted, and available on any source via the trait-level
  `IOBase::with_capacity`.
