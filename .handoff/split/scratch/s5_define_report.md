# S5 define report: `yggdryl-market` for Python and npm, the release learns every crate

Goal: lay out the two market binding packages over the `yggdryl-market` /
`yggdryl-fix` crates and teach `ci.yml` and `release.yml` every crate and
package, as one idempotent script run on an S4-landed tree.

## Deliverables

| Path | What |
| --- | --- |
| `$S/s5_move.py` | the move: `python3 -I s5_move.py <tree> [--no-git-lock] [--residue <file>]`; stdlib only, reuses `s4_move.py`'s `read`/`write`/`git`/`GitLock` unedited; exact-string anchors asserted once (a miss is residue, never a guess); step 0 refuses a dirty tree and makes a second run a no-op; `$S/git.lock` around every git step; runs no cargo, maturin, napi or npm (it runs `node` once, for the generator it writes) |
| `$S/s5_commit_message.txt` | the commit message, the two trailers last |
| `/home/user/yggdryl-s5` (`wip/s5`, base `ef4e241a4`, no S4) | the script's output, uncommitted: 183 status lines, `183 files changed, 5292 insertions(+), 3078 deletions(-)`, 0 untracked, 0 ignored |
| `$S/s5/worktree_residue.md`, `$S/s5/run_residue.md` | residue of the worktree run and of the S4-simulated run |
| `$S/s5/sim.sh`, `$S/s5/run.sh` | rebuild the S4-simulated copy (`s4_move.py` over `ef4e241a4`) and run the move on a copy of it |

## Decisions

- **Native half (D11 option d).** One `yggdryl._native` and one napi addon
  keep linking every crate; both market packages are pure language layers. The
  one native edit: `python/src/scalar.rs` imports the four market enum classes
  from `yggdryl.market.<kind>`, and `classify` treats a missing `yggdryl.market`
  as "no such class" (`market_class`), so a core installed alone still encodes
  every other `Enum`. Node's native sources are unchanged. `_native.pyi` stays
  whole in the core; its four relative imports become
  `from yggdryl.market.<kind> import ...`.
- **Python packaging.** `python/market/pyproject.toml`, `yggdryl-market`,
  backend `flit_core>=3.12,<4`, `[tool.flit.module] name = "yggdryl.market"`,
  `dependencies = ["yggdryl==<version>"]`. Why flit_core: pure Python, no
  dependency of its own, static PEP 621 metadata, and a dotted module name
  builds the namespace portion alone (no `yggdryl/__init__.py` to shadow the
  core's) and installs editable as one `.pth` entry `extend_path` joins.
  Hatchling needs explicit `only-include` config to the same end; maturin
  builds native code. Proven: the built wheel holds `yggdryl/market/**` and
  dist-info only (20 entries), `Requires-Dist: yggdryl==0.1.21`.
- **Namespace.** Core `yggdryl/__init__.py` gains
  `__path__ = __import__("pkgutil").extend_path(__path__, __name__)`;
  `yggdryl.market` exports what the core exported for the market (`fix`,
  `graph`, `side`, `timeinforce`, `marketdatakind`, `marketdatatype`,
  `identifier`, `isin_registry`, `eusipa`) and `yggdryl.market.enums` the five
  market listings (`MARKET_KINDS`, `MARKET_VIEWS`, `MD_UPDATE_ACTIONS`,
  `MARKET_COLUMNS`, `OPERATION_COLUMNS`); `ELEMENT_COLUMNS`/`EVENT_COLUMNS`
  stay core. `State.from_fix_status`/`from_fix_msgtype` become
  `yggdryl.market.fix.state_from_status`/`state_from_msgtype`, no alias; their
  7 test cases move to the market's `test_fix.py`.
- **mypy.** `python/pyproject.toml` gains
  `mypy_path = "$MYPY_CONFIG_FILE_DIR:$MYPY_CONFIG_FILE_DIR/market"` and
  `explicit_package_bases = true` (without it mypy reports the source found
  twice). CI proves the merge twice: the `python` job's existing `mypy --strict`
  (its typing pins import `yggdryl.market`), and the new `python-market` job
  over `python/yggdryl python/market/yggdryl/market` plus both pins.
- **Node.** `node/market/` is `yggdryl-market`, peer `"yggdryl": "<version>"`
  exact, devDependency `"yggdryl": "file:.."`. The core's `binding.js` takes
  the market natives off its surface and hands them over through the
  `yggdryl/implementer` subpath (`exports["./implementer"]`), as Rust's
  `implementer` does. `node/scripts/market-natives.js` classifies the addon's
  natives from the Rust binding sources and writes the hand-over list,
  `node/market/index.js` and `index.d.ts` ("as the core's are generated");
  `build`/`build:debug` run it, `test:package*` run `--check`. `book.js`,
  `book.d.ts`, `book/` move. The core `index.d.ts` exports `NamedField` and
  `FieldOptionsInput`, which the market's factories type with; `FixDirection`
  and `field.fix` stay core (the FIX protocol view is `node/src/field.rs`).
  `node/market/package-lock.json` is derived from the core's lock (9 entries,
  same versions and hashes, the core linked), nothing resolved afresh.
