# yggdryl-s3: the precedent and the backend point

Read at HEAD `d100439d3a1d793475e7168a404463ef903f6940` ("Record the S3 design"), every
`rust/src`, `rust/tests`, `python/src`, `node/src`, `cli/src`, `.github` and `AGENTS.md` line through
`git show HEAD:<path>`. `.handoff/` read from the working tree: only
`.handoff/next/MARKET_SPLIT_NEXT.md` differs from HEAD (it carries the uncommitted D36 row, line 208);
`.handoff/split/DESIGN.md` is unmodified. No cargo command ran, nothing was edited. Every "crate-private
reach" figure below is by grep, which is orientation exactly as D24 words it (DESIGN.md:665-671); the
compiler's list comes from a scratch `git mv` plus `cargo check -p yggdryl-s3`.

Path shorthand: `rust/src/` is dropped (`s3/path.rs:254` is `rust/src/s3/path.rs:254`).

## 0. The answer in twelve lines

1. The register mechanism (`plugin::Register`, `plugin.rs:35-70`) and the claim/seed/refusal shape are reusable
   verbatim. A backend is one more claim kind on it (market kinds, logical names, media, table formats, catalog
   factories and locators hold it today) and the second whose product is a `Holder` (the locator is the first,
   `holder/locator.rs:39`).
2. `Holder` is **not** `Box<dyn IOBase>`: it is a 25-variant concrete enum (`holder/mod.rs:90-172`) whose
   `as_io()` is a `match` returning `&dyn IOBase` (`holder/mod.rs:905-940`). Its own doc says why:
   "`Box<dyn IOBase>` would erase the concrete type a caller needs to match on" (`holder/mod.rs:62-66`). The
   three S3 roles are three variants (`holder/mod.rs:106-114`). So `Holder::Registered(Box<dyn RegisteredHolder>)`
   is a **new** variant, the way `Media::Registered(Box<dyn MediaWrapper>)` (`media/mod.rs:96`) and
   `Table::Registered(Box<dyn RegisteredTable>)` (`warehouse/table.rs:106`) are; it needs one IOBase object, not
   the three role traits.
3. `IOPath`/`IOFolder`/`IOFile` (`iopath.rs:10`, `iofolder.rs:15`, `iofile.rs:11`) are `: IOBase` helper traits
   whose provided methods (`path_kind`, `folder_ls`, `file_kind`, ...) the S3 types call from their own
   `IOBase` impls. The core calls them in exactly one place, `Holder::exists` (`holder/mod.rs:191-223`). They
   stay inside the leaf crate; the core sees one `exists()`.
4. A backend differs from a medium in six ways that matter (section 5): scheme-keyed with several keys per
   claim; the product is a recursive `Holder` the leaf must construct from inside its own trait impls
   (`parent`, `child_by_path`, `ls` return `Holder`); construction must send zero requests; the identity of a
   handle is its client (reopen must reuse it); staged state publishes on `Drop`; and the core special-cases the
   S3 variants in ten more places (six in `iceberg/`, then `Holder::{exists, into_byte_stream, from_handle,
   is_backend_property}`).
5. The core owns, and keeps, the URL vocabulary: ten `Scheme` spellings with `is_s3/is_gs/is_az/is_object_store/
   has_container` (`scheme.rs:263-336`), the whole bucket/key/account/region/endpoint walk (`uri/authority.rs:304-611`),
   and the ARN lowering (`uri/arn.rs:601,606`). That is `MimeType::PARQUET`'s analogue and stays.
6. Nothing in the core opens `s3://` by a literal URL (grep, section 4.6). Every consumer goes through
   `Holder::from_url`. The core's S3 coupling is 146 lines in 15 files outside `rust/src/s3/`
   (`holder/mod.rs` 58, `lib.rs` 26, `iceberg/staging.rs` 12, `aws/session.rs` 11, `warehouse/handle.rs` 9,
   `iceberg/catalog/mod.rs` 6, `scheme.rs` 5, `s3tables/catalog.rs` 5, `auth/lease.rs` 4, `aws/sigv4.rs` 3,
   `xml/scanner.rs` 2, `s3tables/mod.rs` 2, `http/mod.rs` 1, `aws/mod.rs` 1, `auth/mod.rs` 1); 79 of them are
   `cfg(feature = "s3")` gates.
7. The hard problem is not the register. It is (a) `Request::with_sigv4`, an inherent method of `http::Request`
   written in `aws/request.rs:22-88`, which cannot be written from another crate, so `aws/` stays in the core
   (recommendation Q5); (b) about twenty crate-private items the 11,963-line `s3/` reaches (section 3.9); (c) the
   six S3-variant seams in `iceberg/` (`staging.rs` five functions, `catalog/mod.rs` one) and
   `Holder::into_byte_stream`, which become trait capabilities.
8. `Site::Store` (`warehouse/handle.rs:45-52`), the only core variant named after the `s3` feature, should resolve
   through the register (`Backend::holder_under`, Q4), so `warehouse/` names no store type and D15's "implying
   `yggdryl/s3`" becomes `yggdryl/aws`: `yggdryl-iceberg` then has no edge to the leaf.
9. D30 decided the opposite of this request in one sentence: "`Holder::from_url` asks the locators first and keeps
   every byte-backend arm (`local`, `zip`, `s3`, `http`), which stay the core's" (DESIGN.md:1292-1298), and
   D15 fixed `s3tables` as a feature of `yggdryl-iceberg` "implying `yggdryl/s3`" (DESIGN.md:377-382). D36 amends
   both: the `s3` arm leaves the core, and D15's implied feature becomes `yggdryl/aws` (Q4) rather than a dependency on
   `yggdryl-s3`, so the S6 chain avro, parquet, iceberg stays free of an s3 edge.
10. Recommendation in one line: a new `holder/backend.rs` register (`Backend`, keyed by `Scheme`, several keys per
    claim, all-or-none), a new `Holder::Registered(Box<dyn RegisteredHolder>)` variant that replaces
    `S3Folder`/`S3Path`/`S3File`, `reopen` on the object not the register, capabilities as defaulted trait methods,
    `aws/`+`auth/` stay core, `Site::Store` resolves through `Backend::holder_under`, registered in place first (P3),
    folder moved after (S6d), exactly the S2 -> S6 order.
11. The one rejected alternative that deserves a sentence: implement `fs::FileSystem` (`fs/system.rs:172-251`)
    in the leaf and surface as `Holder::FsPath`. It would reuse an existing dyn plug point but loses the request-count
    contract (section 5.2).
12. Size: the leaf would be 26 files / 11,963 lines of source, 20 files / 8,742 lines of tests, 8 files / 1,423
    lines of interop and benchmarks, plus the 3,869-line fake store `tests/support/server.rs` that seven harnesses share.

## 1. What was read (so a reviewer can re-read)

| Source | Lines used |
| --- | --- |
| `.handoff/next/MARKET_SPLIT_NEXT.md` "## Goal", "## Invariants" (working tree) | 9-58 |
| `.handoff/split/DESIGN.md` D6 199-221, D7 223-262, D8 263-287, D11 314-345, D15 377-382, D21 501-530, D24 627-675, "S2: design" 1113-1126, D26 1127-1180, D27 1182-1229, D29 1245-1272, D30 1274-1303, D31 1305-1327, the review's amendments 1329-1365, D33 1380-1442, S2 results 1645-1682, the slice table 714-734, "The CI structure" 1877-1937 | as listed |
| `media/codec.rs` | 1-310 (whole) |
| `holder/locator.rs` | 1-109 (whole) |
| `warehouse/catalog.rs` | 1-440 |
| `media/format.rs` | 1-60, 182-228 |
| `warehouse/table.rs` | 1-160 |
| `plugin.rs` | 1-134 (whole) |
| `holder/mod.rs` | 1-1465 (whole) |
| `s3/mod.rs`, `s3/path.rs`, `s3/folder.rs`, `s3/file.rs` (struct, constructors, IOBase impl heads, Drop), `s3/provider.rs`, `s3/properties.rs`, `s3/options.rs` 1-200, `s3/client.rs` 1-140, 190-520 | cited per claim |
| `local/mod.rs`, `fs/mod.rs`, `zip/mod.rs`, `fs/system.rs`, `fs/path.rs` 1-60 | whole / cited |
| `scheme.rs`, `uri/authority.rs` 290-470, `fs/location.rs` 415-473 | cited |
| `warehouse/handle.rs`, `s3tables/catalog.rs` 395-500, 620-680, 1120-1175, `iceberg/staging.rs` 240-345, `iceberg/catalog/mod.rs` 485-545, `iceberg/table.rs` 525-560 | cited |
| `aws/mod.rs`, `aws/request.rs`, `aws/properties.rs` 1-260, `aws/session.rs` (cfg gates), `auth/*` (cfg gates), `lib.rs`, `rust/Cargo.toml`, `.github/ci/rows.toml` 74-112 | cited |
| `python/src/iobase.rs`, `python/src/holder/handles.rs`, `python/src/holder/fs.rs`, `node/src/iobase.rs` | cited |
| `AGENTS.md` at HEAD: rows `holder/` 376, `warehouse/` 377, `auth/` 378, `aws/` 379, `local/, fs/, zip/, s3/` 381, `http/` 382, `s3tables/` 391, section "Object stores" 1342 | cited |

## 2. The precedent table, row by row

Columns: M = media point (`media/codec.rs`), L = locator (`holder/locator.rs`), C = catalog factory
(`warehouse/catalog.rs`), F = table format (`media/format.rs`), B = the proposed backend point.

