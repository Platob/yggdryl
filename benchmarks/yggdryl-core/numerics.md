# `io::fixed` numerics — benchmark & optimization notes

Time **and** memory for the broadened numeric surface added on top of the base primitives: the
wide non-Arrow-native `[u8; N]` newtypes (measured over `i256`, 32 bytes) and the runtime-`N`
fixed-size byte family (measured over `FixedBinary`, `N = 16`). The point is to show the new
types keep the same allocation discipline as the base `Buffer`/`Serie` — zero-copy reads, a
single materialization on bulk build. The harness is dependency-free (~1 s) with a counting
global allocator; the deterministic `io_numerics` / `io_fixed_size` tests assert the
correctness these numbers ride on.

## Run

```bash
cargo bench -p yggdryl-core --bench numerics
cargo test  -p yggdryl-core --test io_numerics --test io_fixed_size
```

## Rust core (release, counting global allocator, 1024 elements)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `I256Serie::from_options` (1024) | 0.37 | 2.00 | 32824.0 |
| `Buffer::<I256>::as_slice` scan (1024) | (inlined) | **0.00** | 0.0 |
| `I256Serie::get` (one element) | ~4900 | **0.00** | 0.0 |
| `I256Serie` write+read round-trip (1024) | 0.03 | 13.00 | 98785.0 |
| `I256Scalar` write+read round-trip | 3.85 | 4.00 | 201.0 |
| `FixedBinarySerie::push` (1024, N=16) | 258 | **0.01** | 32.0 |
| `FixedBinarySerie::get_bytes` (one) | ~200000 | **0.00** | 0.0 |

## What the numbers show

- **The wide `[u8; N]` newtype behaves like a primitive.** `I256Serie::from_options` is
  `2.00 allocs / op` — one pass materializes the value bytes into a `Vec<u8>`, then
  `Buffer::from_byte_vec` moves that allocation into the Arrow buffer with **no copy** (plus the
  small `Arc` box), exactly the discipline the `i32` column has. `as_slice` reinterprets the
  shared bytes as `&[I256]` and allocates **nothing** — and because the newtype has alignment 1,
  that reinterpret is a *total* function (its element-alignment assert can never fire, unlike the
  align-8/16 native path which can panic on an externally-misaligned Arrow buffer).
- **Fixed-size reads are zero-copy.** `FixedBinarySerie::get_bytes` returns a borrowed `&[u8]`
  slice of the flat data buffer — `0 allocs / op` — and `push` is `0.01 allocs / op` amortized
  (the data `Vec` doubles geometrically).
- **Round-trips are allocation-light and exact.** The `Scalar` round-trip is `4 allocs` (the
  boxed value in + the sink's re-seal); the whole 1024-element `Serie` round-trip is `13 allocs`,
  dominated by the read-side value + validity buffers. Both are exact inverses across nulls,
  empties, and the full 32-byte width (the shared stack scratch is sized to `MAX_WIDTH = 32`).
