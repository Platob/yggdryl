# Standalone prompt: finish and Excel-verify `yggdryl excel serve`

Give the text below the line to the local LLM, with a checkout of the repository. It assumes nothing else: no previous conversation, no cloud environment, no special tools, only a shell, git, and this repository.

---

## 0. Who you are and what you are doing

You are finishing a large feature branch of **yggdryl** (`https://github.com/Platob/yggdryl`), a Rust core with Python and Node bindings and a CLI. An earlier session designed the feature, built its first phases and pushed checkpoints to a branch. That session is gone. Its complete record is in the branch, under `handoff/`.

You have one advantage it lacked: **Microsoft Excel is installed on this machine.** You finish the remaining phases, and you prove what the code writes against real Excel (§8).

Work phase by phase. After every behavior change, run the narrowest test that executes it. Never report a phase done while its checks are red or unrun.

## 1. The request you are fulfilling (the user's words)

> "Continue in another pr to serve a full excel replication … to have optimized self hosted workbook with open, create, sheet full interaction microsoft excel like and with pivot table implementation and media data import in ribbon leveraging our uri abstractions and centralized media"
>
> "Make it cli yggdryl excel serve like others leveraging http optimized implementation and enriching it if you find generic reusable patterns"

**The command.** `yggdryl excel serve [LOCATION]` is a CLI verb built like the existing `yggdryl xmla serve` (`cli/src/xmla.rs`). It serves a workbook named by any URI the crate's `Holder` reaches: a local path, `file://`, `s3://`, `https://`, or a zip member. It serves it on the crate's own HTTP server (`rust/src/http/`) as:

- **A browser app that looks and behaves like Microsoft Excel.**
  - A ribbon, a name box and formula bar, and a virtualized canvas grid.
  - Selection, the Excel keyboard map, editing, formulas with recalculation, styles, and row, column and sheet operations.
  - Copy and paste, undo and redo.
  - Pivot tables.
  - **Data › Get Data**, which imports any medium the crate reads (Parquet, IPC, Avro, JSON, xlsx, Iceberg, …) through `Holder::from_url` and the central `RecordOptions` / `IOMedia::read_arrow` dispatch.
- **A JSON API over an in-memory `Workbook`** that the app calls.

Where serving the app and the existing XMLA service share a generic HTTP need, the code enriches `rust/src/http/` generically and moves XMLA onto it.

## 2. Set up this machine

```bash
git clone https://github.com/Platob/yggdryl && cd yggdryl
git checkout ccr-a1679fa8-oczghl
git log --oneline origin/main..HEAD   # the checkpoints: "WIP P1a", "WIP P1b", "WIP P2", "WIP P3", "WIP handoff", maybe "WIP P4"
```

**Toolchains** (they match CI):

