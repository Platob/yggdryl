# yggdryl-s3: what `rust/src/s3/**` reaches in the core, and what the core reaches in it

Read at HEAD `d100439d3` ("Record the S3 design"), branch `ccr-0fe6f9d0-ruymat`. Every `file:line` is
`git show HEAD:<path>`; the working tree carries 70 modified paths, none of them under `rust/src/{s3,aws,auth,s3tables}`,
and the working-tree diff of `holder/mod.rs` (4 lines) and `http/request.rs` (46 lines) names no S3 site. No cargo command was run,
nothing was edited. Helper scripts and raw lists sit beside this file (`extract_uses.py`, `s3_uses.json`, `sites.json`, `cfg_sites.txt`,
`reverse_raw.txt`).

## 0. Verdict in ten lines

1. `rust/src/s3/` is 26 files, 11,963 lines, one public face (`S3Path`, `S3Folder`, `S3File`, `S3Options`, `Provider`, `StatsSnapshot`,
   the three dialect option structs, `Credentials` re-export, 12 free doors) behind the non-default `s3` feature (`rust/Cargo.toml:50`).
2. It reaches the core through 32 public names (18 rows, section 2.2), 51 crate-private items and 7 `pub`-items-in-private-modules
   (58 to forward, sections 2.3-2.4), and through `ureq` types that cross the boundary (2.5). It never names `fs/`, `zip/`, `media/`,
   `warehouse/`, `iceberg/`, `s3tables/` or `expression/` (`git grep 'crate::(fs|zip|media|warehouse|iceberg|s3tables|expression|Serie|Field|DataType)' HEAD -- rust/src/s3` is empty; the `impl_default_iomedia!` expansion names `media::RecordOptions` and `Serie`, both public).
3. The core reaches it at 30 lines of `holder/mod.rs` (three closed-enum variants `Holder::{S3Folder,S3Path,S3File}` and 12 delegating arms),
   5 lines of `iceberg/staging.rs`, 3 of `iceberg/catalog/mod.rs`, 2 calls in `warehouse/handle.rs` (`Site::Store`), 2 in `s3tables/catalog.rs`,
   the `pub mod s3` line and 12 `internals` re-exports in `lib.rs`, and 3 crate-private methods on `S3File` the core calls
   (`upload_from`, `discard`, `byte_stream`). Python names it at 87 lines, Node at 2, the CLI at 0.
4. `aws/` and `auth/` must NOT move with `s3/`. `aws/` is used by `s3tables/` (4 `use crate::aws::` lines, `error_code` and `with_sigv4` at call sites), by `warehouse/handle.rs`
   (`Site::Store` holds an `aws::Session`), by `http/` (a resend hook written for it) and by `uri/arn.rs` (`ArnPartition::check_region`);
   `auth/` is used by `http/` itself (`Secret`, `variable`: 5 sites). And `aws/request.rs:22` is `impl Request { pub fn with_sigv4 }`,
   an inherent impl on `http::Request`, which compiles only in the crate that defines `Request`.
5. Direction: `yggdryl-s3 -> yggdryl[aws]`; `yggdryl-iceberg[s3tables] -> yggdryl[aws]`; no arrow between the two leaves at compile time
   (the table's store is opened through the new register at run time); the core depends on neither.
6. The price of keeping `aws/` core: 8 fragments, at 23 `#[cfg(feature = "s3")]` sites in `aws/`, `auth/`, `http/`, `xml/`, exist only for `s3`
   today (section 2.6): move three to the leaf, re-key three to `aws`, raise one, keep one. `Session::bucket_regions` (shared state inside
   `Session`) is the one that cannot move.
7. The closed `Holder` enum is the register's hard part: it needs a boxed role (`Holder::Registered`) answering `IOBase + IOMedia + Debug + Send`
   plus seven verbs read off the sites (3.2).
8. `ureq 3.x` types appear in four crate-private signatures (`Session::tls_config`, `agent_for`, `retry::is_retryable_transport`, `retry::is_unsent`),
   so `yggdryl-s3` takes `ureq` as a direct dependency at the core's pin, or the forwarders hide it (2.5).
9. Dependencies that become s3-only: `md-5` (one call), dev-deps `object_store`, `futures` and the bench `tokio` use; `hmac`, `sha2`, `ring`,
   `base64`, `ureq` are shared with the core and are declared again (same pins) in the leaf (section 3.4).
10. AGENTS.md "Object stores"/URI text says the S3 authority hostname rule is `.com`/`.io`; the code is `.com`, `.io`, `.net`
    (`uri/authority.rs:558`, doc at `uri/mod.rs:540-541`). Documentation drift to fix when the rows move.

## 1. The unit

`git ls-tree -r --name-only HEAD -- rust/src/s3` (26 files; lines in parentheses):

| path | lines | role |
| --- | --- | --- |
| `s3/mod.rs` | 423 | `pub(crate) mod answer, client, file, options, properties, xml`; `pub mod aws, azure, google`; private `encryption, folder, path, provider, request`; the 12 free doors (`located`, `located_with`, `file[_with]`, `folder[_with]`, `file_at[_with]`, `folder_at[_with]`, `path_at[_with]`); `pub use crate::aws::Credentials` (`:124`) |
| `s3/client.rs` | 3372 | the one pooled signed client; `Stats`; `StatsSnapshot` (pub, `:77`); `pub(super) struct Client` (`:290`) |
| `s3/file.rs` / `folder.rs` / `path.rs` | 1140 / 529 / 651 | `S3File`, `S3Folder`, `S3Path`: `impl IOBase, IOMedia (impl_default_iomedia!), IOFile / IOFolder / IOPath` (`file.rs:454,498,502`, `folder.rs:325,397,401`, `path.rs:334,357,361`) |
| `s3/options.rs`, `properties.rs`, `provider.rs`, `encryption.rs`, `answer.rs`, `request.rs`, `xml.rs` | 563, 806, 222, 547, 83, 197, 93 | `S3Options`, the property reader, `Provider {Aws, Google, Azure}` (`provider.rs:17`), SSE, shared answer shapes |
| `s3/aws/{mod,options,xml}.rs` | 14, 200, 269 | S3 dialect (`AwsOptions`, `Checksum`, `ListBucketResult` reader) |
| `s3/azure/{mod,auth,dialect,options,sign,xml}.rs` | 16, 453, 184, 454, 277, 93 | Azure dialect (Shared Key, SAS, bearer; `Lease`/`Bearer`) |
| `s3/google/{mod,dialect,json,options,token}.rs` | 14, 261, 149, 262, 691 | Google dialect (ADC chain, RS256 JWT, JSON API) |

The unit's own test hooks: 12 `internals` re-exports, `lib.rs:528-551` (`s3_answer`, `s3_aws_xml`, `s3_azure_{auth,dialect,sign,xml}`, `s3_client`,
`s3_file`, `s3_google_token`, `s3_options`, `s3_properties`, `s3_xml`). They move with the folder; the generator
`scripts/generate_internals.py` writes them.

