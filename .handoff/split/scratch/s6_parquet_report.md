# S6 define: `yggdryl-parquet`

## State

- The worktree is `/home/user/yggdryl-s6-parquet` on branch `wip/s6-parquet` at `a46a2177b`. It is uncommitted: nothing was committed, pushed or published.
- `rust/parquet/` is staged with `git add`, so the new files show in `git diff HEAD`. Everything else is a working-tree change.
- `git status --short | wc -l` is **113**.
- `git diff HEAD -M --stat | tail -1` gives **113 files changed, 5526 insertions(+), 2328 deletions(-)**:
  - 34 added, 73 modified, 5 renamed, 1 deleted.
  - The renames are `rust/src/parquet/{mod.rs -> lib.rs, geospatial.rs, metadata.rs}` and `rust/tests/parquet/{mod_.rs, metadata.rs}` into `rust/parquet/`.
  - The deleted file is the old harness `rust/tests/parquet.rs`, rewritten as `rust/parquet/tests/parquet.rs`.
- The script is `$S/s6_parquet_move.py <tree> [--no-git-lock] [--residue <file>]`, 3433 lines.
  - It copies the helpers it needs from `$S/s4_move.py` and the Avro define rather than importing them, because importing the Avro script would mutate s4's globals.
  - The residue is in `$S/s6_parquet/residue.md`.
  - The simulation trees and logs are in `$S/s6_parquet/`.
- No cargo command was run. `Cargo.lock` is not touched: the first `cargo check` without `--locked` adds `yggdryl-parquet`.
- The script was run on four trees, all clean:
  - the worktree;
  - simA, the base copy;
  - simB, the base with the Avro define applied first;
  - simC, the base with `s4_move.py` then the Avro define applied first.

## Checks

