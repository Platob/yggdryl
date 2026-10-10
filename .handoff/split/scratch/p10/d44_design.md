# P10: design (D44) - the instrument registry by code and market, robust inference, coalescing

The user's instruction (2026-10-10, `$S/user_decisions.md:73-85`, decision 17, typos the user's):
"Make instrument registry unique by intrument code and mic since an instrument can be used on other
markets adding default mic. Ensure it correctly updates the fix message. Ensure fix codec parsing
checks correct instrument informations like checking option type when strike price given, or future
... find informations to make robust inference from straight isin to fallback rules from the internet
and fix registry, with also coalesce validated instrument informations like bloomberg, ric, mic,
currency, country, strikepx, etc ... and propagates in life cycles leverage instrument
centralization". Four later decisions name P10 too: 18 (`:87-94`, the `InstrumentEvent` and the
registry's versions), 19 (`:95-98`, versions time-sorted and deduplicated), 22 (`:120-129`, hourly
snapshots, expiry inferred, the expiry instant in the instrument's country's zone) and 23 (`:130-132`,
rows unique by `(transunix, instcode, mic)`) - the last two recorded at commit `9e6db7dc0`
(08:07:53Z) after this document's first write (08:06:15Z), so D44.13 and D44.14 below were added for
them and D44.10/D44.11 reconciled with them before P10a. `$S` is `.handoff/split/scratch`.

Placement (`user_decisions.md:7,61-72`): after P9 (the Instrument, in the working tree,
`$S/p9/d42_design.md`, `$S/p9/impl_log.md`) and P9b (`$S/p10/p9b_design.md`, decision 16), before
P7 (D40), P8 (D41) and P5R; everything inside `yggdryl-market` and `yggdryl-fix`, the bindings
re-spelled, nothing added to Node (AGENTS §4). Read on the mid-P9 working tree
(`git status` of 2026-10-10 ~07:45Z, P9 uncommitted; every line number below is of that tree and
is re-read on P9's commit - the line numbers already known to have drifted: `is_listing` is
`idtype.rs:599-613`, the medallion's drop check `medallion.py:292`, the market allocation pins
`allocations.rs:2056` and `:2256`, `a_book_crosscode_allocates_nothing_for_any_input` `:893`). No
command was run to write this: every claim names the file and line it was read from, or the URL of
the public source the evidence lane read it from - and that lane could fetch no page but
`raw.githubusercontent.com` (DNS refused `en.wikipedia.org`, `www.onixs.biz`, `fiximate.fixtrading.org`,
`www.iotafinance.com`, `www.esma.europa.eu`, `www.cmegroup.com`, `ibkb.interactivebrokers.com`,
`forum.fixtrading.org`, `anna-web.org`; the OpenFIGI CSV refused by the egress proxy), so every URL
cited here but the Orchestra one is a **search-summary fact**: P10a opens with a phase 0 that fetches
each and records what it says against the table row that rests on it (Implementation plan), and a row
no fetched source supports is dropped or marked so in the code's comment. The one fetched source,
`OrchestraFIXLatest.xml`, confirms field 461 "recommended instead of SecurityType(167) for non-Fixed
Income" and defines 207 as the place of listing and 30 as the market of execution (D44.5 step 6).

**Two commits, one design** (recommended, "Put to the user" 1): **P10a** - decisions 17 and the
flat `(instcode, mic)` row holding one version per key; **P10b** - decisions 18 and 19, the version
history over that same row. The row's shape is laid out once, in P10a, with the event columns
already in it, so nothing is re-pinned twice.

## The user's asks, each mapped to a decision

