# P9: the instrument cross code (D42.x)

The decision over the four proposals (`stable` 24, `readable` 23, `packed` 19, `fixed` 15) and
the three lenses: `stable`'s grammar and identity rule, with the judges' grafts - `readable`'s
per-row lookup, unchanged book key, placeholder flag and minted-number projection; `packed`'s
packed ISIN index and foreign-mint check; `fixed`'s `QY` prefix, `yggdryl:isin` source and
`FxMemo` memoization - and every fatal finding resolved: no digest is a key, no `XX:` prefix on a
real ISIN, no venue-order legs, no cascading leg or underlying uuid in a key, no second identity
that `Element::merge_with` refuses (`element.rs:352` requires equal uuids).

The one deliberate reading of the user's words. "Instrument cross uuid should rely on cficode +
isincode" is honoured per class, not as one literal spelling: an instrument an agency has numbered
is keyed by that number alone, and the CFI letters key everything no agency numbers - FX, a
derivative, a strategy - with the characteristics written beside them ("synthetically in cross
code"). A CFI letter inside a real-ISIN key is the one thing criterion 1 forbids: the first row of
most instruments states no `CFICode(461)`, so `XX:US0378331005` would re-key to `ES:US0378331005`
on the first classified row, and every D40.3 `instuuid` would move. Question 1 below says the exact
cost of the literal reading, should the user want it anyway.

## D42.1 - the grammar

ASCII, upper case, no byte below `0x20`, `:` between fields, at most 128 bytes (refused by name
past it: a strategy of more legs than fit is refused, never truncated), written into a
`[u8; 128]` on the stack by one speller, `Instrument::write_crosscode`, in
`rust/market/src/instrument.rs`. Nothing parses the text per row: the Instrument holds the typed
facts the code was written from, and a human reads it in a table.

```text
crosscode := isin | class ':' body

isin      := the twelve bytes of a REAL ISIN (Isin::rank_of == 2: canonical, closed, listed
             prefix - isin.rs:215-220), as the `isin` column spells it (Isin::is_canonical).
             The production of every instrument an agency has numbered: a cash security (CFI
             category E, C, D, R, T, L, M, or unclassified) stating one, and a derivative stating
             one whose body cannot yet be spelled (the PLACEHOLDER, D42.5). The CFI never enters:
             ISO 6166 already makes the number unique across classes, and Cfi::refined
             (cfi.rs:652-679) is a fact that refines.

class     := the CFI category and group, two letters - the two positions Cfi::refined never
             moves (a differing category or group answers None, cfi.rs:657-660; attributes 3-6
             are what `X` fills in). Never `XX`: an instrument whose class is unknown and whose
             body is empty has no key under this production.

body (I*) := forex                                 IF:EUR/USD   IT:XAU/USD
body (J*) := forex ':' settle                      JF:EUR/USD:M3   JF:EUR/USD:2027-01-15
body (S*) := forex ':' settle ':' settle           SF:USD/JPY:0:M3        (near, then far)
body (O*, H*) := underlying ':' expiry ':' strike  OC:US0378331005:2026-12-18:200
body (F*) := underlying ':' month                  FF:EU0009658145:2026-12
body (K*) := leg ('+' leg)+                        KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03

forex     := the seven canonical bytes of `Forex` (`CCY/CCY`, forex.rs:46-47, canonical_pair).
settle    := the ISO date of a stated SettlDate(64) / SettlDate2(193) (`YYYY-MM-DD`), else the
             FIX SettlType text `FxSymbol` spells (`0`, `1`, `2`, `C`, `W1`, `B`, `M3`, `Y1`;
             forex.rs:311-359). The two never collide: a type is 1-3 bytes with no `-`, a date 10
             with two. A spot stating neither is the class alone (`IF:EUR/USD`).
underlying := the underlying instrument's own cross code, verbatim - a real ISIN for a security
             or an index, a `class:body` for a derivative on a derivative. An underlying no
             instrument keys gives the derivative no body (D42.5); a symbol is never written.
expiry    := the ISO date of MaturityDate(541), MaturityDay folded into it (retired.rs:1014),
             else MaturityMonthYear(200) as `YYYY-MM`, or `YYYY-MMwN` where 200 states a week.
month     := MaturityMonthYear(200) as `YYYY-MM` (`YYYY-MMwN` for a weekly listing), else the
             month of MaturityDate(541). A listed future's identity is its contract month; its
             last trading day is a fact of the instrument.
strike    := the `Decimal` strikepx's Display - the shortest exact text (decimal.rs:1237-1241):
             `200`, `200.5`, never `200.00`. Put/call is the group letter (`OC`/`OP`,
             fix/cfi.rs:58-73 reads PutOrCall(201)); a message classified `OM` with no 201 has
             no body.
leg       := [ratio '*'] crosscode of the leg instrument, ratio a whole number other than 1
             (LegRatioQty(623)); legs sorted bytewise and deduplicated, so a spread and its
             reverse are one instrument and the side stays the order's. One level: a leg is
             never a strategy.
```

Not in the code, ever: ticker, MIC or any listing fact, trading currency, country, LEI, RIC,
FIGI, CUSIP/SEDOL/WKN/Valor, FISN, EUSIPA, multiplier, exercise style and the other CFI
attributes, the underlying's or a leg's uuid (the body names them by their codes), the minted
ISIN (derived from the code, so it cannot be in it), srcuuids. Each is a fact under the uuid and
feeds `hashcode` (D42.4).

The stored market codes are untouched: an order's `{kind}:{side}:{base}` (`stored_crosscode`,
market.rs:719-742), a book's `3:0:{book_crosscode}` with `book_crosscode` the lifted `isin`
column (real or minted), else the ticker, else `Isin::NONE` (market.rs:476-480), every arm still
borrowing. Question 5 is whether a later slice keys the book by the instrument's code.

