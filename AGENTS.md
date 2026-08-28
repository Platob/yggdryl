# Yggdryl agent contract

Rust is the source of truth for `DataType`, `Field`, `Scalar`, identifiers,
I/O, codecs, and generic enums. Python and JavaScript are native runtime views,
not independent models.

## Operating mode

### No backward compatibility

- Project-owned APIs and encodings have one current contract. A change deletes
  the replaced symbol, parser spelling, serialized shape, fallback branch,
  test, and documentation in the same change.
- Never add or retain deprecated aliases, shims, migration readers/writers,
  dual behavior, warning periods, or legacy compatibility code. Update every
  caller directly.
- External standards remain supported only where the current contract names
  the standard and version explicitly. Describe that behavior by its protocol
  or version, never as a project compatibility layer.

### Compress everything

- Be concise by default. Keep only facts that change a decision, prove the
  result, identify a blocker, or enable the next action.
- State each rule and fact once. Remove greetings, praise, throat-clearing,
  repeated context, narrated tool use, generic reassurance, and sign-offs.
- Lead messages with the outcome. The default shape is: state, evidence,
  next action. Omit empty parts.
- Send progress only when work starts, material state changes, a blocker is
  found, or validation completes. Do not restate the request.
- Handoffs and compactions use only `Goal`, `Invariants`, `State` (decisions and
  paths), `Checks` (command and exact result), `Blockers`, and `Next` (exact
  action/command). Omit empty keys plus resolved or stale history.
- Brevity never removes contracts, safety boundaries, error semantics, edge
  cases, verification results, or material uncertainty.

### Optimize documentation for lookup

- Put the contract first, then the smallest runnable example, non-obvious edge
  cases, and measured performance.
- Use canonical symbol names, stable headings, short paragraphs, and compact
  tables only for exact mappings. Keep examples beside the API they prove.
- One fact lives in one place. Link to it instead of paraphrasing it. Do not
  narrate signatures, repeat examples in prose, add marketing text, or create
  benchmark-only pages.
- Optimize source docs and Markdown for fast human scanning and reliable LLM
  retrieval. Preserve exact commands, environment, results, and assertions.

### Keep code simple

- The simplest correct implementation wins: direct control flow, one source of
  truth, existing generic traits/types, and minimal state.
- Add an abstraction only when it removes real duplication or enforces an
  invariant. Prefer object or trait methods over isolated helpers when the
  behavior belongs to a value.
- Delete dead branches and redundant wrappers while touching an area. Do not
  add speculative generality or binding-side implementations of core logic.
- Comments explain non-obvious constraints, ownership, bounds, or safety; they
  do not translate the code into prose.

## Delivery order

1. Implement the Rust core, edge cases, tests, docs, and exchange-format
   interop first.
2. Stabilize the core contract before adding Python and JavaScript redirects.
3. Add parity tests and boundary benchmarks in each extension.
4. Run the required checks and report only results, failures, and material
   caveats.

A Rust-only phase is complete work when the core is the requested scope. Never
pin an unsettled design by implementing a binding first.

## Architecture

### Workspace and ownership

- Root owns workspace pins and lints. Members are `rust/`, `python/`, `node/`,
  each with `src/`, `tests/`, and `benchmarks/`. Runnable examples live in
  docs; no `examples/` directories.
- A non-null Struct `Field` is the only row schema. Rows canonicalize to ordered
  `Scalar::Sequence`; `Scalar::Record` is a sorted name-to-scalar input shape,
  not a second schema. Do not add another row/schema class or schema accessor.
- `rust/src/datatype/` and `rust/src/field/` own categorized schema behavior:
  state, parser, serde, comparison, Arrow, casting, value validation, typed
  markers, and datatype-family modules. Modules own implementation, not empty
  facades around a monolith.
- `Field` alone owns metadata and cache-aware mutation. `DataType` has no
  metadata. Protocol metadata is inert `<scheme>:<property>` text in one map;
  protocol views borrow it. `PARQUET:field_id` is the reserved typed exception.
