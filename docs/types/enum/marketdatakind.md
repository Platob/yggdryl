# MarketDataKind

What kind of market data an element is: FIX's MsgCat code set as an enum of twenty-two members, stored as the `int32` code of its member - the code set's own value, `UNKN` at `0` and `TRAD` at `21` - and the first column of every [market data row](../../graph/market-data.md).

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `marketdatakind`, `MarketDataKindType`/`MarketDataKindField`, the `MarketDataKind` enum and `Scalar::MarketDataKind`; `DataType::marketdatakind()` |
| Validates | A member, the code of one, or a spelling - the four-letter code in any case, or the member's own word folded - reaches one member; anything else is refused rather than stored |
| Lazy | Nothing - the member table is static |
| Cached | The Arrow projection of its [`Field`](../field.md) |
| Refuses | An integer that is the code of no member, naming the code; a spelling that names no kind, naming the spelling |
| Stores | `int32` under `yggdryl.marketdatakind`: the MsgCat value itself |
| Owner | The one owner of the MsgCat set: a FIX dictionary's `FIX:msgcat` resolves to a member by its four-letter code, the crate's `msgcatcodeset` renders from `MarketDataKind::ALL`, and a [FIX message](../../fix/message.md)'s `msgcat` and a market data row's `marketdatakind` both state a member |

A reader tells the leaves of market data apart by one column every FIX engine already speaks: an order is `ORDR`, a quote `QUOT`, an execution `EXEC`, a trade `TRAD` and a book `BOOK`.

## DataType

