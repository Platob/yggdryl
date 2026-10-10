# R1: design (D43) - the first release with market and fix split

The user's instruction (2026-10-10, `$S/p9/user_instruction.md`, item 1: "a first release with
market and fix splitted") and the decision that followed (`$S/user_decisions.md` 6: "upgrade to
0.1.22 to publish before next steps"). **R1 is implemented**: commit `0f411f5ce` "Release 0.1.22:
publish yggdryl-market and yggdryl-fix beside the core" (2026-10-10 00:23 UTC) on the program
branch `ccr-0fe6f9d0-ruymat`, the handoff over it `3d9725495` (the live AWS prompt) and
`4c2edd116` (the user's scope). This document is the **record** of what that commit did, verified
against the tree, what remains for the user alone, and what the commit missed (each a finding to
fix in the lane that follows). Hard rules it keeps: never publish, never push to `main`, no tag, no
release run; the lane prepares a commit on the program branch and proves it with dry runs; the
user's go merges, and the merge is the release.

## What a first release of the split publishes

| Registry | Names | Why |
| --- | --- | --- |
| crates.io | `yggdryl`, `yggdryl-market`, `yggdryl-fix`, in that order | `scripts/release_packages.py crates` lists every workspace member without `publish = false` (`:58-63`), ordered so a crate follows every workspace crate it names, dev-dependencies included (`:78-89`); the three manifests carry `version.workspace = true` and no `publish = false` (`rust/market/Cargo.toml:4`, `rust/fix/Cargo.toml:4`); `yggdryl-cli`, `yggdryl-python`, `yggdryl-node` are `publish = false` (`cli/Cargo.toml:17`, `python/Cargo.toml:9`, `node/Cargo.toml:9`) |
| PyPI | `yggdryl` | `PYPI = ["yggdryl"]` (`release_packages.py:39`); the wheel links all three crates (`python/Cargo.toml`: `yggdryl-market = { workspace = true, features = ["http"] }`, `yggdryl-fix` the same) and ships the `yggdryl` command (`stage_cli.py`, `publish = false` on the CLI) |
| npm | `yggdryl` | `NPM = ["yggdryl"]` (`:40`); the addon links all three (`node/Cargo.toml`) |

**B5's separate `yggdryl-market` Python and npm packages are not needed for R1**: nothing of the
market or FIX surface is missing from the `yggdryl` wheel and addon (`python/src/lib.rs` registers
`IsinRegistry`, `FixCodec`, the market classes in the one `_native` module; `node/src/lib.rs` the
same), and B5 (`$S/b5_manager_prompt.md`) exists to let a consumer install the market view alone -
a packaging convenience the user postponed to the PR after the release (`user_decisions.md` 7).
`release_packages.py`'s `version` already reads B5's manifests (`python/market/pyproject.toml` and
`node/market/package.json` when they exist, `:108-128`); the rest is B5's: `PYPI = ["yggdryl"]` and
`NPM = ["yggdryl"]` (`:39-40`) gain the market names, `build` gains the market wheel and sdist, and
`publish-npm` - which publishes only `yggdryl@$version` from `working-directory: node` with its five
binaries (`release.yml:829-855`) - gains a step for `node/market`.

## The version bump: 0.1.21 -> 0.1.22, the seven files

`main` is at `2ae975674` = tag `v0.1.21` (verified: `git rev-list -n1 v0.1.21`, `git show
main:Cargo.toml` line 13); the branch carries 0.1.22 in every file AGENTS §6 names, moved together
in `0f411f5ce`:

| File | What moved | Verified |
| --- | --- | --- |
| `Cargo.toml` | `[workspace.package] version = "0.1.22"` (`:13`) and the three exact pins `yggdryl = { path = "rust", version = "=0.1.22" }`, `yggdryl-market`, `yggdryl-fix` (`:22-24`) - a published crate names exactly the core it was built with | yes |
| `Cargo.lock` | the six workspace packages at 0.1.22 | yes (`--locked` holds it) |
| `python/pyproject.toml` | `version = "0.1.22"` (`:7`) | yes |
| `node/package.json` | `"version": "0.1.22"` (`:3`) | yes |
| `node/package-lock.json` | top-level and `packages[""]` | yes |
| `docs/assets/fix.json`, `docs/assets/playground.json` | stamped from `node/package.json` by `scripts/build_docs_fix.js:41` and `build_docs_playground.js:30` | yes, regenerated in the commit |

`python3 scripts/release_packages.py version` answers `0.1.22` (every manifest agrees; a workspace
pin not spelled `=<version>` is refused, `:100-101`); `crates` answers `yggdryl yggdryl-market
yggdryl-fix`. CI's inventory job runs `version` (`.github/workflows/ci.yml:944`) on every pull
request that touches a manifest - `Cargo.toml` runs every job; `python/pyproject.toml`, `node/**`
and `scripts/release_packages.py` select the inventory row (`rows.toml:242-253`) - so a manifest a
later bump forgets fails on the PR, not in the release.

The handoff sentence "the version stays `0.1.21` until S9" (`DESIGN.md:17,89,2931` D1) is
**superseded** by the user's decision 6; `MARKET_SPLIT_CONTINUE.md` and `MARKET_SPLIT_NEXT.md`
already say so (`4c2edd116`), DESIGN.md's three lines do not - finding F5.

## The exact `release.yml` edits (as committed)

`.github/workflows/release.yml` spells no crate name and no order (its header comment `:22-25`);
everything reads `release_packages.py`:

1. **`preflight`** (`:64-190`, "Versions agree"): `version=$(python3 scripts/release_packages.py
   version)` (`:85`); a pushed tag must be `v$version` (`:86-89`); `crates=$(... crates)` exported
   (`:92-96`); the mode: `workflow_dispatch` builds and publishes nothing (a rehearsal),
   a tag publishes, a `main` push whose `refs/tags/v$version` already exists on origin does
   nothing, any other `main` push builds and publishes (`:98-116`). **The half-out check**
   (`:133-190`): every crate by the sparse index `https://index.crates.io/${crate:0:2}/${crate:2:2}/$crate`
   grepped for `"vers":"$VERSION"`, every PyPI name by `https://pypi.org/pypi/$name/$VERSION/json`,
   every npm name by `https://registry.npmjs.org/$name/$VERSION`; all yes or all no passes; a mix
   is a warning on a tag push or a rehearsal and a **refusal on a branch push**, printing the two
   ways out (`git tag v$VERSION <commit> && git push origin v$VERSION`, or bump every manifest).
2. **`sources`** (`:192-230`, "Source packages"): `rustup toolchain install stable --profile
   minimal`, then **one** `cargo publish --locked --dry-run -p yggdryl -p yggdryl-market -p
   yggdryl-fix` (`:212-218`, the `-p` list built from `$CRATES`), so cargo verifies each package
   against the others as a local registry - a crate whose sibling crates.io does not hold yet still
   builds from its package. Multi-package `cargo publish` is cargo 1.90+; the workspace MSRV is
   1.94 (`Cargo.toml:15`), `stable` on the runner is above it. Then the sdist through
   `PyO3/maturin-action@v1` at `v1.15.0` (`:219-224`).
3. **`build`** (`:232-766`): eight platform rows, unchanged by the split - every wheel and the addon
   link the three crates through the manifests, no row names a crate.
4. **`publish-pypi`** (`:768-789`) and **`publish-npm`** (`:791-855`): `yggdryl` only, unchanged.
5. **`release`** (`:857-901`, "crates.io, tag and GitHub release"): needs `[preflight, sources,
   publish-pypi, publish-npm]`; `for crate in $CRATES` - skip when the sparse index already holds
   `"vers":"$version"`, else `cargo publish --locked -p "$crate"` under
   `CARGO_REGISTRY_TOKEN` (`:878-890`); one invocation per crate in dependency order, because the
   per-crate loop skips the crates already on the index, so a repaired run is idempotent (cargo
   1.90+'s `--workspace` would skip the `publish = false` members itself; that is not the reason).
   One edge a later lane may close: `cargo publish` waits about 60 s for the index to serve the
   upload and on a timeout warns and exits 0, so the next crate's verification can fail to find
   `yggdryl =0.1.22` and leave the version half out - a retry of `cargo publish -p` once after a
   sparse-index poll is the fix, and the `report` job names the half-out state until then.
   Then `gh release create v$version --target $GITHUB_SHA --generate-notes` once (`:891-901`): the
   tag is created last and is the one record that the release is complete.
6. **`report`** (`:903-1011`): on a failed or cancelled publishing run, every crate, PyPI and npm
   name is read again, `::error title=Half published` on a mix, one issue per version
   (`Release <v> did not finish`) or a comment on the open one.

Also in the commit: `cli/Cargo.toml` `publish = false` (`:17`), `ci.yml`'s inventory step `Every
manifest carries one version` (`:944`), `rows.toml`'s inventory row grown by `python/pyproject.toml`
and `scripts/release_packages.py` (`:251`), AGENTS.md §6 rewritten for every crate (`:2884-2930`).

A fourth crate (M6's) joins the release by its manifest alone: no workflow edit.

## What the user alone does

1. **`CARGO_REGISTRY_TOKEN`** in the repository secrets with the scope to **publish new crates**
   (`publish-new`) over `yggdryl-market` and `yggdryl-fix`, which crates.io has never held
   (`user_decisions.md` 5; AGENTS.md:2927-2930). `yggdryl` exists already and needs
   `publish-update` only. Re-read `https://index.crates.io/yg/gd/yggdryl-market` and
   `.../yggdryl-fix` just before the go: the 404s on record are from 2026-10-08/09 (`DESIGN.md:
   361-368`, `$S/s5_define_report.md:130-135`).
2. **Nothing new on PyPI or npm** for R1: both names stay `yggdryl`, published by PyPI and npm
   trusted publishing (OIDC under the `pypi` and `npm` environments, `id-token: write`, npm 11.5+;
   `release.yml:36-43,803-808,819-823`), already bound to this repository - no stored secret for
   either. The `yggdryl-market` pending publisher and npm bootstrap are B5's, postponed.
3. **The merge of PR #209 into `main`** after P9, P7, P8, P5R have landed green and the live AWS
   run has reported (`.handoff/next/LIVE_AWS_TEST_PROMPT.md`, its report
   `.handoff/next/LIVE_AWS_RESULTS.md`). The merge is a push to `main` whose version 0.1.22 has no
   tag, so `preflight` answers `build=true, publish=true` and the run publishes
   (`release.yml:108-116`). Nothing else: no tag by hand (the run creates it), no manual
   `workflow_dispatch` unless the user wants a rehearsal first (it publishes nothing; it is still a
   release run, so it is the user's to start - `user_decisions.md` 5).

## Where R1 sits in the lane order

R1 is not a lane that edits code; it is the merge. The user decided (`user_decisions.md` 7):
**P9 -> P7 -> P8 -> P5R, each a commit read green, then the live AWS run, then the merge**; M6,
B5, S7, S8, S9 after, from a PR cut from `main`. Recommended HEAD for the merge: the commit
holding the live AWS report, over P5R, with the newest CI run on the branch green to `CI result` -
because (a) `release_packages.py` derives the crate set from the manifests, so merging before M6
keeps 0.1.22 to the three crates the user named, and merging after M6 would publish six more
names under one version (and need six more `publish-new` grants); (b) P9 adds the instruments
table and the `instcode` column (the instrument's crosscode, `utf8`, tag 65_054 - `user_decisions.md`
9) the live prompt's step 3d checks (`LIVE_AWS_TEST_PROMPT.md:50-55`), so the live run has to
follow P9; (c) P7 (D40.5) is the user's hashing correction, which P9 leaves on today's rule
(`$S/p9/d42_design.md`, "Put to the user" 3), so a merge before P7 would ship 0.1.22 with the
uncorrected uuids - the merge waits on P7 as it waits on the live run; (d) a version is out
everywhere or nowhere, and the tree merged is the tree built. R1 can run on any green HEAD of the
branch in principle (the plumbing is version-driven), but a HEAD before P9 would publish 0.1.22
without the Instrument the user scoped into it.

## Findings - what the commit missed, each to fix

- **F1 - the dry runs are not recorded.** The computed task states the three-crate `cargo publish
  --locked --dry-run -p yggdryl -p yggdryl-market -p yggdryl-fix` and the medallion test are green
  on `0f411f5ce`; the commit message does not record the command or its output, and DESIGN.md's
  S4 results hold only `cargo package --locked --list -p yggdryl` (`DESIGN.md:2678`). No CI job
  dry-runs a publish or builds the sdist (`ci.yml` has neither; only `release.yml`'s `sources`
  does, on a release or a rehearsal). The next lane runs `$S/r1/r1_release.sh --prove` **after
  its commit and before its push**, on the clean committed tree (the script refuses a dirty tree,
  and the chain runs while the tree holds the phase edits, so it is not a chain step), and the
  results commit records its log: the dry run, `cargo package --list` for the two new crates
  (README.md present, nothing under `market/`/`fix/` in the core's list), the sdist listing
  `rust/market/Cargo.toml` and `rust/fix/Cargo.toml`, the wheel's command, the two docs manifests'
  `--check`, the npm audit. The seed's path is P9's to settle (`d42_design.md` D42.13), so the
  script asserts nothing about it.
- **F2 - a new name's first publish happens last.** `release` runs after PyPI and npm (`:866`), so
  a token without `publish-new`, or a name taken between the go and the run, leaves 0.1.22 on PyPI
  and npm and not on crates.io - half out, which `preflight` then refuses on `main` until a tag push
  finishes it (the documented repair, `:180-190`). Mitigation a later lane may add (optional, not
  in `0f411f5ce`): a `preflight` step querying `https://crates.io/api/v1/crates/<name>` with a
  `User-Agent` for every crate absent from the sparse index and failing on anything but 404 - it
  proves the name is free, not the token's scope, which only a real publish shows. The user's
  check of the token scope before the go is the real guard.
- **F3 - the packaged crates carry no LICENSE file.** `rust/market/` and `rust/fix/` hold
  `Cargo.toml` and `README.md` beside `src/`, `tests/`, `benchmarks/` (`git ls-files`); the SPDX
  field is set through `license.workspace = true` (`Apache-2.0`), which crates.io accepts, and
  `rust/LICENSE` ships with the core alone. Not a blocker; a later lane may add `license-file` or a
  copy, and say so in the two READMEs.
- **F4 - `yggdryl-fix`'s package cannot run its own tests or doc tests unpacked.** Its tests and
  benches reach the monorepo (`#[path]` into `rust/tests/support`, `include_bytes!("../../../tests/
  support/ulbridge.log")`, `CARGO_MANIFEST_DIR/../../config/fix`) and the crate carries no FIX
  dictionary (read at run time from `YGGDRYL_FIX_REGISTRY` or `~/.config/fix`, `rust/fix/src/
  global.rs`). `cargo publish`'s verification builds the library alone, so the dry run passes; the
  README should say the dictionary is fetched or supplied, not shipped. Documentation, not plumbing.
- **F5 - DESIGN.md still says 0.1.21 until S9** (`:17`, `:89`, the D1 row `:2931`). The P9 results
  commit re-spells the three lines: 0.1.22 on the branch since `0f411f5ce`, published by the merge.
- **F6 - the medallion CLI commits no instruments.** `python/tests/medallion.py main()` builds the
  codec without a registry (`:534-540`), so the live AWS prompt's step 3d ("the instruments table
  ... present and filled") cannot pass on the CLI path today. P9 fixes it (D42.14); recorded here
  because R1's gate depends on it.
- **F7 - the sdist's content is unverified.** Whether maturin 1.15 includes `rust/market` and
  `rust/fix` (path dependencies through `[workspace.dependencies]`) while the core's
  `exclude = ["/market", "/fix"]` (`rust/Cargo.toml:12`) excludes them from the core's *package*
  only, is what `r1_release.sh --prove`'s `tar tzf` step answers. The release's own `sources` job
  is the second proof, at the rehearsal or the release.
- **F8 - the B5 script is stale against this workflow.** `$S/s5_move.py` targets the pre-`0f411f5ce`
  `release.yml`: eleven of its twenty `REL_*_OLD` anchors match nothing, one would duplicate a
  paragraph, its `ci.yml` and `rows.toml` anchors are stale and its AGENTS §6 anchors no longer
  match (the release reader's count). Not R1's work (B5 is postponed) but recorded so B5 re-derives
  the script from the tree it runs on, as the continue prompt requires.
- **F9 - the proof script is linked from no handoff entry.** `r1_release.sh` is named here alone;
  `MARKET_SPLIT_CONTINUE.md`'s release step should say `r1_release.sh --prove` is the first
  release's proof and that the merge is the release. The P9 results commit adds the line.
- **F10 - AGENTS.md §6 misnames the credentials.** `AGENTS.md:2927` says "Cargo and npm secrets,
  PyPI trusted publishing"; the workflow has npm on trusted publishing too (`release.yml:41-43`).
  Re-spell as "the Cargo secret; PyPI and npm trusted publishing" in the P9 results commit.

## Proof a later release runs (no publish)

`$S/r1/r1_release.sh <version>` bumps a release candidate for 0.1.23 and after (0.1.22 is bumped)
and **stops**, printing "commit the bump, then run `--prove`": the proof runs on a committed tree,
as the release's `sources` job does, and cargo's VCS check needs no `--allow-dirty`.
`r1_release.sh --prove` runs the proofs on the current version. Both refuse `main` and a detached
HEAD; `--prove` refuses a dirty tree; the bump refuses a target at or below the current version,
reads the three registries read-only through `curl` under the proxy environment (30 s each) and
stops on an unreachable one, then moves the version in the seven files (the lock through `cargo
update --workspace`, the two docs manifests through their node scripts after `npm run --prefix
node build:debug`, the market manifests too once B5 adds them). `--prove` builds the debug addon,
then proves: `cargo publish --locked --dry-run` over the crates `release_packages.py crates` lists
in one invocation, `cargo package --locked --list` per crate, the two docs manifests' `--check`,
`Cargo.lock`'s six workspace packages at the version, `maturin build --profile dev` of the wheel
(CI's wheel job's profile) into `$R1_OUT`, else a fresh temporary directory it prints, with its
`unzip`/`tar` listings written to files and read from them, `npm pack --dry-run`, `git diff
--stat`. It never runs `cargo publish` without `--dry-run`, never pushes, never tags, edits no
workflow; it ends by saying what the lane does (commit, push, read CI) and what the user does (the
token, the merge).