- All shared and dispatch enums live directly in `rust/src/generic/` and are
  re-exported from `generic`: `Codec`, `DataTypeId`, `DataTypeKind`,
  `EdgeAlgorithm`, `IOKind`, `IOMode`, `Level`, `Magic`, `MediaType`,
  `MimeType`, `Scheme`, `TimeUnit`, `TimeZone`, `UnionMode`. No local copies or
  top-level `enums` module.
- `rust/src/generic/` also owns `Scalar`, `Holder`, `Media`, `RecordOptions`,
  `IORecordOptions`, and `RecordSettings`. Dispatch enums delegate complete
  contracts and add no variant-specific public vocabulary.
- `IOMode` modes are `ReadOnly`, `Overwrite`, `Append`, `Merge`, and `Random`;
  operations reject modes that do not apply. No alternate alias.
- `rust/src/io/` owns the single `IOBase` storage trait, `Buffer`, transparent
  `Coded`, partition routing, and byte streams. `rust/src/buffered/` owns
  `Buffered<H>` only.
- `rust/src/text/line/` exclusively owns `Text<H>`, `TextLine`, and
  `TextLineOptions`. Splitting occurs once; regex is the only structuring
  mechanism. It does not create another storage trait.
- `rust/src/{gzip,zlib,zstd}/` each own `load`, `dump`, `reader`, `writer`, and
  an `IOBase` wrapper. `Codec` is the only dispatcher.
- Storage backends are sibling module folders containing `Path`, `Folder`, and
  `File`. `rust/src/local/` is memory-mapped local storage; remote backends do
  not change `io` or `local`.
- Arrow interop lives in `rust/src/arrow/`; recursive cast planning stays with
  `Field`. The default `arrow` feature is optional for schema-only callers.
- `rust/src/{ipc,parquet,avro}/` each own free functions over `IOBase` plus a
  stateful wrapper. Parquet is feature-gated. Avro's scalar codec is
  unconditional; its record surface uses Arrow. Iceberg uses these codecs.
- `rust/src/iceberg/` separates types, schema, partition, snapshots, metadata,
  manifests, statistics, scalar rendering, scan, table, options, catalog,
  evolution, and inspection. A table format sits on record encodings.
- URI/URL/URN live below `rust/src/uri/`. Shared structured text lives below
  `text`; JSON/YAML/TOML live in their own modules over `Scalar`.
- Bindings mirror core domains. Their crate `lib.rs` files contain only
  boundary helpers, exports, and registration. Python annotation behavior is
  Python-only; all schema and scalar semantics remain native.

### Generic scalar

- `generic::Scalar` is the single cross-platform scalar. Do not add a parallel
  language value tree or retired alias.
- Variants match native/Arrow widths: real `F16`, `F32`, `F64`; `D128`,
  `D256`; `Date32`, `Date64`; `Time32`, `Time64`; `Duration32`, `Duration64`;
  and one `DateTime64`. Temporal values retain the `TimeUnit` and `TimeZone`
  needed by their datatype; `DateTime64` always has a non-null `TimeZone`,
  using `TimeZone::Naive` explicitly.
- `Scalar::Record` stores a deterministic sorted name-to-`Scalar` map.
  Struct-field canonicalization resolves it to an ordered sequence.
- Enum scalars preserve their generic enum identity while using the smallest
  fitting integer representation where the datatype makes that lossless.
- Implement `Clone`, `Debug`, canonical `Display`, `Eq`, total `Ord`, `Hash`,
  serde, arithmetic, and conversion traits wherever semantics exist. Floating
  equality/order/hash must be mutually consistent. Unsupported arithmetic is
  explicit, never a panic.
- Every immutable public wrapper in Rust and both extensions is hashable when
  its state has stable equality. Mutable copy-on-write wrappers become
  unhashable after mutation only when required by the target language.
- Scalar byte/text/JSON access uses the canonical core codec. Borrowing methods
  never allocate; allocating projections use `into_*`. No JSON bridge for
  Arrow or records.
- Shared nesting uses immutable references; empty collections avoid backing
  allocation. Caller-controlled input never reaches `unsafe`, `unwrap`, or a
  panic.

## Public vocabulary

