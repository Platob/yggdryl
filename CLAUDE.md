# yggdryl — contributor & agent instructions

**Keep all new code uniform with the existing patterns.** Before adding anything,
read the nearest existing example and mirror its structure, naming, error
handling, and doc style. Consistency across the Rust core and the two bindings is
the top priority — a reader should not be able to tell which type they are
looking at from the shape of the code.

## Architecture

The workspace is **two crates**: `yggdryl-core` (all the data types + byte IO +
compression) and `yggdryl-http` (the network client). `yggdryl-core` is **one file
per type** — each concern is a module (or module directory) under
`crates/yggdryl-core/src/`, with `lib.rs` as glue (a shared `log_event!` macro,
`mod` declarations, and `pub use` re-exports of every type at the crate root, so
`yggdryl_core::Io` / `::Url` / `::Compression` / … all resolve). Each module owns
its concern wholly — do not scatter a concern's logic across modules:

- `encoding.rs` / `mapping.rs` — dependency-free foundations: the `Mapping` /
  `Params` component maps and percent-encoding. Each value type pairs its own
  inherent `from_str` / `from_mapping` parsers with inherent `to_str` /
  `to_mapping` renderers (no shared rendering trait — keep them per-type).
- `version.rs` — the standalone `Version` type.
- `media/` (`mod` + `mime.rs` + `media_type.rs`) — the `MimeType` enum (single MIME
  types, backed by a mutable global registry of extensions/magic bytes) and the
  `MediaType` stack (an ordered `Vec<MimeType>`, e.g. `csv.gz` → `[Csv, Gzip]`).
  **All media-type logic lives here.**
- `url/` (`mod` + `uri.rs` + `url.rs`) — the `Uri`/`Url` types and the canonical URL
  tests, built on `encoding`/`mapping` (and `media` for the inferred `media_type()`
  accessor). **All URL logic lives here.**
- `io/` (`mod` + `bytesio.rs` + `localpath.rs` + `codec.rs`) — the **byte-IO
  foundation**: one set of methods to read, write, seek and stat bytes wherever they
  live (memory, local path, cloud). See its dedicated section below. **All byte-IO
  logic lives here.**
- `compression/` (`mod` + `codec.rs`) — the `Compression` codec (gzip / Zstandard /
  Snappy / `None` identity) that **streams** compress/decompress over any `Io`
  handle, plus the `CompressIo` extension trait. **All compression logic lives
  here** — do not pull codec SDKs into the `io` module.
- `crates/yggdryl-http/` — a blocking, `requests`-like HTTP client
  (`HttpSession` / `HttpRequest` / `HttpResponse`) whose bodies **stream over the
  `yggdryl-core` `Io` abstraction**. **All HTTP logic lives here** — the transport
  SDK (`ureq`) is a dependency of this crate only. Already split one-file-per-type
  under `crates/yggdryl-http/src/`. See its section below.
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**. They only translate types/errors and call the core; they contain no
  logic. Anything added to the core must be surfaced in *both* bindings.

### The `io` module — what it aims to be (read before extending it)

The goal is a **single byte-IO abstraction** that hides *where* bytes live, so a
reader (think Arrow / Parquet) works the same over an in-memory buffer, a
memory-mapped local file, or a cloud object — mixing **random** access (read a
footer, a column chunk) with **streamed** access (scan record batches) on one
handle. The layering, smallest to largest:

