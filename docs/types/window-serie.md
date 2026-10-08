# Window serie

A window over a [`Serie`](serie.md) that reads and writes through the serie's own implementation, moving nothing: `WindowSerie` over a serie the caller holds, `WindowSerieMut` over one it holds mutably.

## Contract

`WindowSerie` is a live view over a parent serie, an offset and a length. Reads
and writes go through that parent at the time of the call. Indexes are relative
to the window; writes cannot grow or shrink it. `into_serie` shares an owned
slice of the current snapshot. A parent that no longer reaches the window makes
its next access refuse.

Clustering a window returns owned [KeySeries](key-serie.md), with absolute source
starts and shared payload buffers. These snapshots can outlive the window.

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
    import pyarrow as pa

    from yggdryl import Field, Serie

    prices = Serie.from_arrow_array(
        pa.array([9, 3, 1, 2, 0], pa.int64()), Field("price", "int64", nullable=False)
    )

    # A window reads through the serie, window-relative.
    middle = prices.window(1, 3)
    assert (len(middle), middle.offset) == (3, 1)
    assert middle.scalar(0).as_py() == 3
    assert middle.as_py() == [3, 1, 2]
    try:
        middle.scalar(3)
    except ValueError:
        pass
    else:
        raise AssertionError("a row past the window is refused")
    assert middle.window(1, 2).offset == 2
    assert middle.into_serie() == prices.slice(1, 3)
    assert not middle.is_sorted()
    assert middle.into_sorted().as_py() == [1, 2, 3]

    # Identity is the window's rows.
    assert middle == Serie.from_scalars(Field("price", "int64", nullable=False), [3, 1, 2])

    # And writes through it, in place, never past its edges.
    window = prices.window(1, 3)
    window.set(0, 7)
    window.swap(0, 2)
    window.as_sorted().as_reversed()
    try:
        window.splice(0, 1, [])
    except ValueError:
        pass
    else:
        raise AssertionError("a window never shrinks what it views")
    assert prices.as_py() == [9, 7, 2, 1, 0]
    prices.window(0, 2).fill(4)
    assert prices.as_py() == [4, 4, 2, 1, 0]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, Serie } = require('yggdryl')

    const field = Field.from('price: int64 not null')
    const prices = Serie.fromArrowArray(arrow.vectorFromArray([9n, 3n, 1n, 2n, 0n], new arrow.Int64()), field)

    // A window reads through the serie, window-relative.
    const middle = prices.window(1, 3)
    assert.deepEqual([middle.length, middle.offset], [3, 1])
    assert.equal(middle.scalar(0).asJs(), 3)
    assert.deepEqual(middle.asJs(), [3, 1, 2])
    assert.throws(() => middle.scalar(3))
    assert.equal(middle.window(1, 2).offset, 2)
    assert.ok(middle.intoSerie().equals(prices.slice(1, 3)))
    assert.equal(middle.isSorted(), false)
    assert.deepEqual(middle.intoSorted().asJs(), [1, 2, 3])

    // Identity is the window's rows.
    assert.ok(middle.equals(Serie.fromScalars(field, [3n, 1n, 2n])))

    // And writes through it, in place, never past its edges.
    const window = prices.window(1, 3)
    window.set(0, 7n)
    window.swap(0, 2)
    window.asSorted().asReversed()
    assert.throws(() => window.splice(0, 1, []))
    assert.deepEqual(prices.asJs(), [9, 7, 2, 1, 0])
    prices.window(0, 2).fill(4n)
    assert.deepEqual(prices.asJs(), [4, 4, 2, 1, 0])
    ```

## Windows by key

`window_by` and `partition_by` take the shared key intake and return owned
[KeySeries](key-serie.md). The window itself keeps only its live parent, offset
and length; key context belongs to each returned `KeySerie`.

## Reads

| Ask | Cost |
| --- | --- |
| `window`, `window_mut`, a narrower `window` | nothing allocated: a bounds check and two words |
| `scalar(i)`, `get(i)`, `is_null(i)`, `iter()`, `rows()` | the serie's own cost on the rebased row - a leaf value allocates nothing - plus one bounds check against the window; `rows()` lends a run's window and builds a column's rows once |
| `null_count` | constant where the column holds no absent row; one validity read per window row otherwise; a run walks its values |
| `into_serie` | `Serie::slice`: the buffers shared and one leaf boxed for a column, the values shared for a run, the start and length moved |
| `window_by` | what [`Serie::window_by`](key-serie.md#context-and-payload) costs over the window's rows; the resulting `KeySeries` owns explicit key contexts and shared payloads |
| `memory_size` | a column's window as its own slice counts it; a run's values as the row estimator charges them |
| `resident_size`, `is_spilled` | the serie's own answer over the rows the window views: a window is never spilled on its own - [spill the serie](serie.md#spilling-to-disk); a run counts its values and is never spilled |
| `sort_indices_by`, `into_sort_by` | [`Serie::sort_indices_by`](serie.md#sort-by-keys) over the window's rows, window-relative positions, a serie of its own |
| `is_sorted`, `is_unique`, `unique_count`, `sort_indices`, `into_*`, `partition_by` | one boxed leaf - the window's serie, for a column sharing the buffers - plus [what the serie's verb costs](serie.md#what-each-ask-costs) over it |

## Writes

| Ask | Cost |
| --- | --- |
| `set(i, v)` | exactly `Serie::set` on the rebased row: one buffer write on a primitive or boolean leaf |
| `fill(v)` | `v` proved once and written as clones over the rebased range |
| `swap(i, j)` | two rows read and two `Serie::set` |
| `copy_from(window)`, `splice(range, rows)` | `Serie::splice` on the rebased range, every row through the field's contract; refused by name when the counts differ |
| `as_sorted(options)`, `as_reversed()` | a primitive or boolean column held alone: the window of the native slice, or of the boolean's two bitmaps, sorted or reversed where it stands, absent rows gathered to the end the options name - Arrow's builder handshake and never a row; a run held alone: its values in place; any other leaf: the ordered rows written back through `splice` |
| `as_taken(indices)` | the rearranged rows written back through `splice`; exactly as many indices as the window |
| `as_sort_by(by)` | the window's rows sorted by the keys and written back through `splice` on the rebased range |
| any write on a serie declaring an [order](serie.md#a-declared-order) | the serie's rule: the written rows compared against their neighbours, the declaration kept where the order holds and cleared otherwise |

## Edges

- A window past the end is refused naming the serie and both counts - `rows 2..4 reach past the 3 rows price holds` - and so is a narrower window past the window, or a row past it: `row 2 is past the 2 rows price holds`.
- A write that would grow or shrink the window is refused by name: `a window of price never grows or shrinks what it views: 0 rows cannot replace 1`; `1 indices cannot rearrange 2 rows`. `as_unique` and `as_filtered` do not exist on a window for the same reason.
- A value the field refuses refuses `set`, `fill`, `swap`, `copy_from` and `splice` naming the field, and the serie is left as it was; a run accepts any value.
- A window over a shared column writes through `Arc::make_mut`: the column is copied once and the other holder is left alone, exactly as a `Serie` write does.
- `into_serie` shares what the window views - a column's buffers, a run's values - so the ordering reads over a window cost the serie's own verb and, for a column, one boxed leaf.
- Only a window `KeySeries::iter` lent states a record: a narrower `window`, `Serie::window`, a `WindowSerieMut` and its `as_window` state none, and `into_serie` - and every Arrow array built from it - carries rows only.
- Windowing a window `window_by` lent refuses a key cell whose name folds onto a cell its record keeps, as it refuses `windownum` and `rownum`, naming both - alias it.
- The borrowed view of a mutable window is `as_window()`, which answers every read of `WindowSerie`; the mutable window also answers them directly, each through that view.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root window_serie
    cargo test -p yggdryl --test allocations -- a_window_over_a_primitive
    cargo test -p yggdryl --test allocations -- a_held_window_record a_window_reached_by_nth
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_window_serie.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/window_serie.test.js
    ```