Names describe ownership and return type; aliases with alternate verbs are
forbidden.

- `new`: infallible construction from native parts.
- `from_*`: construct or parse a named representation; validate when needed.
- `into_*`: return another representation, borrowing or consuming as useful.
  No project-defined plain `to_*`; foreign protocols (`ToString`, JS
  `toString`) keep their conventional spelling.
- `as_*`: borrowed, allocation-free view.
- `is_*` / `has_*`: side-effect-free predicates.
- `get*`: borrowed lookup; `get_mut` only when validation/caches cannot be
  bypassed.
- `set_*`: validated in-place update; failure leaves self unchanged.
- `with_*`: consuming update; use `try_` only when the paired setter can fail.
- `clear_*`: clear a category. `remove_*`: remove one item.

Implement standard `From`, `TryFrom`, `FromStr`, and `AsRef` where coherent;
bindings redirect through stable inherent methods.

Canonical core spellings:

- `DataType`: `from_str`, `from_arrow`, `from_json`, `from_fields`,
  `into_arrow`, `into_json`, `as_fields`, `default_value`,
  `is_default_value`, `into_scheme_compat`, `dense_union`, `decimal`, `time`.
- `Field`: `from_parts`, `from_str`, `from_arrow`, `from_arrow_ref`,
  `from_json`, `into_arrow`, `into_arrow_ref`, `into_json`, `default_value`,
  `into_scheme_compat`. Use `field`, never `schema`, in options and accessors.
- `Metadata`: `new`, `from_entries`, `from_arrow`, `from_json`, `into_arrow`,
  `into_json`, `get`, `contains_key`, `iter`, `protocol`.
- `Uri`/`Url`/`Urn`: `from_str`, `from_path`, `from_uri`, `into_json`,
  `into_uri`; `Uri` adds `into_url`/`into_urn`, file values add `into_path`.
- Structured text: `from_utf8`, `from_bytes`, `from_reader`, corresponding
  `_all`/iterator/inferred forms, and `into_utf8`, `into_bytes`,
  `into_writer`. JSON/YAML/TOML mirror it without a format argument.
- `TypedScalar<K>` is one validated `Scalar` plus a datatype marker. It owns
  no `Field`; Arrow projection routes through the core scalar-array boundary.

`DataTypeId` names an exact variant; `DataTypeKind` names a family. `TimeUnit`
is the only temporal/interval unit parser and Arrow converter. `MimeType` and
`MediaType` own MIME parsing, suffix/content-coding inference, and preferred
extensions. `Codec` owns coding dispatch. `Scheme` owns URI and compatibility
scheme vocabulary.

## Storage and I/O

### IOBase

- `IOBase` is positional: `pread`/`pwrite` are primitives. Whole reads,
  streams, compression, records, and media derive from them. No second storage
  trait or hidden cursor in the base object.
- Construction is lazy. Missing reads return empty/zero; writes create the
  resource and parents on first mutation. `media_type` is lazy and invalidates
  when bytes change.
- `pstream_bytes(position, batch_size)` yields bounded byte chunks from an
  explicit start with no retained page cache. `stream_bytes(batch_size)` owns
  only its cursor. Both are fused after error and suitable for codecs, text
  line reconstruction, structured parsers, and Arrow readers.
- Compressed streams retain only decoder state and the current bounded chunk;
  never retain prior pages. Line readers may keep only the temporary fragment
  needed to join a line across chunks.
- `IOKind` is authoritative. `is_container`, `is_atomic`, `is_tabular`, and
  `is_io` derive from generic kind/media behavior, not ad-hoc matching.
- `clear` empties but preserves a resource. `remove(recursive)` deletes it.
  Both issue the operation directly and map only backend not-found to success;
  no pre-probe. Wrappers also clear pending writes/caches so flush cannot
  resurrect deleted content.
- `pwrite` stages; `flush`/`close` publish. Whole operations such as
  `write_all_bytes`, `write_lines`, and `append_lines` flush on completion.
- `open` caches expensive metadata for its scope; `close` publishes and drops
  it. Closed reads are fresh. Wrappers use `delegate_iobase!` and override only
  changed behavior.
