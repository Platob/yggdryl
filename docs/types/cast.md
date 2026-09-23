# Cast

The [field](field.md) is the cast target: an Arrow array, a record batch, a stream or a [`Serie`](serie.md) already in hand is reconciled to its datatype and nullability, and what comes out is a `Serie` under that field.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ArrowCastPlan`, `ArrowCastOptions`, `Nullability`, `Representation`; `validate_value` and `canonicalize_value` for rows |
| Ways in | `Serie::cast` for a column in hand; `Serie::from_arrow_array`, `from_arrow_batch`, `from_arrow_reader` for Arrow buffers; `SerieReader::from_arrow_reader` for a stream; an `ArrowCastPlan` held and applied wherever one cast repeats |
| Target | The field, never the source. A `DataType` target is its required `value` field (`dtype.required_field("value")`), so a refusal names `$.value` |
| Returns | A `Serie` under the target field. A typed read is a narrowing of it: `as_int64().values()`, `as_utf8()`, `as_date32()`, `as_fixed_bytes()` |
| Exact input | The identity plan: the same buffers, and a column already under the target is itself |
| `safe` | Whether a *present* value may be converted. `true`: a failed conversion becomes null; `false`: error |
| `nullability` | Whether a *declared* value may be absent. `default`: canonical default (`Field::default_value`); `strict`: error naming the path |
| `representation` | What a *same-width* pair carries. `value`: the number it spells, range-checked; `bits`: the bytes under it, buffer shared |
| Independent | The three answer different questions and compose: a `safe` conversion failure becomes a null, and `nullability` then decides whether that null may stand |
| Empty text | A zero-length text cell entering a non-text column is null before `safe` is asked; `nullability` decides the rest. A string, byte or interval column, and a code whose neutral member is the empty text, keep it as the value it is |
| Validates | `validate_value`: right arity, no null in a required column, every scalar in its declared range |
| One reading | A row and a column read the same spellings: text into a number, a boolean, a decimal or a temporal; any value with a spelling into text; any byte-carrying value into a byte layout |
| Layouts | Every list layout reads every other one, every byte framing reads every other one, and an encoding is a layout: a dictionary or run-end target runs its values' rule, and an encoded source is read as the column it holds |
| Batch children | Target order, ASCII-case-insensitive names |
| Proof | A landed column holds only rows its field accepts; an extension label is never proof of that ([What a landing proves](#what-a-landing-proves)) |
| Errors | The dot/bracket path of the first misfit, from the cast root: `$.users[].zip`; a column is its own first segment, `$.id` |
| Bindings | `Serie`, `SerieReader` and `ArrowCastPlan` in Rust, Python and JavaScript, the three options by name; `Scalar` rows in Rust and Python |

## Use

An array enters as the column of a field. `safe` decides whether a failed conversion errors or defaults.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Nullability, Serie};

    let field = Field::new("id", DataType::Int64, false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));

    // One door: the array lands as the column of `field`, cast on the way in.
    let strict_conversion = ArrowCastOptions::new().with_safe(false);
    let ids = Serie::from_arrow_array(Some(&field), text, strict_conversion)?;
    assert_eq!(ids.field(), Some(&field));
    // A typed read is a narrowing of the column that came out.
    assert_eq!(ids.as_int64().expect("an int64 column").values(), &[1, 2]);

    // safe nulls a failed conversion; the nullability policy then decides
    // whether that null may stand in for a declared value.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    assert!(Serie::from_arrow_array(Some(&field), Arc::clone(&broken), strict_conversion).is_err());
    let repaired =
        Serie::from_arrow_array(Some(&field), Arc::clone(&broken), ArrowCastOptions::new())?;
    assert_eq!(repaired.as_int64().expect("an int64 column").values(), &[1, 0]);
    assert_eq!(repaired.null_count(), 0);

    // Strict refuses the same null instead of defaulting it, naming the path.
    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);
    let refused = Serie::from_arrow_array(Some(&field), broken, strict)
        .unwrap_err()
        .to_string();
    assert_eq!(refused, "required Arrow field $.id holds 1 null values");

    // A column in hand casts once under another field.
    let narrow = ids.cast(&Field::new("id", DataType::Int32, false), ArrowCastOptions::new())?;
    assert_eq!(narrow.as_int32().expect("an int32 column").values(), &[1, 2]);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import DataType, Field, Serie

    field = Field("id", "int64", nullable=False)

    ids = Serie.from_arrow_array(pa.array(["1", "2"]), field, safe=False)
    assert ids.field == field
    assert ids.into_arrow_array().equals(pa.array([1, 2], type=pa.int64()))

    # safe nulls a failed conversion; the nullability policy then decides
    # whether that null may stand in for a declared value.
    broken = pa.array(["1", "not a number"])
    repaired = Serie.from_arrow_array(broken, field)
    assert repaired.as_py() == [1, 0]
    assert repaired.null_count() == 0

    try:
        Serie.from_arrow_array(broken, field, safe=False)
    except ValueError:
        pass
    else:
        raise AssertionError("an unsafe cast must fail")

    # Strict refuses the same null instead of defaulting it, naming the path.
    try:
        Serie.from_arrow_array(broken, field, nullability="strict")
    except ValueError as error:
        assert str(error) == "required Arrow field $.id holds 1 null values"
    else:
        raise AssertionError("a strict cast must refuse the null")

    # A column in hand casts once; a DataType is its required `value` field.
    narrow = ids.cast(DataType("int32"))
    assert narrow.field == Field("value", "int32", nullable=False)
    assert narrow.as_py() == [1, 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const field = fields.int64('id', { nullable: false })

    const ids = Serie.fromArrowArray(
      arrow.vectorFromArray(['1', '2'], new arrow.Utf8()),
      field,
      { safe: false },
    )
    assert.ok(ids.field.equals(field))
    assert.deepEqual([...ids.intoArrowArray()], [1n, 2n])

    // safe nulls a failed conversion; the nullability policy then decides
    // whether that null may stand in for a declared value.
    const broken = arrow.vectorFromArray(['1', 'not a number'], new arrow.Utf8())
    assert.deepEqual(Serie.fromArrowArray(broken, field).asJs(), [1, 0])
    assert.throws(() => Serie.fromArrowArray(broken, field, { safe: false }), /not a number/)

    // Strict refuses the same null instead of defaulting it, naming the path.
    assert.throws(
      () => Serie.fromArrowArray(broken, field, { nullability: 'strict' }),
      /required Arrow field \$\.id holds 1 null values/,
    )

    // A column in hand casts once under another field.
    const narrow = ids.cast(fields.int32('id', { nullable: false }))
    assert.equal(narrow.field.dtype.toString(), 'int32')
    assert.deepEqual(narrow.asJs(), [1, 2])
    ```