| Facet | M media codec | L locator | C catalog factory | F table format | B backend (proposed) |
| --- | --- | --- | --- | --- | --- |
| Register line | `static CODECS: Register<MimeType, &'static dyn MediaCodec> = Register::new("record medium");` codec.rs:157 | `static LOCATORS: Register<Scheme, &'static dyn Locator> = Register::new("locator");` locator.rs:42 | `static FACTORIES: Register<FactoryKey, &'static dyn CatalogFactory> = Register::new("catalog factory");` catalog.rs:137-138 | `static FORMATS: Register<&'static str, &'static dyn TableFormat> = Register::new("table format");` format.rs:182 | `static BACKENDS: Register<Scheme, &'static dyn Backend> = Register::new("storage backend");` in new `holder/backend.rs` |
| Key | `MimeType`; one codec names several (`mime_types()` codec.rs:48) | one `Scheme` (`locator.scheme()` locator.rs:26, 80) | `FactoryKey::Type(&str)` or `FactoryKey::Scheme` (catalog.rs:117-123); one factory can hold both | the format `name()` (format.rs:28) | `Scheme`; one backend names **ten** (section 3.2) |
| Claim door | `media::codec::claim(codec, by)` codec.rs:192 | `holder::claim_locator` (re-export holder/mod.rs:17; def locator.rs:70) | `warehouse::claim_factory` catalog.rs:167 | `media::format::claim` format.rs:206 | `holder::claim_backend(backend, by)` |
| Atomicity | all-or-none: every type checked, then claimed, under `static CLAIMING: Mutex<()>` (codec.rs:160, 210-229, 248-250) | one key, no mutex | all-or-none under its own `CLAIMING` (catalog.rs:141, 178-213) | one key | **all-or-none over the scheme list**, under a `CLAIMING` mutex as M and C do |
| Core self-claim refused | `by == CORE` -> `Error::InvalidRecord` at `$.encoding` (codec.rs:194-198) | at `$.url` (locator.rs:72-79) | at `$.with.type` (catalog.rs:169-173) | format.rs:208-213 | `by == CORE` at `$.url`; plus a refusal of a scheme the core's own arms hold (`file`, `http`, `https`), as C refuses `memory`/`folder` (catalog.rs:134-135, 192-197) |
| Other claim rules | rank >= `EXTERNAL_RANK` (32) outside the core (codec.rs:29, 199-205); a codec naming no type (213-218); one `name()` per medium (233-247) | none | a factory naming neither word nor scheme (187-191); a core `type` word (192-197) | none | a backend naming no scheme; no rank (nothing orders two backends) |
| Second claim | `Register::claim` -> `Error::Conflict { expected: self.what, actual: held.by, path: key }` (plugin.rs:56-69) naming the first claimant | same | same | same | same, `path` the scheme |
| Seed | `SEEDED: OnceLock<()>`, `seed()` claims the core's seven by `CORE`, `expect("the core's own media claim cleanly")` (codec.rs:158, 163-181) | `seed()` claims `S3TABLES_LOCATOR` under `cfg(feature = "s3tables")` (locator.rs:47-60) | `seed()` claims `HADOOP_FACTORY` under `iceberg`, `S3TABLES_FACTORY` under `s3tables` (catalog.rs:145-156) | `seed()` claims `ICEBERG_FORMAT` (format.rs:186-) | `seed()` claims `&crate::s3::S3_BACKEND` under `cfg(feature = "s3")` until S6d, then `yggdryl_s3::install()` does |
| Intake read, once | `codec_of(base)` (no alloc on miss, codec.rs:264) / `codec_for` (275) at a handle's media type and `RecordOptions` construction | `locator::locate(location, props)` (locator.rs:97) as the **first** line of `Holder::from_url` (holder/mod.rs:371-373), **before** `location.locator()` lowers an ARN | `scheme_factory`/`typed_factory` in `Catalog::from_url` (catalog.rs:343-347) | `media::format::locate(handle)` in the record doors | `backend_for(url.scheme())` in `Holder::from_url`, **after** `location.locator()?` (holder/mod.rs:374), replacing the arm at holder/mod.rs:384-422 |
| What the value in hand carries | `RecordOptions::codec()`, `Media::medium()` (codec.rs:145, media/mod.rs:160) | nothing: it returns a `Holder::Catalog/Namespace/Table` | `Catalog::Registered(Box<dyn RegisteredCatalog>)` (catalog.rs:299) | the `Box<dyn LocatedTable>` a write session holds | `Holder::Registered(Box<dyn RegisteredHolder>)`; its `reopen`, `exists`, `as_any` are the object's own, not a register lookup |
| Object trait supertraits | `MediaWrapper: IOBase + Sync + Debug` (codec.rs:142) | - | `RegisteredCatalog: CatalogValue + Send + Sync + Debug` (catalog.rs:41) | `LocatedTable: Debug + Send` (format.rs:56) | `RegisteredHolder: IOBase + Send + Sync + Debug` (`Holder` derives `Debug` only, holder/mod.rs:87, so no `clone_box`/`dyn_eq`/`dyn_hash` is owed; `Site::Native` holds `Arc<Holder>`, handle.rs:29, which is why `Sync`) |
| Downcast to the concrete type | `IOMedia::as_any` (iomedia.rs, DESIGN.md:1317-1324) | - | `Catalog::downcast_ref::<T>()` (catalog.rs:398-405) | - | `RegisteredHolder::as_any/as_any_mut` and `Holder::downcast_ref::<T>()`, the `Table::downcast_ref` shape (table.rs:137-152) |
| Refusal when unclaimed | `unregistered(base)`: "a record encoding this build implements (<list>; install the crate that claims it and call its `install()`)" (codec.rs:289-310) | none (None, properties unread, locator.rs:92) | "holding a location of this scheme; install the crate that claims it and call its `install()`" for `s3tables` (catalog.rs:351-357) | named at `$.encoding` (DESIGN.md:1269-1272) | the identical sentence **already exists** at `holder/mod.rs:439-443`; the `#[cfg(not(feature = "s3"))]` arm (416-421, "holding an object store location without the s3 feature") is deleted |
| What the core keeps as routing vocabulary | `MimeType::PARQUET`, `.xmla` (DESIGN.md:1407-1412) | `Scheme::S3TABLES`, `is_s3_tables` (scheme.rs:273) | the Iceberg folder names (DESIGN.md:1301-1303) | the layout detection (DESIGN.md:1267-1272) | the ten `Scheme` consts and predicates, `uri/authority.rs` `StoreLocation`, `fs/location.rs` `published_endpoint` (section 4) |
| Who installs | D7: explicit idempotent `install()` per crate; bindings' module init and the CLI `main` call it (DESIGN.md:227-231) | same | same | same | `yggdryl_s3::install()` claims the ten schemes as `"yggdryl-s3"`; every harness calls it through one support function; a pure-Rust `Holder::from_url("s3://..")` is refused until it is called |
| Ordering of two claims | `rank()` (0..6 core, >= 32 outside) because option order is a persisted hash input (DESIGN.md:1153-1160) | by scheme | by key order | by name | none: schemes are disjoint, `BTreeMap` order is irrelevant |

Verdict: B copies M for the claim mechanics (multi-key, all-or-none, `CORE` refusal, seed, `install()`), copies L for the key and
the intake position in `from_url`, and copies C/Table for the "`Registered` variant + `downcast_ref`" value shape. It does **not**
copy M's rank, `MediumSettings`, or typed options (a backend has no persisted option order and no options struct on the register).


## 3. The backend-side facts, exactly (answer a)

### 3.1 What `rust/src/s3/` is

26 files, 11,963 lines (`git ls-tree -r HEAD --name-only | grep '^rust/src/s3/'`, `wc -l` per file at HEAD):

| Part | Files (lines) |
| --- | --- |
| The trio | `path.rs` (651), `folder.rs` (529), `file.rs` (1140) |
| The one client | `client.rs` (3372): pool, signing hook, resuming reader, request accounting, `Stats` |
| One dialect each | AWS: `aws/options.rs` (200), `aws/xml.rs` (269), `aws/mod.rs` (14). Google: `google/dialect.rs` (261), `google/json.rs` (149), `google/options.rs` (262), `google/token.rs` (691), `google/mod.rs` (14). Azure: `azure/auth.rs` (453), `azure/dialect.rs` (184), `azure/options.rs` (454), `azure/sign.rs` (277), `azure/xml.rs` (93), `azure/mod.rs` (16) |
| Configuration | `options.rs` (563, `S3Options`), `properties.rs` (806), `encryption.rs` (547) |
| Dispatch and shared shapes | `provider.rs` (222, `Provider`, "the sole dispatcher"), `request.rs` (197), `answer.rs` (83), `xml.rs` (93), `mod.rs` (423) |

The other backends for scale: `local/` 4 files / 1,972 lines, `fs/` 10 / 3,424, `zip/` 8 / 4,874, `http/` 36 / 19,991.
Support the leaf would take or share: tests `rust/tests/s3.rs` + `rust/tests/s3/` 20 files / 8,742 lines; interop
`rust/tests/interop/s3/{aws,azure,gcs,mod}.rs` and benchmarks `rust/benchmarks/holder/s3/{bytes,listing,mod,records}.rs`
8 files / 1,423 lines; the fake store `rust/tests/support/server.rs` 3,869 lines, `#[path]`-included by `holder.rs:6`,
`iceberg.rs:15`, `iobase_calls.rs:19`, `medallion_ledger.rs:34`, `s3.rs:24`, `s3tables.rs:33`, `warehouse.rs:12`
(`git grep -n 'support/server.rs' HEAD -- 'rust/tests/*.rs'`); `object_store = "=0.13.1"` is the benchmark baseline
dev-dependency (`rust/Cargo.toml:201`).

### 3.2 The scheme words (exact, ten)

`Provider::from_scheme` (`s3/provider.rs:35-46`) maps through the core predicates:
"`s3`, `s3a`, `s3n` name the first; `gs` and `gcs` the second; `az`, `abfs`, `abfss`, `wasb`, `wasbs` the third".

| Store | Schemes | Core vocabulary |
| --- | --- | --- |
| Amazon S3 (`Provider::Aws`, canonical `Scheme::S3`, provider.rs:51) | `s3`, `s3a`, `s3n` | `Scheme::{S3, S3A, S3N}` scheme.rs:102-106; `is_s3` scheme.rs:263 |
| Google Cloud Storage (`Provider::Google`, canonical `Scheme::GS`, :52) | `gs`, `gcs` | `Scheme::{GS, GCS}` :110-112; `is_gs` :282 |
| Azure Blob Storage (`Provider::Azure`, canonical `Scheme::AZ`, :53) | `az`, `abfs`, `abfss`, `wasb`, `wasbs` | `Scheme::{AZ, ABFS, ABFSS, WASB, WASBS}` :114-122; `is_az` :292-301 |

`Scheme::is_object_store` = `is_s3 || is_gs || is_az` (scheme.rs:308-310), documented as "This is what selects that
backend" (scheme.rs:303-307). `s3tables` is **not** among them (`is_s3_tables`, scheme.rs:273: "deliberately not part
of `is_object_store`": a table bucket holds tables, "no byte-level backend opens one"). A handle reports the spelling
it was handed (`s3/mod.rs:17-19`; `Client.scheme`, client.rs:295-298). `Provider`'s own `FromStr` accepts more words
(`amazon`, `minio`, `gcp`, `adls`, `blob`, provider.rs:203-221) but those are configuration spellings, never URL schemes.
`Scheme`'s `FromStr` lists the same ten plus `s3tables` (scheme.rs:405-416).

### 3.3 How it reads its properties (exact)

Intake in `Holder::from_url` (`holder/mod.rs:384-415`), in this order, all of it backend-specific:

1. `url.parameters(true)?`: each query parameter must satisfy `S3Options::is_property(name)` or the call fails with
   `Error::Parse { target: "object store location", position: 0, reason: "the query parameter {name:?} names no property an object store reads" }` (:390-403).
2. `S3Options::from_properties(stated.iter().chain(properties.iter()))`: the query first, the caller's properties
   after, "so a property stated twice is the caller's" (:404-409, doc :327-333).
