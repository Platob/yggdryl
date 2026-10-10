# yggdryl-s3 map: tests, pins, exchange scripts, CI rows, docs

Read at HEAD `d100439d3` ("Record the S3 design"), branch `ccr-0fe6f9d0-ruymat`. Every `rust/src`, `rust/tests`,
`rust/benchmarks`, `scripts`, `.github`, `docs`, `skills` and `AGENTS.md` line below is a HEAD line
(`git show HEAD:<path>`); the working tree is dirty (another agent: media files, iomedia.rs, docs, AGENTS.md, inventories). Its hunks, in HEAD
coordinates (`git diff HEAD -U0`): `docs/holder/index.md` 31-2098, `AGENTS.md` 339, 352, 386, 670, 687, 1106, 1452, 1503, 1629,
`.api-inventory.txt` 3065-4881, `.api-bindings.txt` 61 and 64 - none touches an S3 region I cite. `.handoff/` read from
the working tree. No cargo run, nothing edited. Scratch copies of the big HEAD files (ci.yml, plan.py tests, the docs
page, AGENTS.md, the inventories) are beside this report in `s3_backend_map/*.txt`; `blk.py` there lists the `#[test]`
names inside a `mod { }` block at a HEAD line. Counting commands are in the appendix.

Program context (read): `.handoff/next/MARKET_SPLIT_NEXT.md` "## Goal" (eight crates, the split first, one slice per
session, one draft PR #209) and "## Invariants" (registers seeded by the core "until the leaving crate's `install()`
claims them"; cost pins never re-pinned from a sweep); D36 row, line 208: "the object-store backend `s3/` ... a ninth
crate, `yggdryl-s3`, through a storage-backend extension point on the register, registered in place first and moved
after; the CI leaf table gains `rust/s3`". `.handoff/split/DESIGN.md` D21 (501-530, the role table), D27 (1182-1229,
`MediaCodec`, `Media::Registered`), D30 (1274-1303, `Catalog::Registered`, `CatalogFactory`, `Locator`;
"`Holder::from_url` ... keeps every byte-backend arm (`local`, `zip`, `s3`, `http`), which stay the core's",
`Site::Store` "gated to `s3` alone"), "S6's shape"/Risks (686-712: "A target that moves out of the core takes its shard
of `[shards.yggdryl]` with it, and an exchange that moves into a leaf is named in its line's `jobs`").

Headline numbers (HEAD):

| What | Count |
| --- | --- |
| `rust/src/s3/` | 26 files, 11,963 lines |
| `rust/tests/s3/**` + `rust/tests/s3.rs` | 19 + 1 files, 8,677 + 65 lines, **239 `#[test]`** |
| `rust/tests/interop/s3/**` | 4 files, 949 lines, **20 tests** (5 AWS, 8 Azure, 7 Google) |
| `rust/tests/support/server.rs` (the fake store, crate-free) | 3,869 lines, **25 inline `#[test]`** |
| S3-driven tests living in core-resident harnesses | 11 (holder 2, iobase_calls 2, warehouse 2, iceberg 5; 1.5) + 3 in `aws/` (environment :252, sigv4 :127, :253; 1.6) |
| S3 Tables tests sitting on `FakeS3` | s3tables/catalog.rs 19 + medallion_ledger.rs 1 (+ live.rs 1, ignored) |
| `feature = "s3"` gates outside `rust/src/s3/` | 79 in `rust/src`; 23 in tests/benches outside the s3-owned files (`interop.rs:12` among them), + 20 inside the s3-owned test files |
| exchange drivers | 3 scripts, 3 CI jobs, 3 `[rows.x-*]` |
| docs sentences/blocks | holder/index.md 918-line section, 30 tagged blocks of which 12 Rust + 1 Python name the s3 API |

---------------------------------------------------------------------------------------------------------------

## LIST 1 - TESTS, PINS, BENCHMARKS

### 1.1 `rust/tests/s3.rs` and `rust/tests/s3/**` (239 tests, 20 files)

Harness `rust/tests/s3.rs:23-65`: `#[cfg(feature = "s3")] #[path = "support/server.rs"] mod server;` (23-25), then 13
module declarations (14 `cfg` lines with the server), each `cfg(feature = "s3")`, and four of them `cfg(all(feature = "s3", feature = "internals"))`:
`client` (33-35), `file` (39-41), `properties` (57-59), `xml` (63-65). Dialect folders are declared by their own
`mod_.rs` (`aws/mod_.rs:624-626` declares `xml`; `azure/mod_.rs:4-15` declares `auth`, `dialect`, `sign`, `xml`, all
`s3 + internals`; `google/mod_.rs:4-6` declares `token`). The header (1-22) states the mirror rule and that the
identity chain and SigV4 are pinned under `rust/tests/aws/`.

| Test file | Tests | Lines | Pins (src) | Shape / gate | `FakeS3` uses | `internals::s3_*` |
| --- | ---: | ---: | --- | --- | ---: | --- |
| `s3/aws/mod_.rs` | 16 | 626 | `s3/aws/mod.rs` | `mod protocol` 7-563 (14), `mod internal` 565-622 (2, internals) | 4 | `logging_warning::count` (566; no s3 internals) |
| `s3/aws/xml.rs` | 22 | 371 | `s3/aws/xml.rs` | internals | 0 | `s3_answer`, `s3_aws_xml`, `s3_xml`, `xml_scanner` |
| `s3/azure/auth.rs` | 4 | 51 | `s3/azure/auth.rs` | internals | 0 | `s3_azure_auth::read_token` |
| `s3/azure/dialect.rs` | 2 | 23 | `s3/azure/dialect.rs` | internals | 0 | `s3_azure_dialect::{batch_boundary, block_id}` |
| `s3/azure/mod_.rs` | 0 | 15 | `s3/azure/mod.rs` | declares 4 internals modules | 0 | - |
| `s3/azure/sign.rs` | 6 | 120 | `s3/azure/sign.rs` | internals | 0 | `s3_azure_sign::{SharedKey, http_date}` |
| `s3/azure/xml.rs` | 4 | 66 | `s3/azure/xml.rs` | internals | 0 | `s3_azure_xml::{parse_list, render_block_list}` |
| `s3/client.rs` | 38 | 1404 | `s3/client.rs` | internals (file-level) | 3 | `s3_client::{Client, DEFAULT_REGION, Endpoint, RETRY_BACKOFF, backoff, bucket_region_of, total_of_content_range}` (16-19) |
| `s3/encryption.rs` | 6 | 257 | `s3/encryption.rs` | `s3` | 1 | - |
| `s3/file.rs` | 33 | 1271 | `s3/file.rs` | internals; `accounting` 4-423 (10), `protocol` 425-576 (6), `create` 581-930 (8), `tail` 932-1146 (7), `parquet` 1152-1271 (2, `cfg(feature = "parquet")`) | 6 | `s3_file::upload_from` (6) |
| `s3/folder.rs` | 13 | 480 | `s3/folder.rs` | `accounting` 5-303 (8), `protocol` 305-480 (5) | 3 | - |
| `s3/google/mod_.rs` | 0 | 6 | `s3/google/mod.rs` | declares `token` | 0 | - |
| `s3/google/token.rs` | 5 | 67 | `s3/google/token.rs` | internals | 0 | `s3_google_token::read_token` |
| `s3/mod_.rs` | 24 | 1388 | `s3/mod.rs` (fixtures, `[BUCKET]`, `store()`, `options()`, `file()`/`folder()`/`located()`) | `accounting` 149-938 (7: 2 own + `accounting::iceberg` 264-937 = 5, `cfg(feature = "iceberg")`), `roles` 940-1388 (17) | 15 | - |
| `s3/options.rs` | 5 | 229 | `s3/options.rs` | `protocol` 5-114 (3), `internal` 116-229 (2) | 1 | `s3_options::signs_payload` |
| `s3/path.rs` | 13 | 495 | `s3/path.rs` | `accounting` 3-361 (9), `protocol` 363-495 (4) | 3 | - |
| `s3/properties.rs` | 34 | 1414 | `s3/properties.rs` | internals | 0 | `s3_client::Client` (20), `s3_properties::swept` (713, 806, 1007, 1090) |
| `s3/provider.rs` | 12 | 345 | `s3/provider.rs` | `s3` | 0 | - |
| `s3/xml.rs` | 2 | 49 | `s3/xml.rs` | internals | 0 | `s3_answer::ErrorBody`, `s3_xml::parse_error` |

(`aws/mod_.rs`'s two `internal` tests, 565-622, count warnings through `yggdryl::internals::logging_warning::count`
(566) and use no s3 internals.) Sum 239 = 85 non-internals + 154 internals-gated; 5 of the 85 are the `iceberg`-gated
`accounting::iceberg` tests, so 80 need `s3` alone. **Today none of the 239 runs in the default-feature lane**
(`s3` is non-default and `s3.rs` gates everything on it): they run only in `core-tests-full`
(`.github/workflows/ci.yml:280-306`, `--all-features`). A leaf `yggdryl-s3` with no `s3` feature runs the 85 in its
default lane and all 239 in `--all-features` (`scripts/ci/plan.py:61`, `LANES`).

src files with no test file of their own (pinned through neighbours): `s3/answer.rs` (via `aws/xml.rs`, `xml.rs`),
`s3/aws/options.rs`, `s3/azure/options.rs`, `s3/google/{dialect,json,options}.rs`, `s3/request.rs`.

Tests that need a sibling feature (leaf dev-dependency or a re-home): `s3/mod_.rs` `accounting::iceberg` (5:
`what_a_table_costs_over_the_store` :379, `racing_appenders_over_the_store_each_land_every_commit_once` :600,
`a_failed_upload_publishes_nothing_and_leaves_no_staged_file` :709, `a_manifest_recording_no_length_is_not_believed` :838,
`an_unstaged_commit_costs_what_a_staged_one_costs` :907; imports `yggdryl::iceberg::{IcebergTable, PartitionSpec, ...}`
and `yggdryl::s3::S3Folder`) and `s3/file.rs` `mod parquet` (2). `s3/mod_.rs:264-937` is the pin behind the Iceberg
request table in `docs/holder/index.md:4352-4365`.

### 1.2 Cost pins in the S3 suites (request counts asserted exactly)

These are the tests the docs table (`docs/holder/index.md:4325-4346`) and `AGENTS.md` "### Object stores" table
(`AGENTS.md:1379-1389` in HEAD) say "asserted by tests". By file and test name (pinned counts live in the bodies, via
`store.requests()` and `Stats`):

- `s3/file.rs` accounting 4-423: `a_whole_read_is_one_request_and_so_is_a_ranged_one` :45,
  `a_full_stream_drain_is_one_request_not_one_per_chunk` :79, `a_whole_write_is_one_request_and_an_append_is_two` :109,
  `a_removal_is_one_request_and_absence_is_a_success` :192, `an_opened_object_stops_asking_for_its_metadata` :210,
  `a_read_teaches_an_open_handle_the_size_it_did_not_ask_for` :246,
  `a_large_write_uploads_in_parts_and_a_small_one_does_not` :261, `a_streamed_upload_reads_its_source_one_part_at_a_time`
  :308, `opening_an_object_twice_asks_once` :382, `a_ranged_digest_asks_for_the_range_and_not_the_tail` :401.
- `s3/file.rs` create 581-930 (exclusive create: `a_create_is_one_request_carrying_the_condition_its_store_reads` :621,
  `a_chunked_create_carries_the_condition_on_the_request_that_decides_the_object` :671, throttle/409/412 retry rules
  :729-:892) and tail 932-1146 (`a_tail_read_is_one_suffix_ranged_get_and_states_the_size` :983,
  `google_reads_the_same_suffix_and_azure_counts_back_from_its_size` :1080, ...).
- `s3/path.rs` accounting 3-361: `resolving_a_location_costs_one_listing_and_a_slash_costs_none` :9,
  `resolving_a_location_costs_one_listing_or_two_when_a_sibling_hides_the_prefix` :70,
  `a_parquet_read_through_a_location_is_its_listing_and_one_tail_get` :172, `whether_a_spelled_container_is_there_is_one_listing` :322.
- `s3/folder.rs` accounting 5-303: `a_prefix_streams_its_objects_for_one_listing_and_one_get_each` :43,
  `a_recursive_listing_is_one_page_not_one_request_per_directory` :232, `emptying_a_prefix_deletes_in_batches_rather_than_one_by_one` :284.
- `s3/mod_.rs` accounting 149-938: `a_text_read_of_a_glob_is_one_listing_and_one_get_per_object` :194,
  `building_a_handle_costs_nothing` :236, and the Iceberg-over-S3 counts (docs rows: create 4, append one partition 7,
  three partitions 9, upsert 11, nothing staged 7, open 5, full scan 7, pruned scan 3, projected 7, after rename 7 -
  `docs/holder/index.md:4356-4365`).
- `s3/client.rs`: backoff 945-948 `assert_eq!(backoff(1), RETRY_BACKOFF); ... backoff(20) == RETRY_BACKOFF * 64`
  (this reads `crate::http::retry`'s constants through `s3::client::internals`, `src/s3/client.rs:3208-3214`).

### 1.3 `rust/tests/interop/s3/**` - the three exchange halves (20 tests)

`rust/tests/interop.rs:12-14`: `#[cfg(feature = "s3")] #[path = "interop/s3/mod.rs"] mod s3;` (the other halves, avro,
charset, excel, iceberg, variant, zip, are siblings in the same `interop` target). `interop/s3/mod.rs:14-19` declares
`aws`, `azure`, `gcs`. The test path is therefore `s3::aws::...`, which is the filter the drivers use.
Nothing runs without an endpoint env var; the reading halves print `SKIPPED` and the drivers fail on that word
(`aws.rs` header 18-22; `azure.rs`/`gcs.rs` headers).

| File | Env var | Tests | What each proves |
| --- | --- | ---: | --- |
| `interop/s3/aws.rs` (332 lines) | `YGGDRYL_S3_ENDPOINT` | 5 | :98 `objects_written_here_are_readable_here_and_by_boto3` (awkward keys + a multipart upload, boto3 re-reads names, sizes, part count); :160 `objects_boto3_wrote_are_readable_here` (reading half, SKIPPED if absent); :208 `a_removal_here_is_a_removal_there`; :243 `a_second_create_of_an_object_is_a_conflict_and_the_first_stands` (`If-None-Match: *` on `PutObject` and `CompleteMultipartUpload`, MinIO's own 412, `checkPreconditionsPUT`); :318 `the_encryption_headers_are_what_botocore_computes` (SSE-C/KMS header names and base64 spellings, no store) |
| `interop/s3/azure.rs` (316 lines) | `YGGDRYL_AZURE_ENDPOINT` (+ `AZURE_STORAGE_ACCOUNT_NAME/KEY`) | 8 | :86 `a_signature_azurite_recomputes_is_the_one_this_client_built` (Shared Key StringToSign, 13 lines, the account twice in a path-style resource); :98 whole + ranged read; :128 `a_key_that_spells_differently_as_a_url_and_as_a_name_survives`; :151 `a_large_blob_is_staged_as_blocks_and_committed_as_a_list`; :185 second create is `409 BlobAlreadyExists`; :243 `a_listing_reads_the_enumeration_azurite_answers`; :273 `what_the_reference_client_wrote_reads_back_here`; :296 `removing_a_prefix_is_one_listing_and_one_batch` |
| `interop/s3/gcs.rs` (282 lines) | `YGGDRYL_GCS_ENDPOINT` | 7 | :70 whole + ranged read; :93 `a_key_whose_separators_are_escaped_into_one_segment_survives`; :117 `a_large_object_is_sent_as_a_resumable_session_of_chunks` (Content-Range + 308); :153 second create `412 conditionNotMet` (needs fake-gcs-server >= 1.55.0); :211 `a_listing_reads_the_json_page_the_emulator_answers`; :240 reading half; :262 prefix removal. Proves the dialect and not the identity (the emulator accepts any bearer token, header 3-9) |

They import `yggdryl::s3::{Credentials, Provider, S3Options}`, `AzureOptions`, `GoogleOptions`,
`yggdryl::s3::azure::DEVELOPMENT_ACCOUNT`/`DEVELOPMENT_KEY` (azure.rs:37, :43) and `yggdryl::s3::S3Folder`/`S3File`.

### 1.4 The shared fixture `rust/tests/support/server.rs`

`FakeS3` (3,869 lines; speaks S3, Google JSON and Azure Blob over one object set, records every request; header 1-14:
"a leaf file included with `#[path]` ... so it names nothing of the crate" - confirmed: `grep -c yggdryl` = 0).
Its inline `#[cfg(test)] mod tests` (2216) holds **25 `#[test]`**, and the file is `#[path]`-declared by **seven test
harnesses** (each executes the 25 again) and two benchmarks:

| Declared at | Gate | Uses |
| --- | --- | --- |
| `rust/tests/s3.rs:24` | `s3` | the S3 suite |
| `rust/tests/holder.rs:6` | `s3` | `holder/mod_.rs` object_store_holders |
| `rust/tests/iobase_calls.rs:19` | `s3` | `mod object_store` |
| `rust/tests/warehouse.rs:12` | `s3` | `warehouse/handle.rs`, `warehouse/media.rs` |
| `rust/tests/iceberg.rs:15` | `all(iceberg, s3)` | `iceberg/{catalog/mod_,scan,staging,table}.rs` |
| `rust/tests/s3tables.rs:33` | `s3tables` | `s3tables/catalog.rs` (9 uses) |
| `rust/tests/medallion_ledger.rs:34` | file-level `s3tables` (30) | the two-bucket ledger |
| `rust/benchmarks/holder/s3/mod.rs:20` | `s3` | `holder` bench |
| `rust/benchmarks/media/iceberg.rs:37` | `s3` | the `s3` group |

`rust/tests/support/s3tables.rs` (1,789 lines, the fake control plane, also crate-free, `S3TablesFake`) is
`s3tables`'s fixture, not S3's; it is declared by `rust/tests/s3tables.rs:27`, `rust/tests/s3tables_handle.rs:17` and
`rust/tests/medallion_ledger.rs:32`.

### 1.5 Cost pins that must NOT move (core-resident tests that drive an S3 handle)

Each drives an S3 handle: most construct `yggdryl::s3::{file_with, folder_with, S3Options, Credentials}` and `Holder::S3File/S3Folder/S3Path`; `iceberg/table.rs:3567` reaches the same client through `IcebergTable::from_url` properties:

| Test | File:line | Pinned |
| --- | --- | --- |
| `create_bytes_is_one_put_won_or_lost` | `rust/tests/iobase_calls.rs:512` (`mod object_store` 483-580) | `methods == ["PUT"]` won and lost; loser `status == 412`; `is_conflict()` |
| `a_move_between_two_objects_is_the_open_the_copy_and_the_removal` | `rust/tests/iobase_calls.rs:532` | `["HEAD","GET","GET","PUT","DELETE"]` (5), onto itself `["HEAD"]` + `Error::Conflict`, refused HEAD `["HEAD","GET"]` |
| `a_query_states_the_store_beneath_the_properties_and_leaves_the_location` | `rust/tests/holder/mod_.rs:686` (`mod object_store_holders` 665-784, imports `server::FakeS3`) | `Holder::from_url` reads `endpoint_override`/`region`/`path_style`/`anonymous` off the query; `store.request_count() == 0` on hold; `versionId` refused by name |
| `a_store_role_is_held_again_on_its_own_client_sending_nothing` | `rust/tests/holder/mod_.rs:737` | `Holder::from_handle` of `S3Folder`/`S3Path`/`S3File`: 0 requests, role kept, `Credential=AKIAIOSFODNN7EXAMPLE/` in every request |
| `a_clone_of_an_object_on_a_callers_store_folder_keeps_its_client` | `rust/tests/warehouse/handle.rs:137-` (`cfg(feature = "s3")`, test at ~139) | a clone sends nothing, reads the fake, signed with the folder's key |
| `a_tail_read_through_a_table_over_an_object_is_one_suffix_ranged_get` | `rust/tests/warehouse/media.rs:233-` | `["GET bytes=-4"]`, `["GET bytes=-10"]`, no HEAD, through `MediaTable` and warehouse `Table` |
| `a_namespace_and_a_table_are_created_and_read_back_on_an_object_store`, `what_the_catalog_costs_over_the_store` | `rust/tests/iceberg/catalog/mod_.rs:1412`, `:1514` (`mod object_store` 1364-1618; sibling `mod call_counts` 1216 pins the counting-filesystem counts, `docs/media/iceberg.md:1385-1388`) | the catalog's listing/GET counts over the store (docs `media/iceberg.md:1389-`) |
| `a_renamed_column_reads_each_data_file_once` | `rust/tests/iceberg/scan.rs:1918` (`mod object_store` 1819-1959) | one `GET` per data file after a rename |
| `an_unstaged_commit_costs_one_put_per_file_and_reads_nothing_back` | `rust/tests/iceberg/staging.rs:325` (`mod object_store` 246-367) | `WriteStaging::Off` = one PUT per file |
| `a_table_on_an_object_store_states_what_the_store_did_not_read` | `rust/tests/iceberg/table.rs:3567` (`cfg(feature = "s3")`) | open by location = hint + document (both spellings) + next version's two spellings, `404`s |

`iobase_calls.rs` has 41 tests in all; `holder/mod_.rs` 31; the S3 share of each is 2. Per the handoff invariant
(`MARKET_SPLIT_NEXT.md` "Invariants": "a cost pin ... is never re-pinned from a sweep") these keep their assertions.

### 1.6 Other tests reaching across the s3 boundary

- `rust/tests/aws/environment.rs:169-` `mod internal` (`cfg(all(feature = "internals", feature = "s3"))`, 5 tests:
  :215, :222, :234, :252, :304). Only :252 `a_key_spelling_botocore_never_reads_is_kept_from_the_sweep` uses
  `yggdryl::internals::s3_properties::swept` and `yggdryl::s3::S3Options` (253-254, 289, 294); the other four read
  `aws_environment::is_native` and need `aws` + `internals` only.
- `rust/tests/aws/sigv4.rs`: three `cfg(feature = "s3")` items - `use yggdryl::internals::aws_sigv4::encode_key`
  (15), `put_object_matches_the_aws_example` (127), `encode_key_keeps_separators_and_unreserved_bytes_and_escapes_the_rest`
  (253). `encode_key` is `rust/src/aws/sigv4.rs:81-82` and `:182`, `:452`, all `cfg(feature = "s3")`, used by
  `s3/azure/dialect.rs:122-129`. The file's header (1) says it pins `rust/tests/s3/client.rs` and `rust/tests/aws/request.rs`.
- `rust/tests/xml.rs:16` gates `xml/scanner.rs` (13 tests) on `all(feature = "s3", feature = "internals")` while the
  source gate is `cfg(feature = "aws")` (`rust/src/xml/mod.rs:57-58`; users: `aws/sts.rs:18`, `s3/xml.rs:10`).
- `rust/tests/http/retry.rs` (9 tests, `internals::http_retry`): header 6-7 "The S3 client reads the same rules, and
  `rust/tests/s3/client.rs` pins that it does." The S3 client reads `crate::http::retry::{self, RETRY_COST,
  RETRY_REFUND, RetryBudget, fresh_jitter, is_resumable, is_retryable_transport, is_unsent}` (`s3/client.rs:35-38`),
  `crate::http::HttpOptions::DEFAULT_MAX_PAUSE` (:44; pinned by `http/options.rs:101`), `crate::http::client::agent_for`
  (:431), `crate::http::record_process` (:1158, :1274; `http/mod.rs:82-83` re-exports it under `cfg(feature = "s3")`)
  and `crate::http::render_http_date` (`s3/azure/sign.rs:215`; public, pinned by `http/headers/date.rs:172-188`).
  Nothing in `rust/tests/http/**` names the S3 client otherwise (the `s3://` hits at `http/options.rs:348-349`,
  `http/request.rs:47`, `http/netrc.rs` are literals).
- `rust/tests/root/error.rs:63-77` `a_remote_refusal_names_the_store_the_operation_and_the_verdict` builds
  `Error::remote("s3", "GetObject", 403, ...)`: pins `Error::Remote` formatting, core, stays.
- `rust/tests/fs/location.rs:101,118`, `rust/tests/uri/arn.rs`, `rust/tests/uri/mod_.rs:271`: URL/ARN grammar and the
  bridged `FileSystem` named "s3"; core, untouched (the `Arn` values are `uri/arn.rs`, not the backend).
- `rust/tests/allocations.rs`: **no S3 row** (the only `s3` hit is the literal `"s3://warehouse/table/data/part-{index:05}.parquet"`
  in an Iceberg manifest fixture, :11177). Nothing to re-pin.
- `rust/tests/benchmark_mode.rs`: nothing S3-specific.

### 1.7 S3 Tables tests that sit on the object-store fake (belong with `yggdryl-iceberg`, need `yggdryl-s3`)

`rust/tests/s3tables/catalog.rs` 19 tests (FakeS3 in 9 places: :21, :38-46, :54, :151, :168, :686, :717), `bucket.rs` 9,
`client.rs` 33, `listing.rs` 10, `mod_.rs` 4, `namespace.rs` 8, `table.rs` 14, `live.rs` 1 (ignored; `use
yggdryl::s3::S3Options`, :23), `rust/tests/s3tables_handle.rs` 1, `rust/tests/medallion_ledger.rs` 1 (FakeS3 at :44, :113,
:247, :261). The doc sentence `docs/media/iceberg.md:1439` says each table's warehouse is "a bucket of the fake object
store the `s3` suites run on, whose log shows no listing and no delete".

### 1.8 Benchmarks

- `rust/benchmarks/holder.rs:26-30` `#[cfg(feature = "s3")] #[path = "holder/s3/mod.rs"] mod s3;` with the stub module at
  :41-54 for `not(s3)`; `criterion_group!` lists `s3::bytes::byte_benchmarks`, `s3::listing::listing_benchmarks`,
  `s3::records::record_benchmarks` (:78-80). Files `holder/s3/{mod,bytes,listing,records}.rs`, 474 lines. Groups:
  `object_bytes` (read_all, read_footer, stream_drain, write_all x yggdryl / yggdryl_unsigned_payload / object_store - 9
  legs), `object_listing` (first_entries, drain_recursive, drain_level x 2 - 6 legs), `object_records`
  (`read/{arrows,parquet}`, `read_opened`, `overwrite`; the parquet leg `cfg!(feature = "parquet")`, records.rs:64-67).
  Baseline `object_store` (`rust/Cargo.toml:201` `object_store = { version = "=0.13.1", features = ["aws"] }`, with
  `futures` :197 and `tokio` :210 as dev-deps for it).
- `rust/benchmarks/media/iceberg.rs:1861-2216` `mod s3` (group `s3`: commit legs, `upsert/one_partition_of_8`,
  `scan/full_8_partitions`, `scan/pruned_1_of_8`, `log/text_read`), `:2218 #[cfg(feature = "s3")] s3::benchmarks(criterion)`;
  its doc comment says the counts "are the ones `rust/tests/s3/mod_.rs` pins" (:1853-1860); the `FakeS3` include is :36-37.
- `rust/benchmarks/holder/aws.rs` mentions `s3tables`/`s3` only in profile fixtures (identity group, stays with `aws`).
- `rust/Cargo.toml` `[[bench]] holder` has no `required-features`; the comment at :292-297 states the S3 groups are
  compiled "only with the `s3` feature".

### 1.9 Binding tests naming the S3 roles (the roles stay in the one native module, D11 option d)

`python/tests/holder/test_init.py` (35 tests; 10 touch S3: `test_store_options_cross_as_keywords_too` :434,
`test_each_role_commits_to_what_it_is`, `test_a_bucket_and_a_raw_key_name_an_object_a_url_cannot_spell`,
`test_the_ordinary_constructor_answers_the_store_role_it_reached`, `test_a_handle_held_again_keeps_its_role_on_its_own_client`
:503, `test_every_object_store_spelling_selects_this_backend` :545, `test_a_bare_azure_container_takes_its_account_from_the_options`,
`test_a_location_naming_no_container_is_refused`, `test_options_are_read_in_whichever_vocabulary_they_are_written`,
`test_an_option_that_will_not_parse_is_an_argument_error`); `python/tests/holder/test_fs.py` (57; 7 touch S3, :488-1049,
pyarrow `S3FileSystem` classification); `python/tests/typing_bindings.py:125,659`; Node: `node/tests/iobase.test.js` (3
refs), `node/src/iobase.rs` (2). `python/Cargo.toml:22-27` and `node/Cargo.toml:22-27` both enable
`features = ["http3", "iceberg", "s3", "s3tables"]` on `yggdryl`.

### 1.10 `yggdryl::internals` plumbing for s3

`rust/src/lib.rs:528-549` re-exports 12 s3 internals under `cfg(feature = "s3")`: `s3_answer`, `s3_aws_xml`,
`s3_azure_auth`, `s3_azure_dialect`, `s3_azure_sign`, `s3_azure_xml`, `s3_client`, `s3_file`, `s3_google_token`,
`s3_options`, `s3_properties`, `s3_xml` (`s3/options.rs:550` and the eleven other `pub mod internals` blocks listed by
`git grep -n 'pub mod internals' HEAD -- rust/src/s3`). They are *generated* by `scripts/generate_internals.py`, whose
`SRC = ROOT/"rust"/"src"` (line 32) and `guards()` walk only the core `lib.rs`; a leaf's internals are outside it today.

---------------------------------------------------------------------------------------------------------------

## LIST 2 - EXCHANGE SCRIPTS, CI ROWS, WORKFLOW, MANIFESTS

### 2.1 The three drivers

| Script | Lines | Server fetched | Reference client | Rust half command | Env | Port | Scratch |
| --- | ---: | --- | --- | --- | --- | ---: | --- |
| `scripts/check_object_interop.py` | 397 | MinIO `RELEASE.2025-04-22T22-12-26Z` from `github.com/minio/minio/releases/download/...` (pinned :101-116; `dl.min.io` answers 410) | `boto3` (`>=1.34` in ci.yml:354) | `cargo test --locked --manifest-path rust/Cargo.toml --features s3 --test interop s3::aws:: -- --nocapture --test-threads=1` (:334-350; **no `cwd`**) | `YGGDRYL_S3_ENDPOINT`, `AWS_ACCESS_KEY_ID`/`SECRET`/`REGION` | 9123 | `rust/target/s3-interop` (:56) |
| `scripts/check_azure_interop.py` | 265 | Azurite `azurite@3` via `npm install` (:104-110), `azurite-blob --skipApiVersionCheck` (:131) | `azure-storage-blob>=12` | same, `... --features s3 --test interop s3::azure:: ...` (:208-224), `cwd=REPO` | `YGGDRYL_AZURE_ENDPOINT`, `AZURE_STORAGE_ACCOUNT_NAME/KEY` (Microsoft's published dev account) | 10123 | `rust/target/azure-interop` (:58) |
| `scripts/check_gcs_interop.py` | 265 | `fake-gcs-server` 1.56.1 from `github.com/fsouza/fake-gcs-server/releases/download/v1.56.1/...tar.gz` (:71, :101) | `google-cloud-storage>=2` | same, `... --features s3 --test interop s3::gcs:: ...` (:212-228), `cwd=REPO` | `YGGDRYL_GCS_ENDPOINT` | 4499 | `rust/target/gcs-interop` (:61) |

All three: drive the reference client to write, run the Rust half, fail on `"SKIPPED" in stdout`, then re-read with the
reference client (aws :365-366; azure :253-254; gcs :251-252). aws additionally compares the SSE header set captured from
`botocore` (docstring item 5, `check_encryption_headers`). The cargo line is the only coupling to the core crate: the manifest path,
`--features s3`, `--test interop`, and the `s3::<dialect>::` filter. Other drivers for comparison: `check_zip_interop.py`,
`check_avro_interop.py`, `check_excel_interop.py` (default features, `--test interop`), `check_iceberg_interop.py`
(`--features "parquet iceberg"` from `rust/`).

### 2.2 `.github/ci/rows.toml` (HEAD, 282 lines) - the rows, quoted

```toml
# rows.toml:90-97
[leaves]
# market = { package = "yggdryl-market" }
# fix = { package = "yggdryl-fix", after = ["market"] }
# avro = { package = "yggdryl-avro", jobs = ["avro-interop"] }
# parquet = { package = "yggdryl-parquet", jobs = ["pyiceberg-interop", "spark-interop"] }
# iceberg = { package = "yggdryl-iceberg", after = ["avro", "parquet"], msrv = "--features s3tables", jobs = ["pyiceberg-interop", "spark-interop", "iceberg-msrv"] }
# excel = { package = "yggdryl-excel", jobs = ["excel-interop"] }
# xmla = { package = "yggdryl-xmla" }

# rows.toml:104-105
[leaf]
jobs = ["fmt", "inventory", "docs-rust", "python-wheel", "python", "node"]

# rows.toml:243-282
# One `interop` target holds every exchange's Rust half, so a change to any
# of them can break the build of all seven.
[rows.interop]
extends = ["corelib"]
paths = ["rust/tests/interop.rs", "rust/tests/interop/**"]
[rows.x-zip]      extends = ["interop"]  paths = ["scripts/check_zip_interop.py"]     jobs = ["zip-interop"]
[rows.x-avro]     extends = ["interop"]  paths = ["scripts/check_avro_interop.py"]    jobs = ["avro-interop"]
[rows.x-excel]    extends = ["interop"]  paths = ["scripts/check_excel_interop.py"]   jobs = ["excel-interop"]
[rows.x-iceberg]  extends = ["interop"]  paths = ["scripts/check_iceberg_interop.py"] jobs = ["pyiceberg-interop"]
[rows.x-s3]       extends = ["interop"]  paths = ["scripts/check_object_interop.py"] jobs = ["object-interop"]   # 269-272
[rows.x-azure]    extends = ["interop"]  paths = ["scripts/check_azure_interop.py"]  jobs = ["azure-interop"]    # 274-277
[rows.x-gcs]      extends = ["interop"]  paths = ["scripts/check_gcs_interop.py"]    jobs = ["gcs-interop"]      # 279-282
```
(each `[rows.x-*]` is four lines in the file - header, `extends`, `paths`, `jobs` - shown on one line here). Also relevant: `[rows.core]` 136-139
(`paths = ["rust/tests/**", "rust/benchmarks/**", "rust/examples/**", "config/**"]`, jobs `lint-default`,
`iceberg-msrv`, `core-tests-default`, `core-tests-full`); `[rows.corelib]` 130-132 (`rust/Cargo.toml`, `rust/src/**`);
`[rows.inventory]` 232-241 (`extends = ["corelib", "crates"]`); `[rows.docs-rust]` 218-220; `[shards.yggdryl]` 72-74
(names `fix` and `allocations` only - no s3 shard to take along); `inert` 28-51.

How a leaf row maps to its exchanges (`scripts/ci/plan.py`): the pseudo-row `crates` (:53) expands to every
`crate-<leaf>` (:108-110); a leaf line becomes (:167-172)

```python
rows[row] = Row(
    name=row,
    extends=("corelib",) + tuple(crate_row(upstream) for upstream in leaf.after),
    paths=(f"rust/{name}/**", "config/**"),
    jobs=tuple(dict.fromkeys((LEAVES_JOB, *leaf_jobs, *leaf.jobs))),
)
```
i.e. a change under `rust/s3/**` runs `leaves` (clippy/rustdoc on `-p yggdryl-s3`, tests in both lanes, `rest` shard
unless `[shards.yggdryl-s3]`), the `[leaf]` jobs (fmt, inventory, docs-rust, python-wheel, python, node) and the line's
`jobs`. `check_leaves` (:348-373) requires `rust/<name>/Cargo.toml` <-> a line, and `Cargo.toml` `members` to list it
(`MEMBERS = {"rust","python","node","cli"}` :57; `CORE_FOLDERS` :55 includes `target`).

**What a leaf `s3` needs there**

1. A `[leaves]` line, uncommented in the commit that creates `rust/s3/Cargo.toml`:
   `s3 = { package = "yggdryl-s3", jobs = ["object-interop", "azure-interop", "gcs-interop"] }`. If `yggdryl-iceberg`
   (`s3tables` implies `s3` today, `rust/Cargo.toml:57`) depends on it: `after = ["avro", "parquet", "s3"]` on the
   iceberg line and its `msrv = "--features s3tables"` then builds `yggdryl-s3` at 1.94 (a change under `rust/s3/**`
   reruns iceberg's leaf lanes through `extends`, :169).
2. `[rows.x-s3]`, `[rows.x-azure]`, `[rows.x-gcs]` stop extending `interop` (whose paths are `rust/tests/interop*`; those
   Rust halves leave the core) and extend `crate-s3` instead (so the row fingerprint reads `rust/s3/**`). `extends =
   ["crate-s3"]` is a "no row" error until the `[leaves]` line is live (`plan.py:112-115`, `resolved`), so row and line flip in one commit.
3. `interop` row and its comment ("...all seven") shrink to the four that remain (zip, avro, excel, iceberg until they leave).
4. No `[shards.yggdryl-s3]` needed at first (`rest` = `--all-targets`).

### 2.3 `.github/workflows/ci.yml` (HEAD, 1070 lines) - what names s3

- `core-interop` 217-243: comment 219-222 ("`--features s3` for S3, Azure and Google ... The full lane's cache is the one
  with `s3` and Iceberg compiled"); step 234-235 `cargo test --locked --manifest-path rust/Cargo.toml --features s3
  --test interop --no-run` ("Build the S3, Azure and Google exchanges' Rust half"); step 237-238 PyIceberg half
  `--features "parquet iceberg" --test interop --no-run` from `rust/`; packs `lane-interop`.
- `object-interop` 329-356, `azure-interop` 358-386, `gcs-interop` 388-413: each `needs: [changes, core-interop]`,
  `if: contains(needs.changes.outputs.jobs, ' <name> ')`, `rust-lane` `mode: restore`, `cache: stable-full`,
  `lane: interop`; installs `boto3>=1.34` (:354) / `azure-storage-blob>=12` (:384; also `setup-node` 22 :374-376) /
  `google-cloud-storage>=2` (:411); runs the script. Comments 330-335 and 344-346 ("The driver runs the `interop` target
  with `--features s3`, which the exchange lane built").
- `core-full` 194-215 builds `--all-features --test docs_index` (the cache the exchanges restore).
- `lint-full` 131-153 runs `--workspace --all-features` clippy/doc: covers a leaf as a workspace member.
- `docs-rust` 877-895: comment 870-876 "compiled against `parquet iceberg s3 s3tables http3`"; the command is
  `python scripts/check_docs_examples.py --lang rust`.
- `leaves` 959-1024: "none until S4" (comment 960-967); matrix `fromJSON(needs.changes.outputs.leaves)`; `setup` mode
  (builds in place over `leaf-<name>` cache, no core lane); steps clippy (`--no-deps -D warnings`), tests via
  `plan.py targets "$PACKAGE" "$LANE" "$SHARD"`, rustdoc examples, `cargo doc`, `cargo check --profile bench --benches`.
- `ci` gate 1026-1042: `needs:` lists `object-interop, azure-interop, gcs-interop` (1038); unchanged by a move.
- `release.yml` publishes `-p yggdryl` only (199, 864); a ninth crate adds to preflight/publish and to AGENTS §6's name list.

### 2.4 Planner and its tests - what a leaf named `s3` trips

- `scripts/tests/test_ci_plan.py:37` `LEAF_LINE = re.compile(r"^(?:# )?([a-z]+) = \{ package = .*\}$", re.MULTILINE)`:
  `[a-z]+` does not match `s3`, so `with_leaves("s3")` asserts "rows.toml has no `[leaves]` line for `s3`" (:62-63).
  Needs `[a-z0-9]+`.
- `TheLeaves.EXCHANGES` (:312-320) lists seven leaves (`avro` :316 ...); `s3` is missing, so the per-leaf loop (:329-343)
  would not cover it.
- `NEVER` (:322-326) includes `"object-interop", "azure-interop", "gcs-interop", "zip-interop"` and
  `assertFalse(self.NEVER & result.jobs)` (:339) runs for **every** leaf: with an `s3` leaf running those three, that
  assertion fails for `s3` unless `NEVER` is computed per leaf.
- `test_an_exchange_driver_runs_its_exchange` (:255-257): `planned(["scripts/check_object_interop.py"]).jobs ==
  {"object-interop", "core-interop"}`; after the move the exchange no longer needs `core-interop` (its Rust half is the
  leaf's), so that set changes.
- `test_an_exchange_half_runs_every_exchange` (:259-262): a change to `rust/tests/interop/zip.rs` is asserted to run
  `object-interop`, `azure-interop`, `gcs-interop` among seven; after the move it must not.
- `scripts/ci/plan.py:503` comment "seven leaves inside the repository's cache budget" (one cache per leaf,
  `leaf-<name>`, :507-511).

### 2.5 Manifests and tools that carry the feature today

- `rust/Cargo.toml:49-50,57`: `aws = ["http", "dep:hmac", "dep:ring", "dep:sha2"]`, `s3 = ["aws", "dep:md-5"]`,
  `s3tables = ["s3", "iceberg"]`; `md-5` (:93) and the Google JWT use of `ring` (:108-113) are s3's; dev-deps
  `object_store`, `futures`, `tokio` (:191-210) are the S3 benchmark baseline.
- `python/Cargo.toml:22-27`, `node/Cargo.toml:22-27`: `features = ["http3", "iceberg", "s3", "s3tables"]`.
- `scripts/check_docs_examples.py:209`: `["cargo", "test", "--locked", "--features", "parquet iceberg s3 s3tables http3",
  "--test", "docs_examples"]` with `cwd=ROOT`; the generated `rust/tests/docs_examples.rs` compiles every Rust fence;
  12 fences name `yggdryl::s3` (3.1), so the target needs the leaf as a dev-dependency and `yggdryl_s3::` paths.
- `scripts/check_api_inventory.py:101-103` resolves a section's crate as the path up to `src`, so a
  `### ... [rust/s3/src/...]` header checks; but the omitted-names count scans only `rust/src` (:324).
- `scripts/generate_internals.py:32` core-only (1.10).
- `.github/actions/rust-lane/action.yml`: lane pack/restore of `target/` files newer than a mark; no s3 text.

---------------------------------------------------------------------------------------------------------------

## LIST 3 - DOCS, SKILLS, AGENTS.md, INVENTORIES

### 3.1 `docs/holder/index.md` (HEAD 6,893 lines) - "## Object stores" 4255-5172 (918 lines)

Subsection map: intro + 3 tabs 4255-4319 (Rust `no_run` 4269-4288, Python `ignore` 4292-4309, JavaScript `ignore`
4313-4319); "What each operation costs" 4321-4368; "Naming an object" 4369-4410; "Configuration" 4411-4526;
"AWS identity" 4527-5043, of which 4527-5007 (481 lines) document `yggdryl::aws` and **stay core**, while its last `####`, "Google and Azure" 5008-5043, is S3-crate;
"Encryption at rest" 5044-5095; "Retries and failures" 5096-5130; "Object stores performance" 5131-5172.
30 tagged fences in the section: 12 Rust blocks name the s3 API (4269 no_run, 4373, 4386, 4415, 4430, 4459, 4569, 4638
no_run, 5012, 5024, 5056, 5112 no_run), 9 of them executed by `check_docs_examples.py --lang rust`. Two of those
(4569-4591 and 4638-4658) combine `yggdryl::aws::{Session, AssumedRole, SsoLogin}` with `S3Options::with_session`/`s3::file_with`.

Sentences that state the shape (quote, HEAD line):
- 4257: "`S3Path`, `S3Folder` and `S3File` reach Amazon S3 ..., Google Cloud Storage and Azure Blob Storage ... Behind the
  non-default `s3` feature. The scheme picks the store: `s3`/`s3a`/`s3n`, `gs`/`gcs`, `az`/`abfs`/`abfss`/`wasb`/`wasbs`".
- 4323: "The request count is the contract, asserted by tests."; the table is 4325-4346 (header + 20 rows: building a handle
  none; resolving a `lake/` location none; other location one single-key listing or two; ranged read; tail read
  (S3/Google one suffix `GET`, Azure `HEAD` + ranged `GET`); whole read; text/CSV read; `size`; whole write one `PUT` /
  `POST` / `PUT`; large write `parts + 2` / `chunks + 1` / `blocks + 1` (4339); exclusive create (4340); append (4341);
  removal; **move onto another object five requests** (4343); listing; prefix stream; emptying a prefix 1000 / 100 / 256 (4346)).
- 4348 (a recursive listing is one flat listing; `S3File::with_known_size`; a move is copy + removal, "`CopyObject` is not what a move does yet") and 4350 (the corrected bucket region kept on the shared session, "64 buckets at most"; the Google bearer token held per credential source and scope).
- 4352: "An [Iceberg] table over the store costs what `accounting::iceberg` in `rust/tests/s3/mod_.rs` pins against the
  in-process store" + table 4354-4365 (create 4, append 7, ... projected scan after a column rename 7).
- 5098 (retries): token budget 500, `Retry-After` up to 30 s; 5131-5172 performance table (8 rows vs `object_store` 0.13.2,
  "Criterion medians on a containerized x86_64 Linux host (Intel Xeon @ 2.10 GHz, 4 cores)"), 5162
  `cargo bench --bench holder --features s3 -- object_ --noplot`, 5165-5170 the three `scripts/check_*_interop.py` lines
  (a ```console block 5167-5171 with the three commands).
Elsewhere in the page: 16 (overview table row `s3` feature), 40 (`s3:`, `gs:`, `az:` location row), 96-97 (variants table:
`S3Folder`, `S3Path`, `S3File`), 382, 1400 (`create_bytes` per store), 3105 ("`http::process_stats()` ... the object stores'
and a catalog service's alike"), 3768-4016 (bridged-filesystem classification, Python blocks 3778 and 3987 executed),
4067, 4081. The other agent's working-tree edit of this page ends at HEAD line 2098, before every region above.

### 3.2 Other docs

| File | Lines (HEAD) | What it says |
| --- | --- | --- |
| `docs/media/iceberg.md` (1,931) | 211, 1389-1392 ("Over an object store the same catalog costs requests, pinned in the same file (`mod object_store`, `what_the_catalog_costs_over_the_store`)"), 1414-1440 ("Iceberg on Amazon S3 Tables": "behind the `s3tables` feature (which implies `s3` and `iceberg`)"; request table 1424-1437: ListTableBuckets per page, GetTableMetadataLocation per table, CreateTable ..., DeleteTable; 1439 "The counts are pinned in `rust/tests/s3tables/catalog.rs` ... the fake object store the `s3` suites run on"), 1456 (`FakeS3` in the pipeline count), 1547-1568, 1691-1800 (`yggdryl::s3tables::S3Tables`), 1905-1922 ("Iceberg over S3", `cargo bench --features "iceberg s3" -p yggdryl --bench media -- 's3/' --quick`), 1925-1927 (PyIceberg on S3 Tables, `python/benchmarks/media/s3tables.py`) |
| `docs/warehouse/index.md` | 9, 1057, 1136, 1177 ("An object rooted on a caller's own object-store ... handle - an `S3Folder` built with its endpoint ... keeps that handle's client") |
| `docs/testing.md` | 75 `cargo test -p yggdryl --all-features --test s3` (and 76 `--test s3tables`), 94 `--test interop`, 212-214 + 224-226 the three drivers and their one-line proofs, 251-258 the leaf/exchange table (avro, parquet, iceberg, excel, "market, fix, xmla: none"), 272 "an in-process S3" in the `support/` row |
| `docs/benchmarks.md` | 21 (Holder row: "Both clients against one in-process store ... beside `object_store` 0.13.2"), 72 `cargo bench --bench holder --features "parquet s3"` |
| `docs/architecture.md` | 32 (layout: "one folder per backend - `local/`, `fs/`, `zip/`, `s3/` ... `aws/`, ... Signature Version 4 every S3 request signs with"), 240 (feature row `s3` "implies `aws`"), 241 (`s3tables` "implies `s3` and `iceberg`"), 244 "Every build - the default one, `s3`, `iceberg` and both bindings - compiles on Rust 1.94" |
| `docs/contributing.md` | 38 ("the exchanges with MinIO, Azurite, fake-gcs-server, `zipfile`, fastavro, openpyxl, PyIceberg and Spark"), 50 (the `rust/src/...` -> Holder page mapping: "one root folder per backend: `rust/src/local/`, `fs/`, `zip/`, `s3/`") |
| `docs/expression/plans.md` | 221 "an object-store scheme needs the `s3` feature and reads its credentials from the properties" |
| `README.md` | 16 ("the S3, Google Cloud Storage, and Azure Blob object stores behind the `s3` feature"), 66 (`src/{local,fs,zip,s3,http}/`) |
| `rust/README.md` | 15 (`src/s3/  S3Path, S3Folder, and S3File over the S3 dialect`) |
| `docs/uri/arn.md`, `docs/uri/*.md` | only `arn:aws:s3:::` ARN strings: URI grammar, **core**, no change |

Skills (HEAD): `skills/yggdryl-storage/SKILL.md` 8 matching lines (3, 58, 66, 75, 116, 223, 224, 229: the `s3`-feature door
row 58 `s3::file(url)?`, `S3Options`, `S3File::with_known_size`), `references/backends.md` 14 (13, 45, 53, 58, 72, 81, 97, 111,
145-148, 190-192), `references/rust.md` 14 (4, 495-533 one executed Rust block `use yggdryl::s3::{self, Provider, S3Options}`,
541-542, 704), `references/python.md` 21 (449-470 and 499, 547-566, 655-687: four executed Python blocks), `references/javascript.md`
2 (627-628); `skills/yggdryl-uri/SKILL.md:145`; feature-table lines naming `s3 (implies aws)`: `skills/yggdryl/SKILL.md:21,26,149`,
`skills/yggdryl/references/rust.md:17,28`; `skills/yggdryl-expressions/references/rust.md:495`; S3 Tables doors in
`skills/yggdryl-warehouse/SKILL.md` 31-33, 90, 150-158 and `skills/yggdryl-records/SKILL.md:58` (iceberg-owned).

### 3.3 `AGENTS.md` (HEAD, 2,756 lines; the working tree has 61 changed lines elsewhere)

| Where | Lines | S3-bearing text |
| --- | --- | --- |
| Layout features line | 309-312 | "features are `default = []`, `parquet`, `iceberg` (implies `parquet`), `http`, `http2` ..., `aws` (implies `http`), `s3` (implies `aws`), `s3tables` (implies `s3` and `iceberg`)" |
| `holder/` row | 376 | "the local, ZIP, object-store and HTTP backends stay `Holder::from_url`'s own arms, and a scheme no backend holds and no locator claims is refused naming the crate to install" |
| `auth/` row | 378 | `Bearer` "under `s3` for the two dialects that hand one"; "`aws/`, `s3/google/` and `s3/azure/` carry only where their answer comes from" |
| `aws/` row | 379 | `environment.rs` "(under `s3`) the variables the session reads for itself and the S3 sweep leaves to it"; `sigv4.rs` S3 family canonical URI; `properties.rs` "which `S3Options::with_properties` hands its identity names to"; `answers_another` "shared by the S3 client's own transport and `with_sigv4`" |
| `xml/` row | 380 | scanner "for the small fixed-shape XML documents S3, Azure Blob Storage and STS answer" |
| `local/`, `fs/`, `zip/`, `s3/` row | 381 | "one root folder per storage backend ... `S3Path`, `S3Folder`, `S3File` in `local/`, `fs/` and `s3/` ... `s3/` holds Amazon S3, Google Cloud Storage and Azure Blob Storage inside it, since all three answer that dialect, under the non-default `s3` feature" |
| `http/` row | 382 | `retry.rs` "crate-private, the retry budget, backoff and verdicts the S3 client draws on too" |
| `s3tables/` row | 391 | "(`s3` and `iceberg`)" |
| "Common changes" storage-backend row | 161 | "`<name>/` at the root with a location/container/leaf trio ... state and assert its call/request counts -> interop script -> docs" |
| Smoke loop | 186 | "a gated path works | ... `--features s3`" |
| Test layout | 436-446 | support fixtures "declared by each of them"; `interop/` own target |
| Zip/HTTP/Object stores | 1158, 1184, **1342-1413** | `### Object stores (`s3/`, non-default `s3` feature)` - 72 lines: Provider dispatcher, session-driven signing, refused-key resend, `xml.rs`/`answer.rs`, the 9-line request-count table (header 1379, rows 1381-1389), region/redirect/bearer/payload-signing/conflict paragraph |
| §2 CI table | 2233, 2237, 2238 | "Core build (... exchange features)"; "S3 / Azure / Google exchange | the object stores against MinIO with boto3, Azurite ..., fake-gcs-server ... | `python scripts/check_object_interop.py`, `check_azure_interop.py`, `check_gcs_interop.py`"; "ZIP / Avro / Excel exchange" |
| §2 leaf paragraph | 2262-2296 | leaf rule + exchanges table `avro`/`parquet`/`iceberg`/`excel`/`market, fix, xmla` |
| §5 docs | 2644, 2701 | "Local, Filesystems, Object stores, Buffered, ZIP"; `check_docs_examples.py --lang rust # compiled against parquet iceberg s3 http3` |

### 3.4 Inventories

`.api-inventory.txt` (HEAD, 8,436 lines): `### yggdryl::s3  [rust/src/s3/mod.rs]` **2041-2130** (90 lines, 81 non-blank, 58
`pub` lines: 12 constructors `located`...`path_at_with`, `Provider` + 13 const fns, the three roles, `StatsSnapshot`,
`S3Options` with `PROPERTY_NAMES: [&str; 47]`, `AwsOptions`/`Checksum`, `GoogleOptions` (`DEFAULT_SCOPE`), `AzureOptions`
(`BlobType`, `DEFAULT_API_VERSION`, `DEVELOPMENT_ACCOUNT`, `DEVELOPMENT_KEY`), `pub use crate::aws::Credentials`, `Encryption`,
`KmsKey`, `CustomerKey`); `### yggdryl::aws  [rust/src/aws/mod.rs]` **2131-2184** (54 lines, 32 `pub`; stays core; 2132
"behind the `aws` feature, which `s3` implies"; 2150 `endpoint_url`, 2152 `PROPERTY_NAMES: [&str; 32]`, 2154 `with_properties`
"S3Options::with_properties delegates its identity names to it"); `### yggdryl::s3tables [rust/src/s3tables/mod.rs]` **2185-2252** (68 lines, 59
`pub`; iceberg-owned). Outside those: 2254 ("`http` feature, which `aws` and `s3` imply"), 2530 `with_sigv4`, **2735**
`S3Folder(crate::s3::S3Folder), S3Path(crate::s3::S3Path), S3File(crate::s3::S3File)  (behind the `s3` feature)` (the
`Holder` variants), 2746-2747 (`Holder::from_url` / `from_handle` text), 2996 (`read_tail_bytes` per store), 3554
(`IcebergTable::from_url`), 4537 `is_s3_tables`, 5669 `as_s3_mut` (the `S3` metadata scheme protocol view, `metadata.rs:212`,
core). `scripts/check_api_inventory.py` header-resolves `[rust/s3/src/...]` (2.5).

`.api-bindings.txt` (HEAD, 1,018 lines): Python `holder:` export list **line 141** (`..., LocalPath, S3File, S3Folder,
S3Path`), role notes **146** (`FsPath`... "a filesystem this build holds natively - LocalFileSystem, S3FileSystem,
GcsFileSystem, AzureFileSystem ... answers ... S3Path / S3File / S3Folder") and **147** (`S3Path / S3File / S3Folder: the same
three roles on Amazon S3, Google Cloud Storage, and Azure Blob Storage, each constructible as (location) or as (container,
key, provider=) ... options= mapping in PyIceberg's s3./gcs./adls. names ...`); 69 (`Url(value)` resolves an Amazon S3 Arn),
73 (Arn bucket), 132. JavaScript section has no S3 class (Node reaches S3 only through `IOBase(url)`, lines 538, 554).
No S3 line in either inventory is a binding-only entry that the move changes if the classes stay in the one native module.

---------------------------------------------------------------------------------------------------------------

## WHAT MOVING `s3/` OUT WOULD HAVE TO RE-SPELL

### A. Tests that move with the crate (to `rust/s3/tests/`, mirror rule kept)

1. `rust/tests/s3.rs` (65 lines) and `rust/tests/s3/**` (19 files, 239 tests) -> `rust/s3/tests/**` under the leaf's own
   mirror rule (AGENTS §1 "Where a test lives": one harness per top-level source entry of `rust/s3/src`, `root` for the
   files its `lib.rs` holds); the `cfg(feature = "s3")` on the 13 module lines and the server line vanishes (the crate is the
   feature); `internals` stays a crate feature of the leaf and `yggdryl::internals::s3_*` (12 re-exports, `lib.rs:528-549`)
   becomes `yggdryl_s3::internals::*` with a generator that walks `rust/s3/src` (today `generate_internals.py:32` is core-only).
   Default-lane coverage grows from 0 to 85 tests; full lane stays 239.
2. `rust/tests/interop/s3/**` (3 files, 20 tests) -> `rust/s3/tests/interop/**` with an `interop` target of the leaf;
   drop `rust/tests/interop.rs:12-14`. The filter strings `s3::aws::` etc. change with the new module nesting.
3. `rust/tests/support/server.rs` (25 inline tests, crate-free): either stays as the single shared fixture (`#[path]`
   from the seven harnesses + two benchmarks, which already do that with `../../tests/support/server.rs`) or moves to
   `rust/s3/tests/support/` and the six non-s3 includers point at it; each declaring harness re-runs its 25 tests.
4. `rust/benchmarks/holder/s3/**` (4 files, 474 lines) + `rust/benchmarks/media/iceberg.rs` `mod s3` (1861-2216) +
   the `object_store`/`futures`/`tokio` dev-deps (`rust/Cargo.toml:191-210`); `holder.rs:26-54` stub block and
   `criterion_group!` lines 78-80 lose the `s3::*` entries.
5. The 5 `accounting::iceberg` tests (`s3/mod_.rs:264-937`) and 2 `file.rs` `mod parquet` tests need `yggdryl-iceberg` /
   `yggdryl-parquet`; either dev-dependencies of `yggdryl-s3` (a dev-dependency cycle with `yggdryl-iceberg[s3tables]`) or
   re-homed to the iceberg/parquet crates, with `docs/holder/index.md:4352` re-pointed.
6. `rust/tests/aws/environment.rs` `mod internal` :252 (the `swept` sweep test) and `rust/tests/aws/sigv4.rs` :15, :127,
   :253 (`encode_key`, `put_object_matches_the_aws_example`) follow `encode_key` (`aws/sigv4.rs:81`) and `swept`
   (`s3/properties.rs`) to the leaf, or the helpers stay core and the `s3` gates become `aws`.
7. `rust/tests/xml.rs:16`: scanner tests gate `s3` -> `aws` (the source gate is already `aws`).

### B. Pins that stay core (assertions unchanged; only paths/imports/gates re-spelled)

- Cost pins: `iobase_calls.rs` :512, :532 (2 of 41); `holder/mod_.rs` :686, :737 (2 of 31); `warehouse/handle.rs` (1),
  `warehouse/media.rs` (1); `iceberg/catalog/mod_.rs` :1412, :1514; `iceberg/scan.rs` :1918; `iceberg/staging.rs` :325;
  `iceberg/table.rs` :3567 - 11 tests that construct `S3Options`/`Holder::S3*`; their `cfg(feature = "s3")` becomes the
  leaf dependency (dev-dep or install) and `yggdryl::s3::` becomes `yggdryl_s3::` (a role-by-name `Holder::S3*` change
  depends on the extension point's shape: D30 keeps the byte-backend arms in `Holder::from_url`).
- `http/retry.rs` (9) and `http/options.rs:101` pin the constants the S3 client shares: they stay with `http/`, which
  must publish `retry::*`, `HttpOptions::DEFAULT_MAX_PAUSE`, `client::agent_for`, `record_process` and `render_http_date`
  to the leaf (D6's implementer module); `s3/client.rs:945-948` keeps pinning the S3 reading.
- `root/error.rs:63-77`, `uri/arn.rs`, `fs/location.rs`, `allocations.rs` (no S3 row), `benchmark_mode.rs`: untouched.
- `[shards.yggdryl]`: no s3 entry; nothing to take.
- Moved-out wire/count contracts are not re-pinned: the request-count tables (docs 4325-4346, 4356-4365; AGENTS.md
  1379-1389; iceberg.md 1424-1437) stay byte-identical.

### C. Scripts' cargo commands

- `check_object_interop.py:337-350`, `check_azure_interop.py:211-224`, `check_gcs_interop.py:215-228`: replace
  `--manifest-path <rust/Cargo.toml> --features s3 --test interop <s3::dialect::>` with `-p yggdryl-s3 --test interop
  <filter>` (or `--manifest-path rust/s3/Cargo.toml`); the aws driver also gains the `cwd=REPO` the other two set;
  `rust/target/<store>-interop` scratch paths (:56, :58, :61) stay valid (workspace target dir) or move; SKIPPED rules,
  ports (9123, 10123, 4499), pinned server versions (MinIO RELEASE.2025-04-22T22-12-26Z, fake-gcs-server 1.56.1,
  `azurite@3`) and reference clients are unchanged.
- `check_docs_examples.py:209` features string drops `s3 s3tables`, gains the leaf(s) as dev-dependencies of the
  generated `docs_examples` target.
- Docs-quoted commands: `cargo bench --bench holder --features s3 -- object_ --noplot` (holder/index.md:5162,
  benchmarks.md:72), `cargo bench --features "iceberg s3" -p yggdryl --bench media -- 's3/' --quick` (iceberg.md:1922),
  `cargo test -p yggdryl --all-features --test s3` (testing.md:75).

### D. CI rows and jobs

1. `rows.toml`: add the `s3` line (comment form until the manifest exists), re-base `x-s3/x-azure/x-gcs` on `crate-s3`,
   shrink the `interop` comment (:243-244), iceberg `after` gains `s3` (2.2).
2. `ci.yml`: `core-interop` (217-243) loses the `--features s3` step (234-235) and its comment (219-222), its description
   becomes "the PyIceberg exchange" only; `object-interop`, `azure-interop`, `gcs-interop` (329-413) `needs: [changes,
   core-interop]` and their `rust-lane restore ... lane: interop` change (the leaf builds in place on `leaf-s3`; or a small
   `s3` exchange build job is added); comments 330-346 re-spelled; `leaves` job comment 959-967 "none until S4".
3. `scripts/ci/plan.py:503` ("seven leaves"), `scripts/tests/test_ci_plan.py` :37 (`[a-z]+` -> `[a-z0-9]+`), :255-262,
   :312-326 (EXCHANGES gets `"s3": {"object-interop", "azure-interop", "gcs-interop"}`, NEVER is per-leaf).
4. `release.yml` preflight and publish for the new crate name (`yggdryl-s3`); `python/Cargo.toml:22-27` and
   `node/Cargo.toml:22-27` feature lists become dependencies on the leaf.

### E. Docs sentences

- `docs/holder/index.md`: 4257 ("Behind the non-default `s3` feature"), 4323 ("asserted by tests" - names the suite), 4352
  (`accounting::iceberg` in `rust/tests/s3/mod_.rs`), 4412-4526 (configuration, `S3Options` blocks), 5162 and 5165-5170
  (bench and driver commands), 16, 40, 96-97; the 12 Rust fences (`use yggdryl::s3::...`) become `use yggdryl_s3::...` or
  a re-export if the core keeps a facade; the AWS identity subsection 4527-5007 stays.
- `docs/media/iceberg.md` 1389, 1439, 1456, 1922; `docs/testing.md` 75, 212-226, 251-258, 272; `docs/benchmarks.md` 21, 72;
  `docs/architecture.md` 32, 240-244; `docs/contributing.md` 38, 50; `docs/warehouse/index.md` 1177; `docs/expression/plans.md` 221;
  `README.md` 16, 66; `rust/README.md` 15.
- `AGENTS.md`: Layout features line (309-312), rows `holder/` 376, `auth/` 378, `aws/` 379, `xml/` 380, `local/ fs/ zip/ s3/` 381
  (the `s3/` clause leaves), `http/` 382, `s3tables/` 391, "Common changes" 161, smoke loop 186, "### Object stores" 1342-1413
  (a new leaf row or a pointer to the crate), §2 CI rows 2233-2238 and the leaf/exchange table 2288-2296, §5 2701, §6 name list.
- Skills: `yggdryl-storage` (SKILL.md 8 lines; references/backends.md 14, rust.md 14, python.md 21, javascript.md 2),
  `yggdryl-uri/SKILL.md:145`, the three `s3 (implies aws)` feature tables.
- Inventories: `.api-inventory.txt` 2041-2130 moves to a `### yggdryl_s3  [rust/s3/src/lib.rs]`-style section (90 lines, 58 `pub`);
  2735 (`Holder` variants) re-spelled; 2131-2184 (`aws`) and 2185-2252 (`s3tables`) stay in place; `.api-bindings.txt` 141,
  146-147 unchanged if the Python classes remain in the one native module.

### F. Hazards found (decisions for the foreground, none taken)

1. `LEAF_LINE` regex cannot read `s3` (digit) - the planner tests would fail on the first `with_leaves("s3")`.
2. `NEVER` in `TheLeaves` forbids the three exchanges for every leaf; the s3 leaf owns them.
3. Seven harnesses plus two benchmarks include `support/server.rs`; its 25 self-tests run in each declaring harness (175
   executions today), and the fixture is the one shared artifact between the leaf and core/iceberg tests.
4. `encode_key` (`aws/sigv4.rs:81`), `Bearer`/lease (`auth/lease.rs:19,48,55,71`, `auth/mod.rs:29`), `aws/session.rs` (11 gates
   incl. 77, 208, 214, 343, 360, 616, 704, 711, 1331, 1675, 1686), `aws/mod.rs:62`, `http/mod.rs:82`, `xml/scanner.rs:67,78` carry
   `cfg(feature = "s3")` for s3-only pieces; each is either published to the leaf or its gate rewritten, and its tests
   (1.6) follow. Counted 79 gates in `rust/src` outside `rust/src/s3/` (appendix).
5. The Iceberg-over-S3 pins (`s3/mod_.rs` `accounting::iceberg`, 5) create an `s3 <-> iceberg` dev-dependency cycle if
   left in the leaf.
6. `check_docs_examples.py` compiles page fences into a `yggdryl` integration target; every `yggdryl::s3` fence (12 Rust in
   holder/index.md + 1 Rust and 4 Python in skills/yggdryl-storage) needs the leaf there, or the fences change spelling.

---------------------------------------------------------------------------------------------------------------

## APPENDIX - commands I counted with (all via `git -C /home/user/yggdryl ... HEAD`)

```bash
# files and test counts
git ls-tree -r --name-only HEAD rust/tests/s3 rust/tests/s3.rs rust/tests/interop/s3
for f in $(git ls-tree -r --name-only HEAD rust/tests/s3); do git show HEAD:$f | grep -c '#\[test\]'; done | paste -sd+ | bc   # 239
for f in $(git ls-tree -r --name-only HEAD rust/tests/interop/s3); do git show HEAD:$f | grep -c '#\[test\]'; done | paste -sd+ | bc  # 20
git show HEAD:rust/tests/support/server.rs | grep -c '#\[test\]'                                                            # 25
git ls-tree -r --name-only HEAD rust/src/s3 | wc -l                                                                         # 26
# who reaches the s3 backend
git grep -n -l -E 'yggdryl::s3|S3Options|S3File|S3Folder|S3Path|FakeS3|feature = "s3"|feature = "aws"|"s3"' HEAD -- rust/tests rust/benchmarks
git grep -n 'support/server.rs' HEAD -- rust/tests rust/benchmarks
git grep -n -E 'internals::(s3_|xml_scanner|http_retry)' HEAD -- rust/tests rust/benchmarks
git grep -c 'feature = "s3"' HEAD -- rust/src ':!rust/src/s3/'     # 79
git grep -c 'feature = "s3"' HEAD -- rust/tests rust/benchmarks    # 43 (20 inside s3-owned tests)
git grep -n -E 'crate::s3|s3::' HEAD -- rust/src/holder rust/src/iceberg rust/src/uri rust/src/warehouse rust/src/xml rust/src/lib.rs
git grep -n -E 'crate::http|http::' HEAD -- rust/src/s3
# #[test] names inside a block (helper): python3 s3_backend_map/blk.py <path> <start-line>
# docs and skills
git grep -n -c -E 'yggdryl::s3|S3Options|S3File|S3Folder|S3Path|--features[ =]"?[a-z0-9 ]*s3\b|feature = "s3"|`s3` feature|rust/(src|tests|benchmarks)/s3[/.]|[^:]s3::[a-z]|check_(object|azure|gcs)_interop|AzureOptions|GoogleOptions|object-interop|azure-interop|gcs-interop|s3/mod' HEAD -- docs skills README.md rust/README.md '*.md'
# CI
git show HEAD:.github/ci/rows.toml | cat -n ; git show HEAD:.github/workflows/ci.yml | grep -n -i -E 's3|azure|gcs|object'
git show HEAD:scripts/ci/plan.py | grep -n -E 'rows\[row\] = Row|CRATES|crate_row|seven leaves|CORE_FOLDERS|MEMBERS'
git show HEAD:scripts/tests/test_ci_plan.py | grep -n -E 'LEAF_LINE|EXCHANGES|NEVER|object-interop'
# inventories
git show HEAD:.api-inventory.txt | awk '/^### yggdryl::(s3|aws|s3tables|http) /{print NR": "$0}'
```

Not run (reader rules): any cargo command, any script. Not read in full: `rust/src/s3/**` bodies (the implementer side is another
agent's map), the 12 `internals` module bodies, `rust/tests/s3/aws/mod_.rs` internals imports (counted, not listed), `docs/holder/index.md`
4411-4526 and 4527-5007 beyond headings and the fences named above.
