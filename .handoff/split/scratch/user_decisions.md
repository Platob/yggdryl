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
25. P8 tightened (answered "Scope it"): a current element matches a previous alive element of the
    same `MarketDataKind`, the same instrument (`instcode`) where both state one, the same side
    whenever both state one (not only for ORDR/EXEC), sharing one identifier of the same type and
    value; identifiers many elements share never match - `trdmatchid`, `quotereqid`, `mdreqid` and
    the parent slots. A bid and an offer under one entry id stay two entries; entries of two
    instruments under one entry id stay apart; two orders filled in one match are no conflict.
26. P12, the fill accounting (its own design and commit, after wave 1): "refine fixmessage
    lifecycle to find a stable way to refine the state for partiallyfilled orders executed
    leveraging the with_previous and compare the execid to when if different updates the real
    consumed and update the remaining quantities, and if it hits 0 it changes the part fill like
    states to fully filled". Read as: along an order's chain, an execution report whose `execid`
    differs from every fill the chain already counted adds its `lastqty` to the consumed quantity
    (`cumqty`) and recomputes the remaining one (`leavesqty` = `ordqty` - `cumqty`); a repeated
    `execid` (a resend, a duplicate) counts nothing - stable whatever the order of arrival; where
    `leavesqty` reaches 0 a partial-fill-like state becomes its filled state.
27. P12 (with decision 26): "ensure to have coherent overall definition for generic quantity
    field which is always the remaining available quantity too or of others the executed
    quantity". Read as: the market fact `quantity` has one definition per kind - on an order, a
    quote and a book entry it is the quantity still available (an order's `leavesqty`, a quote
    leg's or an entry's remaining size), on an execution and a trade it is the quantity executed
    (`lastqty`); every door that sets or derives it (the FIX parse, the fill accounting, the book
    fold, the Arrow rows, the docs) follows that one rule.
28. P7 (amends D40.5/D40.6, the lineage): "then ensure the uuid lineages where fix messages cross
    code are the same as leafs market operations, srcuuids for fixmessages are the log text messages
    uuids and the book srcuuids are the distinct srcuuids of all its inner events - thus ensuring a
    lineage filtering on crosscode either on fix messages or market operation leaves and be able to
    get the used books and log messages in medaillon". Read as: a FIX message's `crosscode` is the
    same text as the cross code of the market operation leaves it splits into (one stored cross
    code, `{kind}:{side}:{base}`, so a filter on it finds the message and its leaves alike); a FIX
    message's `srcuuids` are the `uuid`s of the log text lines it was parsed from; a leaf carries
    its message's `srcuuids`; a book's `srcuuids` are the distinct `srcuuids` of every event inside
    it (its delta and its events), sorted - superseding D40.6's "the delta's and the events' uuids
    and the previous book's uuid"; the medallion can go from a book to the log lines it used and
    from a cross code to every FIX message and leaf of that chain.
29. D45's readings (`p12/d45_design.md`, "Put to the user") are taken as recommended, by decision
    24's rule for pace, each overridable: the count anchored on the chain's first stated total
    (the user's "compare the execid ... updates the real consumed and update the remaining
    quantities") - a later stated CumQty/LeavesQty that disagrees is warned, not adopted, so the
    capture's order 557 ends 472 / 128 `PARTIALLY_FILLED` (its From Exchange `151=0` frames
    describe the exchange-side child); the ended-chain tombstone (D45.8, 60 s) lands in P12, so a
    replayed execid restates its chain and the capture's executions read 39 / 21 / 7; ExecRefID(19)
    read by tag inside `FixMsg::fill_of`, no `FIX:idmap` entry (the dictionary hash stands); the
    ledger for orders only (quotes later); `execid` stays in the followed set; disagreement,
    overfill and unidentified fills are deduplicated warnings, not `FixAnomaly`; order 9623's
    `partfilled` read through `State::from_spelling`; decision 27 (EXEC/TRAD `quantity` =
    `lastqty`) lands in P12's commit.
