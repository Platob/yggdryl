# yggdryl-excel move map (S6c) - read from /home/user/yggdryl-s3 @ wip/s3 0a24f1806 (uncompiled, read only)

Method: refs.py on the s3 tree (356 `crate::` refs in `excel/`: 219 pub, 131 pub(crate), 6 pub(super)) plus a `.method(` pass against every pub(crate) impl fn (hits read by line; local HashMap/Vec/own-type false hits dropped). wip/s3 has no `.github/ci/`; rows.toml cited from /home/user/yggdryl (main).

## 1. File set that moves
| what | files (lines) |
|---|---|
| src `rust/src/excel/` -> `rust/excel/src/` | cell 1356, workbook 1302, sheet 1017, media 687, package 616, parser 470, styles 459, writer 457, reader 548, shared_strings 269, options 197, mod 117 = 7495. No cfg gates, no `internals` modules. quick-xml used directly (package.rs:22,356,399,444; parser.rs:15-16,258; shared_strings.rs:22; styles.rs:18) |
| tests harness | rust/tests/excel.rs 36 (12 `#[path]` mods + `#[path="support/excel_package.rs"]` at :10) |
| tests `rust/tests/excel/` | cell 991, media 1020, mod_ 141, options 855, package 833, parser 604, reader 925, shared_strings 624, sheet 1166, styles 614, workbook 1072, writer 1048 = 9893. No `internals` use |
| shared fixture | rust/tests/support/excel_package.rs 208 - used ONLY by the excel harness (10 test files) -> moves with it |
| interop | rust/tests/interop/excel.rs 301 (+ `mod excel;` at rust/tests/interop.rs:7-8). Tests: `writes_a_workbook_for_the_external_reader` :107, `reads_the_workbook_the_external_writer_produced` :195 (SKIPPED :199), `reads_the_1904_workbook_the_external_writer_produced` :275 (SKIPPED :279). `exchange_dir()` :29-35 = `current_dir()/target/excel-interop` |
| benches | rust/benchmarks/media/excel.rs 155 (`use crate::bench_profile::corpus` :14 -> copy/`#[path]` bench_profile.rs). Registered in rust/benchmarks/media.rs:8-9 and :45 (`excel::excel_benchmarks`); the `media` bench ([[bench]] rust/Cargo.toml:279-283) keeps ipc/csv/avro/... groups |
| docs | docs/media/excel.md 462 (3 rust/3 python/3 javascript/2 bash blocks; docs.rs links `yggdryl/excel/` -> yggdryl-excel); mkdocs.yml:223; docs/media/index.md:21,27,365; benchmarks.md:45-46,133; holder/index.md:123; compression.md:15; csv.md:11; architecture.md:27,36; contributing.md:55; README.md:73,76; AGENTS.md:388 |
| skills | yggdryl-records/SKILL.md:47; yggdryl/SKILL.md:60; references/rust.md:404-435 (`use yggdryl::excel::{..}`), python.md:308-, javascript.md:327-, formats.md:14,38-40,53,69 |
| scripts | scripts/check_excel_interop.py 173 (cargo :56, EXCHANGE :33). No generator touches excel |
| binding side (stay) | python/yggdryl/excel.py 41, tests/test_excel.py 639, benchmarks/media/excel.py 198, node/tests/excel.test.js 429, excel.types.ts 269 |
| inventory | .api-inventory.txt:3784-3978 (sections excel, cell, media, options, package, sheet, styles, workbook); :3857 "claimed by the core ... rank 6" and :3995 EXTERNAL_RANK sentence (excel 6) need rewriting |

## 2. Core items the crate reaches that are pub(crate)/private
Already in rust/src/implementer.rs (no work): `field_from_arrow_schema` :897 (media.rs:15,227 reach it by `crate::arrow::` path today), `BOOLEAN_SPELLINGS` :903 (cell.rs:1272), `bool_from_text` :908 (cell.rs:1267), `Serie::is_string_storage` -> `serie_is_string_storage` :947 (writer.rs:308), `Serie::value_bytes` -> `serie_value_bytes` :961 (writer.rs:309). Already used from implementer today (pub path, just rename the crate): `expected_got` (cell.rs:1271, media.rs:455), `elide_to` + `ERROR_TEXT_LIMIT` (cell.rs:1275), `with_field` (cell.rs:1327,1331,1342; sheet.rs:888), `result_reader` (reader.rs:534).

