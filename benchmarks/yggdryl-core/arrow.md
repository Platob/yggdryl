# `arrow` — the one-copy interop bridge

Time **and** memory for the Apache Arrow interop bridge
([`crate::arrow`](../../crates/yggdryl-core/src/arrow), feature `arrow`) — the leaf
[`column_to_arrow`](../../crates/yggdryl-core/src/arrow/array.rs) /
[`column_from_arrow`](../../crates/yggdryl-core/src/arrow/array.rs) conversions and the top-level
[`struct_serie_to_record_batch`](../../crates/yggdryl-core/src/arrow/record_batch.rs) /
[`struct_serie_from_record_batch`](../../crates/yggdryl-core/src/arrow/record_batch.rs) round-trip.
The point: the handoff is **one buffer copy per column buffer**, independent of the row count — the
entry point borrows the `&Column`, so its owning `Heap` cannot be moved into a zero-copy
`Buffer::from_vec`; the copy is the single unavoidable cost, and the from-Arrow direction re-encodes
the logical values once (into a pre-sized `Heap`, respecting a sliced input).

## Run

```bash
cargo bench -p yggdryl-core --features arrow --bench arrow
cargo test  -p yggdryl-core --features arrow --test arrow         # functional round-trips
cargo test  -p yggdryl-core --features arrow --test arrow_alloc   # one-copy allocation budget
```

## Release, counting global allocator, 100 000 rows, 2000 iters

### Leaf column ↔ Arrow array

| op | Melem/s | allocs/op | bytes/op | note |
|---|--:|--:|--:|---|
| `column_to_arrow` `i64` | 336.6 | **3** | 800 168 | reinterpret + **1 copy** (800 000 B) + `Arc`/`ArrayData` |
| `column_from_arrow` `i64` | **792.7** | **1** | 800 000 | one vectorized `encode_slice` into a pre-sized `Heap` |
| `column_to_arrow` `utf8` | 218.6 | 5 | 989 240 | offsets **+** data, one copy each + `Arc`/`ArrayData` |
| `column_from_arrow` `utf8` | 21.3 | 19 | 2 497 148 | rebuilt element-by-element (respects a sliced input, rebases offsets) |

### Struct "table" ↔ `RecordBatch` (3 columns: `Int64` + `Utf8` + `Int64`)

| op | Melem/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `struct_serie_to_record_batch` | 61.1 | 43 | 2 591 188 |
| `struct_serie_from_record_batch` | 15.9 | 46 | 4 098 616 |

## Deterministic allocation budget

From [`tests/arrow_alloc.rs`](../../crates/yggdryl-core/tests/arrow_alloc.rs): a leaf
`column_to_arrow` on a non-null `Int64` column allocates the **same 3 times at 1 000 rows and at
100 000 rows** — the decisive proof that the buffer handoff is **one bulk copy, not one allocation per
element**. Those 3 are the value-buffer copy, the `ArrayData`, and the `Arc<dyn Array>` — fixed
overhead, never scaling with the row count.

## What the numbers show

- **To-Arrow is one copy per buffer.** A numeric column reinterprets its little-endian data `Heap` into
  a single 64-byte-aligned Arrow `Buffer` (`i64`: the 800 000-byte copy), wraps it in a `ScalarBuffer`
  (no copy), and `Arc`s the array — **3 allocations total, constant in the row count**. A `utf8` column
  copies its offsets **and** its data buffer (2 copies + overhead → 5). The copy is unavoidable because
  the API borrows `&Column`; moving the `Heap` in would be zero-copy but would consume the column.
- **From-Arrow into a numeric column is the least-copy path (1 allocation).** `column_from_arrow` for
  `i64` runs one vectorized `encode_slice` straight into a `Heap::with_capacity(len * 8)` — a single
  800 000-byte buffer, **792 Melem/s**, the fastest row here. It reads Arrow's logical `.values()`
  slice, so a sliced/offset input array is respected with no extra buffer.
- **Variable-length from-Arrow is value-by-value by design.** `column_from_arrow` for `utf8` rebuilds
  through the logical `arr.value(i)` accessor so a **sliced** input is honored and offsets are rebased
  from 0 — that costs a per-element write path (19 allocations, 21 Melem/s) rather than a raw buffer
  reinterpret. It trades peak throughput for slice-correctness; a fast path for the contiguous,
  offset-0 case is marked in the source.
- **`RecordBatch` is the leaf costs summed, plus the schema.** `struct_serie_to_record_batch` converts
  each column through `column_to_arrow` (the same one-copy leaves) and assembles the schema (43
  allocations for the 3-column table); the inverse rebuilds each column and carries the schema metadata
  back. A record batch has **no row-level validity**, so the bridge refuses a struct that actually holds
  null rows rather than silently dropping them (see the
  [record-batch module docs](../../crates/yggdryl-core/src/arrow/record_batch.rs)).