## 2. Part 1: `s3/` -> the core

### 2.1 How it was counted

```
git grep -h '^use crate::\|^    use crate::' HEAD -- rust/src/s3 | sort -u          # 31 distinct lines (brace groups)
git grep -n -o 'crate::[A-Za-z_0-9:{}, ]*' HEAD -- rust/src/s3                      # 162 matches, 152 not crate::s3 (docs included)
git grep -n 'crate::' HEAD -- rust/src/s3 | grep -v 'crate::s3' | grep -v ' use ' | grep -v '//'   # 91 inline code lines, 92 paths, read one by one
python3 -I extract_uses.py rust/src/s3 s3_uses.json                                  # multi-line use groups expanded: 248 leaves, 103 reach crate:: outside s3
```

Result: 195 reaching sites (103 use leaves + 92 inline paths), 83 distinct `crate::` paths. After resolving the two `self` imports
(`use crate::http::retry::{self,..}` at `client.rs:35`, `use crate::aws::sigv4::{self,..}` at `client.rs:32`) to the members really used,
the distinct leaf items are the 32 + 7 + 51 of the tables below. Counts in the "sites" column are `use` leaves plus inline `crate::` paths (`sites.json`);
visibility is read at the definition.

### 2.2 Public: nameable from outside as `yggdryl::...` (no forwarding)

| # | item | definition / re-export | sites in `s3/` |
| --- | --- | --- | --- |
| 1 | `Error` (variants `Io`, `Parse`, `Remote`, `Conflict`; ctors `absent`, `conflict`, `remote`, `unsupported`), `Result` | `error.rs:67,102,143,386,395,405,416`; `lib.rs:194` | `Error` 13, `Result` 19 path/use sites (35 `Error::Io`, 7 `Error::remote`, 5 `Error::Remote`, 3 `Error::Conflict`, 3 `Error::Parse`) |
| 2 | `Scalar` (`from`, `from_struct`, `from_sequence`, `as_str`) | `lib.rs:343` | 3 `use` + `Scalar::from` 17 |
| 3 | `Scheme` (consts `S3`, `GS`, `AZ`; `is_s3`, `is_gs`, `is_az`) | `lib.rs:254`; `scheme.rs:263,282,292` | `provider.rs:13,36-53`, `client.rs:298,331-372` |
| 4 | `Uri`, `Url` (`bucket`, `key`, `account`, `store_endpoint`, `region`, `scheme`, `authority`, `path`, `password`, `user`, `is_glob`, `has_trailing_slash`, `joinpath`, `parent`, `extension`, `media_type`, `from_str`) | `lib.rs:262-265`; `uri/url.rs:277-312` | `Url` 6 `use` + 36 more, `Uri` 3 |
| 5 | `IOBase`, `IOFile`, `IOFolder`, `IOPath`, `IOMedia`, `IOKind` | `lib.rs:225,229,230,231,232,234` | 3 `impl IOBase`, 1 each role, 3 `impl crate::IOMedia`, `IOKind` 30 tokens |
| 6 | `Listing` (`new`, `failing`, `empty`), `MediaType`, `MimeType` (`FILE`, `DIRECTORY`) | `lib.rs:238,240,242` | 3 / 3 / 2 `use` |
| 7 | `holder::Holder` (variants constructed: `S3Folder`, `S3Path`, `S3File`) | `holder/mod.rs:90` (`#[derive(Debug)] #[non_exhaustive]`, `:87-89`) | 4 `use`, 8 constructions (`mod.rs:160`, `file.rs:1014`, `folder.rs:318,320,464,475,477`, `path.rs:560`) |
| 8 | `Digest`, `DigestAlgorithm` | `lib.rs:192` | `file.rs:716-717` |
| 9 | `ByteStream::{from_reader, from_container}` | `lib.rs:185`; `bytestream.rs:40,92` | `file.rs:426,433`, `path.rs:397,404` |
| 10 | `DEFAULT_STREAM_BATCH_SIZE`, `not_empty` | `lib.rs:224-226`; `iobase/lifecycle.rs:42` | `file.rs:731`; `folder.rs:391` (named through the private `crate::iobase::` path) |
| 11 | `impl_default_iomedia!` (`#[macro_export]`) | `iomedia.rs:1117-1118`; expands to `$crate::{IOBase, overwrite_serie_default, media::RecordOptions, Serie, Result, IOResult}`, all public (`lib.rs:223`) | `file.rs:499`, `folder.rs:398`, `path.rs:358` |
| 12 | `json::{into_utf8, from_bytes, from_utf8}` | `json/mod.rs:576,124,114` (`pub mod json`, `lib.rs:102`) | 5 + 4 + 1: `client.rs:2672`, `google/json.rs:132,137`, `google/token.rs:271,364,386,424,425,512`, `azure/auth.rs:391` |
| 13 | `xxhash::xxh3` | `xxhash/mod.rs:140` | `azure/dialect.rs:151`, `google/dialect.rs:143` |
| 14 | `local::LocalFolder::config` | `local/folder.rs:169` | `google/token.rs:624` |
| 15 | `ArnPartition::{from_region, dns_suffix, as_str}` | `lib.rs:263`; `uri/arn.rs:727,748,696` | `client.rs:160,412` |
| 16 | `http::render_http_date` | `http/mod.rs:86-89` (`pub use headers::{..}`) | `azure/sign.rs:215` |
| 17 | `http::HttpOptions` (`default`, `DEFAULT_MAX_PAUSE`, `with_timeout`, `with_connect_timeout`, `with_read_environment`, `with_proxy`) | `http/options.rs:680,99,364,371,432,409` | `client.rs:44,424-430` |
| 18 | `aws::{Credentials, Session}` and the `Session` methods that are `pub`: `region`, `profile`, `endpoint_url`, `use_fips_endpoint`, `use_dualstack_endpoint`, `max_attempts`, `reads_environment`, `with_environment`, `with_region`, `with_credentials`, `with_anonymous`, `variable`; `Credentials::{new, access_key_id, secret_access_key, with_session_token}` | `aws/mod.rs:75,77`; `aws/session.rs:997,982,1080,1129,1138,1307,723,539,395,405,414,770` | `Credentials` 4 `use` (`client.rs:33`, `mod.rs:124`, `options.rs:22`, `properties.rs:24`), `Session` 2 `use` (`client.rs:33`, `options.rs:22`), calls at `client.rs:478-498,506,516,616,678,724,733-734,813`, `properties.rs:225,301` |