## Reading the bits

`representation` decides what a cast carries when the two datatypes occupy the same physical
width. `value` is the number they spell, range-checked as always. `bits` is the bytes under it:
an `int64`, a `uint64`, a `float64` and a `fixed_size_binary(8)` are one buffer under four
readings, so every source bit pattern maps, every chain round-trips, and the value buffer is
shared rather than rebuilt. `u64::MAX` reads as `-1`, and back.

It is a preference, not a mode. A pair that is *not* the same bytes - two different widths, or
text and a number - takes the ordinary conversion, and a datatype whose values follow a rule
(a [fixed string](text/string.md), a [registered code](codes/index.md), a [UUID](uuid.md), a [version](version.md))
keeps that rule: four arbitrary bytes are not a currency merely because a currency is four bytes.

Nullability is unaffected: the reading says what the bytes mean, and `nullability` still says
what an absent value means.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, UInt64Array};
    use yggdryl::{ArrowCastOptions, DataType, Field, Representation, Serie};

    let bits = ArrowCastOptions::new().with_representation(Representation::Bits);
    let source: ArrayRef = Arc::new(UInt64Array::from(vec![0, u64::MAX]));

    let signed = Field::new("digest", DataType::Int64, true);
    let signed = Serie::from_arrow_array(Some(&signed), Arc::clone(&source), bits)?;
    assert_eq!(signed.as_int64().expect("an int64 column").values(), &[0, -1]);

    // The same eight bytes, now as raw payload - and back again exactly.
    let payload = Field::new("digest", DataType::fixed_binary(8)?, true);
    let stored = Serie::from_arrow_array(Some(&payload), Arc::clone(&source), bits)?;
    let bytes = stored.as_fixed_bytes().expect("a fixed byte column");
    assert_eq!(bytes.value(1), Some(&[0xff_u8; 8][..]));
    let restored = stored.cast(&Field::new("digest", DataType::UInt64, true), bits)?;
    assert_eq!(restored.as_uint64().expect("a uint64 column").values(), &[0, u64::MAX]);

    // Four bytes are not eight, so this stays the ordinary numeric widening.
    let narrow: ArrayRef = Arc::new(Int32Array::from(vec![7]));
    let widened =
        Serie::from_arrow_array(Some(&Field::new("id", DataType::Int64, true)), narrow, bits)?;
    assert_eq!(widened.as_int64().expect("an int64 column").values(), &[7]);
    ```

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl import Field, Serie

    source = pa.array([0, 2**64 - 1], type=pa.uint64())

    signed = Serie.from_arrow_array(source, Field("digest", "int64"), representation="bits")
    assert signed.as_py() == [0, -1]

    # The same eight bytes, now as raw payload - and back again exactly.
    stored = Serie.from_arrow_array(
        source, Field("digest", "fixed_size_binary(8)"), representation="bits"
    )
    assert stored.as_py()[1] == b"\xff" * 8
    restored = stored.cast(Field("digest", "uint64"), representation="bits")
    assert restored.into_arrow_array().equals(source)

    # Four bytes are not eight, so this stays the ordinary numeric widening.
    widened = Serie.from_arrow_array(
        pa.array([7], type=pa.uint32()), Field("id", "int64"), representation="bits"
    )
    assert widened.as_py() == [7]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const bits = { representation: 'bits' }
    const source = arrow.vectorFromArray([0n, 2n ** 64n - 1n], new arrow.Uint64())

    const signed = Serie.fromArrowArray(source, fields.int64('digest'), bits)
    assert.deepEqual([...signed.intoArrowArray()], [0n, -1n])

    // The same eight bytes, now as raw payload - and back again exactly.
    const stored = Serie.fromArrowArray(source, fields.fixedSizeBinary('digest', 8), bits)
    assert.deepEqual([...stored.intoArrowArray().get(1)], new Array(8).fill(255))
    assert.deepEqual(
      [...stored.cast(fields.uint64('digest'), bits).intoArrowArray()],
      [...source],
    )

    // Four bytes are not eight, so this stays the ordinary numeric widening.
    const narrow = arrow.vectorFromArray([7], new arrow.Uint32())
    assert.deepEqual(Serie.fromArrowArray(narrow, fields.int64('id'), bits).asJs(), [7])
    ```

## Row values

`validate_value` checks a [`Scalar`](scalar.md) row is representable; `canonicalize_value` rewrites it exactly.