- `Io` — **the one byte-IO trait**; there are no separate `ReadBytes` / `WriteBytes`
  / `Seek` traits (they were folded in). Every IO has a `url()` (in-memory ones use
  `mem://<address>`). It carries a **cursor** (`seek` / `stream_position` /
  `stream_len`), does **streamed** access with `read` / `write` (advance the cursor;
  `read` is the source primitive a memory backend gets free from `as_slice`, `write`
  defaults to `Unsupported`), and **random** access with `pread` / `pwrite` — a
  `Whence` selects positional (`Start`/`End`, cursor untouched) versus cursor-relative
  (`Current`, the same as `read`/`write`, advancing the cursor); the default `pread`
  serves zero-copy from `as_slice` or, on a seekable streamed backend, seeks-reads-
  restores, while `pwrite` defaults to `Unsupported`. `read_exact` / `read_to_end` /
  `write_all` / `flush` are provided on top. Storage is managed with `capacity` /
  `reserve_capacity` / `truncate` (`Unsupported` on read-only backends; `BytesIO`
  adds the `with_capacity` constructor). Each handle carries an access `mode()`
  (`Mode` — `Read`/`Write`/`Append`/`ReadWrite`, parsed from Python strings via
  `Mode::from_str`) and an optional `parent()`, and can `open()` a derived handle
  (records the parent, applies mode/stream) and `close()` it (idempotent; the default
  is a no-op as memory/mmap backends free their storage on drop). Plus `as_slice`
  (the zero-copy hook a memory backend overrides), `stats`, and `copy_to` (transfer
  into another `Io` with a memory fast path; `copy` is the free fn). `media_type` is
  lazy and behind the `media` feature. (`Io: Debug + Send + Sync` so handles can be
  boxed as parents and held across threads; a blanket `impl Io for &mut T` lets a
  borrowed handle be passed by value to an adapter.) A **streamed** backend (an HTTP
  body, a compression `Decoder`) overrides `read` (and `pread` if it supports
  positioning); a **memory-resident** one overrides `as_slice` so the zero-copy paths
  light up for free.
- `IoStats` — cheap metadata eager (`size`/`mtime`/`content_type`/`etag`),
  expensive metadata (`media_type`) discovered lazily and cached.
- `Path: Io` — a local, hierarchical resource. `LocalPath` is a filesystem
  **instance**: `open` is infallible — it stats the path up front (holding
  `url`/`stats`; a missing path reports `Kind::Missing`) and memory-maps the file
  **lazily** on first read (mmap via the `mmap` feature). Its instance `write`
  **auto-creates missing parent dirs lazily** — attempt the write, create the
  tree only on a `NotFound` failure, then retry; never stat the dir up front.
- `RemotePath: Io` — the URL-addressed cloud sibling of `Path` (flat keys, no dir
  creation; range reads via `pread`). The address is the universal `Io::url()`.
  **Cloud backends (S3, Azure) are downstream crates that implement `RemotePath`
  — do not pull network SDKs into the `io` module.**
- `Codec<T>` — typed read/write/stream of values over any `&mut dyn Io` handle;
  `Frames` is the reference length-delimited codec. (`Codec` is the *value* coder;
  `Io` is the *byte* handle — keep them distinct.) Byte-stream **compression** is a
  separate concern in the `compression` module (see its section), not here.
- The **factory** `from_str` / `from_url` / `from_uri` returns the right
  `Box<dyn Io>` for a location, dispatching on the URL scheme: a bare path / `file://`
  opens a `LocalPath`; any other scheme is looked up in the `register_scheme` registry
  (a global `OnceLock<RwLock<…>>`, like the MimeType registry) so downstream crates
  plug in without the `io` module depending on them — `yggdryl-http` registers
  `http`/`https` (lazily, on first `HttpSession::new`), cloud stores their schemes
  later. `mem://` and unregistered schemes return an actionable `Unsupported`.

Rules when extending: the `io` module builds on the `url` / `encoding` modules (for
the universal `Io::url()`); new heavy deps are **optional features** (like `log` /
`mmap` / `media` / `serde`). A new memory-resident backend must override `as_slice`
so the zero-copy `pread` / `copy_to` paths light up; positional reads go through
`pread` with `Whence::Start`, never by mutating the cursor.

### Serialization — a cross-cutting optional concern

Every value type is **serializable**, but the mechanism is idiomatic per language
(adapt to each, keep the semantics identical). In Rust it is the off-by-default
`serde` feature: value types with a canonical string render to **that string**
(`Version` → `"1.4.2"`, `Url` → `"https://…"`, `MimeType` → `"image/png"`,
`Compression` → `"gzip"`), `MediaType` to a **sequence of MIME strings**
(lossless), and the plain enums/structs (`Mode` / `Whence` / `Kind` / `IoStats` /
`Signature`) `derive`. The bindings surface the same: **Python** implements
`__reduce__` (so `pickle` / `copy` reconstruct through the existing constructors),
**Node** implements `toJSON()` + a static `fromJSON()` (used by `JSON.stringify`).
Live/stream resources (`Io` handles, an HTTP body, `HttpSession`) are **not**
serialised. When you add a type, add its serde impl and replicate the pickle /
`toJSON` surface in both bindings.

