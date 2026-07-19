# `typed::nested` — the recursive table layer's build / access / mutation costs

Time **and** memory for the [`nested`](../../../crates/yggdryl-core/src/typed/nested) typed layer —
the erased [`Column`](../../../crates/yggdryl-core/src/typed/nested/column.rs) carrier and the three
recursive "tables" it composes:
[`StructSerie`](../../../crates/yggdryl-core/src/typed/nested/struct_/serie.rs) (a heterogeneous set
of equal-length child columns + optional row validity),
[`ListSerie`](../../../crates/yggdryl-core/src/typed/nested/list/serie.rs) (`i32` offsets + a
flattened child column), and
[`MapSerie`](../../../crates/yggdryl-core/src/typed/nested/map/serie.rs) (`i32` offsets + a two-column
`key`/`value` entries struct). The point: the layout is **buffers, not boxed rows** — a build owns one
backing per leaf column plus each nested level's offsets/validity, a **`column_by_name` lookup is a
pure borrow** (zero allocation), a **deep `column_by_name_mut` + `set` sweep** rewrites a child in
place with no per-row allocation, and only a **row/list/map materialize** allocates — the small,
bounded cost of the one owned scalar it hands back.

## Run

```bash
cargo bench -p yggdryl-core --bench typed_nested
cargo test  -p yggdryl-core --test typed_nested         # functional round-trips
cargo test  -p yggdryl-core --test typed_nested_alloc   # deterministic allocation budgets
```

## Release, counting global allocator, 100 000 rows, 2000 iters

The **build** and **deep-sweep** rows report per-row throughput (`Melem/s`); the **lookup** and
**materialize** rows report per-call throughput over the same built table (millions of calls/s).

### Struct "table" — 3 columns (`Int64` + `Utf8` + a nested `Struct` of `Int64` + `Utf8`)

| op | M/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `StructSerie` build (`from_columns`, nested) | 10.4 | 79 | 6 844 476 |
| `column_by_name` (borrow lookup) | 36.0 | **0** | **0** |
| `row(i)` (random, materialize) | 0.4 | 13 | 440 |
| `column_by_name_mut` + `set` (deep sweep) | **74.2** | **0** | **0** |

### List column (`i32` offsets + flattened `Int64` child, 25 000 lists × 4)

| op | M/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `ListSerie` build (`push` demarcation) | 30.3 | 18 | 1 062 460 |
| `list(i)` (random, materialize) | 5.2 | 1 | 256 |

### Map column (`i32` offsets + `Utf8`→`Int64` entries, 25 000 maps × 4)

| op | M/s | allocs/op | bytes/op |
|---|--:|--:|--:|
| `MapSerie` build (`push` demarcation) | 3.7 | 59 | 4 208 652 |
| `get(i)` (random, materialize) | 0.7 | 10 | 559 |

## Deterministic allocation budgets

From [`tests/typed_nested_alloc.rs`](../../../crates/yggdryl-core/tests/typed_nested_alloc.rs) —
counts are **independent of the row count**, so they are measured over a length-3 table and asserted
exactly / by a tight bound:

| path | allocations | why |
|---|--:|---|
| `from_columns` (over a pre-built `Vec<Column>`) | **0** | reuses the caller's Vec, empty `Headers`, no validity — a pure combine |
| `column_by_name` (borrow lookup) | **0** | walks the children, compares borrowed `&str` names |
| `column_path` (dotted, into a nested child) | **0** | `split_once` borrows; still name compares only |
| `column_by_name_mut` + `set` (deep, in-place) | **0** / row | recovers the concrete leaf, one positioned write |
| `row(i)` (materialize a `StructScalar`) | ≤ 13 | `names` + `values` Vecs, one owned value per child, nested row recurses |
| from-scratch build (leaf buffers + combine) | ≤ 22 | one data/offsets backing + a name per leaf, the nested combine, the two Vecs |

## What the numbers show

- **The combine is free; only the leaf buffers cost.** `from_columns` allocates **nothing** — it takes
  ownership of the caller's `Vec<Column>`, seeds an empty `Headers`, and holds no validity buffer. The
  79 allocations the 100 000-row build row measures all belong to the **leaf columns** it is handed:
  each `FixedSerie`/`VarSerie::from_values` owns its data (and a var column its offsets), and the two
  `Utf8` children grow their offset/data heaps through the `Vec`-doubling append path (`O(log n)`
  reallocations). Real code builds each column once and reads it many times — the structural combine
  never adds to that.
- **Lookups are pure borrows.** `column_by_name` (and the dotted `column_path`) return a `&Column` by
  walking the children and comparing borrowed names — **zero** allocation, ~36 M lookups/s, so
  navigating the tree is free.
- **Deep mutation is in place, no copy.** Recovering the concrete `FixedSerie<Int64>` behind the erased
  `&mut Column` and rewriting every row with a positioned `set` runs at **74 Melem/s with zero
  allocations** — the erased `Column` is a *view* onto the concrete carrier, not a boxing wrapper, so a
  deep edit never rebuilds a column.
- **Only materialize allocates — and bounded.** `row(i)` / `list(i)` / `map get(i)` hand back an owned
  scalar (a `StructScalar` row, a `ListScalar`, a `MapScalar`), so they allocate the small constant the
  result owns (13 / 1 / 10 here), never a per-element cost. A `list(i)` of four `Int64`s is one 256-B
  `Vec`; the struct `row` costs more because it owns two Vecs plus a name per child and the nested row
  recurses.
- **List builds beat map builds.** A `ListSerie` demarcates lists by advancing one `i32` offset per
  `push` over a single flattened child (30 Melem/s); a `MapSerie` carries a two-column entries
  `StructSerie` **and** its offsets, and its `Utf8` key child dominates the build cost (3.7 Melem/s) —
  the same variable-length growth seen in the struct build.

## The seams that remain

- `push`-driven builds advance offsets and (once nulls appear) set validity **one bit at a time**;
  the offsets/validity heaps are not pre-sized to the known list/map count, so the build pays the
  `Vec`-doubling reallocations. A `with_capacity` / bulk-offset front door would cut the build
  allocation count to one buffer per level — marked for the pre-sized nested-build path.
- `row(i)` allocates two Vecs plus a name per child every call; a reusable row cursor (fill-into a
  caller buffer, like the byte layer's `pread_into`) would make repeated row scans allocation-free.