## D42.2 - the examples

Digests computed with the reference XXH3 (`python/.venv` xxhash 4.0.1, seed 0, the algorithm
`twox_hash` one-shots implement); the Luhn re-spelled from isin.rs:229-267 and checked against
`US0378331005` -> 5, `EU0009658145` -> 5, `EZN11TD1F7K` -> 3. The pins belong in
`rust/market/tests/root/instrument.rs` once the file exists, run by the crate.

| Instrument | crosscode | bytes | crosshashcode (XXH3-64) | crossuuid = uuid (raw XXH3-128) | minted ISIN |
| --- | --- | --- | --- | --- | --- |
| Apple common stock, US0378331005, CFI ESVUFR | `US0378331005` | 12 | `27e376388c8738fd` | `0751ad36-5cf7-4b90-a684-97f910de04d1` | - |
| Apple before its CFI is known (ESXXXX, or none) | `US0378331005` | 12 | same | same - D40.3's `xxh3_128(isin)` byte for byte | - |
| Apple on a ticker-only feed (no ISIN) | none | - | - | no instrument; `instrumentuuid` null, the book keyed `3:0:AAPL` as today | - |
| EURO STOXX 50 index, EU0009658145 (CFI TI....) | `EU0009658145` | 12 | `fe3a11ff3d28a176` | `a12e6b6d-e671-1288-33fa-826e6c0e1431` | - |
| EUR/USD spot (parse derives CFI IFXXXP, fix/forex.rs:188) | `IF:EUR/USD` | 10 | `f4bd070ccad3d90f` | `f321b727-ba38-79b3-deef-7802c15140ed` | `QYLTVIRYHNX5` |
| EUR/USD 3M outright forward (JFTXFP) | `JF:EUR/USD:M3` | 13 | `817c2b867e12a51a` | `26bc7f23-4ee6-ebc8-dfbd-af8ea5ffd9c3` | `QYIJ9KBCDDV1` |
| EUR/USD forward settling 2027-01-15 (SettlDate stated) | `JF:EUR/USD:2027-01-15` | 21 | `b1e8c6896a1c7a77` | `51bfacda-c6df-7eff-1749-ae5bba336cea` | `QYI2FZFJUNE8` |
| USD/JPY FX swap, spot against 3M (SF....) | `SF:USD/JPY:0:M3` | 15 | `7fffc4bbae889ca0` | `72ccaabc-120d-ea3a-3c74-35b742e2643b` | `QYKXOCJXPVF9` |
| XAU/USD spot (metal, ITKXXX) | `IT:XAU/USD` | 10 | `f0083480b6c87b8d` | `65c69e65-9b16-397a-d8b7-9717bf6d52ff` | `QY900CSWCFZ9` |
| Call on Apple, strike 200, expiry 2026-12-18, American, multiplier 100 | `OC:US0378331005:2026-12-18:200` | 30 | `6f50390c1bd8f746` | `0524521f-6a2f-062f-94db-b3d76f1c4324` | `QYK7DLZMSYS1` |
| The same call at strike 210 | `OC:US0378331005:2026-12-18:210` | 30 | `3b15a30b31569400` | `7e1e77f0-fbe9-6a15-534c-a92efc67609f` | `QYG1U5ULBQ73` |
| The put at strike 200 | `OP:US0378331005:2026-12-18:200` | 30 | `abb15048d5817f7b` | `26b686c7-c4a5-0b7d-acd7-917bbe025587` | `QY6TB00ZO934` |
| FESX future, Dec 2026 (FFICSX) | `FF:EU0009658145:2026-12` | 23 | `4de3b6fc251f4c55` | `c6d73527-abdd-58ed-7b71-8bef09fe7d0a` | `QY4NFU6XFYI7` |
| FESX future, Mar 2027 | `FF:EU0009658145:2027-03` | 23 | `1024ff3991480a91` | `1701b470-2248-4246-f964-c92f08f37805` | `QY3KUS57QPX3` |
| Calendar spread Dec 2026 / Mar 2027 (KE....), stated in either leg order | `KE:FF:EU0009658145:2026-12+FF:EU0009658145:2027-03` | 50 | `69efa5da470620f0` | `1899ed32-50c0-559c-2e61-5931a5e003a7` | `QY9THOC9IUF9` |
| An Apple option stating Eurex's `DE000C...` and no strike (placeholder) | `DE000C......` | 12 | its XXH3-64 | its XXH3-128 - D40.3's value - until the strike and expiry arrive (D42.5) | - |