### The `compression` module — streamed codecs over `Io`

Compression is layered **on top of** the `io` module, never inside it (so the IO
base stays codec-free and the dependency points one way — `compression` builds on
`io`, never the reverse). The shape:

- `Compression` — `None` / `Gzip` / `Zstd` / `Snappy`; `from_str` /
  `from_extension` / `as_str` / `extension` / `is_available`, and (under `media`)
  `from_mime` / `from_media` / `from_stats` for inference.
- `encoder(sink: impl Io) → Encoder: Io` (write-only, compress-on-write; `finish()`
  flushes the trailer and recovers the sink) and `decoder(source: impl Io) → Decoder:
  Io` (read-only, decompress-on-read); both are **streamed `Io` handles** themselves,
  so a decoder composes straight into an HTTP body. The one-shot `compress` /
  `decompress` build on them over a `BytesIO`. Internal `std::io` shims bridge `Io`'s
  `read`/`write` to the `flate2`/`zstd`/`snap` stream codecs.
- `CompressIo: Io` — a blanket extension trait adding `compress(codec)` /
  `decompress(codec)` to every handle, returning a fresh `BytesIO`. `decompress`
  with no codec infers one from the handle's URL extension, then its `stats()`
  media/content type.

Each backend is an **optional feature** (`gzip`/`zstd`/`snappy`); a variant whose
feature is off still parses and names itself but reports `Unsupported` on
encode/decode (`is_available` tells ahead of time). `media` adds the
stats-inference path. When you add a codec, surface it in *both* bindings.

### `yggdryl-http` — a requests-like client streaming over `Io`

