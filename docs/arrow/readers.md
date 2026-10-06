# Readers

`BatchReader` is the one shape of a record read or write: a schema plus an iterator of `Result<RecordBatch>`.

## Contract

| Key | Value |
| --- | --- |
| Owns | `BatchReader`, `batch_reader`, `combined`, `combined_as` |
| Shape | `Box<dyn arrow_array::RecordBatchReader + Send>`; owns what it reads from |
| Paths | Every read path returns one; `overwrite_arrow_reader`, `append_arrow_reader`, `merge_arrow_reader` consume one ([../holder/index.md#records](../holder/index.md#records)) |
| Feature flag | `parquet::read_batch_reader` needs the non-default `parquet` feature |
| Roots | `combined` merges both schemas into the root; `combined_as` casts both onto the caller's |
| Lazy | Schema before any batch; `combined` pulls no row and collects nothing |
| Cast | [`SerieReader`](../types/cast.md#eager-and-lazy)`::from_arrow_reader(root, inner, options)`: one compiled plan for the whole stream, one record `Serie` per batch; `into_arrow_reader` hands it back as a `BatchReader`, and an identity plan hands back the inner reader unwrapped |
| Cast errors | Reported at the pull of the batch that carries them; the reader is fused after one, and the source is released then |
| Windows | [`SerieReader::window_by(by, sorted)`](#windows-of-a-stream): one lazy `SerieReader` per window of equal adjacent keys, read in order, each stating its record as `static_values` |
| Bindings | Rust; Python `combined(left, right, schema=None, *, safe=True)` and `SerieReader.from_arrow_reader(reader, root=None, ...)`; JavaScript `BatchReader.combined(other, schema?, safe?)` and `SerieReader.fromArrowReader(reader, root?, options?)`; `window_by(by, sorted=False)` / `windowBy(by, sorted?)` and `static_values` / `staticValues` in both |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, StructType};

    let left_root = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("id")])?)
        .required_field("row");
    let right_root = DataType::from(StructType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");

    let left_schema = left_root.into_arrow_schema()?;
    let right_schema = right_root.into_arrow_schema()?;
    let left = yggdryl::arrow::batch_reader(
        Arc::clone(&left_schema),
        [RecordBatch::try_new(left_schema, vec![Arc::new(Int64Array::from(vec![1_i64]))])?],
    );
    let right = yggdryl::arrow::batch_reader(
        Arc::clone(&right_schema),
        [RecordBatch::try_new(
            right_schema,
            vec![
                Arc::new(Int64Array::from(vec![2_i64])),
                Arc::new(StringArray::from(vec!["XPAR"])),
            ],
        )?],
    );

    let joined = yggdryl::arrow::combined(left, right)?;
    assert_eq!(joined.schema().fields().len(), 2);
    // The left's rows carry no `venue`, so they read null for it.
    let batches: Vec<_> = joined.collect::<Result<_, _>>()?;
    assert!(batches[0].column(1).is_null(0));
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import combined

    left = pa.table({"id": [1]})
    right = pa.table({"id": [2], "venue": ["XPAR"]})

    joined = combined(left, right).read_all()
    assert joined.column_names == ["id", "venue"]
    assert joined.num_rows == 2
    # The left's row has no `venue`, so it reads null.
    assert joined.column("venue").to_pylist() == [None, "XPAR"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { BatchReader } = require('yggdryl')
    const { tableFromArrays } = require('apache-arrow')

    const left = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([1n]) }))
    const right = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([2n]) }))

    const joined = left.combined(right)
    assert.equal(joined.field.dtype.length, 1)
    assert.equal(joined.intoTable().numRows, 2)
    ```

## Casting a stream

`SerieReader` is a stream reconciled to a non-null Struct root: the plan is compiled from the
reader's schema before a batch is pulled, so a cast the two schemas alone refuse is refused by the
constructor, and each pulled batch is one record [`Serie`](../types/serie.md). With no root it is
the stream's own schema, read as the record `row`. `into_arrow_reader` is the transport face - the
same batches reconciled to the root as they are pulled and never landed - which is what a record
write takes. [Eager and lazy](../types/cast.md#eager-and-lazy) has the failure timing.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch, RecordBatchReader};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::{ArrowCastOptions, DataType, SerieReader, StructType};

    let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        false,
    )]));
    let first = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef],
    )?;
    let second = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![3])) as ArrayRef],
    )?;

    // One record column per batch, every one cast by the one plan.
    let series = SerieReader::from_arrow_reader(
        Some(&root),
        batch_reader(Arc::clone(&schema), [first.clone(), second.clone()]),
        ArrowCastOptions::new(),
    )?;
    assert_eq!(series.field(), &root);
    let mut lengths = Vec::new();
    for serie in series {
        lengths.push(serie?.len());
    }
    assert_eq!(lengths, [2, 1]);

    // The transport face states the root's schema before a batch is pulled.
    let reader = SerieReader::from_arrow_reader(
        Some(&root),
        batch_reader(schema, [first, second]),
        ArrowCastOptions::new(),
    )?
    .into_arrow_reader();
    assert_eq!(reader.schema().field(0).data_type(), &ArrowDataType::Int64);
    let mut rows = 0;
    for batch in reader {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, SerieReader

    root = Field("row", "struct<id: int64>", nullable=False)
    table = pa.table({"id": pa.array([1, 2, 3], pa.int32())})

    # One record column per batch, every one cast by the one plan.
    series = SerieReader.from_arrow_reader(table.to_reader(max_chunksize=2), root)
    assert series.field == root
    assert [serie.child("id").as_py() for serie in series] == [[1, 2], [3]]

    # The transport face is a pyarrow reader that casts as it is read.
    reader = SerieReader.from_arrow_reader(table, root).into_arrow_reader()
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.schema.field("id").type == pa.int64()
    assert reader.read_all().num_rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, SerieReader, fields } = require('yggdryl')

    const root = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const chunk = (ids) => new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int32()) })
    const source = () => new arrow.Table([...chunk([1, 2]).batches, ...chunk([3]).batches])

    // One record column per batch, every one cast by the one plan.
    const series = SerieReader.fromArrowReader(BatchReader.from(source()), root)
    assert.ok(series.field.equals(root))
    assert.deepEqual(
      [...series].map((serie) => serie.child('id').asJs()),
      [[1, 2], [3]],
    )

    // The transport face is a native BatchReader, read once.
    const reader = SerieReader.fromArrowReader(BatchReader.from(source()), root).intoArrowReader()
    assert.ok(reader instanceof BatchReader)
    assert.equal(reader.intoTable().numRows, 3)
    ```

