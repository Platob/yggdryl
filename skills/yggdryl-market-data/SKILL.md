---
name: yggdryl-market-data
description: Models and streams market data with yggdryl's graph layer in Rust, Python and Node.js - orders, quotes, executions and trades as dated events (OrderEvent, QuoteEvent, ExecutionEvent, TradeEvent), identities and side-keyed chains (uuid, crossuuid, with_previous / withPrevious, EventIterator), order books keyed by ISIN or ticker (BookIterator and its filter, BookEvent, delta books rebuilt with with_previous, is_complete, alive_on, limits, best_price / bestPrice, spread, depth, SnapshotEvent), OHLC candles of the best bid and ask per zone-aligned bucket (CandleIterator, CandleOptions, graph.candles, Candle.field), the Side and MarketDataKind enums and the lifted marketdata Arrow row (MarketData.arrow_reader / arrowReader, from_arrow_reader / fromArrowReader, apply_view / applyView). Use when building market events, folding them into books or candles, or persisting or querying marketdata batches.
---

# yggdryl market data

The graph layer is market data as **elements that name each other by
identity**, never by reference. Four Rust traits say what an element answers -
`Element` (identity, cross code, digest, sources), `Event` (instant, state,
place at its instant, clocks), `Market` (thirty-seven facts: the `marketdatatype` of its kind, price, stop price, the `strikepx` of the option it is about,
quantity and its shown and hidden parts, currency, the `origccy` the instrument was issued in (`origin_currency()` reads `currency` where none is held), unit, side, the security's identifiers `securityids` and the `isincode` they hold, the `instcode` of the instrument it is about, classification, market,
the `execunix` it last executed at, last-trade, progress, FX parts, the stated bid and ask - `bidpx`, `bidqty`,
`bidccy`, `askpx`, `askqty`, `askccy` - the `fxrates`, ticker, metadata) and
`Operation` (five more: the `ordqty` it asked for, the `TimeInForce` member it stands for, whether it trades, the `identifiers`, the
`partyids` it names - the account included) - and
typed leaves answer them: `Order`/`OrderEvent`, `Quote`/`QuoteEvent`,
`Execution`/`ExecutionEvent`, the composite `TradeEvent`, and the book types
`BookEvent` and `SnapshotEvent`. `MarketData` is the one value over every
leaf, and the lifted **`marketdata` Arrow row** (66 columns: the element,
event, market and operation columns every generated row opens with, the book
controls `bookscope`, `bookaction` and `bookposition`, then the nested
`alive`, `delta`, `events`, `executions`, `bidlimits` and `asklimits` -
[Row schemas](https://platob.github.io/yggdryl/graph/schemas/)) is how any of
them crosses a boundary.

Hold these facts:

- **In Rust the market half is its own crate.** `Element`, `Event`,
  `ElementColumn` and `EventColumn` are the core's `yggdryl::graph`; the
  market traits, the leaves, the walk, the books, the candles, `MarketData`,
  the identifiers, `Instrument`, `Instruments` and the four enums (`Side`,
  `MarketDataKind`, `MarketDataType`, `TimeInForce`) are `yggdryl-market`'s -
  `yggdryl_market::graph::OrderEvent`, `yggdryl_market::Side`. Call
  `yggdryl_market::install()?` once before anything reads a market kind: until
  then `side` is refused naming the crate. Python and Node.js install it on
  import.
- **Instants are `i64` nanoseconds since the Unix epoch, UTC** - `transunix`,
  `creaunix`, `sendunix`, `exprunix`, `prevunix`, `snapunix`, and the market's
  `execunix`, which an undated element states too.
  Python ints, JavaScript `bigint`s, Arrow `datetime64(ns, UTC)`.
- **Identity is derived, not assigned.** A leaf is finalized on construction:
  `hashcode` is the XXH3-64 of its content, `uuid` a UUIDv7 of
  millisecond + place for a dated leaf (UUIDv8 of content otherwise),
  `crossuuid` the chain every incarnation shares, from the cross code.
- **A side is never null; it keys an order's and an execution's chain.**
  `Side` and `MarketDataKind` are `uint8` enums; an element stating no side
  holds `UKNW` (code 0). A cross code is stored as `{kind}:{side}:{base}` - a
  buy order `O-1001` is `10:1:O-1001` - and only an order and an execution
  state their side there (`MarketDataKind::is_sided`, Rust-only), so the two
  sides of one order identifier are two chains. Every other kind stores side
  `0` whatever side it states: a quote (`14:0:Q-7`), a trade (`21:0:T-1`), a
  book (`3:0:AAPL`), a snapshot control. A book holds both sides, so its side
  is always `BOTH` (code 99), and still stores side `0`.
- **A quote is one element holding two legs.** Its bid is `bidpx`/`bidqty`/
  `bidccy`, its ask `askpx`/`askqty`/`askccy`, and its `side` is a tag: a
  quote stating a side and a `price`/`quantity` states the leg that side
  takes, and a two-sided quote tagging none states `BOTH` once finalized.
  Every statement of one quote is one chain under its code, whatever side it
  tags; a follower stating nothing of a leg carries that leg from its chain.
- **Chains are walked, not rebuilt.** An event names only its predecessor
  (`prevuuid`); `seqnum` is its place among the events of its instant, which
  orders the identities of one millisecond, and a step keeps its own unless
  its predecessor shares or passes its instant. A follower takes what its
  chain states and it does not - every `metadata` key, every `identifiers` key but
  `mdentryrefid` (a book entry's reference to its predecessor, which names one
  step) and every `partyids` source and role, its own values standing, each
  identifier a follower states anew keeping its own value - and its identity
  digests what it took, and a follower naming no other ISIN than its
  predecessor takes its instrument facts too - the security identifiers it
  lacks and an option's `strikepx`. Parents move along a chain: a
  changed `orderid` leaves its previous value as `parentorderid` and the
  chain's first as `origorderid`, a changed `clordid` leaves `origclordid`
  (`FIX:parents` states the list a FIX field has). A FIX lifecycle message takes the `metadata` keys and
  only the ids its dictionary follows, each with its parents. `EventIterator` joins a stream by cross
  identity and by the type and value of a chain identity a live element went by
  (`orderid`, `clordid`, `quoteid`, `tradeid`, `tradereportid` and their
  secondary ones - never `execid`, `trdmatchid` or `quotereqid`) or the chain's
  first value a lineage identifier names (`origclordid`, `origorderid`), filed
  under its base, each name held by the first live chain that stated it; an
  element citing two live chains joins neither - it stands under its own
  identity, `Operation::note_conflict` tells it (a FIX message's `FixAnomaly`)
  and the walk warns once per kind - and every element of a chain is re-keyed
  onto the chain's side and first cross code,
  within one `marketdatakind` (an order and an execution under one cross code
  are two chains, so a fill never restates, follows or ends its order),
  folds twins, emits expiries, and leaves every element stating `creaunix`. A grid view is the live element
  as of its tick: dated at it (`transunix` = the tick), its `snapunix` the
  instant the element it copies was stated at (never the tick, never a
  predecessor's, and in no digest), so its `uuid` is the tick's own
  while content, `seqnum`, `prevuuid` and `crossuuid` are the live
  element's; it advances nothing. `srcuuids` is provenance only - the line a
  message was read from and the message a parse split it off - and travels
  along no chain.
- **A ticker of an identifier's shape names it.** `US0378331005`, a
  `BBG` FIGI, a CUSIP, a SEDOL, a detailed CFI code, `AAPL.OQ`, `HOLN SW
  Equity` or an instrument key `CH0012214059_XSWX_CHF` derive the identifier
  (and the key's market and currency) where none is stated; a currency pair
  with no unit takes the currency dealt as its unit.
- **Identifiers are typed maps.** `securityids`, `identifiers` and `partyids` are
  each an `Identifiers` map from a key to a value, `Identifier { key, value }`
  (`yggdryl_market::Identifier`, Python `from yggdryl import Identifier`, JavaScript
  `require('yggdryl').Identifier`): the key is a source and a type, spelled
  `src:type`, and the base source's key - what a FIX field states - is spelled
  as its type alone (`isin`); one value per key, sorted by that spelling,
  displayed `isin=US0378331005`, every word lower case; `derived` is what the
  crate derived (an ISIN's CUSIP, a ticker's shape, a FX pair and its minted
  number, an instruments fill). The base key is the type's answer: a named source fills it where it
  is empty (`ullink:isin=X` alone is also `isin=X`), a statement takes back
  the type's derivation, and it moves only through its own key - removing it
  removes the type. A value under the `bic` source is held to a BIC's shape
  and one under `legalentityidentifier` to an LEI's, whatever its type
  (`Identifier("bic:executingfirm", "deutdeff")` is `DEUTDEFF`; `"T-1"` is
  refused on its key), and ranks by the lower of its type's rank and the
  code's (a BIC's listed country, an LEI's closing check digits); every other
  source follows its type's rule alone. `fisn` (also `fisncode`,
  `financialinstrumentshortname`) is a security type of the FISN's shape -
  FIX's `FinancialInstrumentShortName(2737)` lands in `securityids` under it -
  and `elf` (`entitylegalform`) an entity legal form's, neither a security
  nor a party. On a `marketdata` row the cell holds the base keys alone,
  one per type; every other key is side information in `metadata` under its
  map's name and its `src:type` spelling (`securityids.ullink:isin`), read
  back into the map the name says. Build one with
  `Identifier(key, value)` - the key read
  exactly (`"isin"`, `"ullink:isin"`, `"fix:isin"` is `isin`), the value
  held to its type's shape, so `Identifier("isin", code)` takes any twelve
  characters of an ISIN's shape and refuses only another shape - a check digit
  that does not close or a masked `XX0000000001` is a value of a lower rank
  (`IdType::rank`, Rust-only), which a real value replaces on every merge and
  follow whatever the order - or `Identifier.from_key(name, value)`, which reads the identifier
  name a bridge's own spelling ends with (`OMS_InstrumentID` is
  `oms:instrumentid`) - another instrument's word before a security type,
  after any namespace, names none (`OMS_UnderlyingISIN`, `FIX.LegISIN`); Rust
  takes an `IdKey` (`IdKey::base(IdType::Isin)`, `"ullink:isin".parse()`). A map
  crosses as a `dict` / plain object of key text to value
  (`Identifiers.from_dict`, `into_dict`; `fromObject`, `intoObject`). Parentage
  is a relation between types, never a field of a value: when an `orderid`
  changes along a chain, a follower keeps the value it held as
  `parentorderid` and the chain's first as `origorderid` (a `clordid` keeps
  only `origclordid`), and one stating only a parent takes the parent's own
  type from it.
- **A setter fills, or overwrites when told.** Every Rust `Market` and
  `Operation` setter takes a trailing `overwrite: bool`: `false` lands only
  where the fact is unstated, `true` states it. A change carries what it
  implies - a buyer's price and quantity are its bid, a side moves the quote
  and a sided cross code, `leavesqty` is the quantity, an order's `ordqty`,
  `cumqty` and `leavesqty` fill one another by its state (`LeavesQty = OrderQty
  - CumQty` while it works, `0` once done, the rest `cxlqty` when canceled) -
  and the binding constructors run the same fills. Every fill is a column, so
  a row read back answers it unchanged; never recompute one by hand.
- **Books are folded, complete or delta.** `BookIterator` folds a sorted
  stream into one `BookEvent` per book and instant that moved it. A book
  folds orders and quotes into its sides (`MarketDataKind::is_booked`,
  Rust-only) and records every execution among its `events` at its instant
  (`is_recorded`), moving no side - a fill moved the book through its
  order's or quote's own report; a trade or a batch is pruned before the
  walk. A book is **complete** (`is_complete`: its `alive` entries and
  `limits`) only at a snapshot tick - every grid tick a positive
  `snapshot_millis` crosses, and a snapshot input (a FIX `W` full refresh, an
  empty `W`, inputs stating `snapunix`); every other book is a **delta
  book**, holding no sides: its `delta` (the orders and quotes its instant
  applied) and its `events` (the executions and snapshot controls it
  recorded) beside the top of book they settled on (`bidpx`, `askpx`,
  `best_price`, `spread`). With `snapshot_millis = 0` and no snapshot input
  every book is a delta book. `with_previous` over the complete book before it
  rebuilds a delta book whole under its own identity; a code's first book
  follows no book (`prevuuid` null) and rebuilds over the empty
  `BookEvent.keyed(unix, key)`. Sorted books fold on into `Candle`s - one
  OHLC of the best bid, the best ask, the mid and the spread per book cross
  code and bucket.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| a dated order / quote / execution | `OrderEvent::at(unix)`, `set_*`, `finalize()` | `graph.OrderEvent(unix, **facts)` | `new graph.OrderEvent(unix, facts)` |
| an undated leaf, dated later | `Order::new()`, `order.at(unix)`, `event.into_element()` | `graph.Order(**facts)`, `.at(unix)`, `.into_element()` | `new graph.Order(facts)`, `.at(unix)`, `.intoElement()` |
| a quote and its two legs | `set_bidpx`, `set_askpx`, `set_bidqty` ..., or `set_side` + `set_price` for one leg | `graph.QuoteEvent(unix, bidpx=..., askpx=...)` | `new graph.QuoteEvent(unix, { bidpx, askpx })` |
| a market-data entry's book control | `event.with_book(BookRef { .. })` | `event.with_book(graph.BookRef(action="new", position=1))` | `event.withBook(new graph.BookRef({ action: 'new', position: 1 }))` |
| an identifier | `Identifier::new(IdKey::base(IdType::Isin), value)?`, `"ullink:isin".parse::<IdKey>()?`, `Identifiers` | `Identifier(key, value)` - `"isin"`, `"ullink:isin"` - `Identifiers([...])`, `Identifiers.from_dict({...})`, `into_dict()` | `new Identifier(key, value)`, `new Identifiers([...])`, `Identifiers.fromObject({...})`, `intoObject()` |
| what instruments are known by | `Instruments::from_url(&url, props)?`, `instruments.enrich(&mut event)`, `get("CH0012214059")` (by cross code, alias or ISIN), `get_by_ticker("HOLN", Some(&mic))`, `commit()?` | `Instruments.from_url(path)`, `instruments.get("CH0012214059")` (a `dict`), `get_by_uuid(uuid)`, `get_by_ticker("HOLN", "XSWX")`, `enrich(fix_msg)`, `commit()` | `Instruments.fromUrl(path)`, `instruments.get('CH0012214059')` (a plain object), `getByTicker('HOLN', 'XSWX')`, `enrich(fixMsg)`, `commit()` |
| an instrument's key, and the one a market row names | `Instrument::for_security(isin)?`, `Instrument::for_body(class, forex, &characteristics, underlying, legs)?` -> `get_crosscode()` (`US0378331005`, `IF:EUR/USD`, `OC:US0378331005:2026-12-18:200`); `element.get_instcode()` - join on `instcode = crosscode` | `merge({"cficode": "OCXXXX", "underlying": ..., "characteristics": {...}})`, `row["crosscode"]`, `row["aliascodes"]`; `.instcode` | `merge({...})`, `row.crosscode`; `.instcode` |
| the number minted for an instrument no agency numbers | `Instrument::minted_number(code)`, `instrument.minted_isin()`, `is_own_mint(&isin)` | `Instruments.mint(code)`, `row["isin"]` | `row.isin` |
| an instrument's listings, one per market | `instrument.listings()` (MIC order), `listing(Some(&mic))`, `instruments.get_listing(key, &mic)`, `remove_listing(key, &mic)`, `rows()` (one per instrument) | `listings(key)`, `get_listing(key, "XSWX")`, `rows` (a property), `remove_listing(key, "XSWX")` | `listings(key)`, `getListing(key, 'XSWX')`, `rows` (a getter), `removeListing(key, 'XSWX')` |
| what no typed fact holds, and every source's own code | `instrument.metadata()`, `set_metadata(key, value)?`; `securityids().get_from(&"ullink:isin".parse()?)` | `row["metadata"]`, `row["securityids"]["ullink:isin"]`; `merge({..., "metadata": {...}})` | `row.metadata`, `row.securityids.get('ullink:isin')` |
| when an instrument was first and last met, and last changed | `instrument.firstunix()` (an earlier `learn` moves it back), `instrument.lastunix()` (a later one moves it), `instrument.updunix()` (only a moved fact does) | `row["firstunix"]`, `row["lastunix"]`, `row["updunix"]` (a `datetime`) | `row.firstunix`, `row.lastunix`, `row.updunix` (a `Date`, or a datetime `Scalar` past millisecond precision) |
| which instrument an element means, and how | `instruments.resolve(&element)` -> `Resolution::Matched { entry, tier, derived, listing }` or `Resolution::Unmatched(Unmatched::..)`; `get_by_code(&IdType::Cusip, "037833100")`; `Instruments::LOOKUP_CODES` | `instruments.resolve(element)` -> `Resolution` (`.matched`, `.entry`, `.tier`, `.kind`, `.unmatched`, `.codes`, ...); `get_by_code("cusip", "037833100")`; `Instruments.LOOKUP_CODES` | `instruments.resolve(element)` -> a plain object (`.matched`, `.entry`, `.tier`, `.kind`, `.unmatched`, `.codes`, ...); `getByCode('cusip', '037833100')`; `Instruments.lookupCodes()` |
| match by short name, scored | `set_economic_match(true)` (a fill takes it), `set_economic_threshold(0.9)?` (default `0.85`) | `set_economic_match(True)`, `set_economic_threshold(0.9)`, `economic_threshold` | `setEconomicMatch(true)`, `setEconomicThreshold(0.9)`, `economicThreshold` |
| the currency an instrument was issued in | `get_origccy()` (stated or filled; `Ccy::none()` otherwise), `origin_currency()` (else the currency), `set_origccy(ccy, overwrite)`; an instrument's `origccy()` | `.origccy` (`Scalar` or `None`), `.origin_currency`; `origccy="USD"` at build; `row["origccy"]` | `.origccy` (or `null`), `.originCurrency`; `origccy: 'USD'` at build; `row.origccy` |
| security, own and party identifiers | `insert_securityid(id)?`, `insert_identifier(id)?`, `insert_partyid(id)?` (Rust-only verbs) | `securityids=[Identifier("isin", ...)]`, `identifiers=[...]`, `partyids=[...]` at build | `securityids: [new Identifier('isin', ...)]`, `identifiers: [...]`, `partyids: [...]` at build |
| read an identifier map | `get_securityids().get(&IdType::Isin)`, `get_from(&src, &kind)` | `order.securityids.get("isin")`, `get_from(src, type)`, iterate `Identifier`s | `order.securityids.get('isin')`, `getFrom(src, type)`, `toArray()` |
| FX rates (nothing fills them) | `insert_fxrate(ccy, rate)`, `set_fxrates(map)` | `fxrates={"EUR": Decimal("1.1")}` at build | `fxrates: { EUR: '1.1' }` at build |
| an option's strike price (a follower of the same instrument carries it) | `set_strikepx(Some(px), overwrite)`, `get_strikepx()` (`Market`) | `strikepx=Decimal("190")` at build, `.strikepx` | `strikepx: '190'` at build, `.strikepx` |
| the common instruments, and what an instrument derives | `Instruments::seeded()`, `Instruments::seeded_from_url(&url, props)?` (a store laid over them), `instrument.fisn()`; a folded instrument's embedded CUSIP, WKN or Valor, its listings' SEDOL and each listing's market's currency where it states none | `Instruments.seeded()`, `Instruments.seeded_from_url(path)`, `row["fisn"]`, `row["securityids"]["cusip"]`, `row["listings"][0]["currency"]` | `Instruments.seeded()`, `Instruments.seededFromUrl(path)`, `row.fisn`, `row.securityids.get('cusip')`, `listings(key)[0].currency` |
| a venue's facts, a country's currency | `Mic::operating()`, `is_segment()`, `country()`; `Country::currency()` | `Mic.from_str("XNGS").operating`, `.is_segment`, `.country`; `Country.from_str("GB").currency` (`yggdryl.enums`) | `new Mic('XNGS').operating`, `.isSegment`, `.country`; `new Country('GB').currency` |
| a structured product's category, as an instrument holds it | `instruments.get(key).and_then(Instrument::eusipacode)` -> `Eusipa`, `instrument.with_eusipacode(Some(code))`; `name()`, `sspa_name()` | `instruments.get(key)["eusipacode"]` (an `int`), `Eusipa(code).name`, `.sspa_name`; `merge({..., "eusipacode": 2300})` | `instruments.get(key).eusipacode` (a number); no `Eusipa` |
| a composite trade | `TradeEvent::from_parts(&root, executions)?` | `graph.TradeEvent.from_parts(root, executions)` | `graph.TradeEvent.fromParts(root, executions)` |
| follow a predecessor | `event.with_previous(&prev)` | `event.with_previous(prev)` | `event.withPrevious(prev)` |
| merge two statements of one event | `event.merge_with(&other)` | `event.merge_with(other)` | `event.mergeWith(other)` |
| walk a stream into chains | `EventIterator::new(items, sorted)`, `.with_snapshot_ns(ns)` | `graph.EventIterator(items, sorted=True, snapshot_ns=None)` | `new graph.EventIterator(items, sorted, snapshotNs)` (sorted defaults to `true`) |
| any leaf as one value | `MarketData::from(leaf)`, `kind()`, `marketdatakind()`, `as_order_event()`, `TryFrom` | `graph.MarketData(leaf)`, `.kind`, `.marketdatakind`, `.as_order_event()`, `.into_leaf()` | `new graph.MarketData(leaf)`, `.kind`, `.marketdatakind`, `.asOrderEvent()`, `.intoLeaf()` |
| a FIX message held whole | `MarketData::from(msg)` (kind `fix`, its `marketdatakind`), `as_message::<FixMsg>()`, `FixMsg::try_from(value)?`; written and folded as the leaves it splits into | `graph.MarketData(msg)`, `.as_fix()` | `new graph.MarketData(msg)`, `.asFix()` |
| the `marketdata` row schema | `MarketData::field()?` | `graph.MarketData.field()` | `graph.MarketData.field()` |
| leaves to Arrow batches | `MarketData::arrow_reader(values, None, None)?` | `graph.MarketData.arrow_reader(values)` | `graph.MarketData.arrowReader(values)` |
| Arrow batches to leaves | `MarketData::from_arrow_reader(reader)?` | `graph.MarketData.from_arrow_reader(source)` | `graph.MarketData.fromArrowReader(reader)` |
| fold a sorted stream into books | `BookIterator::new(items, snapshot_millis)?`, `.with_filter("side = 'BUYS'")?` | `graph.BookIterator(items, snapshot_millis=0, filter=None)` | `new graph.BookIterator(items, snapshotMillis = 0, filter = undefined)` |
| one book by hand | `BookEvent::new(unix, ticker)`, `add_operations(..)?` | `graph.BookEvent(unix, ticker).with_operations([...])` | `new graph.BookEvent(unix, ticker).withOperations([...])` |
| the empty book a code starts from | `BookEvent::keyed(unix, key)` | `graph.BookEvent.keyed(unix, key)` | `graph.BookEvent.keyed(unix, key)` |
| whether a book holds its sides | `is_complete()` | `book.is_complete` | `book.isComplete` |
| rebuild a delta book whole | `book.with_previous(&previous)` | `book.with_previous(previous)` | `book.withPrevious(previous)` |
| a book's entries | `alive()`, `alive_on(Side::Buy)`, `delta()`, `events()` | `book.alive`, `book.alive_on(Side.BUYS)`, `book.delta`, `book.events` | `book.alive()`, `book.aliveOn('BUYS')`, `book.delta()`, `book.events()` |
| a book's entries by kind | `ordlive()`; `orddelta()` and `quotes()` partition `delta`, `executions()` and `controls()` partition `events` | `book.ordlive`, `book.orddelta`, `book.quotes`, `book.executions`, `book.controls` | `book.ordlive()`, `book.orddelta()`, `book.quotes()`, `book.executions()`, `book.controls()` |
| a table of books' delta or events as rows | `MarketData::delta_serie(books, None)?`, `MarketData::events_serie(books, Some(MarketDataKind::Execution))?` | `graph.MarketData.delta_serie(books)`, `graph.MarketData.events_serie(books, "EXEC")` | `graph.MarketData.deltaSerie(books)`, `graph.MarketData.eventsSerie(books, 'EXEC')` |
| read a side | `limits(Side::Buy)`, `best_price(Side::Buy)`, `best_quantity(..)`, `depth(Side::Buy, n)` | `book.limits(Side.BUYS)`, `book.best_price(Side.BUYS)`, `book.depth(Side.BUYS, n)` | `book.limits('BUYS')`, `book.bestPrice('BUYS')`, `book.depth('BUYS', n)` |
| read both sides | `get_bidpx()`, `get_askpx()`, `spread()`, `is_crossed()`, `imbalance(n)` | `book.bidpx`, `book.askpx`, `book.spread`, `book.is_crossed`, `book.imbalance(n)` | `book.bidpx`, `book.askpx`, `book.spread`, `book.isCrossed`, `book.imbalance(n)` |
| clear a scope with a snapshot | `SnapshotEvent::snapshot(&event, scope)` | `graph.SnapshotEvent.snapshot(event, scope=None)` | `graph.SnapshotEvent.snapshot(event, scope)` |
| a named view of a stream | `MarketData::apply_view(&MarketView::Orders, &lifts, reader)?` | `graph.MarketData.apply_view("orders", source, lifts)` | `graph.MarketData.applyView('orders', reader, lifts)` |
| a view as a plan | `MarketData::plan(&view, &lifts)?` | `graph.MarketData.plan("trades")` | `graph.MarketData.plan('trades')` |
| FIX to sorted market data / books | `codec.market_data(codec.lifecycle(msgs))`, `codec.book_arrow_reader(msgs, 0, None)?` | `codec.market_data(...)`, `codec.book_arrow_reader(msgs, snapshot_millis=0, filter=None)` | `codec.marketData(..)`, `codec.bookArrowReader(msgs, 0, filter)` |
| fold sorted books into candles | `CandleIterator::new(books, CandleOptions::from_spelling("1m")?.with_timezone(zone))` | `graph.candles(books, "1m", timezone=None)`, `graph.CandleIterator(books, graph.CandleOptions("1m", zone))` | `graph.candles(books, '1m', zone)`, `new graph.CandleIterator(books, new graph.CandleOptions('1m', zone))` |
| a candle's row, and candles as Arrow | `Candle::field()?`, `Candle::arrow_reader(candles, None)?`, `candle.into_scalar()`, `Candle::from_scalar(&value)?` | `graph.Candle.field()`, `candle.into_scalar()`, `candle.as_py()`, `graph.Candle.from_scalar(value)` | `graph.Candle.field()`, `candle.intoScalar()`, `candle.toJSON()`, `graph.Candle.fromScalar(value)` |

## Rules for fast, correct use

1. Stay in the `marketdata` row for bulk. `MarketData.arrow_reader` writes
   column by column with no per-row `Scalar`, closing batches on a row and a
   byte bound; `from_arrow_reader` lands each batch once and resolves columns
   by name once per stream. Persist and exchange that row (Parquet, IPC)
   rather than bespoke per-leaf tables.
2. `from_arrow_reader` is tolerant of shape: any subset of columns, any order,
   any case, extra columns ignored, castable columns cast by one plan compiled
   before the first batch. A `marketdatakind` column and, for a dated leaf,
   `transunix` are the minimum: an undated `ORDR`, `QUOT` or `EXEC` row is an
   order, a quote or an execution, a dated one the event; `TRAD` and `BOOK`
   must be dated, and a `BOOK` row is a complete book where its `alive` cell
   is a list (even an empty one), a delta book where `alive` is null and
   `delta` or `events` a list, and a snapshot control where all three are
   null; a table storing a null list as an empty one reads an empty `alive`
   beside a non-empty `delta` or `events` and no `snapunix` as a delta book,
   so a row recording only an execution is a delta book, never a control. An
   `EXEC` item in `delta`, or an `ORDR` or `QUOT` one in `events`, is refused
   by path. A `BOOK` row
   holding an `executions` entry is refused there: that column is a trade's.
3. Views are `Plan`s run by the expression engine: `apply_view` binds once
   against the reader's schema and streams; `plan()` shows the text. Add a
   nested fact as a column with a lift (`identifiers['clordid'] as clordid`,
   a map read by its `src:type` key) instead of post-processing rows. The `lifecycle` view collects (it orders).
4. Feed `BookIterator` a **sorted** stream (by `snapunix`, else `transunix`); it
   leaves an operation dated before its book out with a warning, so an unsorted
   stream loses operations without an error. `FixCodec.market_data` is the
   sorted door for a FIX capture; `EventIterator(sorted=false)` sorts a finite
   stream itself.
5. One book per instant and book key that moved it. The key is the input's
   instrument ISIN where it holds one, whatever its rank, else its non-empty
   ticker, else `XX0000000000` (`Isin::NONE`, the ISIN that states none) - so
   one instrument is one book wherever its ISIN is known, and a ticker-only
   input joins it once a lifecycle's instruments learned the pair; an FX
   pair's ISIN is its minted number (`3:0:QYLTVIRYHNX5`). A book opens
   keyed and takes its ticker and its ISIN from the first input stating each.
   An entry restated under another key - stated by its ticker, then under its
   ISIN - leaves the book it stood in by a `REMOVED` delta and opens in its
   new one at the same instant, so an entry rests in one book at a time. A
   book is yielded where its instant recorded a `delta` or an `events` entry,
   or at a snapshot tick where it holds an entry (a snapshot emptying a book
   is yielded too, empty and complete, its control in `events`); an instant that only repeats what the book holds yields none.
   Depth persists; `delta` carries only that instant's orders and quotes
   (`orddelta` and `quotes` partition it) and `events` its executions and
   snapshot controls (`executions` and `controls` partition it), each in the
   order applied; `ordlive` reads the orders resting on a complete book. An
   instant that recorded only an execution yields a delta book whose `delta`
   is empty. A trade or a batch is pruned and an undated leaf or a book
   refused before the fold.
   A positive `snapshot_millis` adds the complete live book
   at every crossed epoch-aligned tick.
6. A book row nests `delta` and `events` (their rows in the order applied)
   and, on a complete book, `alive` and `bidlimits`, `asklimits` (one `Limit` per price
   level, best first: `price`, `quantity`, `uuids`, `tradable`) - null on a
   delta book; its `executions` cell is null. A book rests an order on the
   side it takes and a quote on every side it states a leg for: a two-sided
   quote is one entry, listed once by `alive` and on each side by
   `alive_on`, at that leg's price. A level trades unless every entry at it
   states `tradable = false`; `best_price`, the book's `bidpx`/`askpx`, the
   spread and the crossed and locked readings read the first level that
   trades, and every book - delta or complete - answers them from the top of
   book it settled on.
7. Join and chain by identity: `crossuuid` is one chain whatever identifier an
   event used; a later event joins a live one of its side and its
   `marketdatakind` through the type and value of a chain identity
   (`orderid`, `clordid`, `quoteid`, `tradeid`..., whatever its source, never
   an `execid`) - a quote's name alive on the side it tags, so a bid and an
   offer going by one name are two chains - and one stating no side joins the
   single side alive under its code; one citing two chains joins neither. Name identifiers there, as `Identifier`s,
   rather than inventing a column.
8. Leaves are immutable in the bindings: `with_previous`, `merge_with`,
   `restating`, `with_book`, `with_operations` answer a new value; only
   `with_previous` and `merge_with` answer `None`/`null` when nothing moved.
   Build with named facts: a fact given as `...`/`undefined` is skipped,
   `None`/`null` clears it; a derived identity (`uuid`, `crossuuid`,
   `hashcode`, `crosshashcode`) is refused.
9. Sources are provenance: `srcuuids` never changes identity, never travels
   along a chain, and merges as a sorted union - use it to point back at the
   lines an event was read from. A book states none (`srcuuids` empty,
   setting it keeps nothing): its provenance is the events it holds. Its row
   writes `srcuuids` null, and null for every entry nested in `alive` - that
   entry is the one the `delta` of the book that applied it holds, whose row
   writes its sources - so read the sources off the `delta_serie` and
   `events_serie` rows, never off a book's `alive`.
10. Numbers are exact decimals: pass `Decimal('189.5')` in Python and the text
    `'189.5'` in JavaScript - a float price (`189.5`) is refused at `$.price`
    (`got f64`). Python answers `Scalar` (`.as_py()` -> `Decimal`), JavaScript
    exact text (`'189.5'`), Rust `Decimal`.
11. Candles bucket by a zone's wall clock. `CandleOptions` carries the interval
    (`30s`, `1m`, `5m`, `1h`, `1d`, `1w`, sub-second `ms`/`us`/`ns`) and a
    `Timezone`, UTC by default, so a daily candle opens at local midnight and
    hourly candles follow a saving-time change: the hour a spring-forward skips
    yields no candle, the hour a fall-back repeats is one two-hour candle, the
    day is 23 or 25 hours. Feed `CandleIterator` books sorted by `transunix`
    (a `BookIterator`'s are); a regression is refused at `$.book.transunix`.
    One candle per book cross code and bucket, 23 cells (`Candle.field()`):
    `crosscode`, `ticker`, `start`, `end`, then `bid`, `ask`, `mid` and
    `spread` each `{open, high, low, close}` over the books that stated one,
    `bidqty`/`askqty` the last book's touch and `books` how many folded; an
    empty bucket yields no candle. A candle reads a book's top of book alone,
    so delta books fold as complete ones do.

## Pitfalls

- `hashcode` and `crosshashcode` read back from any layout a table stored
  them in: a whole `decimal(20, 0)` as the number, an `int64` cell as its
  bits - the `long` an Iceberg column stating `FIELD:representation=bits`
  holds - at the root and in `alive`, `delta`, `events` and `executions`, every
  identity still verified against the rebuilt leaf. `MarketData::field()`
  states no declaration, so `into_scheme_compat` widens unless the caller
  states it on the digests at every depth (`set_field_by_path`).
- A millisecond or second timestamp where nanoseconds are expected lands in
  1970: `graph.OrderEvent(1_700_000_000_000, ...)` is 28 minutes after the
  epoch. Multiply to nanoseconds first.
- `crosscode` answers the stored code: a buy order set to `O-1` reads
  `10:1:O-1`, and the lifecycle view and any lookup name it that way. A trade
  built on that order reads `21:0:O-1` and a quote `Q-1` reads `14:0:Q-1`
  whatever side it tags: neither is sided.
- Python enum facts are `IntEnum` members (`order.side is Side.BUYS`); JavaScript
  getters answer the name (`'BUYS'`) while an Arrow column stores the code
  (`Side.BUYS === 1`). Compare against the one you hold.
- `BookIterator` over an unsorted list is no error: each operation dated
  before the book it would fold into is left out with a deduplicated warning,
  and the books lack it; sort first, or take the operations from
  `FixCodec.market_data`. The walk leaves out, never fails on, a group the
  book refuses; only a source's own failure ends it, and so does a value no
  book folds (the next bullet).
- A book folds dated orders, dated quotes and snapshot controls, and
  records a dated execution among its `events`, resting on no side. A trade or
  a batch is pruned - no error, no book, no instant - and a filter
  (`with_filter`, `filter=`) narrows what is left, never admitting them
  back. An undated `Order` or a `BookEvent` is refused - by
  `BookIterator` at `$.operation.kind`, by `with_operations`/`add_operations`
  at `$.operations[i].kind`. An order or a quote resting on neither the bid
  nor the ask (an order of side `UKNW`, a quote stating no leg, a leg sized zero) is
  placed nowhere, with a warning, and still counts among the book's `delta`.
- A grid multiplies: every tick `snapshot_millis` crosses yields every live
  book complete, each repeating every alive entry it holds, so a fine grid
  over deep books is `alive entries x ticks` nested rows whatever the input's
  size. A batch of books is bounded by the codec's row and byte bounds
  (`with_batch_row_size`, `with_batch_byte_size`, each nested row charged)
  and casts to a table's stored layout whole; to hold fewer rows, coarsen the
  grid or narrow the fold with `with_filter`.
- A delta book (`is_complete` false) answers `alive`, `alive_on` and
  `limits` empty and `depth`/`imbalance` as none, and `with_operations`/
  `add_operations` refuse it at `$.alive`: rebuild it with `with_previous`
  over the complete book before it first. `with_previous` answers
  `None`/`null` where it cannot rebuild over the book given: another key, or
  not the book its `prevuuid` names.
- `EventIterator` defaults to `sorted=True` / `true` and trusts the order: an
  unsorted stream is not refused, it silently yields broken chains (an
  element before the live one follows nothing, though it carries its chain's
  side and code). Pass `sorted=False` / `false` for a stream you have not
  sorted (it collects to sort); only `BookIterator` notices a regression, and
  leaves it out with a warning.
- A trade is built only through `TradeEvent.from_parts`: at least one
  execution, each on any side - `UKNW` included - at the root's instant,
  one ticker, distinct cross codes.
- A lift names an identifier by its key, and keys are lower case:
  `identifiers['clordid']`, never `['FIX:ClOrdID']` (that reads null).
  `securityids['isin']` reads the ISIN a leaf took without a source.
- An identifier map is no dict: compare `str(id)` / `id.toString()`, read
  `get(type)` - the base key's value, whichever source stated it - and
  `get_from(key)` (`"ullink:isin"`), or cross it with `into_dict()` /
  `intoObject()`. An `Identifier` is its key and value: `key` is `src:type`, the
  type alone for the base source, and `Identifier("fix:clordid", value)` is the
  base `clordid`.
- An `Instruments` holds one `Instrument` element per instrument, keyed by its
  cross code - a security by its real ISIN alone, everything no agency
  numbers by `class:body` (`IF:EUR/USD`, `OC:US0378331005:2026-12-18:200`,
  `KE:<leg>+<leg>`), minted a `QY` number - `uuid = crossuuid`, the code alone
  moving the identity, a placeholder under a venue's ISIN re-keyed once its
  body arrives with the old code in `aliascodes`. It fills what an element
  leaves unsaid about its instrument from what earlier elements stated -
  keyed by the ISIN, the code the element's facts spell (an FX spot's), a
  minted number, a code of `LOOKUP_CODES` (a CUSIP, a SEDOL, a FIGI, a RIC, a
  Bloomberg symbol) or a ticker on its market leading back to it, and a short
  name in the element's currency only under `set_economic_match(true)`;
  `resolve(element)` names the instrument, its tier and whether the key was
  derived, or why none - an ISIN or a code it lacks (`UnknownIsin`,
  `UnknownCode`, ending the cascade), a key two instruments hold
  (`Ambiguous`), a CFI or origin-currency conflict, a score below the
  threshold - as `derived` identifiers, so a filled code reads back
  `is_derived`, plus the ticker, the CFI code and the listing's currency as
  market facts, and the instrument's cross code as `instcode`, which a reader
  joins the instruments table on; a valid stated value fills and replaces
  whatever the time; an instrument also holds its `underlying` and `legs` (as
  codes), its `characteristics`, its `eusipacode` - a structured product's
  EUSIPA category, `int32`, read as an `Eusipa` (`yggdryl-types`) - and its
  `metadata`, a FIX lifecycle learned - never the bindings' `learn` - which
  `merge` takes as stated. Its listings are nested, one per market: the
  instrument's facts (`cficode`, `fisn`, `underlying`, `eusipacode`,
  `updunix`, `firstunix`, `lastunix`, `origccy`, the non-listing codes) are
  its own, the ticker, currency and listing codes one market's; a listing
  fact stated on no market lands on the single listing, or with a warning on
  none of several. Every `learn` states `firstunix` and `lastunix`, so meeting
  a known instrument earlier or later than before dirties the collection and
  the next `commit()` writes it, one row per instrument. A store laid out
  before the instrument row is refused by name: drop and recreate it. The
  bindings' `learn`/`fill`/`enrich` take a `FixMsg`, a FIX lifecycle runs
  them on every message, and a parse fills the identifiers from the table its
  door fixed. Bind one to a store with `from_url` and write it back with
  `commit()`.
- `MarketData.kind` is `order_event` for a dated order; the leaf's own `kind`
  is `order`; both stand under `marketdatakind` `ORDR`.
- An order's `price` is what it states, never its last execution and never a
  bid: read `lastpx`/`lastqty` for fills, `bidpx`/`askpx` for the bid and ask
  a quote or a book states.
- `fxrates` maps a target currency to the rate an amount in the element's
  `currency` is divided by; nothing fills it, and a merge unions the targets.
- A leaf read from a FIX message keeps its parties and `Account(1)` in
  `partyids`, not `metadata`, and its `identifiers` can hold more than the
  message's: an identifier-like `metadata` key (`marketorderid`) is lifted into
  it (`market:orderid`); see the `yggdryl-fix` skill. A FIX `ExecutionReport`
  of no fill - a venue's acknowledgement, cancel, reject or expiry - is its
  order's leaf (its quote's, where it names a `QuoteID(117)`), so a venue
  cancel takes the entry off its book; a filling one is that report plus an
  `EXEC` leaf, which no book folds.