| Tool | Setup |
| --- | --- |
| Rust stable (CI lints with it; 1.98.1 at the time) | `rustup toolchain install stable --component clippy,rustfmt` |
| Rust 1.85.0 (the core's MSRV; a CI lane) | `rustup toolchain install 1.85.0 --profile minimal` |
| Python 3.12, in `python/.venv` | `python -m venv python/.venv`, then install `maturin>=1.9,<2`, `pytest`, `mypy`, `pyarrow`, `pandas`, `polars`, `tzdata`, `xxhash`, `openpyxl` into it. A silently skipped suite counts as a failed check, so install them all |
| Node 22 | `npm ci --prefix node` |
| Browser checks for the UI | `npm i -g playwright`, then `npx playwright install chromium` |
| Excel automation (§8) | Windows: `pip install pywin32` into a Windows Python. macOS: `pip install xlwings` |
| LibreOffice Calc (optional) | A second oracle, used through `handoff/lo_recalc/recalc.py` |

On Windows the venv interpreter is `python\.venv\Scripts\python.exe`; wherever a command below says `python/.venv/bin/python`, use that instead. The Rust, Python and Node builds may run in WSL. The Excel harness must run in a Windows Python that can reach the same files.

**Verify the starting point before you change anything:**

```bash
cargo check -p yggdryl --all-targets --keep-going --message-format=short        # expected: clean
cargo test -p yggdryl --test excel                                              # expected: ~490+ passed, 0 failed
cargo test -p yggdryl --features internals --test excel                         # expected: ~500+ passed
cargo test -p yggdryl --test media                                              # expected: 145 passed
python/.venv/bin/python scripts/check_excel_interop.py                          # expected: exit 0, every half printed, never "SKIPPED"
cargo check --workspace --all-targets --keep-going --message-format=short        # expected: FAILS, but only in python/src/excel.rs and node/src/excel.rs (see §5)
```

If `WIP P4` is the last checkpoint, the counts are higher. Record what you actually see. It is your baseline.

## 3. The contract you work under

`AGENTS.md` at the repository root is binding; read it first. The parts that bite most often:

- **Layer order.** Rust core → Python → Node → docs. A binding only redirects into the core and implements no logic.
- **No back-compat.** A change deletes the symbol, spelling, test and doc it replaces, and updates every caller. No shims, aliases or deprecation.
- **One owner per fact.** Keep the vocabulary: `new`, `from_*`, `into_*`, `as_*`, `is_*`, `get*`, `set_*`, `with_*`. Never a project-defined `to_*`. `*Value` names a trait, never a type.
- **Wide in, typed through.** Accept every spelling at one boundary and resolve it once. Errors are typed and located. Mutations are atomic: a failure leaves the value unchanged.
- **Tests mirror sources, file for file.** `rust/src/excel/x.rs` is pinned by `rust/tests/excel/x.rs`, declared in `rust/tests/excel.rs`, opening with a `//!` line that names the source file.
  - What callers cannot reach goes through the `internals` feature (`yggdryl::internals::…`). Regenerate its re-export with `python scripts/generate_internals.py`; `--check` must pass.
- **Cost is pinned.**
  - `IOBase` call counts in `rust/tests/iobase_calls.rs`, allocations in `rust/tests/allocations.rs`.
  - A pin that moves is a design question. Never re-pin silently; the comment on a moved pin says why it moved.
- **Exchange formats are checked in both directions against an outside implementation.** For Excel files that is `scripts/check_excel_interop.py` with openpyxl. A skipped half is a failure.
- **Every public item** has docs, a rustdoc example where its neighbours have one, an entry in `.api-inventory.txt` (Rust) or `.api-bindings.txt` (Python and JS), and a docs entry in three languages.
- **The CI gates to keep green:**
  - `cargo fmt --all`
  - clippy `-D warnings` on stable, in both lanes: `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings` and `cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings`
  - `RUSTDOCFLAGS=-D warnings cargo doc`
  - the MSRV check on 1.85.0
  - `--locked` everywhere

## 4. The record the earlier session left: `handoff/`

| Path | Use |
| --- | --- |
| `handoff/design.md` | **The single source of truth.** Every remaining phase follows it: scope, Rust signatures, the JSON API, the UI, tests and pins, and the plan in §13. Read §0 (decision log), §1 (scope), §13 (plan) and §14 (risks) fully. Read each other section when your phase names it. |
| `handoff/contract-addendum.md` | Thirteen refinements decided after the design froze. **They override the design where the two differ.** |
| `handoff/phases/P1a.json … P3.json` (and `P4.json` if present) | Each finished phase's report: what it built, deviations from the design, the checks it ran, review findings with their fixes, and **`open`: items addressed to later phases**. Read every `open` list before you start a phase. |
| `handoff/ui-dev/` | The UI's development server, mock store and browser checks (§7). |
| `handoff/lo_recalc/recalc.py` | Recalculates an `.xlsx` with headless LibreOffice and prints each sheet as CSV. |
| `handoff/pivot_probe/probe.py` | Builds a minimal hand-written pivot table. openpyxl 3.1.5 and LibreOffice accept it; Excel was never tried. |

**Translate these foreign references wherever the record uses them:**

| The record says | It means for you |
| --- | --- |
| `/home/user/yggdryl/…` | your repository root |
| `$SCRATCH`, the "scratchpad", `/tmp/claude-0/.../scratchpad/…` | `handoff/` in your checkout. Scratchpad `ui/` is `handoff/ui-dev/`; `lo_recalc/` and `pivot_probe/` are the same names under `handoff/`. |
| `$SCRATCH/bin/yggdryl` | your own `target/debug/yggdryl` |
| "Agent A" | whoever runs cargo. Only one cargo job at a time: they share one target directory and lock. |
| "Agent B" | work on non-cargo files only: UI assets, scripts, docs. If you work alone, do A's then B's part of each phase. |

## 5. What exists now, and what is deliberately broken

**Built and reviewed, in `rust/src/excel/`.** Every phase passed its tests plus an adversarial review round, and every finding was fixed with a test.

- **P1a, storage.**
  - `Cell` holds at most 80 bytes (const-asserted): value, reference, `Option<Formula>`, `StyleId`, kind, `NumberFormat` and `Option<ExcelError>`, with 19 error variants and `Unrecognized` carrying its literal.
  - Rows are sorted `Vec`s. An `Extent` cache makes `dimension()` O(log rows) and `cell_count()` O(1).
  - Sparse `CellExtra` (`cm`, `vm`, `ph`, rich inline text, array and data-table attributes), `Sheet::revision`, and the `CellMut` guard.
  - `ZipArchive::copy_member_from` copies a member raw, inflating nothing.
- **P1b, saving.**
  - Dirty tracking: an unparsed or merely read sheet is clean. `SheetKey`.
  - **One writer** over `Source::{Archive, Template}`; the old fresh-workbook writer is deleted. `Workbook::{into_package, rebase, write_into, into_bytes, parse_all, is_dirty}`. `write_into` builds, adopts, writes, then marks saved, so a failed write leaves the workbook dirty.
  - `fullCalcOnLoad` is set; `calcChain` goes together with its relationship and `Override`.
  - `package::rewrite` gained `attributes` and `insert` hooks.
  - `StyleSheet` parses every styles fact, with an append-only splice and `TemporalStyles`.
- **P2, fidelity.**
  - `carried.rs`: a byte-exact worksheet frame. Every element the crate does not model is carried in CT_Worksheet order and classed Free, Shifted or Blocking.
  - `layout.rs`: columns, row formats, merges, the frozen pane.
  - Formula **shapes** (`formula.rs`, `formula/{lexer,reference,shape}.rs`):
    - verbatim text runs, plus references stored relative to the host cell
    - shared formulas share one `Arc`
    - `_xlfn.` and `@` spellings
    - array and dynamic-array attributes carried
  - Lossless shared strings.
  - `names.rs` defined names, `active_tab`, `move_sheet`, and orphan-part removal by reference counting.
  - The `rich_package()` fixture, with pins proving that read → save → reopen changes nothing but what was edited.
- **P3, display, entry and styles.**
  - `format.rs` `FormatCode`: parse, classify, render. 288 test vectors. 24 of them follow what the earlier session believed Excel does where LibreOffice differs. **Confirm them in Excel.**
  - `theme.rs`, with ECMA tint on HLS.
  - `entry.rs`: typed en-US entry.
  - `Workbook::{set_style, cell_style, set_entry, entry_text, display_text}`, `StylePatch`.
- **P4, structure and edits** (design §4). **Only if `WIP P4` is committed.** Otherwise do P4 from scratch per design §4 plus addendum items 6, 10 and 11.
- **The UI**, in `cli/assets/excel/`:
  - the ribbon, the canvas grid with its tile cache, the keyboard map and editing
  - Home-tab styling, menus, the status bar, the clipboard, dialogs and cross-tab sync
  - sheet tabs, the fill handle, Find/Replace, sort, and formula assist when P4 is committed
  - Everything is built against the mock server only. It has never talked to the real service, which does not exist yet.

**Deliberately broken until their phase. Do not fix early:**

- **The Python and Node bindings** (`python/src/excel.rs`, `node/src/excel.rs`) still call the old string-based formula and error API, so `cargo check --workspace` fails there. P10 and P11 fix them.
- **`.api-inventory.txt`** still lists the deleted `style_index`, lists `DateSystem` variants `Excel1900/Excel1904` (the code says `Year1900/Year1904`), and lacks every new public name. P12 fixes it, using the names listed in each phase report's `open`.
- **Docs and skills** have no entries for the new surfaces. P12 adds them.

## 6. The remaining phases

Run them in this order. Settle each with its narrow commands, never the whole suite; the whole run is P12. Before each phase, read the design sections it names, the addendum, and the `open` items addressed to it.

| Phase | Build (design §) | Exit criteria (all green, recorded) |
| --- | --- | --- |
| **P4**, if not committed | §4 whole: `Workbook`-level insert/remove of rows and columns, rewriting every reference (formulas, names, CF, DV, hyperlinks, autoFilter, tables, drawing anchors, comments and VML, chart `c:f`) through one adjuster in `excel/shift.rs`. Blocking elements are refused by name. Layout setters. Fill (series and copy), sort, paste, replace, `find_next`. `Edit`/`Journal` undo and redo, with exact inverses proven by test. `Edit::from_scalar`. Addendum 6, 10, 11 | `cargo test -p yggdryl --test excel`; the same with `--features internals`; `--test allocations -- excel`; `--test iobase_calls -- excel`; `--doc excel::`; `cargo test -p yggdryl --bench media -- excel`; rustdoc `-D warnings`; the interop script, including an openpyxl check that inserted rows and columns shifted every reference exactly |
| **P5** | §5: the formula evaluator. Parser, a crate-private `Operand` value type, 15-significant-digit number rules, evaluator, dependency graph with incremental recalc, cycle report, volatile functions with an injected `Clock`, one `Accumulator` (also used by SUBTOTAL, pivots and the status bar), `*IF(S)` criteria, **159 functions** (list in §5.6), `_xlfn` registry. Anything it cannot compute exactly is *uncomputed*, never guessed | `--test excel -- formula`; allocation pins (`SUM` allocates nothing per cell; a chain recalculation evaluates each node once); `cargo bench -p yggdryl --bench media -- excel_recalc --quick` runs; **plus the Excel oracle of §8.3** |
| **P6** | §6: pivot tables. Tabular form, populated `sharedItems`, pivot items and `rowItems`/`colItems` with `t="grand"`, an empty `pivotCacheRecords` part, `saveData="0" refreshOnLoad="1"`, location offsets counting the Σ Values pseudo-field. Create, update, refresh and remove. Foreign pivots are parsed, editable when representable. openpyxl in both directions | `--test excel -- pivot`; `--test allocations -- excel_pivot`; the interop script; **plus the Excel check of §8.4** |
| **P7** | §7: generic HTTP in `rust/src/http/`. Pattern routes `{name}`/`{*rest}` with a handler `Context`; `http::Assets` registered as ordinary GET routes; `Conditions` (304/412) applied to any 200 with a validator; `Status::from_error` with RFC 9457 `application/problem+json`; panic containment; `Stopper`/`shutdown_within`; allowed hosts; default security headers. XMLA moves onto them, and every `.route(` caller is swept, including `python/src/http.rs` and `node/src/http.rs`. Beware the Deref trap: `request.clone()` on `&Context` clones the reference | `cargo check -p yggdryl --all-targets --features http`; `cargo test -p yggdryl --features http --test http -- server assets headers::conditional`; `--features http --test xmla -- server`; `--features "http internals" --test allocations -- http_` |
| **P8** | §8: the Excel service in `rust/src/excel/server*`, `#[cfg(feature = "http")]`. `Service`/`ServiceOptions` and their locking; build under a read lock, write with no lock, rebase under a short write lock. The full JSON API of §8.3–8.4, **plus every addendum item**: `GET find`, `GET format`, `instance` in `GET workbook` and in the copy marker, `text` in `replace`, `fillEntry` with `ref`, widths in `<col width>` units with default 9.140625, `fill` as `[char, offset]`, `activeSheet` as a sheet key. Tile ETags, the change-feed long poll, Get Data preview and load, save with the size/mtime conflict check, the security posture of §8.2, and `RecordOptions::mime_types()` | `cargo test -p yggdryl --features http --test excel -- server`; the service rows in `iobase_calls` and `allocations`; `cargo test -p yggdryl --test media -- options` |
| **P9** | §9: the CLI. `cli/src/serve.rs` with the server flags lifted out of `cli/src/xmla.rs`, which moves onto it. `cli/src/excel.rs`, `yggdryl excel serve`. The UI embedded from `cli/assets/excel/` with `include_bytes!`, plus a test that the embedded list equals the folder. The token from `getrandom` (already in `Cargo.lock`), sent in the URL fragment. `signal-hook` shutdown. `--max-cells`. An `s3` cli feature, and `scripts/stage_cli.py` builds with `iceberg,s3`. `scripts/check_wheel_smoke.py` gains `serve_excel`. Then point the UI dev server at the real binary (§7) and fix what the UI gets wrong | `cargo check -p yggdryl-cli --all-targets`; `cargo test -p yggdryl-cli --test excel --test xmla`; `cargo clippy -p yggdryl-cli --all-targets --no-deps -- -D warnings`; the UI browser check passing against the real binary |
| **P9b** | The modern look with full light and dark modes. UI files only, so it can run beside P10 and P11. The instructions are in §7.1. **It is part of this PR, not optional polish** | Every item of §7.1's checklist, and its verification passing in both colour schemes |
| **P10** | §11, Python: new surfaces bound in `python/src/excel.rs` and `http.rs`, `_native.pyi` stubs, parity tests, boundary benchmark rows. `...` skips a `StylePatch` field and `None` clears it. `Workbook.__hash__ = None`. `write_into` redirects to `Workbook::write_into` | `python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_excel.py python/tests/test_http.py -x -q`; `python/.venv/bin/python -m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py` |
| **P11** | §11, Node: `node/src/excel.rs` and `http.rs`, `binding.js`/`.d.ts`, parity tests, types tests, benchmark rows. Regenerate `node/index.js`/`index.d.ts` and the docs manifests | `npm run --prefix node build:debug`; `node --test node/tests/excel.test.js node/tests/http.test.js`; `npm test --prefix node`; `node scripts/build_docs_playground.js --check && node scripts/build_docs_fix.js --check` after regenerating both; `git diff --exit-code -- node/index.js node/index.d.ts` |
| **P12** | §12: docs and skills. `docs/media/index.md` Excel section: saving and fidelity, styles, formulas (with §5.1's paragraph verbatim), editing, pivot tables, serving a workbook. Also `docs/holder/index.md` server sections, `skills/yggdryl-records/**`, `skills/yggdryl-storage/**`, and both inventories. **Then the whole run** (below) | `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_api_inventory.py` clean; `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript` |

**The whole run** (P12). Run it as one background chain writing one log with a marker per step:

1. `cargo test --locked --workspace --all-targets --all-features --no-fail-fast`
2. `cargo clippy --locked -p yggdryl --all-targets --no-deps -- -D warnings`
3. `cargo clippy --locked --workspace --all-targets --all-features --no-deps -- -D warnings`
4. `cargo test --locked -p yggdryl --doc`
5. `cargo test --locked -p yggdryl-cli --all-targets`
6. `pytest python/tests` and mypy
7. `npm run --prefix node test:package:debug`, `npm test --prefix node`, and both manifests with `--check`
8. `cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets`

Re-pin only non-cost pins the change explains exactly, each with its reason.

**Your own review after each phase.** The earlier session found 12–21 real defects per phase this way, so budget for it. Re-read the phase's diff adversarially, through two lenses:

- **Design and contract conformance:** every promised item present, replaced symbols deleted, callers updated, vocabulary, tests mirrored, pins where promised.
- **Correctness:** XML element order and attributes, off-by-one in ranges and references, lossy round trips, panics reachable from input (slicing inside a UTF-8 character, unchecked indexing, integer overflow), non-atomic mutations, wrong Excel semantics, and tests that assert too little.

Prove each suspected bug with a failing test before you fix it.

**Checkpoints.** After each phase, commit `WIP Pn: …` and push to the branch. That triggers no CI (see §9).

**Disk.** The target directory grows quickly, about 15 GB of incremental cache over a few phases. When space runs low, delete `target/debug/incremental` and stale large test executables in `target/debug/deps`.

## 7. The UI and its development tools

- **Where it lives.** The app is vanilla ES modules in `cli/assets/excel/` (`index.html`, `styles.css`, `icons.svg`, `js/*.js`), with no framework, build step or CDN. It stays CSP-clean: no inline style or script, no `innerHTML`.
- **The dev server.** `python3 handoff/ui-dev/dev.py --port 8765 --token abc` serves the assets from your working tree at `/`, and answers `/api/*` from `handoff/ui-dev/devstore.py` and `fixtures/`. It finds the repository through `YGGDRYL_REPO`, else two folders up.
- **From P9 on:** make it proxy `/api/*` to `target/debug/yggdryl excel serve --no-token --bind 127.0.0.1:0`, or serve the real binary directly. The UI must work against the real service.
- **Checks:**
  - `node handoff/ui-dev/unit.mjs`: the unit tests.
  - `NODE_PATH=$(npm root -g) node handoff/ui-dev/check.js`: the Playwright browser check (about 71 steps). It asserts through the hidden ARIA grid mirror and fails on any console error.
  - `for f in cli/assets/excel/js/*.js; do node --experimental-default-type=module --check "$f"; done`: a syntax check. The files are ES modules, so the flag is required.
- **CI.** Design §10 and §12.5 add `scripts/check_excel_ui.py` (Python Playwright) and an `excel-ui` CI job. Write both during P7–P9, porting the essential steps of `check.js`.
- **Contract changes.** Any change to the JSON contract goes into `handoff/contract-addendum.md` first, then into both the service and the UI.

### 7.1 Phase P9b: a modern look, with full light and dark modes

**Goal.** Today's chrome is a plain Excel-blue theme with one light palette: `:root` custom properties in `cli/assets/excel/styles.css` plus a `forced-colors` override. Restyle the whole app in the visual language of **Next.js and Vercel**:

- neutral gray scales and one accent colour
- 1px hairline borders at low-alpha gray
- soft layered shadows in light mode; in dark mode, borders and slightly raised surfaces instead of shadows
- restrained radii and generous whitespace inside a compact, dense layout
- crisp typography
- a first-class dark mode

It must still read as a spreadsheet: the ribbon, formula bar, grid and sheet tabs keep their places and behaviour. Only the look changes.

**Take the style, not the brand.** Reproduce the *style*, with no Vercel or Next.js logos, wordmarks, names or other proprietary assets. The values below are your own approximations. Tune them, and prove contrast rather than copying numbers.

**Constraints that stay:**

- No framework, build step or CDN.
- CSP-clean: no inline style or script, no `innerHTML`.
- `forced-colors` still overrides both themes.
- `prefers-reduced-motion` is honoured.
- Every control stays keyboard reachable with a visible focus ring.
- The hidden ARIA grid mirror is untouched.
- **The workbook's own cell fonts keep rendering as authored** (Calibri, with Carlito as fallback). Only the chrome typography changes.

**1. Typography.**

- UI: `ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif`.
- The name box, formula bar and function tips: `ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace`.
- Optionally vendor Geist Sans/Mono `.woff2` (SIL OFL 1.1) into `cli/assets/excel/fonts/`, with the OFL text beside them. Use `font-display: swap`. Add them to the embedded asset list and its equality test (P9), and serve them from `'self'` only.
- Type scale: 12/13/14 px, weights 400/500/600, `letter-spacing: -0.01em` on headings.

**2. Design tokens.** Put one semantic set in `:root`. Every component uses only these tokens: no literal colours outside the token block. Starting values:

| Token | Light | Dark |
| --- | --- | --- |
| `--bg-100` (page, sheet) | `#ffffff` | `#0a0a0a` |
| `--bg-200` (chrome, ribbon, status bar) | `#fafafa` | `#000000` |
| `--gray-100` (hover) | `#f2f2f2` | `#1a1a1a` |
| `--gray-200` (pressed, active tab fill) | `#ebebeb` | `#1f1f1f` |
| `--gray-300` (headers, gridline strong) | `#e6e6e6` | `#292929` |
| `--gray-alpha-400` (hairline border) | `rgba(0,0,0,.08)` | `rgba(255,255,255,.14)` |
| `--gray-700` (disabled, placeholder) | `#8f8f8f` | `#7c7c7c` |
| `--gray-900` (secondary text) | `#666666` | `#a1a1a1` |
| `--gray-1000` (primary text) | `#171717` | `#ededed` |
| `--accent` / `--accent-hover` | `#0070f3` / `#0060d1` | `#3291ff` / `#52a8ff` |
| `--accent-soft` (selection fill, pressed toggles) | `rgba(0,112,243,.10)` | `rgba(50,145,255,.18)` |
| `--danger` / `--warning` / `--success` | `#e5484d` / `#f5a524` / `#16a34a` | `#ff6369` / `#ffb224` / `#3dd68c` |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,.04)` | `0 0 0 1px rgba(255,255,255,.08)` |
| `--shadow-menu` | `0 0 0 1px rgba(0,0,0,.08), 0 4px 8px -4px rgba(0,0,0,.04), 0 16px 24px -8px rgba(0,0,0,.08)` | `0 0 0 1px rgba(255,255,255,.12), 0 16px 24px -8px rgba(0,0,0,.6)` |
| `--focus-ring` | `0 0 0 2px var(--bg-100), 0 0 0 4px var(--accent)` | same expression |
| `--radius-sm` / `--radius-md` / `--radius-lg` | `6px` / `8px` / `12px` | same |

Also give the grid colours their own tokens in both schemes: `--sheet`, `--grid`, `--header`, `--header-text`, `--header-line`, `--header-active`, `--header-active-text`, `--header-full`, `--selection`, `--frozen`, `--text`, `--pending`, and the reference-highlight palette of point mode. Derive each from the semantic set above.

Then **delete** today's literal-valued `--ui-*`, `--accent*` and `--menu-shadow` tokens, and move every rule onto the new ones. One owner per colour.

**3. Light, dark and system.**

- `:root { color-scheme: light dark; }` plus `<meta name="color-scheme" content="light dark">`, and two `<meta name="theme-color">` tags carrying `media="(prefers-color-scheme: light|dark)"`.
- Three preferences: **System** (the default), **Light**, **Dark**. Reach them from **View › Theme** and from a compact toggle in the title bar.
- Selectors:
  - light values in `:root`
  - dark values in `@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { … } }`
  - dark values again in `:root[data-theme="dark"] { … }`
  - `forced-colors` last, winning over all of them
- Persist the choice in `localStorage` inside `try/catch`, because storage can be absent or throw. Sync it to other tabs through the `storage` event.
- **No flash of the wrong theme:** a tiny blocking **external** classic script, `js/theme-boot.js`, loaded from `<head>` before the stylesheet, sets `data-theme` before first paint. It must be external, never inline, so the CSP holds.
- **One `themechange` path.** Fire it on the toggle, on the `matchMedia('(prefers-color-scheme: dark)')` `change` event and on the `storage` event. It re-reads the canvas palette probes (`render.js`), then repaints every quadrant, header and drawn scrollbar. Generalize the existing `forced-colors` listener in `grid.js` into this path, so one mechanism serves both.

**4. Document colours in dark mode.** This is the part that needs care, so spell it out in the code's comments.

- **Chrome and grid furniture** follow the theme: headers, gridlines, selection, frozen dividers, the fill handle, marching ants, reference highlights, scrollbars.
- **Cell content, modelled on Excel 365's dark-mode behaviour:**
  - A cell with **no fill** paints `--sheet`.
  - **Automatic** font and border colour paint `--text`.
  - An **explicit** fill or font colour paints exactly as authored.
  - **Automatic** text on an **explicit** fill picks black or white by the fill's WCAG relative luminance, so it stays legible.
- Add **View › Theme › "Dark cells"** (default on). When off, the sheet renders as printed, in white with black automatic text, inside dark chrome. Excel offers the same switch.
- This is render-time mapping only; the server's resolved colours never change. It needs the styles payload to tell automatic from explicit: an automatic font or border colour and a missing fill must arrive as `null`, never as a resolved `#000000`/`#FFFFFF`. Addendum item 12 already states this rule; P8 must implement it.

