# Order

`market::Order` is one order's life: the chain a venue's identifiers name, its state, the quantity ordered against what filled and what is left, its price ladder, its side, its instrument and how long it stands - one statement per message that says anything about the order, chained.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/order.rs`: the `Order` trait, the `OrderData` holder, `Pricing`, the row and the `Product` answers; the FIX reading is `FixCodec::orders` beside the [codec](../fix/index.md) |
| Chain | the order's: `crosscode` is the venue's `OrderID(37)`, else the `OrigClOrdID(41)` a request is about - so a replace or a cancel names the order it amends - else the `ClOrdID(11)`; `crossuuid` is the identity it derives, and every statement of one order stands in that chain whichever identifier it spelled |
| Facts | `px` the limit, `stoppx` the stop, `avgpx` what filled averaged; `qty` what was ordered, `cumqty` what filled, `leavesqty` what is left; `ordtype` the type as the wire spelled it, `FIX`'s `OrdType(40)`; the side, the currency, the unit, the ticker and the six codes; `tif` and `expirunix` how long it stands |
| Reads | what the stated facts imply, computed on every call and never stored: `pricing()` - `Market`, `Limit`, `Stop` or `StopLimit`, the stated type read as one of the four, else what the limit and the stop imply, and nothing for a type outside them; `remaining()` what is left, `filled()` what traded and `filled_ratio()` their share, each from whichever of the three quantities the order states; `notional()` its worth; `is_resting()` whether it rests on a ladder right now, and `is_alive()` whether it can still change |
| Chained | a statement takes the order's terms its predecessor stated where it restates none - the limit, the quantity, the stop, the type, what filled and at what average - because what an order was placed as does not vanish when a report is silent about it |
| Doors | `FixCodec::orders(messages)` over a placement (`35=D`), a replace (`G`), a cancel (`F`), an execution report (`8`) and a cancel reject (`9`); `orders_arrow_reader(source)` under `OrderData::field()` |
| Writes back | `FixMsg::from_order(&codec, &order)` states a new order single, `35=D`, exactly: `ClOrdID(11)` from the name the order goes by, else its chain, refused naming `clordid` where it has neither; `OrdType(40)` the type the order states, else what its pricing implies; the instrument, the side, the quantity, the prices, the time in force and the expiry; the instant as `TransactTime(60)` and `SendingTime(52)` |
| Bindings | Rust `Order`/`OrderData`; Python `yggdryl.market.Order`, `FixCodec.orders`, `orders_arrow_reader`, `FixMsg.from_order`; JavaScript `market.Order`, `orders`, `ordersArrowReader`, `FixMsg.fromOrder` |

## Use

One order's life: the placement under the client's identifier, the acknowledgement under the venue's, a partial fill and the fill naming the venue's alone.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Order, OrderData, Pricing, Product};
    use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 4] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|59=0|15=USD|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|6=10.5|55=AAPL|54=1|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.5|32=60|6=10.5|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|",
    ];
    let orders: Vec<OrderData> = reader.orders(reader.parse_lines(lines)).collect::<yggdryl::Result<_>>()?;
    let [placed, acked, partial, filled] = orders.as_slice() else { panic!("four statements") };
    // The chain is the order's, whichever identifier each message spelled.
    assert!(orders.iter().all(|held| held.get_crosscode() == "A1"));
    assert_eq!(orders.iter().map(Event::get_seqnum).collect::<Vec<_>>(), [0, 1, 2, 3]);
    assert_eq!(filled.get_prevuuid(), Some(partial.get_curruuid()));
    assert_eq!(filled.get_identifiers()["orderid"], "O1");
    // The stated facts, and the terms carried forward where a report is silent.
    assert_eq!(placed.get_px(), "10.5".parse()?);
    assert_eq!(placed.get_qty(), Decimal18::from_int(100));
    assert_eq!(filled.get_px(), "10.5".parse()?, "the limit it was placed at");
    assert_eq!(filled.get_tif(), Some("0"));
    // What they imply, read on every call.
    assert_eq!(placed.pricing(), Some(Pricing::Limit));
    assert_eq!(partial.remaining(), Decimal18::from_int(60));
    assert_eq!(partial.filled(), Decimal18::from_int(40));
    assert_eq!(partial.filled_ratio(), Some("0.4".parse()?));
    assert!(partial.is_resting() && !filled.is_resting(), "a filled order rests nothing");
    assert_eq!(placed.notional(), Some(Decimal18::from_int(1_050)));
    // The state each report stated, and a filled order that ended its chain.
    assert_eq!(placed.get_state().as_str(), "00UNKNOWN", "a placement states no status");
    assert_eq!(filled.get_state().as_str(), "80FILLED");
    assert!(!filled.is_alive());
    // The placement states a new order single, dated by the order.
    let message = FixMsg::from_order(&reader, placed)?;
    assert_eq!(message.header().msgtype(), "D");
    assert_eq!(message.by_tag(40)?.as_str(), Some("2"), "a limit order");
    assert_eq!(message.get_currunix(), placed.get_currunix());
    assert_eq!(message.get_srcuuids(), [placed.get_curruuid()]);
    // The row: the sixteen event columns, then the order's own.
    let field = OrderData::field()?;
    assert_eq!(field.fields()[16].name(), "px");
    assert_eq!(field.fields().last().map(yggdryl::Field::name), Some("ordtype"));
    assert_eq!(OrderData::from_row(&field, &placed.into_row()?)?.get_curruuid(), placed.get_curruuid());
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixMsg, FixRegistry
    from yggdryl.market import Order

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|59=0|15=USD|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|6=10.5|55=AAPL|54=1|52=20260102-10:15:31.100|10=0|",
        b"8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.5|32=60|6=10.5|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|",
    ]
    placed, acked, partial, filled = reader.orders(reader.parse_lines(lines))
    # The chain is the order's, whichever identifier each message spelled.
    assert placed.crosscode == "A1" and filled.crosscode == "A1"
    assert [held.seqnum for held in (placed, acked, partial, filled)] == [0, 1, 2, 3]
    assert filled.prevuuid == partial.curruuid
    assert filled.identifiers["orderid"] == "O1"
    # The stated facts, and the terms carried forward where a report is silent.
    assert placed.px.as_py() == Decimal("10.5")
    assert placed.qty.as_py() == Decimal("100")
    assert filled.px.as_py() == Decimal("10.5")  # the limit it was placed at
    assert filled.tif == "0"
    # What they imply, read on every call.
    assert placed.pricing == "limit"
    assert partial.remaining.as_py() == Decimal("60")
    assert partial.filled.as_py() == Decimal("40")
    assert partial.filled_ratio.as_py() == Decimal("0.4")
    assert partial.is_resting and not filled.is_resting
    assert placed.notional.as_py() == Decimal("1050")
    # The state each report stated, and a filled order that ended its chain.
    assert placed.state.as_py() == "00UNKNOWN"
    assert filled.state.as_py() == "80FILLED"
    assert not filled.is_alive
    # The placement states a new order single, dated by the order.
    message = FixMsg.from_order(reader, placed)
    assert message.header().msgtype == "D"
    assert message.by_tag(40).as_py() == "2"  # a limit order
    assert message.currunix == placed.currunix
    assert message.srcuuids == [placed.curruuid]
    # The row: the sixteen event columns, then the order's own.
    field = Order.field()
    assert field.field_at(16).name == "px"
    assert [child.name for child in field][-1] == "ordtype"
    assert Order.from_row(field, placed.into_row()).curruuid == placed.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, market } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|59=0|15=USD|52=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=1|38=100|14=40|151=60|31=10.5|32=40|6=10.5|55=AAPL|54=1|52=20260102-10:15:31.100|10=0|',
      '8=FIX.4.4|35=8|37=O1|17=E2|150=F|39=2|38=100|14=100|151=0|31=10.5|32=60|6=10.5|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|',
    ].map((line) => Buffer.from(line))
    const [placed, acked, partial, filled] = [...reader.orders(reader.parseLines(lines))]
    // The chain is the order's, whichever identifier each message spelled.
    assert.equal(placed.crosscode, 'A1')
    assert.deepEqual([placed, acked, partial, filled].map((held) => held.seqnum), [0, 1, 2, 3])
    assert.equal(filled.prevuuid, partial.curruuid)
    assert.equal(filled.identifiers.orderid, 'O1')
    // The stated facts, and the terms carried forward where a report is silent.
    assert.equal(placed.px, '10.5')
    assert.equal(placed.qty, '100')
    assert.equal(filled.px, '10.5') // the limit it was placed at
    assert.equal(filled.tif, '0')
    // What they imply, read on every call.
    assert.equal(placed.pricing, 'limit')
    assert.equal(partial.remaining, '60')
    assert.equal(partial.filled, '40')
    assert.equal(partial.filledRatio, '0.4')
    assert.ok(partial.isResting && !filled.isResting)
    assert.equal(placed.notional, '1050')
    // The state each report stated, and a filled order that ended its chain.
    assert.equal(placed.state, '00UNKNOWN')
    assert.equal(filled.state, '80FILLED')
    assert.equal(filled.isAlive, false)
    // The placement states a new order single, dated by the order.
    const message = fix.FixMsg.fromOrder(reader, placed)
    assert.equal(message.header().msgtype, 'D')
    assert.equal(message.byTag(40).asJs(), '2') // a limit order
    assert.equal(message.currunix, placed.currunix)
    assert.deepEqual(message.srcuuids, [placed.curruuid])
    // The row: the sixteen event columns, then the order's own.
    const field = market.Order.field()
    assert.equal(field.fieldAt(16).name, 'px')
    assert.equal(field.fieldAt(34).name, 'ordtype') // the last column
    assert.equal(market.Order.fromRow(field, placed.intoRow()).curruuid, placed.curruuid)
    ```