32 names in 18 rows. Nothing to forward; they need the leaf to depend on `yggdryl` with `aws`.

### 2.3 Crate-private: the D6-style list yggdryl-s3 would need forwarded (51)

"vis" is read at the definition; "route" is DESIGN.md D6's (R raise inside a private module and `pub use`; F forwarder for a published module;
A free forwarder over an inherent `pub(crate)` item; M definition moved; X `#[macro_export]` macro).

| # | item | defined | vis | s3 sites | route |
| --- | --- | --- | --- | --- | --- |
| 1 | `iobase::oversized` | `iobase/lifecycle.rs:82`, re-exported `iobase.rs:241`; `mod iobase` private `lib.rs:89` | `pub(crate)` | 17 (`client.rs:1612,1616,1698,1702,1831,2319,2780`; `file.rs:355,359,626,747,750,837,885,890,900,1061`) | F |
| 2 | `ByteStream::from_handle` | `bytestream.rs:126` | `pub(super)` (= crate-wide, `bytestream` is a root module) | `file.rs:555` | A |
| 3 | `ArnPartition::check_region` | `uri/arn.rs:832` (`#[cfg(feature = "aws")]`) | `pub(crate)` | `client.rs:723,1320` (the other 3 grep hits are doc comments) | A |
| 4 | `uri::percent_decode` | `uri/parser.rs:655`, `uri/mod.rs:47` | `pub(crate)` | `client.rs:834`, `mod.rs:375` | F |
| 5 | `uri::percent_encode_segment` | `uri/parser.rs:45`, `uri/mod.rs:47` | `pub(crate)` | `mod.rs:328,386`, `google/token.rs:575` | F |
| 6 | `integer::BYTE_COUNT_SPELLINGS` | `integer.rs:743` | `pub(crate)` | `properties.rs:26` | F |
| 7 | `integer::byte_count_from_text` | `integer.rs:818` | `pub(crate)` | `properties.rs:26` | F |
| 8 | `integer::integer_from_scalar_as` | `integer.rs:779` | `pub(crate)` | `google/json.rs:12`, `azure/auth.rs:401`, `google/token.rs:522` | F |
| 9 | `boolean::bool_from_text` | `boolean.rs:142` | `pub(crate)` | `azure/options.rs:12`, `client.rs:34` (6 tokens) | F |
| 10 | `holder::system_time_ns` | `holder/mod.rs:33` | `pub(crate)` | `azure/sign.rs:215` | F |
| 11 | `warned!` (expands to `$crate::logging::warning::warn`, itself `pub(crate)` at `warning.rs:50`) | `logging/warning.rs:109,121` (`pub(crate) use warned`); `pub(crate) mod warning` `logging/mod.rs:76` | `pub(crate)` macro | `file.rs:10,200`, `path.rs:9,278` | X |
| 12 | `xxhash::stream::read_range_digest` | `xxhash/stream.rs:33`; `pub(crate) mod stream` `xxhash/mod.rs:98` | `pub(crate)` | `file.rs:721` | A |
| 13 | `http::record_process` (the process-wide host ledger every client records into) | `http/client.rs:91`, `http/mod.rs:82-83` (`#[cfg(feature = "s3")]`) | `pub(crate)` | `client.rs:1158,1274` | A |
| 14 | `http::client::agent_for` | `http/client.rs:1128` (`pub mod client`) | `pub(crate)` | `client.rs:431` | A |
| 15-25 | `http::retry::{RETRY_BACKOFF, RETRY_COST, RETRY_REFUND, RetryBudget (withdraw, refund, remaining), backoff, delay, fresh_jitter, retry_after, is_retryable_transport, is_unsent, is_resumable}` | `http/retry.rs:15,21,23,37 (54,66,75),81,98,114,132,141,155,173`; `pub(crate) mod retry` `http/mod.rs:67` | `pub(crate)` (11 items) | `client.rs:35-38,44,844,880,896,904,3210,3214` | A |
| 26-33 | `aws::sigv4::{Signer (sign), encode_key, encode_query_component, canonical_query, signed_access_key, sha256_hex, EMPTY_PAYLOAD_SHA256, UNSIGNED_PAYLOAD}` | `aws/sigv4.rs:119 (211),82,87,93,68,74,34,31`; `pub(crate) mod sigv4` `aws/mod.rs:71`; `encode_key` is `#[cfg(feature = "s3")]` (`:81`) | `pub(crate)` (8) | `client.rs:32,245-251,873,1020,1074,1114-1120`; `azure/dialect.rs:122-129`; `azure/auth.rs:416`; `google/dialect.rs:46`; `google/token.rs:586` | A |
| 34 | `aws::environment::is_native` | `aws/environment.rs:24`; `#[cfg(feature = "s3")] pub(crate) mod environment` `aws/mod.rs:62-63` | `pub(crate)` | `properties.rs:244` | A |
| 35-40 | `aws::properties::{EndpointName (of), Identity (read, apply), count, flag, refusal, seconds}` | `aws/properties.rs:535 (579),187 (237,327),645,606,654,636`; `pub(crate) mod properties` `aws/mod.rs:68` | `pub(crate)` (6) | `properties.rs:25,252,378,383,589,609,641` | A / M |
| 41-42 | `xml::scanner::{parse_document, parse_root}` | `xml/scanner.rs:100,86`; `#[cfg(feature = "aws")] pub(crate) mod scanner` `xml/mod.rs:57-58` | `pub(crate)` (2) | `xml.rs:10` (re-exported `pub(crate)` to `aws/xml.rs`, `azure/xml.rs`, `client.rs`) | A |
| 43-51 | `Session` methods: `signer`, `answers_another`, `given_variables`, and the s3-gated `tls_config`, `bucket_region`, `learn_bucket_region`, `stated_region`, `states_identity`, `under` | `aws/session.rs:1630,1594,760; 1332,1676,1687,705,712,617` | `pub(crate)` (9) | `client.rs:360,413,486,874,1031,1334`; `properties.rs:225,301,348` | A |

