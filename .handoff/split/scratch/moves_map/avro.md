# yggdryl-avro move map (S6a) - read from /home/user/yggdryl-s3 @ wip/s3 0a24f1806 (uncompiled, read only)

Method: refs.py copy rooted at the s3 tree (scratchpad/avro_work) plus a pub(crate)-method pass. wip/s3 has no `.github/ci/`; rows.toml/ci.yml cited from /home/user/yggdryl (main).

## 1. File set that moves
| what | files (lines) |
|---|---|
| src `rust/src/avro/` -> `rust/avro/src/` | mod.rs 49, arrow.rs 435, batch.rs 2957, container.rs 1165, datum.rs 1215, resolve.rs 1062, schema.rs 1154, single.rs 89 = 8126 |
| tests harness | rust/tests/avro.rs 25 (8 `#[path]` mods) |
| tests `rust/tests/avro/` | arrow 442, batch 1826, container 493, datum 268, mod_ 162, resolve 644, schema 353, single 72 = 4260 |
| interop | rust/tests/interop/avro.rs 161 (+ `mod avro;` at rust/tests/interop.rs:3-4); drives `scripts/check_avro_interop.py` |
| benches | rust/benchmarks/media/avro.rs 10 (5 `#[path]` mods) + media/avro/{codecs 101, container 94, format 209, projection 117, resolution 98} = 629; registered in rust/benchmarks/media.rs:4-5,39-43 (7 lines, the one `media` target also holds ipc/csv/excel/iceberg groups); all use `crate::bench_profile::corpus` (shared benchmarks/bench_profile.rs: copy or `#[path]`) |
| docs | docs/media/avro.md 344 (2 rust/2 python/2 js/4 bash blocks; :10 and :290-343 say snappy needs `parquet`); mkdocs.yml:215; docs/media/index.md:13,370 (rust block uses `yggdryl::avro::AvroOptions`); docs/benchmarks.md:36-37,90; docs/holder/index.md:1230; docs/types/variant.md:417; 17 docs files mention avro |
| skills | skills/yggdryl-records/references/{rust.md:376-390 (`use yggdryl::avro::{self, Schema}`), python.md:283-305, javascript.md:285-322, formats.md:13,36-37,52 (snappy "needs parquet")} |
| scripts | check_avro_interop.py 228, bench_avro_baseline.py 147 (inert in rows.toml:48) |
| inventory | .api-inventory.txt:3285-3355 (5 sections batch/container/resolve/schema/single; no arrow/datum/mod sections); :3299 states "claimed by the core ... rank 2" (rewrite); :3995 EXTERNAL_RANK sentence lists avro 2 |
| AGENTS.md | `ipc/, parquet/, avro/, csv/` row (~385); rust/README.md:17 |

