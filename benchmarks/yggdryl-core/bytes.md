# `io::Bytes` — benchmark & optimization notes

Time **and** memory for the byte-I/O type `Bytes` (the implementor of `IOBase` / `IOCursor`
/ `IOSlice`). The harness is dependency-free and finishes in ~1 s, so it doubles as fast
performance validation. Allocation *counts* are build-independent (same in debug and
release), which is why the Rust harness — and the deterministic `io_bytes_alloc` test —
assert them as a regression guard; wall-clock is release-only and reported, not asserted.

## Run

```bash
cargo bench -p yggdryl-core --bench bytes          # Mops/s + allocs/op + bytes/op
cargo test  -p yggdryl-core --test io_bytes_alloc  # deterministic memory budgets (ms)
```

## Rust core (release, counting global allocator, 4 KiB block)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `pread` (positioned, into buf) | 13.63 | **0.00** | 0.0 |
| `read` (cursor, into buf) | 11.75 | **0.00** | 0.0 |
| `slice` (zero-copy window) | 177.24 | **0.00** | 0.0 |
| `pwrite` (in-place overwrite) | 5.07 | 2.00 | 112.0 |
| `write` (grow from empty) | 2.59 | 4.00 | 4264.0 |
| `pwrite` (copy-on-write, shared) | 3.18 | 3.00 | 4208.0 |
| `read_to_end` (owning) | 5.66 | 1.00 | 4096.0 |
| `seek` + `read_exact` (session) | 170.82 | **0.00** | 0.0 |

## What the numbers show

The physical layer is an `Arc`-shared, immutable Arrow `Buffer`, and the table is the payoff:

- **Reads are zero-copy.** `pread` / `read` copy into the caller's buffer — **0 allocs, 0
  bytes** — and `seek` + `read_exact` over a cloned handle is **0 allocs** too (the clone
  shares the `Arc`; no payload is copied).
- **Slices are zero-copy.** `slice` is an atomic refcount bump — **0 allocs**, ~177 Mops/s,
  an order of magnitude faster than anything that touches the payload.
- **In-place writes reuse the payload.** `pwrite` over a uniquely-owned buffer shows **112
  bytes/op**, not 4096 — the 4 KiB payload allocation is reused; only the small `Arc` control
  block is re-created. Contrast the two write rows that *do* move the payload: `write (grow
  from empty)` and `pwrite (copy-on-write)` both spend ~4 KiB/op.
- **Copy-on-write copies once, only when shared.** `pwrite` to a slice that still shares its
  parent's allocation costs exactly **one allocation more** than the in-place case (the
  copied payload). `io_bytes_alloc` asserts this as `cow == inplace + 1`, so a regression that
  copied on every write — or aliased a shared slice — would fail the budget.

## Design — copy-on-write over an Arc-shared Arrow buffer

`Bytes` holds an immutable `arrow_buffer::Buffer`. A read borrows its slice; a `slice`
shares the `Arc` (`Buffer::slice_with_length`). A write must mutate, so `pwrite` reaches for
an owned `Vec` via `Buffer::into_vec::<u8>()`:

- **uniquely owned, offset 0** → `Ok(vec)`: the allocation is reused in place (grow only if
  the write runs past the current end).
- **shared, or an offset slice** → `Err(self)`: copy-on-write into a fresh `Vec`, leaving any
  other holder of the allocation untouched.

`into_vec` is used rather than `into_mutable` deliberately: `into_mutable` **panics** (a debug
`assert_eq!` on the pointer) when handed a uniquely-owned *slice* with a non-zero offset,
whereas `into_vec` returns `Err` in exactly that case — which is precisely the copy path we
want. The result is: reads and slices never allocate, in-place writes reuse the payload, and
the single payload copy happens only when the allocation is genuinely shared.