**Missing from implementer.rs - the move must ADD (22 path items + 2 methods = 24):**
| item (def) | sites |
|---|---|
| `arrow::arrow_schema_from_field` (arrow/mod.rs:314) | media.rs:183 (+import :15) |
| `floating::f64_from_text` (floating.rs:827) | cell.rs:1188 |
| `iobase::hierarchy::owned_handle` (hierarchy.rs:173) | media.rs:36 |
| `iobase::transfer::overwrite_arrow_reader_default_with_field` (transfer.rs:265) | media.rs:560 |
| `iobase::transfer::leaf_writer` (transfer.rs:1204) | media.rs:572 |
| `iobase::transfer::append_arrow_reader_default` (transfer.rs:119) | media.rs:585 |
| `iobase::transfer::merge_arrow_reader_default` (transfer.rs:162) | media.rs:598 |
| `iomedia::container_field` (iomedia.rs:54) | media.rs:512,535 (2) |
| `iomedia::container_row_size` (iomedia.rs:1095) | media.rs:493 |
| `iomedia::dimension_options` (iomedia.rs:1070) | media.rs:495,514 (2) |
| `iomedia::own_options` (iomedia.rs:68) | media.rs:545,555,580,593 (4) |
| `iomedia::read_record_serie` (iomedia.rs:1385) | media.rs:547 |
| `temporal::format_datetime` (temporal.rs:584) | cell.rs:165 |
| `temporal::parse_date` (temporal.rs:711) | cell.rs:1205 |
| `temporal::parse_time` (temporal.rs:960) | cell.rs:1214 |
| `temporal::scalars::nanoseconds_per` (temporal.rs:1844, const fn) | cell.rs:280 |
| `TemporalKind` enum (temporal.rs:1592; root `pub(crate) use` lib.rs:353; variants Date/Time/DateTime/Duration/Interval) | cell.rs:24 import, :240-1325 (~9 match arms), writer.rs:58 (`Interval`) |
| `xml::wire::write_leaf_text` (wire.rs:150; re-exported `pub(crate) use wire::{..}` xml/mod.rs:62) | cell.rs:1108; writer.rs:400 (2) |
| `xml::wire::write_x_escape` (wire.rs:161) | shared_strings.rs:218 |
| `xml::wire::decode_x_escapes` (wire.rs:173) | shared_strings.rs:235 |
| `xml::wire::write_element_text` (wire.rs:224) | cell.rs:1151; shared_strings.rs:201; sheet.rs:960,997,1013 (5) |
| `xml::wire::write_attribute_text` (wire.rs:233) | workbook.rs:1280 |
By method call on a core type (not visible to a path scan):
| method | sites |
|---|---|
| `ZipArchive::member_reader(self: &Arc<Self>, path) -> Result<Option<Box<dyn Read+Send>>>` pub(crate), zip/archive.rs:452 | workbook.rs:571 (1) - needs a forwarder `zip_archive_member_reader` |
| `DataTypeId::temporal_kind` pub(crate) const, datatype_id.rs:603 | cell.rs:239,1320,1341; writer.rs:57 (4). Alternative with no export: public `DataTypeId::temporal_family() -> Option<&'static str>` (datatype_id.rs:727, "date"/"time"/"datetime"/"duration"/"interval") would let `TemporalKind` stay private (match on the 5 strings) |
Other `ZipArchive` calls are pub (workbook.rs: new, with_restart_stride, entries, get_entry, read_member, write_member_with/_from, handle_reads, into_handle). Excel keeps its own `cached: OnceLock<Workbook>` (media.rs:370-445), no `MediaCache`. Macros `delegate_iobase!` (media.rs:603), `record_options_fields!` expand to pub items only.

