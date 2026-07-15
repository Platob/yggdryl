# Arrow interop — benchmark & zero-copy notes

Time **and** memory for the zero-copy Apache Arrow interop (feature `arrow`): the fixed family
(`Buffer` / `Serie`) ↔ `arrow_array::PrimitiveArray`. The point of the interop is that the
value payload is **never copied**, so the allocations/op column is the story.

## Run

```bash
cargo bench -p yggdryl-core --features arrow --bench arrow
cargo test  -p yggdryl-core --features arrow --test io_arrow   # ptr_eq zero-copy assertions
```

## Rust core (release, counting global allocator, 4096 × i32)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Buffer::to_arrow_array` (zero-copy) | 28.10 | **0.00** | 0.0 |
| `Buffer::from_arrow_array` (zero-copy) | 164.64 | **0.00** | 0.0 |
| `Serie::to_arrow_array` (dense) | 21.62 | **0.00** | 0.0 |
| `Serie::to_arrow_array` (nullable) | 4.35 | 2.00 | 568.0 |
| `Serie::from_arrow_array` (nullable) | 0.14 | 2.00 | 1024.0 |

## What the numbers show

The physical layer is an `Arc`-shared Arrow `Buffer`, so conversion is a refcount bump:

- **Buffer ↔ Arrow is fully zero-copy.** `to_arrow_array` wraps the shared `Buffer` in a
  `ScalarBuffer` (`slice_with_length(0, ..)` = an `Arc` clone) and `from_arrow_array` takes the
  array's values buffer back the same way — **0 allocs, 0 bytes**, for a 16 KiB payload. The
  `io_arrow` test asserts the shared allocation with `Buffer::ptr_eq`.
- **A dense column is zero-copy too.** `Serie::to_arrow_array` on a null-free column has no
  validity buffer to build, so it is also **0 allocs**.
- **A nullable column copies only the validity mask.** The `568` / `1024` bytes/op are the
  4096-bit (512-byte) validity bitmap plus small metadata — **not** the 16 KiB values, which
  stay shared. Our bitmap is LSB-first with `1 = valid`, byte-identical to Arrow's
  `NullBuffer`, so `to`/`from` round-trip nulls exactly; only the tiny mask is materialized.

## The trait hierarchy is zero-cost

The generic trait layer (`DataType` / `TypedDataType` / `ScalarType` / `SerieType` /
`BufferType` / `FieldType` + the `Fixed*` sub-traits) adds **no runtime cost**: the descriptors
are zero-sized, the `Fixed*` default methods monomorphize to the same code as the inherent
methods, and `&dyn DataType` / `&dyn FieldType` are used only where erasure is wanted. All
existing `io_fixed` / `io_fixed_types` allocation budgets are unchanged by the refactor.
