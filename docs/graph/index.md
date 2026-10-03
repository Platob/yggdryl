# Graph

`yggdryl::graph` is market data as elements that name each other by identity: four traits say what an element answers, typed leaves answer them, and `MarketData` carries any leaf across a boundary.

## Traits

Signatures with no storage: a FIX message, a text line or a book entry can each be an element the graph does not own.

| Trait | Page | Answers |
| --- | --- | --- |
| `Element` | [Element](element.md) | identity, cross element/code, digest, sources; order, finalization, following, merging |
| `Event: Element` | [Event](event.md) | instant, state, place at its instant, clocks (creation, recording, expiration, predecessor, snapshot), UUIDv7 identity; lifecycle walk `EventIterator` |
| `Market` | [Market](market.md) | thirty-four facts, each setter [filling or overwriting](market.md#setting-fill-or-overwrite): the `marketdatatype`, price and stop price, quantity and its shown and hidden parts, currency/unit, side, the security's identifiers (`securityids`) and the ISIN, classification/market, the last execution clock, trade and FX numbers, bid and ask, FX rates, ticker, metadata; `marketdatakind` - the category a lifecycle chains within - `is_sided` (true for an order or an execution, whose stored cross code `{kind}:{side}:{base}` states its side; a quote holds its bid and its ask and tags a side) and the [book key](market.md#the-book-key) - the ISIN, else the ticker, else `XX0000000000` |
| `Operation: Market` | [Operation](operation.md) | five more: the ordered quantity (`ordqty`), time in force, tradability, its own identifiers (`identifiers`), the parties it names (`partyids`) |

- **Names.** Accessors `get_`, mutators `set_`, never bare; the three identifier maps and FX rates use fallible or filling `insert_`/`remove_`/`derive_` verbs instead: a view holder may refuse, a plain holder always answers `Ok` ([detail](market.md#security-identifiers)).
- **Identifiers.** `securityids`, `identifiers` and `partyids` are each one [`Identifiers`](identifier.md) map - a value per key `src:type`, the base source's key spelled as its type alone and holding the type's answer, which a named source fills - with the [parents](identifier.md#parentage) a chain gives a type, read, written, merged and digested alike; an [`IsinRegistry`](isin-registry.md) learns what elements state about their instruments and fills what later ones leave unsaid.
- **Links.** Elements name a predecessor, source or cross element by identity, never reference; a caller resolves it via whatever holds the graph.
- **Objects.** Object-safe except `is_after`, `is_before`, `with_previous`, `merge_with`, `following`, `restating`, `merging`, `fold_lifecycle`, the fills, and the market/operation digests and merges: `dyn Event`/`dyn Operation` walks read every fact through one reference.

## Leaves

| Leaf | Page | Rust types | Traits |
| --- | --- | --- | --- |
| Order | [Order](order.md) | `Order`, `OrderEvent` - `OperationElement<OrderKind>`, `OperationEvent<OrderKind>` - plus book control `BookRef`, `MdUpdateAction` | `Element`, `Market`, `Operation`; event also `Event` |
| Quote | [Quote](quote.md) | `Quote`, `QuoteEvent` | the same |
| Execution | [Execution](execution.md) | `Execution`, `ExecutionEvent` | the same |
| Trade | [Trade](trade.md) | `TradeEvent` | all four |
| Book | [Book](book.md) | `BookEvent` - complete, or its deltas alone - `SnapshotEvent`, `BookIterator`, `yggdryl::Limit` | book/snapshot: `Element`, `Event`, `Market` |
| Market data | [Market data](market-data.md) | `MarketData`, `MarketKind`, `ElementColumn`, `EventColumn`, `MarketColumn`, `OperationColumn`, `MarketView` | `Element`, `Market`, through the leaf held |
| Row schemas | [Row schemas](schemas.md) | the text line, the FIX row and the `marketdata` row, column by column, over the one element, event, market and operation prefix | the same listing through `enums` and `MarketData.field()` |

Every leaf is filed under one [`MarketDataKind`](../types/enum/marketdatakind.md) - an order `ORDR`, a quote `QUOT`, an execution `EXEC`, a trade `TRAD`, a book or a snapshot control `BOOK` - the first market column of its [row](market-data.md#arrow).

Two readings stand over the books, neither a leaf:

| Reading | Page | Rust types | Bindings |
| --- | --- | --- | --- |
| Candle | [Candle](candle.md) | `Candle`, `Ohlc`, `CandleOptions`, `CandleIterator` - one OHLC of the best bid, the best ask, the mid and the spread per stored book cross code (`3:0:ACME`) and bucket, with the touch and the count of books, the buckets aligned to a zone's wall clock | `graph.Candle`, `graph.CandleOptions`, `graph.CandleIterator` and `graph.candles` in both, a candle built by the walk or read back, never by hand |
| Book display | [Book display](serve.md) | `BookService`, `BookServiceOptions`, `BookTable`, `BookQuery` (the `http` feature) - a `marketdata` table's book keys, candles, books and audits over HTTP, and the Node.js display in front of them | the service is Rust-only; the command `yggdryl market serve` ships in the wheel, and the npm package's `book.js` spawns it |

## Bindings

Rust-only traits; the leaves plus `MarketData`, `BookRef`, `BookIterator`, `EventIterator` and `MarketDataRowIterator` are one class each in Python's `yggdryl.graph`/JS's `graph`:

- built from named facts by column name (`...`/`undefined` skips one, `None`/`null` clears it), checked by its field, finalized on construction; a derived identity (`curruuid`, `crossuuid`, `currhashcode`, `crosshashcode`) refused by name;
- immutable: every verb - `with_previous`, `merge_with`, `restating`, `with_book`, `with_operations` - answers a new value;
- Python reads a decimal, code or identity as [`Scalar`](../types/scalar.md) (`.as_py()`), `isincode` as `str`, `securityids`, `identifiers` and `partyids` as an [`Identifiers`](identifier.md), `fxrates` as a `dict` of decimal `Scalar`s, and an enum fact - `state`, `side`, `marketdatakind` - as its `IntEnum` member (`yggdryl.State`, `Side`, `MarketDataKind`); JavaScript reads decimals and codes as text, the three identifier maps as an `Identifiers`, `fxrates` as an object of decimal text, an enum fact as its member's name (`'BUYS'`, `'ORDR'`), an instant as `bigint`;
- a side is never absent: `Side.UNKN`/`'UNKN'` where none is stated;
- `insert_`/`remove_`/`derive_`, `insert_fxrate`, `is_sided`, `stored_crosscode` and `book_crosscode` are Rust-only: a binding states identifiers - a list of `Identifier` or an `Identifiers` - and rates when building a leaf, and reads the stored `crosscode`.

A FIX message implements all four traits, and the [text line](../media/text.md) it is read from is an `Event`. [`FixMsg::market_data`](../fix/message.md#market-data) reads one message as its one leaf - the fills, the sides of a trade and the entries of a batch were split into messages of their own at the parse, and a quote stays one quote holding both legs - and [`FixCodec::market_data`, `market_arrow_reader` and `book_arrow_reader`](../fix/arrow.md#fix-market-books) read a capture into sorted market data, its rows and its books: Python `FixCodec.market_data(messages)`, `market_arrow_reader(messages)`, `book_arrow_reader(messages, snapshot_millis=0, filter=None)`; JavaScript `codec.marketData(messages)`, `marketArrowReader(messages)`, `bookArrowReader(messages, snapshotMillis, filter)`. A leaf's `metadata` leaves out what its `identifiers` and `partyids` hold ([what a leaf's metadata holds](../fix/message.md#what-a-leafs-metadata-holds)).

## Example

An Apple buy order - 100 shares at 189.50 USD - built, finalized, written as one `marketdata` row and read back.

=== "Rust"

    ```rust
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{Element, Event, Market, MarketData, OrderEvent};
    use yggdryl::{Ccy, Decimal, IdKey, IdType, Identifier, MarketDataKind, Side};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.set_price(Some("189.50".parse()?), true);
    order.set_quantity(Some(Decimal::from_int(100)), true);
    order.set_currency(Ccy::new("USD")?, true);
    order.set_ticker(Some("AAPL".into()), true);
    order.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "US0378331005")?)?;
    order.finalize();

    // Finalizing derived the identity and what the facts imply: the CUSIP
    // the ISIN carries, and the cross code stored under the side.
    assert_eq!(order.get_curruuid(), order.time_uuid()?);
    assert_eq!(order.get_crosscode(), "10:1:O-1001");
    assert_eq!(order.get_isincode(), Some("US0378331005"));
    assert_eq!(order.get_securityids().to_string(), "[cusip=037833100, derived:cusip=037833100, isin=US0378331005]");

    // One row out, one value back: the same order, filed under ORDR.
    let value = MarketData::from(order);
    assert_eq!(value.marketdatakind(), MarketDataKind::Order);
    let batches = MarketData::arrow_reader([value.clone()], None, None)?
        .collect::<Result<Vec<_>, _>>()?;
    let read: Vec<MarketData> = MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(read, [value]);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Identifier, MarketDataKind, Side, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        side="BUYS",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        ticker="AAPL",
        securityids=[Identifier("isin", "US0378331005")],
    )

    # Built finalized: the CUSIP the ISIN carries, the cross code under the side.
    assert order.side is Side.BUYS and order.crosscode == "10:1:O-1001"
    assert order.isincode == "US0378331005"
    assert str(order.securityids) == "[cusip=037833100, derived:cusip=037833100, isin=US0378331005]"
    assert order.price is not None and order.price.as_py() == Decimal("189.50")

    # One row out, one value back: the same order, filed under ORDR.
    [read] = graph.MarketData.from_arrow_reader(graph.MarketData.arrow_reader([order]))
    assert read.marketdatakind is MarketDataKind.ORDR
    assert read.into_leaf() == order
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      side: 'BUYS',
      price: '189.50',
      quantity: 100,
      currency: 'USD',
      ticker: 'AAPL',
      securityids: [new Identifier('isin', 'US0378331005')],
    })

    // Built finalized: the CUSIP the ISIN carries, the cross code under the side.
    assert.equal(order.side, 'BUYS')
    assert.equal(order.crosscode, '10:1:O-1001')
    assert.equal(order.isincode, 'US0378331005')
    assert.equal(order.securityids.toString(), '[cusip=037833100, derived:cusip=037833100, isin=US0378331005]')
    assert.equal(order.price, '189.5')

    // One row out, one value back: the same order, filed under ORDR.
    const [read] = graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([order]))
    assert.equal(read.marketdatakind, 'ORDR')
    assert.ok(read.intoLeaf().equals(order))
    ```
