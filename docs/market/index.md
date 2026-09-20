# Market

A message is what a venue said; a product is what the market did. `yggdryl::market` holds five products the crate reads out of a stream of messages - an [order](order.md), a [quote](quote.md), an [execution](execution.md), a [trade](trade.md) and a [book](book.md) - each a trait beside its holder, each a value of this crate with its own layout, its own Arrow row and its own identity, and each a [graph event](../graph.md) exactly as a message is, so its row opens with the same sixteen columns and joins the message table on `srcuuids` to `curruuid` with no mapping.

## Pages

| Page | Purpose |
| --- | --- |
| [Order](order.md) | one order's life: the chain a venue's identifiers name, its state, the quantity ordered against what filled and what is left, its price ladder, side, instrument, validity |
| [Quote](quote.md) | a price stated at an instant: one or two lanes, each a price, a size, a currency and a unit, under the quote's own identifiers and its validity |
| [Execution](execution.md) | one fill: the price and quantity that traded, the execution's own identifier, the order it belongs to |
| [Trade](trade.md) | the settled transaction an execution reports: the matched quantity at its price, its identifier, its parties and its clocks |
| [Book](book.md) | the ladder for one instrument at one instant: levels of price, size and order count per side, to a declared depth, with every reading they imply |
| [Book iterator](iterator.md) | the `Symbol` a book is keyed by, the `Statement` a mixed stream holds, and the `BookIterator` that reads one book per symbol per instant out of one |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `rust/src/market/`: one file per product, `symbol.rs`, `statement.rs` and `iterator.rs` for what reads books, and `product.rs` for what the five share - `MarketColumn`, the columns the market's facts are stated in; the `Product` trait, what a product answers about its row; `Folded`, the fold of two statements of one product; `Walked`, the one walk over a fallible stream; `arrow_reader` and `read_arrow_reader`, the row doors every Arrow twin composes. Nothing about a protocol is here: the FIX reading of a product lives beside the [codec](../fix/index.md), so this stays a vocabulary a second protocol could feed |
| Trait beside holder | each product is a trait - `Order`, `Quote`, `Execution`, `Trade`, `Book` - carrying its own accessors and mutators and the readings its stated facts imply, beside the holder that stores them - `OrderData`, `QuoteData`, `ExecutionData`, `TradeData`, `BookData`. A `get_`/`set_` pair reads and records a stated fact; a bare noun or an `is_` predicate reads what the stated facts imply, on every call, and stores nothing, so no fact has two owners |
| Event | every holder implements `Element`, `Event` and `MarketElement` - so `MarketEvent` - through the [`MarketEventData`](../graph.md) it holds, exactly as a message does: import the [graph traits](../graph.md) to read `get_curruuid`, `get_crosscode`, `get_state`, `get_px` and the rest, and the product's own trait for what it adds; `event()` borrows the holder |
| Identity | what the content digests to, settled by `finalize`: the event's facts, the market's facts and the product's own, never an instant, never a source, so two statements of one product are one identity, and a walked statement that moved is settled again |
| Sources | `srcuuids` are the identities of the messages the product was read from - provenance, never lineage, and no walk moves it. The third hop of the chain the crate already has: a line's `curruuid` is a message's `srcuuids`, a message's `curruuid` is a product's |
| Chain | `parentuuids`, `prevuuid`, `prevunix` and `seqnum` are the product's own chain, filled by the walk and nothing else; `crosscode` names the chain a product belongs to and `crossuuid` is the identity every statement of it shares - an order's is the order's, an execution's the order it fills, a trade's the match, a quote's the quote, a book's the symbol; `state` and `expirunix` fold forward through the walk as they do for messages |
| Row | `P::field()` (`BookData::field(depth)`): `EventColumn::ALL` first - the same sixteen columns, names and datatypes every event's row opens with - then the market columns the product publishes, then its own; `into_row` and `from_row` are the doors, `MarketColumn` names the market columns as `EventColumn` names the sixteen. Every column is typed as the thing it holds - `decimal128(38, 18)` for a price or a quantity, `side`, `state`, `currency`, `mic`, `cfi`, `timeinforce` and the securities identifiers as the code leaves they are, `uuid` for an identity, nanosecond UTC clocks for an instant, `uint64` for a count - never as the text a venue spelled; a column that restates a fact another column holds does not exist, so a reading has no column and a walked step before the product is its predecessor row, reached by `prevuuid` |
| Doors | one per product on `FixCodec`, over a stream of messages, because a product is rarely one message: `orders`, `quotes`, `executions`, `trades`, `books(depth, snapshot_ns)`; `statements`, the one stream of every statement a capture makes; and one over an Arrow reader of message rows, `orders_arrow_reader` and the other four, which is the message door over the messages the rows hold, row for row. Every door runs [`lifecycle`](../fix/lifecycle.md) first, reads a statement out of each message that makes one, folds the statements of one instant that are one product by the product's own `merge_with` - a message logged at two hops - and walks the rest with the one [walk](../graph.md), so a statement carries its predecessor, its place and its lineage |
| Writes back | `FixMsg::from_order` states a new order single and `FixMsg::from_execution` an execution report, exactly, dated by the product and naming it as the message's one source; `from_quote`, `from_trade` and `from_book` refuse, each with the one sentence on why the crate does not guess - a product is a reading of many messages, and only some readings have one message that states them |
| Tables | a product row is an Arrow row, so Parquet, Iceberg and IPC come for free; a table is keyed on `curruuid` and laid out by the hour of `currunix` - an Iceberg spec takes an `hour` transform over that column - and `uint64` has no Iceberg type, so anything written goes through `into_scheme_compat(&Scheme::ICEBERG)`, which widens a code to `decimal(20, 0)` |
| Bindings | Rust; Python `yggdryl.market` - `Order`, `Quote`, `Execution`, `Trade`, `Book`, the streams `Orders`, `Quotes`, `Executions`, `Trades`, `Books`, `Statements`, plus `Symbol` and `BookIterator`, the doors on `FixCodec` and the writers on `FixMsg`; JavaScript `market` with the same classes, the doors and writers camel-cased |