30. P13, the CBlock counters (its own small commit, first of the next lanes): "ensure also in the
    cfb files fix ingestion parsing it ignores silently the components having the numingroups fields
    since our representations are structures and correct group list typed". Read as: a CBlock
    grammar - a message root, a group's entry, a folded second binding of one wire type - that
    states a NumInGroup counter as a plain member beside the group it counts (or a counter the
    reader knows counts a group the same structure holds) has that counter member dropped by the
    reader, silently - no warning, no `FixMerge::dropped` entry - and the message, the group and
    every other member kept, since a group is its typed list and its length the count; today that
    message is dropped whole with a warning (`catalog.rs` validate_references, pinned by
    `rust/fix/tests/root/cfb.rs` `a_message_the_catalog_will_not_hold_is_named_at_its_grammar_binding`).
    The catalog's own refusal stays for a dictionary built by hand; the reader never produces one.
31. P14, the text line's metadata (design D47, then its own commit): "add then also in textline
    generated schema a metadata map str str which can parse key values handling doubled keyed
    values json serialized to list and value default json serialized" (`p14/user_instruction.md`).
    Read, to be settled by D47: the plain-text row a `TextLine` generates gains a `metadata`
    column, `map<utf8, utf8>`, filled by a key-value reading of the line the options ask for; a key
    stated twice or more holds the JSON array of its values in order; a value that is not plain
    text is held as its JSON serialization.
32. P15, CSV over the text line (design D48, then its own commit): "then make the csv like media
    internally use the existing text line implementations but auto handling headers or infer
    headers with separator etc optimizely". Read: the delimited-text media (`text/csv`,
    `text/tab-separated-values`) read and write through the text medium's line machinery (the
    bounded physical-line splitting, the transport's decoding, `TextLine`) instead of a second
    tokenizer of their own, a header row handled or inferred and the separator inferred, at no
    worse cost than today's reader (its benchmark and allocation pins the bar).
33. With 31: "thus first medaillon layer can accept key value inputs with this metadata field
    too" - the medallion's first (bronze) layer reads key-value log lines into that `metadata`
    column beside the FIX capture it reads today.
34. Amends 31 and 33: "rename the text line metadata to keyvalues instead" - the text row's
    column read from key-values is `keyvalues` (`map<utf8, utf8>`), never `metadata`; every
    door, option, doc and the medallion bronze table spell it so.