Canonicalization decides before it builds, so a row already in its declared representation
allocates nothing whatever it carries: text, byte, code, and geospatial columns are
recognized from the value in hand rather than rebuilt and compared. A column that does rewrite a
layout - `utf8` to `large_utf8`, `binary` to `binary_view`, bytes to a geometry - retags the
storage handle it was given, so the payload is never copied. `rust/tests/allocations.rs` counts
both.

`DataType::scalar` and `Field::scalar` are the one value contract, and they read every spelling
the column tier reads. Text becomes the number, boolean, decimal or temporal a column declares -
through this crate's own readers, so a digit a scale cannot hold is refused rather than rounded.
Any value that prints a spelling enters a text column as that spelling, a geometry included.
Any value that carries bytes enters a byte column as that payload; a fixed string carries the
width it declares and pads to it on the way out, because that is what the fixed column stores.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, StructType};

    let money: DataType = "decimal128(10, 2)".parse()?;
    assert_eq!(money.scalar("10.50")?, Scalar::d128(1_050, 2));
    // A digit the declared scale cannot state is a value change, so it is refused.
    assert!(money.scalar("1.005").is_err());

    assert_eq!(DataType::date32().scalar("1970-01-02")?, Scalar::date32(1));
    assert_eq!(DataType::utf8().scalar(7_i64)?, Scalar::from("7"));
    assert_eq!(DataType::binary().scalar("hi")?, Scalar::from(b"hi".to_vec()));

    // A record is a map keyed by name, so a map column reads one.
    let prices: DataType = "map<utf8, int32>".parse()?;
    assert_eq!(
        prices.scalar(Scalar::from_struct([("a", Scalar::from(1_i32))])?)?,
        Scalar::from_mapping([(Scalar::from("a"), Scalar::from(1_i32))])?
    );

    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float32.nullable_field("price"),
    ])?)
    .required_field("trade");

    // A row is one ordered sequence with one value per struct child.
    let row = Scalar::from_sequence([Scalar::from(7u64), Scalar::from(0.1f64)]);
    schema.validate_value(&row)?;

    // Canonicalizing narrows every value into the representation the root declares.
    let canonical = schema.canonicalize_value(row)?;
    assert!(matches!(canonical.get(0).as_deref(), Some(Scalar::Int64(_))));
    assert_eq!(
        canonical.get(1).as_deref().and_then(Scalar::as_f64),
        Some(f64::from(0.1f32))
    );

    // A value that does not fit names the path walked to reach it.
    let wrong = Scalar::from_sequence([Scalar::from("seven"), Scalar::Null]);
    let message = schema.validate_value(&wrong).unwrap_err().to_string();
    assert!(message.contains("$.trade.id"), "{message}");
    ```

=== "Python"

    ```python
    import decimal

    import pytest

    from yggdryl import Field

    price = Field("price", "decimal128(18,4)")
    assert price.scalar(decimal.Decimal("1.5")).as_py() == decimal.Decimal("1.5000")
    with pytest.raises(ValueError, match="n"):
        Field("n", "int64", nullable=False).scalar(None)

    root = Field("row", "struct<id:int64,symbol:utf8>", nullable=False)
    row = root.canonicalize_value([1, "AAPL"])
    assert row.kind == "list"
    assert row.as_py() == [1, "AAPL"]
    root.validate_value(row)
    with pytest.raises(ValueError):
        root.canonicalize_value([1])
    ```

=== "JavaScript"

    !!! note "`validate_value` and `canonicalize_value` are Rust and Python only"
        JavaScript binds `scalar` on both `DataType` and `Field`; it binds no
        `validate_value` or `canonicalize_value`.

## Record batches

A `RecordBatch` is a `StructArray` plus a schema, so it takes the same recursive cast: it lands as the record column of its rows under a non-null Struct root, children reconciled by name and returned in the root's order. With no root the batch is the record `row` of its own schema, its columns shared.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
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

    let trades = Serie::from_arrow_batch(Some(&root), &source, ArrowCastOptions::new().with_safe(false))?;
    assert_eq!(trades.field(), Some(&root));
    let ids = trades.child("id").and_then(Serie::as_int64).map(|ids| ids.values().to_vec());
    assert_eq!(ids, Some(vec![7]));

    // Back out as a table, in the root's order.
    let batch = trades.into_arrow_batch()?;
    assert_eq!(batch.num_columns(), 2);
    assert_eq!(batch.schema().field(0).name(), "id");
    assert_eq!(batch.column(0).data_type(), &ArrowDataType::Int64);

    // With no root the batch is the record `row` of its own schema.
    let own = Serie::from_arrow_batch(None, &source, ArrowCastOptions::new())?;
    assert_eq!(own.field().map(Field::name), Some("row"));
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import yggdryl
    from yggdryl import DataType, Field, Serie

    root = Field(
        "trade",
        DataType.from_fields([
            yggdryl.int64("id", nullable=False),
            yggdryl.utf8("symbol"),
        ]),
        nullable=False,
    )

    source = pa.record_batch({
        "symbol": pa.array(["ACME"]),
        "id": pa.array([7], type=pa.int32()),
    })

    trades = Serie.from_arrow_batch(source, root, safe=False)
    assert trades.child("id").as_py() == [7]

    batch = trades.into_arrow_batch()
    assert batch.schema.names == ["id", "symbol"]
    assert batch.column("id").type == pa.int64()

    # With no root the batch is the record `row` of its own schema.
    assert Serie.from_arrow_batch(source).field.name == "row"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, Serie, fields } = require('yggdryl')

    const root = fields.struct(
      'trade',
      [Field.from('id: int64 not null'), Field.from('symbol: utf8')],
      { nullable: false },
    )
    const source = new arrow.Table({
      symbol: arrow.vectorFromArray(['ACME'], new arrow.Utf8()),
      id: arrow.vectorFromArray([7], new arrow.Int32()),
    })

    const trades = Serie.fromArrowBatch(source, root, { safe: false })
    assert.deepEqual(trades.child('id').asJs(), [7])

    const batch = trades.intoArrowBatch()
    assert.deepEqual(
      batch.schema.fields.map((field) => `${field.name}: ${field.type}`),
      ['id: Int64', 'symbol: Utf8'],
    )

    // With no root the batch is the record `row` of its own schema.
    assert.equal(Serie.fromArrowBatch(source).field.name, 'row')
    ```

