# Continue: `window_by(by, sorted)`, static values on every series, `SerieSlice` → `WindowSerie`

This prompt stands alone. You are continuing work on the Rust crate **yggdryl** (`Platob/yggdryl`), which has two native views (Python in `python/`, Node in `node/`) and a CLI. `AGENTS.md` at the repo root is the binding contract: read it first and follow it, especially Smoke loop, Pace, §1 Patterns (Serie is the collection, Zero copy, Public vocabulary), §2 Validation, §3/§4 bindings and §5 docs.

## Where things are

- **Branch:** `claude/iceberg-partition-expressions-oc6r3k`. Develop, commit and push there: `git push -u origin claude/iceberg-partition-expressions-oc6r3k`.
- **Never force-push.** The environment denies it. To bring in `main`, merge it into the branch with a merge commit.
- **`main`** is at `655f596`. That commit is the squash merge of PR #187 (Iceberg transforms, `commit_batch_num`, the `by` protocols, Serie ordering, `SerieSlice`), on top of `183b0f8`.
- **The branch, in order on top of `main`:**
  1. `2f4340d` merges `main` back in. Its tree is identical to `main`.
  2. `3851a88`, the **base `window_by` core**:
     - `Run` is a view over one shared `Arc<[Scalar]>` (start and length), so slicing a run shares the values. A slice of a slice reaches the holder with the offsets summed. The whole serie is the serie itself. `Serie` is 40 B, `Scalar` stays 48 B, and `Run::into_inner` is gone.
     - `Selector::bind_key` is the one key rule, shared by merge and `window_by`.
     - `BoundSelector::apply_serie` computes the key record.
     - `window_by(by)` is on `Serie`, `SerieSlice` and `SerieSliceMut`, plus `SerieSliceMut::window_mut`.
     - `ChunkedSerie::window_by` merges a run that crosses a chunk edge.
     - Allocation pins cover all of the above.
  3. The **WIP extension-core commit** that holds this file. It implements the extension spec's phases X1a, X2, X1b, X3 and X4, described next.
- **The extension, as it stands in the WIP commit:**
  - **The verb:** `window_by(by, sorted: bool)` on `Serie`, `SerieSlice`, `SerieSliceMut`, `ChunkedSerie` and `SerieReader`.
  - **Comparison:** `Compare` has a `Record` rung, and `window_starts(regroup)` answers a `WindowCut` (the descent, and the stable run regrouping).
  - **Keys:** a hoisted `KeyPlan` with direct, narrow and whole arms, plus `apply_serie_window`. `Bound::lies_where` and `child_position` exist.
  - **Proof:** this adds one new `Proof::Proven` site, for key cells that "lie" (cells that are selections of a landed column, re-stated rather than computed). It is commented in `rust/src/expression/selector.rs`.
  - **Held windows:** `SerieWindows` is an owner (a `Cow` holder, `Cuts::Starts` or `Cuts::Gathered`) that lends `(Scalar, SerieSlice)` through `iter()` and `SerieWindowsIter`.
  - **Chunked:** `ChunkedSerie` regroups sorted windows as zero-copy pieces.
  - **Streams:**
    - `SerieReader` has crate-private `Statics` and inherent static-value accessors.
    - `SerieReader::window_by` answers `SerieReaderWindows`, lazy sub-readers whose static values are `{kept statics, key cells, windownum, rownum}`.
    - Windows are read in order, and a window passed with rows unread is refused by name.
    - With `sorted = true`, a key that goes backwards is refused, naming the batch and the row.
  - **Pins:** cost pins in `rust/tests/allocations.rs` and `rust/tests/iobase_calls.rs`, plus benchmarks.
- **State of the WIP commit, all checked at hand-off:**
  - `cargo check -p yggdryl --all-targets` is clean.
  - Default-feature suites all pass: serie 236, root 1293, expression 175, media 145, allocations 125, iobase_calls 27.
  - Clippy `-D warnings` is clean in the default lane and the `internals` lane.
  - `--features internals --test expression selector`: 35 passed.
  - `generate_internals.py --check` is current.
  - `--doc window`: 8 passed.
  - `rustfmt --check` passes on the changed files.
- **Not yet done on the WIP:**
  - the adversarial review and fix of the extension core (the workflow was stopped before that stage);
  - the all-features lane (`cargo test --all-targets --all-features`, including `--test iceberg merge`);
  - the `--quick` benchmarks;
  - the bindings, docs, skills and inventories for all of this.

## Design specs (temporary; delete `.handoff/` before the PR)

All are in `.handoff/window-by/`:

