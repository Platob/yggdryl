# PluginSide

The role of a FIX plugin - the side of the session a dialect's plugin stands on - as an enum of three members, `UKNW`, `BUYS` and `SELL`, stored as the `uint8` code of its member.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `pluginside`, `PluginSideType`/`PluginSideField`, the `PluginSide` enum and `Scalar::PluginSide`; `DataType::pluginside()` |
| Validates | A member, the code of one, or a spelling - the stored name in any case, or the role's own name folded (`BuySide`, `buy-side`, `sell_side`, `SELL SIDE`, `Unknown`) - reaches one member; anything else is refused rather than stored |
| Lazy | Nothing - the member table is static |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | An integer that is the code of no member, naming the code; a spelling that names no member, naming the spelling; a member of another enum, a [`Side`](side.md) among them |
| Stores | `uint8` under `yggdryl.pluginside`: `UKNW` at `0`, `BUYS` at `1`, `SELL` at `2` |
| Reads a CBlock | `from_plugin_type(class)` reads the plugin class a CBlock root's `type` attribute names - its last `.`-separated segment, folded - and never refuses: `buyside` in it is `BUYS`, `sellside` is `SELL`, anything else `UKNW` ([below](#a-cblock-names-its-plugins-role)) |
| FIX | A dictionary's [source entry](../../fix/registry.md#membership) states its plugin's role, the crate's `msgpluginsidecodeset` renders the three members, and a codec reading under a source stamps the role on every message as the required [`msgpluginside`](../../fix/capture.md#the-plugins-role-is-the-sources) column, tag 65043 |
| Default | `UKNW`, no role stated: a stated value, never an absence |

A plugin's role is a fact about the session, not about an order: a Buy-Side plugin originates orders and cancels and receives execution reports, a Sell-Side plugin receives them and answers. It is a separate enum from [`Side`](side.md), though `BUYS` and `SELL` are spelled alike and stored under the same codes - never shared, and never cast one into the other.

## DataType

