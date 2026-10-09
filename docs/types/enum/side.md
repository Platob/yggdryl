# Side

Which side of the market a trade took: FIX `Side(54)` as an enum of nineteen members - `UKNW`, the seventeen sides the code set names, and `BOTH` - stored as the `uint8` code of its member, a wire side's code being the position of its wire character.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `side`, the registered kind `SIDE_KIND` under `DataType::Market`, its marker `SideType`, the `Side` enum; `Side::dtype()` and `Side::field(name)` |
| Validates | A member, the code of one, or a spelling one of three vocabularies names - the four-letter code, a FIX wire code, the specification's name - reaches one member; anything else is refused rather than stored |
| Lazy | Nothing - the member table and the vocabularies are static |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | An integer that is the code of no member, naming the code; a spelling that names no side, naming the spelling |
| Stores | `uint8` under `yggdryl.side`: `UKNW` at `0`, then `1` to `17` in FIX's own order, wire codes `1`-`9` then `A`-`H`, and `BOTH` at `99` |
| Default | `UKNW`, a side stated as none: a stated value, never an absence |

A `Side` is one byte in memory and its code in a column. What `as_str` answers and every text format writes is the member's fixed four-letter code - `BUYS`, `SSHT`, `CRSX` - never FIX's one-character code, which `fix_code` answers on its own; the long name is the member's `description`. Former spellings (`BUY`, `SSHORT`, `ASDEF`, `UNKN`, ...) are refused; only the current four-letter member codes are read and written.

## DataType