The derivation of every row: `crosshashcode = xxh3(crosscode)` (`crosshash`, element.rs:516-520),
`crossuuid = Uuid::new(xxh128(crosscode))` (D40.5), `uuid = crossuuid`, the minted number D42.3's
projection of `crossuuid`. Uniqueness in the table: the option and its underlying differ by
`OC:` + fields; the two strikes by the strike bytes; the call and the put by the group letter;
spot and forward by `IF`/`JF` and the settle; the swap's legs are the `IF` and `JF` instruments
and the swap a third; the spread's legs are the two futures, and sorting makes Dec/Mar and Mar/Dec
one. Two processes, in any order of arrival, write one code from one set of facts: every byte is
a pure function of what a message of the instrument states, canonicalized once (`Forex`,
`Date32`, `Decimal`, the sorted legs), and nothing held between messages enters.

## D42.3 - the custom ISIN

Minted only for a `class:body` instrument whose `securityids` hold no ISIN of rank >= 1 - never
for a real-ISIN instrument, never for a placeholder - by one function in `rust/src/isin.rs`,
`Isin::minted(nsin: &[u8; 9]) -> Isin`, and held as an identifier, never as a key byte:

- **Prefix `QY`.** ISO 3166-1 reserves `QM`-`QZ` as user-assigned and `StringEnum::COUNTRIES`
  excludes the whole range (string.rs:2314-2319), so no national agency numbers under it, and it
  is none of the ten agency prefixes (`AGENCY_PREFIXES`, isin.rs:44-45): `is_listed_prefix` is
  false (isin.rs:190-196), `securityid::embedded` derives no CUSIP/SEDOL/WKN/Valor from it
  (securityid.rs:34-52 reads national prefixes alone), `Country::currency` answers none. Not `XX`:
  that is `Isin::NONE` and the masked-line prefix (isin.rs:38). Not `ZZ`: the crate's own rustdoc
  gives it ISO 6166's meaning, "a derivative no agency has numbered yet" (isin.rs:176-177), which
  is exactly the number a venue mints privately, so a stated `ZZ` number and ours would stand at
  one rank indistinguishably; a `QY` number in any table is this crate's and nothing else's. The
  rustdoc of `is_listed_prefix` gains the sentence saying so.
