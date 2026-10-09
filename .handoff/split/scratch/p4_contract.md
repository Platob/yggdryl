# P4 contract - D38: `currunix` -> `transunix`, `recdunix` -> `sendunix`, `curruuid` -> `uuid`, `currhashcode` -> `hashcode`

Design: `scratchpad/d38_design.md` (normative). Maps: `scratchpad/instants_map/core.md`,
`outer.md`. Program rules (AGENTS.md is in your context): no test code under any `src/`; no
back-compat (no alias, no legacy reader, the old names gone everywhere in one commit); one
commit for the slice; the exact trailer on it; no model identifier in the repo; never push to
main, never publish, never touch live AWS; a cost pin never re-pinned upward.

## What the slice is
A rename of two event instants and of the element's two `curr` names at every door, with
their meaning restated: `transunix` the transaction instant (required, the identity and order
axis), `sendunix` the technical wire clock (optional, the merge reference), `uuid` the element's
own identifier (was `curruuid`), `hashcode` its content code (was `currhashcode`); `prevuuid`,
`crossuuid`, `crosshashcode`, `srcuuids` and `currency` are untouched. Precedences, values, counts, column positions and tag
numbers do not move. The crate dump, the dictionary hash, the equivalence snapshot and
`docs/assets/fix.json` are regenerated once each; the census is unmoved.

## Stage 1 - the define worker (no cargo; one script)
Work in a detached worktree of the swept S3 tree: `git -C /home/user/yggdryl worktree add
--detach /home/user/yggdryl-p4 wip/s3` (that tree is uncompiled; it is where the anchors
are). Never edit `/home/user/yggdryl` or `/home/user/yggdryl-s3`. Write
`scratchpad/p4_sweep.py`: a Python script taking the tree root as its argument, applying
exact-string edits file by file, each anchor asserted to match the count the script expects,
idempotent (a second run changes nothing), covering:

1. Bare names, word-bounded, in every text file of the tree but `target/`, `.git/`,
   `node/node_modules/`, `python/.venv*/`, `.handoff/` (history stays), `rust/tests/fix/equivalence.snapshot`
   (regenerated), `config/fix/**` (regenerated), `docs/assets/*.json` (regenerated),
   `node/index.d.ts`, `node/index.js` (regenerated): `currunix` -> `transunix`, `recdunix` ->
   `sendunix`, `curruuid` -> `uuid`, `currhashcode` -> `hashcode`, `CurrUnix` -> `TransUnix`,
   `RecdUnix` -> `SendUnix`, `CurrUuid` -> `Uuid`, `CurrHashCode` -> `HashCode`, `CURRUNIX` ->
   `TRANSUNIX`, `RECDUNIX` -> `SENDUNIX`, `CURRUUID` -> `UUID`, `CURRHASHCODE` -> `HASHCODE`,
   `Currunix` -> `Transunix`, `Recdunix` -> `Sendunix`, `Curruuid` -> `Uuid`, `Currhashcode` ->
   `Hashcode` (Node camel identifiers such as `eventCurrunix`), `walked_currunix` ->
   `walked_transunix`; `get_curruuid` -> `get_uuid`, `get_currhashcode` -> `get_hashcode` and
   their setters. Collisions to check and resolve by hand, each named in the report: a type,
   module or import already called `Uuid`/`uuid`/`HashCode` in a scope the new variant or method
   lands in (the `uuid` crate, `Scalar::Uuid`, `DataType::Uuid`, `ElementColumn::Uuid` are
   namespaced and fine; a `use ElementColumn::*` or a local `uuid` binding is not), a Python or
   Node property `uuid` or `hashcode` already present on a class that gains one. The regex
   excludes `refrecdunix` (the retired FIX field 65_064 keeps its name: use a negative
   lookbehind for `ref`) and any other identifier where the old name is a substring of a
   longer FIX field name - list each exclusion the grep finds and why.