## Use

One order's life read out of its placement and the reports against it: the orders the messages state, chained; the fills; and the row every order publishes, joined to the message it was read from.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::graph::{Element, Event, MarketElement};
    use yggdryl::local::Folder;
    use yggdryl::market::{Execution, ExecutionData, Order, OrderData, Product};
    use yggdryl::{Decimal18, FixCodec, FixMsg, FixRegistry};

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../config/fix");
    let registry = Arc::new(FixRegistry::from_handle(&Folder::new(root)?)?);
    let reader = FixCodec::new(Arc::clone(&registry));
    let lines: [&[u8]; 3] = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|",
    ];
    let messages: Vec<FixMsg> = reader.parse_lines(lines).collect::<yggdryl::Result<_>>()?;

    // Three messages about one order are three statements of it, chained.
    let orders: Vec<OrderData> = reader.orders(messages.clone()).collect::<yggdryl::Result<_>>()?;
    let [placed, acked, filled] = orders.as_slice() else { panic!("three statements") };
    assert!(orders.iter().all(|held| held.get_crosscode() == "A1"));
    assert_eq!(orders.iter().map(Event::get_seqnum).collect::<Vec<_>>(), [0, 1, 2]);
    assert_eq!(filled.get_prevuuid(), Some(acked.get_curruuid()));
    assert_eq!(placed.get_qty(), Decimal18::from_int(100));
    assert_eq!(filled.get_state().as_str(), "80FILLED");
    // What the stated facts imply is a reading, never a column.
    assert!(placed.is_resting() && !filled.is_resting());
    assert_eq!(filled.remaining(), Decimal18::ZERO);

    // The third hop: each statement names the message it was read from.
    let chained: Vec<FixMsg> = reader.lifecycle(messages.clone()).collect::<yggdryl::Result<_>>()?;
    assert_eq!(filled.get_srcuuids(), [chained[2].get_curruuid()]);

    // A fill is one report: the narrow door reads one execution, in the
    // order's chain.
    let fills: Vec<ExecutionData> = reader.executions(messages).collect::<yggdryl::Result<_>>()?;
    assert_eq!(fills.len(), 1);
    assert_eq!(fills[0].get_crossuuid(), placed.get_crossuuid());
    assert_eq!(fills[0].get_identifiers()["execid"], "E1");
    assert!(fills[0].completes(), "the fill that finished the order");

    // The row opens with the sixteen event columns, then the order's own.
    let field = OrderData::field()?;
    assert_eq!(field.field_at(0)?.name(), "currunix");
    assert_eq!(field.field_at(16)?.name(), "px");
    let row = filled.into_row()?;
    assert_eq!(OrderData::from_row(&field, &row)?.get_curruuid(), filled.get_curruuid());
    ```

=== "Python"

    ```python
    from decimal import Decimal
    from pathlib import Path

    from yggdryl.fix import FixCodec, FixRegistry
    from yggdryl.market import Order

    registry = FixRegistry.from_handle(Path("config/fix").resolve())
    reader = FixCodec(registry)
    lines = [
        b"8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.250|10=0|",
        b"8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|",
        b"8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|",
    ]
    messages = list(reader.parse_lines(lines))

    # Three messages about one order are three statements of it, chained.
    placed, acked, filled = reader.orders(messages)
    assert {held.crosscode for held in (placed, acked, filled)} == {"A1"}
    assert [held.seqnum for held in (placed, acked, filled)] == [0, 1, 2]
    assert filled.prevuuid == acked.curruuid
    assert placed.qty.as_py() == Decimal("100")
    assert filled.state.as_py() == "80FILLED"
    # What the stated facts imply is a reading, never a column.
    assert placed.is_resting and not filled.is_resting
    assert filled.remaining.as_py() == Decimal("0")

    # The third hop: each statement names the message it was read from.
    chained = list(reader.lifecycle(messages))
    assert filled.srcuuids == [chained[2].curruuid]

    # A fill is one report: the narrow door reads one execution, in the
    # order's chain.
    (fill,) = reader.executions(messages)
    assert fill.crossuuid == placed.crossuuid
    assert fill.identifiers["execid"] == "E1"
    assert fill.completes  # the fill that finished the order

    # The row opens with the sixteen event columns, then the order's own.
    field = Order.field()
    assert field.field_at(0).name == "currunix"
    assert field.field_at(16).name == "px"
    row = filled.into_row()
    assert Order.from_row(field, row).curruuid == filled.curruuid
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const path = require('node:path')
    const { fix, market } = require('yggdryl')

    const registry = fix.FixRegistry.fromHandle(path.resolve('config', 'fix'))
    const reader = new fix.FixCodec(registry)
    const lines = [
      '8=FIX.4.4|35=D|11=A1|55=AAPL|54=1|38=100|44=10.5|15=USD|52=20260102-10:15:30.250|10=0|',
      '8=FIX.4.4|35=8|11=A1|37=O1|17=E0|150=0|39=0|38=100|151=100|14=0|55=AAPL|54=1|52=20260102-10:15:30.500|10=0|',
      '8=FIX.4.4|35=8|37=O1|17=E1|150=F|39=2|38=100|14=100|151=0|31=10.5|32=100|55=AAPL|54=1|52=20260102-10:15:33.100|10=0|',
    ].map((line) => Buffer.from(line))
    const messages = [...reader.parseLines(lines)]

    // Three messages about one order are three statements of it, chained.
    const [placed, acked, filled] = [...reader.orders(messages)]
    assert.ok([placed, acked, filled].every((held) => held.crosscode === 'A1'))
    assert.deepEqual([placed, acked, filled].map((held) => held.seqnum), [0, 1, 2])
    assert.equal(filled.prevuuid, acked.curruuid)
    assert.equal(placed.qty, '100')
    assert.equal(filled.state, '80FILLED')
    // What the stated facts imply is a reading, never a column.
    assert.ok(placed.isResting && !filled.isResting)
    assert.equal(filled.remaining, '0')

    // The third hop: each statement names the message it was read from.
    const chained = [...reader.lifecycle(messages)]
    assert.deepEqual(filled.srcuuids, [chained[2].curruuid])

    // A fill is one report: the narrow door reads one execution, in the
    // order's chain.
    const [fill] = [...reader.executions(messages)]
    assert.equal(fill.crossuuid, placed.crossuuid)
    assert.equal(fill.identifiers.execid, 'E1')
    assert.equal(fill.completes, true) // the fill that finished the order

    // The row opens with the sixteen event columns, then the order's own.
    const field = market.Order.field()
    assert.equal(field.fieldAt(0).name, 'currunix')
    assert.equal(field.fieldAt(16).name, 'px')
    const row = filled.intoRow()
    assert.equal(market.Order.fromRow(field, row).curruuid, filled.curruuid)
    ```

## The three hops join on one column set

A [text line](../media/text/index.md#row-schema)'s batch, a [FIX row](../fix/capture.md#the-crates-own-columns), a [chained row](../fix/lifecycle.md) and every product row open with the same sixteen [event columns](../graph.md#columns) under one name and one datatype each, so the hops join without a mapping: a message's `srcuuids` are the `curruuid` of the lines it was parsed out of, a product's `srcuuids` are the `curruuid` of the messages it was read from, and a walked product's `prevuuid` and `parentuuids` are the `curruuid` of the statements before it in its own table. Over the bridge's own capture, `rust/tests/fix/ulbridge.log`, `rust/tests/market/hops.rs` walks the three hops and pins what they answer.

| Hop | Count |
| --- | --- |
| lines | 144 |
| messages, under the codec's own defaults | 79 |
| chains the lifecycle chains them into | 11 |
| orders | 18 |
| executions | 14 |
| trades | 19 |
| quotes | 0 |
| books, at depth 5 over a one-second grid | 10 |

Each product names only messages the capture holds, and none multiplies the messages or their identities.

## A twin is one statement

A bridge logs one message at every hop it passes, and the [lifecycle](../fix/lifecycle.md#a-twin-is-not-a-successor) already reads the copies as one message under one identity. A product read out of each copy is the same product - the same instant, the same facts - so the doors fold the statements of one instant that share an identity into one by the product's own `merge_with`, which unions their sources and changes nothing else, and the walk then chains the one statement. A product folding that stream counts the event once: a trade's matched quantity is not doubled by a second hop, and an order rests once on a book's level however many hops logged it.

## Edges

- A product is not a message: the row holds what the market did, typed, and the message it was read from is named by `srcuuids` and nothing else; a FIX field that is not a market fact is on the message's row, reached by that join.
- A reading is not a column: `pricing`, `remaining`, `mid`, `microprice` and the rest are what the stated facts imply, computed on every call, so nothing stores a fact twice and a row read back re-derives them.
- Depth is a declaration: `books` refuses a depth of zero and a negative grid step, and a book refuses a ladder deeper than it was opened for rather than growing to whatever arrived.
- No wall clock after intake: every product is dated by the messages it was read from, a written-back message by the product, so a replay of one capture answers the same products.
- A door yields a source's error where the source had it, never advancing the walk; exhaustion is fused.
- A message that states no product for a door is skipped by it, not refused: a keepalive is nobody's order.

## Commands

```bash
cargo test -p yggdryl --test market
cargo test -p yggdryl --test allocations a_product_door
cargo test -p yggdryl --doc market::
cargo bench -p yggdryl --bench market
python/.venv/bin/python -m pytest python/tests/market -q
node --test node/tests/market/market.test.js
```