- The lifecycle view needs its chain: `apply_view("lifecycle", source,
  crosscode="10:1:O-1")`; every other view refuses a `crosscode`.
- Rust's `Limit` value type, the column enums' verbs (`EventColumn::fact`,
  `record`) and the `insert_`/`remove_`/`derive_` identifier verbs are
  Rust-only; the bindings answer a limit as a struct `Scalar` (Python) or a
  plain object (JavaScript) and state identifiers when they build a leaf.
- `graph.candles` and `CandleIterator` take `BookEvent`s, or `MarketData`
  holding one: a bare order is refused (`$.kind: expected book_event, got
  order_event`). A one-sided book states no `mid` and no `spread`, and a
  bucket whose ask side empties keeps the ask readings its earlier books made
  while `askqty` reads `None`/`null`, because the touch is the last book's.

## Language references

- Rust: [references/rust.md](references/rust.md) - `yggdryl::graph::*` leaves, traits, candles, `Decimal`, `Side`, `MarketDataKind`, `Ccy`.
- Python: [references/python.md](references/python.md) - `from yggdryl import graph`, `Side`, `MarketDataKind`, pyarrow readers, `graph.candles`.
- JavaScript: [references/javascript.md](references/javascript.md) - `graph`, `Side`, `MarketDataKind`, `graph.candles`.

