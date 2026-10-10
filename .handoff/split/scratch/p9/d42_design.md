# P9: design (D42) - the Instrument replaces the ISIN registry

The user's instruction (2026-10-10, `$S/p9/user_instruction.md`, verbatim there, typos the
user's), six items, designed as one slice. The user's later decisions (`$S/user_decisions.md`):
6 and 7 place it - P9 lands **first** in PR #209, on today's tree (today's column names
`isincode`, `cficode`, `miccode`; today's cross-identity rule in `rust/src/graph/element.rs`,
D40.5 not landed), before P7, P8 and P5R, and ships in release 0.1.22; 8 keys a security by its
real ISIN alone ("Bare ISIN for securities"); 9 **replaces the user's first spelling**: the market
rows carry `instcode` - the instrument's `crosscode` as text (`utf8`, tag `65_054`) - not an
`instrumentuuid` ("Replace the instrumentuuid of market to instrumentcode mapped to the instrument
crosscode", then "Use instcode instead of instrumentcode"), and a re-keyed instrument keeps
`aliascodes`. Where this text says `instcode`, the instruction's "intrumentuuid" is meant and
superseded. R1 (the release plumbing) is already committed as `0f411f5ce` (`$S/r1/d43_design.md`).
One commit, one push, CI read; the crate dump, the dictionary hash, the census, the equivalence
snapshot and `fix.json`/`playground.json` move once each, with their sentence.

**The cross code, the identity, the minted number, the hashing, the re-key and the per-row cost
are the judge panel's decision**, `$S/p9/crosscode_decision.md` (committed `0c147d403`, re-spelled
onto `instcode` in `80510b080`/`4418f0adc`, sections D42.1-D42.7), adopted here; its "Put to the
user" item 1 was answered by the user and item 6 is decision 9. This document does not restate
its grammar: it names what each remaining decision reads from it, numbers its own decisions D42.8
onward, and **names every sentence of the panel it amends** (D42.4's feed of `instcode`, D42.7's
`underlying`, `legs`, `identifiers` and characteristics columns, D42.3's `is_minted` argument,
D42.6's `FxMemo` slot) with the reason. A first draft of this document, written before the
panel's file existed, is at `7ebd6f8ed`; where the two disagree, the panel holds.

Every claim below names the file and line it was read from; the implementing session verifies
each by reading the file (no command was run to write this).

## The user's asks, each mapped to a decision

| # | The ask (the user's words) | Decision |
| --- | --- | --- |
| 1 | "a first release with market and fix splitted" | R1, committed `0f411f5ce`; recorded in `$S/r1/d43_design.md`; `$S/r1/r1_release.sh --prove` is the first release's proof; the merge publishes 0.1.22 after P9, P7, P8, P5R and the live AWS run (`user_decisions.md` 7) |
| 2 | "replace current isin registry into our own market/instrument.rs ... graph Element implementations containing all mapping for isin, cfi, lei, ric with learnings from market recording / fix parsing, thus filling the value of market instrumentuuid and dumped in medaillon" | D42.8 (what is deleted, one owner per fact), D42.9 (`Instrument`, an `Element` holding every mapping), panel D42.4 (its hashing), D42.10 (`instcode` - decision 9's spelling of "market instrumentuuid" - a market holder on every row and the FIX row's crate tag 65_054), D42.11 (learning from any market element and from a FIX message), D42.14 (the medallion's `silver.instruments` table and the column on every market table) |
| 3 | "create custom isin codes, fill country currency" | panel D42.3 (the `QY` minted number: nine base-36 digits of the identity's low 46 bits, Luhn-closed, rank 1), D42.12 (country and currency) |
| 4 | "autocreating for forex pairs which dont have existing isin" | D42.11 (an FX pair detected off `Symbol(55)` - or stated as `forex` plus an `I*`/`J*`/`S*` class by any market element - is keyed `IF:EUR/USD`, mints its number into `derived:isin`, the lifecycle learns the instrument, its book is keyed by the minted number) |
| 5 | "optional instrument underlying pointing to another instrument uuid ... and legs uuids list" | D42.9: `underlying: Option<Str>` and `legs: Vec<Leg>` hold the **codes** (the key names them by their codes, never their uuids - panel D42.1, "no cascading uuid in a key"); `underlying_uuid()` and `leg_uuids()` answer the uuids by the graph's cross rule; a leg's ratio is a key byte (`n*`), legs sorted (panel item 8) |
| 6 | "characteristics to handle generic finance products like future options strikepx ... include it syntheticqlly in cross code then correct graph defined hashing in uuids" | panel D42.1: the body of a `class:body` code is written once by `Instrument::write_crosscode` from the pair (`securityids`' `forex` entry, D42.9), the characteristics (settle, expiry, month, strike), the underlying and the legs. **The hashing correction is not P9's**: P9 leaves every instrument's `crossuuid`/`uuid` on today's `from_v8` rule, and the user's correction lands with P7 (D40.5) in this PR before the merge - the merge must not happen before P7 (d43's gate) |
| 7 | "instrument cross uuid should rely on cficode + isincode, and fill the market intrumentuuid with this cross uuid" | panel D42.1 as the user confirmed (decision 8): a real ISIN alone keys an agency-numbered instrument, the CFI class keys everything no agency numbers; `crossuuid` by the graph's rule; decision 9 replaces the second half: the market row carries the instrument's **code** (`instcode`, D42.10), and the code's digest is the `crossuuid`, so the code carries both |

## What the panel decided (adopted, not restated)

| Panel section | What it fixes | Read by |
| --- | --- | --- |
| D42.1 the grammar | `crosscode := isin \| class ':' body`; the bodies per CFI category (`I*`, `J*`, `S*`, `O*`/`H*`, `F*`, `K*`); at most 128 bytes into a `[u8; 128]`, one speller `Instrument::write_crosscode`; what never enters a key (ticker, MIC, LEI, RIC, multiplier, exercise, uuids, the minted number) | D42.9, D42.10, D42.11 |
| D42.2 the examples | the pins (`US0378331005` -> `27e376388c8738fd` / `0751ad36-...`; `IF:EUR/USD` -> `QYLTVIRYHNX5`; the option, future and spread rows) - computed under D40.5's XXH3-128 | D42.17 (which of them pin in P9, which at P7) |
| D42.3 the custom ISIN | prefix `QY`; NSIN nine base-36 digits of the identity's low 46 bits; `Isin::minted(nsin)` in `rust/src/isin.rs`; `closing_digit`; rank 1; lands under `derived:isin` at the parse and `yggdryl:isin` on the Instrument; a collision refused at the mint, the instrument keeping its identity with no number. **Amended** (below): `is_minted` takes the digest the mint read, not `crossuuid` | D42.11, D42.17, "What P7 changes" |
| D42.4 the hashing | an `Element`, no `Event`; `uuid = crossuuid`; `hashcode` the content code over every fact through typed accessors; `srcuuids` two at most; `instcode` a market holder (`Option<Str>`) with a setter taking `overwrite`, filled along a chain. **Amended** (D42.10): `instcode` is fed to no digest, as `crossuuid` is not | D42.9, D42.10, D42.17 |
| D42.5 what moves | nothing but the placeholder re-key (`isin` -> `class:body`, the old code into `aliascodes`); multiplier and exercise out of the key; tenor-keyed and date-keyed forwards two instruments; `instcode = crosscode or instcode in aliascodes` the gold join | D42.9, D42.17 |
| D42.6 the per-row cost | the packed-`i128` ISIN index, the `[u8; 128]` speller, two digests per creation, none per row; the table's `PARTITION:by truncate(crosscode, 2)`, `SORT:by crosscode`. **Amended** (D42.10): `FxMemo` gains no instrument slot | D42.13, D42.17 |
| D42.7 the table | one row per Instrument element: the six element columns, `aliascodes`, `placeholder`, `cfi`, `country`, `currency`, `securityids`, the characteristics struct, `listings` nested, the three stamps; silver replayed from bronze. **Amended** (D42.13): `underlying` and `legs` hold codes, `identifiers` is dropped, `forex` is no characteristic (the pair's one holder is `securityids`' `forex` entry, D42.9), `expiry` and `settle` are the key's text, the commit is today's whole overwrite | D42.13, D42.14 |

Three places where P9's position before P7 bends a panel sentence, decided here:

- **The mint reads `xxh128(crosscode)` directly**, not `crossuuid`'s low bits. Today
  `Element::cross_uuid` is `Uuid::from_v8(u128::from(crosshashcode))` (`element.rs:297-302`), so
  an instrument's `crossuuid` in P9 is the XXH3-64 widened, and D40.5 moves it to the raw XXH3-128
  at P7. If the number were the projection of today's `crossuuid`, every `QY` number would move at
  P7 and the panel's D42.2 pins would hold neither now nor then. So `Instrument::mint` computes
  `xxh128(crosscode)` (`yggdryl::xxhash::xxh128`, `xxhash/mod.rs:174`) once at the mint - the value
  `crossuuid` takes at P7 - and the panel's twelve-byte literals pin from P9 on and never move; after
  P7 the two are one value and the mint reads `crossuuid` as the panel says (one digest fewer).
- **`is_minted` takes the digest the mint read.** The panel's `Isin::is_minted(text, crossuuid)`
  would compare a number against `crossuuid`'s low bits, which in P9 are the `from_v8` of the
  XXH3-64 - `IF:EUR/USD` answers `QY2QX016JGV0` there by the reviewer's recomputation and
  `QYLTVIRYHNX5` from the mint; the implementing session's test pins both - so
  every number the crate minted would read as a foreign one and D42.3's foreign-mint rule would
  invert. P9 spells it `Isin::is_minted(text: &str, digest: u128)`, the caller passing
  `xxh128(crosscode)`, and `Instrument::is_own_mint(&Isin)` recomputes it from the held code. A
  test pins `is_minted(minted(code), xxh128(code))` true for every D42.2 row and a hand-written `QY`
  number false. At P7 the argument becomes `crossuuid.as_u128()` and the extra digest goes.
- **The instrument's `crossuuid` and `uuid` follow the rule of the day** (the panel's last
  paragraph): `from_v8` over the XXH3-64 in P9, the raw XXH3-128 at P7, moving once with every
  other cross identity in the tree. The D42.2 `crossuuid`/`uuid` column pins at P7; P9 pins the
  `crosscode`, `crosshashcode` and minted-ISIN columns, which do not move.

## D42.8 - what is deleted, and what owns each fact afterwards

Deleted in the one commit, no alias, no shim, no dual reader (AGENTS "No back-compat"):

| Deleted | Where | Replaced by |
| --- | --- | --- |
| `IsinRegistry`, `IsinEntry`, `IsinTable`, `MatchTier`, `Resolution`, `Unmatched`, `EconomicMemo`, `Learned`, `warn_full` | `rust/market/src/isin_registry.rs` (3,510 lines); `lib.rs:6,18,30,112-113` | `Instruments` (the collection a process learns into), `Instrument` (one element), `InstrumentTable` (the snapshot a parse door fixes), `Resolution`/`MatchTier`/`Unmatched` under the same names in `instrument.rs` (they describe a match), `EconomicMemo`, `Learned`, `warn_full` moved with their meaning |
| `isin_registry/store.rs`, `env.rs`, `seed.rs`, `seed.json` | `rust/market/src/isin_registry/` | `rust/market/src/instrument/{store,env,seed}.rs`, `instrument/seed.json`; the two new types in root files `rust/market/src/characteristics.rs` and `rust/market/src/listing.rs` (D42.9) |
| `isin_registry_as_table`, `isin_registry_learn_stating`, the four `pub use` | `rust/market/src/implementer.rs:32-39,160-179` | `instruments_as_table`, `instruments_learn_stating`, `pub use instrument::{EconomicMemo, InstrumentTable, Learned, warn_full}` |
| `FixCodec::{with_isin_registry, isin_registry}`, the field, the crate-private `instruments()` snapshot door | `rust/fix/src/codec.rs:527,533,1081-1103` | `with_instruments(Arc<Mutex<Instruments>>)`, `instruments()` (the shared collection), `instrument_table()` (the snapshot, crate-private) |
| `Codes::{Walk(IsinRegistry), Shared(..)}` | `rust/fix/src/enrich.rs:707-760` | the same two arms over `Instruments` |
| `FixMsg::{fill_instrument_ids, fill_instrument, refill_instrument_ids}` over `IsinTable` | `rust/fix/src/msg.rs:3883-3915,4070` | the same verbs over `InstrumentTable`; `fill_instrument` (the lifecycle) also sets `instcode` (D42.10); `fill_instrument_ids` (the parse) keeps its rustdoc's contract - derived `securityids` only, no field |
| `config/isin/instruments.json`, `scripts/check_isin_seed.py`, the inert path `.github/ci/rows.toml:47` | the seed and its checker | `config/instruments/instruments.json` (the same 209 listings, re-expressed per panel D42.7: one object per instrument with a `listings` array), `scripts/check_instruments_seed.py`, the inert path re-spelled |
| `YGGDRYL_ISIN_REGISTRY_URI`, `~/.config/yggdryl/isin/` | `isin_registry/env.rs:19-20` | `YGGDRYL_INSTRUMENTS_URI`, `~/.config/yggdryl/instruments/` |
| Python `IsinRegistry`, `Resolution`, `FixCodec(isin_registry=)`, `codec.isin_registry`, `yggdryl.isin_registry` | `python/src/isin_registry.rs`, `python/src/fix.rs:2827-3055`, `python/yggdryl/isin_registry.py`, the stubs | `Instruments`, `Resolution`, `FixCodec(instruments=)`, `codec.instruments`, `yggdryl.instrument` |
| Node `IsinRegistry`, `isinRegistry` option and getter | `node/src/isin_registry.rs`, `node/src/fix.rs:2551-2688,3414-3418` | `Instruments`, `instruments` - the existing door re-spelled, nothing added (§4) |
| `docs/graph/isin-registry.md`, nav `mkdocs.yml:257` | | `docs/graph/instrument.md`, nav `Instrument: graph/instrument.md` |
| the 46-column `isinregistry` row (`isin_registry.rs:73-107,305-334`) | | the `instrument` row (D42.13); a store written under the old row is refused at load naming the table (D42.13) |

The sweep is grep-driven, not compiler-driven: `cargo check --keep-going` names every call site
and no intra-doc link, and rustdoc under `-D warnings` (CI's lint jobs) names the links. Phase 0's
list is `git grep -n -i -E 'isin[-_ ]?registr|IsinEntry|IsinTable|check_isin_seed|config/isin' -- . ':!.handoff'`
(1,458 lines in 60-odd files today, `.handoff/` excluded because it records the old names on
purpose), which reaches the sites `cargo check` cannot: `rust/market/src/idtype.rs:669`,
`securityid.rs:3`, `lib.rs:6` (module docs); `rust/fix/src/codec.rs:732-744,1052-1076,2697-2701`,
`enrich.rs:725`, `msg.rs:3884,3900,3923` (doc links inside re-spelled files);
`rust/market/tests/root/implementer.rs:10,64-67`, `rust/market/tests/root/securityid.rs:6`;
`node/src/fix.rs:3414-3418` (the napi option's doc and `ts_type`).

One owner per fact afterwards:

| Fact | Owner |
| --- | --- |
| which instrument a market element is about | `instcode`, a `Market` holder (D42.10): the instrument's `crosscode` as text; the element's `securityids` still hold the identifiers it stated or derived |
| the instrument's non-listing identifiers (isin, cfi, lei, cusip, wkn, valor, fisn, forex, the minted `yggdryl:isin`) | `Instrument::securityids: Identifiers` - the one map, base keys, the rank rule (`identifier.rs:507-560`); partitioned by `IdType::is_listing` (`idtype.rs:555-569`): a type that is **not** a listing type lives here alone |
| the listing codes (`ric`, `bbg`, `exchsymb`, `cta`, `sedol`, `figi`, `mktassigned`, `fim`, `umtf`, `instrumentid` - `IdType::is_listing`) | the `codes` of the `Listing` of the market the statement names; a listing code stated on **no** market (an FX RIC `EUR=`, a RIC on a message with no `SecurityExchange(207)`) lives in `securityids` under its key until a market is learned for it, when `Instruments::fold` moves it onto that listing (removed from `securityids`) - one holder at any instant, `get(&IdType)` reading `securityids` then the listings in MIC order |
| its class | `IdType::Cfi` in `securityids`; `cficode` on the row is its projection (as `isincode` is on a market row, `market.rs:447`) |
| its key | `crosscode`, written once by `Instrument::write_crosscode` from typed facts (panel D42.1) |
| its identity | `crossuuid` by `Element::sync_cross` (`element.rs:275-302`); `uuid = crossuuid` (panel D42.4) |
| its minted number | `Isin::minted` in the core (panel D42.3), called by `Instrument::mint` once |
| country, currency, origin currency | `Instrument` holders (D42.12) |
| underlying, legs (as codes), characteristics, listings | `Instrument` holders (D42.9); the uuids provided readings over the codes |

There is no instrument-level `identifiers` map (panel D42.7 lists one; amended): every type a
feed states of an instrument is a security type or a listing type, both placed above, and
`IdType::InstrumentId` (`idtype.rs:131,555-569`, an OMS instrument id) is a listing type.

## D42.9 - the `Instrument`: `rust/market/src/instrument.rs`, `characteristics.rs`, `listing.rs`

`instrument.rs` is the type file (AGENTS "One type, one file"): `Instrument`, its `Element`
implementation, its row (`Instrument::field()`), its scalar doors, `write_crosscode` and `mint`;
then `Instruments`, `InstrumentTable`, `Resolution`, `MatchTier`, `Unmatched`, `EconomicMemo`,
`Learned`, `warn_full`. `rust/market/src/characteristics.rs` holds `Characteristics`, `Settle`,
`Expiry` and `Exercise`, and `rust/market/src/listing.rs` holds `Listing` - public types of
their own, each a root file (AGENTS: a folder of a type's name holds that type's overflow, never
another type), re-exported from the crate root; `FixMsg::stated_characteristics` answers the first.
`instrument/` holds the overflow alone: `store.rs`, `env.rs`, `seed.rs` + `seed.json`.

### One element per instrument; listings nested

Panel D42.7: one row per Instrument element, its listings nested. The first draft's "one element
per listing" is **not** taken: the identity is the instrument's (`uuid = crossuuid`), and two
elements with one uuid stating different listing facts would be two statements of one thing that
`merge_with` folds (`element.rs:348-358`) - so they are one element holding every listing. The
registry's `(ISIN, market)` row model (`isin_registry.rs:192-273`, `Target::of`, `:1137-1235`)
becomes `listings: Vec<Listing>` on the instrument, `Listing { mic: Mic, ticker: Option<SmolStr>,
currency: Option<Ccy>, codes: Identifiers }` - the listing codes `IdType::is_listing` names, one
entry per market, sorted by MIC. The seed's 209 listing objects are re-expressed as 208 instrument
objects (HSBC with two listings).

### Fields

```rust
pub struct Instrument {
    // Element (the six, `ElementColumn::ALL` order): uuid (= crossuuid), crossuuid, crosscode,
    // hashcode, crosshashcode, srcuuids (two at most: the creating event, the last that moved a fact).
    aliascodes: Vec<Str>,              // the codes this instrument had before its re-keys (panel D42.5), sorted unique
    placeholder: bool,                 // keyed by a real ISIN while its body cannot be spelled (panel D42.5)
    securityids: Identifiers,          // every non-listing mapping: isin, cfi, lei, forex, fisn, cusip, wkn, ..., `yggdryl:isin`;
                                       // plus a listing code stated on no market, until its market is learned (D42.8)
    countrycode: Option<Country>,
    currency: Option<Ccy>,             // the instrument's currency (an FX pair's quote leg; a derivative's underlying's)
    origccy: Option<Ccy>,              // stated or seeded, never derived
    underlying: Option<Str>,           // the underlying instrument's crosscode - what the key is written from (panel D42.1)
    legs: Vec<Leg>,                    // the legs, sorted bytewise by code as the key sorts them (panel item 8); one holder
    eusipacode: Option<Eusipa>,
    characteristics: Characteristics,  // the typed body facts (below), `Characteristics::default()` for a cash security
    listings: Vec<Listing>,            // one per market, sorted by MIC
    updunix: Option<i64>, firstunix: Option<i64>, lastunix: Option<i64>,
}
pub struct Leg { code: Str, ratio: u32 }  // ratio 1 writes no `n*` prefix
pub struct Characteristics {           // every field nullable; the body is written from them, never parsed back per row
    settle: Option<Settle>, settle2: Option<Settle>,   // Settle = Date(Date32) | Tenor(SmolStr)
    expiry: Option<Expiry>,            // Expiry = Day(Date32) | Month { year: u16, month: u8, week: Option<u8> }
    strikepx: Option<Decimal>, multiplier: Option<Decimal>, exercise: Option<Exercise>,
}                                      // no `forex`: the pair is `securityids`' `forex` entry (below)
```

**The FX pair has one holder**: `securityids`' `forex` entry (`IdType::Forex`, the market row's
`forexcode` is its projection), never a `Characteristics` field - the panel's D42.1 list and
D42.7 struct name `forex` among the characteristics, amended here, because a pair held twice on
one element is two owners of one fact (AGENTS §1) and D42.13 persists only the projection.
`write_crosscode` reads the pair for an `I*`, `J*` or `S*` body through `get(&IdType::Forex)`
(one binary search, at the finalize, never per market row); `for_body` takes the pair as intake
and writes it into `securityids` before it spells the code.

The underlying and the legs are held as **codes**, not uuids (panel D42.7's `underlying: uuid`,
`legs: serie<uuid>` amended): the `O*`, `H*`, `F*` and `K*` bodies are written from the
underlying's and the legs' cross codes verbatim (panel D42.1), and a uuid under today's rule is a
lossy `from_v8` projection of an XXH3-64, so `write_crosscode` could not write the instrument's
own key from a uuid without a table lookup at every finalize. The user's "underlying pointing to
another instrument uuid" and "legs uuids list" are answered by provided readings:
`underlying_uuid() -> Option<Uuid>` and `leg_uuids() -> impl Iterator<Item = Uuid>`, each the
graph's cross rule over the held code (`yggdryl::implementer::crosshash`, `implementer.rs:355`,
then `Uuid::from_v8` today - the one place the market crate spells the rule, and P7 moves it with
D40.5). The ratio rides the leg it belongs to (`Leg`), the one holder, because the `n*` prefix is
a key byte. When an underlying is re-keyed (the placeholder path), the derivatives naming it are
re-keyed in the same `Instruments::rekey` (panel D42.5's cascade): their held `underlying` or
`Leg::code` rewritten to the survivor's code, their own code re-written, each pushing its old code
into `aliascodes` - so a held code is never stale, and nothing resolves a uuid to find one.

Readers: the registry's (`isin()`, `cficode()`, `country()`, `forexcode()`, `fisn()`,
`get(&IdType)`, `iter()`, `currency()`, `origccy()`, `eusipacode()`, the stamps - `isin()`,
`cficode()`, `forexcode()`, `fisn()` projections of `securityids`, one binary search each) plus
`class()` (the CFI's two class letters, `None` where unclassified), `is_placeholder()`,
`aliascodes()`, `underlying()` (the code), `underlying_uuid()`, `legs()`, `leg_uuids()`,
`characteristics()`, `listings()`, `listing(&Mic)`, `ticker(&Mic)`, `minted_isin()` (the
`yggdryl:isin` entry), `is_own_mint(&Isin)`. Builders: the registry's `with_*` normalizing as
`isin_registry.rs:579-799` does, `with_listing(Listing)`, `with_underlying(&str)`,
`with_legs(&[Leg])`, `with_characteristics`, `set_code`/`try_with_code`. **Two constructors write a
code and there is no `new(&str)`**: `Instrument::for_security(isin)` and
`Instrument::for_body(class, forex: Option<&Forex>, &Characteristics, underlying, legs)` - the
pair written into `securityids` as the body's one source, the second minting the number -
a constructor taking the code text would give the key two owners (the text and the facts
`write_crosscode` reads) that can disagree. A load (D42.13) rewrites the code from the typed
columns and refuses a row whose stored `crosscode` differs, naming both at `$.crosscode`.

**Bounds, each stated** (AGENTS §1: held state names its bound and reason; `IdType::Other(IdWord)`,
`idtype.rs:8,33`, is open, so a map is bounded only by a stated count): `MAX_EQUIVALENTS = 12`
stays, over `securityids`; `MAX_LISTINGS = 16` per instrument (the registry's `Listings` smallvec
precedent, `isin_registry.rs:134`); `MAX_LEGS` is what the 128-byte code holds (the shortest leg
code is ten bytes, so at most eleven; the speller refuses the 129th byte by name and `with_legs`
refuses before it spells); `MAX_ALIASES = 8` (one re-key per level of the placeholder cascade);
`Listing::codes` under `MAX_EQUIVALENTS` too. A statement past a bound is skipped by name as
`adopt_code` skips today (`isin_registry.rs:707-714`), never a refusal of the row. `ENTRY_COST`
(`isin_registry.rs:134-148`) is re-derived from these five bounds and `MAX_TICKER_WIDTH`, and
`const _: () = assert!(ENTRY_CHARGE >= ENTRY_COST)` holds over the new struct, so
`DEFAULT_MAX_INSTRUMENTS * ENTRY_CHARGE` (16,384 x 8 KiB) stays the memory claim.

**The LEI**: an issuer's code, held in `securityids` under `lei` as the user asked, never a lookup
key (today's `LOOKUP_CODES` exclude it because an issuer numbers many instruments,
`isin_registry.rs:2362-2403`; `LOOKUP_CODES` keeps its twenty). **RIC**: a listing code
(`is_listing`); on a message naming its market, held on that listing and read by the cascade's
code tier on that market; on a message naming none - every FX pair, which has no MIC - held in
`securityids` under its key (D42.8), so an FX instrument holds `EUR=` and a reader finds it by
`get(&IdType::Ric)`; pinned by a test of an FX instrument holding its RIC. The alternative, a
listing derived from the RIC's exchange suffix, infers a market from a vendor's spelling and is
not taken.

### The `Element` implementation (panel D42.4, spelled out)

- `get_*`/`set_*` over the struct's fields; `set_srcuuids` canonicalizes through
  `yggdryl::implementer::canonicalize_uuids` (`facts.rs:434-437`).
- `is_after` answers `false` (no instant: `facts.rs:431-433`). Not an `Event`: the three stamps
  are the instrument's facts, moved by `learn` as the registry's `fold` moves them
  (`isin_registry.rs:1137-1235`), fed to no digest (when it was met is provenance).
- `finalize`: `self.sync_cross(); self.uuid = self.crossuuid; self.hashcode =
  self.digest_instrument().as_u64();` - `digest_instrument` starts from `Element::digest`
  (`element.rs:321-327`) and feeds, by name through `yggdryl::implementer::Staged::feed`
  (`implementer.rs:448-466`, the market crate's door to the core's private `feed`,
  `element.rs:524-529`), every `securityids` entry (`src`, `type`, `value` as `feed_market` feeds
  them, `market.rs:1142-1145`), the CFI whole, `countrycode`, `currency`, `origccy`, the
  underlying's code, each leg's code and ratio, each characteristic as its canonical bytes
  (`Scalar::write_bytes`), `eusipacode`, each listing (`mic`, `ticker`, `currency`, its codes),
  `aliascodes`, `placeholder`. Not fed: the stamps, `srcuuids`, the uuids of the element itself.
- `with_previous`: `yggdryl::implementer::follow_element` (`facts.rs:452-455`); the update rule is
  `Instruments::fold` (the registry's, re-spelled: fill an empty fact, replace a differing one by
  rank, `Cfi::refined` for the class's attributes, a listing fact into the listing of the stated
  market, a listing code stated on no market into `securityids` and moved onto its listing when
  that market is learned, `warn_withheld` for a listing fact other than a code stated on no market
  where several listings exist).
- `set_crosscode` is called by the two constructors and by the one re-key (`Instruments::rekey`,
  panel D42.5): the old code pushed into `aliascodes` sorted unique (`MAX_ALIASES`), `finalize`,
  the table's `aliases` index grown (old code -> survivor, and the old code's digest -> survivor, so
  a uuid a store wrote before the re-key resolves too), the cascade into the derivatives holding the
  old code as `underlying` or a `Leg::code`. Any other call that would move a key field in place is
  refused at `$.crosscode` naming both codes.

## D42.10 - the market column `instcode`

Decision 9, panel D42.4's last row: **a market fact**, `Market::get_instcode(&self) -> Option<&str>`
/ `set_instcode(&mut self, code: Option<Str>, overwrite: bool) -> bool` on the `Market` trait
(`market.rs:147-375`), held in `MarketFacts` (`facts.rs:43-83`) as `instcode: Option<Str>` (`Str`
is 24 bytes, inline to 23, one `Arc<str>` beyond, `string.rs:2544,2576`), followed along a chain
by `following_market` (`market.rs:630`) as every unstated fact is, merged by `merging_market_event`
with the rank rule (a held value stands; `overwrite` replaces). The value is the **resolved
instrument's `crosscode`**: for a real-ISIN security the ISIN itself, for an FX pair or a
derivative the `class:body` no market element holds whole. The code is the instrument table's own
key, so a reader joins a market table to the instruments table on `instcode = crosscode` with no
lookup, and the instrument's `crossuuid` is the code's digest, so the code carries both.

**Fed to no digest** (panel D42.4's "fed to the row's `hashcode` like every fact" amended). The
`Element::digest` contract (`element.rs:304-311`, not AGENTS) says what is derived is never fed:
`instcode` is a reading of the facts the row already feeds - its `isin` for a security, its
`derived:isin` and the fields the body is written from for FX and a derivative - so feeding it
discriminates no two rows the content code does not already tell apart, and it would move the
`hashcode` and `uuid` of every resolved FIX and market row in the equivalence snapshot
(`time_uuid` reads `txhash`, the instant and the `hashcode`, `element.rs:1228-1253`). So
`instcode` is held, followed and written, as `crossuuid` is, and `digest_market`
(`market.rs:1119-1209`) does not read it; the snapshot gains its key and moves no `hashcode` cell
for it (D42.17). Feeding it is **put to the user** (#1 below) as the alternative, with that cost;
if the user picks it, the `Element::digest` rustdoc is amended in the same commit to name
`instcode` as the one derived fact that is fed.

Where it is filled - the two tiers AGENTS Ownership states, neither reading the table at the parse:

- **At the parse, from the message's own facts alone** - the local enrichment Ownership allows
  ("depend only on that event"), beside `derive_forex` and never inside `fill_instrument_ids`,
  whose rustdoc holds ("Nothing here reaches a field", `msg.rs:3883-3890`; it keeps taking derived
  `securityids` from the fixed table and nothing else): a message stating a real ISIN
  (`Isin::rank_of == 2`) writes it as `instcode` (the `isin` production: the code *is* the ISIN); a
  detected FX pair writes the `class:body` it mints its number from (D42.11). A derivative, a
  placeholder, a ticker-only security: null at the parse - the body needs the underlying's code,
  which only the table resolves. So a bronze FIX row of a security or an FX pair holds its code
  from the first parse, with `overwrite = false`.
- **In the lifecycle** (`FixMsg::fill_instrument`, `learn_and_fill`, `enrich.rs:730-760`;
  `Instruments::fill` for any market element, D42.11): learned first, then filled from the table's
  resolved instrument - `overwrite = false`, so a parse-written code stands and a null is filled -
  so every silver row of an instrument the walk met holds it; a follower takes its chain's.

A parse-written code on a bronze row of a placeholder's ISIN (a derivative first met stating only
its venue number) is the survivor's alias after the re-key, which the gold join reads
(`instcode = crosscode or instcode in aliascodes`, panel D42.5).

**`FxMemo` stays a function of the symbol text** (panel D42.6's "the memo value gains the
instrument slot" amended). `FxMemo` is held by the `FixRegistry` (`registry.rs:714-718`,
`forex.rs:32-82`), one memo shared by every codec and door reading that dictionary for the life of
the process, keyed by the trimmed `Symbol(55)`: an instrument slot there would be table-derived
state outside the snapshot one door fixed under one lock, outliving it (Ownership), and the FX
code is not a function of the symbol - `JF:EUR/USD:2027-01-15` reads the message's own
`SettlDate(64)`, the class its `CFICode(461)`/`SecurityType(167)`. So the memo keeps answering the
`FxSymbol` (pair, tenor), the message spells its code into the `[u8; 128]` from that plus its own
461/167/64/193 and mints from those bytes, and the lifecycle probes the `InstrumentTable` code
index by the same bytes. No table answer is cached in a registry-held memo.

**The `marketdata` row** (`rust/market/src/graph/arrow.rs`): `MarketColumn::InstCode`, datatype
`utf8` nullable, **after `securityids` and before `isincode`** (`market_column.rs:103-118` `ALL`
36 -> 37; the row `6 + 9 + 37 + 5 + 3 + 6 = 66`, the doc assert at `arrow.rs:140` re-pinned with
"`instcode` joined the market band, D42" and the nested slice `fields()[59..]` at `arrow.rs:141`
-> `[60..]`), so that P7 lifts `instcode`, `isincode`, `cficode`, `miccode` as one contiguous band
in D40.2's order and the row stays 66 after P7.

**The FIX row**: crate tag **65_054 `instcode`** (`utf8`, nullable), the next unused (`crated.rs`
ends at 65_053 `fixmsg`, `:267`), declared as `Crated::market(INSTCODE_TAG_NAME,
MarketColumn::InstCode, "...")` (`crated.rs:831-838`, the `MARKETDATAKIND` row the model,
`:1057`). It enters the row through `shared_tags()` (`schema.rs:312-331`), which walks
`MarketColumn::ALL` in order, so it lands **right after `securityids`** and before every later
band - **no edit to the instrument band of `fix_schema_tags`** (`schema.rs:245-262`), which `band`
would skip for a tag already placed. `fix_schema_tags` 152 -> 153 (`rust/fix/tests/root/schema.rs:95`),
`shared = 6 + 9 + 36 + 5` -> `37` (`:103`), the schema's fields 153 -> 154 (`:221`); `crated.rs`
"Fifty-two definitions" (`:102`) -> fifty-three, `held.len() == 52` -> 53
(`rust/fix/tests/root/crated.rs:722,1016`); `CRATED` gains the row; `fix_schema` reads it back as
a market fact like `securityids`. The two size pins **move once each**, re-pinned from what the
compiler reports, with one sentence each ("`instcode` joined `MarketFacts`, D42"): `MarketData`
912 (`rust/market/tests/graph/market_data.rs:120-127,152`) and `BookEvent` 896 (`:151`, which
holds `MarketEventFacts`); an `Option<Str>` is 24 bytes where `Str` leaves a niche and 32 where it
does not - the compiler says which - and a rise beyond that one field is a defect. The column
count pin at `market_data.rs:167` 65 -> 66.

`Market::stored_crosscode` and `book_crosscode` are unchanged (panel item 5): an FX element's
`isincode` is now its minted number (D42.11), so its book is `3:0:QYLTVIRYHNX5` - a book identity
that moves once (D42.17).

## D42.11 - learning: from any market element, and from a FIX message

**The generic path** (the user's "learnings from market recording / fix parsing"): today's
`IsinRegistry::learn<E: Market + Event>` (`isin_registry.rs:2912`), `fill<E: Market + Element>`
(`:3012`) and `enrich` (`:3022`) keep their signatures as `Instruments::{learn, fill, enrich}` and
key the instrument from `Market` facts alone: a real ISIN in `securityids` gives the security
(`isin` production); a `forex` entry with a CFI whose class is `I*`, `J*` or `S*` gives the FX
body - the pair from `get_forexcode`, the settle from no `Market` fact (a bare market element
states none: `Market` carries `strikepx` and `cficode`, `market.rs:160,269`, but no expiry or
settle), so a bare `J*` or `S*` element learns an instrument only where its `securityids` already
hold a minted `derived:isin` (a FIX row) and otherwise nothing; an `I*` spot mints its `QY` number
and keys `IF:`/`IT:` from the pair and class alone; a derivative stating a real ISIN and no
spellable body gives a placeholder; neither gives nothing (a ticker-only security has no
instrument, panel item 7). `fill` sets `instcode` (`overwrite = false`) from the resolved
instrument on any `Market + Element`, so an `OrderEvent`, a book fold or a text capture's market
row is filled as a FIX row is. Pinned in `rust/market/tests/root/instrument.rs`: an `OrderEvent`
stating `forex=EUR/USD` and CFI `IFXXXP` and no ISIN creates `IF:EUR/USD` through `learn`, and
`fill` writes `instcode = "IF:EUR/USD"` and `isincode = QYLTVIRYHNX5` on it.

**The FIX path** adds what only a message spells:

- **`FixMsg::stated_characteristics() -> Option<(class, Option<Forex>, Characteristics)>`**,
  beside `stated_underlying_isin` (`msg.rs:3963`) - the pair answered beside the class and the
  characteristics as intake (what `derive_forex` settled off `Symbol(55)`, else the `forex`
  entry of the message's `securityids`), for `for_body` to write into the instrument's
  `securityids`: the class from `CFICode(461)` as settled (`fix/cfi.rs`,
  `PutOrCall(201)` refining `OM` into `OC`/`OP`); `MaturityDate(541)` with `MaturityDay` folded
  (`retired.rs:1014`), else `MaturityMonthYear(200)` as a month or a week; `StrikePrice(202)`
  (already read, `msg.rs:244,2539`); `SettlDate(64)`/`SettlDate2(193)` else `SettlType(63)`;
  `ContractMultiplier(231)`, `ExerciseStyle(1194)`, `StrikeCurrency(947)` as facts under the uuid;
  the `NoLegs(555)` group's `LegSecurityID(602)`/`LegSecurityIDSource(603)` and `LegRatioQty(623)`
  for a `K*` strategy. Every value read by its field's type (AGENTS Ownership). The underlying's
  code is the code of the instrument `stated_underlying_isin` (`UnderlyingSecurityID(309)`, the
  related `Underlier`) resolves to in the table; an underlying no instrument keys gives no body
  (panel D42.1, the placeholder path). `Instruments::learn_stating` (`isin_registry.rs:2931`'s
  successor, through `implementer::instruments_learn_stating`) takes it as an override of what the
  generic path reads off the `Market` facts.
- **`derive_forex`** (`rust/fix/src/forex.rs:118-152`): once the pair, the tenor and the CFI are
  settled, the message writes the FX key into the `[u8; 128]` (`IF:EUR/USD`, `JF:EUR/USD:M3`,
  `IT:XAU/USD`, `SF:USD/JPY:0:M3` - panel D42.1), mints its number through `Isin::minted` over the
  low 46 bits of `xxh128(code)`, derives it into `securityids` under `derived:isin` beside
  `derived:forex` (`set_derived_pair`, `msg.rs:7090-7100`), so a stated ISIN always wins (rank 2
  over 1), and writes the code as `instcode` (D42.10). Pure, local to the event, no table: every
  parse door derives the same number for the same pair. **A collision is detected only in the
  lifecycle** (the parse has no table): the parse mints unconditionally; `Instruments::learn`
  meeting a second code at a held minted number refuses the mint by name (`Error::Conflict` naming
  both codes, panel D42.3), the refused instrument keeps its identity with no number, and
  `fill_unsettled` takes `derived:isin` back off that instrument's rows (the `set_derived_pair`
  overlay), so its silver rows and its book fall back to the ticker; its bronze rows keep the
  number the parse wrote. The residual - two bronze FX rows sharing one `derived:isin` and one book
  key - is accepted at `p ~ 1.9e-6` for 16,384 instruments (panel D42.3), and the refused-mint path
  is pinned (`rust/market/tests/root/instrument.rs`, `rust/fix/tests/root/enrich.rs`).
- **In the lifecycle**, `learn_stating` is keyed by the code the message's facts write: the real
  ISIN of a cash security, a `class:body` for an FX pair or a derivative whose body the message
  spells, a real-ISIN placeholder for a derivative stating a number and no body; a message stating
  neither a real ISIN nor a body learns nothing. The statement's listing is the message's
  `SecurityExchange(207)` market with its `Symbol(55)` ticker and `Currency(15)`; a listing code
  on a message naming no market lands in `securityids` (D42.8); for an FX pair `currency` is the
  quote leg and `countrycode` none (D42.12); `firstunix`/`lastunix`/`updunix` as today.
  `learn_and_fill` then fills the message's `instcode` where null (D42.10).
- Not in P9: an FX option or future CFI (none exists, `cfi.rs:224-258,313-315`); a
  SecurityDefinition (`d`) read as a definition (`MarketDataKind::Securities` exists,
  `marketdatakind.rs:79`; no cross-tag rule, no `batch_groups` arm, `fix/market.rs:953-1000`).

## D42.12 - country and currency fill

Panel D42.3's last bullet, with the registry's derivations kept (`derive_defaults`,
`isin_registry.rs:810`), each one function of the `Instrument` called by `Instruments::fold`:

- **Country**: stated where listed; else the ISIN's prefix where `StringEnum::COUNTRIES` lists it
  (`IsinEntry::country`, `isin_registry.rs:483-492`); a stated country equal to the prefix is taken
  back; an FX pair none; a derivative its underlying's where that states one.
- **Currency**: stated; an FX pair's the quote leg (`Forex::quote`, `forex.rs:132`: EUR/USD is
  dollars per euro; today's registry stores none for a pair, `isin_registry.rs:2951-2972`, and
  `fill_market`'s unit rule - the dealt currency - is untouched, `market.rs:529-546`); a
  derivative's its underlying's where it states one; a listing's currency the legal tender of its
  market's country (`Mic::country`, `Country::currency`) until a statement replaces it.
- **Origin currency**: stated or seeded, never derived.
- The embedded national numbers derive into empty `securityids` entries through
  `securityid::embedded` (`securityid.rs:34-52`), none for a `QY` prefix.

## D42.13 - the store, the environment, the seed

Carried over from `isin_registry/{store,env,seed}.rs` (`store.rs:14-138,147-435`; `env.rs:19-205`;
`seed.rs:1-90`), re-spelled over the new row:

- The row (panel D42.7, today's column names; every deviation from the panel named): the six
  element columns; `aliascodes: serie<utf8>`; `placeholder: boolean` required; `isin` (the real or
  minted number, nullable: a collision leaves a hole), `cficode: cfi`, `forexcode: forex`, `fisn:
  fisn` - **projections of `securityids`**, as `cficode` is, holders of nothing (the panel's
  characteristics struct holds `forex`; here the pair's one holder is `securityids`' `forex`
  entry, D42.9, and `forexcode` is its projection, one spelling with the market row's column); `countrycode: country`; `currency: ccy`; `origccy: ccy`; `securityids` as
  `map<utf8, utf8>` sorted (`identifier.rs:1183-1195`); **no `identifiers` column** (D42.8);
  `underlying: utf8` and `legs: serie<struct<code: utf8, ratio: int32>>` - codes, not the panel's
  uuids, for the reason D42.9 gives, the ratio carried because it is a key byte; `eusipacode:
  int32`; `characteristics: struct<settle: utf8, settle2: utf8, expiry: utf8, strikepx: decimal,
  multiplier: decimal, exercise: utf8>` (each nullable) - `settle` and `expiry` as **text**, not
  the panel's `date32`, because each is a date **or** a tenor / a month / a week code and one
  typed column cannot hold both, and because the text is exactly what the key spells, so the row
  and the code agree byte for byte; the load reads each once per row into `Settle`/`Expiry` and
  nothing is parsed per market row; `listings: serie<struct<miccode: mic, ticker: utf8, currency:
  ccy, codes: map<utf8, utf8>>>`; `updunix`, `firstunix`, `lastunix`. `PARTITION:by
  ["truncate(crosscode, 2)"]` - the ISIN's country prefix for a security, the class letters for
  everything else - `SORT:by ["crosscode"]`. The doc test on `field()` pins the count once written
  (twenty-two columns by this list; the implementing session counts the children `field_len()`
  answers and pins that).
- `Instruments::from_holder`, `from_url`, `seeded_from_holder`, `seeded_from_url`, `set_holder`,
  `try_with_holder`, `holder()`, `commit() -> Result<IOResult>`: **today's commit contract,
  unchanged** - only where dirty (`is_dirty`), the store left exactly the snapshot whatever its
  layout. **Dirty compares content with what the store holds** (implementing session, 2026-10-10,
  the medallion's second run): the collection records, at every load and commit, each instrument's
  content code (`hashcode`) and the window it was met in (`firstunix`, `lastunix`) by code
  (`Stored`, `StoredRows`; the seed's recorded once beside its table), and `is_dirty` is "something
  moved since" (the cheap flag) **and** the table differs from that record - an instrument added or
  removed, a content code or a window edge moved. A fact that moves and moves back - D42.18's
  disagreeing sources - leaves the record equal, so the medallion's replay commits nothing
  (`IOResult(0, 0)`); `updunix` follows the flips and is recorded with nothing. A whole overwrite
  where changed, as before; pinned in `rust/market/tests/instrument/store.rs`
  (`a_fact_that_moves_and_moves_back_is_no_change_to_the_store`). The paragraph's next sentence
  says what the layout does with the snapshot once a commit is owed: the store left exactly the snapshot whatever its
  layout: a leaf overwritten whole (Arrow IPC default), a plain folder one part, an Iceberg table
  whole (`Located::overwrite_whole`, `iceberg/mod.rs:273`; a partition refused at `$.holder`). Not
  the panel D42.7's "merged by `uuid` with `hashcode` the change detector": a keyed merge cannot
  delete (an instrument removed by `remove`/`clear` or folded by a re-key would stay in the store),
  a merge compares every column through Arrow's row format and reads no `hashcode` (AGENTS "IOMedia
  and records"), and a keyed merge on an Iceberg v3 table is refused before it pulls
  (`iceberg/table.rs:2929`) - the medallion creates every table at `format-version 3`
  (`medallion.py:103`). An unchanged run rewrites nothing because a clean collection commits
  nothing, which is what the medallion test asserts (D42.14). Unbound refuses as
  `Error::absent("instruments holder")`; `read_lacking`/`warn_lacking` warn once per missing column
  and migrate nothing; a store without the `crosscode` column (the 46-column registry row) is
  refused at load naming the table and saying to drop it; a row whose stored `crosscode` is not
  what its typed columns spell is refused naming both (D42.9).
- `InstrumentTable` (the snapshot): `rows: Arc<HashMap<i128, Slot>>` keyed by the packed ISIN
  (panel D42.6), `codes: Arc<HashMap<Box<str>, Slot>>` keyed by the cross code's bytes, `aliases:
  Arc<HashMap<Box<str>, Slot>>` keyed by alias code with each alias's digest beside it
  (`get_by_uuid` reads both), the ticker index (`get_by_ticker`, market-gated) and the lookup-code
  index (`LOOKUP_CODES`, twenty) as today, the minted-number index (the collision check),
  `generation`, `economic`. `fill_identifiers` (the parse) keeps its contract - derived
  `securityids` entries only; `fill_unsettled` (the lifecycle) also sets `instcode` (D42.10).
- `env.rs`: `YGGDRYL_INSTRUMENTS_URI`, else `~/.config/yggdryl/instruments/`, else seeded and
  unbound; `from_env`, `install_env`, `install_env_shared`, `autoload` under `internals`.
- `seed.rs`: `config/instruments/instruments.json` embedded byte-equal as `instrument/seed.json`,
  one object per instrument (`isin`, `cficode`, `countrycode`, `fisn`, `origccy`, `listings: [{
  miccode, ticker, currency }]`), parsed once per process; `scripts/check_instruments_seed.py
  [--sync]` keeps the registry checker's rules (the check digit, sort order, uniqueness, the MICs
  against `rust/src/mic/tables.rs`, the CFI, the FISN) over the new shape.
- Bounds: `DEFAULT_MAX_INSTRUMENTS` 16,384, `ENTRY_CHARGE` 8 KiB with `ENTRY_COST <=
  ENTRY_CHARGE` re-asserted over the new struct from the five bounds D42.9 states, `LOOKUP_CODES`
  twenty, `DEFAULT_ECONOMIC_THRESHOLD` 0.85, `MAX_TICKER_WIDTH` 64, the code at most 128 bytes.
- `internals`: `instrument::internals` and `instrument::env::internals`;
  `scripts/generate_internals.py` regenerates `lib.rs`'s block.

## D42.14 - the medallion dump

`python/tests/medallion.py`:

- `instruments(silver, namespace)` (`:241-259`) opens or creates `silver.record_keeping.instruments`
  from `Instruments.field().into_scheme_compat("iceberg")` - the D42.13 row, partitioned
  `truncate(crosscode, 2)` - and answers the `Instruments` **and the table**: `main()` registers
  the handle, `lake.tables["silver", "instruments"] = table`, because `report()` (`:501-511`) reads
  each name from `lake.tables`, which only `table_of`/`source_of` fill (`:207-231`) - without it
  the report prints `silver.instruments absent` and the live prompt's step 3d fails on the CLI
  path. A table found under the old 46-column row (a lake that ran before P9) is dropped and
  recreated by `instruments()`, since the load refuses it by name (D42.13) and silver is replayed
  from bronze (panel D42.7); the docstring and the live prompt say so in one line.
  `commit_instruments` (`:262-269`) and `commit_instruments_stage` (`:315-324`) read
  `lake.codec.instruments`; the stage key stays `silver.instruments` between `silver.fix_messages`
  and `silver.books` (`STAGES_OF`, `:369-376`); the commit is today's whole overwrite where dirty
  (D42.13), so a second run over the same capture commits nothing.
- **Finding, fixed in P9**: `TABLES` (`:379-387`) omits `silver.instruments`, and `main()`
  (`:534-540`) builds the codec with no registry (`FixCodec(FixRegistry.from_handle(...),
  exclude_msgtypes=[], threads=...)`, no `silver` local), so the CLI run's stage answers `{}` -
  which the live AWS prompt's step 3d (`LIVE_AWS_TEST_PROMPT.md:50-55`) would fail on. P9:
  `main()` builds the silver catalog first, then `table, held = instruments(silver_catalog,
  args.namespace)`, `FixCodec(..., instruments=held)`, registers the table on the lake; `TABLES`
  lists `silver.instruments` after `silver.fix_messages`; `report()` prints its row.
- `instcode` reaches every market table through the Rust schemas with no pipeline edit:
  `bronze.fix_messages` and `silver.fix_messages` (65_054), `silver.books`, `silver.orders`,
  `silver.quotes`, `silver.executions` (the 37th market column, nested rows included).
  `PRIMARY_KEY`, `REQUIRED`, `PARTUNIX` (`:83-97`) unchanged.
- `python/tests/test_fix.py::test_the_medallion_pipeline_lands_every_stage_over_two_catalogs`
  (`:5625-5830`) re-spelled plus: every silver market row whose `isincode` the seed or the walk
  resolves holds a non-null `instcode` equal to the `crosscode` of one instruments row, or held in
  its `aliascodes`; the report prints the `silver.instruments` line; a second run commits no
  instruments snapshot (`is_dirty` false, the table's snapshot count unchanged). **No FX clause**:
  the capture it reads, `rust/tests/support/ulbridge.log`, holds no FX pair (`55=CCY/CCY` matches
  no line; verified), and editing the shared capture would move `scale_ulbridge`'s pins. The FX
  `instcode = IF:EUR/USD` / `isincode = QYLTVIRYHNX5` pins (panel D42.2) sit where a hand-built FX
  message exists: `rust/fix/tests/root/forex.rs`, `rust/fix/tests/root/enrich.rs` (D42.17) and
  `python/tests/test_instrument.py` (D42.15).

## D42.15 - the bindings

Python (§3): `python/src/instrument.rs` (was `isin_registry.rs`): `#[pyclass(name = "Instruments",
frozen)]` with the registry's doors re-spelled - `__new__(max_instruments=16384)`, `field()`,
`seeded()`, `from_url`, `seeded_from_url`, `from_env`, `install_env`, `from_arrow_reader`,
`extend_from_handle`, `extend_from_arrow_reader`, `into_arrow_reader`, `commit`, `is_dirty`, `get`
(by ISIN or by cross code), `listings` (an instrument's `listings` list), `get_by_ticker`,
`get_by_code` (the alias index answers too), `get_by_uuid` (the digests of the codes and aliases),
`resolve`, `LOOKUP_CODES`, `DEFAULT_ECONOMIC_THRESHOLD`, the economic pair, `merge(dict)`,
`remove`, `clear`, `rows`, `max_instruments`, `learn`/`fill`/`enrich`, `__len__`, `__bool__`,
`__eq__`, `__repr__` - plus `mint(crosscode)` (the minted number of a code, for a reader joining on
`isin`); `remove_listing` becomes `remove_listing(isin, mic)` over the nested list. Rows cross as
`dict` through `as_py(into_scalar())` with the new keys. `Resolution` unchanged.
`FixCodec(instruments=...)`, `FixCodec.from_env()` attaching `Instruments.from_env()`,
`codec.instruments`; `FixMsg.instcode` property beside `isincode`. `python/yggdryl/instrument.py`
re-exports; `__init__.py`, `__init__.pyi`, `_native.pyi` re-spelled; `python/tests/test_instrument.py`
(the 31 registry tests re-spelled plus the mint, the FX creation - a hand-built `EUR/USD`
message's row holding `instcode = IF:EUR/USD` and `isincode = QYLTVIRYHNX5` - the placeholder
re-key and `instcode` parity), `conftest.py` sets `YGGDRYL_INSTRUMENTS_URI`, `typing_bindings.py`,
`python/benchmarks/graph.py`.

Node (§4): the request does not name Node, so **no door is added**. The existing `IsinRegistry`
door (`node/src/isin_registry.rs`, 712 lines; `node/src/fix.rs:2551-2688,3414-3418`; `node/tests/
isin_registry.test.js`, 20 tests; `node/benchmarks/graph.js`) is **re-spelled** onto `Instruments`
and the `instruments` codec option and getter, because the no-back-compat rule deletes the name it
binds; its tests re-spelled; the loader and declarations regenerated (`npm run --prefix node
build:debug`); the two docs manifests regenerated. The handoff **proposes retiring** the Node
`Instruments` door and retires nothing unasked (#7 below).

## D42.16 - docs, skills, inventories, the contract

- `docs/graph/instrument.md` replaces `docs/graph/isin-registry.md` (1,585 lines, 51 tabbed
  examples; nav `mkdocs.yml:257`): Contract (the element, the row, the identity: panel D42.1's
  grammar and D42.2's table of examples as the page's pins), Use, Seed, The minted number, Forex,
  Derivatives and strategies (the bodies), Derived facts, The update rule, Listings (and where a
  code with no market lives), Matching (the four tiers), Identity moves (the placeholder re-key and
  `aliascodes`, plainly), The underlying and the legs (codes held, uuids read), The product
  category, Persistence (the whole overwrite where dirty), The process instruments, Bounds, Edges,
  Performance (no numbers until the release run regenerates them; the bench name alone), Commands.
  Rust and Python tabs; JavaScript tabs only for the doors Node carries.
- Pages re-spelled: `docs/fix/lifecycle.md:22,655-735`, `docs/fix/capture.md:661` (65_054 in the
  crate's columns table), `docs/fix/message.md:828`, `docs/graph/schemas.md:15,19,20,45,61,77` (36
  -> 37, 153/152 -> 154/153, 65 -> 66 in the three tabs) and its positional FIX table,
  `docs/types/codes/{isin (the `QY` prefix and `Isin::minted`), fisn:206-282, cfi:257,287,
  lei:229, dti:226, mic:276, country:276}.md`, `docs/graph/{market:171,203,297, market-data:180,
  identifier:15,313,521,800, index:17}.md`, `docs/architecture.md:30,52`,
  `docs/contributing.md:77`, `docs/benchmarks.md:17`, `docs/testing.md:103,110` (`--test
  instrument`), `docs/media/iceberg.md:1443`.
- Skills: `skills/yggdryl-fix/SKILL.md:90-91,442-463` and `references/{rust:518-558,
  python:453-496, javascript:447-481}.md`, `skills/yggdryl-market-data/SKILL.md:32,181-193,387`,
  `skills/yggdryl-types/references/{rust:424-447, python:378-406, javascript:573}.md` - every
  registry recipe re-spelled; one recipe added for reading `instcode` off a row and joining the
  instruments table on `crosscode`, one for the minted number.
- Inventories by hand: `.api-inventory.txt` 7377-7470 (the three registry sections), 7692, 845,
  the `securityid` section's sentence at 7371 -> `yggdryl_market::instrument` and its submodules,
  `yggdryl_market::characteristics`, `yggdryl_market::listing`, `Isin::{minted, is_minted}`,
  `Market::{get_instcode, set_instcode}`, `MarketColumn::InstCode`, the crate tag;
  `.api-bindings.txt` 103-104, 221-222 (Python), 625-628, 798-799 (Node);
  `python scripts/check_api_inventory.py`.
- AGENTS.md: the `yggdryl-market` paragraph (`:355-356`), the Layout rows (`:480`, `:484` -> an
  `instrument.rs` row carrying the panel's grammar in one sentence, `:485`, plus
  `characteristics.rs` and `listing.rs`), Ownership (`:613-625`: "the instrument table a shared
  `Instruments` held" - the sentence's rule is kept, a parse taking derived security identifiers
  only; `instcode` at the parse is written from the message, not the table, D42.10), the
  `market.rs` graph row's `book_crosscode` sentence (the minted number keys an FX book), "What CI
  never runs" (`:2471`), `rust/market/Cargo.toml:3`, `rust/market/README.md:3`, `README.md:21`.

## D42.17 - tests, cost pins, the pins that move

Tests at the mirrored paths (AGENTS "Where a test lives"):

| Deleted | Written |
| --- | --- |
| `rust/market/tests/root/isin_registry.rs` (44), `tests/isin_registry.rs`, `tests/isin_registry/{env,seed,store}.rs` (3 + 8 + 22), `root.rs:37-38` | `rust/market/tests/root/instrument.rs`: the 44 re-spelled; panel D42.2's `crosscode`, `crosshashcode` and minted-ISIN columns as literals (`IF:EUR/USD` -> `f4bd070ccad3d90f`, `QYLTVIRYHNX5`; the thirteen rows); the `crossuuid`/`uuid` columns pinned as `Uuid::from_v8(u128::from(crosshashcode))` (the rule of the day, P7 moves them to the panel's literals); `uuid == crossuuid`; stamps and sources not fed; the placeholder re-key, `aliascodes` and the cascade into a derivative's `underlying`; `is_own_mint` true for every minted row and false for a hand-written `QY`; a collision refused by name and the refused instrument's rows losing `derived:isin`; a foreign `QY` number kept as stated; `write_crosscode` refusing a 129th byte by name; every grammar production written and the "never in the code" facts moving `hashcode` alone; an FX instrument holding its RIC in `securityids`; a listing code moved onto its listing when the market is learned; the generic path (D42.11: an `OrderEvent` creating `IF:EUR/USD` and filled); each bound skipped by name; a load refusing a stored `crosscode` its columns do not spell; `rust/market/tests/root/{characteristics,listing}.rs`; `tests/instrument.rs` (the isolated runner, child name = path), `tests/instrument/{env,seed,store}.rs`; `rust/tests/root/isin.rs` gains `minted`/`is_minted` |
| `rust/fix/tests/isin_registry.rs` + `isin_registry/env.rs` | `rust/fix/tests/instrument.rs` + `instrument/env.rs` |
| - | `rust/fix/tests/root/{codec,enrich,batch,forex,schema,store,msg,securityids,crated}.rs` re-spelled plus: a walk creates the pair's instrument and its rows' `instcode` is `IF:EUR/USD`; the parse writes `instcode` for a real ISIN and an FX pair and null for a derivative; the parse derives `QYLTVIRYHNX5` under `derived:isin` for `EUR/USD` and a stated ISIN wins; `stated_characteristics` for the option, the future and the strategy; `fix_schema_tags` 153, `shared` 37, the schema 154; 65_054 on the fixed row right after `securityids`, its index pinned; `held.len() == 53`; the dump and the hash |
| - | `rust/market/tests/graph/{market_column,arrow,book,market,facts,market_data}.rs`: 37 columns, the 66-column row (`arrow.rs:339,351,449,1918` and `market_data.rs:167`), the FX book keyed `3:0:QYLTVIRYHNX5`, the holder followed along a chain and fed nowhere, the two size pins |
| - | `cli/tests/docs_examples.rs` through the docs runner |

Cost pins:

| Pin | Moves? | Rule |
| --- | --- | --- |
| `rust/market/tests/allocations.rs:1752-2200`, the eight registry rows | re-spelled onto `Instruments`, claims equal | never upward; `learn new inline` claims the element's allocations and no more |
| new `allocations` rows (panel D42.6's list) | a thousand real-ISIN rows resolving one instrument allocate nothing after the first (the packed probe); an FX row nothing on a memo hit (the code spelled into the stack slot, probed by bytes); a mint one `Instrument` and one `String` (`set_crosscode`'s signature) and nothing else; `instcode` set from an inline code nothing, from a code past 23 bytes one refcount; the strike spellings `200`, `200.0`, `2.0E2` one code; the expiry intakes (541, 200 with or without a day or week) one text | red first |
| `rust/market/tests/iobase_calls.rs:35-90` `mod isin_registry` -> `mod instrument` | a load is the calls of one record read; a clean commit `none` | unchanged |
| `rust/market/tests/instrument/store.rs` `filesystem.costs` pins | carried over | |
| `rust/fix/tests/allocations.rs`, `iobase_calls.rs` | **unchanged**: the FX row's `derived:isin` insert lands in the `Vec` the `derived:forex` insert already grew (`set_derived_pair`, `msg.rs:7090-7100`), whose capacity holds it, and the inline `instcode` copies nothing - proven red-first on the existing FX row. A count that moves is a design answer (reserve once), never a re-pin | a moved count is a defect |
| `MarketData` 912, `BookEvent` 896 (`market_data.rs:151-152`) | move once each for the `Option<Str>` holder, from what the compiler reports, with the sentence | more is a defect |
| the benches `graph/isin_registry.rs` -> `graph/instrument.rs` (+ resolve-hit and mint rows), `fix/pipeline.rs:294,1155-1167` | direction only by `--quick` | |

Pins that move once, each with its sentence:

- the crate dump (`YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl-fix --test root
  the_committed_store_carries_the_crate_dump`): the 65_054 definition;
- the dictionary hash (`rust/fix/tests/root/store.rs:3550`, `12_613_356_107_921_639_431` today):
  "It last moved when the crate's own block gained `instcode` (65_054): the resolved instrument's
  cross code on every row, a nullable `utf8`, hashes beside the fifty-two and as a member of the
  fixed row (D42)"; the census one definition more;
- the counts: `fix_schema_tags` 152 -> 153, `shared` 36 -> 37, the schema 153 -> 154
  (`rust/fix/tests/root/schema.rs:95,103,221`), `held.len()` 52 -> 53 (`crated.rs:722,1016`),
  `MarketColumn::ALL` 36 -> 37, the `marketdata` row 65 -> 66 (`arrow.rs:140-141`,
  `tests/graph/arrow.rs:339,351,449,1918`, `market_data.rs:167`, `docs/graph/schemas.md`); the two
  sizes;
- **every FX book identity**: keyed `3:0:<ticker>` before, `3:0:QY...` after, because the FX row's
  `isincode` is now its minted number (`book_crosscode`, `market.rs:476-480`; panel D42.7 says so)
  - `rust/fix/tests/root/market.rs` and `rust/market/tests/graph/book.rs` where an FX book is
  pinned, each "the FX book took its minted number, D42";
- **every FX FIX and market row's `securityids`, `hashcode` and `uuid`**: `derived:isin=QY...` and
  the base `isin=QY...` join the map `feed_market` feeds (`market.rs:1142-1145`) -
  `rust/fix/tests/root/securityids.rs:378,1117` (`["derived:forex=EUR/USD", "forex=EUR/USD"]`
  gains the two `isin` lines) and every FX row of the equivalence snapshot, each "the FX pair
  derived its minted ISIN, D42"; the FX rows' `isincode` cells likewise;
- the equivalence snapshot (`YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo test --locked -p yggdryl-fix
  --test root equivalence`, `rust/fix/tests/root/codec.rs:3535`): every FIX row gains the key
  `instcode` (a code or null); the FX rows move as the bullet above says; **no other `hashcode`,
  `uuid`, `crossuuid` or `crosshashcode` cell moves** - `instcode` is fed nowhere (D42.10), and a
  moved one outside the FX rows is a defect. Under the alternative of #1 every resolved row's
  `hashcode` and `uuid` move too;
- `docs/assets/fix.json` and `playground.json`; `.api-inventory.txt`, `.api-bindings.txt` by hand.

Pins that must not move: every `DataTypeId` byte, rank, `Shape` position, value-stream byte and
Arrow extension name (S0); **every book identity of a real-ISIN or ticker-only instrument** (D40.6
is P7's; the FX books move as stated above); every `crossuuid` and `crosshashcode` of every market
element; the S0/S2 pins; every cost pin not named above; the FX detection's wire pins
(`rust/fix/tests/root/forex.rs` gains the `derived:isin` and `instcode` lines and loses none).

## D42.18 - metadata and sourced identifiers (user decision 10)

The user (2026-10-10, `user_decisions.md` 10): "Add also metadata in instrument to put any other
complementary infos and identifiers to put other sources securityids". Two facts, one owner each:

- **`Instrument::metadata: Metadata`** - the market crate's own `Metadata`
  (`graph/market.rs`, `BTreeMap<SmolStr, SmolStr>`, what every market element holds under
  `Market::get_metadata`; no second map type) - the complementary facts no typed field holds,
  by key. Readers `metadata()`, `set_metadata(key, value)` (an empty key or value, one past
  `MAX_METADATA_WIDTH` = 128 bytes, a new key past `MAX_METADATA` = 16 refused at `$.metadata`),
  `try_with_metadata`, `remove_metadata`. **Merged on learn by key** (`Instruments::fold`, the
  `Statement`'s `metadata` slice): a stated value fills a key the instrument lacks and replaces a
  differing one; an equal one moves nothing, so a known instrument restated allocates nothing; a
  new key past the bound is passed over with one warning per key, never a refusal of the row. Fed
  to the content code (`digest_instrument`, under `metadata`, key then value), since it is a fact
  the instrument states - so a metadata move moves `hashcode` alone, never the key or the identity.
  Two sources disagreeing on one key within a run - the FIX lines' `SecurityType(167)` `CS` and the
  bridge lines' `equity` for one instrument - replace each other at every statement and leave the
  **last statement of the run**; the commit compares **content** (below), so the flips cost the store
  nothing where the run ends as the store stands.
  **The 25th column `metadata`**, a sorted `map<utf8, utf8>` (the market row's own `metadata`
  datatype, `map_of(utf8, utf8, true)`), after `listings` and before the three stamps; read back
  through `set_metadata`, a non-text key or value refused at `$.metadata`. **Seeded from a golden
  file's columns no field reads**: a seed object's key other than the six a typed field reads
  (`isin`, `cficode`, `countrycode`, `fisn`, `origccy`, `listings`) is a metadata entry, its value
  text (`instrument/seed.rs`, `FIELD_KEYS`); a stored table's column no field reads lands in each
  row's metadata under the column's name, a text cell as it is and any other as its JSON text
  (`extend_from_arrow_reader`, the stream then landed under its own root and each row read whole -
  the ordinary store, whose columns are all the row's, keeps the fast path of D42.19's note). The
  generic `learn` carries no metadata: a market element's own `metadata` describes the event (a
  bridge's unmapped keys), not the instrument.
- **Sourced identifiers**: `securityids` holds every source's statement under its own `src:type`
  key (`ullink:isin`, `bloomberg:figi`) as `Identifiers` already does under the base rule, and the
  learn path carries them: the `Statement`'s `ids` are `(&IdKey, &str)` - every named source's
  statement of any type, and the base keys of every type but the four key types (the ISIN, the
  CFI, the pair and the short name, which have statement fields of their own), never a derivation
  nor a base key echoing one (`states_own_key`) - adopted under their own keys by
  `Instrument::from_statement`, `folded` (a listing-type key onto the listing of the stated market,
  compared by `get_from(key)`) and `Instrument::statement` (so a stored row's sourced keys survive a
  merge and a reload). The bound: `MAX_SECURITYIDS` = 2 x (`MAX_EQUIVALENTS` + 4) = 32 entries in
  all - each type's base key beside one named source - a new named key past it passed over by name
  (`accepts_equivalent(&IdKey)`); `MAX_EQUIVALENTS` keeps bounding the base types. `ENTRY_COST`
  re-derived over the new struct (the 32 entries, the 16 metadata entries at the widest key and
  value, the listing inline) and `ENTRY_CHARGE` **32 KiB** (was 16; `DEFAULT_MAX_INSTRUMENTS` x
  32 KiB = 512 MiB, the memory claim re-spelled where it is stated).
- **What a FIX message contributes** (`FixMsg::stated_metadata`, `rust/fix/src/msg.rs`
  `INSTRUMENT_METADATA_TAGS`): the Instrument component's *descriptive* fields that no typed fact
  holds, each under the dictionary's name for the field, its text trimmed, a null or empty field
  unstated: `Issuer(106)` `issuer`, `SecurityDesc(107)` `securitydesc`, `SecurityType(167)`
  `securitytype`, `SecuritySubType(762)` `securitysubtype`, `Product(460)` `product`,
  `ProductComplex(1227)` `productcomplex`, `SecurityGroup(1151)` `securitygroup`,
  `SecurityStatus(965)` `securitystatus`, `UnitOfMeasure(996)` `unitofmeasure`,
  `StateOrProvinceOfIssue(471)` `stateorprovinceofissue`, `LocaleOfIssue(472)` `localeofissue` -
  eleven, under `MAX_METADATA`. Not contributed: the component's identifiers (`securityids`), its
  market, class, currency and country of issue (typed facts), the characteristics a cross code is
  written from (`Characteristics`), the numeric and dated terms (`CouponRate(223)`,
  `IssueDate(225)`, `Factor(228)`: typed facts of a later slice, not free text), the encoded
  twins (`EncodedIssuer(349)`, `EncodedSecurityDesc(351)`), and the message's own metadata (a
  bridge's unmapped keys describe the order). The text is borrowed off the row where the field
  is text (`stated_text_by_tag`, `Cow::Borrowed`), so a message states them at no allocation;
  `learn_and_fill` hands them to `learn_stating` through `Stated::metadata: &[(&str, &str)]` for
  the learn alone - the fill reads none of it - and the capture's three pinned lines state at most
  `167=CS`, inline, so the FIX lifecycle pin moves by nothing for them.

## D42.19 - instcode shares the code's allocation (user decision 11)

The user: "Make the instcode share single allocated from instrument crosscode". The Instrument
holds its cross code **once**, as the crate's `Str` (`Instrument::crosscode`, 24 bytes, inline to
23, one `Arc<str>` beyond - `INLINE_CAPACITY`), and every other holder of the code is a clone of
that value: the table's key (`InstrumentTable::rows: BTreeMap<Str, Instrument>`, `isins:
HashMap<i128, Str>`, `aliases: HashMap<Str, Str>`, the ticker and lookup indexes' `CodeSlots =
SmallVec<[Str; 1]>`, the snapshot cursor's `after: Option<Str>`), the alias a re-key keeps, a
dependent's `underlying` rewritten by the cascade, and every `instcode` a fill writes
(`fill_unsettled`: `set_instcode(Some(instrument.crosscode.clone()), false)`). So a market row's
`instcode` is a byte copy for a code of at most 23 bytes and one refcount for a longer one - an
option's `OC:US0378331005:2026-12-18:200`, a strategy's - never an allocation, and the table copies
no code into a `SmolStr` (`crosscode_smol` is gone; a refusal that names codes - `Unmatched::
Ambiguous`, `CfiConflict`, `CurrencyConflict`, `BelowThreshold` - spells them from the `Str`'s own
storage, `Str::storage`/`into_inner`, at no copy). Pinned: `rust/market/tests/root/instrument.rs`
(a filled `instcode` of a long code shares the instrument's allocation - the two `as_str().as_ptr()`
equal, which only one `Arc<str>` makes true) and `rust/market/tests/allocations.rs` (a thousand
rows resolving one option whose code is longer than 23 bytes allocate nothing after the instrument
exists, at two corpus sizes).

**The reload's known-row path** (`extend_from_arrow_reader`): a row whose `crosscode` and
`hashcode` are a held instrument's states exactly what is held - the content code digests every
fact, the metadata of D42.18 included - so only its stamps can move, and they move where the
instrument stands, read off the landed `utf8`, `uint64` and `datetime64` leaves through their
typed accessors once per batch (AGENTS "a surface reading Arrow rows"): nothing is built per row,
no `Serie::scalar(row)`, no `from_cells`, no `merge`. Any other row - a code the table lacks, a
content code that differs (a fact moved, or a store written by a version digesting differently) -
takes the whole path as before. A 64-bit collision passing a changed row as unchanged is the
content code's own residual, accepted where `hashcode` is the change detector elsewhere.

### D42.18/19 - what the pins measured (implementing session, 2026-10-10)

- Green: `rust/market/tests/root/instrument.rs` (16: the two new tests,
  `sourced_identifiers_and_metadata_are_learned_stored_and_read_back` and
  `a_filled_instcode_shares_the_instruments_one_allocation_of_its_code`), `--test instrument` (33,
  the seed's `internal::a_seed_objects_extra_key_is_a_metadata_entry` added; the column pins 24 ->
  25 and 23 -> 24 moved once: "`metadata` joined the row, D42.18"), `--test allocations
  filling_a_long_instcode_shares_the_instruments_allocation` (a thousand rows, a 31-byte `OC:` code,
  0 allocations at 64 and 4,096), `the_instruments_snapshot_stream...` opening 11 (the 12 the phase-4
  residue named is gone: the table's keys are the instruments' own `Str`), `rust/fix/tests/root/
  enrich.rs::a_walk_learns_the_instruments_description_and_its_sourced_identifiers`.
- Re-pinned once: the FIX landing 1524/1503/1541 -> 1533/1512/1550 and batch 213 -> 214, the
  sentence naming `instcode`'s three arrays (`rust/fix/tests/allocations.rs`).
- Fixed at cause and still red, **not re-pinned** (the program's hard rule; the foreground decides):
  - `instruments_learn_a_new_instrument_into_its_row_inline`: 3 -> **1** (pin 0): the one
    remaining allocation is the element's own `Identifiers` vector, reserved once at the statement's
    count (`from_statement`); the listing is inline (`SmallVec<[Listing; 1]>`), the key a clone of
    the instrument's `Str`, every index slot inline. Zero needs `Identifiers` to hold its first entry
    inline, which moves every market element's size pins (`IDENTIFIERS_SIZE` 24, `MarketData` 928,
    `BookEvent` 912) - a design answer the design did not take. D42.17's own rule for this row -
    "the element's allocations and no more" - is what 1 states.
  - `instruments_reload_known_rows_at_a_cost_per_batch`: [981, 7253] -> **[94, 94]** (pin [56, 56]):
    the per-row path is gone (a known row is read off the landed leaves, nothing built); 94 is the
    landing of the 25-column nested row per batch - the identity of the cast with no row work
    measures the same 94 - against the flat 46-column row's 56: structural, as the FIX landing is.
  - `the_instruments_snapshot_stream_is_constant_to_open_and_reads_by_row`: the drain 9/row -> **5
    per row + 3** per doubling (pin 1 per row + 1): the nested values stream as ordered runs
    (`Listing::into_row`, `Characteristics::into_row`, `Leg::into_row`, no named struct to fold); the
    five are the row's run and one `Arc` per collection the row states - `securityids`, the
    `listings` serie, its one listing's run and that listing's `codes` - the nested row's structural
    minimum under the row-to-batch reader. A columnar builder for the snapshot would remove them
    and is not P9's.
  - `rust/fix/tests/allocations.rs::a_real_line_costs_the_same_at_every_stage_every_time`: lifecycle
    **22/16/16** (pins 12/12/11; the phase-3 measurement 20/16/16): the fresh walk collection's
    first learn pays the element's own storage - the identifiers' vector, one metadata node (all three
    lines state `Product(460)`, which the native plan implies off `SecurityType(167)`), the bridge
    line's listing codes (`Bloomberg`, and `instrumentid` under `oms:` and `ullink:` as decision 10
    carries them). The metadata read itself costs nothing: text borrowed off the row, an integer
    spelled inline (`StatedText`), read through the message's own tag index and never the fallback
    by name, whose table would have cost two per message.

## Implementation plan

One commit, one push. Phases in AGENTS order; each phase a partition of files by path, so workers
never share a file; a phase is settled by its build check and the suite it touched; the whole run
leads the chain when the last phase holding the cargo lock is settled. The caller list is phase
0's grep (D42.8), re-run until empty; `cargo check -p yggdryl-market --all-targets --keep-going
--message-format=short` after the market core lists the compile sites among them, and
`RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-market -p yggdryl-fix --no-deps` the doc links.

| Phase | Files (disjoint) | Smoke (exact) | Pins expected to move | Pins that must not |
| --- | --- | --- | --- | --- |
| 0 - the sweep script | `$S/p9/p9_sweep.py` (the P4 sweep is the model: `git show 7b566b3b2:.handoff/split/scratch/p4_sweep.py`): exact-string edits `IsinRegistry`->`Instruments`, `IsinEntry`->`Instrument`, `IsinTable`->`InstrumentTable`, `isin_registry`->`instrument(s)` per site, `with_isin_registry`->`with_instruments`, `isinRegistry`->`instruments`, `YGGDRYL_ISIN_REGISTRY_URI`->`YGGDRYL_INSTRUMENTS_URI`, each anchor asserted to match once, driven by D42.8's `git grep`; `git mv` of the module, test, seed, docs and binding files | `python3 -I $S/p9/p9_sweep.py --check`; the `git grep` of D42.8 empty | - | - |
| 1 - the core | `rust/src/isin.rs` (`minted`, `is_minted(text, digest)`, the `is_listed_prefix` rustdoc sentence on `QY`), `rust/tests/root/isin.rs` | `cargo check -p yggdryl --all-targets`; `cargo test -p yggdryl --test root isin` | none | every core pin |
| 2 - market core, with its mirrored tests | sources: `rust/market/src/instrument.rs`, `characteristics.rs`, `listing.rs`, `instrument/{store,env,seed}.rs`, `instrument/seed.json`, `lib.rs`, `implementer.rs`, `idtype.rs:669`, `securityid.rs:3` (doc links), `graph/market.rs` (the holder's verbs, the follow; not the feed), `graph/facts.rs` (the field), `graph/market_column.rs`, `graph/arrow.rs:140-141`, `graph/view.rs`; delete `isin_registry.rs` + folder; `config/instruments/instruments.json`, `scripts/check_instruments_seed.py`. Tests (each written beside its source, so the smoke column can run): `rust/market/tests/root.rs` (`:37-38` -> `instrument`, `characteristics`, `listing`), `root/{instrument,characteristics,listing,implementer,securityid}.rs`, `instrument.rs` (the isolated runner, child name = path), `instrument/{env,seed,store}.rs`, `graph/{market_column,arrow,market_data,book,market,facts}.rs`; delete `tests/root/isin_registry.rs`, `tests/isin_registry.rs`, `tests/isin_registry/` | `cargo check -p yggdryl-market --all-targets`; `RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-market --no-deps`; `cargo test -p yggdryl-market --test root instrument`; `--test root characteristics`; `--test root listing`; `--test root implementer`; `--test root securityid`; `--test instrument`; `--test graph market_column`; `--test graph arrow`; `--test graph market_data` (the sizes, re-pinned once); `--test graph book`; `--test graph market`; `--test graph facts`; `--features internals --test instrument`; `python scripts/generate_internals.py --check` | `MarketColumn::ALL` 37, the row 66, the two sizes | `allocations` claims |
| 3 - FIX crate, with its mirrored tests | sources: `rust/fix/src/{codec,enrich,msg,market,messages,forex,crated,schema,identity,build,lib}.rs`. Tests: `rust/fix/tests/instrument.rs` + `instrument/env.rs` (the isolated runner, child name = path), `root/{codec,enrich,batch,forex,schema,store,msg,securityids,crated,market}.rs`, `root/equivalence.snapshot`; delete `tests/isin_registry.rs`, `tests/isin_registry/env.rs` | `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `RUSTDOCFLAGS='-D warnings' cargo doc -p yggdryl-fix --no-deps`; `cargo test -p yggdryl-fix --test root codec`; `--test root enrich`; `--test root forex`; `--test root schema`; `--test root crated`; `--test root securityids`; `--test root msg`; `--test root batch`; `--test root market`; `--test instrument`; then the dump write and the hash test (`--test root store`) once, re-pinned with the sentence; the snapshot regenerated by its own writer once with the sentence | the dump, the hash, the census, 153/37/154, 53, the snapshot (the FX rows and the new key alone) | every non-FX `hashcode`/`uuid`/`crossuuid`/`crosshashcode` cell; fix cost rows |
| 4 - cost pins and benches | `rust/market/tests/{allocations,iobase_calls}.rs` (the registry rows re-spelled, the new rows red first), `rust/market/benchmarks/graph/{instrument,mod}.rs`, `graph.rs`, `rust/fix/benchmarks/fix/pipeline.rs`; `rust/fix/tests/{allocations,iobase_calls}.rs` are run, not edited (D42.17) | `cargo test -p yggdryl-market --test allocations instrument`; `--test iobase_calls instrument`; `cargo test -p yggdryl-fix --test allocations forex`; `--test iobase_calls`; `cargo bench -p yggdryl-market --bench graph -- instrument --quick` | the allocations rows re-spelled | no cost pin rises |
| 5 - Python | `python/src/{instrument,fix,lib}.rs`, `python/yggdryl/{instrument.py,__init__.py,__init__.pyi,_native.pyi}`, `python/tests/{test_instrument.py,test_fix.py,conftest.py,typing_bindings.py,medallion.py}`, `python/benchmarks/graph.py` | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_instrument.py -x -q`; `... -m pytest python/tests/test_fix.py -k medallion -x -q` (the user's named validation); the `mypy --strict` line of §3 | - | - |
| 6 - Node re-spelled | `node/src/{instrument,fix,lib}.rs`, `node/{binding.js,binding.d.ts}`, `node/tests/{instrument.test.js,instrument.types.ts,fix.test.js}`, `node/benchmarks/graph.js`; then `node/index.js`, `node/index.d.ts` regenerated | `npm run --prefix node build:debug`; `node --test node/tests/instrument.test.js`; `node --test node/tests/fix.test.js`; `npm run --prefix node test:package:debug` | the loader and declarations | no new door |
| 7 - docs manifests | `docs/assets/{fix,playground}.json` | `node scripts/build_docs_fix.js && node scripts/build_docs_playground.js`, then `--check` | both | - |
| 8 - docs and skills | `docs/graph/instrument.md` (new), delete `docs/graph/isin-registry.md`, `mkdocs.yml`, the pages of D42.16 (`docs/graph/schemas.md` included), `skills/**` | `python -m mkdocs build --strict --config-file mkdocs.yml`; the three `python scripts/check_docs_examples.py --lang {rust,python,javascript}` as chain steps | - | - |
| 9 - inventories, contract | `.api-inventory.txt`, `.api-bindings.txt`, `AGENTS.md`, `.github/ci/rows.toml:47`, `rust/market/{Cargo.toml,README.md}`, `README.md`, `.handoff/next/LIVE_AWS_TEST_PROMPT.md` (the one-line drop of a pre-P9 instruments table) | `python scripts/check_api_inventory.py`; `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py`; `python3 scripts/release_packages.py version` | - | - |
| 10 - the chain | `$S/logs/chain.sh` adapted: the three crates' whole runs `--all-features --no-fail-fast`, clippy both lanes, `cargo doc -D warnings`, the rustdoc examples, the CLI tests, `pytest python/tests`, Node, the manifests, mkdocs, the inventories, the docs runners | one background script, one log, read once | the pins of phases 2-3 only | everything else |

Then, in this order (the program's standing rules, `.handoff/next/MARKET_SPLIT_CONTINUE.md`):

1. `cargo fmt --all` once after the last worker; the commit with the message file, ending with
   exactly `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and
   `Claude-Session: https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp` and no other
   `Co-Authored-By`, whatever the harness's attribution reminder says.
2. `$S/r1/r1_release.sh --prove` on that clean committed tree (it refuses a dirty one, so it is
   no chain step), its log kept under `$R1_OUT` for step 4 (d43 F1).
3. One push; the CI run read to `CI result` (the change touches `rust/src/isin.rs`, so every job
   runs); a red job fixed at cause in a commit of its own, pushed, read again.
4. The results commit, with the same two lines: DESIGN.md gains D42 and D43 (a row each in the
   decisions table, the two designs' sections) and `### P9 results` holding the CI run and the
   `--prove` log of step 2; the P7 and P5R documents amended as "What P7 changes" says -
   DESIGN.md "## P7: design" (D40.2 `:2759,2766`, D40.3 `:2770-2784`, D40.5 `:2806`, reading 3
   `:2874`, the D40 row `:2970`), `$S/p7/d40_design.md` (`:67-75`, `:78-92`, `:104-105`, `:114`,
   `:164`, `:182`) and `$S/p5r_manager_prompt.md` (`:9`): `instuuid` dropped, 65_054 =
   `instcode`, the lifted band `instcode, isin, cfi, mic`, D40.3's derivation superseded,
   `Uuid::new(u128)` where they spell `Uuid::from_u128`, the registry's files, seed path and
   `check_isin_seed.py` re-spelled onto the instrument's; d43's F5 (DESIGN.md `:17`, `:89`, `:2931`:
   0.1.22 on the branch since `0f411f5ce`, published by the merge), F9 (`MARKET_SPLIT_CONTINUE.md`'s
   release step names `r1_release.sh --prove` as the first release's proof and the merge as the
   release) and F10 (AGENTS.md `:2927`: "the Cargo secret; PyPI and npm trusted publishing");
   `MARKET_SPLIT_NEXT.md`'s `State`, `Checks` and `Next`. Pushed, CI read to `CI result`.

## What P7 changes

Written here so P7's design (DESIGN.md "## P7: design", `$S/p7/d40_design.md`) and P5R's brief
(`$S/p5r_manager_prompt.md:9`) are amended before they are implemented - by P9's results commit
(plan step 4), not by this file alone:

1. **`instuuid` is dropped from D40.2 and D40.3.** P9 lands `instcode` (decision 9) as the market
   holder after `securityids` and the crate tag 65_054. D40.3's derivation -
   `Uuid::from_u128(xxh3_128(isin))` of the real ISIN, a provided reading - is superseded: the row
   carries the code, and a reader who wants the uuid digests it (for a real-ISIN security that is
   D40.3's value byte for byte once D40.5 lands, panel D42.2). D40.2's lifted band becomes
   `instcode, isin, cfi, mic` (then `bbg`, `figi`, `forex` on the FIX row), lifted together from
   where P9 leaves them contiguous; the `marketdata` row stays 66 (`6 + 9 + 33 + 5 + 3 + 4 + 6`).
   65_054 keeps its number and its name. **No market `instcode` cell moves at P7**: it is text.
2. **D40.5 moves every cross identity once, the instruments' included**: `Element::cross_uuid` to
   the raw XXH3-128 (`Uuid::new(u128)`, `uuid.rs:441`, is the constructor that keeps every bit; D40's
   `Uuid::from_u128` does not exist), so every instrument's `crossuuid` and `uuid`, the
   `underlying_uuid()`/`leg_uuids()` readings, the alias digests and the panel D42.2 `crossuuid`
   column take their final values at P7 - re-pinned once with D40.5's sentence; the minted `QY`
   numbers do **not** move (the mint read `xxh128` directly in P9, see "What the panel decided");
   after P7 `Instrument::mint` and `is_own_mint` read `crossuuid` instead and the `xxh128` call
   goes. The medallion's instruments table and silver tables are **rebuilt from bronze after P7**
   (panel D42.7): the instruments store written by P9 holds P9's uuids, and a replay rewrites them.
3. **D40.4's renames apply to P9's columns uniformly**: the `instrument` row's `cficode` -> `cfi`,
   `countrycode` -> `country`, `forexcode` -> `forex`, `eusipacode` -> `eusipa`, `miccode` ->
   `mic` (the nested `listings` too), the seed's keys; `Instrument`'s readers `cficode()` ->
   `cfi()`, `forexcode()` -> `forex()`; `Market::get_isincode` -> `get_isin`, `get_cficode` ->
   `get_cfi`; `book_crosscode` reads the lifted `isin`.
4. **D40.3's one hold map** moves the market element's `cficode` into `securityids`; the Instrument
   already holds its CFI there. D40.6 is untouched by P9.

## Put to the user (interpretations taken; say if another was meant)

1. **`instcode` is fed to no digest** (the `Element::digest` contract, `element.rs:304-311`: a
   lookup's answer is derived; the panel's D42.4 row amended): held, followed along a chain,
   written to every row, and no `hashcode` or `uuid` cell moves for it, so the equivalence
   snapshot gains the key alone. The alternative - fed like every stated fact - moves the
   `hashcode` and `uuid` of every resolved FIX and market row once and amends the `Element::digest`
   rustdoc; say which before phase 3 regenerates the snapshot.
2. **One element per instrument, listings nested** (panel D42.7), the dump one row per instrument,
   committed whole where dirty (today's contract, not a keyed merge: D42.13); the seed re-expressed
   as 208 instrument objects.
3. **P9 leaves the instrument uuids on today's hashing rule** (`from_v8` over the XXH3-64); the
   user's "correct graph defined hashing in uuids" lands with P7 (D40.5) in this PR, before the
   merge - d43's gate names it. The mint reads `xxh128(crosscode)` directly in P9 so the panel's
   `QY` literals hold now and survive P7.
4. **An FX pair's currency is its quote leg**, its country none; a derivative takes its underlying's.
5. **The underlying and the legs are held as codes**, with the uuids provided readings over them
   (`underlying_uuid()`, `leg_uuids()`), the row columns `underlying: utf8` and `legs:
   serie<struct<code, ratio>>` - because the key is written from the codes (panel D42.1) and a uuid
   under today's rule cannot give the code back; `NoLegs(555)` is read in P9 for a `K*` strategy.
6. **A ticker-only security has no instrument** and a null `instcode` (panel item 7); a derivative
   whose body no message spells is a placeholder under its real ISIN; the parse writes `instcode`
   only where the code is a function of the message alone (a real ISIN, an FX pair) and the
   lifecycle fills the rest.
7. **Node keeps its registry door re-spelled** (`Instruments`, the `instruments` codec option); the
   handoff proposes retiring it and retires nothing unasked.
8. **The environment variable and the default folder are renamed** (`YGGDRYL_INSTRUMENTS_URI`,
   `~/.config/yggdryl/instruments/`); the seed moves to `config/instruments/instruments.json`; a
   store written under the old row is refused by name, and the medallion drops and recreates its
   table.
9. **The CLI `medallion.py main()` binds the instruments** to the codec and registers the table on
   the lake so the live AWS run's step 3d can see the table and the `instcode` column.
10. **A RIC on no market lives in `securityids`** (every FX pair's); a listing code stated on a
    market lives on that listing alone; there is no instrument-level `identifiers` map.
11. **Not in P9**: SecurityDefinition/SecurityList read as definitions, FX option or future CFIs,
    keying books by the instrument's code (panel item 5), the P8 identifier index, a collision
    check at the parse (the lifecycle alone refuses a mint).