- `Buffered<H>` is idempotent, bounded by bytes and last-access TTL, writes
  through, invalidates touched pages, and pins the first and current final page.
  No cache crate or background thread.

### Existence and iteration

- EAFP everywhere: act once and use the typed result. Never guard an operation
  with `exists`, `is_dir`, `contains`, `mkdir`, `ensure`, or ancestry walks.
- Creation is a write consequence. Normalize backend absence/conflict once at
  the boundary. Repair absence and retry the original act at most once.
- `create` derives conflict from the create attempt; `open_or_create` absorbs
  that conflict; `get` raises absence. Public existence queries are answers,
  never internal guards.
- `IOBase` has no compare-and-swap. Concurrent creation may converge or return
  typed conflict, never silently select or corrupt.
- Resource-sized listings are deterministic lazy iterators of `Result`, fused
  after first error. Recursive walks hold a bounded frontier, not results.
  Owned reports are allowed only when bounded by the operation.
- Object-safe traits return one named iterator type per item kind. Python and
  JS expose native lazy protocols without collecting. Benchmarks include time
  to first item and full drain.

### IOMedia and records

- `IOMedia` owns field/datatype, record, Arrow, expression, applier, row/column
  size, and specialized tabular behavior. `IOBase` implements it; no separate
  tabular trait.
- Primitive Arrow methods are `read_arrow_reader(options)`, required
  `overwrite_arrow_reader(reader, options)`, default streamed
  `append_arrow_reader`/`merge_arrow_reader`, and generic
  `write_arrow_reader(reader, options, mode)`.
- Table, record-batch, and row-record entry points infer/wrap input and use the
  same reader pipeline. Nothing streamable accepts/returns `Vec` batches.
- `options.field` is the only declared datatype/schema. Reads project in the
  encoding and cast each batch. Writes cast incoming batches once, pop the
  field before delegating to overwrite, and never materialize the stream.
- `options.commit_row_size`: unset commits once; non-zero `N` publishes each N
  rows and the final remainder, retaining at most one bounded commit. First
  overwrite commit overwrites; later ones append. Failure leaves published
  prefixes visible.
- Overwrite replaces rows under the stored field; append retains stored rows;
  merge requires non-empty keys, updates matches, appends misses, and streams
  incoming batches. No positional upsert.
- `row_size` and `column_size` are lazy cached metadata and use cheap media
  answers without forcing a full read.
- Encoding comes from `MediaType` through `RecordOptions`; no format argument.
  Generic `write_*` accepts `IOMode` and redirects to specialized core paths.
- Content coding belongs to the handle. Reject outer compression for formats
  such as Parquet that compress internally.

### Paths and partitions

- Globs use `Url::is_glob`, `glob_parts`, and `matches_glob`; descend fixed
  prefixes before listing.
- Hive paths use `Url::hive_partitions`, `hive_partitions_under`, and lazy
  `children_where`. A table-format folder routes through its metadata before
  ordinary leaves.
- Stored `column=value` layout is authoritative; otherwise marked root fields
  decide partitions. Contradictions are typed errors naming both declarations.
- `io::partition::partition_text` is the only partition renderer. Partition
  columns move between paths and rows through one typed implementation.

## Media and table formats

- A media wrapper delegates raw bytes and implements the shared `IOMedia`
  primitives. Metadata caches exist only between `open` and `close`; metadata
  reads never decode rows.
- Declared fields drive native projection and one shared cast plan. Exact casts
  reuse arrays. Reads/writes remain streamed; held state states its bound and
  reason in a comment.
- Benchmark release builds against a trusted native implementation on the same
  payload and wire. Regenerate results; never edit numbers.

Iceberg contract:

- A table is a folder accessed only through `IOBase`; no direct filesystem.
  Metadata uses core JSON, manifests core Avro, data core Parquet. Do not add
  Iceberg/Avro/catalog dependencies while their I/O/Arrow model conflicts.
- Plan snapshot -> manifest list -> manifest -> files from metadata, never by
  walking `data/`. Partition summaries and safe column statistics prune; row
  filtering handles residuals. Report read/skipped counts.
