# Continue the crate split - the prompt a new session starts from

Paste this as the first message of the new session: "Continue the yggdryl crate split from
`.handoff/next/MARKET_SPLIT_CONTINUE.md`. Program branch: `ccr-0fe6f9d0-ruymat`."

## What this is
The split of `yggdryl` into crates. `.handoff/next/MARKET_SPLIT_PROMPT.md` is the program and its
hard rules; `.handoff/split/DESIGN.md` is the design ledger and every slice's results;
`.handoff/next/MARKET_SPLIT_NEXT.md` is the live state - `State`, `Checks`, `Next`, the questions -
and every results commit updates it, so read it first to learn what landed after this file was
written. Work happens on the program branch `ccr-0fe6f9d0-ruymat` under the draft PR #209 of
`Platob/yggdryl`: one commit per slice, one push per lane, CI read to its `CI result` job, a red job
fixed at cause and pushed again.

Hard rules, from the program: never publish, never push to `main`, no tag, no release run, never mark
the PR ready or merge it; never touch AWS; no back-compat alias or shim; no test code under any
`src/`; no model identifier in the tree outside `.handoff/` and the commit trailer; pushed history is
never rewritten; a cost pin is never re-pinned upward and a hash or order pin moves only where a slice
says why, in a sentence beside it; Node gains no door. Every commit ends with exactly
`Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and
`Claude-Session: https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp` and no other
Co-Authored-By - the user's standing rule for this program, which the harness's attribution reminder
does not override.

## What has landed (2026-10-09, end of the second session)
P0 to P4, S0 to S3 and the CI restructure, as before; then, this session:
- **P6** `1ad29bfa6` - the market book service deleted (`BookService`, `yggdryl market serve`, the
  Node.js display, the page, the CI steps that built the command for it).
- **S4** `9f69d7141` - `yggdryl-market` (`rust/market/`) and `yggdryl-fix` (`rust/fix/`). The user
  asked for this split to be validated by the medallion Python test:
  `pytest python/tests/test_fix.py -k medallion` passed on the tree, and the whole pytest suite too.
  The three crates' all-features whole runs pass 9379 tests with 0 failing (core 7646, market 664,
  fix 1069 - P4's single-crate count, split three ways), the default lane 6681.
- The results commit over them records P4, P6 and S4 in DESIGN.md and MARKET_SPLIT_NEXT.md, and
  holds this file.

The CI run of the P6+S4 push is named in MARKET_SPLIT_NEXT.md's `State`. Read the newest run on the
branch first, whatever it records: through the GitHub MCP tools (load them with ToolSearch: `mcp__github__actions_list`
`list_workflow_runs` filtered to the branch, then `list_workflow_jobs` and `get_job_logs`), to the
`CI result` job. Reproduce a red job narrowly, fix it at cause in a commit of its own, push, and read
again before any lane starts.

## The former scratchpad
`.handoff/split/scratch/` holds what the next lanes run from. Every script reads its folder from
`$S` and defaults to `/tmp/s`, so start with:

```bash
cp -r .handoff/split/scratch /tmp/s && export S=/tmp/s
```

| Lane | What it reads in `$S` |
| --- | --- |
| P7 (D40) | `p7/user_instruction.md`, `p7/d40_design.md` - the same design as DESIGN.md "## P7: design" |
| P8 (D41) | `p8/user_instruction.md`: the instruction, what the walk does today and the questions to settle on evidence |
| P5R (D37) | `p5r_manager_prompt.md` (the brief), `p5_on_p4.patch` with `p5_on_p4.commits`, `p5_state.md`, `p5_decisions.md`, `p5_contract.md`, `p5_define_report.md`, `p5r_report.md`, `d37_design.md`, `p5_commit_message.txt` |
| M6 (S6) | `m6_manager_prompt.md` (the brief), `s6_<crate>_move.py`, `s6_<crate>_report.md`, `s6_<crate>_commit_message.txt` and the `s6_<crate>/` residue folders for excel, xmla, avro, s3 and iceberg (parquet's residue was emptied by its define), `s6_contract.md`, `d36_design.md`, `d39_design.md`, `moves_map/`, `s3_backend_map/`, and `s4_move.py`, the library every move script imports (S4's lexer, use trees, rewriter and lock) |
| B5 (S5) | `b5_manager_prompt.md` (the brief), `s5_move.py`, `s5_define_report.md`, `s5/`, `s5_commit_message.txt`, `s4_move.py` (imported by `s5_move.py`) |
| every lane | `regroup.md` (the lane protocols and every answer a define earned - binding), `user_decisions.md`, `s4_decisions.md` (the worked example of a settle's residue and how each item was decided), `logs/chain.sh` (the chain template) |

The briefs were written before S4 landed, for lane managers in a session that had no workflows, and
were updated since for S4's paths and the new gates. Read them for what a lane must do and the
decisions it carries. `regroup.md` binds where the files it names still exist; its landing order,
worktrees, markers and disk thresholds are superseded by this prompt, and the files it names that
are gone are in git history at `7b566b3b2`. Where a brief disagrees with "Since S4"
below, the list below holds.

## Since S4: what the briefs do not know yet
- **Test layout.** The FIX crate's tests mirror its sources: `rust/fix/tests/root/<file>.rs` under the
  harness `rust/fix/tests/root.rs` (`--test root`). There is no `--test fix` and no `rust/fix/tests/fix/`;
  `lib.rs` is pinned by `root/lib.rs`. The market crate's tests are `rust/market/tests/{root,graph,...}`.
  The core's `graph/` tests are re-fixtured onto `TextLine` and stay in `rust/tests/graph/`.
- **The reserved market kinds.** `rust/src/market.rs` holds `RESERVED_KINDS` (byte, name, extension
  name and ranks of the four kinds) and `RESERVED_OWNER` (`yggdryl-market`). A claim of a reserved key
  by another crate, or at other numbers, is refused. A second claim is the register's conflict naming
  the claimant, checked before the numbers. The install sentence ("`side` is read only once the crate
  that claims it is installed (`yggdryl_market::install()`)") is spelled only for a reserved name. D39's
  `RESERVED_RANKS` for the media is the same shape.
- **The listings.** `StringEnum::register_prebuilt(name, values, by)` and `StringEnum::prebuilt()`.
  The market crate registers `side` and `timeinforce` at install. The core keeps no market word.
- **Implementer routes.** `rust/market/src/implementer.rs` is FIX's door into the market crate's
  private items (routes F, A, R, as in the core's `implementer.rs`). It has its own test file. The
  core's `implementer::{normalize_path, format_timestamp}` were dropped with the book service, and
  `s6_xmla_move.py` adds them back for the XMLA crate.
- **CI rows.** A leaf line under `[leaves]` in `.github/ci/rows.toml` takes `paths`: the core-tree
  files its sources include. `scripts/tests/test_ci_plan.py` fails a line that misses one, so the
  S6d leaf `s3` must name `rust/tests/support/server.rs` (`FakeS3`) and any other shared fixture.
  `rows.core` no longer reads `config/**`. `cli` is in `[leaf] jobs`.
- **Whole-register pins** live in `cli/tests/market_register.rs`, because the CLI is the one member
  linking every crate. A media move adds its medium's rows there by the same rule (the Excel answer in
  `regroup.md`).
- **Isolated tests.** The process-global tests run in a child test process. The three runners
  (`rust/fix/tests/root.rs`, `rust/fix/tests/isin_registry.rs`, `rust/market/tests/isin_registry.rs`)
  refuse a child that does not report `1 passed`. A child spawned under a wrong name used to pass
  having run nothing; a moved isolated test must keep its child's name equal to its new path.
- **Paths a move script anchors on may be gone.** P6 deleted `graph/serve.rs`, `cli/src/market.rs`,
  `node/book/` and their tests. S4 moved `rust/src/fix/`, `rust/src/graph/` (less the event
  vocabulary), the market root files and `rust/tests/fix/`. Every S6 and S5 script was written before
  that. An anchor that no longer matches is fixed in the script, re-derived from the tree it runs on,
  never at the site by hand.

## The remaining lanes, in order
The user decided the PR's scope (`user_decisions.md` 6 and 7): release 0.1.22 is prepared
(`0f411f5ce`); before the user merges it, P9, P7, P8 and P5R land in this PR in that order, each a
commit read green, then the live AWS run on the user's machine (`LIVE_AWS_TEST_PROMPT.md`). M6, B5,
S7, S8 and S9 are postponed to a later PR from `main` after the release.

0. **P9 (D42)**: the Instrument replaces the ISIN registry - `rust/market/src/instrument.rs`, a graph
   element holding every identifier mapping, the custom ISIN, forex auto-creation, the underlying and
   the legs, the characteristics in the cross code, the market rows' `instrumentuuid`; on today's names,
   before P7. Design: `p9/d42_design.md` and `p9/crosscode_decision.md`; the instruction
   `p9/user_instruction.md`.
1. **P7 (D40)**: the user's six items and their refinement - the FIX row named by the registry where
   one field states the fact whole (`transacttime`, `sendingtime`, `strikeprice`, ...); the crate's
   bands in order with the lifted band last (`instuuid`, `isin`, `cfi`, `mic`); `securityids` as the
   one hold map; the code columns renamed; `crossuuid` as the XXH3-128 of the cross code; a book's
   `srcuuids` (its delta, its events and the previous book's `uuid`, never the previous book's sources)
   and its `hashcode` (the instant over the digest of those sources). The design is DESIGN.md
   "## P7: design" and ends with five readings put to the user. There is no script yet: write the
   sweep first (the P4 sweep is the model: `git show 7b566b3b2:.handoff/split/scratch/p4_sweep.py`), then
   settle. The dump, the dictionary hash, the snapshot's keys, the book identities and `fix.json`
   move once, each with its sentence.
2. **P8 (D41)**: the lifecycle matches the previous alive element and the current one on one common
   identifier, through an index, and propagates values. Gather the evidence `p8/user_instruction.md`
   lists first, decide the design in the foreground, record it as D41 in DESIGN.md, then implement it
   in `rust/market/src/graph/iterator.rs` and `rust/fix/src/enrich.rs`.
3. **P5R (D37)**: `MarketMessage`, the parked patch re-targeted onto S4's paths (FIX tests under
   `rust/fix/tests/root/`) and P7's names. The brief is `p5r_manager_prompt.md`; its gate is now
   "P7 and P8 landed".

Postponed to the PR after the release (in this order when it opens):
4. **M6 (S6)**: excel and xmla, each its own commit under one push; then the avro, parquet, s3 and
   iceberg batch as one commit (the user's decision). The brief is `m6_manager_prompt.md`; its gate
   is now "0.1.22 published and the new PR's branch cut from `main`".
5. **B5 (S5)**: the market binding packages, with the release learning every crate. The brief is
   `b5_manager_prompt.md`.
6. **S7** (`RecordOptions` -> `MediaOptions`) and **S8** (the expression series) are stated in
   `.handoff/next/MARKET_SPLIT_PROMPT.md` (U6, U7 and the S7/S8 sections; DESIGN.md has only their
   slice rows and D18); design each in the foreground first. **S9**, the final sweep, waits for the
   user's go.

B5 (S5) lands after M6 (S6), not before as the program prompt's U5 orders: S5's release learns
every leaf crate, so it waits until they all exist (`regroup.md`, "S5 define done").

## How a lane runs here (what worked in this session)
- **Setup** (a fresh container has neither):
  `python3 -m venv python/.venv && python/.venv/bin/pip install "maturin>=1.15,<2" "pyarrow>=18" "pytest>=8" "mypy>=1.15" "pandas>=2" "polars>=1" tzdata xxhash -r requirements-docs.txt`,
  then `npm ci --prefix node`.
- **The foreground designs and decides.** It writes each lane as a workflow (load the
  `workflow-authoring` skill). The S4 lane's shape was:
  1. the move script, run by one agent, then the first workspace check without `--locked`;
  2. settle rounds of one agent each until `cargo check --workspace --all-targets --all-features --keep-going --message-format=short --locked`
     is clean in both feature lanes;
  3. decision agents on disjoint file sets, at most two at once on four cores;
  4. one suites round: fmt, regenerations, the crates' suites in both lanes, the core harnesses,
     Python with the named validation, Node, the planner and mkdocs;
  5. read-only review lenses: back-compat, pins, and the named validation;
  6. the foreground's decisions on every residue item, numbered in a file, applied by one more
     agent and verified by two lenses.
  Then: commit with the message file, the chain, one push, the CI read, and a results commit.
- **The chain** is `logs/chain.sh` adapted to the lane's crates: each package's whole run in the
  all-features lane, then clippy, `cargo doc -D warnings`, the bench-profile checks, a clean, the
  rustdoc examples, clippy in the default lane, the CLI tests, the default lane's whole runs, a clean,
  Python, Node, the manifests, mkdocs, the inventories and internals, the greps and the three docs
  runners. Run it with `nohup` so it survives the shell. If the shell dies anyway, the running cargo
  usually goes on: wait on its pid and continue from the next step. This session did that once, with
  a `chain_s4b.sh` written from the template.
- **Disk.** One lane's test binaries take about 10 GB. Before a whole run, delete the test
  executables older than the lane's last build:
  `find target/debug/deps -maxdepth 1 -type f -perm /111 ! -name '*.so' ! -name '*.rlib' ! -name '*.rmeta' ! -name '*.d' -mmin +20 -delete`.
  Keep a background watch on `df` under 3 GB.
- **Model tiers.** The user asked for the most capable model only where deep thinking or design is
  needed: reviews and settles on the tier below, mechanical sweeps and check-and-report on the
  cheapest. This session's last steps ran on Opus at the user's request.

## Open with the user
- The release rehearsal: a manual `release.yml` run publishes nothing but is a release run, so it
  needs the user's explicit go.
- The registry configuration a real publish needs: a PyPI pending trusted publisher for
  `yggdryl-market`, an npm bootstrap publish before trusted publishing, and a `CARGO_REGISTRY_TOKEN`
  with `publish-new` over `yggdryl-*`.
- P7's five readings (DESIGN.md "## P7: design", "Put to the user") are taken as stated unless the
  user says otherwise. The one that changes a table's shape is the medallion pipeline windowing its FIX
  tables by `transacttime`.