- **NSIN.** Nine base-36 digits (`0-9A-Z`, the reading `code::identifier_value` already gives a
  character), most significant first, zero-padded, of the low 46 bits of the instrument's
  `crossuuid` - the XXH3-128 the element already holds. `36^9 = 101,559,956,668,416 > 2^46 =
  70,368,744,177,664`, so nine digits always hold 46 bits, and the mint adds no digest: the
  number is a projection of the identity, the same in every process.
- **Check digit.** `Isin::closing_digit(&body[..11])` (isin.rs:229-267) - the crate's one Luhn,
  letters expanded to two digits - so `Isin::is_closed` answers true. Written into a `[u8; 12]`
  stack slot, adopted through the crate-private `from_proven`.
- **Rank 1** (`rank_of` = closed + listed = 1 + 0, isin.rs:215-220): below every real number (2),
  above a mask (0). So `Identifiers`' base rule and `CodeValue::merge_with` replace it by a real
  ISIN whatever the order, `yield_lower_isin` (market.rs:1480-1499) yields it, and
  `IdType::Isin.is_real` is false for it - every door that today requires a real ISIN keeps its
  meaning. A typo under a listed prefix also ranks 1 and neither displaces the other - stated.
- **Where it lands.** In the parse, under `derived:isin` beside `derived:forex`
  (msg.rs:7092-7097), so a stated ISIN always wins; on the Instrument, also under its own source
  `yggdryl:isin`, so a derivative whose real ISIN is learned later still spells the number it
  had, and a reader joining on an old `isin` cell finds it. It lands on the row's lifted `isin`
  (D40.2-3), so `book_crosscode` keys the FX book `3:0:QYLTVIRYHNX5` from the first parsed row
  and a ticker-only FX symbol joins that book.
- **Collision.** 46 bits: `p ~ n^2 / 2^47` - `1.9e-6` for `DEFAULT_MAX_INSTRUMENTS` (16,384),
  `0.7%` at a million minted instruments. Detected at the mint, never folded: the table indexes
  its minted numbers, a second cross code reaching a held number is refused by name
  (`Error::Conflict` naming both codes) and that instrument keeps its 128-bit identity with no
  minted number - a hole in its `isin` cell, its book then keyed by ticker. The identity is never
  the 46 bits.
- **A foreign `QY` number** stated by a feed: a `QY` ISIN whose NSIN is not the projection of the
  element's own `crossuuid` is a stated identifier of another system, kept as stated at rank 1
  and never read as a mint (`Isin::is_minted(text, crossuuid)`: a prefix compare and nine
  divisions, no digest).
- **Country and currency** come from the characteristics, never from the prefix: an FX instrument
  fills `currency` with the quote leg (`Forex::quote`) and no country; a derivative fills both from
  its underlying where that states them.

## D42.4 - the hashing

The Instrument is an `Element` (element.rs:182-358) and no `Event`: it has no instant, no
`time_uuid`, no `into_sequenced_uuid` (txhash/value.rs:355-366). It uses the trait's own rules and
nothing new; one hash family (XXH3), one Luhn, no `from_v8`.

