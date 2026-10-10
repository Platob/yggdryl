# P9: design (D42) - the Instrument replaces the ISIN registry

The user's instruction (2026-10-10, `$S/p9/user_instruction.md`, verbatim there, typos the
user's), six items, designed as one slice. The user's later decision (`user_decisions.md` 6 and
7) places it: P9 lands **first** in PR #209, on today's tree - today's column names (`isincode`,
`cficode`, `miccode`), today's cross-identity rule in `rust/src/graph/element.rs` (D40.5 has not
landed), the market column spelled `instrumentuuid` - before P7, P8 and P5R, and ships in release
0.1.22. R1 (the release plumbing) is already committed as `0f411f5ce` (`$S/r1/d43_design.md`).
One commit, one push, CI read; the crate dump, the dictionary hash, the census, the equivalence
snapshot's keys and `fix.json`/`playground.json` move once each, with their sentence.

The cross code's grammar is decided by a separate judge panel writing
`$S/p9/crosscode_decision.md`. That file did not exist when this design was written, so "## The
cross code" below is this design's recommendation, marked to be replaced; every other section
reads the cross code through one function, `Instrument::crosscode_of(class, isin)`, and depends on
nothing but the two facts it is built from.

Every claim below names the file and line it was read from; the implementing session verifies
each by reading the file (no command was run to write this).

## The user's asks, each mapped to a decision

