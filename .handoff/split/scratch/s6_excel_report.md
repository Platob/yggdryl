# S6b define: `yggdryl-excel`

## State

- Worktree `/home/user/yggdryl-s6-excel`, branch `wip/s6-excel` at `ef4e241a4` (P3 over S3; S4 has
  not landed there), uncommitted - nothing committed, pushed or published. `rust/excel/` is staged
  (`git add`), so its new files show in `git diff HEAD`; everything else is a working-tree change.
- `git status --short | wc -l`: **88** (26 renames, 14 added, 1 deleted, 47 modified).
- `git diff HEAD -M --stat | tail -1`: **88 files changed, 3340 insertions(+), 1815 deletions(-)**
  (27 `git mv`; git pairs 26 as renames and shows the rewritten harness `rust/tests/excel.rs` ->
  `rust/excel/tests/excel.rs` as delete + add).
- Script: `$S/s6_excel_move.py` (imports `$S/s4_move.py` and `$S/s6_avro_move.py` for their
  helpers, edits neither). Residue: `$S/s6_excel/residue.md`. Simulation trees and checks:
  `$S/s6_excel/` (`base` = this tree's `HEAD` snapshot, `s4base` = base + `s4_move.py`,
  `checks.sh`).
- The worktree result is byte-identical to the simulation on the isolated snapshot (`diff` of the
  two `git diff HEAD -M` outputs, index lines aside: empty).
- No cargo command was run. `Cargo.lock` is not touched: the first `cargo check` without `--locked`
  adds `yggdryl-excel` (its dependencies are all in the lock already).

## Checks

