# S6 define: `yggdryl-iceberg` (with Amazon S3 Tables as its `s3tables` feature)

## State

- Worktree `/home/user/yggdryl-s6-iceberg`, branch `wip/s6-iceberg` at `4e46b5ab7` (P3 `ef4e241a4` + P4),
  uncommitted - nothing committed, pushed or published; no cargo command run; no AWS, no network.
  `rust/iceberg/` and `rust/tests/root/iceberg.rs` are staged (`git add`), every other change is a
  working-tree change.
- Worktree: `git status --short | wc -l` **273** (57 R, 1 RM, 29 A, 1 D, 185 M);
  `git diff HEAD -M --stat | tail -1` **273 files changed, 8006 insertions(+), 5037 deletions(-)**.
  The worktree result is byte-identical to the run on its isolated snapshot (`$S/s6_iceberg/t_base`;
  `cmp` of the two `git diff HEAD -M` outputs, index lines aside: identical).
- Batch (`$S/s6_iceberg/batch`: snapshot + `s6_avro_move.py` + `s6_parquet_move.py` +
  `s6_s3_move.py`, each committed in the isolated repo, then this script): **263** status lines
  (58 R, 1 RM, 28 A, 2 D, 174 M), **263 files changed, 6833 insertions(+), 4245 deletions(-)**,
  residue 1 line. Extended batch (`$S/s6_iceberg/ext`: excel first, then avro, parquet, s3, iceberg):
  same 263, residue 1 line.
- Script: `$S/s6_iceberg_move.py` (4328 lines; imports `s4_move.py` and `s6_avro_move.py` for their
  helpers, edits neither). Residue: `$S/s6_iceberg/residue.md` (worktree), `batch_residue.md`,
  `ext_iceberg_residue.md`. Check runner: `$S/s6_iceberg/checks.sh <tree> HEAD`; pin comparison:
  `$S/s6_iceberg/pins.py <tree> HEAD`.

## Checks (no cargo)

