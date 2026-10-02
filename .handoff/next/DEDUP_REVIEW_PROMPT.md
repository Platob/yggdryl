# Prompt: consolidate duplicated logic in the serie code and its cast engine

You are working in the Rust crate `yggdryl` at `/home/user/yggdryl`. Before anything else, read `AGENTS.md` at the repository root. It is the contract, and this prompt only applies it. Read these sections in full: Workflow and Pace, Smoke loop, Always, §1 Layout, Where a test lives, Patterns (One type one file; Serie is the collection; Zero copy; Intake), Public vocabulary, Arrow and allocation, and §2.

## Goal

Find and verify duplicated logic in the serie code and in the code it uses to cast and move values across, then remove it. Start with casting. Every fact should end up with one owner, and the tree should get smaller. The following stay exactly as they are:

- behaviour
- error text
- wire formats
- allocation and call costs

When the copies of a duplicate disagree, treat the disagreement as a **defect**. Report it, pin it with a red test, and fix it on purpose. Never "unify" it to whichever copy the sweep happened to keep.

## Scope

**In scope:**
- `rust/src/cast.rs`, `budget.rs`
- `serie.rs`, `serie/*.rs` (including `serie/value.rs`, `serie/arrow.rs`, `serie/order.rs`, `serie/layout.rs`)
- `chunked_serie.rs`, `window_serie.rs`
- the per-type `pub(crate) mod casts` modules: `bytes.rs`, `string.rs`, `uuid.rs`, `version.rs`, `timezone.rs`, `mime_type/datatype.rs`, `media_type/datatype.rs`, `uri/datatype.rs`, `enums.rs`, `temporal.rs`, `decimal.rs`, `geospatial.rs`
- `arrow/` (`mod.rs`, `size.rs`, `rows.rs`)
- call sites that re-implement a serie or cast step: `csv/`, `avro/batch.rs`, `parquet/mod.rs`, `iceberg/{scan,table}.rs`, `media/{merge,partition}.rs`, `expression/{arrow,selector,plan,eval,typing,transform}.rs`, `xxhash/arrow.rs`, `graph/arrow.rs`, `merge.rs`, `text/arrow.rs`, `ipc/mod.rs`

**Out of scope here.** Other specs own these. List them in your report and do not consolidate them a second time:
- **X1, record wrapping (`struct<self>`).** This belongs to the struct-conversion spec: `into_struct_type` / `into_struct_field` / `into_struct_scalar` / `into_struct_serie` on `DataType`, `Field`, `Scalar` and `Serie`. The sites it folds into its owner are:
  - `serie/arrow.rs`: `record_root`, `SerieReader::root_of`, `held_root`, `held_record`, `batch_schema`, and the record branch of `batch_under`
  - `serie/order.rs` `record_of` (root named `"key"`)
  - `excel/sheet.rs` `record_columns`
  - `avro/arrow.rs` (≈36-45)
  - `Field::new(DEFAULT_ROOT_NAME, dtype, false)` at `expression/{plan.rs ≈1047, selector.rs ≈781, filter.rs ≈191, records.rs ≈148}` and `xxhash/arrow.rs` `level_root`
  - the literal `"row"` at `parquet/mod.rs ≈610`, `arrow/mod.rs ≈587`, `media/inference.rs ≈133`, `iceberg/metadata.rs ≈1122/1127`, `datatype.rs ≈1669`
  - the about 30 `field_from_arrow_schema(DEFAULT_ROOT_NAME, ..)` imports

  If that spec has already landed when you start, fold any copies it left into its owner. If it has not, leave them alone.
