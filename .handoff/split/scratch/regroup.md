# Regrouped lanes (2026-10-09 10:15 UTC) - deliver faster, same rules

Machine: 4 cores, 15 GB RAM, ~17 GB disk free, one shared `target/` (9.5 GB). The bottleneck is
compiling and chaining on one tree, one slice at a time. The regroup: (1) define work for every
remaining slice runs now, in parallel, with no cargo; (2) a lane lands several slices as several
commits under ONE chain and ONE push/CI read - each commit a tree that builds (its check and its
phase suites run before it is made), the chain over the lane's final tree; (3) settle loops run
as `cargo check` in the lane's worktree under its own check-only target directory, so a lane
settles while another chains; only tests and chains touch `/home/user/yggdryl/target`.

## Protocols
- Git: `mkdir $S/git.lock` ... `rmdir $S/git.lock` around every git step that touches `.git`
  (worktree add/remove, checkout, merge, commit, push, fetch).
- Cargo on the main target (`/home/user/yggdryl`, tests, benches, chains): `mkdir $S/cargo.lock`
  ... `rmdir` around the whole step; wait for it, never start a second.
- Cargo check in a worktree: `CARGO_TARGET_DIR=/home/user/target-<lane> CARGO_PROFILE_DEV_DEBUG=0
  CARGO_INCREMENTAL=0 cargo check -p yggdryl --all-targets --keep-going --message-format=short`
  (and `--workspace` for bindings' call sites) in `/home/user/yggdryl-<lane>`; `cargo check` only -
  a `cargo test` there would build a second full target. At most ONE such check runs while a chain
  runs on the main target (4 cores). Remove the directory when the lane lands.
- Disk: before a chain, `df -h /home/user`; under 6 GB free, remove `target/debug/incremental`
  and the finished lanes' check targets first.

## Lanes and their order of landing on the program branch
| lane | slices (commits) | define now | lands after |
| --- | --- | --- | --- |
| S3 | S3 | done (uncompiled in wip/s3) | P2 (in flight) |
| P45 | P4 (D38), P5 (D37) | P4 sweep (worker running); P5 workers in wip/p5 once the sweep exists | S3 |
| P3S | P3 (D36 in place), S6d (`yggdryl-s3` move) | P3 workers (running); S6d move script after P3's define | S3; its chain after P45's if both are ready |
| M46 | S4 (`yggdryl-market`, `yggdryl-fix`), then S6a-c (`avro`, `parquet`, `excel`, `xmla` - moves only, dependency order) | S4 move script + manifests (worker); S6 reach map (readers) then move scripts | P45 (S4), P3S (iceberg needs `yggdryl-s3`: `iceberg`+`s3tables` is the last commit of this lane, after S6d) |
| B5 | S5 (the `yggdryl-market` Python and npm packages; release/ci learn them) | packaging draft (worker) in wip/s5 | S4 |
| R7 | S7 (`RecordOptions` -> `MediaOptions` sweep over every crate) | the sweep script, anchors re-run at landing | every move |
| X8 | S8a, S8b | design in the foreground after S7 | S7 |

Every commit keeps its own message file under the scratchpad and the exact trailer. CI is read
once per push (`CI result`); a red job is fixed at cause and pushed again.

## Budget (added 11:15 UTC)
At ~10:50 UTC every running agent (P2 manager, S3 manager, P4 define, S4/S5 define, four P3
workers + reconciler, three media readers) was killed by the session's rate limit ("session
limit, resets 11:10 UTC"). Relaunch order from 11:12: P2 manager (resume from its log), S3 manager
(waits for `$S/p2_landed`), P4 define (finish `p4_sweep.py`), the three media readers (sonnet, from
the workflow's cache); then, as each completes: P3 W2-W5 + reconciler, S4 define, S5 define. Keep
at most ~4 opus agents live at once; sonnet for readers and mechanical sweeps. A lane manager
writes a `$S/<lane>_landed` marker after its push so the next lane can wait on a file, not a chat.

## Commit trailer (added 11:33 UTC)
Every commit of this program ends with exactly the two lines of its slice's message file:
`Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and `Claude-Session:
https://claude.ai/code/session_01Gfky7FUx35U5i4UJcrQGKp`. That is the user's standing rule and it
overrides the harness's default attribution reminder; a lane manager uses the message file
verbatim and adds no other Co-Authored-By line. Known deviation, kept (pushed history is never
rewritten): P2's commit f7c3c1b58 carries the harness's default co-author line.

## Landed
- P2 f7c3c1b58 (proven green by run 37923561993 on 49bcab3e3, the handoff-only commit over it).

## P45 inputs (added 11:48 UTC)
- `wip/s3` now carries b8ddef717 "wip: S3 settled in its worktree (check and clippy clean)".
- `$S/p4_sweep.py <tree>` (define report `$S/p4_define_report.md`): on the settled S3 tree it changes
  186 files with 0 refusals and is idempotent; `--check` previews; `--emit-table` regenerates the
  bulk table if S3's final settle moves an anchor. Residue by design: `refrecdunix` (retired FIX
  field) and the hash rustdoc's history sentences in `rust/tests/fix/store.rs`. The content-code
  pin (`rust/tests/fix/msg.rs` `a_message_naming_no_pair_digests_as_it_did_before_detection`) must
  stay green: no label enters any digest. `hashcode` is tag 65_004 (the design said 65_002; the
  record script fixes DESIGN.md's text).
- `wip/p5` = 53e7d3c21: that settled tree + the sweep, the base for P5's define workers.

## Landing order after S3 (decided 11:58 UTC)
P3 lands first (its define is complete: 54 files in wip/p3, settled in `/home/user/target-p3`
while S3 chains; the P3S manager squash-merges onto the program branch once `$S/s3_landed`
exists), then P4 (the sweep re-run on the new head; idempotent anchors), then P5 (wip/p5's define
re-based), as lane P45 under one chain. `$S/p3_landed` is the marker the P45 manager waits on.

## Landed (continued, 13:06 UTC)
- S3 a46a2177b "Settle the remaining seams before the crates leave" (run 37932670478 green, 33 jobs);
  `$S/s3_landed` written; `wip/s3` kept at b8ddef717. Rule learned: push once per lane - a push to
  the PR branch cancels the in-flight run (concurrency group), so P2's run 37923405641 read as
  "failure" only because 49bcab3e3 cancelled it.
- Worktrees p4, s4 and s5 sit on 0a24f1806 (the uncompiled wip/s3). Any lane that settles from one
  of them merges a46a2177b first (S3's pre-settle and review edits).

## S4 define done (13:03 UTC)
`$S/s4_move.py <tree>` (report `$S/s4_report.md`, residue `$S/s4/residue.md`, 28 items). Runs with
no cargo on the program branch head after P4 and P5 land; moves by glob so P5's `graph/message.rs`
and `graph/anomaly.rs` travel with the market graph. Left by hand for the M46 manager: first
`cargo check` without `--locked` (Cargo.lock), `cargo fmt --all` once, AGENTS.md per-crate sections
(D13), `market_register.rs:202` claimant `yggdryl-market`, stale refusal text `market.rs:560`,
eight `pub(crate)` forwarders FIX needs (listed in the residue) and ~22 E0624 candidates the
compiler confirms - most of which D37's public message removes once P5 lands first.

## S4 decisions (foreground, 13:08 UTC) - binding on the M46 manager
1. Cargo.lock: the first `cargo check` runs without `--locked`; confirm `cargo package --list -p
   yggdryl` honours `exclude = ["/market", "/fix"]`.
2. `StringEnum::SIDES`/`TIMESINFORCE` and their `PREBUILT` rows move to `yggdryl-market` with the
   kinds they list (one owner per fact); `from_logical_name("side")` resolves after
   `yggdryl_market::install()` and the refusal names the crate to install, as D7/D39 say. The
   core keeps no market word.
3. Re-fixture, never move, a core test that uses a kind only as a fixture (D14): the core's
   `graph/{element,column,element_column}` tests re-fixture onto `TextLine` (the core's own Event)
   and stay, so every core source file keeps its test file; `root/cast`, `xxhash/arrow`,
   `xmla/dbtype`, `valuestream`'s round trip and the other candidates re-fixture onto `State`, a
   code or a string leaf. Only a test pinning a market fact moves.
4. AGENTS.md's D13 per-crate sections and `docs/contributing.md` are written in the S4 commit.
5. `rust/examples/fix_*.rs` are deleted (AGENTS.md: examples live in docs, no `examples/` dir); a
   page that pointed at one points at its own example block instead; confirm no CI row ran them.
6. Features: forward `parquet`/`iceberg` to a binding only where the compiler names a market or
   FIX surface under them.
7. `OperationEventFacts` stays private to the market crate: P5 lands before S4, so FIX holds a
   `MarketMessage` and constructs no facts holder; the script's raise to `pub` is dropped.
8. D7 confirmed: a pure-Rust caller installs what it links; no lazy install in any entry.
9. `serde_json`: the compiler says; a runtime use makes it a dependency, nothing speculative.

## S6 define (13:10 UTC)
S6 define workers cut their worktrees from the program branch head a46a2177b, not from wip/s3 as
the contract first said (S3 is landed and settled there). Launched: avro (`/home/user/yggdryl-s6-avro`,
`wip/s6-avro`). parquet, excel and xmla follow as P5's define workers finish; iceberg waits for S6d's
define (D36) and runs after P3 lands.

## P5 define done (13:55 UTC) and decided (14:05 UTC)
`$S/p5_define_report.md` (reconciler): 97 files + 10 new, 2 deleted, uncommitted in wip/p5; no cargo.
Five conflicts decided in `$S/p5_decisions.md` (C1 idmap rule held with the arrival sequence on the
handle; C2 wire-read facts are stated; C3 the FixMsg market_data wrappers retire; C4 finalize keeps a
held hashcode and the handle's fill door settles; C5 projection losses documented). The P45 manager
implements them in `fix/` (two opus workers at most) before P5's smoke loop; `p45_manager_prompt.md`
updated. Gate for P45: `$S/p3_landed` (P3S still settling).

## S6 order amended (foreground, 14:20 UTC) - binding on the M46 manager
The avro define (`$S/s6_avro_report.md`) proved D39's landing order wrong: the core's `iceberg/manifest.rs`
names Avro at 25 lines and `s3tables/` names `s3::located_with`/`S3Options`, so a tree where avro,
parquet or s3 has left while Iceberg stays in the core is a dependency cycle and does not build. The
rule is DESIGN.md "## S6's shape": one commit in dependency order for the closed batch, now avro,
parquet, s3 (S6d) and iceberg (+ its `s3tables` feature on `yggdryl-s3`), each crate's define script
run in that order on one tree, one settle, one chain, one push; the `iceberg` leaf line
`after = ["avro", "parquet", "s3"]`. excel (S6b) and xmla (S6c) stay their own commits and land
first, right after S4, since nothing in the core reads them. The reversed split (iceberg first,
depending on the core's `avro`/`parquet`/`s3` features, then the three one by one re-pointing the
iceberg crate) is buildable too and is the user's call - put to them in the final reply, not taken.
Landing order after P45: S4 -> excel -> xmla -> [avro+parquet+s3+iceberg] -> S5 -> S7 -> S8.
Avro's open questions answered: cross-medium tests stay moved whole with the crate whose objects they
build (D39), split back by hand only where a core-only fact has no other core pin; a moved test that
builds an Iceberg object belongs to the iceberg crate, so no crate forwards `iceberg` for its tests
(dev-dependency cycles avoided); the docs runner compiles under `cli/` after S4, which lands first;
`FileThreads` raised inside the private `media::options` under route R is acceptable (reaches nobody
but the implementer path). S4 script defects the avro worker found (trait imports used only for
method calls pruned; items a macro call defines lost in a split) are fixed in `s4_move.py` by the M46
manager before it runs, porting the avro script's workaround.

## 14:30 UTC
P3 pushed as ef4e241a4 (CI run 37942794084, suites completed on the head; the P3S manager reads it and
writes `$S/p3_landed`). The P45 lane manager is launched and waits on that marker. Define workers
live: parquet, excel, xmla (avro done). Next defines when slots free: s3 (S6d, from
`$S/d36_design.md` and `$S/s3_backend_map/`, the registered-in-place tree at ef4e241a4) and iceberg
(after the s3 and parquet scripts exist, since it reads their implementer doors). Stale worktrees p2,
p4, s5 removed; disk 15 GB free, the main target 11 GB.

## Landed: P3 ef4e241a4 (14:35 UTC)
"Register the object-store backend on the holder in place" - run 37942794084 green (33 jobs passed,
the empty Leaf matrix skipped as planned), docs run 37942794102 green; `$S/p3_landed` written;
worktree p3 and target-p3 removed, `wip/p3` kept. No request-count pin edited or moved. Caveats for
later lanes: an all-features whole run grows the main target by ~12 GB, so a chaining lane needs 13 GB
free first; D36.5 (the 58 implementer items and the 23 `cfg(feature = "s3")` sites in aws/, auth/,
http/, xml/) is S6d's, and S6d also adds `yggdryl_s3` to the logging facade's CRATES table.

## Parquet define done (14:40 UTC) - answers
`$S/s6_parquet_move.py`, report, commit message; 113 files; composes after the avro script and after
S4. Answers: (1) one commit with Iceberg (the amended order above); (2) `release.yml`'s publish list
and `preflight` learn every leaf crate in S5 (lane B5), whose scope is every crate that exists by
then, not market/fix alone; (3) in the batch the core keeps neither `parquet` nor `iceberg` feature -
the iceberg script deletes both lines, the "until Iceberg moves" branch of the parquet script is moot
and the manager removes it; (4) `dead_code` from copied helpers: the compiler names them, the M46
manager deletes them; (5) the s3 script runs after avro's and parquet's on the batch tree and must
carry `rust/parquet/tests/s3/*` and `rust/parquet/benchmarks/parquet/s3.rs`; (6) the stale
`scripts/docs_core_remaining.js:41` and `scripts/bench_avro_baseline.py:36,108` are re-spelled in the
batch commit.

## Excel define done (14:50 UTC) - answers, one earlier answer revised
`$S/s6_excel_move.py` (88 files; composes after S4 with no residue). Answers, binding on M46:
1. Cross-medium tests split by medium - this REVISES the avro answer above. The s2 pins and the
   media-register tests pin every medium's hash and order in one body; moved whole to the first
   leaving crate they would make its tests depend on every other medium crate as each leaves (a
   dev-dependency web). Rule: each crate's tests pin its own medium's rows (its `RecordOptions` hash,
   rank, options) and the core keeps the core media's rows (IPC, CSV/TSV, text); the whole-register
   pins (`the_media_order_by_their_rank`, the register's expected order, the backend and format
   registers' full listings) move to `cli/tests/` as their own target - the CLI is the one member
   linking every crate - with a `//!` line naming the cross-crate fact they pin. Every value stays
   byte-identical; only the file that holds a row moves. The M46 manager applies this in each commit
   (S4's market-register listing too, where it names every kind and code).
2. The avro and parquet scripts' shared additions (`RESERVED_RANKS`, the implementer media section,
   the `claim` rule) must be per-item idempotent, since excel and xmla land first and add them: the
   M46 manager revises `s6_avro_move.py` (duplicated section, 14 stale anchors after excel - see
   `$S/s6_excel/avro_after_excel_residue.md`) and checks `s6_parquet_move.py` the same way before
   the batch; every anchor re-derived from the tree it runs on.
3. `release.yml`'s publish list and `preflight` learn every leaf crate in S5 (lane B5); D24's "this
   slice" sentence is superseded by that - the S5 define carries it.
4. `TemporalKind` through the implementer (route R): confirmed. Matching the public
   `temporal_family()` text per cell would be a per-row branch on a string.

## Disk (15:10 UTC)
At 96% (1.8 GB free): `target/` 22 GB (8.6 GB `debug/incremental`), 1.7 GB of define-worker tree
copies. Pruned the incremental dirs while cargo was idle and the finished workers' copies (logs
kept). Rule from here: every cargo command runs with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0`;
a define worker deletes its tree copies before it returns; a lane checks `df` before its chain and
needs 13 GB free.

## XMLA define done (15:05 UTC) - answers
`$S/s6_xmla_move.py` (296 files; composes after S4, not before: S4 must run first). Answers: (1) the
rowset read lands through `Serie::from_scalars` with no cross-crate proof - and the reader does not
canonicalize per cell before it, so the one pass is `from_scalars`'s (a per-cell `scalar` before it
would be the re-check AGENTS forbids); the xmla `allocations` rows are verified unmoved by the M46
manager; (2) yes, the avro and parquet scripts are made per-item idempotent before the batch (above);
(3) excel then xmla are run in order on the real tree by the manager, a doubled door shows in the first
`cargo check`; (4) S4 decision 3 holds: the market rows of `xmla/dbtype.rs` are re-fixtured in the
core, so no market test names XMLA and the script's `market` dev-dependency on xmla and its
`after = ["xmla"]` are dropped by the manager; (5) `release.yml`'s preflight and publish list are
S5's for every leaf crate; D33's "in S6c" sentence is superseded. `soap/` moves with xmla per D33;
AGENTS.md's `soap/` row says so. `scripts/generate_internals.py` writes a one-line `{}` for a crate
with nothing behind `internals` (rustfmt-stable).

## S3 (S6d) define done (15:35 UTC) - answers
`$S/s6_s3_move.py` (146 files; composes after avro+parquet and after S4+avro+parquet with 0 residue;
every request-count pin byte-identical; the implementer gains 59 items, D36's 58 plus `Secret`).
Answers, binding on M46: (1) no cargo check between the s3 and iceberg scripts - the batch's first
`cargo check` runs after all four scripts, the core under `s3tables` and the bindings building only
then; (2) `rust/tests/support/server.rs` (`FakeS3`) and the fake stores stay in the core's support and
are added to the `s3` and `iceberg` leaf rows' paths in `.github/ci/rows.toml`, so a fixture change runs
both leaves' tests; (3) the CLI links `yggdryl-s3` always, as the bindings do, and its `s3` feature
goes - one shipped command, one behaviour; recorded as a reversible choice if the user wants a lean
`cargo install yggdryl-cli` (the handoff names it); (4) the three object-store exchange jobs run
behind the `s3` leaf lane and restore its compile, never a core lane's - the leaf table's rule - the
manager wires `rows.toml` and the rust-lane cache key so; (5) the docs runner lives under `cli/` after
S4, which lands first - moot.

## Re-ordered on the user's instruction (15:50 UTC): market and fix first
User: "Focus on landing market and fix first then commit push". New order on the main tree:
P4 alone (the P45 manager parks P5 on `wip/p5-impl` with `$S/p5_state.md`, chains P4, pushes, reads
CI, writes `$S/p4_landed`) -> S4 (lane S4: `yggdryl-market` + `yggdryl-fix` on the P4 tree, its own
push and CI, `$S/s4_landed`) -> P5 re-targeted into the two crates (lane P5R) -> excel, xmla (their
own commits, one push) -> the avro+parquet+s3+iceberg batch -> S5 -> S7 -> S8.
Consequences for S4 on a tree without P5: S4 decision 7 reverses - FIX still builds
`OperationEventFacts` and reaches the ~31 market-owned items the S4 report lists, so the script's
raise through `yggdryl_market::implementer` stands (hidden, route R; a constructor door rather than
public fields where fields are `pub(crate)`), the eight forwarders and the E0624 candidates are real
work the compiler names; the move by glob carries no `graph/message.rs` yet. P5R re-targets
`wip/p5-impl`'s patch by the path map in `$S/p5_state.md` and finishes C1/C2 inside the crates.

## S5 define done (15:55 UTC) - answers for lane B5
`$S/s5_move.py` (183 files on the worktree; clean on an S4-simulated copy), report, commit message.
Decisions taken by the define and confirmed: D11 option (d) - one `_native` and one addon link every
crate, both market packages pure language layers; flit_core; `yggdryl.market.fix.state_from_status`/
`state_from_msgtype`, no alias; `node/market` with the core as an exact peer, natives handed over
through a `yggdryl/implementer` subpath, its lock derived from the core's; `scripts/release_packages.py`
the one list of crates/names/version; `cli/Cargo.toml` `publish = false`. Registry checks all run:
every new name free on crates.io, PyPI and npm (`yggdryl.market` free too). Foreground answers:
npm spelling `yggdryl-market` (D12) until the user says otherwise - the alternative goes to the
user; the Python core tests that reach the market (77 sites, 60 in `typing_bindings.py`) are
re-fixtured so the core suite proves the core wheel alone, the market typing pins moving to the
market package's own typing test (D14); `node/tests/lib.test.js`'s removal is verified test by test
by the B5 manager - every test moved somewhere, none lost; `python/src/scalar.rs`'s re-pointed
module paths are the first `cargo check` of the lane. Repository setup a real publish needs (the
user's, not ours): a PyPI pending trusted publisher for `yggdryl-market`, an npm bootstrap publish
before trusted publishing can be configured, `CARGO_REGISTRY_TOKEN` with `publish-new` over
`yggdryl-*`; a manual `release.yml` rehearsal publishes nothing and waits for the user's go.
Incident to report to the user: the define worker's first crates.io availability request carried a
User-Agent containing part of the user's e-mail address; later requests used a
generic one. Nothing else left the sandbox.

## P6 (16:10 UTC) - user: "Delete also the market served service its useless"
`$S/p6_contract.md`, `$S/p6_commit_message.txt`. The S4 lane manager lands it as the FIRST commit of
its push, on the P4 tree before the S4 move (the move then carries no `serve.rs`); one push, one CI
run for P6 + S4. Scope: `graph/serve.rs` (`BookService`), `cli/src/market.rs` (`yggdryl market
serve`), `node/book/` + `book.js`/`book.d.ts` + `node/tests/book.test.js`, the docs page and every
mention, the CI step building the CLI for the book tests, the allocations row. Candles, books,
snapshots, the audit, the HTTP server and `xmla serve` stay.

## User decisions (17:05 UTC) - "Do what's recommended"
See `$S/user_decisions.md`: the S6 batch commit (not the reversed split), npm `yggdryl-market`,
`sendunix` carrier-first kept, the CLI links `yggdryl-s3` always. The release rehearsal and the
registry setup stay on the user's explicit go. Every lane manager records them in the handoff's
questions section in its next results commit.

## Iceberg define done (17:15 UTC) - answers; every S6 define is now written
`$S/s6_iceberg_move.py` (273 files on the worktree; the batch copy avro+parquet+s3+iceberg 263 files,
one residue line - the docs runner before S4 - and the same with excel first). Answers, binding on
the media lanes: (1) `scale_ulbridge` is the FIX capture pipeline's test and travels with
`yggdryl-fix` (S4 moves it); on the batch tree it dev-depends on `yggdryl-iceberg` and
`yggdryl-parquet`; no `[shards.yggdryl-iceberg]` shard - the fix leaf's own job runs it (the scale
run is `#[ignore]`d) - and the iceberg script's removal of `test:scale_ulbridge` from
`[shards.yggdryl]` is made per-item idempotent since S4 has already done it; (2) the core keeps
`PrimitiveType`/`Transform` as the root file `rust/src/iceberg.rs` (D17), so its test is the core's
`rust/tests/root/iceberg.rs` (the mirror rule); the table-contract items of `tests/iceberg/types.rs`
merge into the crate's `table.rs` test and `types.rs` disappears; (3) the three
`yggdryl::iceberg::table` log-target rows in `rust/tests/logging/` are re-spelled to a module the
core still has (`yggdryl::holder::local`), keeping the assertion that a core module maps under
`yggdryl.`; the `yggdryl_iceberg::table` row covers the crate. Also from the report: excel's edit of
`test_an_exchange_half_runs_every_exchange`'s expected set leaves avro's anchor unmatched - the avro
script's anchor must survive excel's (the per-item idempotency pass covers it); `aws::Answer` moves
into `implementer.rs` rather than being raised (a published module).

## Disk (18:05 UTC)
96% again during P4's chain. Removed the finished define workers' tree copies and their worktrees
(s6-avro, s6-parquet, s6-excel, s6-xmla, s6-s3, s6-iceberg, s5; branches `wip/s6-*`, `wip/s5` kept):
every define is its script, re-run on the program head by its lane, so the worktrees were reference
only. Kept: `yggdryl-s4` (the S4 lane starts next) and `yggdryl-p5`.
Note: `$S/s6_parquet/` was emptied in the cleanup, its `residue.md` and trial logs included; the
script `$S/s6_parquet_move.py` and the report `$S/s6_parquet_report.md` (which lists the residue by
file:line) are intact, and the compiler loop names whatever a residue file would have.

## Handoff after S4 (user, 18:20 UTC): "push current observations to continue in another session"
After `s4_landed`, the S4 lane manager runs `$S/handoff_pack.sh /home/user/yggdryl $S` (copies the
scripts, designs, decisions, reports, briefs, commit messages, chain scripts and the parked P5 work
as `p5_on_p4.patch` into `.handoff/split/scratch/`, and `MARKET_SPLIT_CONTINUE.md` into
`.handoff/next/`; it strips personal data and checks for model identifiers), commits with
`$S/handoff_commit_message.txt`, pushes once, reads CI, writes `$S/handoff_landed`. P5R's gate is
that marker now; M6 and B5 follow as before. The briefs are relocatable (`$S` env var).