A small **blocking** HTTP client shaped after Python's `requests`, layered on
`yggdryl-core` (its `io` / `url`); the transport is `ureq` (rustls TLS, its own
gzip/brotli left off so decompression goes through `yggdryl-core`'s `compression`). The
shape:

- `HttpSession` — like `requests.Session`: a pooled `ureq::Agent` (an idle-connection
  pool, sized by `with_pool_size`, so reused keep-alive connections skip the TLS
  handshake), default headers, a `RetryConfig`, `max_concurrency` (8) and `batch_size`
  (80). **Every request funnels through the one method** `send(req, raise_error,
  keep_alive, stream)` — there is no separate `stream()` method. It `prepare`s the
  request (merge defaults; per-request headers win, case-insensitively), runs it with
  the retry policy, and returns an `HttpResponse` holding the body. `raise_error`
  (`true` on the verb helpers `get`/`post`/…) raises on a 4xx/5xx; `keep_alive` pools
  the connection (a pool-saturation safeguard forces `Connection: close` on streams
  past the pool size); `stream` (`true` by default) keeps the body a **live
  `HttpStream`** read lazily, while `false` drains it into a `BytesIO` during `send`
  so the connection is released at once. `request(req, raise_error)` is the
  keep-alive, streamed shorthand. `send_many(reqs)` is a lazy iterator of
  `HttpResponseBatch`, running each batch up to `max_concurrency` at a time (scoped
  threads). `send` also drives the **redirect** loop (`with_max_redirects`, default
  10) and an RFC 6265 **cookie jar** (`cookies()` / `set_cookie`). An optional
  **`base_url`** (`with_base_url`) prefixes requests: the verb helpers run their
  target through `resolve_url`, so a relative reference (`/path`, `name`) joins onto
  the base (same RFC 3986 rules as a `Location` redirect) while an absolute URL is
  used unchanged. A process-wide **shared singleton** `HttpSession::shared()` (a
  replaceable `Arc` behind an `RwLock`, swapped by `set_shared`) backs the crate-level
  `get`/`head`/`post`/`put`/`patch`/`delete`/`request` **module functions**, the
  `requests.get(...)` equivalent. The bindings mirror this with module-level verbs
  over the shared session and a `set_base_url` to configure it (Node has no `delete`
  verb — a JS reserved word — so use `request('DELETE', …)`).
- `HttpCookies` / `Cookie` — the dependency-free cookie jar: parses `Set-Cookie`
  (`Domain`/`Path`/`Secure`/`HttpOnly`/`Max-Age`/`Expires`), matches per RFC 6265
  (domain §5.1.3, path §5.1.4, `Secure` ⇒ https), and `header_for(url)` emits the
  `Cookie:` value. **All cookie logic lives here.** The session feeds every
  response's `Set-Cookie` in and adds the matching `Cookie` before each dispatch.
- **Redirects** (`redirect.rs`, crate-internal): `send` follows 3xx with a `Location`
  when the request's `allow_redirect` (default `true`) is set and the hop is under
  `max_redirects`. 303 → GET (drop body); 301/302 on POST → GET; 307/308 preserve
  method **and** body but only if replayable, else the 3xx is returned. Loops are
  detected (a `(method, url)` set); a cross-origin hop (scheme+host+**port** differ)
  strips `Authorization` and per-request `Cookie`. `ureq`'s own redirect following is
  off — our layer owns it.
- `HttpHeaders` — the case-insensitive header map all three of `HttpSession`,
  `HttpRequest`, `HttpResponse` and `HttpStream` use; CRUD (`get` / `get_all` /
  `set` / `insert` / `remove` / `contains` / `iter` / `from_mapping`) plus the
  HTTP-typed reads (`retry_after`, `content_size` = `Content-Range` total else
  `Content-Length`). **All header logic lives here.**
- `HttpRequest` — a `Method` + `Url` + `HttpHeaders` + body builder (`with_header` /
  `with_param` / `with_basic_auth` / `with_bearer_auth` / `with_body` /
  `with_body_reader` / `with_body_io` / `with_allow_redirect`). `with_body_io` is the
  preferred upload: the handle's `stream_len` sets `Content-Length` and the bytes
  stream straight off the `Io` (a file is never buffered). `with_allow_redirect(false)`
  opts a request out of the redirect loop (returning the 3xx).
- **Authentication** (`auth.rs`, crate-internal) — `with_basic_auth(user, pass)` and
  `with_bearer_auth(token)` on both `HttpRequest` and `HttpSession` set the
  `Authorization` header (HTTP Basic, RFC 7617, with a dependency-free base64 encoder;
  Bearer, RFC 6750). Session-level auth is a default header, so a per-request value
  overrides it and a cross-origin redirect strips it. The bindings surface it as the
  session `basic_auth`/`bearer_auth` (Python kwargs) / `basicAuth`/`bearerAuth` (Node
  options).
- `HttpResponse` — `status`/`ok`/`raise_for_status`/`headers`/`header`. It **holds the
  body** as a `Box<dyn Io>` (an `HttpStream` when streamed, a `BytesIO` when buffered):
  `reader()` is the decoded body `Io` (decompressed under `compression`),
  `bytes`/`text`/`into_bytesio` drain it, `into_io` takes the whole body.
- `HttpStream: Io` — the seekable HTTP body that **streams off the held connection**:
  sequential `read` pulls bytes straight off the socket on demand, keeping only a
  sliding 4 MiB cache for short seek-backs, while `pread` (footer / column-chunk) and
  a seek-back past the cache re-open a one-off `Range` on a pooled connection. Reads
  retry transient statuses (429/502/503/504, honouring `Retry-After`) and **resume
  from the cursor** on a dropped connection (each range request is idempotent); the
  connection is released on EOF or `close()`. This is the canonical "remote `Io`".
- Retries cover replayable bodies (none/bytes) and all `HttpStream` range
  fetches; a streamed (reader/`Io`) request body is single-shot.

- **Protocol version** (`HttpVersion`, `version.rs`) — the HTTP version is tunable:
  pin one per session (`with_http_version`) or per request, default `Auto`. The
  blocking `ureq` transport always covers **HTTP/1.1**; the optional `http2` feature
  adds an async **HTTP/2** transport in `transport.rs` (hyper over a small
  multi-threaded tokio runtime + tokio-rustls) and the optional `http3` feature an
  async **HTTP/3-over-QUIC** transport (quinn + h3), both routed from `dispatch`
  when a request negotiates that protocol (`https` ALPN — h2c for cleartext h2; h3
  is TLS-only) — the response then re-joins the same redirect/cookie/retry loop, so
  every verb works unchanged whatever the protocol. `HttpResponse::negotiated_version()`
  reports what was actually spoken; `Auto` over TLS does a real ALPN `h2`/`http/1.1`
  fallback. A pinned version whose transport feature is off errors with
  `HttpError::Unsupported` rather than downgrading silently. **The async SDKs
  (`hyper` for h2, `quinn`/`h3` for h3, sharing `tokio`/`tokio-rustls`) are
  dependencies of the `http2`/`http3` features only**; `transport.rs` is
  `#[cfg(any(feature = "http2", feature = "http3"))]`, so the default build stays the
  lean blocking `ureq` client. TLS verification follows the session's `verify` flag
  (a rustls `NoVerify` certifier when off); `with_ca_cert` / `with_ca_cert_file`
  installs custom CA certificates (PEM/DER) that **replace** the default trust store
  across all transports (the secure alternative to `verify=false`, like `requests`'
  `verify=<bundle>`); a proxy applies to the `ureq` h1 path.

Optional features: `compression` (auto `Content-Encoding` decode — it also turns
on the codec backends), `media` (`mime_type()`), `serde` (`Serialize`/`Deserialize`
for `Method` / `HttpVersion` / `HttpHeaders` / `Cookie` / `HttpCookies` /
`RetryConfig`, and transitively the core value types — a live request/response body
is deliberately not serialisable), `http2` / `http3` (the async HTTP/2 and
HTTP/3-over-QUIC transports above), `log`. The base depends on
`yggdryl-core`'s `json` feature so `Io::json()` is available on every handle. **All
HTTP logic lives here; `ureq` stays a dependency of this crate only** (the HTTP/2
SDKs are gated behind `http2`). Unit tests are **hermetic** (a localhost
`TcpListener` that serves HEAD / `Range` / 429 / mid-stream drops; the h2c case runs
a localhost hyper HTTP/2 server, and the h3 case a localhost quinn + h3 QUIC server
with an `rcgen` self-signed cert over UDP loopback — still no network). A separate, `#[ignore]`d
`tests/integration.rs` covers the versions and ALPN fallback against real public
endpoints, opt-in via `--ignored` (needs direct egress; not run in CI). In the bindings the blocking call must not stall
the host runtime: Python releases the GIL (`allow_threads`), Node runs the
request on the libuv pool and returns a `Promise` (so Node's surface is async,
the one idiomatic divergence from the sync Rust/Python API). Bindings pass our
`Io` instances as bodies — never serialized `bytes` — per the Io-centralisation
rule above.

### One module per type, everywhere

Code is organised the same way in every language: **one file per type**, with a
small glue file tying them together. Don't grow a single big file.

- Rust: one module per concern in `yggdryl-core` (`version`, `media`, `url`,
  `io`, `compression`), plus the separate `yggdryl-http` crate.
- Each binding: `src/uri.rs`, `src/url.rs`, `src/version.rs`, `src/mime.rs`,
  `src/media.rs` per type, with
  `src/lib.rs` holding only shared helpers (error conversion, `hash_str`,
  percent-encoding free functions) and the module registration. Per-type
  wrappers keep their `inner` field `pub(crate)` so sibling modules can convert.

### Cross-language replication rule

The Rust core is the source of truth, but the languages move together. **When you
add or change behaviour in Rust, immediately replicate it in the Python and Node
extensions; when you change an extension, fold the behaviour back into the Rust
core and the other extension.** Adapt to each language's idioms (Python dunders /
keyword defaults, JS camelCase / `Option<bool>` defaults) but keep the surface
and semantics identical, so the three codebases stay coherent and a change is
never half-applied.