- `Table` exposes the same `IOMedia` methods as a leaf. Merge reads only files
  whose key bounds may overlap and carries every other file unchanged.
- Manifest tuples are authoritative partition values; paths are layout and a
  text fallback only. Only invertible partition transforms write rows; reject
  unsupported transforms by name.
- Emit bounds only where Parquet and Iceberg encodings agree. Missing bounds
  cost performance; wrong bounds violate correctness.
- `SchemaUpdate` owns evolution. Promotions are Int32->Int64, Float32->Float64,
  and same-scale decimal precision widening. Preserve IDs; never reuse dropped
  IDs. Validate loaded metadata and every commit.
- Every retained snapshot is complete. Time travel uses snapshot/ref methods
  under its stored schema. Inspection returns record batches under canonical
  PyIceberg column names.
- Catalog, namespace, and table collections share `get`, `create`,
  `open_or_create`, `contains`, lazy iteration, `len`, and `is_empty`. Dotted
  names resolve in collections. Metadata writes create ancestry; no pre-checks.
- All options live in `IcebergOptions` and resolve explicit -> table property
  -> default. One resolver per key; resolve only consulted keys.
- One retry gate rechecks versions with bounded full-jitter backoff. Append and
  metadata-only commits may rebase; overwrite/merge/compact never rebase and
  restore state on conflict. Failed commits may leave only unreferenced files.
- Branches/tags are snapshot metadata with retention. `main` never expires.
  Non-main writes remain unsupported until commit parenting supports them.
- Parallel scans honor configured thresholds/width and emit plan order. The
  sequential and parallel paths differ only in speed.
- Validate exchange formats both directions against an outside implementation;
  a skipped half is not a pass.

## Datatypes, fields, parsers, and errors

- `DataType::from_str` and `Field::from_str` are the recursive schema grammars;
  bindings pass expressions directly. Accept canonical plus common Arrow/SQL/
  Hive/Spark forms with an explicit recursion limit.
- Split only at top-level separators while honoring balanced wrappers,
  quoting, and escapes. Reject trailing tokens, duplicates, malformed numbers,
  and invalid nullability with byte position and context.
- `variant(...)` is dense-union input sugar; generic decimal/time constructors
  select the fitting width then use the explicit implementation.
- One core URI parser owns components and suffixes. Canonicalize schemes and
  file paths platform-independently; validate percent escapes and report byte
  offsets. Bindings never split identifiers.
- Parse credentials by splitting authority user info at the first `:` only, so
  passwords may contain `:`. S3 authority inference treats the first path part
  ending in `.com` or `.io` as a hostname; otherwise it is the bucket. Infer
  region lazily from recognized AWS hosts.
- Errors contain expected, actual, and location: nested path, byte position,
  batch index, or URL. Use typed variants, canonical formatting, bounded user
  text, and the shared diff renderer. Mutations fail atomically.
- `yggdryl::Error` owns core failures; `yggdryl::arrow::Error` wraps runtime
  boundaries; external sources preserve their chain. Bindings preserve native
  messages and map only exception type.

## Structured codecs

- JSON/YAML/TOML parse bytes/slices/readers and emit bytes/writers over
  `Scalar`. String conveniences reuse the same parser without an intermediate
  serialization.
- Emit ordinary native shapes only. No tags, envelopes, version markers, or
  private wire representation. Reject kinds a format cannot represent.
- Parsing accepts an optional `Field` to type natural strings, order records,
  and validate/canonicalize in Rust. Without it, return only types proven by
  the document.
- YAML ignores tags as annotations. TOML follows its native root/table,
  integer, date/time, and single-document limits; unsupported values fail.
- Limits bound bytes, depth, nodes, documents, aliases, and hard recursion.
  Errors name format and byte position.
- Streaming iterators/writers process one item at a time with backpressure and
  fail at the failing item. Async buffering, if unavoidable, is bounded and
  documented.
- Inference is deterministic: explicit format, then path suffix; byte-like is
  content; string is a path only when it names an existing file. Content parse
  order is JSON, TOML when complete/non-empty, then YAML. Never infer JSONL
  from content.
