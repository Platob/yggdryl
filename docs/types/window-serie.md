# Window serie

A window over a [`Serie`](serie.md) that reads and writes through the serie's own implementation, moving nothing: `WindowSerie` over a serie the caller holds, `WindowSerieMut` over one it holds mutably.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | `WindowSerie<'a>`: a reference to a serie, an offset, a length and - on a window [`window_by`](serie.md#windows-by-key) lent - where its [record](#static-values) comes from: 40 bytes, `Copy`, pinned by a `const` assert. `WindowSerieMut<'a>` is the same window over a `&mut Serie`, stating no record. Neither holds a row or a buffer of its own |
| Doors | `Serie::window(offset, length)` and `Serie::window_mut(offset, length)`; refused naming the serie and both counts when the window reaches past the end. `window(offset, length)` on a window is a narrower one, rebased onto the serie, so a window of a window is one window of the serie. `SerieWindows::iter` lends the windows [`window_by`](serie.md#windows-by-key) cuts |
| Indexes | Every index is window-relative, bounds-checked against the window, then rebased by the offset before the serie answers - so a leaf's `value(i)` under it is one bounds check and one buffer read, as it was |
| Reads | `len`, `is_empty`, `offset`, `serie()` (the whole), `field`, `dtype`, `null_count` (none when the column holds no absent row, else one validity read per window row, no row built), `is_null(i)`, `scalar(i)`, `get(i)`, `iter()`, `rows()`, `memory_size`, `into_serie()` (`Serie::slice`: zero copy, a column's buffers or a run's values shared), [`window_by(by, sorted)`](#windows-by-key), [`static_values()`](#static-values), and the [ordering verbs](serie.md#sorting-uniqueness-and-partitions) over the window: `is_sorted`, `is_unique`, `unique_count`, `sort_indices`, `into_sorted`, `into_unique`, `into_reversed`, `into_taken`, `into_filtered`, `partition_by`, each what the sliced serie answers |
| Writes | `WindowSerieMut` only: `set(i, v)`, `fill(v)`, `swap(i, j)`, `copy_from(&WindowSerie)`, `splice(range, rows)`, `as_sorted(options)`, `as_reversed()`, `as_taken(indices)`, each through `Serie::splice` or `Serie::set` on the rebased range. A window never grows or shrinks what it views: `splice` takes exactly as many rows as the range, `as_taken` exactly as many indices as the window, and `as_unique` and `as_filtered` are not offered |
| In place | `as_sorted` and `as_reversed` sort or reverse the window of a primitive column's native slice, or a boolean column's two bitmaps, where it stands when the column holds its buffer alone, and a run's values in place when it holds them alone; every other leaf writes the ordered rows back through `splice` |
| Identity | The window's rows alone, as a serie's is its rows: a window equals, orders as and hashes like the serie of the same rows, on either side of the comparison; neither the serie, the offset nor the record is identity. `Display` renders the serie's name - `$` for a run - and the window's rows; `Debug` the name, the offset, the length and the null count |
| Bindings | Rust, Python and JavaScript. In both bindings `serie.window(offset, length)` is one class, `WindowSerie`, holding the serie object beside the offset and the length and delegating per call: it is the shared and the mutable window at once, a write checked when it is made, and `window_mut`, `as_window` and `WindowSerieRows` are Rust only. A window `window_by` lent holds its record beside the serie object, and answers it as a struct `Scalar` read by name - `static_values` in Python, `staticValues` in JavaScript, `None` / `null` for every other window. Python's window is unhashable, as the mutable serie it holds is, and `window[a:b]` is a narrower window; JavaScript's equality is `equals` |

One verb, three spellings:

| Rust | Python | JavaScript |
| --- | --- | --- |
| `serie.window(offset, length)?`, `window_mut` | `serie.window(offset, length)` | `serie.window(offset, length)` |
| `len()`, `offset()`, `serie()`, `field()`, `dtype()` | `len(window)`, `offset`, `serie`, `field`, `dtype` | `length`, `offset`, `serie`, `field`, `dtype` |
| `scalar(i)?`, `get(i)`, `iter()`, `rows()` | `scalar(i)`, `get(i)`, `window[i]`, `iter(window)`, `rows()`, `as_py()` | `scalar(i)`, `at(i)`, `[Symbol.iterator]`, `rows()`, `asJs()` |
| `is_null(i)`, `null_count()`, `is_empty()`, `memory_size()` | the same names | `isNull(i)`, `nullCount()`, `isEmpty()`, `memorySize()` |
| `into_serie()` | `into_serie()` | `intoSerie()` |
| `window_by(by, sorted)?` | `window_by(by, sorted=False)` | `windowBy(by, sorted?)` |
| `static_values()`, an `Option<FieldScalar>` | `static_values`, a struct `Scalar` or `None` | `staticValues`, a struct `Scalar` or `null` |
| `is_sorted(options)`, `sort_indices(options)`, `into_sorted(options)` | `is_sorted(descending=, nulls_first=)`, `sort_indices(...)`, `into_sorted(...)` | `isSorted(options?)`, `sortIndices(options?)`, `intoSorted(options?)` |
| `is_unique()`, `unique_count()`, `into_unique()`, `into_reversed()`, `into_taken(indices)`, `into_filtered(mask)`, `partition_by(keys)` | the same names | `isUnique()`, `uniqueCount()`, `intoUnique()`, `intoReversed()`, `intoTaken(indices)`, `intoFiltered(mask)`, `partitionBy(keys)` |
| `set(i, v)?`, `fill(v)?`, `swap(i, j)?`, `copy_from(window)?`, `splice(range, rows)?` | `set(i, v)`, `window[i] = v`, `fill(v)`, `swap(i, j)`, `copy_from(other)`, `splice(start, end, rows)` | `set(i, v)`, `fill(v)`, `swap(i, j)`, `copyFrom(other)`, `splice(start, end, rows)` |
| `as_sorted(options)?`, `as_reversed()?`, `as_taken(indices)?` | `as_sorted(descending=, nulls_first=)`, `as_reversed()`, `as_taken(indices)` | `asSorted(options?)`, `asReversed()`, `asTaken(indices)` |
| `==`, `<` against a window or a `Serie` | `==`, `<`, `<=`, `>`, `>=` | `equals(other)` |

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

A window windows its own rows as [`Serie::window_by`](serie.md#windows-by-key) windows a serie's, `sorted` meaning what it means there. The key is computed over the window's rows alone, so a row outside it never moves a cut, and every window answered is over the same serie at its offset in that serie - unless `sorted` gathers rows out of order, and then over the one copy of this window's rows, from offset 0. A `WindowSerieMut` answers the same windows through `as_window`.

## Static values

A window `SerieWindows::iter` lent states a record of the values constant over its rows, `static_values()`. Nothing else states one: not a window `Serie::window` takes, a narrower window, a `WindowSerieMut`, or a window turned back into a serie or an Arrow array. The record is never the window's identity.

| Cell | Rule |
| --- | --- |
| The record | One required struct named as the windowed serie's root, typed once for every window by `SerieWindows::static_field()` before any window is walked; `None` for every other window |
| Kept cells | Where the window windowed was itself lent by `window_by`, its record's cells but `windownum` and `rownum`, first - so the outer key cells lead |
| Key cells | The key cells at the window's first row, named as the key's projections are and typed as they declare them - nullable too where the windowed rows hold an absent record row, which keys null in every cell |
| `windownum` | `uint64`, required: the window's place among the windows, from 0 |
| `rownum` | `uint64`, nullable: the number the window's first row has in what was windowed - absolute through windows of windows, since a window lent this way numbers from its own `rownum`; counted from the window's first row where a plain window was windowed; null where `sorted` gathered the rows out of their order |

Every cell is read through [`FieldScalar`](field.md)'s own accessors - `get_key_str`, `get`, `value`, `field`, `into_scalar` - and in Python and JavaScript the record is a struct `Scalar` read by name. A [stream window](../arrow/readers.md#windows-of-a-stream) of the same rows states the same record, field and values.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::utf8().required_field("side"),
        DataType::Int64.required_field("price"),
    ])?)
    .required_field("quote");
    let quote = |venue: &str, side: &str, price: i64| {
        Scalar::from_sequence([Scalar::from(venue), Scalar::from(side), Scalar::from(price)])
    };
    let quotes = Serie::from_scalars(root, [
        quote("XNYS", "B", 1),
        quote("XNAS", "B", 2),
        quote("XNAS", "B", 3),
        quote("XNAS", "S", 4),
    ])?;

    let venues = quotes.window_by("venue", false)?;
    let (_, xnas) = venues.iter().nth(1).expect("the XNAS window");
    let record = xnas.static_values().expect("a window window_by lent");
    assert_eq!(record.get_key_str("venue"), Some(&Scalar::from("XNAS")));
    assert_eq!(record.get_key_str("windownum"), Some(&Scalar::from(1_u64)));
    assert_eq!(record.get_key_str("rownum"), Some(&Scalar::from(1_u64)));

    // A window of a window keeps the outer cells first, and its rownum stays absolute.
    let sides = xnas.window_by("side", false)?;
    let names: Vec<&str> = sides.static_field().fields().iter().map(Field::name).collect();
    assert_eq!(names, ["venue", "side", "windownum", "rownum"]);
    let (_, sell) = sides.iter().nth(1).expect("the S window");
    assert_eq!((sell.offset(), sell.len()), (3, 1));
    let record = sell.static_values().expect("its record");
    assert_eq!(record.get_key_str("venue"), Some(&Scalar::from("XNAS")));
    assert_eq!(record.get_key_str("side"), Some(&Scalar::from("S")));
    assert_eq!(record.get_key_str("windownum"), Some(&Scalar::from(1_u64)));
    assert_eq!(record.get_key_str("rownum"), Some(&Scalar::from(3_u64)));

    // A window of the rows alone states none, and the record is never identity.
    assert!(sell.window(0, 1)?.static_values().is_none());
    assert!(quotes.window(3, 1)?.static_values().is_none());
    assert!(sell == quotes.window(3, 1)?);
    assert_eq!(sell.into_serie(), quotes.slice(3, 1)?);
    ```

=== "Python"

    ```python
    from yggdryl import Field, Serie

    quotes = Serie.from_scalars(
        Field(
            "quote",
            "struct<venue: utf8 not null, side: utf8 not null, price: int64 not null>",
            nullable=False,
        ),
        [["XNYS", "B", 1], ["XNAS", "B", 2], ["XNAS", "B", 3], ["XNAS", "S", 4]],
    )

    _, xnas = quotes.window_by("venue")[1]
    record = xnas.static_values
    assert record is not None
    assert record.as_py() == {"venue": "XNAS", "windownum": 1, "rownum": 1}

    # A window of a window keeps the outer cells first, and its rownum stays absolute.
    _, sell = xnas.window_by("side")[1]
    assert (sell.offset, len(sell)) == (3, 1)
    record = sell.static_values
    assert record is not None
    assert record["side"].as_py() == "S"
    assert record.as_py() == {"venue": "XNAS", "side": "S", "windownum": 1, "rownum": 3}

    # A window of the rows alone states none, and the record is never identity.
    assert sell.window(0, 1).static_values is None
    assert quotes.window(3, 1).static_values is None
    assert sell == quotes.window(3, 1)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Serie } = require('yggdryl')

    const quotes = Serie.fromScalars(
      Field.from('quote: struct<venue: utf8 not null, side: utf8 not null, price: int64 not null> not null'),
      [['XNYS', 'B', 1n], ['XNAS', 'B', 2n], ['XNAS', 'B', 3n], ['XNAS', 'S', 4n]],
    )

    const [, xnas] = quotes.windowBy('venue')[1]
    assert.deepEqual(xnas.staticValues.asJs(), { venue: 'XNAS', windownum: 1, rownum: 1 })

    // A window of a window keeps the outer cells first, and its rownum stays absolute.
    const [, sell] = xnas.windowBy('side')[1]
    assert.deepEqual([sell.offset, sell.length], [3, 1])
    assert.equal(sell.staticValues.get('side').asJs(), 'S')
    assert.deepEqual(sell.staticValues.asJs(), { venue: 'XNAS', side: 'S', windownum: 1, rownum: 3 })

    // A window of the rows alone states none, and the record is never identity.
    assert.equal(sell.window(0, 1).staticValues, null)
    assert.equal(quotes.window(3, 1).staticValues, null)
    assert.ok(sell.equals(quotes.window(3, 1)))
    ```

## Reads

| Ask | Cost |
| --- | --- |
| `window`, `window_mut`, a narrower `window` | nothing allocated: a bounds check and two words |
| `scalar(i)`, `get(i)`, `is_null(i)`, `iter()`, `rows()` | the serie's own cost on the rebased row - a leaf value allocates nothing - plus one bounds check against the window; `rows()` lends a run's window and builds a column's rows once |
| `null_count` | constant where the column holds no absent row; one validity read per window row otherwise; a run walks its values |
| `into_serie` | `Serie::slice`: the buffers shared and one leaf boxed for a column, the values shared for a run, the start and length moved |
| `window_by` | what [`Serie::window_by`](serie.md#cost) costs over the window's rows, plus one slice per key cell the key reads where it stands - never the window as a serie of its own; windowing a window `window_by` lent copies its record's cells once a call |
| `static_values` | built on each call: one run of the cells, plus the text of a cell past what a value holds inline; `None` costs nothing, and a walk that reads no record pays nothing for it |
| `memory_size` | a column's window as its own slice counts it; a run's values as the row estimator charges them |
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

## Edges

- A window past the end is refused naming the serie and both counts - `rows 2..4 reach past the 3 rows price holds` - and so is a narrower window past the window, or a row past it: `row 2 is past the 2 rows price holds`.
- A write that would grow or shrink the window is refused by name: `a window of price never grows or shrinks what it views: 0 rows cannot replace 1`; `1 indices cannot rearrange 2 rows`. `as_unique` and `as_filtered` do not exist on a window for the same reason.
- A value the field refuses refuses `set`, `fill`, `swap`, `copy_from` and `splice` naming the field, and the serie is left as it was; a run accepts any value.
- A window over a shared column writes through `Arc::make_mut`: the column is copied once and the other holder is left alone, exactly as a `Serie` write does.
- `into_serie` shares what the window views - a column's buffers, a run's values - so the ordering reads over a window cost the serie's own verb and, for a column, one boxed leaf.
- Only a window `SerieWindows::iter` lent states a record: a narrower `window`, `Serie::window`, a `WindowSerieMut` and its `as_window` state none, and `into_serie` - and every Arrow array built from it - carries rows only.
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