`marketdatakind` is the one spelling, `DataType::marketdatakind()` the constructor; kind `enum`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeKind};

    assert_eq!(DataType::marketdatakind(), DataType::MarketDataKind);
    assert_eq!(DataType::from_str("marketdatakind")?, DataType::MarketDataKind);
    assert_eq!(DataType::MarketDataKind.to_string(), "marketdatakind");
    assert_eq!(DataType::MarketDataKind.kind(), DataTypeKind::Enum);
    assert_eq!(DataType::MarketDataKind.id().as_u8(), 0xc2);
    assert!(DataType::MarketDataKind.is_enum() && !DataType::MarketDataKind.is_code());
    assert_eq!(DataType::MarketDataKind.code_width(), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType

    kind = DataType("marketdatakind")
    assert (kind.id, kind.kind, kind.code_width) == ("marketdatakind", "enum", None)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const kind = new DataType('marketdatakind')
    assert.equal(kind.id, 'marketdatakind')
    assert.equal(kind.kind, 'enum')
    assert.equal(kind.codeWidth, null)
    ```

## Field

`MarketDataKindField` is the typed marker; Python and JavaScript name the factory `marketdatakind`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, MarketDataKindField};

    let kind = MarketDataKindField::unit("marketdatakind", false);
    assert_eq!(kind.dtype(), &DataType::MarketDataKind);
    assert_eq!(
        kind.to_field(),
        Field::new("marketdatakind", DataType::MarketDataKind, false)
    );
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Field

    kind = yggdryl.marketdatakind("marketdatakind", nullable=False)
    assert isinstance(kind, Field)
    assert kind.dtype == DataType("marketdatakind")
    assert not kind.nullable
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { fields } = require('yggdryl')

    const kind = fields.marketdatakind('marketdatakind', { nullable: false })
    assert.equal(kind.dtype.toString(), 'marketdatakind')
    assert.equal(kind.nullable, false)
    ```

## Scalar

The value is the member, whichever spelling named it: `ORDR` for `ORDR`, `ordr`, `order` or the code `10`. Rust holds the `MarketDataKind` member; Python the member of the `yggdryl.MarketDataKind` `IntEnum`, which is the integer it stores and renders as its four-letter name; JavaScript the member's name, with `MarketDataKind` mapping every name to its code ([Enums](index.md#enum-facts-in-the-bindings)).

=== "Rust"

    ```rust
    use yggdryl::{DataType, MarketDataKind, Scalar};

    let order = DataType::MarketDataKind.scalar("ORDR")?;
    assert_eq!(order, Scalar::MarketDataKind(MarketDataKind::Order));
    assert_eq!(order.kind(), "marketdatakind");
    assert_eq!(MarketDataKind::Order.code(), 10);

    // The four-letter code in any case, the member's word and the code reach
    // one member.
    assert_eq!(DataType::MarketDataKind.scalar("ordr")?, order);
    assert_eq!(DataType::MarketDataKind.scalar("order")?, order);
    assert_eq!(DataType::MarketDataKind.scalar(10_i32)?, order);
    assert_eq!(
        DataType::MarketDataKind.scalar("market_structure")?,
        Scalar::MarketDataKind(MarketDataKind::MarketStructure)
    );

    // A stored code is an integer, never text; the code of no member answers
    // nothing.
    assert!(DataType::MarketDataKind.scalar("10").is_err());
    assert!(DataType::MarketDataKind.scalar(22_i32).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, MarketDataKind

    kind = DataType("marketdatakind")
    assert kind.scalar("ORDR").as_py() is MarketDataKind.ORDR
    assert kind.scalar("quotation").as_py() is MarketDataKind.QUOT
    assert kind.scalar(21).as_py() is MarketDataKind.TRAD
    assert kind.scalar("ORDR").kind == "marketdatakind"

    # A member is the integer it stores and reads as its name.
    assert MarketDataKind.EXEC == 8 and str(MarketDataKind.EXEC) == "EXEC"

    with pytest.raises(ValueError, match="marketdatakind"):
        kind.scalar("10")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, MarketDataKind } = require('yggdryl')

    const kind = new DataType('marketdatakind')
    assert.equal(kind.scalar('ordr').asJs(), 'ORDR')
    assert.equal(kind.scalar('quotation').asJs(), 'QUOT')
    assert.equal(MarketDataKind.TRAD, 21)
    assert.throws(() => kind.scalar('10'), /marketdatakind/)
    ```

## Arrow storage

`Int32` under `yggdryl.marketdatakind`: one value buffer of codes, like every [enum](index.md). Text entering the column is read as a spelling and an integer as a code, each refused - or null under `safe` in a nullable column - where it names no member; the column cast to text answers each member's four-letter name, and cast to an integer its code.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, Int32Array, StringArray};
    use arrow_schema::DataType as ArrowDataType;
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    let kind = Field::new("marketdatakind", DataType::MarketDataKind, false);
    let arrow = kind.clone().into_arrow_field()?;
    assert_eq!(arrow.data_type(), &ArrowDataType::Int32);
    assert_eq!(arrow.metadata()["ARROW:extension:name"], "yggdryl.marketdatakind");
    assert_eq!(Field::from_arrow_field(&arrow)?, kind);

    // Spellings land as the codes their members store.
    let spelled: ArrayRef = Arc::new(StringArray::from(vec!["ORDR", "quotation", "exec"]));
    let stored = Serie::from_arrow_array(Some(&kind), spelled, ArrowCastOptions::new())?
        .into_arrow_array()
        .expect("a column, not a run");
    let codes = stored.as_any().downcast_ref::<Int32Array>().expect("int32 codes");
    assert_eq!(codes.values().to_vec(), [10, 14, 8]);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, MarketDataKind, Serie

    kind = Field("marketdatakind", "marketdatakind")
    arrow_field = kind.into_arrow()
    assert arrow_field.type == pa.int32()
    assert arrow_field.metadata[b"ARROW:extension:name"] == b"yggdryl.marketdatakind"
    assert Field.from_arrow(arrow_field) == kind

    stored = Serie.from_arrow_array(pa.array(["ORDR", "quotation"]), kind, safe=False)
    assert stored.as_py() == [MarketDataKind.ORDR, MarketDataKind.QUOT]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { MarketDataKind, Serie, fields } = require('yggdryl')

    const utf8 = (values) => arrow.vectorFromArray(values, new arrow.Utf8())
    const stored = Serie.fromArrowArray(utf8(['ORDR', 'quotation']), fields.marketdatakind('kind'))
    assert.deepEqual([...stored.intoArrowArray()], [MarketDataKind.ORDR, MarketDataKind.QUOT])
    ```

## The members

The code is the MsgCat value, the stored name its four-letter code, and the word is what `from_spelling` reads beside the code, folded - `market_structure`, `Market Structure`. Every member states what it means (`description`).

| Code | Stored name | Word | Category |
| ---: | --- | --- | --- |
| `0` | `UNKN` | `unknown` | No published category: a message type the dictionary does not file |
| `1` | `ACCT` | `account` | Account reporting |
| `2` | `ALLO` | `allocation` | Allocation instructions, reports and acknowledgements |
| `3` | `BOOK` | `book` | Market data: books, their snapshots, increments and requests |
| `4` | `CERT` | `certificate` | Certificate handling |
| `5` | `COLL` | `collateral` | Collateral management |
| `6` | `COMM` | `communication` | Communication: news and email |
| `7` | `CONF` | `confirmation` | Confirmation and affirmation |
| `8` | `EXEC` | `execution` | Execution reports and their acknowledgements |
| `9` | `MKST` | `marketstructure` | Market structure reference data |
| `10` | `ORDR` | `order` | Order handling: single, list, cross, multileg and mass |
| `11` | `PAYM` | `payment` | Pay management |
| `12` | `POSN` | `position` | Position maintenance |
| `13` | `PRTY` | `parties` | Parties reference data and risk limits reporting |
| `14` | `QUOT` | `quotation` | Quotation and negotiation |
| `15` | `REGI` | `registration` | Registration instructions |
| `16` | `RISK` | `risk` | Party risk limits |
| `17` | `SECU` | `securities` | Securities reference data |
| `18` | `SESS` | `session` | Session, application sequencing and business rejects |
| `19` | `SETL` | `settlement` | Settlement instructions and obligations |
| `20` | `STRM` | `stream` | Stream assignment |
| `21` | `TRAD` | `trade` | Trade capture and matching |

=== "Rust"

    ```rust
    use yggdryl::MarketDataKind;

    assert_eq!(MarketDataKind::ALL.len(), 22);
    assert!(MarketDataKind::ALL.windows(2).all(|pair| pair[0].code() < pair[1].code()));
    assert_eq!(MarketDataKind::from_code(3), Some(MarketDataKind::Book));
    assert_eq!(MarketDataKind::from_name("TRAD"), Some(MarketDataKind::Trade));
    assert_eq!(MarketDataKind::from_spelling("Market Structure"), Some(MarketDataKind::MarketStructure));
    assert_eq!(MarketDataKind::Execution.as_str(), "EXEC");
    assert!(MarketDataKind::Quotation.description().starts_with("Quotation"));
    ```

=== "Python"

    ```python
    from yggdryl import MarketDataKind

    assert [int(kind) for kind in MarketDataKind] == list(range(22))
    assert MarketDataKind(3) is MarketDataKind.BOOK
    assert MarketDataKind.from_spelling("Market Structure") is MarketDataKind.MKST
    assert MarketDataKind.from_spelling("10") is None
    assert MarketDataKind.QUOT.description.startswith("Quotation")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MarketDataKind } = require('yggdryl')

    assert.deepEqual(Object.values(MarketDataKind), Array.from({ length: 22 }, (_, code) => code))
    assert.equal(MarketDataKind.BOOK, 3)
    assert.ok(Object.isFrozen(MarketDataKind))
    ```

## The category of a market data leaf

Every [market data](../../graph/market-data.md) leaf is filed under one member, which its `marketdatakind` column states and a leaf answers without a lookup: `MarketKind::marketdatakind` in Rust, the `marketdatakind` getter on every leaf in Python and JavaScript. A FIX message states the member its dictionary files its message type under, `FixMsg::msgcat`, `UNKN` where it files none ([FIX message](../../fix/message.md)).

| Leaf | Member |
| --- | --- |
| `Order`, `OrderEvent` | `ORDR` |
| `Quote`, `QuoteEvent` | `QUOT` |
| `Execution`, `ExecutionEvent` | `EXEC` |
| `TradeEvent` | `TRAD` |
| `BookEvent`, `SnapshotEvent` | `BOOK` |

=== "Rust"

    ```rust
    use yggdryl::graph::MarketKind;
    use yggdryl::MarketDataKind;

    assert_eq!(MarketKind::OrderEvent.marketdatakind(), MarketDataKind::Order);
    assert_eq!(MarketKind::Quote.marketdatakind(), MarketDataKind::Quotation);
    assert_eq!(MarketKind::ExecutionEvent.marketdatakind(), MarketDataKind::Execution);
    assert_eq!(MarketKind::TradeEvent.marketdatakind(), MarketDataKind::Trade);
    assert_eq!(MarketKind::SnapshotEvent.marketdatakind(), MarketDataKind::Book);
    ```

=== "Python"

    ```python
    from yggdryl import MarketDataKind, graph

    order = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side="BUY")
    assert order.marketdatakind is MarketDataKind.ORDR
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'BUY' })
    assert.equal(order.marketdatakind, 'ORDR')
    ```

## Edges

- A spelling that names no kind, or an integer that is the code of none -> refused naming `marketdatakind`, never stored; a value that names none leaves a nullable column null under `safe`.
- A stored code is an integer, never text: `"10"` is no spelling, `10` is `ORDR`.
- The four-letter code folds case only - `ORDR`, `ordr`, `Ordr`; the word folds the way every name in this crate folds, ASCII case insensitive with `_`, `-` and spaces ignored.
- The default value is `UNKN`, code `0`: a stated value, not an absence. An empty text cell entering the column is null ([Cast](../cast.md#empty-text)), and a required column refuses it.
- In an expression a text constant meets a `marketdatakind` column as the member it spells and an integer as the code it stores: `kind = 'ORDR'`, `kind in ('ORDR', 'quotation')`, `cast('exec' as marketdatakind) = kind` and `kind >= 14` all compare members.
- JSON, TOML, YAML and XML write a kind as its four-letter name, a Hive partition is named by it, and the [value stream](../value-stream.md) and a digest feed its four-byte little-endian code under the kind's own identifier, so a kind, a [state](state.md) and an integer of one code are three values.
- `utf8` under `yggdryl.marketdatakind` is a foreign field wearing the name and imports as the text it is; a [state](state.md) column cast into a kind is read again member by member, and a state's code names no kind.
- The member is not the leaf: `ORDR` files an order and its dated event alike, and `BOOK` a book and a snapshot control, so `MarketKind` is what names the leaf.

## Commands

=== "Rust"

    ```bash
    cargo test --manifest-path rust/Cargo.toml -p yggdryl --test root -- marketdatakind::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_marketdatakind.py -q
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/marketdatakind.test.js
    ```