## Strict nullability

`nullability` decides what a cast does about a non-nullable target field the source cannot fill.
`default` repairs - the canonical [default](field.md), which is what a lake being filled wants.
`strict` refuses, naming the full dot/bracket path from the cast root, which is what a contract
being enforced wants. The two failures happen at different times: a required field *no source
column carries* is decided by the fields alone and so is refused when the cast is compiled,
before any batch exists; a required field *holding null* is a property of the rows and is refused
when that batch is cast.

A missing *nullable* field is still all-null, and an undeclared source column is still dropped:
strictness is about declared values that are absent, not about columns nobody declared.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType, Field, Nullability, Serie, StructType};

    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);
    let root = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ])?)
    .required_field("row");

    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![1])) as ArrayRef],
    )?;

    // Default: the hole is filled with the target's canonical default.
    let filled = Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new())?;
    assert_eq!(filled.into_arrow_batch()?.num_columns(), 2);

    // Strict: refused, by path - and refused at compile time, because two
    // fields are all it takes to know.
    let message = Serie::from_arrow_batch(Some(&root), &batch, strict)
        .unwrap_err()
        .to_string();
    assert_eq!(message, "required Arrow field $.symbol is missing from the source");
    let source = Field::from_arrow_schema("row", &schema)?;
    assert!(ArrowCastPlan::compile(&source, &root, strict).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import ArrowCastPlan, DataType, Field, Serie

    root = Field("row", DataType("struct<id: int64, symbol: string not null>"), False)
    batch = pa.record_batch({"id": pa.array([1], pa.int32())})

    # Default: the hole is filled with the target's canonical default.
    assert Serie.from_arrow_batch(batch, root).into_arrow_batch().num_columns == 2

    # Strict: refused, by path - and refused where the plan is compiled.
    try:
        Serie.from_arrow_batch(batch, root, nullability="strict")
    except ValueError as error:
        assert str(error) == "required Arrow field $.symbol is missing from the source"
    else:
        raise AssertionError("a strict cast must refuse the missing column")
    try:
        ArrowCastPlan(batch.schema, root, nullability="strict")
    except ValueError as error:
        assert "$.symbol" in str(error)
    else:
        raise AssertionError("the plan must refuse the missing column")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { ArrowCastPlan, Field, Serie, fields } = require('yggdryl')

    const root = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8 not null')],
      { nullable: false },
    )
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n], new arrow.Int64()),
    })

    // Default: the hole is filled with the target's canonical default.
    assert.equal(Serie.fromArrowBatch(table, root).intoArrowBatch().numCols, 2)

    // Strict: refused, by path - and refused where the plan is compiled.
    const missing = /required Arrow field \$\.symbol is missing from the source/
    assert.throws(() => Serie.fromArrowBatch(table, root, { nullability: 'strict' }), missing)
    assert.throws(
      () => ArrowCastPlan.compile(table.schema, root, { nullability: 'strict' }),
      missing,
    )
    ```

## Empty text

An empty text cell entering a column that does not hold text is no value. It reads as null
through every door - the Arrow walk and the scalar door alike - before any spelling is parsed
and before `safe` is consulted: an empty cell is not a failed conversion, because there was
nothing to convert. `nullability` then decides what a required column does with that null,
exactly as it does for a null the source carried. Only zero-length text is empty; whitespace is
a spelling no reader takes and keeps the reader's own answer. A string leaf, a byte leaf and an
interval keep the empty cell as what it is, and so does a code whose neutral member is the
empty text, which is also that code's canonical default. A reader that takes only a plain text
layout - version, url, urn, timezone, mimetype, mediatype under a dictionary source - answers a
column with no visible value as a null column, so a required target under `strict` is refused
by path there too.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Nullability, Scalar, Serie};

    let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
    let unsafe_cast = ArrowCastOptions::new().with_safe(false);

    // Nullable: null, and `safe = false` never sees the cell.
    let nullable = DataType::Int64.nullable_field("count");
    assert!(Serie::from_arrow_array(Some(&nullable), Arc::clone(&empty), unsafe_cast)?.is_null(0)?);

    // Required: the default under `default`, refused by path under `strict`.
    let required = DataType::Int64.required_field("count");
    let repaired = Serie::from_arrow_array(Some(&required), Arc::clone(&empty), ArrowCastOptions::new())?;
    assert_eq!(repaired.as_int64().expect("an int64 column").values(), &[0]);
    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);
    let message = Serie::from_arrow_array(Some(&required), empty, strict)
        .unwrap_err()
        .to_string();
    assert_eq!(message, "required Arrow field $.count holds 1 null values");

    // The scalar door answers the same, and a text column keeps the cell.
    assert_eq!(DataType::Int64.scalar("")?, Scalar::Null);
    assert_eq!(DataType::utf8().scalar("")?, Scalar::from(""));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, Serie

    empty = pa.array([""])

    # Nullable: null, and safe=False never sees the cell.
    nullable = Field("count", "int64")
    assert Serie.from_arrow_array(empty, nullable, safe=False).null_count() == 1

    # Required: the default under default, refused by path under strict.
    required = Field("count", "int64", nullable=False)
    assert Serie.from_arrow_array(empty, required).as_py() == [0]
    try:
        Serie.from_arrow_array(empty, required, nullability="strict")
    except ValueError as error:
        assert str(error) == "required Arrow field $.count holds 1 null values"
    else:
        raise AssertionError("a strict cast must refuse the empty cell")

    # The scalar door answers the same, and a text column keeps the cell.
    assert DataType("int64").scalar("").as_py() is None
    assert DataType("utf8").scalar("").as_py() == ""
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, Serie, fields } = require('yggdryl')

    const empty = arrow.vectorFromArray([''], new arrow.Utf8())

    // Nullable: null, and safe: false never sees the cell.
    const nullable = fields.int64('count')
    assert.deepEqual(Serie.fromArrowArray(empty, nullable, { safe: false }).asJs(), [null])

    // Required: the default under default, refused by path under strict.
    const required = fields.int64('count', { nullable: false })
    assert.deepEqual(Serie.fromArrowArray(empty, required).asJs(), [0])
    assert.throws(
      () => Serie.fromArrowArray(empty, required, { nullability: 'strict' }),
      /required Arrow field \$\.count holds 1 null values/,
    )

    // The scalar door answers the same, and a text column keeps the cell.
    assert.equal(DataType.from('int64').scalar('').kind, 'null')
    assert.equal(DataType.utf8().scalar('').asJs(), '')
    ```

