# Cast

The [field](field.md) is the cast target: rows, arrays, and record batches are reconciled to its datatype and nullability.

## Contract

| Key | Value |
| --- | --- |
| Owns | `validate_value`, `canonicalize_value`, `ArrowCast`, `ArrowCastOptions`, `Nullability`, `Representation`, `ArrowCastPlan`, `cast_arrow_scalar/array/batch`, `cast_arrow`, `cast` |
| Target | The field, never the source; an exact input returns unchanged - the same arrays, and the same batch object |
| Returns | `Field`, `DataType`: `ArrayRef`; `TypedField`: its own array (datetime, dictionary: `ArrayRef`) |
| `safe` | Whether a *present* value may be converted. `true`: a failed conversion becomes null; `false`: error |
| `nullability` | Whether a *declared* value may be absent. `default`: canonical default (`Field::default_value`); `strict`: error naming the path |
| `representation` | What a *same-width* pair carries. `value`: the number it spells, range-checked; `bits`: the bytes under it, buffer shared |
| Independent | The three answer different questions and compose: a `safe` conversion failure becomes a null, and `nullability` then decides whether that null may stand |
| Validates | `validate_value`: right arity, no null in a required column, every scalar in its declared range |
| Batch children | Target order, ASCII-case-insensitive names |
| Errors | The dot/bracket path of the first misfit, from the cast root: `$.users[].zip` |
| Bindings | `Scalar` rows Rust only; [Python](../extensions/python.md), [JavaScript](../extensions/javascript.md) cast Arrow data and pass both answers explicitly |

## Use