| # | The ask (the user's words) | Decision |
| --- | --- | --- |
| 1 | "a first release with market and fix splitted" | R1, committed `0f411f5ce`; recorded in `$S/r1/d43_design.md`; the merge publishes 0.1.22 after P9, P7, P8, P5R and the live AWS run (`user_decisions.md` 7) |
| 2 | "replace current isin registry into our own market/instrument.rs ... graph Element implementations containing all mapping for isin, cfi, lei, ric with learnings from market recording / fix parsing, thus filling the value of market instrumentuuid and dumped in medaillon" | D42.1 (what is deleted), D42.2 (`Instrument`, an `Element` holding every mapping in `securityids`), D42.3 (identity), D42.8 (`instrumentuuid`, a provided reading of every market element and the FIX row's crate tag 65_054), D42.10 (the medallion's `silver.instruments` table and the column on every market table) |
| 3 | "create custom isin codes, fill country currency" | D42.5 (the `QZ` custom number, deterministic, check-digit closed, rank 1), D42.6 (country and currency fill rules) |
| 4 | "autocreating for forex pairs which dont have existing isin" | D42.7 (an FX pair detected off `Symbol(55)` mints its instrument at the parse, the lifecycle learns it, its book is keyed by its custom ISIN) |
| 5 | "optional instrument underlying pointing to another instrument uuid ... and legs uuids list" | D42.2: `underlying: Option<Uuid>` (the underlying instrument's `crossuuid`), `legs: Vec<Uuid>` in stated order, no ratio and no side held in P9 (put to the user) |
| 6 | "characteristics to handle generic finance products like future options strikepx ... include it synthetically in cross code then correct graph defined hashing in uuids" | D42.4 (`Characteristics`, a typed value with one canonical spelling; it enters the cross code through the custom number's national part, so nothing is counted twice); the hashing correction is D40.5 (P7), which P9 prepares by routing every cross-uuid derivation through one core door (D42.3) |
| 7 | "instrument cross uuid should rely on cficode + isincode, and fill the market intrumentuuid with this cross uuid" | D42.3: the cross code is the CFI's class and the ISIN; `crossuuid` derives from it by the graph's one rule; `Market::instrumentuuid()` answers the same function over the element's own `cficode` and `isincode`, so a market row and the instrument it names carry one value |

## D42.1 - what is deleted, and what owns each fact afterwards

Deleted in the one commit, no alias, no shim, no dual reader (AGENTS "No back-compat"):

| Deleted | Where | Replaced by |
| --- | --- | --- |
| `IsinRegistry`, `IsinEntry`, `IsinTable`, `MatchTier`, `Resolution`, `Unmatched`, `EconomicMemo`, `Learned`, `warn_full` | `rust/market/src/isin_registry.rs` (3,510 lines), `lib.rs:18,30,112-113` | `Instruments` (the collection a process learns into), `Instrument` (one listing, an `Element`), `InstrumentTable` (the snapshot a parse door fixes), `Resolution`/`MatchTier`/`Unmatched` kept under the same names in `instrument.rs` (they describe a match, not the registry), `EconomicMemo`, `Learned`, `warn_full` moved with the same meaning |
| `isin_registry/store.rs`, `env.rs`, `seed.rs`, `seed.json` | `rust/market/src/isin_registry/` | `rust/market/src/instrument/{store,env,seed,characteristics}.rs`, `instrument/seed.json` |
| `isin_registry_as_table`, `isin_registry_learn_stating` and the four `pub use` | `rust/market/src/implementer.rs:32-39,160-179` | `instruments_as_table(&Instruments) -> &InstrumentTable`, `instruments_learn_stating(..) -> Learned`, `pub use instrument::{EconomicMemo, InstrumentTable, Learned, warn_full}` |
| `FixCodec::{with_isin_registry, isin_registry}`, the field `isin_registry` | `rust/fix/src/codec.rs:527,533,1081-1098` | `with_instruments(Arc<Mutex<Instruments>>)`, `instruments()` (the shared collection), the crate-private snapshot door keeps its name `instruments()` only where no clash - it is renamed `instrument_table()` |
| `Codes::{Walk(IsinRegistry), Shared(..)}` | `rust/fix/src/enrich.rs:707-760` | the same two arms over `Instruments` |
| `FixMsg::{fill_instrument_ids, fill_instrument, refill_instrument_ids}` over `IsinTable` | `rust/fix/src/msg.rs:3891-3915,4070` | the same three verbs over `InstrumentTable` |
| `config/isin/instruments.json`, `scripts/check_isin_seed.py`, the inert path in `.github/ci/rows.toml:47` | the seed and its checker | `config/instruments/instruments.json`, `scripts/check_instruments_seed.py`, the inert path re-spelled |
| `YGGDRYL_ISIN_REGISTRY_URI`, `~/.config/yggdryl/isin/` | `isin_registry/env.rs:19-20` | `YGGDRYL_INSTRUMENTS_URI`, `~/.config/yggdryl/instruments/` |
| Python `IsinRegistry`, `Resolution` classes, `FixCodec(isin_registry=)`, `codec.isin_registry`, `yggdryl.isin_registry` module | `python/src/isin_registry.rs`, `python/src/fix.rs:2827-3055`, `python/yggdryl/isin_registry.py`, the stubs | `Instruments`, `Resolution`, `FixCodec(instruments=)`, `codec.instruments`, `yggdryl.instrument` |
| Node `IsinRegistry`, `isinRegistry` option and getter | `node/src/isin_registry.rs`, `node/src/fix.rs:2551-2688` | `Instruments`, `instruments` - the existing door re-spelled, nothing added (§4) |
| `docs/graph/isin-registry.md`, the nav line `mkdocs.yml:257` | | `docs/graph/instrument.md`, nav `Instrument: graph/instrument.md` |
| the 46-column `isinregistry` row | `isin_registry.rs:73-107,305-334` | the `instrument` row of D42.2; a store written under the old row is not read (D40.4's stance): the instruments table is rebuilt from the seed and the stream |

One owner per fact afterwards:

| Fact | Owner |
| --- | --- |
| which instrument an element is about | the element's own `securityids` (`isincode` its projection, `market.rs:447`) plus `cficode`; `Market::instrumentuuid()` reads them |
| the instrument's identifiers (isin, cfi, lei, ric, cusip, sedol, figi, bbg, fisn, forex, ...) | `Instrument::securityids: Identifiers` - the one map, base keys, the rank rule of `identifier.rs:507-560` |
| the instrument's class | `IdType::Cfi` in `securityids` (the Instrument never holds a second `cficode`; the typed column `cficode` is a projection, as `isincode` is on a market row) |
| its country, currency, origin currency | `Instrument` holders, filled by D42.6 |
| its underlying, legs, characteristics | `Instrument` holders (D42.2, D42.4) |
| the custom number | `Instrument::custom_isin(class, &Characteristics) -> Isin`, the one minting function (D42.5) |
| the cross code | `Instrument::crosscode_of(class, isin)` (D42.3, grammar per the panel) |
| the cross uuid derivation | the core's `Element::cross_uuid` through one new `yggdryl::implementer::cross_uuid_of(crosscode: &str) -> Uuid` forwarder (D42.3) |

## D42.2 - the `Instrument`: `rust/market/src/instrument.rs` and `rust/market/src/instrument/`

`instrument.rs` is the type file (AGENTS "One type, one file"): the `Instrument` element, its
`Element` implementation, its row (`Instrument::field()`), its scalar doors (`into_scalar`,
`from_scalar`), then the `Instruments` collection and the `InstrumentTable` snapshot, `Resolution`,
`MatchTier`, `Unmatched`, `EconomicMemo`, `Learned`, `warn_full`. The folder beside it holds what
is too big for one file: `instrument/characteristics.rs` (D42.4), `instrument/store.rs`,
`instrument/env.rs`, `instrument/seed.rs` + `seed.json` - the four modules `isin_registry/`
has today, re-spelled, their contracts carried over (D42.9).

### One instrument, one listing per market, one identity

**Listings stay rows, and each row is an `Instrument` element.** Today's registry keeps one
`IsinEntry` per (ISIN, market), the instrument facts rewritten on every listing row and the listing
facts (`miccode`, `ticker`, `currency`, the `IdType::is_listing` codes - `ric`, `bbg`, `sedol`,
`figi`, ... `idtype.rs:555-569`) on the row alone (`isin_registry.rs:192-273`; `Target::of`
picks the row, `:1137-1235`). The graph already has the word for several elements that are one
thing: they share a cross code, and so a `crossuuid`, while each keeps its own `uuid`
(`element.rs:189-203`). So: every listing of one instrument is an `Instrument` element stating the
same instrument facts and its own listing facts; all of them share `crosscode`/`crossuuid`
(D42.3); each has its own content `uuid`. The medallion's `silver.record_keeping.instruments`
stays one row per (ISIN, market), now with the element columns in front, and `instrumentuuid` on
a market row joins every listing of its instrument through `crossuuid`. The seed (209 listings of
208 instruments, HSBC on `XHKG` and `XLON`, `config/isin/instruments.json`) is kept row for row
under the new keys. The alternative - one element per ISIN with listings nested as a serie of
structs - was refused: it changes the dump's shape and the seed for no fact gained, and `Target`'s
per-market rules (`fold`, `warn_withheld`) would have to be rewritten over a nested column.

### Fields

```rust
pub struct Instrument {
    // Element (the six, `ElementColumn::ALL` order): uuid, crossuuid, crosscode, hashcode,
    // crosshashcode, srcuuids.
    // The identifier mappings - isin, cfi, lei, ric, cusip, sedol, figi, bbg, fisn, forex,
    // and every other `IdType` the registry's 32 equivalent columns held - one map, base keys,
    // the rank rule of `Identifiers` (identifier.rs:507-560). `isin` is required (base key
    // present, rank >= 1 and real or custom: a typo is never a key).
    securityids: Identifiers,
    countrycode: Option<Country>,      // the country of issue (D42.6)
    currency: Option<Ccy>,             // the listing's trading currency (D42.6)
    origccy: Option<Ccy>,              // the origin currency, stated or seeded, never derived
    miccode: Option<Mic>,              // the listing's market; none for an instrument listed nowhere
    ticker: Option<SmolStr>,           // the listing's ticker, 1..=64 bytes trimmed
    underlying: Option<Uuid>,          // the underlying instrument's crossuuid (item 4)
    legs: Vec<Uuid>,                   // the legs' crossuuids, in the order stated (item 4)
    eusipacode: Option<Eusipa>,        // the structured product's category, as today
    characteristics: Characteristics,  // D42.4, `Characteristics::None` for a plain security
    updunix: Option<i64>, firstunix: Option<i64>, lastunix: Option<i64>, // the three stamps, as today
}
```

Readers are the registry's today (`isin()`, `cficode()`, `country()`, `forexcode()`, `fisn()`,
`get(&IdType)`, `iter()`, `miccode()`, `ticker()`, `currency()`, `origccy()`, `eusipacode()`,
the stamps) plus `underlying()`, `legs()`, `characteristics()`, `class()` (the CFI's category
and group, D42.3); `isin()`, `cficode()`, `forexcode()`, `fisn()` and `get` are projections of
`securityids` (`Identifiers::get(&IdType)`, one binary search). Builders are the registry's
`with_*`/`set_code`/`try_with_code` (each normalizing as `isin_registry.rs:579-799` does; the
thirteenth-type bound `MAX_EQUIVALENTS = 12` goes: the map is bounded by `IdType`'s members)
plus `with_underlying`, `with_legs`, `with_characteristics`. `Instrument::new(isin)` takes a real
or custom ISIN and refuses a typo or a masked one at `$.isin`, naming the rank it found.

**The LEI.** An LEI identifies the issuer, an entity, never an instrument (`idtype.rs`, `Lei` is
FIX `SecurityIDSource(22)` `T`; the registry stores it as an equivalent and excludes it from
`LOOKUP_CODES` for that reason, `isin_registry.rs:2362-2403`). It is held in `securityids` under
`lei` as the user asked - a mapping of the instrument - and it is never a lookup key: a lookup by
`lei` is `Unmatched::Ambiguous` wherever two instruments share it, which is to say always for a
real issuer. `LOOKUP_CODES` keeps today's twenty.

**RIC.** `IdType::Ric` is a listing code (`is_listing`): it is held on the listing's row, read by
the cascade's code tier on that market as today (`MatchTier::Code(Ric)`), and a RIC's exchange
suffix (`Ric::exchange_code`) is not consulted - the row's `miccode` is the market fact.

### The row (`Instrument::field()`), today's names

The required struct `instrument`, `PARTITION:by ["truncate(isin, 2)"]` (the prefix partition is
kept: a custom number's constant `QZ` prefix is one partition, which is what a derivatives
partition should be), `SORT:by ["isin", "miccode"]` (P7 renames to `["isin", "mic"]`). Columns:

| # | Column | Type | Note |
| --- | --- | --- | --- |
| 1-6 | `uuid`, `crossuuid`, `crosscode`, `hashcode`, `crosshashcode`, `srcuuids` | `ElementColumn::ALL` | `srcuuids` nullable as on every element row |
| 7 | `isin` | `isin`, required | the key: real or custom |
| 8 | `cficode` | `cfi` | projection of `securityids`; the detailed code where known |
| 9 | `countrycode` | `country` | |
| 10 | `currency` | `ccy` | the listing's |
| 11 | `origccy` | `ccy` | |
| 12 | `forexcode` | `forex` | projection |
| 13 | `fisn` | `fisn` | projection |
| 14 | `miccode` | `mic` | |
| 15 | `ticker` | `utf8` | |
| 16 | `securityids` | `map<utf8, utf8>` sorted | every mapping, as a market row's `securityids` column is laid out (`arrow.rs` `side_pairs` rule: base keys, `src:type` keys as the map holds them) |
| 17 | `underlying` | `uuid` | |
| 18 | `legs` | `serie<uuid>` | |
| 19 | `eusipacode` | `int32` | |
| 20-29 | `kind`, `tenor`, `strikepx`, `strikeccy`, `putcall`, `expiry`, `exercise`, `multiplier`, `maturity`, `coupon` | D42.4's flat columns | the `Characteristics` value laid out; `kind` is its variant word, every other column null where the variant has no such fact |
| 30-32 | `updunix`, `firstunix`, `lastunix` | `datetime64(ns, UTC)` | |

Thirty-two columns where the registry had forty-six: the thirty-two per-type equivalent columns
(`idtype.rs:249`, `FIX_SECURITY_SOURCES` 33 less the ISIN) are the `securityids` map now, one
owner. The doc test on `field()` asserts `field_len() == 32`.

### The `Element` implementation

Modelled on `MarketFacts` (`rust/market/src/graph/facts.rs:388-526`), the undated pattern:

- `get_*`/`set_*` for the six over the struct's own fields; `set_srcuuids` canonicalizes through
  `yggdryl::implementer::canonicalize_uuids` as `facts.rs:434-437` does.
- `is_after` answers `false` (no instant, no predecessor: `facts.rs:431-433`). **The Instrument is
  not an `Event`**: reference data has no transaction instant, no state, no sequence; its three
  stamps (`firstunix`, `lastunix`, `updunix`) are its own facts, moved by `learn` exactly as the
  registry moves them today (`fold`, `isin_registry.rs:1137-1235`), and they are **not fed to the
  digest** - when an instrument was met is provenance, as `srcuuids` are, not what it states - so
  two processes learning the same facts from different captures derive one `uuid`.
- `finalize`: `self.sync_cross(); self.hashcode = self.digest_instrument().as_u64(); self.uuid =
  Uuid::from_v8(u128::from(self.hashcode)); self.crossuuid = self.cross_uuid();` - the exact
  sequence of `MarketFacts::finalize` less `fill_market`. `digest_instrument` starts from
  `Element::digest` (the cross code fed under its name, `element.rs:321-327`) and feeds, by name
  and only where stated, through the typed accessors: every `securityids` entry (`src`, `type`,
  `value` as `feed_market` feeds them, `market.rs:1119-1209`), `countrycode`, `currency`,
  `origccy`, `miccode`, `ticker`, `underlying`, each leg in order, `eusipacode`, and the
  characteristics' canonical text (D42.4) under `characteristics`. Not fed: the stamps, the
  sources, the uuids.
- `with_previous`: `yggdryl::implementer::follow_element` only (`facts.rs:452-455`), so a listing
  stated twice folds its cross code and sources; the instrument-level update rule is **not**
  `merge_with` (whose identity is `uuid` equality, `element.rs:348-358`, and two statements with
  different content have different uuids): it stays the explicit `Instruments::merge` /
  `Instruments::fold` of today (`isin_registry.rs:2731,1137-1235`), re-spelled over
  `Instrument`.
- `srcuuids`: **empty**, always, for an instrument learned from a stream or a seed. The registry
  learns from thousands of events per instrument, and a list of their uuids would grow without
  bound (the concern D40.6 states for books). A caller stating sources through `set_srcuuids`
  keeps them; `learn` adds none. Said plainly so no reader expects provenance there: the stamps
  say when, the medallion's `silver.fix_messages` says which.

Stable across processes: `uuid` is `from_v8(hashcode)` over facts fed by name through typed
accessors with no clock, no pointer and no process state; `crossuuid` is a function of the cross
code text alone (D42.3); the custom number is a function of the canonical characteristics
(D42.5). The same statement on two machines is one element.

## D42.3 - the identity: `crosscode` from the class and the ISIN, `crossuuid` by the graph's rule

`Instrument::crosscode_of(class: Option<[u8; 2]>, isin: &str) -> String` is the one speller, and
`Instrument::class()` the one reader of the class: the CFI's category and group letters -
`ES` of `ESVUFR`, `IF` of `IFXXXP`, `OC` of `OCASPS` - read off `securityids`' `cfi` through
`Cfi::parsed` (`cfi.rs:583-619`), `None` where the instrument holds no classified CFI. **The class,
not the detailed code**, because the registry refines a coarse CFI into a detailed one as the
stream states more (`Cfi::refined`, `cfi.rs:652-690`; `fold`'s CFI rule) and an identity that
moved on every refinement would re-key an instrument several times a day; the class is what
`Cfi::refined` never changes (`cfi.rs:656-659`: two codes of another category or group fold to
none). The class is what the user's item 7 asks the identity to carry beyond the ISIN: a listed
call (`OC`) on a share and the share (`ES`) with one ISIN in a venue's symbology are two
instruments; an FX spot (`IF`) and an FX forward (`JF`) on one pair are two.

Where the class is unknown the spelling is the two letters of `Cfi::UNCLASSIFIED`, `XX`. When the
class is learned later - a message states `CFICode(461)`, the seed states it, `fill_unsettled`
fills it - **the instrument's cross code moves once**, from `XX:<isin>` to `<class>:<isin>`, as a
custom ISIN moves once to a real one (D42.5): `Instruments::fold` re-keys the instrument's listings
and re-finalizes them. A market row written before that carries the earlier `instrumentuuid`; the
instruments table holds the current one; the medallion's silver tables are derived and a rerun of
the window lands the settled identity (every silver stage overwrites the partitions its rows
reach, `medallion.py:272-296`). The seed states a class for every common instrument, and an FX
pair gets its class at detection (D42.7), so the move is the exception, not the rule.

`crossuuid`: the Instrument implements nothing of its own. `Element::sync_cross`
(`element.rs:275-292`) sets `crosshashcode = crosshash(crosscode)` (XXH3-64) and `crossuuid =
cross_uuid()` - today `Uuid::from_v8(u128::from(crosshashcode))` (`element.rs:297-302`), after P7
`Uuid::from_u128(xxh128(crosscode))` with no version bits (D40.5). P9 does not change the rule; it
makes sure there is **one derivation the market crate can call on a text**, so that
`Market::instrumentuuid()` (D42.8) answers exactly what the Instrument's `sync_cross` wrote:

- the core grows one forwarder in `rust/src/implementer.rs`: `#[inline] pub fn
  cross_uuid_of(crosscode: &str) -> Uuid` - `Uuid::from_v8(u128::from(crosshash(crosscode)))`
  today, the body D40.5 changes - and `Element::cross_uuid`'s non-zero arm reads it too, so the
  two cannot drift. Its doc line names the market crate as the one it is for. This is P9's only
  core edit (plus its test in `rust/tests/root/implementer.rs`), and it runs the core CI shards.
- D40.5's `Uuid::from_u128` does not exist (`rust/src/uuid.rs:398,565,570`: `Uuid(u128)` private,
  `from_v8` masks six bits, `from_bytes` parses text or sixteen bytes); P7 adds it. Said here so
  P7 does not discover it.

"Stable across processes": XXH3-64 of a text with no seed (`element.rs:516-521`), the text built
from two facts spelled canonically (upper-case ISIN, upper-case class).

## The cross code (to be replaced by the panel's decision)

Recommendation: `<class>:<isin>` - the two class letters, a colon, the twelve-byte ISIN -
`ES:US0378331005`, `IF:QZ8K2M4P1R07`, `XX:CH0012214059` while the class is unknown.

- Not the stored `{kind}:{side}:{base}` prefix (`market.rs:719-744`): an instrument is not a
  market element, is never walked by `EventIterator` and takes no side; `split_crosscode`
  (`:753-770`) reads only a decimal `u8` kind and side, so `ES:...` can never be mistaken for a
  stored prefix, and `base_crosscode` leaves it whole.
- Not the full six-letter CFI: refinement moves it (D42.3).
- The characteristics (strike, expiry, put/call, tenor, ...) are **not spelled in the cross code**:
  they are the input of the custom number's national part (D42.5), so a product with no real ISIN
  is identified through its ISIN exactly as a product with a real one, and nothing is counted
  twice - the cross code is one text of one shape, `class:isin`, for every instrument.
- Every other section depends only on `Instrument::crosscode_of(class, isin)` existing and being
  deterministic; the panel may change the separator, the order or the class spelling without
  touching them. What the panel may not change without reopening D42.5: that the custom number
  carries the characteristics, so the cross code need not.

## D42.4 - the characteristics: a typed value, one canonical spelling

`rust/market/src/instrument/characteristics.rs`:

```rust
pub enum Characteristics {
    None,
    Forex { forex: Forex, tenor: FxTenor, settltype: Option<SmolStr> },
    Future { underlying: Option<Uuid>, expiry: Date32, multiplier: Option<Decimal> },
    Option { underlying: Option<Uuid>, strikepx: Decimal, strikeccy: Option<Ccy>, putcall: PutCall,
             expiry: Date32, exercise: Option<Exercise>, multiplier: Option<Decimal> },
    Bond { maturity: Date32, coupon: Option<Decimal> },
}
pub enum PutCall { Put, Call }            // FIX PutOrCall(201): 0 put, 1 call
pub enum Exercise { European, American, Bermudan }  // FIX ExerciseStyle(1194): 0, 1, 2
```

- Generic and closed: a product kind is a variant with named, typed facts; a new kind is a new
  variant with its own canonical spelling, never a free-form map (AGENTS "Wide in, typed
  through": a bag of texts would be a second schema). `Forex`, `Future`, `Option` and `Bond` are
  the four the instruction names or implies ("future options strikepx or other"); `Bond` is one
  variant and costs nothing more.
- `canonical(&self) -> String`, the one speller, read by the custom number (D42.5) and fed to the
  digest (D42.2): `kind=<word>` first, then the variant's facts as `key=value` pairs in a fixed
  order, `;`-separated, no spaces: `kind=forex;forex=EUR/USD;tenor=forward;settltype=M1`,
  `kind=option;underlying=<32 hex>;strikepx=150;strikeccy=USD;putcall=call;expiry=2026-12-18;exercise=american;multiplier=100`.
  A decimal is spelled by `Decimal`'s canonical `Display` with trailing zeros removed (`150`,
  `0.5`), so `150.00` and `150` are one text; a date is ISO 8601; an absent optional fact is
  omitted; a uuid is its 32 hex digits. `from_str` reads the same grammar and refuses anything
  else naming the key (`$.characteristics.<key>`).
- The flat columns of the row (D42.2 #20-29) are what `canonical` spells, laid out typed for
  querying: `kind: utf8`, `tenor: utf8`, `strikepx: decimal`, `strikeccy: ccy`, `putcall: utf8`,
  `expiry: date32`, `exercise: utf8`, `multiplier: decimal`, `maturity: date32`, `coupon:
  decimal`; the `forex` and `underlying` facts are the row's own columns #12 and #17. A row read
  back rebuilds the value from the columns and refuses a `kind` whose required column is null.
- Where they come from in P9: `Forex` from the FX detection (D42.7); `Option` and `Future` from a
  message's `StrikePrice(202)` (already read: `msg.rs:244,2539`), `PutOrCall(201)` (already read
  into the CFI, `fix/cfi.rs`), `MaturityDate(541)` / `MaturityMonthYear(200)`,
  `ContractMultiplier(231)`, `ExerciseStyle(1194)`, `StrikeCurrency(947)` - the last five are
  read by **no** code today (evidence: `grep` over `rust/fix/src` and `rust/market/src` outside
  `constants.rs`), so P9 adds one reader, `FixMsg::stated_characteristics() ->
  Option<Characteristics>`, beside `stated_underlying_isin` (`msg.rs:3952`), reading each tag
  through its field's type as every FIX value is read (AGENTS Ownership, "every FIX value is read
  by its field's type"). `Bond` from `MaturityDate(541)` and `CouponRate(223)` where the CFI
  category is `D`. The `strikepx` market fact (`Market::get_strikepx`) stays what it is - an
  element fact FIX states per message - and is not re-derived from the instrument.

## D42.5 - the custom ISIN

`Instrument::custom_isin(class: [u8; 2], characteristics: &Characteristics) -> Isin`, the one
minting function, pure:

- **Prefix `QZ`.** It collides with no issued ISIN because every ISIN an agency issues opens with
  an ISO 3166-1 alpha-2 country code or one of the ten agency prefixes (`isin.rs:40-45`: `EU`,
  `EZ`, `XA`, `XB`, `XC`, `XD`, `XF`, `XK`, `XS`, `XT`), and `QZ` is neither: ISO 3166-1 reserves
  `QM`-`QZ` for user assignment (`string.rs:2316-2319` lists the user-assigned ranges `AA`,
  `QM-QZ`, `XA-XZ`, `ZZ` as never assigned), so no country will ever be `QZ`, and ISO 6166's
  agency prefixes live in the `X` range, so no agency will. Not `XX`: that is `Isin::NONE`'s
  prefix, the number that states none (`isin.rs:33-38`). Not `ZZ`: ISO 6166 gives it a meaning of
  its own, "a derivative no agency has numbered yet" (`isin.rs:173-189`), and the crate's docs pin
  it as such. Not any `X?`: a future agency prefix may take it.
- **The national part**: nine upper-case base-36 digits of `xxh3(canonical) mod 36^9`, where
  `canonical` is `"class=<CC>;" + characteristics.canonical()` - the class is part of the input
  because the number must be unique per product and two products of one set of facts under two
  classes (a future and a forward on one underlying and expiry) are two instruments. XXH3-64 with
  no seed through `yggdryl::xxhash::xxh3` (`xxhash/mod.rs:140`), the crate's one hash family
  (AGENTS Digests). 36^9 is about 2^46.5; over a bound of 16,384 instruments the birthday
  collision chance is about 2 x 10^-6, and a collision is a second product folding into the
  first's row, which `Instruments::fold` refuses where the two statements' `kind` differ, naming
  both at `$.isin`.
- **The check digit** through `Isin::closing_digit(&body)` over the eleven-byte body (`isin.rs:229`,
  public), so the number closes; `Isin::is_closed` answers true and `Isin::rank_of` answers **1**
  (closed, unlisted prefix: `isin.rs:215-220`).
- **Rank**: below every real ISIN (2) and above a masked one (0). `Isin::merge_with` takes the
  higher rank whichever leads (`value/mod.rs:247-253`), so a real ISIN stated for the same product
  replaces the custom one on merge. A typo under a listed prefix is also rank 1
  (`US0378331006`); rank ties keep the leading value, and a typo is refused as a key anyway
  (`Instrument::new`): `Instrument::is_custom(isin)` is `prefix == "QZ" && is_closed`, and the
  key gate is `is_real || is_custom`.
- **Two processes mint the same code**: the input is the canonical text (D42.4), the hash is
  seedless, the base-36 rendering and the Luhn digit are pure. Pinned in
  `rust/market/tests/root/instrument.rs` by spelling `EUR/USD` spot's number as a literal, so a
  drift in the canonical grammar is a red test.
- **Re-keying when a real ISIN arrives.** An instrument minted under a custom number is keyed by
  it in `Instruments`' rows. A later statement naming the same product - the same characteristics
  (the FX pair and tenor) - under a real ISIN is folded by `Instruments::fold`: the custom row's
  `isin` moves to the real one (rank 2 over 1), its `securityids` keep the custom number under
  `derived:isin`? **No**: nothing keeps the custom number, there is no alias (AGENTS "No
  back-compat"); the row is re-keyed, re-indexed (`Indexes::moved`, `isin_registry.rs:1258-1479`
  today) and re-finalized, so its `crosscode`, `crosshashcode`, `crossuuid` and `uuid` all move
  once. **What happens to `instrumentuuid` then, plainly**: market rows written before the real
  ISIN was known carry the custom identity; rows written after carry the real one; the two are not
  joined by the instruments table, which holds the real one alone. A rerun of the medallion window
  (every silver stage overwrites the partitions its rows reach) lands the settled identity on
  every row of the window, because the lifecycle fills from the instruments the run has learned.
  This is the same move the class makes (D42.3), stated once in `docs/graph/instrument.md`
  "Identity moves".

## D42.6 - country and currency fill

The rules are the registry's today, carried over and extended to the custom case; each is one
function of the `Instrument`, called by `Instruments::fold` (`derive_defaults`,
`isin_registry.rs:1076-1135`) after every row move:

- **Country**: stated `countrycode` where listed; else the ISIN's prefix where `StringEnum::COUNTRIES`
  lists it (`IsinEntry::country`, `isin_registry.rs:483-492`) - so an agency-prefixed or custom
  number derives none; a stated country equal to the prefix is taken back (today's rule). An FX
  pair states no country (`Forex` has no country; `country_of_issue` needs a listed prefix,
  `native_derivations.rs:652-660`): `None`, never one leg's.
- **Currency** (the listing's trading currency): stated; else the legal tender of the listing's
  market's country (`Mic::country`, `Country::currency`: `mic.rs:165`, `country.rs:72`), a
  statement replacing it, never learned back; **for an FX pair** the currency is the **quote** leg
  (`Forex::quote`), because a pair is priced in its quote currency (EUR/USD is dollars per euro),
  and `origccy` is `None` - today's registry deliberately stores neither for a pair
  (`isin_registry.rs:2951-2972`, "Currency(15) is the dealt currency"); `fill_market`'s unit rule
  (the dealt currency as `unit`, `market.rs:529-546`) is untouched. Put to the user (#5): quote leg
  vs base leg.
- **Origin currency**: stated or seeded only, never derived (the registry's rule, "The origin
  currency is an instrument fact stated and never derived").
- The embedded national numbers (CUSIP for `US`/`CA`, SEDOL for `GB`/`IE`/..., WKN, Valor) keep
  deriving into empty `securityids` entries through `securityid::embedded` (`securityid.rs:34-52`),
  which answers none for a custom prefix.

## D42.7 - forex auto-creation

Today an FX pair is detected at the parse (`FixMsg::derive_forex`, `rust/fix/src/forex.rs:120-150`:
`Symbol(55)` through `yggdryl::FxSymbol::from_symbol`, `forex.rs:226-300`) into a derived `forex`
identifier, `SecurityType(167)`, `Product(460)`, `CFICode(461)` (`IFXXXP` spot, `JFTXFP`
forward, `ITKXXX` metal), `Currency(15)`/`SettlCurrency(120)` and `SettlType(63)`; the registry
never learns it (`learn_stating` returns at once without a real ISIN, `isin_registry.rs:2941`), and
its book is keyed `XX0000000000` with every other ISIN-less element (`book_crosscode`,
`market.rs:476-480`).

Decided:

1. **At the parse**, in `derive_forex`, once the pair, the tenor and the CFI are settled, the
   message derives its **custom ISIN** into `securityids` under `derived:isin` through
   `Market::derive_securityid` - `Instrument::custom_isin(class, &Characteristics::Forex { forex,
   tenor, settltype })`, the class read off the CFI it just wrote (`IF` spot or unstated tenor,
   `JF` forward, `IT` metal). This is local per-event enrichment from the event alone (AGENTS
   Ownership: a parse depends only on the event and the reference data its door fixed), pure,
   needs no table, and every parse door derives the same number for the same pair. A message
   stating a real ISIN beside the pair keeps the real one (rank 2 over the derived 1; the derived
   lands only where nothing of its type is held or the base ranks below it, `identifier.rs`).
2. **In the lifecycle**, `Instruments::learn_stating` is keyed by a stated or derived ISIN that
   is real or custom (today: stated real alone). For a custom key the statement's
   `characteristics` is `Characteristics::Forex {..}` from the message's `forex` identifier and
   `SettlType(63)` (`FixMsg::stated_characteristics`), its class the CFI's, its `currency` the
   quote leg (D42.6), `countrycode` none, `miccode` and `ticker` the message's. So the first FX
   message of a pair creates the instrument; later ones restate it; a listing per market as for
   every instrument.
3. **Consequences**: an FX pair's book is keyed by its custom ISIN (`book_crosscode` is unchanged
   and now finds an ISIN), so EUR/USD spot and EUR/USD 1M are two books and `XX0000000000` is
   left to elements stating neither an ISIN, a ticker nor a pair; `Market::instrumentuuid()`
   answers for every FX element; `rust/market/tests/graph/book.rs` gains the pair-keyed book pin
   and `rust/fix/tests/root/enrich.rs` the "a walk creates the pair's instrument" pin.
4. Not in P9: an FX option or future CFI (none exists in the crate, `cfi.rs:224-258,313-315`
   admit the letters but nothing writes them); a SecurityDefinition (`d`) read as an instrument
   definition (`MarketDataKind::Securities` exists, `marketdatakind.rs:79`, but no cross-tag rule
   and no `batch_groups` arm, `fix/market.rs:953-1000`) - said in "Put to the user".

## D42.8 - the market column `instrumentuuid`

- `Market::instrumentuuid(&self) -> Option<Uuid>`, a **provided reading** on the `Market` trait
  (`market.rs:147-375`), no holder, no setter: `self.get_isincode()` (any rank but masked:
  `Isin::rank_of >= 1`, which admits the custom number and a masked `XX` number is `None`) and
  `Instrument::class_of(self.get_cficode())`, spelled by `Instrument::crosscode_of` into a stack
  buffer (`[u8; 16]`: two letters, a colon, twelve bytes, allocation-free) and hashed through
  `yggdryl::implementer::cross_uuid_of`. `None` where the element names no ISIN.
- It is derived, so it is **never fed** to `digest_market` and no `uuid`, `hashcode` or
  `crossuuid` of any market element moves in P9. The market `MarketData` size pin (912 bytes,
  `rust/market/tests/graph/market_data.rs:120-127`) does not move: no holder is added.
- **Where it is filled**: nowhere - it is read. What fills the facts it reads is the existing
  two-tier rule: the parse door's snapshot (`InstrumentTable::fill_identifiers`, derived security
  identifiers only, the custom ISIN included by D42.7 step 1 even without a table) and the
  lifecycle's `learn_and_fill` (`fill_unsettled`: identifiers, ticker, CFI, currency). So a bronze
  FIX row (raw parse) carries `instrumentuuid` wherever the message states or derives an ISIN,
  and a silver row wherever the lifecycle filled one more.
- **The `marketdata` row** (`rust/market/src/graph/arrow.rs`): `MarketColumn::InstrumentUuid`,
  datatype `uuid` nullable, placed **after `securityids` and before `isincode`** (`market_column.rs:
  ALL`, 36 -> 37; `6 + 9 + 37 + 5 + 3 + 6 = 66`, the doc assert at `arrow.rs:140` re-pinned with
  the sentence "`instrumentuuid` joined the market band, D42"). The position is chosen so that P7
  lifts `instrumentuuid`, `isincode`, `cficode`, `miccode` as one contiguous band into its
  lifted band in that order (D40.2's order with `instrumentuuid` where `instuuid` was), and the
  row stays 66 after P7.
- **The FIX row**: the crate tag **65_054 `instrumentuuid`** (`uuid`, nullable), the next unused
  (`crated.rs` ends at 65_053 `fixmsg`), placed in the instrument band of `fix_schema_tags`
  (`schema.rs:245-262`) **first**, before `55` and `FOREXCODE`; `fix_schema_tags`' count moves 152
  -> 153 (`rust/fix/tests/root/schema.rs:95`); `crated.rs`'s "Fifty-two definitions" becomes
  fifty-three; `CRATED` gains the row. The column is written from the reading at every row write
  and, on read-back, re-derived from the row's `isincode`/`cficode` (a stored value that disagrees
  is a defect named by row, as every derived column is checked).
- `Market::stored_crosscode` and `book_crosscode` are unchanged. P7's planned `Market::instuuid()`
  (D40.3, XXH3-128 of the real ISIN alone) is **dropped**: `instrumentuuid` is the instrument's
  cross uuid over class and ISIN, custom numbers included, and P7 changes only the derivation's
  width through `cross_uuid_of` ("What P7 changes").

## D42.9 - the store, the environment, the seed

Carried over from `isin_registry/{store,env,seed}.rs` with the contracts the registry states today
(`store.rs:14-138,147-435`; `env.rs:19-205`; `seed.rs:1-90`), re-spelled:

- `Instruments::from_holder`, `from_url`, `seeded_from_holder`, `seeded_from_url`, `set_holder`,
  `try_with_holder`, `holder()`, `commit() -> Result<IOResult>`; a leaf (Arrow IPC default), a
  plain folder (one part of the store's encoding) or an Iceberg table whole (`Located::
  overwrite_whole`, a partition refused at `$.holder`); `commit` writes nothing when clean and
  refuses unbound as `Error::absent("instruments holder")`; `read_lacking`/`warn_lacking` keep
  warning once per missing column ("instruments column not stored") and migrate nothing. A store
  written under the `isinregistry` row has no `uuid` column: it is refused at load naming the
  missing required column, which is the no-back-compat reading; the medallion's table is dropped
  and recreated by the test's fresh warehouses, and a user's `~/.config/yggdryl/isin/` is not read
  (the path moved).
- `env.rs`: `YGGDRYL_INSTRUMENTS_URI`, else `~/.config/yggdryl/instruments/`, else seeded and
  unbound; `from_env`, `install_env`, `install_env_shared`, the pure `autoload` under `internals`.
- `seed.rs`: `config/instruments/instruments.json` embedded as the byte-equal
  `instrument/seed.json`, the eight seed keys unchanged (`isin`, `ticker`, `miccode`, `currency`,
  `countrycode`, `cficode`, `fisn`, `origccy`; P7 renames four), parsed once per process into an
  `InstrumentTable`; `scripts/check_instruments_seed.py [--sync]` with `COPY` re-pointed; the
  `rows.toml:47` inert path and AGENTS "What CI never runs" re-spelled.
- `Instruments::DEFAULT_MAX_INSTRUMENTS` 16,384, `ENTRY_CHARGE` 8 KiB (`ENTRY_COST <= ENTRY_CHARGE`
  re-asserted: the struct gains `underlying` 17 bytes, `legs` 24, `characteristics` under 80 and
  loses the `SmallVec` of codes for `Identifiers`; the `const` assert is the pin), `LOOKUP_CODES`
  twenty, `DEFAULT_ECONOMIC_THRESHOLD` 0.85, `MAX_TICKER_WIDTH` 64.
- `internals`: `instrument::internals` (`ENTRY_CHARGE`) and `instrument::env::internals`
  (`autoload`), `scripts/generate_internals.py` regenerating `lib.rs`'s block.

## D42.10 - the medallion dump

`python/tests/medallion.py`:

- `instruments(silver, namespace)` (`:241-259`) opens or creates `silver.record_keeping.instruments`
  from `Instruments.field().into_scheme_compat("iceberg")` (now the 32-column `instrument` row,
  partitioned `truncate(isin, 2)`) and answers `Instruments.seeded_from_url(table)`.
- `commit_instruments(instruments)` (`:262-269`) and `commit_instruments_stage` (`:315-324`) read
  `lake.codec.instruments`; the stage key stays `silver.instruments` between `silver.fix_messages`
  and `silver.books` (`STAGES_OF`, `:369-376`).
- **Finding, fixed in P9**: `TABLES` (`:379-387`) omits `silver.instruments`, so `report()` never
  prints it; and `main()` (`:534-540`) builds the codec with no registry, so the CLI run's stage
  answers `{}` - which the live AWS prompt's step 3d (`LIVE_AWS_TEST_PROMPT.md:50-55`, "the
  instruments table and every market table's `instrumentuuid` column must be present and filled")
  would fail on. P9: `main()` builds `instruments = medallion.instruments(catalog_of(args.silver,
  "silver"), args.namespace)` and `FixCodec(..., instruments=instruments)`; `TABLES` lists
  `silver.instruments` after `silver.fix_messages`; `Lake.tables`/`report()` print its row.
- The `instrumentuuid` column reaches every market table through the Rust schemas with no
  pipeline edit: `bronze.fix_messages` and `silver.fix_messages` (the FIX row's 65_054),
  `silver.books`, `silver.orders`, `silver.quotes`, `silver.executions` (the `marketdata` row's
  37th market column; nested `delta`/`events`/`alive` rows carry it too). `PRIMARY_KEY`, `REQUIRED`
  and `PARTUNIX` (`:83-97`) are unchanged: `instrumentuuid` is a derived fact, not a key.
- `python/tests/test_fix.py::test_the_medallion_pipeline_lands_every_stage_over_two_catalogs`
  (`:5625-5830`) re-spelled: `Instruments`, `len(instruments) == len(seed)`, the stage lists, the
  table spec `[('isin_truncate', 'truncate[2]')]`, `field.index_of('underlying') ==
  index_of('legs') - 1`, reload equal; plus the new assertions: every market table's
  `instrumentuuid` is non-null on every row whose `isincode` is non-null, the FX pair's rows share
  one `instrumentuuid` with the `QZ` row of the instruments table, and `silver.instruments` is in
  the report.

## D42.11 - the bindings

Python (§3, the full view) - every door the registry has is re-spelled onto `Instruments`, nothing
else added: `python/src/instrument.rs` (was `isin_registry.rs`): `#[pyclass(name = "Instruments",
frozen)]` with `__new__(max_instruments=16384)`, `field()`, `seeded()`, `from_url`,
`seeded_from_url`, `from_env`, `install_env`, `from_arrow_reader`, `extend_from_handle`,
`extend_from_arrow_reader`, `into_arrow_reader`, `commit`, `is_dirty`, `get`, `listings`,
`get_listing`, `get_by_ticker`, `get_by_code`, `resolve`, `LOOKUP_CODES`,
`DEFAULT_ECONOMIC_THRESHOLD`, the economic pair, `merge(dict)`, `remove`, `remove_listing`,
`clear`, `rows`, `max_instruments`, `learn`/`fill`/`enrich`, `__len__`, `__bool__`, `__eq__`,
`__repr__`; rows cross as `dict` through `as_py(into_scalar())` with the new keys (`uuid`,
`crossuuid`, ..., `underlying`, `legs`, `kind`, `strikepx`, ...); `Resolution` unchanged.
`FixCodec(instruments=...)`, `FixCodec.from_env()` attaching `Instruments.from_env()`,
`codec.instruments`. `FixMsg.instrumentuuid` property and `MarketData` rows' column through the
Arrow row. `python/yggdryl/instrument.py` re-exports; `__init__.py`, `__init__.pyi`, `_native.pyi`
re-spelled; `python/tests/test_instrument.py` (was `test_isin_registry.py`, 37 tests re-spelled plus
the custom-ISIN, forex-creation and `instrumentuuid` parity tests), `conftest.py` sets
`YGGDRYL_INSTRUMENTS_URI`, `typing_bindings.py`, `python/benchmarks/graph.py`.

Node (§4): the request does not name Node, so **no door is added**. The existing `IsinRegistry`
door (`node/src/isin_registry.rs`, 712 lines; `node/src/fix.rs:2551-2688`; `node/tests/
isin_registry.test.js`, 20 tests; `node/benchmarks/graph.js`) is **re-spelled** onto `Instruments`
and the `instruments` codec option and getter, because the no-back-compat rule deletes the name it
binds; its tests re-spelled, no new door for `instrumentuuid` beyond the column every Arrow row
already crosses, the loader and declarations regenerated (`npm run --prefix node build:debug`), the
two docs manifests regenerated (`node scripts/build_docs_fix.js`, `build_docs_playground.js`, the
dictionary moved). The handoff **proposes retiring** the Node `Instruments` door - 712 lines of
binding for a registry the essential view has no page for - and retires nothing unasked ("Put to
the user" #8).

## D42.12 - docs, skills, inventories

- `docs/graph/instrument.md` replaces `docs/graph/isin-registry.md` (1,570 lines, 51 tabbed
  examples; nav `mkdocs.yml:257`): Contract (the element, the row, the identity rule, the cross
  code), Use, Seed, The custom number, Forex, Characteristics, Derived facts, The update rule,
  Listings, Matching (the four tiers, as today), Identity moves (D42.3/D42.5 plainly), The
  underlying and the legs, The product category, Persistence, The process instruments, Bounds,
  Edges, Performance (the table **not** edited by hand: regenerated by the release run, or the
  section states the bench name and no numbers until then), Commands. Rust and Python tabs;
  JavaScript tabs only for the doors Node carries (every existing one, re-spelled).
- Pages naming the registry, each re-spelled: `docs/fix/lifecycle.md:22,655-735` (`with_instruments`,
  `FixCodec(registry, instruments=...)`, `new Instruments()`), `docs/fix/capture.md:661` (the
  crate's columns table gains 65_054), `docs/fix/message.md:828`, `docs/types/codes/{fisn:206-282,
  cfi:257,287, lei:229, dti:226, mic:276, country:276}.md`, `docs/graph/{market:171,203,297,
  market-data:180, identifier:15,313,521,800, index:17, schemas}.md` (the `marketdata` row 66
  columns, the FIX row's band), `docs/architecture.md:30,52`, `docs/contributing.md:77`,
  `docs/benchmarks.md:17`, `docs/testing.md:103,110` (`--test instrument`),
  `docs/media/iceberg.md:1443`.
- Skills: `skills/yggdryl-fix/SKILL.md:90-91,442-463` and `references/{rust:518-558,
  python:453-496, javascript:447-481}.md`, `skills/yggdryl-market-data/SKILL.md:32,181-193,387`,
  `skills/yggdryl-types/references/{rust:424-447, python:378-406, javascript:573}.md` - every
  `IsinRegistry` recipe re-spelled, one recipe added for the custom number and one for reading
  `instrumentuuid` off a row.
- Inventories by hand: `.api-inventory.txt` sections `yggdryl_market::isin_registry` (7357-7470)
  and the implementer lines (7677, 7691-7692), the fix codec lines (845, 869-870) ->
  `yggdryl_market::instrument`, `instrument::characteristics`, `instrument::store`,
  `instrument::env`, the core `implementer::cross_uuid_of`, `Market::instrumentuuid`,
  `MarketColumn::InstrumentUuid`, the crate tag; `.api-bindings.txt` lines 103-104, 221-222 (Python)
  and 625-628, 798-799 (Node). `python scripts/check_api_inventory.py` proves it.
- AGENTS.md: the `yggdryl-market` paragraph (`:355-356`), the Layout rows (`:480` the graph row's
  `IsinEntry` mention, `:484` the `isin_registry.rs` row -> an `instrument.rs` row, `:485` the
  eusipa row), Ownership (`:613-625`, "the instrument table a shared `IsinRegistry` held" -> `Instruments`),
  "What CI never runs" (`:2471`), the market crate description in `rust/market/Cargo.toml:3` and
  `rust/market/README.md:3`, `README.md:21` ("the ISIN registry" -> "the instruments").

## D42.13 - tests, cost pins, the pins that move

Tests at the mirrored paths (AGENTS "Where a test lives"):

| Deleted | Written |
| --- | --- |
| `rust/market/tests/root/isin_registry.rs` (44 tests), `tests/isin_registry.rs` (the isolated runner), `tests/isin_registry/{env,seed,store}.rs` (3 + 8 + 22), `root.rs:37-38` | `rust/market/tests/root/instrument.rs` (the 44 re-spelled on `Instruments`/`Instrument`, plus: the element pins - `uuid` stable across two builds of one statement, `crossuuid == cross_uuid_of(crosscode)`, stamps and sources not fed; the custom number - `EUR/USD` spot's literal, `rank_of == 1`, a real ISIN re-keys and moves the identity once; the class moves the identity once; `underlying`/`legs` round trip; `Characteristics` canonical/from_str round trip for each variant and the refusal by key), `tests/instrument.rs` (the isolated runner, child name = path), `tests/instrument/{env,seed,store,characteristics}.rs` |
| `rust/fix/tests/isin_registry.rs` + `isin_registry/env.rs` | `rust/fix/tests/instrument.rs` + `instrument/env.rs` (`FixCodec::from_env` shares the process instruments) |
| - | `rust/fix/tests/root/{codec,enrich,batch}.rs` re-spelled (`learned_registry` -> `learned_instruments`, the four-threads table test, the underlying/eusipa/economic/country walks) plus: a walk creates the pair's instrument with its custom number and the message's `instrumentuuid` equals it; `fix_schema_tags` 153; the 65_054 column on the fixed row; the store test's dump and hash |
| - | `rust/market/tests/graph/{market_column,arrow,book,market}.rs`: 37 columns, 66-column row, the pair-keyed book, `instrumentuuid()` over isin+cfi and `None` without an ISIN |
| - | `rust/tests/root/implementer.rs`: `cross_uuid_of(text) == element.sync_cross()`'s `crossuuid` on a `TextLine` |
| - | `cli/tests/docs_examples.rs` through the docs runner; `cli/tests/market_register.rs` unmoved |

Cost pins - which move and why they may not rise:

| Pin | Moves? | Rule |
| --- | --- | --- |
| `rust/market/tests/allocations.rs:1752-2200`, the eight registry rows (learn known, fill by ticker, ticker index doubling, learn new inline, resolve without allocating, economic scan, snapshot stream, reload per batch) | re-spelled onto `Instruments`, claims unchanged | a cost pin is never re-pinned upward; the `Identifiers` map replaces a `SmallVec` of twelve pairs - `learn new inline` claims the row's allocations are the map's and the strings', the same count as today or fewer |
| new allocations rows | `a_market_element_reads_its_instrumentuuid_without_allocating` (the stack buffer), `a_custom_isin_is_minted_without_allocating_past_its_twelve_bytes` (one `SmolStr`, inline), `an_instrument_finalizes_without_allocating` | stated before written, red first |
| `rust/market/tests/iobase_calls.rs:35-90` `mod isin_registry` -> `mod instrument` | the same: a load is the calls of one record read | unchanged |
| `rust/market/tests/instrument/store.rs` `filesystem.costs` pins (a clean commit `none`, seeded = unseeded load, a first run's clean commit `none`) | unchanged | |
| `rust/fix/tests/allocations.rs`, `iobase_calls.rs` | do not name the registry; unmoved | a moved count is a defect |
| `MarketData` 912 bytes | unmoved (no holder added) | a rise is a defect |
| the fix bench `fix/pipeline.rs:294,1155-1167`, the market bench `graph/isin_registry.rs` -> `graph/instrument.rs` | re-spelled; direction only by `--quick` | |

Pins that move once, each with its sentence:

- the crate dump under `config/fix/` (`YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl-fix
  --test root the_committed_store_carries_the_crate_dump`): the 65_054 definition;
- the dictionary hash in `rust/fix/tests/root/store.rs:3547` (`12_613_356_107_921_639_431` today):
  the `left` value the test reports, with the newest "It last moved when" sentence: "It last moved
  when the crate's own block gained `instrumentuuid` (65_054): the instrument's cross identity over
  its class and its ISIN, derived and nullable, hashes beside the fifty-two and as a member of the
  fixed row (D42)"; the census one definition more;
- `fix_schema_tags` 152 -> 153; `arrow.rs:140` 36 -> 37 and the `marketdata` row 65 -> 66;
  `MarketColumn::ALL` 36 -> 37 with `rust/market/tests/graph/market_column.rs`;
- the equivalence snapshot `rust/fix/tests/root/equivalence.snapshot`: every FIX row gains the key
  `instrumentuuid` (a value where the message holds an ISIN, null otherwise); **no** `uuid`,
  `hashcode`, `crossuuid` or `crosshashcode` line moves (the reading is derived and fed nowhere) -
  a moved one is a defect;
- `docs/assets/fix.json` and `playground.json` (the dictionary and the addon moved);
- `.api-inventory.txt`, `.api-bindings.txt` by hand.

Pins that must not move: every `DataTypeId` byte, rank, `Shape` position, value-stream byte and
Arrow extension name (S0); every market element's `uuid`/`hashcode`/`crossuuid` and every book
identity (the FIX `uuid`/`hashcode` cells of the snapshot); the S0/S2 pins; every cost pin above;
the FX detection's wire pins (`forex.rs`), since D42.7 adds a derived identifier beside what it
writes and changes no cell it owns - `rust/fix/tests/root/forex.rs` gains the `derived:isin` line
and loses none.

## Implementation plan

One commit, one push. Phases in AGENTS order; each phase a partition of files by path, so workers
never share a file; a phase is settled by its build check and the suite it touched; the whole run
leads the chain when the last phase holding the cargo lock is settled. `cargo check -p
yggdryl-market --all-targets --keep-going --message-format=short` after the market core is the
compiler's list of every caller the sweep re-spells.

| Phase | Files (disjoint) | Smoke (exact) | Pins expected to move | Pins that must not |
| --- | --- | --- | --- | --- |
| 0 - the sweep script | `$S/p9/p9_sweep.py` (the P4 sweep is the model: `git show 7b566b3b2:.handoff/split/scratch/p4_sweep.py`): exact-string edits `IsinRegistry`->`Instruments`, `IsinEntry`->`Instrument`, `IsinTable`->`InstrumentTable`, `isin_registry`->`instrument(s)` per site, `with_isin_registry`->`with_instruments`, `isinRegistry`->`instruments`, `YGGDRYL_ISIN_REGISTRY_URI`->`YGGDRYL_INSTRUMENTS_URI`, each anchor asserted to match once; `git mv` of the module, test, seed, docs and binding files | `python3 -I $S/p9/p9_sweep.py --check` (counts every anchor) | - | - |
| 1 - market core | `rust/market/src/instrument.rs`, `instrument/{characteristics,store,env,seed}.rs`, `instrument/seed.json`, `lib.rs`, `implementer.rs`, `graph/market.rs` (`instrumentuuid()`), `graph/market_column.rs`, `graph/arrow.rs`, `graph/view.rs` where it lists columns; delete `isin_registry.rs` + folder; `config/instruments/instruments.json`, `scripts/check_instruments_seed.py` | `cargo check -p yggdryl-market --all-targets`; `cargo test -p yggdryl-market --test root instrument`; `cargo test -p yggdryl-market --test instrument`; `cargo test -p yggdryl-market --test graph market_column`; `--test graph arrow`; `--features internals --test instrument`; `python scripts/generate_internals.py --check` | `MarketColumn::ALL` 37, the row 66 | `MarketData` 912; `allocations` claims |
| 2 - the core forwarder | `rust/src/implementer.rs` (`cross_uuid_of`), `rust/src/graph/element.rs` (`cross_uuid`'s non-zero arm reads it), `rust/tests/root/implementer.rs` | `cargo check -p yggdryl --all-targets`; `cargo test -p yggdryl --test root implementer`; `cargo test -p yggdryl --test graph element` | none (the body is today's) | every core `crossuuid` pin |
| 3 - FIX crate | `rust/fix/src/{codec,enrich,msg,market,messages,forex,crated,schema,identity,build}.rs` (65_054, `stated_characteristics`, the custom-ISIN derivation in `derive_forex`), `rust/fix/src/lib.rs` where it names the registry | `cargo check -p yggdryl-fix --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-fix --test root codec`; `--test root enrich`; `--test root forex`; `--test root schema`; `--test root batch`; `--test instrument`; then `YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl-fix --test root the_committed_store_carries_the_crate_dump` and `cargo test --locked -p yggdryl-fix --test root the_committed_dictionary_hashes_to_one_pinned_value` once, the hash re-pinned with its sentence | the dump, the hash, the census, `fix_schema_tags` 153, the snapshot's `instrumentuuid` keys | every `uuid`/`hashcode`/`crossuuid` cell of the snapshot; `fix` allocations and iobase rows |
| 4 - tests | `rust/market/tests/{root/instrument.rs, instrument.rs, instrument/*, root.rs, allocations.rs, iobase_calls.rs, graph/{book,market,market_column,arrow}.rs, root/implementer.rs}`, `rust/fix/tests/{instrument.rs, instrument/env.rs, root/{codec,enrich,batch,forex,schema,store}.rs}`, the benches `rust/market/benchmarks/graph/{instrument,mod}.rs`, `graph.rs`, `rust/fix/benchmarks/fix/pipeline.rs` | the rows of phases 1-3 plus `cargo test -p yggdryl-market --test allocations instrument`; `--test iobase_calls instrument`; `cargo bench -p yggdryl-market --bench graph -- instrument --quick` | the allocations rows re-spelled, claims equal | no cost pin rises |
| 5 - Python | `python/src/{instrument,fix,lib}.rs`, `python/yggdryl/{instrument.py,__init__.py,__init__.pyi,_native.pyi}`, `python/tests/{test_instrument.py,test_fix.py,conftest.py,typing_bindings.py,medallion.py}`, `python/benchmarks/graph.py` | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_instrument.py -x -q`; `... -m pytest python/tests/test_fix.py -k medallion -x -q` (the user's named validation); `mypy --strict` line of §3 | - | - |
| 6 - Node re-spelled | `node/src/{instrument,fix,lib}.rs`, `node/{binding.js,binding.d.ts}`, `node/tests/{instrument.test.js,instrument.types.ts,fix.test.js}`, `node/benchmarks/graph.js`; then `node/index.js`, `node/index.d.ts` regenerated | `npm run --prefix node build:debug`; `node --test node/tests/instrument.test.js`; `node --test node/tests/fix.test.js`; `npm run --prefix node test:package:debug` | the generated loader and declarations | no new door |
| 7 - docs manifests | `docs/assets/{fix,playground}.json` | `node scripts/build_docs_fix.js && node scripts/build_docs_playground.js`, then `--check` | both files | - |
| 8 - docs and skills | `docs/graph/instrument.md` (new), delete `docs/graph/isin-registry.md`, `mkdocs.yml`, the pages listed in D42.12, `skills/**` listed | `python -m mkdocs build --strict --config-file mkdocs.yml`; the three `python scripts/check_docs_examples.py --lang {rust,python,javascript}` as chain steps | - | - |
| 9 - inventories, contract | `.api-inventory.txt`, `.api-bindings.txt`, `AGENTS.md`, `.github/ci/rows.toml:47`, `rust/market/{Cargo.toml,README.md}`, `README.md` | `python scripts/check_api_inventory.py`; `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py`; `python3 scripts/release_packages.py version` (the manifest description changed, the version did not) | - | - |
| 10 - the chain | `logs/chain.sh` adapted: the three crates' whole runs `--all-features --no-fail-fast`, clippy both lanes, `cargo doc -D warnings`, the rustdoc examples, the CLI tests, `pytest python/tests`, Node, the manifests, mkdocs, the inventories, the docs runners | one background script, one log, read once | the pins of phase 3 only | everything else |

Then `cargo fmt --all` once after the last worker, the commit with the message file (the two
trailer lines `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and `Claude-Session:
https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp`, the program's rule), one push, the CI
run read to `CI result`. The change touches `rust/src/implementer.rs`, so every CI job runs.

## What P7 changes

Written here so P7's design (DESIGN.md "## P7: design", `$S/p7/d40_design.md`) is amended before it
is implemented, not after:

1. **`instuuid` is dropped from D40.2 and D40.3.** P9 lands `instrumentuuid` (the user's latest
   spelling) as the market band's column after `securityids` and the crate tag 65_054. D40.3's
   derivation - `Uuid::from_u128(xxh3_128(isin))` of the real ISIN alone - is superseded: the
   instrument's identity is the cross uuid over its class and its ISIN, custom numbers included
   (D42.3), and `Market::instrumentuuid()` is the reading. D40.2's lifted band becomes
   `instrumentuuid, isin, cfi, mic` (then `bbg`, `figi`, `forex` on the FIX row), the four lifted
   together from the market band where P9 left them contiguous; the `marketdata` row stays 66
   (`6 + 9 + 33 + 5 + 3 + 4 + 6`). 65_054 keeps its number and its name; no retired number.
2. **D40.4's renames apply to P9's columns uniformly**: the `instrument` row's `cficode` -> `cfi`,
   `countrycode` -> `country`, `forexcode` -> `forex`, `eusipacode` -> `eusipa`, `miccode` ->
   `mic`, `SORT:by ["isin", "mic"]`, the seed's keys, the characteristics columns unchanged
   (`strikepx` is a graph word, not a registered code); `Instrument`'s readers `cficode()` ->
   `cfi()`, `forexcode()` -> `forex()`, `miccode()` -> `mic()`; `Market::get_isincode` ->
   `get_isin`, `get_cficode` -> `get_cfi` (D40.3) and `instrumentuuid()` reads them.
3. **D40.3's one hold map** moves the market element's `cficode` into `securityids` under
   `IdType::Cfi`; `Instrument` already holds its CFI there (D42.2), so P7 changes nothing in it.
4. **D40.5 moves the cross-uuid rule uniformly** by editing one body, `yggdryl::implementer::
   cross_uuid_of` (and `Element::cross_uuid`, which reads it): every `crossuuid` in the tree moves
   once - every instrument's, every market element's and every `instrumentuuid` cell - with the
   one sentence. P7 must add `Uuid::from_u128` (D42.3).
5. D40.6 (the book's sources and content code) is untouched by P9.

## Put to the user (interpretations taken; say if another was meant)

1. **The class, not the detailed CFI, in the identity**: `ES:US0378331005`, never `ESVUFR:...`, so
   a refinement moves nothing; an unknown class spells `XX` and the identity moves once when the
   class is learned (D42.3).
2. **One element per listing** (per ISIN and market), every listing of one instrument sharing the
   cross identity; the dump stays one row per listing (D42.2).
3. **`QZ`** as the custom prefix, rank 1, the national part the hash of the class and the
   characteristics; a real ISIN later stated re-keys the instrument and its identity moves once,
   rows written before keep the custom identity until the window is rerun (D42.5).
4. **The characteristics are not spelled in the cross code**; they identify a product through its
   custom number. The cross code's exact grammar is the panel's.
5. **An FX pair's currency is its quote leg**; its country is none (D42.6).
6. **`legs` is a list of uuids alone**, in stated order, no ratio and no side held (FIX's
   `LegRatioQty(623)` and `LegSide(624)` are read by nothing today); `underlying` is one uuid.
   Both are learned in P9 only where a message states them through the existing underlying readers
   (`UnderlyingSecurityID(309)`, the related `Underlier`) resolved to the underlying's identity;
   `NoLegs(555)` is not read in P9.
7. **`Characteristics` is a closed enum of four product kinds** (`Forex`, `Future`, `Option`,
   `Bond`); a new kind is a new variant, not a free-form map.
8. **Node keeps its registry door re-spelled** (`Instruments`, the `instruments` codec option);
   the handoff proposes retiring it and retires nothing unasked.
9. **`instrumentuuid` is a derived reading, fed to no digest**: no market `uuid`, `hashcode` or
   `crossuuid` moves in P9; only the dictionary hash, the dump, the snapshot's new keys and the
   column counts move.
10. **The environment variable and the default folder are renamed** (`YGGDRYL_INSTRUMENTS_URI`,
    `~/.config/yggdryl/instruments/`); a store written under the old row is not read.
11. **The CLI `medallion.py main()` binds the instruments** to the codec so the live AWS run's
    step 3d can see the table and the column.
12. **Not in P9**: SecurityDefinition/SecurityList messages read as instrument definitions, FX
    option or future CFIs, and the P8 identifier index (the cascade's four tiers stay as they are).
