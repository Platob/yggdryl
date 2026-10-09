# Continue the crate split - a prompt for a session without the original environment

Paste this file's path as the first message of the new session: "Continue the yggdryl crate split
from `.handoff/next/MARKET_SPLIT_CONTINUE.md`. Program branch: `ccr-0fe6f9d0-ruymat`."

## What this is
The split of `yggdryl` into crates (`.handoff/next/MARKET_SPLIT_PROMPT.md` is the program's prompt
and its hard rules; `.handoff/split/DESIGN.md` the design ledger and every slice's results;
`.handoff/next/MARKET_SPLIT_NEXT.md` the live state - `State`, `Checks`, `Next`, the questions -
which every lane's results commit updates, so read it first to learn what has landed since this
file was written). Work happens on the program branch `ccr-0fe6f9d0-ruymat` under the draft PR
#209 of `Platob/yggdryl`; one commit per slice, one push per lane, CI read to its `CI result` job,
a red job fixed at cause. Never publish, never push to `main`, no tag, no release run, never mark
the PR ready or merge it; never touch AWS; no back-compat alias or shim; no test code under any
`src/`; no model identifier anywhere in the tree; pushed history is never rewritten; a cost pin is
never re-pinned upward; Node gains no door; every commit ends with exactly
`Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and
`Claude-Session: https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp` and no other
Co-Authored-By.

## The former scratchpad
`.handoff/split/scratch/` is a copy of the previous environment's scratchpad: the move scripts, the
designs (D36-D39), the decisions (`p5_decisions.md`, `user_decisions.md`), the lane log and every
decision taken after a define (`regroup.md` - read it whole, its "answers" sections bind the lanes),
the define reports with their residue, the lane briefs (`*_manager_prompt.md`), the commit
messages, the chain scripts and the parked P5 patch. Every script reads its folder from `$S`, and
writes residue files into it, so copy it somewhere writable first:
`cp -r .handoff/split/scratch /tmp/s && export S=/tmp/s`. The `$S/<lane>_landed` marker files the
briefs gate on do not exist in a new environment: the gate is "the previous lane's results commit
is on the branch" (`git log --oneline origin/ccr-0fe6f9d0-ruymat | head`), and a lane writes its own
marker when it finishes so the next brief's `until` loop still works.

## What had landed when this was written
P0, P1, S0, S1, S2, S2b, the CI restructure, D34+D35, P2, S3, P3 (the storage-backend extension
point built in place) - every one with CI green and recorded - and P4 (`transunix`, `sendunix`,
`uuid`, `hashcode`; commit 4e46b5ab7), pushed at the end of the previous session with its CI run
NOT yet read: its chain had proven `cargo fmt`, the all-features whole run (63 targets, 9379
passed), clippy all features and `cargo doc` before the session ran out, and its phase suites
(fix, graph, text, root in both lanes, pytest 640, mypy, Node 401, mkdocs) were green before the
commit; the rustdoc tests, the default-features run, the CLI tests, the whole Python and Node
pre-push blocks and the docs runners were not re-run by the chain. Read MARKET_SPLIT_NEXT.md's
`State` for anything landed after.

## First steps of the new session
1. Read P4's CI run on 4e46b5ab7 (the GitHub tools; the run of the push, its `CI result`). A red
   job is reproduced narrowly, fixed at cause in a commit of its own, pushed, read again - before any
   lane starts. The dictionary hash moved once in P4 (14542711836201211247 -> 12613356107921639431,
   its sentence last in `rust/tests/fix/store.rs`); the snapshot's keys were re-spelled, no value.
2. **P6** - delete the market book service (the user's instruction): `$S/p6_contract.md`,
   `$S/p6_commit_message.txt`; its own commit, before S4.
3. **S4** - `yggdryl-market` and `yggdryl-fix`: the brief `$S/m46_manager_prompt.md` (scoped to S4;
   fix `s4_move.py`'s two defects named in `$S/s6_avro_report.md` finding 4 first), the script
   `$S/s4_move.py`, the nine decisions in `$S/regroup.md` "S4 decisions" (decision 7 reversed:
   P5 is not landed, see "Re-ordered on the user's instruction"), the pins rule of "Excel define
   done - answers" item 1, `$S/s4_report.md`, `$S/s4/residue.md`, `$S/s4_commit_message.txt`. P6
   and S4 go in one push; the user asked for market and fix first.
Then the lanes below in order.

## The remaining lanes, in order, each with its brief
4. **P5R** - MarketMessage (D37) finished inside the crates: `$S/p5r_manager_prompt.md`. The parked
   work is `$S/p5_on_p4.patch` (the diff of the previous environment's `wip/p5-impl` over P4's
   4e46b5ab7; `$S/p5_on_p4.commits` names it) - where the brief says `git diff 4e46b5ab7
   wip/p5-impl`, read that file instead. `$S/p5_state.md` says what is done and what remains; C1
   and C2 of `$S/p5_decisions.md` are the unfinished design work; its last run gave graph 411/411 and
   fix 905/906, the one failure the native round trip (an unresolved entry left in the metadata is
   not written on the rendered wire). Commit message
   `$S/p5_commit_message.txt`.
5. **M6** - the media moves: `$S/m6_manager_prompt.md` - excel and xmla each a commit under one
   push, then avro, parquet, s3 and iceberg as one dependency-closed commit (the user's decision).
   Scripts `$S/s6_<crate>_move.py`, reports `$S/s6_<crate>_report.md`, commit messages
   `$S/s6_<crate>_commit_message.txt`. The scripts were written on trees without P5, P6 and S4:
   an anchor that no longer matches is fixed in the script, re-derived from the tree it runs on,
   never at the site by hand.
6. **B5** - the market binding packages and the release learning every crate (S5):
   `$S/b5_manager_prompt.md`, `$S/s5_move.py`, `$S/s5_define_report.md`, `$S/s5_commit_message.txt`.
7. **S7** (`RecordOptions` -> `MediaOptions` over every crate) and **S8** (the expression series):
   designed in DESIGN.md, no brief yet - design in the foreground first, as every slice was.
8. **S9**, the final sweep, on the user's go (MARKET_SPLIT_PROMPT.md).

## How a lane runs
A lane manager (a background agent on the tier below the foreground's) follows its brief: the
script on the program branch head, the decisions, the compile loop (`CARGO_INCREMENTAL=0
CARGO_PROFILE_DEV_DEBUG=0 cargo check --workspace --all-targets --all-features --keep-going
--message-format=short`, the first run without `--locked` when a member is added), the
regenerations in dependency order, the phase suites with every hash, order and request-count pin
green WITHOUT edit, the commit with its message file verbatim, one chain over the tree (the
all-features whole run leading - it needs about 13 GB of free disk - clippy both lanes, `cargo doc
-D warnings`, the rustdoc examples, Python's and Node's pre-push blocks, the three docs example
runners, the inventories, the internals), one push, CI read through the GitHub tools to
`CI result`, the results tables into DESIGN.md and MARKET_SPLIT_NEXT.md as a results commit, pushed
and read. At most two workers at once on disjoint file sets; nobody runs `cargo fmt --all` while
workers edit; one target directory. The foreground designs and decides; it does not edit by hand
what a script can own.

## Open with the user
The release rehearsal (a manual `release.yml` run publishes nothing but is a release run: the
user's explicit go) and the registry configuration a real publish needs (a PyPI pending trusted
publisher for `yggdryl-market`, an npm bootstrap publish before trusted publishing, a
`CARGO_REGISTRY_TOKEN` with `publish-new` over `yggdryl-*`). Everything else the briefs asked the
user is decided in `$S/user_decisions.md`.
