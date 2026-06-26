# Frame (trait)

`Frame` is the shared contract for a tabular frame — **whatever its backing**. One
trait covers an **eager** frame (rows already in memory) and a **lazy** frame (a
query plan yet to run): the same `select` / `filter` / column-access surface, so
callers compose pipelines without caring which they hold.

The [`Schema`](schema.md) is always resolvable (a lazy frame tracks it without
executing), which is why the structural defaults (`width`, `column_names`, `drop`,
`contains_column`, …) are total. Transformations consume `self` and return the same
frame kind, so a generic pipeline works across backings. Each frame yields its own
[`Column`](column.md) type via the associated `type Column`.

`Frame` is not object-safe (associated `Column` type, generic methods), so its
`schema()` lives on an **object-safe base trait `FrameHandle`** that `Frame`
extends (`Frame: FrameHandle`). That base is exactly what a held
[`Column`](column.md) reaches through `column.frame()` — a `&dyn FrameHandle` — to
see its holder's schema without knowing the frame's concrete type.

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## The contract

| method | meaning |
| --- | --- |
| `schema()` | the frame's `Schema` (resolved, never executed) |
| `column(name)` | access a column (lazy or materialized) |
| `width()` / `column_names()` / `contains_column(name)` | schema-derived, **provided** |
| `height()` → `Option<usize>` | row count if known without executing |
| `is_empty()` | `height() == 0`, when known (provided) |
| `select(columns)` | projection, in order |
| `drop(columns)` | complement of a projection (**provided**) |
| `filter(predicate)` | keep rows matching a [`Predicate`](predicate.md) — types it against the schema, then applies/pushes it down |
| `optimize_predicate(predicate)` | type-optimise a predicate against the schema (the helper `filter` uses) (**provided**) |
| `limit(n)` / `head(n)` / `tail(n)` / `slice(offset, length)` | row limits (`head` provided) |

An implementor supplies `schema`, `column`, `select`, `filter`, `limit`, `tail` and
`slice`; everything else is provided.

## Filtering & pushdown

`filter` takes a [`Predicate`](predicate.md) and **does the whole job**: it
type-optimises the predicate against the frame's schema — casting each literal to
its column's type (e.g. a string ISO date → `timestamp`) via the provided
`optimize_predicate` helper — then applies it, **pushing it down** into storage where
it can (a `ParquetFrame` skips row groups, a `CsvFrame` filters on scan). That turns
`col("ts") > "2024-01-01"` into a typed range scan.

=== "Rust"

    ```rust
    use yggdryl_saga::{Frame, FrameError, Predicate, Scalar};

    fn recent_fills<F: Frame>(frame: F) -> Result<F, FrameError> {
        frame
            .select(&["ts", "symbol", "px", "qty"])?
            // `filter` casts the untyped ISO string to the `ts` column's timestamp
            // type and pushes it down.
            .filter(Predicate::ge("ts", Scalar::any("2024-01-01")))?
            .head(100)
    }
    ```

## Next

- [Predicate](predicate.md) — the filtering expressions and type-casting pushdown
- [Column](column.md) — the per-column trait `Frame::column` yields