## 3. What the core reaches INTO the crate
| site | kind | action |
|---|---|---|
| rust/src/media/codec.rs:173 `&crate::excel::EXCEL_CODEC` in `seed()` (+ rank doc :45) | register seed | delete; crate `install()` = `media::codec::claim(&EXCEL_CODEC, "yggdryl-excel")`. Rank problem: key fact 1 |
| rust/src/lib.rs:60 `pub mod excel;` | module decl | delete (no `internals` re-export exists) |
| media/options.rs:1763 (doc order list), :2071 (error text "expected CSV or Excel options to set a header", pinned by tests/csv/options.rs:272) | prose | keep or re-spell with the pin |
| rust/src/logging/facade.rs:87 `("yggdryl_excel","yggdryl.excel")` | logger map | already in place |
| mime_type.rs:253,485,588; mime_type/registry.rs:75,179 (XLS/XLSX vocabulary), media/magic.rs (`MimeType::XLSX`) | routing vocabulary | stay core (as AVRO/PARQUET) |
Direct type references from core code: NONE. Without the seed claim a `.xlsx` handle is refused until `install()` runs.

## 4. Cargo
rust/Cargo.toml: no `excel` feature; module unconditional. Crate deps: yggdryl (path), arrow-array (workspace; `RecordBatchIterator` media.rs), `smol_str = "=0.3.2"` (cell/package/sheet...), `quick-xml = { version = "=0.41.0", default-features = false }` (core keeps it: `xml/` codec, fix CBlock, soap, xmla - Cargo.toml:105-106). No flate2/bytes/serde use in excel/. Dev-deps: criterion 0.7, arrow-array (interop test). Core keeps quick-xml, smol_str. Core tests that name Excel need `yggdryl-excel` as dev-dependency + `install()`. python/Cargo.toml:22, node/Cargo.toml:22, cli/Cargo.toml gain `yggdryl-excel`; each binding init and the CLI must call `install()`. `scripts/check_docs_examples.py:209` compiles Rust page blocks (features "parquet iceberg s3 s3tables http3"); the excel page's and skills' Rust blocks need the new dev-dep there.