**5. The component checklist.** Restyle each one and check it in both schemes:

- title bar, with the theme toggle
- ribbon tabs: text tabs with a 2px accent underline on the active tab, sliding under reduced-motion rules
- ribbon buttons: ghost style, `--gray-100` on hover, `--accent-soft` when pressed
- split buttons, dropdowns and colour pickers
- name box and formula bar: inputs with hairline borders and `--focus-ring`
- row and column headers
- sheet tabs: a quiet pill or underline style, the active tab raised
- status bar
- context and dropdown menus: `--radius-lg`, `--shadow-menu`, 32px rows, a leading icon column, and keyboard shortcuts right-aligned in `--gray-900`
- dialogs: modal cards with `--radius-lg`, backdrop `rgba(0,0,0,.4)` in light and `.6` in dark, `backdrop-filter: blur(4px)` where supported
- toasts: a stacked bottom-right card with a leading status icon
- tooltips and argument tips
- the PivotTable fields pane, and the Get Data dialog with its preview table
- the Format Cells, Find/Replace, Sort, Go To and Insert Function dialogs

**Icons.** Every icon in `icons.svg` uses `currentColor` only, at a consistent 16/20px grid with 1.5px strokes.

**Motion.** 150 ms ease-out on hover, press and focus. Nothing animates under `prefers-reduced-motion: reduce`.

