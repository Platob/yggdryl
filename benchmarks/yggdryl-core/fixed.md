# `io::fixed` — benchmark & optimization notes

Time **and** memory for the fixed-width typed layer (`Buffer<T>` / `Scalar<T>` / `Serie<T>`),
measured over `i32` — a multi-byte type, so element vs byte lengths and the little-endian
codec are exercised. The same generic code backs the whole numeric family (`u8`…`u64`,
`i8`…`i64`, `f32`/`f64`), so these numbers carry across widths (payload sizes scale with
`T::WIDTH`). The harness is dependency-free and finishes in ~1 s; allocation counts are
build-independent, so the Rust harness and the deterministic `io_fixed_alloc` test assert them
as a regression guard.

## Run

```bash
cargo bench -p yggdryl-core --bench fixed          # Mops/s + allocs/op + bytes/op
cargo test  -p yggdryl-core --test io_fixed_alloc  # deterministic memory budgets (ms)
```

## Rust core (release, counting global allocator, 1024 × i32)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Buffer::from_vec` (1024 i32) | 5.20 | 2.00 | 4152.0 |
| `Buffer::get` (one element) | ~200000 | **0.00** | 0.0 |
| `Buffer::as_slice` sum (1024) | (inlined) | **0.00** | 0.0 |
| `Buffer::push` (1024, prealloc) | 7.94 | 2.00 | 116.1 |
| `Serie::from_values` (1024) | 5.60 | 2.00 | 4152.0 |
| `Serie::from_options` (¼ null) | 0.19 | **7.00** | 4400.0 |
| `Scalar` write+read round-trip | 8.29 | 2.00 | 112.0 |
| `Serie` write+read round-trip (1024) | 0.69 | 13.00 | 12769.0 |

## What the numbers show

The typed layer sits on the same `Arc`-shared, immutable Arrow buffer as `Bytes`, so it
inherits the same discipline:

- **Element reads are zero-copy.** `Buffer::get` decodes one value from the borrowed bytes —
  **0 allocs** — and `as_slice()` hands back a typed `&[T]` view of the shared allocation, so
  summing 1024 elements allocates **nothing**. `io_fixed_alloc` asserts both, plus that
  decoding a `Scalar` / `Serie` element from a stream is heap-free.
- **`from_vec` moves the payload.** The `2.00 allocs / 4152 bytes` for `from_vec` is the input
  `Vec` (the benchmark clones one per iteration) plus a ~56-byte `Arc` control block —
  `from_vec` itself copies **no** payload, it takes ownership of the `Vec`'s allocation. The
  `4152` is the clone's 4 KiB, not a re-copy.
- **The serialized `Scalar` is allocation-light.** A full write+read round-trip through a
  reused sink is `2.00 allocs / 112 bytes` — the value (de)serializes through a **stack**
  scratch frame; the only heap is the sink's `Arc` re-seal on write. `Scalar::read_from` on
  its own is **0 allocs**.

## Optimization — bulk build vs `push` loop

`Buffer` / `Serie` store an immutable Arrow buffer so slices can share it zero-copy; the price
is that a **`push` re-seals** that buffer (an `Arc` swap) every call. Building a whole column
by pushing element-by-element therefore costs O(n) allocations. `from_options` originally did
exactly that:

| `Serie::from_options` (¼ null, 1024) | before | after |
|--|-------:|------:|
| allocs/op | 2064.00 | **7.00** (≈295×) |
| bytes/op | 123176 | **4400** (28×) |

The fix builds the values `Vec` and the validity bitmap in **one pass**, then wraps the values
once (`from_options` / the `FromIterator` impl both take this path). `push` remains for
incremental writes, with a doc note pointing at the bulk constructors for hot loops; the
`from_values` row (no nulls, single allocation of the values buffer) is the fast path.