### 2.4 Public items inside private modules: `pub` today, unreachable from outside (7)

| item | defined | module visibility | s3 sites | route |
| --- | --- | --- | --- | --- |
| `auth::Bearer` | `auth/lease.rs:50` (`#[cfg(feature = "s3")]`) | `mod auth` is private (`lib.rs:28-29`, `#[cfg(feature = "http")]`); `pub(crate) use lease::Bearer` `auth/mod.rs:29-30` | `azure/auth.rs:13`, `google/token.rs:19` | R, or move to the leaf (a 30-line struct over `Secret` and `Expiring`) |
| `auth::Expiring` | `auth/lease.rs:24` | idem, `pub(crate) use` `auth/mod.rs:33` | `google/token.rs:678` | R |
| `auth::Lease` | `auth/lease.rs:131` (`new` `:162`, `get` `:204`, `invalidate` `:378`) | idem | `azure/auth.rs:13,223`, `google/token.rs:19,160` | R |
| `auth::instant` | `auth/lease.rs:410` | idem | `google/token.rs:19` | R |
| `auth::variable` | `auth/environment.rs:87` | idem, `pub(crate) use` `auth/mod.rs:32` | `client.rs:31`, `azure/auth.rs:13`, `google/token.rs:19` | R (core `http/` uses it too: `http/client.rs:571`, `session.rs:391,475`, `tls.rs:36`) |
| `xml::scanner::Element` (methods `name`, `child`, `child_text` pub; `children` and `required` are `#[cfg(feature = "s3")]`, `scanner.rs:67,78`) | `xml/scanner.rs:46` | `pub(crate) mod scanner` | `xml.rs:10` | R |
| `xml::scanner::XmlError` | `xml/scanner.rs:35` | idem | `xml.rs:10` | R |

`Secret` is not named by `s3/` but `Bearer::new(value: impl Into<Secret>, ..)` (`lease.rs:58`) names it; `impl From<String> / From<&str> for Secret` are at `auth/secret.rs:40,46`.

Total to forward if `aws/` and `auth/` stay core: 51 + 7 = 58, of which 10 exist only for `s3` today (`encode_key`, `record_process`, `is_native`, the six s3-gated `Session` methods, `Bearer`; 2.6).
For scale, DESIGN.md "S3: design" counts 115 items for the market and FIX crates and 116 for the media folders.

### 2.5 External types that cross the boundary

`ureq 3.x` appears in the signature of four crate-private items the leaf must call:

| signature | defined |
| --- | --- |
| `Session::tls_config(&self) -> Result<Option<ureq::tls::TlsConfig>>` | `aws/session.rs:1332` (builds from `ureq::tls::{parse_pem, Certificate, TlsConfig, RootCerts}`, `:1342-1357`) |
| `agent_for(&HttpOptions, Option<ureq::tls::TlsConfig>) -> Result<ureq::Agent>` | `http/client.rs:1128-1131` ("The one door every client in the crate - the S3 backend's included - takes its connections through", `:1124-1127`) |
| `retry::is_retryable_transport(&ureq::Error)`, `retry::is_unsent(&ureq::Error)` | `http/retry.rs:141,155` |

and `s3/` itself uses `ureq::{Agent, Error, http::Request, tls::TlsConfig}` at `client.rs:291,423,1156-1165,1272-1281,3167`, `azure/auth.rs:137,236,244,252,311,356`,
`google/token.rs:248,258,328,473,485,532`. So `yggdryl-s3` needs `ureq` as a direct dependency at the core's pin (`rust/Cargo.toml:164`,
`ureq = "3.4"`, `default-features = false, features = ["rustls"]`), or the forwarders wrap the three types, which puts a third-party type
in a public signature (AGENTS.md forbids it for xxhash; no rule names ureq).

Other external crates used by `s3/` (count of path uses, `git grep` of `crate::`-free paths): `base64` 16 (`aws/options.rs`, `azure/dialect.rs`, `azure/sign.rs`,
`client.rs`, `encryption.rs`, `google/token.rs`), `ureq` 22, `sha2` 4 (`aws/options.rs:73-74`, `azure/sign.rs:13`, `encryption.rs:20`),
`hmac` 1 (`azure/sign.rs:12`), `md5` 2 (`client.rs:3177-3178`), `ring` 3 (`google/token.rs:446-451`).

### 2.6 What `aws/`, `auth/`, `http/`, `xml/` already carry only for `s3`

`git grep -n 'feature *= *"s3"' HEAD -- rust/src` outside `s3/` and `holder/`/`iceberg/`/`warehouse/` (those are section 3):

| site | what | who else uses it |
| --- | --- | --- |
| `aws/mod.rs:62-63` + `aws/environment.rs` (90 lines) | `is_native`: the AWS names the S3 options' `AWS_` sweep must leave to the session | nobody |
| `aws/session.rs:77-78` `BUCKET_REGIONS`, `:208-209` field `bucket_regions: Arc<Mutex<VecDeque<BucketRegion>>>`, `:214-221` `BucketRegion`, `:343,360-362` `from_knobs(.., parent)` | the region a redirect found a bucket in, "shared by every session derived from this one" (`:205-207`); state inside `Session` | nobody (s3 only) |
| `aws/session.rs:617` `under`, `:705` `stated_region`, `:712` `states_identity`, `:1332` `tls_config`, `:1676` `bucket_region`, `:1687` `learn_bucket_region` | merge of knobs, accessors, the TLS config | `s3tables/catalog.rs:483` calls `stated_region` |
| `aws/sigv4.rs:81-82` `encode_key`, `:182-183` `Signer::access_key_id` (with `internals`), `:452` internals twin | object-key encoding | nobody |
| `auth/lease.rs:19,48,55,71` + `auth/mod.rs:29-30` | `Bearer` | nobody |
| `http/mod.rs:82-83` | `record_process` re-export | `http/client.rs` records into it itself |
| `xml/scanner.rs:67,78` | `Element::children`, `Element::required` | nobody |

