# `memory::Heap` — benchmark & optimization history

Time **and** memory for the in-heap [`Heap`](../../crates/yggdryl-core/src/memory/heap.rs)
source and the byte I/O trait surface (the byte-array primitives, the typed `byte` / `bit` /
`i32` / `i64` accessors, the `pread_into` transfer vs the owning `pread_vec`, cursor streaming,
and slicing). The harness is dependency-free and finishes in **well under a second**, so it
doubles as fast performance validation. Allocation *counts* are build-independent (same in debug
and release), which is why the Rust harness — and the deterministic `memory_heap_alloc` test —
assert them as a regression guard; wall-clock is release-only and reported, not asserted.

## Run

```bash
cargo bench -p yggdryl-core --bench heap                # Rust: Mops/s + allocs/op + bytes/op
cargo test  -p yggdryl-core --test memory_heap_alloc    # deterministic memory budgets (ms)
```

## Rust core (release, counting global allocator)

| op | Mops/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `pread_byte` | 14832 | 0.00 | 0.0 |
| `pread_i32` | 200000 | 0.00 | 0.0 |
| `pread_i64` | 200000 | 0.00 | 0.0 |
| `pread_bit` | 20000 | 0.00 | 0.0 |
| `pread_into` (4 KiB, reused buffer) | 10.7 | **0.00** | **0.0** |
| `pread_vec` (4 KiB, fresh `Vec`) | 8.8 | 1.00 | 4096.0 |
| cursor `write byte+i32+i64` | 351 | 0.00 | 0.0 |
| `slice` (1 KiB window) | 18.9 | 1.00 | 1024.0 |
| `from_slice` (4 KiB ingest) | 12.3 | 1.00 | 4096.0 |

## What the numbers show

- **Typed accessors are zero-copy.** `pread_byte` / `pread_bit` / `pread_i32` / `pread_i64` read
  through a small stack array and allocate **nothing** — the deterministic `memory_heap_alloc`
  test pins each at `0` allocations.
- **`pread_into` reuses the caller's buffer.** Across an entire transfer loop it makes **0
  allocations** and moves **0 heap bytes**, versus `pread_vec`'s one 4 KiB allocation **per
  call** — and it is also faster on the 4 KiB page (10.7 vs 8.8 Mops/s). This is the
  at-most-one-copy / buffer-reuse rule made measurable: prefer `pread_into` in a hot transfer
  loop and `pread_vec` only when a fresh owned `Vec` is genuinely wanted.
- **The owned-copy operations cost exactly one allocation.** `slice` and `from_slice` own their
  bytes, so they show a single allocation sized to the payload — nothing throwaway.
- **`with_capacity` amortizes growth.** Filling a `Heap::with_capacity(N)` to `N` bytes stays at
  one allocation (the reservation) regardless of how many writes it takes — asserted in the alloc
  test.
