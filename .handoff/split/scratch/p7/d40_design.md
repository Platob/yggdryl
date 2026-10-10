# P7: design (D40) - the FIX row named by the registry, the lifted band last, one hold map, the simple cross identity, the book's sources

The user's instruction (2026-10-09, `$S/p7/user_instruction.md`), six items, designed as one slice.
Lands after P9 and before P8 and P5R (`$S/user_decisions.md` 7), inside `yggdryl-market` and
`yggdryl-fix`; P5R's parked patch is re-targeted onto these names. One commit, one push, CI read;
the dump, the dictionary hash, the equivalence snapshot's keys and the book identities move once
each, with their sentence.

**Amended for P9** (`$S/p9/d42_design.md` "## What P7 changes", 2026-10-10): `instuuid` is dropped:
P9 lands `instcode` (65_054, the instrument's `crosscode` as text, user decision 9) as the market
holder after `securityids`, so the lifted band is `instcode, isin, cfi, mic` (D40.2) and D40.3's
derivation is superseded; D40.5 moves every cross identity once, the instruments' included, through
`Uuid::new`; D40.4's renames reach P9's `instrument` row. The mechanical renames are
`$S/p7/p7_sweep.py`; the phases and their pins are `$S/p7/p7_plan.md`.

## D40.1 - the FIX row's column for a fact FIX states is the registry's field

Today the fixed row opens with the crate's bands - the element's, the event's, the market's and the
operation's facts under the names every `marketdata` row states them by - and for a fact FIX states
under a name of its own the crate defines a *derived* column stated a second time beside the
registry's field: `transunix` (65_007) beside `TransactTime(60)`, `sendunix` (65_009) beside
`SendingTime(52)`, `strikepx` (65_036) beside `StrikePrice(202)`, `ordqty` (65_038) beside
`OrderQty(38)`, `askpx`/`bidqty`/`askqty` beside `OfferPx(133)`/`BidSize(134)`/`OfferSize(135)`,
`ticker` beside `Symbol(55)`, `unit` beside `UnitOfMeasure(996)`, `spotrate`/`forwardpoints` beside
`LastSpotRate(194)`/`LastForwardPoints(195)`. A fact FIX names alike - `Price(44)`, `Side(54)`,
`CFICode(461)`, `TimeInForce(59)` and the twelve others `dictionary_market_tag` lists - is already
that field and never tagged twice.

Decided: the rule the named-alike set already follows covers every fact one FIX field states whole.
The FIX row's column for such a fact is the registry's field - its tag, its folded name, its code
set - holding the message's settled fact, as `price` (44) holds the settled price today; the
derived crate definition is retired. The pairing is one table, `fix/schema.rs`'s
`dictionary_market_tag`/`dictionary_operation_tag` grown by a `dictionary_event_tag`
(`transunix` -> 60, `sendunix` -> 52) and the market pairs (`strikepx` -> 202, `askpx` -> 133,
`bidqty` -> 134, `askqty` -> 135, `ticker` -> 55, `unit` -> 996, `spotrate` -> 194,
`forwardpoints` -> 195) and the operation pair (`ordqty` -> 38); `shared_tags()` reads it as it
reads the named-alike set, so the row still opens with every element, event, market and operation
column, each under one tag. A fact no single FIX field states whole keeps its crate definition:
`hiddenqty` (the quantity past the shown part, two fields), `prevpx`/`prevqty` (a predecessor's
word first), `execunix`, `origccy`, `bidccy`/`askccy`, `fxrates`, `securityids`, `identifiers`,
`partyids`, `tradable`, `marketdatakind`, `marketdatatype`, `metadata`, `instcode` (P9), and every element and
event column FIX does not state (`uuid`, `crossuuid`, `crosscode`, `hashcode`, `crosshashcode`,
`srcuuids`, `creaunix`, `exprunix`, `prevunix`, `snapunix`, `prevuuid`, `seqnum`, `state`).

The round trip holds as it does for `price` today: the row's column states the settled fact; the
wire text of the field where it differs from the settled fact (a `TransactTime(60)` outside the
sending delay, an `OrderQty(38)` the walk restated) is conflicting content and stays in
`fixentries` under its `tag:name`, so a message read back re-emits the wire and digests as the
parse did. The retired definitions leave their numbers unused for good (`is_derived_tag` answers
false for them and nothing else, then is deleted when no derived definition remains); a retired
number is never given to another definition - the sentence "a retired definition leaves no gap"
in `crated.rs`/`capture.md` is replaced by "a retired number is never reused", because a stored
capture fills its columns by `FIX:tag`.

What moves: the crate dump (the retired definitions gone), the dictionary hash (once, its
sentence: "the eleven derived definitions FIX states under a name of its own left the dictionary,
D40"), the census (eleven definitions fewer), the equivalence snapshot's keys (`transunix` ->
`transacttime` and the ten others on FIX rows; no value, no digest line), `fix_schema_tags`'
count (153 after P9 -> 142; the lifted band adds no tag, 65_054 `instcode` being P9's),
`docs/graph/schemas.md`'s FIX row listing. The
`marketdata` row keeps every market name: `transunix`, `sendunix`, `strikepx`, `ordqty`, `askpx`,
`ticker` are the graph's vocabulary; the pairing table is where a reader joins the two rows.

Consequence stated to the user: a FIX-row table (the medallion pipeline's `bronze.fix_messages`
and `silver.fix_messages`) is windowed and partitioned by `transacttime` where it was by
`transunix`; `python/tests/medallion.py` reads the window column per stage (`transunix` on the
line, book and event tables, `transacttime` on the FIX tables), its `PRIMARY_KEY` and `PARTUNIX`
spelled per table. The alternative - keep `transunix` on the FIX row and retire the registry's
`transacttime` column instead - was refused: the instruction names the registry's spelling, and a
FIX row that spells FIX's own field by another name is what the instruction removes.

## D40.2 - the crate's bands in order, the lifted band last

Every generated schema keeps its opening order - element (6), event (9), market, operation (5) -
and the lifted instrument identities close the flat columns as one band, in this order:
`instcode`, `isin`, `cfi`, `mic`, then on the FIX row alone `bbg`, `figi`, `forex`. On the FIX
row the band stands after the groups and the frame and before `fixentries`; `cfi` there is
`CFICode(461)` itself (D40.1), moved into the band from the instrument band. On the `marketdata`
row the band stands after the book controls and before the six nested columns, so the row is
6 + 9 + 33 + 5 + 3 + 4 + 6 = 66 columns, as P9 left it: `instcode`, `isin`, `cfi` and `mic` leave
the market band, where P9 left the four contiguous after `securityids` (`MarketColumn::ALL` 37 ->
33; `MarketColumn` loses the four; a `LiftedColumn` enum of the four names them, read through one
`fact`/`record` pair like the other column enums, and `cfi`'s pairing with 461 moves from
`dictionary_market_tag` to the lifted band's). The crate tags keep their numbers (D40.1): `isin`
65_022, `mic` 65_023, `forex` 65_049, `bbg` 65_050, `figi` 65_051, and `instcode` keeps 65_054,
its name and its number from P9; the fixed row's order is `fix_schema_tags`' bands, never the
numbering, and `crated.rs`'s "numbered in the fixed row's order" says so. No `instcode` cell moves
at P7: it is text.

## D40.3 - `securityids` the one hold map; `isin` and `cfi` lifted from it

`MarketFacts` drops `cficode: Option<Cfi>`: the CFI is held in `securityids` under `IdType::Cfi`
(`cfi`, already a security type of the vocabulary), as the ISIN is under `IdType::Isin`, and the
market columns `isin` and `cfi` are views of the map - `Market::get_isin()` (was `get_isincode`)
and `Market::get_cfi()` (was `get_cficode`) read the base key, `set_cfi(code, overwrite)` writes it
through `insert_securityid` with the refinement rule `set_cficode` has today (`Cfi::refined`, a
compatible code refined, a conflicting one replaced under `overwrite`). The MIC is no security
identifier (`IdType` has no venue word, `is_listing` names a listing's code, not the venue), so
`mic: Option<Mic>` stays a market holder, renamed, and its column is lifted by position alone.
The `Instrument` already holds its CFI in `securityids` under `IdType::Cfi` (D42.8), so after this
move the market element and the instrument keep the class by one rule.

**`instuuid` is superseded by P9** (D42.10, user decision 9): there is no `instuuid` column,
reading or tag. The instrument is named on every market row by its code, `instcode` - a `Market`
holder (`get_instcode`/`set_instcode`, `Option<Str>`), the resolved `Instrument`'s `crosscode`,
followed along a chain, fed to no digest - and on the FIX row by the crate tag 65_054 `instcode`
(`utf8`, nullable). A reader who wants the uuid digests the code: `Uuid::new(xxh3_128(instcode))`
after D40.5, which for a real-ISIN security is the value this section derived, byte for byte. P7
moves `instcode` into the lifted band (D40.2) and changes nothing else about it.

## D40.4 - the renames

The registered-code columns and holders drop their `code` suffix, one spelling each: `isincode`
-> `isin`, `cficode` -> `cfi`, `miccode` -> `mic`, `bloombergcode` -> `bbg`, `figicode` -> `figi`,
`forexcode` -> `forex` on the FIX row and the market row (`Market::get_isincode`/`get_cficode`/
`set_cficode`/`get_miccode`/`set_miccode` -> `get_isin`/`get_cfi`/`set_cfi`/`get_mic`/`set_mic`,
`MarketColumn::{IsinCode, CfiCode, MicCode}` -> `{Isin, Cfi, Mic}` before D40.2 lifts them, the
crate tag constants `ISINCODE_TAG_NAME` -> `ISIN_TAG_NAME` and the four others, the displays `ISIN
Code` -> `ISIN` and the five others; the Python/JavaScript properties `isincode`, `cficode`,
`miccode`, `bloombergcode`, `figicode`, `forexcode` follow: `msg.isin`, `msg.cfi`, `msg.mic`,
`msg.bbg`, `msg.figi`, `msg.forex`; `book_crosscode` reads the lifted `isin`); P9's `instrument`
row (D42.13) the same way - `cficode` -> `cfi`, `countrycode` -> `country`, `forexcode` -> `forex`,
`eusipacode` -> `eusipa`, and the nested `listings`' `miccode` -> `mic` - with `Instrument`'s and
`Listing`'s fields, builders and readers (`cficode()` -> `cfi()`, `forexcode()` -> `forex()`,
`eusipacode()` -> `eusipa()`, `Listing::miccode()` -> `mic()`, `try_with_cficode`/`with_countrycode`/
`with_eusipacode` -> `try_with_cfi`/`with_country`/`with_eusipa`; the stated-country reader
`countrycode()` -> `stated_country()`, because `country()` already answers the derived country),
its `SORT:by` staying `["crosscode"]`, the seed's JSON keys (`config/instruments/instruments.json`
and the embedded `rust/market/src/instrument/seed.json`, `scripts/check_instruments_seed.py
--sync`), the golden-file readers (the six EUSIPA spellings keep reading a foreign file's
`eusipacode` column: they are intake spellings, not ours), and an instruments store written before
the rename is refused at load naming the column it lacks (D42.13's rule; no back-compat: the
medallion's instruments table is rebuilt from bronze, D40.5). `crosscode`, `instcode`, `hashcode`
and `crosshashcode` keep their names: they are the element's identity vocabulary (D38) and the
instrument's code (D42), not a registered code; `detailedcficode` is the dictionary's own name of
`CFICode(461)` and keeps it, as does every dictionary and bridge spelling (`LegCFICode`,
`ISOCountryCode`, `ISINCODE`, `OMS_FIGICODE`).

## D40.5 - `crossuuid` is the XXH3-128 of the cross code

`Element::cross_uuid` answers `Uuid::new(xxh3_128(crosscode))` where the cross code is not
empty - the raw 128-bit digest as the UUID, no version bits; `Uuid::new` (`rust/src/uuid.rs:441`)
is the constructor that keeps every bit, `Uuid::from_u128` does not exist - and the element's own
identity where it is empty (an element in no chain is a chain of one, as today). `crosshashcode`
stays the XXH3-64 of the same bytes, the key the pipeline and the FIX row carry; `sync_cross`
writes both. Every cross identity moves once, the instruments' included (D42 "What P7 changes"
2): every pinned `crossuuid` (the equivalence snapshot's `crossuuid` cells, the graph and FIX
tests naming one), every `Instrument`'s `crossuuid` and `uuid` (`uuid = crossuuid`, panel D42.4),
`underlying_uuid()`/`leg_uuids()` and the `aliases` index's digests (`instrument.rs`
`cross_uuid_of`, the market crate's one spelling of the rule, which calls the core's after this
change), and panel D42.2's `crossuuid`/`uuid` columns - re-pinned once with this sentence. The
minted `QY` numbers do not move (P9's `Instrument::mint` read `xxh128(crosscode)` directly); from
here `mint` and `is_own_mint` read `crossuuid.as_u128()` and the second digest goes. No event's
`uuid` moves, since `time_uuid` reads the cross hash code and not the cross UUID. The medallion's
instruments and silver tables, which P9 wrote under the old rule, are rebuilt from bronze.

## D40.6 - a book states its sources, and its content code is its instant over them

`BookEvent::get_srcuuids` answers the unique, sorted UUIDs of the events constituting the book at
its instant: every entry of its `delta`, every item of its `events`, and - where the book was
rebuilt over the one before it (`with_previous`) - that previous book's own `uuid`, one identity
and never its sources, so the list is what this instant brought plus one link back and no book
accumulates the UUIDs of every cycle before it; the alive entries a complete book carries forward
are not sources (they are the earlier cycles' events, each already a source of the book that
applied it). Computed when the book is built or re-finalized and held; `set_srcuuids` keeps
nothing (the sources are derived, and a row read back rebuilds them from its `prevuuid` and its
nested `delta` and `events` columns, so the `srcuuids` column of a `BOOK` row states them and
`MarketData::from_arrow_reader` reads past the cell). The book's content code is
`XXH3-64(transunix as eight big-endian bytes, then the XXH3-64 of the sorted source UUIDs' bytes
as eight big-endian bytes)` - "the transunix with the hash of srcuuids" - replacing
`finalize_book_event`'s digest over the market facts, the sides' digests, the delta words and the
event kinds; since the previous book's uuid is a source, every book's code chains through the one
before it. The book's `uuid` derives from it through `Event::finalized` as before, so every book
identity moves once (the books' rows in the equivalence snapshot and the book tests' pinned UUIDs
and hash codes, each with the sentence). Two books of one instant over the same events after the
same predecessor are one book whatever facts they settled on, which is what makes the code a
content code of a book rather than of a fold.

The user's refinement (2026-10-09, after the six items): "on the with previous add also the
previous uuid in it, but ensure srcuuids dont accumulate all uuids of all cycle events" - taken as
above: the previous book's `uuid` joins the sources, the previous book's sources never do.

## What does not move

The `DataTypeId` bytes, the ranks, the `Shape` positions, the hash feeds of every leaf but the
book's, the value-stream bytes, the Arrow extension names, every cost pin (`allocations`,
`iobase_calls`, the benches) - a lifted column read off `securityids` costs the one binary search
`get` costs today; the FIX allocation rows stay - and the S0/S2 pins. The `MarketData` size pin
(928 bytes after P9, `rust/market/tests/graph/market_data.rs`): `MarketFacts` loses one
`Option<Cfi>` (the size falls or stays; a fall is re-pinned with its sentence, a rise a defect).
The minted `QY` numbers and every `instcode` cell.

## Order and checks

After P9 and before P8 and P5R; it touches the three crates. One commit: the FIX crate's
`rust/fix/src/{crated,schema,identity,msg,build}.rs` (the lifted tags grown by the eleven, the
derived definitions deleted, the band order); the core's `rust/src/graph/element.rs`
(`Element::cross_uuid` and `sync_cross`, D40.5 - the event vocabulary stays core); the market
crate's `rust/market/src/graph/{book,arrow,market_column,facts,market}.rs` + the `LiftedColumn`,
`rust/market/src/{instrument,listing}.rs`, `instrument/{store,seed}.rs` and the seed; the bindings'
properties, the pipeline (`python/tests/medallion.py`), the pages (`docs/fix/capture.md` the
crate's columns table and the derived-column section, `docs/graph/schemas.md`,
`docs/graph/{event,market,book,instrument}.md`,
`docs/types/codes/*.md` where a column is named), the skills, the inventories, and AGENTS.md's
Layout rows naming `isincode`, `cficode`, `miccode` and `get_isincode`. Checks: the core suites
that pin the cross identity (`rust/tests/graph/{element,column,element_column}.rs`,
`rust/tests/text/{line,plan,options}.rs`), the fix and market crates' suites both lanes with every
unmoved pin green without edit, the dump written once and the hash re-pinned once with its
sentence, the snapshot's keys re-spelled and its value lines moved only where D40.5/D40.6 say,
`pytest python/tests` whole with the medallion pipeline green, Node's suite, the three docs
runners, `mkdocs build --strict`.

## Put to the user (interpretations taken; say if another was meant)

1. `transunix`/`sendunix` on FIX-row tables become `transacttime`/`sendingtime` (D40.1), so the
   pipeline windows FIX tables by `transacttime`.
2. `crosscode`, `hashcode`, `crosshashcode` keep their names; only registered-code columns drop
   `code` (D40.4).
3. Superseded by P9 (user decision 9): there is no `instuuid`; the rows carry `instcode`, the
   instrument's code, and its uuid is that code's digest (D40.3).
4. The book's sources are its delta, its events and the previous book's uuid - never the alive
   entries carried forward, never the previous book's sources; its hash code is one digest over
   the instant and the digest of those sources (D40.6).
5. `crossuuid` keeps "its own identity where the cross code is empty" (D40.5).
6. Open, found writing the sweep: on the FIX row, tag 461's column keeps the registry's folded
   name `cficode` (D40.1: the FIX row's column for a fact FIX states is the registry's field), so
   `cfi` is the `marketdata` row's name and the readers' (`get_cfi`, `msg.cfi`), joined to 461 by
   the pairing table as `transunix` is to `transacttime`; D40.2's "`cfi` there is `CFICode(461)`"
   means that column. The sweep leaves every quoted `cficode` under the FIX crate and the FIX
   tests and pages as residue for this item.
7. `Instrument::countrycode()` (the stated country) becomes `stated_country()`, since `country()`
   already answers the derived one (D40.4).