35. D46's readings (`p13/d46_design.md`, "Put to the user") taken by decision 24's rule, each
    overridable: a group's own counter restated inside its entry (`<grammar><555/><555/><556/>`)
    is a NumInGroup field in a component and is dropped too (the user's "components having the
    numingroups fields"); nothing is logged for a dropped counter (the literal "silently"); a
    dropped constraint's `required` carries onto the group (required where either requires it);
    the fold's omission is gated on any fold with another dictionary (`drops.is_some()`), so
    `merge_with`, `add_cfb_file(s)` and `add_json_file` fold without the duplicate and the
    catalog's refusal stays for the doors that write one definition alone; the message-dropped arm
    of `Parse::dictionary` is deleted with its sentences if the empty-type test cannot reach it.
36. D47/D48 answered by the user (2026-10-10): "Only when asked" - `parse_keyvalues` off by
    default, the bronze layer turning it on, today's text reads unchanged; "Plain unless needed" -
    a single text value as itself, a repeated key the JSON array of its values, a nested or
    JSON-opening value its JSON, one owner shared with the FIX row's maps; "Yes, read it" - a
    double-quoted loose value (`msg="hello world"`) is one pair, escapes kept as written; "Infer by
    default" - CSV's header and separator inferred where nothing is stated. The other readings of
    D47/D48 taken as recommended by decision 24's rule: `keyvalues` in `bronze.log_messages` alone;
    the header tri-state on `CsvOptions` with the shared `header()`/`set_header(bool)` unchanged; no
    CSV intake in bronze; no preamble skip; the CSV exchange script with `duckdb` installed in its
    job.
37. P16, the cross identity from the creation instant (design D49; lands with P7, which moves every
    cross identity once and whose D40.5 it supersedes): "change the crossuuid rule to take creaunix
    and use txhash generally taking creation unix if given, else on lifecycle pass or merge with
    different minimum creaunix reset it using the crosshashcode, also updated on crosscode or
    crosshashcode changes" (`p16/user_instruction.md`). Read, to be settled by D49: `crossuuid` is
    the `TxHash` of the element's `creaunix` and its `crosshashcode` - the crate's time-ordered
    identity, as `time_uuid` couples `transunix` and `hashcode` - wherever a creation instant is
    stated; the lifecycle walk and a merge, which keep the earliest `creaunix` of the two
    (`fold_lifecycle`), reset `crossuuid` from the `crosshashcode` and that minimum whenever it
    differs from the one the element held; and `sync_cross` recomputes it whenever the `crosscode`
    or the `crosshashcode` moves. What an element stating no creation instant holds is D49's to
    settle.
38. Restates 37 (the user's second wording replaces the first): "change the crossuuid rule to take
    creaunix and use txhash generally taking creation unix if given else initialize the lifecycle
    with the transunix, else on lifecycle pass or merge with different minimum creaunix reset it
    using the crosshashcode, also updated on crosscode or crosshashcode changes". Read: an element
    stating no `creaunix` has its lifecycle initialized with its `transunix` - the creation instant
    is the transaction instant of the first statement that opened the chain - so every event's
    `crossuuid` is the `TxHash` of a creation instant and its `crosshashcode`; a walk or a merge
    keeping an earlier `creaunix` resets it; a move of the cross code or its hash recomputes it.
39. With 38 (P16, D49): "leverage the optimized set unix of unix and ensure to update on diffs for
    uuid txhash or hashes to not rehash always". Read: the identities and digests an element holds
    - `uuid` (the `TxHash` of `transunix` and `hashcode`), `crossuuid` (the `TxHash` of `creaunix`
    and `crosshashcode`), `crosshashcode` (the XXH3-64 of the cross code) and the content
    `hashcode` - are recomputed only where an input they read moved, through the setters that
    already project on a set (`set_transunix`/`set_seqnum` calling `refresh_uuids`/`derive_uuids`,
    the `moved` diff helper): a setter writing the value already held derives nothing, a cross code
    written unchanged is not hashed again, and `sync_cross`/`finalize` read what moved rather than
    rehashing every time. Each such skip is pinned in the allocation and benchmark rows.
40. D49 answered by the user (2026-10-10): "Reset to new minimum" - a live chain takes an earlier
    creaunix when one arrives and every member from then on carries the new crossuuid, members
    already yielded keeping theirs; "Only lifecycle writes it" - the identity reads creaunix else
    transunix, and a parse, a text read, a walk or a merge writes the column, a hand-built event
    never passed through one keeping a null creaunix; "Read's running minimum" - a text line's
    crossuuid takes the earliest creaunix the read has met for its object. The other D49 readings
    taken as recommended (decision 24's rule): a code-less event keeps its own uuid and an element
    with no instant keeps from_v8(crosshashcode), D40.5's XXH3-128 withdrawn; a pre-epoch instant
    falls back to the code rule; InstrumentEvent's crossuuid follows the event rule, P10 joining by
    instcode; FixMsg's content digest gated by a `digested` bit (decision 39 reaches it); the
    CrossUuid description rewritten inside P7's regeneration; "the optimized set unix" read as the
    projecting setters extended to set_creaunix, finalize still deriving both identities.
41. The user's go for the release (2026-10-10): "once all landed commit to main and monitor main
    realease github action". Read: when every queued lane has landed on the PR branch (P13, P12,
    P10a, P10b, P14, P15, P5R, P16 with P7) and CI reads green at its head, PR #209 is marked
    ready and merged into `main`, which runs `.github/workflows/release.yml` and publishes 0.1.22;
    that run is watched to its end and a failure is fixed at its cause. This lifts the program's
    "never push to main / never merge" rule for this one merge. The live AWS run
    (`.handoff/next/LIVE_AWS_TEST_PROMPT.md`) is the user's and is not done by this session;
    `CARGO_REGISTRY_TOKEN` must allow publishing the new crates `yggdryl-market` and `yggdryl-fix`.
