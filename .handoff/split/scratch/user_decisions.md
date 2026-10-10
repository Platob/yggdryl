# User decisions (2026-10-09 17:05 UTC): "Do what's recommended"
Recorded by every lane manager in `.handoff/next/MARKET_SPLIT_NEXT.md`'s questions section and the
DESIGN.md rows named, in its next results commit; no lane re-asks them.
1. S6 landing shape: ONE dependency-closed commit for avro, parquet, s3 and iceberg (+s3tables),
   after excel and xmla have landed as their own commits (DESIGN.md "## S6's shape"); the reversed
   split (iceberg first) is not taken. D39 row and "S6's shape" say "decided by the user".
2. D12 npm name: `yggdryl-market` on all three registries; `yggdryl.market` is not used.
3. Handoff question 8 (D38): `sendunix` keeps the carrier-first precedence (the carrier's instant,
   else `SendingTime(52)`); the alternative is closed.
4. The CLI links `yggdryl-s3` always, as the bindings do; its `s3` feature goes (S6d).
5. Not covered - stays on the user's explicit go, by the program's hard rule against release runs:
   the manual `release.yml` rehearsal and the registry configuration a real publish needs (PyPI
   pending trusted publisher for `yggdryl-market`, the npm bootstrap publish, `CARGO_REGISTRY_TOKEN`
   with `publish-new` over `yggdryl-*`).

# User decisions (2026-10-10 ~00:40 UTC)
6. Release 0.1.22 from PR #209 (implemented: commit `0f411f5ce`): crates.io takes `yggdryl`,
   `yggdryl-market`, `yggdryl-fix`; PyPI and npm keep `yggdryl`. The user: "upgrade to 0.1.22 to
   publish before next steps with medaillon optimized working".
7. The PR's scope before that release, in this order, each a commit read green: P9 (the
   Instrument replacing the ISIN registry, `p9/`), P7 (D40), P8 (D41), P5R (D37); then the live
   AWS run on the user's machine (`.handoff/next/LIVE_AWS_TEST_PROMPT.md`); then the user's merge,
   which publishes 0.1.22. The other crates' split - M6 (S6: excel, xmla, avro, parquet, s3,
   iceberg) and B5 (S5: the market binding packages) - and S7, S8 and S9 are postponed to a later
   PR from `main` after the release. The user: "Include the instrument implementations and add
   local live aws testing prompt to finalize this pr first and publish 0.1.22", then "ask to
   finalize next implementations postponing the othe crates split", answered "All in this PR".
8. The instrument cross code (`p9/crosscode_decision.md`): a security is keyed by its real ISIN
   alone, the CFI a fact beside it; CFI class + characteristics key everything no agency numbers
   (forex, forwards, swaps, options, futures, strategies). Answered "Bare ISIN for securities
   (Recommended)"; the decision's items 2-9 stand unless the user says otherwise.
9. The market rows carry `instcode` - the instrument's `crosscode` as text (`utf8`, tag
   `65_054`) - instead of an `instrumentuuid`: "Replace the instrumentuuid of market to
   instrumentcode mapped to the instrument crosscode", then "Use instcode instead of
   instrumentcode". A re-keyed instrument keeps `aliascodes`.
10. The Instrument holds a `metadata` map for any complementary information, and its `securityids`
    hold the identifiers other sources state under their own `src:type` keys (`ullink:isin`,
    `bloomberg:figi`): "Add also metadata in instrument to put any other complementary infos and
    identifiers to put other sources securityids".
