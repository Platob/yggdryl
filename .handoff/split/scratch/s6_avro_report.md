# S6a define: `yggdryl-avro`

## State

- Worktree `/home/user/yggdryl-s6-avro`, branch `wip/s6-avro` at `a46a2177b`, uncommitted
  (nothing committed, pushed or published). `rust/avro/` is staged (`git add`), so the new files
  show in `git diff HEAD`; everything else is a working-tree change.
- `git status --short | wc -l`: **121**.
- `git diff HEAD -M --stat | tail -1`: **121 files changed, 5165 insertions(+), 2522 deletions(-)**
  (24 `git mv`; git pairs 21 as renames, and shows `mod.rs` -> `lib.rs` and the two rewritten
  harness/bench roots as delete + add).
- Script: `$S/s6_avro_move.py` (2769 lines, imports `$S/s4_move.py` for its helpers). Residue:
  `$S/s6_avro/residue.md`. Simulation helpers (base copy, rerun, checks, post-S4 simulation):
  `$S/s6_avro/`.
- The worktree result is byte-identical to the simulation on an isolated copy
  (`diff` of the two `git diff HEAD -M` outputs, index lines aside: empty).
- No cargo command was run. `Cargo.lock` is not touched: the first `cargo check` without
  `--locked` adds `yggdryl-avro` and moves `snap` from the core to the crate.

## Checks