- Placeholder substitution walks parsed `Scalar`, uses a closed grammar, and
  requires separate opt-ins for substitution and environment access.
- Benchmark slice, stream, writer, field-directed, wide, and deep paths with
  allocation baselines.

## Arrow and allocation

- One sealed zero-sized marker per datatype variant. `TypedField<K>` owns one
  `Field`; borrowed forms own one pointer. No duplicated state or unchecked
  mutable path that can invalidate the marker.
- Arrow schema parity is lossless. The C Data Interface routes only through
  core recursive `DataType`/`Field` exporters; bindings never build schemas
  recursively.
- Cache complete Arrow projections. No-op mutations retain caches; effective
  mutations invalidate once. Cache state never affects equality, hash, serde,
  or display.
- Core scalar creation, getters, lookup, iteration setup, and shared nesting
  clones do not allocate. Validate claims with the counting allocator at
  multiple corpus sizes, not timing alone.
- Root fields validate once and cache. Rows use `validate_value` and
  `canonicalize_value`. Named records reject missing, extra, duplicate, and
  non-string keys before committing.
- `yggdryl::arrow` owns Struct scalar/array, batch, reader, and IPC conversion.
  It is exhaustive and field-directed, holds at most one source batch, and
  never uses JSON.
- `arrow::scalar_array` and `arrow::scalar_value` are the single scalar/array
  boundary. The exact `Field` controls nullability, dictionaries, and extension
  identity.
- `ArrowCast` owns recursive array/batch casting. Struct casts reconcile names,
  reject ambiguous folds, follow target order, fill valid missing fields, and
  preserve exact arrays after logical validation.
- Wrapper exposure propagates: hidden child failures/nulls remain hidden.
  Preflight slot and fixed-buffer budgets before allocating.
- IPC dictionary IDs are transport-local. Preserve native IDs in one reserved
  root sidecar, remove it on import, and reject unsupported nested dictionary
  layouts.
- Python uses the C Data Interface/PyArrow holders. JavaScript uses copied IPC
  and never claims zero-copy.

## Binding boundary

Both extensions:

- Reach every stable core domain. A missing binding is documented as Rust-only.
- Infer/cast once at the boundary, then redirect to the most specific native
  method. No duplicated parser, schema, suffix, codec, scalar, or record logic.
- Explicit scalar conversion pairs are Python `as_py`/`from_py` and JavaScript
  `asJs`/`fromJs`. Native and Arrow values map through `Scalar` losslessly when
  the target runtime can represent them.
- Coerce only documented wrappers, strings, path-like values, mappings, native
  language scalars, enums, and Arrow values. Never stringify arbitrary objects.
- Preserve argument order/defaults/error semantics across Rust, Python, and JS.
- Bind scope protocols to `open`/`close`; keep no binding-side cache.
- Each public binding method has unit/edge tests, a boundary benchmark, and a
  docs entry.

### Python

- Use Python protocols and native types. Immutable wrappers implement stable
  equality/hash/order/pickle/copy/repr. A mutable wrapper with equality must
  follow Python's hash contract.
- `IOBase`/`Url` are `pathlib`-shaped but core-backed. No modes or cursor state
  are invented in Python.
- Annotation/dataclass inference builds native fields directly, never PyArrow
  schemas merely to import them again.
- The public decorator is `@scalar`; the pure field builder is
  `field(value, name=None)`. The decorator is colocated with the Python
  `Scalar` boundary; typed field factories remain below `yggdryl/fields/`.
- `@scalar` forwards every stdlib dataclass option and installs one cached
  argument-free `staticmethod field()`. It rejects a pre-existing `field`
  member. No static metadata constant is reserved.
- `Class.field()` returns one frozen non-null Struct `Field`, preserves
  dataclass order/metadata, excludes `ClassVar`/`InitVar`/private working
  annotations, resolves forward/generic annotations once, detects recursion,
  and is synchronized on first access.
- Optionality defines default nullability; explicit annotation options win.
  Defaults/factories affect construction, not schema. Generated dataclasses
  derive annotations from the exact native field graph.
