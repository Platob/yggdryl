# Quote

`Quote` is a quote with no instant, `QuoteEvent` one at an instant: an operation priced by its [lanes](operation.md#lanes), and the entry a market-data feed rests on a book side.

## Contract

| Key | Rule |
| --- | --- |
| Types | `Quote = OperationElement<QuoteKind>`, `QuoteEvent = OperationEvent<QuoteKind>`: the [operation leaf contract](order.md#contract), digested as `quote` |
| Side | a quote with one lane and no side, price, last trade or average of its own takes that lane's side (`BUY` bid, `SELL` ask) and, where it states none, its price, quantity, currency, unit and FX parts; two lanes or none give no side ([`fill_operation`](operation.md#contract)) |
| Following | the chain gives a side only to a quote with no lane of its own; lanes never follow ([Operation](operation.md#following-and-merging)) |
| Book entry | a dated quote with a [book control](order.md#book-control) is an entry of a [book side](book.md#book-sides); a change or overlay stating `ORDERID` promotes it to an order |
| Execution | `is_execution()` is always false; only an [execution](execution.md) is one |

## Example

A two-sided Apple quote, then one offer as a market-data entry on a book's ask side.

=== "Rust"

    ```rust
    use yggdryl::graph::{
        BookRef, BookSide, Element, Event, Lane, Market, MarketData, MdUpdateAction, Operation, QuoteEvent,
    };
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let lane = |price: &str, quantity: i64| -> yggdryl::Result<Lane> {
        Ok(Lane { price: Some(price.parse()?), quantity: Some(Decimal::from_int(quantity)), ..Lane::default() })
    };

    // Two lanes are its prices, and they name no side.
    let mut quote = QuoteEvent::at(T);
    quote.set_crosscode("Q-7".to_owned());
    quote.set_ticker(Some("AAPL".into()));
    quote.set_bid(Some(lane("189.48", 300)?));
    quote.set_ask(Some(lane("189.52", 100)?));
    quote.finalize();
    assert_eq!((quote.get_side(), quote.get_price()), (Side::Unknown, None));
    assert_eq!(quote.get_ask().and_then(|ask| ask.price), Some("189.52".parse()?));
    assert!(!quote.is_execution());

    // One offer as a market-data entry: the ask lane names the side.
    let mut entry = QuoteEvent::at(T);
    entry.set_crosscode("MD-1".to_owned());
    entry.set_ticker(Some("AAPL".into()));
    entry.set_ask(Some(lane("189.52", 100)?));
    entry.set_book(Some(BookRef {
        action: Some(MdUpdateAction::New),
        scope: Some("AAPL.XNAS".into()),
        position: Some(1),
        ..BookRef::default()
    }));
    entry.finalize();
    assert_eq!((entry.get_side(), entry.scope()), (Side::Sell, "AAPL.XNAS"));

    let mut asks = BookSide::new(Side::Sell)?;
    asks.add_operation(MarketData::from(entry))?;
    assert_eq!(asks.best_price(), Some("189.52".parse()?));
    assert_eq!(asks.best_quantity(), Some(Decimal::from_int(100)));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000

    # Two lanes are its prices, and they name no side.
    quote = graph.QuoteEvent(
        T,
        crosscode="Q-7",
        ticker="AAPL",
        bid=graph.Lane(price=Decimal("189.48"), quantity=300),
        ask=graph.Lane(price=Decimal("189.52"), quantity=100),
    )
    assert (quote.side.as_py(), quote.price) == ("UNKNOWN", None)
    assert quote.ask is not None and quote.ask.price is not None
    assert quote.ask.price.as_py() == Decimal("189.52")
    assert not quote.is_execution

    # One offer as a market-data entry: the ask lane names the side.
    entry = graph.QuoteEvent(
        T,
        book=graph.BookRef(action="0", scope="AAPL.XNAS", position=1),
        crosscode="MD-1",
        ticker="AAPL",
        ask=graph.Lane(price=Decimal("189.52"), quantity=100),
    )
    assert (entry.side.as_py(), entry.scope) == ("SELL", "AAPL.XNAS")

    asks = graph.BookSide("SELL").with_operation(entry)
    assert asks.best_price is not None and asks.best_price.as_py() == Decimal("189.52")
    assert asks.best_quantity is not None and asks.best_quantity.as_py() == 100
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n

    // Two lanes are its prices, and they name no side.
    const quote = new graph.QuoteEvent(T, {
      crosscode: 'Q-7',
      ticker: 'AAPL',
      bid: new graph.Lane({ price: '189.48', quantity: 300 }),
      ask: new graph.Lane({ price: '189.52', quantity: 100 }),
    })
    assert.equal(quote.side, 'UNKNOWN')
    assert.equal(quote.price, null)
    assert.equal(quote.ask.price, '189.52')
    assert.equal(quote.isExecution, false)

    // One offer as a market-data entry: the ask lane names the side.
    const entry = new graph.QuoteEvent(T, {
      book: new graph.BookRef({ action: '0', scope: 'AAPL.XNAS', position: 1 }),
      crosscode: 'MD-1',
      ticker: 'AAPL',
      ask: new graph.Lane({ price: '189.52', quantity: 100 }),
    })
    assert.equal(entry.side, 'SELL')
    assert.equal(entry.scope, 'AAPL.XNAS')

    const asks = new graph.BookSide('SELL').withOperation(entry)
    assert.equal(asks.bestPrice, '189.52')
    assert.equal(asks.bestQuantity, '100')
    ```