Eight fragments, 23 sites: `aws/mod.rs` 1, `aws/session.rs` 11, `aws/sigv4.rs` 3, `auth/lease.rs` 4, `auth/mod.rs` 1, `http/mod.rs` 1, `xml/scanner.rs` 2 (`Signer::access_key_id` and the `internals` twin are test-only).
Counts per file for `feature = "s3"` outside `s3/`: `holder/mod.rs` 26, `aws/session.rs` 11, `iceberg/staging.rs` 7, `warehouse/handle.rs` 6, `auth/lease.rs` 4,
`iceberg/catalog/mod.rs` 3, `aws/sigv4.rs` 3, `xml/scanner.rs` 2, `lib.rs` 1, `http/mod.rs` 1, `aws/mod.rs` 1, `auth/mod.rs` 1 (`cfg_sites.txt`).

## 3. Part 2: the core -> `s3/`

### 3.1 Every site outside `rust/src/s3` that names it

```
git grep -n 's3::\|S3Options\|S3Path\|S3Folder\|S3File\|S3Client\|Provider::' HEAD -- rust/src | grep -v '^rust/src/s3/'     # 62 lines, 10 files
git grep -n 'feature *= *"\(s3\|aws\|s3tables\)"' HEAD -- rust/src                                                         # cfg_sites.txt, 103 + lib.rs
```

| area | file:lines | what | reach |
| --- | --- | --- | --- |
| holder dispatch | `holder/mod.rs:107-114` | three closed-enum variants `S3Folder`, `S3Path`, `S3File` | type names |
| | `:201-206` | `exists()` arms: `folder_exists`, `path_exists`, `file_exists` | pub trait methods |
| | `:240-241` | `into_byte_stream`: `Self::S3File(file) => file.byte_stream(position, batch_size)` | **`S3File::byte_stream`, `pub(crate)` at `s3/file.rs:412`** |
| | `:384-422` | `from_url` object-store arm: `url.scheme().is_object_store()` (`:384`), per query parameter `S3Options::is_property` (`:392`), `S3Options::from_properties` (`:404`), `s3::located_with(&location.to_string(), options)` (`:414`); without `s3`, `Error::unsupported("holding an object store location without the s3 feature", ..)` (`:416-422`) | pub: `s3/properties.rs:178,363`, `s3/mod.rs:157` |
| | `:518-532` | `from_handle`: `S3Folder` cloned; `S3Path` and `S3File` rebuilt as siblings (`sibling(plain)`, `:47-58`) with `as_directory()` / `as_file()` | pub: `s3/path.rs:151,160` |
| | `:593-597` | `is_backend_property`: `S3Options::is_property(name)` | pub |
| | `:914-919, 952-957, 990-995, 1028-1033` | `as_io`, `as_io_mut`, `as_media`, `as_media_mut`: 4 x 3 arms | the three types as `dyn IOBase` / `dyn IOMedia` |
| | total | 30 lines match the grep, 26 `#[cfg(feature = "s3")]` | |
| byte-backend scheme vocabulary | `holder/mod.rs:384,595`, `warehouse/handle.rs:95`, `fs/location.rs:450-455`, `warehouse/catalog.rs:555`, `s3tables/catalog.rs:158`, `warehouse/catalog.rs:351` | `Scheme::{is_object_store, is_s3_tables, is_gs, is_az, has_container}`, `Url::bucket`/`hostname` | stays core (section 5) |
| `Holder` register | `holder/mod.rs:13,17`, `holder/locator.rs:23-109` | the existing `Locator` register is **for catalog services only** ("the local, ZIP, object-store and HTTP backends stay the core's", `locator.rs:10`); its `holder()` returns a `Holder`, which cannot hold a leaf-crate type | the gap the new register fills |
| warehouse site | `warehouse/handle.rs:45-52` `Site::Store { url, session: crate::aws::Session, region, store: Properties }` (`#[cfg(feature = "s3")]`, `#[cfg_attr(not(feature = "s3tables"), allow(dead_code))]`) | the core enum names `aws::Session` | `aws::Session` pub |
| | `:63-74,112-113,129-145,156-157,173-174` | `Debug`, `url`, `resolve`, `PartialEq`, `Hash` arms; `resolve` builds `S3Options::default().with_environment(session.reads_environment()).with_session(session.clone()).with_region(region.clone()).with_properties(store.iter().chain(properties.iter()))?` then `s3::located_with(&url.to_string(), options)` (`:139-144`) | pub: `s3/options.rs`, `s3/properties.rs` |
| s3tables | `s3tables/catalog.rs:642,645` | `S3Options::is_property` splits the property bag in the session's, the store's and the catalog's | pub |
| | `s3tables/catalog.rs:483,1151-1156` | `session.stated_region()` (crate-private, `aws/session.rs:705`) and the construction of `Site::Store` | |
| | `s3tables/catalog.rs:478` | doc: "`S3Client::session_of`" (a name that does not exist: the item is `Client::session_of`, `s3/client.rs:477`; stale doc) | |
| iceberg | `iceberg/staging.rs:263-276,286-298,306-312,319-325,337-344` | `upload` -> `S3File::upload_from` (**`pub(crate)` `s3/file.rs:344`**), `unpublished` -> `S3File::discard` (**`pub(crate)` `:401`**), `leaf`: `S3Path::as_file`, `container`: `S3Path::as_directory`, `sized`: `S3File::with_known_size` (pub `:125`) | 5 `Holder::S3*` patterns |
| | `iceberg/catalog/mod.rs:505-540` (`folder_role`) | `S3Folder(_)` kept, `S3Path::as_directory`, `S3File::as_directory` | 3 patterns |
| | `iceberg/table.rs:373,449,509` | `#[cfg(feature = "s3tables")]`: calls `s3tables::locate/create/open_or_create`; no `s3::` item named | s3tables only |
| crate root | `lib.rs:131-132` `#[cfg(feature = "s3")] pub mod s3;` and `:528-551` the 12 `internals as s3_*` | | |
| generated files | `.api-inventory.txt:2041` section `### yggdryl::s3  [rust/src/s3/mod.rs]` (70 lines match `s3` in the file); `scripts/generate_internals.py` writes the 12 `internals` lines | | |

A parallel reader's `holder_dispatch.md` in this directory maps the same `Holder` matches (six exhaustive: `exists`, `from_handle`, `as_io`, `as_io_mut`, `as_media`, `as_media_mut`; plus the silent wildcard arms such as `into_byte_stream`'s `other =>`) and agrees with the lines above.

Items the core calls on S3 types that are not public today: `S3File::upload_from` (`file.rs:344`), `S3File::discard` (`:401`), `S3File::byte_stream` (`:412`).
All three become part of what a registered backend must publish (3.2).

### 3.2 What a registered backend has to answer, read off the sites above