- Do not expose a second row decorator/class, a static field constant,
  schema/into-field aliases, or any retired public surface.
- `pyarrow.RecordBatchReader` is the primitive record shape. Table, batch, and
  dataclass row methods redirect through it over the C Stream interface.
- Structured codec facades remain byte-oriented and native; `cls=` is explicit
  reconstruction. Encoders never close caller-owned streams.

### JavaScript

- Use camelCase at the boundary only. Support JS equality/comparison/hash
  helpers, cloning, child iteration, and `Map`-like metadata over native state.
- Record helpers close over one native Struct `Field`; nested structs reuse
  cached layouts. No per-row schema/map/JSON bridge.
- `BatchReader` is the one-shot primitive. `BatchReader.from` accepts readers,
  Arrow JS tables/batches, batch arrays, or IPC bytes; `intoIpc`/`intoTable`
  drain it. One batch crosses as one self-contained IPC stream.
- Arrow JS interop is copied IPC with bounded cursors and validated cached
  schema. Public IDs are transport-local; native records keep canonical IDs.
- JSON/YAML/TOML facades are byte-first over native `Scalar`; preserve `bigint`,
  bytes, Date, arrays, plain objects, maps, sets, and explicit class targets.
- Before N-API recursive conversion, create one bounded detached plain-data
  snapshot and reject cycles, proxies, accessors, symbols, depth, and node
  overflow. Keep the recursive depth ceiling at 48 until traversal is iterative.
- Reserved identities are `javascript:builtins.<Name>`,
  `javascript:<application>`, and `yggdryl:<native>`; detect native identity,
  never `constructor.name`.

## Documentation layout

- Root `mkdocs.yml` is authoritative; strict build, nav, and links change
  together. README is a short landing page.
- One page per core module. Generic enums and `Scalar` share `docs/generic.md`;
  no separate enums page. Extension pages document boundaries only.
- Every page starts with one H1 and one purpose sentence. Media pages present:
  batch read/overwrite/append/merge, row/table adapters, raw text access where
  relevant, then embedded benchmark results.
- Every supported example uses tabs in Rust, Python, JavaScript order with the
  same operation expressed idiomatically. Show Rust-only explicitly; never
  invent a binding.
- Text docs include `Text`, line iteration, raw streaming, and batch read/write
  examples. JSON/YAML/TOML examples use the shared `Scalar` and optional field.
- Every code block is self-contained with an assertion and runs through
  `scripts/check_docs_examples.py`; ignored blocks use valid superfence syntax
  and are reported.
- Notebooks are generated from doc blocks; edit source blocks, not notebooks.
- Benchmark tables live on the method/module page, name machine/runtime/build,
  compare a trusted baseline, and are generated from release runs.

## Verification

Before handoff, run what the touched surface requires and report exact skipped
checks:

- formatting and warning-free Clippy;
- workspace tests with default features and `parquet iceberg`;
- Rust 1.85 default and `--no-default-features --lib` checks;
- rustdoc with warnings denied;
- relevant parser, codec, text, I/O, Arrow, and interop benchmarks;
- Python native/codec/parity tests and release boundary benchmarks;
- Node native/codec/type/parity tests and release boundary benchmarks;
- docs examples and `python -m mkdocs build --strict`;
- dead-code, duplicate-logic, retired-symbol, stale-doc, and Rust-only binding
  sweeps.

Remove only generated targets, site output, virtual environments, binaries,
caches, and `node_modules` created by validation. Preserve unrelated user work.

## Releases

- `main` triggers `.github/workflows/release.yml`; `v<version>` is the receipt
  created after registry publication. Manual workflow runs rehearse only.
- Publishes are idempotent and the tag is last. Repair partial publication by
  rerunning; never reuse a released version.
- Root Cargo, Python, and Node versions match exactly. Publish crates.io, PyPI,
  and npm only after platform smoke tests import and exercise the artifacts.
- Credentials stay in repository configuration: Cargo/npm secrets and PyPI
  trusted publishing. No stored PyPI password or fourth registry.
