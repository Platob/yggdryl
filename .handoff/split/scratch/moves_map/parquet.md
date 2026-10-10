# yggdryl-parquet move map (wip/s3 @ 0a24f1806; CI planner read from /home/user/yggdryl, the s3 tree lacks .github/ci)

refs.py re-rooted on the s3 tree: 167 `crate::`/`super::` refs from parquet/ (135 pub, 25 pub(crate), 3 private, 4 intra-crate), plus a by-name method scan.

## 1. File set that moves

| Path | Lines |
|---|---|
| rust/src/parquet/mod.rs | 2841 |
| rust/src/parquet/geospatial.rs | 803 |
| rust/src/parquet/metadata.rs | 282 |
| rust/tests/parquet.rs | 16 |
| rust/tests/parquet/mod_.rs | 3604 |
| rust/tests/parquet/metadata.rs | 247 |
| docs/media/parquet.md | 425 |

- Total src 3926, tests 3867. geospatial.rs is pinned inside tests/parquet/mod_.rs (`internals::parquet_geospatial`, :33), no file of its own. No rust/tests/interop/parquet*: parquet is exchanged only via the Iceberg exchange (check_iceberg_interop.py:143); interop/variant.rs uses the `parquet-variant` dev-dep (stays core).
- No parquet-only benchmark file; cases sit in shared files: benchmarks/media/io/dimensions.rs (258; parquet_statistics_cases 118-160, Parquet::new 215-224), io/write.rs (843; parquet_target 108, 380-383), io/pushdown.rs (81; 30,33); the whole `media/io` mod is gated `cfg(feature="parquet")` at benchmarks/media.rs:13,22 (record.rs/value.rs/mod.rs have 0 parquet mentions). 1 incidental mention each: media/avro/codecs.rs, media/iceberg.rs, holder/{calls.rs,fs/record.rs,s3/records.rs}. The move splits the Parquet cases into rust/parquet/benchmarks/.
- Docs: mkdocs.yml:214 nav; docs/media/index.md:12 row, 372-385 (`ParquetOptions` Rust block), 538-539 (`codec_for` rank assert `("parquet","Parquet",1)`); docs/holder/index.md:1229. Rust blocks naming `yggdryl::parquet`: index.md 1, parquet.md 4.
- Skills: yggdryl-records (SKILL.md 12, references/rust.md 12 with 3 `yggdryl::parquet`, python.md 12, javascript.md 8, formats.md 3), skills/yggdryl/references/rust.md (1 `yggdryl::parquet`).
- Scripts: check_docs_examples.py:209, check_iceberg_interop.py:143, bench_avro_baseline.py:36,108 (feature string "parquet iceberg"). rustdoc: 1 example (mod.rs, 2 fences).
- Public surface (.api-inventory.txt:4158-4200, 43 lines): ParquetOptions (mod.rs:129), PARQUET_CODEC/ParquetCodec 432, Parquet<H> 2323, ParquetFooter 2252, read_arrow_schema 567, read_field 581, read_batch_reader 611, overwrite_arrow_reader 709, read_statistics 1071, read_media_statistics 1093, read_media_geospatial_statistics 1121, GeospatialStatistics/read_geospatial_statistics, FileStatistics/RowGroupStatistics/ColumnStatistics (metadata.rs), `internals` mod.rs:2775 (open_builder, overwrite_with_write_buffer, with_threads) and geospatial.rs:771 (annotated_schema, extension_schema).

## 2. What the crate reaches in the core

### 2a. Not pub (must be added to implementer.rs or raised) - 15 distinct items, 22 sites, parquet/mod.rs unless noted

