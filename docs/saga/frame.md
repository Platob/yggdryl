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
| `filter(predicate)` | keep rows matching a [`Predicate`](predicate.md) (implementations may push it down) |
| `filter_typed(predicate)` | type-optimise the predicate against the schema, then `filter` (**provided**) |
| `limit(n)` / `head(n)` / `tail(n)` / `slice(offset, length)` | row limits (`head` provided) |

An implementor supplies `schema`, `column`, `select`, `filter`, `limit`, `tail` and
`slice`; everything else is provided.

## Filtering & pushdown

`filter` takes a [`Predicate`](predicate.md). The provided `filter_typed` first
**type-optimises** it against the frame's schema — casting each literal to its
column's type (e.g. a string ISO date → `timestamp`) — so the comparison is typed
and an implementation (a `ParquetFrame`, a `CsvFrame`) can push it down into typed
storage. This is the path that turns `col("ts") > "2024-01-01"` into a typed range
scan.

=== "Rust"

    ```rust
    use yggdryl_saga::{Frame, FrameError, Predicate, Scalar};

    fn recent_fills<F: Frame>(frame: F) -> Result<F, FrameError> {
        frame
            .select(&["ts", "symbol", "px", "qty"])?
            // An untyped ISO string is cast to the `ts` column's timestamp type,
            // then pushed down.
            .filter_typed(Predicate::ge("ts", Scalar::any("2024-01-01")))?
            .head(100)
    }
    ```

## Next

- [Predicate](predicate.md) — the filtering expressions and type-casting pushdown
- [Column](column.md) — the per-column trait `Frame::column` yields
