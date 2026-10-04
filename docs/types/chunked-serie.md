# Chunked serie

Many [`Serie`](serie.md) columns under one [`Field`](field.md), in order and held apart: what `pyarrow` calls a `ChunkedArray`, and, under a non-null record field, a `Table` of one batch per chunk.

## Contract

| Aspect | Rule |
| --- | --- |
| What it is | `ChunkedSerie`: the rows of one field cut into chunks, each its own Arrow buffers. It sits between a [`Serie`](serie.md), one contiguous set of buffers, and a [`SerieReader`](serie.md#arrow-an-array-a-batch-a-reader), a stream read once |
| Chunks | Every chunk is a `Serie` column - never a run - whose field is exactly the collection's, each proven at its own door. A chunk of another field is cast into it on the way in |
| Invariants | The chunk ends are kept beside the chunks, so finding a row's chunk is a binary search and the length is a read. No chunk is handed out mutably, so the ends cannot go stale |
| Identity | The rows alone, as for a `Serie`: a chunked serie equals and orders as the column of the same rows, on either side of the comparison, and as any other chunking of them, and hashes alike. Neither the field nor the cut is identity |
| Field and datatype | `field()` is the one field every chunk is typed by; `dtype()` is `serie(<the field named item>)`, what [`Serie::dtype`](serie.md#reads-and-writes) answers for a column of it, so a chunked serie of no chunk names it too |
| Arrow | A chunked array is one array per chunk through `from_arrow_arrays`/`into_arrow_arrays`; a table is one batch per chunk through `from_arrow_reader`/`into_arrow_reader`. A record's chunks are its batches, their children the columns; a leaf field's chunks each cross as the one column of a `row` root, named as it is, exactly as [`Serie::into_arrow_batch`](serie.md#arrow-an-array-a-batch-a-reader) crosses one column |
| Join | `into_serie` is the one join, and it is spelled as one: no chunk is the empty column of the field, one chunk is itself, and several are concatenated once and landed with no row read |
| Cast | `cast(target, options)` is one [`ArrowCastPlan`](cast.md#compiled-plans) compiled and applied to every chunk; a chunked serie already under the target is itself, its chunks shared |
| Stream | `SerieReader::from_chunked` reads the chunks as a stream, one record column per chunk, nothing cast, copied or read |
| Order, uniqueness, partitions | the [verbs a `Serie` answers](serie.md#sorting-uniqueness-and-partitions), across the chunks: `is_sorted` reads each chunk and every chunk edge with no join, an edge compared on the rung the chunks sort on; `into_reversed`, `into_filtered`, `partition_by` and `partition_by_chunked` work chunk by chunk and keep the chunks apart; `into_sorted`, `into_sort_by` and `into_unique` sort each chunk on its own and [merge](#sorting-uniqueness-and-partitions) the sorted chunks with no join - the output cut into chunks of at most `DEFAULT_RECORD_BATCH_ROW_SIZE` rows, each settled, uniqueness filtering every chunk by its own mask and keeping it apart; `sort_indices`, `sort_indices_by`, `is_unique`, `unique_count` and `into_taken` are the one join then the verb, because their answer is one column addressing every row; `memory_size` sums the chunks; the `as_*` writes replace the chunks in place. A sorted result's field and every chunk [declare](serie.md#a-declared-order) the order, and `declared_order` reads it |
| Spill | `resident_size`, `is_spilled`, `spill(options)`, `as_spilled(options)` and `into_spilled(options)` as a [`Serie`](serie.md#spilling-to-disk) answers them, the heaviest chunks [spilled](#spilling-to-disk) whole first until the resident bytes are under the bound, and `resident_size` a read of the total kept beside the chunks; `into_serie` over several chunks, `push_chunk`, the merge's output and a join's output batches settle under the process default |
| Join | `join_with(other, by, how, options)` is [`Serie::join_with`](serie.md#joins) with the output batches kept apart as chunks, one per probe chunk answering rows, then the build side's unmatched rows where the kind keeps them |
| Windows by key | [`window_by(by, sorted)`](#windows-by-key) cuts the rows as `Serie::window_by` cuts the joined column, every window zero-copy pieces of the chunks, a run crossing an edge one window, `sorted` regrouping runs and never rows; no window states a record |
| Bindings | Rust, Python and JavaScript. Python crosses the C Data Interface and shares buffers: a `pyarrow.ChunkedArray`'s chunks and a `pyarrow.Table`'s batches are kept as chunks, and go back out as a `ChunkedArray` and a `Table`. JavaScript crosses as copied IPC: an Apache Arrow JS `Vector`'s `Data` are the chunks, and so are a `Table`'s batches. The [ordering, uniqueness and grouping verbs](#sorting-uniqueness-and-partitions) are bound in both, `partition_by` taking chunked keys where Rust spells `partition_by_chunked`. `field_ref`, `from_serie_reader` (Python reaches it through `ChunkedSerie.from_`), the `ChunkedRows` iterator type and `Hash` are Rust only: the Python class is mutable and unhashable |

One verb, three spellings, where the runtimes name the Arrow values differently:

| Rust | Python | JavaScript |
| --- | --- | --- |
| `ChunkedSerie::from_arrow_arrays(field, arrays, options)` | `ChunkedSerie.from_arrow_chunked_array(chunked, field=None, *, safe, representation)` | `ChunkedSerie.fromArrowArray(vector, field?, options?)` |
| `ChunkedSerie::from_arrow_reader(root, reader, options)` | `ChunkedSerie.from_arrow_reader(reader, root=None, *, ...)` - a `RecordBatchReader`, a `Table`, a dataset, a scanner, a frame | `ChunkedSerie.fromArrowBatch(batchOrTable, root?, options?)` for a `Table`, `ChunkedSerie.fromArrowReader(reader, root?, options?)` for a native `BatchReader` |
| `ChunkedSerie::from_series(field, chunks, options)` | `ChunkedSerie.from_series(chunks, field=None, *, ...)` | `ChunkedSerie.fromSeries(chunks, field?, options?)` |
| `ChunkedSerie::from_serie_reader(reader)` | `ChunkedSerie.from_(reader)`, which also reads every columnar runtime | none |
| `into_arrow_arrays()` | `into_arrow_chunked_array()` | `intoArrowArray()` |
| `into_arrow_reader()` | `into_arrow_reader()`, `into_arrow_table()` | `intoArrowReader()`, `intoArrowTable()` |
| `len()`, `get(i)`, `iter()` | `len(chunked)`, `get(i)`, `iter(chunked)` | `length`, `at(i)`, `[Symbol.iterator]` |
| `SerieReader::from_chunked(chunked)` | `SerieReader.from_chunked(chunked)` | `SerieReader.fromChunked(chunked)` |

## Use

A chunked serie is built from the chunks it holds, and reads across them as one column would: a row is read out of the chunk that holds it, a window keeps the chunks it reaches, and the rows are the identity however they are cut.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};

    let field = Field::new("price", DataType::Int64, false);
    let first: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    let second: ArrayRef = Arc::new(Int64Array::from(vec![127]));

    // Two arrays cross as two chunks, their buffers shared, under one plan.
    let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;
    assert_eq!((prices.len(), prices.num_chunks()), (3, 2));
    assert_eq!(prices.field(), &field);
    assert_eq!(prices.dtype(), DataType::serie(field.clone().with_name("item")));

    // A row is found by its chunk, and a window keeps the chunks it reaches.
    assert_eq!(prices.scalar(2)?, Scalar::from(127_i64));
    let window = prices.slice(1, 2)?;
    assert_eq!((window.len(), window.num_chunks()), (2, 2));

    // Joining is one verb, and the rows are the identity either way.
    let joined = prices.into_serie()?;
    assert_eq!(joined.len(), 3);
    assert!(prices == joined);
    let rows = [125_i64, 126, 127].map(Scalar::from);
    assert!(prices == Serie::from_scalars(field, rows)?);
    assert_eq!(prices.to_string(), joined.to_string());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ChunkedSerie, DataType, Field, Scalar, Serie

    field = Field("price", "int64", nullable=False)

    # Two arrays cross as two chunks, their buffers shared, under one plan.
    prices = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[125, 126], [127]]), field)
    assert (len(prices), prices.num_chunks) == (3, 2)
    assert prices.field == field
    assert prices.dtype == DataType("serie<item: int64 not null>")

    # A row is found by its chunk, and a window keeps the chunks it reaches.
    assert prices.scalar(2) == Scalar.from_(127)
    assert prices[-1] == Scalar.from_(127)
    window = prices.slice(1, 2)
    assert (len(window), window.num_chunks) == (2, 2)
    assert window.as_py() == [126, 127]

    # Joining is one verb, and the rows are the identity either way.
    joined = prices.into_serie()
    assert type(joined) is Serie and len(joined) == 3
    assert prices == joined
    assert prices == Serie.from_scalars(field, [125, 126, 127])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { ChunkedSerie, Serie, fields } = require('yggdryl')

    const field = fields.int64('price', { nullable: false })
    const int64 = (values) => arrow.vectorFromArray(values, new arrow.Int64())

    // A vector of two Data crosses as two chunks, under one plan.
    const prices = ChunkedSerie.fromArrowArray(int64([125n, 126n]).concat(int64([127n])), field)
    assert.deepEqual([prices.length, prices.numChunks], [3, 2])
    assert.ok(prices.field.equals(field))

    // A row is found by its chunk, and a window keeps the chunks it reaches.
    assert.equal(prices.scalar(2).asJs(), 127)
    const window = prices.slice(1, 2)
    assert.deepEqual([window.length, window.numChunks], [2, 2])
    assert.deepEqual(window.asJs(), [126, 127])

    // Joining is one verb, and the rows are the identity either way.
    const joined = prices.intoSerie()
    assert.ok(joined instanceof Serie)
    assert.equal(joined.length, 3)
    assert.ok(prices.equals(joined))
    assert.ok(prices.equals(Serie.fromScalars(field, [125n, 126n, 127n])))
    ```

## Chunks and rows

The chunks are columns of the one field, lent in order; a chunk may hold no row, and a row lookup steps past it. Every row verb answers across the chunks - `scalar`, `get`, `is_null`, `rows`, `iter` - and a row past the end is refused naming the field and both counts. A child of a record is the child of every chunk, so a table's column is itself a chunked serie, under the child field, a pointer bump per chunk. A chunked serie of no chunk still answers its children, from its field.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, StructType};

    let options = ArrowCastOptions::new();
    let price = Field::new("price", DataType::Int64, false);
    let column = |rows: &[i64]| Serie::from_scalars(price.clone(), rows.iter().copied().map(Scalar::from));

    // A chunk in the middle holds no row; a lookup steps past it.
    let prices = ChunkedSerie::from_series(
        Some(&price),
        [column(&[125, 126])?, column(&[])?, column(&[127])?],
        options,
    )?;
    assert_eq!((prices.len(), prices.num_chunks()), (3, 3));
    assert_eq!(prices.chunks().iter().map(Serie::len).collect::<Vec<_>>(), vec![2, 0, 1]);
    assert!(prices.chunk(3).is_none());
    assert_eq!(prices.scalar(2)?, Scalar::from(127_i64));
    assert_eq!(prices.rows(), [125_i64, 126, 127].map(Scalar::from).to_vec());
    assert_eq!(prices.iter().count(), 3);

    // A row past the end is refused naming the field and both counts.
    let refusal = prices.scalar(3).unwrap_err();
    assert_eq!(refusal.to_string(), "invalid record value at price: row 3 is past the end of 3 rows");
    assert_eq!(prices.get(3), None);

    // A table's column is the child of every batch, under the child field.
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let batch = |id: i64, symbol: &str| {
        Serie::from_scalars(root.clone(), [Scalar::from_sequence([Scalar::from(id), Scalar::from(symbol)])])
    };
    let table = ChunkedSerie::from_series(None, [batch(1, "AAPL")?, batch(2, "MSFT")?], options)?;
    let symbols = table.child("symbol").expect("a column of the table");
    assert_eq!((symbols.field().name(), symbols.num_chunks()), ("symbol", 2));
    assert_eq!(symbols.scalar(1)?, Scalar::from("MSFT"));
    assert_eq!(table.children().len(), 2);
    assert!(table.child("venue").is_none());

    // A chunked serie of no chunk answers its children from its field.
    let empty = ChunkedSerie::empty(root)?;
    let ids = empty.child("id").expect("a column of the field");
    assert_eq!((ids.field(), ids.num_chunks()), (&Field::new("id", DataType::Int64, false), 0));
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ChunkedSerie, Field, Scalar, Serie

    price = Field("price", "int64", nullable=False)

    # A chunk in the middle holds no row; a lookup steps past it.
    prices = ChunkedSerie.from_series(
        [Serie.from_scalars(price, [125, 126]), Serie.empty(price), Serie.from_scalars(price, [127])],
        price,
    )
    assert (len(prices), prices.num_chunks) == (3, 3)
    assert [len(chunk) for chunk in prices.chunks] == [2, 0, 1]
    assert prices.chunk(3) is None
    assert prices.scalar(2) == Scalar.from_(127)
    assert prices.rows() == [Scalar.from_(125), Scalar.from_(126), Scalar.from_(127)]
    assert list(prices) == prices.rows()

    # A row past the end is refused naming the field and both counts.
    try:
        prices.scalar(3)
    except ValueError as error:
        assert "row 3 is past the end of 3 rows" in str(error), error
    else:
        raise AssertionError("a row past the end is refused")
    assert prices.get(3) is None

    # A table's column is the child of every batch, under the child field.
    table = pa.Table.from_batches(
        [
            pa.record_batch({"id": [1], "symbol": ["AAPL"]}),
            pa.record_batch({"id": [2], "symbol": ["MSFT"]}),
        ]
    )
    quotes = ChunkedSerie.from_(table)
    symbols = quotes.child("symbol")
    assert (symbols.field.name, symbols.num_chunks) == ("symbol", 2)
    assert symbols.as_py() == ["AAPL", "MSFT"]
    assert [child.field.name for child in quotes.children()] == ["id", "symbol"]
    assert quotes.child("venue") is None

    # A chunked serie of no chunk answers its children from its field.
    ids = ChunkedSerie.empty(quotes.field).child("id")
    assert ids.field == Field("id", "int64") and ids.num_chunks == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { ChunkedSerie, Field, Serie, fields } = require('yggdryl')

    const price = fields.int64('price', { nullable: false })

    // A chunk in the middle holds no row; a lookup steps past it.
    const prices = ChunkedSerie.fromSeries(
      [Serie.fromScalars(price, [125n, 126n]), Serie.empty(price), Serie.fromScalars(price, [127n])],
      price,
    )
    assert.deepEqual([prices.length, prices.numChunks], [3, 3])
    assert.deepEqual(prices.chunks.map((chunk) => chunk.length), [2, 0, 1])
    assert.equal(prices.chunk(3), null)
    assert.equal(prices.scalar(2).asJs(), 127)
    assert.deepEqual([...prices].map((row) => row.asJs()), [125, 126, 127])

    // A row past the end is refused naming the field and both counts.
    assert.throws(() => prices.scalar(3), /row 3 is past the end of 3 rows/)
    assert.equal(prices.at(3), null)

    // A table's column is the child of every batch, under the child field.
    const root = Field.from('row: struct<id: int64 not null, symbol: utf8 not null> not null')
    const quotes = ChunkedSerie.fromSeries([
      Serie.fromScalars(root, [[1n, 'AAPL']]),
      Serie.fromScalars(root, [[2n, 'MSFT']]),
    ])
    const symbols = quotes.child('symbol')
    assert.deepEqual([symbols.field.name, symbols.numChunks], ['symbol', 2])
    assert.deepEqual(symbols.asJs(), ['AAPL', 'MSFT'])
    assert.deepEqual(quotes.children().map((child) => child.field.name), ['id', 'symbol'])
    assert.equal(quotes.child('venue'), null)

    // A chunked serie of no chunk answers its children from its field.
    const ids = ChunkedSerie.empty(root).child('id')
    assert.equal(ids.numChunks, 0)
    assert.ok(ids.field.equals(Field.from('id: int64 not null')))
    ```

## Sorting, uniqueness and partitions

A chunked serie answers what a [`Serie`](serie.md#sorting-uniqueness-and-partitions) answers, and says which verbs need the rows together. `is_sorted` reads each chunk and compares the last row of one with the first of the next on the rung the chunks sort on - Arrow's comparator over both chunks' buffers, or the values' own order where a chunk orders through its values - so no chunk is joined and the answer is the joined column's; `into_reversed`, `into_filtered` and `partition_by` work chunk by chunk and keep the chunks apart, a mask or a key serie as long as the whole being cut to each chunk's window. `into_sorted`, `into_sort_by` and `into_unique` join nothing either: each chunk is sorted on its own, then the sorted chunks are merged - one cursor per chunk in a heap, the least key on top, a tie to the earlier chunk, each cursor reading its chunk a block at a time, one output batch's worth shared among the cursors - into chunks of at most `DEFAULT_RECORD_BATCH_ROW_SIZE` rows, each gathered by one `interleave` and [settled](serie.md#spilling-to-disk); the rows and their order are the joined column's. At any time the merge holds the sorted chunks - spilled past the bound - one cursor per chunk and its block's key rows, the output batch being gathered and the output so far: the rows are copied twice, sorted then gathered, and never a third time, so a table larger than memory sorts under the spill bound. Uniqueness is the same merge over each chunk's sorted order, marking the first row of every run of equal values in its chunk's mask and filtering each chunk by its own: no gather, the chunks kept apart, one per contributing chunk. `sort_indices`, `sort_indices_by`, `is_unique`, `unique_count` and `into_taken` answer one column addressing every row, so they stay the one join then the verb. Keys held as a chunked serie are `partition_by_chunked`'s: cut at the same rows as the serie, each chunk is grouped by the chunk of keys beside it with no join; cut elsewhere, the keys are joined once and cut to each chunk.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, SortOptions};

    let field = Field::new("price", DataType::Int64, false);
    let first: ArrayRef = Arc::new(Int64Array::from(vec![3, 1]));
    let second: ArrayRef = Arc::new(Int64Array::from(vec![2, 3]));
    let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [first, second], ArrowCastOptions::new())?;

    // Each chunk is sorted; the edge between them is not.
    assert!(!prices.is_sorted(SortOptions::default()));
    assert!(prices.slice(2, 2)?.is_sorted(SortOptions::default()));

    // Each chunk sorted on its own, then merged: no join, the output cut by batch size.
    let sorted = prices.into_sorted(SortOptions::default())?;
    assert_eq!((sorted.num_chunks(), sorted.rows()), (1, [1_i64, 2, 3, 3].map(Scalar::from).to_vec()));
    assert_eq!(prices.unique_count(), 3);
    assert_eq!(prices.into_unique()?.len(), 3);

    // Chunk by chunk, kept apart.
    let reversed = prices.into_reversed();
    assert_eq!((reversed.num_chunks(), reversed.rows()), (2, [3_i64, 2, 1, 3].map(Scalar::from).to_vec()));
    let mask = Serie::new([true, false, true, true].map(Scalar::from).to_vec());
    assert_eq!(prices.into_filtered(&mask)?.num_chunks(), 2);
    let venues = Serie::new(["a", "b", "b", "a"].map(Scalar::from).to_vec());
    let groups = prices.partition_by(&venues)?;
    assert_eq!(groups.len(), 2);
    assert_eq!((groups[0].0.clone(), groups[0].1.num_chunks()), (Scalar::from("a"), 2));
    assert_eq!(groups[0].1.rows(), [3_i64, 3].map(Scalar::from).to_vec());

    // Keys held as chunks cut at the same rows: chunk beside chunk, no join.
    let venue = Field::new("venue", DataType::utf8(), false);
    let cuts: [ArrayRef; 2] = [
        Arc::new(arrow_array::StringArray::from(vec!["a", "b"])),
        Arc::new(arrow_array::StringArray::from(vec!["b", "a"])),
    ];
    let chunked_venues = ChunkedSerie::from_arrow_arrays(Some(&venue), cuts, ArrowCastOptions::new())?;
    let paired = prices.partition_by_chunked(&chunked_venues)?;
    assert_eq!(paired.len(), 2);
    assert_eq!(paired[0].1.rows(), groups[0].1.rows());

    // In place, chaining, and the field kept.
    let mut held = prices.clone();
    held.as_sorted(SortOptions::descending())?.as_unique()?;
    assert_eq!(held.rows(), [3_i64, 2, 1].map(Scalar::from).to_vec());
    assert_eq!(held.field(), &field);
    assert!(held.memory_size() > 0);
    ```

=== "Python"

    ```python
    import copy

    import pyarrow as pa

    from yggdryl import ChunkedSerie, Field, Serie

    field = Field("price", "int64", nullable=False)
    prices = ChunkedSerie.from_arrow_chunked_array(
        pa.chunked_array([[3, 1], [2, 3]], pa.int64()), field
    )

    # Each chunk is sorted; the edge between them is not.
    assert not prices.is_sorted()
    assert prices.slice(2, 2).is_sorted()

    # Each chunk sorted on its own, then merged: no join, the output cut by batch size.
    sorted_prices = prices.into_sorted()
    assert (sorted_prices.num_chunks, sorted_prices.as_py()) == (1, [1, 2, 3, 3])
    assert prices.unique_count() == 3
    assert len(prices.into_unique()) == 3

    # Chunk by chunk, kept apart.
    reversed_prices = prices.into_reversed()
    assert (reversed_prices.num_chunks, reversed_prices.as_py()) == (2, [3, 2, 1, 3])
    assert prices.into_filtered([True, False, True, True]).num_chunks == 2
    venues = ["a", "b", "b", "a"]
    groups = prices.partition_by(venues)
    assert len(groups) == 2
    assert (groups[0][0].as_py(), groups[0][1].num_chunks) == ("a", 2)
    assert groups[0][1].as_py() == [3, 3]

    # Keys held as chunks cut at the same rows: chunk beside chunk, no join.
    chunked_venues = ChunkedSerie.from_arrow_chunked_array(
        pa.chunked_array([["a", "b"], ["b", "a"]]), Field("venue", "utf8", nullable=False)
    )
    paired = prices.partition_by(chunked_venues)
    assert len(paired) == 2
    assert paired[0][1].as_py() == groups[0][1].as_py()

    # In place, chaining, and the field kept.
    held = copy.copy(prices)
    assert held.as_sorted(descending=True).as_unique() is held
    assert held.as_py() == [3, 2, 1]
    assert held.field == field
    assert held.memory_size() > 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { ChunkedSerie, Field } = require('yggdryl')

    const int64 = (values) => arrow.vectorFromArray(values, new arrow.Int64())
    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const prices = ChunkedSerie.fromArrowArray(
      int64([3n, 1n]).concat(int64([2n, 3n])),
      Field.from('price: int64 not null'),
    )

    // Each chunk is sorted; the edge between them is not.
    assert.equal(prices.isSorted(), false)
    assert.equal(prices.slice(2, 2).isSorted(), true)

    // Each chunk sorted on its own, then merged: no join, the output cut by batch size.
    const sorted = prices.intoSorted()
    assert.deepEqual([sorted.numChunks, sorted.asJs()], [1, [1, 2, 3, 3]])
    assert.equal(prices.uniqueCount(), 3)
    assert.equal(prices.intoUnique().length, 3)

    // Chunk by chunk, kept apart.
    const reversed = prices.intoReversed()
    assert.deepEqual([reversed.numChunks, reversed.asJs()], [2, [3, 2, 1, 3]])
    assert.equal(prices.intoFiltered([true, false, true, true]).numChunks, 2)
    const groups = prices.partitionBy(['a', 'b', 'b', 'a'])
    assert.equal(groups.length, 2)
    assert.deepEqual([groups[0][0].asJs(), groups[0][1].numChunks], ['a', 2])
    assert.deepEqual(groups[0][1].asJs(), [3, 3])

    // Keys held as chunks cut at the same rows: chunk beside chunk, no join.
    const venues = ChunkedSerie.fromArrowArray(utf8(['a', 'b']).concat(utf8(['b', 'a'])), Field.from('venue: utf8 not null'))
    const paired = prices.partitionBy(venues)
    assert.equal(paired.length, 2)
    assert.deepEqual(paired[0][1].asJs(), groups[0][1].asJs())

    // In place, chaining, and the field kept.
    const held = prices.clone()
    assert.equal(held.asSorted({ descending: true }).asUnique(), held)
    assert.deepEqual(held.asJs(), [3, 2, 1])
    assert.ok(held.memorySize() > 0)
    ```

The options and the `indices`, `mask` and `keys` arguments cross as [a `Serie`'s do](serie.md#sorting-uniqueness-and-partitions). `partition_by` also takes keys held in chunks: in Python a `ChunkedSerie`, a `pyarrow.ChunkedArray` or a table, in JavaScript a `ChunkedSerie` or an Arrow JS vector of one chunk per `Data`.

## Windows by key

`window_by(by, sorted)` cuts the rows into windows of equal keys exactly as [`Serie::window_by`](serie.md#windows-by-key) cuts the joined column, `sorted` meaning what it means there, and answers one `(key, rows)` per window, every window a chunked serie. Without `sorted` - or with keys already in order - each window is a zero-copy [`slice`](#chunks-and-rows) keeping the chunks it reaches, so a run that crosses a chunk edge is one window over both. With `sorted` and keys out of order, the runs - never the rows - are sorted stably by key and the runs of one key merged, each window the zero-copy pieces of its runs in the order they arrived: no row is copied and the chunks are never joined.

No window states a record of [static values](window-serie.md#static-values), as a held window `Serie::window_by` lends does: a window's key is the first half of its item, and its place among the windows - what a record calls `windownum` - is its place in the `Vec`. So a key cell named `windownum` or `rownum`, which `Serie::window_by` refuses because its record names both, is taken here.

The key is bound once against the field and computed chunk by chunk; at each chunk edge the pending window's key, already built, is compared in place against the next chunk's first key row, so a run continuing across the edge builds no key, and an empty chunk adds no row and no edge. The cost is one bind, then one key column, comparator and bitmap per chunk holding a row, each on the terms `Serie::window_by` states, and per window its key and its slice. A key that changes on every row answers a piece per row, so there [`into_serie`](#chunks-and-rows) then `Serie::window_by` is the cheaper door.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar};

    let field = Field::new("venue", DataType::utf8(), false);
    let chunked = |chunks: [Vec<&str>; 2]| {
        ChunkedSerie::from_arrow_arrays(
            Some(&field),
            chunks.map(|rows| Arc::new(StringArray::from(rows)) as ArrayRef),
            ArrowCastOptions::new(),
        )
    };
    let key = |venue: &str| Scalar::from_sequence([Scalar::from(venue)]);

    // A run crossing the chunk edge is one window over both chunks, zero copy.
    let venues = chunked([vec!["XNAS", "XNAS"], vec!["XNAS", "XNYS"]])?;
    let windows = venues.window_by("venue", false)?;
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[0].0, key("XNAS"));
    assert_eq!((windows[0].1.len(), windows[0].1.num_chunks()), (3, 2));
    assert_eq!(windows[1].1.rows(), vec![Scalar::from("XNYS")]);

    // Sorted over keys out of order: each key once, its runs regrouped as the
    // zero-copy pieces of the chunks they lie in - no row copied, no join.
    let mixed = chunked([vec!["XNYS", "XNAS"], vec!["XNAS", "XNYS"]])?;
    let sorted = mixed.window_by("venue", true)?;
    assert_eq!(sorted.iter().map(|(key, _)| key.clone()).collect::<Vec<_>>(), [key("XNAS"), key("XNYS")]);
    assert_eq!((sorted[0].1.len(), sorted[0].1.num_chunks()), (2, 2));
    assert_eq!(sorted[1].1.rows(), vec![Scalar::from("XNYS"); 2]);

    // No record: a key cell named rownum, which the joined column's window_by refuses, is taken.
    assert_eq!(venues.window_by("venue as rownum", false)?.len(), 2);
    assert!(venues.into_serie()?.window_by("venue as rownum", false).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import ChunkedSerie, Field

    field = Field("venue", "utf8", nullable=False)

    # A run crossing the chunk edge is one window over both chunks, zero copy.
    venues = ChunkedSerie.from_arrow_chunked_array(
        pa.chunked_array([["XNAS", "XNAS"], ["XNAS", "XNYS"]]), field
    )
    windows = venues.window_by("venue")
    assert [(key.as_py(), len(window), window.num_chunks) for key, window in windows] == [
        (["XNAS"], 3, 2),
        (["XNYS"], 1, 1),
    ]

    # Sorted over keys out of order: each key once, its runs regrouped as the
    # zero-copy pieces of the chunks they lie in - no row copied, no join.
    mixed = ChunkedSerie.from_arrow_chunked_array(
        pa.chunked_array([["XNYS", "XNAS"], ["XNAS", "XNYS"]]), field
    )
    ordered = mixed.window_by("venue", sorted=True)
    assert [(key.as_py(), window.as_py(), window.num_chunks) for key, window in ordered] == [
        (["XNAS"], ["XNAS", "XNAS"], 2),
        (["XNYS"], ["XNYS", "XNYS"], 2),
    ]

    # No record: a key cell named rownum, which the joined column's window_by refuses, is taken.
    assert len(venues.window_by("venue as rownum")) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { ChunkedSerie, Field } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const field = Field.from('venue: utf8 not null')

    // A run crossing the chunk edge is one window over both chunks, zero copy.
    const venues = ChunkedSerie.fromArrowArray(utf8(['XNAS', 'XNAS']).concat(utf8(['XNAS', 'XNYS'])), field)
    assert.deepEqual(
      venues.windowBy('venue').map(([key, window]) => [key.asJs(), window.length, window.numChunks]),
      [[['XNAS'], 3, 2], [['XNYS'], 1, 1]],
    )

    // Sorted over keys out of order: each key once, its runs regrouped as the
    // zero-copy pieces of the chunks they lie in - no row copied, no join.
    const mixed = ChunkedSerie.fromArrowArray(utf8(['XNYS', 'XNAS']).concat(utf8(['XNAS', 'XNYS'])), field)
    assert.deepEqual(
      mixed.windowBy('venue', true).map(([key, window]) => [key.asJs(), window.asJs(), window.numChunks]),
      [[['XNAS'], ['XNAS', 'XNAS'], 2], [['XNYS'], ['XNYS', 'XNYS'], 2]],
    )

    // No record: a key cell named rownum, which the joined column's windowBy refuses, is taken.
    assert.equal(venues.windowBy('venue as rownum').length, 2)
    ```

Python answers a `list` of `(Scalar, ChunkedSerie)` pairs and JavaScript an `Array` of `[Scalar, ChunkedSerie]`; `sorted` defaults to false as on a `Serie`. The refusals are [`Serie::window_by`'s](serie.md#refusals) but the one for a key cell named `windownum` or `rownum`, naming the field, before any row is read - with no chunk at all as with many.

## Arrow: a chunked array and a table

A chunked array is one layout in pieces, so every array must lay out as the first, and one plan compiled from that layout lands them all: the identity where the layout is the field's, sharing the buffers. With no field the arrays are the column of their own layout, named `item` and nullable where any array holds an absent row. A table is one chunk per batch - [`SerieReader::from_arrow_reader`](serie.md#arrow-an-array-a-batch-a-reader) collected, where `Serie::from_arrow_reader` joins the batches into one column - and goes back out as one batch per chunk under one schema built once.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array, RecordBatchReader, StringArray};
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, StructType};

    let options = ArrowCastOptions::new();

    // A chunked array: one array per chunk, of its own layout, buffers shared.
    let first: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
    let second: ArrayRef = Arc::new(Int64Array::from(vec![Some(127), None]));
    let prices = ChunkedSerie::from_arrow_arrays(None, [Arc::clone(&first), second], options)?;
    assert_eq!(prices.field(), &Field::new("item", DataType::Int64, true));
    let arrays = prices.into_arrow_arrays();
    assert_eq!(arrays.len(), 2);
    assert_eq!(arrays[0].to_data().buffers()[0].as_ptr(), first.to_data().buffers()[0].as_ptr());

    // Every array lays out as the first, and one that does not is named.
    let text: ArrayRef = Arc::new(StringArray::from(vec!["AAPL"]));
    let refusal = ChunkedSerie::from_arrow_arrays(None, [first, text], options).unwrap_err();
    assert!(refusal.to_string().contains("chunk 1"), "{refusal}");

    // No array states no layout, so a field has to.
    let refusal = ChunkedSerie::from_arrow_arrays(None, Vec::<ArrayRef>::new(), options).unwrap_err();
    assert!(refusal.to_string().contains("names no field"), "{refusal}");

    // A table: one batch per chunk out, and one chunk per batch back in.
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let batch = |id: i64, symbol: &str| {
        Serie::from_scalars(root.clone(), [Scalar::from_sequence([Scalar::from(id), Scalar::from(symbol)])])
    };
    let table = ChunkedSerie::from_series(Some(&root), [batch(1, "AAPL")?, batch(2, "MSFT")?], options)?;
    let reader = table.into_arrow_reader()?;
    assert_eq!(reader.schema().fields().len(), 2);
    let back = ChunkedSerie::from_arrow_reader(Some(&root), reader, options)?;
    assert_eq!(back.num_chunks(), 2);
    assert_eq!(back, table);

    // `Serie::from_arrow_reader` joins the same batches into one column.
    let joined = Serie::from_arrow_reader(Some(&root), table.into_arrow_reader()?, options)?;
    assert!(table == joined);

    // A leaf field's chunks are each the one column of a `row` root.
    let reader = prices.into_arrow_reader()?;
    assert_eq!(reader.schema().field(0).name(), "item");
    assert_eq!(reader.map(|batch| batch.map(|batch| batch.num_rows())).collect::<Result<Vec<_>, _>>()?, vec![2, 2]);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ChunkedSerie, Field, Serie

    # A chunked array is its chunks, and goes back out as one, buffers shared.
    source = pa.chunked_array([[125, 126], [127]])
    prices = ChunkedSerie.from_(source)
    assert (prices.num_chunks, prices.field) == (2, Field("item", "int64", nullable=False))
    back = prices.into_arrow_chunked_array()
    assert isinstance(back, pa.ChunkedArray) and back.num_chunks == 2
    assert back.equals(source)
    assert back.chunk(0).buffers()[1].address == source.chunk(0).buffers()[1].address

    # A declared field casts every chunk by one plan.
    wide = ChunkedSerie.from_arrow_chunked_array(source, Field("price", "float64", nullable=False))
    assert wide.num_chunks == 2
    assert wide.into_arrow_chunked_array().type == pa.float64()

    # A table is one chunk per batch, and goes back out with its batches kept.
    table = pa.Table.from_batches(
        [
            pa.record_batch({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}),
            pa.record_batch({"id": [3], "symbol": ["AMD"]}),
        ]
    )
    quotes = ChunkedSerie.from_(table)
    assert (quotes.num_chunks, len(quotes)) == (2, 3)
    out = quotes.into_arrow_table()
    assert isinstance(out, pa.Table) and out.equals(table)
    assert [batch.num_rows for batch in out.to_batches()] == [2, 1]
    assert isinstance(quotes.into_arrow_reader(), pa.RecordBatchReader)

    # A table's column is the child of every batch.
    assert quotes.child("symbol").into_arrow_chunked_array().equals(table.column("symbol"))

    # `Serie.from_` joins what `ChunkedSerie.from_` keeps apart.
    assert Serie.from_(source) == prices
    assert Serie.from_(prices) == prices
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { ChunkedSerie, Serie, fields } = require('yggdryl')

    // A vector's Data are the chunks, and go back out as a vector of as many.
    const int64 = (values) => arrow.vectorFromArray(values, new arrow.Int64())
    const source = int64([125n, 126n]).concat(int64([127n]))
    const prices = ChunkedSerie.fromArrowArray(source, fields.int64('price', { nullable: false }))
    assert.equal(prices.numChunks, 2)
    const back = prices.intoArrowArray()
    assert.equal(back.data.length, 2)
    assert.deepEqual([...back], [125n, 126n, 127n])

    // A table is one chunk per batch, and goes back out with its batches kept.
    const quotes = (ids, symbols) =>
      new arrow.Table({ id: int64(ids), symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()) })
    const table = new arrow.Table([
      ...quotes([1n, 2n], ['AAPL', 'MSFT']).batches,
      ...quotes([3n], ['AMD']).batches,
    ])
    const held = ChunkedSerie.fromArrowBatch(table)
    assert.deepEqual([held.numChunks, held.length], [2, 3])
    const out = held.intoArrowTable()
    assert.deepEqual(out.batches.map((batch) => batch.numRows), [2, 1])

    // A table's column is the child of every batch.
    assert.deepEqual(held.child('symbol').asJs(), ['AAPL', 'MSFT', 'AMD'])
    assert.equal(held.child('symbol').numChunks, 2)

    // Serie.fromArrowArray joins what ChunkedSerie.fromArrowArray keeps apart.
    assert.ok(prices.equals(Serie.fromArrowArray(source)))
    ```

## Streams

`SerieReader::from_chunked` reads a held chunked serie as the stream of its chunks, one record column per chunk: a record's chunks are the batches they are, and a leaf field's chunks are each the one child of a `row` record, named as it is - the rule [`SerieReader::from_serie`](serie.md#arrow-an-array-a-batch-a-reader) states, applied per chunk. Nothing is cast, copied or read, no plan is compiled, and a chunked serie of no chunk is the empty stream of its root; a stream under another root is [`SerieReader::cast`](serie.md#arrow-an-array-a-batch-a-reader), one plan over every chunk. Coming back, `from_serie_reader` collects a stream, one chunk per batch, none joined. A [write to a handle](serie.md#writing-a-serie-to-a-handle) takes a chunked serie as it is - `write_serie` and its three intents write one batch per chunk, nothing joined - and a `ChunkedSerie` answers `__arrow_c_stream__` - a record's chunks as those batches, any other field's as the column a `pyarrow.ChunkedArray` streams - so `pa.table` and `pa.chunked_array` read one directly.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, SerieReader, StructType};

    let options = ArrowCastOptions::new();
    let root = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            Field::new("id", DataType::Int64, false),
            Field::new("symbol", DataType::utf8(), false),
        ])?),
        false,
    );
    let batch = |id: i64, symbol: &str| {
        Serie::from_scalars(root.clone(), [Scalar::from_sequence([Scalar::from(id), Scalar::from(symbol)])])
    };
    let table = ChunkedSerie::from_series(Some(&root), [batch(1, "AAPL")?, batch(2, "MSFT")?], options)?;

    // One record column per chunk, the chunks themselves.
    let reader = SerieReader::from_chunked(table.clone())?;
    assert_eq!(reader.field(), &root);
    assert_eq!(reader.collect::<Result<Vec<_>, _>>()?, table.chunks().to_vec());

    // And back: a stream collected, one chunk per batch, none joined.
    let again = ChunkedSerie::from_serie_reader(SerieReader::from_chunked(table.clone())?)?;
    assert_eq!(again.num_chunks(), 2);
    assert_eq!(again, table);

    // A leaf field's chunk is the one child of a `row` record.
    let price = Field::new("price", DataType::Int64, false);
    let prices = ChunkedSerie::from_serie(Serie::from_scalars(price, [Scalar::from(125_i64)])?)?;
    let mut stream = SerieReader::from_chunked(prices)?;
    assert_eq!(stream.field().name(), "row");
    let record = stream.next().expect("one chunk")?;
    assert_eq!(record.child("price").map(Serie::len), Some(1));

    // No chunk is the empty stream of its root.
    assert_eq!(SerieReader::from_chunked(ChunkedSerie::empty(root)?)?.count(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    from yggdryl import ChunkedSerie, IOBase, SerieReader

    table = pa.Table.from_batches(
        [
            pa.record_batch({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}),
            pa.record_batch({"id": [3], "symbol": ["AMD"]}),
        ]
    )
    quotes = ChunkedSerie.from_(table)

    # One record column per chunk, the chunks themselves.
    assert [len(chunk) for chunk in SerieReader.from_chunked(quotes)] == [2, 1]
    assert SerieReader.from_(quotes).into_arrow_reader().read_all().equals(table)

    # A write takes it as it is, one batch per chunk.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "quotes.jsonl")
    handle.overwrite_serie(quotes)
    assert len(ChunkedSerie.from_(handle.read_serie())) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { ChunkedSerie, Serie, SerieReader, fields } = require('yggdryl')

    const price = fields.int64('price', { nullable: false })
    const prices = ChunkedSerie.fromSeries(
      [Serie.fromScalars(price, [125n, 126n]), Serie.fromScalars(price, [127n])],
      price,
    )

    // A leaf field's chunks are each the one child of a `row` record.
    const records = [...SerieReader.fromChunked(prices)]
    assert.deepEqual(records.map((record) => record.child('price').asJs()), [[125, 126], [127]])

    // And back: a native reader, consumed, one chunk per batch.
    const again = ChunkedSerie.fromArrowReader(prices.intoArrowReader())
    assert.equal(again.numChunks, 2)
    assert.ok(again.child('price').equals(prices))
    ```

## Casting

`cast` compiles one plan from the field to the target and applies it to every chunk; a chunked serie already under the target is itself. `push_chunk` appends a column under the field as it stands and casts any other into it, compiling one plan per call as [`Serie::cast`](cast.md) does, and a refusal leaves the chunked serie as it was. `from_series` compiles one plan per run of chunks under one source field, so a thousand chunks of one foreign layout compile once. A held [`ArrowCastPlan`](cast.md#compiled-plans) applied to a chunked column - `apply_chunked` in Rust, `apply` in Python and JavaScript, which also take a `pyarrow.ChunkedArray` or `pyarrow.Table` in Python - answers a `ChunkedSerie` of as many chunks, cast chunk by chunk under the one plan.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, ArrowCastPlan, ChunkedSerie, DataType, Field, Scalar, Serie};

    let options = ArrowCastOptions::new();
    let price = Field::new("price", DataType::Int64, false);
    let column = |rows: &[i64]| Serie::from_scalars(price.clone(), rows.iter().copied().map(Scalar::from));
    let mut prices = ChunkedSerie::from_series(Some(&price), [column(&[125, 126])?, column(&[127])?], options)?;

    // One plan, every chunk.
    let wide = Field::new("price", DataType::Float64, true);
    let cast = prices.cast(&wide, options)?;
    assert_eq!((cast.num_chunks(), cast.field()), (2, &wide));
    assert_eq!(cast.scalar(2)?, Scalar::from(127.0_f64));
    assert!(prices.cast(&price, options)? == prices);

    // A column of another field is cast in as it is appended.
    let narrow = Serie::from_scalars(Field::new("p", DataType::Int32, true), [Scalar::from(128_i32)])?;
    prices.push_chunk(narrow, options)?;
    assert_eq!((prices.num_chunks(), prices.scalar(3)?), (3, Scalar::from(128_i64)));

    // A run names no field, and a refusal changes nothing.
    assert!(prices.push_chunk(Serie::new(vec![Scalar::from(1_i64)]), options).is_err());
    assert_eq!(prices.num_chunks(), 3);

    // A held plan casts every chunk and keeps them apart.
    let plan = ArrowCastPlan::compile(&price, &wide, options)?;
    assert_eq!(plan.apply_chunked(&prices)?.num_chunks(), 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ArrowCastPlan, ChunkedSerie, DataType, Field, Serie

    price = Field("price", "int64", nullable=False)
    prices = ChunkedSerie.from_arrow_chunked_array(pa.chunked_array([[125, 126], [127]]), price)

    # One plan, every chunk; a DataType is the required column named `value`.
    wide = prices.cast(Field("price", "float64"))
    assert (wide.num_chunks, wide.as_py()) == (2, [125.0, 126.0, 127.0])
    assert prices.cast(DataType("int32")).field == Field("value", "int32", nullable=False)

    # A column of another field is cast in as it is appended.
    prices.push_chunk(Serie.from_scalars(Field("p", "int32"), [128]))
    assert (prices.num_chunks, prices.as_py()) == (3, [125, 126, 127, 128])

    # A held plan applied to a table answers a chunked serie of its batches.
    table = pa.Table.from_batches(
        [
            pa.record_batch({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}),
            pa.record_batch({"id": [3], "symbol": ["AMD"]}),
        ]
    )
    plan = ArrowCastPlan(table.schema, Field("row", "struct<id: int32, symbol: utf8>", nullable=False))
    cast = plan.apply(table)
    assert isinstance(cast, ChunkedSerie) and cast.num_chunks == 2
    assert cast.into_arrow_table().schema.field("id").type == pa.int32()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { ArrowCastPlan, ChunkedSerie, Serie, fields } = require('yggdryl')

    const price = fields.int64('price', { nullable: false })
    const prices = ChunkedSerie.fromSeries(
      [Serie.fromScalars(price, [125n, 126n]), Serie.fromScalars(price, [127n])],
      price,
    )

    // One plan, every chunk.
    const wide = prices.cast(fields.float64('price'))
    assert.equal(wide.numChunks, 2)
    assert.deepEqual(wide.asJs(), [125, 126, 127])

    // A column of another field is cast in as it is appended.
    prices.pushChunk(Serie.fromScalars(fields.int32('p'), [128]))
    assert.deepEqual([prices.numChunks, prices.scalar(3).asJs()], [3, 128])

    // A run names no field, and a refusal changes nothing.
    assert.throws(() => prices.pushChunk(new Serie([1n])), /declares no field/)
    assert.equal(prices.numChunks, 3)

    // A held plan casts every chunk and keeps them apart.
    const plan = ArrowCastPlan.compile(price, fields.float64('price'))
    assert.equal(plan.apply(prices).numChunks, 3)
    ```

## Spilling to disk

A chunked serie [spills](serie.md#spilling-to-disk) chunk by chunk: `spill(options)` moves the heaviest chunks to disk whole first, each through `Serie::spill`, reading the resident sum again after each, so a chunked serie under the bound is untouched and one over it keeps its lightest chunks resident. `resident_size` is a read - the total is kept beside the chunks and moved by every door that changes them - and `is_spilled` is "some bytes, none of them resident". `as_spilled(options)` is `spill` answering the chunked serie so calls chain, and `into_spilled(options)` a spilled copy, this one untouched, the chunks the bound leaves resident shared. A refused folder leaves the chunks spilled so far mapped and the rest as they were.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Scalar, Serie, SpillOptions};

    let field = DataType::Int64.required_field("price");
    let heavy = Serie::from_scalars(field.clone(), (0..1_024_i64).map(Scalar::from))?;
    let light = Serie::from_scalars(field.clone(), (0..8_i64).map(Scalar::from))?;
    let prices = ChunkedSerie::from_series(Some(&field), [heavy, light], ArrowCastOptions::new())?;

    // The heaviest chunk spills first; the light one stays under the bound.
    let spilled = prices.into_spilled(&SpillOptions::new().with_byte_size(1_024))?;
    assert!(spilled.chunk(0).expect("the heavy chunk").is_spilled());
    assert!(!spilled.chunk(1).expect("the light chunk").is_spilled());
    assert!(!prices.is_spilled(), "into_spilled leaves its source resident");
    assert_eq!(spilled, prices);

    // In place, chaining: a bound of zero spills every chunk.
    let mut all = prices.clone();
    assert!(all.as_spilled(&SpillOptions::new().with_byte_size(0))?.is_spilled());
    assert_eq!(all.resident_size(), 0);
    ```

=== "Python"

    ```python
    from yggdryl import ChunkedSerie, Field, Serie

    field = Field("price", "int64", nullable=False)
    heavy = Serie.from_scalars(field, range(1_024))
    light = Serie.from_scalars(field, range(8))
    prices = ChunkedSerie.from_series([heavy, light], field)

    # The heaviest chunk spills first; the light one stays under the bound.
    spilled = prices.into_spilled(byte_size=1_024)
    assert spilled.chunk(0).is_spilled() and not spilled.chunk(1).is_spilled()
    assert not prices.is_spilled()
    assert spilled == prices

    # In place, chaining: a bound of zero spills every chunk.
    assert prices.as_spilled(byte_size=0) is prices
    assert prices.is_spilled() and prices.resident_size() == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { ChunkedSerie, Field, Serie, SpillOptions } = require('yggdryl')

    const field = Field.from('price: int64 not null')
    const heavy = Serie.fromScalars(field, Array.from({ length: 1_024 }, (_, index) => index))
    const prices = ChunkedSerie.fromSeries([heavy, Serie.fromScalars(field, [0, 1, 2])], field)

    // A copy spilled; this one stays resident.
    const spilled = prices.intoSpilled(new SpillOptions({ byteSize: 0 }))
    assert.equal(spilled.isSpilled(), true)
    assert.equal(prices.isSpilled(), false)
    assert.ok(spilled.equals(prices))

    // In place, chaining.
    assert.equal(prices.asSpilled(new SpillOptions({ byteSize: 0 })), prices)
    assert.equal(prices.residentSize(), 0)
    ```

## Edges

- A run declares no field, so `from_serie`, `from_series` and `push_chunk` refuse one: `invalid record value at $: a schema-free run declares no field`.
- No chunk and no field is refused, because nothing names the layout: `a chunked serie of no chunk names no field; pass one` from `from_series`, `a chunked serie of no array names no field; pass one` from `from_arrow_arrays`. No chunk or array under a field is `empty(field)`, which refuses what `Serie::empty` refuses - a union of no member, wherever it lies, lays out no column.
- With no field, the chunks are one datatype in pieces, as a chunked array is: `from_series` takes the first chunk's field, nullable where any chunk's is, and `from_arrow_arrays` the column of the first array's layout, named `item`, nullable where any array holds an absent row - read logically, so a null value behind a valid dictionary key, a null run or a null union member is one. A chunk of that datatype under another name or nullability, or naming its serie items or mapping entries otherwise (`item`, `element`, `entries`, `key_value`: Arrow says none is part of the type), is the same buffers relabelled. A chunk of another datatype - a nested nullability included - is refused naming it, `chunk 1 of "price" is int32, and the first chunk int64; pass a field to cast into`, because nothing picks which of two datatypes the rows are. With a field, every chunk or array is cast into it, one plan per run of chunks of one source layout.
- `scalar` and `is_null` past the end are refused naming the field and both counts - `invalid record value at price: row 3 is past the end of 3 rows` - and `get` answers `None`. A window reaching past the end is refused the same way: `rows 2..4 reach past the 3 rows price holds`.
- `into_arrow_reader`, `into_arrow_table` and `SerieReader::from_chunked` refuse a record chunk holding an absent row, because a batch states no row validity: `record column "item" holds 1 absent rows, which a table cannot state`. The chunked serie itself holds such rows and reads them; only the table cannot.
- What is zero copy: a clone is a pointer bump per chunk beside two vectors, the chunks and their ends; `slice`, `child`, `child_at`, `children`, `items` and `get_child_by_path` build the same two vectors per selection - `children` per child, beside the vector holding them - slicing only the two chunks at a window's edges, and never touch a row; with no chunk, a selection reads its field off the empty column of the field, built once per call. `into_arrow_arrays` is one array handle per chunk; an identity plan shares every chunk's buffers.
- What allocates: `into_serie` concatenates the chunks once, into one new set of buffers (one chunk is itself, shared); `scalar` and `get` build one row, `rows` and `iter` every row they reach, each time - and in Python and JavaScript iteration builds every row before it hands out the first, as a `Serie`'s does; `cast` builds new buffers wherever the plan is not the identity.
- `into_serie` refuses a join Arrow's concatenation would panic on: a dictionary whose vocabularies it gathers whole - view text, fixed-width bytes or nested values, or any dictionary below a fixed-size serie or a union - past the largest key of its width, naming its path: `joining 2 chunks of "item" gathers 200 dictionary values at $, past the largest int8 key (127)`. Plain text and primitive vocabularies are merged instead, and one vocabulary shared by every chunk is never gathered twice.
- `is_sorted` builds one comparator per chunk and one per chunk edge - Arrow's over the two chunks' buffers where both order as their values, else one row per side - and nothing per row, so it agrees with the joined column's `is_sorted` on every datatype: a version, a windows-1252 text, a NaN payload or a nested absence orders the same at an edge as inside a chunk. `partition_by` and `partition_by_chunked` key their groups across the chunks by the key values, so a key's group holds one chunk per chunk that contributed. `into_filtered`, `partition_by` and `partition_by_chunked` refuse a mask or key serie of another length than the whole, naming the field: `1 keys cannot partition the 5 rows price holds`. `into_taken` names rows across the whole, so it joins first.
- `push_chunk` compiles one plan per call, like `Serie::cast`: a loop of foreign chunks goes through `from_series`, which compiles once per run of chunks under one source field.
- A field declaring an [order](serie.md#a-declared-order) has every chunk edge checked where chunks arrive - `from_series`, `from_arrow_arrays`, `cast`, `push_chunk` - and the first edge out of order is refused by its chunk: `chunk 2 of quote opens out of the order its field declares, \`venue\``. The merge's output declares the order on its field and on every chunk, and chunks already declaring at least the asked order are merged without being sorted again.
- `into_sorted` and `into_unique` refuse a merge Arrow's gather would panic on exactly as `into_serie` refuses the join - a dictionary gathered past its key width - naming the path; `into_sort_by` refuses its keys before any chunk is read, with no chunk as with many.
- `spill` leaves the chunks spilled so far mapped where its folder refuses a later one; `is_spilled` on a chunked serie of no chunk is `false`.
- The field is not identity: two chunked series of one field and the same rows cut differently are equal, and so are a chunked serie and a `Serie` of the same rows under another field or width.
- Python: `ChunkedSerie` is mutable - `push_chunk` - so it is unhashable, and `copy.copy`, `copy.deepcopy` and a pickle share or keep its chunks and its field. `ChunkedSerie.from_(reader)` takes a native `SerieReader` - `from_serie_reader`: its chunks the record columns it yields under its own root, none landed again - and drains it: iterating the reader afterwards yields nothing, and handing it over again is refused. A chunkless `pyarrow.ChunkedArray` with no field is `empty` of the field the core gives the empty array of its type. `Serie.from_` of a `pyarrow.ChunkedArray` or a `ChunkedSerie` is the joined column, `into_serie`. `into_numpy` copies, because NumPy has no null mask: it is `pyarrow`'s `ChunkedArray.to_numpy(zero_copy_only=False)`, a null becoming `nan`.
- JavaScript crosses as copied IPC, never zero copy: a `Vector` of `n` `Data` is `n` chunks, an empty `Data` included, a `Table` one chunk per batch, and each crosses as one self-contained IPC stream. Arrow JS holds no vector or table of no `Data` or batch, so a chunked serie of no chunk goes out as a `Vector` of one empty `Data`, which crosses back as one empty chunk, and a `Table` whose one batch is Arrow JS's placeholder, which it never writes and so crosses back as no chunk. `new ChunkedSerie()` is refused; a chunked serie is built through a static.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --test root chunked_serie
    cargo test -p yggdryl --test serie arrow
    cargo test -p yggdryl --test allocations chunked
    cargo test -p yggdryl --test serie spill
    cargo test -p yggdryl --test allocations -- a_chunked_window_by a_chunked_sorted_window_by
    cargo bench -p yggdryl --bench arrow -- arrow_chunked_serie
    cargo bench -p yggdryl --bench types -- serie/chunked_
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_chunked_serie.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/chunked_serie.test.js
    ```
