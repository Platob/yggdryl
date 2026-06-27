# Frame (DataFrame)

**A struct column *is* a DataFrame** — its child columns *are* the frame's columns, so
`StructSerie` carries the table surface directly (there is no separate `Frame` type). Build
one with the `struct` factory (see [Nested](nested.md)), then project / filter / sort /
stack rows and read records back, with a pandas-like feel: every transform is **functional**
and returns a new lazy frame that **shares the untouched columns' Arrow buffers** (no copy),
assembling the backing `StructArray` only on demand.

See also: [Serie (the typed column)](serie.md) · [Nested](nested.md) · [Lazy & range](lazy.md).

## A full example

A frame from columns, then a small pipeline: add a column, filter rows, sort, prepend a row
index, and read the result back as native records.

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("people", [
        yggdryl.Serie("id", [3, 1, 2]),
        yggdryl.Serie("name", ["c", "a", "b"]),
        yggdryl.Serie("age", [30, 20, 40]),
    ])
    assert df.shape == (3, 3)                            # (rows, columns)
    assert df.column_names == ["id", "name", "age"]

    out = (
        df.with_column(yggdryl.Serie("adult", [True, True, True]))  # add a column
          .drop_columns(["adult"])                                 # ... and drop it again
          .filter([True, False, True])                             # keep rows 0 and 2
          .sort_by("age")                                          # ascending by age
          .with_row_index("i")                                     # prepend 0..n
    )
    assert out.column_names == ["i", "id", "name", "age"]
    assert out.to_dicts() == [
        {"i": 0, "id": 3, "name": "c", "age": 30},
        {"i": 1, "id": 2, "name": "b", "age": 40},
    ]

    # one row back as a native record / dataclass
    assert df.row(1).to_dict() == {"id": 1, "name": "a", "age": 20}
    person = df.row(1).as_dataclass("Person")
    assert (person.id, person.name) == (1, "a")

    print(df.display(max_rows=10))                       # an aligned table
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('people', [
      new Serie('id', [3, 1, 2]),
      new Serie('name', ['c', 'a', 'b']),
      new Serie('age', [30, 20, 40]),
    ])
    if (df.shape[0] !== 3 || df.shape[1] !== 3) throw new Error('shape')
    if (df.columnNames.join() !== 'id,name,age') throw new Error('columns')

    const out = df
      .withColumn(new Serie('adult', [true, true, true]))   // add a column
      .dropColumns(['adult'])                               // ... and drop it again
      .filter([true, false, true])                          // keep rows 0 and 2
      .sortBy('age')                                        // ascending by age
      .withRowIndex('i')                                    // prepend 0..n

    if (out.columnNames.join() !== 'i,id,name,age') throw new Error('pipeline')
    const rows = out.toDicts()
    if (rows[1].name !== 'b' || rows[1].age !== 40) throw new Error('sort')

    // one row back as a native record
    const rec = df.row(1).toObject()                        // { id: 1, name: 'a', age: 20 }
    if (rec.name !== 'a') throw new Error('record')

    console.log(df.display(10))                             // an aligned table
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, VarcharSerie, StructSerie, Serie, SerieRef, DisplayOptions};
    use yggdryl_scalar::Scalar;   // the `to_str()` on a record's scalar is a trait method
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(3), Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("name", vec![Some("c"), Some("a"), Some("b")]));
    let age: SerieRef = Arc::new(Int32Serie::from_values("age", vec![Some(30), Some(20), Some(40)]));
    let df = StructSerie::from_children("people", vec![id, name, age])?;

    assert_eq!(df.shape(), (3, 3));
    assert_eq!(df.column_names(), vec!["id", "name", "age"]);

    // a functional pipeline: filter -> sort -> prepend a row index
    let out = df.filter(&[true, false, true])?     // keep rows 0 and 2
        .sort_by("age", false)?                    // ascending by age
        .with_row_index("i")?;                     // prepend 0..n
    assert_eq!(out.column_names(), vec!["i", "id", "name", "age"]);

    // read a row back as a StructScalar record
    let record = df.row(1)?;
    assert_eq!(record.child_named("name").unwrap().to_str(), "'a'::utf8");

    println!("{}", df.display(&DisplayOptions::default()));
    ```

## Columns: project, add, drop, rename

Every column transform returns a **new** frame sharing the untouched columns' buffers.

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("df", [
        yggdryl.Serie("id", [3, 1, 2]),
        yggdryl.Serie("name", ["c", "a", "b"]),
    ])
    df.select_columns(["name"])                         # keep / reorder a subset
    df.with_column(yggdryl.Serie("ok", [True, True, False]))  # append or replace by name
    df.drop_columns(["name"])                           # drop (absent names are ignored)
    df.rename("id", "key")                              # rename one column
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('df', [
      new Serie('id', [3, 1, 2]),
      new Serie('name', ['c', 'a', 'b']),
    ])
    df.selectColumns(['name'])                          // keep / reorder a subset
    df.withColumn(new Serie('ok', [true, true, false])) // append or replace by name
    df.dropColumns(['name'])                            // drop (absent names are ignored)
    df.rename('id', 'key')                              // rename one column
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, VarcharSerie, StructSerie, Serie, SerieRef};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(3), Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("name", vec![Some("c"), Some("a"), Some("b")]));
    let df = StructSerie::from_children("df", vec![id, name])?;

    let _ = df.select_columns(&["name"])?;              // keep / reorder a subset
    let ok: SerieRef = Arc::new(/* bool column */ Int32Serie::from_values("ok", vec![Some(1), Some(0), Some(1)]));
    let _ = df.with_column(ok)?;                        // append or replace by name
    let _ = df.drop_columns(&["name"])?;                // drop
    let _ = df.rename("id", "key")?;                    // rename one column
    ```