## 2. Core items the crate reaches that are pub(crate)/private
By path (site counts; "impl." = already in `rust/src/implementer.rs`):
| item | sites | impl.? |
|---|---|---|
| `arrow::arrow_schema_from_field` (arrow/mod.rs:314) | batch.rs:336,1229 (2) | NO |
| `arrow::field_from_arrow_schema` (arrow/mod.rs:1153) | batch.rs:641 | YES (implementer.rs:897, same `(name,&Schema)` signature) |
| `cast::PlanCache` (cast.rs:1234) `::new` (1240) `::get_or_compile` (1245) | batch.rs:43,672,679 | NO |
| `cast::Deferred` (cast.rs:1298) `::default` | batch.rs:43,685 | NO |
| `ArrowCastPlan::compile_schema` (cast.rs:584), `::reconcile_batch` (cast.rs:790) | batch.rs:681,688 | NO (methods; need `arrow_cast_plan_compile_schema`/`_reconcile_batch` forwarders) |
| `iobase::transfer::{leaf_writer, overwrite_arrow_reader_default_with_field, append_arrow_reader_default, merge_arrow_reader_default}` | batch.rs:2811,2788,2832,2844 | NO (4) |
| `iomedia::{container_field (2 sites), container_row_size, dimension_options (2), own_options (3)}` | batch.rs:2727-2842 (8) | NO (4) |
| `media::options::FileThreads` (tuple struct, `.0` pub(crate)) + `FileThreads::resolve` (options.rs:106) | batch.rs:118,141,223,227,381,663,1466 (ctor x2, `.0` x1, resolve x2) | NO; raise to a hidden pub type or export `file_threads_resolve`+ctor |
| `string::is_text_storage` (string.rs:96, const fn) | arrow.rs:12,375 | NO (implementer has `serie_is_string_storage`, a different fn) |
| `uuid::{uuid_bytes, uuid_parse, uuid_text}` (root re-exports `crate::uuid_*`) | datum.rs:1181, batch.rs:1703,1705,2306 (4) | NO |
| `variant::arrow::is_variant_storage` (variant.rs:1522) | batch.rs:1592 | NO |
| `metadata::pairs::sorted_pairs` | container.rs:62 | NO |
| `decimal::decimal_parameters` | schema.rs:920 | NO |
| `hashing::stable::stable_hash_of` | container.rs:56 | YES |
| `text::display::expected_got` | batch.rs:2623 | YES |
Already pub (no work): Error/Result, json::*, arrow::{BatchReader,from_reader_error,memory_size}, ArrowCastOptions/Plan, Codec/Level/Limits, Filter/Selector, Holder, IOBase/IOMedia/IOResult, media::{MediaCodec,MediaWrapper,MediumSettings,IORecordOptions,RecordOptions}, StreamSerie/StreamChunkedSerie, Variant, VARIANT_* , Decimal consts, macros `delegate_iobase!`/`record_options_fields!` (`$crate::` expansions name only pub items - checked).
Method-call pass: the only pub(crate) methods reached are the PlanCache/ArrowCastPlan/FileThreads ones above; other name collisions (`Metadata::insert/update/clear/remove`, `Serie::append/write`) were local HashSet/Crc/Vec/own types, verified by line. implementer must ADD 18 entries (counting each fn): arrow_schema_from_field, PlanCache::{new,get_or_compile}, Deferred, ArrowCastPlan::{compile_schema,reconcile_batch}, 4 transfer fns, 4 iomedia fns, FileThreads(+resolve), is_text_storage, uuid_{bytes,parse,text}, is_variant_storage, sorted_pairs, decimal_parameters.
Tests reach `yggdryl::internals::{avro_arrow,avro_batch,avro_schema}` (lib.rs:402-404; 6 sites: tests/avro/arrow.rs:50,55; batch.rs:10,1427; mod_.rs:11): the crate needs its own `internals` feature + the hidden modules (arrow.rs:405, batch.rs:1438, schema.rs:1137); scripts/generate_internals.py must learn it.
Feature gates in the source: `parquet` (container.rs:107,119,170,215,237 = snappy) and `iceberg` (container.rs:841,997,1017 = `raw_metadata`/`Blocks::metadata_bytes`) become unconditional in the crate; tests/avro/container.rs:368 and benchmarks/media/avro/codecs.rs:58 `cfg!(feature="parquet")` snappy cases likewise.
Tree note: Avro<H> still owns `cached_dimensions`/`AvroDimensions` (batch.rs:2653-2705); D34/P2 (MediaCache, in progress) rewrites exactly this - move after P2 or re-read.

## 3. What the core reaches INTO the crate
| site | kind | action |
|---|---|---|
| rust/src/media/codec.rs:169 `&crate::avro::AVRO_CODEC` in `seed()` (+ doc lines 44,163) | register seed | delete; crate `install()` = `media::codec::claim(&AVRO_CODEC, "yggdryl-avro")`. Rank: key fact 1 |
| lib.rs:30 `pub mod avro;`, :402-404 | module decl | delete |
| rust/src/iceberg/manifest.rs - 15 `crate::avro` lines, all DIRECT type refs, not register: `read_blocks().metadata_bytes()` :529, `write_container` :708,945,1706, `read_container` :785,857, `container::{Header, MAGIC, SYNC_LEN, SCHEMA_KEY, check_magic, parse_header, parse_header_entries, header_entry}` :535-540,823,1063-1087,1246-1282, `datum::{Cursor, DatumCodec{names,limits}}` :536,1065-1109, `Header.{schema.names,schema.node,coding.load_into,sync}`. Iceberg is the ONLY core module using the avro scalar codec. The new crate must publish (hidden `pub`) those pub(crate) items: container consts/fns/Header fields/`BlockCoding::load_into`, `Cursor`, `DatumCodec`, `Schema.{names,node}`, `Blocks::metadata_bytes`. Re-point 15 lines to `yggdryl_avro::`; iceberg then depends on avro (DESIGN crate map) |
| rust/src/logging/facade.rs:86 `("yggdryl_avro","yggdryl.avro")` | logger map | already in place, no change |
| rust/src/mime_type.rs (Avro wire variant, AVRO const), mime_type/registry.rs:33-34,132, media/magic.rs:51 (`MimeType::AVRO`) | routing vocabulary | stay core (D10, as PARQUET) |
| bindings and core tests/benches | type refs | sections 6, 7 |
Without the seed claim, a `.avro` handle in a process that never ran `install()` is refused naming the crate to install.

