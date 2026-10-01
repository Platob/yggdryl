# `yggdryl excel serve`: final design

Base commit `19ea5377`. Every `file:line` below was checked at that commit. This document is the single source of truth for the PR. It starts from the judges' winner, **fidelity-first**, fixes every defect they listed against it, and adopts the grafts they named. §0 records each decision taken to resolve a question. Implementers follow this text. Where it is silent, AGENTS.md decides.

Ordering principle, unchanged from the winner: **nothing we write may lose what Excel wrote.**

- Fidelity lands before any editing surface.
- A value the engine cannot compute the way Excel does is *uncomputed*, never guessed, and every dirty save asks Excel to recalculate.
- A structural edit the crate cannot carry through every reference in the package is refused, and the refusal names the part.
- The browser implements no semantics. It renders what the server answers. The only exceptions are font-metric measurements (§10).

---

## 0. Decision log (judge defects and grafts, resolved)

| # | Question or defect | Decision (section) |
| --- | --- | --- |
| 1 | Dirty predicate was wrong for unparsed slots | A slot is dirty iff it has no part, or its parsed sheet's revision differs from `Slot::saved`. An unparsed slot is clean (§2.3) |
| 2 | Cell memory (~112 B, BTreeMap rows, O(rows) `dimension`) | `Cell` ≤ 80 B, const-asserted: `StyleId(u16)`, `Formula` is one `Arc`, rare facts go in a sparse per-sheet `extras` map. `Row.cells` becomes a sorted `Vec`. A cached `Extent` makes `dimension()` O(log rows) and `extend_from_serie` linear. Heap per cell and per row pinned (§2.2) |
| 3 | Crate-wide atomic `LocalFile` write | **Dropped.** The service opens a snapshot (`read_all_bytes`, then `from_bytes`). `Workbook::write_into` swaps its source for the written bytes *before* writing, so no mapping of the target is alive during the write. `local/` is untouched (§2.10) |
| 4 | Inflate and re-deflate of carried members | `ZipArchive::copy_member_from` copies compressed bytes, CRC and sizes raw (§2.11) |
| 5 | Save over a file changed since open | Size and mtime are captured at open and re-read before save. A mismatch answers 409 `modified` unless `force` (§8.8) |
| 6 | Structural edits blocked by every drawing or table | Drawing anchors, table/autoFilter/sortState `ref`, comments `ref`, VML anchors and chart `c:f` are shifted. Refused only: a column band crossing a table, removing a table's header or all its data rows, splitting an array/data-table/foreign-pivot range, and the named blocking elements (§4.4) |
| 7 | Shared formulas expanded to per-cell text | A formula is a **shape**: verbatim text runs plus relative references, one `Arc` per shape. Shared dependents clone the master's `Arc`. Structural rewrites are memoized per (shape, host class) (§5.2) |
| 8 | User spelling vs. relative storage | Only reference tokens are re-rendered. Every other character of the user's spelling is a verbatim run (§5.2) |
| 9 | Rename re-parses formulas | Shapes name sheets by name. A rename rewrites only the shapes whose `sheets` list names the old sheet: one rewrite per distinct shape, then pointer swaps. There is no text re-parse. `Cell` stays self-contained, so its text renders without a workbook. Carried parts are text-rewritten through the same adjuster (§4.4) |
| 10 | Public `Value` enum in the formula engine | The evaluator's value is the crate-private `Operand`. `Entry::parse` is renamed `Entry::from_text`, `Status::of` is `Status::from_error`, and `FormatCode::with_decimals` consumes `self` (§3, §5, §7) |
| 11 | numFmt 41–44 are not ECMA built-ins | The writer never references an id outside ECMA-376 §18.8.30's table (0–22, 37–40, 45–49) without declaring it in `numFmts`. The ribbon's Comma and Accounting styles intern custom ids ≥ 164 (§3.4) |
| 12 | Pivot XML must declare what it renders | Tabular form everywhere: table-level `compact="0" compactData="0" outline="0" outlineData="0"` and per-field `compact="0" outline="0"`. Axis fields get populated `sharedItems`, `items` and `rowItems`/`colItems` with `t="grand"`. An empty `pivotCacheRecords` part carries `count="0"`, reached through the cache's `r:id`, with `saveData="0" refreshOnLoad="1"`. Verified against openpyxl 3.1.5 in `python/.venv` (load, save, reload) (§6.4) |
| 13 | Pivot location off by one; Σ Values pseudo-field | `c' = c + (v > 1)`. Header rows = `1 + c'`. `firstDataRow = 1 + c'`. Location rows = `1 + c' + rowItems.count`. A worked example checks it: `A3:D8` (§6.3) |
| 14 | Unknown `t="e"` literal made the file unopenable | It is carried: `ExcelError::Unrecognized`, with the literal kept in the cell's value slot. A formula reading it is uncomputed (§2.2) |
| 15 | Error mapping and envelope | `Status::from_error` plus RFC 9457 `application/problem+json` with `kind`, `location` and `revision` members. `Unsupported` maps to **422**, never 501 (§7) |
| 16 | Assets as a third dispatcher | `http::Assets::route` registers ordinary GET routes `{prefix}` and `{prefix}/{*path}`. `answer_routed` turns any 200 GET/HEAD answer that carries `ETag`/`Last-Modified` into a 304 through `Conditions` (§7) |
| 17 | Where the UI lives, and `--assets DIR` | Judges disagreed. **UI in `cli/assets/excel/`**, so the core crate carries no JS and the core service is a headless JSON API. **No `--assets` product flag**: the UI agent iterates with a scratchpad dev server that serves the working tree and proxies `/api` (§13) |
| 18 | Six-connections-per-host limit | Long poll held by one Web Locks leader per browser and relayed over `BroadcastChannel` (§10) |
| 19 | Token source and transport | `getrandom 0.3` (0.3.4 locked at `Cargo.lock:1496`), CLI only. The token travels in the URL **fragment**, never the query, and is sent back as `X-Yggdryl-Token` (§8.2) |
| 20 | Completeness gaps | Find/Replace, Go To, fill handle with series AutoFill, multi-range selection and Format Painter are added (§4, §10) |
| 21 | One writer | `write_fresh`, `write_workbook`, `package::write_content_types` and `package::write_workbook_relationships` are deleted. A new workbook writes through the preserving writer over the `Documents::empty()` template (§2.9) |
| 22 | NOW/TODAY/RAND testable | `Clock` is injected through `Workbook::with_clock` and `ServiceOptions::with_clock`. RAND is seeded per pass from it (§5.5) |
| 23 | One aggregate owner | `excel/formula/aggregate.rs` `Accumulator` serves the functions, `SUBTOTAL` (incl. 101–111), pivot compute and `/summary` (§5.4) |
| 24 | Edit intake | `Edit::from_scalar(&Scalar, &Workbook)` gives located errors (`$.edit.entries[3].ref`). The JSON crosses through `Request::scalar` (§4.6, §7) |
| 25 | Get Data media list owner | `RecordOptions::mime_types()`. `for_mime_type`'s refusal text renders from it (§8.7) |
| 26 | StylePatch skip vs. clear in bindings | `Option<Option<_>>` in Rust. Python `...` skips and `None` clears; JavaScript `undefined` skips and `null` clears. `set_style` and pivots are bound (§11) |
| 27 | Held formulas not shifted (array `ref` goes stale) | "Held" means *not evaluated*. Every formula is shifted: array `ref`, data-table `r1`/`r2`. A partial overlap is refused (§4.4) |
| 28 | Save order on failure | Build the package, adopt it as the source, write it, and only then mark saved. A failed write leaves the workbook dirty (§2.10) |
| 29 | S3 PUT under the write lock | The service builds under the read lock, writes with no lock held, then rebases under a short write lock. Edits made meanwhile stay dirty (§8.1) |
| 30 | Docs before bindings | Docs and skills are written after Node settles, while the chain runs (§13) |
| 31 | Invalid multi-filter cargo commands | Every command is `cargo test … --test T -- a b c` (§13) |
| 32 | `scripts/excel_ui_mock.py` in the tree | The mock lives in the scratchpad only. Nothing mock-like is committed (§13) |
| 33 | Route sweep scope | 53 `.route(`, 18 `.respond(` and 16 `.inject(` Rust sites, plus 35 in docs. Includes `node/src/http.rs:1677,1699` and `python/src/http.rs:2434`. The `Deref` trap is called out (§7.1) |
| 34 | Python `Workbook` hash stub mismatch | `#[classattr] const __hash__: Option<Py<PyAny>> = None;` (§11) |
| 35 | AGENTS.md rows | Proposed text only (§12.6). It is applied only with the user's approval and is listed in the handoff |
| 36 | Why a formula engine is not a second expression engine | The paragraph in §5.1 goes into the docs verbatim |

---

## 1. Scope and non-goals

**In scope**

| Area | Covered |
| --- | --- |
| Files | Open any `.xlsx` a `Holder` reaches: local path, `file://`, local ZIP member `file:///a.zip#b.xlsx`, `s3://`/`gs://`/`az://` (CLI feature `s3`), `http(s)://`. Also a browser upload, and **New**. **Save** (to the current location, with a conflict check), **Save As** (any allowed location), **Download** |
| Fidelity | Every part, element and attribute not modelled is carried byte for byte. Modelled facts are regenerated: cells, styles, column/row formats, merges, frozen pane, defined names, sheet list, our pivots. Every carried reference a structural edit moves is rewritten, or the edit is refused by name |
| Grid | Canvas grid virtualized over 1,048,576 × 16,384. Selection, including multi-range with Ctrl+click. Excel keyboard map. In-cell and formula-bar editing with typed en-US entry, point mode, F4. Copy, cut and paste through the system clipboard; internal paste translates formulas. Paste values/formulas/formats, Format Painter. Undo/redo (100). Insert/delete rows and columns, widths/heights, hide/unhide, merge/unmerge, freeze panes. Fill down/right, fill handle with series AutoFill. Sort, Find/Replace, Go To. Sheet tabs: add, rename, remove, move, hide |
| Formatting | Ribbon: font name and size, bold/italic/underline/strike, font colour, fill, border presets, horizontal/vertical alignment, wrap, indent, number formats (presets, custom code, ±decimals, `$ % ,`). Existing styles render from the file, including theme and indexed colours with tint |
| Formulas | A1 grammar (§5.3), 159 functions (§5.6), legacy implicit-intersection semantics, a dependency graph with incremental recalc, cycle report, volatile functions |
| Pivot | Create, edit, refresh and remove a tabular pivot table from a sheet range. Rows and columns, several value fields, 11 aggregations, subtotals, grand totals, item order. Written as real `pivotCacheDefinition` + `pivotCacheRecords` + `pivotTableDefinition` parts that Excel re-renders on load and openpyxl reads and re-saves. Foreign pivots are parsed; the representable ones become editable, the rest are carried read-only |
| Get Data | Ribbon **Data › Get Data**: any location through `Holder::from_url` and the centralized `RecordOptions` / `IOMedia::read_arrow` (`SerieReader`). Previewed, then landed through `Sheet::write_serie` as one undoable edit |
| Serving | `yggdryl excel serve [LOCATION]` on the crate's `http::Server`. The generic router, context, assets, conditional requests, error mapping, panic containment, graceful shutdown, host allow-list and default headers all land in `rust/src/http/`. xmla moves onto them |

**Non-goals.** Each is refused by name wherever a request reaches it.

- **Editing** charts, images, comments, tables (ListObjects), slicers, sparklines, conditional formatting, data validation or defined names. All are carried and shifted (§4.4); none is edited.
- AutoFilter UI. Existing filters are carried and shifted, and their hidden rows are honoured.
- Pivot report filters, value/label filters, date grouping, calculated fields/items, "show values as", compact/outline authoring, pivots with no row field or no value field.
- Dynamic-array spill, `LET`/`LAMBDA`, CSE entry, R1C1 entry, iterative calculation, manual calculation mode, external links. Existing ones are carried, shifted and uncomputed.
- Locales other than en-US, for entry, display and `VALUE`.
- Macros. `.xlsm` parts are carried; nothing runs.
- CSV/TSV import: `RecordOptions::for_mime_type` refuses `text/csv` (`media/options.rs:1544`), and the dialog shows that refusal. A delimited medium is its own PR.
- Remote ZIP members (`zip::from_url` is local-only).
- Real-time co-editing. Tabs of one server get per-range stale-base conflicts (§8.6).
- Copying styles between workbooks. A sheet inserted from another workbook keeps its values, formulas and number-format meanings; its visual styles reset to the default xf (§2.2).
- The Python/Node `located_holder` bypass of `Holder::from_url`, which is follow-up work across every binding door.

---

## 2. Core fidelity (`rust/src/excel/`), phases P1–P2

### 2.1 The invariant and its pins

> **Read, save and reopen change nothing but what was edited.** Every member that is not regenerated is byte-identical. A regenerated part equals the original after XML normalization, except for this closed list:
>
> - worksheet: `dimension@ref` recomputed; `row@spans` dropped; a shared formula group on a *rewritten* sheet written as one plain `<f>` per cell; number `<v>` text written as the shortest round-trip of the same double; `<c>` attributes other than `r s t cm vm ph` and `<c><extLst>` dropped
> - workbook part: `calcPr@fullCalcOnLoad="1"` added
> - `sst`: `count` removed, `uniqueCount` updated, new items appended
> - styles: new entries appended, counts patched
> - `[Content_Types]` and workbook rels: calcChain removed, new parts added
> - `xl/calcChain.xml`: deleted

`rust/tests/support/excel_package.rs` gains `rich_package() -> Vec<u8>`, hand-built in Excel's spelling. It carries:

- a frozen pane with selections, `<cols>`, row `ht`/`hidden`/`s`/`x14ac:dyDescent`, mergeCells
- legacy and x14 conditional formatting, data validation, hyperlinks (`r:id` and `location`), autoFilter with `filterColumn`, sortState
- tableParts + table part, drawing + a chart referencing the sheet, legacyDrawing + comments + VML, an x14 sparkline group
- a shared formula group, an array formula, a dynamic-array formula (`cm="1"` + `metadata.xml`), a data-table formula
- rich, phonetic and duplicate shared strings, and a rich inline string
- fonts, fills, borders, alignment, percent and currency xfs, theme colours with tint
- `definedNames` with `localSheetId`, `bookViews@activeTab`, calcChain
- a foreign pivot cache and table, `docProps`

Pins, in `rust/tests/excel/workbook.rs`:

- `a_read_workbook_writes_back_what_it_read`: every sheet parsed, nothing edited, `into_bytes` → each member compared.
- `an_edited_cell_changes_only_its_cell`
- `a_saved_workbook_reopens_in_place`: `write_into` to the same handle, then read and edit again.
- `a_failed_write_leaves_the_workbook_dirty`: the target refuses the write.

### 2.2 Storage (`cell.rs`, `sheet.rs`, `style.rs`)

```rust
// cell.rs
pub struct Cell {
    value: Scalar,               // typed value, a formula's cached result, or an Unrecognized error's literal
    reference: CellRef,
    formula: Option<Formula>,    // one Arc (§5.2)
    style: StyleId,              // cellXfs index
    kind: CellKind,
    format: NumberFormat,        // value meaning, derived from the xf's code once (§3.4)
    error: Option<ExcelError>,
}
const _: () = assert!(std::mem::size_of::<Cell>() <= 80); // Scalar is 48 B (scalar.rs:318); 16-aligned via i128

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ExcelError { Null, Div0, Value, Ref, Name, Num, NA, GettingData, Spill, Calc, Field,
                      Blocked, Connect, Busy, Unknown, Python, Timeout, External, Unrecognized }
impl ExcelError {
    pub const fn as_str(self) -> &'static str;   // "#NULL!" … "#EXTERNAL!"; Unrecognized answers "#UNRECOGNIZED" and is never written
    pub fn from_text(text: &str) -> Self;        // any other text → Unrecognized
}

// style.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StyleId(u16);                          // StyleId::DEFAULT == 0
impl StyleId { pub const DEFAULT: Self; pub const fn as_u16(self) -> u16; }
```

- `Cell::with_formula(self, Formula) -> Self` and `formula(&self) -> Option<&Formula>` replace the `SmolStr` versions.
- `Cell::with_error(self, ExcelError) -> Self` and `error(&self) -> Option<ExcelError>` replace the free strings.
- `Cell::error_text(&self) -> &str` is the known spelling, or the literal an `Unrecognized` carries in `value`.
- `Cell::with_style(self, StyleId) -> Self` and `style(&self) -> StyleId`.
- The `sheet.rs:1005` `unreachable!` becomes a typed refusal naming the cell.
- A file `s` above 65,535 is refused naming the cell. Excel caps `cellXfs` at 64,000.

```rust
// sheet.rs
pub struct Row { index: u32, cells: Vec<Cell> }      // sorted by column, binary-searched; capacity exact at parse
pub struct Sheet {
    name: SmolStr, state: SheetState, system: DateSystem,
    rows: BTreeMap<u32, Row>,
    extent: Extent,                                   // O(1) cell count, O(log rows) dimension
    layout: Layout,                                   // layout.rs: columns, row formats, merges, pane
    frame: Option<Box<WorksheetFrame>>,               // carried.rs; None for a sheet built in memory
    extras: BTreeMap<CellRef, CellExtra>,             // sparse; moves with its cell in every edit
    revision: u64,                                    // bumped by every &mut method that changes state
    changed: ChangeSet,                               // cells/ranges/formulas touched since the last recalculation
    origin: Option<u64>,                              // the id of the workbook whose styles the ids index
}
pub(crate) struct Extent { cells: usize, columns: Option<Box<[u32]>> /* MAX_COLUMNS counters, 64 KiB at the first cell */,
                           first_column: u32, last_column: u32 }
pub(crate) struct CellExtra {
    shared_string: Option<u32>,          // only when the item read is rich or a duplicate text
    inline_runs: Option<Arc<[u8]>>,      // raw `<is>` inner XML when rich
    cell_metadata: Option<u32>,          // cm
    value_metadata: Option<u32>,         // vm
    phonetic: bool,                      // ph
    formula: Option<Box<FormulaAttributes>>, // t=array|dataTable: ref, aca, ca, dt2D, dtr, del1, del2, r1, r2
}
```

- `Sheet::cell_count() -> usize` is new and O(1). `Sheet::len()` keeps its documented meaning: rows holding a cell.
- `dimension()` reads the first and last `rows` keys plus the cached column span. An emptied edge column rescans only up to the next non-zero counter.
- `extend_from_serie` becomes linear with no API change.
- Held state is stated in the rustdoc: ≤ 80 B per cell, ≤ 64 B per row plus `Vec` slack, one 64 KiB extent box per non-empty sheet, and the sparse extras map.
- A sheet inserted into a workbook whose id differs from `origin` keeps values, formulas and `NumberFormat`. Its `StyleId`s reset to `DEFAULT`, and temporal cells get the interned temporal xfs (§3.4).

### 2.3 Dirty tracking (replaces "parsed ⇒ rewritten", `workbook.rs:751-755`)

