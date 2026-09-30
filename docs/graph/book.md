# Book

A book is live depth over time: `BookEvent` one book at an instant - the entries alive on both sides, the deltas since the book before, the executions at its instant, each side read as its price levels - `SnapshotEvent` the scope-replacing control, and `BookIterator` the fold of a sorted stream into books. Sorted books fold on into [candles](candle.md), and the [book display](serve.md) serves a table of them as candles, books and audits.

## Contract

| Type | Owns | Traits |
| --- | --- | --- |
| `BookEvent` | the orders and quotes alive on its bid and ask sides, the deltas applied since the book before, the executions at its instant | `Element`, `Event`, `Market` |
| `SnapshotEvent` | an empty FIX `W`'s full-snapshot control: event + replaced scope, no entry | `Element`, `Event`, `Market` |
| `BookIterator` | the [fold](#book-fold) from a sorted stream to books | `Iterator<Item = Result<BookEvent>>` |
| `yggdryl::Limit` | one [price level](#limits) of a side, a root value type | - |

All in `graph::book`, with the `IdType` keys `ENTRY_ID` (`mdentryid`) and `ENTRY_REF_ID` (`mdentryrefid`); `Limit` in the root `limit.rs`. A book states no side of its own - `Side::Unknown` - and neither a book nor a snapshot control is [sided](market.md#sides-and-cross-codes): its stored cross code states side `0` whatever side it is set to (`3:0:AAPL`), and a snapshot control over an order takes the order's base code under its own kind. Its row nests exactly `alive`, `deltas`, `executions`, `bidlimits` and `asklimits` ([Market data](market-data.md#arrow)).

## Entries

| Key | Rule |
| --- | --- |
| `alive()` | every live order and quote as `&MarketData`: the bid side's, best price first, ties by `BookRef::position` then arrival, entries stating no price (market orders) last; then the ask side's the same way |
| `deltas()` | the operations applied since the book before: the bid side's in the order they were applied, then the ask side's |
| `executions()` | the `ExecutionEvent`s at the book's instant, sorted by `curruuid` and deduplicated |
| Sides | an `OrderEvent`/`QuoteEvent` rests on the side its [`Side`](../types/enum/side.md) takes - the bid for `Side::is_bid` (`BUYS`, `BUYM`), the ask for `Side::is_ask` (`SELL`, `SELP`, `SSHT`, `SSEX`, `SELU`); any other - `UNKN`, a cross, `OPPO` - is refused by `add_operations` at `$.operation.side` (`expected a bid or ask operation, got "UNKN"`) and left out of its book, with a warning, by the [fold](#book-fold); an applied operation stays in `deltas()` even with no live depth |
| Live key | `(symbol, scope, cross identity)`: a same-symbol, same-scope `mdentryrefid` resolves first, else the incoming identity or `mdentryid`; a second, distinct destination is ambiguous and refused; the reference names one step, so a following entry never [takes the one its predecessor states](operation.md#following-and-merging) |
| New, change, overlay, delete | a new generation replaces and chains the live entry; a change or overlay (`is_partial`) - and a delete - inherits price and size only where it states none (zero stays zero); a change or overlay with no predecessor must state both, never fabricated |
| Orders among quotes | a change or overlay (`1`, `5`) keeps a matched `OrderEvent` when `orderid` is omitted and promotes an unidentified `QuoteEvent` when it is stated; a contradiction is a located `InvalidRecord`; a delete (`2`) keeps the matched kind and predecessor link |
| Anonymous new | a new (`0`) without `mdentryid` never inserts into an occupied position: it states `mdentryid` or arrives in a snapshot |
| Range deletes | delete-through and delete-from (`3`, `4`) need a positive, in-range `BookRef::position` in the same symbol and scope; a missing or malformed position refuses atomically |
| Changing sides | an entry reaching a live entry of the other side - by `mdentryrefid`, its identity or `mdentryid`, in the same symbol and scope - continues it (the chain's predecessor and place, market facts) and retires it from that side before the new generation |

## Limits

`Limit { price: Option<Decimal>, quantity: Decimal, uuids: Vec<Uuid>, tradable: bool }` is one price level of a side: its price (`None` folds the unpriced entries), the sum of its entries' quantities, their `curruuid`s in live order, and whether it can trade - read by equality and hash.

| Key | Rule |
| --- | --- |
| `limits(side)` | one `Limit` per level of the side `side` takes, held order - best first, unpriced last - `uuids` in position then arrival order; nothing for a side that is neither a bid nor an ask; allocates only `uuids`; what the book's row states under `bidlimits` (`BUYS`) and `asklimits` (`SELL`) |
| `tradable` | whether any entry of the level does not state `tradable = false`: an entry stating nothing is a live order no venue halted, so it trades, and only a level every entry of which states `false` cannot; the entries' own facts stay as stated |
| `best_price(side)`, `best_quantity(side)` | the first tradable priced level's price, and the sum of its entries' stated quantities (an unstated one counts 0); `None` for an empty side, one holding only unpriced entries, one no level of which can trade, or a side that is neither - a level that cannot trade is skipped, never answered |
| `depth(side, levels)` | the sum of the first `levels` limits' quantities, tradable or not, unpriced counted where reached: zero for an empty side or no level, `None` past [`decimal`](../types/numeric/decimal.md#decimal) or for a side that is neither |
| Readings | `is_locked()`: both best tradable prices equal; `is_crossed()`: the best tradable bid above the best tradable ask; `spread()`: best ask less best bid, negative if crossed, `None` if a side has no best; `imbalance(levels)`: `(bid - ask) / (bid + ask)` over `depth(levels)` - `1`/`-1` one-sided, `None` if zero total, both empty, no level, or overflow; none is a stored fact |
| A value | not a datatype: `Limit::dtype()` = `struct<price: decimal?, quantity: decimal, uuids: serie<uuid>, tradable: boolean>` (required `uuid` item); `Limit::field()` = the required `limit` item of a `bidlimits`/`asklimits` column; `into_scalar` = the named `Scalar::Struct` of the four cells |
| `Limit::from_scalar` | reads that struct, or what `dtype().scalar` canonicalizes it to, through `Limit::field()`'s value door, refused under `$.limit` as a column would refuse it (empty text, float, grouped digits, 19th fractional digit) - except that a name the struct lacks is null, so a missing quantity or `tradable` is refused, never a zero or a `false` |
| Bindings | `Limit` is Rust-only. A side is a `Side` member, its code or any spelling it reads. Python: `book.limits(side)` a list of struct `Scalar`s (`limit["price"]`; `.as_py()` a dict of `price`, `quantity`, `uuids`, `tradable`), `best_price(side)`, `best_quantity(side)`, `depth(side, levels)`, `imbalance(levels)` a `Scalar` or `None`, and the properties `spread`, `bbo_midpoint`, `median_quantity`, `is_locked`, `is_crossed`; JavaScript: `book.limits(side)` as `{ price, quantity, uuids, tradable }` with decimal text, `bestPrice(side)`, `bestQuantity(side)`, `depth(side, levels)`, `imbalance(levels)` as text or `null`, and the properties `spread`, `bboMidpoint`, `medianQuantity`, `isLocked`, `isCrossed` |

## Books

| Key | Rule |
| --- | --- |
| `BookEvent::new(unix, symbol)` | an empty book at that nanosecond, state `NEW`; `symbol` is its ticker (none when empty) and its cross code, stored as `3:0:{symbol}` |
| `add_operations` | any `IntoIterator` of `MarketData`/`Result<MarketData>`, no-op if empty; `OrderEvent`/`QuoteEvent`/`ExecutionEvent`/`TradeEvent`/`SnapshotEvent` fold, else `InvalidRecord` at `$.operations[index].kind`; every `currunix` must agree, regression refused, applied in source order; atomic - the book is unchanged on every error; the refusals on this page are this explicit call's, which [the fold](#book-fold) answers by leaving what a book refuses out, with a warning |
| Its inputs | a book with a ticker takes an input stating that ticker or none and refuses another at `$.operation.ticker` (`expected "AAPL", got "MSFT"`); a categorized book - what the [fold](#book-fold) opens for inputs stating no ticker, keyed `{miccode}:{cficode}` and stating no ticker - takes only an input whose [`book_crosscode`](market.md#sides-and-cross-codes) is its key (`expected book crosscode "XPAR:ESVUFR", got "XNAS:ESVUFR"`) |
| A new instant | clears the prior instant's deltas, executions and snapshot stamp and keeps live depth |
| Snapshots | a full-snapshot order/quote/`SnapshotEvent` clears only its `(symbol, scope)` partition on both sides (an empty FIX `W` is one) before its group applies; which partitions the last snapshot replaced is walk state - no row states it, and neither the digest nor equality reads it; execution/trade never control resting membership |
| Executions | a trade's children flatten into `executions()`; neither enters `alive()`/`deltas()`, none adds/removes/decrements depth |
| Bounds | the highest place of its members at the book's own instant (reset when the instant advances), earliest creation/recording instants, latest execution instant (the book's [`execunix`](market.md#contract)) of the finalized generation, after any predecessor advanced it; a rehydrated book checks all four against every nested operation, the place only against those at its instant |
| Identity | the book's event and market facts, its instant, each side's digest - its live count, each live entry's kind and canonical `curruuid`, its delta count and each delta's - then the execution UUIDs; no child's `currhashcode` or content |
| As a market | `price`: `bbo_midpoint()` - the overflow-safe `(bid + offer) / 2` of a two-sided BBO, per [SEC](https://www.sec.gov/files/rules/sro/btnl/2026/34-106421-ex4.pdf) - else the one best price, `None` if crossed; `quantity`: `median_quantity()` - [NIST](https://www.itl.nist.gov/div898/handbook/eda/section3/eda351.htm) mean of the two best quantities, one-sided its own; currency/unit those its first priced entries agree on (one-sided: its own; else none); `bidpx`/`bidqty` and `askpx`/`askqty` the best tradable levels, `bidccy`/`askccy` the book's currency beside a best - nothing where no level of a side can trade |
| Following | only the same cross code at a nondecreasing instant: an incremental book starts from the prior live sides, clears only replaced partitions, reapplies deltas; a grid or supplied-membership snapshot is complete, never refilled |
| Merging | only the same cross code/instant, later `recdunix` then `currunix` leading: its live entries lead, the other fills identities it lacks except in replaced partitions - complete membership admits no missing depth; deltas/executions union once, self-merge changes nothing |
| Bindings | Python `graph.BookEvent(currunix, symbol)`, `with_operations(items)` (a new book), the properties `alive`, `deltas`, `executions`; JavaScript `new graph.BookEvent(currunix, symbol)`, `withOperations(items)`, the methods `alive()`, `deltas()`, `executions()` |

## Snapshot controls

`SnapshotEvent::snapshot(&event, scope)` copies the event/market facts of any `Event + Market` (never an operation's), sets the scope under `MdUpdateAction::Snapshot`, finalizes through `digest_market_event`; `book()` reads the control. Python `graph.SnapshotEvent.snapshot(event, scope=None)`, JavaScript `graph.SnapshotEvent.snapshot(event, scope)`.

## Book fold

`BookIterator::new(values, snapshot_millis)` folds sorted `MarketData`/`Result<MarketData>` into `Result<BookEvent>`s; Python `graph.BookIterator(items, snapshot_millis=0)`, JavaScript `new graph.BookIterator(items, snapshotMillis = 0)`.

| Key | Rule |
| --- | --- |
| Input | what `add_operations` folds, else refused by kind at `$.operation.kind` - a value that is no operation of a book is the caller's mistake, not data; that refusal and a source failure each follow the completed prefix once and fuse the iterator |
| Books | one per [`book_crosscode`](market.md#sides-and-cross-codes): the input's ticker, else `{miccode}:{cficode}` (`XXXX` and `XXXXXX` where it names none), so instruments without a ticker still split by market and classification, each book's stored cross code `3:0:{key}`; the first input routed to a key decides whether its book states the key as its ticker, and no book adopts one later |
| One key, two spellings | a ticker may spell a category key - `XPAR:ESVUFR` - and then a ticker-less input of that market and class and an input with that ticker share one book, in either order |
| One book, two instruments | two ticker-less instruments of one market and class share a book, and entries naming one `MDEntryID` in one scope are one live entry; a [FIX entry's scope](../fix/message.md#market-data) names its symbol, else its ISIN or `forex` pair, which keeps them apart where either states one |
| Groups | by effective instant (`snapunix`, else `currunix`) - one cloned book per touched key, key order, atomic per timestamp/key; ordinary groups apply via `add_operations`, each chain where its first step arrived, its steps by place only where a step follows one of its chain at that instant, else in arrival order; supplied-membership stages replacement with deltas/expirations; a group the book refuses is left out whole, with a warning: none of its updates lands, the book and its pending expirations stand as they were, and the walk goes on |
| Left out | each with a deduplicated [warning](../fix/capture.md#warnings) and never an `Err` item: an `OrderEvent` or `QuoteEvent` stating neither the bid nor the ask, which rests on no side; operations dated before the book they would fold into; and a group the book refuses |
| Expiry | a live order/quote with `exprunix` produces a terminal delete of that generation at the exact instant, before equal-time source operations, without clearing its snapshot partition; executions/trades never enter retained lifecycle state |
| Grid | `snapshot_millis == 0` is none; positive emits at every crossed epoch-aligned tick the complete live book with `snapunix` set, no deltas/executions - living orders kept, dead ones only a delta where they died; a tick equal to a source/expiration instant applies first, then one tick regardless |
| Supplied membership | a stream stating `snapunix` is a membership view: orders/quotes replace only the `(symbol, scope)` partitions represented (create/update, purge omitted); `SnapshotEvent` is an empty one; a snapshotted execution keeps `execunix` at the view's instant, a trade rebases root/children to it; later-than-snapshot components left out with a warning, `add_operations` refusing them; distinct from a FIX `W` |
| After each book | the retained book clears deltas/executions, so the next carries only its instant's changes; resting depth persists |
| FIX | [`FixCodec::market_data`](../fix/arrow.md#fix-market-books) sorts a capture's market data by the effective instant this fold checks; `FixCodec::book_arrow_reader(messages, snapshot_millis)` - Python `FixCodec.book_arrow_reader(messages, snapshot_millis=0)`, JavaScript `codec.bookArrowReader(messages, snapshotMillis)` - runs the fold to Arrow, each execution and each side of a quote once, as the messages the parse split off |

## Examples

### A ladder

Three bids and three offers on Apple, plus a market order to buy 50.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, Market, MarketData, OrderEvent};
    use yggdryl::{Decimal, Limit, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let entry = |code: &str, side: Side, price: Option<&str>, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(T);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_side(side, true);
        order.set_price(match price {
            Some(text) => Some(text.parse()?),
            None => None,
        }, true);
        order.set_quantity(Some(Decimal::from_int(quantity)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::new(T, "AAPL");
    book.add_operations([
        entry("B-1", Side::Buy, Some("189.48"), 300)?,
        entry("B-2", Side::Buy, Some("189.47"), 500)?,
        entry("B-3", Side::Buy, Some("189.45"), 200)?,
        entry("A-1", Side::Sell, Some("189.52"), 100)?,
        entry("A-2", Side::Sell, Some("189.53"), 400)?,
        entry("A-3", Side::Sell, Some("189.55"), 250)?,
        entry("MKT", Side::Buy, None, 50)?,
    ])?;
    let px = |text: &str| text.parse::<Decimal>();

    // The two bests, and what the book reads and states from them.
    assert_eq!(book.best_price(Side::Buy), Some(px("189.48")?));
    assert_eq!(book.best_quantity(Side::Sell), Some(Decimal::from_int(100)));
    assert_eq!((book.get_bidpx(), book.get_askpx()), (Some(px("189.48")?), Some(px("189.52")?)));
    assert_eq!(book.spread(), Some(px("0.04")?));
    assert_eq!(book.bbo_midpoint(), Some(px("189.50")?));
    assert_eq!(book.get_price(), book.bbo_midpoint());
    assert_eq!(book.median_quantity(), Some(Decimal::from_int(200)));
    assert!(!book.is_locked() && !book.is_crossed());
    assert_eq!(book.imbalance(1), Some(px("0.5")?));

    // One limit per price, best first, the market order last and unpriced.
    let limits: Vec<Limit> = book.limits(Side::Buy).collect();
    let prices: Vec<Option<Decimal>> = limits.iter().map(|limit| limit.price).collect();
    assert_eq!(prices, [Some(px("189.48")?), Some(px("189.47")?), Some(px("189.45")?), None]);
    assert!(limits.iter().all(|limit| limit.tradable), "an entry stating nothing trades");
    assert_eq!(book.depth(Side::Buy, 2), Some(Decimal::from_int(800)));
    assert_eq!(book.depth(Side::Buy, 4), Some(Decimal::from_int(1_050)));
    let best = book.alive().next().expect("the best bid");
    assert_eq!(best.as_order_event().map(Element::get_crosscode), Some("10:1:B-1"));

    // A limit is a value of its own: its field, and its scalar both ways.
    assert_eq!(Limit::field().name(), "limit");
    assert_eq!(Limit::from_scalar(&limits[0].into_scalar())?, limits[0]);
    let row = Limit::dtype().scalar(limits[3].into_scalar())?;
    assert_eq!(Limit::from_scalar(&row)?.price, None);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def entry(code: str, side: str, price: str | None, quantity: int) -> graph.OrderEvent:
        return graph.OrderEvent(
            T,
            crosscode=code,
            ticker="AAPL",
            side=side,
            price=None if price is None else Decimal(price),
            quantity=quantity,
        )

    book = graph.BookEvent(T, "AAPL").with_operations(
        [
            entry("B-1", "BUYS", "189.48", 300),
            entry("B-2", "BUYS", "189.47", 500),
            entry("B-3", "BUYS", "189.45", 200),
            entry("A-1", "SELL", "189.52", 100),
            entry("A-2", "SELL", "189.53", 400),
            entry("A-3", "SELL", "189.55", 250),
            entry("MKT", "BUYS", None, 50),
        ]
    )

    def value(scalar):
        assert scalar is not None
        return scalar.as_py()

    # The two bests, and what the book reads and states from them.
    assert value(book.best_price(Side.BUYS)) == Decimal("189.48")
    assert value(book.best_quantity(Side.SELL)) == 100
    assert (value(book.bidpx), value(book.askpx)) == (Decimal("189.48"), Decimal("189.52"))
    assert value(book.spread) == Decimal("0.04")
    assert value(book.bbo_midpoint) == Decimal("189.50")
    assert book.price == book.bbo_midpoint
    assert value(book.median_quantity) == 200
    assert not book.is_locked and not book.is_crossed
    assert value(book.imbalance(1)) == Decimal("0.5")

    # One limit per price, best first, the market order last and unpriced.
    limits = [limit.as_py() for limit in book.limits(Side.BUYS)]
    assert [limit["price"] for limit in limits] == [Decimal("189.48"), Decimal("189.47"), Decimal("189.45"), None]
    assert all(limit["tradable"] for limit in limits), "an entry stating nothing trades"
    assert value(book.depth(Side.BUYS, 2)) == 800
    assert value(book.depth("BUYS", 4)) == 1_050
    best = book.alive[0].as_order_event()
    assert best is not None and best.crosscode == "10:1:B-1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Side, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const entry = (code, side, price, quantity) => new graph.OrderEvent(T, {
      crosscode: code, ticker: 'AAPL', side, price, quantity,
    })
    const book = new graph.BookEvent(T, 'AAPL').withOperations([
      entry('B-1', 'BUYS', '189.48', 300),
      entry('B-2', 'BUYS', '189.47', 500),
      entry('B-3', 'BUYS', '189.45', 200),
      entry('A-1', 'SELL', '189.52', 100),
      entry('A-2', 'SELL', '189.53', 400),
      entry('A-3', 'SELL', '189.55', 250),
      entry('MKT', 'BUYS', undefined, 50),
    ])

    // The two bests, and what the book reads and states from them.
    assert.equal(book.bestPrice('BUYS'), '189.48')
    assert.equal(book.bestQuantity(Side.SELL), '100')
    assert.deepEqual([book.bidpx, book.askpx], ['189.48', '189.52'])
    assert.equal(book.spread, '0.04')
    assert.equal(book.bboMidpoint, '189.5')
    assert.equal(book.price, book.bboMidpoint)
    assert.equal(book.medianQuantity, '200')
    assert.equal(book.isLocked || book.isCrossed, false)
    assert.equal(book.imbalance(1), '0.5')

    // One limit per price, best first, the market order last and unpriced.
    const limits = book.limits('BUYS')
    assert.deepEqual(limits.map((limit) => limit.price), ['189.48', '189.47', '189.45', null])
    assert.ok(limits.every((limit) => limit.tradable), 'an entry stating nothing trades')
    assert.equal(book.depth('BUYS', 2), '800')
    assert.equal(book.depth('BUYS', 4), '1050')
    assert.equal(book.alive()[0].asOrderEvent().crosscode, '10:1:B-1')
    ```

### Tradable levels

The best bid is the best level that can trade: a halted top level is skipped, never answered.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, Market, MarketData, Operation, OrderEvent};
    use yggdryl::{Ccy, Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let bid = |code: &str, price: &str, quantity: i64, tradable: Option<bool>| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(T);
        order.set_crosscode(code.to_owned());
        order.set_side(Side::Buy, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(quantity)), true);
        order.set_currency(Ccy::new("USD")?, true);
        order.set_tradable(tradable, true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::new(T, "AAPL");
    book.add_operations([
        bid("B-1", "189.48", 300, Some(false)),
        bid("B-2", "189.47", 500, None),
        bid("B-3", "189.47", 100, Some(false)),
    ])?;

    let tradable: Vec<(String, bool)> = book
        .limits(Side::Buy)
        .map(|limit| (limit.price.map(|px| px.to_string()).unwrap_or_default(), limit.tradable))
        .collect();
    // Every entry at 189.48 says it cannot trade; one at 189.47 says nothing.
    assert_eq!(tradable, [("189.48".to_owned(), false), ("189.47".to_owned(), true)]);
    assert_eq!(book.best_price(Side::Buy), Some("189.47".parse()?));
    assert_eq!(book.best_quantity(Side::Buy), Some(Decimal::from_int(600)));
    assert_eq!(book.get_bidpx(), book.best_price(Side::Buy));
    assert_eq!(book.get_bidccy().map(Ccy::as_str), Some("USD"));

    // No level that can trade: no best, and no bid.
    let mut halted = BookEvent::new(T, "AAPL");
    halted.add_operations([bid("B-1", "189.48", 300, Some(false))?])?;
    assert_eq!((halted.best_price(Side::Buy), halted.get_bidpx()), (None, None));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def bid(code: str, price: str, quantity: int, tradable: bool | None) -> graph.OrderEvent:
        return graph.OrderEvent(
            T,
            crosscode=code,
            side="BUYS",
            price=Decimal(price),
            quantity=quantity,
            currency="USD",
            tradable=tradable,
        )

    book = graph.BookEvent(T, "AAPL").with_operations(
        [bid("B-1", "189.48", 300, False), bid("B-2", "189.47", 500, None), bid("B-3", "189.47", 100, False)]
    )
    levels = [(limit.as_py()["price"], limit.as_py()["tradable"]) for limit in book.limits(Side.BUYS)]
    # Every entry at 189.48 says it cannot trade; one at 189.47 says nothing.
    assert levels == [(Decimal("189.48"), False), (Decimal("189.47"), True)]
    best = book.best_price(Side.BUYS)
    assert best is not None and best.as_py() == Decimal("189.47")
    assert book.bidpx == best
    assert book.bidccy is not None and book.bidccy.as_py() == "USD"

    # No level that can trade: no best, and no bid.
    halted = graph.BookEvent(T, "AAPL").with_operations([bid("B-1", "189.48", 300, False)])
    assert halted.best_price(Side.BUYS) is None and halted.bidpx is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const bid = (code, price, quantity, tradable) => new graph.OrderEvent(T, {
      crosscode: code, side: 'BUYS', price, quantity, currency: 'USD', tradable,
    })
    const book = new graph.BookEvent(T, 'AAPL').withOperations([
      bid('B-1', '189.48', 300, false),
      bid('B-2', '189.47', 500, undefined),
      bid('B-3', '189.47', 100, false),
    ])
    // Every entry at 189.48 says it cannot trade; one at 189.47 says nothing.
    assert.deepEqual(book.limits('BUYS').map((limit) => [limit.price, limit.tradable]), [
      ['189.48', false],
      ['189.47', true],
    ])
    assert.equal(book.bestPrice('BUYS'), '189.47')
    assert.equal(book.bestQuantity('BUYS'), '600')
    assert.equal(book.bidpx, '189.47')
    assert.equal(book.bidccy, 'USD')

    // No level that can trade: no best, and no bid.
    const halted = new graph.BookEvent(T, 'AAPL').withOperations([bid('B-1', '189.48', 300, false)])
    assert.equal(halted.bestPrice('BUYS'), null)
    assert.equal(halted.bidpx, null)
    ```

### A snapshot

An empty snapshot of the book's scope a second later: the stale depth goes, and nothing is invented.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, Market, MarketData, MdUpdateAction, OrderEvent, SnapshotEvent};
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, code: &str, side: Side, price: &str| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_side(side, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(100)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::new(T, "AAPL");
    book.add_operations([order(T, "B-1", Side::Buy, "189.48")?, order(T, "A-1", Side::Sell, "189.52")?])?;
    assert_eq!(book.alive().count(), 2);

    let mut at = OrderEvent::at(T + 1_000_000_000);
    at.set_ticker(Some("AAPL".into()), true);
    at.finalize();
    let control = SnapshotEvent::snapshot(&at, None);
    assert_eq!(control.book().action, Some(MdUpdateAction::Snapshot));

    book.add_operations([MarketData::from(control)])?;
    assert_eq!(book.alive().count(), 0);
    assert_eq!((book.get_price(), book.get_bidpx(), book.get_askpx()), (None, None, None));
    assert_eq!(book.limits(Side::Buy).count(), 0);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, code: str, side: str, price: str) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side=side, price=Decimal(price), quantity=100)

    book = graph.BookEvent(T, "AAPL").with_operations([order(T, "B-1", "BUYS", "189.48"), order(T, "A-1", "SELL", "189.52")])
    assert len(book.alive) == 2

    control = graph.SnapshotEvent.snapshot(graph.OrderEvent(T + 1_000_000_000, ticker="AAPL"))
    assert control.book.action == "snapshot"

    after = book.with_operations([control])
    assert after.alive == []
    assert after.price is None and after.bidpx is None and after.askpx is None
    assert after.limits(Side.BUYS) == []
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, code, side, price) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', side, price, quantity: 100,
    })
    const book = new graph.BookEvent(T, 'AAPL')
      .withOperations([order(T, 'B-1', 'BUYS', '189.48'), order(T, 'A-1', 'SELL', '189.52')])
    assert.equal(book.alive().length, 2)

    const control = graph.SnapshotEvent.snapshot(new graph.OrderEvent(T + 1_000_000_000n, { ticker: 'AAPL' }))
    assert.equal(control.book.action, 'snapshot')

    const after = book.withOperations([control])
    assert.deepEqual(after.alive(), [])
    assert.equal(after.price, null)
    assert.equal(after.bidpx, null)
    assert.deepEqual(after.limits('BUYS'), [])
    ```

### The book fold

A sorted stream: a bid, then a better bid and a fill a second later; then the same with a 500 ms grid.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookIterator, Element, Event, ExecutionEvent, Market, MarketData, Order, OrderEvent};
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    const SECOND: i64 = 1_000_000_000;
    let bid = |unix: i64, code: &str, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()), true);
        order.set_side(Side::Buy, true);
        order.set_price(Some(price.parse()?), true);
        order.set_quantity(Some(Decimal::from_int(quantity)), true);
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut fill = ExecutionEvent::at(T + SECOND);
    fill.set_crosscode("E-1".to_owned());
    fill.set_ticker(Some("AAPL".into()), true);
    fill.set_side(Side::Buy, true);
    fill.set_lastpx(Some("189.52".parse()?), true);
    fill.set_lastqty(Some(Decimal::from_int(100)), true);
    fill.finalize();
    let stream = || -> yggdryl::Result<Vec<MarketData>> {
        Ok(vec![bid(T, "B-1", "189.48", 300)?, bid(T + SECOND, "B-2", "189.49", 200)?, MarketData::from(fill.clone())])
    };

    let books = BookIterator::new(stream()?.into_iter(), 0)?.collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(books.len(), 2, "one book per touched instant");
    let last = &books[1];
    assert_eq!((last.get_currunix(), last.alive().count()), (T + SECOND, 2), "depth persists");
    assert_eq!(last.best_price(Side::Buy), Some("189.49".parse()?));
    assert_eq!(last.deltas().count(), 1, "a book carries its own instant's changes");
    let executed: Vec<&str> = last.executions().iter().map(Element::get_crosscode).collect();
    assert_eq!(executed, ["8:1:E-1"]);

    // A 500 ms grid adds the living book at the crossed tick, with no deltas.
    let gridded = BookIterator::new(stream()?.into_iter(), 500)?.collect::<yggdryl::Result<Vec<_>>>()?;
    let ticks: Vec<(i64, usize)> = gridded.iter().map(|book| (book.get_currunix() - T, book.deltas().count())).collect();
    assert_eq!(ticks, [(0, 1), (500_000_000, 0), (SECOND, 1)]);

    // A value a book does not fold is refused by its kind.
    let mut undated = Order::new();
    undated.finalize();
    let mut refused = BookIterator::new([MarketData::from(undated)].into_iter(), 0)?;
    let error = refused.next().expect("one result").unwrap_err();
    assert!(error.to_string().contains("$.operation.kind"), "{error}");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    T = 1_700_000_000_000_000_000
    SECOND = 1_000_000_000

    def bid(unix: int, code: str, price: str, quantity: int) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side="BUYS", price=Decimal(price), quantity=quantity)

    fill = graph.ExecutionEvent(
        T + SECOND, crosscode="E-1", ticker="AAPL", side="BUYS", lastpx=Decimal("189.52"), lastqty=100
    )
    stream = [bid(T, "B-1", "189.48", 300), bid(T + SECOND, "B-2", "189.49", 200), fill]

    books = list(graph.BookIterator(stream))
    assert len(books) == 2, "one book per touched instant"
    last = books[1]
    assert (last.currunix, len(last.alive)) == (T + SECOND, 2), "depth persists"
    best = last.best_price(Side.BUYS)
    assert best is not None and best.as_py() == Decimal("189.49")
    assert len(last.deltas) == 1, "a book carries its own instant's changes"
    assert [execution.crosscode for execution in last.executions] == ["8:1:E-1"]

    # A 500 ms grid adds the living book at the crossed tick, with no deltas.
    gridded = list(graph.BookIterator(stream, snapshot_millis=500))
    assert [(book.currunix - T, len(book.deltas)) for book in gridded] == [(0, 1), (500_000_000, 0), (SECOND, 1)]

    # A value a book does not fold is refused by its kind.
    try:
        list(graph.BookIterator([graph.Order()]))
    except ValueError as error:
        assert "$.operation.kind" in str(error) and "got order" in str(error)
    else:
        raise AssertionError("an undated order was folded")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const SECOND = 1_000_000_000n
    const bid = (unix, code, price, quantity) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', side: 'BUYS', price, quantity,
    })
    const fill = new graph.ExecutionEvent(T + SECOND, {
      crosscode: 'E-1', ticker: 'AAPL', side: 'BUYS', lastpx: '189.52', lastqty: 100,
    })
    const stream = [bid(T, 'B-1', '189.48', 300), bid(T + SECOND, 'B-2', '189.49', 200), fill]

    const books = [...new graph.BookIterator(stream)]
    assert.equal(books.length, 2, 'one book per touched instant')
    const last = books[1]
    assert.equal(last.currunix, T + SECOND)
    assert.equal(last.alive().length, 2, 'depth persists')
    assert.equal(last.bestPrice('BUYS'), '189.49')
    assert.equal(last.deltas().length, 1, "a book carries its own instant's changes")
    assert.deepEqual(last.executions().map((execution) => execution.crosscode), ['8:1:E-1'])

    // A 500 ms grid adds the living book at the crossed tick, with no deltas.
    const gridded = [...new graph.BookIterator(stream, 500)]
    assert.deepEqual(gridded.map((book) => [book.currunix - T, book.deltas().length]), [
      [0n, 1], [500_000_000n, 0], [SECOND, 1],
    ])

    // A value a book does not fold is refused by its kind.
    assert.throws(() => [...new graph.BookIterator([new graph.Order()])], /\$\.operation\.kind.*got order/)
    ```

### Books by key

One instant, two instruments: one with a ticker, one known only by its market and classification.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookIterator, Element, Market, MarketData, OrderEvent};
    use yggdryl::{Cfi, Decimal, Mic, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let mut apple = OrderEvent::at(T);
    apple.set_crosscode("B-1".to_owned());
    apple.set_ticker(Some("AAPL".into()), true);
    apple.set_side(Side::Buy, true);
    apple.set_price(Some("189.48".parse()?), true);
    apple.set_quantity(Some(Decimal::from_int(300)), true);
    apple.finalize();
    let mut unnamed = OrderEvent::at(T);
    unnamed.set_crosscode("B-2".to_owned());
    unnamed.set_miccode(Some(Mic::new("XPAR")?), true);
    unnamed.set_cficode(Some(Cfi::new("ESVUFR")?), true);
    unnamed.set_side(Side::Buy, true);
    unnamed.set_price(Some("42.10".parse()?), true);
    unnamed.set_quantity(Some(Decimal::from_int(10)), true);
    unnamed.finalize();

    let books = BookIterator::new([MarketData::from(apple), MarketData::from(unnamed)].into_iter(), 0)?
        .collect::<yggdryl::Result<Vec<_>>>()?;
    let keys: Vec<(&str, Option<&str>)> = books.iter().map(|book| (book.get_crosscode(), book.get_ticker())).collect();
    // A ticker names its book; with none, the market and class do, and that
    // book states no ticker.
    assert_eq!(keys, [("3:0:AAPL", Some("AAPL")), ("3:0:XPAR:ESVUFR", None)]);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    apple = graph.OrderEvent(T, crosscode="B-1", ticker="AAPL", side="BUYS", price=Decimal("189.48"), quantity=300)
    unnamed = graph.OrderEvent(
        T, crosscode="B-2", miccode="XPAR", cficode="ESVUFR", side="BUYS", price=Decimal("42.10"), quantity=10
    )

    books = list(graph.BookIterator([apple, unnamed]))
    # A ticker names its book; with none, the market and class do, and that
    # book states no ticker.
    assert [(book.crosscode, book.ticker) for book in books] == [("3:0:AAPL", "AAPL"), ("3:0:XPAR:ESVUFR", None)]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const apple = new graph.OrderEvent(T, { crosscode: 'B-1', ticker: 'AAPL', side: 'BUYS', price: '189.48', quantity: 300 })
    const unnamed = new graph.OrderEvent(T, {
      crosscode: 'B-2', miccode: 'XPAR', cficode: 'ESVUFR', side: 'BUYS', price: '42.10', quantity: 10,
    })

    const books = [...new graph.BookIterator([apple, unnamed])]
    // A ticker names its book; with none, the market and class do, and that
    // book states no ticker.
    assert.deepEqual(books.map((book) => [book.crosscode, book.ticker]), [['3:0:AAPL', 'AAPL'], ['3:0:XPAR:ESVUFR', null]])
    ```

## Edges

- An order/quote with no price (a market order) rests at its side's unpriced level: `best_price` skips it, `limits(side)` answers it last with no price, `depth` counts it once reached; a side of such entries states no best, a book of such sides states no `price`, `spread`, `bidpx` or `askpx`.
- A level whose aggregate quantity would pass `decimal` is refused atomically at `$.quantity`, naming the price (or unpriced level), the book unchanged; `imbalance`/`spread` answer `None` rather than overflow.
- A book folds only a dated operation, a trade or a snapshot control: `BookEvent::add_operations` refuses an undated leaf or a `BookEvent` at `$.operations[index].kind`, `BookIterator` at `$.operation.kind`, naming the kind it got (`expected order_event, quote_event, execution_event, trade_event or snapshot_event, got order`).
- An `OrderEvent` or `QuoteEvent` stating neither side - a quote whose two prices are its bid and ask, with no `Side` - is refused by `add_operations` and left out of its book by `BookIterator` and `book_arrow_reader`, with a warning; the book it would have folded into stands with what else its instant stated.
- An execution is never placed on a side, so it may state any side, `UNKN` included.