| Item (kind) | Def | Sites | implementer.rs today |
|---|---|---|---|
| arrow::arrow_schema_from_field (fn, path) | arrow/mod.rs:314 | 1 (mod.rs:106) | no |
| arrow::field_from_arrow_schema (fn) | arrow/mod.rs:1153 | 2 (108, 770) | YES, implementer.rs:897 (`field_from_arrow_schema(name,&Schema)`) |
| arrow::projection_indices (fn) | arrow/mod.rs:712 | 1 (108) | no (also used by ipc/) |
| arrow::same_columns (fn) | arrow/mod.rs:668 | 1 (787) | no (parquet only) |
| cast::PlanCache::new (assoc) + method `get_or_compile` | cast.rs:1240 / 1245 | 1 + 1 (772, 789) | no |
| cast::ArrowCastPlan::compile_schema (assoc, pub(crate)) | cast.rs:584 | 1 (790) | no |
| method `ArrowCastPlan::reconcile_batch` (pub(crate)) | cast.rs:790 | 1 (797) | no |
| cast::Deferred (struct) `::default()` | cast.rs:1298 | 1 (794) | no |
| serie::land + serie::Proof::Unproven | serie/arrow.rs:860 / 453 | 1 (1380, one call) | partial: `land_unproven_batch(root:&Resolved,batch)` exists (implementer.rs:727) but parquet lands an `Arc<Field>`+`ArrayRef`, a different signature -> needs a new forwarder |
| geospatial::DEFAULT_CRS | geospatial.rs:137, lib.rs:313 | 1 (geospatial.rs:52) | no; used only by parquet+geospatial.rs itself |
| geospatial::GEOARROW_WKB_EXTENSION_NAME | geospatial.rs:208, lib.rs:314 | 1 (geospatial.rs:52) | no |
| uuid::UUID_EXTENSION_NAME | uuid.rs:159 | 1 (geospatial.rs:52) | no |
| variant::is_variant_storage (fn) | variant.rs:1522, lib.rs:375 | 1 (geospatial.rs:53) | no (avro/batch.rs, field.rs also use it) |
| iobase::transfer::{overwrite_arrow_reader_default_with_field 265, append_arrow_reader_default 119, merge_arrow_reader_default 162, leaf_writer 1204} | iobase/transfer.rs | 1 each (2603, 2641, 2653, 2623) | no |
| iomedia::{own_options 68 (3 sites: 2596,2637,2649), container_field 54 (2550,2579), container_row_size 1095 (2530), dimension_options 1070 (2532,2552)} | iomedia.rs | 8 | no |

- The last two rows (8 fns, 12 sites) are the SAME helpers every medium (avro/batch.rs, csv/media.rs, ipc/mod.rs, excel/media.rs, xmla/media.rs, text/handle.rs) calls: one shared implementer addition serves avro, excel, xmla and parquet; do it once, before the four moves.
- PlanCache/compile_schema/reconcile_batch/arrow_schema_from_field are shared with avro/batch.rs and iceberg/scan.rs too.
- Method scan (`.name(` vs pub(crate)/private inherent methods of core types): the real private-method reaches are `get_or_compile` and `reconcile_batch`; the rest are std/pub or parquet-own. `Bounds::new` (mod.rs:1404) is the PUBLIC expression::pushdown::Bounds::new:116 (tool mis-resolved it).
- Macros `record_options_fields!`, `delegate_iobase!`: already exported. The other ~135 refs are pub at a published path (Error/Result/BatchReader, IOBase/IOMedia/IOResult, Holder, media::*, Field/Filter/Selector/Bound/filter_phases, Serie, wkb::*, VARIANT_*, arrow::array_memory_size, implementer::{elide_display,expected_got}).

### 2b. cfg(feature="parquet") outside parquet/ and what it gates (src)

| Site | Gates |
|---|---|
| lib.rs:122 | `pub mod parquet;` |
| lib.rs:312-313 | `pub(crate) use geospatial::DEFAULT_CRS;` (dead once parquet leaves) |
| lib.rs:523-526 | internals re-exports `parquet_geospatial`, `parquet` (generate_internals.py output) |
| media/codec.rs:167-168 | `&crate::parquet::PARQUET_CODEC` in the core seed list (rank 1) |
| avro/container.rs:107,119,170,215,237 | `BlockCoding::Snappy` variant + from_name/load/dump/name arms |
| avro/container.rs:245 | `cfg!` in `implemented_codecs()` text ("null, deflate, snappy, zstandard") |
| Cargo: rust/Cargo.toml:18 | `parquet = ["dep:parquet","dep:snap"]`; :22 `iceberg = ["parquet",...]` |

- Avro's snappy rides the parquet feature only because `snap` (Cargo.toml:141, optional) is pulled by it (comment :139). After the move avro/container.rs (6 sites) and the `snap` dep must be re-gated (own avro feature or unconditional dep) - not covered by any register.
- Tests with cfg("parquet") or `yggdryl::parquet` outside rust/tests/parquet: ~64 lines, 17 files: root/iomedia.rs 14, media/options.rs 14, media/mod_.rs 6, media_register.rs 3, root/media_serie.rs 3, holder/mod_.rs 3, xmla/options.rs 3, iobase_calls.rs 3, iceberg/mod_.rs 2 (11144,11239), isin_registry/store.rs 2, and 1 each in warehouse/folder.rs, graph/serve.rs, root/isin_registry.rs, media/merge.rs, avro/container.rs, s3/file.rs, s3/path.rs. External `parquet::` is used by tests/iobase_calls.rs:1086 too.

