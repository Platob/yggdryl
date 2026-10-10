# S6d define: `yggdryl-s3`

## State

- Worktree `/home/user/yggdryl-s6-s3`, branch `wip/s6-s3` at `4e46b5ab7`, uncommitted: nothing committed, pushed or published; no cargo command run; no AWS, no network to a store.
- `rust/s3/` and the Iceberg remnant (`rust/tests/s3.rs`, `rust/tests/s3/mod_.rs`) are staged with `git add`; every other change is a working-tree change.
- `git status --short | wc -l`: **146** (50 renamed, 18 added, 2 deleted, 76 modified).
- `git diff HEAD -M --stat | tail -1`: **146 files changed, 3431 insertions(+), 2089 deletions(-)**.
  - The two deletions are `rust/tests/s3/{azure,google}/mod_.rs`, 15- and 6-line files whose path rewrite crosses git's rename threshold; they are re-added at `rust/s3/tests/s3/`.
- `Cargo.lock` is not touched: the first `cargo check` without `--locked` adds `yggdryl-s3`; CI's `--locked` needs that lock update committed.

## Deliverables

- `$S/s6_s3_move.py <tree> [--no-git-lock] [--residue <file>]`, 3675 lines. It imports `$S/s4_move.py` for the lexer, use trees, rewriter and lock, and copies the Avro and Parquet helpers it needs rather than importing those scripts. The parts it was assembled from are under `$S/s6_s3/parts/`.
- `$S/s6_s3_report.md` (this file), `$S/s6_s3_commit_message.txt`.
- Residue: `$S/s6_s3/residue.md` (worktree); logs: `$S/s6_s3/worktree.log`, `bsim.log`, `csim.log`.

## Checks