| Check | Command | Result |
| --- | --- | --- |
| parse | `rustfmt --edition 2024 --check <file>` on every changed/added `.rs` (63 files) | **0 parse errors**; 38 files have formatting differences (Stage 2's `cargo fmt --all`) |
| `#[test]` count | `grep -o '#\[test\]'` over `rust/tests` and `rust/excel/tests`, at `HEAD` and after | before: core **9234**; after: core 8869 + excel 367 = **9236** = 9234 + 2 added (`rust/tests/media_register.rs` `a_reserved_rank_is_claimed_under_its_own_name_alone`; `rust/excel/tests/excel/mod_.rs` `install_claims_excel_at_its_reserved_rank_once`); 366 moved, 0 lost |
| moved tests byte-identical | each `#[test]` fn of the 7 split files against its `HEAD` body, install line and `yggdryl_excel::` spelling aside | **31 checked, 0 differ** |
| s2 pins | `mod s2_pins` at `HEAD:rust/tests/media/options.rs` against `rust/excel/tests/media/options.rs` | identical but `use yggdryl::excel::ExcelOptions` -> `use yggdryl_excel::ExcelOptions` and four `crate::install::installed();` lines; every pinned hash, the order pin and `XLSX > TSV` byte-identical |
| no test code in `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/excel/src python/src node/src cli/src` | **0 lines** |
| internals | `python3 scripts/generate_internals.py --check` | `internals are current` (core 140 modules, `yggdryl-excel` 0) |
| inventories | `python3 scripts/check_api_inventory.py` | exit 0, `inventories are current; 180 source file(s) and 565 `pub` name(s) are not described yet` (base: 180 / 569) |
| planner suite | `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | **Ran 51 tests, OK** |
| `plan.check` | `plan.check(load_config(), load_workflow())`, `plan.check_leaves(config, root)` | `[]` and `[]`; leaves `['excel']` |
| model identifiers | `git grep -i -E 'claude-(opus\|sonnet\|haiku\|fable)\|opus [0-9]\|sonnet [0-9]\|fable [0-9]'` (+ untracked) | **0** |
| old path | `git grep -E 'crate::excel\b\|yggdryl::excel\b\|internals::excel_' -- rust cli docs skills`; `yggdryl::excel` in `python node` | **0**; **0** |
| second run | the script again on the moved worktree | `rust/excel/Cargo.toml exists: the move has run on this tree; nothing changed`, exit 0; status and diff hashes identical |
| post-S4 tree | `s4_move.py` on a copy of the snapshot, committed, then this script | exit 0, **residue none**; 86 files (3284+/1791-); tests 9234 -> 9236 (core 7031, market+fix 1838, excel 367); internals current (core 117, excel 0, fix 16, market 7); inventories current (181 / 571); planner 51 OK; `plan.check` `[]`, leaves `market, fix, excel`; installs chained after market and fix in both bindings and the CLI |
| caches | `scripts/ci/__pycache__`, `scripts/tests/__pycache__` the checks created | removed; `git status --ignored` shows nothing |

## What the script does, per step

Guard: a tree with `rust/excel/Cargo.toml` stops at step 0 and changes nothing; a tree without
`rust/src/excel/mod.rs` or with changes is refused. Git writes (`git mv`, `git add`) run under
`$S/git.lock` (waits up to 15 minutes; `--no-git-lock` when the caller holds it).

1. **Test split (contract 4)**, before the moves: every core test item reaching Excel - a
   `yggdryl::excel` path, a name imported from it, a string naming an `.xlsx` handle,
   `MimeType::XLSX`, the medium's name or MIME type - moves byte-identical to
   `rust/excel/tests/<same path>` with the helpers it names (macro invocations carry what they
   define), the harness written from the core's, a `//!` header naming the core file pinned and the
   crate. 7 test files and 2 harnesses, 31 tests: `allocations` (3), `holder/mod_` (3), `iobase_calls` (2), `media/magic`
   (1), `media/mod_` (6), `media/options` (14 incl. the four s2 pins), `media_register` (2). Excluded
   as vocabulary or Excel the client: `mime_type/`, `root/mime_type.rs`, `uri/`, `xmla/`,
   `support/`. Trait imports the prune kept for a method call are listed in the residue.
2. **Register (D39)**: `rust/tests/media_register.rs` gains the avro define's `SQUATTER_CODEC` /
   `MISRANKED_CODEC` and `a_reserved_rank_is_claimed_under_its_own_name_alone`, byte for byte.
3. **Bench**: `rust/benchmarks/media.rs` loses `mod excel` and its group;
   `rust/benchmarks/media/excel.rs` becomes the root of `yggdryl-excel`'s `excel` bench
   (`#[path = "../../benchmarks/bench_profile.rs"]`, `criterion_group!`, a `main` that installs);
   the group keeps its id `media/excel`. `rust/tests/interop.rs` loses `mod excel`.
4. **Moves (contract 3, 4)**: `rust/src/excel/` -> `rust/excel/src/` (`mod.rs` -> `lib.rs`),
   `rust/tests/excel{,.rs}` -> `rust/excel/tests/`, `rust/tests/interop/excel.rs` ->
   `rust/excel/tests/interop/excel.rs`, `rust/benchmarks/media/excel.rs` ->
   `rust/excel/benchmarks/excel.rs`. `rust/tests/support/excel_package.rs` stays; the moved harness
   declares it `#[path = "../../tests/support/excel_package.rs"]`.
5. **Crate (contract 1)**: `lib.rs` - the crate doc ahead of the module's own, `#![deny(unsafe_code)]`,
   the module declarations, re-exports, constants and the `.xlsx.gz` refusal
   (`reject_outer_coding`) as they were, `pub fn install()` (`OnceLock` + `Mutex`, idempotent per D7)
   claiming `yggdryl::media::codec::claim(&EXCEL_CODEC, "yggdryl-excel")` at rank 6, the generated
   (empty) `internals` block. No `implementer` of its own: no crate reaches Excel. `DateSystem` and
   the cell model move whole with `cell.rs`.
6. **Core (contract 3)**: `lib.rs` `pub mod excel` gone; the seed claim of `EXCEL_CODEC` gone;
   `RESERVED_RANKS` (`media::codec`, re-exported from `media`) and the claim rule, in the avro
   define's exact text (excel is now the first medium to leave); `yggdryl::implementer` grown
   (below); `exclude = ["/excel"]` on the core package. The core has no `excel` feature or `cfg`
   gate to re-key; `quick-xml` and `smol_str` stay the core's (XML codec, FIX, SOAP, XMLA).
7. **Paths (contract 2, 5)**: S4's rewriter with Excel's tables over every `crate::` path of the
   moved sources (`yggdryl::X`, `yggdryl::implementer::X`, `crate::excel::` -> `crate::`), every
   `yggdryl::excel` path and `use` tree in tests, benches, bindings (`python/src/{excel,iomedia}.rs`,
   `node/src/{excel,media/options}.rs`), docs and skills (57 `use` statements, 143 paths, 39 files),
   file paths in 17 text files, `#[path]` re-anchored; docs.rs links -> `yggdryl-excel`. Then the
   four crate-private method calls the path pass cannot see (7 sites): `ZipArchive::member_reader`,
   `DataTypeId::temporal_kind` (x4), `Serie::is_string_storage`, `Serie::value_bytes`, through their
   forwarders.
8. **Interop (contract 4, 6)**: `rust/excel/tests/interop.rs`; the exchange directory kept at
   `rust/target/excel-interop` (`path.push("..")` from the crate's folder);
   `scripts/check_excel_interop.py` runs `cargo test --locked -p yggdryl-excel --test interop`.
9. **Install (D7)**: `rust/excel/tests/support/install.rs`, declared by 7 harnesses, called first by
   367 tests; the bench `main`; the crate root's rustdoc example (hidden line); 4 Rust page blocks
   (3 in `docs/media/excel.md`, 1 in the records skill); `python/src/lib.rs` (`_native`),
   `node/src/lib.rs` (`module_init`), `cli/src/main.rs` (`main`), after market and FIX where S4 has
   landed.
10. **Manifests (contract 1, 5)**: `rust/excel/Cargo.toml` (`yggdryl.workspace`, `arrow-array`,
    `arrow-schema`, `quick-xml =0.41.0`, `ryu`, `smol_str =0.3.2`; dev `criterion`; features
    `parquet` forwarded for the moved s2 pins and holder test, `internals` forwarded;
    `[[bench]] excel`; tests auto-discovered) and its README; workspace members and
    `[workspace.dependencies]` (`yggdryl` and `yggdryl-excel` at `=0.1.21`); `yggdryl-excel.workspace
    = true` in `python/`, `node/`, `cli/`.
11. **Tooling**: where S4 has not landed, S4's per-crate edits of `generate_internals.py` and
    `check_api_inventory.py` (the avro define's `edit_tooling`), then the generator run.
12. **CI (contract 6)**: see Planner changes.
13. **Docs (contract 7)**: `docs/media/excel.md` (Build and Rust rows, docs.rs links, the cost-pin
    paths, the bench command `cargo bench -p yggdryl-excel --bench excel -- media/excel`, the run
    named by its group), `docs/media/index.md` (Build column, the core's six, the register section
    and `RESERVED_RANKS` assertion), `docs/benchmarks.md` (target table and command list),
    `docs/contributing.md`, `docs/architecture.md`, the records skill (`references/rust.md` recipe,
    `references/formats.md` build column), the entry skill's install row, `README.md` layout,
    AGENTS.md (the `excel/` row, the `media/` row, the `implementer.rs` sections, the registered
    medium row, the media paragraph, the bench command), the binding modules' own doc sentences,
    `.api-inventory.txt` (sections under `### yggdryl_excel..`, `install()`, `EXCEL_CODEC`'s claim,
    `EXTERNAL_RANK`/`RESERVED_RANKS`, the implementer's media section).
14. **Residue**: written to the residue file.

## Residue (file:line)

1. `rust/excel/tests/media/magic.rs`, `rust/excel/tests/media/mod_.rs`, `rust/tests/media/mod_.rs`:
   `IORecordOptions`, `IOBase`, `IOMedia`; `rust/excel/tests/media_register.rs`: `LocatedTable`,
   `TableFormat`, `ObjectValue` - trait imports the prune kept only because a method of that name is
   called; the compiler loop drops each it finds unused (the last three are likely unused).
2. `scripts/check_docs_examples.py`: before S4 the Rust page blocks compile as a core test target,
   which cannot link `yggdryl-excel` (a cycle); the 4 blocks that install it compile once S4 moves
   the runner under `cli/` (D13). Post-S4 tree: no residue at all.
3. For the compiler loop (no site known wrong): `TemporalKind` raised to `pub` inside the
   crate-private `temporal::scalars` and `nanoseconds_per` with it; `iobase.rs`'s re-export line is one
   long line until `cargo fmt`.

## Implementer entries

Core, `rust/src/implementer.rs`, a new closing section "Media: what the media crates reach" - 27
entries; the map's 24 grew by 3 because D34/D35 made `media.rs` read `container_origin`,
`field_under` and the cache clock:

| Entry | Route |
| --- | --- |
| `append_arrow_reader_default`, `leaf_writer`, `merge_arrow_reader_default`, `overwrite_arrow_reader_default_with_field` (`iobase/transfer.rs`) | R (raised in `transfer`, moved to `iobase.rs`'s `pub use transfer::{..}` - `iobase` is private - and `pub use crate::iobase::{..}`) |
| `owned_handle` (`iobase/hierarchy.rs`) | R (`pub use crate::iobase::hierarchy::owned_handle`) |
| `container_field`, `container_origin`, `container_row_size`, `dimension_options`, `field_under`, `own_options`, `read_record_serie` (`iomedia.rs`) | R (two doc links to private items turned to code spans) |
| `TemporalKind`, `nanoseconds_per` (`temporal::scalars`, `pub(crate) mod`) | R (D24: `TemporalKind` crosses through the implementer) |
| `datatype_id_temporal_kind` (`DataTypeId::temporal_kind`), `zip_archive_member_reader` (`ZipArchive::member_reader`) | A |
| `cache_now`, `arrow_schema_from_field` (the avro define's text), `f64_from_text`, `format_datetime`, `parse_date`, `parse_time`, `write_leaf_text`, `write_element_text`, `write_attribute_text`, `write_x_escape`, `decode_x_escapes` | F (published modules: `media`, `arrow`, `floating` - glob-re-exported at the root, so no raise - `temporal`, `xml`) |

Reused as they were: `field_from_arrow_schema`, `BOOLEAN_SPELLINGS`, `bool_from_text`,
`expected_got`, `elide_to`, `ERROR_TEXT_LIMIT`, `with_field` (`text::prepare_text`), `result_reader`,
`serie_is_string_storage`, `serie_value_bytes`; `string_scalars!`, `delegate_iobase!` and
`record_options_fields!` stay `yggdryl::` root macros (their expansions name public items only).

## Planner changes

- `.github/ci/rows.toml`: `[leaves]` `excel = { package = "yggdryl-excel", jobs = ["excel-interop"] }`
  uncommented (no `after`); `[rows.x-excel]` extends `crate-excel` instead of `interop`; the
  `[rows.interop]` comment (six exchanges in the core's target).
- `scripts/tests/test_ci_plan.py`: `test_an_exchange_half_runs_every_exchange` no longer expects
  `excel-interop` from a change to `rust/tests/interop/zip.rs` and asserts it is absent.
- `.github/workflows/ci.yml`: two comments (`core-default` builds the `interop` target the ZIP and
  Avro drivers run; the `excel-interop` job names `rust/excel/tests/interop/excel.rs`). No job
  changes: the job runs the driver, whose cargo line is now `-p yggdryl-excel`.

## Decisions where the map is silent (or wrong)

- `rust/tests/support/excel_package.rs` stays in support (the brief, D39), though the map said it
  moves; the moved harness reaches it by `#[path]`.
- The map's dependency list missed `ryu` (`cell.rs`, `writer.rs`) and `arrow-schema` (`media.rs`):
  the manifest is computed from the moved sources.
- `TemporalKind` crosses by route R (D24) rather than re-spelling the per-cell match onto
  `DataTypeId::temporal_family()` strings, which would be a string branch per cell and a second owner
  of the family table.
- The xml writers take F, not R: `xml` is published and its `pub(crate) use wire::{..}` would publish
  them if raised; `f64_from_text` takes F because `floating` is glob-re-exported at the root.
- Cross-medium core tests that name Excel move whole (regroup's answer to the avro define), so the
  s2 pins and the order pin are in `yggdryl-excel` with their bytes unchanged.
- `RESERVED_RANKS` and the claim rule land with Excel, in the avro define's exact text, since Excel
  now leaves first.
- The release workflow is left alone: it publishes the core alone and S4 lists neither of its crates
  there (see open question 3).

## Findings

1. **Composes with S4 and stands alone.** The define runs clean on this tree and on a post-S4 copy
   (residue none there); every check passes on both, a second run is a no-op, and the worktree is
   byte-identical to the simulation.
2. **The reserved rank keeps every pin.** `claim` admits `excel` at rank 6 under its own name alone;
   the s2 pins, the order pin, `media_register.rs`'s expected order and the rank-6 assertion move
   with their bytes unchanged but the import path and the install line.
3. **366 tests move, 2 are added, none lost** (9234 -> 9236): the Excel harness and interop (335) by
   `git mv`, 31 cross-medium core tests by the item split, each verified byte-identical.
4. **27 implementer entries** by S3's routes (14 R, 2 A, 11 F) and 7 method-call sites re-spelled;
   no item is raised inside a published module.
5. **The avro define no longer composes after Excel.** Run after this script on a post-S4 copy,
   `s6_avro_move.py` exits 0 but appends its whole media section again (duplicate `pub use
   crate::iobase::{..}`, `crate::iomedia::{..}`, `cache_now`, `arrow_schema_from_field` - E0252 /
   E0428) and reports 14 stale anchors (raises already done, the planner test, `ci.yml`, the media
   page, the inventory). Its sibling-leaf pass does the right thing: the moved cross-medium tests
   name `yggdryl::avro`, so `yggdryl-excel` gains `yggdryl-avro` as dev-dependency and `after =
   ["avro"]`. The M46 manager re-defines avro (per-item guards on its implementer section, anchors on
   the post-excel text) before the batch.

## Open questions

1. Cross-medium tests: 31 tests now in `yggdryl-excel` also pin IPC, CSV/TSV, text, Avro, Parquet
   and XMLA facts (`mod s2_pins`, `record_options_have_complete_value_traits_and_stable_hashes`, the
   three `every_concrete_options_type_*`, `a_record_wrapper_forwards_the_tail_read`, the
   `media/mod_` round trips). `cargo test -p yggdryl` no longer pins those core hashes, and Excel's
   tests will dev-depend on `yggdryl-avro`, `-parquet` and `-xmla` as each leaves (its leaf grows
   `after`). Keep them whole (regroup's rule), split the core-only rows back by hand, or move the s2
   pins to `cli/` tests, which link every crate?
2. Avro re-define: confirm the M46 manager revises `s6_avro_move.py` for the amended order (finding 5).
3. D24 says release preflight and AGENTS §6's name list gain `yggdryl-excel` in S6b, but
   `release.yml` publishes the core alone and S4 adds neither of its crates: does S9 own all of it?
4. `TemporalKind` crossing by route R (D24) rather than `temporal_family()` strings - confirm.
