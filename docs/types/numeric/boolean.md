# Boolean

One bit of logic, and the `null` datatype beside it: the two variants that carry no payload at all.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `boolean` and `null`, the `Boolean` value and the one `Null` value |
| Validates | At the value door: a boolean is `true` or `false`, or the text that spells one of them |
| Lazy | Nothing - neither variant has a parameter, a child or a payload |
| Cached | The Arrow projection of a [`Field`](../field.md), built once per field |
| Refuses | Any text that is not `true` or `false` at the value door; a number, which is a coercion rather than a value |
| Kinds | `DataTypeKind::Boolean` and `DataTypeKind::Null`, and neither is numeric: logic and absence are not arithmetic |

## DataType

Two parameterless variants, so the enum is the constructor. `bool` and `void` are the second spellings.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, DataTypeKind};

    assert_eq!(DataType::Boolean.to_string(), "boolean");
    assert_eq!(DataType::Boolean.id(), DataTypeId::Boolean);
    assert_eq!(DataType::Boolean.kind(), DataTypeKind::Boolean);
    assert_eq!(DataType::Null.kind(), DataTypeKind::Null);
    assert!(!DataTypeKind::Boolean.is_numeric());

    assert_eq!(DataType::from_str("bool")?, DataType::Boolean);
    assert_eq!(DataType::from_str("void")?, DataType::Null);
    assert_eq!(DataType::from_str("void")?.to_string(), "null");
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    assert str(DataType("boolean")) == "boolean"
    assert DataType("boolean").id == "boolean"
    assert DataType("boolean").kind == "boolean"
    assert DataType("null").kind == "null"
    assert not DataType("boolean").is_numeric

    assert DataType("bool") == DataType("boolean")
    assert DataType("void") == DataType("null")
    assert str(DataType("void")) == "null"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.from('boolean').toString(), 'boolean')
    assert.equal(DataType.from('boolean').kind, 'boolean')
    assert.equal(DataType.from('null').kind, 'null')

    assert.ok(DataType.from('bool').equals(DataType.from('boolean')))
    assert.equal(DataType.from('void').toString(), 'null')
    ```

## Field

`BooleanField` and `NullField` are `FieldOf<BooleanType>` and `FieldOf<NullType>`: there is nothing to pass, so `unit(name, nullable)` is the whole constructor.

=== "Rust"

    ```rust
    use yggdryl::FieldValue as _;
    use yggdryl::{BooleanField, DataType, Field, NullField};

    let flag = BooleanField::unit("ok", false);
    assert_eq!(flag.name(), "ok");
    assert_eq!(flag.dtype(), &DataType::Boolean);
    assert!(!flag.is_nullable());

    let root: Field = flag.into_field();
    assert!(BooleanField::from_field(&root).is_some());
    assert!(NullField::from_field(&root).is_none());
    assert_eq!(NullField::unit("gap", true).dtype(), &DataType::Null);
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    flag = yggdryl.boolean("ok", nullable=False)
    gap = yggdryl.null("gap")

    assert isinstance(flag, Field)
    assert str(flag.dtype) == "boolean"
    assert not flag.nullable
    assert str(gap.dtype) == "null"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fields } = require('yggdryl')

    const flag = fields.boolean('ok', { nullable: false })
    assert.ok(flag instanceof Field)
    assert.equal(flag.dtype.toString(), 'boolean')
    assert.equal(flag.nullable, false)
    assert.equal(fields.null('gap').dtype.toString(), 'null')
    ```

## Scalar

`Boolean` wraps one `bool`, `Null` is the one absent value, and the pair is its own family: there is no leaf below either of them and no enum above. `as_bool` is a *reading* and answers only for a boolean, which is what keeps the `None` a filter's three-valued logic walks.

=== "Rust"

    ```rust
    use yggdryl::{Boolean, Scalar};

    let flag = Scalar::from(true);
    assert_eq!(flag, Scalar::Boolean(Boolean::new(true)));
    assert_eq!(flag.kind(), "boolean");
    assert_eq!(flag.as_bool(), Some(true));
    assert_eq!(Boolean::new(true).get(), true);

    // Absence is its own value, and it is not a boolean.
    let absent = Scalar::from(());
    assert!(absent.is_null());
    assert_eq!(absent.as_bool(), None);
    assert_eq!(Scalar::from(1_i32).as_bool(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    flag = DataType("boolean").scalar(True)
    assert flag.kind == "boolean"
    assert flag.family == "boolean"
    assert flag.as_bool() is True
    assert flag == Scalar.from_(True)

    # Absence is its own value, and it is not a boolean.
    absent = DataType("null").scalar(None)
    assert absent.is_null()
    assert absent.as_bool() is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const flag = DataType.from('boolean').scalar(true)
    assert.equal(flag.kind, 'boolean')
    assert.equal(flag.family, 'boolean')
    assert.equal(flag.asJs(), true)
    assert.ok(flag.equals(Scalar.from(true)))

    const absent = DataType.from('null').scalar(null)
    assert.equal(absent.kind, 'null')
    assert.equal(absent.asJs(), null)
    ```

## Arrow storage

Arrow's own two logic-free storages: a boolean is a bit per value, and a null column stores no buffer at all, so it has no fixed byte width to state.

=== "Rust"

    ```rust
    use yggdryl::DataType;

    assert_eq!(DataType::Boolean.into_arrow_datatype()?, arrow_schema::DataType::Boolean);
    assert_eq!(DataType::Null.into_arrow_datatype()?, arrow_schema::DataType::Null);
    assert_eq!(
        DataType::from_arrow_datatype(&arrow_schema::DataType::Boolean)?,
        DataType::Boolean,
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType

    assert DataType("boolean").into_arrow() == pa.bool_()
    assert DataType("null").into_arrow() == pa.null()
    assert DataType.from_arrow(pa.bool_()) == DataType("boolean")

    assert DataType("boolean").fixed_byte_width == 1
    assert DataType("null").fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.fromArrow({ toString: () => 'bool' }).toString(), 'boolean')
    assert.equal(DataType.from('boolean').fixedByteWidth, 1)
    assert.equal(DataType.from('null').fixedByteWidth, null)
    ```

## The value door reads one spelling

`true` and `false` are what a boolean prints, so they are what it reads; the case is not part of the spelling and surrounding space is not part of the value. Anything else is refused, because this door is the String-to-Boolean *cast* rather than the wider [truthiness](../scalar.md#truthiness-and-length) coercion.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar};

    assert_eq!(DataType::Boolean.scalar("TRUE")?, Scalar::from(true));
    assert_eq!(DataType::Boolean.scalar(" false ")?, Scalar::from(false));
    assert!(DataType::Boolean.scalar("off").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    assert DataType("boolean").scalar("TRUE").as_bool() is True
    assert DataType("boolean").scalar(" false ").as_bool() is False

    with pytest.raises(ValueError, match="expected boolean"):
        DataType("boolean").scalar("off")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    assert.equal(DataType.from('boolean').scalar('TRUE').asJs(), true)
    assert.equal(DataType.from('boolean').scalar(' false ').asJs(), false)
    assert.throws(() => DataType.from('boolean').scalar('off'), /expected boolean/)
    ```

## Truthiness is the other question

`is_truthy` is a coercion and answers for every value; `as_bool` is a reading and answers only for a boolean. Nothing falls back between them, and the whole rule is on [Scalar](../scalar.md#truthiness-and-length).

=== "Rust"

    ```rust
    use yggdryl::Scalar;

    assert!(Scalar::from(true).is_truthy());
    assert!(!Scalar::from(0).is_truthy());
    assert!(!Scalar::from("OFF").is_truthy());
    assert_eq!(Scalar::from("OFF").as_bool(), None);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar

    assert bool(Scalar.from_(True))
    assert not bool(Scalar.from_(0))
    assert not bool(Scalar.from_("off"))
    assert Scalar.from_("off").as_bool() is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    assert.equal(Scalar.from(true).isTruthy(), true)
    assert.equal(Scalar.from(0).isTruthy(), false)
    assert.equal(Scalar.from('off').isTruthy(), false)
    ```

## Casts

A column keeps Arrow's wider reading behind the strict value door: `yes`, `no`, `1` and `0` convert in a String-to-Boolean column cast, and text that names nothing becomes null under the default `safe`. A number column converts by whether the value is zero.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, BooleanArray, StringArray};
    use yggdryl::{ArrowCastOptions, BooleanField};

    let text: ArrayRef = Arc::new(StringArray::from(vec!["true", "no"]));
    let flags: BooleanArray = BooleanField::unit("ok", true)
        .cast_arrow_array(text, ArrowCastOptions::new())?;

    assert_eq!(flags.value(0), true);
    assert_eq!(flags.value(1), false);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    flags = Field("ok", "boolean").cast_arrow_array(pa.array(["true", "false", "no", "yes"]))
    assert flags.to_pylist() == [True, False, False, True]

    # Text that names nothing becomes null under the default `safe`.
    assert Field("ok", "boolean").cast_arrow_array(pa.array(["maybe"])).to_pylist() == [None]

    # A number converts by whether it is zero.
    assert Field("ok", "boolean").cast_arrow_array(
        pa.array([1, 0], type=pa.int32())
    ).to_pylist() == [True, False]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const text = arrow.vectorFromArray(['true', 'no'], new arrow.Utf8())
    assert.deepEqual([...fields.boolean('ok').castArrowArray(text)], [true, false])
    ```

## Edges

- The canonical default of `boolean` is `false`, which is what a [strict-nullability](../cast.md#strict-nullability) cast writes where a required column holds a null.
- `as_bool` answers `None` for every non-boolean, including `0`, `1` and the text `"true"`; `is_truthy` answers for all of them. Nothing falls back between the two.
- `null` is a kind of its own and not a numeric one: it carries only nulls, has no fixed byte width, and yields to the defined side when two schemas [merge](../field.md#merging-two-schemas).
- A null operand propagates through every binary arithmetic operation ([Scalar](../scalar.md#variants-and-arithmetic)).
- `Scalar::from(())` is the Rust spelling of the absent value; Python passes `None` and JavaScript `null`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- field::unit default_scalar
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::default
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_defaults.py python/tests/types/test_factories.py
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/types/defaults.test.js
    npm run --prefix node bench:types:defaults
    ```
