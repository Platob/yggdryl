# `io::local::Mmap` — benchmark & optimization history

Time **and** memory for the memory-mapped file source
([`Mmap`](../../../../crates/yggdryl-core/src/io/local/mmap.rs)) — the on-disk implementor of
the same `IOBase` contract as `Heap`, addressed by a `Uri`, with auto-resizing writes. The
harness is dependency-free with the same counting allocator as the heap bench.

## Run

```bash
cargo bench -p yggdryl-core --bench io_local_mmap
cargo test  -p yggdryl-core --test io_local_mmap     # functional suite (temp files)
```

## Rust core (release, counting global allocator, Windows NVMe)

| op | Mops/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `pread_i32` (mapped) | 18182 | **0.00** | 0.0 |
| `pwrite_i32` (mapped, in place) | 95 | **0.00** | 0.0 |
| `pread_into` 4 KiB (mapped) | 7.1 | 0.00 | 0.0 |
| overwrite 4 KiB (mapped) | 9.6 | 0.00 | 0.0 |
| `pwrite_i32_array` (1024, mapped, **direct**) | 18484 | 0.00 | 0.0 |
| `pread_i32_array` (1024, mapped, **direct**) | 19579 | 0.00 | 0.0 |
| `pwrite_byte_repeat` (8 KiB memset) | 78280 | 0.00 | 0.0 |
| append 64×1 KiB (fresh file, auto-resize) | 0.03 | 0.05 | 4.8 |
| open + close (4 KiB file) | syscall-bound | 2.00 | 182.0 |
| flush (4 KiB dirty page) | syscall-bound | 0.00 | 0.0 |

## What the numbers show

- **Mapped I/O allocates nothing.** Every read/write row is `0.00` allocs/op — the OS pages
  back the mapping; only open (2 small bookkeeping allocations) and growth remaps touch the
  allocator. The typed/utf8/cursor surface is inherited from `IOBase`'s defaults and runs at
  memory speed once pages are resident (mapped `pread_i32` ≈ 18 Gops/s).
- **Bulk typed arrays convert *directly* off the mapping.** `Mmap` overrides the bulk
  `pread/pwrite_{i32,i64}_array` and the `_repeat` fills with a dense, branch-free conversion
  straight over the contiguous mapped slice — no stack-staging buffer — exactly as `Heap`
  does over its `Vec`. That lifted bulk throughput from ~4–6 Gelem/s (the stack-staged trait
  default) to **~18–20 Gelem/s** (≈ 4× faster; `pwrite_byte_repeat` is a plain memset at
  ~78 Gelem/s), while staying at `0.00` allocs/op. The same speed is reached through
  `LocalIO`, which delegates its bulk methods to the mapped backing (see
  [`io/local/io.md`](io.md)).
- **Auto-resizing appends amortize.** Growing a fresh file 64 KiB in 1 KiB chunks costs
  ~0.05 allocations per chunk — the capacity doubles (page-aligned, `O(log n)` remaps),
  exactly `Heap`'s reallocation curve, and the on-disk file is truncated back to the logical
  length on drop, so no capacity padding persists.
- **File rows are syscall-bound, as they must be.** open/close and `flush` (msync + fsync)
  are dominated by the OS call, not by this crate — mapped reads/writes between them are
  where the model wins.
