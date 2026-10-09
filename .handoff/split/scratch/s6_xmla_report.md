# S6c define: `yggdryl-xmla`

## State

- Worktree `/home/user/yggdryl-s6-xmla`, branch `wip/s6-xmla` at `ef4e241a4`, uncommitted (nothing
  committed, pushed or published). `rust/xmla/` is staged (`git add`); the moves are staged renames;
  everything else is a working-tree change.
- `git status --short | wc -l`: **296** (249 R, 34 M, 11 A, 2 D - the two harnesses `rust/tests/{xmla,soap}.rs`,
  whose docs were rewritten, pair as D + A).
- `git diff HEAD -M --stat | tail -1`: **296 files changed, 3162 insertions(+), 1217 deletions(-)**
  (251 `git mv`, 224 of them the Excel wire fixtures).
- Script: `$S/s6_xmla_move.py` (2,043 lines; imports `$S/s4_move.py` and `$S/s6_avro_move.py` for their
  helpers, neither edited). Residue log: `$S/s6_xmla/residue.md` (the run's "Done" list; mechanical residue
  **none**). Simulation helpers: `$S/s6_xmla/rerun.sh` (a fresh `git archive ef4e241a4` copy, then the move) and `checks.sh <tree>` (the copies were removed for disk - 1.7 GB free; their logs and residues are kept) and the reach tool
  `$S/s6_xmla/tool/` (a copy of `moves_map/tool` pointed at this tree).
- The worktree result is byte-identical to the simulation on an isolated copy (`git diff HEAD -M`, index
  lines aside: equal).
- No cargo command was run. `Cargo.lock` is not touched: the first `cargo check` without `--locked` adds
  `yggdryl-xmla`.

## Checks

