# Market

`Market` states thirty-six facts: the kind and type of the element, the instrument and an option's strike, the side, the price and quantity - shown, hidden and stopped at -, the bid and the ask, what has traded and when it last did, the currency the instrument was issued in, the rates to other currencies and free-form metadata.

## Contract

| Key | Rule |
| --- | --- |
| Owner | trait `yggdryl::graph::Market` (`graph::market`), no supertrait; Rust-only - [leaves](index.md#leaves) answer it in Python/JavaScript |
| Price, quantity, numbers | `get_price`/`get_quantity` + `set_`: what the element states, exact as [`Decimal`](../types/numeric/decimal.md#decimal), `None` if none - never last-executed, never a default; likewise `lastpx`/`lastqty` (last executed price/quantity), `avgpx`, `cumqty`, `leavesqty`, `prevpx`/`prevqty` (prior step's settlement), `spotrate`/`forwardpoints` (FX parts), `stoppx` (the price a stop order triggers at) and `strikepx` (the strike price of the option the element is about - an instrument fact, which a follower of the same instrument [takes along its chain](#following-and-merging); a FIX message's `StrikePrice(202)`) |
| Currency, unit | `get_currency`/`set_currency`: [`Ccy::none()`](../types/codes/ccy.md) if unstated; `get_unit`/`set_unit`: [`Unit::none()`](../types/codes/unit.md) if unstated; `get_origccy`/`set_origccy`: the currency the instrument was issued in, `Ccy::none()` where neither the element nor a registry stated one, and `origin_currency()` that or else the currency ([below](#origin-currency)) |
| Side | `get_side`/`set_side`: the [side](../types/enum/side.md) by value, never absent - `Side::Unknown` (code `0`) where none is stated, which means "not stated": nothing invents a side. An order's or an execution's side is the one side it takes, and its stored cross code states its code ([below](#sides-and-cross-codes)); any other element's is a tag - a [quote](#a-quotes-two-legs) holds its bid and its ask and tags the leg it states, a two-sided one `Side::Both` (`BOTH`, code `99`), and a [book](book.md) is always `BOTH` |
| Kind | `marketdatakind()`: required - the [category](../types/enum/marketdatakind.md) the element is filed under and a [lifecycle](event.md#lifecycle-walk) chains within (a leaf answers its own kind, a [FIX message](../fix/message.md#market-data) the category its dictionary files its type under) |
| Sided | `is_sided()`: provided as `marketdatakind().is_sided()` - whether the element's stored cross code states its side, true exactly for an order or an execution - any other kind, a quote among them, states `0` there: [`MarketDataKind::is_sided`](../types/enum/marketdatakind.md#sided-kinds-and-batches), the one owner of the rule |
| Execution clock | `get_execunix`/`set_execunix` (`Option<i64>`): when the element last executed - the latest execution clock its lifecycle reached, nanoseconds since the Unix epoch, UTC, `None` where unknown; a market fact, not an event's: an undated order, quote or execution states one, a [text line](../media/text.md) none, and no digest feeds it. The `execunix` column is a nullable nanosecond UTC clock ([Market data](market-data.md#columns)) |
| Bid and ask | `bidpx`, `bidqty`, `bidccy`, `askpx`, `askqty`, `askccy` ([below](#bid-and-ask)) |
| FX rates | `get_fxrates`/`set_fxrates`/`insert_fxrate` ([below](#fx-rates)) |
| Instrument | the security's [identifiers](#security-identifiers), `securityids`; `get_isincode`: their `isin`, borrowed; `get_cficode`/`get_miccode` + setters - a leaf keeps a [CFI](../types/codes/cfi.md) only when it is detailed (one of positions 3-6 not `X`): a coarse code states nothing, neither filling nor clearing, and a detailed code stated over another describing the same instrument takes what that one says where it says nothing ([`Cfi::refined`](../types/codes/cfi.md#two-statements-of-one-instrument)); a market of `XXXX` is unstated, so a stated one fills over it; `get_ticker`/`set_ticker`: an informal name, apart from the codes; `book_crosscode`: the [book key](#the-book-key) |
| Metadata | `get_metadata`/`set_metadata`: a `BTreeMap<SmolStr, SmolStr>` of source facts no typed column reads, keyed by name/[path](../types/paths.md); never identifier/typed; `None`/empty alike; a FIX leaf's = [`FixMsg::market_data`](../fix/message.md#market-data)'s; a follower takes the chain's keys it lacks ([below](#following-and-merging)) |
| `fill_market` | provided, idempotent, called by `finalize` pre-digest: never invents price/quantity/`cumqty`/`leavesqty`/bid/ask/rates; [derives](#security-identifiers) the national id a canonical ISIN embeds |
| `digest_market` | provided (`Self: Element`): extends [`Element::digest`](element.md#contract) - price, currency, quantity, unit, side, security identifiers (each fed as source, type and value under the label `securityids`), classification, market, the stop and strike prices and the shown, hidden and cancelled quantities (each only if stated), last-trade/avg/progress/FX parts, bid/ask (each only if stated), FX rates (only if any), ticker, metadata (key order); excludes `prevpx`/`prevqty`, like the predecessor's instant/identity |
| Provided on events | where `Self: Event`: `digest_market_event`, `following_market`, `merging_market_event` ([below](#following-and-merging)) |

## Setting: fill or overwrite

Every `Market` and `Operation` setter takes a trailing `overwrite: bool`; the `insert_`/`remove_`/`derive_` identifier verbs are the exceptions.

| `overwrite` | Effect |
| --- | --- |
| `true` | states the value whatever the element held; `None` clears the fact |
| `false` | fills: the value lands only where the element states nothing yet - `None`, `Side::Unknown`, `Ccy::none()`, `Unit::none()`, `MarketDataType::Unknown` - ordinary maps fill only keys they lack; `Identifiers` maps replace lower-ranked values and keep equal-ranked held values |
| either | a value equal to the held one changes nothing, and moves nothing |

A change carries what it implies onto the facts that follow it. A source *moves* a fact it implies along with it - where the element states nothing there, or still holds what the source was - and never one stated apart from it; a fact that says something back about its source only *fills* the source where the element states none. Three properties hold for every rule below:

- **A statement stands.** Nothing a setter's fact implies writes that fact back: a fact stated - as none included - stands over every derivation, so a `leavesqty` stated as none stays none whatever `ordqty` and `cumqty` imply.
- **No recursion.** Each redirection writes its target directly, never through the target's setter, so one change runs once and no chain of them loops.
- **Any order.** Facts that do not contradict one another land alike in whatever order they are stated: an order's state stated before its quantities or after them lands the same quantities.

| Changed | Moves | Fills back |
| --- | --- | --- |
| price, quantity | the bid of a buyer, the ask of a seller: `bidpx`/`bidqty`, `askpx`/`askqty` | |
| side | a [sided](#sides-and-cross-codes) element's - an order's, an execution's: the side it left stops quoting the price and quantity, and its stored cross code moves under the new side. Any other's is a tag over the legs it holds, withdrawing none: the price and quantity move from the leg the old tag took to the leg the new one takes. Either way the side it takes quotes them | the price and quantity, from what the side quotes |
| currency | `bidccy`/`askccy` of a bid or ask the element states | |
| quantity, `displayqty`, `hiddenqty` | `hiddenqty`, the quantity past the shown part | the third of an iceberg's three where two are stated: `displayqty` the quantity less the hidden part, the quantity the two parts together |
| `bidpx`/`bidqty`, `askpx`/`askqty` | | the price and quantity of an element taking that side |
| a predecessor's `hiddenqty`, followed | a follower stating none: what it kept back less what traded since - the rise in `cumqty`, else `lastqty` - never below zero | |
| a predecessor's side, followed | a sided follower stating none takes it: the side is part of its identity | |
| a predecessor's `strikepx`, followed | a follower of the same instrument stating none takes it: the strike is the option's | |
| a predecessor's bid or ask, followed | an unsided follower tagging no side and stating neither the price nor the quantity of that leg: the leg whole, its currency with it ([A quote's two legs](#a-quotes-two-legs)) | |
| a predecessor's `ordqty`, `cumqty`, `avgpx`, followed | an operation's follower stating none of them: what its chain ordered, traded and at what average - never a last fill, which no rise in `cumqty` invents | |
| `lastpx`, `spotrate`, `forwardpoints` | | the third, where two are stated: `lastpx` is spot plus points |
| an operation's `ordqty`, `cumqty`, `leavesqty`, `cxlqty` and its state | | [by the state](#order-quantities-by-state) |
| `leavesqty` | the quantity of an order: what is still open is what it is about - never an execution's or a trade's, whose quantity is its own | |
| `cumqty`, `lastqty`, `lastpx` | | `avgpx`, the last price, where all that traded is the last, positive fill |
| `cficode` | | a detailed code over another describing one instrument, what that one says where it says nothing; a coarse code states nothing |

`ordqty` never fills the quantity: an order is about what is left of it, not what it once asked for.

### Order quantities by state

FIX's `LeavesQty(151) = OrderQty(38) - CumQty(14)` while an order works, and nothing left once it no longer does. `set_state` stamps where the element stands, read off the [state](../types/enum/state.md)'s own bands, and the four quantities fill one another against it - never the one a setter stated:

| State | Fills |
| --- | --- |
| unstated, `UNKNOWN` | nothing: an order whose state is unstated implies none of it until one is stated |
| fresh - the pending band, or acknowledged (`NEW`) - with nothing traded | `leavesqty` and `ordqty` each other |
| working - any other live state | any two of `ordqty`, `cumqty`, `leavesqty` give the third; `leavesqty` moves off `ordqty` once something traded |
| `FILLED` | `leavesqty` 0, and `cumqty` and `ordqty` each other |
| ended by someone or by the clock - the cancellation band, `DONE_FOR_DAY`, `EXPIRED` | `leavesqty` 0, `cxlqty` the rest of what was ordered (`ordqty` less `cumqty`), and the third of `ordqty`, `cumqty`, `cxlqty` |
| ended any other way - `CALCULATED`, `REJECTED`, a failure | `leavesqty` 0, and the third of `ordqty`, `cumqty`, `cxlqty` |

An order that ended has nothing left whatever else it states; any other element only where it states what it ordered or traded. An execution or a trade reports a fill rather than an order, so it reads as working whatever its state and its quantity is its own: a fill stating only its `lastqty` states nothing left, and an execution's `leavesqty` never becomes its quantity.

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
| `stored_crosscode(code)` | provided, the one speller of the prefix every element's cross code is stored under: `"{kind}:{side}:{code}"` - the [`MarketDataKind`](../types/enum/marketdatakind.md) code of the category the element is filed under, then the [`Side`](../types/enum/side.md) code of the side it takes where it is sided, `0` otherwise - so `10:1:O-1001` is an order to buy, `10:2:O-1001` an order to sell, `10:0:O-1001` an order stating no side, `8:2:E-1` an execution, `14:0:Q-1` a quote whatever leg it tags, `21:0:T-1` a trade, `3:0:CH0012214059` a book; idempotent, a prefix of another kind or side replaced (`10:2:O-1001` read under a buy is `10:1:O-1001`), an empty code left empty |
| Stored | every holder the crate ships stores its cross code through it, so `set_crosscode`, `set_side` and the kind a holder stamps converge in any order; [`crosshashcode` and `crossuuid`](element.md#contract) follow the stored text, and the two sides of one identifier are two chains ([walk](event.md#lifecycle-walk)) |
| Unsided | an order or an execution stating no side states `0`, and so does every other element whatever side it takes or tags - a quote, a trade, a book, a snapshot control, a FIX message filed under any other category: `14:0:Q-1`, `21:0:T-1`, `3:0:AAPL`, whatever the side, `BOTH` included |
| Copied | a trade or a snapshot control built over a sided element - `TradeEvent::from_parts(&order, ..)`, `SnapshotEvent::snapshot(&order, ..)` - takes the base code under its own kind's prefix (`21:0:O-1001` from `10:1:O-1001`), with the cross hash and element of that stored code; a sided leaf built over such facts stores it under its side again |
| Chained | a [lifecycle walk](event.md#names-re-keying-and-conflicts) states every element of a chain under the chain's side and stored cross code: a sided element stating no side takes the live statement's, quoting its price and quantity as that side's as any side does, then the live statement's stored code where that states one - [`Operation::follow_identity`](operation.md#following-and-merging), the side first because a code is stored under the side its holder takes - so an identifier change, a replace under a new `ClOrdID` or `OrderID`, moves no element onto another code: `crosshashcode` and `crossuuid` derive from the chain's code, and the [book key](#the-book-key) and the entry a book keys by `crossuuid` stay where they stood |

## The book key

`book_crosscode()` is provided, and is the key of the book the element stands in - what [`BookIterator`](book.md#book-fold) keys books by, each storing it as its cross code `3:0:{key}` - borrowed, so no input allocates:

| The element states | Its book key |
| --- | --- |
| an ISIN, as its `isin` security identifier, whatever its [rank](identifier.md#ranks) | the ISIN (`CH0012214059`): one book per instrument wherever an ISIN is known - a masked number keys a book too, since what a walk sees is what the lifecycle already corrected |
| no ISIN and a non-empty ticker | the ticker (`HOLN`); a ticker-only statement joins its instrument's book once the lifecycle's [registry](isin-registry.md) has learned the pair and filled the ISIN |
| neither | `XX0000000000`, [`Isin::NONE`](../types/codes/isin.md#the-check-digit-and-the-rank), the number that states none |

A chain stated by its ticker alone and then under its ISIN moves to the ISIN's book, [withdrawn](book.md#book-fold) from the ticker's. An ISIN-keyed book takes two listings' tickers, which stay apart inside it by their own partition.

## A quote's two legs

A quote is one element holding its bid and its ask - `bidpx`, `bidqty`, `bidccy` and `askpx`, `askqty`, `askccy` - and its side is a tag: a one-sided quote tags the leg it states, and a two-sided one tagging none states `BOTH` once finalized - a tag it states stands. It is not [sided](#sides-and-cross-codes), so its cross code is stored under side `0` (`14:0:Q-1`) and one identifier is one chain whatever leg a statement updates.

| Key | Rule |
| --- | --- |
| Price and quantity | the leg its tag takes: a quote tagging `BUYS` that states a price and a quantity quotes them as its bid, one tagging `SELL` as its ask; a quote tagging none, or `BOTH`, states its legs alone |
| Changing the tag | moves the price and quantity from the leg the old tag took to the leg the new one takes, withdrawing no leg |
| Following | a follower tagging no one leg - none, or `BOTH` - takes each leg of its chain it states neither the price nor the quantity of, whole - its currency with it - so a statement updating one leg keeps the other and an acknowledgement quoting nothing keeps the quote; a leg it states, a zero quantity withdrawing it included, is its own, and a leg its tag takes stated by a quantity alone keeps the chain's price. A tagged follower of a tagged one-leg entry - a book level - restates that entry whole, moving between sides included; one following a quote that holds both legs or tags no one leg - none, or `BOTH` - a fill reported on the leg that traded - keeps the other; a follower left holding both legs and tagging none states `BOTH`. No leg crosses to another instrument or another ticker |
| On a book | it rests on each leg it states a price or a quantity of, one entry on both sides where it states both ([Book](book.md#entries)) |
| From FIX | a quote message is one quote: `BidPx(132)`/`BidSize(134)` its bid, `OfferPx(133)`/`OfferSize(135)` its ask, never split by side ([Quote](quote.md)) |

## Bid and ask

| Key | Rule |
| --- | --- |
| Facts | `get_bidpx`/`get_bidqty`/`get_askpx`/`get_askqty` (`Option<Decimal>`) and `get_bidccy`/`get_askccy` (`Option<&Ccy>`), each with its `set_`: the bid and the ask the element states, `None` where unstated |
| Filled | from the element's own price, quantity and currency on the side it takes or tags - a buy order at 189.50 bids 189.50 - and back: a bid or ask fills the price and quantity of an element taking that side ([Setting](#setting-fill-or-overwrite)); never from the last executed price |
| Sources | a [book](book.md#books) states its best tradable levels, in its currency; a FIX message its `BidPx(132)`/`BidSize(134)` and `OfferPx(133)`/`OfferSize(135)`, each currency from a stated `BidCurrency`/`AskCurrency` (or `OfferCurrency`) field, else the message's ([FIX](../fix/message.md#market-data)) |
| Following, merging | following carries a quote's legs to a follower tagging no side ([A quote's two legs](#a-quotes-two-legs)) and none to a sided element; a merge takes each from the leading statement where it states one, else from the other |

## Origin currency

`origccy` is the currency the instrument was issued in - the one a depositary receipt or a share class listed in another currency trades apart from. It is held only where something stated it, and `origin_currency()` is the reading with its default: `origccy` where held, else `currency`. That default is applied when read, never stored, so a fill always finds an empty slot and nothing derived is learned back.

| Key | Rule |
| --- | --- |
| Held | `get_origccy`/`set_origccy(ccy, overwrite)`: `Ccy::none()` (`XXX`) until a row's cell, a caller's `set_origccy(.., true)` or the [registry's fill](isin-registry.md#matching) states one; the fill writes with `overwrite = false`, so a statement stands |
| Read | `origin_currency()`: `origccy` where held, else `currency` - never `XXX` where a currency is stated. It is the currency an amount converts *from*; [FX rates](#fx-rates) are where it converts *to*, and nothing converts yet |
| One direction | `currency` is never filled from `origccy`, nor `origccy` from `currency`; an FX pair reads its `currency` like any other element |
| Precedence | a statement, then the registry's stated instrument value, then `currency` at read |
| Following | a follower carries it as it carries every market fact it lacks ([below](#following-and-merging)) |
| Columns | `origccy` after `currency` in `MarketColumn::ALL`, `ccy`, null where unheld, on a [`marketdata` row](market-data.md#columns) and on a [FIX row](../fix/capture.md#the-crates-own-columns) (crate tag 65018); fed to the digest only where held, so an element stating none hashes as it did before the fact existed |
| Bindings | Python `origccy` (a `Scalar`, `None` where unheld) and `origin_currency` (`Scalar`); JavaScript `origccy` (a `string`, `null` where unheld) and `originCurrency` (`string`); the constructors take `origccy` like any other column |

=== "Rust"

    ```rust
    use yggdryl::Ccy;
    use yggdryl::graph::{Market, OrderEvent};

    let mut order = OrderEvent::at(1_700_000_000_000_000_000);
    order.set_currency(Ccy::new("EUR")?, true);
    // Unheld: the origin is the currency, read and never stored.
    assert!(order.get_origccy().is_none());
    assert_eq!(order.origin_currency().as_str(), "EUR");

    // A USD-issued share listed in EUR states its origin.
    order.set_origccy(Ccy::new("USD")?, true);
    assert_eq!(order.origin_currency().as_str(), "USD");
    assert_eq!(order.get_currency().as_str(), "EUR", "never filled from it");
    // A fill never displaces a statement.
    order.set_origccy(Ccy::new("CHF")?, false);
    assert_eq!(order.get_origccy().as_str(), "USD");
    ```

=== "Python"

    ```python
    from yggdryl import graph

    listed = graph.OrderEvent(1_700_000_000_000_000_000, currency="EUR")
    # Unheld: the origin is the currency, read and never stored.
    assert listed.origccy is None
    assert listed.origin_currency.as_py() == "EUR"

    # A USD-issued share listed in EUR states its origin.
    issued = graph.OrderEvent(1_700_000_000_000_000_000, currency="EUR", origccy="USD")
    assert issued.origccy is not None and issued.origccy.as_py() == "USD"
    assert (issued.origin_currency.as_py(), issued.currency.as_py()) == ("USD", "EUR")

    # A row's cell is the held value, null otherwise.
    rows = graph.MarketData.arrow_reader([listed, issued]).read_all()
    assert rows.column("origccy").to_pylist() == [None, "USD"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { graph } = require('yggdryl')

    const listed = new graph.OrderEvent(1_700_000_000_000_000_000n, { currency: 'EUR' })
    // Unheld: the origin is the currency, read and never stored.
    assert.equal(listed.origccy, null)
    assert.equal(listed.originCurrency, 'EUR')

    // A USD-issued share listed in EUR states its origin.
    const issued = new graph.OrderEvent(1_700_000_000_000_000_000n, { currency: 'EUR', origccy: 'USD' })
    assert.equal(issued.origccy, 'USD')
    assert.deepEqual([issued.originCurrency, issued.currency], ['USD', 'EUR'])
    ```

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

`securityids` is an [`Identifiers`](identifier.md) map: the names the security goes by, one value per key `src:type` - the base key spelled as its type alone and holding the type's answer ([The base key](identifier.md#the-base-key)) - each validated by its type where the crate knows it - `isin=US0378331005`, `derived:cusip=037833100`, `oms:instrumentid=dbi;CH0012214059_XSWX_CHF`. On a [`marketdata` row](market-data.md#side-information) the cell holds the base keys alone, every other key side information in `metadata` under the map's name, `securityids.derived:cusip`.

| Verb | Rule |
| --- | --- |
| `get_securityids` | the map, sorted by its key's text; `get(&IdType::Isin)` the base key's value - the type's answer, whichever source stated it ([Lookups](identifier.md#contract)) - `get_from(&IdKey::new(src, IdType::Isin))` one source's |
| `get_isincode` | provided: `get_securityids().get(&IdType::Isin)`, borrowed - a projection of the set, never a second store; the `isincode` column writes it through `insert_securityid`, replacing a lower-ranked ISIN and leaving an equal- or higher-ranked value standing ([Market data](market-data.md#arrow)) |
| `set_securityids(ids, overwrite)` | with `overwrite`, replaces the whole map, derived identifiers included, `Identifiers::new()` unsaying it; without, validates each security type then rank-merges: a higher-ranked value replaces a lower-ranked one, equal ranks keep the held value, and other keys remain |
| `insert_securityid(id)` | `Identifiers::insert` after the security check: fills an absent key, or replaces a held value that [ranks](identifier.md#ranks) below it - a real ISIN over a masked one or a typo, whichever came first - a named source filling its type's base key where it is empty or ranks below; it takes back a `derived` identifier of its type unless the derivation outranks it - a statement answers before a derivation of its rank; one from `derived` is a derivation; `ticker` is refused ([`IdType::check_security`](identifier.md#per-type-value-checks)); `true` if it landed |
| `derive_securityid(kind, code)` | fills only a type the element holds none of, from `derived` - the code as `Identifier::new` stores it, a code its type refuses naming nothing - implication, not statement; never reaches a store the holder is a view of |
| `remove_securityid(&key)` | removes what an `IdKey` holds: a named source's key that identifier alone, the base key every key of its type; where no ISIN is left, every `derived` identifier is taken back too, since each hangs on it |
| Building one | [`Identifier::new(key, code)`](identifier.md#contract) checks the code's shape by its type ([per-type checks](identifier.md#per-type-value-checks)) - a check digit that does not close is a [rank](identifier.md#ranks), not a refusal - and holds a type the crate does not know as given; [`IdType::from_security_source`](identifier.md#vocabularies) reads FIX's `SecurityIDSource(22)` - `4`, `isin`, `ISINNumber` are `isin` - and refuses `ticker` |
| `forex` | the crate's own type, which FIX gives no source code: a [currency pair](../types/codes/forex.md) in any spelling `Forex::new` reads, stored canonical (`eurusd` is `EUR/USD`); `forexcode`, `ccypair` and `currencypair` read the type too |
| Derived | an ISIN's embedded national number - CUSIP (`US`/`CA`), SEDOL (`GB`/`IE`/`GG`/`JE`/`IM`, behind `00`), WKN (`DE`, behind `000`), Valor (`CH`/`LI`) ([`securityid::embedded`](../types/codes/isin.md)) - plus what an [`IsinRegistry`](isin-registry.md) filled, each from `derived`; all hang on the `isin`, so removing it revokes them |
| From the ticker | a ticker of an identifier's own shape names it, read by its length before any check - 21 characters `{ISIN}_{MIC}_{CCY}` an instrument key (its ISIN derived, its market and currency filled where none is stated, a part its type refuses skipped), 12 a FIGI behind `BBG` else an ISIN, 9 a CUSIP, 7 a SEDOL, 6 upper-case letters a detailed CFI code, `AAPL.OQ` a RIC, `HOLN SW Equity` a Bloomberg identifier - each closing on its own type's check, so a ticker only the length of one names nothing; derived, never over a stated identifier of its type. `securityid::SymbolCode::from_symbol` is the reading, Rust only |
| Unit of a pair | an element trading a currency pair (a `forex` identifier) and stating no unit states its quantity in the currency dealt: its currency where that is a leg of the pair, else the pair's base (`EUR` for `EUR/USD`) |
| Provenance | a derived identifier is one from `derived`: a row carries it, and equality and the digest read it as they read a stated one |
| Refusals | `Identifier::new` refuses a word that is none, a value that states nothing and a code not of its type's shape; `insert_securityid` and `IdType::from_security_source` refuse `ticker` (use `set_ticker`) |

## Following and merging

| Reading | Rule |
| --- | --- |
| `following_market` | [`Event::following`](event.md#following); `prevpx`/`prevqty`, currency, `origccy`, unit, a sided element's side (this element's where it states one, the chain's where it states `UKNW` - never a `BOTH` the chain held, which tags no one side) - any other element's side is its own tag, and a quote takes the [legs](#a-quotes-two-legs) it states nothing of - ticker, each security id it lacks and the strike price - neither where it names another instrument, below -, classification, market, and every metadata key it lacks - all from the predecessor where this event says nothing, this element's own values standing; always leads, even with the timed link unchanged |
| A FIX message | follows the metadata too, as a [`FixMsg`](../fix/message.md) in the [lifecycle](../fix/lifecycle.md): its metadata is the bridge's namespaced keys its row's `metadata` column holds, so a followed message's row carries the chain's keys |
| Identity | a follower's `hashcode` and `uuid` digest what it takes ([`digest_market`](#contract) feeds the metadata), so they move where it took a key |
| Two instruments | two stated real ISINs that differ - each closing under a listed prefix - name two instruments: no identifier and no strike price is taken from the predecessor, and a merge keeps the leading statement's identifiers whole. A number that is not real - a `ZZ`, a masked one, a typo - names no country's instrument, so it is never the other one: it yields to the higher-ranked ISIN, which replaces it and everything derived under it, whichever statement leads |
| Execution clock | a market event whose state reports an execution ([`is_execution`](event.md#contract)) and states no `execunix` is dated from its own instant first - before it names a predecessor, so an inherited state is never read as its own execution; following then keeps the later of its own clock and its predecessor's, so a delayed report cannot regress it, and a non-execution carries the chain's latest |
| Restating | a market event's [`restating`](event.md#restating) also takes the market's place: `prevpx`/`prevqty`, what the chain is about where this reading said nothing - the metadata keys included, as in following - and the execution clock - the earliest of the two statements' |
| `merging_market` | where `Self: Element`, no event clocks: `self` leads, and the execution clock is the earliest either states |
| `merging_market_event` | [`Event::merging`](event.md#merging)'s reference leads: price/quantity/unit stand; optional numbers, bid/ask facts, ticker = reference's if stated else other's; the execution clock the earliest either statement knows (an unstamped execution observation dates itself first); metadata and FX rates unioned, reference-led; currency/side/classification/market = the better - an unknown yields to a known one (an `XXXX` [MIC](../types/codes/mic.md), the [CFI](../types/codes/cfi.md) merge); security ids: every key either statement holds, a higher-[ranked](identifier.md#ranks) value winning whatever leads, the reference's value where both hold one of a rank |
| Result | each answers nothing where the fold changes nothing, and finalizes where it did |

## Examples

### The facts of an order

=== "Rust"

    ```rust
    use smol_str::SmolStr;
    use yggdryl::graph::{Element, Market, Metadata, OrderEvent};
    use yggdryl::{Ccy, Cfi, Decimal, IdKey, IdSource, IdType, Identifier, Mic, Side};

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
    order.insert_securityid(Identifier::new(IdKey::base(isin), "US0378331005")?)?;
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
    assert_eq!(order.get_securityids().get_from(&IdKey::new(IdSource::Derived, IdType::Cusip)), Some("037833100"));
    assert_eq!(order.get_metadata().get("ordtype").map(SmolStr::as_str), Some("2"));
    // The ISIN names the book it stands in, ahead of the ticker.
    assert_eq!(order.book_crosscode(), "US0378331005");
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
        securityids=[Identifier("isin", "US0378331005")],
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
    assert [str(id) for id in order.securityids] == ["cusip=037833100", "derived:cusip=037833100", "isin=US0378331005"]
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
      securityids: [new Identifier('isin', 'US0378331005')],
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
    assert.equal(order.securityids.toString(), '[cusip=037833100, derived:cusip=037833100, isin=US0378331005]')
    assert.equal(order.isincode, 'US0378331005')
    assert.equal(order.miccode, 'XNAS')
    assert.deepEqual(order.metadata, { ordtype: '2' })
    ```

### Sides and cross codes

One identifier, `O-1001`, on each side of the market: two cross codes, two chains. A trade built on the buy order is not sided: it takes the order's base code under its own kind, `21:0:O-1001`.

=== "Rust"

    ```rust
    use yggdryl::graph::{BookEvent, Element, ExecutionEvent, Market, OrderEvent, QuoteEvent, TradeEvent};
    use yggdryl::{IdKey, IdType, Identifier, Isin, Side};

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

    // Only an order or an execution is sided: a quote, a book and a trade
    // state side 0 whatever side they take, and a trade built on the buy
    // order takes its base code under its own kind.
    let mut book = BookEvent::new(1_700_000_000_000_000_000, "AAPL");
    book.set_side(Side::Buy, true);
    assert!(buy.is_sided() && !book.is_sided());
    assert_eq!(book.get_crosscode(), "3:0:AAPL");
    let mut quote = QuoteEvent::at(1_700_000_000_000_000_000);
    quote.set_crosscode("Q-1".to_owned());
    quote.set_side(Side::Sell, true);
    assert!(!quote.is_sided());
    assert_eq!(quote.get_crosscode(), "14:0:Q-1");
    let mut fill = ExecutionEvent::at(1_700_000_000_000_000_000);
    fill.set_crosscode("E-1".to_owned());
    fill.set_side(Side::Sell, true);
    let trade = TradeEvent::from_parts(&buy, vec![fill])?;
    assert_eq!((trade.get_crosscode(), trade.get_side()), ("21:0:O-1001", Side::Buy));
    assert_eq!(trade.executions()[0].get_crosscode(), "8:2:E-1");

    // The book key: the ISIN, else the ticker, else the number that states none.
    let mut listed = order(Side::Buy);
    assert_eq!(listed.book_crosscode(), Isin::NONE);
    listed.set_ticker(Some("HOLN".into()), true);
    assert_eq!(listed.book_crosscode(), "HOLN");
    listed.insert_securityid(Identifier::new(IdKey::base(IdType::Isin), "CH0012214059")?)?;
    assert_eq!(listed.book_crosscode(), "CH0012214059");
    ```

=== "Python"

    ```python
    from yggdryl import Side, graph

    buy = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side="BUYS")
    sell = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001", side=Side.SELL)
    assert (buy.crosscode, sell.crosscode) == ("10:1:O-1001", "10:2:O-1001")
    assert buy.crossuuid != sell.crossuuid
    # An order stating no side reads UKNW and states 0.
    unsided = graph.OrderEvent(1_700_000_000_000_000_000, crosscode="O-1001")
    assert unsided.crosscode == "10:0:O-1001" and unsided.side is Side.UKNW
    # A quote is not sided: its side is a tag, its code stored under side 0.
    quote = graph.QuoteEvent(1_700_000_000_000_000_000, crosscode="Q-1", side="SELL")
    assert (quote.crosscode, quote.side) == ("14:0:Q-1", Side.SELL)

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
    // An order stating no side reads UKNW and states 0.
    const unsided = new graph.OrderEvent(1_700_000_000_000_000_000n, { crosscode: 'O-1001' })
    assert.equal(unsided.crosscode, '10:0:O-1001')
    assert.equal(unsided.side, 'UKNW')
    // A quote is not sided: its side is a tag, its code stored under side 0.
    const quote = new graph.QuoteEvent(1_700_000_000_000_000_000n, { crosscode: 'Q-1', side: 'SELL' })
    assert.deepEqual([quote.crosscode, quote.side], ['14:0:Q-1', 'SELL'])

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
    use yggdryl::{Ccy, IdKey, IdType, Identifier, Side};

    let mut quote = QuoteEvent::at(1_700_000_000_000_000_000);
    quote.set_crosscode("Q-7".to_owned());
    quote.set_currency(Ccy::new("USD")?, true);
    quote.insert_securityid(Identifier::new(IdKey::base(IdType::Forex), "eurusd")?)?;
    quote.set_bidpx(Some("1.0842".parse()?), true);
    quote.set_askpx(Some("1.0844".parse()?), true);
    quote.set_bidccy(Some(Ccy::new("USD")?), true);
    quote.set_askccy(Some(Ccy::new("USD")?), true);
    quote.set_price(Some("125".parse()?), true);
    assert!(quote.insert_fxrate(Ccy::new("EUR")?, "1.25".parse()?));
    assert!(!quote.insert_fxrate(Ccy::new("EUR")?, "2".parse()?), "a stated rate stands");
    quote.finalize();

    // The pair lands canonical under the crate's own type.
    assert_eq!(quote.get_securityids().to_string(), "[forex=EUR/USD]");
    // Two legs and no tag: both sides.
    assert_eq!(quote.get_side(), Side::Both);
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
        securityids=[Identifier("forex", "eurusd")],
        bidpx=Decimal("1.0842"),
        askpx=Decimal("1.0844"),
        bidccy="USD",
        askccy="USD",
        price=Decimal("125"),
        fxrates={"EUR": Decimal("1.25")},
    )

    # The pair lands canonical under the crate's own type.
    assert str(quote.securityids) == "[forex=EUR/USD]"
    # Two legs and no tag: both sides.
    assert quote.side is Side.BOTH
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
      securityids: [new Identifier('forex', 'eurusd')],
      bidpx: '1.0842',
      askpx: '1.0844',
      bidccy: 'USD',
      askccy: 'USD',
      price: '125',
      fxrates: { EUR: '1.25' },
    })

    // The pair lands canonical under the crate's own type.
    assert.equal(quote.securityids.toString(), '[forex=EUR/USD]')
    // Two legs and no tag: both sides.
    assert.equal(quote.side, 'BOTH')
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
use yggdryl::{IdKey, IdType, Identifier};

let isin = |code: &str| Identifier::new(IdKey::base(IdType::Isin), code);
let ids = |order: &OrderEvent| -> Vec<String> {
    order.get_securityids().iter().map(ToString::to_string).collect()
};
let mut order = OrderEvent::at(1_700_000_000_000_000_000);
order.set_crosscode("O-1001".to_owned());
order.insert_securityid(isin("US0378331005")?)?;
order.finalize();
assert_eq!(ids(&order), ["cusip=037833100", "derived:cusip=037833100", "isin=US0378331005"]);

// A derivation fills only a type the element holds none of, and a stated
// identifier stands.
assert!(!order.derive_securityid(&IdType::Cusip, "594918104"));
assert!(!order.insert_securityid(isin("US5949181045")?)?);

// Everything derived hangs on the isin: removing its type takes the cusip back.
let mut unlisted = order.clone();
assert!(unlisted.remove_securityid(&IdKey::base(IdType::Isin))?);
assert!(unlisted.get_securityids().is_empty());
assert_eq!(unlisted.get_isincode(), None);

// A stated identifier takes back a derived one of its type, and then
// outlives the ISIN.
assert!(order.insert_securityid(Identifier::new(IdKey::base(IdType::Cusip), "037833100")?)?);
assert!(order.remove_securityid(&IdKey::base(IdType::Isin))?);
assert_eq!(ids(&order), ["cusip=037833100"]);

// A currency pair is refused where it names no pair.
assert!(Identifier::new(IdKey::base(IdType::Forex), "EUR/EUR").is_err());
```

### Identifiers along a chain

An amendment to an Apple order, and one naming Microsoft's ISIN instead.

=== "Rust"

    ```rust
    use yggdryl::graph::{Element, Event, Market, OrderEvent};
    use yggdryl::{IdKey, IdType, Identifier, Identifiers};

    const T: i64 = 1_700_000_000_000_000_000;
    let order = |unix: i64, stated: &[(IdType, &str)]| -> yggdryl::Result<OrderEvent> {
        let ids = stated
            .iter()
            .map(|(kind, code)| Identifier::new(IdKey::base(kind.clone()), code))
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
    assert_eq!(
        keys(&placed),
        ["cusip=037833100", "derived:cusip=037833100", "figi=BBG000B9XRY4", "isin=US0378331005"]
    );

    // A follower stating none takes the chain's whole.
    let amended = order(T + 1, &[])?.with_previous(&placed).expect("a later event follows");
    assert_eq!(amended.get_securityids(), placed.get_securityids());
    let other = order(T + 1, &[(IdType::Isin, "US5949181045")])?.with_previous(&placed).expect("it follows");
    assert_eq!(keys(&other), ["cusip=594918104", "derived:cusip=594918104", "isin=US5949181045"]);

    // Two statements naming different ISINs never mix: the leading
    // statement - here the one sent later - keeps its identifiers whole.
    let mut restated = placed.clone();
    restated.set_sendunix(Some(T + 5));
    restated.set_securityids(other.get_securityids().clone(), true)?;
    let merged = placed.clone().merge_with(&restated).expect("the statement sent later leads");
    assert_eq!(keys(&merged), ["cusip=594918104", "derived:cusip=594918104", "isin=US5949181045"]);
    ```

=== "Python"

    ```python
    from yggdryl import Identifier, graph

    T = 1_700_000_000_000_000_000


    def order(unix: int, stated: dict[str, str]) -> graph.OrderEvent:
        securityids = [Identifier(kind, code) for kind, code in stated.items()]
        return graph.OrderEvent(unix, crosscode="O-1001", securityids=securityids)


    def keys(event: graph.OrderEvent) -> list[str]:
        return [str(id) for id in event.securityids]


    placed = order(T, {"figi": "BBG000B9XRY4", "isin": "US0378331005"})
    assert keys(placed) == ["cusip=037833100", "derived:cusip=037833100", "figi=BBG000B9XRY4", "isin=US0378331005"]

    # A follower stating none takes the chain's whole.
    amended = order(T + 1, {}).with_previous(placed)
    assert amended is not None and amended.securityids == placed.securityids
    other = order(T + 1, {"isin": "US5949181045"}).with_previous(placed)
    assert other is not None and keys(other) == ["cusip=594918104", "derived:cusip=594918104", "isin=US5949181045"]
    assert other.isincode == "US5949181045"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Identifier, graph } = require('yggdryl')

    const T = 1_700_000_000_000_000_000n
    const order = (unix, stated) => new graph.OrderEvent(unix, {
      crosscode: 'O-1001',
      securityids: Object.entries(stated).map(([kind, code]) => new Identifier(kind, code)),
    })

    const placed = order(T, { figi: 'BBG000B9XRY4', isin: 'US0378331005' })
    assert.equal(placed.securityids.toString(), '[cusip=037833100, derived:cusip=037833100, figi=BBG000B9XRY4, isin=US0378331005]')

    // A follower stating none takes the chain's whole.
    const amended = order(T + 1n, {}).withPrevious(placed)
    assert.ok(amended.securityids.equals(placed.securityids))
    const other = order(T + 1n, { isin: 'US5949181045' }).withPrevious(placed)
    assert.equal(other.securityids.toString(), '[cusip=594918104, derived:cusip=594918104, isin=US5949181045]')
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

- On an order or an execution, `set_side(Side::Unknown, true)` restates the code under side `0` (`10:0:O-1001`), the base kept. On any other element the code stays under side `0` whatever side `set_side` states.
- A ticker stated empty is none: with no ISIN either, `book_crosscode` answers `XX0000000000`.
- An ISIN keys the book whatever its rank: a masked number keys a book of its own until the lifecycle's registry fills the real one, which outranks it.
- A zero rate is a statement like any other; dividing by it is the caller's refusal to make - `Decimal::checked_div` answers `None`.