## Naming conventions (cross-language)

These names are identical in Rust, Python and JS (JS uses camelCase):

| Concept | Name |
| --- | --- |
| Construct from a string | `from_str(value)` |
| Construct from a component mapping | `from_mapping(fields)` |
| Construct from any supported input | `from_` (Rust trait `FromInput`) |
| Construct from explicit parts | `from_parts(...)` |
| Independent / overriding copy | `copy(...)` — every field optional, omitted fields come from `self` |
| Single-field functional update | `with_<field>(value)` returns a new value |
| Clear an optional field | `without_<field>()` |
| Read query parameters | `params(decode=true)` → `map<str, list<str>>` |
| Replace the whole query | `with_params(map, encode=true)` |
| Add/replace one parameter | `add_param(key, values, encode=true)` |
| Query-param CRUD | `get_param` / `set_param` / `set_params` (bulk) / `remove_param` / `remove_params` (bulk) / `clear_params` |
| Scheme split (`https+zip`) | `scheme_base()` / `scheme_ext()` |
| Join a path reference | `join(reference)` on `Uri`/`Url` — RFC 3986 §5.2.4 dot-segment resolution (`./`, `../`, leading-`/` replace); `reference` is a path string (verbatim), a segment sequence (`["a","b"]`, each percent-encoded), or another `Uri`/`Url` (via the `JoinInput` trait); non-mutating, drops query/fragment |
| Type conversions | `to_uri` / `from_uri` / `to_url` / `from_url` |
| Single MIME type | `MimeType` enum; `from_str` (a full MIME *or* a short name like `json`/`zstd`) / `from_mapping` / `from_parts(type, subtype)` / `from_extension(ext)` / `from_magic(bytes)` / `from_path(path)`; `.mime` / `type` / `subtype` / `extension(s)` |
| Global MIME registry | `MimeType.register(mime, extensions, magic)` / `unregister(mime)` / `reset_registry()` |
| Layered media type (extension stack) | `MediaType` = ordered `[MimeType, …]`; `from_str` / `from_mapping` / `from_extension` / `from_extensions` / `from_path`; `.types` / `first` / `last` |
| Inferred media/MIME type on a URI/URL | `media_type()` → `MediaType` stack or null; `mime_type()` → outermost `MimeType` or null (Rust also has `MediaType::from(&uri)`) |
| Octet-stream fallback | `MimeType.default()` = `application/octet-stream`; `MediaType.default()` = `[OctetStream]` (Rust `Default`, so `from_*(...).unwrap_or_default()`) |