**6. Verification.** The phase is done only when all of these pass and you have shown the user before and after screenshots:

- Extend `handoff/ui-dev/check.js` so the whole walk runs **twice**, under `page.emulateMedia({ colorScheme: 'light' })` and `({ colorScheme: 'dark' })`, plus once with the explicit toggle overriding the system scheme.
  - In each run, assert `getComputedStyle(document.documentElement).colorScheme` and the resolved `--bg-100`.
  - Sample a canvas pixel of an empty cell before and after toggling: near-white, then near-black. This proves the repaint happened.
  - Assert no console errors.
  - Assert that `data-theme` is already set at `DOMContentLoaded` when a preference is stored (no flash).
- With **Dark cells** on and off, check that an explicitly filled cell and an automatic-text cell render as §4 says.
- Save screenshots of every surface in the checklist, in both schemes, into one folder.
- Add a small Node contrast script under `handoff/ui-dev/` (or `scripts/`, if you make it a committed check). It computes WCAG ratios for every text/background token pair, in both schemes, including the tokens that feed the canvas. It requires 4.5:1 for text and 3:1 for UI boundaries and focus rings. Adjust the tokens until every pair passes.
- `forced-colors` emulation (`page.emulateMedia({ forcedColors: 'active' })`) still renders in system colours in both schemes.
- The `excel-ui` CI job (`scripts/check_excel_ui.py`) runs its steps in both colour schemes.
- Docs (P12): the "Serving a workbook" section mentions the theme preference and Dark cells, with one screenshot per scheme only if the docs already use screenshots; otherwise text only.