## Compiled plans

Everything a cast decides from two fields - which source child answers which target field, the
child order, the recursive type dispatch, the target's Arrow projection, the kernel options - is a
function of those fields alone. `ArrowCastPlan` is that work made once: `compile` from a source
field, a target field and the options; `preflight` to exercise the whole recursion over no rows;
`apply` per column of the source layout, each landing a `Serie` under the target. A batch's schema
is a source as the record it lays out as: `Field::from_arrow_schema("row", &schema)`. The plan is
immutable and `Send + Sync`, so one serves every column of a stream and every thread of a
parallel scan; only the masks, offsets, and dictionary reachability a column actually carries vary.

`Serie::cast` and the `Serie` Arrow doors compile one plan for their one input, so a loop that
calls them compiles per iteration. A loop holds the plan instead - and `SerieReader` already
does, so a stream never plans twice. `as_source` answers the Arrow field an input must lay out as,
`as_target` the field every cast lands under, `as_options` the three answers, and `is_identity`
whether the plan hands every input of its source layout straight back.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType, Field, Serie, StructType};

    let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        false,
    )]));

    // A batch's schema is the record `row` its columns are the children of.
    let source = Field::from_arrow_schema("row", &schema)?;
    let plan = ArrowCastPlan::compile(&source, &root, ArrowCastOptions::new())?;
    // The whole recursion runs with no rows, so an impossible cast is known now.
    plan.preflight()?;
    assert_eq!(plan.as_target(), &root);
    assert!(!plan.is_identity());

    for offset in 0..3 {
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![Arc::new(Int32Array::from(vec![offset])) as ArrayRef],
        )?;
        // The batch lands as its own layout, sharing its columns, and the
        // held plan casts it.
        let landed = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())?;
        let cast = plan.apply(&landed)?;
        let id = cast.child("id").and_then(Serie::as_int64).map(|ids| ids.values()[0]);
        assert_eq!(id, Some(i64::from(offset)));
    }
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import ArrowCastPlan, DataType, Field, Serie

    root = Field("row", DataType("struct<id: int64 not null>"), False)
    schema = pa.schema([pa.field("id", pa.int32(), nullable=False)])

    # A pyarrow schema is the record `row` its columns are the children of.
    plan = ArrowCastPlan(schema, root)
    plan.preflight()
    assert plan.source.name == "row"
    assert plan.target == root
    assert not plan.is_identity

    for offset in range(3):
        batch = pa.record_batch([pa.array([offset], pa.int32())], schema=schema)
        # A batch is first the record column of its own schema, then cast.
        assert plan.apply(batch).child("id").as_py() == [offset]
        assert plan.apply(Serie.from_arrow_batch(batch)).as_py() == [{"id": offset}]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { ArrowCastPlan, Field, Serie, fields } = require('yggdryl')

    const root = fields.struct('row', [Field.from('id: int64 not null')], { nullable: false })
    const table = new arrow.Table({ id: arrow.vectorFromArray([1, 2], new arrow.Int32()) })

    // An Arrow JS schema is the record `row` its columns are the children of.
    const plan = ArrowCastPlan.compile(table.schema, root)
    plan.preflight()
    assert.equal(plan.source.name, 'row')
    assert.ok(plan.target.equals(root))
    assert.equal(plan.isIdentity, false)
    assert.deepEqual(plan.options, { safe: true, nullability: 'default', representation: 'value' })

    for (const batch of table.batches) {
      assert.deepEqual(plan.apply(Serie.fromArrowBatch(batch)).child('id').asJs(), [1, 2])
    }
    ```

## What a landing proves

A column holds only rows its field accepts, so every cast ends where its rows land in a
`Serie`, and the landing proves them. The layout and the absence are proven on the buffers:
the projection compared, the validity words counted. A value is proven by the layout wherever
the layout is the datatype's whole contract, and is otherwise read once - unless the plan itself
certified it: an ingest that read every value under the target's rule (a string with a bound, a
width or a charset other than UTF-8, a bounded byte column, a code, a UUID, a URL or a URN), a
target that is its own contract, a compiled default, or an exact node over a column that had
already landed.

An extension label is never such a proof. A foreign column whose Arrow field says
`yggdryl.url` is only claiming to be a URL, so its rows are read once, and the first one the
field refuses is named with its column and its row - under every option, because the claim is
the source's, not a conversion `safe` could null. Foreign bytes an Arrow kernel moves into a
leaf with a rule are read the same way: Arrow reads any `int64` as a `date64`, and a `date64`
here is a whole day.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use arrow_schema::Schema;
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie, StructType};

    // The column claims to be a URL; the landing still reads what it holds.
    let url = Field::new("u", DataType::Url, false);
    let labelled = url.clone().into_arrow_field_ref()?.as_ref().clone();
    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![labelled])),
        vec![Arc::new(StringArray::from(vec!["not a url"])) as ArrayRef],
    )?;
    let root = DataType::from(StructType::from_fields([url])?).required_field("row");
    let message = Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new())
        .unwrap_err()
        .to_string();
    assert!(message.contains("$[0].u"), "{message}");

    // A kernel moves the bytes; the field's rule still reads them.
    let day = Field::new("day", DataType::Date64, false);
    let millis: ArrayRef = Arc::new(Int64Array::from(vec![1_i64]));
    assert!(Serie::from_arrow_array(Some(&day), millis, ArrowCastOptions::new()).is_err());
    let midnight: ArrayRef = Arc::new(Int64Array::from(vec![86_400_000_i64]));
    assert_eq!(Serie::from_arrow_array(Some(&day), midnight, ArrowCastOptions::new())?.len(), 1);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    # The column claims to be a URL; the landing still reads what it holds.
    root = Field("row", "struct<u: url not null>", nullable=False)
    labelled = pa.record_batch([pa.array(["not a url"])], schema=root.into_arrow_schema())
    assert labelled.schema.field("u").metadata[b"ARROW:extension:name"] == b"yggdryl.url"
    for claimed in (root, None):
        try:
            Serie.from_arrow_batch(labelled, claimed)
        except ValueError as error:
            assert "$[0].u" in str(error), error
        else:
            raise AssertionError("a label proves nothing")

    # A kernel moves the bytes; the field's rule still reads them.
    day = Field("day", "date64", nullable=False)
    try:
        Serie.from_arrow_array(pa.array([1], pa.int64()), day)
    except ValueError as error:
        assert "whole-day" in str(error), error
    else:
        raise AssertionError("a date64 is a whole day")
    assert len(Serie.from_arrow_array(pa.array([86_400_000], pa.int64()), day)) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, Serie } = require('yggdryl')

    // The column claims to be a URL; the landing still reads what it holds.
    const plain = new arrow.Table({ u: arrow.vectorFromArray(['not a url'], new arrow.Utf8()) })
    const claim = new Map([['ARROW:extension:name', 'yggdryl.url']])
    const url = plain.schema.fields[0].clone({ metadata: claim })
    const labelled = new arrow.Table(new arrow.Schema([url]), plain.batches)
    const root = Field.from('row: struct<u: url> not null')
    for (const claimed of [root, undefined]) {
      assert.throws(() => Serie.fromArrowBatch(labelled, claimed), /\$\[0\]\.u/)
    }

    // A kernel moves the bytes; the field's rule still reads them.
    const day = Field.from('day: date64 not null')
    assert.throws(
      () => Serie.fromArrowArray(arrow.vectorFromArray([1n], new arrow.Int64()), day),
      /whole-day/,
    )
    const midnight = arrow.vectorFromArray([86_400_000n], new arrow.Int64())
    assert.equal(Serie.fromArrowArray(midnight, day).length, 1)
    ```

