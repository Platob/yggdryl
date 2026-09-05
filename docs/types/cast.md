# Cast

The [field](field.md) is the cast target: rows, arrays, and record batches are reconciled to its datatype and nullability.

## Contract

| Key | Value |
| --- | --- |
| Owns | `validate_value`, `canonicalize_value`, `ArrowCast`, `cast_arrow_scalar/array/batch`, `cast_arrow`, `cast` |
| Target | The field, never the source; an exact input returns unchanged, the same arrays |
| Returns | `Field`, `DataType`: `ArrayRef`; `TypedField`: its own array (datetime, dictionary: `ArrayRef`) |
| `safe` | `true`: failure nulls, non-null fields default (`Field::default_value`); `false`: error |
| Validates | `validate_value`: right arity, no null in a required column, every scalar in its declared range |
| Batch children | Target order, ASCII-case-insensitive names |
| Errors | The dot/bracket path of the first misfit |
| Bindings | `Scalar` rows Rust only; [Python](../extensions/python.md), [JavaScript](../extensions/javascript.md) cast Arrow data |

## Use

`safe` decides whether a failed conversion errors or defaults.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
    use yggdryl::types::Int64Field;
    use yggdryl::{ArrowCast, DataType, Field};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));

    // Any field answers with an ArrayRef, because any field could be any datatype.
    let field = Field::new("id", DataType::Int64, false);
    let cast = field.cast_arrow_array(Arc::clone(&text), false)?;
    assert_eq!(cast.data_type(), &arrow_schema::DataType::Int64);

    // A typed field already knows its variant, so it answers with the array itself.
    let typed = Int64Field::new("id", false);
    let ids: Int64Array = typed.cast_arrow_array(text, false)?;
    assert_eq!(ids.values(), &[1, 2]);

    // safe nulls a failed conversion; a non-null field then defaults it.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    assert!(typed.cast_arrow_array(Arc::clone(&broken), false).is_err());
    let repaired: Int64Array = typed.cast_arrow_array(broken, true)?;
    assert_eq!(repaired.values(), &[1, 0]);
    assert_eq!(repaired.null_count(), 0);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    field = Field("id", "int64", nullable=False)

    ids = field.cast_arrow_array(pa.array(["1", "2"]))
    assert ids.equals(pa.array([1, 2], type=pa.int64()))

    # safe nulls a failed conversion; a non-null field then defaults it.
    repaired = field.cast_arrow_array(pa.array(["1", "not a number"]))
    assert repaired.equals(pa.array([1, 0], type=pa.int64()))
    assert repaired.null_count == 0

    try:
        field.cast_arrow_array(pa.array(["1", "not a number"]), safe=False)
    except ValueError:
        pass
    else:
        raise AssertionError("an unsafe cast must fail")
    ```

## Row values

Rust only.

`validate_value` checks a [`Scalar`](scalar.md) row is representable; `canonicalize_value` rewrites it exactly.

```rust
use yggdryl::{DataType, Field, Scalar};

let schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Float32.nullable_field("price"),
])?
.required_field("trade");

// A row is one ordered sequence with one value per struct child.
let row = Scalar::from_sequence([Scalar::from(7u64), Scalar::from(0.1f64)]);
schema.validate_value(&row)?;

// Canonicalizing narrows every value into the representation the root declares.
let canonical = schema.canonicalize_value(row)?;
assert_eq!(canonical.get(0), Some(&Scalar::from(7_i64)));
assert_eq!(
    canonical.get(1).and_then(Scalar::as_f64),
    Some(f64::from(0.1f32))
);

// A value that does not fit names the path walked to reach it.
let wrong = Scalar::from_sequence([Scalar::from("seven"), Scalar::Null]);
let message = schema.validate_value(&wrong).unwrap_err().to_string();
assert!(message.contains("$.trade.id"), "{message}");
```

## Record batches

A `RecordBatch` is a `StructArray` plus a schema, so it takes the same recursive cast.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCast, DataType, Field};

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("trade");

    let source = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, true),
            ArrowField::new("id", ArrowDataType::Int32, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["ACME"])),
            Arc::new(Int32Array::from(vec![7])),
        ],
    )?;

    let batch = schema.cast_arrow_batch(source, false)?;
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");
    assert_eq!(batch.column(0).data_type(), &ArrowDataType::Int64);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import DataType, Field, types

    schema = Field(
        "trade",
        DataType.from_fields([
            types.int64("id", nullable=False),
            types.utf8("symbol"),
        ]),
        nullable=False,
    )

    source = pa.record_batch({
        "symbol": pa.array(["ACME"]),
        "id": pa.array([7], type=pa.int32()),
    })

    batch = schema.cast_arrow_batch(source)
    assert batch.schema.names == ["id", "symbol"]
    assert batch.column("id").type == pa.int64()
    ```

## The generic cast

`cast_arrow` keeps the input kind; `cast` also takes plain Python values.

| Input | Result |
| --- | --- |
| pyarrow `Scalar`, `Array`, `ChunkedArray`, `RecordBatch`, `Table`, `RecordBatchReader`, `Dataset`, `Scanner` | the same kind; streams per batch |
| polars `DataFrame` | itself, newest compat level, views stay views |
| polars `LazyFrame` | itself, still lazy (`collect_schema`, per batch) |
| pandas `DataFrame`, `Series` | itself, through Arrow |
| plain value, `cast` only | the field's typed scalar |

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    schema = Field("row", DataType("struct<id: int64, symbol: string>"), False)
    table = pa.table({"id": pa.array([1, 2], pa.int32()), "symbol": ["AAPL", "MSFT"]})

    # A table comes back a table, a reader a reader, a frame a frame.
    cast = schema.cast_arrow(table)
    assert cast.schema.field("id").type == pa.int64()

    # The generic name also takes plain values, as the typed scalar.
    price = Field("price", DataType("int64"), False)
    assert price.cast(5).as_py() == 5
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, fields } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    })

    // Whatever Arrow JS holds casts batch by batch and comes back a Table.
    const cast = schema.castArrow(table)
    assert.equal(cast.numRows, 2)
    assert.ok(schema.cast(table).numRows === 2)
    ```

## Edges

- Extra column -> dropped; missing nullable -> nulls; missing required -> canonical default.
- Nullable field, `safe` -> the null stays.
- A scalar wider than the declared type -> accepted when the value fits, then canonicalized into it (`U64` -> `I64`).
- Text into `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`, `Duration64` -> everything [text](../text/index.md) accepts, a duration included, which Arrow reads into none.
- A reading the declared unit or width cannot hold exactly -> null, never a rounded value.
- Bare date into a datetime, twelve-hour clock, compact `YYYYMMDD` -> Arrow's kernel.
- Temporal to text -> the classic form, zoned instants included.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::cast types::value
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- batch_cast::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_field.py -k "safe_casts or pyarrow_kind or polars or pandas"
    ```
