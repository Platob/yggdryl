---
name: yggdryl-uri
description: Parse, canonicalize, inspect and edit resource identifiers with yggdryl - Uri, Url, Urn, Arn and Scheme - including platform file paths (Windows drives, UNC), percent escapes, credentials, S3/GCS/Azure bucket-key-region, filename suffixes and media type, query parameters, navigation, globs and Hive partitions. Use when parsing a URL or path, from_path / into_path / fromPath / intoPath, into_url / into_urn / intoUrl, joinpath, parameters, is_glob / glob_parts / matches_glob, hive_partitions / partitions / childrenWhere, a URN or ARN locator, bucket vs host rules, or url/urn columns. Covers Rust, Python and Node.js.
---

# URI: one canonical identifier

`yggdryl::uri` parses every spelling of a resource identifier - URI, URL,
URN, ARN, or a bare platform path - into **one canonical value**: lowercase
scheme, uppercase percent escapes, `/` for `\` under `file:`, an authority
marker on every absolute `file:` path. Two spellings of one resource compare
and hash equal, and the canonical text re-parses to the same value. `Uri` is
the general value; `Url` (a location), `Urn` (a name) and `Arn` (an AWS
resource name) are its three narrowings, and the scheme decides which. Parse
once at the boundary and carry the value: past it nothing splits a string on
`/`, `?` or `=`.

Opening what a URL names (a handle, its bytes, listing a folder) is
`yggdryl-storage`; reading a partitioned folder's rows is `yggdryl-records`.
Install and conventions are in `yggdryl`.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| parse any identifier | `Uri::from_str(s)?` | `Uri(s)` (answers `Url`/`Urn`/`Arn` subclass) | `Uri.from(s)`, `new Uri(s)` |
| strict location (stored data) | `Url::from_str(s)?` | `Url.from_str(s)` | `Url.fromString(s)` |
| location door (roots a bare path at cwd, resolves a URN/ARN) | `Url::from_location(s)?` | `Url(s)` | `Url.from(s)`, `new Url(s)` |
| from a platform path | `Uri::from_path(p)?`, `Url::from_path(p)?`, `Uri::try_from(PathBuf)?` | `Uri.from_path(p)`, `Url.from_path(p)` | `Uri.fromPath(p)`, `Url.fromPath(p)` |
| back to a platform path | `into_path()?` -> `PathBuf` | `into_path()`, `os.fspath(uri)` | `intoPath()` |
| narrow | `into_url()?`, `into_urn()?`, `into_arn()?`, `Url::from_uri(uri)?` | `into_url()`, `into_urn()`, `into_arn()`, `Url.from_uri(v)` | `intoUrl()`, `intoUrn()`, `intoArn()`, `Url.fromUri(v)` |
| widen | `Uri::from(&url)`, `into_uri()` | `Uri(url)`, `into_uri()` | `Uri.from(url)`, `intoUri()` |
| from parts | `Uri::from_parts(scheme, authority, path, query, fragment)?` | `Uri.from_parts(scheme, authority, path, query=None, fragment=None)` | n/a |
| components | `scheme()`, `authority()`, `path()`, `query(decode)?`, `fragment(decode)?`, `path_text(decode)?` | `scheme`, `authority`, `path`, `query(decode=False)`, `fragment()`, `path_text()` | `scheme`, `authority`, `path`, `query`, `fragment` (stored form only) |
| credentials | `user()`, `password()`, `hostname()` | `user`, `password`, `hostname`, `port`, `host_port` | `user`, `password`, `hostname` |
| object-store location | `bucket()`, `key()`, `region()`, `account()`, `store_endpoint()`, `is_virtual_hosted()` | same names as properties, `is_virtual_hosted()` | `bucket`, `key`, `region`, `account`, `storeEndpoint`, `isVirtualHosted()` |
| filename | `file_name()`, `stem()`, `extension()`, `extensions()` | `file_name`, `stem`, `extension`, `extensions`; `Url`: `name`, `suffix`, `suffixes` | `fileName`, `stem`, `extension`, `extensions`; `Url`: `name`, `suffix`, `suffixes` |
| rename in place | `set_file_name`, `set_stem`, `set_extension`, `set_extensions`, `remove_extension`, `clear_extensions` | same | `setFileName`, `setStem`, `setExtension`, `setExtensions`, `removeExtension`, `clearExtensions` |
| renamed copy | n/a | `Url.with_name`, `with_stem`, `with_suffix` | `Url.withName`, `withStem`, `withSuffix` |
| media type from suffixes | `mime_type()`, `media_type()`, `set_mime_type(m)?`, `set_media_type(m)?` | `mime_type`, `media_type`, `set_mime_type`, `set_media_type` | `mimeType`, `mediaType`, `setMimeType`, `setMediaType` |
| segments | `path_segments()`, `path().segment_len()`, `for s in &uri` | `path_segments`, `len(uri)`, `uri[i]`, `in` | `pathSegments`, `length`, `at(i)`, `[...uri]` |
| join, climb | `joinpath("../d")?`, `parts()`, `parent()`, `parents()` | `joinpath(*others)`, `/`, `parts`, `parent`, `parents` | `Url.joinpath(...others)`, `Uri.joinPath(...others)`, `Url.parts`, `parent`, `parents` |
| join an OS path (encodes names) | `Url::join_path(path)?` | `joinpath(os.PathLike)` | n/a |
| relative to a root | `segments_under(&root)` | `relative_to(root)`, `is_relative_to(root)` | `relativeTo(root)`, `isRelativeTo(root)` |
| query pairs | `parameters(decode)?` -> `into_owned()` -> `set_parameters(&p)?` | `parameters(decode=False)` (live, dict-like) | n/a: read `query` |
| replace the query | `set_query(Some("a=1"))?` | `set_query("a=1")` | n/a |
| glob detect / split | `is_glob()`, `is_recursive_glob()`, `glob_parts()?`, `Url::is_pattern(t)` | `is_glob()`, `is_recursive_glob()`, `glob_parts()`, `Url.is_pattern(t)` | `isGlob()` |
| glob match (`.gitignore` rule) | `matches_glob(p)`, `matches_glob_under(&root, p)` | `match(p)`, `full_match(p)`, `full_match_under(root, p)` | `match(p)`, `fullMatch(p)` |
| Hive partitions | `hive_partitions()`, `hive_partition(c)`, `hive_partitions_under(&root)`, `is_hive_partitioned()`, `with_hive_partition(c, v)?` | `partitions`, `partition(c)`, `partitions_under(root)`, `is_partitioned()`, `with_partition(c, v)` | `partitions` (`{column, value}[]`), `partition(c)` |
| leaves by partition (on a handle) | `children_where(&[("year", "2024")], false)?` | `IOBase(root).children_where({"year": "2024"})` | `new IOBase(root).childrenWhere({ year: '2024' })` |
| private (dot) name | `is_private()` | `is_private()` | `isPrivate()` |
| local predicates (no network) | `is_local()`, `exists()`, `is_dir()`, `is_file()`, `local_mime_type()` | same | `exists()`, `isDir()`, `isFile()` |
| URN | `namespace()`, `namespace_specific()`, `locator_path()?`, `resolve(&base)?`, `locator()?` | `namespace`, `namespace_specific`, `locator_path()`, `resolve(base)`, `locator()` | `namespace`, `namespaceSpecific`, `locatorPath()`, `resolve(base)`, `locator()` |
| ARN | `partition()`, `service()`, `region()`, `account()`, `resource()`, `resource_type()`, `resource_id()`, `bucket()`, `key()`, `table()`, `locator()?`, `Arn::from_parts(..)?` | same names as properties, `locator()`, `Arn.from_parts(..)` | `partition`, `service`, ..., `resourceType`, `resourceId`, `table`, `locator()`, `Arn.fromParts(..)` |
| scheme facts | `scheme()` -> `&Scheme`: `Scheme::S3`, `is_object_store()`, `has_container()`, `default_port()` | `scheme` (str), `default_port`, `is_storage()` | `scheme` (string) |
| equality, stable hash | `==`, `Ord`, `stable_hash()` | `==`, `<`, `hash()` (locks), `stable_hash()` | `equals()`, `compare()`, `stableHash()` |
| structural JSON | `into_json()?`, `Uri::from_json(s)?` | `into_json()`, `Uri.from_json(s)` | `toJSON()`, `Uri.fromJSON(v)` |
| a column of identifiers | `DataType::url()`, `DataType::urn()`, `Field::new(n, DataType::url(), false)` | `yggdryl.url(name, nullable=False)`, `DataType("url")` | `fields.url(name, { nullable: false })`, `new DataType('url')` |

## Rules for fast, correct use

1. **Parse once, carry the value.** Equality, hashing and ordering run on the
   canonical form, so `HTTPS://x/caf%c3%a9` equals `https://x/caf%C3%A9`.
   Never compare or cache raw strings, and never re-parse a value you hold.