| Check | Command | Result |
| --- | --- | --- |
| parse | `rustfmt --edition 2024 --check` on each changed or added `.rs` (112) | **0 parse errors**; 38 files differ in formatting only, left for the lane's one `cargo fmt --all` (the same base files are clean) |
| `#[test]` count | `grep -ro '#\[test\]'` over `rust/tests` + `rust/s3/tests` | before 9234 + 0; after 8960 + 276 = **9236** = 9234 + 2 added |
| test names and pins | `$S/s6_s3/tools/pins.py` (every `#[test]` body before and after, install lines and path spellings normalized) | 260 tests moved; **0 numeric differences** in any moved or kept body (every `iobase_calls`, `rust/tests/s3/`, `s3tables` count byte-identical in value); 35 bodies differ in path spellings and strings only; 2 renamed, 2 added (below) |
| no test code in `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/s3/src python/src node/src cli/src` | **0** |
| internals | `python3 scripts/generate_internals.py --check` | `internals are current` (core 129 modules, `yggdryl-s3` 11) |
| inventories | `python3 scripts/check_api_inventory.py` | exit 0, current; 548 `pub` names undescribed (base 569) |
| planner suite | `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | **Ran 51 tests, OK** |
| plan | `plan.check(load_config(), load_workflow())`, `plan.check_leaves(config, root)` | `[]`, `[]`; leaves `['s3']` |
| old paths | `git grep 'yggdryl::s3\b\|crate::s3::\|internals::s3_'` (outside `.handoff/`) | **0**; the `s3tables/` sites are listed below, spelled `yggdryl_s3::` |
| retired feature | `git grep 'feature = "s3"'` outside `.handoff/` and `cli/` | **0** (`cli/src/main.rs:133` is the CLI's own `s3` feature) |
| model identifiers | `git grep -i -E 'claude-(opus\|sonnet\|haiku\|fable)\|opus [0-9]\|sonnet [0-9]\|fable [0-9]'` | **0** |
| second run | the script again on the moved worktree | `rust/s3/Cargo.toml exists ... nothing changed`, exit 0; status and `git diff HEAD` byte-identical |
| caches | `scripts/**/__pycache__` the checks created | removed; `git status` shows no untracked file |

Added tests (`rust/s3/tests/s3/mod_.rs`): `install_claims_the_backend_once_however_often_it_is_called` (four threads and a later call all `Ok`, the backend listed once, `s3` claimed by it) and `a_second_claim_of_the_backend_is_refused_naming_yggdryl_s3` (`claim_backend(&S3_BACKEND, "another")` is `Error::Conflict`, ``expected to create a storage backend at "s3", got an existing yggdryl-s3``, the claim standing). The facade row is `("yggdryl_s3::client", "yggdryl.s3.client")` in `rust/tests/logging/facade.rs`'s existing test.

Renamed: `holder/backend.rs` `mod core_claim` is `mod claim` in `rust/s3/tests/holder/backend.rs`, its two tests `the_object_stores_ten_schemes_are_yggdryl_s3s_claim` and `a_second_claim_of_an_object_store_scheme_names_yggdryl_s3`: the refusal now names `yggdryl-s3`, not the core's `yggdryl`. The core keeps `an_object_store_location_without_its_claim_names_the_crate_to_install`, ungated.

## Batch composition

| Tree | Result |
| --- | --- |
| worktree (base `4e46b5ab7`) | exit 0, residue 1 (the docs runner, below) |
| base + `s6_avro_move.py` + `s6_parquet_move.py`, each committed | exit 0, residue 1; every check above passes; 8943 + 112 + 182 tests become 8672 + 112 + 182 + 273 (+2); 257 moved, 0 numeric differences; leaves `avro`, `parquet`, `s3` |
| base + `s4_move.py` + Avro + Parquet, each committed | exit 0, **residue 0** (S4 moved the docs runner to `cli/tests/`); every check passes; leaves `avro`, `fix`, `market`, `parquet`, `s3`; the installs chain fix, avro, parquet, s3 in the bindings |

The Parquet crate's object-store half is carried: `rust/parquet/tests/s3.rs` and `tests/s3/{file,mod_,path}.rs` lose their `s3` gates and spell `yggdryl_s3::`; `benchmarks/parquet.rs` loses its gates and its `main` installs `yggdryl-s3` after `yggdryl-parquet`; `benchmarks/parquet/s3.rs` spells `yggdryl_s3::`; the crate's `s3 = ["yggdryl/s3"]` forward is gone, `yggdryl-s3` is a dev-dependency, and its `[leaves]` line is `after = ["s3"]`. Its tests build their handles through the crate's own doors, so none needs a claim.

## What the script does

1. Test split (D39), before the moves: the core items that build an object store's handle - 15 items from `aws/environment.rs`, `holder/backend.rs`, `holder/mod_.rs`, `iobase_calls.rs`, `warehouse/{handle,media}.rs` - move with the helpers they name to the same relative paths under `rust/s3/tests/`, each harness written beside them; `FakeS3` stays in `rust/tests/support/server.rs`, `#[path]`-included. `accounting::iceberg` (5 tests) and its fixtures stay at `rust/tests/s3/mod_.rs` under a fresh `rust/tests/s3.rs` gated `s3tables`; the Iceberg suites and the media bench keep their object-store modules, `s3` gates re-keyed to `s3tables`. `aws/environment.rs`'s `internal`, `aws/sigv4.rs`'s three and `xml.rs`'s scanner gates are the core's own items' and are re-keyed (`internals`, none, `aws`).
2. Moves: 54 files by `git mv` - `rust/src/s3/` to `rust/s3/src/` (`mod.rs` to `lib.rs`), `rust/tests/s3{,.rs}` and `rust/tests/interop/s3/` to `rust/s3/tests/`, `rust/benchmarks/holder/s3/` to `rust/s3/benchmarks/s3/`.
3. Crate: `lib.rs` keeps the module's doc, gains `#![deny(unsafe_code)]` and `install()` (`OnceLock` plus a `Mutex`, D7; `claim_backend(&S3_BACKEND, "yggdryl-s3")`); `client.rs` imports the names it read through `sigv4::`/`retry::` and calls the session's crate-private doors through the implementer; links to what the core keeps private are code spans.
4. Core: `pub mod s3`, the register's seed and the `s3` feature are gone; `md-5`, `object_store`, `futures` leave the manifest; the 23 gates re-keyed or deleted (each module is already behind `aws` or `http`; `auth::Bearer` and `http::record_process`'s crate re-exports go); the holder bench drops the module, its stubs and the three groups; `s3tables/catalog.rs`'s four sites spell `yggdryl_s3::`; `logging/facade.rs` `CRATES` gains `("yggdryl_s3", "yggdryl.s3")`.
5. Paths: every `crate::` path of the moved sources by owner, every `yggdryl::s3`, `use yggdryl::s3::{self, ..}` and `yggdryl::internals::s3_*` (to `yggdryl_s3::internals::<module>`) in Rust, pages and skills, file paths in every text file, `#[path]` re-anchored, `s3::file`-style door names in comments and pages.
6. Install: every crate test opens with `crate::install::installed()` (274), the bench `main`, a hidden line in the 13 rustdoc examples of the moved sources (S4's rule), the one page block that reads the register (`docs/holder/index.md`'s storage-backends block), the Python and Node inits after the media installs, the CLI's `main` under `#[cfg(feature = "s3")]`.
7. Manifests: `rust/s3/Cargo.toml` from the measured reach (`yggdryl` with `aws`; `base64`, `hmac`, `md-5`, `ring`, `sha2`, `ureq` at the core's pins; dev `arrow-array`, `arrow-schema`, `criterion`, `futures`, `object_store`, `tokio`; features `parquet` forward (worktree only: the Parquet move takes those tests) and `internals`; one `[[bench]]`), its README, `rust/s3` in the members and `[workspace.dependencies]` at `=0.1.21`, the bindings link it and drop the core's `s3`, the CLI's `s3 = ["dep:yggdryl-s3"]` and a dev-dependency for the pages its tests compile.
8. Tooling and CI: `generate_internals.py`/`check_api_inventory.py` per crate where S4 has not made them so; the docs runner drops `s3`; `[leaves]` `s3 = { package = "yggdryl-s3", jobs = ["object-interop", "azure-interop", "gcs-interop"] }`; the `iceberg` line `after` gains `"s3"`; `x-s3`/`x-azure`/`x-gcs` extend `crate-s3`; the exchange lane builds `rust/s3/Cargo.toml --test interop`; the three drivers run the crate's `interop`; the planner's leaf grammar takes `s3`, its exchange-half test no longer expects the object stores' three, the leaf test allows a leaf its own exchanges.
9. Docs: the holder page's backend register and object-store sections, the AWS identity note, architecture, contributing, testing, benchmarks, plans, Iceberg, the two URI pages, the storage, URI and entry skills, the READMEs, AGENTS.md (smoke row, features, the implementer, `holder/`, `auth/`, `aws/`, backends, `s3tables/` rows, the storage-backend change row, the object-store section, the leaf exchange table), `.api-inventory.txt` (the section re-homed as `### yggdryl_s3  [rust/s3/src/lib.rs]` with `install()`, the implementer's object-store entries).

