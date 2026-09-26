# Graph

`yggdryl::graph` is market data as elements that name each other by identity: four traits say what an element answers, typed leaves answer them, and `MarketData` carries any leaf across a boundary.

## Traits

Signatures with no storage: a FIX message, a text line or a book entry can each be an element the graph does not own.

| Trait | Page | Answers |
| --- | --- | --- |
| `Element` | [Element](element.md) | identity, cross element/code, digest, sources; order, finalization, following, merging |
| `Event: Element` | [Event](event.md) | instant, state, chain place, clocks, UUIDv7 identity; lifecycle walk `EventIterator` |
| `Market` | [Market](market.md) | nineteen facts: price, quantity, currency/unit, side, security ids, classification/market, ticker, trade and FX numbers, metadata |
| `Operation: Market` | [Operation](operation.md) | eight more: category, time in force, tradability, account/user/alt id maps, bid/ask lanes |

- **Names.** Accessors `get_`, mutators `set_`, never bare; security identifiers and identifier maps use fallible `insert_`/`remove_`/`derive_` instead: a view holder may refuse, a plain holder always answers `Ok` ([detail](market.md#security-identifiers)).
- **Links.** Elements name a predecessor, source or cross element by identity, never reference; a caller resolves it via whatever holds the graph.
- **Objects.** Object-safe except `is_after`, `is_before`, `with_previous`, `merge_with`, `following`, `restating`, `merging`, `fold_lifecycle`, the fills, and the market/operation digests and merges: `dyn Event`/`dyn Operation` walks read every fact through one reference.

## Leaves

| Leaf | Page | Rust types | Traits |
| --- | --- | --- | --- |
| Order | [Order](order.md) | `Order`, `OrderEvent` - `OperationElement<OrderKind>`, `OperationEvent<OrderKind>` - plus book control `BookRef`, `MdUpdateAction` | `Element`, `Market`, `Operation`; event also `Event` |
| Quote | [Quote](quote.md) | `Quote`, `QuoteEvent` | the same |
| Execution | [Execution](execution.md) | `Execution`, `ExecutionEvent` | the same |
| Trade | [Trade](trade.md) | `TradeEvent` | all four |
| Book | [Book](book.md) | `BookSide`, `BookEvent`, `SnapshotEvent`, `SnapshotPartition`, `BookIterator`, `yggdryl::Limit` | side: `Element`, `Market`; book/snapshot: `Event`, `Market` too |
| Market data | [Market data](market-data.md) | `MarketData`, `MarketKind`, `EventColumn`, `MarketColumn`, `OperationColumn`, `MarketView` | `Element`, `Market`, through the leaf held |

The [replay](replay.md) serves these leaves to a browser, book by book.

## Bindings

Rust-only traits; the leaves plus `MarketData`, `Lane`, `BookRef`, `SnapshotPartition`, `BookIterator`, `EventIterator` are one class each in Python's `yggdryl.graph`/JS's `graph`:

- built from named facts by column name (`...`/`undefined` skips one, `None`/`null` clears it), checked by its field, finalized on construction; a derived identity (`curruuid`, `crossuuid`, `currhashcode`, `crosshashcode`) refused by name;
- immutable: every verb - `with_previous`, `merge_with`, `restating`, `with_book`, `with_operation`, `with_operations` - answers a new value;
- Python reads a decimal, code or identity as [`Scalar`](../types/scalar.md) (`.as_py()`); JavaScript as text, an instant as `bigint`;
- `insert_`/`remove_`/`derive_` are Rust-only: a binding states identifiers when building a leaf.

Python `FixCodec.book_arrow_reader`/JS `bookArrowReader`: the FIX-to-book pipeline into [`MarketData.field()`](market-data.md#arrow) rows. `market_operations`/`marketOperations`, `market_arrow_reader`/`marketArrowReader`: the [sorted market door](../fix/arrow.md#fix-market-books). A FIX message implements all four traits, and the [text line](../media/index.md#plain-text) it is read from is an `Event`.

## Example

An Apple buy order - 100 shares at 189.50 USD - built, finalized, written as one `marketdata` row and read back.

=== "Rust"

    ```rust
    use yggdryl::arrow::batch_reader;
    use yggdryl::graph::{Element, Event, Market, MarketData, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal, SecType, SecurityId, Side};

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
    // the ISIN carries, and the bid lane a buy at a price quotes.
    assert_eq!(order.get_curruuid(), order.time_uuid()?);
    assert_eq!(order.get_securityids().get("CUSIP"), Some("037833100"));
    assert_eq!(order.get_bid().and_then(|lane| lane.price), Some("189.5".parse()?));

    // One row out, one value back: the same order.
    let batches = MarketData::arrow_reader([MarketData::from(order.clone())], None, None)?
        .collect::<Result<Vec<_>, _>>()?;
    let read: Vec<MarketData> = MarketData::from_arrow_reader(batch_reader(batches[0].schema(), batches))?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(read, [MarketData::from(order)]);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

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

    # Built finalized: the CUSIP the ISIN carries, the bid lane a buy quotes.
    assert order.securityids == {"CUSIP": "037833100", "ISIN": "US0378331005"}
    assert order.bid is not None and order.bid.price == order.price
    assert order.price is not None and order.price.as_py() == Decimal("189.50")

    # One row out, one value back: the same order.
    [read] = graph.MarketData.from_arrow_reader(graph.MarketData.arrow_reader([order]))
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

    // Built finalized: the CUSIP the ISIN carries, the bid lane a buy quotes.
    assert.deepEqual(order.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
    assert.equal(order.bid.price, '189.5')
    assert.equal(order.price, '189.5')

    // One row out, one value back: the same order.
    const [read] = graph.MarketData.fromArrowReader(graph.MarketData.arrowReader([order]))
    assert.ok(read.intoLeaf().equals(order))
    ```
