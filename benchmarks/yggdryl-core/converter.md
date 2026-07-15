# `io::converter` — benchmark & optimization notes

Time **and** memory for the type converter: the numeric `cast` on a scalar and a column, the
same-type no-copy fast path, and the UTF-8 / binary bridges. The point is to show a **same-type**
serie cast is allocation-free (it shares the `Arc`-backed buffer), a cross-width cast allocates once
for the new buffer, and the string / byte bridges cost only their (expected) format / parse.
Dependency-free harness (~1 s) with a counting global allocator; the deterministic `io_converter`
tests assert the correctness (range checks, non-finite handling, null passthrough) these numbers
ride on.

## Run

```bash
cargo bench -p yggdryl-core --bench converter
cargo test  -p yggdryl-core --test io_converter
```

## Rust core (release, counting global allocator)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Scalar<i32>::cast::<i64>` (value) | (inlined) | **0.00** | 0.0 |
| `Scalar<i32>::cast::<f64>` (value) | (inlined) | **0.00** | 0.0 |
| `Serie<i32>::cast::<i32>` (same type, no copy) | ~147 | **0.00** | 0.0 |
| `Serie<i32>::cast::<i64>` (1024, cross-width) | ~0.26 | 3.00 | ~24.6 KB |
| `Scalar<i32>::to_utf8` (format) | ~8 | 2.00 | 14.0 |
| `Utf8Scalar::parse_to::<i32>` | ~64 | **0.00** | 0.0 |
| `Scalar<i32>::to_binary` | ~22 | 1.00 | 4.0 |
| `BinaryScalar::read_to::<i32>` | (inlined) | **0.00** | 0.0 |

## What the numbers show

- **Same-type casts are free.** `Serie<i32>::cast::<i32>()` is `0 allocs / op` — the `TypeId` fast
  path clones the `Arc`-backed values buffer instead of re-materializing it, so casting a 1024-element
  column to its own type is a pointer bump, not a copy. This is the "if same, return self without
  copying" guarantee, measured.
- **A cross-type cast allocates exactly once for the result.** `cast::<i64>()` over 1024 elements is
  `3 allocs` (the value bytes `Vec` handed to `Buffer::from_byte_vec`, plus the column scaffolding),
  independent of element count — the conversion is a single pass, no per-element allocation.
- **Scalar casts are inlined and allocation-free.** A `Scalar` is `Copy`; the numeric coercion is
  branch-light integer / float arithmetic with a range check, `0 allocs / op`.
- **The bridges cost only their format / parse.** `to_utf8` is `2 allocs` (the `String` and the boxed
  UTF-8 slice), `to_binary` is `1 alloc` (the boxed byte slice); `parse_to` and `read_to` read back
  with `0 allocs`. The bridges are the universal fallback, not the hot path — the direct numeric
  `Converter` impls carry the volume for free.