## 8. Your distinguishing job: verify against Microsoft Excel

Neither CI nor the earlier session could run Excel. You can. Write `scripts/check_excel_desktop.py`, a **local-only** check: no CI job, documented as local-only like `scripts/check_charset_interop.py`. Run it at every point below, and commit the Excel-produced fixtures it yields.

### 8.1 Harness

A Windows skeleton follows. On macOS, use xlwings with the same logic.

```python
import glob, os, tempfile, win32com.client

def excel():
    app = win32com.client.DispatchEx("Excel.Application")
    app.Visible = False
    app.DisplayAlerts = False
    return app

def open_checked(app, path):
    """Open `path`; fail if Excel repaired it (Excel writes a repair log to %TEMP% when alerts are off)."""
    before = set(glob.glob(os.path.join(tempfile.gettempdir(), "error*.xml")))
    book = app.Workbooks.Open(os.path.abspath(path), UpdateLinks=0)
    repaired = set(glob.glob(os.path.join(tempfile.gettempdir(), "error*.xml"))) - before
    if repaired:
        raise AssertionError(f"Excel repaired {path}: " + open(repaired.pop(), encoding="utf-8", errors="replace").read())
    return book
```

**Reading values:**

| Want | Use |
| --- | --- |
| raw values | `Range.Value2` |
| Excel's own formula results | `app.CalculateFull()` first |
| format rendering | `app.WorksheetFunction.Text(value, code)`. Avoid `Range.Text`, which depends on column width |
| pivots | `sheet.PivotTables().Count`, `pt.TableRange2.Address`, `pt.RefreshTable()`, `pt.RowFields` / `ColumnFields` / `DataFields` |
| colours | `Range.Interior.Color` / `Font.Color` (BGR integers) |
| saving through Excel | `book.SaveAs(path, 51)` (xlsx) |