| Code | Rule |
| --- | --- |
| `crosscode` | D42.1, set through `set_crosscode` once at mint and again only at a re-key (D42.5). Never empty for an instrument: a thing with no key has no instrument. |
| `crosshashcode` | `crosshash(crosscode)`, XXH3-64 (element.rs:516-520), through `sync_cross` (element.rs:275-289). |
| `crossuuid` | `Uuid::new(xxh128(crosscode))` - D40.5's raw 128-bit digest, no version bits. D40.5 spells `Uuid::from_u128`; the constructor that keeps every bit is `Uuid::new(u128)` (uuid.rs:441), and `from_v8` would clobber six (uuid.rs:565-567). Two digests over one stack buffer: the low half of XXH3-128 equals XXH3-64 only for 1-3 bytes and past 240 (`rust/tests/xxhash/mod_.rs:19-33`), so neither is derived from the other, and a pin says "two digests per instrument creation", never "one". |
| `uuid` | `= crossuuid`: an element in no chain is a chain of one (element.rs:297-302), and an instrument is a thing, not a statement of one. `finalize` is `sync_cross(); uuid = crossuuid; hashcode = digest().as_u64()`. Two statements of one instrument carry one uuid, which `Element::merge_with` requires (element.rs:352), so the trait's own fold merges them. |
| `hashcode` | the content code: `Element::digest` (the cross code fed first, element.rs:321-327) continued in `feed`'s framing (name NUL bytes NUL, element.rs:524-529) with every fact the instrument states through its typed accessors - `securityids`, `identifiers` (sorted, as `feed_market` feeds them, market.rs:1142-1145), the CFI whole, country, currency, the underlying's uuid, the legs' uuids, each characteristic as its canonical bytes (`Scalar::write_bytes`: a `Date32` count, a `Decimal` at scale 18, the seven-byte `Forex`), the listings, `aliasuuids`. It moves when a ticker, a listing, an LEI or a refined CFI is learned; `uuid` does not. It is what the medallion merge compares to decide whether a stored instrument row is rewritten. |
| `srcuuids` | provenance, never fed (element.rs:304-311): the uuid of the event that created the instrument and of the last event that moved a fact, two at most - D40.6's rule, so no instrument accumulates every event that touched it. |
| `instrumentuuid` | on every market, book and event row: the resolved instrument's `crossuuid`, `uuid` (`arrow.uuid` over `FixedSizeBinary(16)`), nullable - the user's spelling of D40.3's `instuuid`, tag `65_054` renamed and never renumbered (D40.1); a market fact with a holder and a setter taking `overwrite`, fed to the row's `hashcode` like every fact, filled along a chain by `following_market`, and no longer a provided reading. The row's own `uuid` is untouched: `time_uuid` seeds it with the row's own `crosshashcode` (element.rs:1250-1253), the order's or quote's `{kind}:{side}:{base}`. |

For a real-ISIN security `crossuuid` is D40.3's `Uuid::from_u128(xxh3_128(isin))` byte for byte,
so P7's column and P9's are one value and no cash row written under P7 moves.

## D42.5 - what moves the cross uuid, and what never does

**Never**: a CFI refined in positions 3-6 (`ESXXXX` -> `ESVUFR`, `OCXXXX` -> `OCASPS`,
`IFXXXP` detailed) - attributes are not key bytes; a CFI learned at all for a real-ISIN
instrument (none -> `ESVUFR`) - the class is not in an ISIN key; a real ISIN learned for an FX pair
or a derivative after a `QY` number was minted (the DSB's `EZ` for an OTC forward, Eurex's
`DE000C...` for a future) - the `class:body` production never read the ISIN, the real number lands
in `securityids` at rank 2 and in the `isin` column, the minted one stays under `yggdryl:isin`;
a ticker, a MIC, a listing, a trading currency, a country, an origin currency, an LEI, a RIC, a
FIGI, an embedded or stated CUSIP/SEDOL/WKN/Valor, a FISN, an EUSIPA, a multiplier, an exercise
style, a `MaturityDate` on a month-keyed future, a `SettlDate` on a tenor-keyed forward - each
moves `hashcode` alone; the order of arrival and the number of processes - every key byte is a
function of facts a message states and the legs are sorted; a second statement merged
(`merge_with`, same uuid); another event read (`srcuuids` is never fed).

