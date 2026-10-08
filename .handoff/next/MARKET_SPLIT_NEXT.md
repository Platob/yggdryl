# Handoff: split yggdryl into seven crates

Read `.handoff/next/MARKET_SPLIT_PROMPT.md` (the program) and
`.handoff/split/DESIGN.md` (the decisions, the ledger, the pins, the bench
baselines, what S1 built) before any edit. Where this file and those differ,
the committed files hold; where the prompt text a session was given differs
from `MARKET_SPLIT_PROMPT.md`, the committed file holds.

## Goal

Split the core into seven crates - `yggdryl`, `yggdryl-market`,
`yggdryl-fix`, `yggdryl-avro`, `yggdryl-parquet`, `yggdryl-iceberg` (with
`s3tables`) and `yggdryl-excel` (the user's third instruction, D24) - the
split first and the adaptations after, one slice per session, one commit per
slice, on one program branch under one draft PR, nothing published and no
live AWS resource touched.

## Invariants

- The program branch is `ccr-0fe6f9d0-ruymat` (harness-assigned, D1) and
  the draft PR is #209. Never push to `main`, never tag, never run a
  release, never mark the PR ready or merge it, never publish (no
  `cargo publish` without `--dry-run`, no `npm`/`maturin`/`twine` publish).
- Each session starts with `git fetch origin && git switch
  ccr-0fe6f9d0-ruymat && git merge origin/main`, never rebasing a pushed
  slice; a conflict is resolved by re-running the slice's sweep over the
  merged tree and the merge is recorded here.
- No back-compat: a replaced name, spelling, test or doc is deleted in the
  same commit; no alias, shim or dual reader. One owner per fact; the core's
  register is the one place a kind or a medium is claimed.
- The seventeen codes are the core's own flat variants (D25); the market
  register holds enum kinds alone. S4 moves the four enums, `graph/` and the
  ISIN registry into `yggdryl-market`, never a code.
- The wire contracts S0 pinned are byte-identical through every slice: the
  `DataTypeId` bytes, the ranks, the `Shape` positions, the hash feeds, the
  serde documents, the value-stream bytes, the Arrow extension names and the
  FIX dictionary hash `14_542_711_836_201_211_247`. A moved pin is a
  defect, never a re-pin; a cost pin (`allocations.rs`, `iobase_calls.rs`,
  a bench) is never re-pinned from a sweep.
- No test code under any `src/` (`grep -rn '#\[cfg(test)\]\|#\[test\]\|mod tests' rust/src rust/*/src python/src node/src cli/src` is empty before every commit).
- Node gains no door; the slices re-spell and re-pin the doors that exist.
- Layer order inside a slice: Rust core -> Python -> Node -> docs, settled by
  the smoke loop, then one whole run in the `--all-features` lane leading
  the background chain, then push, then the CI run read to the end.
- The model fits the step (the user, mid-S1: "use sonnet when you scan so
  much"): every scan, classification, check-and-report and mechanical sweep
  runs on `sonnet`; a merge of a core file and a review on `opus`; the
  design and the edits no script makes stay in the foreground.

## State

| Fact | Value |
| --- | --- |
| Program branch | `ccr-0fe6f9d0-ruymat` |
| HEAD | `eeb4ec14d` ("Open the market extension point", S1) |
| Draft PR | #209, draft |
| Base | `origin/main` at `2ae975674`; no merge of `origin/main` was needed this session (nothing landed on `main` since) |
| Slices done | P0 `3d8bf84d9` (D22), P1 `08ae4c6b7` (D23), S0 `6d71a36ee` (the pins), S1 `eeb4ec14d` (the market extension point) - each pushed alone, its CI read green before the next |
| Next | S2, the media extension point |

What S1 built is the "S1: what was built" section of DESIGN.md: the four
`Market` variants, `MarketDescriptor`/`MarketType`/`MarketScalar`/
`MarketSerie`/`MarketValue`, the `DataTypeId` newtype, `plugin.rs`'s
`Register<K, V>`, `market.rs`'s register and `unregistered`, the logical-name
register, the four enum kinds claimed in place - the seventeen codes stay the
core's own flat variants (D25, the user's instruction mid-slice) -
`rust/tests/market_register.rs` and `rust/tests/root/{market,plugin}.rs`, the
two allocation rows, the bindings re-spelled with no Python or JavaScript
name change, the docs, skills, inventories and AGENTS.md following.

## Checks

Every command below ran on the tree committed as S1, from `/home/user/yggdryl`,
with `CARGO_INCREMENTAL=0` and the debug info off; a background chain holds the
cargo lock for the long steps and the foreground ran the rest.

| Check | Command | Result |
| --- | --- | --- |
| it builds | `cargo check --workspace --all-targets --all-features --keep-going --message-format=short` | clean: 0 errors, 0 warnings, the four crates |
| the register | `cargo test -p yggdryl --test market_register` | 7 passed |
| the root files | `cargo test -p yggdryl --test root` | 1681 passed |
| the series | `cargo test -p yggdryl --test serie` | 338 passed |
| the value contracts | `cargo test -p yggdryl --test value` | 25 passed |
| the digests | `cargo test -p yggdryl --test xxhash` | 81 passed |
| the dictionary hash | `cargo test -p yggdryl --test fix store` | 83 passed; `14_542_711_836_201_211_247` unmoved |
| the cell and ingest rows | `cargo test -p yggdryl --test allocations -- leaf_cell registered prebuilt` | 4 passed |
| the call counts | `cargo test -p yggdryl --test iobase_calls` | 37 passed |
| the private pins | `cargo test -p yggdryl --features internals --test root` | 1852 passed |
| the whole run | `cargo test -p yggdryl --all-targets --all-features --no-fail-fast` | 70 targets; 8968 passed in the 69 green ones; `allocations` 179 passed, 5 failed - `DECLARED_READ`, `ORDER_PARSE` and the three `sort_by` rows, each seven allocations under its pin, the grammar's deliberate move - re-pinned with that sentence |
| the re-pins | `cargo test -p yggdryl --test allocations -- declared sort_by`; `--test root -- vocabulary market`; `--test market_register`; `--doc vocabulary` | 8, 87, 7 and 3 passed |
| the code files | `git diff 6d71a36ee --stat -- rust/src/code.rs rust/src/{country,ccy,mic,cfi,isin,cusip,sedol,bbg,ric,figi,unit,forex,lei,bic,elf,dti,fisn}.rs docs/types/codes` | empty: byte-identical to HEAD (D25) |
| clippy, all features | `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings` | exit 0 |
| clippy, default features | `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` | exit 0 |
| the CLI | `cargo test -p yggdryl-cli --all-targets --no-fail-fast` | 35 passed, 6 ignored over its five harnesses (`fix`, `market`, `quality`, `style`, `xmla`) |
| the whole run, default features | `cargo test -p yggdryl --all-targets --no-fail-fast` | 70 targets, 6505 passed, 0 failed - after the first attempt died at the linker on a full disk (the sandbox allowance, not the tree): 20 GiB of stale `target/debug` artifacts of the workspace crates removed by `cargo clean -p`, then the run whole |
| rustdoc examples | `cargo test -p yggdryl --doc` | 626 passed |
| the API pages | `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps --all-features` | exit 0 |
| Python | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`, then `-m pytest python/tests -q` and `-m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py` | the extension installed; 2780 passed, 4 skipped - the same four as at P0, P1 and S0; mypy exit 0 |
| Node | `npm run --prefix node build:debug`; `cargo build --locked -p yggdryl-cli`; `npm test --prefix node`; `npx tsc --noEmit` in `node/`; `git diff --stat -- node/index.js node/index.d.ts` | the addon built; the CLI built; 1122 tests, 1120 passed, the 2 failing ones the sandbox `TextDecoder` pair below; tsc exit 0; the generated loader and declarations unchanged |
| the docs manifests | `node scripts/build_docs_fix.js --check`; `node scripts/build_docs_playground.js --check` | both current |
| the page examples | `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript` | Rust 936 passed; Python 836 run, 3 skipped, 0 failed; JavaScript 788 run, 2 skipped, 0 failed |
| the site | `python -m mkdocs build --strict --config-file mkdocs.yml` | built in 18 s, no warning |
| formatting | `cargo fmt --all -- --check`; `git diff --check` | both clean |
| the inventories | `python scripts/check_api_inventory.py`; `python scripts/generate_internals.py --check` | current (181 source files and 595 `pub` names not described yet, as before); `yggdryl::internals` current |
| no test code under `src/` | `grep -rn '#\[cfg(test)\]\|#\[test\]' rust/src python/src node/src cli/src` | empty |
| CI | the run on `ccr-0fe6f9d0-ruymat` for PR #209 | run 37839852909 on `eeb4ec14d`, read to the end: success - Rust quality (default features) and (all features), Iceberg Rust 1.94, the seven exchange jobs (S3, Azure, Google, ZIP, Avro, PyIceberg, Excel), Spark interop, Python binding wheel, Python binding (`pyarrow==18.*`) and (`pyarrow>=18`), Python binding (free-threaded 3.14t, abi3t 3.15), Node.js binding, Documentation examples, and `docs.yml`'s MkDocs build, every one green; the Pages deploy skipped as on every PR |

Not run, as the slice made nothing of theirs stale: the charset table and
interop checks, the ISIN seed check, the country and MIC table checks, every
`cargo bench` and `npm run bench:*`, the Python boundary benchmarks, the
scale run and the free-threaded lane. The two Node charset tests that compare
the package to the runtime's `TextDecoder` (`node/tests/charset.test.js`:
`decoding agrees with TextDecoder over the same names`, `iso-8859-1 is not a
spelling of windows-1252 here`) fail in this sandbox alone, whose Node 22.22
decodes the C1 range of `windows-1252` as ISO 8859-1 does; they failed at P0,
P1 and S0 the same way and CI's Node proves them.

## Blockers

None.

## Next

S2, the media extension point (D21; the prompt's eleven items under "S2"),
the media claimed in place with the core's `parquet`, `iceberg` and
`s3tables` features kept, and `yggdryl-excel` claimed beside them (D24).
The first command, from a fresh checkout of the program branch:

```bash
git fetch origin && git switch ccr-0fe6f9d0-ruymat && git merge origin/main
git grep -nE 'for_mime_type|open_as|into_media_base|require_kind|iceberg::located' -- rust/src ':(exclude).handoff'
```

Then design S2's register shapes against D8 and D21 in DESIGN.md before the
first edit, with the S1 shapes (`plugin::Register`, `market::claim`,
`unregistered`) as the mechanism to reuse.

## The decision ledger

The whole ledger is `.handoff/split/DESIGN.md`, section "The ledger". The
rows decided or changed this session:

| D# | decision | evidence | slice |
| --- | --- | --- | --- |
| D2 | closed shape over `&'static MarketDescriptor`; `Scalar` 48 and `Serie` 40 bytes | the spike; `allocations.rs` | S0, built S1 |
| D3 | `DataTypeId(u8)` newtype with CamelCase consts; the four enum kinds' ids leave it and the seventeen codes' stay (D25); `all()` answers the claimed order | `datatype_id.rs` | S0, built S1 |
| D7 | explicit idempotent `install()`; intake alone reads the register; the refusal list; the core seeds itself until S4 | `market.rs` | S0, built S1 |
| D8 | `plugin.rs`: one claim-once `Register<K, V>`; market and `LOGICAL_NAMES` keys on it, the media in S2 | `plugin.rs`, `vocabulary.rs` | S0, built S1/S2 |
| D19 | nothing moves: ranks, `Shape` positions and feeds as listed; a kind's marker hashes nothing and `DataType` writes its `shape` | the pins; the dictionary hash | S0, held S1 |
| D20 | `MarketValue` with an owned `from_scalar`; `Value` unchanged; the kind's ZST marker over `MarketType`; `SideField = FieldOf<SideType>` | `market.rs`, `typed.rs` | S0, built S1 |
| D25 | the seventeen codes stay core and flat; the register holds enum kinds alone (`Code8`/`Code16`); `yggdryl-market` carries the enums, `graph/` and the ISIN registry | the user's instruction; the S0 pins; one free Code byte | S1 |
| D22 | `pluginside` deleted; the plugin's role is a `Side` | the user's instruction | P0 |
| D23 | the medium holds its `RecordOptions`; the media serie keeps no copy | the user's instruction | P1 |
| D24 | `yggdryl-excel`, a seventh crate through the media point, claimed in S2, its own commit after S6 | the user's instruction | S0, built S6b |

## Questions for the user

1. The npm name (D12): `yggdryl-market` is recommended (one string
   preflight checks on all three registries); `yggdryl.market`, the user's
   spelling, is valid on npm and the alternative. Decide before S5 publishes.
2. The artifact sizes (D11): option (d) - one native module per runtime
   linking every crate - leaves the `yggdryl` wheels at 41-50 MB each
   (PyPI 0.1.21: 32 files, 1.40 GB in all) and the npm package at 337 MB
   unpacked. Is the unchanged size acceptable?
3. S6 as one commit (Avro, Parquet, Iceberg with `s3tables`, in that order),
   or the reversed order S6a-c if its diff is too large to review.
4. A persisted std hash: not needed - D19's plan held through S1 with no
   pin moved.
5. The go for S5's rehearsal and the repository configuration it needs
   (crates.io, PyPI trusted publishing and npm tokens for the new names).
6. Registry and network checks skipped: none - D11's and D12's reads were
   made on 2026-10-08 (crates.io, PyPI and npm all answered).
