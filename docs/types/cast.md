# Cast

The [field](field.md) is the cast target: an Arrow array, a record batch, a stream or a [`Serie`](serie.md) already in hand is reconciled to its datatype and nullability, and what comes out is a `Serie` under that field.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ArrowCastPlan`, `ArrowCastOptions`, `Representation`; `validate_value` and `canonicalize_value` for rows |
| Ways in | `Serie::cast` for a column in hand; `ChunkedSerie::cast` for chunked columns, one plan over every chunk, and `ArrowCastPlan::apply_chunked` for a held one; `Serie::from_arrow_array`, `from_arrow_batch`, `from_arrow_reader` for Arrow buffers; `SerieReader::from_arrow_reader` for a stream; an `ArrowCastPlan` held and applied wherever one cast repeats |
| Target | The field, never the source. A `DataType` target is its required `value` field (`dtype.required_field("value")`), so a refusal names `$.value` |
| Returns | A `Serie` under the target field - a `ChunkedSerie` of as many chunks from `ChunkedSerie::cast` and `apply_chunked`. A typed read is a narrowing of it: `as_int64().values()`, `as_utf8()`, `as_date32()`, `as_fixed_bytes()` |
| Exact input | The identity plan: the same buffers, and a column already under the target is itself - where the source states every nullability the target requires; a nullable source under a required target is read for its nulls |
| `safe` | Whether a *present* value may be converted into a column that may hold null. `true`: a failed conversion becomes null; `false`: error |
| `representation` | What a *same-width* pair carries. `value`: the number it spells, range-checked; `bits`: the bytes under it, buffer shared |
| Absence | No option: the target field's own nullability, one rule at every door - the engine's, a `Selector` projection with a datatype, a `Plan` `create` section, a derived column, the record options' `field`, the stored field a write completes onto, an Iceberg table's schema. A nullable column holds null; a required one refuses a null, an empty text cell and a column the source does not carry by path, and never writes its canonical default ([Required columns](#required-columns)) |
| Independent | `safe` and the field compose: a failed conversion becomes null only where the column may hold one, so a required column refuses a value it cannot convert by that value whatever `safe` says; `representation` never changes what absence means |
| Empty text | A zero-length text cell entering a non-text column is null before `safe` is asked; the field's nullability decides the rest. A string, byte or interval column, and a code whose neutral member is the empty text, keep it as the value it is |
| Validates | `validate_value`: right arity, no null in a required column, every scalar in its declared range |
| One reading | A row and a column read the same spellings: text into a number, a boolean, a decimal or a temporal; any value with a spelling into text; any byte-carrying value into a byte layout |
| Nested as JSON | A scalar cast and a column cast read a struct, serie or map into text or bytes as its JSON, and text or bytes back as the JSON document it holds; the value contract does not, a cast's reading rather than a value's ([Nested values as JSON](#nested-values-as-json)) |
| Layouts | Every serie layout reads every other one, every byte framing reads every other one, and an encoding is a layout: a dictionary or run-end target runs its values' rule, and an encoded source is read as the column it holds |
| Batch children | Target order, ASCII-case-insensitive names |
| Proof | A landed column holds only rows its field accepts; an extension label is never proof of that ([What a landing proves](#what-a-landing-proves)) |
| Errors | The dot/bracket path of the first misfit, from the cast root: `$.users[].zip`; a column is its own first segment, `$.id` |
| Bindings | `Serie`, `ChunkedSerie`, `SerieReader` and `ArrowCastPlan` in Rust, Python and JavaScript, the two options by name; `Scalar` rows in Rust and Python |

## Use

An array enters as the column of a field. `safe` decides whether a failed conversion errors or becomes null, and only a column that may hold a null lets it become one: a required column refuses the value instead ([Required columns](#required-columns)).

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let field = Field::new("id", DataType::Int64, false);
    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));

    // One door: the array lands as the column of `field`, cast on the way in.
    let unsafe_cast = ArrowCastOptions::new().with_safe(false);
    let ids = Serie::from_arrow_array(Some(&field), text, unsafe_cast)?;
    assert_eq!(ids.field(), Some(&field));
    // A typed read is a narrowing of the column that came out.
    assert_eq!(ids.as_int64().expect("an int64 column").values(), &[1, 2]);

    // safe nulls a failed conversion where the column may hold a null.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    let nullable = Field::new("id", DataType::Int64, true);
    let nulled =
        Serie::from_arrow_array(Some(&nullable), Arc::clone(&broken), ArrowCastOptions::new())?;
    assert_eq!(nulled.null_count(), 1);
    assert!(nulled.is_null(1)?);
    assert!(Serie::from_arrow_array(Some(&nullable), Arc::clone(&broken), unsafe_cast).is_err());

    // A required column has nowhere to put that null, so it refuses the
    // value it cannot convert, by that value, whatever safe says.
    for options in [ArrowCastOptions::new(), unsafe_cast] {
        let refused = Serie::from_arrow_array(Some(&field), Arc::clone(&broken), options)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("not a number"), "{refused}");
    }

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

    # safe nulls a failed conversion where the column may hold a null.
    broken = pa.array(["1", "not a number"])
    nullable = Field("id", "int64")
    nulled = Serie.from_arrow_array(broken, nullable)
    assert nulled.as_py() == [1, None]
    assert nulled.null_count() == 1

    try:
        Serie.from_arrow_array(broken, nullable, safe=False)
    except ValueError:
        pass
    else:
        raise AssertionError("an unsafe cast must fail")

    # A required column has nowhere to put that null, so it refuses the
    # value it cannot convert, by that value, whatever safe says.
    for safe in (True, False):
        try:
            Serie.from_arrow_array(broken, field, safe=safe)
        except ValueError as error:
            assert "not a number" in str(error), error
        else:
            raise AssertionError("a required column must refuse the value")

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

    // safe nulls a failed conversion where the column may hold a null.
    const broken = arrow.vectorFromArray(['1', 'not a number'], new arrow.Utf8())
    const nullable = fields.int64('id')
    assert.deepEqual(Serie.fromArrowArray(broken, nullable).asJs(), [1, null])
    assert.throws(() => Serie.fromArrowArray(broken, nullable, { safe: false }), /not a number/)

    // A required column has nowhere to put that null, so it refuses the
    // value it cannot convert, by that value, whatever safe says.
    for (const options of [{ safe: true }, { safe: false }]) {
      assert.throws(() => Serie.fromArrowArray(broken, field, options), /not a number/)
    }

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
keeps that rule: eight arbitrary bytes are not a currency merely because a currency may be eight bytes.

Absence is unaffected: the reading says what the bytes mean, and the target field's nullability
still says whether a value may be absent ([Required columns](#required-columns)).

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
the column tier reads, but for a nested value and text, which meet only in a cast
([Nested values as JSON](#nested-values-as-json)). Text becomes the number, boolean, decimal or temporal a column declares -
through this crate's own readers, so a digit a scale cannot hold is refused rather than rounded.
Any value that prints a spelling enters a text column as that spelling, a geometry included.
Any value that carries bytes enters a byte column as that payload; a fixed string carries the
width it declares and pads to it on the way out, because that is what the fixed column stores.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, StructType};

    let money: DataType = "decimal128(10, 2)".parse()?;
    assert_eq!(money.scalar("10.50")?, Scalar::decimal128(1_050, 2));
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
    assert row.kind == "serie"
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

## Required columns

Whether a value may be absent is no option: it is the target field's own nullability, and every
cast answers it one way. A nullable column holds null - a column the source does not carry is
all-null, a null stays one, and a value it cannot convert becomes null under `safe`. A required
column holds a value in every exposed row. It refuses a column the source does not carry, a null,
and an [empty text cell](#empty-text) entering a non-text column, naming the full dot/bracket path
from the cast root; and it refuses a present value it cannot convert by that value whatever `safe`
says, because the null a lenient conversion would leave has nowhere to stand. It never writes its
canonical [default](datatype.md#default-values) in place of a value it could not read.

The failures happen at different times: a required field *no source column carries* is decided by
the fields alone and so is refused when the cast is compiled, before any batch exists; a required
field *holding null* is a property of the rows and is refused, with its null count, when that
batch is cast.

- A datatype whose own canonical default is null - `null`, or an encoding whose values hold only
  nulls - keeps its nulls under a required field: there null is its value, not an absence.
- An undeclared source column is dropped: the rule is about what the target declares, not about
  what the source carries beyond it.
- The one repair is internal: a [digest](../hashing.md) holder, which `as_digest().apply_arrow_batch`
  fills after the cast that lands its batch, may arrive absent for that fill. A `TRANSFORM:` or
  `PARTITION:` derived column is an ordinary column to the cast, and
  [`Field::apply_arrow_batch`](field.md#applying-a-schema) is this cast alone.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCastOptions, ArrowCastPlan, DataType, Field, Serie, StructType};

    let options = ArrowCastOptions::new();
    let root = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ])?)
    .required_field("row");

    // A required column the source does not carry: two fields are all it
    // takes to know, so it is refused when the plan is compiled.
    let ids = Arc::new(Schema::new(vec![ArrowField::new("id", ArrowDataType::Int32, false)]));
    let source = Field::from_arrow_schema("row", &ids)?;
    let message = ArrowCastPlan::compile(&source, &root, options)
        .unwrap_err()
        .to_string();
    assert_eq!(message, "required Arrow field $.symbol is missing from the source");

    // A missing nullable column is all-null instead.
    let optional = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let batch = RecordBatch::try_new(
        Arc::clone(&ids),
        vec![Arc::new(Int32Array::from(vec![1])) as ArrayRef],
    )?;
    let filled = Serie::from_arrow_batch(Some(&optional), &batch, options)?;
    assert_eq!(filled.child("symbol").map(Serie::null_count), Some(1));

    // A null in a required column is a property of the rows, refused when
    // that batch is cast, by path and count.
    let quotes = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            ArrowField::new("id", ArrowDataType::Int32, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, true),
        ])),
        vec![
            Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("AAPL"), None])) as ArrayRef,
        ],
    )?;
    let message = Serie::from_arrow_batch(Some(&root), &quotes, options)
        .unwrap_err()
        .to_string();
    assert_eq!(message, "required Arrow field $.symbol holds 1 null values");

    // A value a required column cannot convert is refused by that value,
    // even under safe; a nullable column takes it as null.
    let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));
    let required = DataType::Int64.required_field("id");
    let refused = Serie::from_arrow_array(Some(&required), Arc::clone(&broken), options)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("not a number"), "{refused}");
    let nullable = DataType::Int64.nullable_field("id");
    assert!(Serie::from_arrow_array(Some(&nullable), broken, options)?.is_null(1)?);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import ArrowCastPlan, DataType, Field, Serie

    root = Field("row", DataType("struct<id: int64, symbol: string not null>"), False)

    # A required column the source does not carry is refused by path, and
    # refused where the plan is compiled: two schemas are all it takes.
    ids = pa.record_batch({"id": pa.array([1], pa.int32())})
    try:
        Serie.from_arrow_batch(ids, root)
    except ValueError as error:
        assert str(error) == "required Arrow field $.symbol is missing from the source"
    else:
        raise AssertionError("a required column must refuse the missing column")
    try:
        ArrowCastPlan(ids.schema, root)
    except ValueError as error:
        assert "$.symbol" in str(error), error
    else:
        raise AssertionError("the plan must refuse the missing column")

    # A missing nullable column is all-null instead.
    optional = Field("row", DataType("struct<id: int64, symbol: string>"), False)
    assert Serie.from_arrow_batch(ids, optional).as_py() == [{"id": 1, "symbol": None}]

    # A null in a required column is refused when that batch is cast, by
    # path and count.
    quotes = pa.record_batch({"id": pa.array([1, 2], pa.int32()), "symbol": ["AAPL", None]})
    try:
        Serie.from_arrow_batch(quotes, root)
    except ValueError as error:
        assert str(error) == "required Arrow field $.symbol holds 1 null values"
    else:
        raise AssertionError("a required column must refuse the null")

    # A value a required column cannot convert is refused by that value,
    # even under safe; a nullable column takes it as null.
    broken = pa.array(["1", "not a number"])
    try:
        Serie.from_arrow_array(broken, Field("id", "int64", nullable=False))
    except ValueError as error:
        assert "not a number" in str(error), error
    else:
        raise AssertionError("a required column must refuse the value")
    assert Serie.from_arrow_array(broken, Field("id", "int64")).as_py() == [1, None]
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

    // A required column the source does not carry is refused by path, and
    // refused where the plan is compiled: two schemas are all it takes.
    const ids = new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) })
    const missing = /required Arrow field \$\.symbol is missing from the source/
    assert.throws(() => Serie.fromArrowBatch(ids, root), missing)
    assert.throws(() => ArrowCastPlan.compile(ids.schema, root), missing)

    // A missing nullable column is all-null instead.
    const optional = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )
    assert.deepEqual(Serie.fromArrowBatch(ids, optional).child('symbol').asJs(), [null])

    // A null in a required column is refused when that batch is cast, by
    // path and count.
    const quotes = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })
    assert.throws(
      () => Serie.fromArrowBatch(quotes, root),
      /required Arrow field \$\.symbol holds 1 null values/,
    )

    // A value a required column cannot convert is refused by that value,
    // even under safe; a nullable column takes it as null.
    const broken = arrow.vectorFromArray(['1', 'not a number'], new arrow.Utf8())
    assert.throws(
      () => Serie.fromArrowArray(broken, fields.int64('id', { nullable: false })),
      /not a number/,
    )
    assert.deepEqual(Serie.fromArrowArray(broken, fields.int64('id')).asJs(), [1, null])
    ```