- **X2, temporary file and folder guards.** The spill spec (`SpilledSerie`, `is_spilled` / `as_spilled` / `into_spilled`) owns `media/merge.rs` `TemporaryFile` and `iceberg/staging.rs` `Staging`.
- **X3, static values (task #14).** This covers any duplicate `WINDOWNUM` / `ROWNUM` / `Statics` definitions in `serie/arrow.rs` against the statics owner.

## Rules that govern this work (AGENTS.md)

- **One owner per fact, one spelling per verb.** There is no second schema, parser or dispatcher (§1 invariants). Under "Simple code", abstract only to remove real duplication or to enforce an invariant. A value's behaviour is a method on it, not a helper. No speculative generality: a helper with one caller is not a consolidation.
- **One cast engine, reached through Serie.** `cast.rs` owns recursive casting. Only `Serie::cast`, the `Serie` / `ChunkedSerie` Arrow doors, `ChunkedSerie::cast`, `SerieReader`, a held `ArrowCastPlan` and the stage plans reach it. There is no cast door on `DataTypeValue` or `FieldValue`, and no cast function in `arrow/`. A stream, bind or write session holds one plan, and a loop whose schema can change holds a `PlanCache`.
- **`serie/value.rs` is the one row-to-slot codec.** Nothing outside `serie/`, `cast.rs` and `temporal.rs` names it.
- **`DataType::scalar` / `Field::scalar` is the one value contract.** A cast is not a value check.
- **A column holds only rows its field accepts.** Use `Proof::Proven` only where every exposed row was read under the target's rule with no foreign-bytes fallback. An extension label is not a proof. Review every `Proof::Proven` site you touch before committing.
- **No back-compat.** Delete the replaced function, macro, const, test and doc in the same change, and update every caller.
- **Cost pins are design answers.** `rust/tests/iobase_calls.rs`, `rust/tests/allocations.rs` and the benchmarks are never re-pinned from a sweep. A count may move only in the cheaper direction, in the same edit, with a sentence saying why.
- **Tests mirror sources.** A new shared helper in `cast.rs` is pinned in `rust/tests/root/cast.rs`. A private pin goes in that file's `#[cfg(feature = "internals")] mod internal`, through a forwarding `pub fn` in the source module's `internals`.
- **Layout.** A shared helper goes into an existing owner (`cast.rs` `text` / `columns`, `serie/layout.rs`, `arrow/mod.rs`, `datatype.rs`, `time_unit.rs`) or a root file. Never create a new facade folder.
- **Errors are typed and located, and their messages are contract.** Strings such as `field {:?} row {index}: ... does not read as X` are asserted by tests.
- **Pace:**
  - Each sweep is one script of exact-string edits, and each anchor asserts it matches exactly once.
  - Work in one phase per `src/` subtree.
  - Give each worker a disjoint set of files.
  - Nobody runs `cargo fmt --all` while workers edit.
  - Chain long runs in the background, in one script with one log.
  - Use one commit per coherent change. Each commit is a tree that builds and passes CI, never a single phase on its own.
- **The model fits the step.** Finders and verifiers run one tier below the foreground (`opus` where the foreground is `fable`). Mechanical sweeps run cheaper still. Design decisions stay in the foreground.

## Must not change

- **Public API**, unless the candidate below names the change. The public items most at risk are:
  - `SerieValue`, its derived verbs and `ArrowCastPlan::apply_chunked`
  - `Serie` / `ChunkedSerie` / `WindowSerie` / `WindowSerieMut` methods
  - anything listed in `.api-inventory.txt` or `.api-bindings.txt`

  If a public name has to move, run `cargo check --workspace --all-targets --keep-going --message-format=short` to list binding breakage, then update the inventories, Python, Node, docs and skills in the same change.
- **Wire formats:** IPC, Parquet, Avro, CSV output, serde tags, Arrow extension names and documents, the dictionary-id sidecar.
- **Error variants, texts and locations.** The exception is a deliberate edit to a pinned string in the same change, with the reason.
- **Null policy and exposure gating** (`is_exposed`), the `safe` semantics, and the order and amounts of `MaterializationBudget` charges. A different charge order can change which refusal appears.
- **Zero-copy identities:**
  - `Arc::ptr_eq` on identity casts
  - `reconcile_batch` returning the caller's batch
  - `repeat_scalar`'s `len == 1` shortcut
  - the zero-width `FixedSizeList` row-count fallback
- **Size gates:** `size_of::<Serie>() == 40` (`serie.rs`) and `Scalar` at 48 (`scalar.rs`).
- **Per-row paths stay monomorphized.** Keep `impl Fn(usize)` closures; never box a closure or collect a `Vec` per row.
- **Unsafe stays where it is.** The only unsafe is `local/file.rs`'s one `unsafe`.
- **Pin files.** `rust/tests/allocations.rs` and `rust/tests/iobase_calls.rs` are byte-identical at the end, except a cheaper-direction move with its reason.

## Preconditions

1. **Wait for task #14 to merge.** It changes `serie/arrow.rs`, `serie/order.rs`, `window_serie.rs`, `rust/tests/allocations.rs`, `rust/tests/root/window_serie.rs` and `rust/tests/serie/arrow.rs`. Start from a clean `git status --short` and record HEAD.
2. **Re-anchor every candidate by symbol name** (`grep -n 'fn <name>'`) before acting. The anchors below were taken at `d7c4d93` plus uncommitted edits, and they drift. Never edit by line number.
3. **Check `AGENTS.md` itself for names this change makes stale.** The Layout rows for `cast.rs`, `serie.rs` and `serie/` name helpers, and a consolidation that retires one edits that row in the same change.

## Workflow

1. **Re-anchor and extend (parallel, read-only).** Run one finder per area:
   - (a) `cast.rs` plus the per-type `casts` modules
   - (b) `serie/value.rs` plus `serie/arrow.rs` landing
   - (c) the serie leaves plus `serie/layout.rs`
   - (d) `serie.rs` root dispatch plus `serie/order.rs`
   - (e) `chunked_serie.rs` plus `window_serie.rs`
   - (f) `arrow/`, `budget.rs`, `arrow/size.rs`
   - (g) media and expression call sites

   Each finder re-anchors the candidates in its area and returns new ones in the same shape: anchors (`file:symbol ≈line`), what is duplicated, the consolidation idea, the owner, and the pins that guard it.
2. **Verify adversarially (one verifier per candidate group, never the finder).** For each candidate:
   - **Extract the bodies.** Put every copy into a scratch file, normalize names with `sed`, and `diff -u` them. Compare these dimensions:
     - error variant, text and path
     - the order in which rows are checked (which row is named first)
     - exposure and null gating
     - `safe` handling
     - budget charges (which, how much, in what order)
     - proof certification
     - allocations per row and per batch
     - zero-copy identities
     - check-before-write atomicity
     - which wrappers each copy follows (dictionary, run-end, view)
     - feature gates
   - **Find the tests that pin each copy.** If a copy's refusal is pinned by no test, write that test first and run it while it can still fail ("smoke the refusal first").
   - **Give a verdict:**
     - `IDENTICAL`
     - `EQUIVALENT-WITH-PARAMETER` (name the parameter)
     - `DIVERGENT` (drift: decide which copy is right; it goes in the D list)
     - `DIFFERENT-ON-PURPOSE` (reject; a one-line comment at the site saying why is allowed)
3. **Decide and plan.** The foreground picks the owner and signature, then partitions the caller sweep by file into disjoint worker sets. Follow the phase order under Phases.
4. **Consolidate.** Create the owner first, smoke it, then run the caller sweep script. Re-run the script from the `cargo check --keep-going --message-format=short` list until it is clean. Delete the replaced symbols, then prove they are gone with `grep -rn '<old name>' rust/ python/ node/ docs/ skills/ AGENTS.md`, which must return nothing.
5. **Smoke each phase with the narrowest command that can fail:**
   - build: `cargo check -p yggdryl --all-targets --keep-going --message-format=short`
   - cast tier: `cargo test -p yggdryl --test root cast`, `--test root <type>` (`version`, `timezone`, `decimal`, `temporal`, `string`, `bytes`, `uuid`, `enums`, `geospatial`), and `--test uri`, `--test mime_type`, `--test media_type`
   - serie: `cargo test -p yggdryl --test serie <file>`, `--test root serie`, `--test root chunked_serie`, `--test root window_serie`
   - layout and private pins: add `--features internals`
   - gated call sites: add `--features "parquet iceberg"` for Parquet and Iceberg
   - other call sites: `--test csv`, `--test avro`, `--test expression`, `--test media`, `--test ipc`
   - cost: `cargo test -p yggdryl --test allocations <filter>` and `--test iobase_calls <filter>` for the surface touched
   - hot-path direction: `cargo bench -p yggdryl --bench types -- <filter> --quick` or `--bench arrow` for K1, K4, K5, L1, L4 and L9
6. **Keep the pins unchanged.** At the end of each phase:
   - run `--test allocations` and `--test iobase_calls` whole
   - run `git diff --exit-code rust/tests/allocations.rs rust/tests/iobase_calls.rs`

   A moved count is a design answer: stop and restructure, or record a cheaper move with its reason.
7. **Whole run, once.** Run this chain in the background with one log and a marker per step:
   - `cargo test -p yggdryl --all-targets --all-features --no-fail-fast`
   - `cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings`
   - `cargo clippy --workspace --all-features --all-targets -- -D warnings`
   - `cargo test -p yggdryl --doc`
   - `cargo check --workspace --all-targets`
   - only if a binding-visible name moved: §3 and §4 pre-push, then `python scripts/check_api_inventory.py`
   - if docs changed: `python -m mkdocs build --strict --config-file mkdocs.yml`

   Then run `cargo fmt --all` once, after the last worker has returned. Commit, push, and read CI.

## Phases (value first, conflicts last)

| Phase | Candidates | Files (one worker set each) | Gate |
| --- | --- | --- | --- |
| 0 | D1-D9: red tests and decisions | `rust/tests/**` only | each test red, or documenting current behaviour |
| 1 | K2, K3, K12, K14, then K1, K4, K6, K7 | `cast.rs` (owner), then the per-type `casts` files split across workers | `--test root cast` plus per-type |
| 2 | K5, K9 | `serie/value.rs`, the `casts` modules | `--test serie value`, `--test root string bytes uuid` |
| 3 | K8, K10, K11, N1, N3, N4 | `cast.rs` `columns`, `budget.rs`, `arrow/`, `serie/arrow.rs` | `--test serie arrow`, `--test root cast budget` |
| 4 | L1-L12 | serie leaves, `serie/layout.rs`, `serie/order.rs` (after #14) | `--test serie`, `--features internals` |
| 5 | L13-L18, L20 | `chunked_serie.rs`, `window_serie.rs`, `serie.rs` inherent verbs | `--test root serie chunked_serie window_serie` |
| 6 | N2, N5-N12 | media, expression, `ipc/`, `csv/`, `avro/` | per-medium harness |
| 7 | L19 variant table | `serie.rs` alone | everything |

## Candidates

Value and risk are given in brackets. Every candidate is verified before it is acted on.

### D: defects found while scouting. Write the red test first, then fix deliberately.

- **D1 [HIGH, correctness] The Decimal256 / BigDecimal comparator orders by decimal text.**
  - **What it does:** `cast.rs` `make_yggdryl_comparator` (≈5279) compares `DecimalText::as_bytes()` lexicographically. `DecimalText` (`decimal.rs` ≈116-165) is variable-length with a `-` prefix, so `9 > 10` and `-5 > -3`.
  - **What it should match:** `Scalar` orders decimals numerically (`scalar.rs` `Ord`).
  - **Who reads the order:** the `keys_sorted` check in map-key validation (`cast.rs` ≈5080, `compare(index, index + 1) == Greater`), the large-row duplicate check (≈5071), and the dictionary compaction ordering (≈3810, ≈4431). Equality is unaffected.
  - **Red test:** a `sorted_map<decimal256(..), _>` with keys `[9, 10]` and `[-5, -3]` lands. Today it is refused, or an unsorted map is accepted.
  - **Fix:** compare `i256` values directly. While there, check the Float16/32/64 arms against `Scalar`'s float order. Folds into L11 / K8 later.
- **D2 [HIGH, cost or safety] Proof inconsistency between same-algorithm ingests.**
  - **What it does:** `ArrayCastPlan::proof` (`cast.rs` ≈1381) certifies `UrlIngest` and `UrnIngest` as `Proven`. It does not certify `VersionIngest`, `TimezoneIngest`, `MimeTypeIngest` or `MediaTypeIngest`, which run the same algorithm (K1) and write the same canonical text. It also does not certify `DecimalFromFloat` or `GeospatialIngest`.
  - **Cost:** those leaves are not `DataType::layout_is_contract` (`datatype.rs`), so landing re-reads every row (`serie/arrow.rs` `reads_rows` / `prove_rows`). Each cell is parsed twice.
  - **Decide:** if the canonical text an ingest writes always passes `field.scalar`, all six are `Proven`. That is a cheaper move; pin it with an `allocations` or parse-count row. If it does not, then Url and Urn are wrongly `Proven`, which is a safety defect.
  - **Order:** settle this before K1.
- **D3 [MED] `ChunkedSerie::cast` re-implements `ArrowCastPlan::apply_chunked`.**
  - **What differs:** `ChunkedSerie::cast` (`chunked_serie.rs` ≈636) re-implements `apply_chunked` (`cast.rs` ≈699) without `require_layout`. Its identity check is `self.field == target` rather than `plan.identity`, and its field comes from `field_of(&chunks, ..)` instead of `plan.target`.
  - **Decide:** which field is right.
  - **Fix:** route through `apply_chunked`. Do the same for `ChunkedSerie::from_series` (≈211) and the `Held` arm of `SerieReader::cast` (`serie/arrow.rs` ≈1628+).
- **D4 [LOW] `DictionarySerie::slice` skips `require_window`.** In `serie/enums.rs` ≈307, the other 14 leaves call it, so this leaf's out-of-range refusal names a different column (the keys).
- **D5 [MED] Two integer-key width tables disagree.** `key_reach` (`chunked_serie.rs` ≈1530) and `key_capacity` (`serie/enums.rs` ≈66) are reportedly a power of two apart. Find out which is right and keep one table.
- **D6 [LOW] Offset conversion differs between map and list.** `MapSerie::range` uses `usize::try_from` (`mapping.rs` ≈113) where `OffsetSerie` uses `as_usize` (`sequence.rs` ≈203). Compare behaviour on corrupt or negative offsets before L5 merges them.
- **D7 [LOW, layout rule] `decimal.rs` names `serie/value.rs`.** `decimal.rs` ≈113 (`ingest_float_values`) calls `crate::serie::value::array_of_rows`, which breaks the "named by nothing outside serie/, cast.rs, temporal.rs" rule. Fixed by K6 or K9.
- **D8 [MED] `safe` is hard-coded on some Utf8 temporaries.** The canonical-text ingests (`version.rs`, `timezone.rs`, `mime_type/datatype.rs`, `media_type/datatype.rs`, `uri/datatype.rs`) and `temporal.rs` ≈139 pass `safe = true` into `arrow_cast_exposed` for the Utf8 temporary. `string.rs` (≈369, ≈549), `uuid.rs` ≈82, `enums.rs` ≈597 and `cast.rs` ≈1126 pass the caller's `safe`. Decide whether this is deliberate. It becomes a parameter of K4 either way.
- **D9 [LOW, cost] `Run` copies its window even when uniquely held.** `Run::make_mut` (`serie/datatype.rs` ≈356) and `splice_run` (`serie.rs` ≈2284) do the copy. Note that `bytes.rs` ≈275 is a second function also named `splice_run`; rename one.

### K: the cast engine and value crossing (castings first)

- **K2 [LOW risk, opener] One encoded-value peel.** These four are byte-identical (verified):
  - `cast.rs` `encoded_value_of` (≈883)
  - `csv/writer.rs` `encoded_dtype` (≈266)
  - `merge.rs` `decoded` (≈269)
  - `expression/typing.rs` `unwrap_dictionary` (≈403)

  Owner: one crate-private `DataType` method in `datatype.rs`, for example `encoded_value(&self) -> &DataType`. Delete all four.
- **K3 [LOW] Text-layout predicates.** `cast.rs` `holds_text` (≈892) follows dictionary and run-end. `version.rs` `casts::is_text_layout` (≈78) is bare. `text/arrow.rs` `is_string_layout` (≈1441) follows dictionary only. Check each caller's intended coverage, then keep one predicate, or two named by coverage. `string.rs` `is_text_storage` and `Serie::is_string_storage` answer different questions (leaf, landed variant); leave them.
- **K1 [HIGH value, ~250 LOC] One canonical-text ingest.**
  - **Existing generic:** `uri/datatype.rs` already has the private `ingest_canonical_text(array, field, exposure, budget, name, parse)` (≈69), with `ingest_url_array` and `ingest_urn_array` as wrappers.
  - **Copies to replace:** `ingest_version_array` (`version.rs` ≈35), `ingest_timezone_array` (`timezone.rs` ≈74), `ingest_mime_type_array` (`mime_type/datatype.rs` ≈46) and `ingest_media_type_array` (`media_type/datatype.rs` ≈46) are copies of it. They differ only in the parser and the noun in `does not read as X`.
  - **Move the generic** into `cast.rs` `text`, and delete the four copies.
  - **Collapse `ArrayCastKind`.** Replace `VersionIngest | UrlIngest | UrnIngest | TimezoneIngest | MimeTypeIngest | MediaTypeIngest` with one `CanonicalTextIngest(<parser>)`. Merge the six `(DataType::X, source) if is_text_layout(source)` / `DeferredUnsupported` arm pairs in `nested_kind` (≈1884-1906) into one guard, and merge the dispatch arms in `cast_exposed` (≈2479-2506) into one.
  - **Proof:** decide it once, per D2.
  - **Keep messages byte-identical.**
- **K4 [HIGH, ~150 LOC] One source triage and one ingest context.**
  - **The "Utf8 temporary" preamble** (`if Utf8 { Arc::clone } else { arrow_cast_exposed(.., &Utf8, safe, exposure, &Field::new(name, DataType::utf8(), true), budget) }`) appears 11 times: `version.rs`, `timezone.rs`, `mime_type/datatype.rs`, `media_type/datatype.rs`, `uri/datatype.rs`, `enums.rs` ≈597, `string.rs` ≈369 and ≈549, `cast.rs` ≈1126 (`ingest_text_values`), `uuid.rs` ≈82, and `temporal.rs` ≈139 (guarded by `can_cast_types`).
  - **The three-way triage** (FixedSizeBinary gives slot cells; Binary, LargeBinary and BinaryView go through `variable_binary_source` and give byte cells; anything else goes through the Utf8 temporary and gives text cells) is repeated in `ingest_string_array` (`string.rs` ≈313-383), `ingest_code_array` (`string.rs` ≈512-575), `ingest_uuid_array` (`uuid.rs` ≈43-85) and `ingest_bytes_array` (`bytes.rs` ≈252-300).
  - **Owner:** one `CellSource::{Slot, Bytes, Text}::of(array, safe, field, exposure, budget)` in `cast.rs` `text`. Bundle the threaded `(field, safe, exposure, budget)` into an `Ingest<'_>` whose methods are `cell(i)` (wrapping `is_exposed` (≈5820) and `is_valid`) and `refuse(i, raw, noun, err)`. Cell readers stay `impl Fn`.
- **K6 [MED] One read-through-Scalar tail.**
  - **The copies:** `cast.rs` `ingest_text_values` (≈1107-1200) and `temporal.rs` `render_temporal_text` (≈92-160) both do the same thing: read rows to `Scalar`, call `array_of_rows`, `zip(mask, ours, arrow_fallback)`, then re-scan strictly to name the first null the reading produced.
  - **`decimal.rs` `ingest_float_values`** (≈63-115) is the same without the fallback.
  - **Owner:** factor the tail once in `cast.rs` `text`. `decimal.rs` then calls it and stops naming `serie/value.rs` (D7).
  - **Before replacing anything,** add an `allocations` or bench row for LargeUtf8 / Utf8View targets. `render_temporal_text` writes Utf8 only and relies on the shared tail re-cast (`cast_exposed` tail ≈2685).
- **K7 [HIGH, ~400 LOC] Integer-width dispatch owned once.**
  - **Dictionary key** dispatch over 8 widths: `cast.rs` ≈970, ≈3564 (`align!`), ≈4364 (`fill!`), ≈4851 (`build!`), ≈5606 (`rebuild!`), ≈5893-5915 (`logical_null_at`); `serie/value.rs` ≈1522, ≈1553, ≈1700, ≈1719; `bytes.rs` ≈426-455; `budget.rs` ≈710, ≈2161-2177; `serie/enums.rs` ≈67-143, ≈216.
  - **Run-end** dispatch over 3 widths: `cast.rs` ≈980, ≈3473-3526, ≈5557, ≈5735, ≈5917, ≈5982; `budget.rs` ≈1731, ≈1880, ≈2082, ≈2216; `serie/value.rs` ≈1636, ≈1729; `serie/arrow.rs` `gather_runs`; `serie/runend.rs` `over_run_ends!` (≈54-76).
  - **Code-width** dispatch: `cast.rs` ≈2508-2562, `serie/value.rs` ≈155-166, `code.rs` ≈259-271. **Enum-leaf** dispatch: `cast.rs` ≈2433-2470.
  - **Duplicated error strings:** "expected an integer dictionary key datatype" appears 7 times and "run-end type is not a supported signed integer" 8 times (both counted at `d7c4d93`).
  - **Owner:** `with_dictionary_key!`, `with_run_end!`, `with_code_width!` and `with_enum_leaf!` in `arrow/mod.rs` (Arrow-side) or `datatype.rs` (crate-side), each with one error constructor.
- **K5 [HIGH, ~300 LOC, higher risk] One string/byte column builder owned by the leaf.**
  - **The Scalar-row builders** in `serie/value.rs`: `string_array` (≈1979, with fixed-slot padding), `utf8_array` (≈2063), `binary_from_parts` (≈2095), `binary_view_from_parts` (≈2116), `bytes_array` (≈1887), `uuid_array` (≈1931), `code_array` (≈1961).
  - **The cast-side builders:** `string_storage` (`string.rs` ≈405-492), `code_text_array`, `bytes_storage`, `uuid_storage`, `enum_storage`, and the three-layout match in `geospatial.rs` `render_wkt_array` (≈63-127).
  - **What is duplicated:** each side builds the same seven layouts (Utf8 / LargeUtf8 / Utf8View, Binary / LargeBinary / BinaryView, FixedSizeBinary slot), with payload pre-measure, validity and view build.
  - **Owner:** one builder per family on `StringType` / `BytesType`, fed by a cell iterator. After this, `array_of_rows` and the ingests differ only in how a cell is read.
- **K9 [MED] Retire `value_from_array` as a second Arrow-cell reader.**
  - **The second reader:** `serie/value.rs` `value_from_array` (≈746-960) duplicates the leaf `Reading` fn pointers that are resolved once at landing (≈393-730).
  - **Three datatype-to-reading tables:** `value_from_array` (≈765-900), `array_of_rows` (≈52-330) and `serie/primitive.rs` `column_of` (≈595-680).
  - **Outside callers:** `temporal.rs` ≈108 (per row inside `render_temporal_text`) and `serie/arrow.rs` `refused_cell` (≈1051).
  - **Fix:** land the column once, read `scalar(i)`, then delete `value_from_array`. Keep `array_of_rows` as the only Scalar-to-Arrow codec.
- **K8 [HIGH, ~600 LOC, highest K risk] A crate-private `ListRows` view over raw Arrow list layouts.**
  - **The view:** it covers List, LargeList, ListView, LargeListView, FixedSizeList, and Map as entries, with `values`, `len`, `is_valid`, `row_range`, `nulls` and `with_values`. It mirrors `OffsetSerie<O>` / `OffsetViewSerie<O>`.
  - **The five per-arm fan-outs it collapses:**
    - `cast.rs`: `cast_list_array` (≈4006-4172), `align_nested_dictionaries` (≈3180-3400), `array_children_unchanged` (≈4609-4650), the list arms of `make_yggdryl_comparator` (≈5290-5420), `source_list_kind` (≈2949), `nested_kind` (≈2126-2145)
    - `budget.rs` (≈1245-1317, ≈1656, ≈1785, ≈2012, ≈2134)
    - `serie/value.rs` (≈1199-1295)
    - `serie/arrow.rs` `refused_cell`
    - `expression/arrow.rs` `Offsets` (≈1136-1206)
    - `arrow/size.rs` `sliced_size`
  - **Child-range selection by parent offsets** is also unified. It exists 4 times: `cast.rs` `range_exposure` (≈6098) with `offset_pair` and `selected_index_exposure`; `budget.rs` `selected_child_ranges` / `extended_child_ranges` / `source_child_ranges` (≈863-970); `serie/arrow.rs` `gather_ranges` / `rebased` / `compact_offsets` / `offset_parent`; and `expression/arrow.rs` `Offsets::row`.
  - **Execution:** land in sub-steps, one consumer at a time.
- **K10 [MED] One batch reconciler.**
  - **The repeated sequence:** `plans.get_or_compile(batch.schema_ref().fields(), || ArrowCastPlan::compile_schema(..))?.reconcile_batch(batch)` appears at `parquet/mod.rs` ≈629, `avro/batch.rs` ≈398, `media/merge.rs` ≈125, `iceberg/scan.rs` ≈803 and `iceberg/table.rs` ≈4097.
  - **Owner:** a `BatchReconciler { root, options, cache }` in `cast.rs`. `media/partition.rs` ≈1107, `expression/transform.rs` ≈409, `expression/arrow.rs` `ColumnCast` (≈410-455, a `OnceLock` plus a `Mutex<PlanCache>` drift slot) and `iobase/transfer.rs` ≈519 hold it where their key shape allows.
  - **Hand-rolled one-slot caches:** `ChunkedSerie::from_series` (≈193-213) and `from_arrow_arrays` (≈256-302) each have one. Generalise `PlanCache`'s key (Fields, one Field, or one Arrow datatype) and use it in both.
  - **Guard:** `allocations.rs` "`from_series` plan reuse" (≈4766).
- **K11 [MED] One column intake and fewer landing doors.**
  - **The ladder** "`lands_exactly` → `land(Proof::Unproven)`, else compile, then `cast_array`" is written in `Serie::from_arrow_array`, `Serie::from_arrow_batch`, `ChunkedSerie::from_arrow_arrays`, `ChunkedSerie::from_series` and `SerieReader::from_arrow_reader`.
  - **Eight landing doors** in `serie/arrow.rs`: `column_of`, `child_of`, `land`, `land_resolved`, `land_planned`, `land_under`, `land_batch`, `land_once`. They differ only in parent nulls, resolved boxes, projection re-proof and error location.
  - **Owner:** one `ColumnIntake` plus one `Landing { .. }` parameter struct. Coordinate with L20.
- **K12 [LOW-MED] One cell-refusal constructor.**
  - **The message** `field {:?} row {index}: {raw:?} does not read as X: {error}` is written at `timezone.rs` ≈104, `mime_type/datatype.rs` ≈76, `media_type/datatype.rs` ≈76, `version.rs` ≈65, `uri/datatype.rs` ≈101, `decimal.rs` ≈107 and `cast.rs` ≈1194, beside `cast.rs` `named_cell` (≈2938).
  - **Located re-rooting:** `csv/reader.rs` `refused` (≈607) and `csv/writer.rs` `at_cell` (≈403) share an identical core (verified). The reader appends `, reading {text} in row {line} of {location}`. Related shapes are at `serie/arrow.rs` ≈1051, `value/canonical.rs` ≈1685, `graph/arrow.rs` ≈3221 and `graph/serve.rs` ≈1437.
  - **Owner:** one constructor on `Error` or `crate::path`. Keep every message byte-identical.
- **K13 [DESIGN, report only] A second value-level conversion engine.**
  - **Where:** `expression/eval.rs` `convert` (≈1130-1360) handles decimal, temporal, bool / float / int, uuid, and text / bytes / enum / code at the Scalar tier, beside the column ingests. It shares only leaf readers with them: `Scalar::from_temporal_text`, `from_decimal_float`, `uuid_parse`, `is_blank_text`.
  - **Find out:** whether one per-leaf "text to Scalar" reading can serve `convert`, `ingest_text_values`, `ingest_float_values` and `csv/reader.rs` `read_cell`. Merge nothing unless the null, blank and `safe` semantics are identical.
- **K14 [MED] `TimeUnit` owns count restatement.**
  - **Eight copies with different rounding:** `temporal.rs` `per_second` (≈440), `nanoseconds_per` (≈1493), `temporal_count_at` (≈1411), `restated`, `unit_rank`; `text/arrow.rs` `rescale` / `nanos` (≈1175); `avro/datum.rs` `convert_count` (≈1006); `fix/entry.rs` `nanos_per` (≈240, which adds Day); `txhash/time.rs` `restate_unix` (≈79, which floors and is public).
  - **Owner:** `time_unit.rs` gets `nanoseconds()` and one `restate(count, to, Rounding)`.
  - **Constraint:** `txhash::restate_unix` is public and listed in the inventory, so it keeps its signature and becomes a thin caller.

### L: serie leaves and verbs (after #14)

- **L1 [HIGH, low risk] Provide `splice` once.** These `SerieValue::splice` bodies are byte-identical (verified): `require_range`, then `field.scalar` per row, `check`, `write`. They are at `boolean.rs` ≈264, `bytes.rs` ≈442 / ≈650 / ≈903, `null.rs` ≈109, `primitive.rs` ≈536, `runend.rs` ≈449, `sequence.rs` ≈334 / ≈625 / ≈882, `structure.rs` ≈358, `union.rs` ≈409, `variant.rs` ≈230 and `mapping.rs` ≈248. `enums.rs` ≈315 is a variant: intern, `require_fit`, `apply`. Make `splice` a provided method over a crate-private `check` / `write` / `append`, using a sealed supertrait or hidden methods. `check` defaults to `Ok(())` for boolean, null and primitive. `check` must stay free of side effects, and `check` runs before `write`.
- **L2 [MED] Leaf accessor boilerplate.** Every one of the 15 impls repeats `field` / `field_ref` / `len` / `null_count` / `is_null`, the `require_window` prologue of `slice`, `into_arrow_array`, and a hand-written `Debug` / `Clone`. The bounds-plus-validity read `(index < len && is_valid(index)).then(..)` also repeats (`primitive.rs` ≈162, `boolean.rs` ≈42, `bytes.rs` ≈353 / ≈565 / ≈771). Generate them with one `backed_leaf!` beside `serie_leaf!` (`serie.rs` ≈149).
- **L3 [MED] Typed writers.** `require_present` is the same in `boolean.rs` ≈58-73 and `primitive.rs` ≈410-425. `push_value` / `set_value` / `splice_values` / `extend_values` match between `boolean.rs` ≈81-112 and `primitive.rs` ≈433-491. Make one helper.
- **L4 [MED, cheaper pin] A `Validity` helper.**
  - **What repeats:** `present: Vec<bool>` is built per write in 10 places (`boolean.rs`, `bytes.rs` ≈813, `mapping.rs` ≈155 / ≈183, `sequence.rs` ≈120 / ≈251 / ≈273 / ≈542 / ≈563 / ≈816 / ≈826, `structure.rs` ≈267 / ≈299, `variant.rs` ≈131 / ≈153). Alongside it: `nulls.as_ref().is_some_and(|n| n.is_null(i))` 12 times, `map_or(0, NullBuffer::null_count)` 6 times, and a nulls slice in every nested `slice`.
  - **Owner:** `serie/layout.rs` `Validity`, using `BooleanBufferBuilder::append_buffer` on append.
  - **Pins:** the serie-verb pins (`allocations.rs` ≈2468 and ≈2684) may move cheaper, with the reason.
- **L5 [MED] `MapSerie` is an `OffsetSerie<i32>` skin.**
  - **What repeats** between `mapping.rs` and `sequence.rs`: range / row / cut (≈113-136 against ≈203-226), check / write (≈140-162 against ≈230-252), append (≈168-188 against ≈258-280), and slice (≈231-246 against ≈317-332). `scalar` and `range` + `row` also repeat across the view and fixed leaves.
  - **Owner:** an `OffsetCut<O>`, shared by the four leaves. Resolve D6 first.
  - **List `column_of`:** `sequence.rs` ≈1096-1118 (List) and ≈1119-1141 (LargeList) differ only in the offset type, and `mapping.rs` ≈340-359 runs the same pipeline. Make one generic.
- **L6 [LOW] Reuse `splice_scalars`.** `sequence.rs` `extended` (≈134-146) and `recut` (≈488-517), and `bytes.rs` `FixedSerie::write_array` + `rebuilt_payload` (≈810-859), re-implement `layout::splice_scalars`. Today `union.rs` is its only caller.
- **L7 [LOW-MED] Bitmap own-or-copy.**
  - **Copies:** `layout.rs` has three (`splice_nulls` ≈46, `splice_bits` ≈88, `owned_bits` ≈118). Bit-range reverse is written twice: `layout.rs` `reverse_bits` (≈134) and `primitive.rs` ≈386-402.
  - **Precondition:** first add a test that slices a validity bitmap at an offset that is not a multiple of 8. It proves the `.sliced().into_mutable()` and offset-0 rules are equivalent.
- **L8 [LOW] One `layout::rebase_offsets`.** The offset rebase `*held - first` is written 6 times: `serie/arrow.rs` `rebased`, `sequence.rs` ≈321 / ≈612 / `rebased_views` ≈977, `mapping.rs` ≈235 and `runend.rs` ≈261.
- **L9 [LOW, cheaper pin] One `laid_out::<A>(field, rows)`.** The pattern `rows.iter().collect::<Vec<&Scalar>>()` + `array_of_rows` + downcast repeats in `primitive.rs` ≈237, `bytes.rs` `lay_out` (≈149), `union.rs` ≈205, `variant.rs` ≈128 and `serie/arrow.rs` `from_scalars`. Taking `impl Iterator<Item = &Scalar>` removes a `Vec` per write. Also make one `LAID_OUT` message.
- **L10 [LOW] One placeholder-tile helper.** "Build the placeholder once, only when a row is absent" is written in `sequence.rs` `tiles_of` (≈783), `structure.rs` `cells_for` (≈236) and `union.rs` `cells_for` (≈149).
- **L11 [MED] The compare ladder in `serie/order.rs`.**
  - **The absent-against-present triple** is written 3 times (≈437, ≈318, ≈1452). **The record / sequence walk** is written 3 times: `compare_to_row`, `Compare::Record`, and the sequence arm of `compare_values`. **The sortedness test** is written 3 times (`is_sorted` ≈784, `partition_by` ≈1145, `window_starts` ≈1415), plus the chunk edges (`chunked_serie.rs` ≈707).
  - **`ordered_buffers`** (≈733) is rebuilt on every call: ≈280, ≈717, twice in `compare_across` ≈753, ≈780, ≈805, ≈1181.
  - **Fix:** build `ordered_buffers` once per verb and make one `compare_absence`. The D1 fix belongs here.
- **L12 [MED] Kernel prelude.** `taken` (≈997), `filtered` (≈1081), `memory_size` (≈1514), `ordered_buffers` and `Serie::repeat` (`serie/arrow.rs` ≈1281) all repeat `field_ref().expect` + `into_arrow_array()` + kernel + `land(.., Proof::Proven)`. The zero-width `FixedSizeList` fallback is duplicated (`order.rs` ≈1016-1025 and `serie/arrow.rs` ≈1306). Make one `take_rows(&[u32])` that owns the fallback, so `repeat` becomes a call to it.
- **L13 [MED, public] Derived write verbs.** `set` / `push` / `insert` / `remove` / `pop` / `truncate` / `clear` / `extend` / `resize` exist as provided methods on `SerieValue` (`value/mod.rs` ≈458-557) and again on `Serie` (`serie.rs` ≈2771-2856, ≈3058). Add `Serie::splice_canonical` (`check` + `write` without re-proof) for `resize` and `WindowSerieMut::fill`. Moving the public verbs means editing the inventory and checking the bindings.
- **L14 [MED] Verb mirrors.**
  - `WindowSerieMut` re-exposes every read as `self.as_window().X()`. `WindowSerie`'s ordering verbs are `self.into_serie().X()`.
  - `ChunkedSerie`'s join verbs call `into_serie()?.X()` and wrap the result back as one chunk.
  - The `as_*` state writes all have the shape `*self = self.into_X(..)?; Ok(self)` (`order.rs` ≈1619-1698, `chunked_serie.rs` ≈1301-1393, `window_serie.rs`).
  - **Fix:** one forwarding macro over an `into_serie` / `as_window` hook, plus `as_state!`, plus a `write_back(range, serie)` helper.
- **L15 [LOW] Identity blocks.**
  - **Five copies over `Rows`:** `serie_leaf!`, `Serie` (≈3142), `WindowSerie`, `ChunkedSerie` (≈1562) and `Run` (`serie/datatype.rs` ≈382).
  - **Four `Display` implementations** and **three double-ended row walkers** (`WindowSerieRows`, `ColumnRows`, `ChunkedRows`).
  - **Fix:** one `rows_identity!` with `display_rows<R: Rows>`. `ColumnRows` takes a range.
- **L16 [LOW] Running starts.** Chunk walks re-accumulate what `ends` already stores (`chunked_serie.rs` ≈105, ≈495, ≈941, ≈985, ≈1116). Add one `spans()` over `ends`.
- **L17 [LOW] Refusals and value sets.**
  - About 40 sites build `InvalidRecord { path: SmolStr::new(name), reason: format_smolstr!(..) }` by hand.
  - Mirrored refusals: mask length (`order.rs` ≈1052 against `chunked_serie.rs` ≈930), key length, "never grows or shrinks", and "absent rows which a table cannot state". The last is X1's.
  - The `Scalar` hash set with `#[allow(clippy::mutable_key_type)]` appears 6 times.
  - **Fix:** `Error::invalid_record(name, reason)`, `require_same_len`, and one value-set helper.
- **L18 [DESIGN, report only] Four window-cut engines.**
  - **The engines:** `Serie::window_starts` + `Compare::regroup` + `SerieWindows::new`; `ChunkedSerie::window_by` (≈1102); the streaming `Walk` (`serie/arrow.rs` ≈2282+); and `WindowSerie::window_by`.
  - **The question:** the sorted regroup comparator differs (the buffer `Compare` against `compare_values`). Is that intended for cost, or drift?
  - **Constraint:** a shared `RunCursor` is acceptable only if the cost pins at `allocations.rs` ≈3207, ≈3338, ≈3807-3875 and ≈4493-4509 hold.
- **L19 [HIGH value, HIGHEST conflict risk, last and alone] The `serie.rs` variant table.**
  - **What it replaces:** `column!` (≈618), `column_mut!` (≈714-1052, a 340-line copy that exists only to add `Arc::make_mut`), 56 `impl Leaf` blocks (≈1069-2270), 112 `as_*` / `get_*_mut` accessors (≈3178-3962), the paired `append` match (≈2890-3046), `value_bytes`, and `order.rs` `primitive!` / `primitive_mut!`.
  - **The table:** one declarative table, Variant → Leaf, with named groups: primitive, integer key, run-end width, text storage, byte storage. It generates all of the above and feeds `primitive_leaf!`, `byte_leaf!`, `view_leaf!`, `fixed_leaf!` and `offset_leaf!`.
  - **Must hold:** the 40-byte gate.
- **L20 [MED] Trial dispatch in `child_of`.** `child_of` (`serie/arrow.rs` ≈750-784) tries 11 `column_of` functions in order. Each re-matches `array.data_type()` and has the same six-argument signature with a `let _ = (parent, proof);` prologue. Replace it with one match and a `Landing` struct (pairs with K11).

### N: neighbours

- **N1 [LOW-MED] Batch with explicit row count.** `RecordBatch::try_new_with_options(.., with_row_count(..))` is hand-written at `graph/arrow.rs` ≈831, `xxhash/arrow.rs` ≈250, `expression/selector.rs` ≈1677 and ≈1798, `expression/arrow.rs` ≈969, `serie/arrow.rs` ≈1148, `parquet/mod.rs` ≈1818, `cast.rs` ≈806, `arrow/rows.rs` ≈372 and `avro/batch.rs` ≈1012. Extend `arrow::rebuilt_batch` (`arrow/mod.rs` ≈954) into `batch_of(schema, columns, rows)`. Route the raw `StructArray::from(batch)` calls (`expression/plan.rs` ≈1753, `selector.rs` ≈1805, `expression/arrow.rs` ≈518 and ≈544) through `cast::struct_array_from_batch` (≈1246), which keeps the row count of a zero-column batch.
- **N2 [MED, coordinate with X3] Repeat one row.**
  - **Four spellings:** `expression/arrow.rs` ≈85 (`vec![0; rows]`), `Serie::repeat`, `media/partition.rs` ≈204 and `cast.rs` `repeat_scalar` (≈4895).
  - **Constant columns** are restored twice: `media/partition.rs` `Constants::restore` (≈230-250) and `iceberg/scan.rs` `restore_partitions` (≈1281-1310).
  - **Placeholder arrays:** the `physical_placeholder_for_field` + `array_of_rows` pattern at `cast.rs` ≈191, ≈4791 and ≈4809 becomes one `Field`-level helper.
- **N3 [LOW] `downcast` helpers.** There are four, with three different messages: `cast.rs` ≈3007, `serie/value.rs` ≈1747, `xxhash/arrow.rs` ≈1230 and `serie/arrow.rs` `held` / `held_ref`. Keep one in `arrow/mod.rs`.
- **N4 [LOW] `with_nulls` / `with_data_type`.** The `to_data().into_builder().X().build()` rebuilds are at `serie/arrow.rs` ≈745, `expression/arrow.rs` ≈1125, `serie/value.rs` ≈1648, `cast.rs` ≈5775 and ≈4588, and `iceberg/table.rs` ≈3620. The parent-null union is written twice: `serie/arrow.rs` `parent_nulls` and `expression/arrow.rs` ≈787 / ≈1117. Put the helpers in `serie/layout.rs`.
- **N5 [LOW-MED] Byte and text cell reads per layout.**
  - **Copies:** `bytes.rs` `byte_cell_len`, `budget.rs` `text_cell_len`, `serie/value.rs` `bytes_cell` / `string_value`, and `string.rs` `code_cell` / `code.rs` `code_cell_text` / `uuid.rs` `uuid_cell`.
  - **Fix:** add one Arrow-level `cell_bytes(array, i)`. Make `Serie::value_bytes` follow one dictionary level, so `text/arrow.rs` `Bodies::Encoded` (≈1464-1499) and `csv/writer.rs` `CellWriter::of` (≈171-206) drop their own dictionary reads.
- **N6 [LOW-MED] DataType tree walkers.** `contains_dictionary` (`cast.rs` ≈3067), `contains_struct` (≈5781), `requires_yggdryl_key_comparator` (≈5095), `keeps_empty_text` (≈912), `holds_temporal` (`temporal.rs` ≈166), `holds_byte_leaf` (`text/typed.rs` ≈268), `stored_order_is_value_order` (`serie/order.rs` ≈170), `layout_holds_float`, `lands_as_is` (`serie/arrow.rs` ≈105), `gather_layout` (≈320) and `layout_is_contract` each re-walk the tree. Add one `DataType::any_node` / `all_leaves` so each predicate is reduced to its own leaf list.
- **N7 [LOW] Key builders.** `RowConverter` key builders are written at `media/merge.rs` ≈88, `iceberg/table.rs` ≈4169 and `serie/order.rs` ≈91. The window-by key binding is written three ways: `order.rs` `window_key`, `chunked_serie.rs` ≈1107 (inline) and `serie/arrow.rs` ≈1975.
- **N8 [LOW-MED] One `take_batch`.** Raw `take_record_batch` / `take` runs at `media/partition.rs` ≈664, `media/merge.rs` ≈222 and ≈305, `iceberg/table.rs` ≈2956 and ≈3219, `expression/plan.rs` ≈1688 and ≈1803, and `selector.rs` ≈1667. Make one `take_batch` with the dense-union null-index fix from `expression/arrow.rs` `scattered` / `takes_a_dense_union` (≈558-580) built in. Decide per site whether the raw shape stays: AGENTS says other vocabularies keep `RecordBatch`.
- **N9 [LOW] CSV inference.** `csv/reader.rs` `reads_empty_as_null` (≈660) re-derives `cast::text::keeps_empty_text` by probing. `LADDER` / `Candidates` (≈664-725) and `text/arrow.rs` `parse_capture` (≈1104) are two text-to-typed inference ladders.
- **N10 [LOW, report] Avro narrowing.** `avro/batch.rs` `ColumnWriter` (≈1828-2173) narrows with `as_primitive_opt` instead of the `Serie` leaves.
- **N11 [LOW, report] Memory estimators.** There are three: `arrow/size.rs`, `budget.rs` (predictive) and `Serie::memory_size`. Document which one a byte threshold reads. The spill spec needs this answer.
- **N12 [LOW] IPC prologues.** `ipc/mod.rs` `read_batch_reader` (≈489) and `read_owned_batch_reader` (≈512) share a prologue, as do `read_schema` (≈201) and `read_metadata` (≈224).

## Done means

- Every candidate (D, K, L, N, and anything the finders added) has a recorded verdict: landed, rejected with the reason, or deferred to a named spec.
- Every landed consolidation deleted what it replaced. `grep` shows no old name in `rust/`, `python/`, `node/`, `docs/`, `skills/` or `AGENTS.md`.
- Every D item has its test. The test was red before the fix and is green after, or it documents a deliberate behaviour with the decision written in its rustdoc.
- `git diff --exit-code rust/tests/allocations.rs rust/tests/iobase_calls.rs` is clean, or each moved count is cheaper and carries its reason. No benchmark moved slower (`--quick` direction on the hot paths named above).
- Pinned error strings are unchanged, or edited deliberately with the reason.
- Every `Proof::Proven` site touched has been reviewed and listed in the report.
- The whole run passes in the `--all-features` lane with `--no-fail-fast`, and clippy passes at `-D warnings` in both lanes. Doc tests and `cargo check --workspace --all-targets` pass.
- Inventories, docs and skills are updated where a public name moved, and `python scripts/check_api_inventory.py` is clean.
- The change is one commit per coherent change. It is pushed, and its CI run has been read and is green.

## Report

Use the AGENTS handoff keys: `Goal`, `Invariants`, `State`, `Checks` (each command with its exact result), `Blockers`, `Next` (the exact command).

Add a candidate ledger: `ID | verdict | owner | deleted symbols | LOC delta | pins moved (why)`.

List the skipped checks by name, and the D items deferred with the decision each one needs.