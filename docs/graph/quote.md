# Quote

`Quote` is a quote with no instant, `QuoteEvent` one at an instant: one element holding a bid and an ask, and the entry a market-data feed rests on a book.

## Contract

| Key | Rule |
| --- | --- |
| Types | `Quote = OperationElement<QuoteKind>`, `QuoteEvent = OperationEvent<QuoteKind>`: the [operation leaf contract](order.md#contract), filed under `QUOT` |
| Two legs | a quote holds its bid - `bidpx`, `bidqty`, `bidccy` - and its ask - `askpx`, `askqty`, `askccy` - as one element; its `side` is a tag, a one-sided quote tagging the leg it states and a two-sided one tagging none `BOTH`, once finalized - a tag it states stands. A price and a quantity stated under a tag quote that leg ([A quote's two legs](market.md#a-quotes-two-legs)) |
| Cross code | a quote is not [sided](market.md#sides-and-cross-codes): its cross code is stored under side `0` whatever leg it tags (`14:0:Q-7`), so one quote identifier is one chain, a statement updating either leg included |
| Following | a follower tagging no side takes each leg it states nothing of from its chain, whole; a leg it states is its own, a zero quantity withdrawing it included ([Market](market.md#a-quotes-two-legs)) |
| On a book | a dated quote rests on each leg it states a price or a quantity of - one entry, on both sides where it states both, listed once by `alive()` and on each side by `alive_on` - and a leg sized zero rests nowhere; a quote stating no leg rests on neither and is still a delta of its book ([Book](book.md#entries)); a change or overlay stating `orderid` promotes it to an order |
| From FIX | a quote message is one quote: `BidPx(132)`/`BidSize(134)` its bid and `OfferPx(133)`/`OfferSize(135)` its ask, each currency from a stated `BidCurrency`/`AskCurrency` (or `OfferCurrency`) field, else the message's, never split by side; a quote batch - a mass quote, a bid list (`QUOB`) - is split at the parse into one two-sided quote per entry; a `QuoteCancel` (`Z`) reads `PENDING_CANCEL` ([FIX](../fix/message.md#a-parse-splits-what-a-message-reports)) |
| Execution | `is_execution()` is always false; only an [execution](execution.md) is one - a quote's report of a fill splits one off |

## Example

A two-sided Apple quote resting on both sides of a book, then a one-sided offer.

=== "Rust"

    ```rust
    use yggdryl_market::graph::{BookEvent, BookRef, Market, MarketData, MdUpdateAction, QuoteEvent};
    use yggdryl::graph::{Element, Event};
    use yggdryl::Decimal;
    use yggdryl_market::Side;
    yggdryl_market::install()?;

    const T: i64 = 1_700_000_000_000_000_000;

    // Two legs and no tag: the bid and the ask are the quote's own facts,
    // and it holds both sides.
    let mut quote = QuoteEvent::at(T);
    quote.set_crosscode("Q-7".to_owned());
    quote.set_ticker(Some("AAPL".into()), true);
    quote.set_instcode(Some("AAPL".into()), true);
    quote.set_bidpx(Some("189.48".parse()?), true);
    quote.set_bidqty(Some(Decimal::from_int(300)), true);
    quote.set_askpx(Some("189.52".parse()?), true);
    quote.set_askqty(Some(Decimal::from_int(100)), true);
    quote.finalize();
    assert_eq!((quote.get_side(), quote.get_price()), (Side::Both, None));
    assert_eq!(quote.get_crosscode(), "14:0:Q-7", "a quote, stored under side 0");
    assert!(!quote.is_execution());

    // One offer as a market-data entry: tagged `SELL`, its price and
    // quantity are its ask.
    let mut entry = QuoteEvent::at(T);
    entry.set_crosscode("MD-1".to_owned());
    entry.set_ticker(Some("AAPL".into()), true);
    entry.set_instcode(Some("AAPL".into()), true);
    entry.set_side(Side::Sell, true);
    entry.set_price(Some("189.53".parse()?), true);
    entry.set_quantity(Some(Decimal::from_int(50)), true);
    entry.set_book(Some(BookRef {
        action: Some(MdUpdateAction::New),
        scope: Some("AAPL.XNAS".into()),
        position: Some(1),
        ..BookRef::default()
    }));
    entry.finalize();
    assert_eq!((entry.get_crosscode(), entry.scope()), ("14:0:MD-1", "AAPL.XNAS"));
    assert_eq!((entry.get_askpx(), entry.get_bidpx()), (Some("189.53".parse()?), None));

    // The two-sided quote rests on both sides, one entry; the offer on the ask.
    let mut book = BookEvent::keyed(T, "AAPL");
    book.add_operations([MarketData::from(quote), MarketData::from(entry)])?;
    assert_eq!(book.alive().count(), 2, "each entry once");
    assert_eq!((book.alive_on(Side::Buy).len(), book.alive_on(Side::Sell).len()), (1, 2));
    assert_eq!(book.best_price(Side::Buy), Some("189.48".parse()?));
    assert_eq!(book.best_price(Side::Sell), Some("189.52".parse()?));
    assert_eq!(book.get_askqty(), Some(Decimal::from_int(100)));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    # Two legs and no tag: the bid and the ask are the quote's own facts,
    # and it holds both sides.
    quote = graph.QuoteEvent(
        T,
        crosscode="Q-7",
        ticker="AAPL", instcode="AAPL",
        bidpx=Decimal("189.48"),
        bidqty=300,
        askpx=Decimal("189.52"),
        askqty=100,
    )
    assert (quote.side, quote.price) == (Side.BOTH, None)
    assert quote.crosscode == "14:0:Q-7", "a quote, stored under side 0"
    assert not quote.is_execution

    # One offer as a market-data entry: tagged SELL, its price and quantity are its ask.
    entry = graph.QuoteEvent(
        T,
        book=graph.BookRef(action="0", scope="AAPL.XNAS", position=1),
        crosscode="MD-1",
        ticker="AAPL", instcode="AAPL",
        side="SELL",
        price=Decimal("189.53"),
        quantity=50,
    )
    assert (entry.crosscode, entry.scope) == ("14:0:MD-1", "AAPL.XNAS")
    assert entry.askpx is not None and entry.askpx.as_py() == Decimal("189.53") and entry.bidpx is None

    # The two-sided quote rests on both sides, one entry; the offer on the ask.
    book = graph.BookEvent(T, "AAPL").with_operations([quote, entry])
    assert len(book.alive) == 2, "each entry once"
    assert (len(book.alive_on(Side.BUYS)), len(book.alive_on(Side.SELL))) == (1, 2)
    bid, ask = book.best_price(Side.BUYS), book.best_price(Side.SELL)
    assert bid is not None and bid.as_py() == Decimal("189.48")
    assert ask is not None and ask.as_py() == Decimal("189.52")
    assert book.askqty is not None and book.askqty.as_py() == 100
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n

    // Two legs and no tag: the bid and the ask are the quote's own facts,
    // and it holds both sides.
    const quote = new graph.QuoteEvent(T, {
      crosscode: 'Q-7', ticker: 'AAPL', instcode: 'AAPL', bidpx: '189.48', bidqty: 300, askpx: '189.52', askqty: 100,
    })
    assert.equal(quote.side, 'BOTH')
    assert.equal(quote.price, null)
    assert.equal(quote.crosscode, '14:0:Q-7', 'a quote, stored under side 0')
    assert.equal(quote.isExecution, false)

    // One offer as a market-data entry: tagged SELL, its price and quantity are its ask.
    const entry = new graph.QuoteEvent(T, {
      book: new graph.BookRef({ action: '0', scope: 'AAPL.XNAS', position: 1 }),
      crosscode: 'MD-1',
      ticker: 'AAPL', instcode: 'AAPL',
      side: 'SELL',
      price: '189.53',
      quantity: 50,
    })
    assert.equal(entry.crosscode, '14:0:MD-1')
    assert.equal(entry.scope, 'AAPL.XNAS')
    assert.deepEqual([entry.askpx, entry.bidpx], ['189.53', null])

    // The two-sided quote rests on both sides, one entry; the offer on the ask.
    const book = new graph.BookEvent(T, 'AAPL').withOperations([quote, entry])
    assert.equal(book.alive().length, 2, 'each entry once')
    assert.deepEqual([book.aliveOn('BUYS').length, book.aliveOn('SELL').length], [1, 2])
    assert.equal(book.bestPrice('BUYS'), '189.48')
    assert.equal(book.bestPrice('SELL'), '189.52')
    assert.equal(book.askqty, '100')
    ```