3. The query is taken off the handle's location: `location.set_query(None)?` (:412-413); the handle reports the
   resource, not how it is reached.
4. `crate::s3::located_with(&location.to_string(), options)` (:414). `Holder::described` (:452-470) then reads
   `media_type`/`mime_type`/`content_type` and `codec`/`content_encoding` for **every** backend; that part stays core.

The reader, `S3Options::with_properties` (`s3/properties.rs:97-114`): value trimmed, an empty value states nothing
(:107-110), the name folded by `canonical` (:749-763), `read` answers `(options, known)` (:374-594). Three rules
(:10-17): an unknown name is ignored, a known name it cannot honor is refused (`signer`, `signer_uri`,
`signer_endpoint` -> `unsupported(name)`, :585), a name two stores share is applied to both (`storage_class`,
`access_token`/`bearer_token`, :565-581). Half a credential set is refused naming the half (`Parts::credentials`,
:681-710).

The vocabularies, from the doc table (:35-53) and the fold:

| Vocabulary | Examples |
| --- | --- |
| bare names (this crate's) | `endpoint`, `region`, `access_key_id`, `secret_access_key`, `anonymous`, `path_style`, `timeout`, `profile`, `role_arn`, `sso_*`, `sse_type`, `project`, `credentials_file`, `account_name`, `sas_token`, `tenant_id` |
| AWS tools' `AWS_*` | `AWS_ENDPOINT_URL`, `AWS_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_PROFILE` |
| PyIceberg's | `s3.endpoint`, `s3.region`, `s3.access-key-id`, `s3.force-virtual-addressing`, `s3.request-timeout`, `gcs.service.host`, `gcs.project-id`, `gcs.oauth2.token`, `adls.endpoint`, `adls.account-name`, `adls.sas-token`, `client.*` |
| PyArrow's | `endpoint_override`, `scheme`, `access_key`, `secret_key`, `force_virtual_addressing`, `request_timeout`, `credentials_file`, `project_id` |

The fold: `trim`, ASCII lower case, `-` and `.` -> `_`, then peel `PREFIXES` repeatedly
(`["s3_", "gcs_", "gs_", "google_", "gcp_", "azure_", "adls_", "abfs_", "client_", "aws_"]`, :744-746), never from
`client_id`/`client_secret` (:754-756). `S3Options::PROPERTY_NAMES: [&str; 47]` (:120-168) is the list a binding
suggests a mistyped keyword against (`python/src/holder/handles.rs:421`). `is_property` runs the reader on a probe
value, "so the answer can never disagree with what a read does" (:170-182). AWS identity names are delegated to the
core's `aws::properties::Identity` (`parts.identity.read(...)`, :589), the one reader every AWS consumer shares
(`aws/properties.rs:1-11, 187`). The environment is a **separate** sweep, `environment_properties`/`from_environment`
(:221-274), under the prefixes `AWS_ GOOGLE_ AZURE_ YGGDRYL_` (`s3/options.rs:46`), applied in `Client::new` when
`read_environment` (default true, options.rs:123; client.rs:349-355).

### 3.4 How it builds a `Holder` for a location (the trio)

`s3::located_with(url: &str, options)` (`s3/mod.rs:157-161`):

```rust
pub fn located_with(url: &str, options: S3Options) -> Result<Holder> {
    let url = parse(url)?;
    let client = Arc::new(Client::new(&url, options)?);
    S3Path::new(client, url).map(Holder::S3Path)
}
```

- `parse` (mod.rs:333-340) refuses a scheme `Provider::from_scheme` rejects or a URL naming no bucket (`no_container`,
  :348-358, which also strips a password from the message, :400-422).
- `Client::new` (client.rs:330-393) settles provider, environment, session, TLS, region, endpoint and addressing and
  walks no credential chain ("touching nothing", :317). One `Arc<Client>` is cloned into every folder, path and file
  derived from the handle.
- It returns the **undecided** `Path` role, as every backend's door does: `local` -> `Holder::LocalPath`
  (holder/mod.rs:382), `fs::located` -> `Holder::FsPath` (fs/mod.rs:70-72), `zip::from_url` -> `Holder::ZipNode`
  (zip/mod.rs:133-135), `http::located_with` (holder/mod.rs:429). The convention a `Backend::holder` doc must state.
- Zero requests: AGENTS "Object stores" table, "construction, child resolution, trailing-slash location | 0 | 0 | 0"
  (AGENTS.md@HEAD:1389); pinned at `rust/tests/holder/mod_.rs:707` (`store.request_count() == 0`, "holding sends
  nothing") and `:760` (`holding again sends nothing`).
- The raw-name doors (`file_at`, `folder_at`, `path_at` and `_with`, mod.rs:208-311) build a handle from a container and a
  key no URL can spell; the bindings call them directly (`python/src/holder/fs.rs:989-992`,
  `python/src/holder/handles.rs:456-516`), so they are crate API, not register API.

How the three backends that stay build and re-hold theirs, for contrast (`holder/mod.rs`, HEAD):

| Backend | Door in `from_url` | Role returned | Re-hold in `from_handle` |
| --- | --- | --- | --- |
| `local/` (4 files, 1,972 lines) | `url.is_local()` (:375), no fragment (:382) | `Holder::LocalPath(LocalPath::from_url(url.clone())?)` | folder cloned, path rebuilt from its URL, file rebuilt from its path (:509-513) |
| `zip/` (8 files, 4,874 lines) | local URL with a fragment (:376-380) -> `zip::from_url` (zip/mod.rs:121-153) | `Holder::ZipNode(..)` (zip/mod.rs:133-135; `zip::mount` :79-81) | `sibling(plain)`, refused at the archive root (:541-544) |
| `fs/` (10 files, 3,424 lines) | none: reached through `fs::located(BoundLocation)` (fs/mod.rs:70-72) by bindings and `Site::Bound` | `Holder::FsPath` | rebuilt from `bound_location()` (:514-517) |
| `http/` (36 files, 19,991 lines) | `url.scheme().is_http()` (:423) -> `HttpOptions::from_properties` + `http::located_with` (:426-429) | the request/session role | session and request cloned, an answer or a stream refused (:533-540) |
| `s3/` (26 files, 11,963 lines) | `url.scheme().is_object_store()` (:384) | `Holder::S3Path` | the three arms above (:518-532) |

### 3.5 Roles, probing, and the `IOKind` of a trailing-slash location

| Role | Struct | Clone | Constructor | Kind |
| --- | --- | --- | --- | --- |
| `S3Path` (path.rs:42-60) | client, url, bucket, key, `declared`, `inferred`, `Mutex<Option<Resolved>>`, `Mutex<Option<Probed>>` | no | `pub(super) new` :106 | resolved by a probe, below |
| `S3Folder` (folder.rs:35-43) | client, url, bucket, prefix | `#[derive(Clone)]` | `pub(super) new` :47 | `folder_kind()` = `IOKind::Directory` (folder.rs:484-486) |
| `S3File` (file.rs:54-66) | client, url, bucket, key, `declared`, `inferred`, `Mutex<State>` (meta, stage, opened) | no | `pub(super) new` :89 | staged write -> `File`, else `file_kind()` (a `HEAD`) (file.rs:943-951) |

`S3Path` resolves lazily: `unresolved()` (path.rs:252-263) answers `IOKind::Directory` **with no request** when
`self.url.is_glob() || self.url.has_trailing_slash() || self.key.is_empty()` (:254-256); anything else is one
single-key listing (`probe`, :199-228), a second one only because `.` sorts below `/` (:187-192, :218-227). The listing's
size is kept so the resolved `S3File` answers `size()` with no `HEAD` (:168-178, `with_known_size`, file.rs:125-133).
`media_type()` is name-based and free: `inode/directory` for a trailing slash, key empty or glob (path.rs:472-489), and
`S3Path::kind()` is `heard(current_kind(), Unknown)` (path.rs:503-505). So the kind a trailing-slash location answers is
`IOKind::Directory`, at zero requests, from the leaf alone. Nothing in the core re-derives it; the core computes only the
URL-level `container` flag in `Holder::described` (`!url.scheme().is_http() && (url.is_glob() || url.has_trailing_slash())`,
holder/mod.rs:454), which stays core because it is URL vocabulary. The register's contract only has to state "a trailing
slash, a glob or an empty key is a container, decided without a request", which the leaf's own tests pin.

Drop is semantic: `impl Drop for S3File` publishes a staged write (file.rs:1034-1041), and `S3Path::remove` calls
`file.discard()` first "or the removal would race its own resurrection" (path.rs:598-621). A `Box<dyn RegisteredHolder>`
keeps both (dyn drop runs the concrete `Drop`).

### 3.6 What `Holder::from_handle` must do to reopen one on the same client

Today (`holder/mod.rs:498-558`), over `holder.plain()` (:560-569), the three arms are:

```rust
Self::S3Folder(folder) => Self::S3Folder(folder.clone()),
Self::S3Path(path) => match sibling(plain)? {
    Some(sibling) => sibling,
    None => Self::S3Folder(path.as_directory()?),
},
Self::S3File(_) => match sibling(plain)? {
    Some(Self::S3Path(path)) => Self::S3File(path.as_file()?),
    Some(held) => held,
    None => return Err(unsupported("holding again an object with no container")),
},
```

`sibling` (holder/mod.rs:47-58) is `holder.parent()` then `child_by_path(file_name)`, `None` for a trailing slash or no
file name. It works because `S3Path::parent` builds `S3Folder::new(self.client.clone(), parent)` (path.rs:556-561) and
`S3Folder::child_by_path` builds `S3Path::new(self.client.clone(), url)` (folder.rs:472-478): **the `Arc<Client>` is
the identity**, so the endpoint, the key pair, the session, the signer cache, the region found by a redirect and the
`StatsSnapshot` counters all travel. Contract pinned at `rust/tests/holder/mod_.rs:734-770`
(`a_store_role_is_held_again_on_its_own_client_sending_nothing`): zero requests, role kept (folder -> folder, path ->
path, file -> file), same URL, signed with the first handle's key pair. The media type the plain handle declares is
carried across by the core (holder/mod.rs:554-556), role-independent, and stays.

Consequence for the design: `reopen` needs the concrete role and the private `Arc<Client>`; it belongs on the
object, `RegisteredHolder::reopen(&self) -> Result<Holder>` (Q3).

### 3.7 `is_backend_property`, and who else asks "is this name the store's"

`Holder::is_backend_property(url, name)` (`holder/mod.rs:592-604`, `cfg(feature = "iceberg")`, `pub(crate)`):
`url.scheme().is_object_store()` -> `S3Options::is_property(name)` under `s3`; `is_http()` -> `HttpOptions::is_property`;
else `false`. One caller: `iceberg/table.rs:547` (`stating`: "less the ones the store its root opens on reads for
itself"). The S3 reader is asked directly at three more places: `holder/mod.rs:392` (query parameters),
`s3tables/catalog.rs:642,645` (the catalog splits its properties into the store's knobs, which ride each table's
`Site::Store`, and the ones it states), `python/src/holder/handles.rs:420` (keyword checking). All four become
`backend_for(scheme)?.is_property(name)` (or, for the binding, a direct call into the linked leaf).

### 3.8 State the backend shares across clients

- `aws::Session` (core, `aws/session.rs`): `Arc<Inner>`, credential lease, signer cache, refused-key list, and
  `bucket_regions: Arc<Mutex<VecDeque<BucketRegion>>>` bounded at `BUCKET_REGIONS = 64` (:78, :208-209, :360-362,
  :1676-1687), `cfg(feature = "s3")`. "A session shared by many handles resolves and refreshes once for all of them"
  (s3/options.rs:60-65).
- The HTTP pool: `crate::http::client::agent_for(&transport, tls)` returns a `ureq::Agent`, "shared process-wide when the
  transport knobs are the defaults" (client.rs:419-432). `Session::tls_config()` returns `ureq::tls::TlsConfig`
  (aws/session.rs:1332, `cfg(s3)`). `ureq` types appear in `s3/` at 21 places in three files
  (`azure/auth.rs` 6, `client.rs` 9, `google/token.rs` 6).
- Per client: `Stats`, `RetryBudget`, `TokenCache` (Google), `Authorization` (Azure) (client.rs:290-314).
- No `static` register or cache in `s3/` beyond two `LazyLock<MediaType>` (path.rs:475-486): splitting the crate
  doubles no process global (D11 option d: one native module either way, DESIGN.md:314-345). The `warned!` repeat table
  (2 sites: file.rs:200, path.rs:278) and the `log` targets (8 sites) are the core logging's; a `yggdryl_s3::` target is
  "foreign" to `logging/facade.rs:72-76` until the per-crate table D10 plans maps it to `yggdryl.s3`.

### 3.9 The crate-private reach (grep, orientation, ~20 items)

`git grep -ho 'crate::...' -- rust/src/s3` plus the `use crate::` lines. **Already public** (no door needed):
`Holder`, `IOBase`, `IOFile/IOFolder/IOPath`, `IOKind`, `Listing::{new, empty, failing}` (listing.rs:46-68),
`ByteStream::{from_reader, from_fs_reader, from_container}` (bytestream.rs:40-92), `MediaType`, `MimeType`, `Uri`, `Url`,
`Scheme`, `Error`, `Result`, `Scalar`, `aws::{Credentials, Session}`, `json::{into_utf8, from_bytes, from_utf8}`,
`xxhash::xxh3`, `http::HttpOptions::DEFAULT_MAX_PAUSE`, `http::render_http_date`, `local::LocalFolder::config`,
`impl_default_iomedia!` (`#[macro_export]`, iomedia.rs:1117-1118), `overwrite_serie_default` (lib.rs:223).

**Crate-private** (the D6 `implementer` list for S6d; counts are grep hits in `s3/`):

| Item | Defined | Use in `s3/` |
| --- | --- | --- |
| `iobase::oversized` | `iobase/lifecycle.rs:82` `pub(crate)` | 17 (`file.rs` 10, `client.rs` 7) |
| `ByteStream::from_handle` | `bytestream.rs:126` `pub(super)` | `file.rs:555` |
| `uri::percent_encode_segment`, `percent_decode` | `uri/parser.rs:45, 655` | `mod.rs:328, 375, 386`; `client.rs:834`; `google/token.rs:575` |
| `integer::{byte_count_from_text, BYTE_COUNT_SPELLINGS, integer_from_scalar_as}` | `integer.rs:818, 743, 779` | `properties.rs:26`; `azure/auth.rs:401`; `google/json.rs:12`; `google/token.rs:522` |
| `boolean::bool_from_text` | `boolean.rs:142` | `azure/options.rs:12,153`; `client.rs:34` |
| `holder::system_time_ns` | `holder/mod.rs:33` | `azure/sign.rs:215` |
| `xxhash::stream::read_range_digest` | `xxhash/stream.rs:33` | `file.rs:721` |
| `http::retry::{RETRY_COST, RETRY_REFUND, RetryBudget, fresh_jitter, is_resumable, is_retryable_transport, is_unsent, retry_after, backoff, RETRY_BACKOFF}` | private `http/retry.rs` | `client.rs:35-38, 3208-3214` |
| `http::client::agent_for`, `http::record_process` | `http/client.rs:1128, 91` | `client.rs:431, 1158, 1274` |
| `auth::{variable, Bearer, Lease, Expiring, instant}` | private `mod auth` (lib.rs:28-29) | `client.rs:31`; `azure/auth.rs:13`; `google/token.rs:19, 678` |
| `aws::sigv4::{Signer, encode_key, encode_query_component, signed_access_key}` | `pub(crate) mod sigv4` (aws/mod.rs:71) | `client.rs:32, 245-251, 1020`; `azure/auth.rs:416-417`; `azure/dialect.rs:122-129` |
| `aws::properties::{EndpointName, Identity, count, flag, refusal, seconds}`, `aws::environment::is_native` | `pub(crate) mod` (aws/mod.rs:63, 68) | `properties.rs:25, 244` |
| `Session::{tls_config, bucket_region, learn_bucket_region, stated_region, states_identity, under, given_variables, signer, answers_another}` | `aws/session.rs` `pub(crate)`, several `cfg(s3)` | `client.rs`, `options.rs`, `properties.rs:348` |
| `xml::scanner::{Element, XmlError, parse_document, parse_root}` | `pub(crate) mod scanner` under `aws` (xml/mod.rs:57-58) | `xml.rs:10`; `aws/xml.rs`; `azure/xml.rs` |
| `warned!` | `macro_rules!` + `pub(crate) use warned` (logging/warning.rs:109, 121) | `file.rs:200`, `path.rs:278` |

DESIGN.md:665-671 estimated Excel at "about 25"; S6d gets its exact list the same way.

## 4. What the core still needs from a backend after it leaves (answer b)

### 4.1 Stays core: the `Scheme` vocabulary (the `MimeType::PARQUET` analogue)

`scheme.rs` at HEAD names eleven S3-family spellings: wire variants `S3, S3a, S3n, S3Tables, Gs, Gcs, Az, Abfs, Abfss,
Wasb, Wasbs` (:34-44), consts (:101-122), `as_str` (:176-186, :217-227), `FromStr` (:405-416), and the predicates
`is_s3` (:263), `is_s3_tables` (:273), `is_gs` (:282), `is_az` (:292-301), `is_object_store` (:308-310),
`has_container` (:334-336, `is_object_store() || is_s3_tables()`), `is_storage` (:342-358, read by
`python/src/uri.rs:501`). They stay: the URL grammar reads them (next section), `s3tables` is the locator's, and D33
already ruled the same way for `MimeType::XMLA` (DESIGN.md:1407-1412). Outside `scheme.rs` the constants are named in
`s3/provider.rs:51-53`, `uri/arn.rs:601, 606` and `s3tables/{catalog.rs:937, mod.rs:171}` only
(`git grep -n 'Scheme::S3\|Scheme::GS\|Scheme::AZ...'`). One doc line changes: "This is what selects that backend"
(scheme.rs:303-307) becomes a statement about naming a store, since the register selects.

### 4.2 Stays core: the S3 authority rule in `uri/`

`Uri::store_location` (`uri/authority.rs:311-407`) walks authority and path once for all three stores, with
`StoreHost::parse` per store (:439-455: `parse_aws_s3_hostname` :579, `parse_google_hostname` :475,
`parse_azure_hostname` :522, `is_store_hostname` :554); it feeds the public `Uri::{hostname, store_endpoint, bucket,
account, region, is_virtual_hosted, key}` (`uri/mod.rs:545-616`) and `Url`'s delegates (`uri/url.rs:289-310`). The leaf
reads it through `url.bucket()`/`url.key()` (`s3/mod.rs:360-368`, `split_location`). It is URL vocabulary, tested in
`rust/tests/uri/` with no store; AGENTS states the rule in "Datatypes, parsers, errors" ("S3 authority: the first path
part is the hostname when it ends `.com`/`.io`, carries a port, is an IP literal, or is `localhost`; else it is the bucket").
Documentation drift to fix when the AGENTS row is rewritten: the code treats a first component ending `.com`, `.io` **or
`.net`** as a hostname (`is_store_hostname`, `uri/authority.rs:548-562`, `[".com", ".io", ".net"]` at :558), where AGENTS
says `.com`/`.io`.
`Arn::locator` lowers `arn:aws:s3:::b/k` to `s3://b/k` (`uri/arn.rs:601`) and a table to `s3tables://` (:606): this is why
`Holder::from_url` asks the register **after** `location.locator()?` (holder/mod.rs:374) and the locator **before** it
(:371).

### 4.3 Stays core: bridged-filesystem spellings

`fs/location.rs:415-459`: a `BoundLocation` over a foreign filesystem renders a diagnostic URL with
`scheme.has_container()` (:430) and `published_endpoint` (`is_gs`, `is_az`, `is_s3_tables`, :449-459). A PyArrow
`S3FileSystem` is a `Holder::FsPath` (fs/mod.rs:70-72), not a registered backend.

### 4.4 What `holder/mod.rs` loses and gains

Loses: the three variants (`:106-114`) and every arm that names them: `exists` (:201-206), `into_byte_stream` (:240-241),
the `from_url` object-store arm and its `cfg(not(feature = "s3"))` twin (:384-422), `from_handle` (:518-532),
`is_backend_property` (:595-598), `as_io`/`as_io_mut`/`as_media`/`as_media_mut` (:914-919, :952-957, :990-995,
:1028-1033). 58 lines of the file name `S3`/`s3`; 26 are `cfg(feature = "s3")`. Gains one `Registered` arm in each of
those four `as_*` matches plus `exists`, `from_handle` (`reopen`) and `into_byte_stream`, and the
`backend_for(url.scheme())` arm between the HTTP arm and the refusal. `Holder::described` (:452-470), `sibling` (:47-58),
the media-type carry-over (:554-556) and the `Debug` derive (:87) are unchanged. Late resolution means a `Holder::Uri`
holding an `s3://` URL (holder/mod.rs:157-160) fails at its first verb with the install sentence when nothing claims the
scheme, not at construction.

### 4.5 `warehouse/`, `iceberg/`, `s3tables/`

- `warehouse/handle.rs`: `Site::Store { url, session: crate::aws::Session, region, store: Properties }` (:45-52,
  `cfg(feature = "s3")`, `cfg_attr(not(feature = "s3tables"), allow(dead_code))`), its `Debug` (:63-75), `url` (:112-113),
  `resolve` (:129-145: builds `S3Options::default().with_environment(session.reads_environment()).with_session(session.clone()).with_region(region.clone()).with_properties(store.iter().chain(properties.iter()))?` then `s3::located_with`), `PartialEq` (:156-158) and `Hash` (:173-175). `Site::of` (:94-105) takes a native reopen for `scheme.is_object_store() || scheme.is_http()`.
  Only `s3tables/catalog.rs:1148-1161` constructs it (`Bucket::store_session` :480-489 builds the shared session).
- `iceberg/staging.rs` five functions match `Holder::S3*`: `upload` (:264-276, `file.upload_from(&mut source, size)`),
  `unpublished` (:286-298, `file.discard()`), `leaf` (:306-312, `path.as_file()`), `container` (:319-325,
  `path.as_directory()`), `sized` (:338-344, `file.with_known_size(size)`); `iceberg/catalog/mod.rs:505-540`
  (`folder_role`: `S3Folder` kept, `S3Path`/`S3File` -> `as_directory()`); `iceberg/table.rs:543-552` (`stating`, via
  `is_backend_property`). These are capabilities of an object-store handle, not of "S3" (Q7).
- `s3tables/catalog.rs:636-646` splits the catalog's properties by `S3Options::is_property`; the `Locator`
  (`S3TABLES_LOCATOR`) and the `CatalogFactory` stay in their register and move with `iceberg` (D15).

### 4.6 Nothing else opens `s3://` by URL (the grep you asked for)

`git grep -n 's3://' HEAD -- rust/src ':!rust/src/s3/'` finds 16 lines in 9 files, **all comments or doc tests that only
parse a string**: `expression/plan.rs:34`, `holder/mod.rs:329`, `http/options.rs:38`, `iceberg/options.rs:48` (a
refusal test of `WriteStaging`), `s3tables/catalog.rs:12`, `uri/arn.rs:39,391`, `uri/mod.rs:352-353,600-604,900`,
`uri/url.rs:90`, `uri/urn.rs:181-185`. The only `Holder::from_url` callers in `rust/src` are generic:
`expression/plan.rs:2138,2142` (a plan's target), `isin_registry/store.rs:180,260` (the ISIN registry store,
by caller URL), `uri/handle.rs:59` (`Holder::from(Uri)`), `warehouse/handle.rs:120` (`Site::Url`). So the ISIN registry
and the plan need no S3 knowledge; they fail with the install sentence if the leaf is not installed.

### 4.7 Build and CI surface

- `rust/Cargo.toml`: `s3 = ["aws", "dep:md-5"]` (:50), `s3tables = ["s3", "iceberg"]` (:57), `md-5` (:93, used only by
  `s3/client.rs:3175-3180` for `DeleteObjects`' `Content-MD5`), dev-dependency `object_store` (:201, the benchmark
  baseline). `aws = ["http", "dep:hmac", "dep:ring", "dep:sha2"]` (:49) stays if `aws/` stays (Q5). `hmac`/`sha2` are used
  by `s3/azure/sign.rs:12-13` and `s3/aws/options.rs:73-74`, `ureq` by `s3/{client,azure/auth,google/token}.rs`, `base64`
  by six `s3/` files: the leaf declares them too.
- `lib.rs:131-132` `pub mod s3;` and the 12 `internals` re-exports `lib.rs:528-551` (`s3_answer ... s3_xml`) leave with the folder
  (`scripts/generate_internals.py` regenerates).
- 23 `cfg(feature = "s3")` gates sit inside the core's `aws/`, `auth/`, `xml/`, `http/` for the leaf's sake:
  `aws/session.rs` 11 (the region cache, `tls_config`, `under`, `stated_region`, `states_identity`), `auth/lease.rs` 4 and
  `auth/mod.rs:29` (`Bearer`), `aws/sigv4.rs:81,182,452` (`encode_key`), `aws/mod.rs:62` (`environment`), `xml/scanner.rs:67,78`
  (`children`, `required`), `http/mod.rs:82` (`record_process`). They become `aws`-gated or move (Q5).
- `.github/ci/rows.toml:90-96` `[leaves]`: one line, `s3 = { package = "yggdryl-s3", jobs = ["object-interop",
  "azure-interop", "gcs-interop"] }` (the three exchanges are rows `x-s3`, azure, gcs at rows.toml:269-282, each
  `extends = ["interop"]`); `iceberg` gains `after = [..., "s3"]` if its tests dev-depend on the leaf.
  The Rust half of the exchanges, `rust/tests/interop/s3/*.rs`, moves as `rust/tests/interop/excel.rs` moves with Excel (DESIGN.md slice table, S6b).
- `AGENTS.md` rows to rewrite at HEAD: `holder/` (:376), `auth/` (:378), `aws/` (:379), `local/, fs/, zip/, s3/` (:381),
  `http/` (:382), `s3tables/` (:391), "Object stores" (:1342-); and a new "Common changes" row, "registered backend".
  `.api-inventory.txt:2041` (`### yggdryl::s3`) moves to its own section.

### 4.8 Bindings (82 lines in 5 files)

`python/src/holder/handles.rs` 37 (`PyS3File/PyS3Folder/PyS3Path`, :76-92, the constructors :429-516, the keyword reader
:406-425), `python/src/iobase.rs` 33 (`cloned` :101-145, `store_sibling` :151-157, `folder_holder_for` :188-191, `store_path`
:247-249, the `Role` enum :284-317, :354-356, :403-405), `python/src/holder/fs.rs` 9 (`NativeRole` -> `s3::{path,file,folder}_at_with`
:989-992, `store_options` :1063-1075), `python/src/http.rs` 1 (a doc line), `node/src/iobase.rs` 2 (:130-131
`folder_holder_for`). `python/src/iobase.rs:217-232` (`container_holder`) and the two `folder_holder_for` functions (python :178-200, node
`node/src/iobase.rs:127-143`) are role casts too (`as_directory()`, or `yggdryl::s3::folder(..)` for "hold `url` as a
container", since `from_url` answers the undecided `S3Path`): they are served by the same `into_container` capability
(section 6). Node's `rebuilt()` (`node/src/iobase.rs:604-614`) rebuilds a handle from its URL under no properties, so an
S3 handle loses its options there, unlike Python's `cloned`; it needs no change for this work. Node gains no door (AGENTS
section 4): its two lines are re-spelled. Python's `cloned` re-implements
`Holder::from_handle` for the S3 arms; with `reopen` it can call `Holder::from_handle` (a dedupe, not a requirement). The
bindings link `yggdryl-s3` directly for the raw-name doors (D11 option d), so those stay crate API.


## 5. The differences from a medium (answer c)

| # | A medium | A storage backend | Consequence for the point |
| --- | --- | --- | --- |
| 1 | Reached by **MIME type**, read once from a handle's media type or a `RecordOptions` (`codec_of`, codec.rs:264) | Reached by **URL scheme**, read from the lowered `Url` in `Holder::from_url` (holder/mod.rs:374-384). One backend names **ten** schemes of three stores | `Register<Scheme, ..>` with an all-or-none multi-key claim (the codec's mechanism, codec.rs:209-252); a scheme list is the whole of the key |
| 2 | Receives a `Holder` and wraps it: `open(handle: Holder) -> Media` (codec.rs:136); never builds a `Holder` | **Produces** `Holder`s, recursively: `IOBase::parent() -> Option<Holder>` (iobase.rs:491), `child_by_path -> Result<Holder>` (:507), `ls -> Listing` of `Result<Holder>` (:537; `Listing::new(impl Iterator<Item = Result<Holder>>)`, listing.rs:56). `S3Folder::hold` builds `Holder::S3File(file.with_known_size(*size))` per listed entry (folder.rs:315-321) | The core enum must be able to hold what the leaf builds from inside the leaf's own trait impls: `Holder::Registered`, and a public constructor (`From<S3File> for Holder` in the leaf, the "`From` impls stay in the implementations' files" rule, DESIGN.md:1356-1361) |
| 3 | Construction may read: `stated_field` probes (codec.rs:101-105) | **Zero requests** to build, to resolve a child, and for a trailing-slash location (AGENTS.md@HEAD:1389; `s3/mod.rs:50-56`). A locator, by contrast, may send one (`s3tables://b/ns/t` is one `GetTableMetadataLocation`, holder/mod.rs:305-309) | The trait doc states it and a register test pins it (`store.request_count() == 0`, tests/holder/mod_.rs:707, :760) |
| 4 | No role split | Three roles - `IOPath`/`IOFolder`/`IOFile` - and an **undecided** `Path` that resolves by a probe (path.rs:199-228) except `lake/`, a glob or an empty key, which are `Directory` at zero cost (path.rs:254-256, :345, :477) | `Backend::holder` returns the undecided Path role (the "located" convention, 3.4); the roles stay private to the leaf and the core sees one object |
| 5 | Stateless `static` codec; the wrapper owns a cache | The **client is the identity**: `Arc<Client>` (retry budget, signer, session, token caches, stats) cloned into every derived handle; `S3Folder: Clone`, `S3Path`/`S3File` not. `Holder::from_handle` re-derives a handle on the same client (3.6) | `reopen(&self)` on the object, not on the register (Q3) |
| 6 | Value semantics (`Clone`/`Eq`/`Hash` for options) | `Holder` is `Debug` only (holder/mod.rs:87), not `Clone`; identity is **site** equality in `Handle` (warehouse/handle.rs:150-177) | No `clone_box`/`dyn_eq`/`dyn_hash` on the object trait |
| 7 | Drop is inert | `impl Drop for S3File` **publishes** a staged write (file.rs:1034-1041); `discard()` exists to cancel it (file.rs:401-406; iceberg/staging.rs:289-291) | A `Box<dyn ..>` keeps the `Drop`; `discard` is a trait capability |
| 8 | Configuration is a typed struct behind the options enum, read back by `settings::<T>()` (DESIGN.md:1127-1180) | Configuration is a **string property bag in three vocabularies plus an environment sweep**, read by the backend and asked "is this name yours" by the core (3.3, 3.7) | `is_property(name)` on the trait; the bag reaches `holder(url, properties)` as `&[(String, String)]`, the form `from_url` already holds (holder/mod.rs:364-366), since the query merge and refusal need the backend's own reader |
| 9 | Order is a persisted hash input (`rank`, DESIGN.md:1153-1160) | No order | no rank, no `EXTERNAL_RANK` |
| 10 | The core never special-cases a registered medium after S2 (`Media::Registered`, `RecordOptions::codec()`) | The core special-cases the S3 variants in **ten** places (4.4, 4.5) that exist because object stores have capabilities `IOBase` does not (a streaming upload of a file, a known size from a listing, an undecided-to-decided role cast, discard) | Defaulted capability methods on `RegisteredHolder` (Q7) |
| 11 | A `static` codec holds no process state | `aws::Session` is the cross-handle state (credential lease, signer cache, refused keys, a 64-bucket region cache, `aws/session.rs:78, 208-209`) and the HTTP pool is process-wide (`client.rs:419-422`) | the leaf calls the core's `agent_for` and `Session` (hence Q5, Q6); no new global |
| 12 | Native stream: `read_stream(..) -> Option<StreamSerie>` | `owned_stream_bytes` (iobase.rs:321, overridden at file.rs:564-577, path.rs:370-385) is lazy; `Holder::into_byte_stream` has an S3 arm that is **eager** (`file.byte_stream`, file.rs:412-434; holder/mod.rs:240-241) | Q8: the cost pin decides |

### 5.1 Is `Holder` already `Box<dyn IOBase>` inside? No. Does `Holder::Registered` need the three roles?

`Holder` is a `#[derive(Debug)] #[non_exhaustive] #[allow(clippy::large_enum_variant)] pub enum Holder` of 25 variants
(`holder/mod.rs:87-172`): `Buffer`; `LocalFolder/LocalPath/LocalFile`; `FsFolder/FsPath/FsFile`; `S3Folder/S3Path/S3File`;
`HttpSession/HttpRequest/HttpResponse/HttpStream`; `ZipNode/ZipPath/ZipLeaf`; the wrappers `Buffered/Coded/Text/Media`;
`Uri`; and the boxed objects `Catalog/Namespace/Table`. `as_io()` is a 25-arm `match` to `&dyn IOBase`
(holder/mod.rs:905-940), and `IOBase for Holder` forwards every method through it (:1129-1312). The doc at :60-70 refuses
`Box<dyn IOBase>` on purpose. The dyn-inside precedent is **`Holder::Table(Box<Table>)` with `Table::Registered(Box<dyn
RegisteredTable>)`** where `RegisteredTable: TableValue + IOBase + Send + Sync + Debug` (warehouse/table.rs:42): a trait
object that *is* an IOBase plus what the enum needs. The backend object is that, with fewer obligations.

The three roles are not object-needed. `IOPath`/`IOFolder`/`IOFile` (iopath.rs:10-79, iofolder.rs:15-272, iofile.rs:11-96) are
`: IOBase` traits of required accessors (`path_url`, `is_folder`, `has_folder`, `file_exists`, ...) and provided `*_kind`,
`*_ls`, `*_media_type`, `*_is_atomic` bodies that the backend's `IOBase` impl calls (`S3Folder::kind` -> `folder_kind()`,
folder.rs:484-486). The core calls them only in `Holder::exists` (holder/mod.rs:191-223): one trait method replaces it.

### 5.2 The alternative that was weighed and rejected: implement `fs::FileSystem` in the leaf

The crate already has a dyn plug point for storage: `fs::FileSystem` (`fs/system.rs:172-251`), whose trio
`FsPath/FsFolder/FsFile` (`fs/mod.rs:54-70`) lets any `Arc<dyn FileSystem>` surface as `Holder::FsPath` through
`fs::located(BoundLocation)`; Python bridges PyArrow's `S3FileSystem`/`GcsFileSystem`/`AzureFileSystem` through it
(`python/src/holder/fs.rs:4`), and `Holder::from_handle` re-holds it from `bound_location` (`holder/mod.rs:514-517`). A
leaf implementing it would need no `Holder::Registered`, only a scheme -> `Arc<dyn FileSystem>` factory. Rejected, because
the native S3 contract is stricter than Arrow's shape in ways the request-count pins read:

- the trait is "Arrow's seven-method shape for interop, while the core contract ... keep[s] generic `FileSystem`/`Fs*`
  names" and the S3 backend is "a location/container/leaf trio over the root traits" (AGENTS.md@HEAD:381);
- `create_bytes` is one conditioned `PUT` on S3 (exclusive, AGENTS "Storage: IOBase"); the `FileSystem::create_file`
  default is explicitly "the one place a create is not exclusive" (`fs/system.rs:231-248`), an override would be needed and
  the trio's `FsFile::create_bytes` (`fs/file.rs:318`) goes through it;
- `read_tail_bytes` is one suffix `GET` with the total from `Content-Range` (file.rs tail read, AGENTS table); the
  `IOBase` default every `FsFile` inherits is `size` then `read_range_bytes` (one more round trip);
- `S3Path` resolves role and size from one single-key listing and keeps the size (`with_known_size`, path.rs:168-178), a
  state `FsPath` has no slot for; `append_bytes` is a staged `GET` + `PUT` (file.rs:31-32) where `FsFile` opens an append
  stream (`fs/file.rs:328`); `owned_stream_bytes` is lazy (file.rs:564-577) and `StatsSnapshot` is the handle's own
  (path.rs:135-138).

Keep it as the door for *bridged* and simple backends: a store that only needs Arrow's shape implements `FileSystem` and
registers nothing. The register is for native trios whose `IOBase` contract is cost-pinned.

## 6. The proposed surface (a sketch, each method tied to the site it replaces)

```rust
// rust/src/holder/backend.rs (new), re-exported beside Locator: holder/mod.rs:17
pub trait Backend: fmt::Debug + Send + Sync + 'static {
    /// The label a refusal and a claim conflict name: "object stores".
    fn name(&self) -> &'static str;
    /// Every scheme the backend holds, claimed all or none: s3 s3a s3n gs gcs az abfs abfss wasb wasbs.
    fn schemes(&self) -> &'static [Scheme];
    /// Whether `name` is a property the backend reads for itself (S3Options::is_property, properties.rs:178).
    fn is_property(&self, name: &str) -> bool;
    /// The undecided-Path handle `location` names, touching no store. Reads the location's query
    /// (refusing a parameter it has no property for) beneath `properties`, strips it from the handle.
    fn holder(&self, location: &Url, properties: &[(String, String)]) -> Result<Holder>;
    /// (Q4) The same under a signing session, a region and the store's own knobs.
    #[cfg(feature = "aws")]
    fn holder_under(&self, location: &Url, session: &crate::aws::Session, region: &str,
                    properties: &[(String, String)]) -> Result<Holder> { /* Unsupported by default */ }
}

pub trait RegisteredHolder: IOBase + Send + Sync + fmt::Debug {
    fn implementation_name(&self) -> &'static str;           // "S3Folder" | "S3Path" | "S3File"
    fn as_any(&self) -> &dyn Any;  fn as_any_mut(&mut self) -> &mut dyn Any;
    fn exists(&self) -> bool;                                 // holder/mod.rs:191-223 (folder_exists | path_exists | file_exists)
    fn reopen(&self) -> Result<Holder>;                       // holder/mod.rs:518-532, sending nothing, role kept, client shared
    // Defaulted capabilities of an object store handle (Q7):
    fn into_container(self: Box<Self>) -> Result<Holder>;     // iceberg/staging.rs:319-325, iceberg/catalog/mod.rs:505-540
    fn into_leaf(self: Box<Self>) -> Result<Holder>;          // iceberg/staging.rs:306-312
    fn with_known_size(self: Box<Self>, size: u64) -> Holder; // iceberg/staging.rs:338-344
    fn upload_from(&mut self, source: &mut dyn Read, length: u64) -> Result<()>; // :264-276; default reads whole, write_all_bytes
    fn discard(&self) -> Result<bool>;                        // :286-298
    fn into_byte_stream(self: Box<Self>, position: u64, batch_size: usize) -> Result<ByteStream<'static>>; // holder/mod.rs:233-247
}

// Holder: + Registered(Box<dyn RegisteredHolder>);  - S3Folder, S3Path, S3File
pub fn claim_backend(backend: &'static dyn Backend, by: &'static str) -> Result<()>;
pub fn backend_for(scheme: &Scheme) -> Option<&'static dyn Backend>;
pub fn backends() -> Vec<&'static dyn Backend>;
```

`from_url` after the change:

```rust
let held = if url.is_local() { /* zip fragment or LocalPath, unchanged */ }
    else if url.scheme().is_http() { /* HttpOptions arm, unchanged */ }
    else if let Some(backend) = backend_for(url.scheme()) { backend.holder(url, &properties)? }
    else { return Err(Error::unsupported("holding a location of this scheme; install the crate that claims it \
                                           and call its `install()`", url.scheme().as_str())) };   // the existing :439-443 text
held.described(url, &properties)
```

The leaf's `install()` is `holder::claim_backend(&S3_BACKEND, "yggdryl-s3")`, idempotent behind a `OnceLock` (D7,
DESIGN.md:227-231); until S6d the core's `seed()` claims `&crate::s3::S3_BACKEND` as `CORE` under `cfg(feature = "s3")`
(the shape of codec.rs:163-181 and locator.rs:47-60).


## 7. Open design questions, each with a recommendation

**Q1. A new register, or widen `Locator`?**
Facts: `Locator` is asked *before* the identifier is lowered, by predicate (`names(&Uri)`), answers a catalog, a namespace
or a table that "declares no media type and takes no coding", may send requests, and has one scheme (locator.rs:1-11, 23-40,
80, 97-109); its result returns from `from_url` without `described()` (holder/mod.rs:371-373). A backend is asked *after*
lowering (holder/mod.rs:374), by scheme, never sends, takes `media_type`/`codec` through `described()` (:445), and names
ten schemes. `CatalogFactory` already shares the `s3tables` scheme key with `Locator` in a different register (catalog.rs:116-123,
locator.rs:42), so two registers keyed by `Scheme` is the established shape.
Recommend: a separate `Backend` register in `holder/backend.rs`, re-exported beside `Locator` (holder/mod.rs:17).
Rejected: adding `schemes()` and a bytes/object flag to `Locator` (two readings of one trait, and `from_url` would
consult it twice).

**Q2. What does `Holder` hold?**
Recommend one new variant, `Registered(Box<dyn RegisteredHolder>)`, and delete `S3Folder`, `S3Path`, `S3File` (no
back-compat, AGENTS "Always"). `RegisteredHolder: IOBase + Send + Sync + Debug`, no `Clone`/`Eq`/`Hash` machinery
(`Holder` derives `Debug` only). The leaf keeps its three concrete types and implements `From<S3File> for Holder`
etc. The three role traits stay private to the leaf (5.1). Rejected: three generic variants `RegisteredFolder/Path/File`
(zip has Node/Path/Leaf, fs Folder/Path/File: roles are the backend's vocabulary); making `Holder` a `Box<dyn IOBase>`
(holder/mod.rs:60-70). Cost note: one `Box` per `Holder::Registered`, so one allocation per listed child
(`S3Folder::hold`, folder.rs:315-321) on top of the `S3File`'s own `String`s; `Holder` itself shrinks (the
`large_enum_variant` allow at :89 stays for other variants). No `size_of::<Holder>` pin and no S3 row in
`allocations.rs` exist (`git grep -n 'size_of::<Holder>' HEAD -- rust` is empty), so only the `holder` benchmark's S3
listing groups (`rust/benchmarks/holder/s3/listing.rs`, direction only) read it.

**Q3. Where does `reopen` live?**
Recommend on the object (`RegisteredHolder::reopen(&self)`), as `RegisteredTable::clone_box` is the object's own. The role
and the private `Arc<Client>` are the object's state (3.6), and `Holder::from_handle` on a handle already in hand must
not need the backend claimed. `Holder::from_handle`/`from_handle_with` (holder/mod.rs:498-582) and `Site::of`
(warehouse/handle.rs:94-105) keep their text; only the S3 arms become one `Registered(h) => h.reopen()?`. The pin
`a_store_role_is_held_again_on_its_own_client_sending_nothing` (tests/holder/mod_.rs:736-770) survives, re-spelled
without the variant names. Python's `cloned` (python/src/iobase.rs:101-145) can then call `Holder::from_handle`.

**Q4. What happens to `Site::Store`?**
`Site::Store` (warehouse/handle.rs:45-52) is "the object store's own variant ... a catalog service's implementation
constructs it" and its `resolve` arm builds `S3Options` (:129-145). Recommend keeping the variant, re-gating it from `s3`
to `aws` (the session type stays core, Q5), and resolving through the register:
`backend_for(url.scheme()).ok_or(<install sentence>)?.holder_under(url, session, region, store_and_properties)`.
Effects: `warehouse/` names no store type; `s3tables/catalog.rs:642,645` ask `backend_for(&Scheme::S3)?.is_property`
(refusing with the install sentence at `Catalog::from_url`, where it reads its properties, if nothing is claimed);
and, because nothing in `yggdryl-iceberg` then names the leaf, **D15's "implying `yggdryl/s3`" becomes `yggdryl/aws`**: `s3tables`
needs the leaf only as a dev-dependency for its fake-store tests (`rust/tests/s3tables/catalog.rs`). This keeps the S6
chain avro, parquet, iceberg free of an s3 edge.
Alternative if the user wants `warehouse/` free of `aws::Session` as well: a `SiteOpener` trait object
(`Site::Opener { url, opener: Arc<dyn ..> }`, Eq/Hash by `url`), implemented by `s3tables` over `yggdryl_s3::S3Options`; that
adds a trait, an `Arc` per handle, and the `iceberg -> s3` edge back. Not recommended.

**Q5. Do `aws/`, `auth/`, `xml/scanner.rs` and `http/retry` leave with it?**
Recommend no, in this change: the leaf is `rust/src/s3/` only; `aws/` (14 files / 9,141 lines), `auth/` (5 / 828), the scanner
and the HTTP internals stay core. Reasons:
(1) `Request::with_sigv4` is an inherent method of `http::Request` written in `aws/request.rs:22-88` "so that `http/` depends
on nothing in `aws/`"; an inherent impl cannot be written in another crate (E0116), so moving `aws/` away from `http/`
forces an extension trait and publishing `Request::with_resend_on` (`http/request.rs:748-756`, `pub(crate)`, `cfg(aws)`);
(2) `auth/` is shared with `http/` (`http/authorization.rs:12`, `http/client.rs:571`, `http/session.rs:391,475`,
`http/tls.rs:36`), so it would split along `Lease`/`Bearer` (aws-gated) versus `Secret`/`variable`;
(3) AGENTS defines `aws/` for "every consumer that signs an AWS request", `s3tables` among them (AGENTS.md@HEAD:379);
(4) the 23 `cfg(feature = "s3")` gates inside those folders (4.7) need a decision either way: recommend the s3-only
helpers that no AWS consumer outside the leaf reads (`auth::Bearer`, `xml::scanner::{children, required}`,
`sigv4::encode_key`, `aws::environment::is_native`'s sweep) move into the leaf and the `Session` state (region cache,
`tls_config`, `stated_region`, `under`) is published, ungated from `s3` and gated `aws`.
The cost: the core keeps feature `aws` (hmac, ring, sha2) with the leaf and `s3tables` as its only consumers, and the
D6 list is longer (section 3.9). If the user wants `aws/` out too, that is a tenth crate (`yggdryl-aws`) or an
extension trait, and it is a separate decision, not a prerequisite.

**Q6. The `implementer` doors and `ureq` in a public signature.**
The leaf builds its own requests on `ureq` (`s3/client.rs:1156-1159` `ureq::http::Request::builder()`, 21 `ureq::` uses
in three files) over the core's shared pool, `agent_for(&HttpOptions, Option<ureq::tls::TlsConfig>) -> ureq::Agent`
(client.rs:423-432), and `Session::tls_config` returns `ureq::tls::TlsConfig` (aws/session.rs:1332). Publishing them puts a
`ureq` type in two core signatures. AGENTS bars the xxHash types from public signatures ("Digests": "The pinned xxHash
dependency's types never appear in a public signature") and `rust/Cargo.toml` keeps the official Iceberg stack private
("official Arrow 58 types never enter public APIs"); nothing states a rule for `ureq`, so the exception is new and must be
written. Recommend: publish them through the `implementer` module (DESIGN.md:199-221) and declare `ureq` in
`[workspace.dependencies]` next to the Arrow crates so the two crates share one resolution and the pool stays one
process-wide pool; state the exception in the `implementer` section of AGENTS. Rejected: rewriting the leaf's transport on
`http::Request` (changes the request-count contract and the retry accounting that `iobase_calls` and `rust/tests/s3/client.rs`
pin) and a second pool.

**Q7. Capabilities: trait methods or downcast?**
The six iceberg seams and `into_byte_stream` (4.5) ask one thing: "an object-store handle can be recast in a role, told a
size, streamed up, discarded". Recommend defaulted methods on `RegisteredHolder` (section 6), because `yggdryl-iceberg`
must not depend on the leaf outside `s3tables` (Q4). The callers differ on the default, which the trait must settle:
`staging::container` and `leaf` pass any other handle through (`other => Ok(other)`, staging.rs:310, 323) while
`folder_role` refuses it (catalog/mod.rs:531-538), so the defaults are `into_leaf`/`into_container` = `Ok(Holder::Registered(self))`
and `folder_role` keeps its own refusal for a `Registered` whose `into_container` declines (return `Option`, or let the
catalog test `implementation_name`). `upload_from` default = read the source whole and `write_all_bytes` (staging.rs:271-273's
`other` arm); `discard` default `Ok(false)` -> the caller removes (staging.rs:292-294); `with_known_size` default = identity
(staging.rs:342). `S3File::upload_from` is `pub(crate)` under `cfg(any(feature = "iceberg", feature = "internals"))`
(file.rs:343-344) and becomes the trait override.

**Q8. `into_byte_stream` timing.**
`Holder::into_byte_stream` has an S3 arm (holder/mod.rs:240-241) calling `S3File::byte_stream` (file.rs:412-434), which
opens the resuming `GET` **at construction**; `owned_stream_bytes` for the same file is the **lazy** stream (file.rs:564-577,
`lazy_object_stream` :1104-1139: "a reader never pulled costs no request at all"), and the generic arm is positional
(`Cursor::at`, holder/mod.rs:245). Callers: `bytestream.rs:451, 460`. Recommend keeping the eager override as
`RegisteredHolder::into_byte_stream` for this change so the `iobase_calls` and `rust/tests/s3/*` request orders stay
byte-identical (a cost pin is never re-pinned from a sweep, AGENTS "Pace"); unifying on `owned_stream_bytes` is a
separate, pinned, cost-reducing change.

**Q9. Properties into `holder()`.**
Recommend `holder(&self, location: &Url, properties: &[(String, String)])`: the form `from_url` already holds
(holder/mod.rs:364-366), and the query merge, the refusal of an unknown query parameter and the query strip need the
backend's own reader (3.3), so the core cannot pre-merge. `warehouse::Properties` (ordered, one value per name, replaced in
place) would be equivalent for duplicates but costs a conversion on the hot construction path. `described()` stays core.

**Q10. Do `local`, `zip`, `http` join the register?**
Recommend no. D30 already kept them (DESIGN.md:1292-1298). Their variants stay (`LocalFolder` etc. are the root traits'
memory-mapped types; HTTP is the transport the leaf rides on), so registering them deletes nothing and adds a lookup per
`from_url`. `claim_backend` refuses a scheme the core's arms hold (`file`, `http`, `https`) with the "the core answers it"
sentence, as `claim_factory` refuses `memory`/`folder` (catalog.rs:134-135, 192-197).

**Q11. One owner for the scheme list.**
The fact "which spellings address which store" is stated in `scheme.rs` (`is_s3`, `is_gs`, `is_az`), `Provider::from_scheme`
(provider.rs:35-46, which reads those predicates) and would be a third time in `Backend::schemes()`. Recommend
`schemes()` returns the ten consts and one core test (beside `rust/tests/media_register.rs`, whose 14 tests pin the
four registers over five test-only implementations, DESIGN.md S2 results) asserts, for each, `is_object_store()`, a claim by
the leaf, and `Provider::from_scheme(..).is_some()`; no second parser. Not recommended: moving the predicates into the
leaf (the URL grammar needs them, 4.2).

**Q12. Tests, benchmarks, docs, the CI row.**
Fixtures: the 3,869-line fake store `tests/support/server.rs` is `#[path]`-included by seven harnesses (3.1); the leaf's
harnesses include it from `../../tests/support/server.rs`, and the core harnesses that use it
(`iobase_calls`, `holder`, `warehouse`, `iceberg`, `medallion_ledger`) either dev-depend on the leaf (and call
`yggdryl_s3::install()` through one support function, D7) or move their S3 rows to the leaf. The moves are not re-pins:
`rust/tests/iobase_calls.rs:483-545` (`mod object_store`), `rust/tests/holder/mod_.rs:664-770`,
`rust/tests/warehouse/handle.rs:137-` and the `rust/tests/s3/` suite keep their counts. Benchmarks: `rust/benchmarks/holder/s3/` and its `object_store` baseline move with the leaf.
Docs: `docs/holder/index.md` "Object stores", `docs/media/iceberg.md`, `docs/media/index.md`, `docs/warehouse/index.md` and
`skills/yggdryl-storage/*`, `skills/yggdryl-uri/SKILL.md` name `yggdryl::s3` (`git grep -l` in section 9); the Rust page
runner compiles against `parquet iceberg s3 http3` (AGENTS section 5), so it gains the leaf. CI: section 4.7. The
crates.io name `yggdryl-s3` was **not read** in this pass (no network use); D24 and D33 each recorded a 404 read
(DESIGN.md:657, 1432), so S6d does the same before the manifest exists.

**Q13. Logging.** `s3/` has 8 `log` sites and 2 `warned!` sites. `is_foreign` (logging/facade.rs:72-76) treats
`yggdryl_s3::..` as foreign (a prefix of `yggdryl` followed by neither end nor `::`), so the per-crate target table D10
plans (`yggdryl_<folder>` -> `yggdryl.<folder>`) must name `yggdryl_s3` before S6d, as for `yggdryl_fix`.

**Q14. Merge risk with the working tree.** `git status` shows the in-flight P2/D34-D35 slice modifying files a P3 sweep
also owns: `rust/src/holder/mod.rs` (+4 lines: `read_origin_field` forward), `lib.rs`, `iceberg/table.rs`,
`iceberg/mod.rs`, `http/request.rs`, `warehouse/{media,namespace,table}.rs`, `media/format.rs`, `iomedia.rs` (the
`impl_default_iomedia!` the three S3 types expand lives there, iomedia.rs:1117-1138). Every line number above is HEAD's;
P3 must start from P2's commit and re-anchor its exact-string scripts.

**Q15. Wildcard arms: the compiler will not list the sites.**
Adding a variant to a `#[non_exhaustive]` enum does not break a `match` that ends in a wildcard, and these sites end in one,
so a `Registered` handle would take the generic branch with the tree green: `Holder::into_byte_stream` (`other =>
ByteStream::from_reader(Cursor::at(other, position), batch_size)`, holder/mod.rs:245: one ranged `GET` per batch instead of
one resuming `GET`); `iceberg/staging.rs` `upload` (:271, reads the whole file into memory), `unpublished` (:292, `remove(false)`
issues a `DELETE` for a key that was never written), `leaf` (:310) and `container` (:323, the undecided Path stays undecided and
probes), `sized` (:342, the size is not told and a `HEAD` follows); `iceberg/catalog/mod.rs:531` (loud: a refusal);
`python/src/iobase.rs:339` (`_ => Self::Held`: the class falls back to the base `IOBase`, the comment at :335-338 says so) and
:230 (`_ => return Ok(None)` in `container_holder`). Recommend the P3 sweep writes an explicit `Registered` arm at each, and
ends with `git grep -n 'other =>' HEAD -- rust/src/holder/mod.rs rust/src/iceberg/staging.rs` style checks plus the existing
request-count pins (`rust/tests/iceberg/staging.rs`, `rust/tests/iobase_calls.rs`), which are what would catch a miss.

**Q16. Names.** `Backend` collides with nothing in `rust/src`, `python/src`, `node/src` or `cli/src` (the only hit for the
word is a comment, `fs/system.rs:173`), but it reads loosely beside `Codec` and `MediaCodec`; AGENTS calls the row "storage
backend" (AGENTS.md@HEAD "Common changes": "storage backend"). `StorageBackend` + `holder::claim_backend`/`backend_for`/
`backends`, `RegisteredHolder` (parallel to `RegisteredTable`, `RegisteredCatalog`), `S3_BACKEND` and `S3Backend`; the
crate `yggdryl-s3`, folder `rust/s3`. Any consistent spelling serves; pick one before P3 because the inventory, AGENTS and
the pins carry it.

### 7.1 The recommendations at a glance

| Q | Recommendation |
| --- | --- |
| 1 | A separate `Backend` register in `holder/backend.rs`; `Locator` stays the catalog-service point |
| 2 | One `Holder::Registered(Box<dyn RegisteredHolder>)`; delete `S3Folder`, `S3Path`, `S3File`; the role traits stay in the leaf |
| 3 | `reopen(&self)` on the object, not on the register |
| 4 | `Site::Store` stays, gated `aws`, resolved by `Backend::holder_under`; D15's `yggdryl/s3` becomes `yggdryl/aws`, no `iceberg -> s3` edge |
| 5 | `aws/`, `auth/`, `xml/scanner.rs`, `http/` internals stay core (`with_sigv4` is an inherent method); s3-only helpers inside them move to the leaf or are re-gated |
| 6 | Publish the ~20 crate-private items through `implementer`, `ureq` types included; `ureq` in `[workspace.dependencies]`; write the exception in AGENTS |
| 7 | Defaulted capability methods on `RegisteredHolder` (`into_container`, `into_leaf`, `with_known_size`, `upload_from`, `discard`, `into_byte_stream`) |
| 8 | Keep the eager `into_byte_stream`; unify on `owned_stream_bytes` only as its own pinned change |
| 9 | `holder(&Url, &[(String, String)])`; query merge and refusal stay in the backend, `described()` in the core |
| 10 | `local`, `zip`, `http` stay `from_url` arms; `claim_backend` refuses their schemes |
| 11 | `schemes()` is the ten consts; one core test ties it to `is_object_store` and `Provider::from_scheme` |
| 12 | Fixtures by `#[path]`, S3 rows moved not re-pinned, crates.io name read at S6d |
| 13 | The D10 logging table names `yggdryl_s3` before S6d |
| 14 | Start from P2's commit; every line number here is HEAD's |
| 15 | Write an explicit `Registered` arm at each wildcard site (list in Q15); the compiler will not |
| 16 | Name the trait `StorageBackend`, the object `RegisteredHolder`; decide before P3 |

## 8. What the two steps contain (to size them, not a plan to execute)

P3 and S6d are this report's labels for "registered in place" and "moved"; the DESIGN.md slice table (:714-734) has no D36 row yet,
and the ledger row says only "registered in place first and moved after" (MARKET_SPLIT_NEXT.md:208).

**P3, registered in place (the S2 analogue, the core builds and every pin holds):** `holder/backend.rs` (`Backend`,
`RegisteredHolder`, register, seed, `claim_backend`, `backend_for`, `backends`); `S3Backend` and `S3_BACKEND` in
`s3/mod.rs`; `Holder::Registered`; the three variants deleted and `From<S3Folder|S3Path|S3File> for Holder`; `holder/mod.rs`
(58 lines) re-cut; the six iceberg seams on the trait; `Site::Store` through `holder_under`; the bindings' 82 lines re-spelled
(Python subclass chosen by `implementation_name()` or `downcast_ref`); `rust/tests/media_register.rs` (or a sibling) gains a
backend register test over a test-only backend: a second claim names the first claimant, `CORE` is refused, a core scheme
is refused, an unclaimed scheme is refused with the install sentence, and the install refusal before the claim; docs,
`.api-inventory.txt`, `.api-bindings.txt`, AGENTS rows ("registered backend" next to "registered medium"). One pin is missing and should be written first: no test passes `media_type` or `codec` to an
`s3://` location through `Holder::from_url` (`git grep -n '"media_type"\|"codec"' HEAD -- rust/tests/s3 rust/tests/s3.rs rust/tests/holder/mod_.rs`
hits only the local block, holder/mod_.rs:467-471, and the HTTP block, :824-839), yet `described()` applies both to every
backend (holder/mod.rs:452-470); the backend arm must keep falling through it. Pins that must
hold unmoved: `rust/tests/holder/mod_.rs:686-770`, `iobase_calls.rs` `object_store`, the whole `rust/tests/s3/` suite, the
`s3tables` request counts (`rust/tests/s3tables/catalog.rs`), `rust/tests/iceberg/{staging,scan,catalog}` against the fake.
**S6d, the move:** `git mv rust/src/s3 rust/s3/src` (26 files), tests (20 + 8 files), benchmarks, `internals` re-exports;
D6 list from the scratch `git mv` + `cargo check -p yggdryl-s3 --message-format=short` (section 3.9 is its orientation);
`install()`; `[leaves]` row; AGENTS section 6 name list and release preflight gain the crate (as D24/D33 did).
Nothing is published.

## 9. Commands the counts came from (all against HEAD unless noted)

```bash
git rev-parse HEAD                                                    # d100439d3a1d793475e7168a404463ef903f6940
git ls-tree -r HEAD --name-only | grep -E '^rust/src/(s3|local|fs|zip|aws|auth|s3tables|http)/' | while read f; do echo "$(git show HEAD:$f | wc -l) $f"; done
git grep -cE 'crate::s3\b|Holder::S3|S3Options|S3Folder|S3Path|S3File|feature = "s3"|is_object_store|yggdryl::s3\b' HEAD -- rust/src ':!rust/src/s3'   # 146 lines, 15 files
git grep -n 'feature = "s3"' HEAD -- rust/src ':!rust/src/s3' | wc -l                                  # 79
git show HEAD:rust/src/holder/mod.rs | grep -c 'feature = "s3"'                                         # 26
git grep -n 'S3Folder\|S3Path\|S3File' HEAD -- rust/src ':!rust/src/s3' ':!rust/src/holder/mod.rs'     # 8 (iceberg/staging.rs x5, catalog/mod.rs x3 lines)
git grep -n 's3://' HEAD -- rust/src ':!rust/src/s3/'                                                  # 16 lines, 9 files, all comments/doc tests
git grep -n 'Holder::from_url' HEAD -- rust/src                                                        # callers: plan.rs, isin_registry/store.rs, uri/handle.rs, warehouse/handle.rs
git grep -nE 'Scheme::S3|Scheme::GS|Scheme::AZ|Scheme::GCS|Scheme::ABFS|Scheme::WASB|Scheme::S3A|Scheme::S3N' HEAD -- rust/src ':!rust/src/scheme.rs'
git grep -n 'store_location()' HEAD -- rust/src                                                        # uri/{mod,arn}.rs
git grep -n 'is_object_store\|is_s3()\|is_gs()\|is_az()\|has_container\|is_s3_tables' HEAD -- rust/src ':!rust/src/s3/' ':!rust/src/scheme.rs'
git grep -n 'is_backend_property\|Holder::from_handle\|from_handle_with' HEAD -- rust/src python/src node/src cli/src
git grep -ho 'crate::[a-z_0-9]*\(::[A-Za-z_0-9]*\)\{0,2\}' HEAD -- rust/src/s3 | grep -v '^crate::s3' | sort | uniq -c | sort -rn
git grep -h '^use crate::\|^    use crate::\|^pub use crate::' HEAD -- rust/src/s3 | sort -u             # the use-line view, as D24/D33 did
git grep -c 'ureq::' HEAD -- rust/src/s3 rust/src/aws rust/src/http                                   # s3: azure/auth 6, client 9, google/token 6
git grep -n 'static \|thread_local\|OnceLock<\|LazyLock' HEAD -- rust/src/s3                          # no process-global state
git grep -n 'support/server.rs\|support/s3tables.rs' HEAD -- 'rust/tests/*.rs'                         # seven harnesses share the fake store
git grep -n 'Holder::S3' HEAD -- rust/tests rust/benchmarks python/tests node/tests                   # 14 test lines in 6 files
git grep -cE 'yggdryl::s3\b|S3Options|Holder::S3|S3Folder|S3Path|S3File' HEAD -- python/src node/src cli/src   # 82 lines, 5 files
git grep -l 'yggdryl::s3\|S3Options\|S3File\|S3Folder\|S3Path' HEAD -- docs skills README.md AGENTS.md .api-inventory.txt .api-bindings.txt scripts .github
git grep -n 'size_of::<Holder>' HEAD -- rust                                                           # empty
git status --short | awk '{print $2}' | grep '^rust/src'                                               # working tree: the in-flight slice's files (Q14)
```

Counts that are my own reading rather than a grep: the 25 `Holder` variants (counted from holder/mod.rs:90-172), the ten
iceberg/Holder special-case sites (4.4, 4.5), and the "about twenty" crate-private items (section 3.9 lists them).