## Windows of a stream

`SerieReader::window_by(by, sorted)` cuts a stream into windows of equal adjacent keys and answers `SerieReaderWindows`: one lazy `SerieReader` per window, in the order the windows arrive. `by` is read as [`Serie::window_by`](../types/serie.md#windows-by-key) reads it and bound once against the reader's root before any batch is pulled; the reader is consumed.

| Aspect | Rule |
| --- | --- |
| A window | An ordinary `SerieReader` of the root's rows as they stand in the stream, one piece per batch it spans: a batch a window spans whole is served as the landed batch itself, and only a batch a window opens or closes in is sliced. No window is ever held whole |
| In order | Every window is pulled through one walk the windows share, which holds at most one batch, that batch's key record and one bit per row of it. Taking the next window pulls and drops the open one's unread rows; a window read after its walk passed rows of it refuses once, naming it, then ends - so collecting the windows before reading them is loud, never a silent loss. A window dropped unread costs only the pull of its rows, and a window may outlive its walk |
| `sorted` | Each key once, in key order - ascending, absent keys last - and a stream is never reordered: it verifies that the keys arrive in that order and refuses the first window whose key orders before the one before it, naming the batch and the row, every window before it delivered whole. A stream whose keys do not arrive in order is held first - [`ChunkedSerie::from_serie_reader`](../types/chunked-serie.md#windows-by-key), then `ChunkedSerie::window_by` - to be windowed sorted |
| The record | `field()` is the root every window yields and `static_field()` the record every window states, both known before the first pull. Each window's [`static_values()`](../types/window-serie.md#static-values) is that record: where the windowed reader is itself a window, its cells but `windownum` and `rownum`; the key cells; `windownum`, the window's place from 0; and `rownum`, the number its first row has in the stream - absolute through windows of windows, and never null, since a stream is never reordered. It is the record a held window of the same rows states, field and values; `cast` keeps it, and the Arrow face, `into_arrow_reader`, drops it |
| Failures | A failure of the stream, of a cast or of a key is the item of whichever reader pulled it - the walk or a window - once; then the walk and every window end and the stream is dropped. A window the walk was skipping when it failed refuses as passed, so a partial window is never presented as complete |
| Threads | `SerieReaderWindows` is `Send + Sync` and every window `Send`: a pull takes the walk's lock, and a window of a window takes its own walk's lock before its parent's |

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, SerieReader, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::Int64.required_field("price"),
    ])?)
    .required_field("quote");
    let quote = |venue: &str, price: i64| Scalar::from_sequence([Scalar::from(venue), Scalar::from(price)]);
    // Two batches, XNAS spanning the edge between them.
    let batches = [
        Serie::from_scalars(root.clone(), [quote("XNAS", 1), quote("XNAS", 2)])?,
        Serie::from_scalars(root.clone(), [quote("XNAS", 3), quote("XNYS", 4)])?,
    ];
    let stream = || -> yggdryl::arrow::Result<SerieReader> {
        SerieReader::from_chunked(ChunkedSerie::from_series(Some(&root), batches.clone(), ArrowCastOptions::new())?)
    };

    // One lazy reader per window, in the order the keys arrive; both records known before a pull.
    let mut windows = stream()?.window_by("venue", false)?;
    assert_eq!(windows.field(), &root);
    let names: Vec<&str> = windows.static_field().fields().iter().map(Field::name).collect();
    assert_eq!(names, ["venue", "windownum", "rownum"]);

    // Read each window before taking the next: one piece per batch it spans.
    let xnas = windows.next().expect("a window")?;
    let record = xnas.static_values().expect("a window states its record");
    assert_eq!(record.get_key_str("venue"), Some(&Scalar::from("XNAS")));
    let pieces: Vec<usize> = xnas.map(|piece| piece.map(|piece| piece.len())).collect::<Result<_, _>>()?;
    assert_eq!(pieces, [2, 1]);
    let xnys = windows.next().expect("a window")?;
    let record = xnys.static_values().expect("its record");
    assert_eq!(record.get_key_str("windownum"), Some(&Scalar::from(1_u64)));
    assert_eq!(record.get_key_str("rownum"), Some(&Scalar::from(3_u64)));
    assert!(windows.next().is_none());

    // A window read after its walk passed it is refused once, naming it, then ends.
    let collected: Vec<SerieReader> = stream()?.window_by("venue", false)?.collect::<Result<_, _>>()?;
    let mut first = collected.into_iter().next().expect("a window");
    let refused = first.next().expect("one refusal").unwrap_err();
    assert!(refused.to_string().contains("window 0 was passed by its walk"));
    assert!(first.next().is_none());

    // Sorted verifies the keys arrive in order, refusing the first that goes backwards.
    let backwards = Serie::from_scalars(root.clone(), [quote("XNYS", 1), quote("XNAS", 2)])?;
    let mut sorted = SerieReader::from_serie(backwards)?.window_by("venue", true)?;
    let xnys = sorted.next().expect("a window")?;
    assert_eq!(xnys.map(|piece| piece.map(|piece| piece.len())).sum::<Result<usize, _>>()?, 1);
    let refused = sorted.next().expect("the refusal").unwrap_err();
    assert!(refused.to_string().contains("batch 0 row 1"));
    assert!(sorted.next().is_none());
    ```

=== "Python"

    ```python
    from yggdryl import ChunkedSerie, Field, Serie, SerieReader

    root = Field("quote", "struct<venue: utf8 not null, price: int64 not null>", nullable=False)


    def stream() -> SerieReader:
        # Two batches, XNAS spanning the edge between them.
        batches = [[["XNAS", 1], ["XNAS", 2]], [["XNAS", 3], ["XNYS", 4]]]
        return SerieReader.from_chunked(
            ChunkedSerie.from_series([Serie.from_scalars(root, rows) for rows in batches], root)
        )


    # One lazy reader per window, in the order the keys arrive; both records known before a pull.
    windows = stream().window_by("venue")
    assert windows.field == root
    assert [child.name for child in windows.static_field] == ["venue", "windownum", "rownum"]

    # Read each window before taking the next: one piece per batch it spans.
    xnas = next(windows)
    assert xnas.static_values is not None
    assert xnas.static_values.as_py() == {"venue": "XNAS", "windownum": 0, "rownum": 0}
    assert [len(piece) for piece in xnas] == [2, 1]
    xnys = next(windows)
    assert xnys.static_values is not None
    assert xnys.static_values["rownum"].as_py() == 3
    assert next(windows, None) is None

    # A window read after its walk passed it is refused once, naming it.
    first, _ = list(stream().window_by("venue"))
    try:
        next(first)
    except ValueError as error:
        assert "window 0 was passed by its walk" in str(error)
    else:
        raise AssertionError("collected windows are read in order or refused")

    # Sorted verifies the keys arrive in order, refusing the first that goes backwards.
    backwards = Serie.from_scalars(root, [["XNYS", 1], ["XNAS", 2]])
    walk = SerieReader.from_serie(backwards).window_by("venue", sorted=True)
    assert sum(len(piece) for piece in next(walk)) == 1
    try:
        next(walk)
    except ValueError as error:
        assert "batch 0 row 1" in str(error)
    else:
        raise AssertionError("a key going backwards is refused")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { ChunkedSerie, Field, Serie, SerieReader } = require('yggdryl')

    const root = Field.from('quote: struct<venue: utf8 not null, price: int64 not null> not null')
    // Two batches, XNAS spanning the edge between them.
    const stream = () =>
      SerieReader.fromChunked(
        ChunkedSerie.fromSeries(
          [
            Serie.fromScalars(root, [['XNAS', 1n], ['XNAS', 2n]]),
            Serie.fromScalars(root, [['XNAS', 3n], ['XNYS', 4n]]),
          ],
          root,
        ),
      )

    // One lazy reader per window, in the order the keys arrive; both records known before a pull.
    const windows = stream().windowBy('venue')
    assert.ok(windows.field.equals(root))
    assert.equal(windows.staticField.name, 'quote')

    // Read each window before taking the next: one piece per batch it spans.
    const xnas = windows.next().value
    assert.deepEqual(xnas.staticValues.asJs(), { venue: 'XNAS', windownum: 0, rownum: 0 })
    assert.deepEqual([...xnas].map((piece) => piece.length), [2, 1])
    const xnys = windows.next().value
    assert.equal(xnys.staticValues.get('rownum').asJs(), 3)
    assert.equal(windows.next().done, true)

    // A window read after its walk passed it is refused once, naming it.
    const [first] = [...stream().windowBy('venue')]
    assert.throws(() => [...first], /window 0 was passed by its walk/)

    // Sorted verifies the keys arrive in order, refusing the first that goes backwards.
    const backwards = Serie.fromScalars(root, [['XNYS', 1n], ['XNAS', 2n]])
    const walk = SerieReader.fromSerie(backwards).windowBy('venue', true)
    assert.equal([...walk.next().value].length, 1)
    assert.throws(() => walk.next(), /batch 0 row 1/)
    ```

`sorted` is `False` by default in Python, where `None` clears to it, and absent or `null` is `false` in JavaScript. Python's `SerieReaderWindows` is an iterator - every pull runs off the GIL - and JavaScript's an iterable that is its own iterator, `next()` answering one window; both carry `field` and `static_field` / `staticField`, and a window's record is a struct `Scalar` read by name.

## Partitions of a stream

`SerieReader::partition_by(by, options)` cuts a stream by a key into partitions and answers `SerieReaderPartitions`: each partition yielded as soon as it closes, as a `SeriePartition` - its key, the record of the cells `by` computes, and its rows, a [`ChunkedSerie`](../types/chunked-serie.md) under the reader's root in the order they arrived. `by` is read as [`window_by`](#windows-of-a-stream) reads it and bound once against the root before any batch is pulled; the reader is consumed. It is the one partitioner every split of a stream by partition runs through: an [Iceberg table's write](../media/iceberg.md#write) and a [partitioned folder's](../holder/index.md#partitions) are this cut.

| Aspect | Rule |
| --- | --- |
| A batch | Landed, keyed and cut on [`PartitionOptions::threads`](#partitions-of-a-stream) threads, one batch per thread and answered in the order the batches arrive - a run of a key one zero-copy slice, any other key one take. The pieces are pushed to their partitions on the pulling thread in that order, so a partition's rows are the order they arrived in whatever thread cut them |
| Held | Every open partition is a `ChunkedSerie` held under the [process spill bound](../types/serie.md#spilling-to-disk): the open partitions are settled after each batch, heaviest first. A closed partition's rows are the caller's and no longer counted |
| `max_open` | Past it, the open partitions of the lowest keys close. A stream in key order closes each partition once it was read whole, so no partition is held longer than it takes to read it. Unset, every partition stays open until the stream ends |
| `clustered` | Every row of a key arrives before any row of the next, as a stream sorted on the key's terms in either direction is: each partition closes as soon as another key arrives, so one is open at a time and the partitions close in the order they arrived. A reader whose root [declares an order](../types/serie.md#a-declared-order) - proven as its batches land - leading with the terms `by` projects, uncast and in any order, is clustered without being told |
| A key again | A key arriving again after its partition closed opens a new piece of it, yielded again under the same key: a stream that is not what the options say costs pieces, never rows |
| The end | Every partition still open closes in ascending key order |
| Failures | The stream's own failure, or a spill that cannot be written, is the next item, once; then the partitions end. A key that does not bind is refused before any batch is pulled, naming the root |

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, PartitionOptions, Scalar, Serie, SerieReader, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("venue"),
        DataType::Int64.required_field("price"),
    ])?)
    .required_field("quote");
    let quote = |venue: &str, price: i64| Scalar::from_sequence([Scalar::from(venue), Scalar::from(price)]);
    let batches = [
        Serie::from_scalars(root.clone(), [quote("XPAR", 1), quote("XNAS", 2)])?,
        Serie::from_scalars(root.clone(), [quote("XLON", 3), quote("XPAR", 4)])?,
    ];
    let stream = || -> yggdryl::arrow::Result<SerieReader> {
        SerieReader::from_chunked(ChunkedSerie::from_series(Some(&root), batches.clone(), ArrowCastOptions::new())?)
    };
    let closed = |stream: SerieReader, options: PartitionOptions| -> yggdryl::arrow::Result<Vec<(Scalar, usize)>> {
        stream
            .partition_by("venue", options)?
            .map(|partition| partition.map(|partition| (partition.key().clone(), partition.rows().len())))
            .collect()
    };
    let venue = |name: &str| Scalar::from_sequence([Scalar::from(name)]);

    // Every partition open until the stream ends, then closed in key order.
    let all = closed(stream()?, PartitionOptions::new())?;
    assert_eq!(all, [(venue("XLON"), 1), (venue("XNAS"), 1), (venue("XPAR"), 2)]);

    // One open at a time: the lowest closes while the stream still arrives.
    let bounded = closed(stream()?, PartitionOptions::new().with_max_open(1))?;
    assert_eq!(bounded, [(venue("XNAS"), 1), (venue("XLON"), 1), (venue("XPAR"), 2)]);

    // Clustered: each closes once another key arrives; XPAR returning is a second piece.
    let clustered = closed(stream()?, PartitionOptions::new().with_clustered(true))?;
    assert_eq!(
        clustered,
        [(venue("XPAR"), 1), (venue("XNAS"), 1), (venue("XLON"), 1), (venue("XPAR"), 1)]
    );

    // A root declaring an order that leads with the key is clustered untold.
    let rows = Serie::from_scalars(root.clone(), [quote("XLON", 3), quote("XPAR", 1), quote("XNAS", 2)])?;
    let sorted = rows.into_sort_by("venue desc")?;
    let declared = closed(SerieReader::from_serie(sorted)?, PartitionOptions::new())?;
    assert_eq!(declared, [(venue("XPAR"), 1), (venue("XNAS"), 1), (venue("XLON"), 1)]);
    ```