Read the one for the language you write; recipes appear in the same order in each.

## Deeper

- Graph overview and traits: https://platob.github.io/yggdryl/graph/
- Element (identity, cross code, sources): https://platob.github.io/yggdryl/graph/element/
- Event (instants, following, merging, the walk): https://platob.github.io/yggdryl/graph/event/
- Market facts, fill or overwrite, order quantities, security identifiers: https://platob.github.io/yggdryl/graph/market/
- `Identifier` and `Identifiers`, the `IdType`/`IdSource` vocabularies, parentage, FIX party naming: https://platob.github.io/yggdryl/graph/identifier/
- The three row schemas, column by column: https://platob.github.io/yggdryl/graph/schemas/
- Operation facts, identifiers and party ids: https://platob.github.io/yggdryl/graph/operation/
- Order, quote, execution leaves and book control: https://platob.github.io/yggdryl/graph/order/
- Trade: https://platob.github.io/yggdryl/graph/trade/
- Book, limits, snapshots, the fold: https://platob.github.io/yggdryl/graph/book/
- `MarketData`, columns, Arrow row, views: https://platob.github.io/yggdryl/graph/market-data/
- Candles, buckets and zones, the candle row: https://platob.github.io/yggdryl/graph/candle/
- `Side`, `MarketDataKind`, `MarketDataType` and `TimeInForce`: https://platob.github.io/yggdryl/types/enum/
- Sibling skills: `yggdryl-fix` (FIX captures into market data and books),
  `yggdryl-expressions` (the `Plan` a view is), `yggdryl-records` (persisting
  `marketdata` batches), `yggdryl-arrow` (`BatchReader`, casts),
  `yggdryl-hashing` (the digests and UUIDs identities are built from),
  `yggdryl-types` (`Decimal`, the codes `ccy` and `forex`, the enums `side`,
  `marketdatakind`, `state`).