**Moves** - the one planned re-key, the placeholder: a derivative stating a real ISIN and no
body (an option with no strike, an `OM` with no 201, a future with no month, an underlying no
instrument keys) is created under the `isin` production with `placeholder = true`; when a message
spells its body, the instrument is re-keyed once to its `class:body` code - `set_crosscode`,
`finalize`, the old uuid pushed into `aliasuuids` (sorted, unique, like `srcuuids`), the real ISIN
kept in `securityids` at rank 2, no `QY` number minted. A real-ISIN instrument created
unclassified that later shows a derivative body is the same path. Nothing else re-keys: a
restated strike, expiry, pair or settle that disagrees with a held key is another instrument, and
a `class` that disagrees is another instrument by `Cfi::refined`'s own reading (cfi.rs:657-660);
the Instrument refuses to move a key field in place. An underlying's re-key cascades into the
derivatives naming it - the placeholder path alone, since a security's key never moves - and each
takes the same alias step.

Two cases every feed leaves open, decided here and put to the user: a mini and a standard
contract, or an American and a European series, at one underlying, expiry and strike share an
instrument (multiplier and exercise style are venue conventions a feed states irregularly, and
putting one in the key would re-key every option and future the day a feed first states it); a
forward quoted by rolling tenor and the same contract stated by its settlement date are two
instruments, which cannot collide and do not merge.

**`instrumentuuid` on stored market rows stays valid** because it is the instrument's
`crossuuid`, which no fact moves, and the one re-key is reconcilable: the surviving instrument
row carries `aliasuuids`, the table builds its alias index from that column at load (old uuid ->
survivor) so a lookup by either resolves, and a gold join reads `instrumentuuid = uuid or
instrumentuuid in aliasuuids`; a medallion stage re-run rewrites `instrumentuuid` on the rows of
the rebuilt window alone (every silver stage overwrites its 15-minute window, medallion.py:94),
rows outside it keeping the value they were written with. There are no alias rows: an alias row
re-finalized would have `sync_cross` overwrite the `crossuuid` it points with (element.rs:275-289),
the hazard the cost lens named; the survivor owns the list.

## D42.6 - the per-row cost

Per row, in the parse (the door's fixed snapshot, as `IsinTable` is fixed under one lock at open
today, isin_registry.rs:1620) and in the lifecycle (the table under its lock):

- A row stating a real ISIN: the twelve canonical bytes of the `isin` cell packed to an `i128`
  the way `DataType::fixed_ascii(12).ascii_packed` packs them (ascii.rs:277-285: big-endian,
  NUL-padded, ordered as the text), the `DataType` hoisted once - a copy, not a parse - probed in
  the table's `HashMap<i128, Slot>`; a hit copies 16 bytes into `instrumentuuid`. No digest, no
  allocation.
- An FX row: `FxSymbol::from_symbol` already runs allocation-free and memoized per symbol text in
  `FxMemo` (fix/forex.rs:45-48); the memo value gains the instrument slot, so a row pays the one
  map hit it pays today, and the `derived:isin` insert is the `Vec` insert `derived:forex` already
  pays (msg.rs:7097).