## 4. Cargo
rust/Cargo.toml today: no `avro` feature (module unconditional). Only Avro-driven edge: `parquet = ["dep:parquet","dep:snap"]` (:18) and optional `snap` (:141, comments :16-17,:139-140) - D16: drop both from the core; `yggdryl-avro` carries `snap = "1.1"`. Crate deps: arrow-array/-buffer/-schema (workspace), `bytes = "1"` (6 uses), `smol_str = "=0.3.2"`, `flate2` (zlib-rs, `Crc` only, container.rs:198,223), `snap`, `yggdryl` path. Dev-deps: criterion 0.7, arrow-array/-schema, `yggdryl` + `internals`. Core bench `media` (Cargo.toml ~280) keeps ipc/csv/excel groups; core tests naming Avro need `yggdryl-avro` as dev-dependency (same-package cycle is allowed; confirm one `yggdryl` instance links). python/Cargo.toml:22, node/Cargo.toml:22, cli/Cargo.toml:31 gain `yggdryl-avro`; core keeps bytes/flate2/smol_str.

## 5. CI
- .github/ci/rows.toml:93 (commented): `# avro = { package = "yggdryl-avro", jobs = ["avro-interop"] }` -> uncomment; planner refuses crate-without-line and line-without-crate, so the Cargo.toml and the line land together. Not `after` anything (iceberg is `after = ["avro","parquet"]`, :95).
- rows.toml:254-257 `[rows.x-avro] extends=["interop"] paths=["scripts/check_avro_interop.py"] jobs=["avro-interop"]`; `[rows.interop]` (:244-246) paths `rust/tests/interop.rs`, `rust/tests/interop/**` - the Avro half leaves it (rust/avro/tests/..); inert list :48 holds scripts/bench_avro_baseline.py; `[rows.corelib]` :130 paths `rust/src/**`.
- ci.yml (main) :444-473 job `avro-interop` (needs core-default, fastavro, `python scripts/check_avro_interop.py`); :1039 needs-list. check_avro_interop.py:93-103 runs `cargo test --test interop avro::` cwd `rust/` -> `-p yggdryl-avro`; EXCHANGE `rust/target/avro-interop` and tests/interop/avro.rs:15-19 (`target/avro-interop` relative to cwd) must agree under cwd `rust/avro/`; apache-avro probe (:148-200) is a scratch project, unaffected.
- Leaf-only change runs `[leaf]` jobs (:99): fmt, inventory, docs-rust, python-wheel, python, node, plus avro-interop.

## 6. Bindings (yggdryl::avro:: type refs)
- python/src/avro.rs (618 lines): 7 refs - `use yggdryl::avro::{Block, Blocks, Container, Resolution, Schema}` (:14-16), read_container(_resolved)_with_limits :429,431, read_blocks_owned_with_limits :461, write_container :497, from_single_object_slice_with_limits :529, into_single_object_vec :545. python/src/lib.rs:29 `mod avro`, :686-689 4 classes, :804-808 5 fns. python/src/media/partition.rs:110 `yggdryl::avro::MAX_SCHEMA_DEPTH`. python/src/iomedia.rs: `use yggdryl::avro::AvroOptions` :63; `settings::<AvroOptions>` :1556,2261,2278; `require_settings_mut` :2269,2288 (`AvroOptions::block_codec` :2262). media/handles.rs names "avro" by codec name only. python/yggdryl/{avro.py, avro.pyi, __init__.py:21,481, _native.pyi:7024}.
- node/src/avro.rs (354): 7 refs - `use yggdryl::avro::{Block, Blocks, Container, Resolution, Schema}` :7; :149,161,273,275,291,328. node/src/lib.rs:15,83-86. node/src/media/options.rs:10 `AvroOptions` + :606,614,623,632. node/tests/{avro.test.js, avro.types.ts 109}.
- cli/src: no `yggdryl::avro::` ref (market.rs:110 is a string) but CLI main, python module init and node init must each call `install()`.
- .api-bindings.txt: python `avro.*` :341-351, `AVRO_MAX_SCHEMA_DEPTH` :112, media :150-153; node `Avro*`/`avro` :719-730. check_api_inventory.py resolves `[rust/src/avro/...]` brackets -> `rust/avro/src/...`.