## 3. What the core reaches INTO the crate

| Site | Reference | Covered by register? |
|---|---|---|
| media/codec.rs:167-168 | `PARQUET_CODEC` in seed list | YES: replace by `install()` -> `media::codec::claim(&PARQUET_CODEC,"parquet")`; BUT claim() refuses rank < EXTERNAL_RANK=32 (codec.rs:199) and parquet is rank 1 |
| lib.rs:122, 523-526 | module + internals re-export | direct; delete (generate_internals.py regenerates) |
| iceberg/scan.rs:919,926,1177,1283,1294,1304,1307 (+ doc 1128,1214; external `ParquetMetaData` :47,:1320) | `ParquetOptions` (settings::<>), `read_batch_reader_with` (mod.rs:637), `READ_AHEAD_BATCHES` (1950), `WHOLE_READ_BYTES` (1728), `load_metadata` (1192), `schema_from_metadata` (1241) | NO: direct types; 5 are `pub(crate)` and must be raised to pub; iceberg is `after` parquet |
| iceberg/table.rs:4818, 6165, 6168, 6172, 6174 | `ParquetOptions::new`, `overwrite_buffered` (731), `WRITE_BUFFER_BYTES` (846), `FileStatistics::from_metadata` | NO: same; `overwrite_buffered`, `WRITE_BUFFER_BYTES` pub(crate) -> pub |
| iceberg/statistics.rs:16 | `use crate::parquet::{ColumnStatistics, FileStatistics}` | NO (already pub types) |
| python/src/iomedia.rs:66 (use), 1562, 2301-2350 (10 `settings::<ParquetOptions>`/`require_settings_mut`), iobase.rs:2732,2741,2751,2765 (`read_media_*statistics`), media/handles.rs:33,88,105,130,151 (PyParquet; `"parquet"` matched by codec name) | type refs | NO: re-point to `yggdryl_parquet::` |
| node/src/media/options.rs:15 (use), 645-679, 839 (8 sites), node/src/iobase.rs:1746,1756 (2) | same | NO |
| cli/src | none (market.rs:110 is help text) | - |
| Stay core (vocabulary, no move): `MimeType::PARQUET` (mime_type.rs:119 + registry.rs:30,126-127), media/magic.rs:41, `PARQUET:field_id` key (metadata.rs:54, field.rs `parquet_field_id`), iceberg/options.rs:232 DEFAULT_DATA_MIME_TYPE, iceberg/manifest.rs format maps | n/a |
| logging/facade.rs:91 | `("yggdryl_parquet","yggdryl.parquet")` target already mapped | already prepared for the crate name |

- Nothing in core downcasts `ParquetFooter` (comment iomedia.rs:369 only); warehouse/ and holder/ reach parquet through codec lookups only.

## 4. Cargo

- Today: feature `parquet = ["dep:parquet","dep:snap"]` (rust/Cargo.toml:18); `iceberg = ["parquet",...]` (:22); dep `parquet` 59.2.0, default-features=false, features arrow base64 brotli flate2-zlib-rs lz4 snap zstd, optional (:118-126); `snap` 1.1 optional (:141, avro snappy); dev-dep `parquet-variant =59.2.0` (:208, stays core).
- Crate needs: yggdryl, parquet 59.2.0 (same feature list), arrow-array, arrow-schema (workspace), bytes, smol_str =0.3.2; std threads only (no rayon, no `log::`).
- Stays core: `snap` (or an avro feature), `parquet-variant` dev-dep. python/Cargo.toml:22-27 and node/Cargo.toml:22-27 get parquet only transitively via `iceberg`: add a yggdryl-parquet dep (cli/Cargo.toml:31,47 likewise). Workspace members Cargo.toml:7 gain rust/parquet; release.yml:199,864 `cargo publish -p yggdryl` order: yggdryl, yggdryl-parquet, then iceberg.
- The core `parquet` feature disappears: every cfg in 2b and the tests is re-spelled; core tests using Parquet as exemplar medium need yggdryl-parquet as a dev-dependency (a cycle, allowed for integration tests) or move.