Record the Excel version (`app.Version`) in every fixture's accompanying note.

### 8.2 Fidelity: do this first, now

1. Dump `rich_package()` from `rust/tests/support/excel_package.rs` to a file; a throwaway test that writes the bytes is fine. Also keep the workbooks `scripts/check_excel_interop.py` produces, and produce a workbook after inserting a row and a column on a sheet carrying conditional formatting, data validation, hyperlinks, an autofilter, a table, a chart, comments and merges.
2. Open each in Excel. There must be **no repair log**, every feature must be intact, and every shifted reference must match what Excel does when you make the same insert on the original file.
3. Save one representative file from Excel, commit it as `rust/tests/excel/fixtures/rich_excel.xlsx` with a note, and add a Rust pin that reading and rewriting it is lossless under the rules of design §2.1.

### 8.3 Number formats (P3's `format.rs`) and formulas (P5)

- **Formats.**
  - For every (code, value, text) vector in `rust/tests/excel/format.rs`, compare the crate's text with `WorksheetFunction.Text(value, code)` under en-US.
  - A mismatch is a bug in `format.rs`. Fix it, update the vector, and record in the pin's comment that Excel confirmed it. Pay special attention to the 24 vectors marked as following Excel against LibreOffice.
  - Commit a fixture (xlsx or JSON) of Excel's answers, plus a test that reads it.