- **Tests.** Mirrored: `python/market/tests/` (own `conftest.py`, the moved
  `medallion.py`, `test_init.py`, `test_enums.py`, and a copy of the benchmark
  smoke over `python/market/benchmarks/`); `node/market/tests/`. Node's core
  suite cannot resolve `yggdryl-market`, so every Node D14 site is handled in
  the script: the factory table, type pins and vocabulary listing drop the
  market kinds (the market suite pins them), two member-lookup benchmarks move
  to the market's `graph.js`, and the two FIX-driven logging tests
  (`lib.test.js`, removed, and one of `logging.test.js`) move to the market's
  `fix.test.js`, each child handed both packages.
- **CI.** `rows.toml`: `python-market` (extends `wheel`) and `node-market`
  (extends `addon`, `cli`); the `[leaves]` lines of `market` and `fix` name both
  jobs; `rows.cli` reads `node/market/book/**`. `ci.yml`: the two jobs, the
  market package installed beside the core in `python`, `python-freethreaded`,
  `docs-python`, `node`, `docs-javascript`, the inventory job running
  `release_packages.py version`, both in `CI result`'s needs.
  `test_ci_plan.py`: pins updated and
  `test_a_market_package_change_runs_its_suite`.
- **Release.** `scripts/release_packages.py` is the one list: `version` (every
  manifest, pin, peer and npm lock agree, else names each), `crates` (every
  publishable workspace crate in dependency order, dev-dependencies included,
  ties by member order: `yggdryl yggdryl-market yggdryl-fix` on S4; the S6
  crates join as their manifests land), `pypi`, `npm`. `release.yml`:
  preflight reads every name on every registry (sparse index, PyPI JSON, npm)
  and refuses a version some hold and others lack; `sources` dry-runs every
  crate in one `cargo publish --dry-run -p ...`; a `market-packages` job builds
  the market wheel and sdist once and asserts namespace-only plus the exact
  pin; every platform installs and smoke-tests both (`check_wheel_smoke.py
  --market`, `npm test --prefix node/market` against the release addon); PyPI
  and npm publish `yggdryl-market` after the core, idempotently; `release`
  publishes each crate in order, skipping what the index holds; the report
  reads every name. `cli/Cargo.toml` gains `publish = false`.
- **Docs, skills, AGENTS.md.** Python and JavaScript blocks under `docs/` and
  `skills/` re-spelled (`yggdryl.market`, `require('yggdryl-market')`); the docs
  runner installs the market package; `.api-bindings.txt` re-homes 44 Python
  and 34 JavaScript entries under `### python: yggdryl.market` and
  `### javascript: yggdryl-market`; AGENTS.md §2, §3, §4, §6 (13 edits; the bump
  list is ten files, `preflight` reading seven).
- **D12 names.** `yggdryl-market` on crates.io, PyPI and npm.

## Checks