## Eager and lazy

A cast of held data is eager and a cast of a stream is lazy, and the type says which.
`Serie::from_arrow_reader` drains a stream into one column: a column is one contiguous set of
buffers, so the bound is the stream itself. `SerieReader::from_arrow_reader` compiles one plan
from the stream's schema before a batch is pulled - a planning failure is raised there - and then
yields one record `Serie` per batch as it is pulled, holding at most one source batch, so a
resource larger than memory casts in bounded memory. A batch's failure therefore surfaces when
*that batch* is pulled, not when the reader is built - and the reader is fused after it, because a
stream that has reported it cannot be honoured has nothing further to say. The inner reader is
dropped at that point, which releases a C stream behind it.

`into_arrow_reader` is the stream's transport face: its batches reconciled to the root as they
are pulled and never landed, so no row is read beyond what the cast itself reads. Over an
identity plan it is the inner reader, handed back untouched.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::{ArrowCastOptions, DataType, Nullability, Serie, SerieReader, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ])?)
    .required_field("row");
    let schema = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
    ]));
    let quoted = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int32Array::from(vec![1])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("AAPL")])) as ArrayRef,
        ],
    )?;
    let unquoted = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int32Array::from(vec![2])) as ArrayRef,
            Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef,
        ],
    )?;

    // Eager: the stream is drained here, into one column.
    let stream = batch_reader(Arc::clone(&schema), [quoted.clone(), unquoted.clone()]);
    let held = Serie::from_arrow_reader(Some(&root), stream, ArrowCastOptions::new())?;
    assert_eq!(held.len(), 2);

    // Lazy: one plan compiled now, and nothing cast until a batch is pulled.
    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);
    let stream = batch_reader(Arc::clone(&schema), [quoted.clone(), unquoted, quoted]);
    let mut series = SerieReader::from_arrow_reader(Some(&root), stream, strict)?;
    assert_eq!(series.field(), &root);
    assert_eq!(series.next().transpose()?.map(|serie| serie.len()), Some(1));

    // The refusal arrives with the batch that carries it, and the reader is
    // fused after it.
    let refusal = series.next().expect("a second batch").unwrap_err();
    assert!(refusal.to_string().contains("$.symbol"), "{refusal}");
    assert!(series.next().is_none());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, Serie, SerieReader

    root = Field("row", DataType("struct<id: int64, symbol: string not null>"), False)
    table = pa.table({
        "id": pa.array([1, 2], pa.int32()),
        "symbol": ["AAPL", None],
    })

    # Eager: a table is drained here, into one column.
    held = Serie.from_arrow_reader(table, root)
    assert held.as_py() == [{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": ""}]

    # Lazy: one plan compiled now, and nothing cast until a batch is pulled.
    series = SerieReader.from_arrow_reader(
        table.to_reader(max_chunksize=1), root, nullability="strict"
    )
    assert series.field == root
    assert next(series).as_py() == [{"id": 1, "symbol": "AAPL"}]

    # The refusal arrives with the batch that carries it.
    try:
        next(series)
    except ValueError as error:
        assert "$.symbol" in str(error), error
    else:
        raise AssertionError("the null must be refused at the pull")

    # The transport face is a pyarrow reader that casts as it is read.
    reader = SerieReader.from_arrow_reader(table, root, nullability="strict").into_arrow_reader()
    assert reader.schema.names == ["id", "symbol"]
    try:
        reader.read_all()
    except pa.ArrowInvalid as error:
        assert "$.symbol" in str(error), error
    else:
        raise AssertionError("the null must be refused at the pull")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, Serie, SerieReader, fields } = require('yggdryl')

    const root = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8 not null')],
      { nullable: false },
    )
    const batch = (id, symbol) =>
      new arrow.Table({
        id: arrow.vectorFromArray([id], new arrow.Int32()),
        symbol: arrow.vectorFromArray([symbol], new arrow.Utf8()),
      }).batches
    const source = () => new arrow.Table([...batch(1, 'AAPL'), ...batch(2, null)])

    // Eager: the stream is drained here, into one column.
    const held = Serie.fromArrowReader(BatchReader.from(source()), root)
    assert.deepEqual(held.child('symbol').asJs(), ['AAPL', ''])

    // Lazy: one plan compiled now, and nothing cast until a batch is pulled.
    const series = SerieReader.fromArrowReader(BatchReader.from(source()), root, {
      nullability: 'strict',
    })
    assert.ok(series.field.equals(root))
    const pulled = series[Symbol.iterator]()
    assert.deepEqual(pulled.next().value.child('id').asJs(), [1])

    // The refusal arrives with the batch that carries it, and the reader is
    // fused after it.
    assert.throws(() => pulled.next(), /required Arrow field \$\.symbol holds 1 null values/)
    assert.equal(pulled.next().done, true)
    ```

## The generic cast

`cast` is the one cast of a column in hand: any column casts into any field its layout reaches,
through one compiled plan. A column already under the target is itself, and a schema-free run is
refused, because it lays out no buffers for a plan to read - [`Serie::from_scalars`](serie.md) is
the door that types a run's rows. Python also takes a `DataType`, as its required `value` field.

What each binding door accepts, each resolved once at the door:

| Door | Python | JavaScript |
| --- | --- | --- |
| `from_arrow_array` / `fromArrowArray` | a pyarrow `Array`, or anything exporting the Arrow C array interface | an Arrow JS `Vector`, chunks cast as one column |
| `from_arrow_batch` / `fromArrowBatch` | a pyarrow `RecordBatch` | an Arrow JS `RecordBatch` or `Table` |
| `from_arrow_reader` / `fromArrowReader`, `SerieReader` | a pyarrow `RecordBatchReader`, `Table`, `RecordBatch`, `Dataset` or `Scanner`, an Arrow C stream exporter, a pandas or polars frame, or an iterable of any of those | a native `BatchReader`; `BatchReader.from(value)` converts anything else |
| `cast` | a `Field`, a field expression, a pyarrow `Field`, or a `DataType` | a `Field` or a field expression |

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, DataType, Field, Nullability, Scalar, Serie};

    let ids = Serie::from_scalars(
        Field::new("id", DataType::Int32, true),
        [Scalar::from(1_i32), Scalar::Null],
    )?;

    // A bare datatype is carried as its required `value` field.
    let value = DataType::Int64.required_field("value");
    let wide = ids.cast(&value, ArrowCastOptions::new())?;
    assert_eq!(wide.as_int64().expect("an int64 column").values(), &[1, 0]);
    let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);
    let message = ids.cast(&value, strict).unwrap_err().to_string();
    assert_eq!(message, "required Arrow field $.value holds 1 null values");

    // The column's own field is the column itself; a run has no layout to cast.
    assert_eq!(ids.cast(&Field::new("id", DataType::Int32, true), strict)?, ids);
    assert!(Serie::new(vec![Scalar::from(1_i32)]).cast(&value, strict).is_err());
    ```