=== "Python"

    ```python
    from yggdryl import ChunkedSerie, Field, Serie, SerieReader

    root = Field("quote", "struct<venue: utf8 not null, price: int64 not null>", nullable=False)


    def stream() -> SerieReader:
        batches = [[["XPAR", 1], ["XNAS", 2]], [["XLON", 3], ["XPAR", 4]]]
        return SerieReader.from_chunked(
            ChunkedSerie.from_series([Serie.from_scalars(root, rows) for rows in batches], root)
        )


    def closed(reader: SerieReader, **options: object) -> list[tuple[object, int]]:
        return [(key.as_py(), len(rows)) for key, rows in reader.partition_by("venue", **options)]


    # Every partition open until the stream ends, then closed in key order.
    assert closed(stream()) == [(["XLON"], 1), (["XNAS"], 1), (["XPAR"], 2)]
    # One open at a time: the lowest closes while the stream still arrives.
    assert closed(stream(), max_open=1) == [(["XNAS"], 1), (["XLON"], 1), (["XPAR"], 2)]
    # Clustered: each closes once another key arrives; XPAR returning is a second piece.
    assert closed(stream(), clustered=True) == [(["XPAR"], 1), (["XNAS"], 1), (["XLON"], 1), (["XPAR"], 1)]

    # A root declaring an order that leads with the key is clustered untold.
    rows = Serie.from_scalars(root, [["XLON", 3], ["XPAR", 1], ["XNAS", 2]])
    declared = closed(SerieReader.from_serie(rows.into_sort_by("venue desc")))
    assert declared == [(["XPAR"], 1), (["XNAS"], 1), (["XLON"], 1)]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { ChunkedSerie, Field, Serie, SerieReader } = require('yggdryl')

    const root = Field.from('quote: struct<venue: utf8 not null, price: int64 not null> not null')
    const stream = () =>
      SerieReader.fromChunked(
        ChunkedSerie.fromSeries(
          [
            Serie.fromScalars(root, [['XPAR', 1n], ['XNAS', 2n]]),
            Serie.fromScalars(root, [['XLON', 3n], ['XPAR', 4n]]),
          ],
          root,
        ),
      )
    const closed = (reader, options) =>
      [...reader.partitionBy('venue', options)].map(([key, rows]) => [key.asJs(), rows.length])

    // Every partition open until the stream ends, then closed in key order.
    assert.deepEqual(closed(stream()), [[['XLON'], 1], [['XNAS'], 1], [['XPAR'], 2]])
    // One open at a time: the lowest closes while the stream still arrives.
    assert.deepEqual(closed(stream(), { maxOpen: 1 }), [[['XNAS'], 1], [['XLON'], 1], [['XPAR'], 2]])
    // Clustered: each closes once another key arrives; XPAR returning is a second piece.
    assert.deepEqual(closed(stream(), { clustered: true }), [
      [['XPAR'], 1],
      [['XNAS'], 1],
      [['XLON'], 1],
      [['XPAR'], 1],
    ])

    // A root declaring an order that leads with the key is clustered untold.
    const rows = Serie.fromScalars(root, [['XLON', 3n], ['XPAR', 1n], ['XNAS', 2n]])
    const declared = closed(SerieReader.fromSerie(rows.intoSortBy('venue desc')))
    assert.deepEqual(declared, [[['XPAR'], 1], [['XNAS'], 1], [['XLON'], 1]])
    ```

