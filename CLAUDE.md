# yggdryl — contributor & agent instructions

**Keep all new code uniform with the existing patterns.** Before adding anything,
read the nearest existing example and mirror its structure, naming, error
handling, and doc style. Consistency across the Rust core and the two bindings is
the top priority — a reader should not be able to tell which type they are
looking at from the shape of the code.

**Everything must be serializable and hashable.** Do your best to make every value
type round-trip through *all* of: a canonical string (`from_str`/`to_str`), a
component map (`from_mapping`/`to_mapping`), JSON (`serde`, plus `to_json`/`from_json`
where a crate exposes a `json` feature) and **bytes** (`to_bytes`/`from_bytes`), and
to derive (or hand-implement) `Hash` + `Eq` so it can key a map or set. In the
bindings this means `__hash__` + `__reduce__` (pickle) in Python and `toJSON()` + a
static `fromJSON()` in Node. The only exceptions are live/stream resources (`Io`
handles, HTTP bodies, sessions). When a field cannot be part of a value's identity
(e.g. a `Field`'s navigational `parent`, which would create cycles), exclude it
from `Hash`/`Eq`/`serde` rather than dropping hashability — and document why.

## Architecture

The workspace is **four crates**: `yggdryl-core` (all the data types + byte IO +
compression + the self-contained calendar/time module), `yggdryl-schema` (the
Arrow-compatible `DataType` / `Field` schema layer), `yggdryl-serie` (the Arrow-backed
columnar `Serie` layer built on the schema) and `yggdryl-http` (the network
client). `yggdryl-core` is **one file per type** — each concern is a module (or
module directory) under
`crates/yggdryl-core/src/`, with `lib.rs` as glue (a shared `log_event!` macro,
`mod` declarations, and `pub use` re-exports of every type at the crate root, so
`yggdryl_core::Io` / `::Url` / `::Compression` / … all resolve). Each module owns
its concern wholly — do not scatter a concern's logic across modules:

- `encoding.rs` / `mapping.rs` — dependency-free foundations: the `Params`
  query-parameter map and percent-encoding (component maps are a plain
  `BTreeMap<String, String>`, not a type alias). Each value type pairs its own
  inherent `from_str` / `from_mapping` parsers with inherent `to_str` /
  `to_mapping` renderers (no shared rendering trait — keep them per-type).
- `version.rs` — the standalone `Version` type.
- `media/` (`mod` + `mime.rs` + `media_type.rs`) — the `MimeType` enum (single MIME
  types, backed by a mutable global registry of extensions/magic bytes/**category** —
  add a common type by appending one `builtin(category, …)` row to `BUILTINS`, keeping a
  specific magic *before* a broader one, e.g. AVIF's `ftypavif` before MP4's `ftyp`;
  programming languages like Python/Rust/TypeScript are built-ins in the `Code`
  category, recognised by extension only) and the `Category` classifier (`Blob`
  default / `Directory` / `Tabular` / `Code` / `Codec`, stored per registry entry and
  read back via `MimeType::category()`; set on `register`), plus the `MediaType` stack
  (an ordered `Vec<MimeType>`, e.g. `csv.gz` → `[Csv, Gzip]`; **compound archive
  extensions** like `.tgz`/`.tbz2`/`.txz`/`.tzst` expand to `[Tar, <codec>]` via
  `expand_extension`). **All media-type logic lives here.**
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
- `time/` (`mod` + `date.rs` + `time.rs` + `datetime.rs` + `duration.rs` +
  `timezone.rs`) — a **self-contained calendar/time foundation** (std has no civil
  date/time types): `Date` (days since epoch), `Time` (ns of day), `DateTime` (a UTC
  instant + optional display `Timezone`), `Duration` (signed ns) and the shared
  `TimeUnit`. `Timezone` carries an **embedded POSIX-TZ DST engine** + a broad IANA
  name table (`ZONE_TABLE`), so zone/DST conversions need **no external tz database**
  (current rules only; historical transitions are not modelled). Civil math uses the
  exact Hinnant algorithms in `mod.rs`. The point-in-time types do **calendar
  arithmetic** (`add`/`sub` a `Duration`, `duration_since`, `truncate` to a `Duration`
  boundary, with `std::ops` `+`/`-` and `Duration` `*`/`/` operators), an **empty
  string/buffer parses to the zero default** (epoch / midnight / `0`), and the
  `Temporal` trait is **bidirectional** — `to_datetime`/`to_date`/`to_time` plus
  `from_datetime` (required) and a `from_temporal<T>` default that redirects through
  `to_datetime`. **All calendar/time logic lives here** — add a zone by appending one
  `(name, posix)` row to `ZONE_TABLE`.
- `crates/yggdryl-schema/` — the Arrow-compatible schema layer. See its section
  below. The `arrow-schema` SDK is a dependency of this crate only.
- `crates/yggdryl-serie/` — the Arrow-backed columnar layer built on the schema. See
  its section below. The `arrow-array` SDK is a **required** dependency of this crate
  only (every `Serie` is array-backed).
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
- The **factory** `from_str` / `from_url` / `from_uri` (the location-open factory —
  distinct from the `Io::open` *method*, which derives a child handle) returns the
  right `Box<dyn Io>` for a location, dispatching on the URL
  scheme: a bare path / `file://` opens a `LocalPath`; `http`/`https` send a `GET` and
  return the live `HttpResponse` (itself an `Io`); any other scheme is looked up in the
  `register_scheme` registry (a global `OnceLock<RwLock<…>>`, like the MimeType registry)
  so downstream crates plug in without the `io` module depending on them — `yggdryl-http`
  registers `http`/`https` (lazily, on first `HttpSession::new`), cloud stores their
  schemes later. `mem://` and unregistered schemes return an actionable `Unsupported`.

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

- `Compression` — `None` / `Gzip` / `Deflate` (zlib, HTTP `Content-Encoding: deflate`)
  / `Zstd` / `Snappy` / `Brotli` (HTTP `Content-Encoding: br`); `from_str` /
  `from_extension` / `as_str` / `extension` / `is_available`, and (under `media`)
  `from_mime` / `from_media` / `from_stats` for inference plus `mime()` (the inverse of
  `from_mime`, used to add an encoding layer to a media type). `Deflate` is the zlib
  format (RFC 1950) and shares the `gzip` feature/`flate2` backend; like `Snappy` it has
  no registered file MIME. Brotli has no magic bytes, so it is recognised by the `.br`
  extension / `application/x-brotli` MIME only, never by content sniffing.
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

Each backend is an **optional feature** (`gzip`/`zstd`/`snappy`/`brotli`, all on by
`default`); a variant whose feature is off still parses and names itself but reports
`Unsupported` on encode/decode (`is_available` tells ahead of time). `media` adds the
stats-inference path. **`gzip` uses `flate2`'s pure-Rust `zlib-rs` backend** (not the
default `miniz_oxide`): near-C-zlib throughput (~3x faster compress, matching decompress)
with **no C compiler / cmake build dependency**, so the wheels / npm builds stay
pure-Rust — keep `default-features = false` + `features = ["zlib-rs"]` on the `flate2`
dep. When you add a codec, surface it in *both* bindings.

### `yggdryl-schema` — Arrow-compatible `DataType` / `Field`

A compact schema layer built to back a future dataframe, **centred on two types**
(`DataType` + `Field`) and split one-file-per-concern under
`crates/yggdryl-schema/src/`:

- `charset.rs` — the `Charset` of a string (`Utf8` default / `Utf16` / `Utf32` /
  `Ascii` / `Latin1`).
- `datatype/` (`mod` + `primitive.rs` + `logical.rs` + `nested.rs` + `coerce.rs`) —
  the central `DataType` enum, its `TypeCategory` (`Any` / `Primitive` / `Logical` /
  `Nested`), the `SchemaError`, the canonical string **grammar** (`from_str`/`to_str`
  spanning every variant) and the uniform physical accessors (`bit_size` / `is_large`
  / `is_view` / `is_fixed_size` / `physical_type` — the last returns a logical type's
  storage primitive, identity for the rest — plus the `Numeric` trait mutualising the
  numeric types' `numeric_bits` + common `signed` accessor). **Unlike Arrow, the model
  is parameterized, not combinatorial**: `Int{bits,signed}` (**any** width, not just
  8/16/32/64 — `int24`/`uint128` parse; `integer()` defaults to `int64`),
  `Float{bits}` (likewise **any** width — `float24` parses; `floating()` defaults to
  `float64`), `Decimal{precision,scale,bits}`, `Varchar{charset,large,view,size}` (a `Some` `size`
  is a fixed-length `char(n)`, rendered `char[…]`; `varchar(n)`'s length is a dropped
  max-hint), `Binary{large,view,size}`, the **string-backed `Json` and binary-backed
  `Bson`** logical types, the temporal types reuse the core `TimeUnit`/`Timezone`
  (`Date{large}` / `Time{unit}` / `Timestamp{unit,tz}` / `Duration{unit}` /
  `Interval{unit}`), and the nested `List{item,large,view,size}` / `Struct(Vec<Field>)`
  / `Map{key,value,sorted}` / `Union{fields,mode}` / `RunEndEncoded` / `Dictionary`,
  plus the `Any` wildcard. The category split lives across the three sibling files
  (`primitive`/`logical`/`nested` hold that category's checks + constructors);
  `coerce.rs` holds the `MergeStrategy`, `can_cast_to`, `common_type` (the promotion
  lattice) and `merge`. **All DataType logic lives here.**
- `field.rs` — the `Field` graph node (name + `DataType` + nullable + metadata + an
  optional, identity-excluded `parent`). Carries the metadata getters/setters
  (`comment` is the named convenience), the case-insensitive / index child accessors,
  `with_linked_children` / `root` graph wiring, and `merge`. A struct-typed `Field`
  **is** a schema.
- `arrow.rs` (feature `arrow`) — fast, near-total conversion to/from `arrow-schema`'s
  `DataType` / `Field` / `Schema` (`to_arrow`/`from_arrow`, `Field::to_arrow_schema`
  / `from_arrow_schema`). `Any` has no Arrow equivalent and errors; a non-UTF-8
  `Charset` maps to UTF-8 (Arrow has no charset). **All Arrow conversion lives here.**

Features: `serde` (structural, lossless — DataType/Field derive, the enums derive),
`json` (`to_json`/`from_json`, implies `serde`), `arrow`, `log`. The temporal types
come from `yggdryl-core`. **Anything added here must be surfaced in both bindings.**

### `yggdryl-serie` — Arrow-backed columnar `Serie`

The layer between the schema types and a future dataframe: a `Serie` is a single
named, typed **column** — a [`Field`](schema) paired with an Apache **Arrow** array,
so it carries both its logical type and its physical storage. Built **on top of**
`yggdryl-schema` (with its `arrow` feature) and `arrow-array` (a **required** core
dependency — every serie is array-backed), the dependency points one way (serie builds
on schema, never the reverse). Split one-file-per-type under `crates/yggdryl-serie/src/`,
mirroring the schema crate's three [categories](#yggdryl-schema--arrow-compatible-datatype--field):

- `error.rs` — the `SerieError` / `SerieResult` (with `From<SchemaError>` and
  `From<arrow_schema::ArrowError>`; actionable messages).
- `serie.rs` — the **two traits** and the **redirect factory**. `Serie` is the
  object-safe base (untyped column ops: `field` / `array` (the backing `ArrayRef`) /
  `len` / `num_rows` / `null_count` / `is_null` / `is_valid` / `as_any` for downcast,
  plus the convenient field reflections `name` / `dtype` (alias of `data_type`) /
  `get_metadata(key)` (a **narrow, safe** accessor — the whole metadata map stays
  encapsulated in the field, no wide `metadata()` getter), `category`, type-erased value
  access by index (`value_at` → `Scalar`) and by range (`slice` / `slice_range`,
  zero-copy), the navigational `parent` graph link, `is_materialized` and `materialize`
  (realise a lazy/child column into an independent in-memory one), all defaulting off the
  field); `TypedSerie<T>` adds typed value access (`get` / `value` / `iter` / `to_vec`)
  over a column's native value type. `from_arrow(field, array)` / `from_array(name,
  array)` **redirect** an Arrow array to the right concrete series, returning a boxed
  `SerieRef = Arc<dyn Serie>`. `from_arrow` checks the field's `DataType` maps to the
  array's Arrow type then calls the crate-internal `dispatch`; `from_array` derives the
  field from the array and calls `dispatch` **directly** (skipping the equality check,
  which would trip on the schema's documented Arrow normalisations — e.g. a map's
  `key`/`value` entry names vs Arrow's `keys`/`values`). The recursive nested builders
  call `dispatch` too. **All dispatch logic lives here.** A default `display(&DisplayOptions)`
  renders the column to a readable string (see `display.rs`). (Object-safety: the base
  trait is *not* generic; the `<T>` lives in `TypedSerie<T>`, recovered via
  `as_any().downcast_ref`.)
- `display.rs` — `DisplayOptions` (`max_rows` / `header` / `width` / `null` / `index`)
  and the `render` routine behind `Serie::display`, the building block for a future
  `Frame`'s table rendering. **All display logic lives here.**
- `scalar.rs` — the type-erased `Scalar` (a single value read by index: integers /
  decimals-128 / temporal-physicals widen to `Int(i128)`, floats to `Float(f64)`,
  plus `Boolean` / `Utf8` / `Binary`, and an `Other(String)` for the exotic physicals —
  256-bit decimals, interval structs — so no value is dropped) and the `scalar_at`
  array-cell extractor. **All scalar logic lives here.**
- `primitive/` (`mod` + `numeric.rs` + `boolean.rs` + `varchar.rs` + `binary.rs`) — the
  **primitive** concrete series. `PrimitiveSerie<A: ArrowPrimitiveType>` is the one
  generic backing every fixed-width scalar (integers, floats, decimals, **and** the
  date/interval physical types — **timestamps/times/durations unify into the temporal
  series**, not here); `BooleanSerie`, `VarcharSerie<O: OffsetSizeTrait>` (`Utf8`/`LargeUtf8`,
  plus a zero-copy `str_value`) and `BinarySerie<O>` (`Binary`/`LargeBinary`, plus
  `bytes_value`) cover the rest. Named aliases (`mod.rs`: `Int32Serie`, `Float64Serie`,
  `Date32Serie`, …) pin the common widths. Each concrete stores its typed array +
  `Field`, overrides the length/null methods for zero-overhead, and `array()` returns a
  cheap `Arc` clone.
- `temporal/` (`mod` + `datetime.rs` + `time.rs` + `duration.rs`) — the **temporal**
  series. `TemporalSerie` is the shared trait (a uniform `datetime_at`, with derived
  `date_at` / `time_at`, all in core `DateTime`/`Date`/`Time`). `DatetimeSerie` /
  `TimeSerie` / `DurationSerie` are the **unified** timestamp / time / duration columns:
  each backs an Arrow array of **any** `TimeUnit` (timestamps also carry an optional
  `Timezone`), reads the unit/zone from its field, and presents values as the core
  `DateTime` / `Time` / `Duration` — replacing the per-unit aliases. `DatetimeSerie` /
  `TimeSerie` implement `TemporalSerie`; `DurationSerie` is a span, so it does not. **All
  `TemporalSerie` / unified-temporal logic lives here.**
- `nested/` (`mod` + `struct_serie.rs` + `list_serie.rs` + `map_serie.rs`) — the
  **nested** series. `NestedSerie` is the shared trait (`child_count` / `child(index)` /
  `child_by_name`). `StructSerie` (a child `Serie` per field), `ListSerie<O>` (a flattened
  values child + a zero-copy per-row `value_slice`) and `MapSerie` (flattened key/value
  children) build their children **recursively** via `dispatch`, so arbitrarily deep
  nesting resolves uniformly; each `value_at` renders a readable `{…}` / `[…]`. **All
  nested-series logic lives here.**
- `lazy/` (`mod` + `range.rs` + `daterange.rs` + `datetimerange.rs` + `timerange.rs`) —
  the **lazy / computed** series, not resident in memory: they store a compact
  description and compute each value on demand (`is_materialized()` → `false`),
  `array()`/`materialize()` realising a real column (a `slice` of any stays lazy).
  `RangeSerie` is a `uint64` arithmetic range (`start + i*step`); `DateRangeSerie` a
  day-resolution `Date32` range; `DateTimeRangeSerie` a nanosecond `Timestamp` range
  (tz-naive); `TimeRangeSerie` a `Time64` time-of-day range (wraps within the day). The
  three temporal ranges implement `TemporalSerie`. **All lazy-series logic lives here.**
- `index.rs` — `IndexSerie`, a row index (a `Serie` of labels with `at` (label at a row)
  / `position` (row of a label) / `contains` lookups), **defaulting to a lazy `uint64`
  `RangeSerie`** (`is_range()` enables the O(1) lookups); wraps any column via
  `from_serie` / `from_array`.
- `enum_serie.rs` — `EnumSerie`, a categorical view that scans a column once and holds
  the **mapping of unique values** to a compact `code` (`0..unique_count`) and to their
  `first_row` index (`code` / `first_row` / `value_of` / `code_at`); a `Serie`
  delegating data access to the backing column. **All enum/categorical logic lives here.**
- `slice.rs` — `SliceSerie`, a zero-copy **child** view that records its `parent`, and
  the `child` / `child_range` constructors that build the parent→child graph;
  `materialize` detaches a child into an independent column. **All slice-graph logic
  lives here.**
- **Still to build** (next increments): the **union** nested type, the **dictionary** /
  **view** backends, a **`ChunkedSerie`** mirroring Arrow's `ChunkedArray`, cast /
  arithmetic operations, **benchmarks**, and the **Python / Node bindings** (not yet
  surfaced — the cross-language rule applies once the Rust base settles).

Features: `log` only so far (`arrow-array` is required, not optional). **All serie logic
lives here; `arrow-array` stays a dependency of this crate only.**

### `yggdryl-http` — a requests-like client streaming over `Io`

A small **blocking** HTTP client shaped after Python's `requests`, layered on
`yggdryl-core` (its `io` / `url`); the transport is `ureq` (rustls TLS, its own
gzip/brotli left off so decompression goes through `yggdryl-core`'s `compression`). The
shape:

- **Transactions are centralised on `HttpRequest` / `HttpResponse`; `HttpSession` is
  a defaulting factory + transport.** A request is *self-sufficient*: `request.send(raise_error)`
  dispatches it through the process-wide shared session and returns an `HttpResponse`
  — no session reference needed. A custom-configured client (its own pool, TLS, proxy,
  default headers) is an `HttpSession`, whose job is to **build** requests
  (`prepare` merges its defaults) and **run** them (`session.send(request, raise_error)`).
  Keep this shape when extending: new request behaviour lives on `HttpRequest`, new
  client configuration on `HttpSession`; don't add a second send path.
- **The verb helpers centralise on `prepare` → `HttpRequest` and `send(request)` →
  `HttpResponse`, and always return an `HttpResponse`.** `get`/`head`/`delete`/`post`/
  `put`/`patch`/`request` take a **`send` flag** (default `true`): with `send` the
  request is dispatched; with `send` `false` no network call is made and an **unsent**
  `HttpResponse` is returned — `is_sent()` is `false`, the status is `0`, the body
  empty — carrying the prepared request via `response.request()`, dispatchable later
  with `response.send(raise_error)` (through the shared session). Every response
  **holds the request that produced it** (`response.request()`, like
  `requests.Response.request`). The bindings configure the whole request from the
  verb's signature args (kwargs in Python, options in Node: `headers` / `params` /
  `basic_auth` / `bearer_auth` / `allow_redirect` / `keep_alive` / `http_version` /
  `raise_error` / `send`); Rust keeps lean verbs (`url`/`body` + `send`) and richer
  configuration on the `HttpRequest` builder, per the per-language idiom rule.
- `HttpSession` — like `requests.Session`: a pooled `ureq::Agent` (an idle-connection
  pool, sized by `with_pool_size`, so reused keep-alive connections skip the TLS
  handshake; idle connections past the keep-alive TTL are dropped), default headers, a
  `RetryConfig`, a `read_timeout` (`with_read_timeout`, default 120s — errors with a hint
  when the server sends no data for that long), `max_concurrency` (8) and `batch_size`
  (80). **Every dispatch funnels through the one method** `send(req, raise_error)` — there
  is no `stream` flag (the body is **always** a live `HttpStream`, which handles buffering
  and random access itself) and no separate `stream()` method; the verb helpers
  (`get`/`post`/…/`request`) all build a request and delegate to `send` through a private
  `run_verb` (so `request(req, raise_error, send)` is a verb, not a second send path). It
  `prepare`s the request (merge defaults; per-request headers win, case-insensitively), runs
  it with the retry policy, and returns an `HttpResponse` holding the live body. `raise_error`
  (`true` on the verb helpers `get`/`post`/…) raises on a 4xx/5xx; connection reuse is the
  request's own **keep-alive idle TTL** in seconds (`with_keep_alive(seconds)`, default 300
  — `0` → `Connection: close`; a pool-saturation safeguard still forces `close` on streams
  past the pool size). `send_many(reqs)` is a lazy iterator of
  `HttpResponseBatch`, running each batch up to `max_concurrency` at a time (scoped
  threads). `send` also drives the **redirect** loop (`with_max_redirects`, default
  10) and an RFC 6265 **cookie jar** (`cookies()` / `set_cookie`). An optional
  **`base_url`** (`with_base_url`) prefixes requests: the verb helpers run their
  target through `resolve_url`, so a relative reference (`/path`, `name`) joins onto
  the base (same RFC 3986 rules as a `Location` redirect) while an absolute URL is
  used unchanged. A process-wide **shared singleton** `HttpSession::shared()` (a
  replaceable `Arc` behind an `RwLock`, swapped by `set_shared`) backs the crate-level
  `get`/`head`/`post`/`put`/`patch`/`delete`/`request` **module functions**, the
  `requests.get(...)` equivalent. Alongside it, **`HttpSession::shared_for(host)`** keeps
  one pooled singleton **per hostname** (a global `host → Arc<HttpSession>` registry):
  this is the session a request is dispatched through when none is given —
  `HttpRequest::send`, the `http`/`https` `Io` factory, the bindings' `request.send` /
  `response.send`, and the session a returned `HttpResponse` carries
  (`response.session()`) — so a session is shared by host, never copied per request.
  Each per-host session is **seeded from the global `shared()` config** (default headers,
  auth, TLS/CA, proxy, retry, redirects, version — via `copy()`) at first use, so
  configuring `shared()` (`set_shared`) up front propagates to per-host sessions; the
  registry is **bounded** (`MAX_HOST_SESSIONS`, idle entries evicted) and resettable with
  `clear_host_sessions()`. The bindings mirror the module-level verbs over the shared
  session and a `set_base_url` to configure it (Node has no `delete` verb — a JS reserved
  word — so use `request('DELETE', …)`).
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
  `with_body_reader` / `with_body_io` / `with_allow_redirect` / `with_keep_alive`), plus
  `send(raise_error)` (dispatch via the shared session) and `copy()` (an independent
  copy; a streamed body can't be duplicated, so the copy carries none).
  `with_keep_alive(seconds)` sets the keep-alive idle TTL (default 300; `0` →
  `Connection: close`). `with_body_io` is the preferred upload: the handle's
  `stream_len` sets `Content-Length` and the bytes stream straight off the `Io` (a file
  is never buffered). `with_allow_redirect(false)` opts a request out of the redirect
  loop (returning the 3xx).
- **Authentication** (`auth.rs`, crate-internal) — `with_basic_auth(user, pass)` and
  `with_bearer_auth(token)` on both `HttpRequest` and `HttpSession` set the
  `Authorization` header (HTTP Basic, RFC 7617, with a dependency-free base64 encoder;
  Bearer, RFC 6750). Session-level auth is a default header, so a per-request value
  overrides it and a cross-origin redirect strips it. The bindings surface it as the
  session `basic_auth`/`bearer_auth` (Python kwargs) / `basicAuth`/`bearerAuth` (Node
  options).
- `HttpResponse` — `status`/`ok`/`raise_for_status`/`headers`/`header`, plus the typed
  reads `mime_type` (from `Content-Type`), `media_type` (**combines `Content-Type` with
  `Content-Encoding`** — a gzipped CSV is `[Csv, Gzip]`, like a `data.csv.gz` path; under
  `media`) and `compression` (the codec named by `Content-Encoding`, under
  `compression`). It also **holds the request that produced it** (`request()`, like
  `requests.Response.request`) and reports whether it was dispatched (`is_sent()` —
  `false` for the unsent placeholder a verb returns with `send=false`, status `0`).
  It **carries the shared per-host [`session`](HttpSession::shared_for) it belongs to**
  (`session()` — a `shared_for(host)` singleton, never a per-response copy); `send(raise_error)`
  re-dispatches its request through that session (how an unsent response is sent later).
  **`HttpResponse` is itself an `Io`** (delegating to its body, the `HttpStream`), so a
  response reads/seeks/`pread`s like any byte source — the `http`/`https` `Io` factory
  (`from_str`/`Io::open`) hands a sent response straight back. It **holds the live body**
  as a `Box<dyn Io>` (the `HttpStream`):
  `reader()` is the decoded body `Io` (decompressed under `compression`),
  `bytes`/`text`/`json`/`into_bytesio` drain it (`text`/`json` decompress transparently
  first), `read_all` drains and returns the `received_at` finish time together (used by
  the buffering bindings), `body_mut` borrows the raw body to read/seek in place,
  `into_io` takes the whole body. In the **bindings** the decoded body is exposed as a
  yggdryl `BytesIO` handle (`response.io`) — the performant, Rust-backed byte result you
  `json()`/`decompress()` without an FFI copy — while `content` (native `bytes`/`Buffer`)
  and `BytesIO`'s `__bytes__` / `to_bytes_io()` are the on-demand native converters.
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
  lean blocking `ureq` client. **The default trust store is the OS-native certificate
  store** (Windows SChannel, macOS Security framework, Linux system bundle) via ureq's
  `platform-verifier` (`RootCerts::PlatformVerifier`), so corporate/OS roots are
  honoured out of the box. TLS verification follows the session's `verify` flag (a
  rustls `NoVerify` certifier when off); `with_ca_cert` / `with_ca_cert_file` installs
  custom CA certificates (PEM/DER) that **replace** that store (the secure alternative
  to `verify=false`, like `requests`' `verify=<bundle>`); a proxy applies to the `ureq`
  h1 path. A `read_timeout` (`with_read_timeout`, default 120s) bounds the wait for
  server data via ureq's recv timeouts, surfacing an actionable error.

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
| Single MIME type | `MimeType` enum; `from_str` (a full MIME *or* a short name like `json`/`zstd`) / `from_mapping` / `from_parts(type, subtype)` / `from_extension(ext)` / `from_magic(bytes)` / `from_path(path)`; `.mime` / `type` / `subtype` / `extension(s)` / `category` |
| MIME category | `Category` enum (`blob` default / `directory` / `tabular` / `code` / `codec`); `from_str` / `as_str`; `MimeType.category()` reads it from the registry |
| Global MIME registry | `MimeType.register(mime, extensions, magic, category=blob)` / `unregister(mime)` / `reset_registry()` |
| Layered media type (extension stack) | `MediaType` = ordered `[MimeType, …]`; `from_str` / `from_mapping` / `from_extension` / `from_extensions` / `from_path`; `.types` / `first` / `last` / `category` (outermost layer's, `blob` default) |
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

## Documentation

User-facing docs live in **`docs/`** as a **MkDocs Material** site (config:
`mkdocs.yml`), published to **GitHub Pages** (https://platob.github.io/yggdryl/) by
the `Docs` workflow (`.github/workflows/docs.yml`) on every push to `main` that
touches `docs/**` or `mkdocs.yml`. Benchmark numbers/results live in
`benchmarks/README.md` (organised by theme), surfaced on the docs `Benchmarks` page.

**The docs tree mirrors the code tree** — one page per concern/module, so code and
documentation map 1:1 and a reader can find the doc for any type by its module:

| code | doc page |
| --- | --- |
| `yggdryl-core/src/version.rs` | `docs/core/version.md` |
| `yggdryl-core/src/media/` | `docs/core/media.md` |
| `yggdryl-core/src/url/` | `docs/core/url.md` |
| `yggdryl-core/src/io/` | `docs/core/io.md` |
| `yggdryl-core/src/compression/` | `docs/core/compression.md` |
| `yggdryl-core/src/time/` | `docs/core/time.md` |
| `yggdryl-schema/src/datatype/` | `docs/schema/datatype.md` |
| `yggdryl-schema/src/field.rs` | `docs/schema/field.md` |
| `yggdryl-serie/src/serie.rs` | `docs/serie/serie.md` |
| `yggdryl-http/src/session.rs` | `docs/http/session.md` |
| `yggdryl-http/src/{request,response}.rs` | `docs/http/request-response.md` |
| `yggdryl-http/src/stream.rs` | `docs/http/stream.md` |
| `yggdryl-http/src/cookies.rs` | `docs/http/cookies.md` |

Rules (treat them like the cross-language replication rule — a change is not done
until the docs match):

- **When you add or change behaviour, update the matching doc page** in the same
  commit, keeping the code↔doc mapping above intact. A new module/type gets a new
  page added to the `nav` in `mkdocs.yml` mirroring its code location.
- **Every code example is a synced language tab block**, in this order and with
  these exact labels (so Material's linked tabs switch the whole page at once):
  `=== "Python"` then `=== "Node"` then `=== "Rust"` (4-space-indented fenced
  block under each). Never write raw, one-after-another per-language sections.
- Keep examples **accurate to the current API** (the same surface the bindings
  expose); prefer copy-runnable snippets.
- **Doc build check** (add it to the gate when you touched docs):
  `pip install mkdocs-material && mkdocs build --strict` must pass (strict catches
  broken links and missing nav pages).

## Required checks before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
mkdocs build --strict   # when docs/ or mkdocs.yml changed (pip install mkdocs-material)
```

All must pass.

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

The Python extension is built against PyO3's **stable ABI** (`abi3-py37`), so one
`cp37-abi3` wheel per OS/arch covers every CPython from **3.7** up
(`requires-python = ">=3.7"`) — don't build a wheel per interpreter version. Keep
new binding code within the limited API (the PyO3 `*_bound` helpers already are).

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
5. **Docs in sync** — the matching `docs/` page (per the code↔doc mapping in
   [Documentation](#documentation)) reflects the new/changed behaviour, with
   synced Python/Node/Rust language tabs; `benchmarks/README.md` is updated if the
   numbers moved.

If any point fails, fix it before committing.