=== "Python"

    ```python
    import pandas as pd
    import pyarrow as pa

    from yggdryl import DataType, Field, Serie

    root = Field("row", DataType("struct<id: int64, symbol: string>"), False)

    # A frame is a stream of batches, drained into one column.
    frame = pd.DataFrame({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})
    assert Serie.from_arrow_reader(frame, root).child("id").as_py() == [1, 2]

    # A DataType is its required `value` field, and a refusal names it.
    ids = Serie.from_arrow_array(pa.array([1, None], pa.int32()))
    assert ids.cast(DataType("int64")).as_py() == [1, 0]
    try:
        ids.cast(DataType("int64"), nullability="strict")
    except ValueError as error:
        assert str(error) == "required Arrow field $.value holds 1 null values"
    else:
        raise AssertionError("a strict cast must refuse the null")

    # A run has no layout to cast.
    try:
        Serie([1, 2]).cast(DataType("int64"))
    except ValueError as error:
        assert "run" in str(error), error
    else:
        raise AssertionError("a run is refused")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    // A vector crossing as several chunks is cast once, as one column.
    const chunked = arrow
      .vectorFromArray([1, 2], new arrow.Int32())
      .concat(arrow.vectorFromArray([3], new arrow.Int32()))
    const ids = Serie.fromArrowArray(chunked, fields.int64('id'))
    assert.deepEqual(ids.asJs(), [1, 2, 3])

    // The column's own field is the column itself; a run has no layout to cast.
    assert.ok(ids.cast(ids.field).equals(ids))
    assert.throws(() => new Serie([1, 2]).cast(fields.int64('id')), /run/)
    ```