Rules:
- Parsing entry points are `from_*`, never `parse*` (the public API does not use
  the word "parse").
- Parsing always validates and returns an error / raises on malformed input;
  there is no lenient mode and no `safe` flag.
- `with_*` / `without_*` / `copy` are **non-mutating** and return a new value.
- URL-safe `percent_encode` / `percent_decode` are the only encoding helpers;
  modifiers that build query strings percent-encode their inputs.

## Patterns to mirror

- **Errors**: one `enum` per type (`UriError`, `UrlError`, …) implementing
  `Display` + `std::error::Error`, with `From` conversions between layers. Core
  errors map to `ValueError` (Python) / thrown `Error` (Node).
  **Make error messages actionable**: when the fix is knowable, say it in the
  message — name the missing feature (`enable the \`gzip\` cargo feature`), the
  expected input (`expected 0, 1 or 2`), or the offending value (`unknown mode
  "rw+"`). A reader should learn *how to fix it* from the message, not just that
  it failed.
- **Docs**: every public item has a `///` doc comment; types carry a runnable
  doctest. Match the existing terse style.
- **Bindings**: each wrapper method is one or two lines delegating to
  `self.inner`. Use `#[pyo3(signature = ...)]` / napi `Option<T>` for defaults.

## Performance: zero-copy with checks

Prefer **borrowing over copying**. A function that returns string data should
hand back a borrow (`&str`) or a [`Cow`] and allocate **only when the data must
actually change** — guarded by a cheap up-front check:

- Decode/validate paths (`percent_decode`, `validate_percent_encoding`) check for
  the trigger byte (`%`) first and return the input untouched when it is absent —
  no allocation, no second scan.
