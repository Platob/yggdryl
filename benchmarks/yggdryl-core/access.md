# Column access — benchmark & optimization notes

Time **and** memory for the column **access** surface shared by every `Serie` family: element /
scalar `get`, single `set`, and the bulk `set_range` / `set_scalars` / `set_values`. The point is
to show `get` stays zero-copy, a single `set` is O(1), and — the optimization this report drove —
a **bulk** set materializes the values in **one** copy-on-write of the values buffer rather than
re-sealing the Arc-backed buffer once per element. The harness is dependency-free (~1 s) with a
counting global allocator; the deterministic `io_access` test asserts the correctness these numbers
ride on.

## Run

```bash
cargo bench -p yggdryl-core --bench access
cargo test  -p yggdryl-core --features arrow --test io_access
```

## Rust core (release, counting global allocator, 256 elements)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Serie::<i32>` `get_scalar` (one) | (inlined) | **0.00** | 0.0 |
| `Serie::<i32>` `set` (one) | ~9 | 2.00 | 112.0 |
| `Serie::<i32>` `set_range` (256, from Serie) | ~1.0 | **3.00** | 1136.0 |
| `Serie::<i32>` `set_values` (256, native) | ~2.0 | **3.00** | 1136.0 |
| `FixedBinarySerie` `set` (one, N=16) | ~2400 | **0.00** | 0.0 |
| `Utf8Serie` `set_str` same-length (one) | ~62 | **0.00** | 0.0 |
| `Utf8Serie` `set_str` grow (one, offset rewrite) | ~10 | **0.00** | 0.0 |
| `D128Serie` `set` (one) | ~6 | 2.00 | 112.0 |
| `D128Serie` `set_range` (256, from Serie) | ~0.04 | **3.00** | 8304.0 |

## What the numbers show

- **`get` / `get_scalar` are zero-copy.** A read decodes the value (or borrows the slot for the
  byte families) — `0 allocs / op`.
- **The bulk set is the fast path — one COW, not N.** The Arc-backed families (`Serie<T>`,
  `DecimalSerie<B>`) are *immutable* under the hood, so every write re-seals the buffer. The naive
  bulk (one `set` per element) therefore cost **512 allocs** for 256 elements (`2 × N`); building
  the contiguous byte patch and committing it with a **single** `pwrite` / `into_vec` COW drops
  that to a constant **3 allocs** (the patch `Vec` + the one re-seal) — a **33–67× throughput** win
  on `set_range` / `set_values`. This is why the bulk methods exist: prefer them to a `set` loop.
- **A single `set` is O(1) but re-seals (2 allocs).** That is inherent to the immutable Arc buffer
  — there is no in-place mutation without taking ownership and re-wrapping. For a hot update loop,
  reach for the bulk methods.
- **The `Vec`-backed families set in place (0 allocs).** `FixedBinarySerie` overwrites its flat
  `N`-byte slot, and `Utf8Serie` overwrites its data buffer in place when the length is unchanged —
  both `0 allocs / op`.
- **A variable-length `set` that changes length is deliberately expensive but allocation-free.**
  `Utf8Serie::set_str` splices the data buffer and shifts every trailing offset (an O(n) rewrite) —
  ~6× slower than a same-length overwrite, still `0 allocs / op`. Correctness is proven by a
  serialize/deserialize round-trip (which validates the rewritten offsets) after a mix of
  grow / shrink / null sets. For replacing most of a variable-length column, build a fresh one.