| File | What it is | State |
| --- | --- | --- |
| `WINDOW_BY_SPEC.md` | Base spec: `window_by(by)`, Run view, `bind_key`, `apply_serie` | Implemented in `3851a88`; its bindings and docs phases are folded into the extension's X5–X8 |
| `WINDOW_BY_EXTENSION_SPEC.md` | `window_by(by, sorted)`, the Record rung, the key plan arms, the `SerieWindows` owner, `SerieReader` static values and stream sub-readers, MediaSerie/partition seam (described only) | X1a–X4 implemented in the WIP commit; review, X5–X8 pending |
| `STATIC_VALUES_SPEC.md` | One generic static-values contract: public `Statics` in a new `rust/src/statics.rs`, a sealed `StaticValues` trait answered by `Serie`, `SerieSlice`, `SerieSliceMut`, `ChunkedSerie`, `SerieReader`; storage inline in each column leaf; per-verb keep/shift/scatter/derive/take/common/drop algebra; held windows state records too | Not started: phases S1–S5, then X5–X8 bind and document it |
| `EXTENSION_PHASE_REPORTS.md` | The implementers' reports for X1a, X2, X1b, X3: what each did, deviations from the spec, smoke results | Read before reviewing; X4's report was lost when the workflow stopped (its pins and benches are in the tree and pass) |

Spec line numbers refer to the tree at the time of writing. Read the code as it stands now.

## The user's requests, verbatim, in order

1. "Enrich the serie windowing to have window_by expressions, and also detecting current serie ins already a slice view to get holder and try merge to evict too recursive window encapsulations"
2. "Ensure it works for any series optimizely both simple or nested, especially on structs to have really powerful streaming, also have sorted_window_by which optimizely streams sorted keys"
3. "Add optional to seriereader a static_values: struct scalar accessor to link static values queryable for a reader and leverage it in windowing to simply yield sub readers optimized to attach generically either indexes, row num, or the expressions result scalars, which will later be used to have generic reader accessor from any media with partition values handling through it with a MediaSerie holding a media and optimized access on getter and setters and data muttators updates delete etc ..."
4. "Merge the methods window_by and sorted_window_by adding bool arg sorted defaulted to false"
5. "Make more generic for all series static_values accessors"
6. "Rename serieslice by windowserie"

A `MediaSerie` and media partition values flowing through static values are **later** work. Build only the seam the specs describe.

## Settled decisions (do not reopen)

- **Base spec:** the open decisions keep their defaults: the Run view with `usize`, no vectorized epoch arm, no `window_by_keys`, `partition_by_paths` unchanged.
- **Extension spec:** decisions 1–11 keep their defaults, **except decision 10**. In Python, `sorted=None` clears to `False` exactly as Node's `null` does, because AGENTS.md says `None` and `null` are values and they clear. So the Python binding takes `Option<bool>`, and `...` or omission means `False`.
- **Static-values spec:** decisions 1–14 keep their defaults, **except decision 12**. The `WindowSerie` rename runs **before** the static-values phases, so the new code is written once, under the new names.
- **No `sorted_window_by` name anywhere.** One method, `window_by(by, sorted)`. Rust takes a trailing `sorted: bool`; Python `window_by(by, sorted=False)`; JavaScript `windowBy(by, sorted)`.
- **The rename:**
  - `SerieSlice` becomes `WindowSerie`, `SerieSliceMut` becomes `WindowSerieMut` and `SerieSliceRows` becomes `WindowSerieRows`.
  - Files: `rust/src/serie_slice.rs` and `rust/tests/root/serie_slice.rs` become `window_serie.rs`; `python/src/serie_slice.rs`, `node/src/serie_slice.rs`, `python/tests/test_serie_slice.py`, `node/tests/serie_slice.test.js` and `node/tests/serie_slice.types.ts` are renamed the same way; `docs/types/serie-slice.md` becomes `docs/types/window-serie.md`, with its `mkdocs.yml` nav title "Window serie" and every link.
  - The Python and JavaScript class `SerieSlice` becomes `WindowSerie`, and the native classes `PySerieSlice`/`JsSerieSlice` follow.
  - Bench keys `serie_slice/...` become `window_serie/...`; the inventories, `AGENTS.md` and the skills follow too.
  - No alias. `SerieWindows` and `SerieWindowsIter` keep their names.
  - About 35 files hold references: `git grep -l "SerieSlice\|serie_slice\|serie-slice"`.

## What to do, in order

Ultracode is on: run each substantive phase as a `Workflow`, as described in the Ultracode section of the workflow authoring reference.
- Give parallel workers disjoint file sets, taken from the spec's Phases tables.
- Run review and verify stages on the tier below yours, and give mechanical sweeps a cheaper tier still.
- Read every workflow result before deciding the next phase.

0. **Set up the container.**
   - Build env, always: `export CARGO_TARGET_DIR=<one dir> CARGO_INCREMENTAL=0`. Use one target dir per tree; a second one doubles disk use.
   - Disk is a fixed allowance. Watch `df -h`, and prune stale test binaries of old feature sets rather than letting the disk fill.
   - Python: `python/.venv`. Mirror CI's `python-binding` job (`.github/workflows/ci.yml`) for the packages, including pandas, polars, tzdata, xxhash and mypy. Build with `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`.
   - Node: `npm ci --prefix node`, then `npm run --prefix node build:debug`.