## 7. Pins the move must keep green
- s2_pins hashes, rust/tests/media/options.rs `mod s2_pins` (:1538-): `"avro"` default options 6_316_100_862_033_290_799 (:1609); shared-sections 16_103_427_577_062_353_326 (:1640); `avro/codec=null` 2_341_579_485_644_533_238 (:1803); `avro/sync_marker` 10_665_391_877_395_609_766 (:1804); order pin `the_media_order_by_their_rank` (:1651-1686: ipc < parquet < avro < text < xmla < tsv < csv < xlsx) and `media()` label list (:1560). These tests `use yggdryl::avro::AvroOptions` (:1539) -> dev-dep + `install()`.
- media_register.rs:437 expected register order `["avro","text","xmla","csv","excel","testmedium"]` (:23,561 AvroOptions).
- iobase_calls.rs: `fn avro_costs` :1134-1147 (rows `pread=1 size=1 media_type=1 is_container=1` / `read_all_bytes=1 media_type=2 is_container=1` / `pread=1 size=2 media_type=2 is_container=2` x2) and `check("avro", ...)` tail-read row :315; benchmarks/holder/calls.rs:302. allocations.rs has NO avro row (:11230 is iceberg_official parse_avro only).
- Other core tests needing dev-dep + install: root/iomedia.rs:3177,3364; root/media_serie.rs:259,399-419; media/mod_.rs:7,72,104,110; csv/options.rs:4,277; ipc/mod_.rs:351; xmla/options.rs:13,251-270; root/ascii.rs:294; graph/serve.rs:574; warehouse/folder.rs:1238,1253; iceberg/manifest.rs (15 write/read_container) and iceberg/mod_.rs:9376; benches media/io/write.rs:23,128, dimensions.rs:12,208-247. interop/iceberg.rs, s3/mod_.rs:858, medallion_ledger.rs:232, uri/arn.rs:164 only name `.avro` files.
- Interop: rust/tests/interop/avro.rs tests `writes_a_container_for_the_external_reader` :115, `reads_the_container_the_external_writer_produced` :130 (prints SKIPPED :133), `reads_the_container_the_apache_avro_crate_produced` :148; driven by scripts/check_avro_interop.py (refuses SKIPPED).
- Python/Node avro suites (test_avro.py, avro.test.js, avro.types.ts) stay green unchanged.

## Decision-relevant facts for the move script
1. RANK MOVES THE PINNED HASHES: `AvroCodec::rank()` is 2 (batch.rs:2481) and `RecordOptions::hash` feeds `self.rank()` (media/options.rs:1777) and `Ord` orders by it (:1760-1770). `media::codec::claim` refuses any rank < `EXTERNAL_RANK` 32 (codec.rs:199) and refuses `by == CORE`. A plain external claim changes all four avro s2_pins hashes and the order pin, which AGENTS/DESIGN (D19, :1160-1170) call "a changed identity, never a number to re-pin". The move needs a core-side reserved-rank door (cf. `MarketDescriptor::RESERVED_*`) or explicit user sign-off; parquet/xmla/excel moves share the same issue.
2. implementer.rs already covers 3 of the avro crate's non-pub reaches (`stable_hash_of`, `expected_got`, `field_from_arrow_schema`); the move must add the 18 entries of section 2 (cast plan cache, 4 transfer + 4 iomedia helpers, FileThreads, uuid_*, storage probes, sorted_pairs, decimal_parameters).
3. Iceberg (manifest.rs, 15 direct `crate::avro` lines) is the only core consumer and reaches pub(crate) container/datum internals (Header fields, Cursor, DatumCodec, parse_header*, MAGIC, `Blocks::metadata_bytes`); these must be hidden-pub in yggdryl-avro, so avro lands before iceberg (S6c). The seed claim at media/codec.rs:169 is replaced by `install()`, which every binding init, the CLI and ~15 core tests must call; `snap` leaves the core's `parquet` feature and snappy/`raw_metadata` cfg gates become unconditional.
