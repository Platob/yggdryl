# Arrow

`yggdryl::arrow` is where a Field meets Apache Arrow: one-row scalars, projected schemas, streamed batches.

## Pages

| Page | Purpose |
| --- | --- |
| [Scalars](scalars.md) | One value across the array boundary, with materialization budgets. |
| [Readers](readers.md) | `BatchReader`, the one shape of a record read or write. |
| [Schema](schema.md) | A non-null Struct root to an Arrow `Schema` and back. |

## Contract

| | |
| --- | --- |
| Units | Exactly two: a `RecordBatch` and a one-row array; no row objects. |
| Owns | `default_arrow_array` on `DataType` and `Field`; `scalar_value` decodes the row under its Field. |
| `DataType` default | The datatype's present value through a synthetic required Field; never null. |
| `Field` default | Logical null when nullable; carries name, dictionary options, metadata, extension identity. |
| Struct root | The schema ([../types/field.md](../types/field.md)); its default is one row. |
| Errors | Required `Null` field: `Err`, `ValueError`, or throw; message `logical-null default`. |
| Bindings | Rust `ArrayRef`; Python `pyarrow.Scalar` (C Data Interface); JavaScript the value (IPC). |
| Batches | Anything wider: [../holder/index.md](../holder/index.md), [../media/index.md](../media/index.md). |

## Use

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field};

    let field = Field::new("symbol", DataType::Utf8, false);
    let array = field.default_arrow_array()?;

    // A scalar is one Arrow row; the exact Field beside it says what it means.
    assert_eq!(array.len(), 1);
    assert_eq!(scalar_value(&field, array.as_ref())?.as_str(), Some(""));
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    field = Field("symbol", "utf8", nullable=False)
    scalar = field.default_arrow_scalar()

    assert isinstance(scalar, pa.Scalar)
    assert scalar.type == pa.string()
    assert scalar.as_py() == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field } = require('yggdryl')

    const field = new Field('symbol', 'utf8', false)
    assert.equal(field.defaultArrowScalar(), '')
    ```

## Nullability picks the default

Only the `Field` method can return a logical null.

=== "Rust"

    ```rust
    use arrow_array::Array;
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field, Scalar};

    // A bare DataType projects through its own datatype default planner.
    let array = DataType::Int64.default_arrow_array()?;
    let value = Field::new("value", DataType::Int64, false);
    assert_eq!(scalar_value(&value, array.as_ref())?.as_i128(), Some(0));

    // A nullable Field defaults to a logical null under its own identity.
    let optional = Field::new("symbol", DataType::Utf8, true);
    let array = optional.default_arrow_array()?;
    assert!(array.is_null(0));
    assert_eq!(scalar_value(&optional, array.as_ref())?, Scalar::Null);

    // A required Null column has no value it could ever hold.
    let refused = Field::new("never", DataType::Null, false).default_arrow_array();
    assert!(refused.is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    assert DataType("int64").default_arrow_scalar().as_py() == 0

    # A field is nullable unless you say otherwise, and its default is null.
    optional = Field("symbol", "utf8").default_arrow_scalar()
    assert not optional.is_valid
    assert optional.as_py() is None

    try:
        Field("never", "null", nullable=False).default_arrow_scalar()
    except ValueError as error:
        assert "logical-null default" in str(error)
    else:
        raise AssertionError("a required null column has no default")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    assert.equal(new DataType('int64').defaultArrowScalar(), 0n)

    // A field is nullable unless you say otherwise, and its default is null.
    assert.equal(new Field('symbol', 'utf8').defaultArrowScalar(), null)

    assert.throws(
      () => new Field('never', 'null', false).defaultArrowScalar(),
      /logical-null default/,
    )
    ```

## A struct root is one row

A Rust struct row is positional; Python and JavaScript key it by name.

=== "Rust"

    ```rust
    use yggdryl::arrow::scalar_value;
    use yggdryl::{DataType, Field, Scalar};

    let schema = Field::new(
        "row",
        DataType::from_fields([
            DataType::Int64.required_field("id"),
            DataType::Utf8.nullable_field("symbol"),
        ])?,
        false,
    );

    let row = scalar_value(&schema, schema.default_arrow_array()?.as_ref())?;
    let values = row.as_sequence().ok_or("a struct row is an ordered sequence")?;
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].as_i128(), Some(0));
    assert_eq!(values[1], Scalar::Null);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Field

    schema = Field(
        "row",
        DataType.from_fields(
            [Field("id", "int64", nullable=False), Field("symbol", "utf8")]
        ),
        nullable=False,
    )

    scalar = schema.default_arrow_scalar()
    assert scalar.as_py() == {"id": 0, "symbol": None}
    assert scalar["id"].as_py() == 0
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Field } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('symbol', 'utf8'),
      ]),
      false,
    )

    const scalar = schema.defaultArrowScalar()
    assert.equal(scalar.id, 0n)
    assert.equal(scalar.symbol, null)
    assert.deepEqual(
      [...scalar],
      [
        ['id', 0n],
        ['symbol', null],
      ],
    )
    ```

## Edges

- Python or JavaScript `Field` with no nullability argument -> nullable, so its default is null.
- Python registered `ExtensionType` -> rehydrates, never its storage type.
- JavaScript `int64` -> `BigInt` (`0n`).
- Python: `DataType.arrow_scalar`, `Field.arrow_scalar`, `Field.cast_arrow_array`, `Field.cast_arrow_batch` -> [../extensions/python.md](../extensions/python.md).
- JavaScript: `defaultArrowScalar` only; `fromArrow` -> [../types/datatype.md](../types/datatype.md), [../types/field.md](../types/field.md), [../extensions/javascript.md](../extensions/javascript.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test types default_scalar::
    cargo test --features "parquet iceberg" -p yggdryl --test arrow row_value::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_defaults.py
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/types/defaults.test.js
    npm run --prefix node bench:types:defaults
    ```
