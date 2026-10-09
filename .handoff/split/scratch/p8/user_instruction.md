# P8 - the user's instruction (2026-10-09 ~22:25 UTC), verbatim

Once done ensure le fix lifecycle match previous alive and current in lifecycle by matching one common identifier optimizely and propage values

(Reading taken: once the split has landed, the FIX lifecycle walk matches a current event to the
previous alive event of its lifecycle by any one identifier the two share - a chain identity
such as orderid, clordid, secondaryorderid, quoteid, tradeid, whichever one both state - found
through an index keyed by identifier value rather than a scan (optimally), and the predecessor's
values propagate onto the follower (the facts it states nothing of). Design in the foreground
(DESIGN.md D41) after S4 is pushed; order relative to P7 and P5R to decide - the lifecycle walk is
`rust/market/src/graph/iterator.rs` + `rust/fix/src/enrich.rs` after S4.)

## Foreground notes (22:30 UTC) - what the walk does today, and the design questions D41 answers

Today (`rust/market/src/graph/iterator.rs`, `docs/fix/lifecycle.md` "A chain is named by its cross
code"): the walk keys live chains by `(crossuuid, marketdatakind)`; it indexes every name a live chain
goes by - its chain identities (`IdType::is_chain_identity`: orderid, clordid, secondaryorderid,
secondaryclordid, quoteid, secondaryquoteid, tradeid, secondarytradeid, secondaryfirmtradeid,
tradereportid) and the chain's first value a lineage field names - in `named: HashMap<IdType,
HashMap<String, Vec<(Side, Chain)>>>` (an index, no scan) and the sided bases in `bases`; a message
cites its own live chain, the one side its base is alive on where it states no side, and each chain
a name it states is held by; one chain cited -> it follows (values propagate: `following_market`,
`follow_parents`, metadata, identifiers under the `FIX:idmap` follow flags); none -> a chain of its
own; two -> a conflict, standing alone with a `FixAnomaly`; "nothing merges two chains".

Open for D41 (decide on evidence, never by assumption):
1. Which identifier types count as "one common identifier": the ten chain identities alone (today),
   or every type two statements share (`execid` and a twin, `mdentryid`/`mdentryrefid` for book
   entries, a bridge's `oms:orderid`)? Evidence: over `rust/tests/support/ulbridge.log`'s lifecycle,
   count the messages that share an identifier of any type with a live element of their kind and
   did not join it, by type.
2. The two-chains case: a message bridging two live chains (one identifier in common with each)
   stands alone today; D41 may let it join the chain it shares the most recent statement with and
   carry the other's values, or keep the conflict. Evidence: the conflicts the capture's walk
   records (`FixAnomaly` under `crosscode`), what each cited.
3. "optimizely": the index is per name already; what costs a scan today is `bases` (a Vec per
   base) and `Cited`'s per-name slot filter; measure with the `fix` bench (`lifecycle` rows) and
   the allocations rows before and after.
4. "propagate values": which facts a follower takes today and which it does not (a stated `NEW`
   over a live one, the legs of a quote, the identifiers whose idmap entry does not follow); the
   instruction may want every fact the predecessor states and the follower lacks to propagate.
Plan: after S4 is pushed, an evidence workflow (readers on the cheaper tier over the capture's lifecycle output
and the walk's code) -> the design D41 in the foreground -> implementation inside the crates, with
P7 (D40) before it where the two touch the same files (the FIX row's columns), and P5R after both.
