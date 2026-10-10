# Holder and backend dispatch map for isolating yggdryl-s3

Read at HEAD d100439d3a1d793475e7168a404463ef903f6940 (branch ccr-0fe6f9d0-ruymat), every rust/src, rust/tests, python/src, node/src, cli file through `git show HEAD:<path>` / `git grep ... HEAD`. No cargo run, nothing edited. Line numbers are HEAD's. Working-tree drift touching this map: `rust/src/holder/mod.rs` gains one forward (`read_origin_field`, +4 lines near 1086, D34/D35), `rust/src/iceberg/table.rs` is edited (144 lines); neither moves a site below except that `iceberg/table.rs` lines are HEAD's. "S3" below means the object-store backend (rust/src/s3), never the program's slice S3.

Program context read: MARKET_SPLIT_NEXT.md "## Goal"/"## Invariants" (registers on `plugin::Register`, "each seeded by the core with its own until the leaving crate's `install()` claims them"); DESIGN.md D21 (locator row: "`Locator`: `holder(location, properties) -> Holder` replaces `Holder::from_url`'s per-scheme arms"), D27 (`Media::Registered(Box<dyn MediaWrapper>)`), D30 ("`Holder::from_url` asks the locators first and keeps every byte-backend arm (`local`, `zip`, `s3`, `http`), which stay the core's" - the line the new instruction supersedes for `s3`), D15 (`s3tables` is a feature of `yggdryl-iceberg` implying the s3 backend), D36 ledger row (design pending).

## 1. Facts: the Holder today

### 1.1 The enum (holder/mod.rs:87-172)

```rust
#[derive(Debug)]            // :87  - the only derive
#[non_exhaustive]           // :88
#[allow(clippy::large_enum_variant)]   // :89
pub enum Holder {           // :90
```

| variant | holds | line |
| --- | --- | --- |
| Buffer | `Buffer` | 92 |
| LocalFolder / LocalPath / LocalFile | `LocalFolder` / `local::LocalPath` / `LocalFile` | 94 / 96 / 98 |
| FsFolder / FsPath / FsFile | `fs::FsFolder` / `FsPath` / `FsFile` | 100 / 103 / 105 |
| **S3Folder** | `crate::s3::S3Folder`, `#[cfg(feature = "s3")]` | 107-108 |
| **S3Path** | `crate::s3::S3Path`, cfg s3 | 110-111 |
| **S3File** | `crate::s3::S3File`, cfg s3 | 113-114 |
| HttpSession / HttpRequest / HttpResponse / HttpStream | `http::Session` / `Request` / `Response` / `Stream`, cfg http | 117 / 121 / 124 / 127 |
| ZipNode / ZipPath / ZipLeaf | `zip::ZipNode` / `ZipPath` / `ZipLeaf` | 129 / 131 / 133 |
| Buffered | `Box<Buffered<Self>>` | 138 |
| Coded | `Box<Coded>` | 143 |
| Text | `Box<text::Text<Self>>` | 149 |
| Media | `Box<media::Media>` | 156 |
| Uri | `Uri` (resolved late through `from_url`) | 160 |
| Catalog / Namespace / Table | `Box<crate::Catalog>` / `Box<Namespace>` / `Box<Table>` | 166 / 168 / 171 |

The S3 backend is exactly three variants, one per role of the trio: `S3Folder`, `S3Path`, `S3File`. There is no single `S3` variant and no `Object`/`Aws`/`Gcs`/`Azure` variant (`git grep -n "Holder::\(Object\|Aws\|Gcs\|Azure\)"` is empty). The three are held inline (no Box), which is why `#[allow(clippy::large_enum_variant)]` exists.

**No variant holds a bare `Box<dyn IOBase>` or any `Box<dyn ...>` of its own.** The only trait objects reachable inside a `Holder` are one level down, in the enums behind a Box: `Media::Registered(Box<dyn MediaWrapper>)` (media/mod.rs:96; `MediaWrapper: IOBase + Sync + fmt::Debug`, media/codec.rs:142), `Table::Registered(Box<dyn RegisteredTable>)` (warehouse/table.rs:106; `RegisteredTable: TableValue + IOBase + Send + Sync + fmt::Debug`, :42), `Catalog::Registered` and `Namespace::Registered` (warehouse/catalog.rs:41, namespace.rs:81). None can carry a byte backend: `Holder::Table` answers `has_media_surface() == true` (holder/mod.rs:815) so `into_media`/`into_declared_media` skip it, and `Table::Registered` demands `TableValue`, `clone_box`, `dyn_eq`, `dyn_hash`. Reusing it for an object leaf would be wrong.

### 1.2 Trait impls on Holder