## Empty text

An empty text cell entering a column that does not hold text is no value. It reads as null
through every door - the Arrow walk and the scalar door alike - before any spelling is parsed
and before `safe` is consulted: an empty cell is not a failed conversion, because there was
nothing to convert. The field's nullability then decides what that null is, exactly as for a
null the source carried: a nullable column holds it and a required one refuses it by path
([Required columns](#required-columns)). Only zero-length text is empty; whitespace is text.
The integer, float, decimal and boolean readers read past surrounding whitespace, so ` 1 ` is
`1`, and whitespace alone is a spelling none of them takes: a failed conversion, null in a
nullable column under `safe` and refused naming the cell otherwise. The date and timestamp
readers trim nothing, so ` 2024-01-02 ` fails the same way. A string leaf, a byte leaf and an
interval keep the empty cell as what it is, and so does a code whose neutral member is the
empty text, which is also that code's canonical default. A reader that takes only a plain text
layout - version, url, urn, timezone, mimetype, mediatype under a dictionary source - answers a
column with no visible value as a null column, so a required target is refused by path there
too.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Scalar, Serie};

    let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
    let unsafe_cast = ArrowCastOptions::new().with_safe(false);

    // Nullable: null, and `safe = false` never sees the cell.
    let nullable = DataType::Int64.nullable_field("count");
    assert!(Serie::from_arrow_array(Some(&nullable), Arc::clone(&empty), unsafe_cast)?.is_null(0)?);

    // Required: refused by path whatever `safe` says, never given its default.
    let required = DataType::Int64.required_field("count");
    for options in [ArrowCastOptions::new(), unsafe_cast] {
        let message = Serie::from_arrow_array(Some(&required), Arc::clone(&empty), options)
            .unwrap_err()
            .to_string();
        assert_eq!(message, "required Arrow field $.count holds 1 null values");
    }

    // The scalar door answers the same, and a text column keeps the cell.
    assert_eq!(DataType::Int64.scalar("")?, Scalar::Null);
    assert_eq!(DataType::utf8().scalar("")?, Scalar::from(""));

    // Whitespace is text: a number reads past it, and whitespace alone is a
    // failed conversion - null under `safe`, refused in a required column.
    let spaced: ArrayRef = Arc::new(StringArray::from(vec![" 1 ", " "]));
    let read = Serie::from_arrow_array(Some(&nullable), Arc::clone(&spaced), ArrowCastOptions::new())?;
    assert_eq!(read.scalar(0)?, Scalar::from(1_i64));
    assert!(read.is_null(1)?);
    let refused = Serie::from_arrow_array(Some(&required), spaced, ArrowCastOptions::new());
    assert!(refused.unwrap_err().to_string().contains("Cannot cast string ' '"));
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, Serie

    empty = pa.array([""])

    # Nullable: null, and safe=False never sees the cell.
    nullable = Field("count", "int64")
    assert Serie.from_arrow_array(empty, nullable, safe=False).null_count() == 1

    # Required: refused by path whatever safe says, never given its default.
    required = Field("count", "int64", nullable=False)
    for safe in (True, False):
        try:
            Serie.from_arrow_array(empty, required, safe=safe)
        except ValueError as error:
            assert str(error) == "required Arrow field $.count holds 1 null values"
        else:
            raise AssertionError("a required column must refuse the empty cell")

    # The scalar door answers the same, and a text column keeps the cell.
    assert DataType("int64").scalar("").as_py() is None
    assert DataType("utf8").scalar("").as_py() == ""

    # Whitespace is text: a number reads past it, and whitespace alone is a
    # failed conversion - null under safe, refused in a required column.
    spaced = pa.array([" 1 ", " "])
    assert Serie.from_arrow_array(spaced, nullable).as_py() == [1, None]
    try:
        Serie.from_arrow_array(spaced, required)
    except ValueError as error:
        assert "Cannot cast string ' '" in str(error)
    else:
        raise AssertionError("a required column must refuse whitespace alone")
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

    // Required: refused by path whatever safe says, never given its default.
    const required = fields.int64('count', { nullable: false })
    for (const options of [{ safe: true }, { safe: false }]) {
      assert.throws(
        () => Serie.fromArrowArray(empty, required, options),
        /required Arrow field \$\.count holds 1 null values/,
      )
    }

    // The scalar door answers the same, and a text column keeps the cell.
    assert.equal(DataType.from('int64').scalar('').kind, 'null')
    assert.equal(DataType.utf8().scalar('').asJs(), '')

    // Whitespace is text: a number reads past it, and whitespace alone is a
    // failed conversion - null under safe, refused in a required column.
    const spaced = arrow.vectorFromArray([' 1 ', ' '], new arrow.Utf8())
    assert.deepEqual(Serie.fromArrowArray(spaced, nullable).asJs(), [1, null])
    assert.throws(() => Serie.fromArrowArray(spaced, required), /Cannot cast string ' '/)
    ```

## Nested values as JSON

A nested column - a struct, any serie layout, a map - and a text or byte column cast into one
another through JSON, at every depth. Each row writes the compact JSON its field names: a struct
is an object keyed by its fields in declaration order, a serie an array, a map an object whose
keys are spelled as text, a union beneath one its `[type_id, value]` pair, and every leaf the
spelling the [JSON codec](../media/json.md) writes - a decimal as its text, a temporal in
ISO 8601, bytes in base64. Each text or byte cell reads back as one JSON document under the
target field, through the value contract a JSON document is read by, so a cast there and back
is the identity - but for a map's entries, which a JSON object holds in no order of its own
([Edges](#edges)). The cast recurses like every other: a struct beneath a map's values spells its
own object where the map holds it, so `map<utf8, struct<..>>` and `map<utf8, utf8>` are one cast
apart in either direction, and a list of records and a list of their text the same.

The JSON is written straight into the column's one buffer from the column's own leaves - no
value and no text built per row - and the rows of a column that already landed are not proven
again. Reading, the target is planned once: each document is read along that plan through the
one JSON grammar, every leaf through its datatype's own value door, a struct's cells into the
columns its children are laid out from and a serie's items or a map's entries onto one run, so
no row value is built either. A document the plan cannot answer for alone - a struct spelled as a positional
array, a union or an encoding anywhere in the target, a refusal - is read by the field-directed
door instead, which says what a refusal is. A target with a rule of its own - a
charset, a bound, a fixed width - then runs it over the JSON, so `ascii` refuses a non-ASCII
character by row. An empty cell and the document `null` are absence, for the field's
nullability to answer; a cell that is not JSON, or not a value of the target, and a float JSON
cannot spell are failed conversions ([Required columns](#required-columns)). A union reads text
through the member that takes it, and a variant holds text as the string it is, so neither reads
a document where a cast meets it.

A scalar cast reads and writes the same JSON - `cast_scalar`, `try_cast_scalar` and an
expression's `cast`, whose rows agree with a cast of their column. A value carries no field, so
it spells the JSON its own shape is - a record an object in name order, a run an array - where a
column's rows are keyed by their field. The value contract, `DataType::scalar` and
`Field::scalar`, is not a cast: every document reader runs it, and there a nested value has no
text spelling and text is no nested value, so a repeated field never lands in a text column as an
array's JSON.

=== "Rust"

    ```rust
    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

    let options = ArrowCastOptions::new();
    let books = Field::new(
        "books",
        "map<utf8, struct<px: decimal128(10, 2), qty: int64>>".parse()?,
        true,
    );
    let quote = Scalar::from_struct([("px", Scalar::from("1.50")), ("qty", Scalar::from(3_i64))])?;
    let column = Serie::from_scalars(
        books.clone(),
        [Scalar::from_mapping([(Scalar::from("AAPL"), quote)])?, Scalar::Null],
    )?;

    // Each record beneath the map's values spells its own object.
    let text = column.cast(&Field::new("books", "map<utf8, utf8>".parse()?, true), options)?;
    assert_eq!(
        text.scalar(0)?,
        Scalar::from_mapping([(Scalar::from("AAPL"), Scalar::from(r#"{"px":"1.5","qty":3}"#))])?
    );
    assert!(text.is_null(1)?);
    // And the text reads back under the records: the round trip is the identity.
    assert_eq!(text.cast(&books, options)?, column);

    // Text that is not a document of the target is a failed conversion.
    let lists = Serie::from_scalars(
        Field::new("lots", DataType::utf8(), true),
        [Scalar::from("[1, 2]"), Scalar::from("abc"), Scalar::from("null")],
    )?;
    let lots = lists.cast(&Field::new("lots", "serie<int64>".parse()?, true), options)?;
    assert_eq!(lots.scalar(0)?, Scalar::from_sequence([Scalar::from(1_i64), Scalar::from(2_i64)]));
    assert!(lots.is_null(1)? && lots.is_null(2)?);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    quote = pa.struct([("px", pa.float64()), ("sym", pa.string())])
    quotes = Serie.from_arrow_array(pa.array([{"px": 1.5, "sym": "AAPL"}, None], quote))

    # A struct spells an object keyed in declaration order...
    text = quotes.cast(Field("q", "utf8"))
    assert text.as_py() == ['{"px":1.5,"sym":"AAPL"}', None]
    # ...and the text reads back under the struct.
    assert text.cast(Field("q", "struct<px: float64, sym: utf8>")).as_py() == [
        {"px": 1.5, "sym": "AAPL"},
        None,
    ]

    # Text that is not a document of the target is a failed conversion.
    lots = Serie.from_arrow_array(pa.array(["[1, 2]", "abc", "null"]))
    assert lots.cast(Field("lots", "serie<int64>")).as_py() == [[1, 2], None, None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Serie } = require('yggdryl')

    const quote = Field.from('q: struct<px: float64, sym: utf8>')
    const quotes = Serie.fromScalars(quote, [quote.dtype.scalar({ px: 1.5, sym: 'AAPL' }), null])

    // A struct spells an object keyed in declaration order...
    const text = quotes.cast(Field.from('q: utf8'))
    assert.deepEqual(text.asJs(), ['{"px":1.5,"sym":"AAPL"}', null])
    // ...and the text reads back under the struct.
    assert.deepEqual(text.cast(quote).asJs(), [{ px: 1.5, sym: 'AAPL' }, null])
    ```

## Compiled plans

Everything a cast decides from two fields - which source child answers which target field, the
child order, the recursive type dispatch, the target's Arrow projection, the kernel options - is a
function of those fields alone. `ArrowCastPlan` is that work made once: `compile` from a source
field, a target field and the options; `preflight` to exercise the whole recursion over no rows;
`apply` per column of the source layout, each landing a `Serie` under the target, and `apply_chunked`
per chunked column, every chunk under the one plan. A batch's schema
is a source as the record it lays out as: `Field::from_arrow_schema("row", &schema)`. The plan is
immutable and `Send + Sync`, so one serves every column of a stream and every thread of a
parallel scan; only the masks, offsets, and dictionary reachability a column actually carries vary.

`Serie::cast` and the `Serie` Arrow doors compile one plan for their one input, so a loop that
calls them compiles per iteration. A loop holds the plan instead - and `SerieReader` already
does, so a stream never plans twice, and `SerieReader::cast` re-roots a stream under one more.
A plan also resolves the target's tree once - every level's field and projection - and lands
each column under that tree, so what a landing proves per batch is the buffers alone: the
validity words, and each row of a leaf whose layout is not its datatype's whole contract. A
`Serie` Arrow door handed a record root that is exactly the batch's schema lands the same way,
resolving the tree for that one call. `as_source` answers the Arrow field an input must lay out
as, `as_target` the field every cast lands under, `as_options` the two answers, and
`is_identity` whether the plan hands every input of its source layout straight back.

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
    assert.deepEqual(plan.options, { safe: true, representation: 'value' })

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
    assert labelled.schema.field("u").type.extension_name == "yggdryl.url"
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
    use yggdryl::{ArrowCastOptions, DataType, Serie, SerieReader, StructType};

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

    // Eager: the stream is drained here, into one column, so a batch the
    // cast refuses fails the whole call.
    let stream = batch_reader(Arc::clone(&schema), [quoted.clone(), quoted.clone()]);
    let held = Serie::from_arrow_reader(Some(&root), stream, ArrowCastOptions::new())?;
    assert_eq!(held.len(), 2);
    let stream = batch_reader(Arc::clone(&schema), [quoted.clone(), unquoted.clone()]);
    assert!(Serie::from_arrow_reader(Some(&root), stream, ArrowCastOptions::new()).is_err());

    // Lazy: one plan compiled now, and nothing cast until a batch is pulled.
    let stream = batch_reader(Arc::clone(&schema), [quoted.clone(), unquoted, quoted]);
    let mut series = SerieReader::from_arrow_reader(Some(&root), stream, ArrowCastOptions::new())?;
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

    # Eager: a table is drained here, into one column, so a null the
    # required column refuses fails the whole call.
    held = Serie.from_arrow_reader(table.slice(0, 1), root)
    assert held.as_py() == [{"id": 1, "symbol": "AAPL"}]
    try:
        Serie.from_arrow_reader(table, root)
    except ValueError as error:
        assert "$.symbol" in str(error), error
    else:
        raise AssertionError("the null must be refused while draining")

    # Lazy: one plan compiled now, and nothing cast until a batch is pulled.
    series = SerieReader.from_arrow_reader(table.to_reader(max_chunksize=1), root)
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
    reader = SerieReader.from_arrow_reader(table, root).into_arrow_reader()
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

    // Eager: the stream is drained here, into one column, so a null the
    // required column refuses fails the whole call.
    const held = Serie.fromArrowReader(BatchReader.from(new arrow.Table(batch(1, 'AAPL'))), root)
    assert.deepEqual(held.child('symbol').asJs(), ['AAPL'])
    assert.throws(
      () => Serie.fromArrowReader(BatchReader.from(source()), root),
      /required Arrow field \$\.symbol holds 1 null values/,
    )

    // Lazy: one plan compiled now, and nothing cast until a batch is pulled.
    const series = SerieReader.fromArrowReader(BatchReader.from(source()), root)
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
    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

    let options = ArrowCastOptions::new();
    let ids = Serie::from_scalars(
        Field::new("id", DataType::Int32, true),
        [Scalar::from(1_i32), Scalar::Null],
    )?;

    // A nullable target keeps the null.
    let wide = ids.cast(&Field::new("id", DataType::Int64, true), options)?;
    assert_eq!((wide.len(), wide.is_null(1)?), (2, true));

    // A bare datatype is carried as its required `value` field, so the null
    // is refused by that path.
    let value = DataType::Int64.required_field("value");
    let message = ids.cast(&value, options).unwrap_err().to_string();
    assert_eq!(message, "required Arrow field $.value holds 1 null values");

    // The column's own field is the column itself; a run has no layout to cast.
    assert_eq!(ids.cast(&Field::new("id", DataType::Int32, true), options)?, ids);
    assert!(Serie::new(vec![Scalar::from(1_i32)]).cast(&value, options).is_err());
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

    # A nullable target keeps the null; a DataType is its required `value`
    # field, and the refusal of the null names it.
    ids = Serie.from_arrow_array(pa.array([1, None], pa.int32()))
    assert ids.cast(Field("id", "int64")).as_py() == [1, None]
    try:
        ids.cast(DataType("int64"))
    except ValueError as error:
        assert str(error) == "required Arrow field $.value holds 1 null values"
    else:
        raise AssertionError("a required column must refuse the null")

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

- Extra column -> dropped; missing nullable -> nulls.
- Missing required -> refused at compile time, naming its path; never its canonical default.
- Null in a required column -> refused at that batch, with its path and the null count; never its canonical default.
- Null hidden inside a null parent row -> stays hidden; a required column reads only the exposed rows.
- A value the target cannot convert -> null in a nullable column under `safe`; refused by that value in a required column whatever `safe` says, and in any column under `safe=False`.
- An empty text cell into a column that holds neither text nor bytes -> null before `safe` is asked, under every text layout and through the scalar door; a required column then refuses it by path. Whitespace is a spelling, not an empty cell. Into a string, byte or interval column, or a code whose neutral member is the empty text, it is the value it is.
- A `Null` datatype, or an encoding whose values hold only nulls, under a required field -> null is its canonical default, so it is not absence and stays.
- A digest holder under `as_digest().apply_arrow_batch` -> may arrive absent, landing as its canonical default for the fill to replace; nothing else repairs an absence, a `TRANSFORM:` or `PARTITION:` column included.
- A `DataType` target -> its required `value` field, so a refusal names `$.value`.
- `into_arrow_scalar` -> exactly one row; any other length is refused naming it, and a run is refused by name.
- A scalar wider than the declared type -> accepted when the value fits, then canonicalized into it (`U64` -> `I64`).
- Text into `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`, `Duration64` -> everything [text](../media/json.md#read) accepts, a duration included, which Arrow reads into none.
- Text into a decimal -> the one exact [decimal grammar](numeric/decimal.md#text) - a sign, `.5` or `5.`, leading and trailing zeros, an exponent, `_` grouping between the digits ahead of the point - read at the declared scale and refused when a non-zero digit would be dropped, on both tiers; Arrow's rounding is never the answer, and a comma, `NaN` and `inf` are no decimal.
- A float into a decimal -> the number its shortest text names, rounded half away from zero at the declared scale, on both tiers: `0.125` into `decimal(10, 2)` is `0.13`, and `1.15` is `1.15` where Arrow's kernel would scale the binary fraction. A `nan`, an infinity or a float past the precision -> null under `safe`, refused by row under strict.
- A decimal column into text -> the shortest exact text on both tiers, every width and both fixed leaves alike: `1.125` for `decimal128(38,18)` and for `decimal`, never the `1.125000000000000000` of the storage, and `100` for `100.00`; the scale stays in the datatype, and the text reads back at it.
- Text into a boolean at either tier -> the one [boolean table](numeric/boolean.md#the-one-text-reader), Arrow's String-to-Boolean vocabulary (`true`/`false`, `yes`/`no`, `y`/`n`, `on`/`off`, `1`/`0` and their prefixes), case-insensitive and trimmed, read by the crate rather than Arrow's kernel, with no allocation per row; strict, a cell outside it is refused naming the field and the row, `expected true/false, yes/no, y/n, on/off or 1/0`. Text into a number at the row tier -> this crate's spelling (a sign, blanks around it, an exponent for a float); a column keeps Arrow's vocabulary behind it, as it does for temporals.
- Two fixed sizes, serie or binary -> a value change rather than a layout change, refused by name.
- A string target declaring a bound, a fixed width or a charset other than UTF-8 -> `StringIngest`: every cell validated, a `yggdryl.string` source read under its own parameters first, bare binary storage read as bytes already in the target charset; a bounded variable byte target -> `BytesIngest`, every cell's length checked ([String](text/string.md#casts) and [Bytes](text/bytes.md#casts) casts). Under `safe` a refused cell is null in a nullable column; in a required one, or under `safe=False`, the row and column are named.
- A byte source entering a code or a UUID -> read as bytes under all four binary framings, so a payload that is not US-ASCII is a refused value: null in a nullable column under `safe`, named in a required one. A fixed slot is trimmed of the padding it wrote, except into `uuid` at sixteen bytes, where every byte carries identity.
- A code or a UUID source entering a string -> read as the text the code holds and as the canonical spelling of the identifier, under the target's own layout, charset and bound: one `StringIngest`, not a second renderer per source.
- A fixed-width byte target -> `BytesIngest` too: a cell that does not fill the width exactly is refused naming the field, the row and both lengths, rather than left to Arrow's builder to complain about a slice. A source whose own width is declared and disagrees is refused at plan time instead.
- A dictionary or run-end target -> its values' own rule runs, then the encoding; a `dictionary<int32, ascii>` refuses what `ascii` refuses.
- An encoded source into a plain target -> decoded first, so a dictionary of a recognized code still renders as text.
- A bare null into a `run_end_encoded` -> refused: it spells absence inside its values child, so the value is the entry that carries it.
- A bare null into a `union` -> the payload of its `null` member, else of the one member that takes a null; with none, or several, it is refused naming the members ([Union](nested/union.md)).
- A reading the declared unit or width cannot hold exactly -> a failed conversion, never a rounded value: null in a nullable column under `safe`, refused otherwise.
- Twelve-hour clock, and a bare date into a zoned datetime -> Arrow's kernel; a bare date into a naive datetime is that day at midnight on both tiers, compact `YYYYMMDD` included.
- Temporal to text -> the classic form, zoned instants included.
- A struct, serie or map into text or bytes -> its compact JSON, a struct keyed in declaration order and every leaf the JSON codec's spelling; a NaN or an infinite float has no JSON and is a failed conversion ([Nested values as JSON](#nested-values-as-json)).
- Text or bytes into a struct, serie or map -> each cell one JSON document under the target, never a one-item list wrapped around the cell; `""` and the document `null` are absence; a JSON object holds no order, so a map's entries come back in the order of their keys' text and a sorted map's in the order of its keys.
- Text into a union or a variant -> the union's member that takes it, the variant's string; neither reads a document at the top of a cast.
- `representation="bits"` over two different widths, or into a datatype with a value rule -> the ordinary conversion, range check and all.
- A required `bits` target over source nulls -> refused by path, exactly as under `value`.
- A foreign column carrying a `yggdryl.*` extension label -> its rows read once under the field's rule, a refused row named with its column and row under every option; a label is never a proof.
- A column of another layout handed to a compiled plan -> error naming both layouts; a plan is compiled for one source.
- An equal layout whose source is nullable where the target is required -> not the identity: the plan reads for the null the source may hold.
- `SerieReader::into_arrow_reader` whose plan is the identity - the target's layout, every nullability included -> the inner reader itself, unwrapped; a batch only moved is its producer's claim, and a `Serie` landed from it is proven at its landing.

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

### Nested JSON text

A nested column into JSON text and back, per row: the commodity tape as one
`struct<symbol: utf8, price: decimal128(12, 4), size: int64>` column, a basket of four
`int64` sizes per row as a `serie<int64>`, and four marks per row as a `map<utf8, int64>`. The
baselines are Arrow's own list-to-text kernel over
the same basket - a display form, not JSON - and serde_json parsing every cell into its own value
tree with no column built. One containerized x86_64 Linux run: Intel Xeon @ 2.80 GHz, 4 cores,
16 GiB; rustc 1.97.0 release with thin LTO. Criterion medians; the container's run-to-run spread
is 10 to 20%.

| Rows | Write struct | Write serie | Arrow list kernel | Read struct | serde_json struct | Read serie | serde_json serie |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1,024 | 175 µs | 73.8 µs | 102 µs | 532 µs | 283 µs | 327 µs | 144 µs |
| 16,384 | 2.96 ms | 1.19 ms | 1.70 ms | 9.13 ms | 4.49 ms | 4.95 ms | 2.29 ms |

| Rows | Write map | Read map | serde_json map |
| ---: | ---: | ---: | ---: |
| 1,024 | 195 µs | 931 µs | 297 µs |
| 16,384 | 3.07 ms | 15.5 ms | 5.08 ms |

Writing is the JSON spelled from the column's leaves straight into its one buffer: the basket
writes as JSON faster than Arrow's kernel writes its display text, which is no JSON at all.
Reading is each document walked once along the target planned for it, its values landing in the
buffers the column is laid out from; serde_json's parse of the same cells, which builds no
column, is the baseline beside each, about half the read - a third for the map, whose every row
also puts its entries in the order of their keys and holds them to one key each. Neither
direction allocates per row.

```bash
cargo bench --manifest-path rust/Cargo.toml -p yggdryl --bench arrow -- arrow_serie_json
```

### Row canonicalization

Row canonicalization over a three-column row - `utf8`, `binary`, `ccy` - at two payload
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
