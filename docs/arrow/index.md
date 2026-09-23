# Arrow

`yggdryl::arrow` is where a Field meets Apache Arrow: one-row scalars, projected schemas, streamed batches.

## Pages

| Page | Purpose |
| --- | --- |
| [Scalars](scalars.md) | One value across the array boundary, with materialization budgets. |
| [Values](values.md) | `ArrowScalar`: one scalar, column, table, or stream, and the one entry from every columnar runtime. |
| [Readers](readers.md) | `BatchReader`, the one shape of a record read or write. |
| [Schema](schema.md) | A non-null Struct root to an Arrow `Schema` and back. |

## Contract

| | |
| --- | --- |
| Units | Exactly two: a `RecordBatch` and a one-row array; no row objects. |
| Shapes | `ArrowScalar` regroups both plus a column and a stream ([Values](values.md)). |
| Columns | A column, a table or a stream reconciled to a field is a [`Serie`](../types/serie.md), and a cast is [`Serie`'s](../types/cast.md). |
| Owns | `scalar_value` decodes the row under its Field; `Serie::from_default` lays out a field's default and `into_arrow_scalar` hands one row over. |
| `DataType` default | A bare datatype is the required `value` field it declares, so its default is the datatype's present value; never null. |
| `Field` default | `Field::default_value`, repeated: logical null when nullable; carries name, dictionary options, metadata, extension identity. |
| Struct root | The schema ([../types/field.md](../types/field.md)); its default is one row, and `into_arrow_batch` makes it a table. |
| Errors | A required `null` field has no default: `Err`, `ValueError`, or throw; message `logical-null default`. |
| Bindings | Rust `Serie::from_default(field, rows)`, then `require_arrow_array` or `into_arrow_scalar`; Python `Serie.from_default(field, rows=1)`, `into_arrow_scalar()` a `pyarrow.Scalar` (C Data Interface); JavaScript `Serie.fromDefault(field, rows)`, `intoArrowScalar()` the value (IPC). |
| Batches | Anything wider: [../holder/index.md](../holder/index.md), [../media/index.md](../media/index.md). |

## Use

Nullability picks the default: a required field answers its datatype's present value, a nullable one a logical null.

=== "Rust"

    ```rust
    use arrow_array::{Array, Datum};
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field, Scalar, Serie};

    // A default is a column of the field's canonical value; one row is one
    // Arrow scalar, and the exact Field beside it says what it means.
    let field = Field::new("symbol", DataType::utf8(), false);
    let datum = Serie::from_default(field.clone(), 1)?.into_arrow_scalar()?;
    let (array, is_scalar) = datum.get();
    assert!(is_scalar);
    assert_eq!(scalar_value(&field, array)?.as_str(), Some(""));

    // A bare DataType is the required `value` field it declares.
    let value = DataType::Int64.required_field("value");
    let array = Serie::from_default(value.clone(), 1)?.require_arrow_array()?;
    assert_eq!(scalar_value(&value, array.as_ref())?.as_i128(), Some(0));

    // A nullable Field defaults to a logical null under its own identity.
    let optional = Field::new("symbol", DataType::utf8(), true);
    let defaults = Serie::from_default(optional, 3)?;
    assert_eq!(defaults.null_count(), 3);
    assert_eq!(defaults.scalar(0)?, Scalar::Null);

    // A required Null column has no value it could ever hold.
    let refused = Serie::from_default(Field::new("never", DataType::Null, false), 1);
    assert!(refused.unwrap_err().to_string().contains("logical-null default"));
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import DataType, Field, Serie

    scalar = Serie.from_default(Field("symbol", "utf8", nullable=False)).into_arrow_scalar()
    assert isinstance(scalar, pa.Scalar)
    assert scalar.type == pa.string()
    assert scalar.as_py() == ""

    # A bare DataType is the required `value` field it declares.
    value = Field("value", DataType("int64"), nullable=False)
    assert Serie.from_default(value).into_arrow_scalar().as_py() == 0

    # A field is nullable unless you say otherwise, and its default is null.
    optional = Serie.from_default(Field("symbol", "utf8"), 3)
    assert optional.into_arrow_array().null_count == 3
    assert not optional.into_arrow_array()[0].is_valid

    try:
        Serie.from_default(Field("never", "null", nullable=False))
    except ValueError as error:
        assert "logical-null default" in str(error)
    else:
        raise AssertionError("a required null column has no default")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Serie } = require('yggdryl')

    assert.equal(Serie.fromDefault(new Field('symbol', 'utf8', false)).intoArrowScalar(), '')

    // A bare DataType is the required `value` field it declares.
    assert.equal(Serie.fromDefault(new Field('value', 'int64', false)).intoArrowScalar(), 0n)

    // A field is nullable unless you say otherwise, and its default is null.
    assert.equal(Serie.fromDefault(new Field('symbol', 'utf8')).intoArrowScalar(), null)
    assert.equal(Serie.fromDefault(new Field('symbol', 'utf8'), 3).nullCount(), 3)

    assert.throws(
      () => Serie.fromDefault(new Field('never', 'null', false)),
      /logical-null default/,
    )
    ```

## A struct root is one row

A Rust struct row is positional; Python and JavaScript key it by name. Asked for more rows, a root's default is a table.

=== "Rust"

    ```rust
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field, Scalar, Serie, StructType};

    let schema = Field::new(
        "row",
        DataType::from(StructType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])?),
        false,
    );

    let array = Serie::from_default(schema.clone(), 1)?.require_arrow_array()?;
    let row = scalar_value(&schema, array.as_ref())?;
    let values = row.as_sequence().ok_or("a struct row is an ordered sequence")?;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].as_i128(), Some(0));
    assert_eq!(values[1], Scalar::Null);

    let table = Serie::from_default(schema, 2)?.into_arrow_batch()?;
    assert_eq!((table.num_rows(), table.num_columns()), (2, 2));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field, Serie

    schema = Field(
        "row",
        DataType.from_fields(
            [Field("id", "int64", nullable=False), Field("symbol", "utf8")]
        ),
        nullable=False,
    )

    scalar = Serie.from_default(schema).into_arrow_scalar()
    assert scalar.as_py() == {"id": 0, "symbol": None}
    assert scalar["id"].as_py() == 0

    table = Serie.from_default(schema, 2).into_arrow_batch()
    assert table.to_pylist() == [{"id": 0, "symbol": None}] * 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field, Serie } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('symbol', 'utf8'),
      ]),
      false,
    )

    const scalar = Serie.fromDefault(schema).intoArrowScalar()
    assert.equal(scalar.id, 0n)
    assert.equal(scalar.symbol, null)
    assert.deepEqual(
      [...scalar],
      [
        ['id', 0n],
        ['symbol', null],
      ],
    )

    const table = Serie.fromDefault(schema, 2).intoArrowBatch()
    assert.deepEqual([table.numRows, table.numCols], [2, 2])
    ```

## Edges

- Python or JavaScript `Field` with no nullability argument -> nullable, so its default is null.
- Python registered `ExtensionType` -> rehydrates, never its storage type.
- JavaScript `int64` -> `BigInt` (`0n`).
- `from_default` with zero rows -> the empty column of the field.
- `into_arrow_scalar` on anything but one row -> refused, naming the count.
- Python only: `DataType.arrow_scalar` and `Field.arrow_scalar` build one value's `pyarrow.Scalar`; a cast into a field is a [`Serie`](../types/cast.md) door.
- JavaScript `fromArrow` -> [../types/datatype.md](../types/datatype.md), [../types/field.md](../types/field.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test root -- default::scalars
    cargo test --features "parquet iceberg" -p yggdryl --test arrow -- rows::row_values
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test__defaults.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/defaults.test.js
    npm run --prefix node bench:types:defaults
    ```