## Decisions where the design is silent

- The implementer section is 59 items: D36.5's 58 plus `auth::secret::Secret`, because `Bearer::new(impl Into<Secret>)` would otherwise name a type the crate cannot reach (`private_bounds`). Functions and constants are forwarders (no visibility change); types are raised to `pub` inside their private module and re-exported (`RetryBudget`, `Signer`, `Identity`, `EndpointName`, their methods); the session's nine crate-private doors and `ArnPartition::check_region`, `ByteStream::from_handle` are free functions taking the receiver first (`session_signer`, `arn_partition_check_region`, ...). The scanner's `Element` is published as `ScannedElement`, because `implementer.rs` already imports `graph::Element`; the crate aliases it back. Each item is gated as its owner (`http`, `aws`); `Signer::access_key_id` is `pub` under `internals` for the crate's client internals.
- `accounting::iceberg` stays in the core (brief) at its path with a remnant harness, its paths spelled `yggdryl_s3::` so the Iceberg move only moves it; D36.7's "the 11 core-resident pins move to `rust/iceberg/tests/`" is read through D39: the ones building only object-store handles move here, the ones building Iceberg tables stay for the Iceberg move.
- A page block installs only where it reads the register (a location door handed an object-store literal or a variable bound to one, a plan reading one, `backend_for`/`backends`/`claim_backend`); the crate's own doors need no claim, which the crate root's doc now says.
- The crate's tests resolve the core's `aws` gates as held (the crate always links the core with `aws`), so no `aws` feature is forwarded.

## For the Iceberg move

These name `yggdryl_s3` in the core, which cannot depend on it; each waits under `s3tables`:

- `rust/src/s3tables/catalog.rs:507,512,671,674` (`S3Options`, `located_with`, `is_property`)
- `rust/tests/s3.rs` (remnant harness) and `rust/tests/s3/mod_.rs:30,63,73,83,116` (`accounting::iceberg` and fixtures)
- `rust/tests/s3tables/live.rs:26,246,270`
- `rust/tests/iceberg/catalog/mod_.rs:1373,1390,1395`, `rust/tests/iceberg/scan.rs:1827,1922`, `rust/tests/iceberg/staging.rs:254,329` (`rust/tests/iceberg.rs` and `iceberg/table.rs` re-keyed)
- `rust/benchmarks/media/iceberg.rs:1878`

## Residue (worktree)

- `scripts/check_docs_examples.py`: before S4 the Rust pages compile in the core's `rust/tests/docs_examples.rs`, which cannot name `yggdryl_s3`; after S4 (D13) they compile in `cli/tests/`, where the CLI's new dev-dependency links it (residue 0 on the S4-composed tree).

## Open questions

1. The core does not build under `s3tables` between this move and the Iceberg one (above), and both bindings enable `s3tables`, so `--all-features` and the bindings build only once the batch's Iceberg move lands; default and every other feature set build. The batch's one commit hides it; a lane checking between scripts sees it.
2. `FakeS3` stays at `rust/tests/support/server.rs` (D36.7); the `crate-s3` row reads `rust/s3/**` and `config/**`, so a change to the fixture alone runs the core's rows and the three exchanges' (`x-*` extend `crate-s3`, not the fixture), not the leaf's tests. Add the fixture to the leaf's paths, or move it into the crate with the Iceberg move?
3. The CLI's `s3` feature links the crate optionally; its default build stays the schema-only core, and the staged binary's `s3tables` implies `s3`. Keep it optional, or link it always as the bindings do?
4. The exchange lane (`core-interop`) now builds `rust/s3/Cargo.toml`, a feature set of the core (`aws`) no core lane compiles, so the job compiles the core once more; a leaf lane may be the better home for it.
