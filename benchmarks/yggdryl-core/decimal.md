# `io::fixed` decimal — benchmark & optimization notes

Time **and** memory for the scaled-decimal family (`d32`/`d64`/`d128`/`d256`): the self-describing
value type's checked arithmetic and identity, and the columnar `DecimalSerie` build / read /
round-trip. The point is to show the decimals keep the same discipline as the rest of `io::fixed`
— **stack-only value arithmetic** (no per-op allocation even at 256 bits), a **single**
materialization on bulk column build, and **zero-copy** element reads and Arrow export. The
harness is dependency-free (~1 s) with a counting global allocator; the deterministic
`io_decimal` / `io_decimal_alloc` tests assert the correctness these numbers ride on.

## Run

```bash
cargo bench -p yggdryl-core --bench decimal --features arrow
cargo test  -p yggdryl-core --features arrow --test io_decimal --test io_decimal_alloc
```

## Rust core (release, counting global allocator, 1024 elements)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `D128 checked_add` (aligned scales) | ~114 | **0.00** | 0.0 |
| `D128 checked_mul` | (inlined) | **0.00** | 0.0 |
| `D256 checked_add` | ~13 | **0.00** | 0.0 |
| `D128 cmp` (cross-scale) | (inlined) | **0.00** | 0.0 |
| `D128Serie::from_options` (1024) | 0.01 | 2.00 | 16440.0 |
| `D128Serie::get` (one element) | ~22000 | **0.00** | 0.0 |
| `D128Serie` write+read round-trip (1024) | 0.42 | 12.00 | 33251.0 |
| `D128Serie::to_arrow_array` (1024) | ~28 | **0.00** | 0.0 |

## What the numbers show

- **Value arithmetic is stack-only.** `checked_add` / `checked_mul` / `checked_div` and the
  scale-aligning helpers operate entirely on the coefficient integer (`i128`, and Arrow's `i256`
  for `d256`) — `0 allocs / op` across the board, including the 256-bit width. Overflow is
  *checked* rather than wrapping, so the guided `DecimalError` costs nothing on the happy path.
- **Identity normalizes on the stack.** `==` / `cmp` compare by value (`2.5 == 2.50`) by aligning
  scales through a checked multiply, and `Hash` streams the normalized coefficient bytes into the
  hasher through a `MAX_WIDTH` stack scratch — `0 allocs / op`, so a decimal is a first-class map
  key and set member with no per-lookup cost.
- **Bulk build materializes once.** `DecimalSerie::from_options` is `2.00 allocs / op` — one pass
  fits + encodes every coefficient into a `Vec<u8>`, then `Buffer::from_vec` moves that allocation
  into the Arrow buffer with **no copy** (plus the small `Arc` box), the same discipline as the
  primitive columns. The incremental `push` re-seals the immutable buffer per element (like
  `Serie::push`), so the bulk builders are the fast path.
- **Reads and Arrow export are zero-copy.** `DecimalSerie::get` decodes one coefficient from the
  borrowed bytes — `0 allocs / op` — and `to_arrow_array` shares the coefficient allocation with
  the Arrow `Decimal128Array` (an `Arc` bump), carrying the column's `(precision, scale)` with no
  payload copy. The round-trip through a byte sink is `12 allocs` for the whole 1024-element
  column (dominated by the read-side value + validity buffers) and is an exact inverse across
  nulls and the full width.