`git grep -n "impl.*\(Clone\|PartialEq\|Eq\|Hash\|Display\|Default\|Drop\) for Holder" HEAD -- rust/src` is empty. Holder is `Debug` (derived, :87) and `IOBase` (:1129) / `IOMedia` (:1057) by hand; it is not `Clone`, `Eq`, `Hash`, `Display`, serde. There is no `From<S3Folder|S3Path|S3File> for Holder` (the From list is 1314-1465: `TryFrom<&Url>`, Uri/Url/Urn/Arn, Catalog/Namespace/Table/Object, Buffer, LocalFolder, LocalFile, FsFolder/FsPath/FsFile, the four http types, Buffered, Coded, Text, Media); callers write `Holder::S3File(...)` directly. No wire contract names a Holder variant (no serde, no pickle: `git grep -n "__reduce__\|__getstate__" HEAD -- python/src/iobase.rs python/src/holder` only hits PyArrow's own `__reduce__` in holder/fs.rs:908,999,1003).

Consequences for a registered variant: needs `Debug + Send + Sync` and nothing else from the trait object (`Sync` because `Handle` keeps `OnceLock<Box<Holder>>` (warehouse/handle.rs:214) inside `IcebergTable<Handle>` inside `Box<dyn RegisteredTable>` which is `Send + Sync`, and `Site::Native` keeps `Arc<Holder>` (:29)); no `clone_box`/`dyn_eq`/`dyn_hash` (unlike Table/Catalog) because Holder has none of those. "Held again on the same client" is the explicit `Holder::from_handle` (1.5), not `Clone`.

### 1.3 Every match on Holder variants

Exhaustive (compile-forced to gain a `Registered` arm) - all in holder/mod.rs:

| fn | lines | S3 arms | note |
| --- | --- | --- | --- |
| `exists` | 191-223 | 201-206: `S3Folder(inner) => inner.folder_exists()`, `S3Path(inner) => inner.path_exists()`, `S3File(inner) => inner.file_exists()` | role-specific question |
| `from_handle` | 498-558 | 518-532 | see 1.5; also the `unreachable!("the plain handle beneath every wrapper")` group 550-552 |
| `as_io` | 905-940 | 914-919 | `Holder` -> `&dyn IOBase` |
| `as_io_mut` | 943-978 | 952-957 | |
| `as_media` | 981-1016 | 990-995 | `&dyn IOMedia` |
| `as_media_mut` | 1019-1054 | 1028-1033 | |

Wildcard (compile-silent; a new variant lands in the `_`/`other` arm and **changes behavior**):

| site | lines | what the wildcard does to a registered object handle |
| --- | --- | --- |
| `Holder::into_byte_stream` | 233-247 (`S3File(file) => file.byte_stream(...)` at 240-241, wildcard 245) | falls to `ByteStream::from_reader(Cursor::at(other, position), batch_size)` = one ranged read per batch instead of S3's one resuming `GET` (s3/file.rs:412-434). Used by bytestream.rs:451,460 (container streaming). |
| `local_url` | 1218-1226 | `_ => None`; no change |
| `into_declared_media` | 718-722 | `_ => None`; no change |
| `plain` / `has_media_surface` | 561-568 / 812-819 | no change |
| iceberg `folder_role` | iceberg/catalog/mod.rs:505-540 (`S3Folder(_) => Ok(child)` 508-509; `S3Path` 527-528; `S3File` 529-530; wildcard 531-538 returns `invalid("expected a backend that can hold a folder for a catalog name")`) | refuses the catalog name |
| iceberg `folder_present` | iceberg/catalog/mod.rs:477-490 | `other => other.ls(false, true).next()` (already S3's path) |
| iceberg `upload` | iceberg/staging.rs:264-276 (`S3File(file) => file.upload_from(&mut source, size)` 266-270) | falls to `std::fs::read(path)` + `write_all_bytes`: the whole staged file in memory, no multipart |
| iceberg `unpublished` | staging.rs:286-298 (`S3File(file) => { let _ = file.discard(); }` 288-291) | falls to `other.remove(false)` unless `keeps`: a `DELETE` for a key never written (changes request counts the staging tests pin) |
| iceberg `leaf` | staging.rs:306-312 (`S3Path(path) => S3File(path.as_file()?)` 308-309) | handle stays an undecided path, resolved by one extra listing |
| iceberg `container` | staging.rs:319-325 (`S3Path(path) => S3Folder(path.as_directory()?)` 321-322) | same extra listing |
| iceberg `sized` | staging.rs:338-344 (`S3File(file) if size > 0 => S3File(file.with_known_size(size))` 340-341) | handle forgets the manifest-recorded size, one `HEAD` more per file |

The four wildcard iceberg fns are the real design constraint: they call S3-only inherent verbs - `S3File::upload_from` (`pub(crate)`, file.rs:344), `discard` (`pub(crate)`, :401), `with_known_size` (pub, :125, by value), `S3Path::as_file`/`as_directory` (pub, path.rs:160,151), `S3File::as_directory` (pub, file.rs:168) - none of which is on `IOBase`/`IOFile`. Python has the same set (python/src/iobase.rs:115-133, 227-229, 247-249).

Other core sites that name S3 (not Holder matches):

| site | lines | what |
| --- | --- | --- |
| `Holder::from_url` object-store arm | holder/mod.rs:384-422 | 3.1 |
| `Holder::is_backend_property` | 592-604 (`#[cfg(feature = "iceberg")]`) | `url.scheme().is_object_store()` -> `crate::s3::S3Options::is_property(name)`; read by iceberg/table.rs:535-548 `stating` |
| `Site` (warehouse/handle.rs) | `Store` variant 45-52 (`#[cfg(feature = "s3")]`, `allow(dead_code)` unless s3tables); Debug 63-74; `Site::of` 86-105 (`scheme.is_object_store() || scheme.is_http()` at 95, `Holder::from_handle(holder)` at 98); `url()` 112-113; `resolve` 129-145 builds `crate::s3::S3Options::default().with_environment(session.reads_environment()).with_session(...).with_region(...).with_properties(store.iter().chain(properties.iter()))?` then `crate::s3::located_with(&url.to_string(), options)`; `Eq` 156-157; `Hash` 173-174 | `Site::Store` constructed only at s3tables/catalog.rs:1151 |
| s3tables catalog | s3tables/catalog.rs:642,645 | `crate::s3::S3Options::is_property(name)` splits the bag into store knobs vs catalog properties |
| lib.rs | 131 (`#[cfg(feature = "s3")] pub mod s3`), 528-551 (12 `internals` re-exports `s3_answer ... s3_xml`) | |

Dispatching uses of the scheme predicate `Scheme::is_object_store` (scheme.rs:308-310 = `is_s3() || is_gs() || is_az()`): holder/mod.rs:384, :595, warehouse/handle.rs:95, python/src/iobase.rs:188, node/src/iobase.rs:129. (scheme.rs:335 `has_container` and uri/authority.rs:312-451, fs/location.rs:430-454 are URI vocabulary, not dispatch; they stay core.)

### 1.4 Forwarding over the variants

`Holder` does not use `delegate_iobase!` (that macro, iobase/bytes.rs:138, is for wrappers over a handle field). `impl IOBase for Holder` (:1129-1312) is 40 hand-written one-line forwards, every one `self.as_io()` / `self.as_io_mut()` except `local_url` (:1218, a 4-arm match with wildcard). `impl IOMedia for Holder` (:1057-1127) forwards 11 methods through `self.as_media()`/`as_media_mut()`. So `uri`, `url`, `kind`, `is_container`, `is_atomic`, `is_thread_bound` (:1305), `is_tabular`, `parent`, `child_by_path`, `ls` are all four-matches' dispatch; a variant gaining its arms in `as_io`, `as_io_mut`, `as_media`, `as_media_mut` gains the whole 51-method surface. `is_thread_bound` default is `false` (iobase.rs:725); only `fs/` overrides it.

### 1.5 `Holder::from_handle` per variant (holder/mod.rs:498-558)

Doc (472-497): "a second handle on the resource `holder` addresses, over the same store, touching nothing". `plain` strips the wrappers (561-569). Arms: LocalFolder cloned; LocalPath rebuilt from url; LocalFile rebuilt from path; Fs* via `plain.bound_location()` -> `fs::located(bound.clone())`; Http session/request cloned; Http response/stream `Unsupported`; Zip* -> `sibling(plain)` and check `member.url() == plain.url()`; Uri/Catalog/Namespace/Table cloned; Buffer `Unsupported`. **S3 arms (518-532):**

```rust
Self::S3Folder(folder) => Self::S3Folder(folder.clone()),            // S3Folder is #[derive(Clone)] (s3/folder.rs:35)
Self::S3Path(path) => match sibling(plain)? {                        // sibling: parent().child_by_path(file_name) (:47-58)
    Some(sibling) => sibling,
    None => Self::S3Folder(path.as_directory()?),                    // bucket root / trailing slash: a container
},
Self::S3File(_) => match sibling(plain)? {
    Some(Self::S3Path(path)) => Self::S3File(path.as_file()?),
    Some(held) => held,
    None => return Err(unsupported("holding again an object with no container")),
},
```

The point (doc 478-481): the client (endpoint, credentials, `aws::Session`, pool, options) travels because the new handle is built on `Arc<Client>` through the parent folder's `S3Folder::new(self.client.clone(), parent)` (s3/path.rs:560, file.rs:1014, folder.rs:464). Afterwards `media_type` is carried (554-556). `S3Path` and `S3File` are not `Clone` (S3File has `Drop` file.rs:1034 and a state `Mutex`; S3Path has two `Mutex`es path.rs:42-63), which is why this is a method and not `clone()`. Callers: warehouse/handle.rs:98 (`Site::of`) and :126 (`from_handle_with`, holder/mod.rs:576-582, which then applies `described`); tests rust/tests/holder/mod_.rs:737-780 pin "holding again sends nothing" and the roles kept.
Python re-implements this logic in `cloned` (python/src/iobase.rs:101-145, S3 arms 115-133, `store_sibling` 151-157) - a binding-side second dispatcher.

## 2. What a backend implements (the trio)

Traits (all `: IOBase`): `IOPath` iopath.rs:10 requires `path_url`, `is_folder`, `is_file` (rest provided: `path_exists`, `path_kind`, `path_media_type`, `path_is_atomic`, `path_is_tabular`); `IOFolder` iofolder.rs:15 requires `folder_url`, `has_folder`, `create_folder`, `list_folder`, `delete_folder` (provided: `folder_exists` :65, `folder_clear`, `folder_remove`, `folder_ls`, `folder_pread`, `folder_pstream_bytes`, `folder_read_all_bytes`, `folder_read_range_bytes`, `folder_pwrite`, `folder_truncate`, `folder_media_type`, `folder_kind`, `folder_is_atomic`, `folder_is_tabular`); `IOFile` iofile.rs:11 requires `file_url`, `file_exists`, `clear_file`, `delete_file` (provided `file_remove`, `file_ls`, `file_kind`, `file_is_atomic`, `file_is_tabular`, `file_child_by_path`).
`IOBase: Send + IOMedia` (iobase.rs:283) requires 10 methods: `pread`, `pwrite`, `size`, `capacity`, `reserve`, `truncate`, `uri`, `media_type`, `set_media_type`, `create_bytes`; `IOMedia` (iomedia.rs:192) requires `as_io_base`, `as_io_base_mut`, `overwrite_serie`, all three supplied by `crate::impl_default_iomedia!()` (a `#[macro_export]`, iomedia.rs:1117-1139, expanding only to `$crate::` public paths).
S3 implements: `S3File` IOFile (file.rs:454) + IOMedia (498) + IOBase (502) overriding 32 methods; `S3Folder` IOFolder (folder.rs:325) + IOMedia (397) + IOBase (401), 22 overrides; `S3Path` IOPath (path.rs:334) + IOMedia (357) + IOBase (361), 30 overrides. Hierarchy methods return `Holder` (`parent() -> Option<Holder>`, `child_by_path -> Result<Holder>`, `ls -> Listing` of `Result<Holder>`; iobase.rs:491,507,537): the S3 types construct `Holder::S3*` at s3/folder.rs:318,320 (`hold`), :464 (`parent`), :475,477 (`child_by_path`), s3/path.rs:560, s3/file.rs:1014, s3/mod.rs:160 (`located_with`). `S3Path` holds `Resolved::{Directory(S3Folder), File(S3File)}` (path.rs:82-100), "Deliberately not [`Holder`]" (path.rs:80-83), so the three roles are closed over each other inside the s3 crate and never need a Holder variant to hold each other.
What S3 reaches in the holder/iobase layer that is not public: `crate::holder::system_time_ns` (`pub(crate)`, holder/mod.rs:33; s3/azure/sign.rs:215), `crate::iobase::oversized` (`pub(crate)`, iobase/lifecycle.rs:82; 17 uses), `ByteStream::from_handle` (`pub(super)`, bytestream.rs:126; s3/file.rs:555). `ByteStream::from_reader` and `not_empty` are `pub`. (Everything else S3 reads from the core - `aws::`, `http::retry`, `json::`, `xxhash::`, `logging::warning`, `uri::percent_*` - is the implementer-surface mapper's.)

## 3. `Holder::from_url`, today

### 3.1 Flow (holder/mod.rs:355-446)

1. Properties collected as `Vec<(String, String)>` (364-367) - order and duplicates kept.
2. **Locator ask, before lowering** (368-373): `if let Some(holder) = locator::locate(location, &properties)? { return Ok(holder); }` - returns the locator's holder **verbatim**; step 7 (`described`) is skipped.
3. `let url = &location.locator()?;` (374) lowers a URN to a path, an ARN to its store URL (`Arn::locator`, uri/arn.rs:406-426: only an Amazon S3 bucket ARN -> `s3://` and an S3 Tables ARN -> `s3tables://`; `arn:aws:s3:::trades/2026/x` -> `s3://trades/2026/x`), a relative `file:` to rooted (uri/mod.rs:369-376).
4. `url.is_local()` (375): fragment -> `zip::from_url`, else `LocalPath` (375-383).
5. **`url.scheme().is_object_store()`** (384), cfg s3:
   ```rust
   let mut stated = Vec::new();
   for (name, value) in &url.parameters(true)? {
       if !crate::s3::S3Options::is_property(name) {
           return Err(crate::Error::Parse { target: "object store location", position: 0,
               reason: format_smolstr!("the query parameter {name:?} names no property an object store reads") });
       }
       stated.push((name.to_owned(), value.to_owned()));
   }
   let options = crate::s3::S3Options::from_properties(
       stated.iter().chain(properties.iter()).map(|(name, value)| (name, value)))?;   // query first, caller's last: "a property stated twice is the caller's"
   let mut location = url.clone();
   location.set_query(None)?;                                           // "names the resource, not how it is reached"
   crate::s3::located_with(&location.to_string(), options)?             // s3/mod.rs:157-161 -> S3Path -> Holder::S3Path
   ```
   Without the feature (416-422): `Error::unsupported("holding an object store location without the s3 feature", scheme)` - no test, doc or skill pins that string (`git grep -n "without the s3 feature" HEAD` only hits holder/mod.rs:419).
6. `url.scheme().is_http()` (423-437), else (438-444) `Error::unsupported("holding a location of this scheme; install the crate that claims it and call its `install()`", scheme)` (pinned by docs/holder/index.md:2526, rust/tests/warehouse/catalog.rs:133-134).
7. `held.described(url, &properties)` (445; private fn 452-470): reads **only** `media_type|mime_type|content_type` (-> `set_media_type`) and `codec|content_encoding` (-> `into_coded_with`, skipped when the location spells a container: `url.is_glob() || url.has_trailing_slash()`, not for http). Every other property is "left to the backend, which ignores what it does not know".

### 3.2 How properties reach `S3Options`

All caller properties (including `media_type`, `codec`) go to `S3Options::from_properties` (s3/properties.rs:363-369 = `Self::default().with_properties(...)`, :97-114) because "A name this crate does not know is *ignored*" (:10); explicit rule list at :10-17. `S3Options::is_property(name)` (:178-182) is answered by the reader itself (`read(&canonical(name), name, "x", ...)`), so the query-refusal and `is_backend_property` can never disagree with a read. Vocabularies: this crate's, PyIceberg (`s3.*`, `gcs.*`, `adls.*`), PyArrow, each store's env names; names matched loosely, prefixes `s3.`, `gcs.`, `gs.`, `google.`, `gcp.`, `azure.`, `adls.`, `abfs.`, `client.`, `aws_` dropped (:55-58). `S3Options::PROPERTY_NAMES` is a `[&str; 47]` (:120) the bindings suggest against.

### 3.3 The schemes the object-store arm takes

`Scheme::is_object_store` (scheme.rs:308) = `is_s3` (:263: `s3`, `s3a`, `s3n`) + `is_gs` (:282: `gs`, `gcs`) + `is_az` (:292: `az`, `abfs`, `abfss`, `wasb`, `wasbs`) = **10 spellings**, listed in the s3/mod.rs:14-16 doc, parsed by `Scheme::from_str` (scheme.rs:405-415), consts `Scheme::S3 ... WASBS` (:101-122), and mapped to a store by `Provider::from_scheme` (s3/provider.rs:35-46). `s3tables` is deliberately not one (scheme.rs:267-275). `s3/mod.rs::parse` (:334-340) re-checks `Provider::from_scheme(url.scheme()).is_none() || url.bucket().is_none()`. The `Scheme` ordering is by its text (`Ord`, scheme.rs:471-475), so a per-scheme register lists `abfs, abfss, az, gcs, gs, s3, s3a, s3n, s3tables, wasb, wasbs`.

## 4. `locator.rs` today: what it can and cannot answer for `s3://`

```rust
pub trait Locator: fmt::Debug + Send + Sync + 'static {          // locator.rs:23
    fn scheme(&self) -> Scheme;                                  // :26  ONE scheme, the register key
    fn names(&self, location: &Uri) -> bool;                     // :31  asked BEFORE the identifier is lowered
    fn holder(&self, location: &Uri, properties: &Properties) -> Result<Holder>;   // :39
}
static LOCATORS: Register<Scheme, &'static dyn Locator> = Register::new("locator");   // :42
```
`seed()` (:47-60) claims only `S3TABLES_LOCATOR` (cfg s3tables) under `CORE` (plugin.rs:18 `"yggdryl"`); `claim` (:70-81) refuses `by == CORE` from outside; `locators()` (:85-88); `locate` (:97-109) iterates `LOCATORS.values()` (a fresh `Vec` clone per call) and answers the first whose `names` is true. The one implementation (s3tables/mod.rs:164-183) answers `Ok(locate(location, properties)?.into_holder())`, `names` = `location.names_s3_tables()` (uri/mod.rs:386, `pub(crate)`; reads private `arn::service_of`).

**It can answer a Holder, not only warehouse objects**: the signature returns any `Holder`. Doc text (:4-10) and the from_url comment (368-370) scope it to "an object a locator names - a catalog service's, not bytes - declares no media type and takes no coding", and the doc says the byte backends "stay the core's". Gaps to answer `s3://` through it as written:

1. One scheme per claim; the backend has 10 (+ the S3 ARN). Ten claims of one static, with no all-or-none helper (media/codec.rs:160,209-250 has the pattern: a `CLAIMING` mutex, check every key, then claim).
2. `from_url` returns the locator's holder at 371-373, skipping `described` (445): `Holder::from_url("s3://b/k.bin", [("media_type", ...)])` and `codec` would silently stop applying to S3 handles (no s3 test pins them today, section 8.2). `described` is private (452).
3. The query-parameter refusal, the query strip, and "query beneath the caller's properties" (384-414) are in the arm; a locator would carry them (needs `Url::parameters`, `set_query`, both public).
4. `names` runs before lowering, so for an Amazon S3 ARN it receives the `arn:` Uri; the backend must lower it itself (public `Arn::locator`), while `names_s3_tables` is `pub(crate)`.
5. `Holder::is_backend_property` (594-604) has no locator-side equivalent (`Locator` has no `is_property`).
6. The register is asked first, for every location including `file:` and `http:`; `names` must be a scheme compare before any lowering.
7. Zero variant can hold the answer: while `Holder::S3*` name `crate::s3::S3*` the types cannot leave the crate (1.1).

## 5. (a) Shape of `Holder::Registered`

Recommended (mirrors `Media::Registered(Box<dyn MediaWrapper>)`, media/mod.rs:96, and `Table::Registered`, warehouse/table.rs:106 - one variant for every outside implementation, the role named by the object, not by three variants):

```rust
// holder/mod.rs
/// An outside byte backend's handle: a path, a folder or a file ...
Registered(Box<dyn RegisteredHandle>),

// holder/backend.rs (new; holder/ is "what every backend shares")
pub trait RegisteredHandle: IOBase + Sync + fmt::Debug {
    fn implementation_name(&self) -> &'static str;      // "S3Folder"|"S3Path"|"S3File": a refusal's word and Python's class pick (cf. media `Encoding::Other(title)`, python/src/iobase.rs:374)
    fn role(&self) -> HandleRole;                       // Path | Folder | File: replaces the three `exists` arms and drives the role casts
    fn exists(&self) -> bool;                           // exists() arm (201-206)
    fn held_again(&self) -> Result<Holder>;             // from_handle arm (518-532): clone / sibling-of-parent, role kept, nothing sent
    fn byte_stream(&self, position: u64, batch_size: usize) -> Result<ByteStream<'static>>;  // into_byte_stream arm (240-241); needed because the wildcard changes the request count
    // role casts the iceberg staging/catalog fns call (defaults: the handle unchanged / None):
    fn as_folder(&self) -> Result<Holder>;              // path.as_directory(), file.as_directory()  (catalog/mod.rs:528,530; staging.rs:322; python 120,228,229)
    fn as_file(&self) -> Result<Holder>;                // path.as_file()  (staging.rs:309; python 130)
    fn with_known_size(self: Box<Self>, size: u64) -> Box<dyn RegisteredHandle>;   // staging.rs:341
    fn upload_from(&mut self, source: &mut dyn Read, length: u64) -> Option<Result<()>>;  // staging.rs:269 (None: the caller writes whole)
    fn discard(&self) -> Option<Result<()>>;            // staging.rs:290
    fn as_any(&self) -> &dyn Any; fn as_any_mut(&mut self) -> &mut dyn Any; fn into_any(self: Box<Self>) -> Box<dyn Any>;   // downcast door, cf. RegisteredTable::as_any (table.rs:58-61)
}
impl Holder { pub fn registered(h: impl RegisteredHandle + 'static) -> Self; pub fn downcast_ref<T: 'static>(&self) -> Option<&T>; pub fn downcast_mut<T: 'static>(&mut self) -> Option<&mut T>; }   // cf. Table::downcast_ref (table.rs:137-152)
impl From<Box<dyn RegisteredHandle>> for Holder
```
`Sync` supertrait is forced (1.2). Trait upcasting (`&**handle` to `&dyn IOBase`/`&dyn IOMedia`) is the exact pattern of media/mod.rs:210,220,235,245 and warehouse/table.rs:121-130. The role casts could instead be a downcast to `S3File` from `yggdryl-iceberg`, but then plain `iceberg` (no `s3tables`) would depend on `yggdryl-s3`; hooks with default bodies keep staging.rs free of every `#[cfg(feature = "s3")]` (staging.rs:263,337 `cfg_attr(not(feature = "s3"), expect(unused_variables ...))`, 7 cfg lines in staging.rs, 3 in catalog/mod.rs).

**Arms to add or delete, with HEAD lines** (all deletions of `Holder::S3*` arms are the "no back-compat" sweep):

| file | lines | change |
| --- | --- | --- |
| holder/mod.rs | 106-114 | delete 3 variants, add `Registered(Box<dyn RegisteredHandle>)` |
| holder/mod.rs | 201-206 | `exists`: one arm `Self::Registered(inner) => inner.exists()` |
| holder/mod.rs | 240-241 | `into_byte_stream`: one arm `Self::Registered(h) => h.byte_stream(position, batch_size)` |
| holder/mod.rs | 518-532 | `from_handle`: one arm `Self::Registered(h) => h.held_again()?` |
| holder/mod.rs | 914-919, 952-957, 990-995, 1028-1033 | `as_io`, `as_io_mut`, `as_media`, `as_media_mut`: `Self::Registered(inner) => &**inner` / `&mut **inner` |
| holder/mod.rs | 384-422 | object-store arm becomes the backend lookup (section 6) |
| holder/mod.rs | 592-604 | `is_backend_property`: `backend_for(url.scheme()).is_some_and(...)` |
| holder/mod.rs | 1314-1465 | add `From<Box<dyn RegisteredHandle>>` |
| iceberg/catalog/mod.rs | 508-509, 527-530 | `folder_role`: 3 S3 arms -> one `Registered(h) => h.as_folder()` |
| iceberg/staging.rs | 266-270, 288-291, 308-309, 321-322, 340-341 | `upload`, `unpublished`, `leaf`, `container`, `sized`: one `Registered` arm each; **required**, the wildcard silently regresses (1.3) |
| warehouse/handle.rs | 45-52, 63-74, 112-113, 129-145, 156-157, 173-174; 95 | `Site::Store` names `crate::s3::S3Options`/`located_with`: becomes a backend-opened site (`Site::Opened { url, open: Arc<dyn Fn(&Properties) -> Result<Holder> + Send + Sync> }`, Eq/Hash by url as `Store` is) constructed by the iceberg crate's s3tables code; `Site::of` 95 uses the backend register |
| s3tables/catalog.rs | 642, 645, 1151 | `S3Options::is_property` and `Site::Store` construction go to the s3 crate (s3tables stays in iceberg, D15) |
| s3/{file,folder,path,mod}.rs | file.rs:1014; folder.rs:318,320,464,475,477; path.rs:560; mod.rs:160 | the 8 `Holder::S3*` constructions become `Holder::registered(...)`; they move with the folder |
| lib.rs | 131, 528-551 | `pub mod s3` and 12 internals re-exports leave (D6 `implementer` carries what the crate needs) |

Counts (grep commands in section 9): `Self::S3*` in holder/mod.rs 21 lines; `Holder::S3*` in rust/src 16 (iceberg 8, s3 8); python/src 23; node/src 1; cli/src 0; rust/tests 14 (holder/mod_.rs 7, iceberg/catalog/mod_.rs 2, s3/mod_.rs 2, iobase_calls.rs 1, warehouse/handle.rs 1, warehouse/media.rs 1); benchmarks 0 (they build `yggdryl::s3::file_with(...)` directly). `feature = "s3"` cfg lines in rust/src outside s3/: 79 (holder 26, lib 14, aws/session 11, iceberg/staging 7, warehouse/handle 6, auth/lease 4, aws/sigv4 3, iceberg/catalog 3, xml/scanner 2, auth/mod, aws/mod, http/mod 1 each).

Cost/pin notes: Registered boxes the S3 handle (one allocation per child; `hold()` makes one per listing entry, s3/folder.rs:313-321). No `allocations.rs` row names S3 (`git grep -n -i "s3" HEAD -- rust/tests/allocations.rs` only matches `variant_object`); `iobase_calls.rs:483-535` counts requests on the fake server (`Holder::S3File(file_with(...))` at :498), not wrapper calls. The `holder` bench has S3 rows (benchmarks/holder.rs:78-80) - direction only. A cost pin is not re-pinned from a sweep.

## 6. (b) Dispatch for `s3://`, `gs://`, `az://` and the other spellings

Today: section 3.1 step 5, all 10 spellings take the same arm; the store comes from `Provider::from_scheme` inside `s3::parse`. An outside crate cannot take it through `Locator` unchanged (section 4 gaps). Two ways to the same end:

A. Extend `Locator`: `schemes() -> &'static [Scheme]` (register keyed per scheme, all-or-none claim), an answer type that says "bytes" (`Located::Bytes(Holder)`, `Located::Object(Holder)`) so `from_url` applies `described` (445) to bytes only, `Locator::is_property(&self, name) -> bool`, and a public `Uri` name test for ARNs. Mixes two jobs (catalog objects "declare no media type", byte backends do) in one trait.

B. (recommended) A sibling register of the same `plugin::Register` in `holder/backend.rs`, asked **after** lowering (so ARN lowering, `is_local`, zip stay where they are) and before `is_http`:
```rust
pub trait StorageBackend: fmt::Debug + Send + Sync + 'static {
    fn name(&self) -> &'static str;                       // "s3"
    fn schemes(&self) -> &'static [Scheme];               // the ten
    fn is_property(&self, name: &str) -> bool;            // S3Options::is_property: the query refusal and is_backend_property
    fn holder(&self, url: &Url, properties: &[(String, String)]) -> Result<Holder>;   // step 5 whole: query read beneath properties, stripped, located_with
}
static BACKENDS: Register<Scheme, &'static dyn StorageBackend>   // claim_backend(b, by) all-or-none, backends(), backend_for(&Scheme)
```
`from_url` step 5 becomes `else if let Some(backend) = backend_for(url.scheme()) { backend.holder(url, &properties)? }` and still falls to `held.described(url, &properties)` (media_type/codec unchanged). A scheme no backend claims keeps the line-439 "install the crate that claims it" sentence; the "without the s3 feature" sentence (419) disappears. Registered in place first: the core seeds `S3_BACKEND` under the ten schemes with `by = CORE` exactly as locator.rs:47-60 seeds `S3TABLES_LOCATOR`; the leaving crate's `install()` claims it as `"yggdryl-s3"` (`claim` refuses `CORE` from outside, locator.rs:72-79). `Scheme::is_*`, `Url::bucket/key`, `uri/authority.rs` Azure/S3 host parsing and `Scheme::S3 ... WASBS` stay in the core as URI vocabulary (AGENTS: "`Scheme` owns URI and compatibility scheme vocabulary"); `Provider`, `S3Options`, `Credentials` (re-export of `aws::Credentials`) go with the crate.
Other dispatchers to redirect to `backend_for`: `Site::of` (handle.rs:95), python/src/iobase.rs:188 `folder_holder_for` and node/src/iobase.rs:129 (both build `yggdryl::s3::folder(...)` for "hold `url` as a container": a second per-scheme dispatcher in each binding; they need a role argument on the backend or a `/`-terminated `from_url`, since `folder()` is the S3Folder role and `from_url` answers S3Path).

## 7. (c) Bindings and CLI

**Python** (`grep -n -i "S3\|aws\|azure\|gcs\|google\|object.store"` over python/src: iobase.rs 52, holder/handles.rs 50, holder/fs.rs 23, uri.rs 16, field.rs 6, iceberg.rs 5, others 1 each):
- python/Cargo.toml:22-27 `yggdryl` features `http3, iceberg, s3, s3tables` -> adds the s3 crate.
- python/src/holder/handles.rs: `use yggdryl::s3::{Provider, S3Options}` :15; `role!` classes `PyS3File`/`PyS3Folder`/`PyS3Path` (Python names `S3File`, `S3Folder`, `S3Path`) :75-95; `s3_holder` :355-390 (location or container+key+`provider=`); `s3_options` :403-426 (`S3Options::is_property`, `PROPERTY_NAMES`, `from_properties`); constructors :429-521 call `yggdryl::s3::{located_with, file_with, folder_with, path_at_with, file_at_with, folder_at_with}` and wrap `Holder::S3Path/S3File/S3Folder` (:458,485,487,514,516); `add_class` :624-626.
- python/src/holder/fs.rs: `use yggdryl::s3::{self, Provider, S3Options}` :26; `native_redirect` :923-959 (PyArrow `type_name` `s3|gcs|abfs` -> native store), `store_role` :975-995 builds `Holder::S3Path/S3File/S3Folder` (:989-992), `stays_bridged` :1024-1051 (per `Provider`), `store_options` :1063-1076 (`S3Options::from_properties`).
- python/src/iobase.rs: `use yggdryl::s3::S3Options` :24, `S3TablesCatalog` :25; `cloned` :101-145 (S3 arms 115-133) with `store_sibling` :151-157; `located_holder` :172-175 and `from_uri` :1114-1139 (both just `Holder::from_url`, no S3 code); `folder_holder_for` :178-200 (`is_object_store` at 188); `container_holder` :217-232 (227-229); `native_path` :240-261 (`bucket()`/`prefix()`/`key()` 247-249); `Role` enum :278-302 (S3 286-288), `Role::of` :306-341 (315-317; **`_ => Self::Held` at 339 swallows a new variant and returns the base `IOBase` class, no compile error**), `name` :344-378, `describe` :387-419 (403-405); `is_location_property` :929-936 and `location_property_names` :939-945 (`S3Options::is_property` 933, `PROPERTY_NAMES` 941, `S3TablesCatalog` 935,943). The Python class for a registered handle is picked by `implementation_name()`, as media picks by codec name.
- Package/typing/inventory: python/yggdryl/_native.pyi:4258 `S3File`, :4271 `S3Folder`, :4284 `S3Path` (+ unions 3774,3787); python/yggdryl/holder/__init__.py:21-23,37-39; python/tests/typing_bindings.py:125,659; `.api-bindings.txt:40,141,146,147`; tests python/tests/holder/test_init.py (40 mentions), test_fs.py (19); docs/holder/index.md (51), skills/yggdryl-storage (48).
- Vocabulary only, unaffected: uri.rs:89,400,460,501,1323-1479 (ARN/Azure account/`is_storage`), field.rs:1461-1475 (`Scheme::S3` protocol views).

**Node**: one S3 site, node/src/iobase.rs:128-133 (`folder_holder_for`: `is_object_store()` -> `yggdryl::s3::folder(...).map(Holder::S3Folder)`); node/Cargo.toml:22-27 enables `s3`, `s3tables`; `git grep "S3File\|S3Folder\|S3Path" HEAD -- node/index.d.ts node/tests` is empty: Node carries no S3 class and no `S3Options` door, so per AGENTS §4 the change only keeps this one arm building (re-spell via the backend lookup) and adds nothing. Node's `rebuilt()` (iobase.rs:604-614, `local_holder(url)` at 612 = `Holder::from_url` under no properties) rebuilds an S3 handle from its URL with no S3 arm, dropping its options, unlike Python's `cloned`.

**CLI**: no S3 symbol in cli/src (`git grep -n "yggdryl::s3\|S3Options\|Provider\b\|is_object_store" HEAD -- cli` empty). Doors are `cli/src/location.rs:105-106` (`Holder::from_url(url, none)`), market.rs:413 and xmla.rs:186; help strings only (market.rs:110, xmla.rs:29). Build wiring: cli/Cargo.toml:41-53 (`s3 = ["yggdryl/s3"]` :52, `s3tables = ["yggdryl/s3tables", "iceberg"]` :53), scripts/stage_cli.py:131 (`--features iceberg,s3tables`), .github/workflows/ci.yml:219,235,344,874 (`--features s3`).

## 8. Risks and open points for the design

1. Wildcard arms (1.3): the registered variant must get explicit arms in `into_byte_stream`, staging `upload`/`unpublished`/`leaf`/`container`/`sized`, and Python `Role::of`; otherwise request counts (iceberg staging tests, `iobase_calls`), memory (whole-file read on upload) or the Python class silently change with the tree still green.
2. `from_url` returns locator answers before `described` (371-373): any route that sends s3 through the existing `Locator` loses `media_type`/`codec` (3.1 step 7). The contract is stated for every backend (holder/mod.rs:321-326) and pinned for local (rust/tests/holder/mod_.rs:467-471) and http (:824-839) but **not for an s3 location** (`git grep -n '"media_type"\|"codec"' HEAD -- rust/tests/s3 rust/tests/holder/mod_.rs` has no s3 hit), so the loss would be green; add the s3 row before the move.
3. `Site::Store` is the only core type constructing `S3Options` from a held `aws::Session`; it must not keep naming `crate::s3` after the move (D30 had parked it "until S6's implementer door constructs it from outside").
4. The query-strip/query-refusal contract (holder/mod.rs:384-414, tested rust/tests/holder/mod_.rs:686-730 including the `versionId` refusal naming the parameter) moves with the backend; `Holder::from_url` doc (321-334) states it.
5. `S3Options::is_property` is read by 5 places (holder/mod.rs:392,597; s3tables/catalog.rs:642,645; python iobase.rs:933, handles.rs:420, + `PROPERTY_NAMES` at iobase.rs:941, handles.rs:421); all become `backend.is_property` or a direct dependency on the s3 crate (Python and iceberg may depend on it directly; the core may not).
6. `aws/`, `auth/`, `http/retry`, `xml/scanner` are shared substrate with `s3tables` (79 cfg lines, section 5); out of this map.

## 9. Commands the counts come from (all `HEAD`)

```
git grep -n "Self::S3" HEAD -- rust/src/holder/mod.rs | wc -l                       # 21
git grep -c "Holder::S3" HEAD -- rust/src                                           # iceberg/catalog/mod.rs 3, iceberg/staging.rs 5, s3/file.rs 1, s3/folder.rs 5, s3/mod.rs 1, s3/path.rs 1
git grep -c "Holder::S3" HEAD -- python/src node/src cli/src                        # python 23 (fs.rs 3, handles.rs 5, iobase.rs 15), node 1, cli none
git grep -c "Holder::S3" HEAD -- rust/tests rust/benchmarks                         # 14 in six test files, 0 in benchmarks
git grep -n "Holder::\(Object\|Aws\|Gcs\|Azure\)" HEAD -- rust python node cli      # empty
git grep -n "crate::s3::" HEAD -- rust/src ':!rust/src/s3'                          # holder/mod.rs 108,111,114,392,404,414,597; s3tables/catalog.rs 642,645; warehouse/handle.rs 139,144; lib.rs internals
git grep -n "is_object_store()" HEAD -- rust/src python/src node/src cli/src        # holder 384,595; warehouse/handle.rs 95; python iobase.rs 188; node iobase.rs 129
git grep -c 'feature = "s3"' HEAD -- rust/src ':!rust/src/s3'                       # total 79
git grep -n "impl.*\(Clone\|PartialEq\|Eq\|Hash\|Display\|Default\|Drop\) for Holder" HEAD -- rust/src   # empty
git grep -n "owned_stream_bytes" HEAD -- rust/src                                    # trait iobase.rs:321; S3File file.rs:564, S3Path path.rs:370
git grep -n "pub trait RegisteredTable\|pub trait MediaWrapper\|pub trait Locator" HEAD -- rust/src   # table.rs:42, codec.rs:142, locator.rs:23
```
