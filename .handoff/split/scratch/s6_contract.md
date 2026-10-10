# S6 contract - the media crates leave: avro, parquet, excel, xmla, then iceberg (+s3tables)

Design: DESIGN.md "## The crate map", "## S6's shape", D15, D16, D17, D24, D33, D36 and
`scratchpad/d39_design.md` (normative for the rank, the door, the Iceberg view, the tests,
install() and the order). Maps: `scratchpad/moves_map/<crate>.md`. Precedent: the S4 move
script (`scratchpad/s4_move.py`, when it exists) and S3's `implementer` routes. Program rules
as every slice: no test code under `src/`, no alias or old-path re-export, one commit per
crate, the exact trailer, no model identifier, never publish, never main, never AWS.

## Stage 1 - one define worker per crate, no cargo
Worktree per worker: `git -C /home/user/yggdryl worktree add /home/user/yggdryl-s6-<crate> -b
wip/s6-<crate> wip/s3` under the git lock (`mkdir $S/git.lock` ... `rmdir`). Deliver
`$S/s6_<crate>_move.py <tree>` (idempotent `git mv` + exact-string rewrites, anchors asserted)
and `$S/s6_<crate>_report.md`:
1. `rust/<crate>/Cargo.toml` (`yggdryl-<crate>`, version through `[workspace.dependencies]` as
   `=<version>`, deps from the map's section 4, features mirroring the core's gates it used,
   `internals` forwarded, `[[test]]`/`[[bench]]` targets), the workspace members line,
   `rust/<crate>/src/lib.rs` (the folder's `mod.rs` becomes it; `pub fn install()` claiming the
   codec through `yggdryl::media::codec::claim(&<NAME>_CODEC, "yggdryl-<crate>")` at its
   reserved rank, plus `media::format::claim`, `warehouse::claim_factory`, `holder::claim_locator`
   where the crate brings one; idempotent as D7 says; the crate's own `#[doc(hidden)] pub mod
   implementer` for what Iceberg reaches, Avro and Parquet only).
2. Path rewrites inside the moved files: `crate::X` core -> `yggdryl::X` or
   `yggdryl::implementer::X` (the map's section 2 lists which; a missing forwarder is an entry
   the script adds to `rust/src/implementer.rs` by route R/F/A/M/X, with its rustdoc and no
   intra-doc link to a private item); intra-crate `crate::<crate>::` -> `crate::`; `super::`
   fixes.
3. The core: the folder deleted (`git mv`), `lib.rs` `pub mod <crate>` and re-exports gone, the
   seed claim gone, the `cfg(feature = "<crate>")` sites outside the folder re-keyed or deleted as
   the map says, the feature line kept only where another core gate still needs it (say which).
4. Tests: `rust/tests/<crate>/**` and the harness -> `rust/<crate>/tests/`; the core tests the
   map names as building the crate's objects move too (`//!` line naming the core file they pin);
   interop tests under `rust/tests/interop/<crate>*` move with the exchange script re-pointed
   (`--manifest-path`/`-p yggdryl-<crate>`); benches -> `rust/<crate>/benchmarks/` with a
   `#[path]` to `bench_profile.rs`; `FakeS3`/`excel_package.rs` stay in `rust/tests/support/`.
5. Bindings and CLI: `python/Cargo.toml`, `node/Cargo.toml`, `cli/Cargo.toml` depend on the
   crate; `yggdryl_<crate>::install()` at module init / `main`; `yggdryl::<crate>::` paths
   re-spelled `yggdryl_<crate>::`.
6. CI: `.github/ci/rows.toml` `[leaves]` line uncommented with its `jobs` and `after`;
   `scripts/tests/test_ci_plan.py` expectations; the exchange scripts' cargo lines.
7. Docs/skills/inventory: docs.rs links, the "claimed by the core until" sentences, the
   `EXTERNAL_RANK` sentence, `.api-inventory.txt` sections re-homed under
   `### yggdryl_<crate>::...`, AGENTS.md's Layout rows (the folder row says "in
   `yggdryl-<crate>`", the media list), skills' `use yggdryl::<crate>` -> `use yggdryl_<crate>`.
8. The iceberg worker also: the `IcebergField` view through `protocol_field_types!` and the
   `as_iceberg` sweep; `s3tables/` as its feature; the dead `cfg` arms; its dependence on
   `yggdryl-s3` (D36) - it runs after S6d's define exists and reads `scratchpad/d36_design.md`.
Run the script on the worktree, `rustfmt --check` on changed `.rs`, `git status --short | wc -l`,
diff stat, the sites not rewritten mechanically listed for the lane manager, the planner test
changes, the open questions. Leave the worktree uncommitted.

## Stage 2 - lane M46's manager (after S4 lands; iceberg after S6d)
For each crate in order: re-cut from the program branch head, run the script, settle
(`CARGO_TARGET_DIR=/home/user/target-m46 cargo check --workspace --all-targets --all-features
--keep-going --message-format=short` in the worktree until clean), merge into the program
branch, phase suites (the crate's own `cargo test -p yggdryl-<crate> --all-targets`, both
lanes, the s2_pins and order pins green WITHOUT edit, `iobase_calls`/`allocations` rows
unmoved, the exchange script against its own fetched server where the sandbox allows), commit
with `$S/s6_<crate>_commit_message.txt`. One chain over the batch's final tree, one push, CI to
`CI result` (the leaf rows now run), fix at cause. DESIGN.md "## S6: what was built"; handoff rows.