- Encode paths (`encode_component`) scan for the first byte that needs escaping;
  if there is none they return `Cow::Borrowed`, otherwise they allocate once and
  copy the already-valid prefix verbatim before encoding the rest.
- Single-key lookups (`query_param`) scan for the one key instead of building the
  whole `Params` map, and compare the raw bytes without allocating unless an
  escape forces a decode.

When you add a hot path, ask "does this allocate when nothing changed?" — if so,
add the check and borrow. Never copy speculatively; never re-scan what a single
pass can decide.

### All byte access goes through `Io`

**Centralise every byte/memory access behind the [`Io`] trait** — it is the one
place that fully manages where bytes live and how they move, so it is where
zero-copy wins live. A new source (cloud object, HTTP body, …) implements `Io`
and overrides `as_slice` when it is memory-resident, so `pread` / `copy_to` /
`json` / media-sniffing all light up the zero-copy path for free; a partly-cached
source (e.g. `HttpStream`'s 4 MiB window) keeps its buffer management inside the
`Io` impl and never leaks raw buffers to callers. Operations that consume bytes
(`json`, compression, codecs, HTTP bodies) take an `Io`/`ReadBytes`, never a
pre-collected `Vec` — so the data is read once, lazily, and copied at most once.

This extends to the **bindings**: a Python/JS wrapper that needs bytes should
accept and pass our `Io` instances (`BytesIO` / `LocalPath` / `HttpStream`), not
serialized `bytes`, so a large body or upload streams through Rust and is never
materialised in the host language. Prefer `Io.json()` (parsed in Rust) over
handing raw bytes back across the FFI boundary.

## Logging

The Rust crates carry an optional, **off-by-default** `log` feature, emitted only
through the crate-local `log_event!(level, …)` macro (which compiles to nothing
when the feature is off, so the crates stay dependency-free and pay no runtime
cost). Never call `log::` directly, and keep the `log` dependency `optional`.

When you add or change behaviour, instrument it at the right level:

- `trace` — very frequent, per-call detail (e.g. each parse entry).
- `debug` — a routine **action being performed** (e.g. inferring a media type).
- `info` — an **important action that completed**, especially a change to global
  or shared state (e.g. a MIME-registry `register` / `unregister` / `reset`).
- `warn` — a **skipped** input or a **defaulted** fallback was applied (e.g. an
  unknown extension dropped from a media stack, a missing URI scheme defaulted to
  `file`, a drive letter treated as a Windows path).

A new code path that skips, defaults, or mutates shared state must log it; the
`log` feature must compile and pass `clippy -D warnings` both on and off.

## Required checks before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
```

All five must pass.

## Releasing

The workspace `version` under `[workspace.package]` in the root `Cargo.toml` is the
single source of truth. To cut a release, bump it and merge to `main`: the
`Release` workflow detects the new version (no matching `v<version>` tag yet),
runs the gate, publishes to crates.io / PyPI / npm, then creates the tag and a
GitHub Release. `yggdryl-http`'s dependency on `yggdryl-core` is a caret range, so a
`0.1.x` bump only touches that one line (the Python wheels inherit it via
`version.workspace = true`; the npm `package.json` is synced from it at publish time
— keep it in sync locally too). Never re-use a published version number;
crates.io/npm reject re-uploads.

## Code-coherence review (after every implementation)

Once the change compiles and the checks pass, do a final coherence pass before
committing — treat it as a required step, not an optional polish:

1. **No redundancy** — fold duplicated logic into one place; a new `from_*`
   should delegate to an existing one (e.g. `from_extension` → `from_extensions`
   → `from_path`) rather than re-implement it. Don't add a second API that
   merely restates an existing one.
2. **Cross-language parity** — the same surface and semantics exist in the Rust
   core and *both* bindings (adapting only to each language's idioms); a change
   is never half-applied.
3. **One concern per file/type** — the new code lives in the right crate/module
   and mirrors the structure of its neighbours (naming, error handling, doc
   style, terseness).
4. **Readability** — names match the conventions table, every public item has a
   `///` doc, and a reader cannot tell which type they are looking at from the
   shape of the code.

If any point fails, fix it before committing.
