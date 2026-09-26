# Integer

Eight Arrow widths - four signed, four unsigned - each a parameter-free datatype, a field marker, and a `Scalar` variant of its own.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, and the `Int128`/`UInt128` values a wide computation lands in |
| Validates | At the value door: a magnitude the declared width cannot hold is refused, never wrapped |
| Lazy | Nothing - a width carries no parameter to resolve and no payload to build |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | An out-of-range magnitude, a float or a decimal where an integer is declared, and division by zero |
| Errors | Rust `Error::InvalidRecord` at the value door, `Error::ArithmeticOverflow` and `Error::DivisionByZero` in arithmetic; Python `ValueError`, `OverflowError`, `ZeroDivisionError`; JavaScript throws |

## DataType

Every width is a variant that carries nothing, so the enum is the constructor and there is no parameter to validate. The [grammar](../datatype.md#logical-names) spells one width several ways and displays exactly one.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    let id = DataType::Int32;
    assert_eq!(id.to_string(), "int32");
    assert_eq!(id.id(), DataTypeId::Int32);
    assert_eq!(id.kind(), DataTypeKind::Integer);
    assert!(id.is_integer() && DataType::UInt8.is_integer());
    assert!(DataTypeId::Int32.is_signed_integer());
    assert!(DataTypeId::UInt8.is_unsigned_integer());

    // One width, several spellings, one display.
    assert_eq!(DataType::from_str("bigint")?, DataType::Int64);
    assert_eq!(DataType::from_str("int")?, DataType::Int32);
    assert_eq!(DataType::from_str("utinyint")?, DataType::UInt8);
    assert_eq!(DataType::from_str("smallint")?.to_string(), "int16");

    // A FIX name is one more spelling of the width it means.
    assert_eq!(DataType::from_logical_name("SeqNum")?, DataType::Int64);
    assert_eq!(DataType::from_logical_name("DayOfMonth")?, DataType::Int8);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    id_type = DataType("int32")
    assert str(id_type) == "int32"
    assert id_type.id == "int32"
    assert id_type.kind == "integer"
    assert id_type.is_integer and id_type.is_signed_integer
    assert DataType("uint8").is_unsigned_integer

    assert DataType("bigint") == DataType("int64")
    assert DataType("int") == DataType("int32")
    assert DataType("utinyint") == DataType("uint8")
    assert str(DataType("smallint")) == "int16"

    assert DataType.from_logical_name("SeqNum") == DataType("int64")
    assert DataType.from_logical_name("DayOfMonth") == DataType("int8")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const id = DataType.from('int32')
    assert.equal(id.toString(), 'int32')
    assert.equal(id.id, 'int32')
    assert.equal(id.kind, 'integer')

    assert.ok(DataType.from('bigint').equals(DataType.from('int64')))
    assert.ok(DataType.from('utinyint').equals(DataType.from('uint8')))
    assert.equal(DataType.from('smallint').toString(), 'int16')

    assert.equal(DataType.fromLogicalName('SeqNum').toString(), 'int64')
    ```

### The spellings one width answers

| datatype | also spelled | `DataTypeId` |
| --- | --- | --- |
| `int8` | `tinyint`, `byte` | `Int8` |
| `int16` | `smallint`, `short` | `Int16` |
| `int32` | `int`, `integer` | `Int32` |
| `int64` | `bigint`, `long` | `Int64` |
| `uint8` | `utinyint`, `unsignedtinyint` | `UInt8` |
| `uint16` | `usmallint`, `unsignedsmallint` | `UInt16` |
| `uint32` | `uint`, `unsignedint`, `unsignedinteger` | `UInt32` |
| `uint64` | `ubigint`, `unsignedbigint` | `UInt64` |

Case, `_`, `-` and spaces fold, so `unsigned_bigint` and `UnsignedBigInt` are the same keyword; the spelling must still be one word to the tokenizer, so write `unsignedbigint` or `ubigint` rather than two words. The FIX names resolving here are `Length`, `TagNum`, `NumInGroup`, `Reserved100Plus`, `Reserved1000Plus` and `Reserved4000Plus` at `int32`, `SeqNum` at `int64` and `DayOfMonth` at `int8`.

## Field

A width has no parameter, so the leaf is named and nothing is passed: `Int8Field` through `UInt64Field` are `FieldOf<Int8Type>` .. `FieldOf<UInt64Type>`, built with `unit(name, nullable)`.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, Field, Int64Field, UInt8Field};

    let sequence = Int64Field::unit("seqnum", false);
    assert_eq!(sequence.name(), "seqnum");
    assert_eq!(sequence.dtype(), &DataType::Int64);
    assert!(!sequence.is_nullable());

    // The root enum is the leaves, so narrowing is a match and not a check.
    let root: Field = sequence.into_field();
    assert!(Int64Field::from_field(&root).is_some());
    assert!(UInt8Field::from_field(&root).is_none());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    sequence = yggdryl.int64("seqnum", nullable=False)
    counter = yggdryl.uint32("count", metadata={"unit": "rows"})

    assert isinstance(sequence, Field)
    assert str(sequence.dtype) == "int64"
    assert not sequence.nullable
    assert counter.metadata["unit"] == "rows"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const sequence = fields.int64('seqnum', { nullable: false })
    const counter = fields.uint32('count', { metadata: { unit: 'rows' } })

    assert.ok(sequence instanceof Field)
    assert.equal(sequence.dtype.toString(), 'int64')
    assert.equal(sequence.nullable, false)
    assert.equal(counter.get('unit'), 'rows')
    ```

## Scalar

A native integer keeps its width - an `i32` is `Scalar::Int32`, not an `Int64` that happens to fit - because the width is what a column declaration reads back. The readers answer across widths and `None` rather than wrapping, and one number at two widths is one value.

=== "Rust"

    ```rust
    use yggdryl::{Int32, Scalar};

    let held = Scalar::from(7_i32);
    assert_eq!(held, Scalar::Int32(Int32::new(7)));
    assert_eq!(held.kind(), "i32");
    assert!(held.is_integer());
    assert_eq!(Int32::new(7).get(), 7);

    // The readers answer across widths, and answer `None` instead of wrapping.
    assert_eq!(held.as_i64(), Some(7));
    assert_eq!(held.as_u128(), Some(7));
    assert_eq!(Scalar::from(-1_i64).as_u64(), None);
    assert_eq!(Scalar::from(u64::MAX).as_i64(), None);

    // One number at two widths is one value, and hashes as one.
    assert_eq!(Scalar::from(7_u8), Scalar::from(7_i32));
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    held = DataType("int32").scalar(7)
    assert held.kind == "i32"
    assert held.id == "int32"
    assert held.as_int() == 7
    assert held.as_py() == 7
    assert held.is_integer()

    # A bare Python integer takes the widest signed width until a column names one.
    assert Scalar.from_(7).kind == "i64"

    # One number at two widths is one value, and hashes as one.
    assert Scalar.from_(7) == held
    assert DataType("uint8").scalar(7) == held
    assert Scalar.from_(7).stable_hash() == held.stable_hash()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const held = DataType.from('int32').scalar(7)
    assert.equal(held.kind, 'i32')
    assert.equal(held.id, 'int32')
    assert.equal(held.family, 'integer')
    assert.equal(held.asJs(), 7)

    // One number at two widths is one value, and hashes as one.
    assert.ok(held.equals(Scalar.from(7)))
    assert.ok(held.equals(DataType.from('uint8').scalar(7)))
    assert.equal(held.stableHash(), Scalar.from(7).stableHash())
    ```

## Arrow storage

Every width is one of Arrow's own and imports back as itself, so the round trip is the identity and the stored bytes per value are the width.

| datatype | Arrow storage | bytes |
| --- | --- | ---: |
| `int8` / `uint8` | `Int8` / `UInt8` | 1 |
| `int16` / `uint16` | `Int16` / `UInt16` | 2 |
| `int32` / `uint32` | `Int32` / `UInt32` | 4 |
| `int64` / `uint64` | `Int64` / `UInt64` | 8 |

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::Int16.into_arrow_datatype()?, arrow_schema::DataType::Int16);
    assert_eq!(DataType::UInt64.into_arrow_datatype()?, arrow_schema::DataType::UInt64);
    assert_eq!(
        DataType::from_arrow_datatype(&arrow_schema::DataType::UInt8)?,
        DataType::UInt8,
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    assert DataType("int16").into_arrow() == pa.int16()
    assert DataType("uint64").into_arrow() == pa.uint64()
    assert DataType.from_arrow(pa.uint8()) == DataType("uint8")

    # The width is the layout, so one value's stored bytes are known.
    assert DataType("int32").fixed_byte_width == 4
    assert DataType("int64").fixed_byte_width == 8
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // Any Apache Arrow JS type is read through its own textual form.
    assert.equal(DataType.fromArrow({ toString: () => 'uint8' }).toString(), 'uint8')

    assert.equal(DataType.from('int32').fixedByteWidth, 4)
    assert.equal(DataType.from('int64').fixedByteWidth, 8)
    ```

## Casts and overflow

The declared width is the target: text reads into it, another number converts into it by the value it spells, and a magnitude it cannot hold is an error rather than a wrap. `safe` and the column's nullability then decide what a failure becomes, on [Cast](../cast.md#required-columns).

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let strict = ArrowCastOptions::new().with_safe(false);
    let id = Field::new("id", DataType::Int32, false);

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));
    let ids = Serie::from_arrow_array(Some(&id), text, strict)?;
    assert_eq!(ids.as_int32().expect("an int32 column").values(), &[1, 2]);

    // A magnitude the width cannot hold is refused, in a column and in a row.
    let wide: ArrayRef = Arc::new(StringArray::from(vec!["2147483648"]));
    let refused = Serie::from_arrow_array(Some(&id), wide, strict).unwrap_err().to_string();
    assert!(refused.contains("$.id"), "{refused}");
    assert!(DataType::Int32.scalar(2_147_483_648_i64).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, Serie

    id_ = Field("id", "int32")
    ids = Serie.from_arrow_array(pa.array(["1", "2"]), id_)
    assert ids.into_arrow_array().equals(pa.array([1, 2], type=pa.int32()))

    # Surrounding space is not part of the number.
    assert Serie.from_arrow_array(pa.array([" 7 "]), id_).as_py() == [7]

    with pytest.raises(ValueError, match=r"\$\.id"):
        Serie.from_arrow_array(pa.array(["2147483648"]), id_, safe=False)

    with pytest.raises(ValueError, match="expected int32"):
        DataType("int32").scalar(2**31)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const ids = Serie.fromArrowArray(utf8(['1', '2']), fields.int32('id'))
    assert.deepEqual([...ids.intoArrowArray()], [1, 2])

    const wide = utf8(['2147483648'])
    assert.throws(() => Serie.fromArrowArray(wide, fields.int32('id'), { safe: false }), /\$\.id/)
    assert.throws(() => DataType.from('int32').scalar(2 ** 31), /expected int32/)
    ```

Two widths of the same size are the same eight or four bytes under two readings, so `representation: "bits"` carries the bit pattern instead of the number: `u64::MAX` reads as `-1`, and back. That reading is [Cast](../cast.md#reading-the-bits)'s.

## Arithmetic

Two integers meet at a width that holds both - mixed signs promote only where the promotion is lossless - and the result keeps that width. An overflow of it is an error, not a wrap.

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    assert_eq!(Scalar::from(2_i32).checked_mul(&Scalar::from(3_i32))?, Scalar::from(6_i32));
    assert_eq!(Scalar::from(-1_i8).checked_add(&Scalar::from(2_u8))?, Scalar::from(1_i16));

    assert!(Scalar::from(i32::MAX).checked_add(&Scalar::from(1_i32)).is_err());
    assert!(Scalar::from(1_i32).checked_div(&Scalar::from(0_i32)).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Scalar

    assert (Scalar.from_(40) + 2).as_py() == 42
    assert (Scalar.from_(6) * 7).as_py() == 42

    with pytest.raises(OverflowError):
        DataType("int32").scalar(2147483647) + DataType("int32").scalar(1)

    with pytest.raises(ZeroDivisionError):
        Scalar.from_(1) / Scalar.from_(0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    assert.equal(Scalar.from(40).add(2).asJs(), 42)
    assert.equal(Scalar.from(6).multiply(7).asJs(), 42)

    assert.throws(
      () => DataType.from('int32').scalar(2147483647).add(DataType.from('int32').scalar(1)),
      /overflows i32/,
    )
    assert.throws(() => Scalar.from(1).divide(Scalar.from(0)), /division by zero/)
    ```

## Beyond sixty-four bits

`Int128` and `UInt128` are values without a datatype of their own: Arrow has no 128-bit integer, so the datatype such a value answers is the narrowest exact [decimal](decimal.md) at scale zero that holds its digits. Rust computes wider still in `i256`/`u256`, which are the decimals' arithmetic and are documented [there](decimal.md#the-256-bit-pair).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Int128, Scalar, Value as _};

    let wide = Int128::new(i128::from(u64::MAX) + 1);
    let held = wide.into_scalar();

    assert_eq!(held.kind(), "i128");
    assert_eq!(held.as_i128(), Some(18_446_744_073_709_551_616));
    assert_eq!(held, Scalar::from(18_446_744_073_709_551_616_i128));

    // Twenty digits, so twenty is the precision a column would declare.
    assert_eq!(wide.dtype()?, DataType::decimal(20, 0)?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    held = Scalar.from_(2**64)
    assert held.kind == "i128"
    assert held.as_int() == 2**64

    # No `int128` datatype: the digits are the precision a column declares.
    assert held.dtype == DataType.decimal(20, 0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    const held = Scalar.from(2n ** 64n)
    assert.equal(held.kind, 'i128')
    assert.equal(held.dtype.toString(), 'decimal128(20,0)')
    ```

## Edges

- A magnitude outside the declared width -> refused at the value door and at the column door; nothing ever wraps.
- `as_u64` on a negative value -> `None`; `as_i64` on `u64::MAX` -> `None`. A wider integer that does not fit answers `None` rather than losing magnitude.
- Reading an integer out of text takes the widest signed then unsigned storage the crate has, and the declared width narrows it; surrounding space is not part of the number.
- Division or remainder by zero -> `Error::DivisionByZero`, Python `ZeroDivisionError`, a JavaScript throw; an overflow of the shared width -> `Error::ArithmeticOverflow`, Python `OverflowError`.
- `int16`, `int32` and `int64` are the only widths a [run-end](../nested/runend.md) column may index with.
- An `int64` or `uint64` crosses into JavaScript as a `BigInt` wherever the value may exceed `Number.MAX_SAFE_INTEGER`; a small one reads back as a `Number`.
- `unsigned tinyint` written as two words is refused by the tokenizer - fold it to one keyword (`unsignedtinyint`) or use `utinyint`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- integer parser::aliases
    cargo test --features "iceberg internals parquet" --manifest-path rust/Cargo.toml -p yggdryl --test root -- arithmetic
    cargo bench --manifest-path rust/Cargo.toml --bench types -- 'typed/integer'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^arrow_integer_bits/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py python/tests/test_scalar.py
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="the bits reading crosses every same-width pair" node/tests/fields.test.js
    node --test --test-name-pattern="typed field factories cover every native datatype variant" node/tests/fields.test.js
    npm run --prefix node bench:types
    ```
