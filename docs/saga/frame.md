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
| `filter(predicate)` | keep rows where the named boolean column is true |
| `limit(n)` / `head(n)` / `tail(n)` / `slice(offset, length)` | row limits (`head` provided) |

An implementor supplies `schema`, `column`, `select`, `filter`, `limit`, `tail` and
`slice`; everything else is provided.

## A backing-agnostic pipeline

Because every method is on the trait, one function runs over **any** `Frame` — the
eager and lazy implementations (added later) both qualify.

=== "Rust"

    ```rust
    use yggdryl_saga::{Frame, FrameError};

    fn top_trades<F: Frame>(frame: F) -> Result<F, FrameError> {
        frame
            .select(&["ts", "symbol", "px", "qty"])?
            .filter("is_fill")?   // keep rows where the boolean column `is_fill` is true
            .head(100)
    }
    ```

## Next

- [Column](column.md) — the per-column trait `Frame::column` yields
