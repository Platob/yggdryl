# Values

One Arrow-backed value across all four shapes Arrow spells a payload in.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ArrowValue`, `ArrowShape`, `IOMedia::read_arrow_value`, `IOMedia::write_arrow_value` |
| Shapes | `scalar` one pinned row · `array` one column · `batch` one held table · `stream` a one-shot reader |
| Pairs | The exact `Field`: one element for a scalar or a column, the non-null Struct root for a table or a stream |
| Funnel | `into_reader` — every shape becomes the [`BatchReader`](readers.md) a record read or write already takes |
| Crossings | The one scalar/array boundary ([Scalars](scalars.md)); never JSON, never a second conversion |
| Stream length | `row_size()` is `None` until drained; counting a stream is deciding to read it |
| Media | Record encodings stream; JSON, JSON Lines, YAML, and TOML documents read as one batch |
| `TypedScalar` | No bridge: it owns no `Field`, and this owns one ([Scalars](scalars.md)) |
| Bindings | Rust; Python `yggdryl.ArrowValue`; JavaScript none |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array};
    use yggdryl::{ArrowShape, ArrowValue, DataType, Field};

    let field = Field::new("price", DataType::Int64, false);
    let column: ArrayRef = Arc::new(Int64Array::from(vec![125_i64, 126, 127]));
    let value = ArrowValue::from_array(field, column)?;

    assert_eq!(value.shape(), ArrowShape::Array);
    assert_eq!(value.row_size(), Some(3));

    // Whatever the shape, one funnel answers the record surface.
    let rows: usize = value
        .into_reader()?
        .map(|batch| batch.expect("a batch reads").num_rows())
        .sum();
    assert_eq!(rows, 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ArrowValue

    value = ArrowValue.from_py(pa.array([125, 126, 127]), "price: int64 not null")

    assert value.shape == "array"
    assert value.row_size == 3
    assert value.into_arrow_array().to_pylist() == [125, 126, 127]
    ```

## The shape is what the payload already is

A held container keeps the length it knows; a stream keeps its laziness.

=== "Rust"

    ```rust
    use yggdryl::{ArrowShape, ArrowValue, DataType, Field, Scalar};

    let root = DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])?
    .required_field("row");

    let rows = Scalar::from_sequence([
        Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
        Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
    ]);
    let held = ArrowValue::from_rows(&root, &rows)?;

    assert_eq!(held.shape(), ArrowShape::Batch);
    assert_eq!(held.row_size(), Some(2));
    assert_eq!(held.column_size(), 2);

    // A stream states its schema before its first batch, and nothing else.
    let schema = root.clone().into_arrow_schema()?;
    let streamed = ArrowValue::from_reader(yggdryl::arrow::batch_reader(
        schema,
        [held.into_batch()?],
    ))?;
    assert_eq!(streamed.shape(), ArrowShape::Stream);
    assert_eq!(streamed.row_size(), None);
    assert_eq!(streamed.column_size(), 2);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ArrowValue

    table = pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})

    # A batch is held, so it knows its length.
    held = ArrowValue.from_py(table.to_batches()[0])
    assert (held.shape, held.row_size, held.column_size) == ("batch", 2, 2)

    # A table may hold many chunks, so it crosses as a stream over the C stream.
    assert ArrowValue.from_py(table).row_size is None
    ```

## One entry point from every columnar runtime

Python reads `PyArrow`, pandas, polars, NumPy, and any Arrow C data or stream
exporter through one call, and the declared `Field` casts the result in Rust.

=== "Python"

    ```python
    import numpy as np
    import pyarrow as pa
    from yggdryl import ArrowValue

    table = pa.table({"symbol": ["AAPL", "MSFT"], "size": [100, 250]})

    assert ArrowValue.from_py(table.to_pandas()).shape == "stream"
    assert ArrowValue.from_py(pa.chunked_array([[1, 2], [3]])).row_size == 3
    assert ArrowValue.from_py(np.array([1.5, 2.5])).shape == "array"
    assert ArrowValue.from_py(pa.scalar(7, pa.int64())).shape == "scalar"

    # A record dtype names its members, so it is rows.
    records = np.array([("AAPL", 100)], dtype=[("symbol", "U4"), ("size", "i8")])
    assert ArrowValue.from_py(records).shape == "batch"

    # The declared Field is applied by the core's one recursive cast.
    prices = ArrowValue.from_py(pa.array([1, 2, 3]), "price: float64 not null")
    assert prices.into_arrow_array().type == pa.float64()
    ```