- **Formulas, after P5.**
  - Write the function test vectors into an xlsx through openpyxl with no cached values. Open it in Excel, `CalculateFull()`, and save it as `rust/tests/excel/fixtures/functions_excel.xlsx`.
  - Pin that `Workbook::calculate_all` reproduces every cached value under the 15-significant-digit rule.
  - Include at least: `ROUND(2.675,2)`, `=0.1+0.2=0.3`, `=1-0.9-0.1`, `WEEKDAY(61)`, `DATE(1900,2,29)`, `MOD` with negative operands, mixed-type comparison order, approximate `MATCH`/`VLOOKUP` on sorted data, `TEXT`, `SUMIFS` with wildcards, and `SUBTOTAL` 1–11 and 101–111 over hidden rows.

### 8.4 Pivot tables (P6)

1. Write pivots of several shapes: one row field; a row and a column field; two value fields (the Σ Values column); subtotals and grand totals each on and off.
2. Open each in Excel. There must be **no repair log**. `RefreshTable()` must leave `TableRange2` and every value equal to the cells the crate rendered.
3. Save one from Excel as `rust/tests/excel/fixtures/pivot_excel.xlsx`, and pin that the crate reads it as `editable` with an equal spec.

### 8.5 Styles (P3)

A workbook styled through `set_style` must show in Excel the colours `StyleSheet::resolve` answers: theme colours with tint, indexed colours, fills, borders and fonts.

