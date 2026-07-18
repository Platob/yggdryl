# `io::amd` — device-memory compute: CPU vs GPU dispatch

Time **and** memory for the AMD device-memory family (feature `amd`) — the vectorized statistical
reductions and the threshold filter from [`Aggregate`](../../../crates/yggdryl-core/src/io/memory/aggregate.rs),
plus the host↔device transfer, over an [`AmdHeap`](../../../crates/yggdryl-core/src/io/amd/heap.rs).

The compute is **auto-dispatched**: [`compute_backend(n)`](../../../crates/yggdryl-core/src/io/amd/mod.rs)
picks the **GPU** when the buffer is on a real Radeon adapter *and* the workload crosses
`GPU_ELEMENT_THRESHOLD` (65 536 elements), else the **CPU** — the dense, LLVM-vectorized reduction
streamed through a fixed stack chunk. Today both arms run the CPU kernel (the `AmdHeap` stages
through host memory; the device queue + kernels are the marked seam), so the numbers below are the
**CPU compute path** — the honest baseline the GPU kernel must beat before the dispatch flips it on.
The **allocs/op = 0** column proves the reductions run with zero heap allocation in the hot loop.

## Run

```bash
cargo bench -p yggdryl-core --features amd --bench io_amd
cargo test  -p yggdryl-core --features amd --test io_amd
```

## CPU compute path (release, counting global allocator, 65 536 elements, 2000 iters)

| op | Melem/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `sum_i32` (reduce) | 2317 | **0** | 0 |
| `min_i32` (reduce) | 3045 | **0** | 0 |
| `max_i32` (reduce) | 3113 | **0** | 0 |
| `count_ge_i32` (filter) | 1916 | **0** | 0 |
| `sum_f64` (reduce) | 607 | **0** | 0 |
| `mean_f64` (reduce) | 634 | **0** | 0 |
| `upload` (host → device) | 33679 | **0** | 0 |
| `download_vec` (device → host) | 1326 | 1 | 1.0 |

## The CPU-vs-GPU dispatch

`compute_backend(elements)` is the seam every device-aware op consults. Its decision on **this**
machine (no real Radeon kernel wired yet — a present adapter still routes CPU until the queue lands):

| workload | `compute_backend` | why |
|---|---|---|
| `8` elements | **Cpu** | below threshold — a host↔device transfer would not amortize |
| `GPU_ELEMENT_THRESHOLD * 4` (262 144) | **Gpu** *iff* `device().is_present()`, else **Cpu** | large enough to amortize the transfer, **and** a real adapter is present |

## What the numbers show

- **The reductions are allocation-free and vectorized.** Every `sum`/`min`/`max`/`count_ge` streams
  the typed data through a stack chunk (no per-call heap buffer) and runs a dense, branch-free loop
  the compiler auto-vectorizes on stable Rust — `i32` reductions clear **2–3 Gelem/s**, and the
  hot loop makes **zero** allocations. Floats are ~4× slower per element (8-byte `f64` vs 4-byte
  `i32`, half the elements per cache line), still allocation-free.
- **`upload` is a memcpy, `download_vec` owns one `Vec`.** Upload replaces the device content in
  place (no allocation); `download_vec` makes the single pre-sized host allocation the copy needs
  (allocs/op = 1) — the honest cost of handing bytes back to the host.
- **The dispatch is the optimization seam.** `compute_backend` already routes GPU-vs-CPU by workload
  size and device presence; wiring a device reduction/filter/DMA kernel behind the GPU arm
  accelerates every `Aggregate` op transparently, with no change to calling code. Until then the CPU
  path above is the baseline — and it is the same shared `Aggregate` kernel a plain `Heap` runs, so
  a device buffer never regresses the portable path.
