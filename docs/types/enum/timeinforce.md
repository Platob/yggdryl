# TimeInForce

How long an order stands: FIX `TimeInForce(59)` as an enum of fifteen members - `UKNW`, one member per wire value of the code set in wire order, and `OTHER` for a venue's own value - stored as the `uint8` code of its member.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `timeinforce`, the registered kind `TIMEINFORCE_KIND` under `DataType::Market`, its marker `TimeInForceType`, the `TimeInForce` enum; `TimeInForce::dtype()` and `TimeInForce::field(name)` |
| Validates | A member, the code of one, or a spelling - the stored name in any case, the FIX specification's name folded, or the `TimeInForce(59)` wire value unfolded - reaches one member; anything else is refused rather than stored |
| Lazy | Nothing - the member table is static |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | An integer that is the code of no member, naming the code; a spelling that names no member, naming the spelling |
| Stores | `uint8` under `yggdryl.timeinforce`: `UKNW` at `0`, `DAY` `1` to `GFM` `13` in FIX's wire order, `OTHER` at `99` |
| Reads FIX | `from_fix(wire)` reads one `TimeInForce(59)` value - the wire value, else any spelling `from_spelling` reads, else `OTHER` - and never refuses; `fix_code()` answers the wire value back. A [registry](#a-dictionary-maps-a-venues-values) maps any field's values onto members through `FIX:timeinforce`, read before the crate's table |
| Default | `UKNW`, no time in force stated: a stated value, never an absence |

What `as_str` answers and every text format writes is the member's stored name - `DAY`, `GTC`, `IOC` - never FIX's one-character wire value, which `fix_code` answers on its own; the long meaning is the member's `description`.

## DataType

`timeinforce` is the one spelling, `TimeInForce::dtype()` the datatype (a `const fn`: what every column of the kind declares) and `TimeInForce::field(name)` a nullable field of it, `TimeInForce::ID` the id - a [registered kind](../datatype.md#registered-kinds) under `DataType::Market`; kind `enum`. `DataType` holds no constructor per kind: the kind's own type answers its datatype, and the bindings' doors are unchanged (`DataType("timeinforce")` in Python, `new DataType('timeinforce')` in JavaScript).

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind, TimeInForce};

    assert!(matches!(TimeInForce::dtype(), DataType::Market(kind) if kind.id() == TimeInForce::ID));
    assert_eq!(DataType::from_str("timeinforce")?, TimeInForce::dtype());
    assert_eq!(TimeInForce::dtype().to_string(), "timeinforce");
    assert_eq!(TimeInForce::dtype().kind(), DataTypeKind::Enum);
    assert_eq!(TimeInForce::dtype().id().as_u8(), 0xc5);
    assert!(TimeInForce::dtype().is_enum() && !TimeInForce::dtype().is_code());
    assert_eq!(TimeInForce::dtype().code_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    tif = DataType("timeinforce")
    assert (tif.id, tif.kind, tif.code_width) == ("timeinforce", "enum", None)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const tif = new DataType('timeinforce')
    assert.equal(tif.id, 'timeinforce')
    assert.equal(tif.kind, 'enum')
    assert.equal(tif.codeWidth, null)
    ```

## Field

`TimeInForceField` is `FieldOf<TimeInForceType>`, `TimeInForceType` the kind's marker over the `Market` variant; Python and JavaScript name the factory `timeinforce`.

=== "Rust"

    ```rust
    use yggdryl::{TimeInForce, TimeInForceField};

    let tif = TimeInForceField::unit("timeinforce", true);
    assert_eq!(tif.dtype(), &TimeInForce::dtype());
    assert_eq!(tif.to_field(), TimeInForce::field("timeinforce"));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    tif = yggdryl.timeinforce("timeinforce", nullable=False)
    assert isinstance(tif, Field)
    assert str(tif.dtype) == "timeinforce"
    assert not tif.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const tif = fields.timeinforce('timeinforce', { nullable: false })
    assert.equal(tif.dtype.toString(), 'timeinforce')
    assert.equal(tif.nullable, false)
    ```

## Scalar

The value is the member, whichever spelling named it: `GTC` for `GTC`, `gtc`, FIX's `GoodTillCancel`, its wire value `1`, or its code `2`. Text is read as a spelling and an integer as a code, so `"1"` is the wire value of `GTC` while `1` is the code of `DAY`. Rust holds the `TimeInForce` member; Python the member of the `yggdryl.TimeInForce` `IntEnum`, which is the integer it stores and renders as its stored name; JavaScript the member's name, with `TimeInForce` mapping every name to its code ([Enums](index.md#enum-facts-in-the-bindings)).

=== "Rust"

    ```rust
    use yggdryl::{Scalar, TimeInForce};

    let gtc = TimeInForce::dtype().scalar("GTC")?;
    assert_eq!(gtc, Scalar::from(TimeInForce::GoodTillCancel));
    assert_eq!(gtc.kind(), "timeinforce");
    assert_eq!(TimeInForce::GoodTillCancel.code(), 2);

    // The stored name in any case, FIX's name, the wire value and the code
    // reach one member.
    assert_eq!(TimeInForce::dtype().scalar("gtc")?, gtc);
    assert_eq!(TimeInForce::dtype().scalar("GoodTillCancel")?, gtc);
    assert_eq!(TimeInForce::dtype().scalar("1")?, gtc);
    assert_eq!(TimeInForce::dtype().scalar(2_i32)?, gtc);
    // Text is a spelling, an integer a code: `1` is the code of `DAY`.
    assert_eq!(
        TimeInForce::dtype().scalar(1_i32)?,
        Scalar::from(TimeInForce::Day)
    );

    // A spelling nothing publishes, or the code of no member, answers nothing
    // rather than a guess.
    assert!(TimeInForce::dtype().scalar("Z").is_err());
    assert!(TimeInForce::dtype().scalar(14_i32).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, TimeInForce

    tif = DataType("timeinforce")
    assert tif.scalar("GTC").as_py() is TimeInForce.GTC
    assert tif.scalar("GoodTillCancel").as_py() is TimeInForce.GTC
    assert tif.scalar("1").as_py() is TimeInForce.GTC
    assert tif.scalar(2).as_py() is TimeInForce.GTC
    # Text is a spelling, an integer a code: `1` is the code of `DAY`.
    assert tif.scalar(1).as_py() is TimeInForce.DAY
    assert tif.scalar("GTC").kind == "timeinforce"

    # A member is the integer it stores and reads as its stored name.
    assert TimeInForce.GTC == 2 and str(TimeInForce.GTC) == "GTC"

    with pytest.raises(ValueError, match="timeinforce"):
        tif.scalar("Z")
    with pytest.raises(ValueError, match="timeinforce"):
        tif.scalar(14)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, TimeInForce } = require('yggdryl')

    const tif = new DataType('timeinforce')
    assert.equal(tif.scalar('gtc').asJs(), 'GTC')
    assert.equal(tif.scalar('1').asJs(), 'GTC')
    assert.equal(tif.scalar('day').asJs(), 'DAY')
    assert.equal(TimeInForce.GTC, 2)
    assert.equal(TimeInForce.OTHER, 99)
    assert.throws(() => tif.scalar('Z'), /timeinforce/)
    ```

## Arrow storage

`UInt8` under `yggdryl.timeinforce`: one value buffer of codes, like every [enum](index.md). Text entering the column is read as a spelling and an integer column of any width as codes, each refused - or null under `safe` in a nullable column - where it names no member; the column cast to text answers each member's stored name, and cast to an integer its code.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int64Array, StringArray, UInt8Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, Field, Serie, TimeInForce};

    let tif = Field::new("timeinforce", TimeInForce::dtype(), false);
    let arrow = tif.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.timeinforce");
    assert_eq!(Field::from_arrow_field(&arrow)?, tif);

    // Every spelling lands as the code its member stores.
    let spelled: ArrayRef = Arc::new(StringArray::from(vec!["day", "1", "IOC", "FillOrKill"]));
    let stored = Serie::from_arrow_array(Some(&tif), spelled, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<UInt8Array>().expect("uint8 codes");
    assert_eq!(codes.values().to_vec(), [1, 2, 4, 5]);

    // An integer column of any width lands as codes.
    let coded: ArrayRef = Arc::new(Int64Array::from(vec![1, 99]));
    let stored = Serie::from_arrow_array(Some(&tif), coded, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<UInt8Array>().expect("uint8 codes");
    assert_eq!(codes.values().to_vec(), [1, 99]);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie, TimeInForce

    tif = Field("timeinforce", "timeinforce")
    arrow_field = tif.into_arrow()
    assert arrow_field.type.storage_type == pa.uint8()
    assert arrow_field.type.extension_name == "yggdryl.timeinforce"
    assert Field.from_arrow(arrow_field) == tif

    # Every spelling, and an integer column of any width, lands as a member.
    stored = Serie.from_arrow_array(pa.array(["day", "1", "IOC"]), tif, safe=False)
    assert stored.as_py() == [TimeInForce.DAY, TimeInForce.GTC, TimeInForce.IOC]
    coded = Serie.from_arrow_array(pa.array([1, 99], pa.int64()), tif)
    assert coded.as_py() == [TimeInForce.DAY, TimeInForce.OTHER]

    # Out again as the stored name, or as the code.
    assert stored.cast(Field("t", "utf8")).as_py() == ["DAY", "GTC", "IOC"]
    assert stored.cast(Field("t", "int16")).as_py() == [1, 2, 4]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, TimeInForce, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['day', '1', 'IOC']), fields.timeinforce('tif'))
    const codes = stored.intoArrowArray()
    assert.ok(arrow.DataType.isInt(codes.type) && codes.type.bitWidth === 8 && !codes.type.isSigned)
    assert.deepEqual([...codes], [TimeInForce.DAY, TimeInForce.GTC, TimeInForce.IOC])
    ```

## The FIX vocabulary

Three vocabularies name one member: the stored name, the specification's name and the wire value. Names fold the way every other name in this crate folds - ASCII case insensitive, with `_`, `-` and spaces ignored - so `GoodTillCancel`, `good_till_cancel` and `GOOD TILL CANCEL` are one spelling, and so are `GTC` and `gtc`. A wire value does **not** fold, because `A` and `a` are different values in FIX. `UKNW` and `OTHER` stand for no one wire value.

| Code | Wire value | Name | Specification's name | Description |
| ---: | --- | --- | --- | --- |
| `0` | - | `UKNW` | - | No time in force stated. |
| `1` | `0` | `DAY` | `Day` | Good for the trading day. |
| `2` | `1` | `GTC` | `GoodTillCancel` | Good till canceled. |
| `3` | `2` | `OPG` | `AtTheOpening` | At the opening. |
| `4` | `3` | `IOC` | `ImmediateOrCancel` | Immediate or cancel: what does not fill at once is canceled. |
| `5` | `4` | `FOK` | `FillOrKill` | Fill or kill: filled whole at once, or canceled. |
| `6` | `5` | `GTX` | `GoodTillCrossing` | Good till crossing. |
| `7` | `6` | `GTD` | `GoodTillDate` | Good till a date. |
| `8` | `7` | `ATC` | `AtTheClose` | At the close. |
| `9` | `8` | `GTHX` | `GoodThroughCrossing` | Good through crossing. |
| `10` | `9` | `ATX` | `AtCrossing` | At crossing. |
| `11` | `A` | `GFT` | `GoodForTime` | Good for a time. |
| `12` | `B` | `GFA` | `GoodForAuction` | Good for the auction. |
| `13` | `C` | `GFM` | `GoodForMonth` | Good for the month. |
| `99` | any other | `OTHER` | - | A time in force no member names. |

`from_spelling` answers the member or nothing, and `read` is the same reading as a refusal - the value door. `from_fix` is the wire reading and never refuses: the wire value, else any spelling - a bridge writing `day` where the standard writes `0` - else `OTHER`, because a venue's own value is still a time in force. `fix_code` answers the wire value, `None` for `UKNW` and `OTHER`. `StringEnum::TIMESINFORCE` is the thirteen wire values, sorted, the listing the logical name `timeinforce` prebuilds for a US-ASCII column declaring the vocabulary it holds; `DataType::from_logical_name("timeinforce")` is this enum.

=== "Rust"

    ```rust
    use yggdryl::{StringEnum, TimeInForce};

    assert_eq!(TimeInForce::ALL.len(), 15);
    assert_eq!(TimeInForce::from_spelling("0"), Some(TimeInForce::Day));
    assert_eq!(TimeInForce::from_spelling("good_till_cancel"), Some(TimeInForce::GoodTillCancel));
    assert_eq!(TimeInForce::from_spelling("A"), Some(TimeInForce::GoodForTime));
    // A wire letter does not fold, and a stored code is never text.
    assert_eq!(TimeInForce::from_spelling("a"), None);
    assert_eq!(TimeInForce::from_spelling("10"), None);
    assert_eq!(TimeInForce::from_code(10), Some(TimeInForce::AtCrossing));

    // The wire reading never refuses: a venue's own value is `OTHER`.
    assert_eq!(TimeInForce::from_fix("3"), TimeInForce::ImmediateOrCancel);
    assert_eq!(TimeInForce::from_fix("day"), TimeInForce::Day);
    assert_eq!(TimeInForce::from_fix("Z"), TimeInForce::Other);
    assert_eq!(TimeInForce::GoodTillCancel.fix_code(), Some("1"));
    assert_eq!(TimeInForce::Other.fix_code(), None);
    assert_eq!(TimeInForce::Unknown.fix_code(), None);

    assert!(TimeInForce::read("Z").is_err());
    assert_eq!(StringEnum::TIMESINFORCE.len(), 13);
    ```

=== "Python"

    ```python
    from yggdryl import StringEnum, TimeInForce

    assert len(TimeInForce) == 15
    assert TimeInForce.from_spelling("0") is TimeInForce.DAY
    assert TimeInForce.from_spelling("good_till_cancel") is TimeInForce.GTC
    assert TimeInForce.from_spelling("a") is None
    assert TimeInForce.from_spelling("10") is None
    assert TimeInForce(10) is TimeInForce.ATX

    assert TimeInForce.from_fix("3") is TimeInForce.IOC
    assert TimeInForce.from_fix("day") is TimeInForce.DAY
    assert TimeInForce.from_fix("Z") is TimeInForce.OTHER
    assert TimeInForce.GTC.fix_code == "1"
    assert TimeInForce.OTHER.fix_code is None
    assert TimeInForce.DAY.description == "Good for the trading day."
    assert len(StringEnum.prebuilt()["timeinforce"]) == 13
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { TimeInForce, timeInForceFixCode, timeInForceFromFix } = require('yggdryl')

    assert.equal(Object.keys(TimeInForce).length, 15)
    assert.ok(Object.isFrozen(TimeInForce))
    assert.equal(timeInForceFromFix('3'), 'IOC')
    assert.equal(timeInForceFromFix('day'), 'DAY')
    assert.equal(timeInForceFromFix('Z'), 'OTHER')
    assert.equal(timeInForceFixCode('GTC'), '1')
    assert.equal(timeInForceFixCode('OTHER'), null)
    ```

## A dictionary maps a venue's values

A venue that states how long an order stands in a field of its own, or spells `TimeInForce(59)` its own way, says so on the field: `FIX:timeinforce` is a word list of `wire=MEMBER` pairs, `["D=DAY","G=GTC"]`, the same shape as [`FIX:marketdatatype`](../../fix/registry.md#a-field-maps-its-values-onto-a-market-data-type).

| Contract | Rule |
| --- | --- |
| Key | `FIX:timeinforce`, read with `FixField::timeinforces()` - each item a `(wire, TimeInForce)` pair, a word naming no member passed over - and written with `FixFieldMut::set_timeinforces(&[(&str, TimeInForce)])`, an empty list removing it; Python `field.fix.timeinforces` crosses `(wire, member)` pairs, a member given as a member, its code or a spelling; JavaScript `field.fix.timeinforces` crosses `{wire, timeinforce}` objects |
| Refuses | a member that names none and one wire value stated twice, naming the field; the field is left as it was |
| Lookup | `FixRegistry::timeinforce_of(tag, wire)`: this dictionary's mapping first, then, for `TimeInForce(59)` alone, the crate's `TimeInForce::from_fix`; `None` for any other field the dictionary maps no such value of; Python `timeinforce_of`, JavaScript `timeinforceOf` |
| Compiled | `FixRegistry::timeinforce_sources() -> &[(i32, SmolStr, TimeInForce)]`, every field's pairs beside its tag, compiled once and forgotten by every change to the fields; Python `timeinforce_sources()`, Rust and Python only |
| Parse | a [message](../../fix/message.md) states its [`timeinforce`](../../graph/operation.md#contract) as it is built: `TimeInForce(59)` through `timeinforce_of`, else the first other field the dictionary maps a stated value of; none where nothing states one |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::Operation;
    use yggdryl::{DataType, FixCodec, FixFieldMut, FixRegistry, TimeInForce};

    let mut venue = DataType::utf8().nullable_field("VenueTif");
    FixFieldMut::new(&mut venue).set_tag(20059)?;
    FixFieldMut::new(&mut venue)
        .set_timeinforces(&[("D", TimeInForce::Day), ("G", TimeInForce::GoodTillCancel)])?;
    assert_eq!(venue.get_metadata("FIX:timeinforce"), Some(r#"["D=DAY","G=GTC"]"#));

    let mut registry = FixRegistry::new();
    registry.insert(venue)?;
    // The dictionary's own values first; `TimeInForce(59)` falls back on FIX's.
    assert_eq!(registry.timeinforce_of(20059, "G"), Some(TimeInForce::GoodTillCancel));
    assert_eq!(registry.timeinforce_of(20059, "X"), None);
    assert_eq!(registry.timeinforce_of(59, "3"), Some(TimeInForce::ImmediateOrCancel));
    assert_eq!(registry.timeinforce_of(59, "Z"), Some(TimeInForce::Other));
    assert_eq!(registry.timeinforce_of(54, "1"), None);
    assert_eq!(registry.timeinforce_sources().len(), 2);

    // A parse reads the venue's field when `TimeInForce(59)` is absent.
    let codec = FixCodec::new(Arc::new(registry));
    let message = codec
        .parse_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|38=5|20059=G|10=0|")?
        .next()
        .expect("one frame")?;
    assert_eq!(message.get_timeinforce(), Some(&TimeInForce::GoodTillCancel));
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import Field, TimeInForce
    from yggdryl.fix import FixCodec, FixRegistry

    venue = Field("VenueTif", "utf8")
    venue.fix.tag = 20059
    venue.fix.timeinforces = [("D", "day"), ("G", TimeInForce.GTC)]
    assert venue.fix.timeinforces == [("D", TimeInForce.DAY), ("G", TimeInForce.GTC)]
    with pytest.raises(ValueError):
        venue.fix.timeinforces = [("D", "not a time in force")]

    registry = FixRegistry()
    registry.insert(venue)
    assert registry.timeinforce_of(20059, "G") is TimeInForce.GTC
    assert registry.timeinforce_of(20059, "X") is None
    assert registry.timeinforce_of(59, "3") is TimeInForce.IOC
    assert registry.timeinforce_of(59, "Z") is TimeInForce.OTHER
    assert (20059, "D", TimeInForce.DAY) in registry.timeinforce_sources()

    codec = FixCodec(registry)
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|38=5|20059=G|10=0|")
    assert message.timeinforce is TimeInForce.GTC
    message = codec.parse_fix_line(b"8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|38=5|59=3|10=0|")
    assert message.timeinforce is TimeInForce.IOC
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, fix } = require('yggdryl')

    const venue = Field.from('VenueTif: utf8')
    venue.fix.tag = 20059
    venue.fix.timeinforces = [{ wire: 'D', timeinforce: 'DAY' }, { wire: 'G', timeinforce: 'GTC' }]
    assert.equal(venue.get('FIX:timeinforce'), '["D=DAY","G=GTC"]')
    assert.throws(() => {
      venue.fix.timeinforces = [{ wire: 'D', timeinforce: 'nope' }]
    })

    const registry = new fix.FixRegistry()
    registry.insert(venue)
    assert.equal(registry.timeinforceOf(20059, 'G'), 'GTC')
    assert.equal(registry.timeinforceOf(59, '3'), 'IOC')
    assert.equal(registry.timeinforceOf(54, '1'), null)

    const codec = new fix.FixCodec(registry)
    const message = codec.parseFixLine(Buffer.from('8=FIX.4.4|35=D|11=C1|55=AAPL|54=1|40=2|38=5|20059=G|10=0|'))
    assert.equal(message.timeinforce, 'GTC')
    ```

## Edges

- A spelling that names no member, or an integer that is the code of none -> refused naming `timeinforce`, never stored; a column typed `timeinforce` therefore holds members only, and a value that names none leaves a nullable column null under `safe`. Only the wire reading, `from_fix`, turns an unknown value into `OTHER`.
- Text is a spelling and an integer a code: `"1"` is the wire value of `GTC`, `1` is the code of `DAY`; `"10"` is no spelling at all.
- A wire value never folds: `A` is `GFT` and `a` names nothing, because a folded lookup would answer the wrong member for a dialect whose values differ by case.
- The default value is `UKNW`, code `0`: a stated value, not an absence. An empty text cell entering the column is null ([Cast](../cast.md#empty-text)), and a required column refuses it. A [FIX capture](../../fix/capture.md) under the shipped dictionary reads an absent `TimeInForce(59)` as `0`, a day order, from the field's own definition rather than from this datatype.
- `UKNW` is the zero member's spelling as of this release; `UNKN`, the retired spelling, names no time in force. A `timeinforce` column stores the code, so a stored `0` reads as `UKNW` unchanged; a text column or a document that spells `UNKN` is rebuilt by its writer, never reinterpreted. An [operation](../../graph/operation.md)'s digest feeds the stored name of the time in force it states, so an order or an execution stating `UKNW` digests under the new name: its `currhashcode` and `curruuid` move, its `crossuuid` where it states no cross code, and its followers' identities with them. A `marketdata` table written before this release is rebuilt from its capture, never merged into by `curruuid`; the UKNW spelling alone does not move FIX hashes; the separate FIX digest-label rename from `msgcat` to `marketdatakind` changes each FIX message's `currhashcode` and `curruuid`, and an anonymous split execution's `crosshashcode`.
- JSON, TOML, YAML and XML write a time in force as its stored name; the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code under its own identifier, so a time in force, a [side](side.md) and an integer of one code are three values.
- An [operation](../../graph/operation.md)'s `timeinforce` is a member of this enum, a `timeinforce` column in the [`marketdata` row](../../graph/schemas.md#the-marketdata-row). The [FIX row](../../graph/schemas.md#the-fix-row)'s `timeinforce` column is `TimeInForce(59)` itself, the wire text the dictionary types it as, which the message reads its member from; the crate adds no FIX field of its own for it.
- `utf8` under `yggdryl.timeinforce` is a foreign field wearing the name and imports as the text it is.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- timeinforce::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_timeinforce.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/timeinforce.test.js
    ```
