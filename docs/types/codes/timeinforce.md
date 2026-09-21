# TimeInForce

How long an order stands: FIX `TimeInForce(59)`, stored as the wire value rather than a name for it, exactly as [`side`](side.md) is.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `timeinforce`, `TimeInForceType`/`TimeInForceField`, the `TimeInForce` value and `Scalar::TimeInForce` |
| Validates | US-ASCII, no NUL, at most eight bytes - and nothing else: the code set is a declared vocabulary, never a gate |
| Lazy | Nothing |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A ninth byte or a byte past `0x7F`, naming the width: `at most 8 bytes` |

The standard's values are one character; eight leaves room for a venue's own code, and a value no version defines is held rather than refused. That is the difference from [`side`](side.md) and [`state`](state.md), which read a spelling and refuse what names nothing.

## DataType

`timeinforce` is the one spelling; the datatype is the variant, and there is no shorthand constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::from_str("timeinforce")?, DataType::TimeInForce);
    assert_eq!(DataType::TimeInForce.to_string(), "timeinforce");
    assert_eq!(DataType::TimeInForce.kind(), DataTypeKind::Code);
    assert_eq!(DataType::TimeInForce.code_name(), Some("timeinforce"));
    assert_eq!(DataType::TimeInForce.code_width(), Some(8));
    assert_eq!(DataType::TimeInForce.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    tif = DataType("timeinforce")
    assert tif.id == "timeinforce"
    assert tif.kind == "code"
    assert tif.code_width == 8
    assert tif.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const tif = new DataType('timeinforce')
    assert.equal(tif.id, 'timeinforce')
    assert.equal(tif.kind, 'code')
    assert.equal(tif.codeWidth, 8)
    assert.equal(tif.fixedByteWidth, null)
    ```

## Field

`TimeInForceField` is the typed marker; Python and JavaScript name the factory `timeinforce`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, TimeInForceField};

    let tif = TimeInForceField::unit("timeinforce", true);
    assert_eq!(tif.dtype(), &DataType::TimeInForce);
    assert_eq!(tif.to_field(), Field::new("timeinforce", DataType::TimeInForce, true));
    ```

=== "Python"

    ```python
    from yggdryl import Field, types

    tif = types.timeinforce("timeinforce")
    assert isinstance(tif, Field)
    assert str(tif.dtype) == "timeinforce"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const tif = fields.timeinforce('timeinforce')
    assert.equal(tif.dtype.toString(), 'timeinforce')
    ```

## Scalar

The value is the wire code itself, under the time-in-force identity. Nothing is translated on the way in: a dictionary turns `0` into `Day`, and this column holds what the message carried.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeInForce};

    let day = DataType::TimeInForce.scalar("0")?;
    assert_eq!(day, Scalar::TimeInForce(TimeInForce::new("0")?));
    assert_eq!(day.as_str(), Some("0"));
    assert_eq!(day.kind(), "timeinforce");

    // A side and a time in force of the same bytes are two values.
    assert_ne!(DataType::Side.scalar("BUY")?, DataType::TimeInForce.scalar("BUY")?);
    // The width is the whole rule, and it names itself in the refusal.
    let refused = DataType::TimeInForce.scalar("TOOLONGTIF").unwrap_err().to_string();
    assert!(refused.contains("8 bytes"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    day = DataType("timeinforce").scalar("0")
    assert day.as_py() == "0"
    assert day.kind == "timeinforce"
    assert DataType("timeinforce").scalar("BUY") != DataType("side").scalar("BUY")

    with pytest.raises(ValueError, match="8 bytes"):
        DataType("timeinforce").scalar("TOOLONGTIF")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const day = new DataType('timeinforce').scalar('0')
    assert.equal(day.asJs(), '0')
    assert.equal(day.kind, 'timeinforce')
    assert.throws(() => new DataType('timeinforce').scalar('TOOLONGTIF'), /8 bytes/)
    ```

## Arrow storage

`Utf8` under `yggdryl.timeinforce` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let tif = Field::new("timeinforce", DataType::TimeInForce, false);
    let arrow = tif.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.timeinforce");
    assert_eq!(Field::from_arrow_field(&arrow)?, tif);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field

    tif = Field("timeinforce", "timeinforce")
    arrow_field = tif.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.timeinforce"
    assert Field.from_arrow(arrow_field) == tif
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    assert.deepEqual([...fields.timeinforce('tif').castArrowArray(utf8(['0', '1']))], ['0', '1'])
    ```

## The FIX vocabulary

`StringEnum::TIMESINFORCE` is FIX's `TimeInForceCodeSet` as the union across every version, sorted: fourteen wire values, `0`-`9` and `A`-`D`. The shipped registry under `config/fix/codesets/timeinforcecodeset.json` is what names them for a dialect, and a [FIX column](../../fix/capture.md) reads a code through the name the [code set its own field reads by](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) gives it.

| Value | Name |
| --- | --- |
| `0` | Day (or session) |
| `1` | Good Till Cancel (GTC) |
| `2` | At the Opening (OPG) |
| `3` | Immediate Or Cancel (IOC) |
| `4` | Fill Or Kill (FOK) |
| `5` | Good Till Crossing (GTX) |
| `6` | Good Till Date (GTD) |
| `7` | At the Close |
| `8` | Good Through Crossing |
| `9` | At Crossing |
| `A` | Good for Time (GFT) |
| `B` | Good for Auction (GFA) |
| `C` | Good for this Month (GFM) |

`D` is in the union `StringEnum::TIMESINFORCE` carries and is not named by the shipped code set, which is the whole reason the listing is a vocabulary rather than a gate: a value no version this build knows defines is held.

=== "Rust"

    ```rust
    use yggdryl::{DataType, StringEnum};

    let tifs = StringEnum::from_logical_name("timeinforce")?;
    assert_eq!(tifs.len(), StringEnum::TIMESINFORCE.len());
    assert!(StringEnum::TIMESINFORCE.contains(&"0"));

    // A value no version defines is held rather than refused, and it packs
    // and unpacks like any other.
    let held = DataType::TimeInForce.scalar("X")?;
    assert_eq!(held.as_str(), Some("X"));
    let packed = DataType::TimeInForce.ascii_packed(b"X")?;
    assert_eq!(DataType::TimeInForce.ascii_value(packed)?, "X");
    ```

=== "Python"

    ```python
    from yggdryl import DataType, StringEnum

    tifs = StringEnum.from_logical_name("timeinforce")
    assert len(tifs) == len(StringEnum.prebuilt()["timeinforce"])
    assert "0" in StringEnum.prebuilt()["timeinforce"]

    # A value no version defines is held rather than refused.
    assert DataType("timeinforce").scalar("X").as_py() == "X"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, StringEnum } = require('yggdryl')

    const tifs = StringEnum.fromLogicalName('timeinforce')
    assert.equal(tifs.length, StringEnum.prebuilt().timeinforce.length)
    assert.equal(new DataType('timeinforce').scalar('X').asJs(), 'X')
    ```

## Edges

- `at most 8 bytes` is the refusal, whatever the source: a scalar, a cast row, or `ascii_packed`.
- A value outside the code set is stored: this code declares a vocabulary the way [`mic`](mic.md) does, and gates nothing.
- The default value is the empty text, answered as a `timeinforce` scalar ([Cast](../cast.md#empty-text)); a [FIX capture](../../fix/capture.md) reads an absent `TimeInForce(59)` as `0`, a day order, from the field's own definition rather than from this datatype.
- A `timeinforce` and a [`side`](side.md) of the same bytes are two values: the identity leads, then the text.
- Nothing about a time in force merges: it has no value stating none, so [`merge_with`](index.md#the-code-family-value) keeps this one.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::coded vocabulary::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/types/datatype.test.js
    ```
