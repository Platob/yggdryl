# Unit

The unit a quantity is stated in: FIX `UnitOfMeasure(996)`, held as the text it is - `Shares`, `Bbl`, `MWh` - under its own identity, the way [`timeinforce`](timeinforce.md) holds a wire value rather than a name for it.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `unit`, `UnitType`/`UnitField`, the `Unit` value and `Scalar::Unit` |
| Validates | US-ASCII, no NUL, at most thirty-two bytes - and nothing else: no registry closes the space a venue's own unit may take |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A thirty-third byte or a byte past `0x7F`, naming the width: `at most 32 bytes` |

`Unit::none()` is the empty unit, the value stated as none: it is the code's default, what [`Market::get_unit`](../../graph/index.md#market) answers where a market states none and a `Lane` leaves out, and a [merge](index.md#the-code-family-value) takes the other unit over it.

## DataType

`unit` is the one spelling; `DataType::unit()` is the shorthand, the code family, a bound rather than a layout.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::from_str("unit")?, DataType::Unit);
    assert_eq!(DataType::unit().to_string(), "unit");
    assert_eq!(DataType::Unit.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Unit.code_name(), Some("unit"));
    assert_eq!(DataType::Unit.code_width(), Some(32));
    assert_eq!(DataType::Unit.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    unit = DataType("unit")
    assert unit.id == "unit"
    assert unit.kind == "code"
    assert unit.code_width == 32
    assert unit.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const unit = new DataType('unit')
    assert.equal(unit.id, 'unit')
    assert.equal(unit.kind, 'code')
    assert.equal(unit.codeWidth, 32)
    assert.equal(unit.fixedByteWidth, null)
    ```

## Field

`UnitField` is the typed marker; Python and JavaScript name the factory `unit`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, UnitField};

    let field = UnitField::unit("unit", true);
    assert_eq!(field.dtype(), &DataType::Unit);
    assert_eq!(field.to_field(), Field::new("unit", DataType::Unit, true));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    unit = yggdryl.unit("unit")
    assert isinstance(unit, Field)
    assert str(unit.dtype) == "unit"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const unit = fields.unit('unit')
    assert.equal(unit.dtype.toString(), 'unit')
    ```

## Scalar

The value is the text under the unit identity, and the width is the whole rule.

=== "Rust"

    ```rust
    use yggdryl::CodeValue as _;
    use yggdryl::{DataType, Scalar, Unit};

    let shares = DataType::Unit.scalar("Shares")?;
    assert_eq!(shares, Scalar::Unit(Unit::new("Shares")?));
    assert_eq!(shares.as_str(), Some("Shares"));
    assert_eq!(shares.kind(), "unit");
    assert_eq!(serde_json::to_string(&shares).unwrap(), r#"{"type":"unit","value":"Shares"}"#);

    // None, and the merge that takes the other unit over it.
    assert!(Unit::none().is_none());
    assert_eq!(DataType::Unit.default_value()?, Scalar::Unit(Unit::none()));
    assert_eq!(Unit::none().merge_with(&Unit::new("MWh")?), Unit::new("MWh")?);

    // The width is the whole rule, and it names itself in the refusal.
    let refused = DataType::Unit.scalar("X".repeat(33).as_str()).unwrap_err().to_string();
    assert!(refused.contains("32 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    shares = DataType("unit").scalar("Shares")
    assert shares.as_py() == "Shares"
    assert shares.kind == "unit"
    assert DataType("unit").scalar("Day") != DataType("timeinforce").scalar("Day")

    with pytest.raises(ValueError, match="32 bytes"):
        DataType("unit").scalar("X" * 33)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const shares = new DataType('unit').scalar('Shares')
    assert.equal(shares.asJs(), 'Shares')
    assert.equal(shares.kind, 'unit')
    assert.throws(() => new DataType('unit').scalar('X'.repeat(33)), /32 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.unit` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let unit = Field::new("unit", DataType::Unit, false);
    let arrow = unit.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.unit");
    assert_eq!(Field::from_arrow_field(&arrow)?, unit);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    unit = Field("unit", "unit")
    arrow_field = unit.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.unit"
    assert Field.from_arrow(arrow_field) == unit
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['Shares', 'MWh']), fields.unit('unit'))
    assert.deepEqual([...stored.intoArrowArray()], ['Shares', 'MWh'])
    ```

## Edges

- `at most 32 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- Nothing gates the value: this code declares no vocabulary, so a unit no standard names is held as it stands.
- The default value is the empty unit, `Unit::none()`, answered as a `unit` scalar ([Cast](../cast.md#empty-text)).
- A `unit` and a [`timeinforce`](timeinforce.md) of the same bytes are two values: the identity leads, then the text.
- [`merge_with`](index.md#the-code-family-value) keeps this unit unless it is none, in which case the other stands.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- unit::coded code::datatypes
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code or typed_field_factory"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