```rust
struct Slot { name: SmolStr, key: SheetKey, sheet_id: u32, kind: SheetKind, state: SheetState,
              part: Option<SmolStr>, parsed: OnceLock<Sheet>, saved: u64 }
```

- **Dirty rule.** `slot.part.is_none() || slot.parsed.get().is_some_and(|sheet| sheet.revision() != slot.saved)`. A parse under `&self` yields revision 0 with `saved == 0`, so an unparsed or merely read sheet is clean. No interior mutability is needed.
- `Workbook` also tracks:
  - `documents_dirty: bool`: sheet list, names, pivots, date system, part overrides
  - `styles_dirty: bool`
  - `pub fn is_dirty(&self) -> bool`
- `pub struct SheetKey(u32)` is stable for the workbook's life and never reused. It has `as_u32()`, `Workbook::sheet_key(&self, name) -> Option<SheetKey>` and `Workbook::sheet_by_key(&self, SheetKey) -> Option<&str>`.
- The writer rewrites exactly the dirty slots. Clean parsed slots are copied raw (§2.11). The sst is rewritten only when a dirty sheet appended a string; styles only when `styles_dirty`.

### 2.4 The worksheet part: regenerated, modelled, carried (`parser.rs`, `carried.rs`, `layout.rs`)

`SheetRows` keeps its one streamed pass. Outside `<sheetData>` it runs in capture mode: every top-level child and every inter-element text run is re-serialized from its raw event bytes (`<` + raw + `>`, `/>` for empty tags, raw text, CDATA, comments, PIs). That is byte-exact for everything Excel writes; only whitespace inside end tags is not kept. The XML declaration and the root `<worksheet …>` start tag are kept verbatim, so namespaces and `mc:Ignorable` survive, including Strict files.

```rust
pub(crate) struct WorksheetFrame { declaration: Option<Arc<[u8]>>, root: Arc<[u8]>, items: Vec<Carried> }
pub(crate) struct Carried { name: SmolStr /* local name; empty for a text run */, slot: u8 /* CT_Worksheet index */,
                            bytes: Arc<[u8]>, class: Class }
pub(crate) enum Class { Free, Shifted, Blocking }
```

On write, model elements and carried items are emitted in CT_Worksheet order:

`sheetPr, dimension, sheetViews, sheetFormatPr, cols, sheetData, sheetCalcPr, sheetProtection, protectedRanges, scenarios, autoFilter, sortState, dataConsolidate, customSheetViews, mergeCells, phoneticPr, conditionalFormatting*, dataValidations, hyperlinks, printOptions, pageMargins, pageSetup, headerFooter, rowBreaks, colBreaks, customProperties, cellWatches, ignoredErrors, smartTags, drawing, legacyDrawing, legacyDrawingHF, drawingHF, picture, oleObjects, controls, webPublishItems, tableParts, extLst`.

An unknown element keeps its position after the preceding known slot.

| Class | Elements | Write | Structural edit, rename or removal |
| --- | --- | --- | --- |
| Regenerated | `dimension`, `sheetData` | from the model | n/a |
| Modelled | `cols`, `mergeCells`, the first `sheetView`'s `pane` + `selection` (its other attributes and children carried) | from the model; `<sheetViews><sheetView workbookViewId="0">` created when absent | model edited |
| Carried, free | `sheetPr`, `sheetFormatPr`, `sheetCalcPr`, `sheetProtection`, `printOptions`, `pageMargins`, `pageSetup`, `headerFooter`, `legacyDrawingHF`, `drawingHF`, `picture`, `phoneticPr`, `customProperties`, `webPublishItems` | verbatim | untouched |
| Carried, shifted | `hyperlinks` (`@ref`, `@location`), `conditionalFormatting` (`@sqref`, `formula`), `dataValidations` (`@sqref`, `formula1`, `formula2`), `autoFilter` (`@ref`, `filterColumn@colId`), `sortState` (`@ref`, `sortCondition@ref`), `protectedRanges` (`@sqref`), `ignoredErrors` (`@sqref`), `rowBreaks`/`colBreaks` (`brk@id`), `cellWatches` (`@r`), `drawing`/`legacyDrawing`/`tableParts` (the element; the parts behind them are shifted, §4.4), `extLst` entries `{78C0D931-6437-407d-A8EE-F0AAD7539E65}` (x14 CF), `{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}` (x14 DV), `{05C60535-1F16-4fd2-B633-F4F36F0B64E0}` (sparklines) (`xm:sqref`, `xm:f`) | verbatim once rewritten | rewritten in memory at edit time by `package::rewrite` + the adjuster |
| Carried, blocking | `oleObjects`, `controls`, `scenarios`, `dataConsolidate`, `customSheetViews`, `smartTags`, any other `extLst` URI, any unknown top-level element | verbatim | structural edit **refused**: `Error::Unsupported { operation: "inserting rows through a sheet carrying oleObjects", filesystem: "xl/worksheets/sheet2.xml" }` |

Modelled facts (`layout.rs`):

```rust
pub(crate) struct Layout { columns: Columns, rows: BTreeMap<u32, RowFormat>, merges: Merges, pane: Option<Frozen>,
                           default_column_width: Option<f64>, default_row_height: Option<f64> }
pub(crate) struct RowFormat { height: Option<f64>, hidden: bool, style: Option<StyleId>, outline_level: u8,
                              collapsed: bool, thick_top: bool, thick_bottom: bool, phonetic: bool,
                              extra: Option<Arc<str>> /* other attributes, raw, e.g. x14ac:dyDescent */ }
pub(crate) struct ColumnFormat { width: Option<f64>, hidden: bool, style: Option<StyleId>, best_fit: bool,
                                 outline_level: u8, collapsed: bool }
pub(crate) struct Columns(Vec<(Range<u32>, ColumnFormat)>);   // sorted, non-overlapping spans
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub struct Frozen { pub rows: u32, pub columns: u32 }
```

- A row with a format and no cell now survives. Today it never enters the model (`sheet.rs:846`).
- `sheetFormatPr` is read for its defaults and carried verbatim.

### 2.5 Formulas on the wire

- `<f>` attributes are read: `t` (`normal|shared|array|dataTable`), `ref`, `si`, `ca`, `aca`, and the data-table ones.
- **Shared formulas.** The master's text is lexed once into a shape relative to the master host (§5.2). Each dependent `<f t="shared" si="n"/>` takes a clone of the same `Arc`. Dependents seen before their master resolve at the end of the part. A dependent with no master is refused, naming the cell and `si`.
- **Normal formulas** are interned per sheet by shape while the part is parsed, so `=A2*B2` and `=A3*B3` share one `Arc`. The intern map is dropped after the parse.
- **Array, data-table and dynamic-array anchors** keep their attributes in `CellExtra::formula`. They are shifted like any formula (§4.4) and never evaluated (§5.7). `ca="1"` is written when the formula is volatile per the registry or was read with `ca`.
- `_xlfn.`/`_xlws.` are the file spelling. They are stripped in entry text and added on entry (§5.2).
- On write, a dirty sheet writes one plain `<f>` per formula cell. Clean sheets are copied raw and keep their groups.

### 2.6 Shared strings (`shared_strings.rs`)

- `SharedStrings` keeps the text per item plus a bitset `plain` (the item is one `<t>` run).
- The write is `package::rewrite` over the **original part bytes**. Existing items are copied as they were, new plain items go in with `before_end(sst)`, `uniqueCount` is set and `count` removed through the `attributes` hook.
- A dirty sheet's string cell writes `CellExtra::shared_string` when the table's text at that index still equals the cell's. Otherwise it interns by text against *plain* items only; if none matches, it appends. Rich runs and duplicate items therefore survive, and typing a rich item's text never inherits its runs.
- A package with no sst part gets a fresh `<sst>`.

### 2.7 Workbook documents (`workbook.rs`, new `names.rs`)

`read_workbook` (`workbook.rs:1149`) reads, beyond `<sheet>` and `workbookPr`:

| Fact | Model | Written |
| --- | --- | --- |
| `sheet@sheetId` | `Slot::sheet_id` | preserved; an added sheet takes `max + 1` (today renumbered `index + 1`, `workbook.rs:999-1001`) |
| `definedNames` | `names.rs` `DefinedName { name: SmolStr, scope: Option<SheetKey>, formula: Formula /* host A1 */, hidden: bool, comment: Option<SmolStr>, raw: Arc<[u8]> }` | carried until a rename, removal, move or structural edit touches one; then regenerated at its CT_Workbook slot |
| `workbookView@activeTab`, `@firstSheet` | `Workbook::active_tab` | remapped on remove and move to the nearest visible sheet |
| `calcPr` | carried | `fullCalcOnLoad="1"` set, or `<calcPr fullCalcOnLoad="1"/>` inserted before the first present of `oleSize customWorkbookViews pivotCaches smartTagPr smartTagTypes webPublishing fileRecoveryPr webPublishObjects extLst`, on **every save that rewrote a worksheet or changed the sheet list** |
| `pivotCaches` | cache ids read | ours appended (§6.4) |
| `xl/calcChain.xml` | none | removed with its workbook relationship and its `Override` on the same saves; this fixes the dangling case at `workbook.rs:883-893` |

- **Orphans.** A removed sheet's parts are dropped when no remaining `.rels` in the package targets them: drawing → chart → chart style/colours, table, pivotTable, comments, vmlDrawing, threadedComments, printerSettings. This is a reference count over all relationships; each dropped part's `Override` goes with it. A workbook-level pivot cache whose last table went is dropped with its `<pivotCache>` entry and relationship.
- `docProps/app.xml` is carried. `TitlesOfParts` may go stale; Excel ignores it and rewrites it.

### 2.8 `package::rewrite` (`package.rs`)

```rust
pub(crate) struct Rewrite<'a> {
    pub(crate) skip: &'a dyn Fn(&BytesStart<'_>) -> bool,
    pub(crate) patch_count: &'a dyn Fn(&[u8]) -> Option<u32>,
    pub(crate) before_end: Fragment<'a>,
    pub(crate) after_start: Fragment<'a>,
    pub(crate) before_start: Fragment<'a>,
    /// Replaces `set_attribute`: the attributes to set (`Some`) or remove (`None`) on the element named.
    pub(crate) attributes: &'a dyn Fn(&[u8]) -> Vec<(&'static str, Option<String>)>,
    /// Replaces an element's character data; the argument is its local name.
    pub(crate) text: &'a dyn Fn(&[u8], &str) -> Option<String>,
    /// Fragments inserted once, before the first present sibling of a schema list.
    pub(crate) insert: &'a [Insertion<'a>],
}
pub(crate) struct Insertion<'a> { pub(crate) fragment: String, pub(crate) parent: &'a [u8],
                                  pub(crate) before_first_of: &'a [&'a [u8]] }
```

- `set_attribute` is deleted. Its four callers migrate: `workbook.rs:984`, `1025`, `1121`, `styles.rs:384`.
- `RelationshipKind` gains `PivotTable`, `PivotCacheDefinition`, `PivotCacheRecords`, `Theme`, `Table`, `Drawing`, `Chart`, `Comments`, `VmlDrawing`, `CalcChain`, `QueryTable`.
- `relationships_part_of`, `folder_of`, `relative_to` (generalized to `..` targets) and `escape_attribute` move from `workbook.rs:1248-1276` to `package.rs` as `pub(crate)`.

### 2.9 One writer, and the empty template

`write_fresh` (`workbook.rs:699`), `write_workbook` (`:1209`), `package::write_content_types` (`package.rs:303`) and `package::write_workbook_relationships` (`:275`) are **deleted**. A `Workbook` built with `new()` holds a `Source::Template`, which answers four documents from `package.rs` constants and lists no other member:

- `[Content_Types].xml`
- `_rels/.rels`
- `xl/workbook.xml`: `<workbookPr/>`, `<sheets/>`, `<calcPr/>`; `date1904` is set for 1904
- `xl/_rels/workbook.xml.rels`

Styles and sst are created by the writer when absent.

```rust
enum Source { Archive(Arc<ZipArchive>), Template }
```

`write_preserving` is the one writer over either source. `Workbook::new()` stays infallible and still holds **no sheet**; the service adds `Sheet1`. The record path's `Replaced` stream now receives `&TemporalStyles` (§3.4) instead of the `u32` offset (`workbook.rs:1133`).

### 2.10 Save, save-as, rebase

```rust
pub struct Package { bytes: Vec<u8>, snapshot: Snapshot }        // Snapshot: workbook id, per-slot revision, parts written
impl Package { pub fn as_bytes(&self) -> &[u8]; pub fn into_bytes(self) -> Vec<u8>; }
impl Workbook {
    pub fn into_package(&self) -> Result<Package>;                // &self: runs under a read lock
    pub fn rebase(&mut self, package: Package) -> Result<()>;     // adopt + mark saved; another workbook's package → Error::Conflict
    pub fn write_into(&mut self, target: &mut (impl IOBase + ?Sized)) -> Result<()>;
    pub fn into_bytes(&self) -> Result<Vec<u8>>;                  // = into_package()?.into_bytes(); state unchanged
    pub fn parse_all(&self) -> Result<()>;
}
```

- **Adopt** (private) mounts the package bytes as `Source::Archive` over `Holder::buffer` (zero handle calls). It gives every added slot the part it was written under and resets the sst and styles readers. Style ids are unchanged, because the table is append-only. Adopt changes no `saved` marker.
- **`write_into`** runs `into_package`, then adopt, which drops any mapping of a lazily opened local source, then `target.write_all_bytes`, then marks each slot's `saved` with the snapshot revision. A failed write leaves the workbook dirty and equivalent.
- **`rebase`** is adopt plus mark, for a caller that wrote the bytes itself (the service, §8.1). Edits made after the snapshot stay dirty.
- This fixes the verified "`writeInto(samePath)` then corrupt deflate stream" hazard. The Python and Node `write_into` become redirects to this.

### 2.11 Raw member copy (`rust/src/zip/archive.rs`)

```rust
impl ZipArchive {
    /// Copy `path` from `source` as stored: compressed bytes, method, CRC-32, sizes and the 0x5967
    /// restart map; nothing inflated or deflated.
    pub fn copy_member_from(&self, source: &Arc<ZipArchive>, path: &str) -> Result<ZipEntry>;
}
```

The carry loop (`workbook.rs:884-896`) and clean sheets use it. It is pinned in `rust/tests/zip/archive.rs` (entry equal on CRC and compressed size) and in `iobase_calls` (no decode reads).

### 2.12 Deleted in the same commit

- `Styles`, `Styles::spliced`, `Styles::format`, `NumberFormat::style_index`, `NumberFormat::ALL` as a cellXfs order, `NumberFormat::from_code` (`styles.rs:119`, moved to `FormatCode::kind`), and `NumberFormat::builtin` (to `FormatCode::builtin`)
- `Replaced`'s `u32` offset
- `Sheet::insert_rows` and `Sheet::remove_rows` (to `Workbook`, §4.1) and their tests (`rust/tests/excel/sheet.rs:527-552`, rewritten against `Workbook`)
- `write_fresh`, `write_workbook`, `write_content_types`, `write_workbook_relationships`, `Rewrite::set_attribute`
- the empty-`<f>` writing (`sheet.rs:1011-1018`)
- `.api-inventory.txt:2979` `Excel1900/Excel1904`, which becomes `Year1900/Year1904`
- `.api-bindings.txt:582` "the sheet a new workbook opens with", which becomes "the sheet a record write creates when the options name none"
- `docs/media/index.md` Excel prose that untouched means unread

---

## 3. Style model, display, typed entry (P1 table, P3 semantics)

### 3.1 Files

| File | Holds |
| --- | --- |
| `excel/styles.rs` | `StyleSheet`: the part as a model; its append-only splice writer; `TemporalStyles` |
| `excel/style.rs` | `StyleId`, `CellStyle`, `Font`, `Fill`, `Border`, `Edge`, `Alignment`, `Protection`, `Color`, `Underline`, `VerticalRun`, `FontScheme`, `Horizontal`, `Vertical`, `BorderStyle`, `PatternType`, `StylePatch`, `Borders`, `BorderPreset` |
| `excel/theme.rs` | `Theme`: `a:clrScheme` of the workbook's theme relationship; Office 2013+ defaults when absent |
| `excel/format.rs` | `FormatCode` (parse, classify, render), `Rendered` |
| `excel/entry.rs` | `Entry`: user text to a cell |

### 3.2 Parsed facts

- **Read:** `numFmts`; `fonts` (name, sz, b, i, u, strike, color, vertAlign, family, scheme, charset, outline, shadow, condense, extend); `fills` (patternFill type, fg, bg; `gradientFill` kept raw); `borders` (left, right, top, bottom, diagonal, vertical, horizontal: style + colour, diagonalUp/Down, outline); `cellXfs` (numFmtId, fontId, fillId, borderId, xfId, every `alignment` attribute, `protection`, quotePrefix, pivotButton, apply*); `colors/indexedColors`.
- **Carried, never reordered:** `cellStyleXfs`, `cellStyles`, `dxfs`, `tableStyles`, `colors/mruColors`, `extLst`. Every carried `dxfId`, `xfId` and `style` therefore stays valid.

### 3.3 Public values

```rust
pub struct CellStyle { pub number_format: SmolStr, pub font: Font, pub fill: Fill, pub border: Border,
                       pub alignment: Alignment, pub protection: Protection, pub quote_prefix: bool, pub parent: u32 }
pub struct Font { pub name: SmolStr, pub size: f64, pub bold: bool, pub italic: bool, pub underline: Underline,
                  pub strike: bool, pub color: Option<Color>, pub vertical: VerticalRun, pub family: Option<u8>,
                  pub scheme: Option<FontScheme>, pub charset: Option<u8>, pub outline: bool, pub shadow: bool,
                  pub condense: bool, pub extend: bool }
pub enum Color { Rgb(u32 /* ARGB */), Theme { index: u8, tint: f64 }, Indexed { index: u8, tint: f64 }, Auto }
pub enum Fill { None, Pattern { pattern: PatternType, foreground: Option<Color>, background: Option<Color> }, Gradient(Arc<[u8]>) }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StylePatch {
    pub font_name: Option<SmolStr>, pub font_size: Option<f64>, pub bold: Option<bool>, pub italic: Option<bool>,
    pub underline: Option<Underline>, pub strike: Option<bool>,
    pub font_color: Option<Option<Color>>,   // Some(None): automatic
    pub fill: Option<Option<Color>>,         // Some(Some): solid; Some(None): no fill
    pub borders: Option<Borders>,            // Borders { preset: BorderPreset, style: BorderStyle, color: Option<Color> }
    pub horizontal: Option<Horizontal>, pub vertical: Option<Vertical>, pub wrap: Option<bool>, pub indent: Option<u8>,
    pub number_format: Option<SmolStr>,      // a code; "General" clears
}
pub enum BorderPreset { Bottom, Top, Left, Right, All, Outside, ThickOutside, None }
impl StyleSheet {
    pub fn style(&self, id: StyleId) -> Option<&CellStyle>;
    pub fn len(&self) -> usize;
    pub fn resolve(&self, color: &Color, theme: &Theme) -> Option<u32 /* RGB */>;
    pub(crate) fn intern(&mut self, style: &CellStyle) -> Result<StyleId>;
}
impl Workbook {
    pub fn style_sheet(&self) -> Result<&StyleSheet>;
    pub fn theme(&self) -> Result<&Theme>;
    pub fn cell_style(&self, sheet: &str, at: CellRef) -> Result<CellStyle>;
    pub fn set_style(&mut self, sheet: &str, ranges: &[CellRange], patch: &StylePatch) -> Result<()>;
}
```

