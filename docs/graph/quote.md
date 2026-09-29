# Quote

`Quote` is a quote with no instant, `QuoteEvent` one at an instant: an operation standing on one side of a market, and the entry a market-data feed rests on a book.

## Contract

| Key | Rule |
| --- | --- |
| Types | `Quote = OperationElement<QuoteKind>`, `QuoteEvent = OperationEvent<QuoteKind>`: the [operation leaf contract](order.md#contract), filed under `QUOT` |
| Side | a quote rests on the side it states; the two prices a two-sided quote states are its [bid and ask](market.md#bid-and-ask) facts, which name no side and fill no price |
| From FIX | a quote message stating a bid and an offer and no side of its own is split at the parse into two sided quotes: `BUY` with the bid's price, quantity and FX parts, `SELL` with the offer's, each keeping both bid and ask facts and naming the source's identity among its sources, each chained on its side (`BUY:Q1`, `SELL:Q1`); a book reads each sided quote once and never the two-sided source; a one-sided quote stays one message; a quote batch - a mass quote, a bid list (`QUOB`) - is first split into one quote per entry, each then split the same way ([FIX](../fix/message.md#a-parse-splits-what-a-message-reports)) |
| Following | the chain gives its side only to a quote stating `UNKNOWN` ([Operation](operation.md#following-and-merging)) |
| Book entry | a dated quote with a [book control](order.md#book-control) is an entry of a [book](book.md#entries); a change or overlay stating `ORDERID` promotes it to an order |
| Execution | `is_execution()` is always false; only an [execution](execution.md) is one - a quote's report of a fill splits one off |

## Example

A two-sided Apple quote, then one offer as a market-data entry on a book.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, BookRef, Element, Event, Market, MarketData, MdUpdateAction, QuoteEvent};
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;

    // Two prices and no side: the bid and the ask are facts of the quote.
    let mut quote = QuoteEvent::at(T);
    quote.set_crosscode("Q-7".to_owned());
    quote.set_ticker(Some("AAPL".into()));
    quote.set_bidpx(Some("189.48".parse()?));
    quote.set_bidqty(Some(Decimal::from_int(300)));
    quote.set_askpx(Some("189.52".parse()?));
    quote.set_askqty(Some(Decimal::from_int(100)));
    quote.finalize();
    assert_eq!((quote.get_side(), quote.get_price()), (Side::Unknown, None));
    assert_eq!(quote.get_crosscode(), "Q-7");
    assert!(!quote.is_execution());

    // One offer as a market-data entry: a sided quote a book rests.
    let mut entry = QuoteEvent::at(T);
    entry.set_crosscode("MD-1".to_owned());
    entry.set_ticker(Some("AAPL".into()));
    entry.set_side(Side::Sell);
    entry.set_price(Some("189.52".parse()?));
    entry.set_quantity(Some(Decimal::from_int(100)));
    entry.set_book(Some(BookRef {
        action: Some(MdUpdateAction::New),
        scope: Some("AAPL.XNAS".into()),
        position: Some(1),
        ..BookRef::default()
    }));
    entry.finalize();
    assert_eq!((entry.get_crosscode(), entry.scope()), ("SELL:MD-1", "AAPL.XNAS"));

    let mut book = BookEvent::new(T, "AAPL");
    book.add_operations([MarketData::from(entry)])?;
    assert_eq!(book.best_price(Side::Sell), Some("189.52".parse()?));
    assert_eq!(book.get_askqty(), Some(Decimal::from_int(100)));

    // A quote stating no side rests on neither.
    let error = book.add_operations([MarketData::from(quote)]).unwrap_err();
    assert!(error.to_string().contains("$.operation.side"), "{error}");
    assert_eq!(book.alive().count(), 1, "the book is unchanged");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    # Two prices and no side: the bid and the ask are facts of the quote.
    quote = graph.QuoteEvent(
        T,
        crosscode="Q-7",
        ticker="AAPL",
        bidpx=Decimal("189.48"),
        bidqty=300,
        askpx=Decimal("189.52"),
        askqty=100,
    )
    assert (quote.side, quote.price) == (Side.UNKNOWN, None)
    assert quote.askpx is not None and quote.askpx.as_py() == Decimal("189.52")
    assert not quote.is_execution

    # One offer as a market-data entry: a sided quote a book rests.
    entry = graph.QuoteEvent(
        T,
        book=graph.BookRef(action="0", scope="AAPL.XNAS", position=1),
        crosscode="MD-1",
        ticker="AAPL",
        side="SELL",
        price=Decimal("189.52"),
        quantity=100,
    )
    assert (entry.crosscode, entry.scope) == ("SELL:MD-1", "AAPL.XNAS")

    book = graph.BookEvent(T, "AAPL").with_operations([entry])
    best = book.best_price(Side.SELL)
    assert best is not None and best.as_py() == Decimal("189.52")
    assert book.askqty is not None and book.askqty.as_py() == 100

    # A quote stating no side rests on neither.
    try:
        book.with_operations([quote])
    except ValueError as error:
        assert "$.operation.side" in str(error)
    else:
        raise AssertionError("a quote with no side was rested")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n

    // Two prices and no side: the bid and the ask are facts of the quote.
    const quote = new graph.QuoteEvent(T, {
      crosscode: 'Q-7', ticker: 'AAPL', bidpx: '189.48', bidqty: 300, askpx: '189.52', askqty: 100,
    })
    assert.equal(quote.side, 'UNKNOWN')
    assert.equal(quote.price, null)
    assert.equal(quote.askpx, '189.52')
    assert.equal(quote.isExecution, false)

    // One offer as a market-data entry: a sided quote a book rests.
    const entry = new graph.QuoteEvent(T, {
      book: new graph.BookRef({ action: '0', scope: 'AAPL.XNAS', position: 1 }),
      crosscode: 'MD-1',
      ticker: 'AAPL',
      side: 'SELL',
      price: '189.52',
      quantity: 100,
    })
    assert.equal(entry.crosscode, 'SELL:MD-1')
    assert.equal(entry.scope, 'AAPL.XNAS')

    const book = new graph.BookEvent(T, 'AAPL').withOperations([entry])
    assert.equal(book.bestPrice('SELL'), '189.52')
    assert.equal(book.askqty, '100')

    // A quote stating no side rests on neither.
    assert.throws(() => book.withOperations([quote]), /\$\.operation\.side/)
    ```
