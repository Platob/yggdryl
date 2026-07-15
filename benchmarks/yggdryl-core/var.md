# `io::var` — benchmark & optimization notes

Time **and** memory for the variable-length typed layer (`ByteScalar<E>` / `ByteSerie<E>`),
measured over `Utf8` — the same generic code backs `Binary`, so these numbers carry across both
kinds. The focus is the **zero-copy** `get_str` / `get_bytes` accessors and the serialization
path (offsets + data + validity). The harness is dependency-free and finishes in ~1 s;
allocation counts are build-independent, so the Rust harness and the deterministic
`io_var_alloc` test assert them as a regression guard.

## Run

```bash
cargo bench -p yggdryl-core --bench var         # Mops/s + allocs/op + bytes/op
cargo test  -p yggdryl-core --test io_var_alloc # deterministic memory budgets (ms)
```

## Rust core (release, counting global allocator, 1024 × utf8 short strings)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Utf8Serie::from_strs` (1024) | 0.05 | 13.00 | 36860.0 |
| `Utf8Serie::from_strs` (¼ null) | 0.05 | 17.00 | 20724.0 |
| `Utf8Serie::get_str` (one element) | 74.82 | **0.00** | 0.0 |
| `Utf8Serie::get_str` scan (1024) | 59.27 | **0.00** | 0.0 |
| `Utf8Serie::push_str` (1024, prealloc) | 47.68 | **0.01** | 36.0 |
| `Scalar` write+read round-trip | 1.23 | 11.00 | 434.0 |
| `Serie` write+read round-trip (1024) | 0.09 | 10.00 | 34991.0 |

(The round-trip rows count one *whole* 1024-element column as a single op; per element the
`Serie` round-trip is ≈ 92 M elem/s.)

## What the numbers show

- **Element reads are zero-copy.** `get_str` / `get_bytes` return a borrowed `&str` / `&[u8]`
  into the data buffer — **0 allocs** — so scanning all 1024 elements and summing their lengths
  allocates **nothing**. `io_var_alloc` asserts this for the serie accessors *and* for a
  scalar's `as_str` / `value_bytes`.
- **Append is amortized-free.** With a pre-sized column, `push_str` is `0.01 allocs/op` — the
  offsets and data `Vec`s double geometrically, so the per-element amortized cost rounds to
  zero.
- **`from_strs` builds two buffers.** The `13 allocs` are the geometric growth of the offsets
  `Vec` and the data `Vec` as 1024 values stream in (the column can't presize without knowing
  the total byte length up front); the `push_str` prealloc row is the fast path when the count
  is known.

## Optimization — pack the offsets, don't drip them

`ByteSerie::write_to` first wrote each of the `len + 1` offsets with its own `write_all`. The
sink ([`Bytes`](../../docs/io.md)) is copy-on-write, so **every** small write reallocated the
growing buffer — O(n) allocations for one column:

| `Serie` write+read round-trip (1024 utf8) | before | after |
|--|-------:|------:|
| allocs/op | 2073.00 | **10.00** (≈207×) |
| bytes/op | 161294 | **34991** (4.6×) |

The fix packs the header + validity + all offsets + the data length into **one pre-sized
buffer** and issues a single bulk `write_all`, then writes the (potentially large) data payload
directly — two writes total, no per-offset reallocation, and the big payload is never copied
into an intermediate. This mirrors the fixed `Serie`, which writes its values buffer in one
shot.