1. **Review and fix the extension core**, the stage that was stopped. Run an adversarial review with three lenses:
   - **Correctness:** the sorted semantics, the gather and its `u32` bound, chunked regrouping, the stream walk (in-order rule; passed, served and dropped sub-readers; a descent at an edge and inside a batch; error fusing; absolute `rownum`), the direct key arm against the engine under absent parents, and the soundness of the `Proven` site.
   - **The AGENTS.md contract.**
   - **Cost:** per-batch, per-window and per-piece laws, never per row, and constant in record width.

   Then verify the findings and fix the real ones. Also run:
   - the `--all-features` lane for the touched targets (`--test iceberg merge` included);
   - `cargo bench -p yggdryl --bench types -- window_by --quick` and `--bench arrow -- serie_reader/window_by --quick`, for direction only.

   Commit.
2. **Run the `WindowSerie` rename**, task 6 of the requests. AGENTS.md's Pace section asks for one exact-string script:
   - drive it from `cargo check -p yggdryl --all-targets --keep-going --message-format=short` until the check is clean, and then from `cargo check --workspace --all-targets --keep-going --message-format=short` for the bindings;
   - use `git mv` for the files;
   - run `cargo fmt --all` once at the end.

   Smoke the touched suites, then commit.
3. **Static values S1–S5** (`STATIC_VALUES_SPEC.md`, its Phases table), written under the `WindowSerie` names.
   - It retypes and moves the in-flight `SerieReader` `Statics` into a public `rust/src/statics.rs`.
   - Held and chunked windows now state their record.
   - Re-pin only what the spec's "Pins that move" accounts for, each with its reason sentence. Any other moved count is a defect.

   Commit.
4. **Bindings, Python then Node** (extension X5/X6, plus the static-values Bindings section, plus the rename):
   - Python: `window_by(by, sorted=False)` on `Serie`, `WindowSerie` and `ChunkedSerie`; `SerieReader.window_by` returning `SerieReaderWindows` (an iterator of sub-readers, each pull running in `py.detach`); the static-value members on every carrier; `_native.pyi`, `typing_bindings.py`, `mypy --strict`; parity tests; boundary benchmarks.
   - Node: the camelCase twins, the regenerated `node/index.{js,d.ts}`, `*.types.ts`.
   - Update `.api-bindings.txt`.
5. **Docs (X7).**
   - Pages: `docs/types/serie.md`, the new `docs/types/window-serie.md`, `docs/types/chunked-serie.md`, and a new `docs/types/static-values.md` (add it to the nav in `mkdocs.yml`).
   - Also `docs/arrow/readers.md` (windows of a stream) and `docs/media/index.md` (one sentence and a link).
   - Every example uses Rust, Python and JavaScript tabs and asserts its result.
   - `skills/yggdryl-arrow/**`, `AGENTS.md` (Layout rows, tables, Zero copy), `.api-inventory.txt`, and rustdoc examples on every new public item.
   - Fix the stale "Size | 24 bytes" row in `serie.md`: it is now 40.
   - **Delete `.handoff/`.**
6. **The chain (X8).** One background script and one log, in AGENTS.md order:
   1. `cargo test -p yggdryl --all-targets --all-features --no-fail-fast`;
   2. clippy in both lanes;
   3. `cargo test -p yggdryl --doc`;
   4. the §3 Python pre-push block;
   5. the §4 Node pre-push block, which includes `build_docs_playground.js --check` and `build_docs_fix.js --check`;
   6. `python -m mkdocs build --strict --config-file mkdocs.yml`;
   7. `python scripts/check_docs_examples.py --lang rust|python|javascript`;
   8. `python scripts/check_api_inventory.py`.
7. **Ship.**
   - Commit and push.
   - Open a PR to `main`. No PR template exists.
   - Subscribe to the PR's activity and drive CI to green.
   - For the previous batch the user asked "Once done merge to main", and PR #187 was squash-merged (`main`'s history is one commit per change). Do the same here once CI is green, unless the user says otherwise.

## Rules that bit last time

- **Never run `cargo fmt --all` while workers edit.** Run `rustfmt --edition 2024 <file>` on the files you edited, and `cargo fmt --all` once after the last worker.
- **A moved allocation or call-count pin is a design answer.** Re-pin only with an exact accounting sentence, and never re-pin a cost pin from the whole run.
- **The `internals` feature.** A crate-private module that an `internals` forwarder reaches must be `pub(crate)` in its parent, as `expression/mod.rs` is now. After adding a forwarder, run `python scripts/generate_internals.py`.
- **Commits** end with these lines, and carry no model names:
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01J1MU39fMaysDDhE5UiXDb2
  ```
  Use the attribution your own session gives you if it differs.
- **Every GitHub comment** ends with:
  ```

  ---
  _Generated by [Claude Code](https://claude.ai/code)_
  ```

## Follow-ups outside this work (do not do them here)

- **Iceberg scan planning:** a scan decodes every manifest entry at about 58 µs each, so planning a one-hour scan over a `minutes(ts, 15)` table costs about 90 ms. A suggested task for it was already queued.
- **A vectorized epoch arm** (`PrimitiveArray::unary_opt`) for `days`/`minutes(ts, n)` keys, which would speed up `select`, `where`, `TRANSFORM`, `PARTITION`, `DIGEST` and `window_by`. It stays out by decision.