- A derivative row: the code written into the `[u8; 128]` from typed cells - the underlying's
  code borrowed off its instrument, the `Date32` through `temporal.rs`' writer, the `Decimal`
  through `write_decimal` into a `fmt::Write` over the slot - and probed in the table's code
  index by those bytes (`&str` lookup, the map's own hash over at most 128 bytes, no `String`).
- A miss, once per instrument for the life of the table: `set_crosscode(String)` (the one
  allocation the trait's signature forces, element.rs:204), `sync_cross` (XXH3-64 and XXH3-128
  over at most 128 bytes), nine divisions by 36 and `Isin::closing_digit`'s 22-digit stack Luhn
  for the mint, one `Instrument` allocation. Order of a microsecond, amortized to nothing.
- A fact learned on a held instrument: `Cfi::refined` over six stack bytes, `Identifiers::merge`
  where the message outranks what is held, one 64-bit content digest - only where something moved.
- A follower in the lifecycle: 16 bytes copied from its chain, as every unstated market fact.
- Arrow: `instrumentuuid` is `FixedSizeBinary(16)` - fixed width, so equality, a join to the
  instrument table, a hash partition and Arrow's row format read 16 bytes with no offsets. The
  variable-length code lives on the instrument table alone, one row per instrument,
  `PARTITION:by truncate(crosscode, 2)` - the ISIN's country prefix for a security, the CFI
  category and group for everything else, two letters either way, the successor of
  `truncate(isin, 2)` (isin_registry.rs:278) - `SORT:by crosscode`.
- Digests: none per row; two at an instrument's creation, one 64-bit content digest per change.
- Pins to add: `allocations` rows in `rust/market/tests/allocations.rs` and
  `rust/fix/tests/allocations.rs` - a thousand real-ISIN rows resolving one instrument allocate
  nothing after the first, an FX row through `FxMemo` nothing on a hit; a `graph` bench row for
  resolve-hit and mint; the strike spellings a feed writes (`200`, `200.0`, `2.0E2`) landing as
  one `Decimal` and one code; the expiry intakes (541, 200 with and without a day or week,
  MaturityDay folded) landing as one text per instrument.

## D42.7 - the instrument table and the migration

`silver.record_keeping.instruments` stops being the ISIN registry's snapshot (one row per ISIN and
market) and holds one row per Instrument element: the six element columns (`uuid` = `crossuuid`,
`crosscode`, `hashcode`, `crosshashcode`, `srcuuids`), `aliasuuids`, `placeholder`, `cfi`,
`country`, `currency`, `securityids` and `identifiers` (the sorted `map<utf8, utf8>`,
identifier.rs:1183-1195), `underlying` (`uuid`, nullable), `legs` (`serie<uuid>`), the typed
characteristics as one struct (`forex`, `settle`, `settle2`, `expiry: date32`, `strike: decimal`,
`multiplier`, `exercise`, each nullable), `listings` nested (`mic`, `ticker`, `currency`, the
listing codes, one entry per market), `firstunix`/`lastunix`/`updunix` as today; merged by `uuid`
with `hashcode` the change detector, so an unchanged instrument rewrites nothing. The seed
(`config/isin/instruments.json`, 209 listings of 208 instruments) is re-expressed as instruments
and read before any parse, which is also what settles most classes at first sight.

Market rows (`silver.fix_messages`, the books, the orders, quotes and executions) carry
`instrumentuuid`. No back-compat (AGENTS.md): silver is replayed from bronze. Nothing written
under P7 moves for a real-ISIN security (D42.4); P7 rows of FX and derivatives hold null and are
filled by the replay. The book's `3:0:{isin}` identity moves only where the lifted `isin` cell
moves - an FX book gains its `QY` key where it had a ticker - with the one sentence D40.6's re-pin
already carries.

## Rejected, one line each

- `fixed`'s digest NSIN as the key (`IF:QYC65MS5KSX9`): a 46-bit identity re-hashed to 128 bits
  folds two instruments silently at `0.5%` per million series, and a human reads nothing.
- `packed`'s and `fixed`'s class prefix on a real-ISIN security (`ES:US0378331005`): the first
  row of most instruments states no CFI, so `XX:` -> `ES:` re-keys once per instrument and every
  D40.3 `instuuid` moves.
- `packed`'s "real ISIN wins" for a derivative stating one: `FF:EU0009658145:2026-12` ->
  `FF:DE000C...` re-keys on a late venue ISIN and two processes mint two keys for one future.
- `readable`'s `uuid = from_v8(hashcode)`: a second, churning identity that `Element::merge_with`
  refuses to fold (element.rs:352).
- `stable`'s degraded ticker key (`XX:AAPL`): folds two companies sharing a ticker across
  venues, re-keys on the first ISIN, and mints two uuids in two processes; a ticker-only security
  keeps its book and no instrument.