Not a design, only the verbs the 30 + 5 + 3 + 2 + 2 sites already call, in the order the core calls them:

1. the role value itself as `IOBase + IOMedia (+ IOPath | IOFolder | IOFile) + Debug + Send` (the four `as_*` matches; `Holder` is `Debug` only, not `Clone`: `holder/mod.rs:87`);
2. `exists` (`:201-206`), `byte_stream(position, batch_size) -> ByteStream<'static>` (`:241`);
3. reopen on the same client: clone for a folder, `parent().child_by_path(name)` for the other two (`from_handle`, `:518-532`);
4. role casts `as_directory()`, `as_file()` (staging `leaf`/`container`, `folder_role`);
5. `with_known_size(size)`, `upload_from(reader, size)`, `discard()` (staging `sized`, `upload`, `unpublished`);
6. `is_property(name)` and `from_url(location, properties)` keyed by scheme (`holder/mod.rs:392,404,414,597`; `s3tables/catalog.rs:642,645`);
7. `open_under_session(url, &aws::Session, region, store_properties)` for `Site::Store` (`warehouse/handle.rs:139-144`).

The third and fifth are the awkward ones for a `Box<dyn>`: `iceberg/staging.rs` today matches the concrete `S3File` to reach a streaming multipart upload; any
`Holder` can be written with `write_all_bytes`, but the staged-file path avoids reading the file whole (`staging.rs:255-259`).

### 3.3 Bindings, tests, benchmarks, docs, CI

| area | reach |
| --- | --- |
| Python | `python/src/iobase.rs` 34 lines (`Holder::S3*` arms at `:115-130,227-229,247-249,315-317`, role enum `:286-288`, `:403-405` the pyclasses, `:933,941` `S3Options::is_property`/`PROPERTY_NAMES`); `python/src/holder/handles.rs` 37 (classes `S3File`, `S3Folder`, `S3Path`, `:76-92,429-526,624-626`); `python/src/holder/fs.rs` 15 (`Provider`, `S3Options`, `s3::{path_at_with,file_at_with,folder_at_with}` for PyArrow filesystems `:938-992`); `python/src/http.rs` 1 (doc). `python/Cargo.toml:25` enables `"s3"`. |
| Node | `node/src/iobase.rs:130-131` (`yggdryl::s3::folder` -> `Holder::S3Folder`) and docs; `node/Cargo.toml:25` enables `"s3"`. |
| CLI | none. |
| tests | `rust/tests/s3.rs` + `rust/tests/s3/**` (19 files + the `s3.rs` harness: `aws/{mod_,xml}`, `azure/{auth,dialect,mod_,sign,xml}`, `client`, `encryption`, `file`, `folder`, `google/{mod_,token}`, `mod_`, `options`, `path`, `properties`, `provider`, `xml`), `rust/tests/interop/s3/{aws,azure,gcs,mod}.rs`; 210 `s3::`/`S3Options` references. 17 other test and bench files name the module or its types (65 references; `git grep -c -E 'yggdryl::s3|S3Options|S3Path|S3Folder|S3File|Holder::S3|feature = "s3"'`): `tests/holder.rs` 1, `holder/mod_.rs` 10, `iceberg.rs` 1, `iceberg/{catalog/mod_,scan,staging,table}.rs` 6+5+4+1, `interop.rs` 1, `iobase_calls.rs` 5, `warehouse.rs` 1, `warehouse/{handle,media}.rs` 4+4, `xml.rs` 1, `aws/{environment,sigv4}.rs` 4+3, `benchmarks/holder.rs` 2, `benchmarks/media/iceberg.rs` 12. (`uri/{arn,handle,url}.rs`, `medallion_ledger.rs` and `benchmarks/uri.rs` match a looser grep only through `arn:aws:s3:::` literals.) |
| benchmarks | `rust/benchmarks/holder/s3/{bytes,listing,mod,records}.rs`, `holder/aws.rs`; baseline `object_store` 0.13.1 + `futures` (dev-deps `rust/Cargo.toml:195-201`). |
| scripts / CI | `scripts/check_object_interop.py`, `check_azure_interop.py`, `check_gcs_interop.py`; `ci.yml:219-235` ("Build the S3, Azure and Google exchanges' Rust half", `--features s3 --test interop`), jobs `object-interop` (`:329-356`), `azure-interop`, `gcs-interop`; `.github/ci/rows.toml`. |
| docs | `docs/holder/index.md` (Object stores), `docs/uri/arn.md`, `docs/media/{iceberg,index}.md`, `skills/yggdryl-storage/**`, `skills/yggdryl-uri/**`, AGENTS.md rows "`holder/`", "`local/`, `fs/`, `zip/`, `s3/`", "`aws/`", "`auth/`", "`http/`", "`s3tables/`", section "### Object stores". |

### 3.4 `rust/Cargo.toml` features and dependencies

```
[features]                                                                       # rust/Cargo.toml:12-63
default = []
parquet = ["dep:parquet", "dep:snap"]
iceberg = ["parquet", "dep:iceberg-official", "dep:uuid"]
http = ["dep:psl", "dep:ureq"]
http2 = ["http", "dep:h2", "dep:rustls", "dep:tokio", "dep:tokio-rustls", "dep:webpki-roots"]
http3 = ["http2", "dep:h3", "dep:h3-quinn", "dep:quinn", "dep:rcgen"]
aws = ["http", "dep:hmac", "dep:ring", "dep:sha2"]
s3 = ["aws", "dep:md-5"]
s3tables = ["s3", "iceberg"]
internals = []
```

| dependency | turned on by | used by (outside `s3/`) | after the move |
| --- | --- | --- | --- |
| `md-5` 0.11 (`:93`) | `s3` | none (`s3/client.rs:3177`, bulk-delete `Content-MD5`) | s3-only: moves |
| `object_store` =0.13.1 [aws], `futures` 0.3 (dev, `:195-201`) | dev | `rust/benchmarks/holder/s3/{bytes,listing,mod}.rs` only | s3-only: moves with the benches |
| `tokio` `rt` (dev, `:210`) | dev | `benchmarks/holder/s3/mod.rs`, `tests/http/runtime.rs`, `tests/http/server/h2.rs` | shared |
| `hmac` 0.13 (`:90`) | `aws` | `aws/sigv4.rs`; `s3/azure/sign.rs:12` | shared: declared again in the leaf |
| `sha2` 0.11 (`:131`) | `aws` | `aws/sigv4.rs`; `s3/aws/options.rs:73`, `azure/sign.rs:13`, `encryption.rs:20` | shared |
| `ring` 0.17 (`:113`) | `aws` | `aws/{login,mod}.rs`, `http/tls.rs`; `s3/google/token.rs:446` | shared |
| `ureq` 3.4 (`:164`) | `http` | `http/*`, `aws/session.rs`; `s3/client.rs`, `azure/auth.rs`, `google/token.rs` | shared and crossing signatures (2.5) |
| `base64` (workspace) | always | `aws/login.rs`, `bytes.rs`; `s3/*` 16 uses | shared |
| `psl`, `h2`, `h3*`, `quinn`, `rcgen`, `rustls`, `tokio*`, `webpki-roots` | `http*` | `http/**` only | unaffected |