## Merge rules

| Rule | Behavior |
| --- | --- |
| Column identity | Name, ASCII case-insensitive; left's order, then right-only columns in right's order |
| Shared column | One datatype, never widened |
| One-sided column | Nullable, even when non-nullable on its side; the other side reads null |
| Metadata, field ids | Left's kept |
| Root | Left's name; bounded non-nullable Struct; appendable to Iceberg wherever both inputs were |

## Streaming batches

`arrow::batch_reader(schema, batches)` takes any `IntoIterator` of owned batches, so a generator encodes as it produces. Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use yggdryl::holder::Buffer;
use yggdryl::StructType;
use yggdryl::ipc::{self, IpcOptions};
use yggdryl::DataType;

let projected = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row")
    .into_arrow_schema()?;
let batch = |ids: Vec<i64>| {
    RecordBatch::try_new(Arc::clone(&projected), vec![Arc::new(Int64Array::from(ids))])
};

// Two batches, written as the iterator yields them.
let mut handle = Buffer::new();
let options = IpcOptions::new();
ipc::overwrite_arrow_reader(
    &mut handle,
    yggdryl::arrow::batch_reader(Arc::clone(&projected), [batch(vec![1, 2])?, batch(vec![3])?]),
    &options,
)?;

// A BatchReader knows its schema before it yields anything.
let reader = ipc::read_batch_reader(&handle, None, &options)?;
assert_eq!(reader.schema().as_ref(), projected.as_ref());