| Check | Command | Result |
| --- | --- | --- |
| parse | `rustfmt --edition 2024 --check <file>` on every changed or added `.rs` (83 files) | **0 parse errors**. 45 files have formatting differences, left for Stage 2's single `cargo fmt --all` (the same base files show none). |
| `#[test]` count | `grep -o '#\[test\]'` over `rust/tests` and `rust/parquet/tests`, at `HEAD` and after | Before: core 9211 + parquet 0 = **9211**. After: core 9081 + parquet 132 = **9213**, which is 9211 + 2 added: `rust/tests/media_register.rs` `a_reserved_rank_is_claimed_under_its_own_name_alone` (added only where the Avro define has not added it) and `rust/parquet/tests/parquet/mod_.rs` `install_claims_parquet_at_its_reserved_rank_once`. Test names are otherwise identical; 0 lost. |
| no test code in `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/parquet/src python/src node/src cli/src` | **0 lines** |
| internals | `python3 scripts/generate_internals.py --check` | `internals are current`: core 139 modules, `yggdryl-parquet` 1 |
| inventories | `python3 scripts/check_api_inventory.py` | Exit 0, current. 180 source files and 570 `pub` names omitted, the same as base. |
| planner suite | `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | **Ran 51 tests, OK** |
| `plan.check` | `plan.check(load_config(), load_workflow())` and `plan.check_leaves(config, root)` | `[]` and `[]`; leaves are `['parquet']` |
| old path | `git grep 'crate::parquet\|yggdryl::parquet\|internals::parquet'` | Code: 0. Prose only: `.handoff/` design notes (history) and `scripts/docs_core_remaining.js:41`, a stale workflow prompt (see the open questions). |
| model identifiers | `git grep -i -E 'claude-(opus\|sonnet\|haiku\|fable)\|opus [0-9]\|sonnet [0-9]\|fable [0-9]'` | **0** |
| caches | `scripts/ci/__pycache__`, `scripts/tests/__pycache__` the checks created | removed |
| s2 pins | `mod s2_pins` at `HEAD:rust/tests/media/options.rs` against both halves after the split | **16 pinned values byte-identical**. Only the `use` paths and `crate::install::installed();` lines changed. Rank 1 is reserved, so no hash or order moves. |
| second run | the script again on the moved worktree | `rust/parquet/Cargo.toml exists: the move has run on this tree; nothing changed`, exit 0; status and diff are unchanged |
| simB (after the Avro define) | the Avro define, then this script | Exit 0, residue 5. All checks above pass (0 parse errors, internals and inventories current, 51 OK, `[]`/`[]`). `RESERVED_RANKS`, the claim rule, the reserved-rank test and the shared implementer section are found and not duplicated. |
| simC (after S4, then Avro) | `s4_move.py` (committed), the Avro define, then this script | Exit 0, residue 4, every check passes. Sibling leaves (market, fix, avro) whose tests reach Parquet install it, gain the dev-dependency and `after = [.., "parquet"]`. Installs are chained in dependency order in the bindings and the CLI. |

## What the script does, per step

The script has two guards:
- A tree that already has `rust/parquet/Cargo.toml` stops at step 0 and changes nothing.
- A tree without `rust/src/parquet/mod.rs`, or one with uncommitted changes, is refused.

Git writes (`git mv`, `git add`) run under `$S/git.lock`. The script waits up to 15 minutes for it; pass `--no-git-lock` when the caller already holds the lock.

1. **Test split (contract item 4, D39).** This runs before the moves.
   - **What moves:** every core test item that reaches Parquet moves byte-identical to `rust/parquet/tests/<same path>`. An item reaches Parquet when it has:
     - a `yggdryl::parquet` or `internals::parquet*` path;
     - a name imported from one of them; or
     - a positive `feature = "parquet"` gate.
   - **What travels with it:** the helpers it names, found by closure, and a macro invocation together with the items it defines.
   - **Harnesses** are written from the core's, each with a `//!` header naming the core file it pins.
   - **Gates are resolved in each half.** In the crate, a positive gate holds, a `not(..)` branch is dropped, `if cfg!(..)` is unwrapped and `assert_eq!(x, cfg!(..))` becomes `assert!(x)`. In the core, the opposite holds.
   - **Orphaned helpers** are dropped only where the old file used them.
   - **Totals:** 45 items in 15 files (counts per file are in `residue.md`), plus the Parquet suite that `git mv` moves, for 132 crate tests in all. The s3 helpers `BUCKET`, `file`, `path` and `store` are copied to `rust/parquet/tests/s3/mod_.rs`.
   - **Left in place:** Avro's tests (their `parquet` gates are Avro's snappy, D16) and Iceberg's tests (residue).
2. **Register (D39).**
   - The `#[cfg(feature = "parquet")]` seed line leaves `media/codec.rs`.
   - If the Avro define has not already added them, the script adds `RESERVED_RANKS = [("parquet",1),("avro",2),("xmla",4),("excel",6)]` and the `claim` rule: a reserved rank under its own name only, that name at no other rank, everything else at or above `EXTERNAL_RANK`.
   - It also adds the reserved-rank test to `rust/tests/media_register.rs`.
3. **Bench split.** The Parquet rows of the shared record and holder benches leave the core for `rust/parquet/benchmarks/parquet/{io,calls,fs,s3}.rs` under the bench root `parquet.rs`. The core's helpers are copied, and the core's media `io` group becomes ungated.
4. **The crate.**
   - `git mv` the source into place, with `mod.rs -> lib.rs`.
   - `lib.rs` gains the crate docs, `#![deny(unsafe_code)]` and `pub fn install()`. It claims `PARQUET_CODEC` through `yggdryl::media::codec::claim(&PARQUET_CODEC, "yggdryl-parquet")` and is idempotent per D7, using a `OnceLock` plus a mutex.
   - The root's own `internals` holds the generated re-exports between the markers.
   - The `PlanCache` compile and reconcile calls go through `yggdryl::implementer`, and `crate::serie::land(..)` becomes `crate::implementer::land_unproven(..)`.
   - It writes `Cargo.toml` (dependencies: `arrow-array`, `arrow-schema`, `bytes`, `parquet`, `smol_str`; `arrow-cast` and `arrow-select` as dev-dependencies) and `README.md`.