Consumers' feature lists: `python/Cargo.toml:22-27` and `node/Cargo.toml:22-27` (the `"s3"` entry at `:25` in both) are `features = ["http3", "iceberg", "s3", "s3tables"]`.
`s3tables = ["s3", "iceberg"]` is the line that changes: DESIGN.md D15 plans `s3tables/` as a feature of `yggdryl-iceberg` implying `yggdryl/s3`.

## 4. Part 3: who uses `aws/` and `auth/`, and the verdict

### 4.1 `aws/` outside `s3/` (`git grep -n 'aws::' HEAD -- rust/src ':!rust/src/aws' ':!rust/src/s3'`)

| consumer | sites | what it takes |
| --- | --- | --- |
| `s3tables/client.rs` | `:17` `credentials::Refusal`, `:18` `sigv4::{canonical_query, encode_query_component}`, `:19` `{Answer, Session}`, `:265` `.with_sigv4(&self.session, SERVICE, region)`, `:356` `aws::error_code`, `:480` `http::is_unanswered`, `:201` `ArnPartition::check_region`, `:530` `ArnPartition::is_opt_in` | `Answer` (`aws/mod.rs:88`), `error_code` (`:127`), `Refusal`, `sigv4::*` are `pub(crate)`; `is_unanswered` is `http/client.rs:1113` |
| `s3tables/catalog.rs` | `:63` `Session`, `:483` `stated_region` (s3-gated `pub(crate)`), `:592` `Session::from_properties`, `:638` `Session::is_property` | |
| `warehouse/handle.rs` | `:49` `session: crate::aws::Session` in `Site::Store` | a core enum variant holding the type |
| `http/` | `aws/request.rs:22` `impl Request { pub fn with_sigv4 }` (an inherent impl on `http::Request`); `http/request.rs:746-751` `pub(crate) fn with_resend_on` under `#[cfg(feature = "aws")]` ("The rule's one writer is `Request::with_sigv4`"); `http/client.rs:1112-1113` `is_unanswered` (`aws` or `internals`); `aws/request.rs:18` `http::request::host_header` (`pub(crate)`, `http/request.rs:1872`); `ResendOn` is a `pub(crate)` type (`http/request.rs:250`) | the HTTP client names no AWS item but carries three `aws`-gated hooks |
| `uri/arn.rs` | `:831,863,873-883,1086,1092` seven `aws`-gated sites: `check_region`, `is_opt_in`, the consts `MAX_REGION_BYTES`, `ECHO_BYTES`, `OPT_IN_REGIONS`, and the two `internals` twins; used by `aws/{login,session,sts}.rs`, `s3/client.rs`, `s3tables/client.rs` | `ArnPartition` is core vocabulary (`lib.rs:263`) |
| `xml/mod.rs:57` | `#[cfg(feature = "aws")] pub(crate) mod scanner`; users `aws/sts.rs:18` and `s3/xml.rs:10` | shared by `aws/` and `s3/` |
| Python / Node / CLI | none: `git grep 'yggdryl::aws\|aws::' HEAD -- python/src node/src cli/src` is empty | |
| tests | `rust/tests/aws/**` 14 files, `rust/tests/auth/**` 5 files | |

`aws/` names nothing of `s3/` or `s3tables/`: `git grep -E 'crate::(s3|s3tables|warehouse|iceberg|holder)\b' HEAD -- rust/src/aws` is empty.
`auth/` names no `aws/` or `s3/` item either (only the module doc, `auth/mod.rs:9`).

### 4.2 `auth/` outside `aws/` and `s3/`

`http/authorization.rs:12` (`Secret`), `http/client.rs:571`, `http/session.rs:391,475`, `http/tls.rs:36` (`variable`). So `auth::{Secret, variable, environment}` (the
`http` feature half) are core's own; `auth::{Lease, Expiring, Report, Environment, instant, instant_from_millis, iso8601, write_private}` (the `aws` half,
`auth/mod.rs:20-36`) are used by `aws/` in 8 files and `s3/` in 3 (`Lease` x2, `Expiring` x1, `instant` x1); `Bearer` (`s3` half) by `s3/` alone.

### 4.3 The verdict

**`aws/` and `auth/` stay core.** Reasons, each pinned above:

1. `http::Request::with_sigv4` is an inherent impl on a core type (`aws/request.rs:22`); out of the `yggdryl` crate it is error E0116. Moving `aws/` means
   rewriting the door as an extension trait or a free function and publishing `with_resend_on`, `ResendOn` and `host_header` first.
2. `s3tables/` (to live in `yggdryl-iceberg`) needs `aws::{Session, Refusal, Answer, error_code, sigv4::*}` and `with_sigv4`. If `aws/` lived in `yggdryl-s3`,
   an Iceberg catalog consumer would compile the object-store backend only to sign a REST-JSON call.
3. `warehouse/handle.rs:45-52` makes `aws::Session` part of a core enum; a leaf-crate `Session` would be a cycle there.
4. `auth/` is two halves; the `http` half cannot leave (4.2), and the `aws` half is `aws/`'s.
5. No binding names `aws/` (4.1), so keeping it core costs the bindings nothing.

**Dependency directions** (no cycle, the core depends on no leaf):

```
yggdryl-s3            -> yggdryl [features: aws (-> http)]            + direct ureq, hmac, sha2, ring, base64, md-5
yggdryl-iceberg       -> yggdryl [features: parquet, iceberg's deps; aws under its s3tables feature]
yggdryl-iceberg[s3tables] -/-> yggdryl-s3  (compile time)               the table's store is opened through the new register;
                                                                         `S3Options::is_property` becomes the backend's `is_property`
bindings (python, node) -> yggdryl, yggdryl-iceberg, yggdryl-s3         one native module links all and calls each `install()` (D11 option d)
```