## 5. CI

- rows.toml (main tree /home/user/yggdryl/.github/ci/rows.toml):94 commented `parquet = { package = "yggdryl-parquet", jobs = ["pyiceberg-interop", "spark-interop"] }`; :95 iceberg is `after = ["avro","parquet"]` (jobs pyiceberg-interop, spark-interop, iceberg-msrv). Leaf row = core lib + `rust/parquet/**` + `config/**`; `[leaf].jobs` fmt, inventory, docs-rust, python-wheel, python, node always run (:99-103); the planner refuses a crate with no line (flip the `#`).
- Rows reading its paths: `core` :138 (rust/tests/**, rust/benchmarks/**), `corelib` :132 (rust/src/**), `interop` :247, x-iceberg :264-267 (check_iceberg_interop.py -> pyiceberg-interop). None names rust/tests/parquet or docs/media/parquet.md.
- ci.yml (main tree): :220/:238 `--features "parquet iceberg" --test interop --no-run`, :873; spark-interop :492-525, pyiceberg-interop :97, iceberg-msrv :85-95. The s3 tree's ci.yml is pre-planner (parquet at :537).

## 6. Bindings and inventories

- python/src: 3 files, ~17 sites (iomedia.rs 11, iobase.rs 4 + 2 strings, media/handles.rs 5); node/src: 2 files, 10 sites (media/options.rs 8, iobase.rs 2); cli/src: 0. Generated: python/yggdryl/_native.pyi 25 mentions, node/index.d.ts 22.
- .api-inventory.txt: sections `### yggdryl::parquet::metadata [rust/src/parquet/metadata.rs]` :4158 and `### yggdryl::parquet [rust/src/parquet/mod.rs]` :4165-4200 (geospatial has no section of its own); cross-mentions :3198 (as_any), :3995-4131 (codec doc strings naming parquet), :4490 (MimeType::PARQUET stays), :5756-5850 (parquet_field_id stays core).
- .api-bindings.txt: :40 (python IOBase `read_parquet_statistics/_geospatial_statistics/_null_count/_split_offsets`), :150-151 (media: `Parquet` class), :553 (node `readParquetStatistics`, `readParquetGeospatialStatistics`), :61/:571 RecordOptions compression/max_row_group_size settings. Inventory paths must be re-pointed to `rust/parquet/src/...`.

## 7. Pins to keep green

- rust/tests/iobase_calls.rs: `a_parquet_read_past_a_megabyte_reads_its_footer_first` :1074-1110 ("read_range_bytes=1 read_tail_bytes=1 media_type=2 is_container=1", 200_000 rows); `parquet_costs` :1112-1132 via `surfaces` :911 (schema "read_tail_bytes=1 media_type=1 is_container=1", reader "read_tail_bytes=1 media_type=2 is_container=1", columns "read_tail_bytes=1 size=1 media_type=2 is_container=2", rows "read_tail_bytes=1 media_type=2 is_container=2"). They pass only if the codec is claimed (crate installed in the test process). tests/parquet/mod_.rs holds 26 `Counted/calls` assertions.
- rust/tests/allocations.rs: NO parquet row (only an s3 file-name string at :11197).
- s2_pins (rust/tests/media/options.rs:1538): `("parquet", 8_840_416_273_347_448_133)` :1608, `("parquet", 9_870_246_672_429_389_253)` :1639; `the_media_order_by_their_rank` :1660-1676 (parquet second); parquet states in `the_medium_settings_feed_the_hash` :1718-1760. **`RecordOptions::hash` feeds `self.rank()` (media/options.rs:1777) and `cmp` falls back to rank (:1770)**: rank >= EXTERNAL_RANK (32) moves both hash pins and the order pin; to stay byte-identical the register must admit a reserved rank 1 for a leaving core medium (like the market's `RESERVED_*`), else re-pin with the sentence why.
- media_register.rs:434 (`expected.push("parquet")` in the listed codecs, rank order), :562-565; docs/media/index.md:538-539 asserts `codec_for(PARQUET)` rank 1 and sorted `codecs()`.
- Interop: none Parquet-specific; the Iceberg exchange (PyIceberg v1/v2/v3, Spark) reads/writes parquet files through `iceberg/scan.rs` + `table.rs` (the 15 direct refs in section 3): `rust/tests/interop/iceberg.rs` runs under features "parquet iceberg" (ci.yml:238).