`safe` decides whether a failed conversion errors or defaults.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
    use yggdryl::types::Int64Field;
    use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field, Nullability};

    let strict_conversion = ArrowCastOptions::new().with_safe(false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));

    // Any field answers with an ArrayRef, because any field could be any datatype.
    let field = Field::new("id", DataType::Int64, false);
    let cast = field.cast_arrow_array(Arc::clone(&text), strict_conversion)?;
    assert_eq!(cast.data_type(), &arrow_schema::DataType::Int64);

    // A typed field already knows its variant, so it answers with the array itself.
    let typed = Int64Field::new("id", false);
    let ids: Int64Array = typed.cast_arrow_array(text, strict_conversion)?;
    assert_eq!(ids.values(), &[1, 2]);

    // safe nulls a failed conversion; the nullability policy then decides
    // whether that null may stand in for a declared value.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    assert!(typed.cast_arrow_array(Arc::clone(&broken), strict_conversion).is_err());
    let repaired: Int64Array =
        typed.cast_arrow_array(Arc::clone(&broken), ArrowCastOptions::new())?;
    assert_eq!(repaired.values(), &[1, 0]);
    assert_eq!(repaired.null_count(), 0);

    // Strict refuses the same null instead of defaulting it.
    let refused = typed
        .cast_arrow_array(broken, ArrowCastOptions::new().with_nullability(Nullability::Strict))
        .unwrap_err()
        .to_string();
    assert_eq!(refused, "required Arrow field $.id holds 1 null values");
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    field = Field("id", "int64", nullable=False)

    ids = field.cast_arrow_array(pa.array(["1", "2"]))
    assert ids.equals(pa.array([1, 2], type=pa.int64()))

    # safe nulls a failed conversion; the nullability policy then decides
    # whether that null may stand in for a declared value.
    repaired = field.cast_arrow_array(pa.array(["1", "not a number"]))
    assert repaired.equals(pa.array([1, 0], type=pa.int64()))
    assert repaired.null_count == 0

    try:
        field.cast_arrow_array(pa.array(["1", "not a number"]), safe=False)
    except ValueError:
        pass
    else:
        raise AssertionError("an unsafe cast must fail")

    # Strict refuses the same null instead of defaulting it, naming the path.
    try:
        field.cast_arrow_array(pa.array(["1", "not a number"]), nullability="strict")
    except ValueError as error:
        assert "$.id" in str(error), error
    else:
        raise AssertionError("a strict cast must refuse the null")
    ```

## Reading the bits

`representation` decides what a cast carries when the two datatypes occupy the same physical
width. `value` is the number they spell, range-checked as always. `bits` is the bytes under it:
an `int64`, a `uint64`, a `float64` and a `fixed_size_binary(8)` are one buffer under four
readings, so every source bit pattern maps, every chain round-trips, and the value buffer is
shared rather than rebuilt. `u64::MAX` reads as `-1`, and back.

It is a preference, not a mode. A pair that is *not* the same bytes - two different widths, or
text and a number - takes the ordinary conversion, and a datatype whose values follow a rule
(an [ASCII width](ascii.md), a registered code, a [UUID](uuid.md), a [version](../types/index.md))
keeps that rule: four arbitrary bytes are not a currency merely because a currency is four bytes.

Nullability is unaffected: the reading says what the bytes mean, and `nullability` still says
what an absent value means.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, FixedSizeBinaryArray, Int64Array, UInt64Array};
    use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field, Representation};

    let bits = ArrowCastOptions::new().with_representation(Representation::Bits);
    let source: ArrayRef = Arc::new(UInt64Array::from(vec![0, u64::MAX]));

    let signed = Field::new("digest", DataType::Int64, true)
        .cast_arrow_array(Arc::clone(&source), bits)?;
    let signed = signed.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!(signed.values(), &[0, -1]);

    // The same eight bytes, now as raw payload - and back again exactly.
    let stored = Field::new("digest", DataType::FixedSizeBinary(8), true)
        .cast_arrow_array(Arc::clone(&source), bits)?;
    let bytes = stored.as_any().downcast_ref::<FixedSizeBinaryArray>().unwrap();
    assert_eq!(bytes.value(1), &[0xff; 8]);
    let restored = Field::new("digest", DataType::UInt64, true)
        .cast_arrow_array(stored, bits)?;
    let restored = restored.as_any().downcast_ref::<UInt64Array>().unwrap();
    assert_eq!(restored.values(), &[0, u64::MAX]);

    // Four bytes are not eight, so this stays the ordinary numeric widening.
    let widened = Field::new("id", DataType::Int64, true)
        .cast_arrow_array(Arc::new(arrow_array::Int32Array::from(vec![7])), bits)?;
    assert_eq!(widened.data_type(), &arrow_schema::DataType::Int64);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field

    source = pa.array([0, 2**64 - 1], type=pa.uint64())

    signed = Field("digest", "int64").cast_arrow_array(source, representation="bits")
    assert signed.to_pylist() == [0, -1]

    # The same eight bytes, now as raw payload - and back again exactly.
    stored = Field("digest", "fixed_size_binary(8)").cast_arrow_array(
        source, representation="bits"
    )
    assert stored.to_pylist()[1] == b"\xff" * 8
    restored = Field("digest", "uint64").cast_arrow_array(stored, representation="bits")
    assert restored.equals(source)

    # Four bytes are not eight, so this stays the ordinary numeric widening.
    widened = Field("id", "int64").cast_arrow_array(
        pa.array([7], type=pa.uint32()), representation="bits"
    )
    assert widened.to_pylist() == [7]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const bits = { representation: 'bits' }
    const source = arrow.vectorFromArray([0n, 2n ** 64n - 1n], new arrow.Uint64())

    const signed = fields.int64('digest').castArrowArray(source, bits)
    assert.deepEqual([...signed], [0n, -1n])

    // The same eight bytes, now as raw payload - and back again exactly.
    const stored = fields.fixedSizeBinary('digest', 8).castArrowArray(source, bits)
    assert.deepEqual([...stored.get(1)], new Array(8).fill(255))
    assert.deepEqual(
      [...fields.uint64('digest').castArrowArray(stored, bits)],
      [...source],
    )
    ```

## Row values

Rust only.

`validate_value` checks a [`Scalar`](scalar.md) row is representable; `canonicalize_value` rewrites it exactly.

Canonicalization decides before it builds, so a row already in its declared representation
allocates nothing whatever it carries: text, byte, ASCII, code, and geospatial columns are
recognized from the value in hand rather than rebuilt and compared. A column that does rewrite a
layout - `utf8` to `large_utf8`, `binary` to `binary_view`, bytes to a geometry - retags the
storage handle it was given, so the payload is never copied. `rust/tests/allocations.rs` counts
both.

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
    use yggdryl::{ArrowCast, ArrowCastOptions, DataType, Field};

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

    let batch = schema.cast_arrow_batch(source, ArrowCastOptions::new().with_safe(false))?;
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

## Strict nullability

`nullability` decides what a cast does about a non-nullable target field the source cannot fill.
`default` repairs - the canonical [default](field.md), which is what a lake being filled wants.
`strict` refuses, naming the full dot/bracket path from the cast root, which is what a contract
being enforced wants. The two failures happen at different times: a required field *no source
column carries* is decided by the schemas alone and so is refused when the cast is compiled,
before any batch exists; a required field *holding null* is a property of the rows and is refused
when that batch is cast.