## Edges

- Extra column -> dropped under both policies; missing nullable -> nulls under both policies.
- Missing required -> canonical default under `default`; refused at compile time under `strict`.
- Null in a required column -> canonical default under `default`; refused at that batch under `strict`, with the null count.
- Null hidden inside a null parent row -> stays hidden; strictness reads only the exposed rows.
- `safe=True` plus `strict` -> the failed conversion becomes a null and the null is then refused; `safe=False` refuses the conversion first.
- An empty text cell into a column that holds neither text nor bytes -> null before `safe` is asked, under every text layout and through the scalar door; a required column then defaults it under `default` and refuses it by path under `strict`. Whitespace is a spelling, not an empty cell. Into a string, byte or interval column, or a code whose neutral member is the empty text, it is the value it is.
- A `Null` datatype under `strict` -> null is its only value, so it is not absence.
- A `DataType` target -> its required `value` field, so a refusal names `$.value`.
- `into_arrow_scalar` -> exactly one row; any other length is refused naming it, and a run is refused by name.
- Nullable field, `safe` -> the null stays.
- A scalar wider than the declared type -> accepted when the value fits, then canonicalized into it (`U64` -> `I64`).
- Text into `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`, `Duration64` -> everything [text](../media/index.md#json) accepts, a duration included, which Arrow reads into none.
- Text into a decimal -> read at the declared scale and refused when a digit would be dropped, on both tiers; Arrow's rounding is never the answer.
- Text into a boolean or a number at the row tier -> this crate's canonical spelling; a column keeps Arrow's wider vocabulary behind it, as it does for temporals.
- Two fixed sizes, list or binary -> a value change rather than a layout change, refused by name.
- A string target declaring a bound, a fixed width or a charset other than UTF-8 -> `StringIngest`: every cell validated, a `yggdryl.string` source read under its own parameters first, bare binary storage read as bytes already in the target charset; a bounded variable byte target -> `BytesIngest`, every cell's length checked ([String](text/string.md#casts) and [Bytes](text/bytes.md#casts) casts). Under `safe` a refused cell is null, under strict the row and column are named.
- A byte source entering a code or a UUID -> read as bytes under all four binary framings, so a payload that is not US-ASCII is refused rather than nulled under strict. A fixed slot is trimmed of the padding it wrote, except into `uuid` at sixteen bytes, where every byte carries identity.
- A code or a UUID source entering a string -> read as the text the code holds and as the canonical spelling of the identifier, under the target's own layout, charset and bound: one `StringIngest`, not a second renderer per source.
- A fixed-width byte target -> `BytesIngest` too: a cell that does not fill the width exactly is refused naming the field, the row and both lengths, rather than left to Arrow's builder to complain about a slice. A source whose own width is declared and disagrees is refused at plan time instead.
- A dictionary or run-end target -> its values' own rule runs, then the encoding; a `dictionary<int32, ascii>` refuses what `ascii` refuses.
- An encoded source into a plain target -> decoded first, so a dictionary of a recognized code still renders as text.
- A bare null into a `union` or a `run_end_encoded` -> refused: both spell absence inside a child, so the value is the pair or the values entry that carries it.
- A reading the declared unit or width cannot hold exactly -> null, never a rounded value.
- Twelve-hour clock, and a bare date into a zoned datetime -> Arrow's kernel; a bare date into a naive datetime is that day at midnight on both tiers, compact `YYYYMMDD` included.
- Temporal to text -> the classic form, zoned instants included.
- `representation="bits"` over two different widths, or into a datatype with a value rule -> the ordinary conversion, range check and all.
- A required `bits` target over source nulls -> the canonical default under `default`, refused by path under `strict`; the buffer is rebuilt only when a null is actually filled.
- A foreign column carrying a `yggdryl.*` extension label -> its rows read once under the field's rule, a refused row named with its column and row under every option; a label is never a proof.
- A column of another layout handed to a compiled plan -> error naming both layouts; a plan is compiled for one source.
- `SerieReader::into_arrow_reader` whose plan is the identity, under `default` -> the inner reader itself, unwrapped; under `strict` it is wrapped, because a non-null Arrow field can still carry a logical null in a nested child.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cast::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test serie -- arrow::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test allocations -- cast
    cargo bench --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --bench types -- cast_plan
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_cast.py python/tests/test_serie.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/cast.test.js node/tests/serie.test.js
    ```

## Performance

### Compiling a plan once

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

Both paths assert identical rows before timing. The custom median gate runs
only in an optimized, explicit benchmark invocation; it refuses a reused
plan taking more than 1.25 times the rebuilt plan's duration. All-target
tests keep the row assertions without warm-up, samples or timing thresholds.

```bash
cargo bench --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --bench types -- cast_plan
```

### Row canonicalization

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
