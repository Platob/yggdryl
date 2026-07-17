# `io::local::LocalIO` — benchmark & optimization history

Time, memory, **and concurrency** for the local-filesystem access point
([`LocalIO`](../../../../crates/yggdryl-core/src/io/local/io.rs)) — the lazy handle that
self-optimizes onto a kept memory mapping on first write — plus the concurrency of the raw
[`Mmap`](../../../../crates/yggdryl-core/src/io/local/mmap.rs) it maps onto. Dependency-free
harness with the same counting allocator as the heap/mmap benches.

## Run

```bash
cargo bench -p yggdryl-core --bench io_local_io
cargo test  -p yggdryl-core --test io_local_io          # functional + concurrency suite
cargo test  -p yggdryl-core --test io_local_mmap_alloc  # deterministic zero-alloc budgets
```

## Rust core (release, counting global allocator, Windows NVMe, 8 threads)

| op | Mops/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| lazy first write (`mkdir -p` + create + map) | 0.005 | 13.0 | 1508 |
| `pread_i32` (mapped) | 137 | **0.00** | 0.0 |
| `pwrite_i32` (mapped, in place) | 65 | **0.00** | 0.0 |
| overwrite 4 KiB (mapped) | 14.5 | 0.00 | 0.0 |
| `pread` 4 KiB (ad-hoc, never written) | 0.01 | 6.0 | 4620 |
| `pread_into` 4 KiB (mapped, reused buf) | 8.8 | **0.00** | 0.0 |
| `Mmap` `pwrite_i32_array` (1024, direct) | 18484 | 0.00 | 0.0 |
| **`LocalIO` `pwrite_i32_array` (1024, mapped)** | **17686** | **0.00** | 0.0 |
| **`LocalIO` `pread_i32_array` (1024, mapped)** | **19579** | **0.00** | 0.0 |
| tree `byte_size` (16 blocks, lazy sum) | 0.004 | 87.0 | 7409 |
| tree `pread` whole (16×256, stitched) | 0.003 | 153.0 | 17415 |

### Concurrency — throughput (Mops/s), 1 / 2 / 4 / 8 threads

| op | ×1 | ×2 | ×4 | ×8 |
|---|--:|--:|--:|--:|
| shared-mapping reads (`Arc<Mmap>`, `&self`) | 144 | 254 | 422 | **777** |
| disjoint-file writes (own `LocalIO` each) | 33 | 58 | 102 | **192** |

## What the numbers show

- **Lazy auto-create is a one-time cost, then everything is mapped.** The first write to a
  fresh nested path pays `create_dir_all` + create + `mmap` (13 allocations, ~1.5 KB of
  bookkeeping) — after that the handle holds its mapping and every read/write is `0.00`
  allocs/op at memory speed. No `mkdir`/`touch` pre-flight; callers just write.
- **Self-optimization is the whole point.** A never-written handle reads *ad hoc* — it opens
  the file per call (2 syscalls + a fresh `Vec`), ~0.01 Mops/s. The same bytes through the
  mapped handle read ~800× faster with zero allocation. Writing once flips a handle from the
  first mode to the second.
- **Bulk SIMD reaches the mapping through `LocalIO`.** `LocalIO`'s bulk `pread/pwrite_*_array`
  and `_repeat` methods **delegate to the mapped backing** when it exists, so its bulk
  throughput (~17.7–19.6 Gelem/s) matches raw `Mmap` (~18.5–19.6) — a **≈ 4.3× lift** over the
  previous stack-staged path (was ~4.1 Gelem/s), at `0.00` allocs/op. Before the handle is
  mapped, the same methods fall back to the shared staged kernels over the ad-hoc / memory-tree
  byte methods, so the surface stays correct everywhere.
- **Concurrent reads from one mapping scale near-linearly** — 144 → 777 Mops/s across 1 → 8
  threads. A mapping is `Send + Sync` and `&self` reads never take a lock, so an `Arc<Mmap>`
  fans out to as many readers as there are cores with no contention.
- **Concurrent writes to disjoint files scale too** — 33 → 192 Mops/s across 1 → 8 threads.
  Each `LocalIO` owns its own mapping (an exclusive `&mut self` resource), so independent
  files never contend; the model is one writer per file, many readers per mapping.
- **Directory (memory-tree) reads are syscall-bound**, as they must be: `byte_size` and a
  full stitched `pread` walk the directory and `stat`/read each child block, so their cost is
  the filesystem's, not this crate's — they are a convenience over the graph, not a hot path.

## Optimization history

- **Bulk direct-conversion override (this round).** `Mmap` gained direct contiguous
  `pread/pwrite_{i32,i64}_array` + `_repeat` overrides (converting straight over the mapped
  slice, no staging buffer), and `LocalIO` now delegates its bulk methods to that mapped
  backing. The stack-staged kernels were factored into shared `pub(crate)` functions so the
  trait default, `LocalIO`'s pre-mapped fallback, and any future source stay DRY. Net: bulk
  arrays through the local access point went **~4.1 → ~17.7 Gelem/s**, proven zero-alloc by
  `tests/io_local_mmap_alloc.rs`.