A held shape shares its buffers back, so it stays readable; a stream crosses once.

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import ArrowValue

    table = pa.table({"size": [100, 250]})

    held = ArrowValue.from_py(table.to_batches()[0])
    assert held.into_arrow_batch().num_rows == 2
    assert held.into_pandas().shape == (2, 1)
    assert not held.is_consumed

    streamed = ArrowValue.from_py(table)
    assert streamed.into_arrow_table().num_rows == 2
    assert streamed.is_consumed
    try:
        streamed.into_arrow_table()
    except ValueError as error:
        assert "crosses once" in str(error)
    else:
        raise AssertionError("a stream is one-shot")
    ```

## A handle reads and writes it whatever it holds

`read_arrow_value` is the Arrow-shaped sibling of `read_scalar`: a record
encoding answers its batch stream, a structured text document the batch its
rows parse into.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{ArrowShape, ArrowValue, DataType, IOBase, IOMedia, IOMode, Scalar, Url};

    let root = DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])?
    .required_field("row");

    let rows = Scalar::from_sequence([Scalar::from_sequence([
        Scalar::from("AAPL"),
        Scalar::from(100_i64),
    ])]);

    let mut handle = Buffer::new().with_media_type(Url::from_str("file:///quotes.jsonl")?.media_type());
    handle.write_arrow_value(ArrowValue::from_rows(&root, &rows)?, IOMode::Overwrite)?;

    // Rows carry the names their Field declares, one document per row.
    let text = String::from_utf8(handle.read_all_bytes()?)?;
    assert!(text.contains(r#""symbol":"AAPL""#), "{text}");

    let read = handle.read_arrow_value(Some(&root))?;
    assert_eq!(read.shape(), ArrowShape::Batch);
    assert_eq!(read.into_scalar()?, rows);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "quotes.jsonl")
    handle.write_arrow_value(pa.table({"symbol": ["AAPL"], "size": [100]}))

    assert b'"symbol":"AAPL"' in handle.read_bytes()
    assert handle.read_arrow_value().row_size == 1
    ```

## Edges

- `from_scalar_array` with a length other than 1 -> `Error::IncompatibleSchema`.
- `from_array` whose Arrow datatype is not the Field's -> `Error::IncompatibleSchema` naming both.
- `from_batch_as` / `from_reader_as` whose schema is not exactly the root's projection -> `Error::SchemaMismatch` carrying the diff.
- `into_arrow_scalar` on anything but one row -> `Error::IncompatibleSchema` naming the shape.
- `into_batch` on a stream drains and concatenates it: memory is the whole result.
- A Struct column holding a null row is not rows: a batch has no row validity -> `Error::IncompatibleSchema`.
- A structured text document is one frame around its rows, so `write_arrow_value` takes only `IOMode::Overwrite`; append and merge go through `write_arrow_reader`.
- A structured text read holds the parsed document, so it answers `batch`, never `stream`. JSON writes one array, TOML one array of tables under the root's name, and JSON Lines and YAML stream one batch of rows at a time.
- A media type that names neither a record encoding this build implements nor a structured text format -> `Error::InvalidRecord` naming it.
- Python: only a `stream` is one-shot; reading a consumed one -> `ValueError`. A NumPy array of more than one dimension -> `TypeError`.
- JavaScript: Rust and Python only. A stream has no honest copied-IPC representation.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib arrow::scalars
    cargo test --features "parquet iceberg" -p yggdryl --lib media::structured
    cargo test --features "parquet iceberg" -p yggdryl --test arrow arrow_value::
    cargo bench --bench arrow --features "parquet iceberg"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/arrow
    python/.venv/bin/python python/benchmarks/arrow.py --iterations 10000
    ```