`side` is the one spelling, `Side::dtype()` the datatype (a `const fn`: what every column of the kind declares) and `Side::field(name)` a nullable field of it, `Side::ID` the id - a [registered kind](../datatype.md#registered-kinds) under `DataType::Market`; kind `enum`. `DataType` holds no constructor per kind: the kind's own type answers its datatype, and the bindings' doors are unchanged (`DataType("side")` in Python, `new DataType('side')` in JavaScript).

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    assert!(matches!(Side::dtype(), DataType::Market(kind) if kind.id() == Side::ID));
    assert_eq!(DataType::from_str("side")?, Side::dtype());
    assert_eq!(Side::dtype().to_string(), "side");
    assert_eq!(Side::dtype().kind(), DataTypeKind::Enum);
    assert_eq!(Side::dtype().id().as_u8(), 0xc3);
    assert!(Side::dtype().is_enum() && !Side::dtype().is_code());
    assert_eq!(Side::dtype().code_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    side = DataType("side")
    assert (side.id, side.kind, side.code_width) == ("side", "enum", None)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const side = new DataType('side')
    assert.equal(side.id, 'side')
    assert.equal(side.kind, 'enum')
    assert.equal(side.codeWidth, null)
    ```

## Field

`SideField` is `FieldOf<SideType>`, `SideType` the kind's marker over the `Market` variant; Python and JavaScript name the factory `side`.

=== "Rust"

    ```rust
    use yggdryl::Field;
    use yggdryl_market::{Side, SideField};
    yggdryl_market::install()?;

    let side = SideField::unit("side", false);
    assert_eq!(side.dtype(), &Side::dtype());
    assert_eq!(side.to_field(), Field::new("side", Side::dtype(), false));
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

The value is the member, whichever vocabulary named it: `BUYS` for FIX's `1`, its `Buy`, the four-letter code itself, or its code `1`. Rust holds the `Side` member; Python the member of the `yggdryl.Side` `IntEnum`, which is the integer it stores and renders as its name; JavaScript the member's name, with `Side` mapping every name to its code ([Enums](index.md#enum-facts-in-the-bindings)).

=== "Rust"

    ```rust
    use yggdryl::Scalar;
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    let buy = Side::dtype().scalar("BUYS")?;
    assert_eq!(buy, Scalar::from(Side::Buy));
    assert_eq!(buy.kind(), "side");
    assert_eq!(Side::Buy.code(), 1);

    // Three vocabularies and the code reach one member.
    assert_eq!(Side::dtype().scalar("1")?, buy);
    assert_eq!(Side::dtype().scalar("BUY")?, buy);
    assert_eq!(Side::dtype().scalar("Buy")?, buy);
    assert_eq!(Side::dtype().scalar(1_i32)?, buy);
    assert_eq!(Side::dtype().scalar("SellShortExempt")?, Scalar::from(Side::SShortEx));

    // A spelling nothing publishes, or the code of no member, answers nothing
    // rather than a guess.
    assert!(Side::dtype().scalar("Z").is_err());
    assert!(Side::dtype().scalar(18_i32).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, Side

    side = DataType("side")
    assert side.scalar("1").as_py() is Side.BUYS
    assert side.scalar("Buy").as_py() is Side.BUYS
    assert side.scalar("BUY").as_py() is Side.BUYS
    assert side.scalar(1).as_py() is Side.BUYS
    assert side.scalar("SellShortExempt").as_py() is Side.SSEX
    assert side.scalar("1").kind == "side"

    # A member is the integer it stores and reads as its code.
    assert Side.BUYS == 1 and str(Side.BUYS) == "BUYS"

    with pytest.raises(ValueError, match="side"):
        side.scalar("Z")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Side } = require('yggdryl')

    const side = new DataType('side')
    assert.equal(side.scalar('1').asJs(), 'BUYS')
    assert.equal(side.scalar('SellShort').asJs(), 'SSHT')
    assert.equal(Side.BUYS, 1)
    assert.throws(() => side.scalar('Z'), /side/)
    ```

## Arrow storage

`UInt8` under `yggdryl.side`: one value buffer of codes, like every [enum](index.md). Text entering a `side` column is read as a spelling and an integer of any width, signed or unsigned, as a code, each refused - or null under `safe` in a nullable column - where it names no member; a `side` column cast to text answers each member's four-letter code, and cast to an integer its code.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray, UInt8Array};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, Field, Serie};
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    let side = Field::new("side", Side::dtype(), false);
    let arrow = side.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::UInt8);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.side");
    assert_eq!(Field::from_arrow_field(&arrow)?, side);

    // Every vocabulary lands as the code its member stores.
    let spelled: ArrayRef = Arc::new(StringArray::from(vec!["Buy", "1", "SELL", "SellShort"]));
    let stored = Serie::from_arrow_array(Some(&side), spelled, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<UInt8Array>().expect("uint8 codes");
    assert_eq!(codes.values().to_vec(), [1, 1, 2, 5]);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Serie, Side

    side = Field("side", "side")
    arrow_field = side.into_arrow()
    assert arrow_field.type.storage_type == pa.uint8()
    assert arrow_field.type.extension_name == "yggdryl.side"
    assert Field.from_arrow(arrow_field) == side

    # Every vocabulary lands as the member it names.
    stored = Serie.from_arrow_array(pa.array(["Buy", "1", "SELL"]), side, safe=False)
    assert stored.as_py() == [Side.BUYS, Side.BUYS, Side.SELL]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Serie, Side, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['Buy', '1', 'SELL']), fields.side('side'))
    assert.deepEqual([...stored.intoArrowArray()], [Side.BUYS, Side.BUYS, Side.SELL])
    ```

## Three vocabularies, one member

A FIX `Side(54)` wire code, the specification's own name, and the four-letter code all name one thing, so they all reach it. Names fold the way every other name in this crate folds - ASCII case insensitive, with `_`, `-` and spaces ignored - so `SellShort`, `sell_short` and `SELL SHORT` are one spelling, and so are `SSHT` and `ssht`. A wire code does **not** fold, because `A` and `a` are different codes in FIX. A code is the member's position, so it is the wire character's position too.

| Code | Wire code | Name | Specification's name | Description |
| ---: | --- | --- | --- | --- |
| `0` | - | `UKNW` | `Unknown` | A side stated as none, which a merge takes the other side over. |
| `1` | `1` | `BUYS` | `Buy` | Buy. |
| `2` | `2` | `SELL` | `Sell` | Sell. |
| `3` | `3` | `BUYM` | `BuyMinus` | Buy minus. |
| `4` | `4` | `SELP` | `SellPlus` | Sell plus. |
| `5` | `5` | `SSHT` | `SellShort` | Sell short. |
| `6` | `6` | `SSEX` | `SellShortExempt` | Sell short exempt. |
| `7` | `7` | `UNDI` | `Undisclosed` | Undisclosed. |
| `8` | `8` | `CROS` | `Cross` | Cross. |
| `9` | `9` | `CRSH` | `CrossShort` | Cross short. |
| `10` | `A` | `CRSX` | `CrossShortExempt` | Cross short exempt. |
| `11` | `B` | `ASDF` | `AsDefined` | As defined. |
| `12` | `C` | `OPPO` | `Opposite` | Opposite. |
| `13` | `D` | `SUBS` | `Subscribe` | Subscribe. |
| `14` | `E` | `REDM` | `Redeem` | Redeem. |
| `15` | `F` | `LEND` | `Lend` | Lend. |
| `16` | `G` | `BORR` | `Borrow` | Borrow. |
| `17` | `H` | `SELU` | `SellUndisclosed` | Sell undisclosed. |
| `99` | - | `BOTH` | - | Both sides at once: a book, or a quote holding its bid and its ask and tagging neither. |

`from_spelling` answers the member or nothing and `read` is the same reading as a refusal; `fix_code` is the wire character, `None` for `UKNW` and `BOTH`, which no message carries. A dialect's own code maps as its dictionary says, because FIX's `Side(54)` reaches these members through the name the [code set it reads by](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) gives each code. Two more names reach `BUYS` and `SELL`: `BuySide` and `SellSide`, the roles a FIX plugin plays, folded like a name - because a plugin's role is the side of the market its session stands on ([below](#a-fix-plugins-role-is-a-side)).

=== "Rust"

    ```rust
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    assert_eq!(Side::from_spelling("1"), Some(Side::Buy));
    assert_eq!(Side::from_spelling("sell_short"), Some(Side::SShort));
    assert_eq!(Side::from_spelling("H"), Some(Side::SellUnd));
    // A wire letter does not fold, and a stored code is never text.
    assert_eq!(Side::from_spelling("h"), None);
    assert_eq!(Side::from_spelling("10"), None);
    assert_eq!(Side::from_code(10), Some(Side::CrossShX));

    assert_eq!(Side::Buy.fix_code(), Some('1'));
    assert_eq!(Side::CrossShX.fix_code(), Some('A'));
    assert_eq!(Side::Unknown.fix_code(), None);
    assert_eq!(Side::Both.fix_code(), None);

    // `read` is the same reading as a refusal naming the spelling.
    assert_eq!(Side::read("Buy")?, Side::Buy);
    assert!(Side::read("X").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Side

    assert Side.from_spelling("1") is Side.BUYS
    assert Side.from_spelling("sell_short") is Side.SSHT
    assert Side.from_spelling("h") is None
    assert Side.from_spelling("10") is None
    assert Side(10) is Side.CRSX

    assert Side.BUYS.fix_code == "1"
    assert Side.CRSX.fix_code == "A"
    assert Side.UKNW.fix_code is None
    assert Side.BOTH.fix_code is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Side } = require('yggdryl')

    // JavaScript reads a spelling through the value door.
    const side = new DataType('side')
    assert.equal(side.scalar('sell_short').asJs(), 'SSHT')
    assert.equal(side.scalar('H').asJs(), 'SELU')
    assert.throws(() => side.scalar('h'), /side/)
    assert.equal(Side.CRSX, 10)
    ```

## Which side of a book

`is_bid` and `is_ask` say which side of a book a side takes: a [book](../../graph/book.md) rests an order among its bid or its ask levels by them, and a one-sided quote tagging the side states the leg that side takes (`bidpx` for a bid, `askpx` for an ask); an order of a side that takes neither, or a quote stating no leg, places nothing - still a delta of its book, warned of, never refused. `BUYS` and `BUYM` take the bid; `SELL`, `SELP`, `SSHT`, `SSEX` and `SELU` take the ask. Everything else takes neither: a cross is both sides at once, `OPPO` means "whatever the other leg was", `BOTH` holds both legs and so takes no one of them, and `ASDF`, `UNDI`, the four remaining sides and `UKNW` say nothing about a side of a book. Domain knowledge written where a reviewer can check it, because Orchestra does not publish it. Rust and Python; JavaScript holds the names only.

=== "Rust"

    ```rust
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    assert!(Side::Buy.is_bid() && !Side::Buy.is_ask());
    assert!(Side::SShortEx.is_ask() && !Side::SShortEx.is_bid());

    // A cross, `OPPOSITE`, a side stated as none and both sides at once
    // take neither.
    assert!(!Side::Cross.is_bid() && !Side::Cross.is_ask());
    assert!(!Side::Opposite.is_bid() && !Side::Opposite.is_ask());
    assert!(!Side::Unknown.is_bid() && !Side::Unknown.is_ask());
    assert!(!Side::Both.is_bid() && !Side::Both.is_ask());
    ```

=== "Python"

    ```python
    from yggdryl import Side

    assert Side.BUYS.is_bid() and not Side.BUYS.is_ask()
    assert Side.SSEX.is_ask() and not Side.SSEX.is_bid()
    assert not Side.CROS.is_bid() and not Side.CROS.is_ask()
    assert not Side.UKNW.is_bid() and not Side.UKNW.is_ask()
    assert not Side.BOTH.is_bid() and not Side.BOTH.is_ask()
    ```

## `UKNW` states no side

`UKNW` is what a value that must state a side states where none was said: a market element's side is never null, and holds `UKNW` until something states one ([Market](../../graph/market.md#contract)). `merge_with` folds two statements of one side: a side stated as none takes the other, and anything stated stands. Rust only.

```rust
use yggdryl_market::Side;
yggdryl_market::install()?;

assert_eq!(Side::default(), Side::Unknown);
assert_eq!(Side::Unknown.merge_with(Side::Sell), Side::Sell);
// Anything stated stands.
assert_eq!(Side::Buy.merge_with(Side::Sell), Side::Buy);
assert_eq!(Side::Buy.merge_with(Side::Unknown), Side::Buy);
```

## `BOTH` holds both sides

`BOTH` (`99`) is the side of an element holding both legs: a [book](../../graph/book.md) always states it, whatever it is set to, and a [quote](../../graph/quote.md) stating no side that quotes a bid and an ask states it once finalized - a tag it states stands, and a quote quoting one leg stays `UKNW`. It takes neither leg, quotes no price onto one, has no wire character and is stated, so a merge keeps it over another side. Neither a book nor a quote is [sided](../../graph/market.md#sides-and-cross-codes), so each still stores its cross code under side `0`.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, Market, QuoteEvent};
    use yggdryl::graph::Element;
    use yggdryl::Decimal;
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    let book = BookEvent::new(1, "ACME");
    assert_eq!(book.get_side(), Side::Both);
    assert_eq!(book.get_crosscode(), "3:0:ACME");

    // A bid and an ask and no tag: both sides, stored under side 0.
    let mut quote = QuoteEvent::at(1);
    quote.set_crosscode("Q-1".to_owned());
    quote.set_bidpx(Some(Decimal::from_int(99)), true);
    quote.set_askpx(Some(Decimal::from_int(101)), true);
    quote.finalize();
    assert_eq!(quote.get_side(), Side::Both);
    assert_eq!(quote.get_crosscode(), "14:0:Q-1");
    assert_eq!(quote.get_price(), None);
    assert_eq!(Side::Both.code(), 99);
    assert_eq!(Side::Unknown.merge_with(Side::Both), Side::Both);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    book = graph.BookEvent(1, "ACME")
    assert book.side is Side.BOTH and book.crosscode == "3:0:ACME"

    # A bid and an ask and no tag: both sides, stored under side 0.
    quote = graph.QuoteEvent(1, crosscode="Q-1", bidpx=Decimal("99"), askpx=Decimal("101"))
    assert quote.side is Side.BOTH and quote.crosscode == "14:0:Q-1"
    assert quote.price is None
    assert Side.BOTH == 99 and Side.from_spelling("both") is Side.BOTH
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Side, graph } = require('yggdryl')

    const book = new graph.BookEvent(1n, 'ACME')
    assert.equal(book.side, 'BOTH')
    assert.equal(book.crosscode, '3:0:ACME')

    // A bid and an ask and no tag: both sides, stored under side 0.
    const quote = new graph.QuoteEvent(1n, { crosscode: 'Q-1', bidpx: '99', askpx: '101' })
    assert.equal(quote.side, 'BOTH')
    assert.equal(quote.crosscode, '14:0:Q-1')
    assert.equal(quote.price, null)
    assert.equal(Side.BOTH, 99)
    ```

## A FIX plugin's role is a side

A FIX plugin stands on one side of its session: a Buy-Side plugin originates orders and cancels and receives execution reports, a Sell-Side one receives them and answers. That role is a side - `BUYS`, `SELL`, or `UKNW` where none is stated - so `BuySide` and `SellSide` are two spellings of the two members, folded the way every name folds (`buy-side`, `sell_side`, `SELL SIDE`), and there is no second enum for it. An Ullink CBlock names its plugin's class in its root's `type` attribute - `com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock.SellSideFIXCPluginCBlock` - and `yggdryl_fix::plugin_side` (Python `yggdryl.fix.plugin_side`, JavaScript `fix.pluginSide`) reads the role off that class: the last `.`-separated segment alone, folded, so `BuySide` anywhere in it is `BUYS` and `SellSide` is `SELL`; a class naming neither, a package naming a role rather than the class, and no attribute at all are `UKNW`, never a refusal, because a plugin whose class states no role is a plugin of no stated role - the one reading that turns an unknown spelling into `UKNW` where the value door refuses it. Reading a CBlock under a dialect (`FixRegistry::from_cfb_file`, `add_cfb_file`, `add_cfb_files`, [`yggdryl fix ingest`](../../fix/cli.md#ingest-and-sync)) records that role on the dialect's [catalog entry](../../fix/registry.md#membership), beside the file it was read from, and a codec reading under that source stamps it on every message as the required [`msgpluginside`](../../fix/capture.md#the-plugins-role-is-the-sources) column, tag 65043 - the session's role, never the order's `Side(54)`. The role is the entry's and never a field's: a field states only which sources contributed it.

=== "Rust"

    ```rust
    use yggdryl_fix::plugin_side;
    use yggdryl::local::LocalFile;
    use yggdryl_fix::FixRegistry;
    use yggdryl::Scalar;
    use yggdryl_market::Side;
    yggdryl_fix::install()?;

    let cblock = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock";
    assert_eq!(plugin_side(&format!("{cblock}.BuySideFIXCPluginCBlock")), Side::Buy);
    assert_eq!(plugin_side(&format!("{cblock}.SellSideFIXCPluginCBlock")), Side::Sell);
    assert_eq!(plugin_side("x.Buy_Side_FIXCPluginCBlock"), Side::Buy);
    // Only the last segment names the role, and a class naming none is `UKNW`.
    assert_eq!(plugin_side("buyside.FIXCPluginCBlock"), Side::Unknown);
    assert_eq!(plugin_side(""), Side::Unknown);
    // The role's own name is a spelling of the side.
    assert_eq!(Side::dtype().scalar("sell-side")?, Scalar::from(Side::Sell));
    assert_eq!(Side::from_spelling("BuySide"), Some(Side::Buy));

    // A CBlock read under a dialect records the role on the dialect's entry.
    let path = std::env::temp_dir().join(format!("ygg-doc-side-plugin-{}.cfb", std::process::id()));
    std::fs::write(&path, format!(r#"<?xml version="1.0" encoding="US-ASCII"?>
    <cplugin-configuration fix-version="4.4" type="{cblock}.SellSideFIXCPluginCBlock">
      <vocabulary><vocabulary-tag name="20001" alt="VenueFlag" type="string" /></vocabulary>
    </cplugin-configuration>
    "#))?;
    let (venue, _) = FixRegistry::from_cfb_file(&LocalFile::new(&path)?, Some("venue"))?;
    let (bare, _) = FixRegistry::from_cfb_file(&LocalFile::new(&path)?, None)?;
    std::fs::remove_file(&path)?;
    let entry = venue.get_source("venue").expect("the dialect's entry");
    assert_eq!(entry.pluginside(), Side::Sell);
    assert_eq!(bare.sources().len(), 0, "no dialect, no entry");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import DataType, Side
    from yggdryl.fix import FixRegistry, plugin_side

    cblock = "com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock"
    assert plugin_side(f"{cblock}.BuySideFIXCPluginCBlock") is Side.BUYS
    assert plugin_side(f"{cblock}.SellSideFIXCPluginCBlock") is Side.SELL
    # Only the last segment names the role, and a class naming none is `UKNW`.
    assert plugin_side("buyside.FIXCPluginCBlock") is Side.UKNW
    # The role's own name is a spelling of the side.
    assert DataType("side").scalar("sell-side").as_py() is Side.SELL
    assert Side.from_spelling("BuySide") is Side.BUYS

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
    assert venue.get_source("venue")["pluginside"] is Side.SELL
    assert bare.sources() == [], "no dialect, no entry"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { DataType, Side, fix } = require('yggdryl')

    const cblock = 'com.ullink.ulbridge2.toolkit.plugins.fix.model.state.cblock'
    assert.equal(fix.pluginSide(`${cblock}.BuySideFIXCPluginCBlock`), 'BUYS')
    assert.equal(fix.pluginSide(`${cblock}.SellSideFIXCPluginCBlock`), 'SELL')
    // Only the last segment names the role, and a class naming none is `UKNW`.
    assert.equal(fix.pluginSide('buyside.FIXCPluginCBlock'), 'UKNW')
    // The role's own name is a spelling of the side.
    assert.equal(new DataType('side').scalar('sell-side').asJs(), 'SELL')
    assert.equal(Side[fix.pluginSide(`${cblock}.BuySideFIXCPluginCBlock`)], 1)

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

- A spelling that names no side, or an integer that is the code of none -> refused naming `side`, never stored; a column typed `side` therefore holds members only, and a value that names none leaves a nullable column null under `safe`.
- A wire code never folds: `A` is `CRSX` and `a` names no side, because they are different FIX codes and a folded lookup would answer the wrong one.
- A name folds: `SellShort`, `sell_short` and `SELL SHORT` are one spelling, `SSHT`.
- A stored code is an integer, never text: `"10"` is no spelling, because `1`-`9` are wire codes and a number read as text would answer the wrong member for one of the two vocabularies; `10` is `CRSX`.
- The default value is `UKNW`, code `0`: a stated value, not an absence. An empty text cell entering the column is null ([Cast](../cast.md#empty-text)), and a required column refuses it.
- An order or an execution taking a side states its numeric code in its stored cross code `{kind}:{side}:{base}` - `10:1:ORD-1` to buy, `10:2:ORD-1` to sell - so the two sides of one identifier are two chains; `UKNW` states `0` there, and no other element - a quote (`14:0:Q-7`), a trade, a book, a snapshot control - states a side but `0` whatever side it takes or tags, `BOTH` on a quote or a book included ([Market](../../graph/market.md#sides-and-cross-codes)).
- In an expression a text constant meets a `side` column as the member it spells and an integer as the code it stores: `side = 'BUYS'`, `side in ('1', 'SellShort')`, `cast('2' as side) = side` and `side < 3` all compare members.
- A Hive partition over a `side` column is named by the member, `side=BUYS`.
- JSON, TOML, YAML and XML write a side as its four-letter code, the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code under the side's own identifier, so a side, a [state](state.md) and an integer of one code are three values.
- `utf8` under `yggdryl.side` is a foreign field wearing the name and imports as the text it is; a side packs into no US-ASCII integer, because its column already holds its code.
- A FIX plugin's role is a side ([above](#a-fix-plugins-role-is-a-side)): `BuySide` and `SellSide` are spellings of `BUYS` and `SELL`, `yggdryl_fix::plugin_side` reads a CBlock's class into one and refuses nothing - a class naming no role is `UKNW` - where the value door refuses a spelling that names none. A FIX dictionary renders the nineteen members as its intrinsic `msgpluginsidecodeset`, which no [registry](../../fix/registry.md#a-field-names-the-code-set-it-reads-by) write may change and a [store](../../fix/store.md#edges) document is held to member for member; `msgpluginside` is outside a FIX message's `hashcode`, `uuid` and wire, so one line read under two sources is one message stamped two ways.
- `yggdryl_market::SIDES` is the listing of the nineteen four-letter codes, sorted, reached by the logical name `side` once `install()` registers it, for a US-ASCII column declaring the vocabulary it holds; `DataType::from_logical_name("side")` is this enum. Python's `yggdryl.enums` declares no side: `yggdryl.Side` is the vocabulary.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl-market --test root -- side::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_side.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/side.test.js
    ```