11. A market row's `instcode` shares the instrument's one allocation of its `crosscode`: the
    instrument holds its code once (the crate's `Str`, inline to 23 bytes, one `Arc<str>` beyond)
    and every `instcode` it fills is a clone of it - no allocation per row: "Make the instcode share
    single allocated from instrument crosscode".
12. P8's match (D41): "Ensure also lifecycle fix messages match by marketdatatype, side if sided
    marketdatatype and if any identifier of current is in previous" - answered "Kind": the current
    element matches a previous alive element of the same `MarketDataKind`, of the same side where
    the kind is sided (ORDR, EXEC), sharing at least one identifier the current states.
13. P8 (D41), answered 2026-10-10 ~06:00 UTC: a shared identifier is the same type and value
    (orderid=A matches orderid=A, never clordid=A); every identifier type counts but quotereqid,
    mdreqid and a bridge's parent-order slot. The design's other recommendations stand (the conflict
    kept for two cited chains, the quote tag slot dropped, the follow flags unchanged).
14. P7 (D40): the CFI moves into `securityids` as the one hold map - every row's `securityids` cell
    gains `cfi=..`, re-pinned once with its sentence, the digests feeding it under its own name
    (no hash moves); on the FIX row tag 461's column keeps the registry's name `cficode`, market rows
    say `cfi`.
15. P9's four structural allocation rises are re-pinned once, each with the sentence naming the
    structure that costs it (answered "Re-pin with sentence"): learning a new instrument 1 (its own
    identifiers vector), reloading known rows 94 per batch (the nested instrument row's landing),
    the snapshot drain 5 per row (the nested runs), the FIX lifecycle 22/16/16 (a fresh walk's
    first learn of the instrument's storage). The per-row regressions were fixed at cause.
16. P9b, right after P9 (its own commit): a book is keyed by the instrument's code - the book's
    cross code `3:0:{instcode}` - "Make then book use the instcode as crosscode and check correct
    book iterator generations". A real-ISIN security's book keeps its key (`3:0:<isin>` is its
    instcode); an FX book moves to `3:0:IF:EUR/USD`, a derivative's to its `class:body` code. Then
    refined: "Only instcode, filterout messages with non attributed instcode. Since all isin should
    have clear [auto] created instrument in registry" - the book key is the `instcode` alone, no
    fallback to the ISIN, the ticker or `Isin::NONE`; an element with no `instcode` is pruned before
    the book walk (as an unrecorded kind is), and every element stating a real ISIN gets its
    instrument auto-created and its `instcode` filled, so only a ticker-only or code-less element
    goes unbooked. The book iterator's generation is verified adversarially (one book stream per
    instrument, withdrawals across keys, snapshot ticks whole, the medallion's books table).
    This supersedes the cross-code decision's item 5 (books keyed by the code deferred).
17. P10, after P9b (its own design D44, then its commit): "Make instrument registry unique by
    intrument code and mic since an instrument can be used on other markets adding default mic.
    Ensure it correctly updates the fix message. Ensure fix codec parsing checks correct instrument
    informations like checking option type when strike price given, or future ... find informations
    to make robust inference from straight isin to fallback rules from the internet and fix
    registry, with also coalesce validated instrument informations like bloomberg, ric, mic,
    currency, country, strikepx, etc ... and propagates in life cycles leverage instrument
    centralization". Read as: the registry's rows unique by `(instcode, mic)`, the market `XXXX`
    (`Mic::none()`) where none is stated; the FIX message filled from its `(instcode, mic)` row; the
    parse infers the instrument by a ladder from a stated real ISIN down to fallback rules (CFI,
    SecurityType(167), the fields stated - a strike makes an option, a maturity with no strike a
    future - checked for contradictions) drawn from the FIX dictionary and public standards; the
    validated facts (Bloomberg, RIC, MIC, currency, country, strike, ...) coalesced on the
    instrument by rank and propagated along the lifecycle from the central registry.
18. P10 (with D44): "Create then the IntrumentEvent leveraging the Instrument and make registry
    use intrumentevent to store versions". Read as: `InstrumentEvent` - the Instrument as an
    `Event` (an instant, a state, a predecessor named by `prevuuid`), the way an order is an
    `OrderEvent` - and the registry keeps each `(instcode, mic)` row's versions as instrument
    events: a learn that changes an instrument's content appends a new version (its own `uuid`
    from the graph's hashing of its content and instant, `prevuuid` the version it replaces), the
    current instrument being the latest version per key; the store and the medallion's instruments
    table hold the versions.
19. P10 (with decision 18): "And then registry keeping correct time sorted deduplicated" - each
    `(instcode, mic)` key's instrument events are held sorted by their instant (`transunix`), a
    version arriving out of order inserted at its place with the `prevuuid` links following the
    order, and deduplicated: a statement that changes no content appends no version, two equal
    versions are one.
20. P11, a lane of its own (core + Python + docs + the medallion): "Add also field compatibility
    scheme to doris to cast timestamp ns timezoned to timestamp us zoned since iceberg
    timestamptz_ns is not handled by doris, apply it in medaillon example" - a `Scheme::DORIS`
    compatibility scheme beside `Scheme::ICEBERG` and `Scheme::SPARK` (`rust/src/scheme.rs`,
    `rust/src/compatibility.rs`, `into_scheme_compat`): a zoned nanosecond timestamp becomes a zoned
    microsecond one (Iceberg v3 `timestamptz_ns` -> `timestamptz`), with whatever else Apache
    Doris's Iceberg reader cannot read (from its type-mapping documentation), and the medallion
    example writes its tables under it.
21. "refine transunix fix parsing rule to be by default accepting with less than 500ms gap": an
    official transaction clock (`TransactTime(60)`, the `TrdRegTimestamp(769)` ranks) dates the
    message's `transunix` only where its gap to `SendingTime(52)` is LESS than the delay
    (`within`: strict), the default delay 500 ms (`FixCodec::DEFAULT_OFFICIAL_TIME_DELAY_MS`, was
    1,000 inclusive); a nonpositive delay still admits only equality. Its own small commit.
22. P10: "optimize then the instrument registry to push snapshots every hours and handle correctly
    expired instruments like futures or options or others inferring at maximum the maturity dates
    but also check its correct time leveraging the fact that we know the instrument country to
    timezone". Read as: the registry publishes a snapshot of its instruments once per hour of event
    time (the medallion's instruments stage writes per hour, not per message batch); an instrument
    with a maturity (future, option, forward, bond, ...) expires: its maturity inferred as far as
    the facts allow (MaturityDate(541), MaturityMonthYear(200) with MaturityDay(205) or the
    contract's rule - a week code, the month's third Friday where the venue's rule says so -, a
    settle date, a tenor from the trade date), its expiry instant the maturity's end of day in the
    instrument's own timezone (the country of its market or of issue -> its zone, the crate's
    bundled IANA registry), and the expiry recorded as an instrument event whose state says it
    expired; a statement after expiry is checked against it.
23. P10: "make then instrument registry unique by unix instcode and mic since its events now" - the
    registry's rows (instrument events) are unique by `(transunix, instcode, mic)`: one version per
    instant per key, sorted by the instant (decision 19), the current instrument the latest.
24. D44's and P9b's readings (`p10/d44_design.md`, `p10/p9b_design.md` "Put to the user") are taken
    as recommended to keep the pace the user asked for ("Compact all remaining steps ... the
    fastest"), each overridable: P10 lands as P10a (decision 17 + the flat (instcode, mic) row)
    then P10b (decisions 18, 19, 23); a statement naming no market lands on XXXX even where the
    instrument lists one market; a classified CFI (two letters) is kept; coalescing is validity
    first, then warn-and-keep for instrument facts under learn and replace under merge; the
    registry commit is append-only; the Bloomberg exchange-code rung is left out of P10 (its
    source table, the OpenFIGI CSV, is refused by this container's proxy) and listed for later;
    P9b deletes `BookEvent::new` and its hand-built fixtures state `instcode` = their symbol; an
    FX book keys `3:0:IF:EUR/USD`. Decision 22 (hourly snapshots, expiries in the instrument's
    timezone) is designed by worker B as D44's next section before it is implemented.
