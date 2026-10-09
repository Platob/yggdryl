# S4 define report: `yggdryl-market` and `yggdryl-fix`

Goal: one script, `s4_move.py <tree>`, that moves the market vocabulary to
`rust/market/` (`yggdryl-market`) and FIX to `rust/fix/` (`yggdryl-fix`). The
lane manager runs it on the program branch after P4 (D38) and P5 (D37) have
landed. It runs no cargo command. The compiler loop (`cargo check --workspace
--all-targets --keep-going --message-format=short`) then starts from the
residue below.

## State

- Script: `$S/s4_move.py` (3,300 lines, stdlib Python). Run it as
  `python3 -I s4_move.py <tree> [--no-git-lock] [--residue <file>]`. It takes
  `$S/git.lock` around its two git steps (`git mv`, `git add -A rust/market
  rust/fix`) and waits up to 15 minutes for the lock.
- Worktree `/home/user/yggdryl-s4`, base `0a24f1806` (wip/s3). It is left
  uncommitted with 385 entries in `git status --short` (159 M, 57 A, 3 D,
  166 R). `git diff HEAD -M --stat`: `385 files changed, 18625 insertions(+),
  13921 deletions(-)`. The 3 D entries are `rust/benchmarks/graph.rs` (shown
  as D plus A because it is mostly rewritten) and the two
  `rust/examples/fix_*.rs`.
- Residue file: `$S/s4/residue.md`, 28 items. The run log is
  `$S/s4/worktree_run.log`.
- The script was also run over `wip/p5` (`53e7d3c21`, which is P4's sweep
  over S3), by the script before its last three residue checks: the same
  383-entry shape as the worktree at that point and the same residue,
  with 0 rustfmt parse errors in 313 files, internals current and
  inventories current. So P4's renames do not disturb it.

## Checks run (no cargo)