`pluginside` is the one spelling, `DataType::pluginside()` the constructor; kind `enum`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::pluginside(), DataType::PluginSide);
    assert_eq!(DataType::from_str("pluginside")?, DataType::PluginSide);
    assert_eq!(DataType::PluginSide.to_string(), "pluginside");
    assert_eq!(DataType::PluginSide.kind(), DataTypeKind::Enum);
    assert_eq!(DataType::PluginSide.id().as_u8(), 0xc6);
    assert!(DataType::PluginSide.is_enum() && !DataType::PluginSide.is_code());
    assert_eq!(DataType::PluginSide.code_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    role = DataType("pluginside")
    assert (role.id, role.kind, role.code_width) == ("pluginside", "enum", None)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const role = new DataType('pluginside')
    assert.equal(role.id, 'pluginside')
    assert.equal(role.kind, 'enum')
    assert.equal(role.codeWidth, null)
    ```

## Field

`PluginSideField` is the typed marker; Python and JavaScript name the factory `pluginside`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, PluginSideField};

    let role = PluginSideField::unit("msgpluginside", false);
    assert_eq!(role.dtype(), &DataType::PluginSide);
    assert_eq!(role.to_field(), Field::new("msgpluginside", DataType::PluginSide, false));
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import Field

    role = yggdryl.pluginside("msgpluginside", nullable=False)
    assert isinstance(role, Field)
    assert str(role.dtype) == "pluginside"
    assert not role.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const role = fields.pluginside('msgpluginside', { nullable: false })
    assert.equal(role.dtype.toString(), 'pluginside')
    assert.equal(role.nullable, false)
    ```

## Scalar

The value is the member, whichever spelling named it: `SELL` for `SELL`, `sell`, `SellSide`, `sell-side`, or its code `2`. Text is read as a spelling and an integer as a code, so `2` is `SELL` while `"2"` names nothing. Rust holds the `PluginSide` member; Python the member of the `yggdryl.PluginSide` `IntEnum`, which is the integer it stores and renders as its stored name; JavaScript the member's name, with `PluginSide` mapping every name to its code ([Enums](index.md#enum-facts-in-the-bindings)).

=== "Rust"

    ```rust
    use yggdryl::{DataType, PluginSide, Scalar, Side};

    let sell = DataType::PluginSide.scalar("SELL")?;
    assert_eq!(sell, Scalar::PluginSide(PluginSide::SellSide));
    assert_eq!(sell.kind(), "pluginside");
    assert_eq!(PluginSide::SellSide.code(), 2);

    // The stored name in any case, the role's own name folded and the code
    // reach one member.
    assert_eq!(DataType::PluginSide.scalar("sell")?, sell);
    assert_eq!(DataType::PluginSide.scalar("SellSide")?, sell);
    assert_eq!(DataType::PluginSide.scalar("sell-side")?, sell);
    assert_eq!(DataType::PluginSide.scalar(2_i32)?, sell);
    assert_eq!(
        DataType::PluginSide.scalar("buy_side")?,
        Scalar::PluginSide(PluginSide::BuySide)
    );

    // A stored code is an integer, never text; the code of no member, and a
    // member of another enum spelled alike, are refused.
    assert!(DataType::PluginSide.scalar("2").is_err());
    assert!(DataType::PluginSide.scalar(3_i32).is_err());
    assert!(DataType::PluginSide.scalar(Scalar::Side(Side::Sell)).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, PluginSide

    role = DataType("pluginside")
    assert role.scalar("SELL").as_py() is PluginSide.SELL
    assert role.scalar("sell-side").as_py() is PluginSide.SELL
    assert role.scalar(2).as_py() is PluginSide.SELL
    assert role.scalar("BuySide").as_py() is PluginSide.BUYS
    assert role.scalar("SELL").kind == "pluginside"

    # A member is the integer it stores and reads as its stored name.
    assert PluginSide.SELL == 2 and str(PluginSide.SELL) == "SELL"

    with pytest.raises(ValueError, match="pluginside"):
        role.scalar("middle")
    with pytest.raises(ValueError, match="pluginside"):
        role.scalar(3)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, PluginSide } = require('yggdryl')

    const role = new DataType('pluginside')
    assert.equal(role.scalar('sell-side').asJs(), 'SELL')
    assert.equal(role.scalar('BuySide').asJs(), 'BUYS')
    assert.equal(role.scalar(PluginSide.SELL).asJs(), 'SELL')
    assert.deepEqual(PluginSide, { UKNW: 0, BUYS: 1, SELL: 2 })
    assert.throws(() => role.scalar('middle'), /pluginside/)
    ```

## Members

The stored name is what `as_str` answers and every text format writes; the role's own name is read beside it, folded the way every other name in this crate folds - ASCII case insensitive, with `_`, `-` and spaces ignored - and never written.

| Code | Name | Also read | Description |
| ---: | --- | --- | --- |
| `0` | `UKNW` | `Unknown` | No plugin role found, or one no member names. |
| `1` | `BUYS` | `BuySide`, `buy-side`, `buy_side` | A Buy-Side plugin: it originates orders and cancels and receives execution reports. |
| `2` | `SELL` | `SellSide`, `sell-side`, `sell_side` | A Sell-Side plugin: it receives orders and cancels and answers with execution reports. |

`from_spelling` answers the member or nothing, and `read` is the same reading as a refusal - the value door; `from_code` and `read_code` read the code.

=== "Rust"

    ```rust
    use yggdryl::PluginSide;

    assert_eq!(PluginSide::ALL, [PluginSide::Unknown, PluginSide::BuySide, PluginSide::SellSide]);
    assert_eq!(PluginSide::default(), PluginSide::Unknown);
    assert_eq!(PluginSide::from_spelling("buys"), Some(PluginSide::BuySide));
    assert_eq!(PluginSide::from_spelling("SELL SIDE"), Some(PluginSide::SellSide));
    assert_eq!(PluginSide::from_spelling("unknown"), Some(PluginSide::Unknown));
    assert_eq!(PluginSide::from_spelling("1"), None);
    assert_eq!(PluginSide::from_code(1), Some(PluginSide::BuySide));
    assert!(PluginSide::read("middle").is_err());
    assert!(PluginSide::SellSide.description().contains("Sell-Side"));
    ```

=== "Python"

    ```python
    from yggdryl import PluginSide

    assert [member.name for member in PluginSide] == ["UKNW", "BUYS", "SELL"]
    assert PluginSide.from_spelling("buys") is PluginSide.BUYS
    assert PluginSide.from_spelling("sell_side") is PluginSide.SELL
    assert PluginSide.from_spelling("1") is None
    assert PluginSide(1) is PluginSide.BUYS
    assert "Sell-Side" in PluginSide.SELL.description
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { PluginSide } = require('yggdryl')

    assert.deepEqual(Object.keys(PluginSide), ['UKNW', 'BUYS', 'SELL'])
    assert.ok(Object.isFrozen(PluginSide))
    assert.equal(PluginSide.BUYS, 1)
    ```

## Arrow storage

`UInt8` under `yggdryl.pluginside`: one value buffer of codes, like every [enum](index.md). Text entering the column is read as a spelling and an integer column of any width as codes, each refused - or null under `safe` in a nullable column - where it names no member; the column cast to text answers each member's stored name, and cast to an integer its code. A [`side`](side.md) column stores `BUYS` and `SELL` under the same two codes and still never lands here: a cast between two enums is refused by name.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray, UInt8Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie, Side};

    let role = Field::new("msgpluginside", DataType::PluginSide, false);
    let arrow = role.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.pluginside");
    assert_eq!(Field::from_arrow_field(&arrow)?, role);

    // Every spelling lands as the code its member stores.
    let spelled: ArrayRef = Arc::new(StringArray::from(vec!["buyside", "SELL", "UKNW"]));
    let stored = Serie::from_arrow_array(Some(&role), spelled, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<UInt8Array>().expect("uint8 codes");
    assert_eq!(codes.values().to_vec(), [1, 2, 0]);

    // A side column is no plugin side, whatever its codes.
    let sides = Serie::from_scalars(
        Field::new("side", DataType::Side, false),
        [Scalar::Side(Side::Buy), Scalar::Side(Side::Sell)],
    )?;
    assert!(sides.cast(&role, ArrowCastOptions::new()).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, PluginSide, Serie

    role = Field("msgpluginside", "pluginside")
    arrow_field = role.into_arrow()
    assert arrow_field.type.storage_type == pa.uint8()
    assert arrow_field.type.extension_name == "yggdryl.pluginside"
    assert Field.from_arrow(arrow_field) == role

    # Every spelling, and an integer column of any width, lands as a member.
    stored = Serie.from_arrow_array(pa.array(["buyside", "SELL", "UKNW"]), role, safe=False)
    assert stored.as_py() == [PluginSide.BUYS, PluginSide.SELL, PluginSide.UKNW]
    coded = Serie.from_arrow_array(pa.array([2, 1], pa.int64()), role)
    assert coded.as_py() == [PluginSide.SELL, PluginSide.BUYS]

    # Out again as the stored name, or as the code.
    assert stored.cast(Field("r", "utf8")).as_py() == ["BUYS", "SELL", "UKNW"]
    assert stored.cast(Field("r", "int16")).as_py() == [1, 2, 0]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { PluginSide, Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['buyside', 'SELL', 'UKNW']), fields.pluginside('role'))
    const codes = stored.intoArrowArray()
    assert.ok(arrow.DataType.isInt(codes.type) && codes.type.bitWidth === 8 && !codes.type.isSigned)
    assert.deepEqual([...codes], [PluginSide.BUYS, PluginSide.SELL, PluginSide.UKNW])
    ```

## A CBlock names its plugin's role

An Ullink CBlock is the configuration of one plugin, and its root element names the plugin's class in its `type` attribute - `com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock`. `from_plugin_type` reads the role off that class: the last `.`-separated segment alone, folded - case, `_`, `-` and blanks dropped - so `BuySide` anywhere in it is `BUYS` and `SellSide` is `SELL`; a class naming neither, a package naming a role rather than the class, and no attribute at all are `UKNW`, never a refusal, because a plugin whose class states no role is a plugin of no stated role. Reading a CBlock under a dialect (`FixRegistry::from_cfb_file`, `add_cfb_file`, `add_cfb_files`, [`yggdryl fix ingest`](../../fix/cli.md#ingest-and-sync)) records that role on the dialect's [catalog entry](../../fix/registry.md#membership), beside the file it was read from; a file read under no dialect records no entry, and the role with it. The role is the entry's and never a field's: a field states only which sources contributed it.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFile;
    use yggdryl::{FixRegistry, PluginSide};

    let cblock = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock";
    assert_eq!(PluginSide::from_plugin_type(&format!("{cblock}.BuySideFIXCPluginCBlock")), PluginSide::BuySide);
    assert_eq!(PluginSide::from_plugin_type(&format!("{cblock}.SellSideFIXCPluginCBlock")), PluginSide::SellSide);
    assert_eq!(PluginSide::from_plugin_type("x.Buy_Side_FIXCPluginCBlock"), PluginSide::BuySide);
    // Only the last segment names the role, and a class naming none is `UKNW`.
    assert_eq!(PluginSide::from_plugin_type("buyside.FIXCPluginCBlock"), PluginSide::Unknown);
    assert_eq!(PluginSide::from_plugin_type(""), PluginSide::Unknown);

    // A CBlock read under a dialect records the role on the dialect's entry.
    let path = std::env::temp_dir().join(format!("ygg-doc-pluginside-{}.cfb", std::process::id()));
    std::fs::write(&path, format!(r#"<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration fix-version="4.4" type="{cblock}.SellSideFIXCPluginCBlock">
      <vocabulary><vocabulary-tag name="20001" alt="VenueFlag" type="string" /></vocabulary>
    </cplugin-configuration>
    "#))?;
    let (venue, _) = FixRegistry::from_cfb_file(&LocalFile::new(&path)?, Some("venue"))?;
    let (bare, _) = FixRegistry::from_cfb_file(&LocalFile::new(&path)?, None)?;
    std::fs::remove_file(&path)?;
    let entry = venue.get_source("venue").expect("the dialect's entry");
    assert_eq!(entry.pluginside(), PluginSide::SellSide);
    assert_eq!(bare.sources().len(), 0, "no dialect, no entry");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import PluginSide
    from yggdryl.fix import FixRegistry

    cblock = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock"
    assert PluginSide.from_plugin_type(f"{cblock}.BuySideFIXCPluginCBlock") is PluginSide.BUYS
    assert PluginSide.from_plugin_type(f"{cblock}.SellSideFIXCPluginCBlock") is PluginSide.SELL
    # Only the last segment names the role, and a class naming none is `UKNW`.
    assert PluginSide.from_plugin_type("buyside.FIXCPluginCBlock") is PluginSide.UKNW

    # A CBlock read under a dialect records the role on the dialect's entry.
    with tempfile.TemporaryDirectory() as directory:
        path = pathlib.Path(directory) / "venue.cfb"
        path.write_text(
            '<?xml version="1.0" encoding="US-ASCII"?>\n'
            f'<cplugin-configuration fix-version="4.4" type="{cblock}.SellSideFIXCPluginCBlock">\n'
            '  <vocabulary><vocabulary-tag name="20001" alt="VenueFlag" type="string" /></vocabulary>\n'
            "</cplugin-configuration>\n"
        )
        venue, _ = FixRegistry.from_cfb_file(path, "venue")
        bare, _ = FixRegistry.from_cfb_file(path)
    assert venue.get_source("venue")["pluginside"] is PluginSide.SELL
    assert bare.sources() == [], "no dialect, no entry"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { fix, pluginSideFromPluginType } = require('yggdryl')

    const cblock = 'com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock'
    assert.equal(pluginSideFromPluginType(`${cblock}.BuySideFIXCPluginCBlock`), 'BUYS')
    assert.equal(pluginSideFromPluginType(`${cblock}.SellSideFIXCPluginCBlock`), 'SELL')
    // Only the last segment names the role, and a class naming none is `UKNW`.
    assert.equal(pluginSideFromPluginType('buyside.FIXCPluginCBlock'), 'UKNW')

    // A CBlock read under a dialect records the role on the dialect's entry.
    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ygg-'))
    const file = path.join(folder, 'venue.cfb')
    fs.writeFileSync(file, `<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration fix-version="4.4" type="${cblock}.SellSideFIXCPluginCBlock">
      <vocabulary><vocabulary-tag name="20001" alt="VenueFlag" type="string" /></vocabulary>
    </cplugin-configuration>
    `)
    const [venue] = fix.FixRegistry.fromCfbFile(file, 'venue')
    const [bare] = fix.FixRegistry.fromCfbFile(file)
    fs.rmSync(folder, { recursive: true, force: true })
    assert.deepEqual(venue.sources(), [{ id: 'venue', file: 'venue.cfb', pluginside: 'SELL' }])
    assert.deepEqual(bare.sources(), [])
    ```

## Edges

- A spelling that names no member, or an integer that is the code of none -> refused naming `pluginside`, never stored; a column typed `pluginside` therefore holds members only, and a value that names none leaves a nullable column null under `safe`. Only the CBlock reading, `from_plugin_type`, turns an unknown class into `UKNW`.
- Text is a spelling and an integer a code: `2` is `SELL`, `"2"` names nothing.
- `BUYS` and `SELL` are spelled like two [sides](side.md) and stored under the same two codes, and are not them: a `side` member is refused at this value door and a `pluginside` member at the side's, and a cast between the two columns is refused by name whatever `safe` says, as between [any two enums](index.md#edges). An integer column of codes, or a text column of spellings, is the way in.
- The default value is `UKNW`, code `0`: a stated value, not an absence. An empty text cell entering the column is null ([Cast](../cast.md#empty-text)), and a required column - the FIX row's `msgpluginside` is one - refuses it.
- A FIX dictionary renders the members as its intrinsic `msgpluginsidecodeset`, which no [registry](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) write may change and a [store](../../fix/store.md#edges) document is held to member for member.
- `msgpluginside` is outside a FIX message's `currhashcode`, `curruuid` and wire: one line read under two sources is one message stamped two ways.
- JSON, TOML, YAML and XML write a role as its stored name; the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code under its own identifier, so a plugin side, a [side](side.md) and an integer of one code are three values.
- `utf8` under `yggdryl.pluginside` is a foreign field wearing the name and imports as the text it is.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- pluginside::
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test fix -- the_root_type_names_the_plugins_role a_codec_reading_under_a_source
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_pluginside.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/pluginside.test.js
    ```
