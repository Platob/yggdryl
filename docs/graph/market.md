# Market

`Market` states thirty-four facts: the kind and type of the element, the instrument, the side, the price and quantity - shown, hidden and stopped at -, the bid and the ask, what has traded and when it last did, the rates to other currencies and free-form metadata.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Market` (`graph::market`), no supertrait; Rust-only - [leaves](index.md#leaves) answer it in Python/JavaScript |
| Price, quantity, numbers | `get_price`/`get_quantity` + `set_`: what the element states, exact as [`Decimal`](../types/numeric/decimal.md#decimal), `None` if none - never last-executed, never a default; likewise `lastpx`/`lastqty` (last executed price/quantity), `avgpx`, `cumqty`, `leavesqty`, `prevpx`/`prevqty` (prior step's settlement), `spotrate`/`forwardpoints` (FX parts) |
| Currency, unit | `get_currency`/`set_currency`: [`Ccy::none()`](../types/codes/ccy.md) if unstated; `get_unit`/`set_unit`: [`Unit::none()`](../types/codes/unit.md) if unstated |
| Side | `get_side`/`set_side`: the [side](../types/enum/side.md) by value, never absent - `Side::Unknown` (code `0`) where none is stated, which means "not stated": nothing invents a side; an order's, a quote's or an execution's stored cross code states its code ([below](#sides-and-cross-codes)) |
| Kind | `marketdatakind()`: required - the [category](../types/enum/marketdatakind.md) the element is filed under and a [lifecycle](event.md#lifecycle-walk) chains within (a leaf answers its own kind, a [FIX message](../fix/message.md#market-data) its `msgcat`) |
| Sided | `is_sided()`: provided as `marketdatakind().is_sided()` - whether the element's stored cross code states its side, true exactly for an order, a quote or an execution - any other kind states `0` there: [`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches), the one owner of the rule |
| Execution clock | `get_execunix`/`set_execunix` (`Option<i64>`): when the element last executed - the latest execution clock its lifecycle reached, nanoseconds since the Unix epoch, UTC, `None` where unknown; a market fact, not an event's: an undated order, quote or execution states one, a [text line](../media/index.md#plain-text) none, and no digest feeds it. The `execunix` column is a nullable nanosecond UTC clock ([Market data](market-data.md#columns)) |
| Bid and ask | `bidpx`, `bidqty`, `bidccy`, `askpx`, `askqty`, `askccy` ([below](#bid-and-ask)) |
| FX rates | `get_fxrates`/`set_fxrates`/`insert_fxrate` ([below](#fx-rates)) |
| Instrument | the security's [identifiers](#security-identifiers), `securityids`; `get_isincode`: their `isin`, borrowed; `get_cficode`/`get_miccode` + setters - a leaf keeps a [CFI](../types/codes/cfi.md) only when it is detailed (one of positions 3-6 not `X`); `get_ticker`/`set_ticker`: an informal name, apart from the codes |
| Metadata | `get_metadata`/`set_metadata`: a `BTreeMap<SmolStr, SmolStr>` of source facts no typed column reads, keyed by name/[path](../types/paths.md); never identifier/typed; `None`/empty alike; a FIX leaf's = [`FixMsg::market_data`](../fix/message.md#market-data)'s; a follower takes the chain's keys it lacks ([below](#following-and-merging)) |
| `fill_market` | provided, idempotent, called by `finalize` pre-digest: never invents price/quantity/`cumqty`/`leavesqty`/bid/ask/rates; [derives](#security-identifiers) the national id a canonical ISIN embeds |
| `digest_market` | provided (`Self: Element`): extends [`Element::digest`](element.md#contract) - price, currency, quantity, unit, side, security identifiers (each fed as source, type and value under the label `securityids`), classification, market, last-trade/avg/progress/FX parts, bid/ask (each only if stated), FX rates (only if any), ticker, metadata (key order); excludes `prevpx`/`prevqty`, like the predecessor's instant/identity |
| Provided on events | where `Self: Event`: `digest_market_event`, `following_market`, `merging_market_event` ([below](#following-and-merging)) |

## Setting: fill or overwrite

Every `Market` and `Operation` setter takes a trailing `overwrite: bool`; the `insert_`/`remove_`/`derive_` identifier verbs are the exceptions.

| `overwrite` | Effect |
| --- | --- |
| `true` | states the value whatever the element held; `None` clears the fact |
| `false` | fills: the value lands only where the element states nothing yet - `None`, `Side::Unknown`, `Ccy::none()`, `Unit::none()`, `MarketDataType::Unknown` - and a map fills only the keys it lacks |
| either | a value equal to the held one changes nothing, and moves nothing |

A change carries what it implies. A source *moves* a fact it implies where the element states nothing there or still holds what the source was, and never a fact stated apart from it; a fact that says something back about its source only *fills* the source where the element states none. Each redirection writes its target directly, never through that target's setter, so one change runs once and no chain of them loops.

| Changed | Moves | Fills back |
| --- | --- | --- |
| price, quantity | a buyer's `bidpx`/`bidqty`, a seller's `askpx`/`askqty` | |
| side | the side it left stops quoting the price and quantity, the side it takes quotes them; the side of a [sided](#sides-and-cross-codes) kind's stored cross code | the price and quantity, from what the new side quotes |
| currency | `bidccy`/`askccy` of a bid or ask the element states | |
| quantity, `displayqty` | `hiddenqty`, the quantity past the shown part | |
| `bidpx`/`bidqty`, `askpx`/`askqty` | | the price and quantity of an element taking that side |
| `hiddenqty` | | `displayqty`, the quantity less it - or the quantity, the two parts together |
| `lastpx`, `spotrate`, `forwardpoints` | | the third, where two are stated: `lastpx` is spot plus points |
| `leavesqty` | the quantity: what is still open is what the element is about | |
| `cumqty`, `lastqty`, `lastpx` | | `avgpx`, the last price, where all that traded is the last fill |
| an operation's `ordqty`, `cumqty`, `leavesqty` and its state | | [by the state](#order-quantities-by-state) |

`ordqty` never fills the quantity: an order is about what is left of it, not what it once asked for.

### Order quantities by state

FIX's `LeavesQty(151) = OrderQty(38) - CumQty(14)` while an order works, and nothing left once it no longer does. `set_state` stamps where the element stands, and the three quantities fill one another against it:

| State | Fills |
| --- | --- |
| fresh - pending or `NEW` - with nothing traded | `leavesqty` = `ordqty` |
| working - partially filled, replaced, any other live state | any two of `ordqty`, `cumqty`, `leavesqty` give the third; `leavesqty` moves off `ordqty` once something traded |
| `FILLED` | `leavesqty` 0 and `cumqty` = `ordqty`, where either is stated |
| `CANCELED`, `DONE_FOR_DAY`, `EXPIRED` | `leavesqty` 0 and `cxlqty` = `ordqty` - `cumqty` |
| `CALCULATED`, `REJECTED` | `leavesqty` 0 |

An execution or a trade reports a fill rather than an order, so it reads as working whatever its state: a fill stating only its `lastqty` states nothing left.

Along a chain, an iceberg's hidden part carries into a follower that states none: the predecessor's `hiddenqty` less what traded since - the rise in `cumqty`, else `lastqty` - never below zero ([Following and merging](#following-and-merging)).

Every fill is part of the element, so it is a column of the [`marketdata` row](schemas.md#the-marketdata-row) and of the [FIX row](schemas.md#the-fix-row): a row read back through an Arrow reader answers the same facts without filling them again.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, Operation, OrderEvent};
    use yggdryl::{Decimal, Side, State};

    let dec = |text: &str| text.parse::<Decimal>().expect("a decimal");

    let mut order = OrderEvent::at(1);
    order.set_crosscode("ORD-1".to_owned());
    order.set_price(Some(dec("101.5")), true);
    order.set_side(Side::Buy, false);
    // A buyer bids its price, under its side's cross code.
    assert_eq!(order.get_bidpx(), Some(dec("101.5")));
    assert_eq!(order.get_crosscode(), "10:1:ORD-1");

    // Without `overwrite` a stated fact stands; with it, it moves the bid too.
    order.set_price(Some(dec("99")), false);
    assert_eq!(order.get_price(), Some(dec("101.5")));
    order.set_price(Some(dec("102")), true);
    assert_eq!(order.get_bidpx(), Some(dec("102")));

    // Working: ordered less traded is left, and what is left is the quantity.
    order.set_state(State::PartiallyFilled);
    order.set_ordqty(Some(dec("100")), true);
    order.set_cumqty(Some(dec("40")), true);
    assert_eq!(order.get_leavesqty(), Some(dec("60")));
    assert_eq!(order.get_quantity(), Some(dec("60")));
    assert_eq!(order.get_bidqty(), Some(dec("60")));

    // Canceled: nothing left, and the rest was canceled.
    let mut canceled = OrderEvent::at(2);
    canceled.set_ordqty(Some(dec("10")), true);
    canceled.set_cumqty(Some(dec("4")), true);
    canceled.set_state(State::Canceled);
    assert_eq!(canceled.get_leavesqty(), Some(dec("0")));
    assert_eq!(canceled.get_cxlqty(), Some(dec("6")));
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import graph

    order = graph.OrderEvent(
        1, crosscode="O-1", side="BUYS", state="PARTIALLY_FILLED",
        price="10.5", ordqty="100", cumqty="40",
    )
    assert order.leavesqty.as_py() == Decimal("60")
    assert order.quantity.as_py() == Decimal("60")
    assert (order.bidpx.as_py(), order.bidqty.as_py()) == (Decimal("10.5"), Decimal("60"))
    assert order.crosscode == "10:1:O-1"

    canceled = graph.OrderEvent(1, crosscode="O-2", state="CANCELED", ordqty="10", cumqty="4")
    assert (canceled.leavesqty.as_py(), canceled.cxlqty.as_py()) == (Decimal("0"), Decimal("6"))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const order = new graph.OrderEvent(1n, {
      crosscode: 'O-1', side: 'BUYS', state: 'PARTIALLY_FILLED', price: '10.5', ordqty: '100', cumqty: '40',
    })
    assert.equal(order.leavesqty, '60')
    assert.equal(order.quantity, '60')
    assert.deepEqual([order.bidpx, order.bidqty], ['10.5', '60'])
    assert.equal(order.crosscode, '10:1:O-1')
    ```

## Sides and cross codes

| Key | Rule |
| --- | --- |
| `stored_crosscode(code)` | provided, the one speller of the prefix every element's cross code is stored under: `"{kind}:{side}:{code}"` - the [`MarketDataKind`](../types/enum/marketdatakind.md) code of the category the element is filed under, then the [`Side`](../types/enum/side.md) code of the side it takes where it is sided, `0` otherwise - so `10:1:O-1001` is an order to buy, `10:2:O-1001` an order to sell, `10:0:O-1001` an order stating no side, `14:1:Q-1` a quote, `8:2:E-1` an execution, `21:0:T-1` a trade, `3:0:XNAS:ESVUFR` a book; idempotent, a prefix of another kind or side replaced (`10:2:O-1001` read under a buy is `10:1:O-1001`), an empty code left empty |
| Stored | every holder the crate ships stores its cross code through it, so `set_crosscode`, `set_side` and the kind a holder stamps converge in any order; [`crosshashcode` and `crossuuid`](element.md#contract) follow the stored text, and the two sides of one identifier are two chains ([walk](event.md#lifecycle-walk)) |
| Unsided | an order, a quote or an execution stating no side states `0`, and so does every other element whatever side it takes - a trade, a book, a snapshot control, a FIX message filed under any other category: `21:0:T-1`, `3:0:AAPL`, whatever the side |
| Copied | a trade or a snapshot control built over a sided element - `TradeEvent::from_parts(&order, ..)`, `SnapshotEvent::snapshot(&order, ..)` - takes the base code under its own kind's prefix (`21:0:O-1001` from `10:1:O-1001`), with the cross hash and element of that stored code; a sided leaf built over such facts stores it under its side again |
| `book_crosscode()` | provided: the base of the book the element stands in - its ticker where it states one (borrowed), else `{miccode}:{cficode}` with `XXXX` for no market and `XXXXXX` for no classification (`XPAR:ESVUFR`, `XXXX:XXXXXX`); what [`BookIterator`](book.md#book-fold) keys books by, each stored as `3:0:{base}` |

## Bid and ask

| Key | Rule |
| --- | --- |
| Facts | `get_bidpx`/`get_bidqty`/`get_askpx`/`get_askqty` (`Option<Decimal>`) and `get_bidccy`/`get_askccy` (`Option<&Ccy>`), each with its `set_`: the bid and the ask the element states, `None` where unstated |
| Filled | from the element's own price, quantity and currency on the side it takes - a buy order at 189.50 bids 189.50 - and back: a bid or ask fills the price and quantity of an element taking that side ([Setting](#setting-fill-or-overwrite)); never from the last executed price |
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

`securityids` is an [`Identifiers`](identifier.md) map: the names the security goes by, one identifier per key `src:type`, each validated by its type where the crate knows it - `base:isin=US0378331005`, `derived:cusip=037833100`, `oms:instrumentid=dbi;CH0012214059_XSWX_CHF`.

| Verb | Rule |
| --- | --- |
| `get_securityids` | the map, sorted by its key `src:type`; `get(&IdType::Isin)` the value the wire stated (`fix`), else another named source's in key order, else `base`'s, else the derived one ([Lookups](identifier.md#contract)), `get_from(&IdSource::Fix, &IdType::Isin)` one source's |
| `get_isincode` | provided: `get_securityids().get(&IdType::Isin)`, borrowed - a projection of the set, never a second store; the `isincode` column writes it and a stated cell fills an absent `isin` from `base` ([Market data](market-data.md#arrow)) |
| `set_securityids(ids, overwrite)` | with `overwrite`, replaces the whole map, derived identifiers included, `Identifiers::new()` unsaying it; without, each identifier fills an absent key as `insert_securityid` does |
| `insert_securityid(id)` | fills an absent source and type, never a held one, and takes back a `derived` identifier of its type - a statement answers before a derivation; one from `derived` is a derivation; `ticker` is refused ([`IdType::check_security`](identifier.md#per-type-value-checks)); `true` if it added |
| `derive_securityid(kind, code)` | fills only a type the element holds none of, from `derived` - the code as `Identifier::new` stores it, a code its type refuses naming nothing - implication, not statement; never reaches a store the holder is a view of |
| `remove_securityid(src, kind)` | removes the identifier of one type from one source; removing an `isin` takes back every `derived` identifier, since each hangs on it |
| Building one | [`Identifier::new(src, kind, code)`](identifier.md#contract) checks the code by its type ([per-type checks](identifier.md#per-type-value-checks)) and holds a type the crate does not know as given; [`IdType::from_security_source`](identifier.md#vocabularies) reads FIX's `SecurityIDSource(22)` - `4`, `isin`, `ISINNumber` are `isin` - and refuses `ticker` |
| `forex` | the crate's own type, which FIX gives no source code: a [currency pair](../types/codes/forex.md) in any spelling `Forex::new` reads, stored canonical (`eurusd` is `EUR/USD`); `forexcode`, `ccypair` and `currencypair` read the type too |
| Derived | an ISIN's embedded national number - CUSIP (`US`/`CA`), SEDOL (`GB`/`IE`/`GG`/`JE`/`IM`, behind `00`), WKN (`DE`, behind `000`), Valor (`CH`/`LI`) ([`securityid::embedded`](../types/codes/isin.md)) - plus lifecycle-learned entries, each from `derived`; all hang on the `isin`, so removing or replacing it revokes them |
| From the ticker | a ticker of an identifier's own shape names it, read by its length before any check - 21 characters `{ISIN}_{MIC}_{CCY}` an instrument key (its ISIN derived, its market and currency filled where none is stated, a part its type refuses skipped), 12 a FIGI behind `BBG` else an ISIN, 9 a CUSIP, 7 a SEDOL, 6 upper-case letters a detailed CFI code, `AAPL.OQ` a RIC, `HOLN SW Equity` a Bloomberg identifier - each closing on its own type's check, so a ticker only the length of one names nothing; derived, never over a stated identifier of its type. `securityid::SymbolCode::from_symbol` is the reading, Rust only |
| Unit of a pair | an element trading a currency pair (a `forex` identifier) and stating no unit states its quantity in the currency dealt: its currency where that is a leg of the pair, else the pair's base (`EUR` for `EUR/USD`) |
| Provenance | a derived identifier is one from `derived`: a row carries it, and equality and the digest read it as they read a stated one |
| Refusals | `Identifier::new` refuses a word that is none, a value that states nothing and a code its type does not check; `insert_securityid` and `IdType::from_security_source` refuse `ticker` (use `set_ticker`) |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_market` | [`Event::following`](event.md#following); `prevpx`/`prevqty`, currency, unit, side (this element's where it states one, the chain's where it states `UNKN`), ticker, each security id it lacks, classification, market, and every metadata key it lacks - all from the predecessor where this event says nothing, this element's own values standing; always leads, even with the timed link unchanged |
| A FIX message | follows the metadata too, as a [`FixMsg`](../fix/message.md) in the [lifecycle](../fix/lifecycle.md): its metadata is the bridge's namespaced keys its row's `metadata` column holds, so a followed message's row carries the chain's keys |
| Identity | a follower's `currhashcode` and `curruuid` digest what it takes ([`digest_market`](#contract) feeds the metadata), so they move where it took a key |
| Two instruments | two stated ISINs that differ name two instruments: nothing is taken from the predecessor, and a merge keeps the leading statement's identifiers whole; a `ZZ` ISIN names no country's instrument, so it is never the other one - it yields to the real ISIN, which replaces it and everything derived under it |
| Execution clock | a market event whose state reports an execution ([`is_execution`](event.md#contract)) and states no `execunix` is dated from its own instant first - before it names a predecessor, so an inherited state is never read as its own execution; following then keeps the later of its own clock and its predecessor's, so a delayed report cannot regress it, and a non-execution carries the chain's latest |
| Restating | a market event's [`restating`](event.md#restating) also takes the market's place: `prevpx`/`prevqty`, what the chain is about where this reading said nothing - the metadata keys included, as in following - and the execution clock - the earliest of the two statements' |
| `merging_market` | where `Self: Element`, no event clocks: `self` leads, and the execution clock is the earliest either states |
| `merging_market_event` | [`Event::merging`](event.md#merging)'s reference leads: price/quantity/unit stand; optional numbers, bid/ask facts, ticker = reference's if stated else other's; the execution clock the earliest either statement knows (an unstamped execution observation dates itself first); metadata and FX rates unioned, reference-led; currency/side/classification/market = the better - an unknown yields to a known one (an `XXXX` [MIC](../types/codes/mic.md), the [CFI](../types/codes/cfi.md) merge); security ids: the reference's replace the other's of the same source and type, the other's fill the sources and types it lacks |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Examples

### The facts of an order

=== "Rust"

    ```rust
    use smol_str::SmolStr;
    use yggdryl::graph::{Element, Market, Metadata, OrderEvent};
    use yggdryl::{Ccy, Cfi, Decimal, IdSource, IdType, Identifier, Mic, Side};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_crosscode("O-1001".to_owned());
    order.set_side(Side::Buy, true);
    order.set_price(Some("189.50".parse()?), true);
    order.set_quantity(Some(Decimal::from_int(100)), true);
    order.set_currency(Ccy::new("USD")?, true);
    order.set_ticker(Some("AAPL".into()), true);
    order.set_miccode(Some(Mic::new("XNAS")?), true);
    order.set_cficode(Some(Cfi::new("ESVUFR")?), true);
    // `4` is FIX's SecurityIDSource(22) code for an ISIN.
    let isin = IdType::from_security_source("4")?;
    order.insert_securityid(Identifier::new(IdSource::Base, isin, "US0378331005")?)?;
    order.set_metadata(Some(Metadata::from([(SmolStr::new("ordtype"), SmolStr::new("2"))])), true);
    order.finalize();

    assert_eq!(order.get_price(), Some("189.5".parse()?));
    assert_eq!(order.get_currency().as_str(), "USD");
    assert_eq!(order.get_side(), Side::Buy);
    assert!(order.get_unit().is_none(), "unstated");
    // A price is what the element states, never a last executed price; a
    // buyer's price is its bid.
    assert_eq!((order.get_lastpx(), order.get_bidpx()), (None, Some("189.5".parse()?)));
    // The ISIN is stated under its type, and carries a CUSIP the crate derived.
    assert_eq!(order.get_securityids().get(&IdType::Isin), Some("US0378331005"));
    assert_eq!(order.get_isincode(), Some("US0378331005"));
    assert_eq!(order.get_securityids().get_from(&IdSource::Derived, &IdType::Cusip), Some("037833100"));
    assert_eq!(order.get_metadata().get("ordtype").map(SmolStr::as_str), Some("2"));
    // The ticker names the book it stands in.
    assert_eq!(order.book_crosscode(), "AAPL");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Identifier, Side, graph

    order = graph.OrderEvent(
        1_700_000_000_000_000_000,
        crosscode="O-1001",
        side="BUYS",
        price=Decimal("189.50"),
        quantity=100,
        currency="USD",
        ticker="AAPL",
        miccode="XNAS",
        cficode="ESVUFR",
        securityids=[Identifier("base", "isin", "US0378331005")],
        metadata={"ordtype": "2"},
    )

    assert order.price is not None and order.price.as_py() == Decimal("189.50")
    assert order.currency.as_py() == "USD"
    assert order.side is Side.BUYS
    assert order.unit == "", "unstated"
    # A price is what the element states, never a last executed price; a
    # buyer's price is its bid.
    assert order.lastpx is None
    assert order.bidpx is not None and order.bidpx.as_py() == Decimal("189.50")
    # The ISIN carries a CUSIP the crate derived.
    assert [str(id) for id in order.securityids] == ["base:isin=US0378331005", "derived:cusip=037833100"]
    assert order.isincode == "US0378331005"
    assert order.miccode is not None and order.miccode.as_py() == "XNAS"
    assert order.metadata == {"ordtype": "2"}
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
      miccode: 'XNAS',
      cficode: 'ESVUFR',
      securityids: [new Identifier('base', 'isin', 'US0378331005')],
      metadata: { ordtype: '2' },
    })

    assert.equal(order.price, '189.5')
    assert.equal(order.currency, 'USD')
    assert.equal(order.side, 'BUYS')
    assert.equal(order.unit, '', 'unstated')
    // A price is what the element states, never a last executed price; a
    // buyer's price is its bid.
    assert.equal(order.lastpx, null)
    assert.equal(order.bidpx, '189.5')
    // The ISIN carries a CUSIP the crate derived.
    assert.equal(order.securityids.toString(), '[base:isin=US0378331005, derived:cusip=037833100]')
    assert.equal(order.isincode, 'US0378331005')
    assert.equal(order.miccode, 'XNAS')
    assert.deepEqual(order.metadata, { ordtype: '2' })
    ```

### Sides and cross codes

One identifier, `O-1001`, on each side of the market: two cross codes, two chains. A trade built on the buy order is not sided: it takes the order's base code under its own kind, `21:0:O-1001`.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, ExecutionEvent, Market, OrderEvent, TradeEvent};
    use yggdryl::{Cfi, Mic, Side};

    let order = |side: Side| {
        let mut order = OrderEvent::at(1_700_000_000_000_000_000);
        order.set_crosscode("O-1001".to_owned());
        order.set_side(side, true);
        order.finalize();
        order
    };
    let (buy, sell) = (order(Side::Buy), order(Side::Sell));
    assert_eq!((buy.get_crosscode(), sell.get_crosscode()), ("10:1:O-1001", "10:2:O-1001"));
    assert_ne!(buy.get_crossuuid(), sell.get_crossuuid());
    // The prefix is spelled in one place, and a side taken later restates it.
    assert_eq!(buy.stored_crosscode("10:2:O-1001"), "10:1:O-1001");
    let mut flipped = buy.clone();
    flipped.set_side(Side::Sell, true);
    assert_eq!(flipped.get_crosscode(), "10:2:O-1001");
    // An order stating no side states 0.
    assert_eq!(order(Side::Unknown).get_crosscode(), "10:0:O-1001");

    // Only an order, a quote or an execution is sided: a book and a trade
    // state side 0 whatever side they take, and a trade built on the buy
    // order takes its base code under its own kind.
    let mut book = BookEvent::new(1_700_000_000_000_000_000, "AAPL");
    book.set_side(Side::Buy, true);
    assert!(buy.is_sided() && !book.is_sided());
    assert_eq!(book.get_crosscode(), "3:0:AAPL");
    let mut fill = ExecutionEvent::at(1_700_000_000_000_000_000);
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::Sell, true);
    let trade = TradeEvent::from_parts(&buy, vec![fill])?;
    assert_eq!((trade.get_crosscode(), trade.get_side()), ("21:0:O-1001", Side::Buy));
    assert_eq!(trade.executions()[0].get_crosscode(), "8:2:E-1");

    // With no ticker, the book an element stands in is its market and class.
    let mut unnamed = order(Side::Buy);
    unnamed.set_miccode(Some(Mic::new("XPAR")?), true);
    unnamed.set_cficode(Some(Cfi::new("ESVUFR")?), true);
    assert_eq!(unnamed.book_crosscode(), "XPAR:ESVUFR");
    assert_eq!(order(Side::Buy).book_crosscode(), "XXXX:XXXXXX");
    ```

=== "Python"

    ```python
    from yggdryl import Side, graph

    buy = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side="BUYS")
    sell = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side=Side.SELL)
    assert (buy.crosscode, sell.crosscode) == ("10:1:O-1001", "10:2:O-1001")
    assert buy.crossuuid != sell.crossuuid
    # An order stating no side reads UNKN and states 0.
    unsided = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    assert unsided.crosscode == "10:0:O-1001" and unsided.side is Side.UNKN

    # A trade is not sided: built on the buy order, it takes the order's base
    # code under its own kind whatever side it states, and each execution its
    # own sided one.
    fill = graph.ExecutionEvent(1_700_000_000_000_000_000, crosscode="E-1", side="SELL")
    trade = graph.TradeEvent.from_parts(buy, [fill])
    assert (trade.crosscode, trade.side) == ("21:0:O-1001", Side.BUYS)
    assert trade.executions[0].crosscode == "8:2:E-1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Side, graph } = require('yggdryl')

    const buy = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'BUYS' })
    const sell = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001', side: 'SELL' })
    // A side crosses as its member's name; `Side` maps each name to its code.
    assert.equal(Side[sell.side], 2)
    assert.deepEqual([buy.crosscode, sell.crosscode], ['10:1:O-1001', '10:2:O-1001'])
    assert.notEqual(buy.crossuuid, sell.crossuuid)
    // An order stating no side reads UNKN and states 0.
    const unsided = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    assert.equal(unsided.crosscode, '10:0:O-1001')
    assert.equal(unsided.side, 'UNKN')

    // A trade is not sided: built on the buy order, it takes the order's base
    // code under its own kind whatever side it states, and each execution its
    // own sided one.
    const fill = new graph.ExecutionEvent(1_700_000_000_000_000_000n, { crosscode: 'E-1', side: 'SELL' })
    const trade = graph.TradeEvent.fromParts(buy, [fill])
    assert.deepEqual([trade.crosscode, trade.side], ['21:0:O-1001', 'BUYS'])
    assert.equal(trade.executions[0].crosscode, '8:2:E-1')
    ```

`is_sided`, `stored_crosscode` and `book_crosscode` are Rust-only; a binding reads the stored `crosscode`.

### Bid, ask and FX rates

A two-sided EUR/USD quote in dollars, and the rate a dollar amount is divided by to state it in euros.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Market, QuoteEvent};
    use yggdryl::{Ccy, IdSource, IdType, Identifier, Side};

    let mut quote = QuoteEvent::at(1_700_000_000_000_000_000);
    quote.set_crosscode("Q-7".to_owned());
    quote.set_currency(Ccy::new("USD")?, true);
    quote.insert_securityid(Identifier::new(IdSource::Base, IdType::Forex, "eurusd")?)?;
    quote.set_bidpx(Some("1.0842".parse()?), true);
    quote.set_askpx(Some("1.0844".parse()?), true);
    quote.set_bidccy(Some(Ccy::new("USD")?), true);
    quote.set_askccy(Some(Ccy::new("USD")?), true);
    quote.set_price(Some("125".parse()?), true);
    assert!(quote.insert_fxrate(Ccy::new("EUR")?, "1.25".parse()?));
    assert!(!quote.insert_fxrate(Ccy::new("EUR")?, "2".parse()?), "a stated rate stands");
    quote.finalize();

    // The pair lands canonical under the crate's own type.
    assert_eq!(quote.get_securityids().to_string(), "[base:forex=EUR/USD]");
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

    from yggdryl import Identifier, Side, graph

    quote = graph.QuoteEvent(
        1_700_000_000_000_000_000,
        crosscode="Q-7",
        currency="USD",
        securityids=[Identifier("base", "forex", "eurusd")],
        bidpx=Decimal("1.0842"),
        askpx=Decimal("1.0844"),
        bidccy="USD",
        askccy="USD",
        price=Decimal("125"),
        fxrates={"EUR": Decimal("1.25")},
    )

    # The pair lands canonical under the crate's own type.
    assert str(quote.securityids) == "[base:forex=EUR/USD]"
    # Two prices and no side: a bid and an ask are facts, not a side.
    assert quote.side is Side.UNKN
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
    const { Identifier, graph } = require('yggdryl')

    const quote = new graph.QuoteEvent(1_700_000_000_000_000_000n, {
      crosscode: 'Q-7',
      currency: 'USD',
      securityids: [new Identifier('base', 'forex', 'eurusd')],
      bidpx: '1.0842',
      askpx: '1.0844',
      bidccy: 'USD',
      askccy: 'USD',
      price: '125',
      fxrates: { EUR: '1.25' },
    })

    // The pair lands canonical under the crate's own type.
    assert.equal(quote.securityids.toString(), '[base:forex=EUR/USD]')
    // Two prices and no side: a bid and an ask are facts, not a side.
    assert.equal(quote.side, 'UNKN')
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
    resting.set_execunix(Some(T), true);
    assert_eq!(resting.get_execunix(), Some(T));

    let mut placed = OrderEvent::at(T);
    placed.set_crosscode("O-1001".to_owned());
    placed.set_execunix(Some(T - 1_000_000), true);
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
    delayed.set_execunix(Some(T - 5_000_000), true);
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
use yggdryl::{IdSource, IdType, Identifier};

let isin = |code: &str| Identifier::new(IdSource::Base, IdType::Isin, code);
let ids = |order: &OrderEvent| -> Vec<String> {
    order.get_securityids().iter().map(ToString::to_string).collect()
};
let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.insert_securityid(isin("US0378331005")?)?;
order.finalize();
assert_eq!(ids(&order), ["base:isin=US0378331005", "derived:cusip=037833100"]);

// A derivation fills only a type the element holds none of, and a stated
// identifier stands.
assert!(!order.derive_securityid(&IdType::Cusip, "594918104"));
assert!(!order.insert_securityid(isin("US5949181045")?)?);

// Everything derived hangs on the isin: removing it takes the cusip back.
let mut unlisted = order.clone();
assert!(unlisted.remove_securityid(&IdSource::Base, &IdType::Isin)?);
assert!(unlisted.get_securityids().is_empty());
assert_eq!(unlisted.get_isincode(), None);

// A stated identifier takes back a derived one of its type, and then
// outlives the ISIN.
assert!(order.insert_securityid(Identifier::new(IdSource::Base, IdType::Cusip, "037833100")?)?);
assert!(order.remove_securityid(&IdSource::Base, &IdType::Isin)?);
assert_eq!(ids(&order), ["base:cusip=037833100"]);

// A currency pair is refused where it names no pair.
assert!(Identifier::new(IdSource::Base, IdType::Forex, "EUR/EUR").is_err());
```

### Identifiers along a chain

An amendment to an Apple order, and one naming Microsoft's ISIN instead.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, OrderEvent};
    use yggdryl::{IdSource, IdType, Identifier, Identifiers};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, stated: &[(IdType, &str)]| -> yggdryl::Result<OrderEvent> {
        let ids = stated
            .iter()
            .map(|(kind, code)| Identifier::new(IdSource::Base, kind.clone(), code))
            .collect::<yggdryl::Result<Identifiers>>()?;
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1001".to_owned());
        order.set_securityids(ids, true)?;
        order.finalize();
        Ok(order)
    };
    let keys = |order: &OrderEvent| -> Vec<String> {
        order.get_securityids().iter().map(ToString::to_string).collect()
    };
    let placed = order(T, &[(IdType::Isin, "US0378331005"), (IdType::Figi, "BBG000B9XRY4")])?;
    assert_eq!(keys(&placed), ["base:figi=BBG000B9XRY4", "base:isin=US0378331005", "derived:cusip=037833100"]);

    // A follower stating none takes the chain's whole.
    let amended = order(T + 1, &[])?.with_previous(&placed).expect("a later event follows");
    assert_eq!(amended.get_securityids(), placed.get_securityids());
    let other = order(T + 1, &[(IdType::Isin, "US5949181045")])?.with_previous(&placed).expect("it follows");
    assert_eq!(keys(&other), ["base:isin=US5949181045", "derived:cusip=594918104"]);

    // Two statements naming different ISINs never mix: the leading
    // statement - here the later recording - keeps its identifiers whole.
    let mut restated = placed.clone();
    restated.set_recdunix(Some(T + 5));
    restated.set_securityids(other.get_securityids().clone(), true)?;
    let merged = placed.clone().merge_with(&restated).expect("the later recording leads");
    assert_eq!(keys(&merged), ["base:isin=US5949181045", "derived:cusip=594918104"]);
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    T = 1_700_000_000_000_000_000


    def order(unix: int, stated: dict[str, str]) -> graph.OrderEvent:
        securityids = [Identifier("base", kind, code) for kind, code in stated.items()]
        return graph.OrderEvent(unix, crosscode="O-1001", securityids=securityids)


    def keys(event: graph.OrderEvent) -> list[str]:
        return [str(id) for id in event.securityids]


    placed = order(T, {"figi": "BBG000B9XRY4", "isin": "US0378331005"})
    assert keys(placed) == ["base:figi=BBG000B9XRY4", "base:isin=US0378331005", "derived:cusip=037833100"]

    # A follower stating none takes the chain's whole.
    amended = order(T + 1, {}).with_previous(placed)
    assert amended is not None and amended.securityids == placed.securityids
    other = order(T + 1, {"isin": "US5949181045"}).with_previous(placed)
    assert other is not None and keys(other) == ["base:isin=US5949181045", "derived:cusip=594918104"]
    assert other.isincode == "US5949181045"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, stated) => new graph.OrderEvent(unix, {
      crosscode: 'O-1001',
      securityids: Object.entries(stated).map(([kind, code]) => new Identifier('base', kind, code)),
    })

    const placed = order(T, { figi: 'BBG000B9XRY4', isin: 'US0378331005' })
    assert.equal(placed.securityids.toString(), '[base:figi=BBG000B9XRY4, base:isin=US0378331005, derived:cusip=037833100]')

    // A follower stating none takes the chain's whole.
    const amended = order(T + 1n, {}).withPrevious(placed)
    assert.ok(amended.securityids.equals(placed.securityids))
    const other = order(T + 1n, { isin: 'US5949181045' }).withPrevious(placed)
    assert.equal(other.securityids.toString(), '[base:isin=US5949181045, derived:cusip=594918104]')
    assert.equal(other.isincode, 'US5949181045')
    ```

### Metadata along a chain

An amendment states its own `desk` and leaves the `venue` the placement stated to the chain.

=== "Rust"

    ```rust
    use smol_str::SmolStr;
    use yggdryl::graph::{Element, Market, Metadata, OrderEvent};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, stated: &[(&str, &str)]| {
        let mut order = OrderEvent::at(unix);
        order.set_crosscode("O-1001".to_owned());
        let metadata: Metadata = stated
            .iter()
            .map(|&(key, value)| (SmolStr::new(key), SmolStr::new(value)))
            .collect();
        order.set_metadata(Some(metadata), true);
        order.finalize();
        order
    };
    let placed = order(T, &[("desk", "EQ"), ("venue", "XPAR")]);
    let amended = order(T + 1, &[("desk", "FX")]).with_previous(&placed).expect("a later event follows");

    // Its own value stands, and the key it did not state is the chain's.
    let metadata: Vec<(&str, &str)> = amended
        .get_metadata()
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    assert_eq!(metadata, [("desk", "FX"), ("venue", "XPAR")]);
    ```

=== "Python"

    ```python
    from yggdryl import graph

    T = 1_700_000_000_000_000_000
    placed = graph.OrderEvent(T, crosscode="O-1001", metadata={"desk": "EQ", "venue": "XPAR"})
    amended = graph.OrderEvent(T + 1, crosscode="O-1001", metadata={"desk": "FX"}).with_previous(placed)

    # Its own value stands, and the key it did not state is the chain's.
    assert amended is not None and amended.metadata == {"desk": "FX", "venue": "XPAR"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const placed = new graph.OrderEvent(T, { crosscode: 'O-1001', metadata: { desk: 'EQ', venue: 'XPAR' } })
    const amended = new graph.OrderEvent(T + 1n, { crosscode: 'O-1001', metadata: { desk: 'FX' } }).withPrevious(placed)

    // Its own value stands, and the key it did not state is the chain's.
    assert.deepEqual(amended.metadata, { desk: 'FX', venue: 'XPAR' })
    ```

## Edges

- On an order, a quote or an execution, `set_side(Side::Unknown)` after a sided code keeps the prefix the code has: a code is re-prefixed only under a known side. On any other element `set_side` never touches the code.
- A ticker stated empty is none: `book_crosscode` falls back to the market and class.
- A zero rate is a statement like any other; dividing by it is the caller's refusal to make - `Decimal::checked_div` answers `None`.