## 5. CI
- .github/ci/rows.toml:96 (commented): `# excel = { package = "yggdryl-excel", jobs = ["excel-interop"] }` -> uncomment together with rust/excel/Cargo.toml (planner refuses either alone). No `after`.
- rows.toml:259-262 `[rows.x-excel] extends=["interop"] paths=["scripts/check_excel_interop.py"] jobs=["excel-interop"]`; `[rows.interop]` (:244-246 area) paths `rust/tests/interop.rs`, `rust/tests/interop/**` extends `corelib` - the Excel half leaves it. Nothing reads rust/src/excel but `corelib` (rust/src/**).
- ci.yml (main) :475-503 job `excel-interop` (needs core-default, pip openpyxl; comment :478 cites rust/tests/interop/excel.rs); needs-list :1039. wip/s3 copy: .github/workflows/ci.yml:317-342.
- check_excel_interop.py:56 runs `cargo test --locked --test interop excel:: -- --nocapture` cwd `rust/` -> `-p yggdryl-excel --test interop` (or cwd rust/excel); EXCHANGE `rust/target/excel-interop` (:33) and tests/interop/excel.rs:29-35 (`current_dir()/target/excel-interop`) must agree under the new cwd (workspace target is the root `target/` either way - the test writes under cwd, the script reads under `rust/`). The driver greps stdout for "excel-interop: wrote/read/read 1904" (:160-167) and fails on SKIPPED.

## 6. Bindings (yggdryl::excel:: type refs)
- python/src/excel.rs (1429 lines): `use yggdryl::excel::{Cell, CellKind, CellRange, CellRef, DateSystem, Row, Sheet, SheetState, Workbook}` :16-18 + `SheetKind` :1235, `MAX_ROWS/MAX_COLUMNS/MAX_CELL_TEXT/MAX_SHEET_NAME/DEFAULT_SHEET_NAME` :1420-1426 (7 qualified refs + the use). python/src/iomedia.rs: `use yggdryl::excel::ExcelOptions` :64; `settings::<ExcelOptions>` :1546,2222,2239; `require_settings_mut::<ExcelOptions>` :2229,2252. python/src/media/handles.rs names Excel by codec string only (:43-44,110,134,155); iobase.rs:373. python/src/lib.rs:38 `mod excel`, :690 `excel::register`.
- node/src/excel.rs (1135): same `use` :14-16, `SheetKind` :962, MAX_* :1123-1127, DEFAULT_SHEET_NAME :565. node/src/media/options.rs: `use yggdryl::excel::ExcelOptions` :11; `settings::<ExcelOptions>` :552,571; `require_settings_mut` :561,581. node/src/lib.rs:26 `mod excel`.
- cli/src: no excel ref (only needs `install()`).
- Inventories: .api-bindings.txt python `excel`, `excel.CellRef/CellRange/Cell/Row/Sheet/Workbook` :354-380, media :150-151 (Excel role under Media), :61; node :571-573 (`excel sections: sheet, withTrim`), :745-756 (`ExcelRow`, Sheet, Workbook, `excel` namespace). check_api_inventory.py must map `[rust/src/excel/...]` -> `rust/excel/src/...`.

## 7. Pins the move must keep green
- s2_pins (rust/tests/media/options.rs `mod s2_pins` :1538): `("excel", 9_017_146_332_497_625_255)` :1614; shared-sections `("excel", 15_607_496_437_425_493_778)` :1645; `excel/sheet=Trades` 15_880_239_124_503_455_895 :1812; `excel/header=false` 4_068_267_976_377_555_643 :1813; order pin `the_media_order_by_their_rank` :1651-1686 (xlsx last after csv; `XLSX > TSV` :1685); `media()` list :1565.
- media_register.rs:437 expected register order `["avro","text","xmla","csv","excel","testmedium"]`; :616-620 `excel < testmedium` rank assertion ("core's seven hold the positions below the external ones").
- iobase_calls.rs: `check("excel", Excel::new(handle))` tail-read row :317-319; `fn excel_costs` :1149-1167 (4 pinned rows, `pstream_bytes=1 bound_location=3 ... parent=1`).
- allocations.rs: `excel_field`/`excel_rows` helpers :9922-9952; `excel_cell_reads_allocate_nothing` :9955; `excel_sheet_costs_per_row_and_nothing_per_cell` :9997; `excel_record_doors_cost_per_row_and_nothing_per_cell` :10058 (uses `MimeType::XLSX` :10065). They need the counting allocator harness of allocations.rs: either stay in the core target with a dev-dep, or the harness is copied/shared.
- Other core tests needing the dev-dep + `install()`: media/mod_.rs:8 (`Workbook`), :92-93,120-124,170,203,243-288; holder/mod_.rs:163-209,551 (`.xlsx.gz` refusal); media/magic.rs:11-16,63; benches: none outside media/excel.rs. mime_type/registry.rs, root/mime_type.rs, csv/options.rs:272 only name the vocabulary.
- Interop: the 3 tests above + scripts/check_excel_interop.py (refuses SKIPPED).

## Decision-relevant facts for the move script
1. RANK MOVES FOUR PINNED HASHES AND THE ORDER PIN: `ExcelCodec::rank()` is 6 (media.rs:301-303); `RecordOptions::hash` feeds `self.rank()` (media/options.rs:1777) and `Ord` uses it (:1770). `media::codec::claim` refuses `by == CORE` and `rank < EXTERNAL_RANK` 32 (codec.rs:192-205), so a plain external claim changes excel's 4 s2_pins hashes and the media_register order assertion - the "changed identity, never re-pin" case. Same blocker as avro/parquet/xmla: needs a core-side reserved-rank door or explicit sign-off before this move.
2. The crate reaches 22 path items + 2 methods (24; each xml::wire fn counted) that implementer.rs lacks - it already covers 5 (`field_from_arrow_schema`, `BOOLEAN_SPELLINGS`, `bool_from_text`, `serie_is_string_storage`, `serie_value_bytes`) and 5 more are used from it today. Clusters: 9 `iomedia`/`iobase::transfer` medium-composition helpers (media.rs only), 5 `xml::wire` text writers (xml/mod.rs:62 `pub(crate) use`), 6 temporal items incl. the `TemporalKind` enum (can be avoided via public `temporal_family`), `ZipArchive::member_reader`, `owned_handle`, `arrow_schema_from_field`, `f64_from_text`.
3. Excel is a leaf: nothing in the core references it by type (only the `seed()` claim at media/codec.rs:173 and `pub mod excel` at lib.rs:60), no cargo feature or cfg gate, no `internals` module, and its only core dependents are tests (media/options, media_register, iobase_calls, allocations, media/mod_, holder/mod_, media/magic) plus the Python/Node `ExcelOptions`/`Workbook` refs. media.rs (687) is what D34 (MediaCache) rewrites - re-read if D34 lands first.