A missing *nullable* field is still all-null, and an undeclared source column is still dropped:
strictness is about declared values that are absent, not about columns nobody declared.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCast, ArrowCastOptions, ArrowCastPlan, DataType, Field, Nullability};

    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);
    let root = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
    ])?
    .required_field("row");

    let source = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        Arc::clone(&source),
        vec![Arc::new(Int32Array::from(vec![1])) as ArrayRef],
    )?;

    // Default: the hole is filled with the target's canonical default.
    assert_eq!(root.cast_arrow_batch(batch.clone(), ArrowCastOptions::new())?.num_columns(), 2);

    // Strict: refused, by path - and refused at compile time, because two
    // schemas are all it takes to know.
    let message = root.cast_arrow_batch(batch, strict).unwrap_err().to_string();
    assert_eq!(message, "required Arrow field $.symbol is missing from the source");
    assert!(ArrowCastPlan::compile(&source, &root, strict).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    root = Field("row", DataType("struct<id: int64, symbol: string not null>"), False)
    batch = pa.record_batch({"id": pa.array([1], pa.int32())})

    # Default: the hole is filled with the target's canonical default.
    assert root.cast_arrow_batch(batch).num_columns == 2

    # Strict: refused, by path.
    try:
        root.cast_arrow_batch(batch, nullability="strict")
    except ValueError as error:
        assert str(error) == "required Arrow field $.symbol is missing from the source"
    else:
        raise AssertionError("a strict cast must refuse the missing column")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, fields } = require('yggdryl')

    const root = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8 not null')],
      { nullable: false },
    )
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n], new arrow.Int64()),
    })

    // Default: the hole is filled with the target's canonical default.
    assert.equal(root.castArrow(table).numCols, 2)

    // Strict: refused, by path.
    assert.throws(
      () => root.castArrow(table, { nullability: 'strict' }),
      /required Arrow field \$\.symbol is missing from the source/,
    )
    ```

## Compiled plans

Everything a cast decides from two schemas - which source column answers which target field, the
child order, the recursive type dispatch, the target schema, the kernel options - is a function of
those schemas alone. `ArrowCastPlan` is that work made once: `compile` from a source schema, a
non-null Struct root, and the options; `preflight` to exercise the whole recursion over no rows;
`apply` per batch. The plan is immutable and `Send + Sync`, so one serves every batch of a stream
and every thread of a parallel scan; only the masks, offsets, and dictionary reachability a batch
actually carries vary.

`cast_arrow_batch` compiles one plan for its one batch. A caller with many batches of one schema
compiles the plan itself - and `cast_reader` already does, so a streamed cast never plans twice.

Rust only: the bindings reach the same reuse through their reader casts.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType};

let root = DataType::from_fields([DataType::Int64.required_field("id")])?
    .required_field("row");
let source = Arc::new(Schema::new(vec![ArrowField::new(
    "id",
    ArrowDataType::Int32,
    false,
)]));

let plan = ArrowCastPlan::compile(&source, &root, ArrowCastOptions::new())?;
// The whole recursion runs with no rows, so an impossible cast is known now.
plan.preflight()?;
assert_eq!(plan.as_schema().field(0).data_type(), &ArrowDataType::Int64);

for offset in 0..3 {
    let batch = RecordBatch::try_new(
        Arc::clone(&source),
        vec![Arc::new(Int32Array::from(vec![offset])) as ArrayRef],
    )?;
    assert_eq!(plan.apply(batch)?.num_rows(), 1);
}
```

## Eager and lazy

A cast of held data is eager and a cast of a stream is lazy, and the method names say which.
A table is already in memory, so casting one drains its batches and hands back a table. A reader
is not, so casting one hands back a reader that has cast nothing yet: it pulls one source batch,
casts it, and yields it, so a resource larger than memory casts in bounded memory. A batch's
failure therefore surfaces when *that batch* is pulled, not when the reader is built - and the
reader is fused after it, because a stream that has reported it cannot be honoured has nothing
further to say. Closing or dropping the reader releases the source stream at that point.