The alternative arrow `yggdryl-iceberg[s3tables] -> yggdryl-s3` (D15's "implying yggdryl/s3" kept literally) is acyclic too; it buys direct access to
`S3Options` and costs the compile-time edge. The register route needs the 7 verbs of 3.2, of which `open_under_session` is the only one that names
`aws::Session` (core, so legal).

**What moves with `s3/` out of the core's `aws`-adjacent code** (2.6), by feasibility:

| fragment | move to the leaf | re-key to `aws` in the core | blocker |
| --- | --- | --- | --- |
| `auth::Bearer` | yes (needs `Expiring`, `Secret` published) | | none |
| `aws::environment::is_native` | yes (a pure list) | | none |
| `Session::tls_config` | yes (`Session::ca_bundle` is `pub`, `session.rs:1317`; `ureq` is then a direct dep) | | none |
| `sigv4::encode_key`, `Signer::access_key_id` | | yes (3 lines) | none |
| `xml::scanner::{children, required}` | | yes | none |
| `http::record_process` | | yes (it is the process ledger of every client) | none |
| `Session::{stated_region, states_identity, under}` | | raise to `pub` | `under` reads `Knobs` (private) |
| `Session::{bucket_region, learn_bucket_region}` + `bucket_regions` | no | keep in `Session` | state is shared by session clones through an `Arc` built in `from_knobs` (`session.rs:343-362`) |

**What stays in the core for `s3/`'s and `s3tables/`'s sake and is not s3 vocabulary:** `Scheme::{is_s3, is_gs, is_az, is_s3_tables, is_object_store, has_container}`
(`scheme.rs:263-335`; used by `fs/location.rs:450`, `holder/mod.rs:384,595`, `warehouse/handle.rs:95`, `warehouse/catalog.rs:351`), the store-location walk
(section 5), `ArnPartition` including its `aws`-gated region rules, `Holder::from_url`'s scheme dispatch.

## 5. The S3 authority rule and region inference: who owns them

Core `uri/`, not `s3/`:

| fact | owner |
| --- | --- |
| which schemes name a container | `scheme.rs:263-335` (`is_s3` = `s3`/`s3a`/`s3n`, `is_gs`, `is_az`, `is_s3_tables`, `is_object_store`, `has_container`) |
| the walk: endpoint, container, key | `uri/authority.rs:311` `Uri::store_location` (one walk for all three stores), result type `StoreLocation` `:412-422` |
| "which first part is a hostname" | `uri/authority.rs:554-563` `is_store_hostname`: `localhost`, an IP literal, or a name ending `.com`, `.io` or `.net` (ASCII case-insensitive); a port decides earlier (`:343`) |
| per-store hostnames and regions | `parse_aws_s3_hostname` `:579`, `parse_google_hostname` `:475`, `parse_azure_hostname` `:522`, `google_endpoint_region` `:503` |
| public accessors | `Uri::{hostname, store_endpoint, bucket, account, region, is_virtual_hosted, key}` `uri/mod.rs:544-617`; `Url` delegates `uri/url.rs:277-312` |
| region rule used by every AWS client | `ArnPartition::{from_region, dns_suffix, check_region, is_opt_in, service_host}` `uri/arn.rs:727,748,832,864,800` |

`s3/` only consumes them (`Url::bucket` at `client.rs:408,610`, `mod.rs:336,365`; `Url::key` `mod.rs:366`; `store_endpoint` `client.rs:575`; `account`
`client.rs:545`; `region` `client.rs:808`). The authority rule therefore does not move; `rust/tests/uri/authority.rs` and `uri/url.rs` pin it in the core.

## 6. Findings worth carrying into the design

1. **Closed `Holder` is the blocker, the 58 forwards are the chore.** The register must give `Holder` a boxed variant before the folder can move (30 + 12 sites);
   the existing `Locator` register cannot do it because its product is a `Holder`.
2. **`ureq` crosses four crate-private signatures** (2.5): decide pin-and-depend versus wrap.
3. **`with_sigv4` blocks moving `aws/`** (4.3 reason 1); keep it core.
4. **8 `s3`-gated fragments at 23 sites in the core** (2.6) are the leak the cut exposes: move three (`environment.rs`, `tls_config`, `Bearer`), re-key three
   (`encode_key`, `record_process`, `Element::{children, required}`), raise one (`Session::{under, stated_region, states_identity}`), keep one (bucket-region memory).
5. **`Site::Store`** (`warehouse/handle.rs:45`) is the single core place that both names `aws::Session` and constructs `S3Options`; D30 already marks it
   "gated to `s3` alone, with a targeted `dead_code` allowance until S6's implementer door constructs it from outside".
6. **`S3Options::is_property` is read by four core sites** (`holder/mod.rs:392,597`, `s3tables/catalog.rs:642,645`): it is the backend's vocabulary and should be
   a register method, not a name the core keeps importing.
7. **Stale doc** `s3tables/catalog.rs:478` names `S3Client::session_of`; the item is `Client::session_of` (`s3/client.rs:477`).
8. **Doc drift** `.com`/`.io` versus `.com`/`.io`/`.net` (verdict item 10).
9. **Dependencies**: `md-5` and the S3 bench baseline (`object_store` with its `aws` feature, `futures`) become the leaf's alone, so the core's test graph sheds
   the `object_store` tree and `md-5`; `hmac`, `sha2`, `ring`, `base64`, `ureq` stay declared in both.

## 7. Not verified

- No compile: every visibility is read from the definition, and every "used by" from `git grep`; the first `cargo check -p yggdryl-s3` after a scratch
  `git mv` remains the authority for the method-reached residue (D6: "S6's `cargo check -p` finds the method-reached residue").
- Trait-method reach through `IOBase` defaults (e.g. `children_where`) was not enumerated: `IOBase`, `IOMedia`, `IOFile`, `IOFolder`, `IOPath` are `pub` traits with no
  sealed supertrait (`iobase.rs:283`, `iomedia.rs:192`, `iofile.rs:11`, `iofolder.rs:15`, `iopath.rs:10`), the `#[doc(hidden)]` items at `iomedia.rs:194,198,338,628`
  are `pub`, so an external implementer compiles; `impl_default_iomedia!` expands to public paths only.
- The Python class layout for a registered backend (the `Role` enum at `python/src/iobase.rs:286-288`) was counted, not designed.