| Check | Result |
| --- | --- |
| `rustfmt --edition 2024 --check` parse, every changed `.rs` | 0 parse errors in 315 files (formatting differences expected: run `cargo fmt --all` once after) |
| `#[test]` count, all three crates | 7245 core + 763 market + 1075 fix = 9083, the same as before the move |
| `scripts/generate_internals.py --check` (now one block per crate) | current: 116 core, 16 fix, 7 market modules = 139, the same as before |
| `scripts/check_api_inventory.py` | current |
| Planner suite `test_ci_plan.py` (simulation tree with the program branch's CI files) | 51 OK |
| `plan.check(config, workflow, root)` (simulation) | `[]` |
| Second run on a moved tree | step 0 stops and changes nothing |

Not run: any cargo, the compiler loop, the bindings, the docs runner, and
`Cargo.lock` regeneration. These are the lane manager's.

## What the script does, per step

0. **Guard.** If `rust/market/Cargo.toml` or `rust/fix/Cargo.toml` exists, it
   is a no-op. On a dirty tree it refuses.
1. **Tables.** The names each crate owns are read off the tree as it stands,
   never listed by hand:
   - **Market:** the core root `pub use` of the moved modules, the
     `enum_leaf!` names, the `*_KIND` statics, the `define_field_types`
     markers and the three `delegate_*` macros.
   - **FIX:** `fix/mod.rs`'s re-exports and items, plus `FixCategory`.
   - **Graph:** the 14 market modules leave. `mod.rs`, `element.rs`,
     `column.rs` and `element_column.rs` stay (the event vocabulary).
   - **`internals` aliases** (`fix_codec` -> `fix::codec`, and so on).
   - **Core-private routes:** read from the core implementer's `pub use
     crate::<unpublished>::{...}`, so a `crate::path::Path` in FIX becomes
     `yggdryl::implementer::Path`.
2. **Moves, by glob** (`git mv`, under the lock).
   - `rust/src/fix/**` -> `rust/fix/src/` (`mod.rs` -> `lib.rs`), plus
     `fix_category.rs`.
   - Market root files: `marketdatakind`, `marketdatatype`, `side`,
     `timeinforce`, `idkey`, `identifier`, `idtype`, `idsource`,
     `securityid`, `eusipa`, `limit`, `isin_registry` and its folder ->
     `rust/market/src/`.
   - `rust/src/graph/*` less the four core files -> `rust/market/src/graph/`.
   - Tests and benches for whole moved targets follow:
     - `fix`, `graph`, `isin_registry`, `market_register`, `scale_ulbridge`
     - the root test files of moved sources
     - `bench:fix`, `bench:fix_allocations`, `bench:graph`
   - `rust/tests/fix/ulbridge.log` -> `rust/tests/support/ulbridge.log`
     (D14).
   - The two `rust/examples/fix_*.rs` are deleted (AGENTS: no `examples/`).
   - Because the moves are globbed, P5's `graph/message.rs` and
     `graph/anomaly.rs` move, and its deleted `fix/anomaly.rs` stays
     deleted.
3. **Graph split.**
   - `rust/src/graph/mod.rs` keeps its 6 event-vocabulary items under a
     rewritten module doc.
   - `rust/market/src/graph/mod.rs` gets the other 29.
   - `delegate_event!`, `delegate_market!` and `delegate_operation!` become
     `#[macro_export] #[doc(hidden)]`. They name the core as `::yggdryl::`
     and recurse through `$crate::`.
   - In the market's graph files, `super::<event name>` is re-pointed to
     `yggdryl::graph::`.
4. **Crate roots.**
   - `rust/market/src/lib.rs`:
     - the moved modules' declarations and re-exports from the core
       `lib.rs` (deleted there)
     - `pub mod graph`
     - `#[doc(hidden)] pub mod implementer`
     - `install()`, which claims the four kinds under `"yggdryl-market"`;
       it is idempotent and locked
   - `rust/fix/src/lib.rs` (was `fix/mod.rs`):
     - crate attributes, `pub(super)` -> `pub(crate)`
     - `mod fix_category; pub use fix_category::FixCategory;`
     - `install()`: market first, then the FIX Latest logical names under
       `"yggdryl-fix"`
5. **Core seams.**
   - **`market.rs`.** The seed of the four kinds is gone. `claim` now holds
     a `HELD` table of the five wire numbers the core gave each kind:
     - `(0xc2, marketdatakind, 28, 73, 64)`
     - `(0xc3, side, 29, 53, 28)`
     - `(0xc4, marketdatatype, 30, 74, 65)`
     - `(0xc5, timeinforce, 31, 56, 30)`

     A claim of one of those names or bytes must state that pair and those
     ranks. Every other claim takes `RESERVED_*`. This is how the market
     crate's claims keep every hash and order pin byte-identical, the way
     D39 does for media ranks.
   - **`vocabulary.rs`.** `.chain(crate::fix::LOGICAL_NAMES)`, the
     `market::seed()` call and the doctest's `price` line are removed.
   - **`arrow/extension.rs`.** The market arm and the four `ExtensionType`
     impls are removed (orphan rule: they must live with `Side` and the
     others). In their place:
     - `marker_metadata`/`marker_supports` (now `pub fn`) and the exported
       `#[doc(hidden)] market_extension!` are reached through
       `yggdryl::implementer`
     - each kind file ends with
       `yggdryl::implementer::market_extension!(SideType, Side);`
6. **Paths.** Every `crate::`, `$crate::` and `yggdryl::` path and `use`
   tree is re-owned: 1099 use statements and 1937 inline paths.
   - **Scope:** moved sources, moved and split tests and benches, bindings,
     CLI, and the Rust fences in docs and skills.
   - **Use trees:** parsed into leaves and re-rendered by owner.
   - **Rustdoc fences:** rewritten as code; prose gets inline rewrites.
   - **Market items FIX needs:**
     - In an unpublished module: raised and re-exported through
       `yggdryl_market::implementer` (R route).
     - In a published module: residue (F/A route).
   - **Anchored file paths:** `#[path]`, `include_*!`, and
     `concat!(env!("CARGO_MANIFEST_DIR"), ..)` / `.join` chains are
     re-anchored through the move table, with the split copies overlaid.
   - **Plain-text file paths:** rewritten in 133 text files. Commands
     naming a moved target (`-p yggdryl --test fix`) become
     `-p yggdryl-fix --test fix`.
   - **Doc links:** core doc links to moved items, and market doc links to
     FIX, become plain code spans (residue: reword).
7. **Cost rows and mixed test files (D39), run before the moves.**
   - **The rule:** every top-level item of every `rust/tests` file whose
     names reach a higher crate moves byte-identical to
     `rust/<crate>/tests/<same path>`.
   - **What counts as reaching a crate:** imports, inline paths, inner
     `use`s, a parse of a kind or FIX Latest name, a schema expression
     typing a field with one, and a parse door fed a kind name held in a
     variable.
   - **Other mechanics:**
     - inline modules are recursed
     - helpers are copied where both sides use them
     - imports are pruned on each side
     - split harnesses declare only `support/` helpers
   - **Result:** 32 files split. The full list is in the residue file's
     "Done" section, which covers both the cost targets
     (`allocations.rs` 79 items, `iobase_calls.rs` 3) and the
     root/arrow/iceberg/xxhash/xmla/value/serie/json/expression files.
8. **Install.**
   - **Tests:** each new crate gets `tests/support/install.rs`
     (`installed()`), and each moved `#[test]` calls it first. That is 21
     harnesses and 1835 tests.
   - **Benches:** each moved bench's `criterion_main!` becomes an explicit
     `main` that installs.
   - **Rustdoc examples:** each one in the moved crates gets a hidden
     `# yggdryl_market::install().unwrap();` (or the fix one).
   - **Docs and skills:** each Rust page block naming a moved crate gets a
     visible `yggdryl_fix::install()?;` (or the market one): 204 blocks.
   - **Bindings and CLI** install both crates in dependency order:
     - Python `_native`: `map_err(value_error)?`
     - Node `module_init`: `.expect`
     - CLI `main`: a refusal printed through `style::bad`, then
       `ExitCode::FAILURE`
9. **Manifests.**
   - `rust/market/Cargo.toml` and `rust/fix/Cargo.toml` are written, with
     dependencies computed from the crates each source names, and features
     `http`, `parquet`, `iceberg` and `internals` forwarding to the core
     (and market). Benches and READMEs move.
   - Core: `exclude = ["/market", "/fix"]`; the moved bench blocks are
     removed.
   - Workspace: members, plus `[workspace.dependencies]` with `yggdryl`,
     `yggdryl-market` and `yggdryl-fix` at `=0.1.21` (the version is read,
     not spelled).
   - Bindings and CLI: `{ workspace = true, features = ["http"] }`. The CLI
     dev-dependencies cover the docs runner.
10. **Tooling.**
    - `generate_internals.py` handles one block per crate (`crates()`,
      `crate_name()`) and is run.
    - `check_api_inventory.py` handles every crate; leaf sections also read
      the core's tokens, for macro-generated names.
    - `check_docs_examples.py`: the Rust target is now
      `cli/tests/docs_examples.rs`, run with
      `-p yggdryl-cli --features "yggdryl/parquet ..."` (D13). `.gitignore`
      moves with it (CRLF kept).
    - `generate_fix_dictionary.py` and `check_isin_seed.py` are re-pointed.
11. **CI table.**
    - `[leaves]`: `market` and `fix` (`after = ["market"]`) are uncommented.
    - `[shards.yggdryl]` is re-split into `[shards.yggdryl-market]` (`full:
      allocations, test:graph`) and `[shards.yggdryl-fix]` (`default` and
      `full: fix, bench:fix`, `allocations, test:scale_ulbridge`).
    - `rows.libs`: the capture path is updated.
    - Planner test: see "Planner and CI table" below.
12. **Inventory.**
    - `.api-inventory.txt` section headers and paths are re-homed.
    - The graph section is split, and its market half becomes `###
      yggdryl_market::graph [rust/market/src/graph/mod.rs]`.
    - New `### yggdryl_market [rust/market/src/lib.rs]`: `install`, the four
      markers and the extension types.
    - `install` is added under `### yggdryl_fix [rust/fix/src/lib.rs]`.
13. **Residue scan.** It writes `--residue`, then `git add -A rust/market
    rust/fix` under the lock.

Decisions the script takes:
- **Stay core:** `market.rs`, `plugin.rs`, `parallel.rs` and `txhash/`.
- **Stay put, re-pointed:** `config/fix/`, `config/isin/`, `docs/`,
  `skills/` and the generator.
- **The extension name stays `yggdryl.side`.**
- **Raised, then re-exported** through `yggdryl_market::implementer` (R
  route): `graph::facts::OperationEventFacts`,
  `idtype::names_another_instrument`, `isin_registry::{EconomicMemo,
  IsinTable, warn_full}`.

## Residue for the compiler loop (`file:line` in the worktree; lines shift after P4/P5)

**Compile errors (the compiler will list these too).**
- **Fix reaches `pub(crate)` market items in published modules.** These need
  a forwarder in `yggdryl_market::implementer` (S3's F/A route), or
  disappear under D37:
  - `rust/fix/src/enrich.rs:84`: `graph::iterator::order`
  - `rust/fix/src/market.rs:17`: `graph::market::base_crosscode`
  - `rust/fix/src/msg.rs:2194`: `graph::market::merge_operation_event`
  - `rust/fix/src/msg.rs:6411`: `graph::market::restating_operation`
  - `rust/fix/src/msg.rs`, the `identifier` helpers:
    - `575`, `580`: `is_word`
    - `583`: `folded_len`
    - `3987`, `3988`, `4018`, `4019`, `5497`, `5498`: `WORD_PAIR_WIDTH` and
      `fold_into`
- **Core rustdoc examples naming moved items.** The core cannot name them,
  so re-fixture on a core type or drop the line:
  - `rust/src/enums.rs:637,640`: `Side::dtype()`. Drop it; `State` already
    shows the point.
  - `rust/src/graph/column.rs:26-41`: `OrderEvent`. Re-fixture on
    `TextLine`, the core's one `Event`.
  - `rust/src/graph/element_column.rs:24-45`: `OrderEvent`, same.
- **Core benches naming moved items** (D39 move, or re-fixture per D14):
  - `rust/benchmarks/media/iceberg.rs:1880,1995`
  - `rust/benchmarks/text/line.rs:452`
  - `rust/benchmarks/types/datatype/serie.rs:18`
  - `rust/benchmarks/types/datatype/value.rs:4,5,12,14`
- **Method-reached `pub(crate)` items.** Only the compiler sees these
  (E0624). Candidates by name, a market `pub(crate)` method FIX calls:
  - **`graph/facts.rs`:** `facts`, `from_facts`, `into_event`,
    `set_marketdatakind`, `settle_orders`
  - **`graph/` elsewhere:**
    - `with_kind` (`operation.rs:721`)
    - `with_placing` (`iterator.rs:707`)
    - `as_event` (`market_data.rs:531`)
    - `event` and `quantity` (`book.rs`)
  - **`IsinRegistry`:** `as_table`, `fill_identifiers`, `fill_unsettled`,
    `into_row`, `learn_stating`
  - **Identifier types:**
    - `IdSource::from_namespace`
    - `held_at` (`identifier.rs:1174`)
    - `IdKey::infer`, `read` and `value_into`
    - `SecurityId::identifier`
    - `IdType::underlying_security`
    - `MarketDataKind::stored_side`

  S3 counted 31 items (15 paths, 16 methods). D37's public message removes
  most of the `facts` family.

**Prose (no compile effect).**
- **Market doc links to FIX.** The market cannot link `yggdryl_fix`, so
  reword:
  - `rust/market/src/graph/arrow.rs:146`
  - `idtype.rs:331`
  - `isin_registry/env.rs:33`
  - `isin_registry.rs:2905`
  - `marketdatatype.rs:30`
  - `timeinforce.rs:17`
- **Core doc mentions of moved items** (D14 reword):
  - `rust/src/fisn.rs:107`
  - `isin.rs:25,37`
  - `graph/element.rs:748`
  - `hashing/mod.rs:9`
  - `string.rs:2364,2374,2393,2394`
  - `text/options.rs:607`
- **Unsorted imports.** The rewritten `use` lists are not sorted. Run
  `cargo fmt --all` once after the last worker.

**Tree-specific.**
- `.github/ci/rows.toml` does not exist in wip/s3, so the worktree carries
  no CI-table change. On the program branch, step 11 applies; it was proven
  in the simulation.

**Runtime only (no compile error; the whole run finds these).**
- **Claimant pins.**
  - `rust/market/tests/market_register.rs:202,203` expects `"yggdryl"` as
    the claimant of `side`/`yggdryl.timeinforce`. It becomes
    `"yggdryl-market"`.
  - `market.rs`'s refusal text at line 560 ("which the core's kinds alone
    claim as") is stale. Reword it.
- **Core `StringEnum::PREBUILT`** (`string.rs:2408`) still keys `side` and
  `timeinforce`. `StringEnum::from_logical_name("side")` now needs the market
  claim, because it calls `DataType::from_logical_name` first. Open question
  2.
- **Any remaining core test or page block** that reads a kind or FIX
  Latest name by a door the scan does not know. The scan covers literal
  doors, schema grammar, variable-fed doors, and `helper("side", "...")`
  calls. The Python and JavaScript pages need no change, because the init
  installs.

## Tests moved (D39) and what the move did not do

- **Whole targets moved:**
  - `fix` (all of `rust/tests/fix/`), `graph`, `isin_registry`,
    `market_register` and `scale_ulbridge`
  - the root test files of the moved sources (`eusipa`, `identifier`,
    `idkey`, `idsource`, `idtype`, `isin_registry`, `limit`, `market`,
    `marketdatakind`, `marketdatatype`, `securityid`, `side`,
    `timeinforce`)
  - `support/allocations.rs` and `support/ulbridge.rs`, to the fix crate
- **Split by item into the crates' own targets:** 32 core test files (see
  the residue "Done" list). They are byte-identical, `//!` naming the core
  file they pin.
  - `rust/market/tests/root.rs` declares 13 root files.
  - Split harnesses exist under `rust/market/tests/` and `rust/fix/tests/`
    for `allocations`, `iobase_calls`, `arrow`, `expression`, `iceberg`,
    `json`, `serie`, `value`, `xmla`, `xxhash`, `graph` and
    `isin_registry`.
- **`rust/tests/graph/{element,column,element_column}.rs` moved with the
  graph target.** They pin core files, but their fixture is `OrderEvent`,
  so D39 puts them in the market crate. The core then has no test of its
  `graph/` files. A re-fixture on `TextLine` would bring them back (D14).
  This is a judgement left to the lane manager.
- **D14 asks to re-fixture, not move, core tests that use a kind only as a
  fixture.** The script moved them, since a move is mechanical and keeps
  coverage: CI runs every crate on a core change. The candidates for a
  hand re-fixture onto the test-only kind are:
  - `root/cast.rs` (7)
  - `xxhash/arrow.rs` (12)
  - `xxhash/scalar.rs` (5)
  - `xmla/dbtype.rs` (7)
  - `root/enums.rs` (6)
  - `value/canonical.rs` (2)
  - `serie/order.rs`
  - `iceberg/{mod_,partition,value}.rs`
  - `json/field.rs` (3)
  - `root/valuestream.rs` (the whole-corpus round trip, which moved
    because its corpus holds two kind rows: better to keep the core rows
    and move the two)

## `implementer` items beyond S3's 114

- **Core `yggdryl::implementer`, written by the script:**
  - `ExtensionType` (arrow-rs)
  - `ArrowError`
  - `ArrowDataType`
  - `marker_metadata` and `marker_supports` (`arrow::extension`, raised to
    `pub fn` in the now-`pub(crate)` module)
  - `market_extension!`

  That is 6 new names.
- **`yggdryl_market::implementer`, written by the script:**
  - the 3 `delegate_*` macros
  - 5 R-route re-exports: `OperationEventFacts`, `names_another_instrument`,
    `EconomicMemo`, `IsinTable`, `warn_full`
- **Still needed (residue):**
  - 8 F/A forwarders: `order`, `base_crosscode`, `merge_operation_event`,
    `restating_operation`, `WORD_PAIR_WIDTH`, `fold_into`, `folded_len`,
    `is_word`
  - the method-reached list above, after P5

  Each route is one line in that file plus a `pub` raise in an unpublished
  module, or a forwarding `pub fn`.
- **Possible core-private reaches:** inherent methods only the compiler
  sees, the same Tier C S3 named. The static check `t_reach` found 0 path
  reaches from the new crates into core-private items that are not routed.

## Ordering with P4 and P5

- **Run order:** S3, then P4, then P5, then S4, on the program branch. The
  program branch (`ccr-0fe6f9d0-ruymat`) does not contain S3 yet. The
  script reads S3's `rust/src/implementer.rs` routes and needs its D5/D9
  seams.
- **Nothing in the script names P4's or P5's files or names.** Moves are
  globbed and tables are read from the tree. Neither the old (`currunix`…)
  nor the new (`transunix`/`sendunix`/`uuid`/`hashcode`) names are spelled.
  Over `wip/p5` (P4 applied) the result and residue were identical.
- **P5 adds `graph/message.rs` and `graph/anomaly.rs` and deletes
  `fix/anomaly.rs`.** Message and anomaly leave with the market graph
  unless P5 names them as core event vocabulary. `CORE_GRAPH_FILES` is
  `{mod, element, column, element_column}`; extend it if P5 puts
  `anomaly.rs` in the core's half.
- **P5's new `pub(crate)` items that FIX reaches** surface as R-route
  raises (unpublished module) or F/A residue (published module). No list
  needs editing. D37's public `MarketMessage` should retire most of the F/A
  set and the `facts` methods.
- **Re-check after P5:** `GENERIC_FIX_NAMES` and `CORE_GRAPH_NAMES`, in case
  P5 adds an event-vocabulary item to `graph/mod.rs` that must stay core.
  The split keeps every item of `graph/mod.rs` whose name is in the event
  vocabulary (`Element`, `Event`, `ElementColumn`, `EventColumn`, ...) and
  moves the rest.

## Planner and CI table

- **`scripts/tests/test_ci_plan.py`:**
  - The capture path in `test_the_capture_the_bindings_replay_runs_them_too`
    becomes `rust/tests/support/ulbridge.log`.
  - `test_the_named_shards_are_targets_the_core_has` becomes
    `test_the_named_shards_are_targets_their_package_has`. It walks every
    `[shards.<package>]` against that package's folder (`rust/` or
    `rust/<leaf>/`), its `tests/*.rs` and its `[[bench]]`s, and uses
    `lanes.get(lane, {})`. Without this, the core's emptied `default` lane
    raised `KeyError`.
- **`.github/ci/rows.toml`:**
  - the `market`/`fix` leaves are uncommented
  - the shards move with their targets: the core keeps only `full = {
    allocations = ["test:allocations"] }`
  - `rows.libs.paths` takes the capture's new path
- **`plan.py`:** no change. It already reads `config.shards[leaf.package]`
  for a leaf job (line 505).

## Open questions

1. **`Cargo.lock`.** It must be regenerated (two new workspace members, new
   dev-dependency edges) before CI's `--locked`. Run the first `cargo check`
   without `--locked`. Also confirm `cargo package --list -p yggdryl` honours
   `exclude = ["/market", "/fix"]`.
2. **`StringEnum::SIDES` / `TIMESINFORCE` and the `PREBUILT` rows for `side`
   and `timeinforce`** stay in the core's `string.rs`, but the names they key
   now resolve only after the market's claim. Either move the two listings
   to the market crate (with a claim-time registration), or leave them and
   accept that `from_logical_name("side")` needs `yggdryl_market::install()`.
3. **Re-fixture or move** for the D14 candidates above and the three
   `graph/` column tests. The script moved them. Choose per item.
4. **AGENTS.md.**
   - The script re-pointed paths only. D13's per-crate sections are still
     to write:
     - The Layout rows for `graph/`, `fix/`, `idkey.rs`, `identifier.rs`,
       `idtype.rs`/`idsource.rs`, `isin_registry.rs`, `eusipa.rs`,
       `limit.rs`, `side.rs`, `timeinforce.rs`, `marketdatakind.rs` and
       `marketdatatype.rs` say "under `rust/src/`".
     - The register text says "the core claims its own four kinds itself".
   - `docs/contributing.md` changes with it.
5. **Example files.** The two deleted `rust/examples/fix_*.rs`: confirm
   nothing in docs or CI runs them. The script re-points text references but
   does not fold their content into a page.
6. **Binding and CLI features.** They depend on `yggdryl-market` and
   `yggdryl-fix` with `http` only. If a binding uses market or FIX surfaces
   under `parquet` or `iceberg` (the ISIN registry's Iceberg store), it
   needs those features forwarded too. The compile will say.
7. **`OperationEventFacts`.** It is raised to `pub` with `pub(crate)` fields.
   FIX constructs it, so its fields may need raising, or D37 removes the
   need.
8. **D7 for pure-Rust callers.** D39 says a pure-Rust caller installs what
   it links. Tests, rustdoc examples, pages and skills now do, but no market
   or FIX public entry installs lazily. Confirm that is wanted (it is the
   current design).
9. **The market's runtime `serde_json` dependency.** The computed manifest
   lists `serde_json` as a market dev-dependency only. If a market source
   uses it at runtime, the compiler will name it.