| Check | Command | Result |
| --- | --- | --- |
| parse | `rustfmt --edition 2024 --check <file>` on every changed/added `.rs` (52 files) | **0 parse errors**; 37 files have formatting differences (one-line `use` trees the rewriter rendered; Stage 2's `cargo fmt --all`) |
| `#[test]` count | `git grep -o '#\[test\]' HEAD -- ':(glob)rust/**/tests/**' cli/tests` vs the tree | before **9278**; after core 8388 + xmla 848 + cli 44 = **9280** = 9278 + 2 added (`rust/tests/media_register.rs` `a_reserved_rank_is_claimed_under_its_own_name_alone`, S6a's text; `rust/xmla/tests/xmla/mod_.rs` `install_claims_xmla_at_its_reserved_rank_once`); 847 moved, 0 lost |
| no test code in `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/xmla/src python/src node/src cli/src` | **0 lines** |
| internals | `python3 scripts/generate_internals.py --check` | `internals are current` (core 140 modules, `yggdryl-xmla` 0: `pub mod internals {}`) |
| inventories | `python3 scripts/check_api_inventory.py` | exit 0, `inventories are current; 180 source file(s) and 567 `pub` name(s) are not described yet` (base 180 / 569) |
| planner suite | `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | **Ran 51 tests, OK** |
| `plan.check` | `plan.check(load_config(), load_workflow())`, `plan.check_leaves(config, root)` | `[]` and `[]`; leaves `['xmla']` |
| model identifiers | `git grep -i -E 'claude-(opus\|sonnet\|haiku\|fable)\|opus [0-9]\|sonnet [0-9]\|fable [0-9]'` | **0** (tracked and untracked) |
| old paths | `git grep -E 'yggdryl::(xmla\|soap)\b\|crate::xmla\b\|crate::soap\b'` outside `rust/xmla/src` | **0** (`crate::soap` inside the crate is its own module) |
| second run | the script again on the moved worktree | `rust/xmla/Cargo.toml exists: the move has run on this tree; nothing changed`, exit 0; status and diff hashes identical |
| pinned values | `mod s2_pins`, `xmla_costs`, `mod provider`, `a_record_wrapper_forwards_the_tail_read`, `a_rowset_write_allocates_nothing_per_row`, the media order test, against `HEAD` | identical but `yggdryl::xmla::` -> `yggdryl_xmla::`, `crate::install::installed();` lines and one `use` tree re-rendered on one line; every hash, count and order byte-identical |
| post-S4 tree | `s4_move.py` on a copy of the base, committed, then this script | exit 0, residue none; 298 files; tests 9278 -> 9280 (core 6556, xmla 842, market+fix 1838, cli 44); internals current (core 117, fix 16, market 7, xmla 0); inventories current; planner 51 OK; `plan.check` `[]`, leaves `market, fix, xmla`; installs chained after market and fix |
| reverse order | this script, committed, then `s4_move.py` | S4 exits 0 with 4 more residue lines than on the base (workspace `members` anchor, Node init anchor, two `check_api_inventory.py` anchors) and leaves `rust/xmla/tests/xmla/dbtype.rs` naming `Side`, `MarketDataKind`, `MarketDataType`, `TimeInForce` through `yggdryl::` (S4 scans only `rust/tests/`): **the scripts do not commute; S4 runs first**, as the amended order says |
| caches | `scripts/ci/__pycache__`, `scripts/tests/__pycache__` the checks created | removed |

## What the script does, per step

Guard: a tree with `rust/xmla/Cargo.toml` stops at step 0; a tree without `rust/src/xmla/mod.rs` or
`rust/src/soap/mod.rs`, or with changes, is refused. Git writes (`git mv`, `git add`) run under `$S/git.lock`
(waits up to 15 minutes; `--no-git-lock` when the caller holds it).

1. **Test split (contract 4), before the moves.** S6a's split with XMLA's detection - a `yggdryl::xmla` or
   `yggdryl::soap` path, a name imported from either, `MimeType::XMLA`, a string naming a `.xmla` handle,
   `"xmla"`, `"application/xmla+xml"` - moves items byte-identical to `rust/xmla/tests/<same path>` with
   their helpers (S6a's prune keeping traits called by method and the items a macro call defines), a
   `//!` header naming the core file pinned and the crate. 17 items, 5 files: `allocations` (1, the
   Rowset row with the counting allocator), `iobase_calls` (5: the tail-read check, `xmla_costs` inside
   `mod records`, `mod provider` with `counting_filesystem` re-anchored to `../../tests/support/`),
   `media/mod_` (2), `media/options` (8, the s2 pins among them), `media_register` (1, the order pin);
   the `media` harness written from the core's. `rust/tests/warehouse/catalog.rs` stays: its `xmla` is
   the catalog `type` word `Catalog::from_url` refuses (D10). Then S6a's reserved-rank pin is appended to
   the core's `media_register.rs` where absent.
2. **Moves.** `rust/src/xmla/` -> `rust/xmla/src/` (`mod.rs` -> `lib.rs`), `rust/src/soap/mod.rs` ->
   `rust/xmla/src/soap/mod.rs`, `rust/tests/{xmla,soap}{,.rs}` with the 224 fixtures ->
   `rust/xmla/tests/`, `rust/benchmarks/media/xmla.rs` -> `rust/xmla/benchmarks/xmla.rs` (the bench
   root; the core's `media` bench drops its module and group).
3. **Crate (contract 1, 2).** `lib.rs`: the crate's doc ahead of the module's (its summary paragraph
   dropped, a `soap` row in its table), `#![deny(unsafe_code)]`, `pub mod soap`, `install()` -
   `OnceLock` + `Mutex`, idempotent (D7) - claiming `yggdryl::media::codec::claim(&XMLA_CODEC,
   "yggdryl-xmla")` at rank 4, the generated `internals` block. The rowset read's landing,
   `crate::serie::from_canonical_rows`, becomes `Serie::from_scalars` (see findings); `Serie::
   {is_string_storage, is_byte_storage, value_bytes}` and `Plan::map_sources` are reached through the
   core's `serie_*` and `plan_map_sources` forwarders; `XMLA_CODEC`'s doc links `crate::install`; the
   SOAP and crate docs say the HTTP server is the core's. No hidden `implementer` of its own: no crate
   reaches XMLA.
4. **Core (contract 3).** `pub mod soap` and `pub mod xmla` gone; the seed claim of `XMLA_CODEC` gone;
   `RESERVED_RANKS` and the claim rule written with S6a's exact text where no earlier media move wrote
   them (so S6a's own edits of `codec.rs` and `media/mod.rs` see their replacement and skip); the
   implementer grown item by item (below), each item skipped where an earlier move added it under its
   name; `exclude = ["/xmla"]`. No core feature line: the core has no `xmla` feature, and `http`,
   `parquet`, `internals` stay the core's.
5. **Paths (contract 2, 5, 7).** S4's rewriter over `XmlaTables`: inside the crate `crate::xmla::X` ->
   `crate::X`, `crate::soap::X` kept, a core path `yggdryl::X`, a crate-private one
   `yggdryl::implementer::<name>`; outside, `yggdryl::xmla::X` -> `yggdryl_xmla::X`,
   `yggdryl::soap::X` -> `yggdryl_xmla::soap::X` - tests, benches, the CLI (`cli/src/xmla.rs`,
   `cli/tests/xmla.rs`), sibling leaves, page blocks and prose naming them, AGENTS.md, the READMEs, the
   inventory (72 `use` statements, 214 paths). File paths in 18 text files, `#[path]` and
   `CARGO_MANIFEST_DIR` re-anchored (the fixtures stay at `tests/xmla/fixtures/excel` under the crate),
   docs.rs links to `docs.rs/yggdryl-xmla/latest/yggdryl_xmla/`, `.gitattributes`' `-text` line.
6. **Install (D7).** `rust/xmla/tests/support/install.rs`, declared by 6 harnesses, opening 847 tests;
   the install test appended last; a hidden `# yggdryl_xmla::install().unwrap();` in the crate's 6
   rustdoc examples; the bench `main`; 4 page blocks of `docs/media/xmla.md`;
   `python/src/lib.rs` (`_native`), `node/src/lib.rs` (`module_init`), `cli/src/main.rs` (`main`,
   before `Cli::parse`), each after whatever installs there already (the market and FIX installs on a
   post-S4 tree, chained in the CLI). A sibling leaf's test reaching XMLA - S4's market half of
   `xmla/dbtype.rs` on a post-S4 tree - stays where S4 left it, re-spelled, installs the crate, its
   manifest gains the dev-dependency and its `[leaves]` line `after = ["xmla"]`.
7. **Manifests (contract 1, 5).** `rust/xmla/Cargo.toml`: `yggdryl.workspace`, `arrow-array`,
   `arrow-schema`, `smol_str` (deps computed from what the sources name), dev `criterion`; features
   `http = ["yggdryl/http"]` (the provider's routes), `parquet = ["yggdryl/parquet"]` (the one gate a
   moved test reads), `internals = ["yggdryl/internals"]`; `[[bench]] xmla`; README. The workspace
   members and `[workspace.dependencies]` (`yggdryl` and `yggdryl-xmla` at `=0.1.21` where absent);
   `python` and `node` `yggdryl-xmla.workspace = true` (no feature: they link it for the claim);
   `cli` `yggdryl-xmla = { workspace = true, features = ["http"] }`.
8. **Tooling.** S6a's `edit_tooling` (S4's per-crate generator and inventory reader where S4 has not
   landed), then one edit of `generate_internals.py`: a crate with nothing behind the feature gets
   `pub mod internals {}` on one line - the `{\n}` the generator wrote is what `cargo fmt` rewrites,
   so `--check` would fail after the format pass. `yggdryl-xmla` is the first such crate.
9. **CI (contract 6).** `xmla = { package = "yggdryl-xmla" }` uncommented (no `after`, no `jobs`, no
   `msrv`); no `[shards]` (the leaf's targets run in `rest`). No exchange script names XMLA, no workflow
   job names its targets, and `test_ci_plan.py` already lists `"xmla": set()`; nothing else changed.
10. **Docs (contract 7).** `docs/media/xmla.md` (the Build and Rust rows, the bench command
    `cargo bench -p yggdryl-xmla --bench xmla -- media/xmla`, the provider's HTTP server the core's),
    `docs/media/index.md` (the table row, the core's own media list without XML for Analysis,
    `RESERVED_RANKS` and the example's assertion where absent, the count one fewer, the claim sentence
    - merged with an earlier move's clause where one is there), `docs/testing.md` (`-p yggdryl-xmla --test
    {xmla,soap}`), `docs/contributing.md`, `README.md` (the layout's medium list and a `xmla/` crate
    line), AGENTS.md (below), `.api-inventory.txt` (11 sections re-homed under `### yggdryl_xmla..`,
    `crate::` in their signatures spelled `yggdryl::`, the crate root's `install()`, `XMLA_CODEC`'s
    claimant, `RESERVED_RANKS`, `claim`'s refusals, the implementer additions).
11. **Residue scan.** Old paths, core-private paths left in the crate, core tests still reading XMLA,
    `#[path]`/`include_*` naming no file, test code under `src/`: none found.

AGENTS.md: the `soap/` row is `` `soap/` in `yggdryl-xmla` `` - the protocol vocabulary only XML for
Analysis speaks here, so it is the crate's (D33), `rust/xmla/src/soap/`, `yggdryl_xmla::soap`; the
`xmla/` row is `` `xmla/` in `yggdryl-xmla` ``, its `XMLA_CODEC` claimed by the crate's `install()` at
the reserved rank 4; the `xml/` row's `Element` view is read by `yggdryl-xmla`'s `soap/` and medium; the
`media/` row's register lists `RESERVED_RANKS` and the core's media count one fewer; the registered-medium
row admits a reserved rank under its own name; `cargo bench -p yggdryl-xmla --bench xmla` beside the
core's bench line.

## Residue (file:line)

Mechanical residue: **none**. For the lane manager's compiler loop and review:

1. `rust/xmla/src/rowset.rs:819` - `Serie::from_scalars(Arc::clone(&self.field), rows)` replaces
   `crate::serie::from_canonical_rows` (finding 1): correct by construction, it costs one more
   `Field::scalar` walk per row and settles and verifies the declared order as every core door does.
   `cargo bench -p yggdryl-xmla --bench xmla -- media/xmla/rowset --quick` gives the direction.
2. `rust/xmla/src/rowset.rs:1203,1204,1230,1231`, `rust/xmla/src/service.rs:942` - the method calls
   re-spelled through the core's A forwarders.
3. `rust/src/iobase.rs:272-273`, `rust/xmla/src/media.rs:16`, `rust/xmla/src/soap/mod.rs:50` and the
   other one-line `use` trees - rendered by the rewriter, reflowed by `cargo fmt --all`.
4. `rust/xmla/src/lib.rs:141` `pub mod internals {}` and `scripts/generate_internals.py` (step 8).
5. For the compiler loop to confirm (no site known wrong): the moved and split files' imports were
   pruned by name plus trait-method calls, so an `unused_imports` warning or a missing trait is possible;
   a core test reading an `.xmla` handle through a computed name, or a loop over every MIME type
   expecting XMLA claimed, would fail at run time with the register's refusal (none found by the scan).
6. Pre-S4 only: the docs runner compiles the Rust page blocks as a core test target, which cannot name
   `yggdryl_xmla` (4 blocks of `docs/media/xmla.md`); post-S4 they compile under `cli/`, which links it.

## Implementer entries

Core, `rust/src/implementer.rs`, appended under "Media: what the media crates reach" (lines 966-1166),
each added only where no earlier move added it under the same name:

| Entry | Route |
| --- | --- |
| `append_arrow_reader_default`, `leaf_writer`, `merge_arrow_reader_default`, `overwrite_arrow_reader_default_with_field` (`iobase/transfer.rs`) | R: raised to `pub fn`, moved from `pub(crate) use transfer::{..}` to `pub use transfer::{..}` in the private `iobase`, `pub use crate::iobase::{..}` |
| `container_field`, `container_origin`, `container_row_size`, `dimension_options`, `field_under`, `own_options`, `read_record_serie` (`iomedia.rs`) | R: raised to `pub fn` in the private `iomedia`; two doc links to crate-private items made code spans |
| `arrow_schema_from_field`, `cache_now`, `uuid_parse` (shared with S6a's list, same names) | F |
| `into_base64`, `parse_timestamp`, `holds`, `no_catalog`, `path_text` | F |
| `write_fragment`, `write_leaf_text`, `write_x_escape`, `decode_x_escapes`, `write_element_text`, `write_attribute_text`, `is_name_start`, `is_name_char` (const), `shaped` (`xml::wire`, a private module inside the published `xml`) | F |
| `plan_map_sources` (`Plan::map_sources`) | A |

Reused as they were: `field_from_arrow_schema`, `normalize_path` (`http`), `integer_from_text_as`,
`format_timestamp`, `expected_got`, `elide_to`, `ERROR_TEXT_LIMIT`, `serie_is_string_storage`,
`serie_is_byte_storage`, `serie_value_bytes`, the exported `record_options_fields!` and
`delegate_iobase!`. Not added: `serie::arrow::from_canonical_rows` (finding 1). Crate: none.

## Planner changes

- `.github/ci/rows.toml`: the `xmla` leaf line uncommented. On a post-S4 tree, `market` gains
  `after = ["xmla"]` (its tests link the crate: S4's market half of `xmla/dbtype.rs`).
- `scripts/tests/test_ci_plan.py`: none needed (51 OK with the leaf listed).
- No workflow, exchange script or shard change.

## Decisions where the map is silent or the contract decides

- **`soap/` moves with the crate** (D33: "`yggdryl-xmla`, with `soap/`"; the map's file set), against the
  template's note and AGENTS.md's old row; it is the crate's module `yggdryl_xmla::soap` at
  `rust/xmla/src/soap/mod.rs`, its tests `rust/xmla/tests/soap{,.rs}` keeping the mirror rule, and
  AGENTS.md's row says so.
- **No `from_canonical_rows` forwarder** although the map lists it: AGENTS.md says `Proof::Proven` never
  crosses and the implementer's own doc says a proven landing never crosses, so the rowset lands
  through the public value door.
- The CLI keeps the `xmla serve` verb; `check_wheel_smoke.py` and `stage_cli.py` need no edit.
- Cross-medium core tests that build an XMLA object move whole (the foreground's ruling for Avro), so on
  this tree the s2 pins, the media order and the tail-read check live in `yggdryl-xmla`.
- The bindings link the crate without `http`; the CLI with it.
- `release.yml` is not touched (it publishes `yggdryl` alone; no media move has added a crate to it).

## Findings

1. **The map's `from_canonical_rows` forwarder would let a proof cross the boundary.** The rowset read
   now lands through `Serie::from_scalars`: same rows, one extra `Field::scalar` walk per row, plus the
   settle and order verification every core door makes. No pinned count moves (the XMLA cost pins are
   `IOBase` calls and a write's allocations). A core door that canonicalizes natural rows itself would
   keep one walk; that is a design question, not a move.
2. **The implementer is now grown per item, and S6a's script is not.** On a tree where Excel and XMLA
   landed first, `s6_avro_move.py`'s `edit_implementer` appends its whole section again: it would
   re-export `append/merge/overwrite_arrow_reader_default`, `container_origin`, `container_row_size`,
   `dimension_options`, `own_options` and redefine `arrow_schema_from_field`, `cache_now`, `uuid_parse`
   (E0252/E0428), and its `raise_in`/`move_use_names` and its `docs/media/index.md`/AGENTS pairs (which
   expect "seven") would land in its residue. Its `RESERVED_RANKS`, claim-rule and `media/mod.rs` edits
   match this script's text and skip cleanly. The batch's run of it needs the same per-item guard.
3. **S4 first, then XMLA, composes cleanly; the reverse does not.** S4 then this script: 0 residue, every
   check green, the market half of `xmla/dbtype.rs` installing the crate. This script then S4: S4 misses
   four anchors and leaves the crate's `dbtype` tests naming the market kinds through `yggdryl::`.
4. **Every pinned value moves byte-identical.** The reserved rank 4 keeps the s2 hashes and the order;
   the `xmla` rows of `iobase_calls` and `allocations` moved with their tests, counts unchanged; 847 tests
   moved, 2 added, none lost.
5. **The generator writes an empty `internals` block rustfmt would rewrite.** `yggdryl-xmla` has nothing
   behind `internals`, and S4's generator wrote `pub mod internals {\n}`; this script makes it write the
   one-line `{}` rustfmt keeps, which every later crate with no `internals` module needs too.

## Open questions

1. Finding 1: keep `Serie::from_scalars` (one more walk per row), or add a core door that takes natural
   rows and canonicalizes them itself (one walk, the proof inside the core)?
2. Finding 2: the M46 manager makes `s6_avro_move.py`'s implementer, raise, docs and AGENTS edits
   per-item idempotent before the avro/parquet/s3/iceberg batch, or regenerates its section from what is
   missing - which?
3. Excel lands before XMLA but its define was not available to compose with: this script skips any
   door already exported under the same name and merges its docs clauses with one an earlier move wrote;
   a forwarder Excel names differently would leave two doors to one item. Run the two in order on a copy
   once `s6_excel_move.py` exists.
4. S4 decision 3 re-fixtures `xmla/dbtype.rs`'s market rows rather than moving them: if the manager does,
   no market test names XMLA and `market` needs no `after = ["xmla"]`; if S4's script stays as written,
   the market crate dev-depends on `yggdryl-xmla`. Which does the manager take?
5. D33 says release preflight and AGENTS §6's name list gain the crate in S6c; neither exists yet for any
   leaf (S4 and S6a did not touch `release.yml`). Is that S5's/S9's, or this commit's?