- **Colour resolution.** Theme index 0↔1 and 2↔3 are swapped as Excel maps them (lt1/dk1). Indexed colours use the file's palette, else the default. Tint follows ECMA-376 §18.8.19 on HLS.
- **Interning.** Each distinct source `StyleId` in a patch is derived once (`HashMap<StyleId, StyleId>`), so a range patch costs O(cells) plus O(distinct styles).
  - Whole rows or columns set `RowFormat::style`/`ColumnFormat::style` and patch present cells only; empty cells are never materialized.
  - A new xf appends at most one `numFmt` (id `max(163, declared) + 1`, the rule of `styles.rs:307`), font, fill, border and xf.
  - Border presets are range-aware: `Outside` sets only outer edges.
  - Past 64,000 xfs → typed refusal.
- Changing a cell's number format re-derives its value meaning through `DateSystem`: `45292` under `m/d/yyyy` becomes `date32`, and the inverse turns a date back into its serial `Float64`.

### 3.4 Number formats (`format.rs`)

```rust
pub struct FormatCode { code: SmolStr, sections: Arc<[Section]> }
impl FormatCode {
    pub fn from_code(code: &str) -> Result<Self>;             // Error::Parse at the byte of the fault
    pub fn builtin(id: u32) -> Option<Self>;                   // ECMA-376 §18.8.30 ids 0–22, 37–40, 45–49; 14 renders en-US m/d/yyyy
    pub fn kind(&self) -> NumberFormat;                        // value meaning (was NumberFormat::from_code)
    pub fn render(&self, value: &Scalar, system: DateSystem) -> Rendered;
    pub fn with_decimals(self, delta: i8) -> Self;             // ribbon ±decimals
    pub fn code(&self) -> &str;
}
pub struct Rendered { pub text: SmolStr, pub color: Option<u32>, pub fill: Option<char>, pub shorter: Vec<SmolStr> }
```

- **General:** Excel's 11-character rule after rounding to 15 significant digits. `shorter` holds the General spellings of decreasing width that the client picks from when a column is narrow.
- **Sections:** up to four sections, conditions `[>=100]`, the eight named colours and `[ColorN]`.
- **Numbers:** `0 # ?`, the decimal point, thousands `,`, trailing-comma scaling, `%`, `E+ E- e+ e-`, fractions `# ?/?`, `# ??/??` and `?/8`.
- **Literals:** `"…"`, `\x`, `_x` (a space), `*x` (returned as `fill`), `@`.
- **Tags:** `[$€-407]` (symbol kept, locale ignored), and `[$-F800]`/`[$-F400]` as en-US long date and time.
- **Dates and times:** `y yy yyyy`, `m mm mmm mmmm mmmmm`, `d dd ddd dddd`, `h hh`, minutes by position, `s ss`, `.0–.000`, `AM/PM`, `A/P`, elapsed `[h] [m] [s]`.
- Rounding is half away from zero on the 15-digit decimal form.
- `[DBNum…]` and other locale digit systems render as General and are refused by `set_style`.
- Ids 41–44 read from a file without a declaration display with their en-US codes and are carried untouched. The writer never references an undeclared id outside the ECMA table.
- `TemporalStyles`: the crate's temporal defaults are interned on demand: `yyyy-mm-dd`, `yyyy-mm-dd hh:mm:ss`, `yyyy-mm-dd hh:mm:ss.000`, `h:mm:ss`, `[h]:mm:ss`. These are the codes the openpyxl exchange asserts today. A cell whose `format` is temporal but whose xf does not classify as that format is written under the interned xf. The record writer receives the same table.

### 3.5 Typed entry (`entry.rs`)

```rust
pub enum Entry { Blank, Value { value: Scalar, format: Option<SmolStr> }, Text(Str), Formula(Formula) }
impl Entry { pub fn from_text(text: &str, host: CellRef, system: DateSystem) -> Result<Self>; }
impl Workbook {
    pub fn set_entry(&mut self, sheet: &str, at: CellRef, text: &str) -> Result<Option<Cell>>;
    pub fn entry_text(&self, sheet: &str, at: CellRef) -> Result<Option<String>>;
    pub fn display_text(&self, sheet: &str, at: CellRef) -> Result<Option<Rendered>>;
}
```

en-US rules:

| Input | Result |
| --- | --- |
| `=…` | formula; a syntax error is `Error::Parse` at the position |
| `+` or `-` followed by a non-number | formula `=+…`, `=-…` |
| `-5`, `+5` | number |
| leading `'` | text, with `quotePrefix` |
| `TRUE`/`FALSE`, any case | boolean |
| the 18 error spellings | error |
| `1,234.5` | number |
| `(12)` | −12 |
| `12%` | number with `0%` (`0.00%` if decimals are typed) |
| `$1,234.50` | number with `"$"#,##0.00` |
| `1e3` | number with `0.00E+00` |
| `1 1/2` | number with `# ?/?` |
| `1/2/2024`, `2024-01-02` | date, builtin 14 |
| `10:30` | time, builtin 20 |
| `10:30 PM` | time, builtin 18 |
| `1/2/2024 10:30` | datetime, builtin 22 |
| anything else | text |

- A suggested format applies only when the cell's current format is General.
- `entry_text` is the inverse: `=` plus the entry spelling of the formula; `m/d/yyyy`; `h:mm:ss AM/PM`; `12%`; up to 15 significant digits; `'text`.
- A property test pins `from_text(entry_text(c)) == c`.
- `VALUE`, `DATEVALUE`, `TIMEVALUE` and text-to-number coercion reuse `Entry`'s number and date rules.

---

## 4. Grid operations, shift, edits (P4)

### 4.1 Workbook-level structure. This is the single door, because formulas span sheets.

```rust
impl Workbook {
    pub fn insert_rows(&mut self, sheet: &str, at: u32, count: u32) -> Result<()>;
    pub fn remove_rows(&mut self, sheet: &str, rows: Range<u32>) -> Result<()>;
    pub fn insert_columns(&mut self, sheet: &str, at: u32, count: u32) -> Result<()>;
    pub fn remove_columns(&mut self, sheet: &str, columns: Range<u32>) -> Result<()>;
    pub fn move_sheet(&mut self, name: &str, to: usize) -> Result<()>;
    pub fn rename_sheet(&mut self, name: &str, new_name: impl Into<SmolStr>) -> Result<()>;  // now rewrites references
    pub fn remove_sheet(&mut self, name: &str) -> Result<Option<Sheet>>;                     // references become #REF!
}
```

Each call is atomic.

1. **Validate first:**
   - grid bounds
   - blocking elements (§2.4) and blocking parts (a `queryTable` relationship; for removal, a slicer or timeline cache naming a pivot or table the sheet hosts)
   - tables: a column band crossing a table's columns is refused; so is removal of its header row or every data row
   - an array, data-table or dynamic-array range, or a foreign pivot location, that the edit would split
   - one of our pivots it would cut
2. **Then mutate:**
   - cells, extras, row formats, column spans
   - merges: an insertion inside a merge expands it; a removal shrinks it, or drops it when removed whole
   - every formula in every sheet, every name, and every shifted carried element in every sheet
   - part overrides (§4.4)
   - pivot sources and locations, then the sheet's `ChangeSet` marked structural and the dependency graph dropped

### 4.2 Sheet-level layout verbs

```rust
impl Sheet {
    pub fn column_width(&self, column: u32) -> f64;                                        // character units
    pub fn set_column_width(&mut self, columns: Range<u32>, width: Option<f64>) -> Result<()>;
    pub fn row_height(&self, row: u32) -> f64;                                             // points
    pub fn set_row_height(&mut self, rows: Range<u32>, height: Option<f64>) -> Result<()>;
    pub fn set_columns_hidden(&mut self, columns: Range<u32>, hidden: bool) -> Result<()>;
    pub fn set_rows_hidden(&mut self, rows: Range<u32>, hidden: bool) -> Result<()>;
    pub fn is_row_hidden(&self, row: u32) -> bool;  pub fn is_column_hidden(&self, column: u32) -> bool;
    pub fn merges(&self) -> impl Iterator<Item = CellRange> + '_;
    pub fn merge(&mut self, range: CellRange) -> Result<Vec<Cell>>;   // refuses an overlap naming both; answers the cleared cells
    pub fn unmerge(&mut self, range: CellRange) -> bool;
    pub fn frozen(&self) -> Option<Frozen>;
    pub fn set_frozen(&mut self, frozen: Option<Frozen>) -> Result<()>;
    pub fn edge(&self, from: CellRef, direction: Direction) -> CellRef;       // Ctrl+arrow
    pub fn current_region(&self, at: CellRef) -> CellRange;                   // Ctrl+A
}
pub enum Direction { Up, Down, Left, Right }
```

- **Widths.** Widths are ECMA character units (§18.3.1.13). The layout JSON states `maxDigitWidth: 7` (Calibri 11). The client converts with `px = trunc(((256·w + trunc(128/7))/256)·7)`.
- **Frozen pane.** `set_frozen` writes the first sheetView's `<pane xSplit ySplit topLeftCell activePane state="frozen"/>`, omitting a zero split; `activePane` is `bottomRight`, `bottomLeft` or `topRight`. One `<selection pane=…>` goes with it. Split (non-frozen) panes are carried and shown unfrozen.

### 4.3 Range operations (`workbook.rs`, `fill.rs`, `find.rs`)

```rust
impl Workbook {
    pub fn clear(&mut self, sheet: &str, ranges: &[CellRange], what: Clear) -> Result<()>;   // Clear: All | Contents | Formats
    pub fn paste(&mut self, from: (&str, CellRange), to: (&str, CellRef), what: Paste, cut: bool) -> Result<CellRange>;
    pub fn paste_text(&mut self, sheet: &str, anchor: CellRef, tsv: &str) -> Result<CellRange>;  // Excel's TSV quoting; one Entry per cell
    pub fn fill(&mut self, sheet: &str, source: CellRange, target: CellRange, mode: FillMode) -> Result<()>;
    pub fn sort(&mut self, sheet: &str, range: CellRange, keys: &[SortKey], header: bool) -> Result<()>;
    pub fn find(&self, options: &FindOptions, after: Option<(&str, CellRef)>) -> Result<Option<(SmolStr, CellRef)>>;
    pub fn replace(&mut self, options: &FindOptions, replacement: &str) -> Result<u64>;
}
pub enum Paste { All, Values, Formulas, Formats }
pub enum FillMode { Series, Copy }
pub struct SortKey { pub column: u32, pub descending: bool }
pub struct FindOptions { pub text: SmolStr, pub sheet: Option<SmolStr> /* None: workbook */, pub within: Within /* Values | Formulas */,
                         pub match_case: bool, pub entire_cell: bool }
```

- **Paste.**
  - A formula keeps its shape `Arc`, so it renders translated at its new host.
  - A reference that falls off the grid at the new host is materialized as `#REF!` in a derived shape.
  - `cut` has Excel's move semantics: references anywhere into the moved block are retargeted, found through the dependency index plus names, carried formulas and charts.
  - A multi-range source is refused ("this action won't work on multiple selections").
  - Merges inside the target are refused.