2. **Pick the door by where the text came from.** Stored data takes the strict
   parse (`Url::from_str`, `Url.from_str`, `Url.fromString`): relative text is
   refused. A user argument takes the location door (`Url::from_location`,
   Python `Url(...)`, JS `Url.from`), which roots a bare path at the working
   directory and resolves a URN or an S3 ARN to where it points.
3. **No scheme token means a file path.** `/var/x` is `file:///var/x`,
   `data/x.csv` the relative `file:data/x.csv`; a colon after the first
   separator is data (`/d/2026-08-16T00:00:00/p`), and an invalid token before
   a colon is a parse error with its byte offset, never a silent fallback.
4. **Cross the OS boundary with `from_path`/`into_path`.** Drive and UNC
   detection is textual, identical on every host; `from_path` encodes names,
   `into_path` decodes them and refuses an escape that would become a
   separator, a dot segment, a drive or a UNC name. Never concatenate.
5. **Escapes are stored as written.** `path`, `query`, `fragment` answer the
   stored bytes; ask `decode=true` for text. Decoded text is never structure:
   `%2F` stays inside its segment, `%26` inside its pair, `+` is a literal plus.
6. **`joinpath` takes URI text; `join_path` takes an OS path.** `joinpath`
   resolves `..` like a shell `cd` and refuses `100%.csv`; Rust `join_path` /
   Python `joinpath(os.PathLike)` encode each component as a name. An absolute
   argument replaces the path; `..` past an absolute root is clamped.