| Check | Result |
| --- | --- |
| script on the S4-simulated copy | rc 0; 183 status lines, `183 files changed, 5292 insertions(+), 3078 deletions(-)`; residue: 6 Python D14 files |
| second run (copy and worktree) | no-op, status unchanged |
| `python3 -m unittest discover -s scripts/tests -p test_ci_plan.py` | 52 tests OK (copy and worktree) |
| `plan.check(load_config(), load_workflow())`, `check_leaves` | `[]`, `[]` (both trees) |
| `python3 scripts/check_api_inventory.py` | current, rc 0 |
| tomllib: both pyprojects, `rows.toml`, `cli/Cargo.toml`, `Cargo.toml`, `rust/{market,fix}/Cargo.toml` | parse |
| `node -e` JSON: `node/package.json`, `node/market/{package.json,package-lock.json,tsconfig.json}` | parse |
| PyYAML (`python/.venv`): `ci.yml`, `release.yml` | parse |
| test code under any `src/` | 0 |
| model identifiers | 0 added (12 pre-existing lines in AGENTS.md and `.handoff/`) |
| worktree after the script | 183 status lines; `183 files changed, 5292 insertions(+), 3078 deletions(-)`; residue: S4 not landed (the `[leaves]` lines stay commented, `crates` lists the core alone) plus the 6 |
| `node --check` new/edited JS; `market-natives.js --check` | pass; rc 0 |
| `tsc --noEmit` core and market (9 market type pins), main repo's TypeScript | 0 errors each |
| `node --test`, core + market, prebuilt P4 addon wired in scratch | 1122 tests as before; failing set identical to the pre-S5 tree (79, P3 tests vs P4 addon) |
| `pytest`, prebuilt P4 native + 4 scratch shims standing in for the `scalar.rs` edit | failing set identical to the pre-S5 tree but 2 simulation artifacts (no installed `yggdryl-market` metadata; the shims) |
| `mypy --strict` both packages + both typing pins | Success |
| market wheel and sdist (flit_core, scratch venv) | namespace only, exact pin |
| derived market lock | `npm install --package-lock-only --offline` leaves it byte-identical |
| `rustfmt --check` on edited `.rs` | no new diff (the sim's 2 are S4's import order) |
| `release_packages.py version` with one lock version moved | refuses, naming the file |
| not run | any cargo, maturin, napi or npm build; `cargo check` of the `scalar.rs` edit; CI |

Registry checks (read-only, all run): crates.io - `yggdryl` 0.1.21 published
(`cargo search yggdryl` lists only it); `yggdryl-market`, `-fix`, `-excel`,
`-xmla`, `-avro`, `-parquet`, `-s3`, `-iceberg`, `-cli` 404 (API; sparse index
404 for `-market`, `-fix`). PyPI - `yggdryl` 0.1.21 latest; `yggdryl-market`
404 (`pip index versions`: no matching distribution). npm - `yggdryl` 0.1.21;
`yggdryl-market` and `yggdryl.market` 404 (`npm view`).

## Findings

1. The native module named the market's Python classes by path
   (`python/src/scalar.rs` `classes!`): without the edit, every market enum
   value's `as_py` and every Python `Enum` the encoder classifies raise
   `ModuleNotFoundError` once the modules move. The edit is rustfmt-clean but
   uncompiled: `cargo check -p yggdryl-python --all-targets` is its first proof.
2. Node's core suite cannot reach `yggdryl-market` (no link from the core), so
   its D14 sites are moved or re-fixtured by the script; Python's core suite
   still reaches the market in 6 files, 77 sites (60 in `typing_bindings.py`),
   passing only because CI installs the market beside the wheel - the lane's
   D14 call.
3. New names need repository configuration before the first publish: a PyPI
   pending trusted publisher for `yggdryl-market` (environment `pypi`); npm
   trusted publishing is configured on an existing package, so the first
   `yggdryl-market` publish needs a bootstrap (verify on npmjs.com); and
   `CARGO_REGISTRY_TOKEN` needs the `publish-new` scope over `yggdryl-*` for the
   eight new crates.
4. `cargo publish --dry-run` over several `-p` needs cargo 1.90 or later
   (MSRV 1.94 holds it); `yggdryl-cli` had no `publish = false` and would have
   joined the crate list.
5. `cli/src/market.rs` embeds `node/book/` with `include_bytes!`; re-pointed
   to `node/market/book/`, without which the CLI stops compiling.

## Open questions

- npm spelling: `yggdryl-market` chosen (D12); `yggdryl.market` is also free.
- Second native extension: not needed; option (d) holds, one native edit.
- Rehearsal: a manual `release.yml` run publishes nothing; it needs the
  configuration in finding 3 only for a real publish.