The generic `cast_arrow`/`castArrow` infers which of the two it is holding and delegates; it adds
no behavior of its own.

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    root = Field("row", DataType("struct<id: int64, symbol: string not null>"), False)
    table = pa.table({
        "id": pa.array([1, 2], pa.int32()),
        "symbol": ["AAPL", None],
    })

    # Eager: a table is drained here and comes back a table.
    assert isinstance(root.cast_arrow_table(table), pa.Table)

    # Lazy: the reader is built, and nothing has been cast yet.
    reader = root.cast_arrow_reader(table.to_reader(), nullability="strict")
    assert reader.schema.names == ["id", "symbol"]

    # The refusal arrives with the batch that carries it.
    try:
        reader.read_all()
    except Exception as error:
        assert "$.symbol" in str(error), error
    else:
        raise AssertionError("the null must be refused at the pull")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, fields } = require('yggdryl')

    const root = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    })

    // Lazy: a BatchReader that has cast nothing yet, and consumes its source.
    const reader = root.castArrowReader(table)
    assert.equal(reader.intoTable().numRows, 2)

    // Eager: one Arrow JS record batch in, one out.
    const [batch] = table.batches
    assert.equal(root.castArrowBatch(batch).numRows, 2)
    ```

## The generic cast

`cast_arrow` keeps the input kind; `cast` also takes plain Python values.

| Input | Result |
| --- | --- |
| pyarrow `Scalar`, `Array`, `ChunkedArray`, `RecordBatch` | the same kind, through the named method for it |
| pyarrow `Table` | a table, eagerly (`cast_arrow_table`) |
| pyarrow `RecordBatchReader`, `Dataset`, `Scanner`, any C stream | a lazy reader (`cast_arrow_reader`) |
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

- Extra column -> dropped under both policies; missing nullable -> nulls under both policies.
- Missing required -> canonical default under `default`; refused at compile time under `strict`.
- Null in a required column -> canonical default under `default`; refused at that batch under `strict`, with the null count.
- Null hidden inside a null parent row -> stays hidden; strictness reads only the exposed rows.
- `safe=True` plus `strict` -> the failed conversion becomes a null and the null is then refused; `safe=False` refuses the conversion first.
- A `Null` datatype under `strict` -> null is its only value, so it is not absence.
- Python `cast_arrow_scalar` -> a scalar has no row to repair, so a null entering a non-nullable Field is refused under either policy.
- Nullable field, `safe` -> the null stays.
- A scalar wider than the declared type -> accepted when the value fits, then canonicalized into it (`U64` -> `I64`).
- Text into `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`, `Duration64` -> everything [text](../text/index.md) accepts, a duration included, which Arrow reads into none.
- A reading the declared unit or width cannot hold exactly -> null, never a rounded value.
- Bare date into a datetime, twelve-hour clock, compact `YYYYMMDD` -> Arrow's kernel.
- Temporal to text -> the classic form, zoned instants included.
- `representation="bits"` over two different widths, or into a datatype with a value rule -> the ordinary conversion, range check and all.
- A required `bits` target over source nulls -> the canonical default under `default`, refused by path under `strict`; the buffer is rebuilt only when a null is actually filled.
- A batch of another schema handed to a compiled plan -> error naming both schemas; a plan is compiled for one source.
- A reader whose source schema is already the target, under `default` -> the reader itself, unwrapped; under `strict` it is wrapped, because a non-null Arrow field can still carry a logical null in a nested child.

## Performance

Row canonicalization over a three-column row - `utf8`, `binary`, `currency` - at two payload
sizes. Containerized x86_64 Linux, Intel Xeon, rustc 1.94.1 release, Criterion point estimates.
`unchanged` hands the root a row already in its declared representation; `relayout` hands the
same row to a `large_utf8`/`large_binary` root. Both are flat in the payload because neither
reads it: the cost is the walk over the three columns, not the bytes behind them.

| row | 64 B payload | 64 KiB payload |
| --- | ---: | ---: |
| unchanged | 235 ns | 238 ns |
| relayout | 280 ns | 275 ns |

```bash
cargo bench --manifest-path rust/Cargo.toml --bench types -- '^value/canonicalize_row'
```

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::cast types::value
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- batch_cast:: strict_cast::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test arrow -- cast_plan::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test allocations
    cargo bench --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --bench types -- cast_plan
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_field.py -k "cast or strict or nullability"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```

## Performance

Compiling the cast once against compiling it per batch, over batches of 64 rows through a
three-column root that widens one column, drops one, and defaults one. One containerized x86_64
Linux run: Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 release with thin LTO. Criterion
medians.

| Batches | One compiled plan | Planned per batch |
| --- | --- | --- |
| 1 | 6.04 µs | 6.01 µs |
| 10 | 39.7 µs | 60.4 µs |
| 1,000 | 3.68 ms | 6.07 ms |

One batch is the same work either way - the plan is compiled once in both - and everything after
it is the saving: 1.5x at ten batches and 1.7x at a thousand, which is what a streamed read pulls.

The benchmark asserts the two paths answer identical rows before timing either, and refuses a
build where reusing the plan is slower than rebuilding it.

```bash
cargo bench --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --bench types -- cast_plan
```
