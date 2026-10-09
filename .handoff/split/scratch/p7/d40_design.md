# P7: design (D40) - the FIX row named by the registry, the lifted band last, one hold map, the simple cross identity, the book's sources

The user's instruction (2026-10-09, `$S/p7/user_instruction.md`), six items, designed as one slice.
Lands after S4 (inside `yggdryl-market` and `yggdryl-fix`) and before P5R, whose parked patch is
re-targeted onto these names. One commit, one push, CI read; the dump, the dictionary hash, the
equivalence snapshot's keys and the book identities move once each, with their sentence.

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
`partyids`, `tradable`, `marketdatakind`, `marketdatatype`, `metadata`, and every element and
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
count (152 -> 141 + the lifted band's new tag), `docs/graph/schemas.md`'s FIX row listing. The
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
`instuuid`, `isin`, `cfi`, `mic`, then on the FIX row alone `bbg`, `figi`, `forex`. On the FIX
row the band stands after the groups and the frame and before `fixentries`; `cfi` there is
`CFICode(461)` itself (D40.1), moved into the band from the instrument band. On the `marketdata`
row the band stands after the book controls and before the six nested columns, so the row is
6 + 9 + 33 + 5 + 3 + 4 + 6 = 66 columns: `isincode`, `cficode` and `miccode` leave the market
band (`MarketColumn` loses the three; a `LiftedColumn` enum of the four names them, read through
one `fact`/`record` pair like the other column enums). The crate tags keep their numbers (D40.1):
`isin` 65_022, `mic` 65_023, `forex` 65_049, `bbg` 65_050, `figi` 65_051, and `instuuid` takes
65_054, the next unused; the fixed row's order is `fix_schema_tags`' bands, never the numbering,
and `crated.rs`'s "numbered in the fixed row's order" says so.

## D40.3 - `securityids` the one hold map; `isin` and `cfi` lifted from it; `instuuid`

`MarketFacts` drops `cficode: Option<Cfi>`: the CFI is held in `securityids` under `IdType::Cfi`
(`cfi`, already a security type of the vocabulary), as the ISIN is under `IdType::Isin`, and the
market columns `isin` and `cfi` are views of the map - `Market::get_isin()` (was `get_isincode`)
and `Market::get_cfi()` (was `get_cficode`) read the base key, `set_cfi(code, overwrite)` writes it
through `insert_securityid` with the refinement rule `set_cficode` has today (`Cfi::refined`, a
compatible code refined, a conflicting one replaced under `overwrite`). The MIC is no security
identifier (`IdType` has no venue word, `is_listing` names a listing's code, not the venue), so
`mic: Option<Mic>` stays a market holder, renamed, and its column is lifted by position alone.
`instuuid` is the instrument's identity, `Uuid::from_u128(xxh3_128(isin))` of the element's real
ISIN (`Isin::is_real`), none where it states none - the reading every later instrument
centralization replaces with the registry's answer, which is why it is a column now and derived
from the one key the registry is keyed by; `Market::instuuid()` is the provided reading, no
holder, no setter; on the FIX row the crate tag 65_054 `instuuid` (`uuid`, nullable) states it,
read back as the row's word like every lifted column.

## D40.4 - the renames

The registered-code columns and holders drop their `code` suffix, one spelling each: `isincode`
-> `isin`, `cficode` -> `cfi`, `miccode` -> `mic`, `bloombergcode` -> `bbg`, `figicode` -> `figi`,
`forexcode` -> `forex` on the FIX row and the market row (`FixMsg`'s `get_isincode`/`get_miccode`
and the Python/JavaScript properties `isincode`, `cficode`, `miccode`, `bloombergcode`, `figicode`,
`forexcode` follow: `msg.isin`, `msg.cfi`, `msg.mic`, `msg.bbg`, `msg.figi`, `msg.forex`); the ISIN
registry's columns `cficode` -> `cfi`, `countrycode` -> `country`, `forexcode` -> `forex`,
`eusipacode` -> `eusipa`, `miccode` -> `mic` with `IsinEntry`'s fields, its `SORT:by`
(`["isin","mic"]`), the seed's JSON keys (`config/isin/instruments.json` and the embedded copy,
`scripts/check_isin_seed.py --sync`), the golden-file readers (the six EUSIPA spellings keep
reading a foreign file's `eusipacode` column: they are intake spellings, not ours), and a store
written before the rename is not read (no back-compat: a registry store is rebuilt from the seed
and the stream it learned from). `crosscode`, `hashcode` and `crosshashcode` keep their names:
they are the element's identity vocabulary (D38), not a registered code; `detailedcficode` is the
dictionary's own name of `CFICode(461)` and keeps it.

## D40.5 - `crossuuid` is the XXH3-128 of the cross code

`Element::cross_uuid` answers `Uuid::from_u128(xxh3_128(crosscode))` where the cross code is not
empty - the raw 128-bit digest as the UUID, no version bits - and the element's own identity where
it is empty (an element in no chain is a chain of one, as today). `crosshashcode` stays the
XXH3-64 of the same bytes, the key the pipeline and the FIX row carry; `sync_cross` writes both.
Every pinned `crossuuid` moves once (the equivalence snapshot's `crossuuid` cells, the graph and
FIX tests naming one); no `uuid` moves, since `time_uuid` reads the cross hash code and not the
cross UUID.

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
`get` costs today; the FIX allocation rows stay - and the S0/S2 pins. The 912-byte `MarketData`
size pin: `MarketFacts` loses one `Option<Cfi>` (the size falls or stays; a fall is re-pinned with
its sentence, a rise a defect).

## Order and checks

After S4 (the files under `rust/market/src/graph/`, `rust/market/src/`, `rust/fix/src/`) and
before P5R. One commit: the core's `crated.rs`/`schema.rs`/`identity.rs`/`msg.rs`/`build.rs`
(the lifted tags grown by the eleven, the derived definitions deleted, the band order), the
market crate's `element.rs`, `book.rs`, `arrow.rs`, `market_column.rs` + the `LiftedColumn`,
`facts.rs`, `market.rs`, `isin_registry.rs` and the seed, the bindings' properties, the pipeline
(`python/tests/medallion.py`), the pages (`docs/fix/capture.md` the crate's columns table and the
derived-column section, `docs/graph/schemas.md`, `docs/graph/{event,market,book,isin-registry}.md`,
`docs/types/codes/*.md` where a column is named), the skills, the inventories. Checks: the fix
and market crates' suites both lanes with every unmoved pin green without edit, the dump written
once and the hash re-pinned once with its sentence, the snapshot's keys re-spelled and its value
lines moved only where D40.5/D40.6 say, `pytest python/tests` whole with the medallion pipeline
green, Node's suite, the three docs runners, `mkdocs build --strict`.

## Put to the user (interpretations taken; say if another was meant)

1. `transunix`/`sendunix` on FIX-row tables become `transacttime`/`sendingtime` (D40.1), so the
   pipeline windows FIX tables by `transacttime`.
2. `crosscode`, `hashcode`, `crosshashcode` keep their names; only registered-code columns drop
   `code` (D40.4).
3. `instuuid` derives from the real ISIN until a registry assigns it (D40.3).
4. The book's sources are its delta, its events and the previous book's uuid - never the alive
   entries carried forward, never the previous book's sources; its hash code is one digest over
   the instant and the digest of those sources (D40.6).
5. `crossuuid` keeps "its own identity where the cross code is empty" (D40.5).