- `stable`'s `{cc}:{venue securityid}` and `FF:TI:SX5E:...` by symbol: a key that moves the day
  the facts arrive; the placeholder and the seed are the one path.
- `packed`'s alias rows: a row whose `crossuuid` is set by hand is re-derived by its own
  `sync_cross`; the survivor's `aliasuuids` has one owner.
- `packed`'s venue-order legs: a reversed spread mints a second instrument; the side is the
  order's.
- `fixed`'s and `readable`'s nesting of the underlying's or a leg's uuid, and `readable`'s swap
  body nesting leg codes: a leg's re-key cascades into every parent; the swap spells its two
  settles, the strategy its legs' codes.
- `stable`'s and `packed`'s NSIN from `crosshashcode` bits: both are free; the 128-bit identity
  is the one every row carries, so its projection is the number.
- The `ZZ` prefix: ISO 6166's own convention for an unnumbered derivative, so a venue's private
  number is likeliest under it (isin.rs:176-177); `QY` means nothing outside this crate.
- A readable NSIN (`QYEURUSD...`): a second minting rule with no room for a settle or two legs;
  readability lives in the cross code column.
- A MIC in a cash key: splits one instrument listed twice into two books.
- Multiplier or exercise style in an option key: a venue convention a feed states late, so a
  one-time re-key of every option and future; out, with the collision named (question 3).
- A per-row `xxh128` of the ISIN in place of the lookup (`stable`'s cost section): a digest per
  row where a 12-byte probe answers.
- Keying the book by the instrument's code text in P9: the element holds no code text, so the
  fold would look one up per element or hold a second store; deferred (question 5).

## Put to the user (interpretations taken; say if another was meant)

Answered 2026-10-10 ~00:50 UTC: item 1 as written - "Bare ISIN for securities". Items 2 to 9 are
taken as written unless the user says otherwise. P9 lands before P7 (the user's decision 7), so
the instrument's `crossuuid` follows the element's rule of the day (`rust/src/graph/element.rs`)
and moves with every other cross identity when P7 lands D40.5; tag `65_054` is P9's own.

1. A real-ISIN security is keyed by its ISIN alone (`US0378331005`), the CFI a fact beside it;
   the CFI letters key only what no agency numbers. The literal `ES:US0378331005` costs: one
   `XX:` -> `ES:` re-key per instrument first met unclassified, every D40.3 `instuuid` moving, and
   `aliasuuids` on most securities.
2. The minted prefix is `QY` (user-assigned, listed nowhere, meaning nothing outside the crate),
   not `ZZ`; the rustdoc of `is_listed_prefix` says so beside its `ZZ` sentence.
3. Multiplier and exercise style stay out of the key: a mini and a standard contract, or an
   American and a European series, at one underlying, expiry and strike are one instrument. Decide
   before the first medallion write; adding either later re-keys every option and future.
4. An option is keyed by its expiry day (541, MaturityDay folded, else 200 as stated), a future
   by its contract month (200, a week code kept: `2026-12w3`); a forward by its settlement date
   where stated, else its tenor.
5. `book_crosscode` is unchanged in P9 (the lifted `isin`, real or `QY`, else the ticker, else
   `Isin::NONE`); keying books by the instrument's code (`3:0:IF:EUR/USD`) is a later slice that
   moves every book identity once.
6. The column and tag `65_054` are spelled `instrumentuuid` (D40.3's `instuuid` renamed, never
   renumbered), a holder with a setter taking `overwrite`, filled from the instrument's
   `crossuuid`.
7. A ticker-only security has no instrument and a null `instrumentuuid` until an ISIN is learned;
   a derivative whose underlying no instrument keys, or whose body a message does not spell, is a
   placeholder under its real ISIN or, with none, has no instrument.
8. Strategy legs sorted bytewise with an `n*` ratio prefix where `LegRatioQty(623)` is not 1;
   one level of nesting.
9. The one re-key (placeholder -> body) is reconciled by the survivor's `aliasuuids` and a stage
   re-run of the window, never by rewriting rows outside it.