### 8.6 The app end to end (after P9)

- Run `yggdryl excel serve some.xlsx`, edit through the browser (values, formulas, styles, a row insert, a pivot), save, and open the result in Excel. There must be no repair, and every edit must be present.
- Open an Excel-authored workbook in the app and compare the grid with Excel's rendering.

## 9. Finishing

1. **Merge a moved main.** If `main` has moved, merge it into the branch and resolve conflicts. Regenerate lockfiles and generated files with the repository's tools, never by hand.
2. **Remove the record.** `git rm -r handoff/`. It must not ship.
3. **Squash to ONE commit** saying what the tree is (AGENTS.md "One commit"): `git reset --soft $(git merge-base HEAD origin/main)`, then one `git commit`. Force-with-lease push the feature branch; it is not shared.
4. **Open the PR.** CI (`.github/workflows/ci.yml`) runs only on pull requests and pushes to `main`, so this is the first CI run. Use `gh pr create` or the web UI, and mirror the repository's PR template if one exists.
5. **Read every CI job to green.** Fix each red job at its root cause; never re-run to see if it passes, and never skip or relax a check. The jobs are two Rust quality lanes, Core Rust 1.85, Iceberg Rust 1.94, the exchange jobs (including `excel-interop` with openpyxl, and the new `excel-ui`), the Python binding on two pyarrow legs, the Node binding, and documentation examples.
6. **Hand off.** State:
   - CI status per job
   - the local-only checks run, with the Excel harness results and the Excel version
   - the checks skipped, and why
   - the AGENTS.md layout rows design §12.6 proposes, as proposals only, applied **only if the user approves**