7. **Suffixes are the media type.** The last suffix is the MIME type, the
   chain is base + codings (`x.csv.gz.zst` = CSV, gzip, zstd). An unknown
   suffix is `application/octet-stream`, not an error. Setters are atomic: a
   refused name (`bad/name`, a MIME type with no preferred extension) changes
   nothing.
8. **Store authorities follow one rule.** On `s3:`/`gs:`/`az:` families, a
   first component ending `.com`/`.io`, carrying a port, an IP literal, or
   `localhost` is the host and the bucket is the next part; otherwise it is the
   bucket. `key()` is the path below the bucket as spelled, trailing slash and
   escapes kept. All of it reads the authority - no network request.
9. **User info splits at the first colon.** `user:pass:word@host` has password
   `pass:word`. Never log a URL that carries one.
10. **Globs live in the path, spelled with `*`.** `?` opens a query and `[` is
    an IPv6 host in a URL, so neither is a glob there; the full syntax
    (`?`, `[a-z]`, `[!a-z]`) is for `matches_glob` pattern text. A pattern with
    no `/` matches the name at any depth; with a `/` it is anchored at the root.
11. **List from the glob's fixed root.** `glob_parts()` answers the deepest
    location with no pattern character and the rest; a handle's `glob` descends
    that prefix instead of listing everything and filtering.
12. **Hive partitions are path segments.** `column=value` directories, in path
    order. Use `hive_partitions_under(root)` / `partitions_under(root)` for the
    table's own columns - `year` is a partition of `/lake`, not of
    `/lake/year=2024`. `children_where` on a handle selects leaves by path with
    no call per file.
13. **A name resolves by reading it as a path.** A URN's namespace then its
    `:`-separated parts are the path (`urn:lake:t:2026:p.parquet` ->
    `lake/t/2026/p.parquet`); `resolve(base)` joins it under a store,
    `locator()` under the working directory. An S3 bucket ARN locates its
    `s3:` URL; every other ARN service is refused by name.
