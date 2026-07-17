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

| op | Mops/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `pread_byte` | 18312 | 0.00 | 0.0 |
| `pread_i32` | ~200000 | 0.00 | 0.0 |
| `pread_i64` | ~200000 | 0.00 | 0.0 |
| `pread_bit` | 25000 | 0.00 | 0.0 |
| `pread_into` (4 KiB, reused buffer) | 7.3 | **0.00** | **0.0** |
| `pread_vec` (4 KiB, fresh `Vec`) | 4.3 | 1.00 | 4096.0 |
| cursor `write byte+i32+i64` | 154 | 0.00 | 0.0 |
| `slice` (1 KiB window) | 10.4 | 1.00 | 1024.0 |
| `from_slice` (4 KiB ingest) | 6.6 | 1.00 | 4096.0 |
| `pwrite_i32_array` (1024 elems) | 5180 | **0.00** | 0.0 |
| `pread_i32_array` (1024 elems) | 6060 | **0.00** | 0.0 |
| `pwrite_i32_repeat` (1024 elems) | **15370** | **0.00** | 0.0 |
| repeat via full `Vec` (compare) | 4407 | — | 4.0 |
| `pread_utf8` (short text) | 12.0 | 1.00 | 23.0 |

## What the numbers show

- **Typed accessors are zero-copy.** `pread_byte` / `pread_bit` / `pread_i32` / `pread_i64` read
  through a small stack array and allocate **nothing** — the deterministic `io_memory_heap_alloc`
  test pins each at `0` allocations.
- **Bulk arrays are vectorized and allocation-free.** `pread_i32_array` / `pwrite_i32_array`
  stage through a fixed 256-element stack chunk and convert in a dense, branch-free loop LLVM
  auto-vectorizes — 5–6 **G**elem/s with **0** heap allocations, asserted across multi-chunk
  transfers (1000 elements) in the alloc test.
- **Repeated-value fills never build the array.** `pwrite_i32_repeat` fills one stack chunk once
  and writes it repeatedly: **~3.5×** the throughput of materializing the full `Vec` first
  (15370 vs 4407 Mops/s), with 0 allocations at any count.
- **`pread_into` reuses the caller's buffer.** Across an entire transfer loop it makes **0
  allocations** and moves **0 heap bytes**, versus `pread_vec`'s one 4 KiB allocation **per
  call** — and it is also faster (7.3 vs 4.3 Mops/s on a 4 KiB page). Prefer `pread_into` in a
  hot transfer loop; `pread_vec` only when a fresh owned `Vec` is genuinely wanted.
- **UTF-8 reads own exactly their `String`.** `pread_utf8` costs the one returned allocation;
  `pwrite_utf8` into a sized sink allocates nothing (both asserted).
- **The owned-copy operations cost exactly one allocation.** `slice` and `from_slice` own their
  bytes, so they show a single allocation sized to the payload — nothing throwaway.
- **The heap itself is lightweight.** It stores no address (every heap reports the lazy-built,
  once-parsed synthetic `mem://heap`; an accessor call costs exactly the 2 small string clones
  of the cached value — asserted) and its metadata is lazy (`None` until the first
  `headers_mut()`; reading untouched headers borrows a shared static and allocates **nothing**
  — asserted).
- **`with_capacity` amortizes growth.** Filling a `Heap::with_capacity(N)` to `N` bytes stays at
  one allocation (the reservation) regardless of how many writes it takes — asserted in the
  alloc test, and available on **any** source via the trait-level `IOBase::with_capacity`.