## Row schema

`OrderData::field()` opens with the sixteen [event columns](../graph.md#columns), then the market columns, then the order's own.

| column | datatype | value |
| --- | --- | --- |
| `px` | `decimal128(38, 18)` | nullable; the limit, `Price(44)` |
| `avgpx` | `decimal128(38, 18)` | nullable; what filled averaged, `AvgPx(6)` |
| `qty` | `decimal128(38, 18)` | nullable; what was ordered, `OrderQty(38)` |
| `cumqty` | `decimal128(38, 18)` | nullable; what filled, `CumQty(14)` |
| `leavesqty` | `decimal128(38, 18)` | nullable; what is left, `LeavesQty(151)` |
| `side` | `side` | required; `UNKNOWN` where none is stated |
| `currency` | `currency` | required; `XXX` where none is stated |
| `unit` | `utf8` | nullable; the unit the quantity is counted in |
| `tif` | `timeinforce` | nullable; how long the order stands, `TimeInForce(59)` |
| `tradable` | `bool` | nullable; whether it can trade, where the venue says |
| `symbolticker` | `utf8` | nullable; the ticker the instrument is known by |
| `isincode`, `cusipcode`, `sedolcode`, `bloombergcode` | `isin`, `cusip`, `sedol`, `bloomberg` | nullable; the instrument under each identifier the market named it by |
| `cficode` | `cfi` | nullable; the instrument's classification |
| `miccode` | `mic` | nullable; the market it was sent to |
| `stoppx` | `decimal128(38, 18)` | nullable; the stop price |
| `ordtype` | `utf8` | nullable; the type as the wire spelled it |

The readings have no columns: `pricing`, `remaining`, `filled`, `filled_ratio`, `notional` and `is_resting` are what the stated facts imply, so a column for one would restate another. The step before a statement is its predecessor row, reached by `prevuuid`. An order's table joins the [message table](../fix/capture.md#the-crates-own-columns) on `srcuuids` to `curruuid`, the [execution table](execution.md) on `crossuuid`, and its own table on `prevuuid` and `parentuuids`.

## Edges

- A cancel request or a status report states nothing of the ladder, so it restates nothing: the order's terms ride forward from the statement before it, and the [book](iterator.md) leaves its lanes where they stand.
- A type outside the four - a pegged order, a market-on-close - answers no `pricing`, because it prices some other way; `ordtype` still states what the wire spelled, and `from_order` writes it back unchanged.
- `remaining` reads `leavesqty` first, then what was ordered less what filled, then what was ordered: a report that states only one of the three is still read.
- `from_order` refuses an order that goes by no client identifier and names no chain, naming `clordid`: a new order single states one.

## Commands

```bash
cargo test -p yggdryl --test market order::
cargo test -p yggdryl --doc market::order
cargo bench -p yggdryl --bench market -- "market/doors/orders|market/rows/order"
```
