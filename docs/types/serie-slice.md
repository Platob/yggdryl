# Serie slice

A window over a [`Serie`](serie.md) that reads and writes through the serie's own implementation, moving nothing: `SerieSlice` over a serie the caller holds, `SerieSliceMut` over one it holds mutably.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | `SerieSlice<'a>`: a reference to a serie, an offset and a length - two words beside the reference, `Copy`. `SerieSliceMut<'a>` is the same window over a `&mut Serie`. Neither holds a row or a buffer of its own |
| Doors | `Serie::window(offset, length)` and `Serie::window_mut(offset, length)`; refused naming the serie and both counts when the window reaches past the end. `window(offset, length)` on a window is a narrower one, rebased onto the serie |
| Indexes | Every index is window-relative, bounds-checked against the window, then rebased by the offset before the serie answers - so a leaf's `value(i)` under it is one bounds check and one buffer read, as it was |
| Reads | `len`, `is_empty`, `offset`, `serie()` (the whole), `field`, `dtype`, `null_count` (none when the column holds no absent row, else one validity read per window row, no row built), `is_null(i)`, `scalar(i)`, `get(i)`, `iter()`, `rows()`, `memory_size`, `into_serie()` (`Serie::slice`: zero copy for a column, the window's values copied for a run), and the [ordering verbs](serie.md#sorting-uniqueness-and-partitions) over the window: `is_sorted`, `is_unique`, `unique_count`, `sort_indices`, `into_sorted`, `into_unique`, `into_reversed`, `into_taken`, `into_filtered`, `partition_by`, each what the sliced serie answers |
| Writes | `SerieSliceMut` only: `set(i, v)`, `fill(v)`, `swap(i, j)`, `copy_from(&SerieSlice)`, `splice(range, rows)`, `as_sorted(options)`, `as_reversed()`, `as_taken(indices)`, each through `Serie::splice` or `Serie::set` on the rebased range. A window never grows or shrinks what it views: `splice` takes exactly as many rows as the range, `as_taken` exactly as many indices as the window, and `as_unique` and `as_filtered` are not offered |
| In place | `as_sorted` and `as_reversed` sort or reverse the window of a primitive column's native slice where it stands when the column holds its buffer alone, and a run's values in place when it holds them alone; every other leaf writes the ordered rows back through `splice` |
| Identity | The window's rows alone, as a serie's is its rows: a window equals, orders as and hashes like the serie of the same rows, on either side of the comparison; neither the serie nor the offset is identity. `Display` renders the serie's name - `$` for a run - and the window's rows; `Debug` the name, the offset, the length and the null count |
| Bindings | Rust. A binding's `serie.window(offset, length)` is one class holding the serie object beside the offset and the length and delegating per call; the binding workers follow the Rust page |

## Use

A window reads a stretch of rows where they stand and writes them back where they stand, so a caller sorting the middle of a column, filling a run of slots or swapping two rows never builds a second serie.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, SortOptions};

    let mut prices = Serie::from_arrow_array(
        Some(&Field::new("price", DataType::Int64, false)),
        Arc::new(Int64Array::from(vec![9, 3, 1, 2, 0])) as ArrayRef,
        ArrowCastOptions::new(),
    )?;

    // A window reads through the serie, window-relative.
    let middle = prices.window(1, 3)?;
    assert_eq!((middle.len(), middle.offset()), (3, 1));
    assert_eq!(middle.scalar(0)?, Scalar::from(3_i64));
    assert_eq!(middle.rows().to_vec(), [3_i64, 1, 2].map(Scalar::from));
    assert!(middle.scalar(3).is_err());
    assert_eq!(middle.window(1, 2)?.offset(), 2);
    assert_eq!(middle.into_serie(), prices.slice(1, 3)?);
    assert!(!middle.is_sorted(SortOptions::default()));
    assert_eq!(middle.into_sorted(SortOptions::default())?.rows().to_vec(), [1_i64, 2, 3].map(Scalar::from));

    // Identity is the window's rows.
    assert!(middle == Serie::new([3_i64, 1, 2].map(Scalar::from).to_vec()));

    // And writes through it, in place, never past its edges: a primitive
    // column holding its buffer alone sorts the window where it stands.
    let mut window = prices.window_mut(1, 3)?;
    window.set(0, Scalar::from(7_i64))?;
    window.swap(0, 2)?;
    window.as_sorted(SortOptions::default())?.as_reversed()?;
    assert!(window.splice(0..1, vec![]).is_err());
    assert_eq!(prices.rows().to_vec(), [9_i64, 7, 2, 1, 0].map(Scalar::from));
    prices.window_mut(0, 2)?.fill(Scalar::from(4_i64))?;
    assert_eq!(prices.as_int64().expect("an int64 column").values(), &[4, 4, 2, 1, 0]);
    ```

=== "Python"

    ```python
    # Rust only.
    ```

=== "JavaScript"

    ```javascript
    // Rust only.
    ```

## Reads

| Ask | Cost |
| --- | --- |
| `window`, `window_mut`, a narrower `window` | nothing allocated: a bounds check and two words |
| `scalar(i)`, `get(i)`, `is_null(i)`, `iter()`, `rows()` | the serie's own cost on the rebased row - a leaf value allocates nothing - plus one bounds check against the window; `rows()` lends a run's window and builds a column's rows once |
| `null_count` | constant where the column holds no absent row; one validity read per window row otherwise; a run walks its values |
| `into_serie` | `Serie::slice`: the buffers shared and one leaf boxed for a column, the window's values copied for a run |
| `memory_size` | a column's window as its own slice counts it; a run's values as the row estimator charges them |
| `is_sorted`, `is_unique`, `unique_count`, `sort_indices`, `into_*`, `partition_by` | [what the serie's verb costs](serie.md#what-each-ask-costs) over the sliced window, which for a column shares the buffers |

## Writes

| Ask | Cost |
| --- | --- |
| `set(i, v)` | exactly `Serie::set` on the rebased row: one buffer write on a primitive or boolean leaf |
| `fill(v)` | `v` proved once and written as clones over the rebased range |
| `swap(i, j)` | two rows read and two `Serie::set` |
| `copy_from(window)`, `splice(range, rows)` | `Serie::splice` on the rebased range, every row through the field's contract; refused by name when the counts differ |
| `as_sorted(options)`, `as_reversed()` | a primitive column held alone: the window of the native slice sorted or reversed where it stands, absent rows gathered to the end the options name - Arrow's builder handshake and never a row; a run held alone: its values in place; any other leaf: the ordered rows written back through `splice` |
| `as_taken(indices)` | the rearranged rows written back through `splice`; exactly as many indices as the window |

## Edges

- A window past the end is refused naming the serie and both counts - `rows 2..4 reach past the 3 rows price holds` - and so is a narrower window past the window, or a row past it: `row 2 is past the 2 rows price holds`.
- A write that would grow or shrink the window is refused by name: `a window of price never grows or shrinks what it views: 0 rows cannot replace 1`; `1 indices cannot rearrange 2 rows`. `as_unique` and `as_filtered` do not exist on a window for the same reason.
- A value the field refuses refuses `set`, `fill`, `swap`, `copy_from` and `splice` naming the field, and the serie is left as it was; a run accepts any value.
- A window over a shared column writes through `Arc::make_mut`: the column is copied once and the other holder is left alone, exactly as a `Serie` write does.
- `into_serie` on a run copies the window's values, so the ordering reads over a run's window cost that copy; a column's window shares its buffers.
- The borrowed view of a mutable window is `as_window()`, which answers every read of `SerieSlice`; the mutable window also answers them directly, each through that view.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root serie_slice
    cargo test -p yggdryl --test allocations -- a_window_over_a_primitive
    ```

=== "Python"

    ```bash
    # Rust only.
    ```

=== "JavaScript"

    ```bash
    # Rust only.
    ```