| # | The ask (the user's words, the task's numbering) | Decision |
| --- | --- | --- |
| 1 | "unique by intrument code and mic ... adding default mic" | D44.1 (the model: one `Instrument` element per code, its `(instcode, mic)` rows the listings, `XXXX` the default market, never taken over), D44.2 (the flat stored row, one per `(instcode, mic)` version), D44.3 (what owns each fact) |
| 2 | "Ensure it correctly updates the fix message" | D44.4 (what a fill writes onto a message from its `(instcode, mic)` row, and how a stated value outranks a filled one) |
| 3 | "checks correct instrument informations like checking option type when strike price given, or future ... robust inference from straight isin to fallback rules from the internet and fix registry" | D44.5 (the ladder at the parse: a real ISIN, the CFI, `SecurityType(167)`, the fields stated, the symbol grammars, the market; every contradiction a named anomaly; typed once at intake) and D44.6 (the one defect found: the options-on-futures CFI) |
| 4 | "coalesce validated instrument informations like bloomberg, ric, mic, currency, country, strikepx" | D44.7 (validity before order: a stronger-validated statement replaces, an equal one warns or fills; the explicit `merge` the one authority that replaces) |
| 5 | "propagates in life cycles leverage instrument centralization" | D44.8 (the central fill before the chain step; what a follower takes from the instrument and what from its chain; `names_other_instrument` by `instcode`) |
| 6 | the cost | D44.9 (zero allocations per row on a hit, the ladder a table probe, the pins) |
| 7 | the interplay with P7 and P8 and the order | "Interplay and order": P9b, P10a, P10b, P7, P8, P5R - the place before P7 put to the user (#14) |
| 18 | "Create then the IntrumentEvent leveraging the Instrument and make registry use intrumentevent to store versions" | D44.10 (`InstrumentEvent`, an `Event`; the registry's versions; the append-only commit) |
| 19 | "registry keeping correct time sorted deduplicated" | D44.11 (versions read in `transunix` order, a committed row never rewritten, an out-of-order arrival folded under validity then time, one version per instant per key) |
| 22 | "push snapshots every hours and handle correctly expired instruments ... inferring at maximum the maturity dates ... instrument country to timezone" | D44.13 (the commit cadence one hour of event time; the maturity inferred, the expiry instant the maturity's end of day in the country's zone, an `EXPIRED` version, a statement after expiry an anomaly) |
| 23 | "unique by unix instcode and mic since its events now" | D44.14 (the row's key `(transunix, instcode, mic)`, `seqnum` 0, one version per instant per key) |

## What the tree holds today (the facts the design moves)

| Fact | Where |
| --- | --- |
| The registry is unique by cross code alone: `InstrumentTable.rows: Arc<BTreeMap<Str, Instrument>>`, the indexes `isins`, `aliases`, `tickers`, `codes`; no index holds a MIC | `rust/market/src/instrument.rs:3111-3133` |
| The market lives only in the nested `Listing { miccode: Option<Mic>, ticker, currency, codes }`, `XXXX` read as `None` (the "unlisted" listing), `MAX_LISTINGS` 6, `MAX_LISTING_CODES` 2, sorted by MIC | `rust/market/src/listing.rs:46-67`; `instrument.rs:620-622,765-769,1202-1216` |
| A statement naming no market: `Unlisted` where no listing, the single `Listing(0)` where one, **`Withheld`** (dropped with one warning per column) where several; the first market learned **takes over** the unlisted listing | `instrument.rs:2370-2402` (`listing_target`, `Target`), `:2407-2426` (`warn_withheld`), `:1618-1669` (`put_listing`) |
| Rank-aware update today: identifiers (`Identifiers`' base rule, `IdType::rank`), the ISIN (`Isin::rank_of`), the CFI (`Cfi::refined`), the country (the ISIN's own prefix taken back); **last statement wins** for `origccy`, `currency` (a body's), `eusipacode`, `fisn`, `multiplier`, `exercise`, the listing's `ticker` and `currency`, and every `metadata` key | `instrument.rs:2443-2651` (`folded`); `$S/p9/d42_design.md:686-689` (the CS/equity flip) |
| The one-row-per-instrument store row, 25 columns, `PARTITION:by truncate(crosscode, 2)`, `SORT:by crosscode`; commit a whole overwrite where `moved && differs(table, stored)` | `instrument.rs:66-116,633-690,913-931`, `:3808-3912`; `instrument/store.rs:393-464` |
| A market keeps only a **detailed** CFI: `set_cficode` refuses a coarse code, `FixMsg::classification` answers none for `ESXXXX`, `167`/`460` alone "reach no detailed code" | `rust/market/src/graph/facts.rs:1161-1187`; `rust/fix/src/cfi.rs:18-21,112-158` (`:156` the `is_detailed` filter); `rust/src/cfi.rs:605-611` |
| The class an instrument is keyed by is the market's `cficode`'s two letters, so a coarse code keys nothing | `instrument.rs:386-393` (`Keying.class`), `:532-554` (`spell_code`), `:4562-4590` (`learn_stating`) |
| The native rules already derive `461` from `167` (`CS` → `ESXXXX`, `OPT` + `201` → `OC`/`OP`, `FUT` → `FXXXXX`, ...) and `167` from `461`, to a fixpoint - the crate's own crosswalk, its comment citing "Appendix 6-D" | `rust/fix/src/native_derivations.rs:25,449-506,565-642` |
| No consistency check: `StrikePrice(202)` is set as `strikepx` whatever the class; a `201` disagreeing with a stated `OC`/`OP` is silently ignored; `1194` is read as a fact and compared to nothing; `947` is not read | `rust/fix/src/msg.rs:2612-2614`, `:4097-4150`; `rust/fix/src/cfi.rs:136-157` |
| The market is settled `LastMkt(30)`, `ExDestination(100)`, the instrument key's MIC, `SecurityExchange(207)`, then `XXXX` for a pair; the RIC's exchange suffix resolves to a MIC in the core but no door reads it for the market | `msg.rs:3110-3140` (`stated_miccode`); `rust/src/mic.rs:335-470` (`from_reuters_exchange_code`), `rust/src/ric.rs:83-107` (`exchange_code`) |
| The lifecycle: `learn_and_fill` (central: learn, then `fill_unsettled`) runs **before** chain following; `chain_market` copies `securityids`, `strikepx`, `origccy`, `instcode` from the predecessor unless `names_other_instrument`, which reads the two **ISINs** | `rust/fix/src/enrich.rs:735-803,834-842`; `rust/market/src/graph/market.rs:1310-1395,1485-1494` |
| What a fill writes: derived identifiers (the listing's codes only where `listed`), the ticker, the CFI, the currency (both markets stated and equal, the ticker equal, the element's none), `origccy`, `instcode`; never `miccode`, `strikepx`, a country | `instrument.rs:3279-3296` (`listed`, `listing_for`), `:3680-3792` |
| The seed: 208 instruments, 209 listings, 17 indices with no `miccode` (`CH0009980894` SMI, `EU0009658145` SX5E, ...), HSBC (`GB0005405286`) the one two-listing instrument | `config/instruments/instruments.json`; `scripts/check_instruments_seed.py:25-27,221-223` |
| The capture: `207=XSWX` (5 lines), `30=XSWX` (5), `30=RJEA` (1), `100=XSWX` (4), `167=CS` (6), `461=ESVTFR` (4), `461=ESXXXX` (1), bridge `SECURITYTYPE=equity` (59) and `option` (1), `BLOOMBERGCODE` `NOVN SW` (6), `HOLN SW` (3), `2454 TT Equity` (3) | `rust/tests/support/ulbridge.log` (grep counts, 2026-10-10) |
| The dictionary: FIX 5.0.2 EP 309; `167` string over `securitytypecodeset` (194 entries by group: Derivatives 34, Money Market 38, ...), `200` `fixed_ascii(8)` MonthYear, `201` `int32` `putorcallcodeset` (0 Put, 1 Call, 2 Other, 3 Chooser), `202` `decimal128(38,18)`, `207` `mic`, `231` `float64`, `460` `int32` `productcodeset`, `461` string, `470` `country`, `541` `datetime64(ns)`, `947` `ccy`, `1193` `settlmethodcodeset` (C/P/E), `1194` `exercisestylecodeset` (0 European, 1 American, 2 Bermuda, 99 Other), `1304` the same set | `config/fix/provenance.json`; `config/fix/fields/*.json`; `config/fix/codesets/*.json` |
| The core's ISO 10962:2021 table (SIX's `cfi-20210507-current`), listed options `OC`/`OP` attributes `["ABE", "BCDFIMNOSTW", "CENP", "NS"]`, futures `FF` `["BCDFIMNOSVW", "CNP", "NS", ""]`, spot `IF` `["", "", "", "P"]` | `rust/src/cfi.rs:12-22,222-258,334-346` |

## D44.1 - the model: one element per code, its `(instcode, mic)` rows the listings, `XXXX` the default market

Three readings of "unique by instrument code and mic" were weighed:

- **A - keep the nested model and key the listings by `Mic`** (recommended for the element):
  `Instrument` stays the one `Element` per `instcode` (`uuid = crossuuid`, the key no fact moves,
  panel D42.1/D42.4), and its listings are its `(instcode, mic)` rows: `Listing::miccode: Mic`,
  **required**, `Mic::none()` (`XXXX`) the default market of a statement naming none, one listing
  per `Mic` in a `SmallVec` sorted by code, `listing_at(&Mic)` the one binary search
  (`instrument.rs:1210-1216` keeps its shape with a required key). One owner per fact: an
  instrument-level fact is held once, a listing-level fact once per market (D44.3).
- **B - one `Instrument` element per `(instcode, mic)`**: refused for the element - two elements of
  one code would share `uuid = crossuuid` and `Element::merge_with` (`rust/src/graph/element.rs:348-358`,
  equal uuids required) would fold them; `aliascodes`, the placeholder re-key and the
  `underlying`/`legs` cascade (`instrument.rs:4472-4526`) are keyed by code; every instrument-level
  fact would be held once per market. The judge panel refused this shape for the same reason
  (`$S/p9/crosscode_decision.md:9,304-305`; `d42_design.md:136-146`).
- **B' - A in memory, B in the store**: the stored row is one per `(instcode, mic)`
  (**recommended**, D44.2): a reader of the registry table - the medallion, a SQL user, Doris
  (decision 20) - wants `(instcode, mic)` as the primary key, which is what "unique by code and mic"
  says in table terms; the nested `listings` serie is what makes a listing fact unreachable from a
  `where mic = 'XLON'`. The row repeats the instrument-level facts on every market's row as
  projections, exactly as `isin`, `cficode`, `forexcode` and `fisn` are projections of `securityids`
  today (`instrument.rs:1075-1116`), and the load folds the rows of one code back into one
  `Instrument`.

Decided: **A + B'**. The element is the aggregate every fill reads at the cost it costs today (one
`BTreeMap` probe, one listing binary search, no allocation); the rows are the registry's table. The
`Element` identity of the aggregate stays the panel's; the row's identity is the `InstrumentEvent`'s
(D44.10), one per `(instcode, mic)` version.

**The default market `XXXX`.** `Listing::new(market: Mic)` takes the market as it is; there is no
`Option` and no "unlisted" listing: `Mic::none()` (`rust/src/mic.rs:71-90`, rank zero) is a market
like any other in the map, the one a statement naming no market lands on. `listing_target`
(`instrument.rs:2370-2388`) becomes two arms - the statement's market, created where absent
(`Target::New(at)`), else `XXXX`, created where absent - and `Target::Unlisted`, `Target::Withheld`,
`warn_withheld` (`:2391-2426`) and `put_listing`'s takeover arm (`:1640-1656`) are deleted: a listing
fact is never dropped and never moves from one market to another, since "the first market learned"
is an inference about where an instrument trades that no statement made. The seed's 17 indices spell
no `miccode` and read as `XXXX` at the one intake (`instrument/seed.rs`; `check_instruments_seed.py:25-27`
re-spelled: "a listing of no `miccode` is the default market `XXXX`, at most one"), stored as `XXXX`.

**What is unique.** `(instcode, mic)` is the key of the listing rows and of the store's rows; the
`tickers` index keeps `codes_by_ticker(ticker, market)` (`instrument.rs:3218-3240`): a ticker on a
stated market answers the listings on that market, else the listings on `XXXX`; a ticker with no
market answers any listing, two instruments ambiguous as today. The `codes` index (`LOOKUP_CODES`,
`:3953-3974`) keeps one slot per instrument, since a listing code names one instrument whichever
market it is stated on; `Resolution::Matched` gains the market read (below), so a reader knows which
row answered.

**`Resolution::Matched { listing: bool }`** (`instrument.rs:2945-2963`) becomes
`market: Option<Mic>` - the `(instcode, mic)` row the match read: the element's stated market where
the instrument lists it; `XXXX` where the element states none and the instrument holds the default
row; the instrument's **single** listing where it states none and holds exactly one (a seeded
instrument met by a line naming no market - today's `listing_for`, `:3288-3296`, kept for reads);
`None` where the element names a market the instrument does not list, and no listing fact is filled
(the instrument's facts are). **The read order for an element naming no market**: the instrument's
single non-`XXXX` listing where it holds exactly one and the `XXXX` row holds no listing fact (no
ticker, no trading currency, no code), else `XXXX` where it holds one - so a seeded single-listing
instrument met by a market-less line keeps answering its real listing's ticker and currency after
a market-less learn created its `XXXX` row. **The `XXXX` row is created only by a listing fact
stated with no market** - a ticker, a `Currency(15)` on a body-less instrument, a listing code
(`BLOOMBERGCODE=NOVN SW`) - never by a statement of instrument facts alone, so a market-less line
stating an ISIN and a price makes no row. Python `Resolution.listing` → `Resolution.market` (`str | None`),
Node `listing` → `market` (`python/src/instrument.rs:858-866`, `node/src/instrument.rs:86-91`;
re-spelled, not added).

**Bounds.** `MAX_LISTINGS` 6 → **7**: the six markets plus the default one, so an instrument on six
venues keeps its `XXXX` row. **Past the bound** a new market is passed over as `put_listing` passes it
over today (`instrument.rs:1659-1663` answers `false`, silently): P10 makes it one `warned!` per key
naming the market and the bound (`"instrument listing passed over: {code} lists {MAX_LISTINGS}
markets, {mic} is one more"`), the instrument's facts still learned, and pins it
(`a_market_past_the_listing_bound_is_warned_and_passed_over`); a US listed option trades on more
venues than seven, so a higher bound for derivatives is put to the user (#19) rather than taken.
`listings` becomes `SmallVec<[Listing; 2]>` (`instrument.rs:623`, inline 1 today): under D44.1 the
capture's three instruments met both on a market and on none - `CH0012005267` (1 line `207=XSWX`,
7 none), `CH0012221716` (4 `XSWX`, 45 none), `TW0001605004` (1 `30=RJEA`, 2 none) - hold two
listings where the takeover kept one, and an inline capacity of one would spill each to the heap
inside `FIX_PIPELINE_COSTS`' lifecycle row (D44.9); whether the market-less lines of each state a
listing fact at all (a `BLOOMBERGCODE` whose `SW` suffix the Bloomberg rung of step 6 resolves to
`XSWX` once the OpenFIGI table is in creates no `XXXX` row) is counted at phase 1 and written in the
pin table. `ENTRY_COST` (`instrument.rs:147-169`) is re-derived with the wider `Instrument`, the
seventh listing and the version stamps of D44.10 (`MAX_VERSION_STAMPS * size_of::<VersionStamp>()`),
and the implementing session states the resulting `ENTRY_CHARGE`: 32 KiB where the `const` assertion
holds, else the rise to 64 KiB put to the user with its number (#21) - never a silent bump.

## D44.2 - the stored row: one row per `(instcode, mic)` version, flat

The row (`InstrumentEvent::field()`, D44.10; today's `Instrument::field()`, `instrument.rs:913-931`,
is retired - the stored row is the event's) in order:

| Band | Columns |
| --- | --- |
| element (6) | `uuid`, `crossuuid`, `crosscode` (= `instcode`, shared by every market and version of one instrument), `hashcode`, `crosshashcode`, `srcuuids` - `ElementColumn::ALL` (`rust/src/graph/element_column.rs:57-73`) |
| event (9) | `transunix` (the version's instant: the statement's `transunix` that moved the content - today's `updunix`; a tombstone's and an expiry's the instant D44.10/D44.13 name), `creaunix` (when the key was first met - today's `firstunix`), `sendunix` (**the registry's clock when the version was written**: the greatest `transunix` the registry had learned, equal to `transunix` for a version arriving in order and later for one arriving late - the merge reference, so the current row of a key is its greatest `sendunix`, D44.11), `exprunix` (the expiry instant once inferred, D44.13, else null), `prevunix` (the `transunix` of the version this one was built over), `snapunix` (null), `prevuuid` (the version this one was built over - the key's current version when it was learned, never rewritten, D44.11), `seqnum` (0: one version per instant per key, D44.14), `state` (`NEW` for a key's first version, `UPDATED` after, `EXPIRED` for the expiry version, `REMOVED` for a tombstone, D44.10/D44.13) - `EventColumn::ALL` (`rust/src/graph/column.rs:60-80`) |
| instrument (15) | `aliascodes`, `placeholder`, `isin`, `cficode`, `forexcode`, `fisn`, `countrycode`, `currency`, `origccy`, `securityids`, `underlying`, `legs`, `eusipacode`, `characteristics`, `metadata` - as today (`instrument.rs:66-116,633-690`), the four projections included, repeated on every market's row |
| listing (4) | `miccode: mic` **required** (`XXXX` written), `ticker`, `listingccy` (the trading currency on this market; `currency` above is the instrument's, a body's or an FX pair's quote leg, `d42_design.md:423-427`; none for an `I*`/`J*`/`S*` class, D44.7), `codes` (the listing codes of this market, `Identifiers::dtype()`) - flattened out of today's `listings` serie (`listing.rs:70-78`) |

Thirty-four columns - 6 + 9 + 15 + 4 - pinned as `field_len() == 34`. There is **no `lastunix`
column**: a stamp that moves with no content is no event, so it is the aggregate's in-memory fact
alone (`Instrument::lastunix()`, the Python `dict`'s `lastunix`, D44.3) and never written - a reader
wanting the latest statement of a key reads the events that cite it. `PARTITION:by
["truncate(crosscode, 2)"]` stays (`instrument.rs:633`: the ISIN's country prefix or the class
letters); `SORT:by ["crosscode", "miccode", "transunix"]` - the key, the market, then time, so the
snapshot streams a key's versions together and in time order (D44.11), the table's
`identifier-field-ids` the three key columns of D44.14 where the store is an Iceberg table. P7's renames (`d40_design.md:111-137`:
`cficode` → `cfi`, `countrycode` → `country`, `forexcode` → `forex`, `eusipacode` → `eusipa`,
`miccode` → `mic`) apply to these columns uniformly at P7 (`listingccy` is the one name P7 does not
touch; "Interplay").

A store written under P9's nested row is refused at load naming the table and saying to drop it, by
the rule and door D42.13 already has for the 46-column registry row (`instrument.rs:4728-4760`: the
check reads `miccode` at the top level and the absence of `listings`); the medallion drops and
recreates its table (`python/tests/medallion.py:245-269`, the `"crosscode" not in table.field()`
test becomes `"miccode" not in ...`), silver replayed from bronze (no back-compat).

`Instrument::into_scalar()`/`from_scalar()` (the named struct with `listings` nested,
`instrument.rs:1781-1946`) stay as the **aggregate's reading**: what Python's `Instruments.get(key)`
answers as a `dict` and what `merge(dict)` reads (`python/src/instrument.rs:84-131,586`), what Node's
`get` answers; neither is a stored row. `Instruments.field()` in both bindings answers the event
row. `Instruments::rows()` (`instrument.rs:4045`) counts the current `(instcode, mic)` rows, `len()`
the instruments, as today's doc says ("rows the listings").

## D44.3 - what owns each fact

| Fact | Owner | Level |
| --- | --- | --- |
| the key `instcode`, `placeholder`, `aliascodes`, `underlying`, `legs`, `characteristics` (settle, settle2, expiry, strike, multiplier, exercise) | `Instrument` (panel D42.1/D42.5) | instrument |
| the class (`cfi`), the ISIN, the pair, the short name, every non-listing identifier (`cusip`, `wkn`, `valor`, `lei`, `dti`, the minted `yggdryl:isin`, every named source's statement of one) | `Instrument::securityids` (D42.8) | instrument |
| `countrycode` (of issue), `currency` (a body's), `origccy`, `eusipacode`, `metadata` | `Instrument` holders | instrument |
| the market `mic` (`XXXX` the default), the `ticker` on it, the trading currency on it, the listing codes `IdType::is_listing` names (`ric`, `bbg`, `exchsymb`, `cta`, `sedol`, `figi`, `mktassigned`, `fim`, `umtf`, `instrumentid`; `rust/market/src/idtype.rs:555-569`) | the `Listing` of that market | listing |
| the version's instant, state, predecessor, place, the registry clock at its write | `InstrumentEvent` (D44.10) | row |
| `lastunix` | the aggregate, in memory (a stamp, no version, no column) | instrument |

A listing code stated on **no** market lands on the `XXXX` listing (today it lands in `securityids`
until a market is learned and then moves, `instrument.rs:2560-2583,1618-1639`; that move is gone
with the takeover: a RIC `EUR=` on an FX pair is the pair's `XXXX` listing's code, which is where a
market-less instrument's listing facts live). `Instrument::get(&IdType)` (`:1231-1242`) keeps reading
`securityids` then the listings in MIC order, so `get(&IdType::Ric)` on an FX pair still answers
`EUR=`; the pinned test `a_listing_code_held_on_no_market_has_one_holder_once_its_market_is_stated`
(`rust/market/tests/root/instrument.rs:2082`) is re-spelled: the code has one holder, the `XXXX`
listing, and a later statement on `XLON` is a second listing holding its own code - nothing moves.

## D44.4 - the FIX message filled from its `(instcode, mic)` row

The fill is `InstrumentTable::fill_unsettled` (`instrument.rs:3737-3792`), reached by
`FixMsg::fill_instrument` (`msg.rs:4009-4019`) in `learn_and_fill` (`enrich.rs:735-803`) and by
`Instruments::fill` for any market element. It reads one row - `Resolution::Matched.market`'s
listing - and writes **market facts through the `Market` setters alone**: a FIX message's setters
set the event's fact and mark it `stated` so no later settle restates it (`msg.rs:7069-7078`); no
wire field and no digest moves ("a fill moves nothing its identity reads", `msg.rs:4009-4019`).
**Where a filled fact shows**: the user's "correctly updates the fix message" is read as the FIX
row the medallion persists (`silver.fix_messages`, `lifecycle_arrow_reader`), so the table below
names for each fact the cell it reaches - a crate-field column (`rust/fix/src/crated.rs`: `ticker`
65_035, `strikepx` 65_036, `securityids` 65_021 with its `isincode`/`bloombergcode`/`figicode`/
`forexcode` projections 65_022/65_050/65_051/65_049, `instcode` 65_054, `origccy` 65_018) is written
from the event's fact and shows the fill; a dictionary field's cell (`Currency(15)`, `CFICode(461)`)
is the wire's as the message stated it and shows no fill unless the row writer (`schema.rs`) reads
the fact - the implementing session reads it for `currency` and `cficode` at phase 3 and, where they
stay off the row, that is put to the user (#15), never changed silently; the wire bytes
(`into_text`) never move. One `lifecycle_arrow_reader` test pins the filled cells row by row
(`a_fill_from_the_instrument_row_shows_in_the_fix_row`): a line stating an ISIN alone, after a line
of the same instrument stated its ticker, Bloomberg code and strike, reads back with those cells
filled and `Currency(15)` as it stated it. **One rule for which row may fill an element naming no
market**: the row D44.1's read order answers - the single non-`XXXX` listing where `XXXX` holds no
listing fact, else `XXXX` - fills every listing fact alike (the codes, the ticker, the trading
currency); there is no fact-by-fact exception. What it writes, each only where the message states
none (`overwrite = false`; a stated value always outranks a filled one - that is the whole rule, and
it is already how every setter reads `overwrite`, `rust/market/src/graph/market.rs:147-375`):

| Fact | From | Today | P10 | FIX row cell |
| --- | --- | --- | --- | --- |
| derived `securityids` of every type the message lacks (the ISIN where it stated no real one, the national numbers, the FISN, the LEI) | the instrument | yes | unchanged | `securityids` 65_021, `isincode` 65_022 |
| the listing codes (`bbg`, `ric`, `figi`, `sedol`, ...) | the matched listing | where `listed` | where `market` is `Some`: the stated market's row, else the row the read order answers | `securityids`, `bloombergcode` 65_050, `figicode` 65_051 |
| `ticker` | the matched listing | where `listed` and none stated | unchanged in rule, the row by `market` | `ticker` 65_035 |
| `cficode` | the instrument's class, `Cfi::refined` | yes | unchanged; a coarse instrument class fills a message stating none (D44.5 lifts the detailed gate) | `CFICode(461)` is the wire's: read at phase 3, put to the user where it stays off the row (#15) |
| `currency` | the matched row's trading currency | both markets stated and equal, the ticker equal, none stated | the matched row's `listingccy` where the message states none; the ticker condition goes (a venue's ticker is a listing fact, not a precondition) - put to the user (#6); **never on an `I*`/`J*`/`S*` instrument**, whose `Currency(15)` is the dealt currency of that trade and whose rows hold no `listingccy` (D44.7) | `Currency(15)` is the wire's: as `cficode` |
| `origccy` | the instrument | yes | unchanged | `origccy` 65_018 |
| `instcode` | the instrument's code | yes | unchanged (a clone of the one `Str`, D42.19); the alias chain's survivor where the message held an alias (D44.8) | `instcode` 65_054 |
| `strikepx` | `characteristics.strikepx` | no | **new**: filled where the message states none and the instrument is an `O*`/`H*`/`R*` class - the strike is a key fact of the instrument the row names (the user's "strikepx") | `strikepx` 65_036 |
| `miccode` | - | never | never: the market is the message's own fact (where it executed, `stated_miccode`), `XXXX` is "unstated" to `set_miccode` (`facts.rs:1189-1200`), and the row the read order answers says where the instrument is listed, not where this message traded - the refusal to fill or propagate `miccode` is put to the user (#16), since the user listed `mic` among the facts to coalesce | `miccode` 65_023 stays the message's |
| country, multiplier, exercise, expiry, settle, the underlying | - | never | never on a market row (no `Market` holder for them; `expiry`/`settle` are the key's); a reader joins the instruments table on `instcode` | - |

`Resolution::Matched.market` is also what `derive_into` (`:3680-3714`) reads for the listing codes
in place of `listed`. The fill's cost is unchanged: one listing binary search more where the element
states no market and the instrument holds `XXXX` (D44.9).

## D44.5 - the inference ladder at the parse

Where it runs: `enrich_detected` (`enrich.rs:150-185`) - detection, the native rules to a fixpoint,
the views, one settle, the table's derived identifiers, one stamp - and every step below is a
function of the message alone (AGENTS Ownership: "parse enrichment depends only on that event and
the reference data its door fixed"), so it stays at the parse; what needs the table (the
underlying's code, a `(instcode, mic)` row) stays the lifecycle's (`learn_stating`,
`fill_unsettled`). Every value is read once by its field's type - `201` an `int32`, `202` a
`decimal128`, `541` a `datetime64`, `200` a `fixed_ascii(8)` read by `Expiry::from_text`
(`rust/market/src/characteristics.rs:145-179`), `1194` an `int32` by a new `Exercise::from_code(i32)`
beside `from_fix(&str)` (`characteristics.rs:246`, which reads text - the integer door is the one
`1194`'s landed cell takes, no text round trip), `947` a `ccy` - through the dictionary's own field
(`config/fix/fields/`), and nothing past the settle re-parses a cell. The ladder, in precedence:

| Step | Reads | Answers | Source |
| --- | --- | --- | --- |
| 1 key | a stated real ISIN (`Isin::rank_of == 2`: canonical, closed, listed prefix, `rust/src/isin.rs:208-238`) | the instrument's key, its country of issue where the prefix is a country (`StringEnum::COUNTRIES`, none for `XS EU EZ XA XB XC XD XF XK XS XT`, `isin.rs:44-49`), the embedded national number; **never** the type, the market or the currency | ISO 6166 (https://en.wikipedia.org/wiki/International_Securities_Identification_Number; https://anna-web.org/faq-page/ for the `XS`/`XA-XD` agency prefixes; https://www.anna-dsb.com/ufaqs/dsb-scope/ for `EZ`); today's `derive_instcode` (`msg.rs:4038-4069`) and `country_of_issue` (`native_derivations.rs:652-659`) |
| 2 class of record | `CFICode(461)` stated, a bridge's `DETAILEDCFICODE` folded into it (`fix/cfi.rs:32-49`); `PutOrCall(201)` the **group** of a listed option (a stated `OM` → `OC`/`OP`, `fix/cfi.rs:58-66`; a stated `OM` with no `201` stays the message's class but **keys no body** - `Keying.class` reads `OM` and `HM` as a group not yet known, so the series keys by its real ISIN or nothing until the group arrives and never holds an `OM:` and an `OC:` code at once); `ExerciseStyle(1194)` attribute 1 of an `O` class (`0` European → `E`, `1` American → `A`, `2` Bermuda → `B`; `99` nothing); `SettlMethod(1193)` the delivery attribute (`C`/`P`; `E` → `E` where the group lists it: `OC`/`OP` position 5 `CENP`) | the classification, **classified** (category and group valid) - the detailed gate (`fix/cfi.rs:156`, `facts.rs:1161-1187`) is lifted: a coarse `ESXXXX` is the class `ES`, which keys and refines (`Cfi::refined`, `rust/src/cfi.rs:657-686`) | ISO 10962:2021 (`rust/src/cfi.rs:12-22`; https://www.iotafinance.com/en/Attributes-CFI-Codes-Group-OC.html, `.../OP.html`); FIX Orchestra FIXLatest field 461 ("recommended instead of SecurityType(167) for non-Fixed Income", https://raw.githubusercontent.com/FIXTradingCommunity/orchestrations/099914dd0edd49a699326f0441776d6e21cfaf93/FIX%20Standard/OrchestraFIXLatest.xml); `config/fix/codesets/{putorcall,exercisestyle,settlmethod}codeset.json` |
| 3 coarse class | `SecurityType(167)` through the crate's crosswalk (`native_derivations.rs:565-642`, `cfi_code`), the one table, with `Product(460)` in **one role**: it narrows the group of a class the type left coarse and never names a category by itself. Rows that move or are added: `FUT` → `FCXXXX` where `460` is 2 (`COMMODITY`), else `FFXXXX` (today `FXXXXX`, which `Cfi::is_classified` refuses - `F` has only the groups `C` and `F` and no Others, `rust/src/cfi.rs`, so the market dropped every future); `OPT`/`OOP`/`OOC` → `OCXXXX`/`OPXXXX` by `201`, **no class** where `201` is absent or `2`/`3` (today `OXXXXX`; `OM` is a valid group but the crate never derives it - put/call unknown is "not yet keyed", so a series stated once without `201` keys by its real ISIN or nothing and takes its body when the group arrives, through the placeholder re-key, never two codes `OM:...` and `OC:...` for one series); `OOF` D44.6's; `WAR` → `RWXXXX` only where `460` is not 11 (`MUNICIPAL`) and no `D*` class is stated (the dictionary files `WAR` under Municipal, `securitytypecodeset.json`: a municipal warrant is debt); `MLEG` → none (a strategy's class is `K*` by its legs, `KE`/`KF`/`KR`/`KT`/`KC`/`KM`, written by `learn_stating` from the legs' classes, never from `167`); `FWD`/`EQFWD` → `JEXXXX` where `460` is 5 (`EQUITY`), `JFXXXX` 4 (`CURRENCY`), `JTXXXX` 2, `JRXXXX` 6 (`GOVERNMENT`) or 12 (`INTEREST RATE`?, read at phase 0 off `productcodeset.json`), else none; `FOR` → none (ambiguous `IF`/`JF`/`SF`); `IRS` → `SRXXXX`, `CDS` → `SCXXXX`, `CMDTYSWAP` → `STXXXX`, `TRS`/`RTRNSWAP`/`VARSWAP`/`CRLTNSWAP`/`DVDNDSWAP`/`PRTFLIOSWAP` → `SEXXXX` where `460` is 5, `STXXXX` 2, `SFXXXX` 4, `SRXXXX` 6, else `SMXXXX` (`S` has an Others group); `SWAPTION`/`CAP`/`FLR`/`CLLR` → `HRXXXX`; `FXBN` → `TCXXXX`; `FRA` → `JRXXXX` (resting on a search summary, phase 0); `CFD`, `SPREADBET`, `SPOTFWD`, `FUTSWAP`, `FWDSWAP`, `FWDFRTAGMT`, `EXOTIC`, `ETC`, `EQBSKT`, `BDBSKT`, `LOANLEASE`, `XMISSION`, `DIGITAL` → **none**, each named in the table's comment as a Derivatives-group type the crosswalk answers nothing for | a coarse class where none is stated; **no official FIX crosswalk exists** - the table is the crate's reading of the code-set definitions, and D44 says so in `native_derivations.rs`'s module doc (today's "Appendix 6-D" citation is only true of the equity rows: Appendix 6-D names `[CS]`, `[PS]`, `[n/a]` for `ES`, `EP`, `EM` and lists the option positions, https://www.onixs.biz/fix-dictionary/4.4/app_6_d.html, read through search summaries) | `config/fix/codesets/securitytypecodeset.json` (194 entries, the `group` labels; Derivatives: 34), `productcodeset.json`; https://fiximate.fixtrading.org/legacy/en/FIX.4.4/tag461.html; https://forum.fixtrading.org/t/cficode-acceptance/268 |
| 4 field shapes | over the facts that remain once steps 2 and 3 stated no class: `StrikePrice(202)` with `PutOrCall(201)` of 0/1 → `OPXXXX`/`OCXXXX`; `202` with no `201` → no class, the strike kept as `strikepx` (a strike with no class is an option-like of unknown group, not a contradiction); a maturity (`541`, else `200`) with no strike, a stated underlying (`309`/`311`), no `167` and no `461` → a future **only** beside a `Product(460)` of 2 (`FCXXXX`), 5 or 7 (`FFXXXX`: a single-stock or an index future); a maturity alone, or with any other `460`, proves nothing (a bond has one) - the `167`-of-the-Derivatives-group arm of the first draft is gone, since step 3 answers every `167` | the class, where steps 2 and 3 stated none | ESMA RTS 22/23: an option is `O*****`/`H*****`, FIRDS records option type, strike, exercise style, expiry and delivery type for them (https://www.esma.europa.eu/sites/default/files/library/esma65-11-1193_firds_reference_data_reporting_instructions_v2.1.pdf, phase 0); `config/fix/fields/000000002.json` (202 "Strike Price for an Option", 200 "used for standardized futures and options") |
| 5 symbol grammars | `Symbol(55)` as an OCC OSI symbol - 21 bytes: the root padded to 6, `yymmdd`, `C`/`P`, the strike × 1000 in 8 digits (`MSFT  100116C00047500`); as a futures code - a root, a month letter `F G H J K M N Q U V X Z`, one or two year digits (`ESZ25`, `ESZ5`) - read **only** beside a stated future class (`167=FUT` or `461=F*`), since the shape alone matches any ticker ending in a letter and digits; the FX pair through the existing `FxSymbol` (`rust/src/forex.rs:257-300`) | an OSI symbol fills `461` (`OC`/`OP` + `XXXX`), `541`, `202` and `201` where absent, as `derive_forex` fills `167`/`460`/`461`/`15`/`120`/`63` (`rust/fix/src/forex.rs:120-200`): written where absent or written by an earlier detection, retracted when the symbol changes; a futures code fills `200` (`YYYYMM`, the year's decade resolved against the message's own `transunix`); a `Symbol(55)` that is a ticker fills nothing | https://ibkb.interactivebrokers.com/article/972 and https://en.wikipedia.org/wiki/Option_symbol (OSI; "no single official OCC document specifies the format"); https://www.cmegroup.com/education/courses/introduction-to-futures/understanding-contract-trading-codes.html and https://www.barchart.com/trader/help/symbols/months.php (month codes) |
| 6 market | **two readings, two facts.** The message's own market fact - where it traded - stays `stated_miccode` (`msg.rs:3113-3140`): `LastMkt(30)`, `ExDestination(100)`, the instrument key's MIC, `SecurityExchange(207)` - each an ISO 10383 code or a FIX 4.2 Appendix C mnemonic (`mic_from_market`) - then `XXXX` for a pair. **The registry row's market** - where the instrument is listed, the `mic` of its `(instcode, mic)` row - is a new reading, `listing_miccode`: `SecurityExchange(207)` first (Orchestra: "the place of listing", fetched), then the key's MIC, then the RIC's exchange suffix (`Ric::exchange_code` → `Mic::from_reuters_exchange_code`, a listing code naming its listing's market; `.DE` is in neither Appendix C nor the table and resolves to none), then a Bloomberg exchange code where its table exists (below), then `30`/`100` only where none of those is stated and the venue is a listing venue - **never** an off-venue or segment-less code: `XOFF`, `XXXX`, `SINT` and every MIC whose `Mic::operating()` names no exchange are no listing row (`is_listing_venue`, a new predicate on `Mic` read once), so a multi-listed equity executed on an MTF or over the counter does not grow a `(isin, CEUX)` or `(isin, XOFF)` row and does not fill `MAX_LISTINGS` with venues. `learn_stating` reads `listing_miccode` for the row; `fill_unsettled` matches the element by `stated_miccode` first and `listing_miccode` second (D44.1's read order) | the message's market as today; the row's market one listing-first reading | FIX Orchestra FIXLatest (fetched: 207 "place of listing", 30 "market of execution"); FIX forum on 207 and Appendix C (https://forum.fixtrading.org/t/tag-207-securityexchange-reuters-exchange-code/11980, phase 0); `rust/src/mic.rs:335-470` |
| 7 defaults | the country: stated `470` where listed, else the ISIN prefix (step 1); the currency: stated `15`, else - for a class that is no `I*`/`J*`/`S*` - the listing's market's legal tender (`Mic::country`, `Country::currency`, `instrument.rs:1733-1744`); for `I*`/`J*`/`S*` the body's quote leg is the instrument's `currency` and `15` is the trade's dealt currency, a per-trade fact of the message and never a default, never learned as a listing currency (D44.7); never the ISIN prefix | what `derive_defaults` fills today, the FX arm narrowed | `rust/src/mic.rs:165-171`; `rust/src/country.rs:72` |

**Contradictions, each a named `FixAnomaly`** (`rust/fix/src/anomaly.rs:18-31`, the vehicle every
refused reading already takes, e.g. `msg.rs:675-830`) recorded on the message, one `warned!` per
field, the message kept as stated - never a refusal of the line (AGENTS: "best effort, then a named
refusal"; a line is market data whatever its instrument's terms say):

| Stated | Contradiction | Anomaly field | Kept |
| --- | --- | --- | --- |
| `202` (a strike) under a class `E*`, `D*`, `F*`, `I*`, `J*`, `S*`, `T*`, `L*`, `C*` | a strike belongs to `O`, `H`, `R` (warrants `RW`, mini-futures `RF`), to `K` (a single-strike strategy - a straddle - legitimately states `202`) and to `M` (Others: a class that says nothing of its terms refuses none), and to nothing else | `strikeprice` | the class; `strikepx` **not** set as a market fact (today it is, `msg.rs:2612-2614`); under `O`/`H`/`R`/`K`/`M` and under no class it is set |
| `201` under `OC`/`OP` disagreeing (`201=0` on `OC`) | call/put is the group | `putorcall` | the stated group (today silent, `fix/cfi.rs:150-155`) |
| `201` under `E*`/`D*`/`F*`/`I*`/`J*` | no put/call on a non-option | `putorcall` | the class |
| `1194` disagreeing with position 3 of a stated `OC`/`OP` (`1194=1` American on `OCEXXX`) | one exercise style per series | `exercisestyle` | the stated attribute |
| `1193` disagreeing with the stated delivery attribute (position 5 of `O`, position 4 of `F` - `FF`/`FC`'s second attribute `CNP` - position 6 of `J` and `S` - `JE` `["BFIOS", "", "CFS", "CP"]`, position 4 not applicable; `SE` `["BIMS", "CDLMPTV", "", "CEP"]`) | one delivery | `settlmethod` | the stated attribute |
| `167` whose crosswalk category differs from a stated `461`'s (`167=CS` with `461=DBXXXX`) | two categories | `securitytype` | `461`, the classification of record |
| `470` a listed country differing from a real ISIN's listed prefix | ISO 6166's prefix is the numbering agency's country; a differing country of issue is possible (a Dutch N.V. numbered by another agency) so it is a **warning**, not a refusal, and the stated country is held beside the key as today (`msg.rs:4212-4222`) | `countryofissue` | both |
| an OSI symbol's `C`/`P`, strike or expiry disagreeing with a stated `201`/`202`/`541` | one series | `symbol` | the stated fields; the symbol fills nothing |
| `200` and `541` naming different months | one expiry | `maturitymonthyear` | `541`, the day, as `stated_characteristics` already prefers (`msg.rs:4132-4135`) |
| `947` (`StrikeCurrency`) differing from `15` on an option whose underlying is no pair | read as a **fact** under the uuid, not a key byte (`d42_design.md:380`); a difference is no contradiction (a quanto) and is kept | - | both |

**What is typed once.** The crosswalk is a `match` over the folded `167` text (today's
`member_upper`, `native_derivations.rs:772-783`) - a table probe per message where `461` is absent,
nothing per row after the settle. The OSI reader is a fixed-width byte scan; the futures reader a
suffix scan under a class gate; both allocation-free, one root file per type (AGENTS "One type, one
file"; a theme file `symbology.rs` would be a folder-as-facade in one file): `rust/market/src/osi_symbol.rs`
holds `OsiSymbol`, `rust/market/src/futures_symbol.rs` holds `FuturesSymbol` and the `MonthCode` it
reads by, as `rust/src/forex.rs` holds `Forex` with `FxSymbol` and `FxTenor`; pinned in
`rust/market/tests/root/osi_symbol.rs`, `rust/market/tests/root/futures_symbol.rs` and
`rust/market/tests/allocations.rs`. A **Bloomberg exchange-code table** (`GY` → `XETR`, `LN` → `XLON`,
`UN` → `XNYS`, `UW`/`UQ` → `XNAS` segments, `JT` → `XTKS`, `IM` → `XMIL`; composites `US`, `GR`,
`IM` composite → **none**) is written **only** from the fetched OpenFIGI exchange-code CSV
(https://www.openfigi.com/assets/content/OpenFIGI_Exchange_Codes-3d3e5936ba.csv, refused by the
proxy on 2026-10-10; `FP` → `XPAR`, `NA` → `XAMS`, `SE`/`SW` → `XSWX` unconfirmed by search) - the
implementing session fetches it or step 6's Bloomberg rung is left out of P10 and named in the
handoff; a table from memory is not written.

**The class a market keeps** moves from "detailed" to "classified" (`facts.rs:1161-1187`:
`Cfi::is_classified` in place of `is_detailed`; `fix/cfi.rs:156` the same; `set_cficode`'s `refined`
unchanged). Why: the class that keys an instrument is its two letters (`Keying.class`,
`instrument.rs:386-393`, `class_of`), the native rules already derive `461` from `167` for every
message stating a type (`ESXXXX` for `CS`), and `Cfi::refined` fills attributes as later statements
arrive - so a coarse code is the fact "`ES`" and not "nothing"; the market's `cficode` column then
holds `ESXXXX` where it held null. The pins that move: **two** capture rows gain `cficode = ESXXXX`
and their `hashcode`/`uuid` move (`cficode` is fed, `market.rs:1161`; `docs/graph/market.md:21`) -
of the six `167=CS` lines of `rust/tests/support/ulbridge.log`, four already state `461=ESVTFR`
(detailed, `ABBN`), one states `461=ESXXXX` (`NOVN`, `35=8`) and one states `167=CS` alone (`35=F`,
`55=2454`), so only those two rows can move - the equivalence snapshot regenerated once with the
sentence "a classified coarse CFI is a market fact, D44", every other cell a defect;
`fix/cfi.rs:119-128`'s doc example (`ESXXXX` → `ES`), `rust/fix/tests/root/cfi.rs:138-159`,
`rust/fix/tests/root/enrich.rs:924` and `docs/graph/market.md:18` re-spelled. **What else reads
`is_detailed`**: `msg.rs:2551-2554`'s `debug_assert!` that the classification chain answers a
detailed code or none - re-spelled to `is_classified`, since it would fire in debug the moment the
gate lifts; `securityid.rs:194`'s detection of a bare six-letter CFI among security ids stays on
`is_detailed` (a coarse `ESXXXX` typed in a `SecurityID(48)` cell is too common a placeholder to
detect as a CFI); `IdType::Cfi.is_real` stays "detailed" (`idtype.rs:980`, the validity rank of D44.7,
2 detailed over 1 classified); the FX detector's `ITKXXX` check (`rust/fix/src/forex.rs:187`)
unchanged. **A coarse derivative class now keys a body**: `derive_instcode` (`msg.rs:4041-4071`)
writes the real ISIN as the parse-time `instcode` only under a class keying no body
(`Production::Isin`), so a real-ISIN line whose class is `FF`/`FC` (from `167=FUT`), `OC`/`OP`
(from `167=OPT` with `201`), `J*`, `S*` or `HR` by step 3 keys at the parse by its body where it
keyed by its ISIN - a placeholder where the body is not whole (`DE000C000001`-style), re-keyed by the
lifecycle when the body completes, which moves its book from `3:0:<isin>` to `3:0:<class:body>` by
P9b's withdrawal. Which lines: phase 3 greps the capture and the FIX tests for a real ISIN beside
`167` in {`FUT`, `OPT`, `OOF`, `OOP`, `OOC`, `FWD`, `EQFWD`, `IRS`, `CDS`, `TRS`, ...} or a `461` of
those categories - the capture's one bridge `SECURITYTYPE=option` line states `ISINCODE=EZN11TD1F7K3`
(an `EZ` placeholder prefix, rank 1, not real: it keys nothing today and nothing after) and no `201`,
so **no capture line moves**; a test line that does is listed in the pin table with its book move,
and the equivalence snapshot's `instcode` cells are read row by row for it.

## D44.6 - the options-on-futures defect, fixed here

`native_derivations.rs:464` reads `O?F` and `:588-590` writes `OCFXXX`/`OPFXXX`/`OXFXXX` for `OOF`:
position 3 of a listed option is the exercise style (`ABE`) and the underlying is position 4
(`BCDFIMNOSTW`), so `OCFXXX` is no classified code (`Cfi::parsed` refuses it, `rust/src/cfi.rs:614-627`)
and the reverse rule tests the wrong position. P10 writes `OCXFXX`/`OPXFXX` - and **no class** where
`201` is absent or `2`/`3` (`OM` is never derived, D44.5 step 3) - and reads `O??F` (the underlying
in position 4; `starts_case`, `native_derivations.rs:755`, takes `?` alone as a wildcard, so the first
draft's `O?.F` would have matched a literal `.` and never fired), ordered before the `O` arm,
re-spelling `rust/fix/tests/root/enrich.rs:628,663,924`, `docs/fix/capture.md:1210` and the one
capture row, if any, a derived `OCFXXX` reached (none: the capture states no `OOF`). The sibling
defect: `put_or_call` (`native_derivations.rs:508-517`) maps `HC`/`HP` to call/put, but in ISO
10962:2021 `H`'s groups are `C` Credit, `E` Equity, `F` Foreign exchange, `M`, `R` Rates, `T`
Commodities (`rust/src/cfi.rs`), so every credit option got `PutOrCall=1` and there is no `HP`; the
two arms go, and `H`'s call/put - position 4, `A`-`I` style and type - is not read (nothing asks for
it).

## D44.7 - the coalescing: validity before order

The user's "coalesce validated instrument informations": a stated value that **validates** stronger
than the held one replaces it; one that validates equally fills an empty fact and, where it differs
from a held one, is a **contradiction warned by name** for an instrument-level fact and a
**replacement** for a listing-level fact that a venue may move (a ticker, a trading currency); the
explicit `Instruments::merge` (a golden file, the seed, a caller's row) is the one authority that
replaces whatever the validity, so a wrong first statement is correctable. The `Statement`
(`instrument.rs:2131-2161`) gains `authority: Authority { Learned, Stated }` - `learn_stating` builds
`Learned`, `merge` `Stated`.

| Fact | Validity (`u8`, the core's doors; higher replaces lower) | Equal validity, differing | Where today |
| --- | --- | --- | --- |
| `isin` | `Isin::rank_of` (2 real, 1 closed or listed, 0) | the base rule: equal rank keeps the held (`identifier.rs:565-572`) | unchanged |
| `cfi` | `Cfi::ranked` (2 detailed, 1 classified, 0; `rust/src/cfi.rs:548-554`); `Cfi::refined` fills | another category or group: **replace** where `Stated`, **warn** `cficode` and keep where `Learned` (today: replace, `instrument.rs:2470-2487`) | moved |
| `mic` (the row's key) | `Mic::operating().is_some()` (ISO 10383 assigned, `mic.rs:123-131`) 2, shaped 1 (`Mic::is_iso`), `XXXX` 0 | a key, never merged and never filled onto a message (D44.4): two markets are two rows; the `(instcode, mic)` body leaves the multiplier, the settlement and the week out of an `F`/`O`/`H` body (`write_code`, `instrument.rs:415-480`: `{class}:{underlying}:{expiry}:{strike}`, `F` the expiry's month), so a weekly and a monthly, an E-mini and a standard contract on one index and month, `SPX` and `SPXW` on `XCBO` **collide on one row** and D44.7's rule warns `multiplier`/`exercise` and keeps the first - stated as accepted and pinned (`two_contracts_of_one_body_share_a_row_and_warn`), the alternative - the multiplier, the settlement and the expiry's day as key facts of a derivative body, a P9 key change - put to the user (#19) | new |
| `country` | `Country::is_listed` 1, else 0 (`country.rs:44`); the ISIN's own prefix taken back as today (`:2489-2500`) | **warn** `countrycode`, keep (`Learned`); replace (`Stated`) | moved from replace |
| `currency` (a body's), `listingccy` | `StringEnum::CURRENCIES` lists it 2, `Ccy::ranked` 1 (a digital ticker), `XXX` 0 | listing: **replace** (a venue re-denominates); body: warn, keep; **an `I*`/`J*`/`S*` instrument learns and fills no `listingccy`** - its `currency` is the body's quote leg and `Currency(15)` is the dealt currency of each trade, either leg, so a listing currency would flip with every trade | listing unchanged, body moved, the FX exclusion new |
| `origccy` | as the currency | **warn** `origccy`, keep; `Stated` replaces | moved from replace |
| `ticker` | 1 trimmed to the bound, 0 | **replace** (a venue renames) | unchanged |
| `bbg`, `ric`, `figi`, `sedol`, `cusip`, `wkn`, `valor`, `lei`, `fisn`, every identifier | `IdType::rank` of the type and the source's code (`identifier.rs:470-486`) through `Identifiers`' base rule; a RIC whose exchange suffix resolves to the row's market outranks one that does not (a listing-level validity read once at the learn) | the base rule's | unchanged in rule, the RIC check new |
| `strikepx`, `expiry`, `settle`, `settle2`, `underlying`, `legs` | key facts: never merged, a difference is another instrument (panel D42.5) | - | unchanged |
| `multiplier`, `exercise` | 1 stated | **warn** `multiplier`/`exercise`, keep; `Stated` replaces (today: replace, `:2521-2537`) | moved |
| `eusipacode` | `Eusipa::is_listed` 2, shaped 1 | warn, keep; `Stated` replaces | moved |
| `metadata` by key | 1 non-empty | **warn** the key, keep; `Stated` replaces (today: replace, the CS/equity flip of `d42_design.md:686-689`) | moved |

Warnings go through `warned!` (deduplicated per key: `"instrument fact contradicted: ..."`, the
column, `"{stated:?} against {held:?} under {code} on {mic}"`). **Under time order** (D44.11) "the
held value" is the one the current version holds, which is the latest in time; a statement earlier
than the current version fills what is empty, replaces where it validates stronger, and at equal
validity never replaces a listing-level fact either - so the answer does not depend on arrival order.
With this rule the capture's `securitytype` flips `CS`/`equity` once per instrument into a warning
and no fact moves, which is what makes D44.11's versions bounded on a replay for the instrument-level
facts (one version where today's rule would flip on every line); a listing-level `replace` fact - a
ticker or trading currency a venue really moves back and forth - is one version per flip, and the
bound on a replay holds only through D44.11's dedup, never through the rule.

## D44.8 - lifecycle propagation from the central registry

The order of a walk step today is right and stays: `Prepared::next` (`enrich.rs:834-842`) runs
`learn_and_fill` (the central learn and the central fill, under one lock, `:735-803`) **before** the
chain step (`LifecycleMessage` → `with_previous` → `following_operation` → `chain_market`,
`market.rs:1310-1395`). So every message takes its instrument's facts from the registry first, and
the chain copy (`securityids`, `strikepx`, `origccy`, `instcode`, `cficode`, `miccode`, `metadata`)
only fills what the registry could not - a message the table did not resolve (a ticker-only line
before its instrument is known). What moves:

- **`names_other_instrument`** (`market.rs:1485-1494`) decides "same instrument" first where both
  state one real ISIN and it is the same (today's rule, kept as the first arm), then reads the two
  `instcode`s: two stated codes that differ are two instruments (a follower never takes another
  instrument's `strikepx`, `origccy` or identifiers - the user's "propagates ... leverage instrument
  centralization"); where either states none, today's ISIN rule. **An alias is resolved where the
  registry lives, under its one lock**: `Codes::learn_and_fill` (`enrich.rs:735-803`) already holds
  the `InstrumentTable` for the learn and the fill; the fill answers the alias chain's survivor for a
  message whose held `instcode` is an alias (one `HashMap` probe of the table's `aliases`, read
  through a crate-private `InstrumentTable::survivor_of(&str)` beside `code_for`) and writes it
  through `set_instcode(.., true)` - the one place a message's `instcode` is overwritten, and only
  onto the survivor (panel D42.5's gold join `instcode in aliascodes`). So every message reaches the
  walk with its code already resolved and the chain step compares survivors: `LifecycleMessage`
  (`enrich.rs:509-511`, holding only the `FixMsg`) and `with_previous(self, &Self)` (`:576`, the
  predecessor immutable) change nothing and take no second lock - the first draft's probe before
  `with_previous` is gone, with the per-message lock it would have cost.
- **`strikepx` propagation** keeps its place in `chain_market` (`:1352-1356`) but is gated by the
  class as D44.5 gates the setter: a chain whose instrument is no option carries none.
- **The central fill writes `strikepx`** (D44.4), so a follower of an option chain whose first
  message stated the strike and whose later report states none takes it from the registry, not the
  chain - the same value either way; the registry answers first.
- The P8 index (`$S/p8/d41_design.md`) touches `iterator.rs` and `idtype.rs`; P10 touches
  `market.rs:1485-1494` and `enrich.rs`'s `LifecycleMessage` alone, so the two do not meet.

## D44.9 - the cost

- **Per row, on a hit: zero allocations.** The resolve is the probe it is today
  (`instrument.rs:3335-3424`: the packed ISIN `HashMap`, the spelled code's `&str` probe, the code
  index, the ticker index); the row read is `listing_at(&Mic)` over at most seven listings - one
  more than today where the element states no market and `XXXX` is probed first; every filled value
  is a clone of an inline code or one refcount (`instcode`, D42.19). The class ladder runs at the
  parse inside the native fixpoint, a `match` on a folded text; the OSI scan is 21 bytes on the
  stack; the RIC suffix rung is `Ric::exchange_code` (a byte scan) and one `match`
  (`mic.rs:335-470`).
- **A learn that moves nothing allocates nothing** (today's `folded` answering `None`,
  `instrument.rs:2443-2651`); a contradiction warned allocates its warning once per key
  (`warned!`'s deduplication, `rust/src/logging/repeats.rs`).
- **What the design itself adds, counted before P10a** (each a number the implementing session
  writes in the pin table at phase 1, before any pin is touched): (a) the second listing of the
  three capture instruments met both on a market and on none (D44.1) is held inline by
  `SmallVec<[Listing; 2]>`, so it allocates nothing - the price is a wider `Instrument` in
  `ENTRY_COST`, not an allocation in `FIX_PIPELINE_COSTS`; (b) a version is **not** an `Instrument`
  clone held per key: a learn that moves content builds the key's row once
  (`InstrumentEvent::current(&instrument, &mic).into_scalar()`, D44.10) and pushes it onto the
  registry's one pending `Vec<Scalar>` - the row's `Scalar` is one allocation per version plus its
  cells' heap - so `instruments_learn_a_new_instrument_into_its_row_inline` (1 today,
  `allocations.rs:2056`) rises by the row built, and `FIX_PIPELINE_COSTS`' lifecycle row by one row
  per version a capture line creates. **Neither pin is raised without the user's word**: the
  numbers are counted at phase 1 and put to the user (#21) with the alternative - the row built at
  commit from the aggregate for a key whose only version in the cadence is its current one
  (`versions` then a per-key stamp list and no row until the commit, the clone deferred), which keeps
  `learn` flat and costs the commit the rows it would have held anyway; D44 recommends that
  alternative, since a key moving once per hour is the common case and a key moving several times in
  one hour builds its intermediate rows at the learn only then.
- **Pins** (`rust/market/tests/allocations.rs`, each red first, at two corpus sizes): a thousand
  rows on the second market of a two-listing instrument fill allocation-free; a thousand rows naming
  no market on an instrument holding `XXXX` the same; the OSI reader and the futures reader zero; a
  contradicting statement (a differing `origccy`) zero after the first warning. **Must not move**:
  `rust/fix/tests/allocations.rs:1946-1986` `FIX_PIPELINE_COSTS` (`lifecycle` 22/16/16 as P9 leaves
  them: the ladder adds no allocation, the second listing is inline - the implementing session reads
  the number; a rise is the deferred-row question above, put to the user, never a re-pin), the FIX
  landing and batch rows, `instruments_learn_a_new_instrument_into_its_row_inline` 1
  (`rust/market/tests/allocations.rs:2056`, under the deferred row),
  `instruments_reload_known_rows_at_a_cost_per_batch` (`:2256`, [94, 94] today - the flat row lands
  cheaper than the nested one: a **fall** is re-pinned with its sentence, a rise is a defect), the
  snapshot drain (5/row + 3 today: the flat row has no nested runs, so the drain falls to the row's
  run plus its two maps - re-pinned with the sentence), `rust/market/tests/iobase_calls.rs`
  `mod instrument` (a load the calls of one record read; a commit one write - P10b's append is one
  write too).
- Benches: `cargo bench -p yggdryl-market --bench graph -- instrument --quick` before and after,
  direction only; `cargo bench -p yggdryl-fix --bench fix -- parse --quick` for the ladder.

## D44.10 - `InstrumentEvent` and the registry's versions (decision 18)

**The type**: `rust/market/src/instrument_event.rs` (one type, one file; the root re-exports it as
`yggdryl_market::InstrumentEvent`), pinned by `rust/market/tests/root/instrument_event.rs`
(declared in `rust/market/tests/root.rs` beside `instrument`, `:39-44`):

```rust
pub struct InstrumentEvent {
    instrument: Instrument,      // the facts as they stood at this version (the aggregate's clone)
    market: Mic,                 // the row's market: `XXXX` the default
    transunix: i64,              // the statement's instant that moved the content (today's updunix)
    creaunix: Option<i64>,       // when the key was first met (today's firstunix)
    sendunix: Option<i64>,       // the registry's clock at the write: the merge reference (D44.11)
    exprunix: Option<i64>,       // the expiry instant once inferred (D44.13)
    prevunix: Option<i64>, prevuuid: Option<Uuid>,  // the version this one was built over
    seqnum: u64, state: State,   // 0; NEW, UPDATED, EXPIRED, REMOVED
}
```

`InstrumentEvent` is the **builder and reader of one row**, never what the registry holds per key:
`current(&Instrument, &Mic)` builds it from the aggregate and `into_scalar()` lands it as one row
`Scalar` under `field()`; `from_scalar()` reads a row back and `into_instrument()` folds it into an
`Instrument` holding that one listing. The one clone of the aggregate lives for the row's building
and is dropped with it.

- `Element`: `crosscode` the instrument's code (every market and version of one instrument shares
  it, so `crossuuid` is the instrument's identity, the graph's rule for a chain,
  `rust/src/graph/element.rs:297-302`); `hashcode` the digest of the **row's** content: the
  instrument's `digest_instrument` (`instrument.rs:993-1050`, less its `listings` loop) continued
  with this market's listing (`mic`, `ticker`, `listingccy`, `codes`) - so two markets' rows of one
  instrument digest apart; the stamps, the clock and the sources never fed; `uuid` the event's
  **`time_uuid`** (`element.rs:1250-1253`: the instant, the place, the content code, the cross hash),
  so a version is identified by what it states and when - which is what makes a replay idempotent
  (D44.11); `finalize` = `sync_cross`, `digest`, `finalized(hashcode)` as `OperationEvent` does
  (`operation.rs:798-820`).
- `Event` (`element.rs:962-1040`): the nine getters and setters over the fields above; `is_execution`
  false; `with_previous`/`merging` the provided readings (`:1100-1160`), `restating` provided.
- **Not a `Market`** and no `MarketDataKind`: an instrument is no market datum; it sits in no
  `MarketData` variant and no book.
- **The aggregate stays.** `Instrument` keeps every door it has - the one element a fill reads, the
  `Element` with `uuid = crossuuid` - and gains `lastunix()` as its own in-memory stamp (D44.2); the
  load (`extend_from_arrow_reader`, `instrument.rs:4728-4875`) folds the rows of one code through
  `merge` as today's several-listing entry is folded (`:4292-4309`), the current row of each key -
  its greatest `sendunix` - first.

**The registry's versions.** `Instruments` holds **one** pending set for the whole registry, not one
per key: `pending: Vec<Scalar>`, the rows of the versions created since the last commit, each a row
`Scalar` under `InstrumentEvent::field()` built at the learn (or at the commit, under the deferred-row
alternative of D44.9, put to the user), bounded by `MAX_PENDING_ROWS` = 4096 rows (about a row's
cells' heap each - under 8 MiB in all, named here as the bound held state states): a registry bound to
a store that reaches it **commits early** (the hourly cadence of D44.13 cut short, nothing lost), and
one bound to no store passes the oldest row over with one warning naming the bound. Per key, beside
the aggregate, `stamps: SmallVec<[VersionStamp { transunix: i64, uuid: Uuid, hashcode: u64 }; 2]>`
of the versions the store holds and the pending ones, bounded at `MAX_VERSION_STAMPS` = **32** per key
(the oldest in time dropped from memory, the store keeping the row; 32 × 32 bytes = 1 KiB per key,
added to `ENTRY_COST` as `MAX_VERSION_STAMPS * size_of::<VersionStamp>()` plus the vector's header -
the pending rows are the registry's, so they are no entry cost); no `Instrument` is held per
version, so `ENTRY_COST` gains the stamps alone and `ENTRY_CHARGE` is stated after the `const`
assertion (D44.1's bounds paragraph, #21). A learn that changes the content of a key appends a
version (`state` `NEW` for the first the key ever had, `UPDATED` after, `prevuuid`/`prevunix` the
key's current version when it was learned, `sendunix` the registry's clock, D44.11); a learn that
moves `lastunix` alone appends none and writes nothing (the stamp is in memory, D44.2);
`remove(key, at)`/`remove_listing(key, mic, at)` take the instant `at` the caller states - a removal
is a statement with no wire clock of its own, so the instant is an argument, the bindings re-spelled
with `at` required (put to the user, #18) - append a `REMOVED` tombstone version at `at` (so the
store keeps the history and a reader knows the key ended) and drop the key from the aggregate;
`clear` empties the store on commit (today's `clear` contract, `instrument.rs:4338-4344`). The
current instrument of a key is the version of greatest `sendunix` that is not `REMOVED` (D44.11).

**The commit is append-only and a committed row is immutable** (D42.13's whole overwrite amended):
`commit` writes the pending rows as one `append_serie` under `InstrumentEvent::field()` - an Iceberg
table one appended snapshot, a **keyed** append where the table states `identifier-field-ids`
(`transunix`, `crosscode`, `miccode`, D44.14: a row whose key the partition already holds is declined
into `skipped_rows`, the store's own dedup), a leaf a rewrite (a leaf append is a rewrite, AGENTS
"IOMedia and records"), a plain folder one part per commit - and records their stamps; no committed
row's `prevuuid`, `prevunix` or any cell is ever rewritten, so nothing the store holds is read back,
compared or re-linked. `is_dirty` is "a pending row exists" (the `moved` precheck stays,
`instrument.rs:4065-4075`; `Stored`/`differs` (`:3808-3912`) are deleted with the whole overwrite). A
`clear` is the one whole write (an empty stream over the store). The load reads every row, orders
each key's rows by `transunix` (the `SORT:by` makes a committed store already ordered), folds the row
of greatest `sendunix` per `(instcode, mic)` that is not `REMOVED` into the aggregate and keeps the
stamps. `commit()` keeps answering `IOResult` (the rows appended, the keyed-declined ones skipped).

The medallion's `silver.record_keeping.instruments` holds every version: `rows` on the first run =
the seed's keys + the capture's keys (one version each), on a replay 0 appended
(`python/tests/test_fix.py:5846-5852`'s `IOResult(0, 0)` holds through D44.11's dedup: every
capture key has far fewer than `MAX_VERSION_STAMPS` versions, so no pending row is built and nothing
is pulled); the report line's `snapshots 1` becomes the count of hourly commits that appended
(D44.13: the capture spans one hour or several - counted from the capture at phase 1 and written in
the pin table, not re-pinned from the run). The medallion creates the table from `Instruments.field()`
as before (`medallion.py:292`'s drop check re-read at implementation); its docstring says one row per
`(transunix, instcode, mic)` version.

## D44.11 - time order and deduplication (decision 19)

Decision 19 is read with D44.10's immutable rows, and the reading is put to the user (#17):

- **Time order is the order a key's rows are read in**: `SORT:by ["crosscode", "miccode",
  "transunix"]`, each version's `transunix` the instant of the statement that moved the content. A
  version that arrives out of order (a replay of an older window, a late line) is **inserted at its
  place by that sort** and nowhere else: its own `prevuuid` names the key's current version when it
  was learned - what it was built over, decision 18's "the version it replaces" - and the versions
  after it in time keep the links they were written with. No committed row is rewritten, so a store
  can be append-only and a replay idempotent; the first draft's re-link of the later version is gone,
  since an append cannot rewrite a committed row and a re-finalized row would be a second row of one
  version.
- **The current version is the greatest `sendunix`**, the registry's clock at the write (the greatest
  `transunix` it has learned), not the greatest `transunix`: for a key whose versions all arrived in
  order the two agree; for a late arrival that moved content the late version is current, since it
  holds the fold. This is what a reader of the table does too (`max(sendunix)` per key), and what the
  load does.
- **One fold rule, validity then time.** A statement is folded onto the aggregate under D44.7 with
  "held" meaning the current version's value: a value that validates stronger replaces whatever the
  order; at equal validity a statement **later** than the current version fills the empty facts and
  replaces the listing-level `replace` facts, and one **earlier** than it fills the empty facts only -
  a listing-level fact included, since a newer venue statement is not undone by an older line - and
  warns a differing instrument-level one as D44.7 says. There is no re-fold of the versions after an
  inserted one and no stored content is re-read: the inserted version's row holds the aggregate as it
  stands after the fold (what the registry knows when it writes it, the later versions' facts
  included), so its `uuid` is its own and no other version's `uuid` or `hashcode` moves. The current
  instrument therefore holds every fact every statement gave it, whatever the arrival order, and an
  older statement never overrides a newer fact (today's `fold` applies statements in arrival order,
  `instrument.rs:4389-4466`: a replay of an old window could move a newer fact back - this closes it).
- **One version per `(transunix, instcode, mic)`** (decision 23, D44.14), `seqnum` 0: a second
  content move at one instant - two statements of one line, a batch's entries - is folded into the
  pending version at that instant, the later statement's content, one row; a content move at an
  instant whose version is **committed** (a late restatement of a stamped instant) is a contradiction
  between two statements of one instant: `warned!` `"instrument version restated: {code} on {mic} at
  {transunix} already committed"`, the facts folded onto the aggregate under the rule above and
  reaching the store with the key's **next** version (the committed row stands) - the loss where no
  next version ever comes is accepted and put to the user (#17).
- **Deduplicated**: a statement that changes no content of the key appends no version (today's
  `folded` → `None`); a statement at an instant the key's `stamps` hold with the same `hashcode` is
  a replay and appends nothing, so a replay of lines already committed builds no row and `is_dirty`
  stays false; **past the stamp window** (`MAX_VERSION_STAMPS` = 32 oldest-dropped) a replay of an
  older version builds its row again, which the Iceberg keyed append declines on the store's key
  (`skipped_rows`) and a leaf or plain-folder store appends as a duplicate row, the load keeping the
  one of greatest `sendunix` and warning the duplicate - stated, pinned
  (`a_replay_past_the_stamp_window_is_declined_by_a_keyed_table_and_folded_by_a_leaf_load`) and
  the reason the stamp bound is per key rather than per registry.
- **Pinned** (`rust/market/tests/root/instrument_event.rs`, `tests/instrument/store.rs`): three
  statements at `t1 < t2 < t3` arriving `t1, t3, t2` give three rows read back in time order
  `v1, v2, v3` with `v2.prevuuid == v3.uuid` (what it was built over), `v3.prevuuid == v1.uuid`,
  `v2.sendunix == t3`, and `v2` current - **after `v3` was committed**: the store read back holds
  exactly those three rows, none rewritten, and the aggregate holds `t2`'s fact beside `t3`'s; the
  same with `t2` filling a fact `t3` never stated and with `t2` restating a fact `t3` stated (the
  held one kept, one warning); a replay of the three appends nothing to a store that holds them; a
  flip `CS → equity → CS` at three instants under D44.7's rule is one version and two warnings (under
  `merge`, three versions); a late restatement of a committed instant is one warning and the next
  version carries the fact; a `remove(key, at)` is a `REMOVED` version at `at` and `get` answers
  none; the medallion's second run `IOResult(0, 0)`.

## D44.13 - hourly snapshots and expiry (decision 22)

- **The cadence.** The registry keeps an event clock: the greatest `transunix` any statement gave
  it (`Instruments::clock()`), which `sendunix` stamps (D44.10). A registry bound to a store commits
  its pending rows when a learn moves the clock across an hour boundary of event time -
  `floor(transunix / COMMIT_CADENCE_NS)` moving, `COMMIT_CADENCE_NS` one hour, `with_commit_cadence`
  the explicit knob on `Instruments` (`None` only at the walk's end) - at the end of the walk
  (`FixCodec::lifecycle`'s drop, `commit()` by the caller as today) and when `MAX_PENDING_ROWS` is
  reached (D44.10). So the medallion's instruments stage writes one Iceberg snapshot per hour of
  capture time, not per message batch, and its `snapshots` count is the capture's hours touched (the
  capture is read at phase 1 for the number). An unbound registry has no cadence. The cadence is
  pinned in `tests/instrument/store.rs` over a counting store: statements spanning three hours commit
  three times, two statements in one hour once, and `iobase_calls` `mod instrument` pins a commit as
  one append.
- **The maturity, inferred as far as the facts allow**, at the learn, into `characteristics.expiry`
  (`Expiry`, `characteristics.rs:145-179`, a key fact and so stated once): `MaturityDate(541)` the
  day; else `MaturityMonthYear(200)` with `MaturityDay(205)` the day, with a week code (`YYYYMMwN`)
  the week's last trading day, alone the month's **last calendar day** - a venue's rule (the third
  Friday of a listed option's or future's month) is a fact of the venue the crate does not hold,
  so it is not inferred and the choice is put to the user (#22); else `SettlDate(64)` for an
  `I*`/`J*` instrument; else a tenor (`FxTenor`, `rust/src/forex.rs`) from the trade's `transunix`;
  else none. The day a maturity names is the instrument's; **the expiry instant** is that day's end -
  the next day's `00:00` in the zone - in the instrument's own zone: the zone of its listing market's
  country (`Mic::country`, `mic.rs:165`) where the row names a market, else of its country of issue,
  through one new table `Country::timezone()` of one IANA zone per country (`rust/src/country.rs`,
  beside `country_currency`), read against the crate's bundled registry (`Timezone`); a country with
  several zones takes the zone of its principal financial centre (`US` → `America/New_York`, `CA` →
  `America/Toronto`, `AU` → `Australia/Sydney`, `BR` → `America/Sao_Paulo`, `RU` → `Europe/Moscow`) -
  the table written at phase 0 from IANA's `zone1970.tab` (fetched) with those overrides stated in
  the generator, put to the user (#20), never from memory; an instrument of no market and no listed
  country has no zone and no expiry instant (`exprunix` null, one warning per key).
- **The expiry event.** `exprunix` holds the expiry instant on every version of the key once
  inferred. When the clock moves past a key's `exprunix` (checked at each hourly tick over a
  `BTreeMap<i64, Str>` of the keys by expiry instant, a bound cost per tick, no scan), the registry
  appends a version at `transunix = exprunix` with `state = EXPIRED` (`State::Expired`, 9500,
  `rust/src/state.rs:136`, the lifecycle's "its time ran out"), `sendunix` the clock, and marks the
  aggregate expired (`Instrument::is_expired`); the key stays in the aggregate and its indexes (a
  late line still resolves it), and `remove` is what ends it. A statement after the expiry instant is
  checked against it: the message takes a `FixAnomaly` `expired` (`"instrument expired at
  {exprunix}: a statement at {transunix}"`), one `warned!` per key, the message kept and filled as
  any other; the aggregate learns the facts (a late correction is still a fact), and a version it
  creates after the expiry is `UPDATED` with `exprunix` kept. A snapshot tick thus publishes the
  expiries of the hour beside the hour's versions. Pinned: an option with `541` on `XCBO` expires at its day's
  end in the zone the table states for `US`, `America/New_York`;
  a `200` with a week code; an FX forward by tenor; a statement after expiry warned; the expiry row
  in the store with `state EXPIRED`.
- **Python/Node**: `Instruments.clock`, `with_commit_cadence(ns)` re-spelled onto the existing
  `Instruments` door; a `dict`'s `expiry`/`exprunix`/`is_expired`; nothing added to Node beyond the
  re-spelling (§4).

## D44.14 - the row's key `(transunix, instcode, mic)` (decision 23)

A row is one version of one `(instcode, mic)` at one instant: the key is `(transunix, crosscode,
miccode)` - `identifier-field-ids` on an Iceberg store, so a keyed append declines a key the
partition holds (D44.10), the uniqueness check of the load on every store (two rows of one key: the
one of greatest `sendunix` kept, one warning) - `seqnum` is always 0, and the `SORT:by` opens with
the two key columns and closes with the instant. Two statements at one instant are one version
(D44.11); `uuid` is `time_uuid` over the instant, `seqnum` 0, the content code and the cross hash, so
two rows of one key with one content are one `uuid`, and two with differing content (a late
restatement of a committed instant, D44.11) differ only in `hashcode`/`uuid` - the store's key
refuses the second where it can.

## D44.12 - bindings, docs, inventories, contract

- **Python** (`python/src/instrument.rs`, re-spelled, nothing added beyond what the core's surface
  moves): `Instruments.field()` the event row; `get(key)` the aggregate `dict` with its `listings`
  (each `miccode` a text, `XXXX` for the default); `get_listing(key, mic)`; `listings(key)`;
  `remove_listing(key, mic)`; `Resolution.market` (was `listing`); `rows` the current `(instcode,
  mic)` rows; `into_arrow_reader()` the versions stream; `versions(key, mic) -> list[dict]` **put to
  the user** (#9) - a reader can read them off the table; `merge(dict)` an authoritative statement
  (D44.7). `python/yggdryl/_native.pyi`, `__init__.pyi`, `instrument.py`, `tests/test_instrument.py`
  (the 36 tests re-spelled: `test_one_isin_on_two_markets_is_two_listings_and_every_learn_moves_lastunix`
  gains the `XXXX` row; new: the default market, a contradiction warned through `caplog`, the strike
  filled on an option's report, the versions appended and replayed), `typing_bindings.py`,
  `benchmarks/graph.py`; `python/tests/medallion.py:245-269` (the drop check, the docstring) and
  `test_fix.py:5690-5760,5846-5870` (the rows and snapshots pins).
- **Node** (§4): the existing `Instruments` door re-spelled onto the same surface (`node/src/instrument.rs:66-120,290-646`,
  `binding.js`, `binding.d.ts`, `tests/instrument.test.js` 21 tests, `benchmarks/graph.js`), the
  loader and declarations regenerated; nothing added.
- **Docs**: `docs/graph/instrument.md` - Contract (the key `(instcode, mic)`, the default market),
  The update rule (D44.7's table), Listings (`XXXX`), Matching (`market`), a new **Versions** section
  (D44.10/11) and **Classification** section (D44.5's ladder and anomalies), Persistence (append-only);
  `docs/fix/capture.md:1198-1240` (the derivation rows for `167`/`461`/`201`/`460`, the OOF fix,
  the anomalies); `docs/fix/message.md:839` (`classification`); `docs/graph/market.md:18,21`
  (a classified CFI is a fact); `docs/types/codes/cfi.md` (classified vs detailed);
  `docs/graph/market-data.md:188`; `docs/graph/schemas.md` (the instruments row); the skills
  `skills/yggdryl-fix/SKILL.md` and `references/*.md` (the instrument recipes), `skills/yggdryl-market-data`.
- **Inventories by hand**: `.api-inventory.txt` sections `yggdryl_market::instrument`,
  `::listing`, `::instrument_event` (new), `::osi_symbol` and `::futures_symbol` (new), `yggdryl`
  (`Country::timezone`), `yggdryl_fix` (`classification`, the anomalies), `.api-bindings.txt`
  (`Resolution.market`, `Instruments.field`, `remove(key, at)`, `clock`); `python scripts/check_api_inventory.py`.
- **AGENTS.md**: the Layout rows `instrument.rs` (the key `(instcode, mic)`, `XXXX`, the versions,
  the cadence, the expiry), `listing.rs`, a `instrument_event.rs` row, an `osi_symbol.rs` and a
  `futures_symbol.rs` row, `country.rs` (the zone table); the `graph/` row's
  `names_other_instrument` sentence; the `rust/fix/src/` row (the ladder, the anomalies, the
  classified class); Ownership ("Codec parsing and local per-event enrichment ...": the ladder is
  local, the row lookup the lifecycle's).
- **Generated**: no crate field is added, so the dump and the dictionary hash
  (`rust/fix/tests/root/store.rs:3557`, `857_326_152_662_128_339`) **do not move**; the equivalence
  snapshot moves on the rows D44.5 names; `docs/assets/fix.json` only if a `FixMsg` door it lists
  changes name.

## Tests and the pins that move, each with its sentence

| Pin | Moves to | Sentence |
| --- | --- | --- |
| `rust/market/tests/root/instrument.rs:758` `a_listing_code_lands_on_the_listing_and_a_foreign_number_is_kept`, `:2082` `a_listing_code_held_on_no_market_has_one_holder_once_its_market_is_stated` | a code on no market is the `XXXX` row's; a market learned later is a second row | "a statement naming no market lands on the default market XXXX, D44.1" |
| `:1111` `a_valid_statement_fills_and_replaces_whatever_the_time` | fills and replaces where it validates stronger; an equal differing `origccy`/`country`/`multiplier`/`metadata` warns and keeps under `learn`, replaces under `merge` | "validity before order, D44.7" |
| `:953`, `:1352`, `:1506`, `:1640` (round trip), `:2165` (the named row) | `Resolution.market`; the stored row flat and versioned; `Instrument::into_scalar` nested as the aggregate's reading | "the stored row is one per (instcode, mic) version, D44.2" |
| `rust/market/tests/root/listing.rs:16,81` | `Listing::new(Mic)`, `miccode()` → `&Mic`, `XXXX` stored | "the market is required, XXXX the default, D44.1" |
| `rust/market/tests/instrument/seed.rs:266` `an_index_states_no_market_and_no_short_name` | an index's one listing is `XXXX` | "D44.1" |
| `tests/instrument/store.rs` (26 tests: the layout, the two listings of one ISIN `:957,986`, the dirty rule `:276`, the Iceberg partition `:607`) | one row per key version; `a_fact_that_moves_and_moves_back_is_no_change_to_the_store` becomes "is one version and a warning" (D44.7) and "a replay appends nothing" (D44.11); the Iceberg store appended under its key, a `clear` whole; new: the out-of-order arrival after a commit read back in time order with no row rewritten (D44.11), the hourly cadence over a counting store (D44.13), the expiry row (D44.13), the stamp-window replay (D44.11) | "the commit appends versions, D44.10" |
| `rust/fix/tests/root/batch.rs` new `a_fill_from_the_instrument_row_shows_in_the_fix_row` | the `lifecycle_arrow_reader` cells a fill reaches (D44.4) | "D44.4" |
| `rust/fix/tests/root/ulbridge.rs`, `python/tests/test_fix.py` (the medallion `snapshots` line) | the count of hours the capture touches, **derived from the capture at phase 1** and written here before the run | "one commit per hour of event time, D44.13" |
| `rust/market/tests/allocations.rs:2143` the snapshot drain 5/row + 3, `:2184` the reload [94, 94] | re-pinned from the run **only downward** | "the flat event row has no nested runs, D44.2" |
| `rust/fix/tests/root/cfi.rs:138-159`, `rust/fix/src/cfi.rs:119-128` doc, `enrich.rs:924`, `docs/graph/market.md:18` | a classified coarse code is the class | "a classified CFI is a market fact, D44.5" |
| `rust/fix/tests/root/enrich.rs:628,663,924`, `docs/fix/capture.md:1210` | `OCXFXX`/`OPXFXX`, `O??F`; the `HC`/`HP` arms of `put_or_call` gone | "the underlying of a listed option is position 4, D44.6" |
| `rust/fix/tests/root/msg.rs:2551` (the `debug_assert!`), `rust/fix/tests/root/cfi.rs` | `is_classified` | "D44.5" |
| `rust/fix/tests/root/codec.rs:3535` equivalence snapshot | the **two** `167=CS` rows stating no detailed `461` (`NOVN` `461=ESXXXX`, `2454` `167` alone): `cficode`, `hashcode`, `uuid`; **no other cell** | "a classified coarse CFI is a market fact, D44.5" (a moved cell outside those rows is a defect; an `instcode` cell moved by a coarse derivative class keying a body is listed first or is a defect) |
| `rust/fix/tests/root/msg.rs`/`enrich.rs` new | the anomalies table of D44.5, each red first; an OSI symbol filling `461`/`201`/`202`/`541`; a futures code under `167=FUT`; the RIC suffix rung; the strike filled from the registry on an option's report | - |
| `python/tests/test_fix.py:5718-5722` (`instruments == registry.rows == len(registry) == len(codes)`), `:5736` (`field.index_of("listings")`), `:5846-5852`, the report line | `rows` = `(instcode, mic)` rows ≥ instruments; no `listings` column; `IOResult(0, 0)` on replay | "D44.2, D44.11" |
| `node/tests/instrument.test.js:232` `an instrument holds one listing per market` | the default market | "D44.1" |

**Must not move**: every `DataTypeId` byte, rank, `Shape`, value-stream byte and Arrow extension name
(S0); every market element's `crossuuid`/`crosshashcode`; every book identity P9b settled; the FIX
`FIX_PIPELINE_COSTS` and landing rows; the dictionary hash and the crate dump; the FX detection's
wire pins (`rust/fix/tests/root/forex.rs`); P9's D42.2 `crosscode`/`crosshashcode`/minted columns.

## Implementation plan

One commit per half (P10a, then P10b), one push each, CI read to `CI result`; phases in AGENTS order,
disjoint files per worker, the whole run leading the chain once the last cargo phase settles. The
compiler's list (`cargo check --workspace --all-targets --all-features --keep-going --message-format=short`)
drives every sweep.

### P10a - the key, the ladder, the coalescing, the propagation

| Phase | Files (disjoint) | Smoke (exact) | Pins expected to move |
| --- | --- | --- | --- |
| 0 the sources | `$S/p10/sources.md` (new, not in the tree): one row per URL D44.5 cites - ISO 6166 and the ANNA prefixes, ISO 10962's `OC`/`OP`/`FF`/`JE` pages, Appendix 6-D, the FIX forum threads, ESMA FIRDS, the OSI and month-code pages, the OpenFIGI CSV, IANA `zone1970.tab` - fetched with `WebFetch` and what each says written against the table row that rests on it; a row no fetched source supports is dropped from D44.5 or marked "crate's reading" in the code comment; the Bloomberg rung and the zone table are written only from the fetched files | the fetches read; `python scripts/generate_country_timezone.py --check` (new, the zone table's generator, `zone1970.tab` its input) | none |
| 1 market core: the key and the row | `rust/market/src/listing.rs` (`Mic` required, `listingccy` naming), `instrument.rs` (`listing_target` two arms, `Target::{Unlisted,Withheld}`/`warn_withheld`/the takeover deleted, `MAX_LISTINGS` 7, `Resolution::Matched.market`, `Authority` on `Statement`, D44.7's `folded`, `fill_unsettled` reading the row and filling `strikepx`, the flat row and its load/snapshot through `InstrumentEvent::current`), `instrument_event.rs` (new, one version per key in P10a: `state NEW`, `prevuuid None`), `lib.rs`, `implementer.rs`, `instrument/{store,seed}.rs` (the `XXXX` intake), `graph/market.rs:1485-1494` (`names_other_instrument` by `instcode`), `rust/src/country.rs` (`Country::timezone`, the generated `country/timezones.rs`); tests `rust/market/tests/root/{instrument,listing,instrument_event}.rs`, `root.rs`, `instrument/{seed,store,env}.rs`, `graph/market.rs`, `rust/tests/root/country.rs` | `cargo check -p yggdryl-market --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-market --test root instrument`; `--test root listing`; `--test root instrument_event`; `cargo test -p yggdryl-market --test graph market` (the `names_other_instrument` pins are `rust/market/tests/graph/market.rs`'s; `--test root market` is the core mirror); `cargo test -p yggdryl --test root country`; `--features "internals iceberg parquet" --test instrument`; `cargo test -p yggdryl-market --doc instrument::Instruments`; `RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-market --no-deps`; `python scripts/check_instruments_seed.py`; `python scripts/generate_internals.py --check` | the instrument tests named above; `field_len()` 25 → the count the row has |
| 2 market: the symbol grammars | `rust/market/src/osi_symbol.rs`, `futures_symbol.rs` (new), `characteristics.rs` (`Exercise::from_code(i32)`), `lib.rs`; `rust/market/tests/root/{osi_symbol,futures_symbol,characteristics}.rs`, `root.rs`; `rust/market/tests/allocations.rs` (new rows red first) | `cargo test -p yggdryl-market --test root osi_symbol`; `--test root futures_symbol`; `--test root characteristics`; `cargo test -p yggdryl-market --test allocations symbol`; `--test allocations instrument` | none |
| 3 FIX: the ladder | `rust/fix/src/cfi.rs` (classified, `1194`/`1193` attributes, the `201` anomaly), `native_derivations.rs` (the crosswalk rows, the OOF fix, the `HC`/`HP` arms gone, the OSI and futures rules - `RULE_COUNT` moves), `msg.rs` (the `strikepx` gate and anomaly, `listing_miccode` beside `stated_miccode`, `947` read as a fact, the anomalies of D44.5, the `debug_assert!` at `:2551` on `is_classified`), `enrich.rs` (`learn_and_fill` writing the alias survivor; `LifecycleMessage` untouched), `batch.rs` tests (the FIX-row cells a fill reaches), `forex.rs` untouched; tests `rust/fix/tests/root/{cfi,enrich,msg,market,codec,securityids,forex}.rs`, the snapshot regenerated once | `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-fix --test root cfi`; `--test root enrich`; `--test root msg`; `--test root forex` (must pass unchanged); `--test root market`; `cargo test -p yggdryl-fix --doc FixMsg::classification`; then `YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl-fix --test root equivalence` once, its diff read row by row; `cargo test -p yggdryl-fix --test root store` (the hash **must not move**); `cargo test -p yggdryl-fix --test allocations a_real_line_costs_the_same_at_every_stage_every_time` (must not move) | the snapshot's six rows; the OOF pins |
| 4 Python | `python/src/instrument.rs`, `python/yggdryl/{_native.pyi,__init__.pyi,instrument.py}`, `python/tests/{test_instrument.py,test_fix.py,medallion.py,typing_bindings.py}`, `python/benchmarks/graph.py` | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_instrument.py -x -q`; `... -m pytest python/tests/test_fix.py -k medallion -x -q`; §3's `mypy --strict` line | the medallion pins |
| 5 Node | `node/src/instrument.rs`, `node/{binding.js,binding.d.ts}`, `node/tests/{instrument.test.js,instrument.types.ts}`, `node/benchmarks/graph.js`; then `node/index.js`, `node/index.d.ts` | `npm run --prefix node build:debug`; `node --test node/tests/instrument.test.js`; `npm run --prefix node test:package:debug` | the loader and declarations |
| 6 docs manifests | `docs/assets/{fix,playground}.json` | `node scripts/build_docs_fix.js && node scripts/build_docs_playground.js`, then each `--check` | both, only if a listed door moved |
| 7 docs, skills | the pages of D44.12 | `python -m mkdocs build --strict --config-file mkdocs.yml`; the three `python scripts/check_docs_examples.py --lang {rust,python,javascript}` as chain steps | - |
| 8 inventories, contract | `.api-inventory.txt`, `.api-bindings.txt`, `AGENTS.md` | `python scripts/check_api_inventory.py`; `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | - |
| 9 the chain | the whole runs `--all-features --no-fail-fast` of the three crates, clippy both lanes, `cargo doc -D warnings`, the rustdoc examples, the CLI tests, `pytest python/tests`, Node, the manifests, mkdocs, the inventories, the docs runners | one background script, one log, read once | the pins of phases 1-3 alone |

### P10b - the versions

| Phase | Files | Smoke (exact) | Pins |
| --- | --- | --- | --- |
| 1 market core | `rust/market/src/instrument_event.rs` (the versions' links, states, `sendunix`, `exprunix`), `instrument.rs` (`pending`, `stamps`, the clock, the append-only keyed `commit`, `remove(key, at)` as a tombstone, the fold under validity then time, the dedup, the hourly cadence, the expiry inference and the `EXPIRED` version), `instrument/store.rs` (`append_serie` under the key, the whole `clear`), `characteristics.rs` (the maturity inference doors); tests `root/{instrument,instrument_event,characteristics}.rs`, `instrument/store.rs`, `allocations.rs`, `iobase_calls.rs` | `cargo test -p yggdryl-market --test root instrument_event`; `--test root instrument`; `--test root characteristics`; `--features "internals iceberg parquet" --test instrument`; `--test iobase_calls instrument`; `--test allocations instrument` | the store tests; `iobase_calls` `mod instrument` (a commit one append) |
| 2 FIX | `rust/fix/src/enrich.rs` (the `expired` anomaly), `msg.rs`; tests `rust/fix/tests/root/{enrich,msg,ulbridge}.rs` | `cargo test -p yggdryl-fix --test root enrich`; `--test root ulbridge`; the snapshot and `FIX_PIPELINE_COSTS` must not move | the `snapshots` count |
| 3 Python, Node, docs, inventories | as P10a's phases 4-8, the medallion's `snapshots` pin (its number derived from the capture at P10a's phase 1), `remove(key, at)` and `clock` re-spelled in both bindings | as above | `test_fix.py:5846-5852` |

Then, for each half: `cargo fmt --all` once after the last worker; the commit (the attribution lines
the harness states); one push; the CI run read to `CI result`; the results commit (DESIGN.md gains
D44 and the P10 results, `MARKET_SPLIT_NEXT.md`'s `State`/`Checks`/`Next`).

## Interplay with P7, P8, P5R, and the order

- **P7 (D40, `$S/p7/d40_design.md`, `p7_plan.md`, `p7_sweep.py`)** renames `miccode`/`cficode`/
  `countrycode`/`forexcode`/`eusipacode` on the instrument row and `Listing` (`d40_design.md:111-137`),
  moves the market element's `cficode` into `securityids` (D40.3) and the cross identity to the raw
  XXH3-128 (D40.5). P10 before P7 means: the P7 sweep's anchors (`p7_sweep.py:155-160`, the
  `instrument.rs` column indexes; `:18` `Listing::miccode`) are re-read against P10's tree - the flat
  row's `NAMES[..]` and `instrument_event.rs` join its table, `listingccy` is left as it is (no `code`
  suffix) - and D40.3's `set_cfi` through `insert_securityid` takes D44.5's classified class at
  `Cfi` rank 1 (`IdType::Cfi.is_real` stays "detailed", `idtype.rs:980`: the key types' filter in
  `statement()` already excludes the CFI, `instrument.rs:2318-2325`). P7 after P10 is recommended
  because P10 changes **semantics** (the row's shape, the class, the fill) and P7 **names** (one
  mechanical script re-anchored once); the other order makes P10 re-spell every name P7 just moved
  while P9 is still being read (`d42_design.md:852-881` already amends P7 for P9).
- **P8 (D41)** touches `iterator.rs`, `idtype.rs` (`is_chain_name`) and `enrich.rs`'s tests;
  P10 touches `market.rs:1485-1494` and `enrich.rs`'s `LifecycleMessage` (the alias check) - the
  files meet in `enrich.rs` only, in different functions; P8 after P10 costs nothing.
- **P5R (D37)** re-targets the `MarketMessage` patch onto the crates' names; it names no instrument
  door.
- **Order recommended**: **P9b, P10a, P10b, P7, P8, P5R** - P9b first because its book key is the
  `instcode` P10 fills more often (every message with a `(instcode, mic)` row), and its pins must be
  settled before P10 moves `rows`; P10a before P10b so the row's shape is laid out once and the
  versions land on a green registry; then the three lanes whose designs are written against P9's
  names.

## Put to the user (interpretations taken; say if another was meant)

1. **Two commits, one design**: P10a (decision 17 and the flat `(instcode, mic)` row holding one
   version per key) and P10b (decisions 18 and 19); the alternative is one commit of everything.
2. **The element stays one per `instcode`; the rows are one per `(instcode, mic)` version** (A + B').
   The literal "one Instrument per (instcode, mic)" was refused for the element (shared `uuid`,
   the cascade, the aliases); the stored table is where the pair is the key.
3. **A statement naming no market lands on `XXXX`**, created where absent, even where the instrument
   lists one market; a fill of an element naming no market reads `XXXX`, else the single listing.
   The alternative - a learn landing on the single listing as today - keeps the inference the
   instruction's "default mic" removes.
4. **The first-market takeover is gone**: the seed's indices hold a `XXXX` listing for good, and a
   venue learned later is a second row.
5. **The class a market keeps is any classified CFI** (two letters valid), not only a detailed one;
   six capture rows' `cficode` cells and digests move once.
6. **The currency fill** drops the ticker-equality condition and reads the matched row's trading
   currency where the message states none; the alternative keeps today's three conditions.
7. **Contradictions are anomalies, never refusals**; a stated strike under a non-option class is kept
   on the wire and not set as `strikepx`.
8. **Coalescing by validity, then "warn and keep" for instrument-level facts under `learn`, "replace"
   under `merge`**; listing-level `ticker`/`listingccy` keep "replace". The alternative - today's
   last-writer-wins everywhere - is what produced the CS/equity flip.
9. **No `versions(key, mic)` door in the bindings** in P10b: a reader reads the table (`into_arrow_reader`
   streams every version); say if a Python door is wanted.
10. **The commit becomes append-only and a committed row is immutable** (versions appended under the
    key `(transunix, instcode, mic)`, a `REMOVED` tombstone for a removal, `clear` the one whole write;
    nothing re-linked or rewritten); the alternative - a keyed upsert of the touched keys' rows,
    which could re-link and carry a `lastunix` column - is a merge, refused on an Iceberg v3 table and
    a rewrite of every touched partition per hour. What is held in memory is the registry's pending
    rows (`MAX_PENDING_ROWS` 4096, an early commit past it) and 32 stamps per key, so `ENTRY_CHARGE`
    rises by the stamps alone (#21), not by versions.
11. **The Bloomberg exchange-code rung needs the OpenFIGI CSV**, which the egress proxy refused; it is
    written from the fetched file or left out of P10 and named in the handoff - never from memory.
12. **The crosswalk `167` → CFI is the crate's own** (no official FIX table exists; Appendix 6-D
    names three equity rows); `native_derivations.rs`'s module doc says so.
13. **Not in P10**: a `country` market column (a reader joins the instruments table on `instcode`);
    `SecurityDefinition` read as a definition; reading `UnderlyingStrikePrice(316)`/`UnderlyingMaturity*`;
    Node doors beyond the re-spelled `Instruments`.
14. **The order P9b, P10a, P10b, P7, P8, P5R** places P10 before P7, P8 and P5R, the three lanes
    decision 7 placed after P9; the reason is in "Interplay" (P10 moves semantics, P7 names). Say if
    P7 is to land first.
15. **Which FIX-row cells show a fill**: the crate-field columns (`ticker`, `strikepx`, `securityids`
    and its code projections, `instcode`, `origccy`) do; `Currency(15)` and `CFICode(461)` are the
    wire's cells and show a fill only if the row writer reads the fact - read at phase 3; where they
    stay off the row, the fill reaches the message (its setters) and not that cell. Say if the row
    must carry them.
16. **`miccode` is neither filled nor propagated** from the registry: the market is where the message
    traded, the row's `mic` where the instrument is listed - two facts; the user's "mic" is read as
    the registry's key and the match's `market`, coalesced onto the row and not onto the message.
17. **Decision 19 read with immutable rows**: time order is the `SORT:by transunix` a reader sees,
    `prevuuid` is the version a statement was built over (decision 18's "the version it replaces")
    and is never rewritten, the current version is the greatest `sendunix`; a content move at a
    committed instant is warned and reaches the store with the key's next version - lost where none
    comes. The alternative - the later version re-linked - needs an upsert (#10).
18. **`remove(key, at)` and `remove_listing(key, mic, at)` take the tombstone's instant**, since a
    removal has no wire clock; the bindings re-spelled with `at` required. The alternative stamps
    the registry's clock.
19. **Derivative rows collide on `(instcode, mic)`** where the body leaves the multiplier, the
    settlement and the expiry's day out (weeklies, E-mini against standard, `SPX`/`SPXW`): accepted and
    pinned, D44.7 warning the second contract's `multiplier`/`exercise`. The alternative - those as
    key facts of an `F`/`O`/`H` body - changes P9's key. And `MAX_LISTINGS` 7 passes a venue past it
    over with a warning; a US listed option trades on more: say if the bound rises for derivatives
    (`ENTRY_CHARGE` with it).
20. **One IANA zone per country** for the expiry instant, from `zone1970.tab`, the multi-zone
    countries on their principal financial centre (`US` → `America/New_York`). The alternative - a
    zone per MIC, from a table nobody publishes with the register - is not written.
21. **The cost pins D44 would move**: `ENTRY_CHARGE` (the wider `Instrument`, the seventh listing, 32
    stamps per key - 32 KiB or 64 KiB stated after the `const` assertion), and - unless the row is
    built at commit for a key moving once per cadence, which D44 recommends -
    `instruments_learn_a_new_instrument_into_its_row_inline` and `FIX_PIPELINE_COSTS`' lifecycle row
    by one row `Scalar` per version. Each number is counted at phase 1 and none is re-pinned without
    your word.
22. **The hourly cadence** is one hour of event time, cut short by `MAX_PENDING_ROWS`; **a
    `MaturityMonthYear(200)` with no day is the month's last calendar day**, since a venue's
    third-Friday rule is the venue's and the crate holds none; a statement after expiry is warned
    and still learned. Say if a venue rule table is wanted.