| Check | Worktree | Batch (avro, parquet, s3, iceberg) |
| --- | --- | --- |
| `rustfmt --edition 2024 --check` parse, every changed/added `.rs` (follows `mod`/`#[path]`) | **0 parse errors / 164 files** | **0 / 155** |
| `#[test]` fns before -> after (by name) | 9278 -> 9285: 0 lost; +4 added, +3 copied | 9283 -> 9290: 0 lost; +4 added, +3 copied |
| numeric literals of every `#[test]` fn present before and after (request/call-count pins) | 9054 compared, **1 differs**: `every_named_protocol_view_spells_its_own_scheme_prefix` (23 -> 22, the Iceberg view leaving the core's list) | 9059 compared, same 1 |
| `python3 scripts/generate_internals.py --check` | `internals are current` (core 130, iceberg 10) | `internals are current` (core 114, iceberg 10) |
| `python3 scripts/check_api_inventory.py` | current; not described 569 -> 589 | current; 550 -> 552 |
| `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | **51 OK** | **51 OK** (ext: 1 failure, pre-existing at the s3 commit - finding 4) |
| `plan.check` + `plan.check_leaves` | `[]`, `[]` | `[]`, `[]` |
| test code under `rust/src`, `rust/*/src` | 0 | 0 |
| `yggdryl::{avro,parquet,s3,iceberg,s3tables}::` left (`git grep`, `PrimitiveType` and `.handoff` aside) | the three siblings' paths for their own moves; iceberg: only the 3 logger-target strings in `rust/tests/logging/` | **only the 3 logger-target string literals** (`rust/tests/logging/facade.rs:84,128`, `formatter.rs:324` - data the facade maps, not paths) |
| second run | `rust/iceberg/Cargo.toml exists ... nothing changed`, `git status` md5 unchanged | same |

Added tests: `install_claims_the_table_format_and_its_catalog_once` (4-thread idempotence, format and
`hadoop` factory held by pointer), `a_second_claim_of_the_table_format_is_refused_naming_yggdryl_iceberg`
(format and factory refusals name the crate) in `tests/iceberg/mod_.rs`;
`install_claims_the_table_bucket_factory_and_locator_once` in `tests/s3tables/mod_.rs`;
`the_iceberg_view_is_built_from_the_field_and_reads_its_iceberg_keys` in `tests/iceberg/field.rs`.
The logging facade's `yggdryl_iceberg::table -> yggdryl.iceberg.table` row was already in the tree.
Copied with the gate's other side (absent side stays in the core): `warehouse/catalog.rs`
`from_url_refuses_a_type_this_build_has_no_catalog_for_and_a_nameless_url`, `warehouse/media.rs`
`a_table_bound_to_a_handle_composes_what_its_name_declares`, `warehouse/folder.rs`
`a_table_format_folder_is_never_read_as_the_leaves_it_holds` (avro's in the batch).

## What each step does

1. Test split (D39): 37 items moved (batch 32), 3 copied; byte-identical, helpers copied, gates
   resolved per half, `PrimitiveType`-only items kept in the core un-gated (`root/version.rs`).
   Harnesses written beside them, fixtures re-anchored to `rust/tests/support/`.
2. Benches: `types` `value` group's Iceberg rows -> `rust/iceberg/benchmarks/iceberg/field.rs`;
   `holder` `aws_session` S3 Tables rows -> `iceberg/aws.rs`; `media/iceberg.rs` -> `iceberg/media.rs`;
   root `benchmarks/iceberg.rs` installs, runs, cleans up. Core `media` bench loses its group.
3. Moves (59; batch 61 with the object stores' Iceberg remnant `rust/tests/s3.rs` + `s3/mod_.rs`):
   `rust/src/iceberg/**` -> `rust/iceberg/src/` (`mod.rs` -> `lib.rs`), `types.rs` ->
   `rust/src/iceberg.rs` (core keeps `PrimitiveType`, D17), `rust/src/s3tables/` ->
   `rust/iceberg/src/s3tables/`, tests/interop/`s3tables_handle.rs`/`medallion_ledger.rs`/
   `scale_ulbridge.rs`. `tests/iceberg/types.rs`'s core-only items return to the new
   `rust/tests/root/iceberg.rs`; the official-failure pin joins `tests/iceberg/official.rs`.
4. Crate: `lib.rs` doc + `#![deny(unsafe_code)]`, `pub mod s3tables` under `s3tables`, `install()`
   (OnceLock + Mutex, installs avro, parquet and, under `s3tables`, s3 first; claims
   `ICEBERG_FORMAT` via `media::format::claim`, `HADOOP_FACTORY` via `warehouse::claim_factory`,
   under `s3tables` `S3TABLES_FACTORY` and `S3TABLES_LOCATOR` via `holder::claim_locator`; each claim
   skipped where this crate already holds it), `IcebergField`/`IcebergFieldMut` minted in `field.rs`
   by `yggdryl::implementer::protocol_field_types!(pub, yggdryl::Scheme::ICEBERG, ..)`,
   `official::primitive_into_official` and `official::official_error` (the two doors the core held)
   + `internals::official::invalid_iceberg_metadata`; 38 exact-anchor method redirects (all matched),
   `Handle::at/bound`, `serie::land(.., Unproven)` -> `land_unproven`, `Error::iceberg` ->
   `Error::external("Iceberg", ..)`.
5. Core: `pub mod iceberg` = type vocabulary; `s3tables`, Iceberg internals, `Error::iceberg`/
   `from_iceberg` + `error::internals` gone; the three seeds (`media/format.rs`,
   `warehouse/catalog.rs` with `claim_unseeded` inlined, `holder/locator.rs`) gone;
   `as_iceberg`/`as_iceberg_mut`/`Metadata::as_iceberg` gone with the ICEBERG well-known entry;
   every other `iceberg`/`s3tables` gate made unconditional (16 files); features `iceberg`,
   `s3tables` (and `parquet` once its medium has moved), deps `iceberg-official`, `uuid` (and
   `parquet`) gone; `exclude` gains `/iceberg`.
6. Paths: crate sources by owner (core paths checked against the core's real publication - a
   private module routes through `implementer`, else the root re-export, else residue), tests,
   benches, bindings, CLI, docs, skills, AGENTS, inventory; Avro/Parquet/S3 paths re-spelled only
   inside the crate (elsewhere they are their own defines'). 200 `as_iceberg` calls swept:
   crate/bindings/Iceberg page/records skill -> `yggdryl_iceberg::IcebergField(Mut)::new(..)`,
   core tests/docs -> `.protocol(&yggdryl::Scheme::ICEBERG)` (generic view).
7. Interop: crate `tests/interop.rs`, exchange dir kept at `target/iceberg-interop` (two pops),
   driver and `bench_avro_baseline.py` cargo lines `-p yggdryl-iceberg`.
8. Install: 14 harnesses, 646 tests open with `crate::install::installed()`, 27 doc examples,
   26 page blocks, bench `main`, bindings' init and CLI `main` (`#[cfg(feature = "iceberg")]`).
9. Manifests: `rust/iceberg/Cargo.toml` (deps from what the sources name; `s3tables =
   ["dep:yggdryl-s3", "yggdryl/aws"]`, `http` forwarded, `internals`), README, workspace member +
   `[workspace.dependencies]`, python/node `yggdryl-iceberg` with `s3tables` (core `iceberg`,
   `s3tables` features dropped), CLI `iceberg = ["parquet", "dep:yggdryl-iceberg"]`, `s3tables` ->
   `yggdryl-iceberg/s3tables`, dev-dep for the pages; sibling leaves' `iceberg` forwards dropped.
10. Tooling: per-crate generators (S4's edits where missing), generator run; facade row present.
11. CI: leaf line on (`after = ["avro","parquet","s3"]` from what is listed), `x-iceberg` extends
    `crate-iceberg`, MSRV job `-p yggdryl-iceberg --all-targets --features s3tables`, PyIceberg
    build `-p yggdryl-iceberg`, `test:scale_ulbridge` out of `[shards.yggdryl]`, planner test.
12. Docs: Iceberg page Build row, `docs/index.md`, skills feature tables, generic prose
    (`the iceberg feature` -> `yggdryl-iceberg`, ...), AGENTS (features, layout rows, gated loop,
    MSRV row, pins sentence), inventory (`### yggdryl_iceberg ..`, core `### yggdryl::iceberg
    [rust/src/iceberg.rs]`, view entries moved, implementer lines), every `cargo` line spelling a
    core feature the core lost.

## Implementer (`rust/src/implementer.rs`, section "Iceberg", guarded by name)

61 entries (batch adds 55: `oversized`, `canonical_query`, `encode_query_component`,
`arn_partition_check_region`, `session_stated_region` already the object stores'; worktree also
adds Avro's shared media section and `PlanCache` move via `s6_avro_move.edit_implementer`, and
Parquet's `land_unproven`): `sorted_values`, `PARQUET_FIELD_ID_KEY`, `SORT_BY_KEY`, `parse_by_list`,
`parse_by_ordering`, `UUID_SPELLINGS`, `decimal_text`, `read_enum_code`, `is_null_value`,
`partition_column_name`, `per_second`, `oversized`, `WriteCount` (R), `non_empty_arrow_reader`,
`prepare_arrow_write_deriving`, `absent`, `merged` + `Merged` (R), `Cadence` (R),
`record_options_commit_cadence`/`_commit_arrow_readers` (opaque `impl Iterator`)/`_set_file_threads`,
`compose` + `Composed`/`Residual`/`Scan` (R), `PartitionClosing`/`Partitions`/`Pieces` (R,
`serie::partition` now `pub(crate) mod`), `serie_into_relabeled`,
`stream_chunked_serie_from_landed_iter`/`_map_landed`, `chunked_serie_keeps_order`/`_sorted_by_on`,
`EpochPeriod`/`TimeBucket` (R), `epoch_value`, `order`, `function_epoch_period`,
`derivation_owning_apply`, `selector_unnests`, `field_under`, `Site`/`Opener` (R,
`warehouse::handle` now `pub(crate) mod`), `handle_at`/`_bound`/`_set_properties`/`_url`/`_exists`,
`entry_name`, `table_layout`, `path_text`, `extended`, `implementation_name`,
`holder_is_backend_property`, `uri_names_s3_tables`, `arn_identified_table`, `arn_table_bucket`,
`arn_partition_check_region`, `arn_partition_is_opt_in`, `Answer` (M, `aws` is published;
`aws::error_type` raised to `pub(crate)`, `aws/{container,login,metadata}.rs` re-pointed),
`error_code`, `Refusal` (R), `canonical_query`, `encode_query_component`, `session_http`,
`session_stated_region` (its `s3` gate lifted), `is_unanswered`, `land_unproven`. 33 raises; every
intra-doc link in a raised item's docs made a code span; `# Errors` present on every fallible one.

## Residue (worktree; batch keeps only the first-but-last line)

- `rust/Cargo.toml`: the core's Parquet medium is still the core's here - its `parquet` feature and dep stay.
- `Cargo.toml`: no `yggdryl-s3` workspace dependency until `s6_s3_move.py` runs.
- `.github/ci/rows.toml`: `avro`, `parquet`, `s3` leaves not listed - `after` is empty here.
- `scripts/check_docs_examples.py`: pages compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner to `cli/tests/` (D13); a block naming `yggdryl_iceberg` compiles only there (all three batches).
- tree: the crate names `yggdryl_avro`/`yggdryl_parquet`/`yggdryl_s3`; builds once their moves have run.

## Decisions where the map was silent

- The view is minted in `rust/iceberg/src/field.rs` (D39) and re-exported from the crate root; core
  tests/docs that read only the generic view use `field.protocol(&Scheme::ICEBERG)`, the typed
  ones moved; the core's protocol-view count pin goes 23 -> 22 (the one numeric pin moved).
- Core gates resolved with both features on (the crate reaches those items through `implementer`);
  the seeds deleted, not gated. Inside the crate an `s3` gate is `s3tables` (D36.6).
- The test of the official failure follows its door into `tests/iceberg/official.rs`;
  `tests/iceberg/types.rs` keeps the table contract (it pins no crate file now - open question 2).
- `install()` installs avro, parquet and (under `s3tables`) s3 first, and is retry-safe.
- Standalone, the script still writes the final crate form and adds Avro's shared implementer
  section (guarded), so the worktree result is self-consistent for review; the batch is the real
  composition (avro/parquet/s3 paths outside the crate are left to their own moves).

## Findings

1. Batch composition is clean: 1 residue line, 0 parse errors, 0 tests lost, every pin literal
   identical but the protocol-view count, internals/inventory/planner checks green.
2. Dynamic core-path routing caught what a fixed table misses (`hashing::stable_hash_of`,
   `text::elide_to`, `parallel::ordered` -> `implementer`; `cast::ArrowCastPlan` kept public).
3. `aws::Answer` had to move (M), not be raised: `aws` is a published module.
4. Extended order (excel before avro): `scripts/tests/test_ci_plan.py`
   `test_an_exchange_half_runs_every_exchange` already fails at the s3 commit - excel's edit of the
   expected set leaves avro's anchor unmatched, so `"avro-interop"` stays expected. Not this
   script's; the avro define's edit needs an anchor that survives excel's.
5. `cargo fmt` is still owed (import order, wrapped lines), and the compiler loop must confirm
   `private_interfaces`/`missing_docs` on the raised items and the `&mut` guesses of the sweep
   (none flagged as unknown).

## Open questions

1. `[shards.yggdryl-iceberg]`: should `scale_ulbridge` get a shard of its own in the leaf lane?
2. `rust/iceberg/tests/iceberg/types.rs` pins no crate source (types.rs is the core's): merge its
   table-contract tests into `table.rs`, or keep the file as a contract suite?
3. The three logger-target strings `yggdryl::iceberg::table` in `rust/tests/logging/` are data;
   keep them, or re-spell them `yggdryl_iceberg::table` (the row already exists)?