- **Fill.** The target must extend the source in exactly one direction (down, right, up or left). With `Copy`, cells are copied and formulas translated. With `Series`, the rule is chosen per line along the fill axis, when every source cell of the line is of one class:
  - numbers: one value copies (Excel's rule); two or more follow a linear step when arithmetic within 15 digits, else a least-squares linear trend
  - dates: one value steps +1 day; two values step by day, or by month when the day of month is equal, or by year when month and day are equal
  - times: by the detected step
  - text with a trailing integer (`Item 7`) increments by the detected step
  - built-in lists `Mon…Sun`, `Monday…`, `Jan…Dec`, `January…`, spelled with the source's case pattern
  - quarters `Q1`, `Qtr1`, `Quarter 1` cycle 1–4
  - any other mix: the pattern is copied
- **Sort.** Stable; numbers < text (case-insensitive ordinal after simple case folding, a documented divergence from Windows word sort) < FALSE < TRUE < errors, blanks last in both directions. Formula shapes move with their row. References from outside the range are not updated (Excel's rule). Merges inside the range and array ranges are refused.
- **Find/Replace.** Wildcards `* ? ~` use the matcher `formula/criteria.rs` owns. `Within::Formulas` matches and replaces entry text, then re-enters it through `Entry::from_text`. A replacement that would produce an invalid formula is refused, naming the cell, before any cell changes.

### 4.4 The reference adjuster (`excel/shift.rs`)

```rust
pub(crate) enum Shift<'a> {
    InsertRows { sheet: &'a str, at: u32, count: u32 }, RemoveRows { sheet: &'a str, rows: Range<u32> },
    InsertColumns { sheet: &'a str, at: u32, count: u32 }, RemoveColumns { sheet: &'a str, columns: Range<u32> },
    Move { from: (&'a str, CellRange), to: (&'a str, CellRef) },
    RenameSheet { from: &'a str, to: &'a str }, RemoveSheet { name: &'a str },
}
pub(crate) fn adjust(reference: &Reference, host: (&str, CellRef), shift: &Shift<'_>) -> Adjusted;  // Same | Moved(Reference) | Invalid
pub(crate) fn adjust_range(range: CellRange, sheet: &str, shift: &Shift<'_>) -> Option<CellRange>;  // for plain ref attributes
```

Rules:

- A reference, relative or absolute, keeps pointing at the same cells.
- The formula's host moves too, and relative tokens re-render against the new host.
- An insertion inside a range expands it; an insertion at a range's first row shifts it whole.
- A partial removal shrinks a range; a whole removal is `#REF!`.
- Whole-row and whole-column references follow on one axis.
- A 3D span keeps its endpoint names. A removed endpoint moves inward, as Excel does.

Rewrites are memoized per `(Arc<Shape> pointer, host class)`. A class is the interval of hosts the shift treats alike, so a 1M-cell shared formula costs O(cells) pointer swaps plus O(distinct shapes) allocations.

**Everything the adjuster rewrites:**

| Where | What |
| --- | --- |
| cell formulas, every sheet | shapes, including array `ref` and data-table `r1`/`r2`, `ref` in `CellExtra::formula` |
| defined names | `DefinedName::formula` |
| carried, shifted worksheet elements | attribute ranges through `adjust_range`; `formula`/`formula1`/`formula2`/`xm:f` text parsed as a shape at the `sqref`'s top-left host, adjusted, rendered |
| `hyperlink@location` | shape at host A1 |
| tables `xl/tables/*.xml` | `table@ref`, `autoFilter@ref`, `sortState@ref`, `sortCondition@ref` |
| drawings `xl/drawings/drawing*.xml` | `xdr:from`/`xdr:to` `xdr:row`/`xdr:col` text. An anchor in a removed band moves to the band's start with offset 0. `absoluteAnchor` is untouched |
| comments, threaded comments | `comment@ref`, `threadedComment@ref` |
| VML `xl/drawings/vmlDrawing*.vml` | `x:Row`, `x:Column`, `x:Anchor` text. A VML part that is not well-formed XML refuses the edit, naming the part |
| charts `xl/charts/chart*.xml`, every chart in the package | `c:f` text as shapes at host A1, for structural edits and for rename/removal |
| foreign pivot caches | `worksheetSource@ref`, `@sheet` |
| foreign pivot tables | `location@ref` |
| our pivots | `PivotSpec::source`, `PivotTable::location` |

Rewritten non-worksheet parts are held as `Workbook::overrides: BTreeMap<SmolStr, Arc<[u8]>>`, written at save, and restored by undo snapshots. Every override is computed during validation and committed only if all of them succeed.

A rename or removal also refuses, naming the part, when a slicer or timeline cache names a pivot or table the removed sheet hosts.

### 4.5 Undo and redo (`excel/edit.rs`, `excel/journal.rs`)

```rust
pub enum Edit {
    SetEntries { sheet: SmolStr, entries: Vec<(CellRef, SmolStr)> },
    FillEntry { sheet: SmolStr, ranges: Vec<CellRange>, text: SmolStr },
    Clear { sheet: SmolStr, ranges: Vec<CellRange>, what: Clear },
    SetStyle { sheet: SmolStr, ranges: Vec<CellRange>, patch: StylePatch },
    InsertRows { sheet: SmolStr, at: u32, count: u32 }, RemoveRows { sheet: SmolStr, start: u32, count: u32 },
    InsertColumns { sheet: SmolStr, at: u32, count: u32 }, RemoveColumns { sheet: SmolStr, start: u32, count: u32 },
    RowHeight { sheet: SmolStr, start: u32, count: u32, height: Option<f64> },
    ColumnWidth { sheet: SmolStr, start: u32, count: u32, width: Option<f64> },
    HideRows { sheet: SmolStr, start: u32, count: u32, hidden: bool },
    HideColumns { sheet: SmolStr, start: u32, count: u32, hidden: bool },
    Merge { sheet: SmolStr, range: CellRange, center: bool, across: bool }, Unmerge { sheet: SmolStr, range: CellRange },
    Freeze { sheet: SmolStr, frozen: Option<Frozen> },
    Fill { sheet: SmolStr, source: CellRange, target: CellRange, mode: FillMode },
    Sort { sheet: SmolStr, range: CellRange, keys: Vec<SortKey>, header: bool },
    Paste { from: (SmolStr, CellRange), to: (SmolStr, CellRef), what: Paste, cut: bool },
    PasteText { sheet: SmolStr, anchor: CellRef, text: SmolStr },
    Replace { options: FindOptions, replacement: SmolStr },
    AddSheet { name: Option<SmolStr>, at: Option<usize> }, RenameSheet { name: SmolStr, to: SmolStr },
    RemoveSheet { name: SmolStr }, MoveSheet { name: SmolStr, to: usize }, SheetState { name: SmolStr, state: SheetState },
    Land { destination: Landing, cells: Box<Sheet> },                   // Get Data, already read (§8.7)
    PivotCreate { spec: PivotSpec, destination: Landing }, PivotUpdate { sheet: SmolStr, name: SmolStr, spec: PivotSpec },
    PivotRefresh { sheet: SmolStr, name: SmolStr }, PivotRefreshAll, PivotRemove { sheet: SmolStr, name: SmolStr },
    Batch(Vec<Edit>),
    // inverse-only, refused on the wire:
    SetCells { sheet: SmolStr, cells: Vec<(CellRef, Option<Cell>)> }, Restore(Box<Restore>), RestoreSheet(Box<RestoreSheet>),
}
pub enum Landing { NewSheet(SmolStr), At { sheet: SmolStr, anchor: CellRef } }
pub struct Applied { pub inverse: Option<Edit>, pub touched: Vec<(SmolStr, CellRange)>, pub structural: bool,
                     pub sheets: bool, pub styles: bool, pub calc: Recalculation, pub bytes: usize, pub result: Scalar }
impl Edit { pub fn from_scalar(value: &Scalar, workbook: &Workbook) -> Result<Self>; pub fn label(&self) -> SmolStr; }
impl Workbook { pub fn apply(&mut self, edit: Edit) -> Result<Applied>; }         // validates, mutates, recalculates; atomic
pub struct Journal { /* undo VecDeque, redo Vec */ }
impl Journal {
    pub fn new(entries: usize, bytes: usize) -> Self;                       // service default 100 entries, 64 MiB
    pub fn record(&mut self, forward: Edit, applied: &Applied);
    pub fn undo(&mut self, workbook: &mut Workbook) -> Result<Option<Applied>>;
    pub fn redo(&mut self, workbook: &mut Workbook) -> Result<Option<Applied>>;
    pub fn labels(&self) -> (Option<&str>, Option<&str>);
}
```

- An `Edit` does no I/O.
- Structural inverses are the opposite shift plus a `Restore` of every rewrite the opposite shift would not reproduce. The adjuster checks `adjust(adjust(x, s), s⁻¹) == x` and snapshots `x` otherwise: `#REF!` results, shrunk ranges, overridden parts. Removed rows' cells, extras and formats are snapshotted with `Sheet::slice`.
- An entry over the byte cap is applied non-undoable (`Applied::inverse == None`) and the response says so.
- `apply` recalculates. Direct methods (`insert_rows`, `set_entry` …) do not; the caller calls `recalculate`.

### 4.6 `Edit::from_scalar` wire shape

- Sheets are named by key (a number). `workbook` resolves keys to names at intake.
- Ranges are A1 text, parsed once by `CellRange::from_str`.
- Errors are `Error::InvalidRecord { path: "$.edit.entries[3].ref", … }`.
- The ops are in §8.4.

---

## 5. Formula engine (P2 shapes, P5 evaluation)

### 5.1 Why this is not a second expression engine (goes into the docs verbatim)

> A cell's `<f>` is the workbook medium's own language, the way a FIX message is the FIX codec's. Its semantics are Excel's: text-to-number coercion, error values, serial dates, reference algebra, implicit intersection, 15-digit comparison. They differ from the DuckDB-shaped `expression/` layer, which refuses aggregates (`expression/parser.rs:37`) and reads Arrow columns rather than cells. The engine in `excel/formula/` evaluates exactly what an `.xlsx` states and nothing else. It is reached only through `Workbook`, and it is not a query language over records.

### 5.2 Layout and shapes (`rust/src/excel/formula.rs` + `formula/`)

| File | Owns |
| --- | --- |
| `formula.rs` | `Formula`, `Clock`, `Recalculation`, entry and file spelling |
| `formula/lexer.rs` | total tokenizer: any unrecognized run is an opaque verbatim token, so lexing never fails |
| `formula/reference.rs` | `Coord { Relative(i32), Absolute(u32) }`, `Target` (cell, area, rows, columns, `#REF!`), `SheetSpec` (own, named, 3D span, external opaque), rendering with Excel's quoting (`'Q1 ''24'!A1`) |
| `formula/shape.rs` | `Shape`, per-sheet interning, shape rewrite for shifts |
| `formula/parser.rs` | precedence climbing into the crate-private `Expr` AST |
| `formula/value.rs` | crate-private `Operand { Blank, Number(f64), Text(Str), Boolean(bool), Error(ExcelError), Array(Arc<Matrix>), Reference(Areas) }`, coercions |
| `formula/number.rs` | 15-digit rules, approximate equality and subtraction |
| `formula/eval.rs` | `Evaluator` over `&Workbook`, explicit stack |
| `formula/graph.rs` | dependency index, dirty propagation, ordering, cycles |
| `formula/aggregate.rs` | `Accumulator` (§5.4) |
| `formula/criteria.rs` | `*IF`/`*IFS` criteria and the wildcard matcher |
| `formula/functions.rs` + `formula/functions/{math,statistical,logical,lookup,text,date,information,financial}.rs` | registry (name, category, arity, volatility, `_xlfn` prefix, signature text) and implementations |

```rust
#[derive(Clone)] pub struct Formula(Arc<Shape>);                // Clone is one Arc increment; Eq/Hash by shape
pub(crate) struct Shape {
    tokens: Box<[Token]>,                  // verbatim runs and reference tokens, in order
    sheets: SmallVec<[SmolStr; 1]>,        // sheet names its references name (the rename filter)
    volatile: bool,
    held: Option<Held>,                    // why it is never evaluated: structured/external ref, unknown function, array syntax…
    expr: OnceLock<std::result::Result<Expr, Held>>,   // parsed on first evaluation
}
pub(crate) enum Token { Text(SmolStr), Reference(Reference), Function { prefix: Prefix, name: SmolStr }, Single /* _xlfn.SINGLE( */ }
impl Formula {
    pub fn from_file(text: &str, host: CellRef) -> Self;          // the <f> spelling; total
    pub fn from_entry(text: &str, host: CellRef) -> Result<Self>; // what follows '='; refuses a syntax error at its position
    pub fn at(&self, host: CellRef) -> impl Display + '_;         // file spelling at host, allocation-free
    pub fn entry_at(&self, host: CellRef) -> impl Display + '_;   // entry spelling: no _xlfn./_xlws., @ for SINGLE
    pub fn is_computed(&self) -> bool;
}
```

- References are stored relative to the host (`Relative(target − host)` unless `$`), so rendering at any host yields the translated text. Copy, fill and sort need no translation step.
- Every character that is not a reference token or a function name's prefix is a verbatim run, so the user's spacing and literal spelling survive every edit. Reference tokens render canonically (upper-case, minimal quoting), as Excel itself normalizes them on entry.
- A rename rewrites only shapes whose `sheets` list names the old name, once per distinct shape (§0 #9).
- `_xlfn.SINGLE(x)` is `@x` in entry spelling, and `@x` on entry becomes `_xlfn.SINGLE(x)`.

### 5.3 Grammar

- **Operands:** numbers; `"…"` with `""`; `TRUE`/`FALSE`; the error literals; array constants `{1,2;3,4}`; function calls with dotted names and `_xlfn.`/`_xlws.`; defined names.
- **References:** `A1 $A$1 A$1 A1:B2 A:C $A:$C 1:3 Sheet!A1 'Q1 ''24'!A1:B2 Jan:Mar!B2`.
- **Held tokens** (lexed, rendered, never evaluated): `[1]Sheet!A1`, structured `T[…]`, `_xlfn.ANCHORARRAY`, and the spill reference `A1#`.
- **Precedence**, high to low:
  1. `:`
  2. space (intersection)
  3. `,` (union, inside parentheses)
  4. unary `-`/`+` (`=-2^2` is 4)
  5. postfix `%`
  6. `^`, left-associative (`=2^3^2` is 64)
  7. `* /`
  8. `+ -`
  9. `&`
  10. `= <> < > <= >=`
- Nesting over 64 levels or text over 8,192 characters is refused at the position.

### 5.4 Values, evaluation and aggregation

**Reading cells.**

| Cell | Operand |
| --- | --- |
| number, decimal | `as_f64` |
| temporal | its serial (`DateSystem::serial_of`) |
| text, boolean, error | the same kind |
| absent or null | `Blank` |
| `ExcelError::Unrecognized` | makes the reader uncomputed |

**Coercion.**

- Blank is `0`, `""` or `FALSE` by context.
- Text in arithmetic goes through `Entry`'s number and date rules; `"abc"+1` is `#VALUE!` and `TRUE+1` is 2.
- `&` renders numbers as General at 15 digits.
- Across types, numbers < text < booleans; text compares case-insensitively.
- `=` compares after rounding both sides to 15 significant digits.
- A top-level `+`/`-` whose result cancels below 2⁻⁴⁸ of the larger operand is 0 (LibreOffice's `approxSub`, which reproduces Excel).
- Overflow is `#NUM!`, `0^0` is `#NUM!`, subnormals flush to 0.
- Text functions count UTF-16 code units, and text is capped at 32,767.
- In aggregates, text and booleans inside references are ignored, while direct arguments are coerced: `SUM(TRUE,"3")` is 4.

**Legacy semantics.** A formula without `cm` is legacy. A range where a scalar is expected takes implicit intersection with the host's row or column. A legacy formula's top-level array result takes its top-left value. This is exactly how Excel 365 reads what we write (it shows `=@A1:A3`).

```rust
pub(crate) struct Accumulator { count: u64, counta: u64, sum: f64, sumsq: f64, min: f64, max: f64, product: f64,
                                values: Option<Vec<f64>> /* only for MEDIAN/PERCENTILE/QUARTILE/MODE/LARGE/SMALL/RANK */ }
impl Accumulator { pub(crate) fn push(&mut self, value: &Operand); pub(crate) fn merge(&mut self, other: &Self);
                   pub(crate) fn finish(&self, aggregate: Aggregate) -> Operand; }
```

The accumulator's one owner serves SUM/COUNT/AVERAGE/MIN/MAX/PRODUCT/STDEV*/VAR*, `SUBTOTAL` (1–11 include hidden rows; 101–111 skip rows hidden by `RowFormat::hidden`; both skip nested SUBTOTAL results), pivot compute (§6.2) and `/summary`.

### 5.5 Dependencies, recalculation, clock (`graph.rs`, `formula.rs`)

- **Nodes and index.** One node per formula cell `(SheetKey, CellRef)`. Single-cell precedents go in `HashMap<(SheetKey, CellRef), SmallVec<[NodeId; 2]>>`. Range precedents go in a per-sheet, per-column interval list (start-sorted, max-end augmented). Whole rows and columns go in their own lists; names and 3D spans are expanded at build.
- **Volatile nodes** are `NOW TODAY RAND RANDBETWEEN OFFSET INDIRECT`. They are re-evaluated every pass, and `OFFSET`/`INDIRECT` precedents are unknown by construction.
- **Build.** Lazy, O(formulas), at the first recalculation; it runs `parse_all`. A formula cell added, changed or removed through any `Sheet` mutator is recorded in `ChangeSet::formulas` and patched into the graph in O(precedents). Structural edits, renames and removals drop the graph.
- **Recalculation.**
  1. Collect every sheet's `ChangeSet`.
  2. Take the transitive dependents plus volatiles.
  3. Order them topologically (Kahn).
  4. Evaluate with an explicit stack.
  5. Write back **only values that differ** (Scalar equality). Reopening our own `fullCalcOnLoad` file therefore stays clean.
- A result becomes a cell value through the same serial rule as the file. Its kind becomes `n|str|b|e`.
- **Cycles.** Nodes left after ordering keep their prior value and are reported (≤ 256 listed plus a total). `calcPr@iterate` is honoured as off, and the status says so.
- **Full recalculation** (`calculate_all`) runs on open when the file states `fullCalcOnLoad="1"` or a formula has no cached `<v>` (as openpyxl writes).

```rust
pub struct Recalculation { pub evaluated: u64, pub uncomputed: u64, pub circular: Vec<(SmolStr, CellRef)>, pub circular_count: u64 }
pub struct Clock { /* System | Fixed(i64 unix nanos), timezone: Timezone, seed: u64 */ }
impl Clock { pub fn system(timezone: Timezone) -> Self; pub fn fixed(unix_nanos: i64, timezone: Timezone, seed: u64) -> Self; }
impl Workbook {
    pub fn recalculate(&mut self) -> Result<Recalculation>;
    pub fn calculate_all(&mut self) -> Result<Recalculation>;
    pub fn with_clock(self, clock: Clock) -> Self;  pub fn set_clock(&mut self, clock: Clock);
}
```

`RAND` draws from xorshift, seeded per pass with `xxh3(seed, pass, sheet, cell)`, so it is deterministic under `Clock::fixed`.

### 5.6 Functions (159)

`*` marks a function stored `_xlfn.`-prefixed (20 of them).

| Category (count) | Functions |
| --- | --- |
| Math and trigonometry (43) | ABS ACOS ASIN ATAN ATAN2 CEILING CEILING.MATH* COS DEGREES EVEN EXP FACT FLOOR FLOOR.MATH* GCD INT LCM LN LOG LOG10 MOD MROUND ODD PI POWER PRODUCT QUOTIENT RADIANS RAND RANDBETWEEN ROUND ROUNDDOWN ROUNDUP SIGN SIN SQRT SUBTOTAL SUM SUMIF SUMIFS SUMPRODUCT TAN TRUNC |
| Statistical (34) | AVERAGE AVERAGEA AVERAGEIF AVERAGEIFS COUNT COUNTA COUNTBLANK COUNTIF COUNTIFS LARGE MAX MAXA MAXIFS* MEDIAN MIN MINA MINIFS* MODE MODE.SNGL* PERCENTILE PERCENTILE.INC* QUARTILE QUARTILE.INC* RANK RANK.EQ* SMALL STDEV STDEV.P* STDEV.S* STDEVP VAR VAR.P* VAR.S* VARP |
| Logical (11) | AND FALSE IF IFERROR IFNA* IFS* NOT OR SWITCH* TRUE XOR* |
| Lookup and reference (14) | ADDRESS CHOOSE COLUMN COLUMNS HLOOKUP INDEX INDIRECT (A1 form) LOOKUP (vector form) MATCH OFFSET ROW ROWS VLOOKUP XLOOKUP* (scalar result) |
| Text (23) | CHAR CLEAN CODE CONCAT* CONCATENATE EXACT FIND LEFT LEN LOWER MID PROPER REPLACE REPT RIGHT SEARCH SUBSTITUTE T TEXT TEXTJOIN* TRIM UPPER VALUE |
| Date and time (16) | DATE DATEVALUE DAY DAYS* EDATE EOMONTH HOUR MINUTE MONTH NOW SECOND TIME TIMEVALUE TODAY WEEKDAY YEAR |
| Information (14) | ERROR.TYPE ISBLANK ISERR ISERROR ISEVEN ISLOGICAL ISNA ISNONTEXT ISNUMBER ISODD ISREF ISTEXT N NA |
| Financial (4) | FV NPV PMT PV |

- `TEXT` renders through `FormatCode::render`.
- `ERROR.TYPE` answers 1–8 for `#NULL! #DIV/0! #VALUE! #REF! #NAME? #NUM! #N/A #GETTING_DATA`; any other error makes it uncomputed.
- `ROUND(2.675,2)` is 2.68 (15-digit decimal first). `MOD` takes the divisor's sign. Date functions reproduce the 1900 phantom day (`WEEKDAY(61)=5`).
- Approximate `MATCH`/`VLOOKUP`/`LOOKUP` are exact on sorted data, which is Excel's contract; unsorted results are a documented divergence.
- A variant this engine does not implement (`INDIRECT(x, FALSE)`, an array-returning `XLOOKUP`) is uncomputed.

### 5.7 Carried rather than computed

A formula is uncomputed, and so is every dependent, when:

- it is an array (CSE), dynamic-array (`cm`), data-table or external formula
- it uses a held token or an unknown function
- a precedent is uncomputed or `Unrecognized`

An uncomputed formula keeps its cached value and is flagged `stale` in the API. Every save that rewrote a worksheet sets `fullCalcOnLoad="1"`, so Excel always recalculates what this engine left.

---

## 6. Pivot tables (P6, `rust/src/excel/pivot.rs` + `pivot/{compute,layout,part}.rs`)

### 6.1 Model

```rust
pub struct PivotSpec { pub name: SmolStr, pub source: PivotSource, pub rows: Vec<AxisField>, pub columns: Vec<AxisField>,
                       pub values: Vec<ValueField>, pub subtotals: bool, pub row_grand_totals: bool, pub column_grand_totals: bool }
pub struct PivotSource { pub sheet: SmolStr, pub range: CellRange }        // first row: unique, non-empty headers
pub struct AxisField { pub field: SmolStr, pub order: ItemOrder }           // ItemOrder: Ascending | Descending
pub struct ValueField { pub field: SmolStr, pub aggregate: Aggregate, pub caption: Option<SmolStr>, pub number_format: Option<SmolStr> }
pub enum Aggregate { Sum, Count, Average, Max, Min, Product, CountNumbers, StdDev, StdDevP, Var, VarP }
pub struct PivotTable { spec: PivotSpec, sheet: SmolStr, location: CellRange, cache_id: u32, origin: Origin /* Created | Read { editable: bool, reason: Option<SmolStr> } */ }
pub struct PivotFieldInfo { pub name: SmolStr, pub numeric: bool, pub aggregate: Aggregate, pub items: u64 }
impl PivotSpec { pub fn from_scalar(value: &Scalar) -> Result<Self>; pub fn into_scalar(&self) -> Scalar; }
impl Workbook {
    pub fn pivots(&self) -> impl Iterator<Item = &PivotTable>;
    pub fn pivot_fields(&self, source: &PivotSource) -> Result<Vec<PivotFieldInfo>>;
    pub fn add_pivot(&mut self, spec: PivotSpec, sheet: &str, anchor: CellRef) -> Result<CellRange>;
    pub fn update_pivot(&mut self, sheet: &str, name: &str, spec: PivotSpec) -> Result<CellRange>;
    pub fn refresh_pivot(&mut self, sheet: &str, name: &str) -> Result<CellRange>;
    pub fn remove_pivot(&mut self, sheet: &str, name: &str) -> Result<()>;
}
```

- **Semantics of the totals.** `row_grand_totals` is XML `rowGrandTotals`: the right-hand "Grand Total" column, the total of each row. `column_grand_totals` is `colGrandTotals`: the bottom "Grand Total" row.
- **Document shape** (`from_scalar`/`into_scalar`, the one spelling the API and both bindings use):

  ```json
  {"name":"PivotTable1","source":{"sheet":"Data","range":"A1:D501"},"rows":[{"field":"Region","order":"ascending"}],
   "columns":[{"field":"Year","order":"ascending"}],"values":[{"field":"Sales","aggregate":"sum","caption":null,"numberFormat":"#,##0.00"}],
   "subtotals":true,"rowGrandTotals":true,"columnGrandTotals":true}
  ```

- **Refusals**, typed and named:
  - a duplicate or empty header
  - a source or field that does not exist
  - no row field, or no value field (non-goal, §1)
  - an output overlapping non-empty cells outside the pivot's previous location
  - a name taken on the host sheet

### 6.2 Compute (`pivot/compute.rs`)

1. One `cells_in` pass over the source.
2. Each axis field's values are interned into per-field dictionaries; these become the cache's `sharedItems`.
3. Groups go in a dense `(row_group, col_group)` grid of `Accumulator`s when the product is ≤ 16M, else a `HashMap`.

- **Item order:** numbers ascending, then text (case-insensitive ordinal), then FALSE, TRUE, errors, and `(blank)` last. Descending reverses all but `(blank)`.
- **Default aggregate** in `pivot_fields`: Sum when every non-blank value is a number, else Count.
- **Aggregation rules.** Sum, Average, Max, Min, Product, StdDev(P) and Var(P) use numbers. Count counts non-blank values of any type. CountNumbers counts numbers. An error propagates to every aggregate it reaches. StdDev or Var of fewer than two values is `#DIV/0!`.
- **Totals.** Subtotals and grand totals *merge accumulators* of their children, so Average and StdDev are exact and nothing is re-scanned.
- **Cost:** O(records × (axis + value fields)). The allocation count is independent of the record count (pinned).

### 6.3 Layout: tabular form (`pivot/layout.rs`)

Let `r ≥ 1` row fields, `c ≥ 0` column fields and `v ≥ 1` value fields, anchored at `(R, C)`. Define **`c' = c + (v > 1 ? 1 : 0)`**: the Σ Values pseudo-field is the last column field when `v > 1`.

- **Header rows** are `1 + c'`.
  - Row `R` holds the value field's caption at `(R, C)` when `v = 1` (blank otherwise), and from column `C + r` the names of the `c'` column fields. Σ Values is named by `dataCaption`, "Values".
  - Rows `R+1 … R+c'−1` hold the items of the outer column fields.
  - Row `R+c'` holds the row-field names in `C … C+r−1` and the innermost column items (or the value captions when Σ Values is innermost).
  - With `c' = 0`, the single header row holds the row-field names, then the value captions.
- **Data** follows in item order. With `subtotals`, a `"<item> Total"` row follows each group of row levels `1 … r−1`. A `Grand Total` row closes the table when `column_grand_totals`. Grand-total columns are labelled `Grand Total` (`v = 1`) or `Total <caption>` (`v > 1`) and appear when `row_grand_totals` and `c ≥ 1`.
- **Location:**
  - rows = `1 + c' + rowItems.count`
  - columns = `r + colItems.count`
  - `firstHeaderRow = 1`, `firstDataRow = 1 + c'`, `firstDataCol = r`
- **Worked example:** `r = 1` (Region: East, North, West), `c = 1` (Year: 2023, 2024), `v = 1`, anchor A3. Header rows 2, rowItems 4 (3 items + grand), colItems 3 (2 items + grand), giving **`A3:D8`**, `firstHeaderRow="1" firstDataRow="2" firstDataCol="1"`. With `c = 0, v = 2`: `c' = 1`, location `A3:C8`, `colFields` = `<field x="-2"/>`.
- **Formatting.** Value cells take the value field's `number_format` (interned) or General. Header and total cells take the default xf. Rendered cells are ordinary cells, and the previous location is cleared before a re-render.

### 6.4 Parts written (`pivot/part.rs`)

These shapes were verified with openpyxl 3.1.5 (`python/.venv`): `load_workbook` reads the location, fields, items, cache source, `refreshOnLoad` and the empty `RecordList`. `save` and reload keep them.

```xml
<!-- xl/pivotCache/pivotCacheDefinitionN.xml -->
<pivotCacheDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"
  r:id="rId1" saveData="0" refreshOnLoad="1" createdVersion="6" refreshedVersion="6" minRefreshableVersion="3" recordCount="0">
  <cacheSource type="worksheet"><worksheetSource ref="A1:D6" sheet="Sales"/></cacheSource>
  <cacheFields count="4">
    <cacheField name="Region" numFmtId="0"><sharedItems count="3"><s v="East"/><s v="North"/><s v="West"/></sharedItems></cacheField>
    <cacheField name="Year" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1"
      containsInteger="1" minValue="2023" maxValue="2024" count="2"><n v="2023"/><n v="2024"/></sharedItems></cacheField>
    <cacheField name="Product" numFmtId="0"><sharedItems/></cacheField>
    <cacheField name="Sales" numFmtId="0"><sharedItems containsSemiMixedTypes="0" containsString="0" containsNumber="1"
      minValue="1" maxValue="20"/></cacheField>
  </cacheFields>
</pivotCacheDefinition>
<!-- xl/pivotCache/pivotCacheRecordsN.xml -->
<pivotCacheRecords xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
  xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" count="0"/>
<!-- xl/pivotTables/pivotTableN.xml -->
<pivotTableDefinition xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" name="PivotTable1" cacheId="1"
  applyNumberFormats="0" applyBorderFormats="0" applyFontFormats="0" applyPatternFormats="0" applyAlignmentFormats="0"
  applyWidthHeightFormats="1" dataCaption="Values" updatedVersion="6" minRefreshableVersion="3" createdVersion="6"
  useAutoFormatting="1" itemPrintTitles="1" indent="0" compact="0" compactData="0" outline="0" outlineData="0"
  gridDropZones="1" multipleFieldFilters="0" rowGrandTotals="1" colGrandTotals="1">
  <location ref="A3:D8" firstHeaderRow="1" firstDataRow="2" firstDataCol="1"/>
  <pivotFields count="4">
    <pivotField axis="axisRow" compact="0" outline="0" showAll="0" sortType="ascending">
      <items count="4"><item x="0"/><item x="1"/><item x="2"/><item t="default"/></items></pivotField>
    <pivotField axis="axisCol" compact="0" outline="0" showAll="0" sortType="ascending">
      <items count="3"><item x="0"/><item x="1"/><item t="default"/></items></pivotField>
    <pivotField compact="0" outline="0" showAll="0"/>
    <pivotField dataField="1" compact="0" outline="0" showAll="0"/>
  </pivotFields>
  <rowFields count="1"><field x="0"/></rowFields>
  <rowItems count="4"><i><x/></i><i><x v="1"/></i><i><x v="2"/></i><i t="grand"><x/></i></rowItems>
  <colFields count="1"><field x="1"/></colFields>
  <colItems count="3"><i><x/></i><i><x v="1"/></i><i t="grand"><x/></i></colItems>
  <dataFields count="1"><dataField name="Sum of Sales" fld="3" subtotal="sum" baseField="0" baseItem="0"/></dataFields>
  <pivotTableStyleInfo name="PivotStyleLight16" showRowHeaders="1" showColHeaders="1" showRowStripes="0"
    showColStripes="0" showLastColumn="1"/>
</pivotTableDefinition>
```

Rules:

- **sharedItems** are written in the field's *display order*, so `item x`, `rowItems x v` and `sharedItems` indexes coincide. Element and attribute rules:

  | Field holds | Written |
  | --- | --- |
  | strings only | count only |
  | numbers only | `containsSemiMixedTypes="0" containsString="0" containsNumber="1"`, `containsInteger="1"` if all integral, `minValue`/`maxValue` |
  | strings and numbers | `containsMixedTypes="1" containsNumber="1"` + min/max |
  | a blank | `containsBlank="1"` + `<m/>` last |
  | booleans | `<b v="1"/>` |
  | errors | `<e v="#N/A"/>` |
  | dates | `containsSemiMixedTypes="0" containsNonDate="0" containsDate="1" containsString="0" minDate maxDate`, `<d v="2024-01-02T00:00:00"/>`, `numFmtId="14"` |

- A non-axis text field gets `<sharedItems/>`. A value field gets the numeric attributes without items.
- **rowItems** (and colItems, the same way): each leaf tuple is `<i r="k">` (`r` omitted when 0; `k` = the leading levels shared with the previous tuple), then `<x v="p"/>` per remaining level (`v` omitted when 0). A subtotal is `<i t="default" r="L"><x v="p"/></i>` after its group's last child. The grand total is `<i t="grand"><x/></i>`. With Σ Values on columns, data-field tuples carry `i="n"`: `<i><x/></i><i i="1"><x v="1"/></i>`, and a grand per value field is `<i t="grand" i="n"><x/></i>`. With `c' = 0`: `<colItems count="1"><i/></colItems>`.
- **dataField**:
  - `subtotal` maps sum, count, average, max, min, product, `countNums`, `stdDev`, `stdDevp`, var, varp (ST_DataConsolidateFunction).
  - `name` is the caption or "`<Aggregate> of <field>`": Sum, Count, Average, Max, Min, Product, Count, StdDev, StdDevp, Var, Varp.
  - `numFmtId` is the interned id when a number format is set.
- **Plumbing.**
  - The table's rels point to the cache definition (`../pivotCache/pivotCacheDefinitionN.xml`).
  - The cache's rels (`xl/pivotCache/_rels/pivotCacheDefinitionN.xml.rels`) point to its records as `rId1`.
  - The host sheet's rels gain a `pivotTable` relationship; they are created or rewritten through `Relationships::from_xml` + `rewrite`.
  - The workbook gains a `pivotCacheDefinition` relationship and `<pivotCaches><pivotCache cacheId r:id/></pivotCaches>`: appended inside an existing `pivotCaches`, else inserted before the first present of `smartTagPr smartTagTypes webPublishing fileRecoveryPr webPublishObjects extLst` (CT_Workbook order, checked against openpyxl's `WorkbookPackage.__elements__`).
  - `[Content_Types]` gains three `Override`s: `…spreadsheetml.pivotTable+xml`, `…pivotCacheDefinition+xml`, `…pivotCacheRecords+xml`.
  - `cacheId` = `max(existing) + 1`.
  - Removing one of our pivots drops its parts, rels, overrides and `pivotCache` entry.

### 6.5 Refresh and existing pivots

- Our pivots refresh on demand (Refresh, Refresh All). They do not follow source edits.
- Pivots read from a file are parsed into `PivotTable { origin: Read { editable, reason } }`. `editable` means worksheet source, tabular layout, no page fields, filters, `fieldGroup`, calculated fields or items, or OLAP. An editable one is rewritten on change.
- The rest are carried byte for byte, except `location@ref` and `worksheetSource@ref`/`@sheet`, which the adjuster rewrites. Their location refuses cell and structural edits, naming the pivot.

### 6.6 openpyxl checks, both directions (`scripts/check_excel_interop.py`, `rust/tests/interop/excel.rs`)

1. Rust writes `pivot.xlsx`: one row field, one column field, two value fields. openpyxl asserts:
   - `ws._pivots[0]`: `.name`, `.location.ref`, `rowFields`/`colFields` (incl. `-2`), `dataFields[i].fld`/`.subtotal`, `.cache.cacheSource.worksheetSource.ref`/`.sheet`, `.cache.refreshOnLoad`
   - every rendered cell, with `data_only=True`
2. openpyxl loads and saves `pivot.xlsx`. Rust reopens it and asserts the pivot is `editable`, the spec is equal and the cells are unchanged.
3. openpyxl writes a workbook, Rust adds a pivot and saves, and openpyxl reads both.

No half may print `SKIPPED`.

---

## 7. Generic HTTP (`rust/src/http/`, P7)

### 7.1 Additions

| Addition | Signature (file) | Replaces, and callers migrated |
| --- | --- | --- |
| Pattern router | Crate-private `server/router.rs`: a segment trie. `{name}` (`[A-Za-z_][A-Za-z0-9_]*`) matches one decoded segment; `{*rest}` matches the remainder and only last. Lookup borrows the decoded path, params are ranges into it (`SmallVec<[_; 4]>`), no allocation. Precedence at each segment is literal > param > catch-all, with backtracking: the most specific pattern answering the method wins. `405 Allow` is the union of methods over every matching pattern, `HEAD` beside `GET`. HEAD falls to GET; any-method routes (`None`) are kept; the trailing-slash 308 is kept. Two patterns differing only in a param name at one position are refused as ambiguous | `State.routes` BTreeMap and `State::route/allowed/slashed` (`server.rs:719-805`) |
| Registration returns `Result` | `Server::route(&self, method: Option<Method>, pattern: &str, handler: impl Fn(&Context<'_>) -> Result<Response> + Send + Sync + 'static) -> Result<()>`; `respond(&self, Option<Method>, &str, Response) -> Result<()>`; `inject(&self, &str, Fault, u32) -> Result<()>` (inject stays exact-path). A bad pattern or path is `Error::Parse`, never silently ignored (today ignored at `server.rs:1550`, `1570`) | 53 `.route(`, 18 `.respond(`, 16 `.inject(` Rust sites (`xmla/server.rs:77,85`, `python/src/http.rs:2434,2457,2472`, `node/src/http.rs:1677,1699`, `rust/tests/http/**`, `rust/tests/support/http_server.rs`, `rust/tests/xmla/**`, `rust/benchmarks/holder/http.rs`) plus 35 docs blocks. `#[must_use]` on `Result` makes the compiler list every site |
| Handler context | `server/context.rs`, `pub struct Context<'a>` with `impl Deref for Context<'_> { type Target = Request; }`: `request(&self) -> &'a Request`, `param(&self, &str) -> Option<&'a str>`, `params(&self) -> impl Iterator<Item = (&'a str, &'a str)>`, `path(&self) -> &'a str` (routed, decoded, below `path_prefix`), `query(&self) -> Result<Parameters<'a>>` (decoded on demand), `client(&self) -> Option<IpAddr>`, `is_secure(&self) -> bool`. `pub type Handler = dyn Fn(&Context<'_>) -> Result<Response> + Send + Sync + 'static`. **`Context` does not implement `Clone`**: `request.clone()` on a `&Context` resolves to `<&Context as Clone>::clone`, the Deref trap. The sweep script flags every `.clone()` on a handler argument, and each becomes `context.request().clone()` (`python/src/http.rs:2434`) | `server.rs:186` |
| Body built once, query on demand | `answer_routed` moves the connection's `Vec` into the body `Arc` (one copy instead of two, `server.rs:1024-1027`). `route_of` builds the query `Vec` only when recording | — |
| Typed body | `Request::scalar(&self) -> Result<Scalar>`, `Request::scalar_with_field(&self, &Field) -> Result<Scalar>`; shares `Response`'s private decoded-buffer reader (`response.rs:730`) under `json::Limits` | — |
| Error to status | `Status::from_error(&Error) -> Status`: `Parse InvalidRecord InvalidDataType UnknownDataType InvalidMetadataValue EmptyMetadataKey DuplicateMetadataKey Json Codec InvalidArithmetic ArithmeticOverflow DivisionByZero InexactArithmetic InvalidSecret` → 400; `Absent` → 404; `Conflict NotAtomic` → 409; `Unsupported` → **422**; `Remote` → 502; `Io` NotFound → 404, PermissionDenied → 403, other → 500; `Arrow`, `Iceberg`, anything else → 500. `Response::from_error(&Error) -> Response`: RFC 9457 `application/problem+json` `{"type":"about:blank","title","status","detail","kind","location"?}`, where `kind` is the variant in snake case and `location` the error's path, position or URL. New constants `CONTENT_TOO_LARGE` 413, `UNSUPPORTED_MEDIA_TYPE` 415, `MISDIRECTED_REQUEST` 421, `UNPROCESSABLE_CONTENT` 422, `PRECONDITION_REQUIRED` 428 | handler `Err` → 500 text (`server.rs:1030`); `Answer::from_error` (`server.rs:119`). Server-generated 400/404/405 stay text |
| Panic containment | `catch_unwind(AssertUnwindSafe(…))` around the handler, `mount::serve` and the `with_writer` body writer (h1 `connection.rs`, h2/h3 framed). A panic before the head is a 500 problem with `kind:"panic"`; after the head it severs the transfer as a writer `Err` does. Precedent: `parallel.rs:177` | — |
| Conditional requests | `headers/conditional.rs`: `pub struct Conditions`, `Conditions::from_headers(&Headers) -> Result<Self>`, `evaluate(&self, method: Method, etag: Option<&ETag>, last_modified: Option<i64>) -> Precondition` (`Proceed \| NotModified \| Failed`) in RFC 9110 §13.2.2 order, `if_range_holds(&self, …) -> bool`. `answer_routed` applies it to any 200 GET/HEAD route answer (handler or fixed) that carries `ETag`/`Last-Modified` and answers 304. `ByteRange` moves to `headers/range.rs` | private `not_modified`, `weak_eq`, `if_range_holds`, `ByteRange` (`mount.rs:296-383`) deleted |
| Content coding | `Headers::accepted_codings(&self) -> Result<AcceptedCodings>` (q-values, `identity`, `*`); `AcceptedCodings::best(&self, offered: &[Codec]) -> Option<Codec>`; `Codec::content_coding(self) -> Option<&'static str>` (`gzip`, `deflate` = `Zlib`, `zstd`) | private `content_coding` (`mount.rs:277`) deleted |
| Embedded assets | `http/assets.rs`: `pub struct Assets`; `Assets::new()`; `with_file(self, path: &str, body: impl Into<Arc<[u8]>>) -> Result<Self>` (media type from the suffix through the MIME registry, an unknown suffix refused; `charset=utf-8` on text types; strong ETag = quoted xxh3-64 hex, the mount's spelling; a gzip variant with ETag `"<hex>-gz"` precomputed when ≥ 1 KiB and smaller); `with_index(self, path) -> Result<Self>`; `with_header(self, name, value) -> Result<Self>`; `route(self: Arc<Self>, server: &Server, prefix: &str) -> Result<Url>`, which registers **ordinary GET routes** `{prefix}` and `{prefix}/{*path}` (no second dispatcher). 304 comes from the generic `Conditions` layer; `Vary: Accept-Encoding`; bodies are shared `Arc`s | — |
| Graceful shutdown | `ServerOptions::with_shutdown_grace(Duration)` (default 5 s). `Server::shutdown(self) -> Result<()>`: stop accepting; mark `stopping`, so each answer carries `Connection: close`; `shutdown(Read)` idle keep-alive sockets, found in a registry of `try_clone`d streams with an idle flag; h2 `graceful_shutdown` (GOAWAY); wait for `live == 0` up to the grace; then `shutdown(Both)` what remains. `Drop` does the same with zero grace | `shutdown` (`server.rs:1647`) keeps its spelling |
| Host allow-list | `ServerOptions::with_allowed_hosts<I: IntoIterator<Item = S>, S: AsRef<str>>(self, hosts: I) -> Result<Self>`. The `Host` (h1) or `:authority` (h2/h3) host part is compared case-insensitively, port ignored; anything else gets 421 before routing. Empty = any (unchanged default) | — |
| Default headers | `ServerOptions::with_default_headers(self, Headers) -> Self`, stamped in `write_answer` and the framed `prepared()`; a handler's own header wins | — |

**Out of scope** (stated in docs): mount read-only options, streaming request bodies, SSE, on-the-fly compression, server-side cookies and auth, and JSON bodies for server-generated 404/405.

**Bindings:**

- Python `Server.route(path, handler, method=None)` accepts patterns and calls `handler(request, **params)`, so exact routes are unchanged. `respond`/`inject` raise `ValueError` on a bad pattern. `bind(..., allowed_hosts=..., default_headers=..., shutdown_grace=...)`.
- Node: `respond`/`inject` throw; `Server.bind(address, { allowedHosts, defaultHeaders, shutdownGrace })`. Node has no route, by design.
- `Context`, `Assets`, `Conditions` are Rust-only.

### 7.2 Pins

- `allocations`: `http_route_lookup_allocates_nothing` (internals: literal and param lookup), `http_asset_answers_share_the_body` (a 304 allocates no body, a 200 allocates less than the body).
- `iobase_calls`: `http_assets` makes 0 handle calls per request; the mount's conditional behaviour is unchanged.
- Tests:
  - `rust/tests/http/server/router.rs` (internals), `context.rs`, `rust/tests/http/assets.rs`, `rust/tests/http/headers/conditional.rs`
  - shutdown: a keep-alive connection is closed after `shutdown`; today it is still answered, probed
- Bench: `http_route_param`, `http_asset_304` in `rust/benchmarks/holder/http.rs`.

---

## 8. The excel service (`rust/src/excel/server.rs` + `server/`, `#[cfg(feature = "http")]`, P8)

This section is the **frozen contract** (Phase 0). The UI reads nothing else, and `rust/tests/excel/server*.rs` pins every shape.

### 8.1 Types and locking

```rust
pub struct ServiceOptions { /* private */ }
impl ServiceOptions {
    pub fn new() -> Self;                                            // defaults below
    pub fn with_read_only(self, read_only: bool) -> Self;
    pub fn with_token(self, token: Option<SmolStr>) -> Self;
    pub fn with_locations(self, policy: LocationPolicy) -> Self;     // default Any
    pub fn with_max_cells(self, cells: u64) -> Self;                 // 20_000_000
    pub fn with_max_import_rows(self, rows: u32) -> Self;            // 1_048_575
    pub fn with_clock(self, clock: Clock) -> Self;                   // Clock::system(UTC)
    pub fn with_journal(self, entries: usize, bytes: usize) -> Self; // 100, 64 MiB
}
pub enum LocationPolicy { Any, Prefixes(Vec<Url>) }
impl LocationPolicy { pub fn allows(&self, location: &Url) -> Result<()>; }    // canonical-form prefix match
pub struct Service { /* session: RwLock<Session>, changes: Mutex<Ring> + Condvar, saving: Mutex<()>, options, closed: AtomicBool */ }
impl Service {
    pub fn new(options: ServiceOptions) -> Result<Self>;                                    // "Book1" with "Sheet1", no location
    pub fn open<K: AsRef<str>, V: AsRef<str>>(location: &Url, properties: impl IntoIterator<Item = (K, V)>,
                                              options: ServiceOptions) -> Result<Self>;
    pub fn route(self: Arc<Self>, server: &Server, path: &str) -> Result<Url>;             // API at {path}api/
    pub fn is_dirty(&self) -> bool;
    pub fn describe(&self) -> (Option<Url>, usize /* sheets */, u64 /* cells */);
    pub fn close(&self);                                                                    // wakes long polls; later calls 503
}
```

**Open.**

1. `Holder::from_url(&url, properties)`, then `open()` (1 HEAD over HTTP), `read_all_bytes()` (1 GET), `size()` and `mtime()` (0 while open), `close()`.
2. `Workbook::from_bytes`, so the source is a snapshot, never mapped.
3. `parse_all`, then the `max_cells` check (refused as 413 at runtime; before bind in the CLI).
4. `calculate_all` when needed (§5.5).

An absent location, or size 0, gives a new workbook bound there. A declared media type that is known and not `.xlsx` is refused by name.

**Locking.**

- Reads (tiles, layout, cells, styles, summary, edge, export, find) take `read`.
- An edit takes `write`, applies one `Edit`, bumps `revision`, appends to the change ring and notifies.
- Open, import and upload read outside every lock. Swapping in the new session, or `Land`, takes a short `write`.
- **Save:** holding `saving`, `into_package` runs under `read`; the conflict check (1 HEAD) and `write_all_bytes` (1 PUT) run with no lock; `rebase` runs under a short `write`.
- A panic during an edit poisons the session (`PoisonError::into_inner` for reads). Later edits answer 409 `kind:"poisoned"`; read and download stay available.

**Session state:** workbook, journal, `generation` (bumped by open/new/upload), `revision`, `saved`, location `{ url, properties, size, mtime }`, and tile revisions. Tile revisions are `base: HashMap<SheetKey, u64>` plus `tiles: HashMap<(SheetKey, u32, u32), u64>`. They are bumped from `Applied::touched`; a structural edit, or a range spanning over 4,096 tiles, bumps the sheet's base instead.

### 8.2 Security posture

- **Bind.** Default `127.0.0.1:8080`.
- **Token.**
  - A 128-bit random token, drawn from `getrandom` 0.3 in the CLI, is printed in the URL **fragment**: `http://127.0.0.1:8080/#token=…`. It never reaches a log, `--trace` or `Referer`.
  - The UI moves it to `sessionStorage` and strips the address bar with `history.replaceState`.
  - Every `/api` request must carry `X-Yggdryl-Token`, checked when a token is configured and required to be present even with `--no-token`. A custom header forces a CORS preflight, which is never answered, so cross-origin reads and writes are impossible.
  - Downloads are fetched as blobs with the header.
- **Requests.**
  - Unsafe methods need `Content-Type: application/json`; upload needs the xlsx media type. Otherwise 415.
  - An `Origin` that is present and not the served origin gets 403 `kind:"origin"`.
- **Hosts.** Allowed hosts are `localhost`, `127.0.0.1`, `[::1]`, the bound host, the `--public-url` host and each `--allow-host`; anything else gets 421 (DNS rebinding).
- **Default headers** from the CLI:
  - `X-Content-Type-Options: nosniff`
  - `Referrer-Policy: no-referrer`
  - `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'`
  - The UI therefore sets styles only through the CSSOM, never through `style="…"` markup.
  - The API adds `Cache-Control: no-store`.
- **Read-only.** `--read-only` refuses every edit, save, save-as, open, upload and import with 403 `kind:"read_only"`. Tiles, cells and download stay available.
- **Refused before bind:**
  - a non-loopback bind without `--allow-location` or `--read-only`
  - `--no-token` with a non-loopback bind
- **Credentials.** Credentials in `properties` are never echoed, journaled or logged; `Land` holds cells, not sources. `--trace` writes request bodies, a fact stated beside the flag.

### 8.3 JSON API (under `{path}api/`, UI at `{path}`)

Sheets are addressed by key in URLs and bodies. Ranges and references are A1 text.

| Method, path | Request | Response |
| --- | --- | --- |
| `GET workbook` | — | `{"generation":3,"revision":42,"saved":40,"dirty":true,"readOnly":false,"location":"file:///d/book.xlsx","name":"book.xlsx","dateSystem":"1900","timezone":"Europe/Paris","activeSheet":1,"sheets":[{"key":1,"name":"Data","kind":"worksheet","state":"visible"},{"key":2,"name":"Chart1","kind":"chartsheet","state":"visible"}],"names":[{"name":"Rates","scope":null,"text":"=Data!$F$2:$F$9"}],"pivots":[{"sheet":3,"name":"PivotTable1","range":"A3:F12","editable":true,"reason":null}],"calc":{"uncomputed":3,"circular":[{"sheet":1,"ref":"B2"}],"circularCount":1},"undo":{"label":"Typing '=SUM(A1:A3)' in B4"},"redo":{"label":null},"styles":27}` |
| `GET sheets/{key}/layout` | `If-None-Match` | ETag `"l{generation}.{key}.{rev}"`. `{"defaults":{"columnWidth":8.43,"rowHeight":15,"maxDigitWidth":7},"columns":[[0,0,20.7,false,0],[3,5,null,true,4]],"rows":[[0,0,30,false,null],[7,9,null,true,null]],"merges":["D1:E1"],"frozen":{"rows":1,"columns":0},"dimension":"A1:D501","cells":2004,"pivots":[{"name":"PivotTable1","range":"H1:J20","editable":true}],"arrays":["B1:B3"],"blocking":["oleObjects"],"tables":[{"name":"Table1","range":"A1:D20"}]}`. Columns and rows are runs `[first, last, size\|null, hidden, style\|null]`. Rows list only runs differing from the default |
| `GET sheets/{key}/tiles/{tr}/{tc}` | `If-None-Match` | ETag `"t{generation}.{key}.{tr}.{tc}.{rev}"`, computed without rendering, so a 304 renders nothing. `{"cells":[[0,0,"Region",3,0],[1,3,"$1,234.50",7,5],[3,4,"4.5",0,13,"#FF0000"],[4,1,"",12,16],[5,2,"0.333333333",0,1,null,null,["0.33333","0.333","0.3"]]],"merges":["D1:E1"]}`. A tile is 64 rows × 32 columns. A cell is `[dRow, dCol, text, style, flags, color?, fill?, shorter?]`. `flags`: bits 0–1 kind (0 text, 1 number, 2 boolean, 3 error), 4 formula, 8 uncomputed, 16 styled blank, 32 in an array, 64 in a pivot. `shorter` appears only for General numbers longer than 8 characters |
| `GET styles?from=N` | — | `{"count":27,"styles":[{"font":{"name":"Calibri","size":11,"bold":true,"italic":false,"underline":"none","strike":false,"color":"#000000"},"fill":{"pattern":"solid","color":"#FFFF00"},"border":{"bottom":{"style":"thin","color":"#000000"}},"alignment":{"horizontal":"general","vertical":"bottom","wrap":false,"indent":0,"rotation":0,"shrink":false},"numberFormat":"General"}]}`. The array index + `from` is the style id; ids are immutable (append-only); colours are resolved |
| `GET sheets/{key}/cells/{ref}` | — | `{"ref":"D2","entry":"=SUM(D3:D9)","text":"$1,234.50","style":7,"kind":"number","error":null,"stale":false,"pivot":null,"array":null,"merge":null}` |
| `GET sheets/{key}/summary?range=A1:C10,E1:E3` | — | `{"count":27,"numbers":20,"sum":1234.5,"average":61.725,"min":0,"max":400,"text":{"sum":"$1,234.50","average":"$61.73","min":"$0.00","max":"$400.00"}}`. `count` is COUNTA; `text` uses the format of the first numeric cell |
| `GET sheets/{key}/edge?from=B2&direction=down\|up\|left\|right\|region` | — | `{"ref":"B57"}`, or `{"range":"A1:D501"}` for `region` |
| `GET sheets/{key}/export?range=A1:C3&format=tsv\|html` | — | text; the HTML carries `data-yggdryl-copy="{generation}:{revision}:{key}:A1:C3"`; ≤ 1M cells or 32 MiB, else 413 |
| `GET resolve?text=Rates` | — | `{"sheet":1,"range":"F2:F9"}` (references and names; Name Box and Go To) |
| `GET find?q=AAPL&sheet=1&scope=sheet\|workbook&after=B7&in=values\|formulas&case=0&whole=0` | — | `{"match":{"sheet":1,"ref":"C9"}}` or `{"match":null}` |
| `GET functions` | — | `[{"name":"SUM","category":"Math and trigonometry","signature":"SUM(number1, [number2], ...)","description":"Adds its arguments."}]` |
| `GET media` | — | `{"records":["application/vnd.apache.arrow.stream",…],"documents":["application/json","application/x-ndjson",…]}` |
| `POST formula/assist` | `{"sheet":1,"ref":"B4","text":"=A1+Sheet2!B1","caret":3,"action":"cycle"\|"reference"\|"tokens","range":{"sheet":2,"range":"B1:B3"}}` | `cycle` (F4): `{"text":"=$A$1+Sheet2!B1","caret":5}`. `reference` (point mode): `{"text":"Sheet2!B1:B3"}`. `tokens`: `{"tokens":[{"start":1,"end":3,"sheet":1,"range":"A1:A1"}]}` |
| `POST edits` | `{"base":42,"edit":{…§8.4…}}` | `{"revision":43,"changed":[{"sheet":1,"range":"B2:B2"}],"structural":false,"sheets":false,"styles":false,"undoable":true,"calc":{"evaluated":3,"uncomputed":0,"circular":[]},"result":{}}` |
| `POST undo`, `POST redo` | `{"base":43}` | as `edits`; 409 `kind:"nothing_to_undo"` |
| `POST calculate` | `{"full":true}` | as `edits` |
| `GET changes?since=42&wait=25` | — | `{"revision":45,"changes":[{"revision":43,"changed":[…],"structural":false,"sheets":false,"styles":false}]}`, or `{"revision":45,"reset":true}` after open, new or upload, or for a client behind the 1,024-entry ring. Long poll; woken by an edit or `close()` |
| `POST workbook/new` | `{"discard":false}` | `workbook`. 409 `kind:"dirty"` if dirty and not `discard` |
| `POST workbook/open` | `{"location":"s3://b/k.xlsx","properties":{"region":"eu-west-1"},"discard":false}` | `workbook` |
| `POST workbook/upload?name=x.xlsx` | xlsx bytes | `workbook`, with no location |
| `POST workbook/save` | `{"force":false}` | `{"revision":45,"saved":45,"location":"…"}`. 409 `no_location` or `modified` |
| `POST workbook/save-as` | `{"location":"file:///d/b2.xlsx","properties":{},"force":false}` | as save; the location becomes current. A `.xlsx` name is required; an `.xlsm` source saved as `.xlsx` is refused |
| `GET workbook/download` | — | package bytes, `Content-Disposition: attachment; filename="book.xlsx"; filename*=UTF-8''…`; state unchanged |
| `POST locations/probe` | `{"location":"…","properties":{}}` | `{"exists":true,"size":10240,"mediaType":"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"}`. A public answer the Save As dialog asks; saves never guard with it |
| `POST import/preview` | `{"location":"https://h/t.parquet","properties":{},"mediaType":null,"options":{"sheet":null,"header":true,"range":null,"select":null,"filter":null},"rows":100}` | `{"mediaType":"application/vnd.apache.parquet","field":"struct<id: int64, symbol: utf8>","columns":[{"name":"id","type":"int64"}],"rows":[["1","AAPL"]],"rowCount":123456}` (`rowCount` only when the medium answers from metadata) |
| `POST pivots/fields` | `{"source":{"sheet":1,"range":"A1:D501"}}` | `{"fields":[{"name":"Amount","numeric":true,"aggregate":"sum","items":412}]}` |

### 8.4 Edit ops (`Edit::from_scalar`)

| `op` | Body | Core |
| --- | --- | --- |
| `setEntries` | `sheet, entries:[{ref,text}]` | `SetEntries` |
| `fillEntry` (Ctrl+Enter) | `sheet, ranges:[…], text` | `FillEntry` |
| `clear` | `sheet, ranges, what: all\|contents\|formats` | `Clear` |
| `setStyle` | `sheet, ranges, patch:{fontName,fontSize,bold,italic,underline,strike,fontColor,fill,borders:{preset,style,color},horizontal,vertical,wrap,indent,numberFormat}`. An absent key is skipped; `null` clears | `SetStyle` |
| `insertRows` / `removeRows` / `insertColumns` / `removeColumns` | `sheet, at\|start, count` | the same |
| `rowHeight` / `columnWidth` | `sheet, start, count, size\|null` | `RowHeight` / `ColumnWidth` |
| `hideRows` / `hideColumns` | `sheet, start, count, hidden` | the same |
| `merge` / `unmerge` | `sheet, range[, center, across]` | the same |
| `freeze` | `sheet, rows, columns` (0, 0 unfreezes) | `Freeze` |
| `fill` | `sheet, source, target, mode: series\|copy` | `Fill` |
| `sort` | `sheet, range, header, keys:[{column:"B",descending}]` | `Sort` |
| `paste` | `from:{sheet,range}, to:{sheet,ref}, what: all\|values\|formulas\|formats, cut` (Format Painter is `what: formats`) | `Paste` |
| `pasteText` | `sheet, ref, text` | `PasteText` |
| `replace` | `scope: sheet\|workbook, sheet, find, replacement, in, matchCase, entireCell` → `result:{"replaced":12}` | `Replace` |
| `addSheet` / `renameSheet` / `removeSheet` / `moveSheet` / `sheetState` | `name?, at?` / `sheet, name` / `sheet` / `sheet, to` / `sheet, state` | the same |
| `import` | the preview body + `destination:{newSheet:"Trades"}\|{sheet,ref}, header` → `result:{"range":"A1:B250001","rows":250000,"truncated":false}` | resolved by the service to `Land` (§8.7) |
| `pivotCreate` / `pivotUpdate` / `pivotRefresh` / `pivotRemove` | `spec, destination` / `sheet, name, spec` / `sheet, name` or `all:true` / `sheet, name` → `result:{"range":"A3:D8"}` | the same |
| `batch` | `edits:[…]`: one transaction, one undo step | `Batch` |

### 8.5 Error envelope

Every API error is RFC 9457:

```json
{"type":"about:blank","title":"Bad Request","status":400,"detail":"Data!B3: expected at most 32767 characters in a cell, got 40000",
 "kind":"invalid_record","location":"Data!B3","revision":45}
```

`kind` is the error variant in snake case, or one of: `read_only`, `token`, `origin`, `stale_base`, `dirty`, `no_location`, `modified`, `poisoned`, `nothing_to_undo`, `closed`, `panic`.

### 8.6 Concurrency across tabs

An edit carries `base`. It is refused with 409 `kind:"stale_base"` and the current `revision` when any change after `base` in the ring:

- touched a range intersecting the edit's targets, or
- was structural, or changed the sheet list.

Otherwise it applies. On a 409, the client refetches the touched tiles and keeps the typed text for a retry.

### 8.7 Get Data

One path serves preview and load:

1. `Url::from_location` → `LocationPolicy::allows` → `Holder::from_url(&url, properties)` (`mediaType` becomes the `media_type` property) → `into_declared_media()`.
2. For a structured document (`text::Format::from_media_type` is `Ok`): `read_arrow(None)`, with `rows` applied by `Serie::slice`.
3. Otherwise:
   1. `record_options()`
   2. `set_excel_sheet`/`set_excel_header`/`set_excel_range` (Excel only; the core's refusal elsewhere names `$.sheet`)
   3. `with_select`, `with_filter`
   4. `with_max_row_size(min(limit, MAX_ROWS − anchor − header))`
   5. `read_arrow(Some(&options))` → `SerieReader`

- **Preview** renders the first `Serie` through `Cell::from_scalar` + `FormatCode::render`.
- **Load** streams every `Serie` into a staging `Sheet` with `write_serie(tracked_anchor, &serie, first && header)`. It tracks the next row itself and refuses past `max_import_rows` or `max_cells` before the workbook is touched. It then applies `Edit::Land`, which is atomic and undoable.
- `GET media` lists `RecordOptions::mime_types()` (new in `media/options.rs`; `for_mime_type`'s refusal text renders from it) plus the structured formats. `text/csv` answers the core refusal verbatim.

### 8.8 Save

1. The current location is rebuilt with its properties.
2. Conflict check: `open()` → `size()`/`mtime()` against the values captured at open. A change is 409 `modified` unless `force`.
3. `write_all_bytes`: 1 PUT on S3 or HTTP, 1 write locally.
4. `rebase`.

The package always carries `fullCalcOnLoad` and no calcChain when a sheet was rewritten. Failures leave the session dirty and unchanged. Save As does the same to a new location, which becomes current.

---

## 9. CLI (P9)

**`cli/src/serve.rs`** is new and shared, lifted from `cli/src/xmla.rs:39-211`. The inline copies are deleted, including the park loop at `xmla.rs:175`.

```rust
#[derive(Args)] pub struct ServerArgs { /* --bind (127.0.0.1:8080), --max-body, --trace, --public-url, --trusted-proxy,
                                           --forwarded-header, --path-prefix, --read-timeout (30), --allow-host */ }
pub enum Hosts { Any, Loopback }        // the command's default when --allow-host is absent (xmla: Any, excel: Loopback)
impl ServerArgs {
    pub fn options(&self, max_body: u64, hosts: Hosts, headers: Headers) -> Result<ServerOptions>; // recording off, grace 5 s
    pub fn bind(&self, options: ServerOptions) -> Result<Server>;
    pub fn is_loopback(&self) -> Result<bool>;
}
pub fn location(spelled: &str) -> Result<Url>;              // "://" → Url::from_str, else a path (C:\ is a drive)
pub fn folder(spelled: &str) -> Result<Holder>;             // xmla's holder(): Holder::from_url or Holder::folder
pub fn property(spelled: &str) -> Result<(String, String)>; // KEY=VALUE, a clap value_parser
pub fn announce(line: &str, notes: &[String]);              // line 1 alone via outln!, then style::note
pub fn wait(server: Server, on_stop: impl FnOnce()) -> Result<ExitCode>;
```

`wait` works as follows:

- Unix: `signal_hook::flag::register` for SIGINT and SIGTERM.
- The main thread loops on `park_timeout(200 ms)`, printing `warnings::drain()` through `style::warn` each pass, because a long-lived serve must not hold warnings unbounded.
- When stopped it runs `on_stop()` and then `server.shutdown()` (graceful), and answers `ExitCode::SUCCESS`, so `main.rs`'s `warnings::report` runs.
- Windows: no handler is registered; Ctrl+C terminates without the drain (documented).

**`cli/src/excel.rs`**

```
yggdryl excel serve [LOCATION] [--property KEY=VALUE]... [--path /] [--read-only] [--allow-location PREFIX]...
    [--no-token] [--timezone ZONE] [--max-cells 20000000] [--max-import-rows 1048575]
    <ServerArgs: --bind 127.0.0.1:8080 --max-body 256MiB --trace DIR --public-url URL --trusted-proxy NET...
                 --forwarded-header NAME... --path-prefix P --read-timeout 30 --allow-host HOST...>
```

- **LOCATION.** Absent: `Book1`. Naming nothing yet: a new workbook saved there on the first save.
- **Timezone.** `--timezone` defaults to `TZ` when it is an IANA name, else UTC.
- **Order.**
  1. Parse every argument, check the security refusals, open and parse the workbook (and check `--max-cells`). A refusal costs no port.
  2. Bind.
  3. `Arc<Service>::route(&server, path)`.
  4. Register the assets: `Arc::new(assets()).route(&server, path)`, with `Cache-Control: no-cache`.
  5. `announce`.
  6. `wait(server, || service.close())`.
- **Assets.** The CLI embeds `cli/assets/excel/**` through one `include_bytes!` per file, listed in `fn assets() -> Assets`. There is no build script.
- **Output.**
  - Line 1: `http://127.0.0.1:8080/#token=3f9c…`
  - Notes: `· workbook file:///d/book.xlsx (3 sheets, 12,345 cells)`, `· read-only`, `· structural edits blocked on Data: oleObjects`, `· stop with Ctrl+C`
  - On stop with unsaved changes: `style::warn("unsaved changes to file:///d/book.xlsx were discarded")`, exit 0. It never saves unasked.
- **`main.rs`:** `mod excel; mod serve;` (alphabetical); `Excel { #[command(subcommand)] command: excel::Command }`, boxed if `large_enum_variant` fires; the dispatch arm; the module-docs table row; "There are two" becomes "There are three". `announce` uses `outln!`, replacing `println!` at `xmla.rs:160`.
- **`cli/Cargo.toml`:**
  - description and header comment name excel
  - `getrandom = "0.3"` (0.3.4 locked)
  - `[target.'cfg(unix)'.dependencies] signal-hook = "0.3"` (0.3.18 locked, `Cargo.lock:3407`)
  - features `parquet = ["yggdryl/parquet"]`, `s3 = ["yggdryl/s3"]`
  - `Cargo.lock` gains only the cli edges
- **Staging.** `scripts/stage_cli.py:125` builds `--features iceberg,s3`, which also fixes xmla's advertised `s3://`.
- **Tests: `cli/tests/excel.rs`**, sharing `cli/tests/support/served.rs` (`command`, `Served`, `started`, and a new `connect_within(endpoint, 2 s)` retry helper) with `cli/tests/xmla.rs`.
  - Refusals run by default, assert empty stdout, a nonzero exit and the flag named: non-loopback without a policy; `--no-token` off loopback; an `.xls` location; a bad `--timezone`; an unknown scheme; `s3://` without the feature.
  - Live tests, un-ignored with the retry helper:
    - `GET /` is 200 `text/html; charset=utf-8` with CSP and an ETag, then 304
    - every file under `cli/assets/excel/` is served; an unknown asset is 404
    - a missing token is 403; a foreign `Host` is 421
    - an edit, then save, then reopen
    - SIGTERM drains and exits 0
  - If they flake in CI, `#[ignore]` is restored with the reason and reported.
- **Wheel smoke.** `scripts/check_wheel_smoke.py` gains `serve_excel()`: spawn, read line 1, `GET api/workbook` with the token, `POST edits`, `GET` the cell, kill.

---

## 10. Browser UI (`cli/assets/excel/`, vanilla ES modules, no framework, build step or CDN)

**Frozen file list.** The CLI's embed list and the live test assert it. `.gitattributes` gains `cli/assets/** text eol=lf`.

```
index.html  styles.css  icons.svg
js/main.js      boot, token (fragment → sessionStorage), module wiring
js/api.js       fetch wrapper, X-Yggdryl-Token, base revision, problem+json → toast, 409 handling, blob downloads
js/sync.js      long poll `changes`, Web Locks leader `yggdryl-changes`, BroadcastChannel relay
js/tiles.js     data-tile LRU (256 tiles), ETag revalidation, one-tile prefetch ring, ≤ 6 requests in flight
js/styles.js    style table cache (`styles?from=`), CSS font strings
js/geometry.js  default sizes + override runs + prefix sums (O(log k) position↔index), px↔width, frozen quadrants, hit testing
js/render.js    canvas per quadrant at devicePixelRatio; bitmap tile cache (LRU 64 per zoom/geometry version); fills → gridlines
                (skipped under fills and merges) → borders → text (overflow into empty neighbours, `####`, `*` fill, wrap) →
                merges once → selection, fill handle, marching ants, reference highlights
js/grid.js      scroll state (topRow, rowOffset, leftCol, colOffset), drawn scrollbars (no browser element-size limit), repaint
js/selection.js active cell, ranges, multi-range (Ctrl+click), extend
js/keyboard.js  the keymap below
js/editor.js    in-cell editor, formula bar, Name Box, point mode, F4, token colouring, function autocomplete, argument tip
js/clipboard.js copy/cut/paste, internal marker, marquee, Format Painter state
js/ribbon.js    tabs, groups, commands → edit ops
js/sheets.js    tab strip, drag reorder, rename, hide/unhide
js/menus.js     context menus
js/dialogs.js   open, save as (probe → confirm), format cells (number tab: presets, custom code, sample), row height/column
                width, sort, find/replace, go to, insert function, confirm
js/pivot.js     PivotTable fields pane
js/getdata.js   Get Data dialog
js/status.js    status bar
js/a11y.js      visually hidden role=grid mirror of the visible window, aria-live region (also the Playwright oracle)
```

**Measurement exception.** Only rules that need font metrics run client-side: overflow, `####`, choosing among `shorter` spellings, `*` fill expansion and wrap. Every value, format, reference, navigation, fill, sort and find rule is a server call.

**Ribbon**

| Tab | Groups: commands |
| --- | --- |
| File (backstage) | New · Open (location + properties table / upload) · Save · Save As · Download |
| Home | Clipboard: Paste ▾ (All, Values, Formulas, Formatting), Cut, Copy, Format Painter · Font: name, size, **B** *I* U S, Borders ▾ (Bottom, Top, Left, Right, All, Outside, Thick Outside, None), Fill ▾, Font Colour ▾ · Alignment: top/middle/bottom, left/center/right, Wrap, Merge & Center ▾ (Merge & Center, Merge Across, Merge, Unmerge), Indent −/+ · Number: format ▾ (General, Number `0.00`, Currency `"$"#,##0.00`, Accounting `_("$"* #,##0.00_)…` (custom id), Short Date (14), Long Date `dddd, mmmm d, yyyy`, Time (19), Percentage (10), Fraction (12), Scientific (11), Text (49), More…), `$`, `%`, `,` (custom id), decimals ± · Cells: Insert ▾ (rows, columns, sheet), Delete ▾, Format ▾ (row height, column width, hide/unhide, rename, move) · Editing: AutoSum ▾ (Sum, Average, Count, Max, Min), Fill ▾ (Down, Right, Up, Left), Clear ▾ (All, Formats, Contents), Sort ▾ (A→Z, Z→A, Custom…), Find & Select ▾ (Find, Replace, Go To) |
| Insert | PivotTable |
| Formulas | Insert Function (categories from `functions`), AutoSum, Calculate Now (F9) |
| Data | Get Data, Refresh All (pivots), Sort A→Z / Z→A / Sort… |
| View | Freeze Panes ▾ (Freeze Panes, Top Row, First Column, Unfreeze), Gridlines (view only), Zoom 25–400% |

**Keyboard**

- **Navigation:** arrows, Shift+arrows, Ctrl+arrows / Ctrl+Shift+arrows (`edge`), Home, Ctrl+Home, Ctrl+End, PgUp/PgDn, Alt+PgUp/PgDn, Tab/Shift+Tab and Enter/Shift+Enter (moving within the selection, returning to the start column), Ctrl+PgUp/PgDn (sheets), Ctrl+G/F5 (Go To).
- **Selection:** Ctrl+A (current region, then all), Ctrl+Space, Shift+Space, Ctrl+click (multi-range).
- **Editing:** F2 (edit/enter mode), Esc, Delete (clear contents), Backspace (clear and edit), Alt+Enter, Ctrl+Enter (`fillEntry`), Ctrl+D, Ctrl+R, Ctrl+; and Ctrl+Shift+; (today/now through `setEntries` of the clock's text from `workbook.timezone`), F4 (`assist cycle`).
- **Clipboard and history:** Ctrl+C/X/V, Ctrl+Z, Ctrl+Y and Ctrl+Shift+Z.
- **Formatting and structure:** Ctrl+B/I/U, Ctrl+1 (Format Cells), Ctrl+− and Ctrl+Shift+= (delete/insert), Shift+F11 (new sheet).
- **Workbook:** Alt+= (AutoSum), Ctrl+S, Ctrl+F, Ctrl+H, F9.

**Clipboard**

- **Copy** (a user gesture) fetches `export` TSV and HTML, then `navigator.clipboard.write`; localhost is a secure context. The `copy` event is the fallback.
- **Paste** reads `clipboardData` in the `paste` event:
  - our marker for the current generation → `paste` (with `cut` if the source was cut)
  - otherwise → `pasteText`
- **Fill handle** drag → `fill` with `series`; Ctrl toggles `copy`. Double-click fills to `edge?direction=region`'s extent.

**Status bar:** mode (Ready, Enter, Edit, Point), Average/Count/Numerical Count/Min/Max/Sum (debounced 150 ms `summary`), uncomputed count and circular references, the timezone, saved/unsaved, zoom.

**Dialogs:**

- **Get Data:** location, properties table, media type from `media`, sheet/header/range for `.xlsx`, select/filter/rows; Preview grid with type chips; destination (new sheet name, or existing sheet + anchor); Load, with progress in the status bar.
- **PivotTable pane:** field checklist from `pivots/fields`; drop wells Rows/Columns/Values with keyboard "Add to…" buttons; value settings (aggregate, caption, number format); subtotals and grand-totals toggles; order per field; Refresh; Delete. A foreign read-only pivot shows its reason.

**Accessibility:**

- Every control is a real `<button>` with `aria-label`/`aria-pressed`/`aria-expanded`. Menus are `role=menu`, dialogs are `<dialog>` with a focus trap.
- The canvas is `aria-hidden`. The mirror carries `aria-rowcount`/`aria-colcount`/`aria-rowindex`/`aria-colindex` and `aria-activedescendant`.
- `forced-colors` and `prefers-reduced-motion` are honoured (static ants).
- **Font stack:** `Calibri, Carlito, "Segoe UI", Arial, sans-serif`.
- **Trademark:** the title is "‹file› — yggdryl workbook", with our own icons and palette.

**Verification.** `scripts/check_excel_ui.py` (Python Playwright, Chromium) runs against a binary given by `--binary`:

1. Serve an openpyxl fixture on `--bind 127.0.0.1:0` and read line 1.
2. Type `12%` in A1; the formula bar shows `12%`, and `cells/A1` shows kind number.
3. Type `=A1*2` in B1; the mirror reads `0.24`.
4. Ctrl+B changes the cell's style to bold. Ctrl+Z restores it.
5. Insert a row above; the formula text shifts.
6. Fill-handle drag of `1, 2` down 3 gives `3, 4, 5`.
7. Find `AAPL`.
8. Get Data from a JSON fixture.
9. Create a pivot.
10. Ctrl+S, then openpyxl re-reads the file: the formula, bold, `ws._pivots`, merges kept.

The script fails on a console error or the word `SKIPPED` and writes screenshots to `excel-ui-artifacts/`. CI job `excel-ui` (§12.5).

---

## 11. Bindings (P10 Python, P11 Node)

All of these redirect to the core. Each gets a parity test, a boundary benchmark row, docs tabs and inventory entries.

| Core | Python (`python/src/excel.rs`, `python/yggdryl/excel.py`, `_native.pyi`) | Node (`node/src/excel.rs`, `binding.js`/`binding.d.ts`) |
| --- | --- | --- |
| `Workbook::insert_rows/remove_rows/insert_columns/remove_columns` | `insert_rows(sheet, at, count)`, `remove_rows(sheet, start, stop)`, `insert_columns`, `remove_columns`; `Sheet.insert_rows/remove_rows` **deleted** | camelCase; `Sheet` ones deleted |
| `move_sheet`, `rename_sheet` (now rewrites), `is_dirty` | `move_sheet(name, to)`, property `is_dirty` | `moveSheet`, getter `isDirty` |
| `write_into` (core, rebases) | `write_into(target)`, `py.detach`, lock inside | `writeInto(target)` |
| `set_entry`, `entry_text`, `display_text` | `set_entry(sheet, ref, text)`, `entry_text(sheet, ref)`, `display_text(sheet, ref)` | `setEntry`, `entryText`, `displayText` |
| `recalculate`, `calculate_all` → `Recalculation` | `dict` `{evaluated, uncomputed, circular}` | plain object |
| `cell_style`, `set_style(StylePatch)` | `CellStyle` (frozen, hashable via `stable_hash`); `set_style(sheet, ranges, *, font_name=..., font_size=..., bold=..., italic=..., underline=..., strike=..., font_color=..., fill=..., borders=..., horizontal=..., vertical=..., wrap=..., indent=..., number_format=...)` — `...` skipped, `None` clears; colours are `"#RRGGBB"` | `setStyle(sheet, ranges, { … })` — `undefined` skipped, `null` clears |
| `fill`, `sort` | `fill(sheet, source, target, series=True)`, `sort(sheet, range, keys, header=False)` | `fill`, `sort` |
| `Sheet` layout verbs (§4.2) | `column_width`, `set_column_width(start, stop, width)`, `row_height`, `set_row_height`, `set_columns_hidden`, `set_rows_hidden`, `merges()`, `merge`, `unmerge`, property `frozen` / `set_frozen(rows, columns)` | camelCase |
| `Cell` | `Cell.formula` (file spelling at the cell), `Cell.error` (validated text), `from_parts(…, error)` through `ExcelError::from_text` | the same |
| Pivots | `PivotSpec` (frozen, `from_json`/`into_json`/pickle, modelled on `PyPartitionSpec`, `iceberg.rs:2281`); `Workbook.pivots()` → list of dicts `{sheet, name, range, editable, reason, spec}`; `add_pivot(spec, sheet, anchor)`, `update_pivot`, `refresh_pivot`, `remove_pivot` | `PivotSpec` (`fromJson`/`intoJson`); the same verbs |
| http (§7.1) | `Server.route` patterns with `**params`; `bind(allowed_hosts=, default_headers=, shutdown_grace=)` | `Server.bind({ allowedHosts, defaultHeaders, shutdownGrace })` |

- **Rust/CLI-only**, documented as such: `excel::Service`, `Journal`, `Edit`, `Package`/`rebase`, `Formula`, `FormatCode`, `StyleSheet`, `Theme`, `Clock`, `http::Context`/`Assets`/`Conditions`.
- Python `Workbook` gains `#[classattr] const __hash__: Option<Py<PyAny>> = None;`, fixing the stub/runtime mismatch.
- `python/tests/typing_bindings.py` gains typed calls for the new Workbook methods, so mypy exercises the stubs.
- Node `node/package.json` gains `bench:media:excel`, and `node/benchmarks/media/excel.js` is new.
- `node/index.js` and `node/index.d.ts` are regenerated.
- `docs/assets/playground.json` and `fix.json` are regenerated after the addon build.

---

## 12. Tests, pins, benchmarks, interop, docs, skills, inventories, CI

### 12.1 Rust tests (mirrored; every file opens with `//!` naming its source)

- **`rust/tests/excel.rs` declares, beside today's files:**
  - `excel/{style,theme,format,entry,layout,carried,names,shift,edit,journal,fill,find}.rs`
  - `excel/formula.rs`, which declares `formula/{lexer,reference,shape,parser,value,number,eval,graph,aggregate,criteria,functions}.rs` and `formula/functions/{math,statistical,logical,lookup,text,date,information,financial}.rs`
  - `excel/pivot.rs`, which declares `pivot/{compute,layout,part}.rs`
  - `#[cfg(feature = "http")] excel/server.rs`, which declares `server/{api,session,tiles,import,guard}.rs`
- **Extended:** `excel/{cell,sheet,parser,package,shared_strings,styles,workbook,writer,media}.rs`.
- **Private pins** (`carried`, `shape`, `router`) live in `#[cfg(feature = "internals")] mod internal`. `scripts/generate_internals.py` is re-run and then `--check`ed.
- **HTTP:**
  - `rust/tests/http/server.rs` declares `server/{router,context}.rs`
  - `rust/tests/http.rs` declares `http/assets.rs`
  - `rust/tests/http/headers.rs` declares `headers/conditional.rs`
  - `rust/tests/xmla/server.rs` is updated
- **Zip:** `rust/tests/zip/archive.rs` covers `copy_member_from`.
- **Discipline:** refusal-first. Every refusal in §§2–9 has a test that ran red first. Function tables use hand-verified vectors from Microsoft's documentation.

### 12.2 Cost pins

| Pin | Asserts |
| --- | --- |
| `iobase_calls::excel_costs` (re-pinned, reasons stated) | Open plus `parse_all` = today's open + each sheet part, sst, styles and theme once. `write_into` = 1 `write_all_bytes` on the target and 0 target reads. Clean members are copied raw (0 decode reads). `rebase` = 0 calls |
| `iobase_calls::excel_service_costs` (http) | Service open over the fixture HTTP server = 1 HEAD + 1 GET; save = 1 HEAD + 1 PUT; tiles/edits = 0 |
| `iobase_calls::http_assets` | 0 handle calls per asset request |
| `allocations::excel_cell_is_at_most_80_bytes` | `size_of::<Cell>()`, stating the figure reached |
| `allocations::excel_sheet_costs_per_row_and_nothing_per_cell` (re-pinned: "a row is one exact-capacity `Vec`, not a B-tree node per 11 cells") | one allocation per parsed row plus the rows map's nodes, at two corpus sizes |
| `allocations::excel_cell_reads_allocate_nothing` | holds with `StyleId` |
| `allocations::excel_tile_render_allocates_per_tile` | one JSON buffer per tile at 10k and 1M cells |
| `allocations::excel_sum_allocates_nothing_per_cell` | SUM over 1k and 100k cells cost the same |
| `allocations::excel_shared_formula_parse_allocates_per_shape` | N dependents cost one shape |
| `allocations::excel_pivot_compute_independent_of_records` | equal at 1k and 100k records for fixed distinct items |
| `allocations::http_route_lookup_allocates_nothing`, `http_asset_answers_share_the_body` | §7.2 |
| `excel/formula/graph.rs` counts | editing A1 under a 10,000-link chain evaluates exactly 10,000; an input to `SUM(A:A)` evaluates 1; a cell nothing reads evaluates 0 |

**Benchmarks:**

- `rust/benchmarks/media/excel.rs` gains `excel_open_parse_all`, `excel_tile`, `excel_recalc_chain`, `excel_recalc_fanin`, `excel_save_dirty`, `excel_save_shared` (file size and time for expanded shared groups), `excel_pivot`, `excel_insert_rows`.
- `holder/http.rs` gains `http_route_param`, `http_asset_304`.
- `python/benchmarks/media/excel.py` gains `set_entry`, `recalculate`, `write_into`; `node/benchmarks/media/excel.js` has the same rows.
- No page states a number without a release run.

### 12.3 Interop (`scripts/check_excel_interop.py`, `rust/tests/interop/excel.rs`; CI `excel-interop`, unchanged YAML)

- **Fidelity.**
  - openpyxl writes styles (bold/fill/border/percent/currency), cols, row heights, merges, a frozen pane, rich text, shared formulas, a defined name, conditional formatting, data validation, a hyperlink, an autofilter and a table.
  - Rust opens it, edits one cell per sheet, inserts a row above the table and saves with `write_into`.
  - openpyxl asserts every feature survived and every shift is exact.
- **Formulas.** Rust writes `_xlfn.` formulas with cached values, and openpyxl reads the text and the `data_only` values. openpyxl formulas without cached values are recomputed by Rust.
- **Pivots:** §6.6.

### 12.4 Docs and skills (written after Node settles)

- **`docs/media/index.md` `## Excel`:**
  - fix the "untouched" prose
  - add `### Saving and fidelity` (the §2.4 table + §2.1 list), `### Styles and number formats`, `### Formulas` (the §5.1 paragraph, the function table, uncomputed rules), `### Editing` (structural verbs, fill, sort, find, undo), `### Pivot tables` (cross-link `#excel-as-a-client`), `### Serving a workbook` (bash usage, flag table cross-referencing `### Behind a reverse proxy`, the API table in short, security)
  - every block in Rust/Python/JavaScript tabs; Rust-only stated explicitly
  - `### Excel performance` keeps "no table until a release run"
- **`docs/holder/index.md` `### Serving a handle`:** patterns and `Context`, `Assets`, conditional requests, problem details, shutdown grace, allowed hosts, default headers. Delete "route answers one exact path".
- **Rows** in `README.md` (Layout: `excel/`, `cli/`), `docs/architecture.md:34`, `docs/contributing.md:50`, `docs/index.md` Layers.
- **Skills:**
  - `skills/yggdryl-records/SKILL.md`: door rows for `write_into`, `set_entry`, `set_style`, `recalculate`, structural verbs on `Workbook`, pivots; pitfalls for structural refusals, uncomputed values, CSV
  - `references/{rust,python,javascript,formats}.md` recipes
  - new `references/excel-serve.md`, bash only
  - `skills/yggdryl-storage/SKILL.md:60` + `references/{backends,rust}.md`: routes, patterns, assets
  - `skills/yggdryl/SKILL.md` choose-the-skill row

### 12.5 CI

`.github/workflows/ci.yml` gains the job `excel-ui` ("Excel app"), shaped like `excel-interop`:

1. checkout
2. setup-python 3.12
3. rustup stable
4. rust-cache (`shared-key: stable-default`, `save-if: false`)
5. `python -m pip install playwright==1.52.0 openpyxl`
6. `python -m playwright install --with-deps chromium`
7. `cargo build --locked -p yggdryl-cli`
8. `python scripts/check_excel_ui.py --binary target/debug/yggdryl`
9. `actions/upload-artifact` of `excel-ui-artifacts/` on failure

Nothing else in `ci.yml` changes. The CLI tests, wheel smoke and interop jobs pick up the new work through their existing steps.

### 12.6 Inventories and the contract file

- **`.api-inventory.txt`:**
  - new sections for every new `rust/src/excel/**` and `rust/src/http/**` file, the server one marked `(behind the http feature)`
  - edited `excel::{cell,sheet,styles,shared_strings,workbook,package,mod}`, `http::{server,status,response,request,headers}`, `codec`, `media::options`, `zip::archive`
  - the `DateSystem` line fixed
- **`.api-bindings.txt`:** the Python and JS `excel.*` and `http.Server` changes; the `DEFAULT_SHEET_NAME` text.
- **Check:** `python scripts/check_api_inventory.py`.
- **AGENTS.md, proposed text only:**
  - `excel/` Layout row: add "`formula.rs` + `formula/` the cell formula language and its engine; `pivot.rs` + `pivot/`; `carried.rs`, `shift.rs`, `edit.rs`, `journal.rs`; `server.rs` + `server/` the workbook service (`http`)"
  - `http/` row: add "`server/router.rs` pattern routes, `server/context.rs` `Context`, `assets.rs` `Assets`, `headers/conditional.rs`"
  - applied only with the user's approval, and listed in the handoff

---

## 13. Implementation plan

**Constraints.**

- One cargo target and lock, so **Agent A is the only one running cargo**.
- **Agent B** edits only non-cargo files: UI assets, `.gitattributes`, the Playwright script, the CI job, and later docs, skills and inventories. B never runs cargo.
- A and B file sets are disjoint in every phase.
- Model tiers: A runs on the foreground tier; B on the tier below. Mechanical sweeps are scripts driven by the compiler's list.
- `cargo fmt --all` runs once, after the last edit.

**Scratchpad tooling for B (never committed):**

- `$SCRATCH/ui/dev.py`: serves `cli/assets/excel/**` from the working tree. Until P9 it answers `/api/*` from `$SCRATCH/ui/fixtures/*.json`, written by B from §8's examples. From P9 on it proxies `/api/*` to `$SCRATCH/bin/yggdryl excel serve --no-token --bind 127.0.0.1:0`, a binary A copies at the end of P9.
- B tries `python -m venv $SCRATCH/ui-venv && $SCRATCH/ui-venv/bin/pip install playwright==1.52.0 && $SCRATCH/ui-venv/bin/python -m playwright install chromium` through the proxy. If that fails, B's browser check is reported as skipped and the `excel-ui` CI job is the first browser run.
- **B's smoke per phase:** `for f in cli/assets/excel/js/*.js; do node --experimental-default-type=module --check "$f"; done`, plus `check_excel_ui.py` against the dev server when Playwright is available.

Filters follow `--`, and several filters are one invocation.

| Phase | Agent A: files | A settles with | Agent B: files (concurrent, disjoint) |
| --- | --- | --- | --- |
| P0 | Foreground: this document is the frozen contract (§8, §10 file list) | — | — |
| P1 Storage, dirty, saving, style table | `rust/src/excel/{cell,sheet,layout,style,styles,workbook,package,writer,reader,media,mod}.rs`, `rust/src/zip/archive.rs`, `rust/tests/excel.rs`, `rust/tests/excel/{cell,sheet,style,styles,workbook,package,writer,reader,media,mod_}.rs`, `rust/tests/zip/archive.rs`, `rust/tests/support/excel_package.rs`, `rust/tests/{iobase_calls,allocations}.rs` (excel rows), `rust/benchmarks/media/excel.rs` | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl --test excel`; `cargo test -p yggdryl --test zip -- archive::`; `cargo test -p yggdryl --test iobase_calls -- excel`; `cargo test -p yggdryl --test allocations -- excel`; `cargo test -p yggdryl --doc excel::`; `python/.venv/bin/python scripts/check_excel_interop.py` | `.gitattributes`, `cli/assets/excel/{index.html,styles.css,icons.svg}`, `js/{main,api,geometry,tiles,styles,render,grid,selection,keyboard,a11y}.js`; scratchpad fixtures and `dev.py` |
| P2 Frame, strings, names, formulas on the wire | `excel/{parser,carried,layout,shared_strings,names,sheet,workbook,package,formula}.rs`, `excel/formula/{lexer,reference,shape}.rs`, tests incl. `excel/{carried,names,formula}.rs` + `formula/{lexer,reference,shape}.rs`, `rust/tests/interop/excel.rs`, `scripts/check_excel_interop.py` (fidelity half), `excel_package.rs` (`rich_package`) | `cargo test -p yggdryl --test excel`; `cargo test -p yggdryl --features internals --test excel -- carried shape`; `python scripts/generate_internals.py && python scripts/generate_internals.py --check`; interop script | `js/{editor,ribbon,menus}.js` (Home tab) |
| P3 Display, entry, styles on the ribbon | `excel/{format,theme,entry,style,styles,cell,workbook}.rs`, tests `excel/{format,theme,entry,style,styles,workbook}.rs` | `cargo test -p yggdryl --test excel -- format theme entry style`; `cargo test -p yggdryl --doc excel::format` | `js/{status,clipboard,dialogs}.js` |
| P4 Structure and edits | `excel/{shift,edit,journal,fill,find,sheet,workbook,carried,layout,package,formula}.rs`, `excel/formula/shape.rs`, tests `excel/{shift,edit,journal,fill,find,sheet,workbook}.rs` | `cargo test -p yggdryl --test excel -- shift edit journal fill find sheet workbook`; `cargo test -p yggdryl --test allocations -- excel` | `js/sheets.js`, fill handle and Format Painter in `render.js`/`clipboard.js`, find/replace/sort/go-to in `dialogs.js` |
| P5 Formula engine | `excel/formula/{parser,value,number,eval,graph,aggregate,criteria,functions}.rs`, `excel/formula/functions/*.rs`, `excel/{formula,workbook,sheet,entry}.rs`, their tests, `allocations.rs` rows, `rust/benchmarks/media/excel.rs` | `cargo test -p yggdryl --test excel -- formula`; `cargo test -p yggdryl --test allocations -- excel_sum excel_shared`; `cargo bench -p yggdryl --bench media -- excel_recalc --quick` | assist (F4, point mode, token colours, autocomplete, argument tip) in `editor.js`; Insert Function in `dialogs.js` |
| P6 Pivot | `excel/pivot.rs`, `excel/pivot/{compute,layout,part}.rs`, `excel/{workbook,package,edit}.rs`, tests `excel/pivot.rs` + `pivot/*.rs`, interop pivot half | `cargo test -p yggdryl --test excel -- pivot`; `cargo test -p yggdryl --test allocations -- excel_pivot`; interop script | `js/{pivot,getdata}.js` |
| P7 Generic HTTP + xmla + binding http sites | `rust/src/http/{server.rs,server/{router,context,connection,mount,h2,h3,framed}.rs,assets.rs,headers.rs,headers/{conditional,range}.rs,request.rs,response.rs,status.rs,mod.rs}`, `rust/src/codec.rs`, `rust/src/xmla/server.rs`, `python/src/http.rs`, `node/src/http.rs`, `rust/tests/http/**`, `rust/tests/xmla/**`, `rust/tests/support/http_server.rs`, `rust/tests/allocations.rs` + `iobase_calls.rs` (http rows), `rust/benchmarks/holder/http.rs`. The sweep script flags `.clone()` on handler arguments | `cargo check -p yggdryl --all-targets --features http --keep-going --message-format=short`; `cargo test -p yggdryl --features http --test http -- server assets headers::conditional`; `cargo test -p yggdryl --features http --test xmla -- server`; `cargo test -p yggdryl --features "http internals" --test allocations -- http_`; `cargo check --workspace --all-targets --keep-going --message-format=short` (every `http.rs` binding site fixed; the listed excel binding sites belong to P10/P11) | `scripts/check_excel_ui.py`; `.github/workflows/ci.yml` (`excel-ui` job) |
| P8 Excel service | `rust/src/excel/server.rs`, `excel/server/{api,session,tiles,import,guard}.rs`, `excel/mod.rs`, `rust/src/media/options.rs` (`mime_types`), `rust/tests/excel.rs`, `rust/tests/excel/server.rs` + `server/*.rs`, `iobase_calls.rs` + `allocations.rs` service rows | `cargo test -p yggdryl --features http --test excel -- server`; `cargo test -p yggdryl --features http --test iobase_calls -- excel_service`; `cargo test -p yggdryl --features http --test allocations -- excel_tile`; `cargo test -p yggdryl --test media -- options` | UI adjustments against the frozen contract only |
| P9 CLI | `cli/src/{main,serve,xmla,excel}.rs`, `cli/Cargo.toml`, `Cargo.lock`, `cli/tests/{xmla,excel}.rs`, `cli/tests/support/served.rs`, `scripts/{stage_cli,check_wheel_smoke}.py`; at the end, `cp target/debug/yggdryl $SCRATCH/bin/` | `cargo check -p yggdryl-cli --all-targets --keep-going --message-format=short`; `cargo test -p yggdryl-cli --test excel --test xmla`; `cargo clippy -p yggdryl-cli --all-targets --no-deps -- -D warnings` | `dev.py` switched to proxy mode; UI integration against the real binary |
| P10 Python | `python/src/{excel,http}.rs`, `python/yggdryl/{excel.py,_native.pyi}`, `python/tests/{test_excel,test_http,typing_bindings}.py`, `python/benchmarks/media/excel.py` | `python/.venv/bin/python -m maturin develop -m python/Cargo.toml`; `python/.venv/bin/python -m pytest python/tests/test_excel.py python/tests/test_http.py -x -q`; `python/.venv/bin/python -m mypy --strict --config-file python/pyproject.toml python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py` | UI polish, a11y, Playwright runs if available |
| P11 Node | `node/src/{excel,http}.rs`, `node/{binding.js,binding.d.ts,package.json}`, generated `node/{index.js,index.d.ts}`, `node/tests/{excel,http}.{test.js,types.ts}`, `node/benchmarks/media/excel.js`, regenerated `docs/assets/{playground,fix}.json` | `npm run --prefix node build:debug`; `node --test node/tests/excel.test.js node/tests/http.test.js`; `npm test --prefix node`; `node scripts/build_docs_playground.js && node scripts/build_docs_fix.js` | (idle, or UI polish) |
| P12 Docs ∥ whole run | Background chain, one log with a marker per step: `cargo test --locked --workspace --all-targets --all-features --no-fail-fast` → `cargo clippy --locked -p yggdryl --all-targets --no-deps -- -D warnings` → `cargo clippy --locked --workspace --all-targets --all-features --no-deps -- -D warnings` → `cargo test --locked -p yggdryl --doc` → `cargo test --locked -p yggdryl-cli --all-targets` → §3 pre-push (`pytest python/tests`, mypy) → §4 pre-push (`test:package:debug`, `git diff --exit-code -- node/index.js node/index.d.ts`, `npm test`, both manifests `--check`). In the foreground, A re-pins non-cost pins the run reports, each with its reason; a moved cost pin is a design answer. A's files here are `rust/tests/**` pins only | — | `docs/media/index.md`, `docs/holder/index.md`, `README.md`, `docs/{architecture,contributing,index}.md`, `skills/**`, `.api-inventory.txt`, `.api-bindings.txt`; smoke `python -m mkdocs build --strict --config-file mkdocs.yml` |
| P13 Handoff | After B returns: `python scripts/check_api_inventory.py`, `python scripts/check_docs_examples.py --lang rust`, `--lang python`, `--lang javascript` (chained, background), `mkdocs build --strict`; `cargo fmt --all` once; `git status --short`; one commit; push; read CI (all jobs, including `excel-ui` and `excel-interop`) | CI green and read | — |

**Handoff report**, per AGENTS.md:

- CI status for every job that failed and was fixed.
- Local-only checks run: `cargo bench -p yggdryl --bench media -- excel --quick` and `--bench holder -- http_ --quick`, for direction only.
- Checks skipped as not applicable: charset table drift and charset interop (no charset change).
- Checks skipped as not runnable here: the browser check if Playwright could not be installed; opening the §6.6 files in Excel.
- The AGENTS.md rows awaiting approval.

---

## 14. Risks, with decisions taken

1. **Excel acceptance of our pivot XML, with no Excel in CI.** The shape is the one openpyxl round-trips (verified). With `refreshOnLoad`, a layout divergence costs a re-render, never a repair. *Decision:* ship. The handoff states that Excel acceptance is unverified. A later PR may commit an Excel-resaved fixture.
2. **Formula numerics oracle.** *Decision:* hand-verified vectors from Microsoft's function pages, and `number.rs` thresholds pinned. An Excel-cached `functions.xlsx` fixture is follow-up work.
3. **Memory.** `parse_all` holds every cell at ≤ 80 B + graph. *Decision:* `--max-cells 20,000,000`, refused at startup with the count. The per-cell bound is pinned.
4. **Structural refusals** that remain (§4.1). *Decision:* accept; `layout.blocking` lists them per sheet.
5. **Shared groups written plain on rewritten sheets** inflate the file. *Decision:* accept; `excel_save_shared` measures it. Regrouping is not attempted: Excel never writes overlapping groups, and neither will we.
6. **Collation** differs from Windows word sort. *Decision:* ordinal after simple case folding, documented and pinned.
7. **`fullCalcOnLoad` makes Excel prompt "save changes?"** on close. *Decision:* accept. It is the only guarantee for what the engine left uncomputed.
8. **Windows signals.** *Decision:* no `ctrlc` (not locked). Unix gets `signal-hook`; on Windows Ctrl+C terminates without a drain (documented).
9. **Live CLI tests flaking.** *Decision:* the retry helper, un-ignored. If CI flakes, `#[ignore]` is restored with the reason and `check_wheel_smoke.py` is the live proof.
10. **Playwright cost and flakiness.** *Decision:* one pinned script and job, waiting on the mirror and the API revision, never on timeouts, and never re-run to go green.
11. **Browser vs. server timezone for TODAY/NOW.** *Decision:* the service's `--timezone`, shown in the status bar.
12. **Stale `docProps/app.xml` titles.** *Decision:* carried. Excel rewrites them.
13. **PR size**, tens of thousands of lines. *Decision:* one PR and one commit, as requested. If review capacity forces a split, cut after P6: a lossless, formula-aware, pivot-capable core, with P7–P13 next.
14. **CSV in Get Data.** *Decision:* a separate delimited-medium PR; the dialog shows the core refusal verbatim.
15. **Copying styles between workbooks.** *Decision:* values and number-format meanings move, visual styles reset (§2.2). `copy_sheet_from` is follow-up work.
