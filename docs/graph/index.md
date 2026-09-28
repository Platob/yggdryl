# Graph

`yggdryl::graph` is market data as elements that name each other by identity: four traits say what an element answers, typed leaves answer them, and `MarketData` carries any leaf across a boundary.

## Traits

Signatures with no storage: a FIX message, a text line or a book entry can each be an element the graph does not own.

| Trait | Page | Answers |
| --- | --- | --- |
| `Element` | [Element](element.md) | identity, cross element/code, digest, sources; order, finalization, following, merging |
| `Event: Element` | [Event](event.md) | instant, state, place at its instant, clocks, UUIDv7 identity; lifecycle walk `EventIterator` |
| `Market` | [Market](market.md) | twenty-seven facts: price, quantity, currency/unit, side, security ids and the ISIN, classification/market, trade and FX numbers, bid and ask, FX rates, ticker, metadata; the side-prefixed cross code and the book key |
| `Operation: Market` | [Operation](operation.md) | three more: time in force, tradability, alternate identifiers |

- **Names.** Accessors `get_`, mutators `set_`, never bare; security identifiers, alternate identifiers and FX rates use fallible or filling `insert_`/`remove_`/`derive_` verbs instead: a view holder may refuse, a plain holder always answers `Ok` ([detail](market.md#security-identifiers)).
- **Links.** Elements name a predecessor, source or cross element by identity, never reference; a caller resolves it via whatever holds the graph.
- **Objects.** Object-safe except `is_after`, `is_before`, `with_previous`, `merge_with`, `following`, `restating`, `merging`, `fold_lifecycle`, the fills, and the market/operation digests and merges: `dyn Event`/`dyn Operation` walks read every fact through one reference.

## Leaves

| Leaf | Page | Rust types | Traits |
| --- | --- | --- | --- |
| Order | [Order](order.md) | `Order`, `OrderEvent` - `OperationElement<OrderKind>`, `OperationEvent<OrderKind>` - plus book control `BookRef`, `MdUpdateAction` | `Element`, `Market`, `Operation`; event also `Event` |
| Quote | [Quote](quote.md) | `Quote`, `QuoteEvent` | the same |
| Execution | [Execution](execution.md) | `Execution`, `ExecutionEvent` | the same |
| Trade | [Trade](trade.md) | `TradeEvent` | all four |
| Book | [Book](book.md) | `BookEvent`, `SnapshotEvent`, `BookIterator`, `yggdryl::Limit` | book/snapshot: `Element`, `Event`, `Market` |
| Market data | [Market data](market-data.md) | `MarketData`, `MarketKind`, `EventColumn`, `MarketColumn`, `OperationColumn`, `MarketView` | `Element`, `Market`, through the leaf held |

Every leaf is filed under one [`MarketDataKind`](../types/enum/marketdatakind.md) - an order `ORDR`, a quote `QUOT`, an execution `EXEC`, a trade `TRAD`, a book or a snapshot control `BOOK` - the first column of its [row](market-data.md#arrow).

## Bindings

Rust-only traits; the leaves plus `MarketData`, `BookRef`, `BookIterator`, `EventIterator` and `MarketDataRowIterator` are one class each in Python's `yggdryl.graph`/JS's `graph`:

- built from named facts by column name (`...`/`undefined` skips one, `None`/`null` clears it), checked by its field, finalized on construction; a derived identity (`curruuid`, `crossuuid`, `currhashcode`, `crosshashcode`) refused by name;
- immutable: every verb - `with_previous`, `merge_with`, `restating`, `with_book`, `with_operations` - answers a new value;
- Python reads a decimal, code or identity as [`Scalar`](../types/scalar.md) (`.as_py()`), `isincode` as `str`, `fxrates` as a `dict` of decimal `Scalar`s, and an enum fact - `state`, `side`, `marketdatakind` - as its `IntEnum` member (`yggdryl.State`, `Side`, `MarketDataKind`); JavaScript reads decimals and codes as text, `fxrates` as an object of decimal text, an enum fact as its member's name (`'BUY'`, `'ORDR'`), an instant as `bigint`;
- a side is never absent: `Side.UNKNOWN`/`'UNKNOWN'` where none is stated;
- `insert_`/`remove_`/`derive_`, `insert_fxrate`, `sided_crosscode` and `book_crosscode` are Rust-only: a binding states identifiers and rates when building a leaf, and reads the stored `crosscode`.

A FIX message implements all four traits, and the [text line](../media/index.md#plain-text) it is read from is an `Event`. [`FixMsg::market_data`](../fix/message.md#market-data) reads one message as its one leaf - the fills and the sides of a two-sided quote were split into messages of their own at the parse - and [`FixCodec::market_data`, `market_arrow_reader` and `book_arrow_reader`](../fix/arrow.md#fix-market-books) read a capture into sorted market data, its rows and its books: Python `FixCodec.market_data(messages)`, `market_arrow_reader(messages)`, `book_arrow_reader(messages, snapshot_millis=0)`; JavaScript `codec.marketData(messages)`, `marketArrowReader(messages)`, `bookArrowReader(messages, snapshotMillis)`.

## Example

An Apple buy order - 100 shares at 189.50 USD - built, finalized, written as one `marketdata` row and read back.

=== "Rust"

    ```rust
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{Element, Event, Market, MarketData, OrderEvent};
    use yggdryl::{Ccy, Decimal, MarketDataKind, SecType, SecurityId, Side};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy);
    order.set_price(Some("189.50".parse()?));
    order.set_quantity(Some(Decimal::from_int(100)));
    order.set_currency(Ccy::new("USD")?);
    order.set_ticker(Some("AAPL".into()));
    order.insert_securityid(SecurityId::new(SecType::read("ISIN")?, "US0378331005")?)?;
    order.finalize();

    // Finalizing derived the identity and what the facts imply: the CUSIP
    // the ISIN carries, and the cross code stored under the side.
    assert_eq!(order.get_curruuid(), order.time_uuid()?);
    assert_eq!(order.get_crosscode(), "BUY:O-1001");
    assert_eq!(order.get_isincode(), Some("US0378331005"));
    assert_eq!(order.get_securityids().get("CUSIP"), Some("037833100"));

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

    from yggdryl import MarketDataKind, Side, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        side="BUY",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        ticker="AAPL",
        securityids={"ISIN": "US0378331005"},
    )

    # Built finalized: the CUSIP the ISIN carries, the cross code under the side.
    assert order.side is Side.BUY and order.crosscode == "BUY:O-1001"
    assert order.isincode == "US0378331005"
    assert order.securityids == {"CUSIP": "037833100", "ISIN": "US0378331005"}
    assert order.price is not None and order.price.as_py() == Decimal("189.50")

    # One row out, one value back: the same order, filed under ORDR.
    [read] = graph.MarketData.from_arrow_reader(graph.MarketData.arrow_reader([order]))
    assert read.marketdatakind is MarketDataKind.ORDR
    assert read.into_leaf() == order
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1_700_000_000_000_000_000n, {
      crosscode: 'O-1001',
      side: 'BUY',
      price: '189.50',
      quantity: 100,
      currency: 'USD',
      ticker: 'AAPL',
      securityids: { ISIN: 'US0378331005' },
    })

    // Built finalized: the CUSIP the ISIN carries, the cross code under the side.
    assert.equal(order.side, 'BUY')
    assert.equal(order.crosscode, 'BUY:O-1001')
    assert.equal(order.isincode, 'US0378331005')
    assert.deepEqual(order.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
    assert.equal(order.price, '189.5')

    // One row out, one value back: the same order, filed under ORDR.
    const [read] = graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([order]))
    assert.equal(read.marketdatakind, 'ORDR')
    assert.ok(read.intoLeaf().equals(order))
    ```
