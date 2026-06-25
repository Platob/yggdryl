# yggdryl — contributor & agent instructions

**Keep all new code uniform with the existing patterns.** Before adding anything,
read the nearest existing example and mirror its structure, naming, error
handling, and doc style. Consistency across the Rust core and the two bindings is
the top priority — a reader should not be able to tell which type they are
looking at from the shape of the code.

## Architecture

- `crates/yggdryl-core/` — dependency-free foundations: the `FromInput` /
  `ToOutput` traits and percent-encoding.
- `crates/yggdryl-io/` — the **byte-IO foundation**: one set of methods to read,
  write, seek and stat bytes wherever they live (memory, local path, cloud). See
  its dedicated section below. **All byte-IO logic lives here.**
- `crates/yggdryl-compression/` — the `Compression` codec (gzip / Zstandard /
  Snappy / `None` identity) that **streams** compress/decompress over any
  `yggdryl-io` handle, plus the `CompressIo` extension trait. **All compression
  logic lives here** — do not pull codec SDKs into `yggdryl-io`.
- `crates/yggdryl-http/` — a blocking, `requests`-like HTTP client
  (`HttpSession` / `HttpRequest` / `HttpResponse`) whose bodies **stream over the
  `yggdryl-io` abstraction**. **All HTTP logic lives here** — the transport SDK
  (`ureq`) is a dependency of this crate only. See its section below.
- `crates/yggdryl-version/` — the standalone `Version` type.
- `crates/yggdryl-media/` — the `MimeType` enum (single MIME types, backed by a
  mutable global registry of extensions/magic bytes) and the `MediaType` stack
  (an ordered `Vec<MimeType>`, e.g. `csv.gz` → `[Csv, Gzip]`). **All media-type
  logic lives here.**
- `crates/yggdryl-url/` — the `Uri`/`Url` types and the canonical URL tests, built
  on and re-exporting `yggdryl-core` (and `yggdryl-media` for the inferred
  `media_type()` accessor). **All URL logic lives here.**
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**. They only translate types/errors and call the core; they contain no
  logic. Anything added to the core must be surfaced in *both* bindings.

### `yggdryl-io` — what it aims to be (read before extending it)

The goal is a **single byte-IO abstraction** that hides *where* bytes live, so a
reader (think Arrow / Parquet) works the same over an in-memory buffer, a
memory-mapped local file, or a cloud object — mixing **random** access (read a
footer, a column chunk) with **streamed** access (scan record batches) on one
handle. The layering, smallest to largest:

- `ReadBytes` / `WriteBytes` — byte source/sink primitives (`&[u8]`, `Vec<u8>`).
- `Seek` — the cursor (`seek` / `stream_position` / `stream_len`).
- `Io: ReadBytes + Seek` — **the base handle**. Every IO has a `url()` (in-memory
  ones use `mem://<address>`). It reads/writes at a position via `pread` /
  `pwrite` — a `Whence` selects positional (`Start`/`End`, cursor untouched, the
  default) versus cursor-relative (`Current`, uses and advances the cursor);
  `pwrite` defaults to `Unsupported` (writable backends override it). Storage is
  managed with `capacity` / `reserve_capacity` / `truncate` (also `Unsupported`
  on read-only backends; `BytesIO` adds the `with_capacity` constructor). Each
  handle carries an access `mode()` (`Mode` — `Read`/`Write`/`Append`/`ReadWrite`,
  parsed from Python strings via `Mode::from_str`) and an optional `parent()`,
  and can `open()` a derived handle (records the parent, applies mode/stream)
  and `close()` it (idempotent; the default is a no-op as memory/mmap backends
  free their storage on drop). Plus `as_slice` (the zero-copy hook a memory
  backend overrides), `stats`, and
  `copy_to` (transfer with a memory fast path; `copy` is the free fn).
  `media_type` is lazy and behind the `media` feature. (`Io: …+ Debug + Send +
  Sync` so handles can be boxed as parents and held across threads.)
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
  — do not pull network SDKs into `yggdryl-io`.**
- `Codec<T>` — typed read/write/stream of values over any byte handle; `Frames`
  is the reference length-delimited codec. (`Codec` is the *value* coder; `Io` is
  the *byte* handle — keep them distinct.) Byte-stream **compression** is a
  separate concern in `yggdryl-compression` (see its section), not here.