5. **Core implementer.**
   - If the Avro define has not added it, the script adds the shared media section, and `PlanCache` moves from `cast.rs` into `implementer.rs` with 7 core files re-pointed.
   - It then adds Parquet's own section (listed below).
   - The core's Iceberg `scan.rs`, `table.rs` and `statistics.rs` are re-spelled onto `yggdryl_parquet[::implementer]`.
6. **Paths.** 44 `use` statements and 225 paths in 25 files are re-owned to `yggdryl_parquet`, and file paths are re-pointed in 9 files. There is no alias and no re-export: `yggdryl::parquet` is gone, and `DEFAULT_CRS` leaves the crate root's Parquet re-export.
7. **Install.** `install()` is called in:
   - 11 test harnesses (`rust/parquet/tests/support/install.rs`; 132 tests);
   - the bench `main`;
   - 1 rustdoc example;
   - 11 page blocks;
   - `python/src/lib.rs`, at module init before class registration;
   - `node/src/lib.rs`;
   - `cli/src/main.rs`, under `#[cfg(feature = "parquet")]`.
8. **Manifests.**
   - The workspace gains the member `rust/parquet` and the pinned `yggdryl-parquet = { path, version = "=0.1.21" }`.
   - The core adds `exclude = ["/parquet"]`. It keeps `parquet = ["dep:parquet","dep:snap"]` only because `iceberg` implies it (the core's Iceberg reads footers as `ParquetMetaData`); the comments say so.
   - The bindings depend on the crate.
   - The CLI gains an optional `parquet = ["dep:yggdryl-parquet"]` feature, implied by `iceberg`, plus a dev-dependency so the pages' Rust blocks link it.
9. **Tooling and CI.**
   - `generate_internals.py`: a crate root that declares its own `internals` holds the generated block inside it.
   - `check_api_inventory.py` knows the crate.
   - `scripts/check_iceberg_interop.py` and `ci.yml`: `--features "parquet iceberg"` becomes `--features iceberg`, and the crate commands drop the core features.
   - `.github/ci/rows.toml`: the `parquet` leaf line is uncommented with its exchanges.
10. **Docs.** The Parquet page, the media overview, the holder page, `arrow/readers.md`, `formats.md`, the skills (Cargo snippet, feature table, expressions), `contributing.md`, the READMEs and AGENTS.md now name `yggdryl_parquet`, its install and its feature. `.api-inventory.txt` re-homes the Parquet sections under `yggdryl_parquet` and adds its implementer and the core's additions.
11. **Residue scan.** Every leftover is written with its `file:line`.

## Residue (worktree)

- `rust/src/iceberg/scan.rs`, `rust/src/iceberg/table.rs` and `rust/src/iceberg/statistics.rs` name `yggdryl_parquet`, but the core cannot depend on it (a cycle). They compile once Iceberg is `yggdryl-iceberg`. Until then, every `--all-features` build fails, and so do the bindings (which enable `yggdryl/iceberg`) and the core's Iceberg tests. This is the same state the Avro define leaves.
- `rust/tests/iceberg/mod_.rs:11144`, `:11239` name `yggdryl_parquet`. They belong to Iceberg and move with `yggdryl-iceberg`, which installs Parquet.
- `rust/tests/avro/container.rs:368` and `rust/benchmarks/media/avro/codecs.rs:58` keep a `parquet` gate that is Avro's snappy (D16). The Avro move makes it unconditional.
- `scripts/check_docs_examples.py`: the Rust page blocks compile in the core's `rust/tests/docs_examples.rs` until S4 moves the runner to `cli/tests/` (D13). A block naming `yggdryl_parquet` compiles only after that. After S4 the CLI's dev-dependency carries it, as simC shows.

Out of scope, not edited:
- `scripts/docs_core_remaining.js:41` is a stale workflow prompt naming `yggdryl::parquet`.
- `scripts/bench_avro_baseline.py:36,108` keeps `"parquet iceberg"`; it is Avro's.
- `docs/graph/serve.md:39` names the CLI's `parquet` feature, which still exists as the CLI's own.

## Implementer entries added

Core `yggdryl::implementer`:

- **Shared media section**, added only where the Avro define has not:
  - `pub use` of the `iobase` record-door defaults;
  - `pub use crate::iomedia::{container_origin, container_row_size, dimension_options, held_arrow_field, own_options}`;
  - `cache_now`, `arrow_schema_from_field`, `arrow_cast_plan_compile_schema`, `arrow_cast_plan_reconcile_batch` and `is_variant_storage`;
  - `PlanCache` (route M, moved from `cast.rs`).
- **Parquet's own:**
  - `projection_indices` and `same_columns` (route F);
  - `land_unproven(field: Arc<Field>, array: ArrayRef) -> crate::arrow::Result<Serie>`, a forwarder over the landing, which reads every leaf;
  - the constants `DEFAULT_CRS`, `GEOARROW_WKB_EXTENSION_NAME` and `UUID_EXTENSION_NAME`.

Crate `yggdryl_parquet::implementer` (`#[doc(hidden)] pub mod`; what Iceberg reaches):

- the constants `READ_AHEAD_BATCHES`, `WHOLE_READ_BYTES` and `WRITE_BUFFER_BYTES`;
- `read_batch_reader_with`, `load_metadata` (the footer), `schema_from_metadata`, `overwrite_buffered` (the encoder's closed footer and length) and `file_statistics_from_metadata`.

`ParquetOptions`, `ParquetFooter`, `FileStatistics`, `ColumnStatistics` and `read_media_statistics`/`read_media_geospatial_statistics` are the crate's public API and move with it.

## Planner changes

- In `.github/ci/rows.toml` `[leaves]`, `parquet = { package = "yggdryl-parquet", jobs = ["pyiceberg-interop", "spark-interop"] }` is uncommented.
- On a tree where sibling leaves exist and their tests reach Parquet, their lines gain `after = [.., "parquet"]` (simC).
- No planner code changed. The suite (51 OK), `plan.check` and `check_leaves` pass.

## Open questions

1. **The Iceberg cycle.** The core's `iceberg` feature names `yggdryl_parquet`, so no tree with Parquet moved and Iceberg still in the core builds under `iceberg`, `--all-features` or the bindings. `.handoff/next/MARKET_SPLIT_PROMPT.md:155` says the same: S6 lands as one commit with Iceberg, or in the reversed order (Iceberg first). This define leaves the core's Iceberg re-spelled onto the crate, ready for the Iceberg move.
2. **The publish list.** `release.yml` publishes `yggdryl` alone. Publishing `yggdryl-parquet` (and its preflight read) is not edited here; it belongs to S9 or preflight.
3. **The docs runner before S4.** Until S4 moves the runner to `cli/tests/`, a Rust page block naming `yggdryl_parquet` does not compile in the core's runner.
4. **The core's `parquet` feature.** It stays only because `iceberg` implies it, for `ParquetMetaData`, and it dies with the Iceberg move. Should it be renamed or folded into `iceberg` now instead?
5. **Dead code from the helper closure.** Helpers copied to both halves, or kept in the core for one remaining user, may raise `dead_code` warnings. Only the first `cargo check --all-targets --keep-going` names them; the heuristic drops only what the old file referenced.
6. **The S3 move.** The S6d move must carry `rust/parquet/tests/s3/*` and `rust/parquet/benchmarks/parquet/s3.rs` along with S3's own.
