# Column (trait)

`Column` is the shared contract for a single named, typed column — **whatever its
backing**. One trait covers a **materialized** column (values already in memory)
and a **lazy** column (an expression yet to be evaluated), so the rest of the
engine treats them alike.

The column's *identity* — its [`Field`](field.md) (name, `DataType`, nullability) —
is always known, so `name` / `data_type` / `is_nullable` are total. Its *length*
may not be: `len()` returns `Option<usize>` (`None` when a lazy column would have to
be computed to answer). Transformations consume `self` and return a column of the
same kind, so they compose whether they run now or are recorded for later.

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## The contract

| method | meaning |
| --- | --- |
| `field()` / `name()` / `data_type()` / `is_nullable()` | identity — always known |
| `frame()` → `Option<&dyn FrameHandle>` | the holder frame, if any (provided, default `None`) |
| `is_materialized()` | whether values are in memory |
| `len()` → `Option<usize>` | length if known without evaluating |
| `is_empty()` | `len() == 0`, when known (provided) |
| `rename(name)` | a renamed column |
| `cast(data_type)` | a cast column, or `ColumnError::Cast` |
| `slice(offset, length)` / `head(n)` / `tail(n)` | row sub-ranges (`head` provided) |

`name`, `data_type`, `is_nullable`, `frame`, `is_empty` and `head` are **provided**
methods — an implementor only supplies `field`, `is_materialized`, `len`, `rename`,
`cast`, `slice` and `tail`.

## Holder frame

A column can map back to the [`Frame`](frame.md) that holds it via `frame()`. It is
**optional** — a column built on its own to use as an expression is *detached* and
returns `None` (the default). When present, the holder is handed back behind the
object-safe `FrameHandle` (the base trait of `Frame`), so the column reaches its
holder's schema without naming the frame's concrete type.

=== "Rust"

    ```rust
    use yggdryl_saga::{Column, FrameHandle};

    fn holder_columns<C: Column>(col: &C) -> Vec<String> {
        match col.frame() {
            Some(frame) => frame.schema().map(|s| s.names().iter().map(|n| n.to_string()).collect()).unwrap_or_default(),
            None => Vec::new(), // detached column (used as an expression)
        }
    }
    ```

## Implementing it

=== "Rust"

    ```rust
    use yggdryl_saga::{Column, ColumnError, DataType, Field, PrimitiveType};

    struct Vec64 { field: Field, values: Vec<i64> }

    impl Column for Vec64 {
        fn field(&self) -> &Field { &self.field }
        fn is_materialized(&self) -> bool { true }
        fn len(&self) -> Option<usize> { Some(self.values.len()) }
        fn rename(mut self, name: impl Into<String>) -> Self {
            self.field = self.field.with_name(name); self
        }
        fn cast(self, to: DataType) -> Result<Self, ColumnError> {
            Err(ColumnError::Cast { from: self.field.data_type().clone(), to })
        }
        fn slice(mut self, offset: usize, length: usize) -> Result<Self, ColumnError> {
            let end = offset.saturating_add(length).min(self.values.len());
            self.values = self.values[offset.min(end)..end].to_vec();
            Ok(self)
        }
        fn tail(self, n: usize) -> Result<Self, ColumnError> {
            let len = self.values.len();
            self.slice(len.saturating_sub(n), n)
        }
    }

    let col = Vec64 {
        field: Field::new("px", PrimitiveType::Int64.into(), false),
        values: vec![1, 2, 3],
    };
    assert_eq!(col.name(), "px");
    assert_eq!(col.head(2).unwrap().len(), Some(2));
    ```

## Next

- [Frame](frame.md) — the table-level trait that yields `Column`s