2. Meaning, re-written by hand as edits in the same script (exact anchors):
   - `graph/element.rs`: the `Event` trait docs for the two getters/setters and the trait
     doc paragraphs (:917, :968, :973), `right_is_reference` and `fold_event_instants` docs,
     `time_uuid`/`txhash` docs - the new names and the design's sentences.
   - `graph/element_column.rs`: `Uuid` display `UUID`, `HashCode` display `Hash Code`,
     descriptions re-spelled ("The element's own UUID ...", "The XXH3-64 of what the element
     states."); positions unchanged; `graph/element.rs` `Element` getter/setter docs.
   - `graph/column.rs`: `TransUnix` display `Transaction Time`, description "When the
     operation happened: the settled transaction instant, UTC."; `SendUnix` display
     `Sending Time`, description "When the message crossed the wire, where that is known;
     the earliest its statements know."; positions and nullability unchanged.
   - `fix/crated.rs`: `UUID_TAG_NAME` (65_001), `HASHCODE_TAG_NAME` (65_004),
     `TRANSUNIX_TAG_NAME` (65_007), `SENDUNIX_TAG_NAME` (65_009), their docs, the `.saying(..)` of 65_009 ("When the message crossed the wire: its carrier's
     clock where the carrier states one, else the sender's SendingTime; the earliest its
     statements know."), the `creaunix`/`execunix` texts naming the instant.
   - `fix/msg.rs`, `fix/build.rs`, `fix/codec.rs`, `fix/identity.rs`, `fix/ulbridge.rs`,
     `fix/enrich.rs`: every doc sentence naming the two, re-spelled; the precedence code
     untouched (carrier first, else stated `SendingTime(52)`).
   - `text/line.rs`, `text/options.rs`: `EVENT_CAPTURES` holds `sendunix`,
     `DERIVED_EVENT_COLUMNS` holds `transunix`; docs.
   - AGENTS.md: the text-row sentence "when the record was written is `currunix` ..." ->
     "when the operation happened is `transunix` ..., the row header's `mtime` capture ...",
     "a capture named `state`, `creaunix`, `recdunix`, ..." -> `sendunix`; the `graph/` row
     (`element.rs` `Event`: "an instant (`transunix`, ...), a technical wire clock
     (`sendunix`, optional)"), the `isin_registry.rs` row, any other sentence the grep finds.
   - docs: `docs/fix/capture.md`'s dating rule, `docs/graph/serve.md`'s CSV header,
     the schema tables, every page and skill the grep finds - re-spelled, tag literals kept.
   - tests: test function names carrying the words re-spelled; the Iceberg declarations in
     tests (`time_bucket('15 minutes', currunix)`, sort and key lists) re-spelled.
3. The hash sentence: in `rust/tests/fix/store.rs`'s hash test rustdoc, prepend the newest
   `It last moved when` sentence (the design's wording) above the earlier ones, every earlier
   one kept; leave the `left` literal as it is - the lane manager pins the number the red run
   reports. Do not touch the census counts.
4. Run the script on the worktree, then: `git -C /home/user/yggdryl-p4 grep -n -i
   'currunix\|recdunix\|curruuid\|currhashcode'` must list only `refrecdunix` sites, `.handoff/`, the regenerated
   files named above; `rustfmt --edition 2024 --check` on every changed `.rs`; a `git diff
   --stat`. Hand back: the script path, the grep residue with one reason per line, the
   diff stat, the `currhashcode` answer (read `digest_event`/`feed_timed` and the content
   code feed in `graph/element.rs` and `fix/msg.rs` around :3185-3201: do column labels
   enter the content code? state yes/no with the lines; note the four labels now), and the exclusions list. Then
   `git -C /home/user/yggdryl worktree remove --force /home/user/yggdryl-p4`.

## Stage 2 - the lane manager (after S3 AND P3 are landed on the program branch - the markers `$S/s3_landed` and `$S/p3_landed` exist - and the working tree is clean)
1. Under the git lock (`mkdir $S/git.lock` ... `rmdir`): `git fetch`, cut `wip/p4` from the
   program branch head in `/home/user/yggdryl-p4` (remove the stale detached worktree there first:
   `git worktree remove --force /home/user/yggdryl-p4`); run `python3 $S/p4_sweep.py
   /home/user/yggdryl-p4` (`--check` first to preview; `--emit-table` regenerates the bulk table if
   S3's or P3's landing moved an anchor; `$S/p4_define/fmt_gen.py` the layout edits); if a hand
   anchor still differs, re-anchor that one edit and note it.
2. Compile loop in the worktree? No: the workspace shares one target dir; merge `wip/p4` into
   the program branch working tree (fast-forward or merge, no history rewrite), then on the
   program branch: `cargo check -p yggdryl --all-targets --keep-going --message-format=short`
   until clean; `cargo fmt --all`.
3. Regenerate, in this order: `YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl --test
   fix the_committed_store_carries_the_crate_dump`; `cargo test -p yggdryl --test fix
   the_committed_dictionary_hashes_to_one_pinned_value` - red once: pin the `left` it reports
   (the sentence is already in place), re-run green; `YGGDRYL_FIX_EQUIVALENCE_WRITE=1 cargo
   test --locked -p yggdryl --test fix equivalence` and verify with `git diff` that only key
   spellings changed (no value, no digest line) - a value that moved is a defect to find, not
   a snapshot to accept; `npm run --prefix node build:debug`; `node scripts/build_docs_fix.js`;
   `node scripts/build_docs_playground.js`; `python scripts/check_api_inventory.py`.
4. The content-code pin is the `#[test]` `a_message_naming_no_pair_digests_as_it_did_before_detection`
   in `rust/tests/fix/msg.rs` (~:3200): run `cargo test -p yggdryl --test fix
   a_message_naming_no_pair_digests_as_it_did_before_detection`; the define worker read the
   feeds (no label enters any digest), so it must be green; red is a defect, not a re-pin.
5. Phase suites: `cargo test -p yggdryl --test root element column`, `--test graph`, `--test
   fix`, `--test text`, then the chain `$S/logs/chain_p4.sh` (model it on chain_p2.sh: the
   whole run `--all-features --no-fail-fast`, clippy both lanes, rustdoc, Python develop +
   pytest + mypy, Node test:package:debug + npm test, docs manifests, mkdocs strict, docs
   examples rust/python/javascript) writing `$S/logs/chain_p4.log` with `== CHAIN_DONE ==`.
   Known sandbox-only Node failures (`decoding agrees with TextDecoder over the same names`,
   `iso-8859-1 is not a spelling of windows-1252 here`) are not this slice's.
6. Before the commit: `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src python/src
   node/src cli/src` empty; `git grep -n -i 'currunix\|recdunix\|curruuid\|currhashcode' -- . ':!.handoff'` lists
   only the `refrecdunix` sites and the `rust/tests/fix/store.rs` hash rustdoc's history
   sentences (the earlier "It last moved when" lines and the new one, which names the old
   names by design); DESIGN.md gains "### D38" (the design text from `d38_design.md`) and a
   "### P4 results" (checks with exact results, the pins moved with their sentences, the
   currhashcode answer); the handoff `.handoff/next/MARKET_SPLIT_NEXT.md` gains the D38 ledger
   row (done), the state, and under "Questions for the user" the precedence alternative.
7. One commit with `$S/p4_commit_message.txt` (its trailer exact), push `-u origin
   ccr-0fe6f9d0-ruymat`, read the CI run to its `CI result`; a red job is fixed at cause and
   pushed again, never re-run hoping. Report: `Goal`, `State`, `Checks` (command + exact
   result), `Blockers`, `Next`.