## Rows: filter, sort, stack, slice

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("df", [yggdryl.Serie("id", [3, 1, 2])])
    df.filter([True, False, True])                      # keep where the mask is true
    df.sort_by("id")                                    # ascending; sort_by("id", True) for desc
    df.vstack(df)                                       # stack another frame's rows below
    df.head(2)                                          # first n rows
    df.tail(1)                                          # last n rows
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('df', [new Serie('id', [3, 1, 2])])
    df.filter([true, false, true])                      // keep where the mask is true
    df.sortBy('id')                                     // ascending; sortBy('id', true) for desc
    df.vstack(df)                                       // stack another frame's rows below
    df.head(2)                                          // first n rows
    df.tail(1)                                          // last n rows
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, StructSerie, SerieRef};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(3), Some(1), Some(2)]));
    let df = StructSerie::from_children("df", vec![id])?;

    let _ = df.filter(&[true, false, true])?;           // keep where the mask is true
    let _ = df.sort_by("id", false)?;                   // ascending (true = descending)
    let _ = df.vstack(&df)?;                            // stack another frame's rows below
    let _ = df.head(2)?;                                // first n rows
    let _ = df.slice_rows(1, 1)?;                       // a zero-copy row window
    ```

## Records: rows in, rows out

`row(i)` reads one record as a struct [`Scalar`](../scalar/scalar.md); `to_dicts` projects
the whole frame to native records.

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("df", [
        yggdryl.Serie("id", [1, 2]),
        yggdryl.Serie("name", ["a", "b"]),
    ])
    assert df.row(1).to_dict() == {"id": 2, "name": "b"}
    row = df.row(0).as_dataclass("Row")                 # a real dataclass instance
    assert (row.id, row.name) == (1, "a")
    assert df.to_dicts() == [
        {"id": 1, "name": "a"}, {"id": 2, "name": "b"},
    ]
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('df', [
      new Serie('id', [1, 2]),
      new Serie('name', ['a', 'b']),
    ])
    const rec = df.row(1).toObject()                    // { id: 2, name: 'b' }
    if (rec.name !== 'b') throw new Error('record')
    const all = df.toDicts()                            // [{id:1,name:'a'}, {id:2,name:'b'}]
    if (all.length !== 2) throw new Error('to_dicts')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, VarcharSerie, StructSerie, SerieRef};
    use yggdryl_scalar::Scalar;
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("name", vec![Some("a"), Some("b")]));
    let df = StructSerie::from_children("df", vec![id, name])?;

    let record = df.row(1)?;                             // a StructScalar
    assert_eq!(record.child_named("id").unwrap().to_str(), "2::int32");
    assert_eq!(record.child_named("name").unwrap().to_str(), "'b'::utf8");
    ```