let mut rows = 0;
for batch in reader {
    rows += batch?.num_rows();
}
assert_eq!(rows, 3);
```

## Edges

- Shared column with two datatypes or two `PARQUET:field_id` values -> `combined` refuses, naming both sides.
- Root not a bounded, non-nullable Struct -> `combined_as` and `SerieReader::from_arrow_reader` return `Err`.
- A cast the two schemas alone refuse - an unsupported conversion, an ambiguous name, a [required column](../types/cast.md#required-columns) the source does not carry -> `SerieReader::from_arrow_reader` returns `Err` rather than a reader that fails on its first batch.
- A batch the plan refuses -> reported at the pull that reads it, and the reader is fused after it.
- Dropping a `SerieReader` or its transport face before it is drained -> the source is dropped with it, so a C stream behind it is released there.
- A window read after its walk passed rows of it -> refused once, `window 0 was passed by its walk with rows unread; read each window before taking the next`, then ended; read each window before taking the next.
- `window_by(.., sorted = true)` over keys arriving out of order -> the first window keyed backwards is refused naming the stream row, the batch, the row and both keys - `window by expects keys in order, ascending with absent keys last: batch 0 row 1 ...` - and the walk ends; window it unsorted, or hold it in a `ChunkedSerie` and window that sorted.
- `window_by`'s key is refused before any batch is pulled: text that is not a selector, a key stating no projection, an `unnest`, a term reaching no column, and a key cell folding onto `windownum`, `rownum` or a cell the windowed reader's record keeps, naming both - alias it. In Python, text that does not parse leaves the reader usable and any other refusal spends it, as a refused `cast` does.
- Python batch export caches the exact schema before any pull, retains [nested Map flags and shared buffers](../types/serie.md#exact-map-schemas), and releases the native reader on exhaustion or failure. A batch with no columns still retains its row count.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test arrow -- mod_
    cargo test --features "iceberg internals parquet" -p yggdryl --test arrow -- rows::row_values rows::widening
    cargo test --features "parquet iceberg" -p yggdryl --test root -- cast::coverage
    cargo test --features "parquet iceberg" -p yggdryl --test root -- cast::plans
    cargo test --features "parquet iceberg" -p yggdryl --test serie -- arrow::
    cargo test -p yggdryl --test allocations -- a_windowed_stream a_continuing_window_edge
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_cast.py -k reader
    python/.venv/bin/python -m pytest python/tests/test_serie.py -k TestReaderWindowBy
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/serie.test.js
    ```
