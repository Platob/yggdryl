# Side

Which side of the market a trade took: FIX `Side(54)` as one explicit vocabulary, read by spelling and refused where a spelling names none.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `side`, `SideType`/`SideField`, the `Side` value and `Scalar::Side` |
| Validates | By spelling: a FIX wire code, the specification's name, or a stored value reaches one explicit value; anything else is refused rather than stored |
| Lazy | Nothing - the three tables are static and the lookup is a scan of bounded bytes |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | A spelling that names no side, naming `side`; a ninth stored byte, naming the width |

The stored values are the crate's own explicit spellings - `BUY`, `SELL`, `SSHORTEX`, `CROSSSHX` - rather than FIX's one-character codes, and the longest of them is eight bytes. The width holds the value *read*, never the spelling, so a name as long as `SellShortExempt` still reads.

## DataType

`side` is the one spelling; the datatype is the variant, and there is no shorthand constructor.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::from_str("side")?, DataType::Side);
    assert_eq!(DataType::Side.to_string(), "side");
    assert_eq!(DataType::Side.kind(), DataTypeKind::Code);
    assert_eq!(DataType::Side.code_name(), Some("side"));
    assert_eq!(DataType::Side.code_width(), Some(8));
    assert_eq!(DataType::Side.fixed_byte_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    side = DataType("side")
    assert (side.id, side.code_width, side.kind) == ("side", 8, "code")
    assert side.fixed_byte_width is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const side = new DataType('side')
    assert.equal(side.id, 'side')
    assert.equal(side.kind, 'code')
    assert.equal(side.codeWidth, 8)
    assert.equal(side.fixedByteWidth, null)
    ```

## Field

`SideField` is the typed marker; Python and JavaScript name the factory `side`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, SideField};

    let side = SideField::unit("side", false);
    assert_eq!(side.dtype(), &DataType::Side);
    assert_eq!(side.to_field(), Field::new("side", DataType::Side, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    side = yggdryl.side("side", nullable=False)
    assert isinstance(side, Field)
    assert str(side.dtype) == "side"
    assert not side.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const side = fields.side('side', { nullable: false })
    assert.equal(side.dtype.toString(), 'side')
    assert.equal(side.nullable, false)
    ```

## Scalar

The value is one of the explicit spellings, whichever vocabulary named it. A spelling that names no side is refused, never stored.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, Side};

    let buy = DataType::Side.scalar("BUY")?;
    assert_eq!(buy, Scalar::Side(Side::new("BUY")?));
    assert_eq!(buy.kind(), "side");

    // The wire code, the specification's name and the stored value all reach
    // the one explicit value.
    assert_eq!(DataType::Side.scalar("1")?, buy);
    assert_eq!(DataType::Side.scalar("Buy")?, buy);
    assert_eq!(DataType::Side.scalar("SellShort")?.as_str(), Some("SSHORT"));
    assert_eq!(DataType::Side.scalar("SellShortExempt")?.as_str(), Some("SSHORTEX"));

    // A spelling nothing publishes answers nothing rather than a guess.
    assert!(DataType::Side.scalar("Z").is_err());
    assert_ne!(buy, DataType::TimeInForce.scalar("BUY")?);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    side = DataType("side")
    assert side.scalar("BUY").kind == "side"
    assert side.scalar("1").as_py() == "BUY"
    assert side.scalar("SellShort").as_py() == "SSHORT"
    assert side.scalar("5").as_py() == "SSHORT"

    with pytest.raises(ValueError, match="side"):
        side.scalar("Z")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const side = new DataType('side')
    assert.equal(side.scalar('1').asJs(), 'BUY')
    assert.equal(side.scalar('SellShort').asJs(), 'SSHORT')
    assert.throws(() => side.scalar('Z'), /side/)
    ```

## Arrow storage

`Utf8` under `yggdryl.side` ([Codes](index.md#arrow-storage)).

=== "Rust"

    ```rust
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{DataType, Field};

    let side = Field::new("side", DataType::Side, false);
    let arrow = side.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Utf8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.side");
    assert_eq!(Field::from_arrow_field(&arrow)?, side);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie

    side = Field("side", "side")
    arrow_field = side.into_arrow()
    assert arrow_field.type == pa.string()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.side"
    assert Field.from_arrow(arrow_field) == side

    # A column holds the explicit spellings, whichever vocabulary wrote them.
    stored = Serie.from_arrow_array(pa.array(["BUY", "SSHORT"]), side, safe=False)
    assert stored.as_py() == ["BUY", "SSHORT"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['BUY', 'SSHORT']), fields.side('side'))
    assert.deepEqual([...stored.intoArrowArray()], ['BUY', 'SSHORT'])
    ```

## Three vocabularies, one value

A FIX `Side(54)` wire code, the specification's own name, and the stored value all name one thing, so they all reach it. Names fold the way every other name in this crate folds - ASCII case insensitive, with `_`, `-` and spaces ignored - so `SellShort`, `sell_short` and `SELL SHORT` are one spelling. A wire code does **not** fold, because `A` and `a` are different codes in FIX.

| Wire code | Stored value | Wire code | Stored value |
| --- | --- | --- | --- |
| `1` | `BUY` | `9` | `CROSSSH` |
| `2` | `SELL` | `A` | `CROSSSHX` |
| `3` | `BUYMINUS` | `B` | `ASDEF` |
| `4` | `SELLPLUS` | `C` | `OPPOSITE` |
| `5` | `SSHORT` | `D` | `SUBSCR` |
| `6` | `SSHORTEX` | `E` | `REDEEM` |
| `7` | `UNDISC` | `F` | `LEND` |
| `8` | `CROSS` | `G` | `BORROW` |
| | | `H` | `SELLUND` |

`UNKNOWN` is the eighteenth stored value: a side stated as none, `Side::Unknown`, the variant a `Side` defaults to. A `Side` is a one-byte enum whose variants stand in FIX's code order, `Side::Buy` through `Side::SellUnd`; `as_str` is the stored spelling, `fix_code` the wire character and `None` for `Unknown`. `Side::from_spelling` answers the value or nothing; `Side::read` is the same reading as a refusal. Rust only - the bindings reach the identical reading through the value door above. A dialect's own code maps as its dictionary says, because FIX's `Side(54)` reaches these values through the name the [code set it reads by](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) gives each code.

```rust
use yggdryl::Side;

assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
assert_eq!(Side::Buy.fix_code(), Some('1'));
assert_eq!(Side::from_spelling("SellShort").unwrap().as_str(), "SSHORT");
assert_eq!(Side::from_spelling("sshort").unwrap().as_str(), "SSHORT");
assert_eq!(Side::from_spelling("H").unwrap().as_str(), "SELLUND");
assert!(Side::from_spelling("X").is_none());

// `read` is the same reading as a refusal naming the spelling.
assert_eq!(Side::read("Buy")?.as_str(), "BUY");
assert!(Side::read("X").is_err());
assert_eq!(Side::Unknown.as_str(), "UNKNOWN");
```

## Which lane of a quote

`is_bid` and `is_ask` say which lane a side takes, which is what a FIX lift and [`MarketOperation::fill_lanes`](../../graph.md#contract) fill a lane by; `Market::get_side` answers the value itself, one byte, `Side::Unknown` where a market states none. `BUY` and `BUYMINUS` take the bid; `SELL`, `SELLPLUS`, `SSHORT`, `SSHORTEX` and `SELLUND` take the ask. Everything else takes neither: a cross is both sides at once, `OPPOSITE` means "whatever the other leg was", and `ASDEF`, `UNDISC` and a side stated as none say nothing about a lane. Domain knowledge written where a reviewer can check it, because Orchestra does not publish it. Rust only.

```rust
use yggdryl::Side;

assert!(Side::read("Buy")?.is_bid());
assert!(Side::read("SellShortExempt")?.is_ask());
assert!(!Side::read("SellShort")?.is_bid());

// A cross, `OPPOSITE` and a side stated as none take no lane.
assert!(!Side::read("Cross")?.is_bid() && !Side::read("Cross")?.is_ask());
assert!(!Side::read("Opposite")?.is_ask());
assert!(!Side::Unknown.is_bid() && !Side::Unknown.is_ask());
```

## `UNKNOWN` states no side

`UNKNOWN` is the value a merge takes the other side over ([`merge_with`](index.md#the-code-family-value)). Rust only.

```rust
use yggdryl::{CodeValue, Side};

assert_eq!(Side::Unknown.merge_with(&Side::read("1")?).as_str(), "BUY");
// Anything stated stands.
assert_eq!(Side::read("BUY")?.merge_with(&Side::read("SELL")?).as_str(), "BUY");
```

## Edges

- A spelling that names no side -> refused naming `side`, never stored; a column typed `side` therefore holds the explicit values only, and text that names none leaves the column null under `safe`.
- A wire code never folds: `A` is `CROSSSHX` and `a` names no side, because they are different FIX codes and a folded lookup would answer the wrong one.
- A name folds: `SellShort`, `sell_short` and `SELL SHORT` are one spelling, `SSHORT`.
- The width bounds the value read, never the spelling, so `SellShortExempt` reads to the eight-byte `SSHORTEX`; a stored value past eight bytes is refused naming the width.
- No default value: a published vocabulary spells the members, so there is no neutral one, and an empty text cell entering the column is null ([Cast](../cast.md#empty-text)). `UNKNOWN` is a stated value, not an absence.
- `StringEnum::SIDES` is the listing - the eighteen stored values - reached by the logical name `side`; per-member pedigree stays in the registry's vocabulary named by the field's `FIX:codeset`.
- A `side` and a [`timeinforce`](timeinforce.md) of the same bytes are two values: the identity leads, then the text.
- `ascii_packed` pads a side into eight bytes, exactly as `fixed_ascii(8)` does, and `ascii_value` reads it back.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- cfi_code::coded code::datatypes state::coded string::listings timeinforce::coded
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "registered_code"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="registered code" node/tests/datatype.test.js
    ```
