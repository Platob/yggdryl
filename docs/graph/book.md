# Book

A book is live depth over time: `BookSide` one side's depth, `BookEvent` both sides at an instant, `SnapshotEvent` the scope-replacing control, `BookIterator` folds a sorted stream into books.

## Contract

| Type | Owns | Traits |
| --- | --- | --- |
| `BookSide` | live order/quote depth (`Arc<MarketData>`s) + deltas since the last emitted book | `Element`, `Market` |
| `BookEvent` | a bid/ask side, executions at its instant, scopes its last snapshot replaced | `Element`, `Event`, `Market` - sides are its lanes |
| `SnapshotEvent` | an empty FIX `W`'s full-snapshot control: event + replaced scope, no entry | `Element`, `Event`, `Market` |
| `SnapshotPartition` | `{ symbol: Option<SmolStr>, scope: SmolStr }`: one replaced scope, symbol `None` in global mode | - |
| `BookIterator` | the [fold](#book-fold) from a sorted stream to books | `Iterator<Item = Result<BookEvent>>` |
| `yggdryl::Limit` | one [price limit](#limits) of a side, a root value type | - |

All in `graph::book`: keys `GLOBAL_SYMBOL` (`GLOBAL`), `ENTRY_ID` (`MDENTRYID`), `ENTRY_REF_ID` (`MDENTRYREFID`); `Limit` in root `limit.rs`.

## Book sides

| Key | Rule |
| --- | --- |
| `BookSide::new(side)` | a bid or ask side, else refused |
| `add_operation(MarketData)` | only `OrderEvent`/`QuoteEvent` on that side, else `InvalidRecord` naming the kind; stays in `deltas()` even with no live depth |
| `live()` | live entries in price order, best first, ties by `BookRef::position` then arrival; unpriced (market) orders rest last, and come last in the digest and in a delete-through or delete-from too |
| `best_price()`, `best_quantity()` | the first priced level's price, and the sum of its stated quantities (an unstated one counts 0); `None` when the side is empty or holds only unpriced entries |
| `len()`, `is_empty()`, `deltas()` | the live count; the operations applied since the last emitted book |
| Live key | `(symbol, scope, cross identity)`: a same-symbol, same-scope `MDENTRYREFID` resolves first, else the incoming identity or `MDENTRYID`; a second, distinct destination is ambiguous and refused |
| New, change, overlay | a new generation replaces and chains the live entry; a change or overlay (`is_partial`) inherits price and size only where it states none (zero stays zero); with no predecessor it is refused, never fabricated |
| Orders among quotes | a change or overlay (`1`, `5`) keeps a matched `OrderEvent` when `ORDERID` is omitted and promotes an unidentified `QuoteEvent` when it is stated; a contradiction is a located `InvalidRecord`; a delete (`2`) keeps the matched kind and predecessor link |
| Anonymous new | a new (`0`) without `MDENTRYID` never inserts into an occupied position: it states `MDENTRYID` or arrives in a snapshot |
| Range deletes | delete-through and delete-from (`3`, `4`) need a positive, in-range `BookRef::position` in the same symbol and scope; a missing or malformed position refuses atomically |
| As a market | the first priced level's price and aggregate quantity, and its first entry's currency and unit; nothing without a best |
| Identity | digests the list counts, then each operation's kind and canonical `curruuid` in order - never a child's `currhashcode` or content |

## Limits

`Limit { price: Option<Decimal>, quantity: Decimal, uuids: Vec<Uuid> }` is one price limit of a side: price (`None` folds unpriced entries), sum of entries' quantity, and their `curruuid`s in live order - read by equality and hash.

| Key | Rule |
| --- | --- |
| `BookSide::limits()` | one `Limit` per level, held order - best first, unpriced last - `uuids` in position then arrival order; allocates only `uuids` |
| `BookSide::depth(levels)` | sum of the first `levels` limits' quantities, unpriced counted where reached: zero for an empty side or no level, `None` only past [`decimal`](../types/numeric/decimal.md#decimal) |
| `BookEvent` | `is_locked()`: both bests equal; `is_crossed()`: bid above ask; `spread()`: best ask less best bid, negative if crossed, `None` if no best; `imbalance(levels)`: `(bid - ask) / (bid + ask)` over `depth(levels)` - `1`/`-1` one-sided, `None` if zero total, both empty, no level, or overflow |
| A value | not a datatype: `Limit::dtype()` = `struct<price: decimal?, quantity: decimal, uuids: serie<uuid>>` (required `uuid` item); `Limit::field()` = required `limit` item of a `limits` column; `into_scalar` = named `Scalar::Struct` of the three cells |
| `Limit::from_scalar` | reads that struct, or what `dtype().scalar` canonicalizes it to, through `Limit::field()`'s value door: refused under `$.limit` as a `limits` column would (empty text, float, grouped digits, 19th fractional digit, never zero) - except a name the struct lacks is null, so a missing quantity is refused, not zero |
| Bindings | `Limit` is Rust-only; Python: `side.limits` as `Scalar` structs (`limit["price"]`, `.as_py()` a dict), `side.depth(levels)`/`book.is_locked`/`spread`/`imbalance(levels)` as `Scalar`/`None`; JavaScript: `side.limits` as `{ price, quantity, uuids }` decimal text, same four as text or `null` |

## Books

| Key | Rule |
| --- | --- |
| `BookEvent::new(unix, symbol)` | empty bid/ask sides at that nanosecond; symbol = ticker + cross code; `bid()`/`ask()` expose the persistent sides |
| `add_operations` | any `IntoIterator` of `MarketData`/`Result<MarketData>`, no-op if empty; `OrderEvent`/`QuoteEvent`/`ExecutionEvent`/`TradeEvent`/`SnapshotEvent` fold, else `InvalidRecord` at `$.operations[index].kind`; every `currunix` must agree, regression refused atomically, applied in source order |
| A new instant | clears the prior instant's deltas, executions and snapshot stamp and keeps live depth |
| Snapshots | a full-snapshot order/quote/`SnapshotEvent` clears only its `(symbol, scope)` partition on both sides (an empty FIX `W` is one); `snapshot_partitions() -> &BTreeSet<SnapshotPartition>` reads them back, empty after an ordinary update; execution/trade never control resting membership |
| Changing sides | the same live identity on the other side continues its predecessor - chain place, market facts, lane values - then retires the old side before the new generation |
| Executions | a composite trade's bounds fold from its root, children flatten into `executions() -> &[ExecutionEvent]`; neither enters sides/deltas, none adds/removes/decrements depth; sorted by `curruuid`, deduplicated before the identity is derived |
| Bounds | max sequence, earliest creation/recording instants, latest execution instant of the finalized generation, after any predecessor advanced it; a rehydrated book checks all four against every nested operation |
| Identity | bid/ask canonical UUIDs in fixed positions, then ordered execution UUIDs and the scopes the last snapshot replaced; the fixed-width tail carries the count, no nested hash or content replayed |
| `is_crossed`, `bbo_midpoint` | crossed: bid strictly above ask; midpoint: overflow-safe `(bid + offer) / 2` of a two-sided BBO, per [SEC](https://www.sec.gov/files/rules/sro/btnl/2026/34-106421-ex4.pdf), `None` if one-sided or crossed |
| As a market | `price`: midpoint, else best price if not crossed, else `None`; `quantity`: `median_quantity()` - [NIST](https://www.itl.nist.gov/div898/handbook/eda/section3/eda351.htm) mean of the two best quantities, one-sided its own, else none; currency/unit only where a best exists (agree: one; one-sided: its own; else none); crossed still exposes both sides, their median, a negative `spread()` |
| Following | only the same cross code at a nondecreasing instant: an incremental book starts from the prior live sides, clears only replaced partitions, reapplies deltas; a grid or supplied-membership snapshot is complete, never refilled |
| Merging | only the same cross code/instant, later `recdunix` then `currunix` leading: its live entries lead, the other fills identities it lacks except in replaced partitions - complete membership admits no missing depth; deltas/executions union once, self-merge changes nothing |

## Snapshot controls

`SnapshotEvent::snapshot(&event, scope)` copies the event/market facts of any `Event + Market` (never an operation's), sets the scope under `MdUpdateAction::Snapshot`, finalizes through `digest_market_event`; `book()` reads the control. Python `graph.SnapshotEvent.snapshot(event, scope=None)`, JS `graph.SnapshotEvent.snapshot(event, scope)`.

## Book fold

`BookIterator::new(values, snapshot_millis, global)` folds sorted `MarketData`/`Result<MarketData>` into `Result<BookEvent>`s; Python `graph.BookIterator(items, snapshot_millis=0, global_=False)`, JS `new graph.BookIterator(items, snapshotMillis, global)`.

| Key | Rule |
| --- | --- |
| Input | what `add_operations` folds, else refused by kind at `$.operation.kind`; a timestamp regression refused; an input error follows the completed prefix once and fuses the iterator |
| Groups | by effective instant (`snapunix`, else `currunix`) - one cloned book per touched symbol, symbol order, atomic per timestamp/symbol; ordinary groups apply via `add_operations`; supplied-membership stages replacement with deltas/expirations; a failed group yields its error, book and pending expirations intact for retry |
| Symbols | outside global mode every input states a `ticker`; `global = true` combines under `GLOBAL_SYMBOL`, each symbol's lifecycle isolated first; `global()` answers the mode |
| Expiry | a live order/quote with `exprtime` produces a terminal delete of that generation at the exact instant, before equal-time source operations, without clearing its snapshot partition; executions/trades never enter retained lifecycle state |
| Grid | `snapshot_millis == 0` is none; positive emits at every crossed epoch-aligned tick the complete live book with `snapunix` set, no deltas/executions - living orders kept, dead ones only a delta where they died; a tick equal to a source/expiration instant applies first, then one tick regardless |
| Supplied membership | a stream stating `snapunix` is a membership view: orders/quotes replace only the `(symbol, scope)` partitions represented (create/update, purge omitted); `SnapshotEvent` is an empty one; a snapshotted execution keeps `execunix` at the view's instant, a trade rebases root/children to it; later-than-snapshot components refused; distinct from a FIX `W` |
| After each book | the retained book clears deltas/executions, so the next carries only its instant's changes; resting depth persists |
| FIX | [`FixCodec::market_operations`](../fix/arrow.md#fix-market-books) sorts a capture's operations by the effective instant this fold checks; Python `FixCodec.book_arrow_reader`/JS `bookArrowReader` run the fold to Arrow |

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
        order.set_ticker(Some("AAPL".into()));
        order.set_side(side);
        order.set_price(match price {
            Some(text) => Some(text.parse()?),
            None => None,
        });
        order.set_quantity(Some(Decimal::from_int(quantity)));
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

    // The two bests, and what the book reads from them.
    assert_eq!(book.bid().best_price(), Some(px("189.48")?));
    assert_eq!(book.ask().best_quantity(), Some(Decimal::from_int(100)));
    assert_eq!(book.spread(), Some(px("0.04")?));
    assert_eq!(book.bbo_midpoint(), Some(px("189.50")?));
    assert_eq!(book.get_price(), book.bbo_midpoint());
    assert_eq!(book.median_quantity(), Some(Decimal::from_int(200)));
    assert!(!book.is_locked() && !book.is_crossed());
    assert_eq!(book.imbalance(1), Some(px("0.5")?));

    // One limit per price, best first, the market order last and unpriced.
    let limits: Vec<Limit> = book.bid().limits().collect();
    let prices: Vec<Option<Decimal>> = limits.iter().map(|limit| limit.price).collect();
    assert_eq!(prices, [Some(px("189.48")?), Some(px("189.47")?), Some(px("189.45")?), None]);
    assert_eq!(book.bid().depth(2), Some(Decimal::from_int(800)));
    assert_eq!(book.bid().depth(4), Some(Decimal::from_int(1_050)));
    let best = book.bid().live().next().expect("the best bid");
    assert_eq!(best.as_order_event().map(Element::get_crosscode), Some("B-1"));

    // A limit is a value of its own: its field, and its scalar both ways.
    assert_eq!(Limit::field().name(), "limit");
    assert_eq!(Limit::from_scalar(&limits[0].into_scalar())?, limits[0]);
    let row = Limit::dtype().scalar(limits[3].into_scalar())?;
    assert_eq!(Limit::from_scalar(&row)?.price, None);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

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
            entry("B-1", "BUY", "189.48", 300),
            entry("B-2", "BUY", "189.47", 500),
            entry("B-3", "BUY", "189.45", 200),
            entry("A-1", "SELL", "189.52", 100),
            entry("A-2", "SELL", "189.53", 400),
            entry("A-3", "SELL", "189.55", 250),
            entry("MKT", "BUY", None, 50),
        ]
    )

    def value(scalar):
        assert scalar is not None
        return scalar.as_py()

    # The two bests, and what the book reads from them.
    assert value(book.bid.best_price) == Decimal("189.48")
    assert value(book.ask.best_quantity) == 100
    assert value(book.spread) == Decimal("0.04")
    assert value(book.bbo_midpoint) == Decimal("189.50")
    assert book.price == book.bbo_midpoint
    assert value(book.median_quantity) == 200
    assert not book.is_locked and not book.is_crossed
    assert value(book.imbalance(1)) == Decimal("0.5")

    # One limit per price, best first, the market order last and unpriced.
    limits = [limit.as_py() for limit in book.bid.limits]
    assert [limit["price"] for limit in limits] == [Decimal("189.48"), Decimal("189.47"), Decimal("189.45"), None]
    assert value(book.bid.depth(2)) == 800
    assert value(book.bid.depth(4)) == 1_050
    best = book.bid.live[0].as_order_event()
    assert best is not None and best.crosscode == "B-1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const entry = (code, side, price, quantity) => new graph.OrderEvent(T, {
      crosscode: code, ticker: 'AAPL', side, price, quantity,
    })
    const book = new graph.BookEvent(T, 'AAPL').withOperations([
      entry('B-1', 'BUY', '189.48', 300),
      entry('B-2', 'BUY', '189.47', 500),
      entry('B-3', 'BUY', '189.45', 200),
      entry('A-1', 'SELL', '189.52', 100),
      entry('A-2', 'SELL', '189.53', 400),
      entry('A-3', 'SELL', '189.55', 250),
      entry('MKT', 'BUY', undefined, 50),
    ])

    // The two bests, and what the book reads from them.
    assert.equal(book.bid.bestPrice, '189.48')
    assert.equal(book.ask.bestQuantity, '100')
    assert.equal(book.spread, '0.04')
    assert.equal(book.bboMidpoint, '189.5')
    assert.equal(book.price, book.bboMidpoint)
    assert.equal(book.medianQuantity, '200')
    assert.equal(book.isLocked || book.isCrossed, false)
    assert.equal(book.imbalance(1), '0.5')

    // One limit per price, best first, the market order last and unpriced.
    assert.deepEqual(book.bid.limits.map((limit) => limit.price), ['189.48', '189.47', '189.45', null])
    assert.equal(book.bid.depth(2), '800')
    assert.equal(book.bid.depth(4), '1050')
    assert.equal(book.bid.live[0].asOrderEvent().crosscode, 'B-1')
    ```

### A snapshot

An empty snapshot of the book's scope a second later: the stale depth goes, and nothing is invented.

=== "Rust"

    ```rust
    use yggdryl::graph::{
        BookEvent, Element, Market, MarketData, MdUpdateAction, OrderEvent, SnapshotEvent, SnapshotPartition,
    };
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, code: &str, side: Side, price: &str| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()));
        order.set_side(side);
        order.set_price(Some(price.parse()?));
        order.set_quantity(Some(Decimal::from_int(100)));
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut book = BookEvent::new(T, "AAPL");
    book.add_operations([order(T, "B-1", Side::Buy, "189.48")?, order(T, "A-1", Side::Sell, "189.52")?])?;
    assert_eq!((book.bid().len(), book.ask().len()), (1, 1));

    let mut at = OrderEvent::at(T + 1_000_000_000);
    at.set_ticker(Some("AAPL".into()));
    at.finalize();
    let control = SnapshotEvent::snapshot(&at, None);
    assert_eq!(control.book().action, Some(MdUpdateAction::Snapshot));

    book.add_operations([MarketData::from(control)])?;
    assert!(book.bid().is_empty() && book.ask().is_empty());
    assert_eq!(book.get_price(), None);
    let replaced = SnapshotPartition { symbol: Some("AAPL".into()), scope: "".into() };
    assert!(book.snapshot_partitions().iter().eq([&replaced]));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, code: str, side: str, price: str) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side=side, price=Decimal(price), quantity=100)

    book = graph.BookEvent(T, "AAPL").with_operations([order(T, "B-1", "BUY", "189.48"), order(T, "A-1", "SELL", "189.52")])
    assert (len(book.bid), len(book.ask)) == (1, 1)

    control = graph.SnapshotEvent.snapshot(graph.OrderEvent(T + 1_000_000_000, ticker="AAPL"))
    assert control.book.action == "snapshot"

    after = book.with_operations([control])
    assert after.bid.is_empty and after.ask.is_empty
    assert after.price is None
    assert after.snapshot_partitions == [graph.SnapshotPartition("", "AAPL")]
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
      .withOperations([order(T, 'B-1', 'BUY', '189.48'), order(T, 'A-1', 'SELL', '189.52')])
    assert.equal(book.bid.length + book.ask.length, 2)

    const control = graph.SnapshotEvent.snapshot(new graph.OrderEvent(T + 1_000_000_000n, { ticker: 'AAPL' }))
    assert.equal(control.book.action, 'snapshot')

    const after = book.withOperations([control])
    assert.ok(after.bid.isEmpty && after.ask.isEmpty)
    assert.equal(after.price, null)
    assert.deepEqual(after.snapshotPartitions.map((partition) => [partition.symbol, partition.scope]), [['AAPL', '']])
    ```

### The book fold

A sorted stream: a bid, then a better bid and a fill a second later; then the same with a 500 ms grid.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookIterator, BookSide, Element, Event, ExecutionEvent, Market, MarketData, OrderEvent};
    use yggdryl::{Decimal, Side};

    const T: i64 = 1_700_000_000_000_000_000;
    const SECOND: i64 = 1_000_000_000;
    let bid = |unix: i64, code: &str, price: &str, quantity: i64| -> yggdryl::Result<MarketData> {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode(code.to_owned());
        order.set_ticker(Some("AAPL".into()));
        order.set_side(Side::Buy);
        order.set_price(Some(price.parse()?));
        order.set_quantity(Some(Decimal::from_int(quantity)));
        order.finalize();
        Ok(MarketData::from(order))
    };
    let mut fill = ExecutionEvent::at(T + SECOND);
    fill.set_crosscode("E-1".to_owned());
    fill.set_ticker(Some("AAPL".into()));
    fill.set_side(Side::Buy);
    fill.set_lastpx(Some("189.52".parse()?));
    fill.set_lastqty(Some(Decimal::from_int(100)));
    fill.finalize();
    let stream = || -> yggdryl::Result<Vec<MarketData>> {
        Ok(vec![bid(T, "B-1", "189.48", 300)?, bid(T + SECOND, "B-2", "189.49", 200)?, MarketData::from(fill.clone())])
    };

    let books = BookIterator::new(stream()?.into_iter(), 0, false)?.collect::<yggdryl::Result<Vec<_>>>()?;
    assert_eq!(books.len(), 2, "one book per touched instant");
    let last = &books[1];
    assert_eq!((last.get_currunix(), last.bid().len()), (T + SECOND, 2), "depth persists");
    assert_eq!(last.bid().best_price(), Some("189.49".parse()?));
    assert_eq!(last.bid().deltas().len(), 1, "a book carries its own instant's changes");
    let executed: Vec<&str> = last.executions().iter().map(Element::get_crosscode).collect();
    assert_eq!(executed, ["E-1"]);

    // A 500 ms grid adds the living book at the crossed tick, with no deltas.
    let gridded = BookIterator::new(stream()?.into_iter(), 500, false)?.collect::<yggdryl::Result<Vec<_>>>()?;
    let ticks: Vec<(i64, usize)> = gridded.iter().map(|book| (book.get_currunix() - T, book.bid().deltas().len())).collect();
    assert_eq!(ticks, [(0, 1), (500_000_000, 0), (SECOND, 1)]);

    // A value a book does not fold is refused by its kind.
    let side = MarketData::from(BookSide::new(Side::Buy)?);
    let mut refused = BookIterator::new([side].into_iter(), 0, false)?;
    assert!(refused.next().expect("one result").is_err());
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    SECOND = 1_000_000_000

    def bid(unix: int, code: str, price: str, quantity: int) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode=code, ticker="AAPL", side="BUY", price=Decimal(price), quantity=quantity)

    fill = graph.ExecutionEvent(
        T + SECOND, crosscode="E-1", ticker="AAPL", side="BUY", lastpx=Decimal("189.52"), lastqty=100
    )
    stream = [bid(T, "B-1", "189.48", 300), bid(T + SECOND, "B-2", "189.49", 200), fill]

    books = list(graph.BookIterator(stream))
    assert len(books) == 2, "one book per touched instant"
    last = books[1]
    assert (last.currunix, len(last.bid)) == (T + SECOND, 2), "depth persists"
    assert last.bid.best_price is not None and last.bid.best_price.as_py() == Decimal("189.49")
    assert len(last.bid.deltas) == 1, "a book carries its own instant's changes"
    assert [execution.crosscode for execution in last.executions] == ["E-1"]

    # A 500 ms grid adds the living book at the crossed tick, with no deltas.
    gridded = list(graph.BookIterator(stream, snapshot_millis=500))
    assert [(book.currunix - T, len(book.bid.deltas)) for book in gridded] == [(0, 1), (500_000_000, 0), (SECOND, 1)]

    # A value a book does not fold is refused by its kind.
    try:
        list(graph.BookIterator([graph.BookSide("BUY")]))
    except ValueError as error:
        assert "$.operation.kind" in str(error) and "book_side" in str(error)
    else:
        raise AssertionError("a book side was folded")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const SECOND = 1_000_000_000n
    const bid = (unix, code, price, quantity) => new graph.OrderEvent(unix, {
      crosscode: code, ticker: 'AAPL', side: 'BUY', price, quantity,
    })
    const fill = new graph.ExecutionEvent(T + SECOND, {
      crosscode: 'E-1', ticker: 'AAPL', side: 'BUY', lastpx: '189.52', lastqty: 100,
    })
    const stream = [bid(T, 'B-1', '189.48', 300), bid(T + SECOND, 'B-2', '189.49', 200), fill]

    const books = [...new graph.BookIterator(stream)]
    assert.equal(books.length, 2, 'one book per touched instant')
    const last = books[1]
    assert.equal(last.currunix, T + SECOND)
    assert.equal(last.bid.length, 2, 'depth persists')
    assert.equal(last.bid.bestPrice, '189.49')
    assert.equal(last.bid.deltas.length, 1, "a book carries its own instant's changes")
    assert.deepEqual(last.executions.map((execution) => execution.crosscode), ['E-1'])

    // A 500 ms grid adds the living book at the crossed tick, with no deltas.
    const gridded = [...new graph.BookIterator(stream, 500)]
    assert.deepEqual(gridded.map((book) => [book.currunix - T, book.bid.deltas.length]), [
      [0n, 1], [500_000_000n, 0], [SECOND, 1],
    ])

    // A value a book does not fold is refused by its kind.
    assert.throws(() => [...new graph.BookIterator([new graph.BookSide('BUY')])], /\$\.operation\.kind.*book_side/)
    ```

## Edges

- An order/quote with no price (a market order) rests at its side's unpriced level: `best_price` skips it, `limits()` answers it last with no price, `depth` counts it once reached; a side of such entries states no best, a book of such sides states no `price`/`spread`.
- A level whose aggregate quantity would pass `decimal` is refused atomically at `$.quantity`, naming the price (or unpriced level), the side unchanged; `imbalance`/`spread` answer `None` rather than overflow.
- A book folds only a dated operation, a trade or a snapshot control: `BookEvent::add_operations` refuses an undated leaf, a `BookSide`, or a `BookEvent` at `$.operations[index].kind`; `BookIterator` at `$.operation.kind`, naming the kind it got.