14. **Local predicates never touch the network.** `exists`, `is_dir`, `is_file`
    answer for `file:` URLs and are `false` for every other scheme - they are
    not a remote existence check (and storage code should act, not probe).
15. **Python wrappers lock on first `hash`.** After `hash(uri)` (a dict key,
    a set member) a setter raises `TypeError`; `copy.copy` gives an unlocked
    copy. `stable_hash()` never locks.

## Pitfalls

| Wrong | Right |
| --- | --- |
| `url.split("/")`, `path.rsplit(".", 1)` | `path_segments`, `file_name`, `stem`, `extensions` |
| `Url.from_str("data/x.csv")` for a CLI argument | `Url("data/x.csv")` / `Url::from_location` (roots at cwd) |
| `Url("data/x.csv")` stored in a table | store the strict, absolute form; a relative location names a different file per machine |
| `f"file://{path}"` | `Url.from_path(path)` (encodes spaces, `%`, drives, UNC) |
| `str(uri) == other_text` | `Uri(other_text) == uri` |
| Python `uri.query` (property) | `uri.query()` is a method; JS `uri.query` is a getter; Rust `query(false)?` answers `Result<Option<Cow<str>>>` |
| JS `Uri.from("urn:isbn:1").namespace` | JS `Uri.from` answers a `Uri`; narrow with `intoUrn()` (Python's `Uri(...)` already answers `Urn`) |
| `Url("file:///lake/part-?.parquet").is_glob()` | `?` starts the query in a URL; spell URL globs with `*`, test names with `matches_glob("part-?.parquet")` |
| listing a lake then filtering names in a loop | `handle.glob("year=2024/**/*.parquet")` or `children_where({"year": "2024"})` |
| `s3://my.bucket.com/key` meaning bucket `my.bucket.com` | the `.com` part is a host; build the handle from bucket + key (`S3File(bucket, key, provider="s3")`, `s3::file_at`) |
| `url.joinpath("100%.csv")` | `joinpath` takes URI text: pass `"100%25.csv"`, or an OS path via `join_path` / `joinpath(PurePath(...))` |
| JS `url.parameters()` | not bound in JavaScript; read `url.query`, or build the query text yourself |
| `Url("https://h/x").exists()` to probe a remote object | local only; open a handle and read (absence reads as empty) |
| expecting `urn:isbn:` or `urn:a::b` to locate | `Urn` refuses an empty namespace-specific string; an empty `:` part makes `locator_path` refuse, since two names would spell one path |
| setting a MIME type with no preferred extension | refused and the value is unchanged; use a registered type or `set_extension` |
| parsing an `s3tables://` location to open bytes | it is a container, not an object store: `is_object_store()` is false and no byte backend opens it |

## Language references

- `references/rust.md` - read when writing Rust (`Uri`, `Url`, `Urn`, `Arn`, `UriPath`, `Scheme`, `Parameters`).
- `references/python.md` - read when writing Python (`yggdryl.Uri` subclasses, `pathlib`-style `Url`, live `Parameters`).
- `references/javascript.md` - read when writing Node.js (camelCase getters, `Url.from` vs `Url.fromString`, no `Parameters`).

## Deeper

- URI contract, canonical form, credentials, store locations, columns: https://platob.github.io/yggdryl/uri/
- Path: segments, filenames, media type, platform paths, navigation: https://platob.github.io/yggdryl/uri/path/
- URL and URN, name resolution, scheme facts: https://platob.github.io/yggdryl/uri/url-urn/
- ARN and Amazon S3 Tables: https://platob.github.io/yggdryl/uri/arn/
- Query parameters: https://platob.github.io/yggdryl/uri/parameters/
- Globs and Hive partitions: https://platob.github.io/yggdryl/uri/patterns/
- Listings, globs and partition pruning on a handle: https://platob.github.io/yggdryl/holder/#partitions
- Sibling skills: `yggdryl-storage` (open what a URL names), `yggdryl-records`
  (partitioned reads and writes), `yggdryl-types` (`url`/`urn` columns in a schema).
