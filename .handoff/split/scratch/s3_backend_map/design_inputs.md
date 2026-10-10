# D36 design inputs: `s3/` as `yggdryl-s3`

Anchors: `git show HEAD:` at `d100439d3`, `rust/src/` dropped; the working tree touches none.

## Part A - the facts

### A1. `Holder`'s S3 variants, every match
- holder/mod.rs:87-172: `#[derive(Debug)] #[non_exhaustive] pub enum Holder`, 25 variants, not Clone/Eq/Hash; S3 is three inline variants `S3Folder`/`S3Path`/`S3File` :107-114. None holds a `Box<dyn ..>`; doc :60-66 refuses `Box<dyn IOBase>` (it "would erase the concrete type"). `Arc<Holder>` in `Site::Native` (warehouse/handle.rs:29) makes a boxed variant `Send + Sync + Debug`, as `MediaWrapper` is (media/codec.rs:142).
- Exhaustive (holder/mod.rs): `exists` :191-223 (`folder_exists`/`path_exists`/`file_exists`, `(&self) -> bool`, the core's one call into the role traits); `from_handle` :498-558 (S3 :518-532: folder cloned; path/file via `sibling` :47 = `parent()` + `child_by_path`, then `S3Path::as_directory(&self) -> Result<S3Folder>` path.rs:151 / `as_file(&self) -> Result<S3File>` :160; the `Arc<Client>` is the identity, `S3File`'s `Drop` publishes, file.rs:1034); `as_io`/`as_io_mut`/`as_media`/`as_media_mut` :905-1054, every forward of `Holder` (:1057-1312).
- Wildcards whose fall-through changes behaviour (silent):
  - `into_byte_stream` :233-247 (bytestream.rs:451,460, a container's leaves): `S3File::byte_stream(&self, position: u64, batch_size: usize) -> Result<ByteStream<'static>>` pub(crate) file.rs:412, one eager resuming `GET`; else `Cursor::at` + `from_reader`, one ranged `GET` per batch.
  - iceberg/staging.rs `upload` :264-276: `S3File::upload_from(&mut self, source: &mut dyn Read, length: u64) -> Result<()>` pub(crate), `cfg(any(iceberg, internals))`, file.rs:343 (multipart); else the file read whole + `write_all_bytes`.
  - `unpublished` :286-298: `S3File::discard(&self) -> Result<()>` pub(crate) file.rs:401; else `remove(false)` unless `keeps`: a `DELETE` for a key never written.
  - `leaf` :306-312, `container` :319-325: `S3Path::as_file`/`as_directory`; else undecided, a listing at the first verb.
  - `sized` :338-344: `S3File::with_known_size(self, size: u64) -> Self` pub file.rs:125; else a `HEAD` per file.
  - iceberg/catalog/mod.rs `folder_role` :505-540: + `S3File::as_directory(&self) -> Result<S3Folder>` file.rs:168; else refused (loud).
  - python/src/iobase.rs `Role::of` :339 `_ => Self::Held` (the base class); `container_holder` :230.
- On the traits today: `IOBase::owned_stream_bytes(&self, position) -> Result<Option<Box<dyn Read + Send>>>`, default `None` (iobase.rs:321), overridden by S3 lazily (file.rs:564: the same resuming `GET` at the first read, its error `io::Error::other`); `bound_location` (read by `from_handle`); `IOMedia::as_any -> Option<&dyn Any>`, default `None` (iomedia.rs:368), forwarded by `Holder`. No `IOBase::exists`.

### A2. Reach across the cut
- `s3/`: 26 files, 11,963 lines; 32 public core names and 58 non-public items (51 crate-private, 7 `pub` in private modules; * = gated `s3` today):
  - aws/: `sigv4::{Signer, encode_key*, encode_query_component, canonical_query, signed_access_key, sha256_hex, EMPTY_PAYLOAD_SHA256, UNSIGNED_PAYLOAD}`, `properties::{EndpointName, Identity, count, flag, refusal, seconds}`, `environment::is_native*`, `Session::{signer, answers_another, given_variables, tls_config*, bucket_region*, learn_bucket_region*, stated_region*, states_identity*, under*}`.
  - auth/ (private, lib.rs:28): `Bearer*`, `Expiring`, `Lease`, `instant`, `variable`.
  - http/: `retry::{RETRY_BACKOFF, RETRY_COST, RETRY_REFUND, RetryBudget, backoff, delay, fresh_jitter, retry_after, is_retryable_transport, is_unsent, is_resumable}`, `client::agent_for`, `record_process*`.
  - xml/: `scanner::{parse_document, parse_root, Element, XmlError}`.
  - holder/iobase/uri: `holder::system_time_ns`, `iobase::oversized`, `ByteStream::from_handle` (pub(super)), `uri::{percent_decode, percent_encode_segment}`, `ArnPartition::check_region` (gated `aws`).
  - other: `integer::{BYTE_COUNT_SPELLINGS, byte_count_from_text, integer_from_scalar_as}`, `boolean::bool_from_text`, `warned!`, `xxhash::stream::read_range_digest`.
  - `ureq` types sit in four signatures (`tls_config`, `agent_for`, `is_retryable_transport`, `is_unsent`); no AGENTS rule covers ureq. 23 `cfg(feature = "s3")` sites in aws/auth/http/xml serve `s3/` alone.
- The core into `s3/`, beyond A1 and A4: warehouse/handle.rs `Site::Store { url, session: aws::Session, region, store: Properties }` :45-52, its `resolve` :129-145 building `S3Options` over the session + `s3::located_with`, and `Site::of` :95 (`is_object_store() || is_http()` -> `Native`); iceberg/table.rs:535-548 (`is_backend_property`); s3tables/catalog.rs:642,645 (`S3Options::is_property`), :1148-1161 (the one `Site::Store` constructor); lib.rs:131, :528-551 (12 `internals`). Python 87 lines, Node 2, CLI 0.

### A3. `aws/` and `auth/`
- Beside `s3/`: s3tables/client.rs:17-19,201,231,265,356,480,530 (`Refusal`, `sigv4::{canonical_query, encode_query_component}`, `Answer`, `Session::service_endpoint`, `with_sigv4`, `error_code`, `ArnPartition::{check_region, is_opt_in}`, `is_unanswered`) and catalog.rs:63,483,592,638 (`Session::{stated_region, from_properties, is_property}`); warehouse/handle.rs:49; http/request.rs:750 `with_resend_on` (gated `aws`); uri/arn.rs (7 `aws` sites); aws/sts.rs:18 (`xml::scanner`). auth's `Secret`/`variable` are http's own (5 sites in http/). No binding names `aws::`; `aws/` names nothing of `s3/`.
- rust/Cargo.toml:44-57: `aws = ["http", hmac, ring, sha2]`, `s3 = ["aws", md-5]`, `s3tables = ["s3", "iceberg"]`; both bindings enable `s3`, `s3tables`. D15 (DESIGN.md:379): `s3tables` a feature of `yggdryl-iceberg` "implying `yggdryl/s3`".
- aws/request.rs:22,72: `impl Request { pub fn with_sigv4(self, session: &Session, service: impl Into<SmolStr>, region: impl Into<String>) -> Self }`, inherent on `http::Request`, calling `with_resend_on` and `host_header` (both pub(crate), http/request.rs:750,1872). An inherent impl compiles only in the type's crate (E0116): elsewhere an extension trait (`trait SignV4 { fn with_sigv4(self, ..) -> Self }`, imported per call) or a free fn (`sign_v4(request, &session, service, region)`), either needing `with_resend_on`, `ResendOn`, `host_header` published.

### A4. `from_url`, `from_handle`, the locator
- holder/mod.rs:355-446: properties as `Vec<(String, String)>` :364; `locator::locate` before lowering, answer returned as is (:371-373, no `described`); `location.locator()?` :374 (an S3 ARN -> `s3://`, uri/arn.rs:601); local/zip :375; `is_object_store()` :384 = `s3 s3a s3n gs gcs az abfs abfss wasb wasbs` (scheme.rs:263-310; not `s3tables`): a query parameter `S3Options::is_property` refuses is `Error::Parse` (:390-403), `from_properties(query, then caller)` :404, query stripped :412, `s3::located_with` -> `S3Path` :414; no-`s3` refusal :416-422 (unpinned); http :423; else the install sentence :438-444; `described` :452-470 applies `media_type`/`codec` on every backend (no s3 test pins it).
- `S3Options::is_property` (s3/properties.rs:178) runs the reader on a probe; Python reads it too (handles.rs:420, iobase.rs:933).
- `Locator` (holder/locator.rs:23-40): `scheme() -> Scheme` (one key), `names(&Uri) -> bool`, `holder(&Uri, &Properties) -> Result<Holder>`; seeds `S3TABLES_LOCATOR` alone; `locate` :97 clones `values()`, asks each `names` per `from_url`; doc :4-10 keeps byte backends the core's. D21 (DESIGN.md:510): "`Locator`: `holder(location, properties) -> Holder` | `Holder::from_url`'s per-scheme arms"; D30 (:1295-1298) kept the `s3` arm and gated `Site::Store` "until S6's implementer door constructs it from outside".
- `plugin::Register::{claim, get, claimant, values}` (plugin.rs:56-99); all-or-none multi-key claim: media/codec.rs:209-252.

### A5. Pins that stay
- tests/s3.rs + s3/**: 239 tests (85 without `internals`, 0 in today's default lane); counts in s3/{file,path,folder}.rs `accounting` (+ file.rs `create`, `tail`), s3/mod_.rs :194,:236 and `accounting::iceberg` :264-937 (5, `cfg(iceberg)`, docs/holder/index.md:4352).
- Core-resident: iobase_calls.rs :512, :532; holder/mod_.rs :686, :737; warehouse/handle.rs:139; warehouse/media.rs:235; iceberg/catalog/mod_.rs:1412, :1514; iceberg/scan.rs:1918; iceberg/staging.rs:325; iceberg/table.rs:3569. No `allocations` row.
- `FakeS3` (tests/support/server.rs, crate-free): `#[path]` in seven harnesses, two benches.
- s3tables: tests/s3tables/catalog.rs (19 on `FakeS3`; docs/media/iceberg.md:1424-1439), s3tables_handle.rs:25.
- Exchanges: tests/interop/s3/*.rs (5+8+7); scripts/check_{object,azure,gcs}_interop.py:334/208/212 (`--features s3 --test interop s3::<dialect>::`); ci.yml `core-interop` :217-243, jobs :329-413; rows.toml :269-282 `extends = ["interop"]`.
- Docs: docs/holder/index.md :4255-5172 (costs :4325-4346; :4527-5007 is `aws/`'s); AGENTS.md :1342-1413; docs/media/iceberg.md :1416-1439, :1558-1568.

### A6. Bindings
- Python (one native module, D11 option d): classes `S3File`/`S3Folder`/`S3Path` (holder/handles.rs:75-95, constructors :429-521 over `yggdryl::s3::{located_with, file_with, folder_with, *_at_with}`); holder/fs.rs:923-1076 (PyArrow stores held natively); iobase.rs `cloned` :101-145 (from_handle's S3 arms again), `folder_holder_for` :178, `container_holder` :217, `Role` :278-341. The raw-name doors are crate API, not the register's. Precedent: python/src/warehouse.rs:59-80 picks a class by `downcast_ref`.
- Node: node/src/iobase.rs:128 `folder_holder_for` (`yggdryl::s3::folder`) alone.
- Inventories: .api-inventory.txt:2041-2130, :2735; .api-bindings.txt:141-147.

### A7. CI leaf
- rows.toml:90-97 `[leaves]`: a line is row `crate-s3` (plan.py:167-172: `corelib` + `after`, `rust/s3/**`, jobs `leaves` + `[leaf]` + its own); `check_leaves` (plan.py:348) refuses a crate with no line and the reverse.
- Breaks: test_ci_plan.py:37 `LEAF_LINE` `[a-z]+` cannot read `s3`; `NEVER` :322-326 forbids the three exchanges for every leaf; `EXCHANGES` :312-320 lacks `s3`; :255-262 expect `core-interop`; x-* rows take `extends = ["crate-s3"]` with the line (plan.py:112).

## Part B - three shapes, no recommendation

All three keep the S3 trio's own `IOBase` impls, so a verb sends the same requests; construction, child resolution and a reopen stay request-free (the `Arc<Client>` travels); the register is `plugin::Register<Scheme, &'static dyn ..>`, seeded under `CORE` until `yggdryl_s3::install()` claims the ten schemes all-or-none. No count pin moves unless a wildcard keeps its fall-through.

`aws/`+`auth/`, any shape:
- (a) core, under `aws`: the leaf takes the 58 items through `implementer` (ureq in four); the 23 s3-only gates move or re-key; `with_sigv4` untouched; `s3tables` needs `yggdryl[aws]`, and no `yggdryl-s3` edge if `Site::Store` resolves through the register.
- (b) in `yggdryl-s3`: `with_sigv4` becomes a trait or a free fn, http publishes `with_resend_on`/`ResendOn`/`host_header`, auth splits (`Secret`/`variable` stay), arn region rules and `xml::scanner` published, `Site::Store` cannot name `aws::Session`; `yggdryl-iceberg[s3tables] -> yggdryl-s3` (object stores compiled to sign REST-JSON); tests/aws (14 files), tests/auth (5) move.
- (c) a tenth crate `yggdryl-aws`: (b)'s costs; `s3 -> aws`, `iceberg[s3tables] -> aws`; a tenth `[leaves]` line, `after = ["aws"]` on `s3` and `iceberg`.

### (i) `Holder::Registered(Box<dyn RegisteredHandle>)` + `StorageBackend`
1. `RegisteredHandle: IOBase + Send + Sync + Debug`: `implementation_name`, `as_any(_mut)`, `exists`, `reopen -> Result<Holder>`, `byte_stream`, `upload_from`, `discard`, `as_file`/`as_folder -> Result<Holder>`, `with_known_size(self: Box<Self>, u64)`; capabilities default to today's fall-through.
2. `StorageBackend`: `name`, `schemes() -> &'static [Scheme]`, `is_property(&str)`, `holder(&Url, &[(String, String)]) -> Result<Holder>`; asked after lowering, before http; `described` still runs.
3. Exhaustive `exists`, `from_handle`, `as_io(_mut)`, `as_media(_mut)`: three cfg arms out, one `Registered` arm in.
4. Wildcards stay, each given an explicit `Registered` arm (`into_byte_stream`, staging x5, `folder_role`, Python `Role::of`/`container_holder`); a miss is green, caught only by iceberg/staging.rs:325, s3/mod_.rs `accounting::iceberg`, iobase_calls.
5. `is_backend_property`, s3tables/catalog.rs:642,645 -> `backend_for(scheme)?.is_property`.
6. `Site::Store`: kept, gated `aws`, opened by `StorageBackend::holder_under(url, &Session, region, props)` (needs (a)); or `Site::Opened { url, open: Arc<dyn Fn(&Properties) -> Result<Holder> + Send + Sync> }`, Eq/Hash by url, built by s3tables over `yggdryl_s3` (an iceberg -> s3 edge; any placement).
7. Sweep: holder/mod.rs ~58 lines, iceberg 8 arms, handle.rs ~9, s3tables 3, the leaf's 8 `Holder::S3*` builds, Python ~87, Node 2, tests 14 `Holder::S3` lines + 11 pins' imports, docs, inventories.
8. CI: `s3 = { package = "yggdryl-s3", jobs = ["object-interop", "azure-interop", "gcs-interop"] }`; iceberg `after += "s3"` only with the opener.

### (ii) the verbs as `IOBase` defaults, `Registered(Box<dyn IOBase + Sync>)`
1. `IOBase` gains defaulted `exists`, `reopen -> Result<Option<Holder>>`, `upload_from` (whole + `write_all_bytes`), `discard -> Result<bool>` (false: caller removes), `as_leaf`/`as_container -> Result<Option<Holder>>` (None: as is), `set_known_size(&mut self, u64)` (no-op).
2. `into_byte_stream`: a `self: Box<Self>` default, or the wildcard reads the existing `owned_stream_bytes` (S3's lazy one: same one `GET`, refused at the first read as `io::Error::other`).
3. `dyn IOBase` is neither `Sync` nor `Debug`: the box names `+ Sync`, and `Holder`'s derived `Debug` needs a hand-written arm or `Debug` on `IOBase` (every implementer).
4. Exhaustive: the same six gain one arm; wildcards vanish from staging, `into_byte_stream`, `folder_role` (about 10 cfg lines leave iceberg/), and the `FsFile`/`ZipLeaf` arms may fold in.
5. Wrappers (`delegate_iobase!`, `Buffered`, `Coded`, `Text`, `Media`) must forward the new verbs or lose them for what they wrap.
6. No trait of its own: a binding's class pick reads `IOMedia::as_any` (today the medium's own state) answering the handle.
7. Register, `is_property`, `Site::Store`: as (i).
8. Sweep: iobase.rs gains six public defaults (rustdoc, inventory, holder page); iceberg shrinks; bindings as (i). CI as (i).

### (iii) `Locator` generalised (D21:510) + `Registered` as (i)
1. `Locator` gains `schemes()` (ten claims, or all-or-none), `is_property`, and answers byte handles; the leaf claims its schemes on `LOCATORS`.
2. Asked before lowering: an `arn:aws:s3:::` location arrives unlowered, so its `names` reads ARNs (`names_s3_tables`, `arn::service_of` are pub(crate)).
3. Its answer returns as is (holder/mod.rs:371-373): `media_type`/`codec` stop reaching s3 unless the answer says "bytes" or `from_url` describes every answer (today a locator's objects declare no media type); no test turns red.
4. `locate` walks every locator per `from_url` and builds a `Properties` per hit; the query merge and refusal move into the S3 locator over `Properties` (one value per name).
5. Exhaustive, wildcards, `Site::Store`, bindings: as (i).
6. D30:1295 and locator.rs:4-10 rewritten: one register for catalog services and byte backends.
7. Sweep: (i)'s plus locator.rs; holder/mod_.rs:686's refusal text moves byte-identical. CI as (i).

### Open questions
1. A `StorageBackend` register beside `Locator`, or `Locator` widened?
2. Capabilities on a trait of their own (i) or on `IOBase` (ii); do wrappers forward them?
3. `into_byte_stream`: keep S3's eager `byte_stream`, or read `owned_stream_bytes`?
4. `Site::Store`: `holder_under(&Session)` on the backend, or an opener s3tables builds?
5. `aws/`+`auth/`: core, `yggdryl-s3`, or `yggdryl-aws`?
6. D15: `s3tables` implies `yggdryl/aws`, or depends on `yggdryl-s3`?
7. `ureq` types in published signatures, or wrapped?
8. `accounting::iceberg` (5) and `FakeS3`: in the leaf with an iceberg dev-dependency, or re-homed?
9. The 11 core-resident pins: dev-depend on the leaf and `install()`, or move?
10. Python class by `downcast_ref` or `implementation_name`; `cloned` replaced by `from_handle`?
11. Node `folder_holder_for`: link the leaf, or a container-role door on the register?
12. Write the missing s3 `media_type`/`codec` `from_url` pin before the cut?
13. `from_url("s3://..")` before `install()` refused for every pure-Rust caller (ISIN registry, plans, CLI)?
14. Does a backend claim refuse `file`/`http`/`https`?
15. Names: `StorageBackend`, `RegisteredHandle` or `RegisteredHolder`, `claim_backend`, `backend_for`.
