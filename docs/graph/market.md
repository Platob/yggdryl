# Market

`Market` states twenty-eight facts: the instrument, the side, the price and quantity, the bid and the ask, what has traded and when it last did, the rates to other currencies and free-form metadata.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Market` (`graph::market`), no supertrait; Rust-only - [leaves](index.md#leaves) answer it in Python/JavaScript |
| Price, quantity, numbers | `get_price`/`get_quantity` + `set_`: what the element states, exact as [`Decimal`](../types/numeric/decimal.md#decimal), `None` if none - never last-executed, never a default; likewise `lastpx`/`lastqty` (last executed price/quantity), `avgpx`, `cumqty`, `leavesqty`, `prevpx`/`prevqty` (prior step's settlement), `spotrate`/`forwardpoints` (FX parts) |
| Currency, unit | `get_currency`/`set_currency`: [`Ccy::none()`](../types/codes/ccy.md) if unstated; `get_unit`/`set_unit`: [`Unit::none()`](../types/codes/unit.md) if unstated |
| Side | `get_side`/`set_side`: the [side](../types/enum/side.md) by value, never absent - `Side::Unknown` (code `0`) where none is stated, which means "not stated": nothing invents a side; an order's, a quote's or an execution's cross code carries it ([below](#sides-and-cross-codes)) |
| Sided | `is_sided()`: required - whether the element's cross code is stored under its side, true exactly for an order, a quote or an execution: [`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches) of the kind it is filed under, the one owner of the rule - a leaf answers its own kind, a [FIX message](../fix/message.md#market-data) its `msgcat` |
| Execution clock | `get_execunix`/`set_execunix` (`Option<i64>`): when the element last executed - the latest execution clock its lifecycle reached, nanoseconds since the Unix epoch, UTC, `None` where unknown; a market fact, not an event's: an undated order, quote or execution states one, a [text line](../media/index.md#plain-text) none, and no digest feeds it. The `execunix` column is a nullable nanosecond UTC clock ([Market data](market-data.md#columns)) |
| Bid and ask | `bidpx`, `bidqty`, `bidccy`, `askpx`, `askqty`, `askccy` ([below](#bid-and-ask)) |
| FX rates | `get_fxrates`/`set_fxrates`/`insert_fxrate` ([below](#fx-rates)) |
| Instrument | the [security identifiers](#security-identifiers); `get_isincode`: their `ISIN`, borrowed; `get_cficode`/`get_miccode` + setters - a leaf keeps a [CFI](../types/codes/cfi.md) only when it is detailed (one of positions 3-6 not `X`); `get_ticker`/`set_ticker`: an informal name, apart from the codes |
| Metadata | `get_metadata`/`set_metadata`: a `BTreeMap<SmolStr, SmolStr>` of source facts no typed column reads, keyed by name/[path](../types/paths.md); never identifier/typed; `None`/empty alike; a FIX leaf's = [`FixMsg::market_data`](../fix/message.md#market-data)'s |
| `fill_market` | provided, idempotent, called by `finalize` pre-digest: never invents price/quantity/`cumqty`/`leavesqty`/bid/ask/rates; [derives](#security-identifiers) the national id a canonical ISIN embeds |
| `digest_market` | provided (`Self: Element`): extends [`Element::digest`](element.md#contract) - price, currency, quantity, unit, side, security ids (key/code), classification, market, last-trade/avg/progress/FX parts, bid/ask (each only if stated), FX rates (only if any), ticker, metadata (key order); excludes `prevpx`/`prevqty`, like the predecessor's instant/identity |
| Provided on events | where `Self: Event`: `digest_market_event`, `following_market`, `merging_market_event` ([below](#following-and-merging)) |

## Sides and cross codes

| Key | Rule |
| --- | --- |
| `sided_crosscode(code)` | provided, the one speller of the prefix: for a sided element, `"{SIDE}:{code}"` under the side's stored name - `BUY:O-1001` - and `code` itself for `Side::Unknown` or an empty code, idempotent, another side's prefix replaced (`SELL:O-1001` read under `BUY` is `BUY:O-1001`); for any other element, `code` as given |
| Stored | every holder the crate ships stores its cross code through it, so `set_crosscode` and `set_side` in either order converge; [`crosshashcode` and `crossuuid`](element.md#contract) follow the stored text, and the two sides of one identifier are two chains ([walk](event.md#lifecycle-walk)) |
| Unprefixed | an order, a quote or an execution stating no side keeps its code as given, and so does every other element whatever side it states - a trade, a book, a snapshot control, a FIX message filed under any other category: a `BUY:` in such a code is its own name, never a prefix |
| Copied | a trade or a snapshot control built over a sided element - `TradeEvent::from_parts(&order, ..)`, `SnapshotEvent::snapshot(&order, ..)` - takes its base code, the prefix left off, with the cross hash and element of that code; a sided leaf built over such facts stores it under its side again |
| `book_crosscode()` | provided: the book the element stands in - its ticker where it states one (borrowed), else `{miccode}:{cficode}` with `XXXX` for no market and `XXXXXX` for no classification (`XPAR:ESVUFR`, `XXXX:XXXXXX`); what [`BookIterator`](book.md#book-fold) keys books by |

## Bid and ask

| Key | Rule |
| --- | --- |
| Facts | `get_bidpx`/`get_bidqty`/`get_askpx`/`get_askqty` (`Option<Decimal>`) and `get_bidccy`/`get_askccy` (`Option<&Ccy>`), each with its `set_`: the bid and the ask the element states, `None` where unstated |
| Never filled | not from the element's own price or quantity, nor from its last executed price: a buy order at 189.50 states no bid |
| Sources | a [book](book.md#books) states its best tradable levels, in its currency; a FIX message its `BidPx(132)`/`BidSize(134)` and `OfferPx(133)`/`OfferSize(135)`, each currency from a stated `BidCurrency`/`AskCurrency` (or `OfferCurrency`) field, else the message's ([FIX](../fix/message.md#market-data)) |
| Following, merging | following carries none; a merge takes each from the leading statement where it states one, else from the other |

## FX rates

`FxRates = BTreeMap<Ccy, Decimal>`: target currency to the rate to divide by - an amount in the element's `get_currency()` divided by `rates[target]` is that amount in `target`.

| Key | Rule |
| --- | --- |
| `get_fxrates` | the map, sorted by target; the one shared empty map (`empty_fxrates()`) where none is stated |
| `set_fxrates` | replaces the map; an empty map states none |
| `insert_fxrate(target, rate)` | provided: fills a target the element states no rate for, never replaces one; `true` if it did |
| Filled by | nothing: no FIX field, fill or following states a rate - a caller does |
| Merging | the union by target, the leading statement's rate where both state one |
| Column | `fxrates`: a sorted `map<ccy, decimal>`, keys and values required, the column nullable - null or empty states none ([Market data](market-data.md#columns)) |

## Security identifiers

| Verb | Rule |
| --- | --- |
| `get_securityids` | one validated code per source key, sorted; `get("ISIN")` (also `"4"` or `"isin"`) answers `Option<&str>` |
| `get_isincode` | provided: `get_securityids().get("ISIN")`, borrowed - a projection of the set, never a second store; the `isincode` column writes it and a stated cell fills an absent `ISIN` ([Market data](market-data.md#arrow)) |
| `set_securityids` | replaces the whole set (all then stated); `SecurityIds::default()` unsays them |
| `insert_securityid` | fills an absent key or replaces a derived one (then stated) - never a stated one; `true` if it did |
| `derive_securityid` | fills only an absent key from implication, not statement; never reaches the holder's backing store |
| `remove_securityid` | removes one key; removing the ISIN also revokes everything derived under it |
| `FOREX` | the crate's own key, which FIX gives no source code: a [currency pair](../types/codes/forex.md) in any spelling `Forex::new` reads, stored canonical (`eurusd` is `EUR/USD`); `forex`, `forexcode`, `ccypair` and `currencypair` read the key too |
| Derived | an ISIN's embedded national number - CUSIP (`US`/`CA`), SEDOL (`GB`/`IE`/`GG`/`JE`/`IM`, behind `00`), WKN (`DE`, behind `000`), Valor (`CH`/`LI`) ([`securityid::embedded`](../types/codes/isin.md)) - plus lifecycle-learned entries; all hang on the ISIN, so removing/replacing it revokes them |
| Provenance | derivation provenance isn't content: not carried in a row, and doesn't affect equality |
| Refusals | `SecurityId::new` validates by key; `SecType::read` refuses `TICKER` (use `set_ticker`); a [FIX message](../fix/message.md) refuses keys no field of its dictionary states |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_market` | [`Event::following`](event.md#following); `prevpx`/`prevqty`, currency, unit, side (this element's where it states one, the chain's where it states `UNKNOWN`), ticker, each lacked security identifier, classification, market - all from the predecessor where this event says nothing; always leads, even with the timed link unchanged |
| Two instruments | two stated ISINs that differ name two instruments: nothing is taken from the predecessor, and a merge keeps the leading statement's identifiers whole; a `ZZ` ISIN names no country's instrument, so it is never the other one - it yields to the real ISIN, which replaces it and everything derived under it |
| Execution clock | a market event whose state reports an execution ([`is_execution`](event.md#contract)) and states no `execunix` is dated from its own instant first - before it names a predecessor, so an inherited state is never read as its own execution; following then keeps the later of its own clock and its predecessor's, so a delayed report cannot regress it, and a non-execution carries the chain's latest |
| Restating | a market event's [`restating`](event.md#restating) also takes the market's place: `prevpx`/`prevqty`, what the chain is about where this reading said nothing, and the execution clock - the earliest of the two statements' |
| `merging_market` | where `Self: Element`, no event clocks: `self` leads, and the execution clock is the earliest either states |
| `merging_market_event` | [`Event::merging`](event.md#merging)'s reference leads: price/quantity/unit stand; optional numbers, bid/ask facts, ticker = reference's if stated else other's; the execution clock the earliest either statement knows (an unstamped execution observation dates itself first); metadata and FX rates unioned, reference-led; currency/side/classification/market = the better - an unknown yields to a known one (an `XXXX` [MIC](../types/codes/mic.md), the [CFI](../types/codes/cfi.md) merge); security ids: reference's replace by key, other's fill gaps |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Examples

### The facts of an order

=== "Rust"

    ```rust
    use smol_str::SmolStr;
    use yggdryl::graph::{Element, Market, Metadata, OrderEvent};
    use yggdryl::{Ccy, Cfi, Decimal, Mic, SecType, SecurityId, Side};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy);
    order.set_price(Some("189.50".parse()?));
    order.set_quantity(Some(Decimal::from_int(100)));
    order.set_currency(Ccy::new("USD")?);
    order.set_ticker(Some("AAPL".into()));
    order.set_miccode(Some(Mic::new("XNAS")?));
    order.set_cficode(Some(Cfi::new("ESVUFR")?));
    order.insert_securityid(SecurityId::new(SecType::read("ISIN")?, "US0378331005")?)?;
    order.set_metadata(Some(Metadata::from([(SmolStr::new("ordtype"), SmolStr::new("2"))])));
    order.finalize();

    assert_eq!(order.get_price(), Some("189.5".parse()?));
    assert_eq!(order.get_currency().as_str(), "USD");
    assert_eq!(order.get_side(), Side::Buy);
    assert!(order.get_unit().is_none(), "unstated");
    // A price is what the element states, never a last executed price or a bid.
    assert_eq!((order.get_lastpx(), order.get_bidpx()), (None, None));
    // The ISIN is stated under any spelling of its key, and carries a CUSIP.
    assert_eq!(order.get_securityids().get("4"), Some("US0378331005"));
    assert_eq!(order.get_isincode(), Some("US0378331005"));
    assert_eq!(order.get_securityids().get("cusip"), Some("037833100"));
    assert_eq!(order.get_metadata().get("ordtype").map(SmolStr::as_str), Some("2"));
    // The ticker names the book it stands in.
    assert_eq!(order.book_crosscode(), "AAPL");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        side="BUY",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        ticker="AAPL",
        miccode="XNAS",
        cficode="ESVUFR",
        securityids={"ISIN": "US0378331005"},
        metadata={"ordtype": "2"},
    )

    assert order.price is not None and order.price.as_py() == Decimal("189.50")
    assert order.currency.as_py() == "USD"
    assert order.side is Side.BUY
    assert order.unit == "", "unstated"
    # A price is what the element states, never a last executed price or a bid.
    assert order.lastpx is None and order.bidpx is None
    # The ISIN carries a CUSIP.
    assert order.securityids == {"CUSIP": "037833100", "ISIN": "US0378331005"}
    assert order.isincode == "US0378331005"
    assert order.miccode is not None and order.miccode.as_py() == "XNAS"
    assert order.metadata == {"ordtype": "2"}
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
      miccode: 'XNAS',
      cficode: 'ESVUFR',
      securityids: { ISIN: 'US0378331005' },
      metadata: { ordtype: '2' },
    })

    assert.equal(order.price, '189.5')
    assert.equal(order.currency, 'USD')
    assert.equal(order.side, 'BUY')
    assert.equal(order.unit, '', 'unstated')
    // A price is what the element states, never a last executed price or a bid.
    assert.equal(order.lastpx, null)
    assert.equal(order.bidpx, null)
    // The ISIN carries a CUSIP.
    assert.deepEqual(order.securityids, { CUSIP: '037833100', ISIN: 'US0378331005' })
    assert.equal(order.isincode, 'US0378331005')
    assert.equal(order.miccode, 'XNAS')
    assert.deepEqual(order.metadata, { ordtype: '2' })
    ```

### Sides and cross codes

One identifier, `O-1001`, on each side of the market: two cross codes, two chains. A trade built on the buy order is not sided, so it keeps the order's base code.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, ExecutionEvent, Market, OrderEvent, TradeEvent};
    use yggdryl::{Cfi, Mic, Side};

    let order = |side: Side| {
        let mut order = OrderEvent::at(1_700_000_000_000_000_000);
        order.set_crosscode("O-1001".to_owned());
        order.set_side(side);
        order.finalize();
        order
    };
    let (buy, sell) = (order(Side::Buy), order(Side::Sell));
    assert_eq!((buy.get_crosscode(), sell.get_crosscode()), ("BUY:O-1001", "SELL:O-1001"));
    assert_ne!(buy.get_crossuuid(), sell.get_crossuuid());
    // The prefix is spelled in one place, and a side taken later re-prefixes.
    assert_eq!(buy.sided_crosscode("SELL:O-1001"), "BUY:O-1001");
    let mut flipped = buy.clone();
    flipped.set_side(Side::Sell);
    assert_eq!(flipped.get_crosscode(), "SELL:O-1001");
    // An order stating no side keeps its code as given.
    assert_eq!(order(Side::Unknown).get_crosscode(), "O-1001");

    // Only an order, a quote or an execution is sided: a book and a trade
    // keep their code as given whatever side they state, and a trade built
    // on the buy order takes its base code.
    let mut book = BookEvent::new(1_700_000_000_000_000_000, "AAPL");
    book.set_side(Side::Buy);
    assert!(buy.is_sided() && !book.is_sided());
    assert_eq!(book.get_crosscode(), "AAPL");
    let mut fill = ExecutionEvent::at(1_700_000_000_000_000_000);
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::Sell);
    let trade = TradeEvent::from_parts(&buy, vec![fill])?;
    assert_eq!((trade.get_crosscode(), trade.get_side()), ("O-1001", Side::Buy));
    assert_eq!(trade.executions()[0].get_crosscode(), "SELL:E-1");

    // With no ticker, the book an element stands in is its market and class.
    let mut unnamed = order(Side::Buy);
    unnamed.set_miccode(Some(Mic::new("XPAR")?));
    unnamed.set_cficode(Some(Cfi::new("ESVUFR")?));
    assert_eq!(unnamed.book_crosscode(), "XPAR:ESVUFR");
    assert_eq!(order(Side::Buy).book_crosscode(), "XXXX:XXXXXX");
    ```

=== "Python"

    ```python
    from yggdryl import Side, graph

    buy = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side="BUY")
    sell = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side=Side.SELL)
    assert (buy.crosscode, sell.crosscode) == ("BUY:O-1001", "SELL:O-1001")
    assert buy.crossuuid != sell.crossuuid
    # An order stating no side keeps its code as given, and reads UNKNOWN.
    unsided = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    assert unsided.crosscode == "O-1001" and unsided.side is Side.UNKNOWN

    # A trade is not sided: built on the buy order, it keeps the order's base
    # code whatever side it states, and each execution its own sided one.
    fill = graph.ExecutionEvent(1_700_000_000_000_000_000, crosscode="E-1", side="SELL")
    trade = graph.TradeEvent.from_parts(buy, [fill])
    assert (trade.crosscode, trade.side) == ("O-1001", Side.BUY)
    assert trade.executions[0].crosscode == "SELL:E-1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Side, graph } = require('yggdryl')

    const buy = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'BUY' })
    const sell = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'SELL' })
    // A side crosses as its member's name; `Side` maps each name to its code.
    assert.equal(Side[sell.side], 2)
    assert.deepEqual([buy.crosscode, sell.crosscode], ['BUY:O-1001', 'SELL:O-1001'])
    assert.notEqual(buy.crossuuid, sell.crossuuid)
    // An order stating no side keeps its code as given, and reads UNKNOWN.
    const unsided = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    assert.equal(unsided.crosscode, 'O-1001')
    assert.equal(unsided.side, 'UNKNOWN')

    // A trade is not sided: built on the buy order, it keeps the order's base
    // code whatever side it states, and each execution its own sided one.
    const fill = new graph.ExecutionEvent(1_700_000_000_000_000_000n, { crosscode: 'E-1', side: 'SELL' })
    const trade = graph.TradeEvent.fromParts(buy, [fill])
    assert.deepEqual([trade.crosscode, trade.side], ['O-1001', 'BUY'])
    assert.equal(trade.executions[0].crosscode, 'SELL:E-1')
    ```

`is_sided`, `sided_crosscode` and `book_crosscode` are Rust-only; a binding reads the stored `crosscode`.

### Bid, ask and FX rates

A two-sided EUR/USD quote in dollars, and the rate a dollar amount is divided by to state it in euros.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Market, QuoteEvent};
    use yggdryl::{Ccy, SecType, SecurityId, Side};

    let mut quote = QuoteEvent::at(1_700_000_000_000_000_000);
    quote.set_crosscode("Q-7".to_owned());
    quote.set_currency(Ccy::new("USD")?);
    quote.insert_securityid(SecurityId::new(SecType::read("ccypair")?, "eurusd")?)?;
    quote.set_bidpx(Some("1.0842".parse()?));
    quote.set_askpx(Some("1.0844".parse()?));
    quote.set_bidccy(Some(Ccy::new("USD")?));
    quote.set_askccy(Some(Ccy::new("USD")?));
    quote.set_price(Some("125".parse()?));
    assert!(quote.insert_fxrate(Ccy::new("EUR")?, "1.25".parse()?));
    assert!(!quote.insert_fxrate(Ccy::new("EUR")?, "2".parse()?), "a stated rate stands");
    quote.finalize();

    // The pair lands canonical under the crate's own key.
    assert_eq!(quote.get_securityids().get("FOREX"), Some("EUR/USD"));
    // Two prices and no side: a bid and an ask are facts, not a side.
    assert_eq!(quote.get_side(), Side::Unknown);
    assert_eq!(quote.get_bidpx(), Some("1.0842".parse()?));
    assert_eq!(quote.get_askccy().map(Ccy::as_str), Some("USD"));
    // An amount in the quote's currency divided by a rate is in its target.
    let rate = quote.get_fxrates()[&Ccy::new("EUR")?];
    let price = quote.get_price().expect("a price");
    assert_eq!(price.checked_div(rate), Some("100".parse()?));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Side, graph

    quote = graph.QuoteEvent(
        1_700_000_000_000_000_000,
        crosscode="Q-7",
        currency="USD",
        securityids={"FOREX": "eurusd"},
        bidpx=Decimal("1.0842"),
        askpx=Decimal("1.0844"),
        bidccy="USD",
        askccy="USD",
        price=Decimal("125"),
        fxrates={"EUR": Decimal("1.25")},
    )

    # The pair lands canonical under the crate's own key.
    assert quote.securityids == {"FOREX": "EUR/USD"}
    # Two prices and no side: a bid and an ask are facts, not a side.
    assert quote.side is Side.UNKNOWN
    assert quote.bidpx is not None and quote.bidpx.as_py() == Decimal("1.0842")
    assert quote.askccy is not None and quote.askccy.as_py() == "USD"
    # An amount in the quote's currency divided by a rate is in its target.
    rates = {target: rate.as_py() for target, rate in quote.fxrates.items()}
    assert rates == {"EUR": Decimal("1.25")}
    assert quote.price is not None and quote.price.as_py() / rates["EUR"] == 100
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const quote = new graph.QuoteEvent(1_700_000_000_000_000_000n, {
      crosscode: 'Q-7',
      currency: 'USD',
      securityids: { FOREX: 'eurusd' },
      bidpx: '1.0842',
      askpx: '1.0844',
      bidccy: 'USD',
      askccy: 'USD',
      price: '125',
      fxrates: { EUR: '1.25' },
    })

    // The pair lands canonical under the crate's own key.
    assert.deepEqual(quote.securityids, { FOREX: 'EUR/USD' })
    // Two prices and no side: a bid and an ask are facts, not a side.
    assert.equal(quote.side, 'UNKNOWN')
    assert.equal(quote.bidpx, '1.0842')
    assert.equal(quote.askccy, 'USD')
    // Rates cross as decimal text under their target currency.
    assert.deepEqual(quote.fxrates, { EUR: '1.25' })
    ```

`insert_fxrate` is Rust-only: a binding states the whole map when it builds a leaf.

### The execution clock

When an element last executed is one of its market facts, stated by an undated element as by a dated one. An event that states none carries its predecessor's, and a delayed report of an earlier execution never moves it back.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Market, Order, OrderEvent};

    const T: i64 = 1_700_000_000_000_000_000;
    // An undated element states it like any market fact.
    let mut resting = Order::new();
    resting.set_execunix(Some(T));
    assert_eq!(resting.get_execunix(), Some(T));

    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.set_execunix(Some(T - 1_000_000));
    placed.finalize();

    // A later event stating none carries the chain's latest execution.
    let mut amended = OrderEvent::at(T + 1_000_000_000);
    amended.set_crosscode("O-1001".to_owned());
    amended.finalize();
    let amended = amended.with_previous(&placed).expect("a later event follows");
    assert_eq!(amended.get_execunix(), Some(T - 1_000_000));

    // A delayed report of an earlier execution cannot regress it.
    let mut delayed = OrderEvent::at(T + 2_000_000_000);
    delayed.set_crosscode("O-1001".to_owned());
    delayed.set_execunix(Some(T - 5_000_000));
    delayed.finalize();
    let delayed = delayed.with_previous(&amended).expect("a later event follows");
    assert_eq!(delayed.get_execunix(), Some(T - 1_000_000));
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    # An undated element states it like any market fact.
    assert graph.Order(execunix=T).execunix == T

    placed = graph.OrderEvent(T, crosscode="O-1001", execunix=T - 1_000_000)
    # A later event stating none carries the chain's latest execution.
    amended = graph.OrderEvent(T + 1_000_000_000, crosscode="O-1001").with_previous(placed)
    assert amended is not None and amended.execunix == T - 1_000_000
    # A delayed report of an earlier execution cannot regress it.
    delayed = graph.OrderEvent(T + 2_000_000_000, crosscode="O-1001", execunix=T - 5_000_000).with_previous(amended)
    assert delayed is not None and delayed.execunix == T - 1_000_000
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    // An undated element states it like any market fact.
    assert.equal(new graph.Order({ execunix: T }).execunix, T)

    const placed = new graph.OrderEvent(T, { crosscode: 'O-1001', execunix: T - 1_000_000n })
    // A later event stating none carries the chain's latest execution.
    const amended = new graph.OrderEvent(T + 1_000_000_000n, { crosscode: 'O-1001' }).withPrevious(placed)
    assert.equal(amended.execunix, T - 1_000_000n)
    // A delayed report of an earlier execution cannot regress it.
    const delayed = new graph.OrderEvent(T + 2_000_000_000n, { crosscode: 'O-1001', execunix: T - 5_000_000n })
      .withPrevious(amended)
    assert.equal(delayed.execunix, T - 1_000_000n)
    ```

### The identifier verbs

Rust-only: a binding states identifiers when it builds a leaf.

```rust
use yggdryl::graph::{Element, Market, OrderEvent};
use yggdryl::{SecType, SecurityId};

let id = |key: &str, code: &str| -> yggdryl::Result<SecurityId> {
    SecurityId::new(SecType::read(key)?, code)
};
let keys = |order: &OrderEvent| -> Vec<String> {
    order.get_securityids().iter().map(ToString::to_string).collect()
};
let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.insert_securityid(id("ISIN", "US0378331005")?)?;
order.finalize();
assert_eq!(keys(&order), ["CUSIP:037833100", "ISIN:US0378331005"]);

// A derivation fills an absent key only, and a stated identifier stands.
assert!(!order.derive_securityid(id("CUSIP", "594918104")?));
assert!(!order.insert_securityid(id("ISIN", "US5949181045")?)?);

// Everything derived hangs on the ISIN: removing it takes the CUSIP back.
let mut unlisted = order.clone();
assert!(unlisted.remove_securityid(&SecType::read("ISIN")?)?);
assert!(unlisted.get_securityids().is_empty());
assert_eq!(unlisted.get_isincode(), None);

// A stated identifier replaces a derived one, and then outlives the ISIN.
assert!(order.insert_securityid(id("CUSIP", "037833100")?)?);
assert!(order.remove_securityid(&SecType::read("ISIN")?)?);
assert_eq!(keys(&order), ["CUSIP:037833100"]);

// A currency pair is refused where it names no pair.
assert!(id("FOREX", "EUR/EUR").is_err());
```

### Identifiers along a chain

An amendment to an Apple order, and one naming Microsoft's ISIN instead.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, OrderEvent};
    use yggdryl::{SecType, SecurityId, SecurityIds};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, stated: &[(&str, &str)]| -> yggdryl::Result<OrderEvent> {
        let mut ids = SecurityIds::default();
        for (key, code) in stated {
            ids.insert(SecurityId::new(SecType::read(key)?, code)?);
        }
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1001".to_owned());
        order.set_securityids(ids)?;
        order.finalize();
        Ok(order)
    };
    let keys = |order: &OrderEvent| -> Vec<String> {
        order.get_securityids().iter().map(ToString::to_string).collect()
    };
    let placed = order(T, &[("ISIN", "US0378331005"), ("FIGI", "BBG000B9XRY4")])?;
    assert_eq!(keys(&placed), ["CUSIP:037833100", "FIGI:BBG000B9XRY4", "ISIN:US0378331005"]);

    let amended = order(T + 1, &[])?.with_previous(&placed).expect("a later event follows");
    assert_eq!(keys(&amended), keys(&placed));
    let other = order(T + 1, &[("ISIN", "US5949181045")])?.with_previous(&placed).expect("it follows");
    assert_eq!(keys(&other), ["CUSIP:594918104", "ISIN:US5949181045"]);

    // Two statements naming different ISINs never mix: the leading
    // statement - here the later recording - keeps its identifiers whole.
    let mut restated = placed.clone();
    restated.set_recdunix(Some(T + 5));
    restated.set_securityids(other.get_securityids().clone())?;
    let merged = placed.clone().merge_with(&restated).expect("the later recording leads");
    assert_eq!(keys(&merged), ["CUSIP:594918104", "ISIN:US5949181045"]);
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000

    def order(unix: int, securityids: dict[str, str]) -> graph.OrderEvent:
        return graph.OrderEvent(unix, crosscode="O-1001", securityids=securityids)

    placed = order(T, {"FIGI": "BBG000B9XRY4", "ISIN": "US0378331005"})
    assert placed.securityids == {"CUSIP": "037833100", "FIGI": "BBG000B9XRY4", "ISIN": "US0378331005"}

    amended = order(T + 1, {}).with_previous(placed)
    assert amended is not None and amended.securityids == placed.securityids
    other = order(T + 1, {"ISIN": "US5949181045"}).with_previous(placed)
    assert other is not None and other.securityids == {"CUSIP": "594918104", "ISIN": "US5949181045"}
    assert other.isincode == "US5949181045"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, securityids) => new graph.OrderEvent(unix, { crosscode: 'O-1001', securityids })

    const placed = order(T, { FIGI: 'BBG000B9XRY4', ISIN: 'US0378331005' })
    assert.deepEqual(placed.securityids, { CUSIP: '037833100', FIGI: 'BBG000B9XRY4', ISIN: 'US0378331005' })

    const amended = order(T + 1n, {}).withPrevious(placed)
    assert.deepEqual(amended.securityids, placed.securityids)
    const other = order(T + 1n, { ISIN: 'US5949181045' }).withPrevious(placed)
    assert.deepEqual(other.securityids, { CUSIP: '594918104', ISIN: 'US5949181045' })
    assert.equal(other.isincode, 'US5949181045')
    ```

## Edges

- On an order, a quote or an execution, `set_side(Side::Unknown)` after a sided code keeps the prefix the code has: a code is re-prefixed only under a known side. On any other element `set_side` never touches the code.
- A ticker stated empty is none: `book_crosscode` falls back to the market and class.
- A zero rate is a statement like any other; dividing by it is the caller's refusal to make - `Decimal::checked_div` answers `None`.
