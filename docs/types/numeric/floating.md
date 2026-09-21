# Floating

The three IEEE widths - `float16`, `float32`, `float64` - held bit-for-bit, totally ordered, and hashable.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `float16`, `float32`, `float64`, and the `Float16`/`Float32`/`Float64` values behind them |
| Validates | At the value door: a width is one of 16, 32 or 64, and an integer is not a float |
| Lazy | Nothing - a width carries no parameter to resolve |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | A bit width that is not 16, 32 or 64; an integer where a float is declared, in JavaScript through a dedicated factory because `100` and `100.0` are one `Number` there |
| Normalizes | Every NaN payload at construction, so two NaNs are one value; the two zeros stay distinct, because a codec must round-trip the exact bits |

## DataType

Three parameter-free variants, each spelled several ways by the grammar and displayed one way.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    assert_eq!(DataType::Float64.to_string(), "float64");
    assert_eq!(DataType::Float64.id(), DataTypeId::Float64);
    assert_eq!(DataType::Float64.kind(), DataTypeKind::Floating);

    assert_eq!(DataType::from_str("double")?, DataType::Float64);
    assert_eq!(DataType::from_str("double precision")?, DataType::Float64);
    assert_eq!(DataType::from_str("real")?, DataType::Float32);
    assert_eq!(DataType::from_str("float")?, DataType::Float32);
    assert_eq!(DataType::from_str("half")?, DataType::Float16);

    // A FIX price or quantity is a name for `float64`.
    assert_eq!(DataType::from_logical_name("Price")?, DataType::Float64);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    assert str(DataType("float64")) == "float64"
    assert DataType("float64").id == "float64"
    assert DataType("float64").kind == "floating"
    assert DataType("float64").is_floating

    assert DataType("double") == DataType("float64")
    assert DataType("double precision") == DataType("float64")
    assert DataType("real") == DataType("float32")
    assert DataType("float") == DataType("float32")
    assert DataType("halffloat") == DataType("float16")

    assert DataType.from_logical_name("Price") == DataType("float64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.from('float64').toString(), 'float64')
    assert.equal(DataType.from('float64').kind, 'floating')

    assert.ok(DataType.from('double').equals(DataType.from('float64')))
    assert.ok(DataType.from('real').equals(DataType.from('float32')))
    assert.ok(DataType.from('halffloat').equals(DataType.from('float16')))

    assert.equal(DataType.fromLogicalName('Price').toString(), 'float64')
    ```

| datatype | also spelled | Arrow storage | bytes |
| --- | --- | --- | ---: |
| `float16` | `half`, `halffloat` | `Float16` | 2 |
| `float32` | `float`, `real` | `Float32` | 4 |
| `float64` | `double`, `double precision` | `Float64` | 8 |

The FIX names resolving to `float64` are `Qty`, `Price`, `PriceOffset`, `Percentage` and `Amt`; the specification states no scale for them, so they are a view for arithmetic and an exact column declares a [decimal](decimal.md) instead.

## Field

`Float16Field`, `Float32Field` and `Float64Field` are `FieldOf<Float16Type>` and its siblings: no parameter, so `unit(name, nullable)` is the whole constructor.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, Field, Float32Field, Float64Field};

    let price = Float64Field::unit("px", false);
    assert_eq!(price.name(), "px");
    assert_eq!(price.dtype(), &DataType::Float64);
    assert!(!price.is_nullable());

    let root: Field = price.into_field();
    assert!(Float64Field::from_field(&root).is_some());
    assert!(Float32Field::from_field(&root).is_none());
    ```

=== "Python"

    ```python
    from yggdryl import Field, types

    price = types.float64("px", nullable=False)
    ratio = types.float32("ratio", metadata={"unit": "percent"})

    assert isinstance(price, Field)
    assert str(price.dtype) == "float64"
    assert not price.nullable
    assert str(types.float16("compact").dtype) == "float16"
    assert ratio.metadata["unit"] == "percent"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const price = fields.float64('px', { nullable: false })
    assert.ok(price instanceof Field)
    assert.equal(price.dtype.toString(), 'float64')
    assert.equal(price.nullable, false)
    assert.equal(fields.float16('compact').dtype.toString(), 'float16')
    ```

## Scalar

A float keeps the width it arrived at, because widening it would erase which width the column declared. `as_f64` widens exactly and so answers for all three; `as_f32` answers for the two narrow ones, and narrowing is left to the caller, where the loss is visible.

=== "Rust"

    ```rust
    use yggdryl::{Float32, Float64, Scalar};

    let price = Scalar::from(1.5_f64);
    assert_eq!(price, Scalar::Float64(Float64::from_f64(1.5)));
    assert_eq!(price.kind(), "f64");
    assert_eq!(price.as_f64(), Some(1.5));

    // The narrow widths widen exactly, and the wide one does not narrow here.
    let narrow = Scalar::from(1.5_f32);
    assert_eq!(narrow.kind(), "f32");
    assert_eq!(narrow.as_f64(), Some(1.5));
    assert_eq!(narrow.as_f32(), Some(1.5));
    assert_eq!(price.as_f32(), None);
    assert_eq!(Float32::from_f32(1.5).as_f64(), 1.5);

    // One width is picked by number of bits where a caller has only an `f64`.
    assert_eq!(Scalar::from_float(1.5, 16)?.kind(), "f16");
    assert_eq!(Scalar::from_float(1.5, 16)?.as_f32(), Some(1.5));
    assert!(Scalar::from_float(1.5, 8).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    price = DataType("float64").scalar(1.5)
    assert price.kind == "f64"
    assert price.as_float() == 1.5
    assert price.as_py() == 1.5

    # A bare Python float is `float64` until a column names a narrower width.
    assert Scalar.from_(1.5).kind == "f64"
    assert DataType("float32").scalar(1.5).kind == "f32"
    assert DataType("float16").scalar(1.5).kind == "f16"

    # Narrowing rounds, and the rounding is what the column then holds.
    assert DataType("float32").scalar(0.1).as_float() == 0.10000000149011612
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    // `100` and `100.0` are one Number here, so the float factory is the door:
    // a bare integral Number would be read as an integer and refused.
    assert.equal(Scalar.float(1.5, 32).kind, 'f32')
    assert.equal(Scalar.float(1.5, 32).asJs(), 1.5)
    assert.equal(Scalar.float(1.5, 16).kind, 'f16')
    assert.equal(Scalar.from(1.5).kind, 'f64')

    assert.throws(() => DataType.from('float64').scalar(100), /expected float64/)
    assert.throws(() => Scalar.float(1.5, 8), /float bit width must be 16, 32, or 64/)
    ```

## Arrow storage

Each width is Arrow's own - `Float16`, `Float32`, `Float64` - and imports back as itself; pyarrow spells the narrow one `halffloat`.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::Float32.into_arrow_datatype()?, arrow_schema::DataType::Float32);
    assert_eq!(
        DataType::from_arrow_datatype(&arrow_schema::DataType::Float16)?,
        DataType::Float16,
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    assert DataType("float32").into_arrow() == pa.float32()
    assert DataType("float16").into_arrow() == pa.float16()
    assert DataType.from_arrow(pa.float64()) == DataType("float64")

    assert DataType("float16").fixed_byte_width == 2
    assert DataType("float64").fixed_byte_width == 8
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.fromArrow({ toString: () => 'float32' }).toString(), 'float32')
    assert.equal(DataType.from('float16').fixedByteWidth, 2)
    assert.equal(DataType.from('float64').fixedByteWidth, 8)
    ```

## NaN, infinities and the two zeros

Every NaN payload is normalized at construction, so a float is `Eq`, `Ord` and `Hash` and two NaNs are one value; the infinities are ordinary values that read back as themselves. Positive and negative zero stay distinct, because a codec must round-trip the exact representation.

=== "Rust"

    ```rust
    use yggdryl::{Float64, Scalar};

    let nan = Scalar::from(f64::NAN);
    assert_eq!(nan, Scalar::from(f64::NAN));
    assert!(nan.as_f64().is_some_and(f64::is_nan));

    assert_eq!(Scalar::from(f64::INFINITY).as_f64(), Some(f64::INFINITY));
    assert_eq!(Scalar::from(f64::NEG_INFINITY).as_f64(), Some(f64::NEG_INFINITY));

    // The two zeros are two representations, so they stay two values.
    assert_ne!(Scalar::from(0.0_f64), Scalar::from(-0.0_f64));
    assert_eq!(Float64::from_f64(-1.5).abs(), Float64::from_f64(1.5));
    ```

=== "Python"

    ```python
    import math

    from yggdryl import DataType

    nan = DataType("float64").scalar(float("nan"))
    assert nan == DataType("float64").scalar(float("nan"))
    assert math.isnan(nan.as_float())

    assert DataType("float64").scalar(math.inf).as_float() == math.inf
    assert DataType("float64").scalar(0.0) != DataType("float64").scalar(-0.0)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.ok(Scalar.float(NaN, 64).equals(Scalar.float(NaN, 64)))
    assert.equal(Scalar.float(Infinity, 64).asJs(), Infinity)
    assert.equal(Scalar.float(-0, 64).equals(Scalar.float(0, 64)), false)
    ```

## Casts

Text reads into a float through the `f64` spelling - which names every finite value the three widths hold, plus the infinities and NaN - and the declared width then rounds it. Another number converts by the value it spells.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Float32Array, StringArray};
    use yggdryl::{ArrowCastOptions, Float32Field};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["1.5", "-0.25"]));
    let typed = Float32Field::unit("px", false);
    let prices: Float32Array = typed.cast_arrow_array(text, ArrowCastOptions::new().with_safe(false))?;
    assert_eq!(prices.values(), &[1.5, -0.25]);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    prices = Field("px", "float32").cast_arrow_array(pa.array(["1.5", "-0.25"]))
    assert prices.to_pylist() == [1.5, -0.25]

    # safe nulls what does not convert; strict refuses it instead.
    assert Field("px", "float64").cast_arrow_array(pa.array(["1.5", "x"])).to_pylist() == [
        1.5,
        None,
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const text = arrow.vectorFromArray(['1.5', '-0.25'], new arrow.Utf8())
    assert.deepEqual([...fields.float64('px').castArrowArray(text)], [1.5, -0.25])
    ```

A `float64` and an `int64` are the same eight bytes under two readings, so `representation: "bits"` carries the bit pattern rather than the number; that reading is [Cast](../cast.md#reading-the-bits)'s.

## Edges

- A float beside an exact [decimal](decimal.md) never merges: an exact number and an approximate one have no meeting point that is not a re-encoding. A float beside an integer merges at the float.
- Arithmetic retains the widest float input; mixing an integer in uses `f64` ([Scalar](../scalar.md#variants-and-arithmetic)).
- Narrowing rounds: `0.1` stored at `float32` reads back as `0.10000000149011612`, at every binding.
- `Scalar::from_float(value, bits)` accepts 16, 32 and 64 and refuses everything else; JavaScript spells it `Scalar.float(value, width)`.
- JavaScript has no float literal distinct from an integer one, so `new DataType('float64').scalar(100)` is refused and `Scalar.float(100, 64)` is the spelling. Python needs no such factory, because `100.0` is its own literal.
- `as_f32` answers for `float16` and `float32` and `None` for `float64`: widening is exact, narrowing is not, and the loss belongs where the caller can see it.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::floating field::floating
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::scalar default_scalar
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- arithmetic::tests
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_factories.py python/tests/types/test_native_scalar.py
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="typed field factories cover every native datatype variant" node/tests/types/fields.test.js
    npm run --prefix node bench:types
    ```