## Schema-cast projection

`select_fields` projects **and casts** to an explicit target schema in one step: each
target [`Field`](../schema/field.md) takes the source column of the same name **cast to its
type** (or, if absent, a **filled** column — null when nullable, else the type default), in
the target order, dropping unlisted columns. The schema companion to `select_columns`
(which only reorders / projects), powered by the same `cast` struct kernel.

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("df", [yggdryl.Serie("id", [1, 2])])  # id: int64
    target = [
        yggdryl.Field("id", yggdryl.DataType("int32"), True),       # narrow
        yggdryl.Field("score", yggdryl.DataType("float64"), True),  # missing -> filled null
    ]
    out = df.select_fields(target)
    assert out.column_names == ["id", "score"]
    assert out.child("score").value_at(0) is None
    ```

=== "Node"

    ```javascript
    const { Serie, Field, DataType } = require('yggdryl')

    const df = Serie.struct('df', [new Serie('id', [1, 2])])
    const out = df.selectFields([
      new Field('id', new DataType('int32'), true),
      new Field('score', new DataType('float64'), true),  // filled with null
    ])
    if (out.child('score').valueAt(0) !== null) throw new Error('fill')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, StructSerie, NestedSerie, Serie, SerieRef, DataType, Field, Scalar};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let df = StructSerie::from_children("df", vec![id])?;
    let out = df.select_fields(vec![
        Field::new("id", DataType::int(64, true), true),     // widen
        Field::new("score", DataType::float(64), true),      // filled null
    ])?;
    assert_eq!(out.child_by_name("score").unwrap().value_at(0), Scalar::Null);
    ```

## Arrow interchange (RecordBatch / IPC / reader)

A frame round-trips through Arrow: `to_arrow_ipc()` writes an **Arrow IPC stream** (columns
as top-level fields) that any Arrow library reads back as a table, and `from_arrow_ipc(name,
bytes)` reads it in. In Rust there are also `to_record_batch` / `from_record_batch`, chunked
`to_record_batches(max_rows)` / `from_record_batches`, and a streaming
`to_record_batch_reader` / `from_record_batch_reader` (the shape Parquet readers and
scanners consume).

=== "Python"

    ```python
    import yggdryl
    import pyarrow as pa                                 # any Arrow library

    df = yggdryl.Serie.struct("df", [
        yggdryl.Serie("id", [1, 2, 3]),
        yggdryl.Serie("name", ["a", "b", "c"]),
    ])
    ipc = df.to_arrow_ipc()
    table = pa.ipc.open_stream(ipc).read_all()           # -> a pyarrow.Table
    assert table.column_names == ["id", "name"]
    back = yggdryl.Serie.from_arrow_ipc("df", ipc)        # round-trips
    assert back.to_dicts() == df.to_dicts()
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('df', [new Serie('id', [1, 2, 3])])
    const ipc = df.toArrowIpc()                          // Buffer of an Arrow IPC stream
    const back = Serie.fromArrowIpc('df', ipc)
    if (back.shape[0] !== 3) throw new Error('roundtrip')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, StructSerie, Serie, SerieRef};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2), Some(3)]));
    let df = StructSerie::from_children("df", vec![id])?;

    // one RecordBatch, or chunked batches, or an IPC stream
    let batch = df.to_record_batch()?;
    assert_eq!(batch.num_rows(), 3);
    let chunks = df.to_record_batches(2)?;               // [2 rows, 1 row]
    let reader = df.to_record_batch_reader(2)?;           // a RecordBatchReader (scanner)
    let bytes = df.to_ipc_bytes()?;
    assert_eq!(StructSerie::from_ipc_bytes("df", &bytes)?.shape(), (3, 1));
    let _ = (chunks, reader);
    ```

## Next

- [Serie (the typed column)](serie.md) — the base column, cast, display, serialize
- [Nested (struct / list / map)](nested.md) — building struct / list / map columns
- [Scalar](../scalar/scalar.md) — the single value a `row` record is built from