| Check | Command | Result |
| --- | --- | --- |
| parse | `rustfmt --edition 2024 --check <file>` on every changed/added `.rs` (94 files) | **0 parse errors**; 56 files have formatting differences (Stage 2's `cargo fmt --all`) |
| `#[test]` count | `grep -o '#\[test\]'` over `rust/tests` and `rust/avro/tests`, at `HEAD` and after | before: core 9211 + avro 0 = **9211**; after: core 9031 + avro 182 = **9213** = 9211 + 2 added (`rust/tests/media_register.rs` `a_reserved_rank_is_claimed_under_its_own_name_alone`; `rust/avro/tests/avro/mod_.rs` `install_claims_avro_at_its_reserved_rank_once`); 181 moved, 0 lost |
| no test code in `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/avro/src python/src node/src cli/src` | **0 lines** |
| internals | `python3 scripts/generate_internals.py --check` | `internals are current` (core 138 modules, `yggdryl-avro` 3) |
| inventories | `python3 scripts/check_api_inventory.py` | exit 0, `inventories are current; 179 source file(s) and 572 `pub` name(s) are not described yet` (base: 180 / 570; +3 names raised inside the crate's private modules - `RecordType`, `FixedType`, `load_into` - and `decimal_parameters` now described) |
| planner suite | `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | **Ran 51 tests, OK** |
| `plan.check` | `plan.check(load_config(), load_workflow())` and `plan.check_leaves(config, root)` | `[]` and `[]`; leaves `['avro']` |
| model identifiers | `git grep -i -E 'claude-(opus\|sonnet\|haiku\|fable)\|opus [0-9]\|sonnet [0-9]\|fable [0-9]'` | **0** |
| old path | `git grep 'crate::avro\|yggdryl::avro\|internals::avro'` | **0** |
| second run | the script again on the moved worktree | `rust/avro/Cargo.toml exists: the move has run on this tree; nothing changed`, exit 0; status and diff hashes identical |
| post-S4 tree | `s4_move.py` on a copy of the base, committed, then this script | exit 0; 118 files; all tests 9211 -> 9213; generate_internals current (core 115, avro 3, fix 16, market 7); inventories current; planner 51 OK; `plan.check` `[]`, leaves `market, fix, avro`; installs chained after market and fix in the bindings and the CLI |
| s2 pins | `mod s2_pins` at `HEAD:rust/tests/media/options.rs` against `rust/avro/tests/media/options.rs` | identical but `use yggdryl::avro::AvroOptions` -> `use yggdryl_avro::AvroOptions` and four `crate::install::installed();` lines; every pinned hash and order byte-identical |
| caches | `scripts/ci/__pycache__`, `scripts/tests/__pycache__` the checks created | removed |

## What the script does, per step

Guard: a tree with `rust/avro/Cargo.toml` stops at step 0 and changes nothing; a tree without
`rust/src/avro/mod.rs` or with changes is refused. Git writes (`git mv`, `git add`) run under
`$S/git.lock` (waits up to 15 minutes; `--no-git-lock` when the caller holds it).

1. **Test split (contract 4)**, before the moves: every core test item reaching Avro - a
   `yggdryl::avro` path, a name imported from it, a string naming an `.avro` handle,
   `MimeType::AVRO`, the medium's name - moves byte-identical to `rust/avro/tests/<same path>`
   with the helpers it names (a macro invocation travels with the items it defines:
   `test_options!(TestOptions, ..)`), the harness written from the core's, a `//!` header naming
   the core file pinned and the crate. 38 items, 181 tests, 12 files: `csv/options` (1),
   `graph/serve` (1), `iobase_calls` (3), `ipc/mod_` (1), `media/mod_` (3), `media/options` (13,
   the s2 pins), `media_register` (2), `root/ascii` (1), `root/iomedia` (7), `root/media_serie`
   (3), `warehouse/folder` (2), `xmla/options` (1). Each half's imports pruned, a trait whose
   methods the half calls kept. Iceberg's tests are left (residue). Post-S4, a sibling leaf's
   test reaching Avro (`rust/market/tests/graph/serve.rs`) stays, installs the crate, gains the
   dev-dependency, and its `[leaves]` line gains `after = ["avro"]`.
2. **Register (D39)**: `rust/tests/media_register.rs` gains `SQUATTER_CODEC` (rank 2, another
   name) and `MISRANKED_CODEC` (`parquet` at 36) and the reserved-rank test.
3. **Bench split (contract 4)**: the Avro rows of `rust/benchmarks/media/io/{dimensions,write,
   pushdown}.rs` and `holder/calls.rs` leave the core for `rust/avro/benchmarks/avro/{io,calls}.rs`
   (the core's helpers copied byte-identical, the glue registering the Avro rows alone); the
   core's `media` bench root and the pushdown doc lose Avro; `rust/tests/interop.rs` loses
   `mod avro`.
4. **Moves (contract 3, 4)**: `rust/src/avro/` -> `rust/avro/src/` (`mod.rs` -> `lib.rs`),
   `rust/tests/avro{,.rs}` -> `rust/avro/tests/`, `rust/tests/interop/avro.rs` ->
   `rust/avro/tests/interop/avro.rs`, `rust/benchmarks/media/avro{,.rs}` ->
   `rust/avro/benchmarks/avro{,.rs}`.
5. **Crate (contract 1, 2)**: `lib.rs` (the doc, `#![deny(unsafe_code)]`, the re-exports,
   `pub fn install()` - `OnceLock` + `Mutex`, idempotent per D7 - claiming
   `yggdryl::media::codec::claim(&AVRO_CODEC, "yggdryl-avro")` at rank 2, `#[doc(hidden)] pub mod
   implementer`); items Iceberg reads raised to `pub` inside crate-private modules; the
   `parquet` (snappy) and `iceberg` (raw header bytes) gates made unconditional (D16);
   `implemented_codecs` a constant; the plan-cache calls through the core's forwarders.
6. **Core (contract 3)**: `pub mod avro` and its `internals` re-exports gone; the seed claim of
   `AVRO_CODEC` gone; `RESERVED_RANKS` (`media::codec`, re-exported from `media`) and the claim
   rule; `yggdryl::implementer` grown (below); `PlanCache` moved into it (M) and its 7 core users
   re-pointed (`expression/arrow.rs`, `expression/transform.rs`, `iceberg/scan.rs`,
   `iobase/transfer.rs`, `media/merge.rs`, `parquet/mod.rs`, `serie/arrow.rs`); Iceberg's
   manifest re-spelled onto `yggdryl_avro` (residue: the cycle). `rust/Cargo.toml`: `snap` and
   `parquet = [.., "dep:snap"]` gone (the `parquet` feature stays for `dep:parquet`; the core has
   no `avro` feature to re-key), `exclude = ["/avro"]`, the `bytes` and media-bench comments.
7. **Paths (contract 2, 5)**: S4's rewriter with Avro's tables: every `crate::` path of the moved
   sources by owner (`yggdryl::X` / `yggdryl::implementer::X`), every `yggdryl::avro` path and
   `use` tree in tests, benches, bindings (`python/src/{avro,iomedia,media/partition}.rs`,
   `node/src/{avro,media/options}.rs`), docs and skills (75 `use` statements and 243 paths in 43
   files), `yggdryl::internals::avro_*` -> `yggdryl_avro::internals::*`, file paths in 16 text
   files, `#[path]` re-anchored, `#[path = "../../benchmarks/bench_profile.rs"]`.
8. **Interop (contract 4, 6)**: `rust/avro/tests/interop.rs`; the exchange directory kept at
   `rust/target/avro-interop` (`path.push("..")` from the crate's folder);
   `scripts/check_avro_interop.py` runs `cargo test --locked -p yggdryl-avro --test interop`.
9. **Install (D7)**: `rust/avro/tests/support/install.rs`, declared by 11 harnesses, called first
   by 182 tests; the bench `main`; 4 Rust page blocks; `python/src/lib.rs` (`_native`),
   `node/src/lib.rs` (`module_init`), `cli/src/main.rs` (`main`, before `Cli::parse`), each after
   the market and FIX installs where S4 has landed.
10. **Manifests (contract 1, 5)**: `rust/avro/Cargo.toml` (`yggdryl.workspace`, the Arrow crates,
    `bytes`, `flate2`, `smol_str`, `snap = "1.1"`; dev: `arrow-cast`, `arrow-select`,
    `criterion`; features `parquet`, `iceberg`, `http` forwarded for the moved tests only,
    `internals` forwarded; `[[bench]] avro`; tests auto-discovered), its README; the workspace
    members and `[workspace.dependencies]` (`yggdryl` and `yggdryl-avro` at `=0.1.21`); the
    bindings' and the CLI's `yggdryl-avro.workspace = true`.
11. **Tooling**: where S4 has not landed, S4's per-crate edits of `generate_internals.py` and
    `check_api_inventory.py` (read off `s4_move.py`), then the generator run.
12. **CI (contract 6)**: see Planner changes.
13. **Docs (contract 7)**: `docs/media/avro.md` (contract table, examples, bench commands
    `cargo bench -p yggdryl-avro --bench avro`), `docs/media/index.md` (the register section,
    `RESERVED_RANKS`, the six core media), `docs/testing.md`, `docs/contributing.md`, the
    `yggdryl-records` skill (`SKILL.md`, `references/rust.md`, `references/formats.md`), both
    READMEs, AGENTS.md (the `ipc/`, `parquet/`, `csv/` row, the `media/` row, the registered
    medium row, the Iceberg bullet, the test-mirror example re-pointed to `csv/reader.rs`),
    `.api-inventory.txt` (sections under `### yggdryl_avro..`, the crate root and its
    `implementer`, the core's implementer additions, `RESERVED_RANKS`).
14. **Residue**: written to the residue file.

## Residue (file:line)

The lane manager fixes these by hand or the compiler confirms them.

1. **`rust/src/iceberg/manifest.rs`** - 11, 529, 530, 537, 540, 541, 709, 786, 824, 858, 946, 1064,
   1065, 1069, 1070, 1072, 1087, 1109, 1246, 1249, 1250, 1251, 1281, 1409, 1705: the core's
   `iceberg` feature names `yggdryl_avro` (re-spelled to its final form), which the core cannot
   depend on. Every `--all-features` build, the bindings (they enable `yggdryl/iceberg`) and the
   core's Iceberg tests fail to compile until Iceberg is `yggdryl-iceberg`.
2. Core tests and benches that read Avro and belong to Iceberg (they move with
   `yggdryl-iceberg`, which installs Avro):
   - `rust/tests/iceberg/evolve.rs:67`
   - `rust/tests/iceberg/manifest.rs:172, 267, 494, 505, 516, 529, 548, 587, 918, 976, 1145, 1181, 1273, 1289, 1299`
   - `rust/tests/iceberg/metadata.rs:83`
   - `rust/tests/iceberg/mod_.rs:63, 103, 203, 218, 2200, 2295, 2296, 2352, 2388, 6201, 6597, 6748, 6809, 6892, 7164, 9376, 9414, 9416, 9438, 9442, 9521, 9545, 9566, 9585, 9606, 9620, 11666`
   - `rust/tests/iceberg/official.rs:36, 74`
   - `rust/tests/iceberg/scan.rs:36, 173, 525, 771, 1091, 1277`
   - `rust/tests/iceberg/snapshot.rs:47, 830, 837`
   - `rust/tests/iceberg/table.rs:200, 204, 208, 213`
   - `rust/tests/interop/iceberg.rs:382, 386, 402`
   - `rust/benchmarks/media/iceberg.rs:258, 382, 405, 554, 579`
3. For the compiler loop to confirm (no site known wrong): a core test that names an `.avro`
   handle through a computed string (`format!`) is invisible to the split and would fail at run
   time with the register's refusal; the moved and split files' imports were pruned by name plus
   trait-method calls, so an `unused_imports` warning or a missing trait is possible.

## Implementer entries

Core, `rust/src/implementer.rs`, a new section "Media: what the media crates reach":

| Entry | Route |
| --- | --- |
| `append_arrow_reader_default`, `merge_arrow_reader_default`, `overwrite_arrow_reader_default_with_field` (`iobase/transfer.rs`) | R (`pub use crate::iobase::{..}`, raised inside `transfer` and re-exported `pub` from the private `iobase`) |
| `container_origin`, `container_row_size`, `dimension_options`, `held_arrow_field`, `own_options` (`iomedia.rs`) | R (their doc links to private items turned to code spans) |
| `FileThreads` and `resolve` (`media/options.rs`) | R |
| `sorted_pairs` (`metadata/pairs.rs`) | R |
| `cache_now` (`media::cache::now`) | F |
| `arrow_schema_from_field` (`arrow::arrow_schema_from_field`) | F |
| `arrow_cast_plan_compile_schema` (`ArrowCastPlan::compile_schema` with `Deferred::default()`) | A |
| `arrow_cast_plan_reconcile_batch` (`ArrowCastPlan::reconcile_batch`) | A |
| `is_text_storage`, `is_variant_storage`, `decimal_parameters`, `uuid_bytes`, `uuid_parse`, `uuid_text` | F |
| `PlanCache` (`new`, `get_or_compile`), moved from `cast.rs` | M |

Reused as they were: `field_from_arrow_schema`, `stable_hash_of`, `expected_got`,
`record_options_fields!`. The module doc's M list names `PlanCache`.

Crate, `rust/avro/src/implementer.rs` (`#[doc(hidden)] pub mod`, for `yggdryl-iceberg`'s
manifest reads):

| Entry | Route |
| --- | --- |
| `container::{BlockCoding, Header, HeaderEntries, MAGIC, SCHEMA_KEY, SYNC_LEN, check_magic, header_entry, parse_header, parse_header_entries}` (with `BlockCoding::load_into` and `Header`'s fields) | R |
| `datum::{Cursor, DatumCodec}` (`Cursor::{new, take, long, bytes, is_exhausted}`, `DatumCodec`'s `names`/`limits`, `budget`, `decode`) | R |
| `schema::Node` (with `RecordType`, `EnumType`, `FixedType`, `DecimalType`) | R |
| `schema_names`, `schema_node` (`Schema::names`, `Schema::node`) | A |
| `blocks_metadata_bytes` (`Blocks::metadata_bytes`) | A |

## Planner changes

- `.github/ci/rows.toml`: `[leaves]` `avro = { package = "yggdryl-avro", jobs = ["avro-interop"] }`
  uncommented (no `after`: Avro depends on the core alone); `[rows.x-avro]` extends
  `crate-avro` instead of `interop`; the `[rows.interop]` comment (six exchanges in the core's
  target). Post-S4 only: a leaf whose tests link Avro (`market`) gains `after = ["avro"]`.
- `scripts/tests/test_ci_plan.py`: `test_an_exchange_half_runs_every_exchange` no longer expects
  `avro-interop` from a change to `rust/tests/interop/zip.rs` and asserts it is absent.
- `.github/workflows/ci.yml`: two comments (`core-default` no longer builds the Avro driver's
  target; the `avro-interop` job names `rust/avro/tests/interop/avro.rs`). No job changes: the
  job runs the driver, whose cargo line is now `-p yggdryl-avro`.
- `scripts/check_avro_interop.py`: the cargo line and the docstring.

## Decisions where the map is silent

- Everything Avro leaves: the scalar codec, the container, schema resolution, single-object
  framing and the record medium; nothing Avro-specific stays in the core but Iceberg's manifest
  calls (re-spelled, residue 1).
- Core tests that exercise Avro beside other media (`media_register`, `media/options` with the
  s2 pins, `iobase_calls`, `root/iomedia`, ...) move whole into `rust/avro/tests/` at the core
  file's mirrored path, so a pin is never edited; the crate forwards the core's `parquet`,
  `iceberg` and `http` features only because those moved tests read them.
- The core's `parquet` feature keeps `dep:parquet`; `snap` moves to the crate unconditionally.
- `RESERVED_RANKS` is a `pub const` in `media::codec`, re-exported from `media`; `claim`
  checks it before the external-rank rule, so a reserved name at another rank and another name at
  a reserved rank are refused at `$.encoding`, naming both.
- The two added tests are the only new tests.

## Findings

1. **The D39 order does not build alone.** With Avro out and Iceberg in, the core's `iceberg`
   feature names `yggdryl_avro`, a dependency cycle. The bindings enable `yggdryl/iceberg`, so
   the Python and Node builds, every `--all-features` lane, and the core's Iceberg tests fail
   to compile until S6c moves Iceberg. DESIGN's "S6's shape" (one commit) holds: land S6a with
   S6b and S6c, or reverse the order.
2. **The reserved rank keeps every pin.** `claim` admits `avro` at rank 2 under its own name
   alone. The s2 pins move to `rust/avro/tests/media/options.rs` unchanged except for the import
   path and the install line, so every pinned hash and order is the same value.
3. **181 core tests move byte-identical.** They move with their helpers into 12 split files of
   `rust/avro/tests/`. The core keeps every test that does not name Avro, and the counts add up
   (9211 -> 9031 + 182, two added).
4. **S4's import prune drops traits used only for method calls.** It also loses items a macro
   invocation defines (`test_options!(TestOptions, ..)` dropped from both halves). This script
   patches both for its own run (`prune_imports` keeps a trait whose methods are called;
   `macro_names`). Probing S4's output on a simulated post-S4 tree flags candidate files where
   such a trait may be missing: `rust/fix/tests/{allocations,iobase_calls,graph/iterator,
   graph/market_data,isin_registry/env,root/temporal}.rs`,
   `rust/market/tests/{allocations,iobase_calls,root/datatype,root/implementer,
   root/valuestream}.rs`, `rust/tests/{allocations,iobase_calls,root/ascii,root/datatype,
   root/valuestream,iceberg/mod_,iceberg/partition}.rs`. The probe is heuristic: a common method
   name gives false positives. S4's compiler loop will confirm, with E0599 naming the trait.
5. **`PlanCache` takes route M.** Raising it in the published `cast` module would publish it, so
   it moves into `yggdryl::implementer`, and the core's seven users read it from there. The
   media crates still to come (Parquet, Excel, XMLA) can reuse every forwarder this section adds.

## Open questions

1. Order and commit shape: S6a+S6b+S6c as one commit, or Iceberg first? Answering this unblocks
   the `--all-features` lane and the bindings.
2. Cross-medium tests: is moving them whole right (they now run in the Avro crate), or should
   the lane manager split their non-Avro assertions back into the core by hand?
3. The crate's forwarded `parquet`/`iceberg`/`http` features exist only for the moved tests.
   When Parquet and Iceberg leave the core, should they become dev-dependencies on those crates,
   or should those tests move on to the Iceberg crate?
4. Pre-S4 the docs runner compiles Rust examples as a core test target, which cannot name
   `yggdryl_avro` (the four page blocks this script edits). Post-S4 it compiles under `cli/`,
   which links the crate. S4 lands first in M46, so this is only a question if the order changes.
5. `FileThreads` is raised with its tuple field `pub` inside the private `media::options`. That
   stays within route R, but the lane manager should confirm it reads as intended.
