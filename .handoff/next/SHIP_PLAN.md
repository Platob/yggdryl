# Ship plan for 0.1.22 - the remaining lanes, compacted (2026-10-10)

The user: "Compact all remaining steps to implement it the fastest and ship it with parallel
workers". The lanes and their decisions are in `.handoff/split/scratch/user_decisions.md` (6-20);
this file only orders them for speed. A wave's workers run at once in the one working tree on
disjoint file sets; nobody but the integrator edits `.api-inventory.txt`, `.api-bindings.txt`,
`AGENTS.md`, `mkdocs.yml` or runs `cargo fmt --all`. One push per wave; CI is read at its head
(a push cancels the run before it, so intermediate commits are proven by the head's run).

| Wave | Workers | Files (owner) | Gate |
| --- | --- | --- | --- |
| 0 | P9 (fix agent, then commit) | the P9 tree | medallion green |
| 1 | A: P11 - `Scheme::DORIS` (decision 20) | `rust/src/{scheme,compatibility}.rs`, `rust/tests/root/{scheme,compatibility}.rs`, the Python scheme door, `docs/types/**` scheme page | P9 pushed |
| 1 | B: P9b + P10 (decisions 16-19, design `scratch/p10/{p9b_design,d44_design}.md`) | `rust/market/src/{instrument,characteristics,listing}.rs` + `instrument/**`, `rust/market/src/graph/{book,market}.rs`, `rust/fix/src/{msg,enrich,forex,codec}.rs`, their tests, `python/src/instrument.rs`, `python/tests/{test_instrument,test_fix,medallion}.py` (the Doris line included), `docs/graph/{instrument,book}.md` | P9 pushed; D44 written |
| 1 | C: P8 (decisions 12-13, design `scratch/p8/d41_design.md`) | `rust/market/src/graph/iterator.rs`, `rust/market/src/idtype.rs`, `rust/market/tests/graph/iterator.rs`, `rust/market/tests/root/idtype.rs`, `docs/fix/lifecycle.md` | P9 pushed |
| 2 | P5R (D37, `scratch/p5r/p5r_plan.md`) | the re-mapped patch | wave 1 green |
| 3 | P7 (D40, `scratch/p7/p7_plan.md`): the sweep then D40.1/2/5/6 - last, so every name moves once | the sweep's sites | P5R green |
| 4 | Integrator: inventories, AGENTS.md, mkdocs, the three docs runners, `cargo fmt --all`, commits per lane by file set, push, CI read | - | each wave's end |
| 5 | The user: `LIVE_AWS_TEST_PROMPT.md` on their machine, then the merge of PR #209 (publishes 0.1.22) | - | wave 3 green |

P5R moved before P7 (the user's order was P7, P8, P5R): applied first, its code is renamed by P7's
one sweep instead of being re-targeted twice.