Rules when extending: the base build depends only on `yggdryl-url` (for the
universal `Io::url()`); new heavy deps are **optional features** (like `log` /
`mmap` / `media`). A new memory-resident backend must override `as_slice` so the
zero-copy `pread` / `copy_to` paths light up; positional reads go through `pread`
with `Whence::Start`, never by mutating the cursor.

### `yggdryl-compression` — streamed codecs over `Io`

Compression is layered **on top of** `yggdryl-io`, never inside it (so the IO
base stays codec-free and the dependency points one way: `yggdryl-compression →
yggdryl-io`). The shape:

- `Compression` — `None` / `Gzip` / `Zstd` / `Snappy`; `from_str` /
  `from_extension` / `as_str` / `extension` / `is_available`, and (under `media`)
  `from_mime` / `from_media` / `from_stats` for inference.
- `encoder(sink) → Encoder: WriteBytes` (compress-on-write; `finish()` flushes the
  trailer) and `decoder(source) → Decoder: ReadBytes` (decompress-on-read); the
  one-shot `compress` / `decompress` build on them. Internal `std::io` shims
  bridge `ReadBytes`/`WriteBytes` to the `flate2`/`zstd`/`snap` stream codecs.
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
`yggdryl-io` (and `yggdryl-url`); the transport is `ureq` (rustls TLS, its own
gzip/brotli left off so decompression goes through `yggdryl-compression`). The
shape:

- `HttpSession` — like `requests.Session`: a pooled `ureq::Agent`, default
  headers, a `RetryConfig`, `max_concurrency` (8) and `batch_size` (80). Every
  send funnels through `prepare` (merge defaults; per-request headers win) then
  `request(req, raise_error)` — `raise_error` defaults to `true` on the verb
  helpers (`get`/`post`/…), raising on a 4xx/5xx. `stream(req)` opens an
  `HttpStream`; `send_many(reqs)` is a lazy iterator of `HttpResponseBatch`,
  running each batch up to `max_concurrency` at a time (scoped threads).
- `HttpRequest` — a `Method` + `Url` + headers + body builder (`with_header` /
  `with_param` / `with_body` / `with_body_reader` / `with_body_io`). `with_body_io`
  is the preferred upload: the handle's `stream_len` sets `Content-Length` and the
  bytes stream straight off the `Io` (a file is never buffered).
- `HttpResponse` — `status`/`ok`/`raise_for_status`/`headers`/`header`. The body
  is lazy: `reader()` is a `ReadBytes` source (decoded under `compression`),
  `bytes`/`text`/`into_bytesio` drain it.
- `HttpStream: Io` — a seekable HTTP body. A `HEAD` makes size / content type /
  media discoverable; bytes are then fetched lazily in **4 MiB windows** via
  `Range` (sequential `read`) or one-off ranges (`pread`, footer reads). Reads
  retry transient statuses (429/502/503/504, honouring `Retry-After`) and
  **resume from the cursor** on a dropped connection (each window is an
  idempotent range request). This is the canonical "remote `Io`".
- Retries cover replayable bodies (none/bytes) and all `HttpStream` window
  fetches; a streamed (reader/`Io`) request body is single-shot.

Optional features: `compression` (auto `Content-Encoding` decode — it also turns
on the codec backends), `media` (`mime_type()`), `log`. The base depends on
`yggdryl-io`'s `json` feature so `Io::json()` is available on every handle. **All
HTTP logic lives here; `ureq` stays a dependency of this crate only.** Tests are
**hermetic** (a localhost `TcpListener` that serves HEAD / `Range` / 429 /
mid-stream drops, no network). In the bindings the blocking call must not stall
the host runtime: Python releases the GIL (`allow_threads`), Node runs the
request on the libuv pool and returns a `Promise` (so Node's surface is async,
the one idiomatic divergence from the sync Rust/Python API). Bindings pass our
`Io` instances as bodies — never serialized `bytes` — per the Io-centralisation
rule above.

### One module per type, everywhere

Code is organised the same way in every language: **one file per type**, with a
small glue file tying them together. Don't grow a single big file.

- Rust: one crate per concern (`yggdryl-core`, `yggdryl-version`,
  `yggdryl-media`, `yggdryl-url`).
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
GitHub Release. Inter-crate deps are caret ranges, so a `0.1.x` bump only touches
that one line (the Python wheels inherit it via `version.workspace = true`; the
npm `package.json` is synced from it at publish time — keep it in sync locally
too). Never re-use a published version number; crates.io/npm reject re-uploads.

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
