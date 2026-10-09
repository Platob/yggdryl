# yggdryl-xmla move map (S6c, D33) - read of /home/user/yggdryl-s3 @ 0a24f1806 (wip/s3, uncompiled)

Tool: `work/refs.py` copy repointed at the s3 tree (`moves_map/tool/`), `python3 -I`: 382 refs from xmla+soap (328 pub, 53 pub(crate), 1 private = internal `mod server`); methods by `tool/meth.py` + manual check.

## 1. File set that moves (lines)
- `rust/src/xmla/` 7053: dbtype 263, definitions 372, media 597, mod 82, options 132, request 866, response 508, rowset 1560, server 134 (`#[cfg(feature="http")]`, mod.rs:43), service 1782, vocabulary 757. `rust/src/soap/mod.rs` 995 (only xmla reads it; `soap::distinct` pub(crate) is used at xmla/request.rs:675 - stays inside the crate). Rustdoc fences 14.
- Tests: `rust/tests/xmla.rs` 30 (harness; server.rs gated `http`, :12-ish) + `tests/xmla/` dbtype 1428, definitions 2070, media 2763, mod_ 2298, options 1795, request 2686, response 2051, rowset 4198, server 674, service 1044, vocabulary 1587 = 22594; `tests/xmla/fixtures/excel/{wizard,pivottable,pq-query,refresh..}` 224 files 1.6 MB, read via `env!("CARGO_MANIFEST_DIR")/tests/xmla/fixtures/excel` (tests/xmla/service.rs:888 - re-point to the crate's own `tests/`); `tests/soap.rs` 8 + `tests/soap/mod_.rs` 2472.
- Rows cut out of core test files: `tests/iobase_calls.rs` :320 (`check("xmla", ..Xmla::new)`), :1169-1182 `xmla_costs`, :1573-1689 `mod provider` (needs `tests/support/counting_filesystem.rs`, declared `#[path]` at :16 - share or copy); `tests/allocations.rs` :41 `use yggdryl::xmla::Rowset`, :9574-9630 `a_rowset_write_allocates_nothing_per_row` (64 vs 4096 rows).
- Benchmarks: `rust/benchmarks/media/xmla.rs` 235 (uses `crate::bench_profile::corpus`, benchmarks/bench_profile.rs - shared, needs a copy or a support path); registered at `benchmarks/media.rs:16-17,48`.
- Interop: none (no `tests/interop/xmla*`, no `scripts/check_xmla*`).
- Docs: `docs/media/xmla.md` 577 (22 fences: 1 rust, 1 `{.python .ignore}`, 4 bash, nginx/caddy/powershell/xml/text; mkdocs.yml:222). One-line mentions to re-point: docs/media/index.md (table row), docs/benchmarks.md:44, docs/testing.md:86 (`--test xmla` -> `-p yggdryl-xmla`) and :149, docs/architecture.md (2), docs/contributing.md:55, docs/fix/cli.md:64, docs/warehouse/index.md:1136, README.md:73-74, AGENTS.md (6: Layout rows 386-387, `docs/media/` list ~2572). Skills: none.
- Scripts: `scripts/check_wheel_smoke.py:66-130,216` (`serve_over_xmla` drives the staged `yggdryl xmla serve`), `scripts/stage_cli.py:122` comment. Neither needs editing if the CLI keeps the verb.
- CLI: `cli/src/xmla.rs` 187, `cli/tests/xmla.rs` 513 (:13 `use yggdryl::xmla::{Discover, Request, RequestType, Response}`), `cli/src/main.rs:49,88-90,142` (module + `Command::Xmla`), `cli/src/location.rs:5` (comment).

## 2. Core privates the crate reaches (pub(crate) path, site count = grep of the name in xmla/+soap/)
Already in `implementer.rs` (8): `arrow::field_from_arrow_schema` (media.rs:16,200; implementer.rs:897), `http::server::normalize_path` (server.rs:70; :876, http-gated), `integer::integer_from_text_as` (response.rs:181, vocabulary.rs:618; :863), `temporal::format_timestamp` (rowset.rs:1321; :884), `Serie::is_string_storage` (rowset.rs:1201; :947), `Serie::is_byte_storage` (:1228; :954), `Serie::value_bytes` (:1202,:1229; :961), and `text::{expected_got, elide_to, ERROR_TEXT_LIMIT}` (media.rs:295, vocabulary.rs:754; `pub use text::display`, :915). `record_options_fields!` (options.rs:119) and `xxhash::xxh3` (service.rs:313) are already `pub`.

MISSING from implementer.rs - the list the move adds (shared = also reached by another leaving crate, so one forwarder serves both; per the tool):
| item | sites | also reached by |
| --- | --- | --- |
| `arrow::arrow_schema_from_field` (arrow/mod.rs:314) | media.rs:16,168 (2) | avro excel iceberg parquet |
| `iobase::transfer::overwrite_arrow_reader_default_with_field` (:265) | media.rs:397 | avro excel parquet |
| `iobase::transfer::append_arrow_reader_default` (:119) | media.rs:422 | avro excel parquet |
| `iobase::transfer::merge_arrow_reader_default` (:162) | media.rs:435 | avro excel parquet |
| `iobase::transfer::leaf_writer` (:1204) | media.rs:409 | avro excel parquet |
| `iomedia::container_field` (iomedia.rs:54) | media.rs:334,372 (2) | avro excel parquet |
| `iomedia::container_row_size` (:1095) | media.rs:315 | avro excel parquet |
| `iomedia::dimension_options` (:1070) | media.rs:317,336 (2) | avro excel parquet |
| `iomedia::own_options` (:68) | media.rs:382,392,417,430 (4) | avro excel iceberg parquet |
| `iomedia::read_record_serie` (:1385) | media.rs:384 | excel |
| `uuid::uuid_parse` (uuid.rs:267) | service.rs:291 | avro iceberg |
| `warehouse::object::path_text` (object.rs:286) | service.rs:36,+2 uses (3) | iceberg |
| `xml::wire::write_element_text` (wire.rs:224) | soap:50, request.rs:685,711,725, rowset.rs:1224,1330, +3 (9 name hits) | excel |
| `xml::wire::write_attribute_text` (:233) | response.rs:164,166,168, rowset.rs:1455 (4) | excel |
| `xml::wire::write_leaf_text` (:150) | rowset.rs:1333, service.rs:1340 (2) | excel |
| `xml::wire::write_x_escape` (:161) / `decode_x_escapes` (:173) | rowset.rs:315 / :341 | excel |
| `bytes::into_base64` (bytes.rs:1917) | rowset.rs:1233 | xmla only |
| `serie::arrow::from_canonical_rows` (serie/arrow.rs:1095) | rowset.rs:817 | xmla only |
| `temporal::parse_timestamp` (temporal.rs:1043) | rowset.rs:1032 | xmla only |
| `warehouse::catalog::no_catalog` (catalog.rs:565) | service.rs:36 (+2) | xmla only |
| `warehouse::holds` (warehouse/mod.rs:398) | service.rs:36, ~1 call at 964 (`holds(&root.to_string(), &text)`) | xmla only |
| `xml::wire::write_fragment` (:128) | soap:51, request.rs:693,727,747 (+1) (5) | xmla only |
| `xml::wire::shaped` (:890) | rowset.rs:812 (+4 uses) | xmla only |
| `xml::wire::is_name_start` (:760) / `is_name_char` (:782) | request.rs:818,820; rowset.rs:309,311 (2+2) | xmla only |
Method calls on core types reaching a pub(crate) method: only `Plan::map_sources` (expression/plan.rs:1301; service.rs:946, 1 site; xmla-only; add as `plan_map_sources(plan, f)` forwarder) plus the three `Serie` methods already exported. Looked like core privates but are not: `leaf.bound()` (pub const), `options.set_field` (XmlaOptions'), `catalog.url()`. All signatures use public types, so plain `#[inline]` forwarders. Net 24 new forwarders (15 shared, 9 xmla-only) + 1 method.
Rank: `XmlaCodec::rank()` = 4 (media.rs:529) but `media::codec::claim` refuses rank < `EXTERNAL_RANK` 32 (codec.rs:199) - the install() cannot claim as-is; the order pin (below) and `RecordOptions::cmp` doc (options.rs:1763) state xmla between text and csv. Decision needed (D33 "left to S6": rank of a leaving medium).

## 3. What the core reaches INTO the crate
| site | kind | move action |
| --- | --- | --- |
| `rust/src/media/codec.rs:171` `&crate::xmla::XMLA_CODEC` in `seed()` | direct type ref (core seeds it; the register covers every other lookup) | delete the seed row; crate's `install()` does `media::codec::claim(&XMLA_CODEC, "yggdryl-xmla")`; update codec.rs:44 doc ("`xmla` 4"), seed comment |
| `rust/src/lib.rs:144` `pub mod soap;` `:175` `pub mod xmla;` | module decls | delete; `yggdryl::xmla::*`/`yggdryl::soap::*` paths vanish (callers below) |
| `rust/src/logging/facade.rs:92` `("yggdryl_xmla","yggdryl.xmla")` | already pre-placed for the crate's target | none |
| `rust/src/warehouse/catalog.rs:277` literal `"xmla"` in the refused-type-word message (+ tests/warehouse/catalog.rs:100,102) | string, D10 keeps it | none |
| `mime_type.rs:45,207-209,463,565,642,717,758`, `mime_type/registry.rs:50,147` `MimeType::XMLA`, `.xmla` | routing vocabulary | stays core (D33) |
| `rust/src/xml/element.rs` (+ wire.rs:122 doc) | doc mention; `xml::{Element,Scope,XSD_NAMESPACE,XSI_NAMESPACE,ATTRIBUTE_PREFIX,TEXT_KEY,from_bytes_with_limits,from_utf8}` are pub - the crate reads them as `yggdryl::xml::` | stays core |
| `rust/src/http/server.rs:655-656,1455-1471` | doc strings only | none |
holder/, serie/, expression/: no reference. Core TESTS that name the crate (must install it, move, or take a dev-dep - a dev-dep cycle yggdryl->yggdryl-xmla is legal for integration tests only): `tests/media_register.rs:437`, `tests/media/mod_.rs:80,166` (`catalog.xmla`, `trades.xmla`), `tests/media/options.rs:1186,1545` (`XmlaOptions`), warehouse/catalog.rs:100,102 (string only).
Bindings: python/src, node/src name NO `yggdryl::xmla::`/`soap::` type (0 sites). Python `Xmla` is picked by codec name: `python/src/media/handles.rs:35,91,108,132,153` (`PyXmla`, `"xmla" => Self::Xmla`), `python/src/iobase.rs:371`; Node has no XMLA door (node/src/http.rs:1409,1632 and index.d.ts:9606,13139 are `/olap/xmla` doc strings). CLI: 1 `use yggdryl::xmla::{Service, ServiceOptions}` (cli/src/xmla.rs:21), 1 in cli/tests/xmla.rs:13. BUT no binding or CLI calls any `install()` yet (python/src/lib.rs:545 and node/src/lib.rs:471 only install logging; cli/src/main.rs:130 warnings): each of python/src/lib.rs, node/src/lib.rs, cli/src/main.rs must call `yggdryl_xmla::install()` (idempotent, both lanes) or `.xmla` opens fail - tests/holder/test_init.py:94-95,306 and typing_bindings.py:127,661 depend on it.

## 4. Cargo
- `rust/Cargo.toml` has NO xmla feature or dep: the crate is gated only by `http` (server.rs) - `[features] http = ["yggdryl/http"]`. Direct third-party use is `arrow_array` (media.rs:14 `RecordBatchIterator`), `arrow_schema` (media.rs:169, rowset, tests), `smol_str` (7 files); everything else is `crate::`->`yggdryl::`. Needs: `yggdryl = { path="../rust" }` (+`http` forwarded), `arrow-array`/`arrow-schema` (workspace), `smol_str = "=0.3.2"` (rust/Cargo.toml:138, `serde` feature). quick-xml (=0.41.0) stays core (xml/). Dev-deps for tests/benches: `criterion` (benchmarks/media/xmla.rs:9), `yggdryl-avro`, `yggdryl-parquet` (tests/xmla/options.rs:13,17: `AvroOptions` block_codec/sync_marker, `ParquetOptions::compression_name`; parquet arm `cfg(feature="parquet")` -> crate feature `parquet`), the counting allocator support. Stays core: quick-xml, http/ureq stack, every `yggdryl` feature.
- Binding crates pull the lane features (python/Cargo.toml:22-27, node/Cargo.toml:22-27: `http3, iceberg, s3, s3tables`) so the xmla server compiles there; cli/Cargo.toml:31 `yggdryl = {features=["http"]}` + gains `yggdryl-xmla = {features=["http"]}`; CLI features `iceberg/s3/s3tables` (cli/Cargo.toml:47-53) stay forwarding to core (and later leaves). Workspace `members` (Cargo.toml:9) gains `rust/xmla`; `default-members` stays `["rust"]`. Release/AGENTS §6: crate name 404 on crates.io (D33), `preflight`/version-bump list gains it.

## 5. CI
- `.github/ci/rows.toml` is NOT in the s3 tree (older base); in /home/user/yggdryl it is: `[leaves]` line 97 `# xmla = { package = "yggdryl-xmla" }` - uncomment, no `after`, no `jobs`, no `msrv`; `[shards.yggdryl]` (:72) lists no xmla target. Planner refuses `rust/xmla/` without it. `[leaf]` jobs (:104) = fmt, inventory, docs-rust, python-wheel, python, node. `rows.cli` (:164, `cli/**`) and `rows.wheel/pyext/nodeext` already run the leaf's binding proof. `rows.inventory` reads `.api-*.txt`.
- No exchange job/script names xmla (`x-*` rows); `ci.yml:67` comment only. docs-rust runs the page's one rust fence.

## 6. Bindings / inventories
- Type-name counts: python/src 0, node/src 0, cli/src 2 files (xmla.rs 1 `use`, + location.rs doc), cli/tests 1. Python `Xmla` class (`yggdryl/media/__init__.py:20,37`, `__init__.py:93,476`, `_native.pyi:4801`) unchanged.
- `.api-inventory.txt`: sections `### yggdryl::soap [rust/src/soap/mod.rs]` :5105-5156; `### yggdryl::xmla` :5157, `::dbtype` :5165, `::definitions` :5176, `::media` :5189, `::options` :5205, `::request` :5212, `::response` :5257, `::server` :5313 (http-gated), `::service` :5317, `::vocabulary` :5350 -> end of that section (~5400); `[file]` anchors must be re-pointed and check_api_inventory.py taught the crate tree.
- `.api-bindings.txt`: lines 150-151 (`media: ... Xmla ...` / "media roles ... Xmla") - no change needed.

## 7. Pins the move must keep green
- `tests/media/options.rs` `mod s2_pins` (:1538): default-options hash xmla `8_486_799_845_904_195_949` (:1611, test :1599) ; shared-sections hash xmla `11_124_752_632_585_582_100` (:1642, test :1620); order test `the_media_order_by_their_rank` (:1651, list incl. `application/xmla+xml` :1671); settings hash `xmla/without_envelope` `2_087_720_147_871_917_823` (:1760-1761, :1809, test :1690); `assert_threads(XmlaOptions::new())` :1186. Hash = `stable_hash_of(&(codec.name(), settings))` (name `xmla`, so rank/crate move does not change it); the ORDER pin changes unless rank stays 4 (see section 2).
- `tests/media_register.rs:437` expected order `["avro","text","xmla","csv","excel","testmedium"]` (plus ipc/parquet) - moves with the rank decision.
- `tests/iobase_calls.rs`: `check("xmla")` :320 (tail read = 1 call); `xmla_costs` :1169 (`read_all_bytes=1 media_type=2 is_container=1` / `...media_type=3 is_container=1` / `read_all_bytes=1 size=1 media_type=3 is_container=2` / `read_all_bytes=1 media_type=3 is_container=2`); `mod provider` test `a_discover_over_an_ipc_catalog_costs_its_listing_and_the_columns_a_schema_read` :1660 (counts 53/1/1/1/2 requests; costs none,none,none,`file_info=2 list=1`,`file_info=2 list=1 open_input_stream=1`).
- `tests/allocations.rs:9579` rowset write 64 rows == 4096 rows allocations.
- `tests/media/mod_.rs:80` (`catalog.xmla` -> Registered name `xmla`), :166 (`trades.xmla` round trip).
- Own suites: `--test xmla` (11 files + http-gated server.rs/service.rs door tests replaying `fixtures/excel/*` wire files), `--test soap`, 14 rustdoc fences, `cli/tests/xmla.rs` (513; spawns `yggdryl xmla serve`), `scripts/check_wheel_smoke.py` XMLA serve (:99-130).
