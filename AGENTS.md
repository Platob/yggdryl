# Yggdryl implementation rules

Yggdryl is a schema, resource-identifier, and structured-codec core. The native
public domain model is `DataType`, `Field`, `Uri`, `Url`, `Urn`, and the codec
`Value`; Python and JavaScript are runtime views of those Rust values, never
independent schema or codec implementations. Python records are a convenience
layer compiling annotations into native values.

## Order of work

- **Rust first, fully: implementation, edge-case tests, documentation with
  running examples, and - for an exchange format - a check against an outside
  implementation. Only then bindings.** A binding against an unsettled core
  pins decisions the core never made. A change that touches `rust/` and stops
  there is complete work, not half of one.

## Source layout and scope

- Root owns the workspace manifest, shared pins, and shared lints; members are
  `rust/`, `python/`, `node/`, each with `src/`, `tests/`, `benchmarks/`. No
  `examples/` directories - runnable examples live in the docs.
- **A struct `Field` is the schema.** A non-null `Struct` field describes rows;
  a row is one ordered `Value::Sequence` with a value per child. Never
  reintroduce a `Record`/`RecordSchema` pair.
- Schema behavior lives in categorized `rust/src/datatype/` and
  `rust/src/field/`: generic state and mutation in `field/mod.rs`, Arrow in
  `field/arrow.rs`, typed casting in `field/cast.rs`, grammar in
  `field/parser.rs`, serialization in `field/serde.rs`, comparison in
  `field/diff.rs`, row validation in `field/value.rs`, compile-time markers in
  `field/typed.rs` with per-family modules (`scalar`, `integer`, `floating`,
  `temporal`, `decimal`, `binary`, `nested`). Split `datatype/` the same way
  (scalar/integer/floating/temporal/nested, parsing, Arrow, serialization);
  modules own real implementation, never empty shells around a monolith.
- Immutable metadata value in `rust/src/metadata.rs`; cache-aware mutation
  stays on `Field`. Metadata is owned and validated by `Field` only - never on
  a bare `DataType`, never a binding-side metadata model.
- Shared enums below `rust/src/enums/`: `DataTypeId`, `DataTypeKind`,
  `MimeType`, `MediaType`, `Scheme`, `TimeUnit`, `UnionMode`, `Codec`,
  `Level`, `IOKind`. Reuse them; no local copies. One `TimeUnit` owns temporal
  resolutions and Arrow interval layouts. `MimeType`/`MediaType` own MIME
  spelling, suffixes, content-codings, compound-filename inference, and the
  file-system types `inode/directory` and `inode/file`.
- Generic enums below `rust/src/generic/`: `Holder` over every `IOBase` impl,
  `Media` over every bound record encoding, `RecordOptions` over every
  encoding's settings. A generic enum delegates the whole contract to its
  variant and adds no behavior.
- Byte storage below `rust/src/io/`: the `IOBase` trait, in-memory `Buffer`,
  transparent-compression `Coded`. Shared record settings in
  `generic/options.rs` as `IORecordOptions` and `RecordSettings`.
- The page cache below `rust/src/buffered/`: `Buffered<H>` wraps any handle and
  serves reads from fixed-size pages under a byte budget and a time to live
  counted from last access, writing through and invalidating what a write or a
  resize touched. The value's first page and the page holding its last byte are
  *pinned* - exempt from eviction and expiry - because both ends are where
  discovery lives, and the pin follows the current end. Wrapping is idempotent:
  `IOBase::buffered` is shadowed by inherent methods on `Buffered` and `Holder`,
  so a cache is never stacked on a cache. No cache crate, no background thread.
- The text-record surface lives in `rust/src/text/line/` and nowhere else:
  `Text<H>` is the wrapping handle (`into_text`, idempotent), `TextLine<'_>` a
  borrowed view over the window with lazily cached text, hash, and captures,
  and `TextLineOptions` the whole extractor - which is exactly what a JSON,
  YAML, or TOML document parses into, so a reader is specifiable from
  configuration alone. Regex is the only structuring mechanism; there is no
  format-string parser and no user callback. Splitting records happens in one
  place. It is a surface *beside* the three record methods, never a fourth
  one, and it never introduces a tabular/dataset type or a second storage
  trait.
- One module per content coding - `rust/src/{gzip,zlib,zstd}/` - each with
  `load`/`dump`, `reader`/`writer`, and a transparent `IOBase` handle (`Gzip`,
  `Zlib`, `Zstd`). `Codec` in `enums/codec.rs` dispatches; never a fourth
  spelling of a coding.
- Each file-system backend is one module folder with the same three roles: a
  generic `Path` that reports `IOKind` from what is actually there and routes
  to the fitting implementation, a `Folder` container, a `File` leaf.
  `rust/src/local/` is the local one (its `File` is a memory mapping). A
  remote backend (S3, GCS, Azure) is a sibling module - never a change to `io`
  or `local`.
- Arrow interop in `rust/src/arrow/` (Struct scalars, StructArray,
  RecordBatch, IPC, `schema_from_field`). Recursive casting lives with the
  schema in `rust/src/field/cast/` (plan engine `cast/plan.rs`, typed surface
  `cast/mod.rs`). `arrow` feature is default; schema-only callers use
  `default-features = false`. Never a separate Arrow crate.
- One module per record encoding: `rust/src/ipc/`, `rust/src/parquet/`
  (non-default `parquet` feature), and the record surface of `rust/src/avro/`
  (behind the default `arrow` feature; the Value-level codec beneath it is
  unconditional). Each owns free functions over any `IOBase` handle plus a
  stateful type (`Ipc`, `Parquet`, `Avro`) holding handle, options, cache;
  `IOBase`'s record methods dispatch to those functions.
- Apache Avro below `rust/src/avro/`: an unconditional codec module (it adds
  no dependency) reading and writing object containers as the shared `Value`
  over any `IOBase` handle. Iceberg sits on it, never the other way around.
- Apache Iceberg below `rust/src/iceberg/` (non-default `iceberg` feature,
  implies `parquet`), one file per concern: `types.rs`, `schema.rs`,
  `partition.rs`, `snapshot.rs` (snapshots and refs), `metadata.rs`,
  `manifest.rs`, `statistics.rs`, `value.rs` (Iceberg's text and
  single-value renderings of a scalar), `scan.rs`, `table.rs`, `options.rs`,
  `catalog.rs`, `evolve.rs`, `inspect.rs`. A table format sits on the record
  encodings; it never becomes one.
- No tabular descriptor, dataset, or in-memory table type: a handle plus
  `RecordOptions` is the whole surface; an in-memory table is a `Buffer`.
- URI/URL/URN in `rust/src/uri.rs`. Shared structured-text values, formats,
  limits, envelopes, display, byte positions below `rust/src/text/`; JSON,
  YAML, TOML below `rust/src/{json,yaml,toml}/`, all on the one shared
  `Value`. `rust/src/codec.rs` is a compatibility re-export facade only.
  `lib.rs` holds exports and shared error plumbing only.
- Bindings mirror core domains as `src/{datatype,field,media,uri,codec}.rs`;
  each `lib.rs` is boundary helpers, exports, registration only. Python-only
  annotation behavior below `python/yggdryl/records/`.
- Protocol metadata is inert string properties `<scheme>:<property>`; reuse
  `Scheme` for the prefix and one generic property API. No protocol
  execution, network clients, or duplicate per-protocol maps. A per-protocol
  *view* (remembers the prefix, reads the one shared snapshot) is not a
  duplicate map; modules with namespaced state (`iceberg:doc`,
  `iceberg:transform`, `iceberg:spec-id`) reach it through the view, not a
  private key constant. `PARQUET:field_id` is the reserved exception, owned by
  Field's typed ID API. HTTP representation metadata uses canonical lowercase
  `http:*` keys for both HTTP and HTTPS.

## Storage and I/O contract

- **`IOBase` is the one storage abstraction**, positional not cursor-based:
  `pread`/`pwrite` take an offset; everything else (streams, whole-value
  reads, compression, records) derives from those two. Never a second
  storage trait.
- **Handles are lazy.** Construction never touches the resource. Reads skip
  (`pread` on a missing resource yields 0 bytes, `size` reports 0); writes
  create (`pwrite`, `truncate`, `reserve` make the resource and parents on
  first use). `media_type` is computed on demand and re-derived after bytes
  change.
- `IOKind` (`Memory`, `File`, `Directory`, `Unknown`) is how a handle says
  what it addresses; `is_container` derives from it - never re-answer
  independently.
- **`clear` empties, `remove(recursive)` deletes, and neither pre-calls.** A
  leaf clears to zero bytes and still exists; a container clears to empty and
  still exists; `remove` leaves nothing of the resource - a wrapping handle
  removes what it *wraps*, plus the pending writes and caches it holds, so a
  later flush cannot resurrect it. Absence is a no-op success on both, and it
  is reached by issuing the delete and mapping the backend's own not-found
  answer (`NotFound`, `NoSuchKey`, 404) to `Ok`: never by calling `kind`,
  `size`, `ls`, or an exists check first, because on a remote backend every
  probe is a round trip and a recursive delete becomes a flood of them. Only
  not-found maps to success. A leaf ignores `recursive`; a non-empty container
  without it is refused naming the location. The return is `Result<()>` - a
  bool or a count would force exactly that probe. The one exception is a
  generic `Path`, which routes on the `IOKind` it already resolves.
- A wrapping handle mirrors the inner one via `delegate_iobase!`, overriding
  only what it changes (usually `open`/`close`) - so `Coded`, `Gzip`, `Ipc`,
  `Parquet`, `Media` expose raw bytes.
- `open` materializes and caches what repeated calls would re-derive (schema,
  footer); `close` publishes and releases. Bindings bind scope dunders to
  exactly these.
- **A positional write stages; a whole-value write publishes.** `pwrite` is a
  *piece* of a value, so a backend that buffers - an Arrow filesystem replaces
  whole files, a memory-mapped file grows geometrically - holds it until
  `flush` or `close`. `write_all_bytes`, `write_lines`, and `append_lines`
  each *are* an operation, so they flush when they finish: otherwise a second
  handle on the same location reads a pending value or the mapping's zero
  padding as content.
- **The record surface is exactly three methods**:
  `read_arrow_batch_reader(options)` -> `arrow::BatchReader`,
  `write_arrow_batch_reader(reader, options)` (replace or merge),
  `append_arrow_batch_reader(reader, options)`. Everything else routes through
  them, so exactly one place decodes and one encodes each encoding. Never
  reintroduce `overwrite_arrow_batches`, `append_arrow_batches`,
  `upsert_arrow_batches`, or partitioned spellings. Record reads/writes never
  take or return a `Vec`, slice, or borrowed iterator of batches - a shape
  requiring materialization cannot describe a resource larger than memory;
  `arrow::batch_reader` turns held batches into a reader. `record_options` and
  `read_arrow_field` complete the surface, answered through the three. The
  encoding is never guessed: it comes from the media type via
  `RecordOptions::for_media_type`; a container answers with its leaves'
  encoding. Shared settings (schema, root name, cast strictness, batch size,
  compression level, match key) are flattened fields on each
  `IORecordOptions` implementation.
- **A declared schema selects and casts during the read.** Stored columns the
  schema names become the encoding's own projection (Parquet
  `ProjectionMask`, IPC projection); results are cast to the declared shape
  per batch. Never read-everything-and-drop, never leave the cast to the
  caller. A projection only drops columns; reorder, convert, and absent
  columns are the cast's business. Parquet's projection skips locating and
  decoding chunks; IPC's saves decode and allocation, not bytes - say so.
- **A write is a replacement or a key-matched merge, nothing else.**
  `merge_by_names` empty = overwrite (declared schema applied to incoming, then
  safe-cast to the stored schema so rows are replaced, not columns
  redefined). Non-empty = merge: stored rows read, incoming joined one batch
  at a time, matched keys update, unmatched append, result rewritten. The
  join streams over the incoming side; whatever must be held says why in a
  comment. No positional upsert: a position is not a row identity.
- Globs: `Url::is_glob`, `glob_parts`, `matches_glob` (`.gitignore` rule: no
  separator matches at any depth, a separator anchors at the root). A glob is
  folder-like before the backend is touched; `IOBase::glob` descends every
  fixed prefix before listing.
- Hive paths: `Url::hive_partitions`, `hive_partitions_under`, and
  `IOBase::children_where` (yields leaves only). A container handle reads
  across every leaf of its encoding and routes each written row to the leaf
  its partition values name. A container holding a *table format* is asked
  first: a folder with an Iceberg metadata document reads/writes through
  snapshots, not leaves. The layout is the authority on partition columns:
  `column=value` leaves partition by those; otherwise the declared schema's
  partition-marked fields decide; neither means one table in one leaf. A
  declaration contradicting a stored layout is refused naming both.
  `io/partition.rs` moves partition columns between path and rows, typed as
  declared, left alone when data already carries them, `null` for absence
  (a declared nullable column is what turns that text back into null). A
  restored partition column carries the partition marker.
- One renderer spells every partition value: `io::partition::partition_text`
  (the encoding's display, `null` for absence), used by tables and folders
  alike. Never a second partition-text renderer.
- Content coding belongs to the handle: `trades.arrows.gz` round-trips
  compressed via `IOBase::codec`. Parquet compresses internally, so an outer
  coding on it is rejected, not double-compressed.

## Media implementation standard

Every record encoding (media) implementation - `Ipc`, `Parquet`, the next one -
meets all of these; a new media that cannot yet is not done:

- **It is a wrapping handle first.** `Media<H: IOBase>` owns its handle,
  mirrors bytes via `delegate_iobase!`, and exposes the encoded value raw - so
  the file can be copied, uploaded, or handed to a foreign reader without
  unwrapping. The encoding's smarts live behind the same three record methods
  as every other, never as extra public entry points.
- **Rich metadata, contextually cached.** Whatever the format keeps re-reading
  - IPC's schema, Parquet's footer, statistics - is answered from a cache
  *only between `open` and `close`*: open fills it, close releases it, and a
  closed handle fetches fresh on every ask. Nothing fills the cache as a side
  effect of an ordinary read; a cache nobody asked for is how a handle serves
  a stale answer after the resource changes underneath it. Metadata reads
  (schema, statistics, row counts) never decode rows.
- **Deep, flexible access.** The declared schema drives the encoding's own
  projection so unread columns are never decoded; `select_by_names` narrows
  and orders; option-driven casting has exactly one definition -
  `IORecordOptions::cast_arrow_batch` / `cast_arrow_reader` (declared schema,
  then selection, then completion onto the stored shape) - and a media routes
  through it rather than growing a private variant.
- **Full Arrow interop, streamed.** Reads hand back a `BatchReader` and
  writes take one; nothing that could stream is collected, and anything a
  caller holds (tables, datasets, scanners, frames, record instances) is
  widened *into* a reader at the binding boundary, never into a second write
  path. Casts preserve the caller's kind: a table casts to a table, a lazy
  frame stays lazy (schema via `collect_schema`, work mapped over the
  engine's batches), a polars crossing uses the newest compat level so view
  arrays stay view arrays.
- **Easiest usage is the default path.** The encoding is derived from the
  media type, never passed as a format argument; options default from the
  handle; absence reads as empty and writes create - so the happy path is a
  constructor and one method call, with no existence checks, no format
  strings, and no mode flags anywhere.
- **Internal optimizations are invisible and stated.** Exact-schema casts
  return the same arrays; an already-open handle's `open` is a no-op; a
  memory-bound step (a merge side, a footer) says *why* it is held in a
  comment. An optimization that changes observable behavior is a bug, and a
  silent cap (top-N, sampling) is worse than none.
- **Benchmarked against something the reader trusts** (the stdlib, PyArrow,
  the native library), in release builds, with the numbers regenerated -
  never edited - into `docs/benchmarks.md`.

## Table format contract

- **A table is a folder, reached through `IOBase` only.** `Table` finds
  metadata, manifest lists, manifests, and data files with `child_by`/`ls`;
  no paths, no `std::fs`; recorded absolute locations become relative names
  first. Catalog-free location works like `HadoopTables`:
  `metadata/version-hint.text`, else highest-numbered `*.metadata.json`.
- **No dependency for the format itself.** Metadata is JSON via
  `rust/src/json/`; manifests are Avro via `rust/src/avro/`; data files are
  what `rust/src/parquet/` wrote, with statistics from that footer. Never add
  an Iceberg/Avro/catalog crate or `serde_json` here. The published `iceberg`
  crate was rejected: it pins arrow/parquet 55 vs this workspace's 59, storage
  goes through `opendal` features not `IOBase`, and it is async while `IOBase`
  is sync; re-evaluate only if all three change.
- **A scan is planned from metadata, never a listing.** Snapshot -> manifest
  list -> `FieldSummary` skips manifests, partition tuples skip files, column
  bounds/null counts skip more. `Table::plan` reports read vs skipped so
  pruning is a testable number. A filter is a `(column, value)` text pair
  (same vocabulary as `children_where`), compared as text for `identity`
  partition columns and as the cast value elsewhere; statistics bound files,
  rows are filtered afterwards. Never scan by walking `data/`.
- **A table answers the same three record methods as a folder**: read through
  the snapshot, write/append as one commit each; `merge_by_names` upserts a table
  exactly as a leaf; a handle on a `column=value` directory addresses that
  partition. `Table` itself implements `IOBase` - bytes delegated to its root
  folder via `delegate_iobase!`, record surface overridden to answer from the
  parsed metadata: `read_arrow_field` is the stored schema with its field ids
  (no file opened), `filter_partitions` prunes files through the plan (and a
  filter naming an undeclared column errors, unlike the tolerant folder route),
  commits update the value in place. The folder route reaches the same answers
  through the `located` probe; `Located::record_options` delegates to the
  table's, so the encoding answer is defined once. Merge reads only files whose key-column bounds overlap incoming
  keys; every other file is carried as an `existing` entry (same location,
  statistics, order) - correct however coarse the statistics, since an unread
  file keeps every row. The incoming side is held, and the comment says why.
- **The manifest is the authority on a partition value; the path is layout.**
  Tables write the same `column=value` directories through the one renderer,
  but scans take values from the manifest tuple: a path cannot distinguish
  the string `null` from absence. Compare the manifest value first, text as
  fallback.
- **Spec and schema say the same thing once.** `PartitionSpec::partition_field`
  stamps transform, source column, and the partition marker on tuple children;
  `from_field` reads a spec back; `mark_partitions` marks schema columns;
  `from_schema` builds an identity spec from them.
- **A transform that cannot place a row is refused by name.** Only `identity`
  and `void` are invertible; `bucket`, `truncate`, calendar transforms reject
  writes naming the transform. Reads are unaffected.
- **Statistics only where the encodings agree**: emit a bound only for types
  whose Parquet statistic bytes are the Iceberg single-value encoding, counts
  alone otherwise. A missing statistic costs one file read; a wrong one costs
  correctness.
- **A column change is a new schema via `SchemaUpdate`, gated by
  `can_promote`**: exactly Int32->Int64, Float32->Float64,
  decimal(P,S)->decimal(P',S) with P'>=P; all else refused naming both sides.
  New columns number above `last-column-id`, dropped ids never reused, renamed
  columns keep ids. `TableMetadata` owns the update vocabulary
  (`set_property`, `set_location`, `assign_uuid`, `upgrade_format_version`,
  `set_current_schema`, `add_spec`, `set_default_spec`, `add_sort_order`,
  `set_default_sort_order`, `set_snapshot_ref`, `remove_snapshot_ref`,
  `remove_snapshots`); `validate` runs on load and before every commit, so a
  broken document reads but never writes.
- **Every retained snapshot is a complete table.** Time travel:
  `scan_at`/`plan_at` by snapshot id, `snapshot_by_ref` for a ref, read under
  the schema it was written with. Metadata-only changes commit through
  `Table::commit_changes` (one new document; failure leaves memory
  unchanged). Inspection is `inspect_history`/`inspect_snapshots`/
  `inspect_files` as record batches under PyIceberg's column names; never a
  second struct-shaped spelling.
- **A catalog is a warehouse folder over `IOBase`.** `iceberg::Catalog` maps
  dotted names to nested folders, creates tables from partition-marked
  schemas, answers `append` (create-or-append) and `overwrite`. No network,
  no transaction protocol; REST catalog is future work behind an HTTP
  backend. Drop/rename are absent until the storage contract gains
  delete/move - name that reason, do not emulate.
- **One key names the file-size target**: `write.target-file-size-bytes`,
  falling back to the schema root's `iceberg:` spelling, then 512 MiB. Rolls
  a partition's stream at batch boundaries, sized by Arrow in-memory bytes
  (Parquet lands under target; docs say so). `Table::compact` rewrites
  small-file groups the same way, commits `replace` carrying untouched files,
  reports counts, no-ops without a commit when there is nothing to do.
  Unparseable size = typed error naming key and value, never a silent
  default.
- **Every knob is one field of `iceberg::IcebergOptions`, resolved
  explicit -> table property -> default**, keys in Iceberg's spellings
  (`commit.retry.num-retries`, `commit.retry.min-wait-ms`,
  `commit.retry.max-wait-ms`, `write.target-file-size-bytes`,
  `read.parallelism`, `read.parallel.min-files`,
  `read.parallel.min-file-size-bytes`); the property layer falls back to the
  schema root's `iceberg:` spelling; one resolver function per field. A
  `Table::set_options` override lives on the handle, is never written, and
  shadows its property without parsing it (broken values can be shadowed then
  repaired); each operation resolves only the keys it consults. No knob
  outside this value.
- **One retrying commit gate; what a retry may do is the operation's
  nature.** The gate re-checks the version, counts each newer one as beaten,
  full-jitter backoff up to `commit.retry.num-retries`. `append` and
  `commit_changes` *rebase* (data files and added-entries manifest written
  once; intent re-applies on the winner's document). `overwrite`, `merge`,
  `compact` never rebase (planned files may be replaced, input consumed):
  they wait, re-observe, and exhaust into a `CommitConflict` naming both
  versions, state restored. Say plainly wherever this surfaces: `IOBase` has
  no compare-and-swap - retries shrink the race window, cannot close it; a
  failed commit leaves at worst orphan data files no snapshot names.
- **Branches and tags are metadata with per-ref retention.** `SnapshotRef`
  carries `min-snapshots-to-keep`, `max-snapshot-age-ms`, `max-ref-age-ms`;
  `expire_snapshots` honors them before its cutoff; `main` never expires;
  fast-forward must reach the branch head by parent ids. Table conveniences
  (`create_branch`, `create_tag`, `remove_ref`, `fast_forward`,
  `expire_snapshots`, `scan_ref`) go through the retrying gate. Writing *to*
  a non-`main` branch is future work (a commit's parent is always the current
  snapshot) - name that limit, do not emulate.
- **A scan fans out only when the plan earns it, in plan order always.**
  Parallel decode requires `read.parallelism` >= 2 and >=
  `read.parallel.min-files` files of >= `read.parallel.min-file-size-bytes`
  (defaults 16, 4 MiB; parallelism = host clamped 1..=8); otherwise the
  sequential path. Workers decode and refine whole files, at most
  `read.parallelism` in flight; a reorder buffer releases in plan order, so
  parallel and sequential differ only in speed. Never let an optimization
  change what a caller observes; never exceed the configured width.
- **Exchange formats are validated against an outside implementation.**
  `scripts/check_iceberg_interop.py` + `rust/tests/iceberg_interop.rs` run
  the exchange with PyIceberg both directions and compare rows; the Rust half
  prints `SKIPPED` when the external table is absent (the driver fails on
  that word), so a skipped half can never read as a pass.

## Documentation organization

- Site from root `mkdocs.yml`, pages below `docs/`, strict build stays green;
  page adds/renames update the nav and links in the same change.
- Example-first: smallest runnable example, then only what it cannot show.
  Several focused examples over one oversized one.
- **One page per core module folder**: `docs/<module>.md` documents
  `yggdryl::<module>` and nothing else (`enums`, `datatype`, `field`,
  `arrow`, `io`, `expression`, `buffered`, `generic`, `local`, `gzip`,
  `zlib`, `zstd`, `ipc`, `parquet`, `avro`, `iceberg`, `uri`, `text`, `json`,
  `yaml`, `toml`).
  `docs/extensions/{python,javascript}.md` document only their boundary.
- Every page opens with one H1 and exactly one short sentence saying what the
  module is for.
- **Every example appears in Rust, Python, JavaScript tabs, in that order**,
  same operation, each idiomatic - Rust shows typed calls and `?`; Python
  uses its protocols (`len`, `in`, mapping dunders, `with`, real keyword
  defaults, bare `str`/`bytes`/`Path`/PyArrow where coerced); JavaScript uses
  `from` constructors, `Map` iteration, spread, `for...of`. Check
  `.api-bindings.txt` before showing a language do anything. A module the
  bindings do not expose shows Rust alone under `!!! note "Rust only"` (or
  "Rust first" for a pending surface); never fabricate a binding tab.
- Tests/benchmarks mirror core ownership: dispatchers
  `rust/{tests,benches}/{datatype,field}.rs` over category files; split typed
  cases only when each file owns real cases. Structured-text targets mirror
  `text/`/`json/`/`yaml/`/`toml/`. Keep stable Criterion group IDs when
  splitting. Arrow-runtime regressions:
  `rust/tests/{arrow_record,default_scalar,tabular,value_bounds}.rs`;
  benchmark targets stay `record`, `tabular`, `io` (io needs `parquet`;
  pushdown benchmarks report materialized bytes as throughput). Iceberg
  interop is test target `iceberg_interop` (needs `iceberg`). Extension
  tests/benchmarks live in that extension; no root extension examples.
- **Benchmark against something the reader trusts.** A performance claim in
  the documentation carries a baseline from outside the project on the same
  payload and wire: `python/benchmarks/compression.py` beside the stdlib
  codecs, the PyArrow IPC/Parquet baselines in
  `python/benchmarks/records_io.py`. Numbers in `docs/benchmarks.md` name the
  machine, interpreter, and build profile that produced them; a binding is
  timed only as a **release** build (`maturin build --release`,
  `napi build --release`), and a stale table is regenerated, never edited.
- **Every doc example runs**: `python scripts/check_docs_examples.py`
  compiles rust blocks as tests, runs python blocks under `python/.venv`,
  runs javascript blocks with `yggdryl` rewired to the repo. Each block
  is self-contained with at least one assertion; a block that cannot stand
  alone is tagged `ignore` in the brace form pymdownx.superfences highlights
  (a fence opened with `{ .<lang> .ignore }`), and reported, not hidden. The
  `<lang>,ignore` spelling is not one superfences reads: it renders as prose,
  backticks and all, so the checker fails on it.
- Notebooks under `docs/notebooks/` are generated by
  `python scripts/build_docs_notebooks.py` from the same blocks, committed,
  shipped unexecuted. Edit the block, never the notebook or the
  `<!-- notebooks: ... -->` region.
- `https://platob.github.io/yggdryl/` is the canonical guide; the README is a
  short landing page. Docs tooling stays in `requirements-docs.txt`, never a
  runtime dependency.

## Exact method vocabulary

Names follow ownership and cost; no aliases with different verbs.

- `new`: direct infallible construction from native parts.
- `from_*`: construct from a named representation; `Result` when validating.
- `into_*`: consume self into another owned representation, reusing
  allocations/caches.
- `to_*`: borrow self, produce an owned value (may allocate / bump an `Arc`).
- `as_*`: borrowed view, never allocates.
- `is_*`/`has_*`: side-effect-free predicates.
- `get`/`get_*`: borrowed lookup, no allocation; `get_mut` only when mutation
  cannot bypass validation or cache invalidation.
- `set_*`: validated in-place replacement; error leaves self unchanged.
- `with_*`: consuming persistent update; `try_` prefix only when the sibling
  `set_*` is fallible.
- `clear_*`: remove a category; `remove_*`: remove one keyed value, returning
  the prior when useful.

Implement `From`/`TryFrom`/`FromStr`/`AsRef` alongside the inherent
`from_*`/`into_*`; the inherent methods are the stable API bindings use.

### Stable core spellings

Keep exact; no alternate aliases:

- `Scheme`: uppercase constants for common protocols; `from_str` for
  arbitrary valid schemes; `as_str` borrowed canonical. It doubles as the
  compatibility vocabulary: `ARROW`, `SPARK`, `POLARS`, `PANDAS`, `ICEBERG`
  in `COMPATIBILITY_TARGETS`, accepted by `to_scheme_compat` (a non-target
  parses fine, is rejected there, never by the parser);
  `is_compatibility_target`, `is_storage`, `default_port`. Never a second
  scheme-like enum. The Iceberg target widens losslessly; the Iceberg codec
  stays strict (`PrimitiveType::from_data_type` refuses what the format
  cannot spell).
- `DataTypeId` vs `DataTypeKind`: `id` is variant identity (`int32`,
  `decimal128`), `kind` is the family (`integer`, `decimal`), `name` is
  `id().as_str()`. "Which variant" uses `id`; only family-uniform behavior
  dispatches on `kind`.
- `TimeUnit`: `from_str`/`FromStr` parse temporal and interval spellings
  (ASCII case-insensitive aliases; SQL `year(s)`/`day(s)` -> `YearMonth`/
  `DayTime`); `as_str`/`AsRef<str>` allocation-free; `is_temporal`/
  `is_interval` disjoint. Arrow: `from_arrow_time`, `from_arrow_interval`,
  `into_arrow_time`, `into_arrow_interval`; infallible `From` imports,
  fallible `TryFrom` exports; never silently cross temporal/interval.
  Serialize snake_case full names; deserialize via `from_str`.
- `MimeType`: uppercase constants plus unknown valid values; `from_str`,
  `from_extension`, `from_path`, `from_content_type`, `from_content_coding`;
  borrowed `as_str`, `top_level`, `subtype`, `structured_suffix`;
  `extension`/`content_coding`/`format` are the one reverse table; category
  predicates include top-level classes plus `is_textual`, `is_structured`,
  `is_tabular`, `is_encoding`, `is_archive`, conservative `is_binary`.
- `MediaType`: `new` (unencoded base), `from_parts` (validated encoding
  sequence), `from_str`, `from_extension`, `from_extensions`, `from_path`,
  `from_content_headers`; borrowed `base`, `encodings`, `encoding`; mutation
  `set_base`, `set_encodings`, `push_encoding`, `clear_encodings`;
  persistent `with_base`, `try_with_encodings`. Default
  `application/octet-stream`, no encodings. Preserve encoding order and
  repeats; only core-classified encodings may occupy the sequence.
- `Codec`: closed set `Identity`, `Gzip`, `Zlib`, `Deflate`, `Zstd`;
  `from_str` accepts legacy `x-`; `from_mime_type`/`from_media_type`/
  `from_url`; `load`/`dump`/`reader`/`writer`. `Level` is one shared 0-9
  scale mapped to each codec's native range.
- `IOKind`: `Memory`, `File`, `Directory`, `Unknown`; derived
  `is_container`, `is_leaf`, `is_known`.
- `DataType`: `from_str`, `from_arrow`, `from_json`, `from_fields`,
  `to_arrow`, `into_arrow`, `to_json`, `into_json`, `as_fields`,
  `default_value`, `is_default_value`, `to_scheme_compat`. Generic
  finite-variant constructor is exactly `variant`: Fields in declaration
  order, Arrow type IDs `0..`, canonical dense `Union` (kind `union`, dense
  display, lossless Arrow round trip, <=128 members); parser `variant(...)`
  is input sugar accepting only dense sequential-from-zero, displaying as
  `union(dense,...)`. Generic `decimal` selects decimal128 (P 1..=38) or
  decimal256 (39..=76), then delegates validation; generic `time` selects
  time32 (s/ms) or time64 (us/ns) likewise; intervals are rejected without
  selecting a width.
- `Field`: `from_parts`, `from_str`, `from_arrow`, `from_arrow_ref`,
  `from_json`, `to_arrow`, `to_arrow_ref`, `into_arrow`, `into_arrow_ref`,
  `to_json`, `into_json`, `default_value`, `to_scheme_compat`.
- Field mutation: `set_name`, `set_data_type`, `set_nullable`,
  `set_dictionary_options`, `set_metadata`, `insert_metadata`,
  `update_metadata`, `remove_metadata`, `clear_metadata`. Persistent:
  `with_name`, `try_with_data_type`, `with_nullable`,
  `try_with_dictionary_options`, `try_with_metadata`,
  `try_with_metadata_entries`, `with_metadata_removed`.
- Shared Field properties: borrowed `alias`, `catalog_name`, `schema_name`,
  `table_name`; typed `location`; matching `set_*`/`remove_*`/`try_with_*`/
  `with_location`. Protocol properties: only `get_property`, `has_property`,
  `set_property`, `remove_property`, `clear_properties`, `property_iter`,
  `try_with_property`, `with_properties_cleared`.
- Protocol views: `Metadata::protocol`, `Field::protocol`,
  `Field::protocol_mut`, plus one named accessor per well-known protocol
  generated from the single list in `metadata.rs` (`field.iceberg()`,
  `field.iceberg_mut()`, `metadata.postgres()`, ...; `https` absent - it
  shares `http:`). A view is a borrow, never a copy; vocabulary is `get`,
  `contains_key`, `len`, `is_empty`, `iter`, `next_entry`, `key`, `insert`,
  `update`, `set` (replaces only that protocol's properties), `remove`,
  `clear`. Mutation goes through Field's cache-aware methods. Bindings
  project the view as one live mapping (Python mapping dunders, JS Map
  protocol), never a snapshot copy.
- Partition marker: reserved `field:partition`, read `is_partition`, written
  `set_partition`/`with_partition`. Struct roots answer `partition_fields`,
  `partition_field_names`, `partition_field_len`, `has_partition_fields`,
  `only_partition_fields`, `without_partition_fields`,
  `with_partition_fields`. Unmarked fields store nothing, so equally
  partitioned schemas stay exactly equal.
- Arrow/Parquet field identity is exactly `parquet_field_id`,
  `set_parquet_field_id`, `remove_parquet_field_id`, `with_parquet_field_id`,
  and the walks `assign_parquet_field_ids`, `max_parquet_field_id`,
  `field_by_parquet_field_id`; stored under `PARQUET:field_id` as canonical
  signed 32-bit decimal, with the same validation on every construction and
  import path. No second field-id key, no generic `id` alias on `Field`, no
  confusion with e.g. `iceberg:field_id`.
- HTTP Field reads: raw `accept*`, `cache_control`, `content_*`, `etag`,
  `expires`, `last_modified`, `range`, `vary`; typed `content_length`,
  `http_location`, `mime_type`, `media_type`. Raw mutation via matching
  `set_*`/`remove_*`. `set_mime_type` changes Content-Type only;
  `set_media_type`/`remove_media_type` update Content-Type and
  Content-Encoding as one transaction; failures leave metadata and Arrow
  caches unchanged. Bare `location` stays distinct from `http_location`.
- `Metadata`: `new`, `from_entries`, `from_arrow`, `from_json`, `to_arrow`,
  `into_arrow`, `to_json`, `into_json`, `get`, `contains_key`, `iter`,
  `protocol`.
- `Uri`/`Url`/`Urn`: `from_str`, `from_path`, `from_uri`, `to_json`,
  `into_json`, `to_uri`, `into_uri` where meaningful; `Uri` adds `to_url`,
  `into_url`, `to_urn`, `into_urn`; file values add `to_path`, `into_path`.
  Components: `scheme`, `authority`, `path`, `query`, `fragment`,
  `namespace`, `namespace_specific`; component newtypes expose `as_str`;
  full identifiers use `Display`.
- Resource paths: `path_segments`, `file_name`, `extension`, `extensions`,
  `stem`; iterators borrow without allocating. Filename mutation:
  `set_file_name`, `set_stem`, `set_extension`, `set_extensions`,
  `remove_extension`, `clear_extensions` - atomic, preserving unrelated
  components. MIME access `mime_type`/`media_type`; setters rewrite only the
  inferred suffix chain via the core preferred-extension table and reject a
  type with no known extension unchanged.
- `Format`: `from_str`, `from_extension`, `from_path`, `as_str`,
  `extension`; variants `Json`, `JsonLines`, `Yaml`, `Toml`; extensions
  `.json`, `.jsonl`, `.ndjson`, `.yaml`, `.yml`, `.toml` without opening the
  file.
- Structured text: generic dispatch in `text::{from_str, from_slice,
  from_reader, from_str_all, from_slice_all, from_reader_all,
  from_reader_iter, from_str_inferred, from_slice_inferred, to_vec,
  into_vec, to_writer, to_writer_all}`; `json`/`yaml`/`toml` mirror the same
  vocabulary without a format argument; `json::from_lines_str` for JSON
  Lines. `codec` re-exports for compatibility only. `_with_limits` forms only
  where caller limits differ; no duplicate string-first serializers.
  Redirected TOML output calls `toml::validate_for_write_with_limits`
  (runtime decode limits) before opening a destination, then streams via
  `to_writer`; core-default callers may use `validate_for_write`.
- `TypedValue`: one value paired with its datatype, validated on
  construction, sharing Field's marker family - `TypedValue<K: FieldType>`,
  one alias per datatype (`Int64Value`, `Utf8Value`, ...), `AnyType` default.
  Narrowed: `try_from_parts`, `try_from_value`; dynamic: `from_parts`,
  `from_value`; static datatypes add `new`. Behind the `arrow` feature it is
  the one scalar Arrow projection: `to_arrow_array`, `from_arrow_array`,
  narrowed `try_from_arrow_array`; callers holding Field context use
  `arrow::scalar_array` / `arrow::scalar_value` instead. Never a second
  marker family, never a Field-owning scalar wrapper.
- Codec values: `Value::from_sequence`, `Value::from_mapping`; `Float` has
  `as_f64`/`into_f64`. No application-tag carrier: a name over an untyped
  payload is not a type. Never reintroduce `Tag`, `TaggedValue`, or
  `Value::Tagged`.

Python mirrors these in `snake_case`; JavaScript maps `from_str` ->
`fromString` and other underscores to camelCase. Generic `from`/`from_value`
inference exists only at the extension boundary and dispatches immediately to
the matching core method.

## Error message contract

An error lets the caller fix the input without reading source: expected,
actual, where.

- Show the offending value next to the expectation, `expected X, got Y`
  shape, both sides through the same formatter. `temporal precision must be
  between 0 and 9, got 12`, never the rule alone.
- Locate the failure: dot/bracket path for nested schema/record/value
  errors, byte position for parser/codec, batch index or URL for tabular. A
  path is required wherever recursion reaches more than one node.
- Structured disagreements render through the shared `show_diff`/`show_diffs`
  vocabulary (`≠`, `−`, `+`, `→`, `↳`, `✓`), reporting every differing node.
- Prefer typed variants with `expected`/`actual`/`path` fields over
  interpolated strings; do not widen a catch-all for a case deserving fields.
- Quote user strings with `{value:?}`; render datatypes/fields via canonical
  `Display`. Truncate unbounded values with the shared text limits - never
  allocate proportionally to the payload.
- Errors leave the receiver unchanged and must not imply a partial write.
- One layered family: `yggdryl::Error` owns schema/identifier/codec
  failures; `yggdryl::arrow::Error` wraps it at runtime boundaries; no third
  enum; downstream backends report via `Error::external` preserving the
  chain.
- Bindings surface the native message unchanged, mapped to idiomatic
  exception types; never rewrite or re-prefix it.

## Native value behavior

- Implement `Clone`, `Debug`, `Display`, `Eq`, `Ord`, `Hash`, `Serialize`,
  `Deserialize` where semantics permit; caches are ignored by all of them.
- Comparison: `equals(other, with_metadata)`, `show_diffs`, `show_diff`;
  `with_metadata=false` ignores Field metadata recursively. `show_diffs` is
  lazy; output is stable UTF-8 without ANSI, symbols as above, unambiguous
  path per line; `show_diff` joins with newlines, `✓ equal` when empty. FFI
  runtimes store the core difference cursor, never collect the iterator to
  satisfy lifetimes.
- `Display` is canonical and round-trips through `FromStr`; `Debug` is
  diagnostic, never the serialization format.
- Collections: deterministic iteration, length, indexed and named lookup,
  `IntoIterator`; `Index` only where panic-on-missing is normal. Mutation
  preserves ordering, uniqueness, validation, Arrow-cache correctness.
- Serialization is version-independent structural data; deserialization
  routes through constructor/parser validation.
- Never panic, unwrap, or use unsafe for caller-controlled input.
- `Scheme`, `Authority`, `UriPath` are validated owned non-null values;
  RFC-permitted absence is an empty component, never `Option`/null; query
  and fragment are optional components.

## Parser contract

- `DataType::from_str` and `Field::from_str` are the only recursive schema
  grammars; bindings never pre-parse type expressions. `TimeUnit::from_str`
  is the one flat unit parser; datatype parsing reuses it. Bare `interval`
  defaults to `MonthDayNano`, displayed explicitly.
- Accept canonical output plus common Arrow, SQL, Hive, Spark forms; type
  keywords ASCII case-insensitive, names and quoted values keep case and
  Unicode.
- Support arbitrarily nested lists, structs, maps, dictionaries, unions,
  run-end encoding, decimals, fixed-size, and temporal parameters up to an
  explicit recursion limit.
- `variant(...)` is dense-union sugar (sequential IDs from zero, 128-member
  cap enforced while consuming, sparse rejected, canonical display
  `union(dense,...)`). Generic `decimal`/`numeric` syntax calls
  `DataType::decimal`; generic `time` calls `DataType::time`; explicit
  `decimal128`/`decimal256`/`time32`/`time64` keep exact limits.
- Accept balanced optional outer `()`, `[]`, `{}`, single or double quotes;
  never strip unmatched or interior delimiters heuristically.
- Split only at top-level separators, honor quoting/escapes, reject trailing
  tokens, duplicate names/type IDs, invalid nullability, malformed numbers;
  errors carry byte position and context.
- Every new grammar branch gets round-trip and adversarial tests; benchmark
  cold scalar parsing, deep nesting, parse/display round trips.

## JSON, YAML, and TOML codec contract

- Byte-first boundary: parse borrowed slices or `Read`, emit `Vec<u8>` or
  `Write`. `&str` conveniences use the same parser without an intermediate
  buffer or repeated UTF-8 validation. Parsing still allocates the owned
  `Value` tree - never call it allocation-free.
- One `Value` is the lossless superset: signed/unsigned 64/128-bit integers,
  total-order floats, bytes, sequences, ordered arbitrary-key mappings;
  shared nesting via immutable `Arc`; empty values without backing
  allocation.
- Plain JSON stays plain. Values outside JSON's model use the one versioned
  `$yggdryl` envelope; an ordinary mapping with that shape is escaped so user
  data is never mistaken for a typed value.
- `!yggdryl/*` YAML tags name kinds the value model has and select their
  payloads; every other tag is an annotation whose node decodes as its plain
  value. The write path emits no non-core tag. Comments are never consulted.
- TOML: TOML 1.1 on the newest pinned `toml` keeping the 1.85 floor; decode
  the borrowed spanned `DeTable`/`DeValue` tree (no Serde datetime-marker
  collisions; byte positions retained). Root is always a table; empty
  documents decode as empty mappings. Native date/times decode to the
  temporal they name at the coarsest unit keeping every digit; a temporal
  projects back as native TOML syntax exactly when TOML can spell it
  (four-digit year, fixed offset, within one day) - otherwise, and for every
  decimal, the typed envelope. No multi-document stream: `from_*_all`
  returns exactly one value; `to_writer_all` rejects zero or two.
  Root-scalar/null/bytes/unsigned-or-wide-integer/decimal/arbitrary-key/
  unspellable-temporal use the envelope (same payloads as JSON/YAML);
  colliding user mappings are escaped; a `type` naming nothing the model
  holds decodes as the mapping it is. Native integers are signed 64-bit,
  overflow rejected; non-native storage variants keep typed envelopes. Root
  table counts as depth one; preflight the exact wire projection against the
  published hard depth cap before writing.
- A decoded document never names a class: bindings construct classes only
  from caller-passed targets; never import, evaluate, construct, or mutate
  globals from untrusted content.
- Default limits bound bytes, depth, nodes, documents, aliases, applied
  while reading, per document for depth/nodes and per stream for
  bytes/documents; errors name format and byte position. Recursive parsers
  publish and enforce a conservative hard depth a caller-supplied `Limits`
  cannot exceed.
- Streaming iterators consume one row/document at a time and fail at the
  failing item; streaming writers encode to the sink with backpressure. An
  async single-document convenience may use one bounded buffer - documented,
  never called incremental parsing.
- Inference is deterministic. Path suffixes select the format; bindings
  treat path-likes as paths, byte-likes as content, strings as paths only
  when naming an existing file; explicit format wins. Content inference uses
  `text::from_{str,slice}_inferred` (parse once): JSON wins, empty or
  comment-only stays YAML, complete nonempty TOML next, remainder YAML;
  JSON Lines is never content-inferred; `text::infer_format(&[u8])` only
  when the value is not needed.
- `{{ }}` placeholders are a closed grammar - `{{ NAME }}`,
  `{{ NAME | default(LITERAL) }}`, and `{{{{` for a literal brace - resolved
  by walking the parsed `Value`, never by rendering text before the parse:
  byte positions in diagnostics stay exact and a substitution can never
  change a document's shape. It is not a template engine and no
  template-engine dependency is taken; the docs say "Jinja-style", never
  "Jinja". Substitution is opt-in, **environment access is a second opt-in on
  top of it**, and with that switch off no `std::env` call is made at all - a
  resolved secret that is then dumped or written to a table has leaked. A
  document with no `{{` costs one linear scan and nothing else.
- Benchmark slice parsing, reader streaming, vector and writer emission,
  enveloped values, wide mappings, deep structures; keep allocation
  baselines; report throughput rather than calling unmeasured code
  optimized.

## Arrow and allocation contract

- One sealed zero-sized marker per `DataType` variant with a `*Field` alias
  over `TypedField<K>`; `TypedField<K>` holds exactly one generic `Field`,
  `TypedFieldRef<'_, K>` one borrowed pointer. No duplicated parameters,
  metadata, caches, or children in the typed layer; conversions validate the
  full Field before proving the marker; no `DerefMut`/`as_field_mut`/other
  unchecked route that could swap the datatype behind `K`.
- Lossless Arrow schema parity; validate at construction/import and cold
  projection boundaries, not per record or cache hit.
- C Data Interface schemas go only through `DataType::to_arrow_ffi` and
  `Field::to_arrow_ffi`, which own recursive child/dictionary/nullability/
  ordering/metadata/flag repair; bindings import that schema and keep no
  second recursive FFI builder.
- Both Arrow unit enums import infallibly into `TimeUnit`; exports are the
  fallible category conversions; never coerce across temporal/interval.
- `to_arrow*` may clone shared refs; `into_arrow*` moves what Arrow permits.
- Cache complete Arrow projections; no-op mutations retain the cache,
  effective ones invalidate exactly once; cache state never affects value
  traits or serialization. Arrow dictionary ID and ordering flags are owned
  Field state until the pinned Arrow removes them.
- Core scalar construction, getters, metadata lookup, iteration setup, and
  cloning shared nesting must not allocate (foreign projections like
  `pyarrow.Scalar` excepted). Empty collections have no backing allocation.
  Bulk metadata updates validate once then mutate uniquely or copy-on-write
  once; runtime adapters accumulate one ordered/hashed overlay (last write
  wins) and cross the mutation boundary once.
- No per-record maps or schemas. Measure before claiming optimization; keep
  Criterion out of production graphs. A claim that something does *not* scale
  with N is **counted**, not asserted: `rust/tests/allocations.rs` holds a
  pass-through counting global allocator and compares the same work at two
  corpus sizes, because a timing hides a per-record allocation inside I/O and
  a comment is not evidence at all.
- Defaults are `DataType::default_value`/`Field::default_value`: a datatype
  prefers a present zero/empty value (`Null` and transparent null-only
  wrappers excepted); a nullable Field prefers logical null including
  required Union tags and RunEndEncoded layouts; struct and fixed-list
  defaults delegate per child. Preflight recursion, node, and byte budgets
  before materializing; public enum variants cannot bypass the caps.
- Compatibility targets run the one generic recursive walker with a
  per-target scalar matrix (Arrow is a validated cache-preserving no-op).
  Rewrites preserve name/nullability/metadata, invalidate a populated cache
  exactly once, and reject extension storage rather than relabeling. Never
  fork a per-target walker.
- A root `Field` validates once (`validate_struct_root`), caches its
  projection, stays cheap to clone; rows validate with `validate_value`,
  normalize with `canonicalize_value`; named materialization uses `index_of`/
  `get_field_by_name` and rejects missing/unknown/duplicate/non-string keys
  before committing.
- Runtime materialization is `yggdryl::arrow`: `StructScalar` pairs the
  exact root Field with a real one-row StructArray, zero-copy child slices;
  batch and IPC readers validate the stream schema once, decode lazily,
  retain at most one batch, stop at the first failing row; conversion is
  exhaustive and schema-directed - never JSON as an Arrow bridge.
- One scalar crosses the Arrow array boundary as `arrow::scalar_array`
  (validated caller value in, exact one-row `ArrayRef` out) and
  `arrow::scalar_value` (validated foreign one-row array in, canonical
  `Value` out); the exact `Field` beside them is the authority on
  nullability, dictionary options, and extension identity. `TypedValue`
  projects the same boundary without a Field (`to_arrow_array`,
  `from_arrow_array`, marker-narrowed `try_from_arrow_array`) through a
  synthetic non-nullable Field with the canonical-default null exception.
  Defaults are `DataType::default_arrow_array` / `Field::default_arrow_array`
  over the bounded core default planner - no Field-owning scalar wrapper
  struct, no binding-side placeholder table or redundant validation.
- Casting: `ArrowCast` owns schema-directed array and RecordBatch casts;
  typed fields cast to their own array type
  (`Int64Field::cast_arrow_array -> Int64Array`) via `field::ArrowFieldType`.
  Behind the default `arrow` feature; never duplicated in a binding. Struct
  reconciliation is ASCII-case-insensitive, rejects ambiguous folds, follows
  target order, drops extras, fills missing nullable/required with
  null/defaults; recurse before the outer layout cast so nested names are
  never positional. Exact inputs still pass logical-null and Map validation,
  then keep their arrays.
- Wrapper exposure propagates: null parents, inactive union members,
  unreferenced dictionary values, unused run-ends must not make hidden child
  nulls or failures observable; physically required hidden slots use
  schema-valid placeholders, never invented logical defaults.
- Preflight every new Arrow array against the shared one-million-slot and
  64-MiB fixed-buffer budgets; missing columns charge one aggregate plan;
  repeated dictionary defaults keep one vocabulary value; nullable
  variable-size children `new_null_array` never creates are not charged;
  exact borrowed arrays are not charged.
- `MediaDescriptor` owns one URL, media type, exact Struct root, cached
  Schema. `ArrowTable` owns the batch protocol as inherent methods
  (validated cursor reads, non-consuming snapshot, atomic overwrite-all,
  lazy readers, eager immutable `ArrowDataset`); no media trait over
  `IOBase`; whole-resource replacement is spelled `overwrite` everywhere, no
  `write` alias. Mutations are transactional (error leaves the table
  unchanged); batch upsert replaces an index, appends only at `index ==
  len`, rejects gaps, invents no key semantics. `BatchSelector` takes an
  index or canonical location; a descriptor location selects the resource, a
  numeric fragment one batch; mismatches fail before I/O; successful
  mutation resets in-memory cursors. `ArrowTable` is the growable in-memory
  table over `Vec<RecordBatch>` under the same budgets; its `IOBase` view is
  its IPC encoding, produced on demand, dropped on mutation; `ArrowDataset`
  stays the immutable holder.
- A read target is an optional non-null Struct Field: none = validate and
  preserve the source layout; some = one reusable recursive cast plan.
  Discovery for an uninitialized sink belongs at the optional-target cast
  boundary, not a dishonest optional accessor. `safe` follows Arrow's
  cast-failure policy and never bypasses physical validity, logical
  nullability, Map invariants, or descriptor checks.
- Record adapters bulk-materialize through the native Field-directed paths:
  batch- and row-lazy, one retained source batch, one cast plan per input
  schema, no row maps, no JSON bridge. Python exposes PyArrow holders; JS
  uses its standard copied IPC boundary and never claims zero-copy.
- IPC dictionary IDs are transport-local: native Field IDs travel in one
  reserved versioned root-Schema sidecar keyed by field path, removed on
  import; explicit reads without it keep caller IDs authoritative. Reject
  direct Dictionary-of-Dictionary before IPC output (Arrow IPC 59 cannot
  represent it).

## Resource identifier contract

- Parse URI syntax once, in Rust; bindings never split schemes, authorities,
  paths, queries, fragments, namespaces, or suffixes.
- Canonical output lowercases schemes, forward slashes for files; Windows
  drives -> `file:///C:/...`, UNC -> `file://server/share/...`;
  host-OS-independent.
- Validate schemes, authority delimiters, percent escapes, URN namespace IDs
  and strings at construction; errors carry byte offset and context.
- Segment/file-name/extension lookup borrows the canonical path; cloning and
  component reads do not allocate.
- Compound media inference walks suffixes right to left, reports encodings
  in application order; unknown bases -> `application/octet-stream`;
  archives are base representations, not transparent codings. Bindings call
  these natively, never split filenames themselves.
- Filename/stem/extension changes validate the complete replacement first;
  preserve scheme, authority, query, fragment, leading syntax, URL and URN
  constraints; invalid input leaves the value unchanged.
- File URI/path conversion preserves every accepted component; reject
  queries, fragments, unsafe decoded controls, encoded separators, escapes
  creating drive prefixes, and escaped authority a UNC decode would
  reinterpret; UTF-8, spaces, tabs, literal `%` in UNC servers round-trip.
- `Display` is canonical and losslessly parseable; Serde uses structural
  objects, never a platform path or debug format.

## Binding boundary contract

Both extensions:

- **Rust proves a feature first** (see Order of work).
- **Parity is the goal**: every reachable core module should be reachable
  from both languages; a Rust-only module is a gap with a `Rust only` docs
  note until closed.
- **Interop goes through Arrow**: PyArrow, Arrow JS, C Data Interface or IPC
  between. No second wire format; no zero-copy claims where a copy happens.
- **Values cross through one canonical spelling**: temporals cross as the
  `Value` variant naming an Arrow datatype (`Timestamp`, `Date`, `Time`,
  `Duration`); a runtime value with no variant crosses as its plain shape,
  never as a name a document could carry.
- Conversion pairs are explicit - Python `as_py`/`from_py`, JS
  `asJs`/`fromJs` - and every `load`/`dump` routes through them.
- **Infer at the boundary, compute in Rust**: generic entry points
  (`from_arrow`, `from_str`, `from_dict`, `from_path`, one `from_`/`from`)
  redirect immediately to the optimized core call, never reimplement it.
- **Coerce convenient arguments** once at the boundary via core `from_*`
  (a `str` where a `MediaType`/`MimeType`/`Url`/`Codec`/`IOKind`/`DataType`
  is expected; path-likes as `Url`; mappings as `Metadata`; Arrow values as
  Arrow) - never by stringifying arbitrary objects.
- **Stay idiomatic** (Python protocols; JS `Map`/iterables/spread/`from`):
  same operation, not same syntax. Argument names and meanings are identical
  across languages.
- Every binding feature ships with its own tests and a benchmark of the
  boundary it crosses.
- Surface native error messages unchanged, mapped to idiomatic exception
  types.
- Bind scope constructs (`with`, `using`, `Symbol.dispose`) to
  `IOBase::open`/`close`; no binding-side cache.

## Python extension

- `IOBase` and `Url` are `pathlib`-shaped: everything `Path`/`PurePath`
  answers, same names, core-backed (`name`, `stem`, `suffix`, `suffixes`,
  `parts`, `parent`, `parents`, `joinpath`, `/`, `with_name`, `with_stem`,
  `with_suffix`, `match`, `relative_to`, `is_relative_to`, `exists`,
  `is_dir`, `is_file`, `iterdir`, `glob`, `rglob`, `read_bytes`,
  `read_text`, `write_bytes`, `write_text`, `mkdir`, `touch`, `unlink`). No
  modes, no cursor; failures raise what `pathlib` raises. Never reimplement
  a core rule in Python.
- Boundary inference accepts wrappers, strings, PyArrow schema values, and
  typing annotations, redirecting to core `from_*`/`from_pyhint`. A string
  instance is a datatype expression; the `str` type infers native UTF-8.
  Never stringify arbitrary objects as fallback.
- `DataType.decimal(precision, scale=0)` is a thin call to the core
  selector: exact integer-likes and base-10 integer strings, reject bools
  and floats, no re-implemented width rules.
- Idiomatic `__str__`, `__repr__`, rich comparison, `__hash__`, pickle,
  JSON. **Item access on a `Field` or a `DataType` reaches a nested child and
  nothing else** - `str` by name, `int` by position, with `len`/`iter`/`in`
  speaking children on both, one shared semantic so a schema walk never gets a
  child from one node and a metadata string from the next. `DataType` stays a
  *read-only* child collection (it is hashable); child mutation lives on
  `Field` through cache-aware `set_field`/`set_field_by_name`/`remove_field*`,
  never an `IndexMut` or a handed-out `&mut` child, and assignment is
  dict-like by name (unknown appends) and list-like by position (replaces
  only). `Field` metadata is a live mapping *view* - `field.metadata` with
  mapping dunders plus `get`, `keys`, `values`, `items`, `update`, `clear` -
  because a view whose keys are keys is where item syntax means a key.
- Conversion work stays in Rust over the C Data Interface; no duplicated
  parsing, validation, comparison, hashing.
- Arrow scalars: exactly `DataType.arrow_scalar(value, *, safe=True)` and
  `Field.arrow_scalar(value, *, safe=True)` -> `pyarrow.Scalar` of the
  projected physical type. Exact input Scalars pass by identity; mismatched
  Scalars cast with PyArrow's matching `safe` policy; non-Scalar input uses
  typed construction when `safe=True`, single inference or one unsafe cast
  when `safe=False`, falling back to typed construction where inference
  cannot shape the target (e.g. map pairs). A bare `DataType` permits typed
  null; a non-nullable `Field` rejects `None` and null Scalars; child
  nullability belongs to record builders. Both packages delegate
  StructArray/RecordBatch/IPC materialization to `yggdryl::arrow`; neither
  keeps a second schema or value model.
- Identifier components are read-only properties, segments a read-only
  sequence; path-like/string inference calls Rust `from_*` immediately.
  `MimeType` is immutable and hashable; `MediaType` copy-on-write (mutated
  wrappers unhashable); both shared by identifier and Field APIs; no suffix,
  coding, or category inference in Python code.
- `yggdryl.{json,toml,yaml}` are thin byte-oriented facades; recursive
  conversion happens in Rust; orchestration may resolve I/O, record targets,
  registries, but never serializes `dict -> str -> Rust`.
- Python values (scalars, bytes-likes, temporals, decimal, UUID, enum, path,
  collections, mappings, dataclasses, records) convert to the `Value`
  variant holding their parts; a value with no variant crosses as its plain
  shape (an over-wide integer keeps magnitude as decimal text, losing type).
- Records expose exact `from_json`, `from_toml`, `from_yaml`, `from_`,
  `into_json`, `into_toml`, `into_yaml`, `into_`, delegating to
  `from_dict`/`to_dict` policy - no second record caster. Encoders return
  bytes without a destination and never close caller streams.
- **`pyarrow.RecordBatchReader` is the record shape**: every read returns
  one, every write consumes one, over the C Stream interface only. Writes
  accept anything PyArrow exports a stream from (reader, `Table`,
  `RecordBatch`, `__arrow_c_stream__`, sequence of batches). Never a
  row-level read/write; never a `Table` where the core returns a reader.
- Record methods keep core names and order: `record_options`,
  `read_arrow_field`, `read_arrow_batch_reader`, `write_arrow_batch_reader`,
  `append_arrow_batch_reader`. No read-target argument (the schema lives on
  the options). `options` is the one keyword, accepting the settings value
  or anything naming an encoding; omitted, the media type decides.
  `IOBase.media_type` is settable; `with` binds `open`/`close` (which
  publishes a written file at exact length).
- `RecordOptions` is the core value, never a Python model. A setting another
  encoding lacks reads `None` and raises naming the encoding when set.
  Foreign codec names cross as the text that format's parser accepts, never
  a diverging `Display`.
- **A table format is a module**: `yggdryl.iceberg` holds `Table`,
  `Catalog`, `Compaction`, `SchemaUpdate`, `PartitionField`,
  `PartitionSpec`, `Snapshot`, `ManifestFile`, `DataFile`,
  `assign_field_ids`, `can_promote`, `schema_from_json`, `schema_to_json`,
  nothing else. Tables build from an `IOBase` handle only; scans are
  `pyarrow.RecordBatchReader`; metadata values are read-only views only a
  commit can produce. Both bindings take the same arguments in the same
  order (spec or column names, then format version).
- **Bindings commit granularly, never through a closure**:
  `update_properties(updates, removes)` (one commit; nothing when both
  empty) and `update_schema()` - a builder recording `add_column`,
  `drop_column`, `rename_column`, `update_doc`, `make_nullable`,
  `update_type`, whose `commit` replays onto a fresh core `SchemaUpdate` and
  writes one document; in Python it is a context manager (commit on clean
  exit, discard on exception, spent builder refuses reuse). Time travel is
  `scan_at(snapshot_id, filters, schema)`; refs via `snapshot_by_ref`;
  `compact()` returns the counts; inspection tables come back as the
  record-reader shape under core column names.
- **The catalog crosses with its inference**: `Catalog(warehouse)` accepts a
  handle or anything naming a folder; `create_table` accepts a native Field,
  expression, Arrow schema, or iterable of Fields; `append`/`overwrite`
  accept what `Table.append` accepts and return the table. Names stay
  dotted.

### Python records and annotations

- Annotation inference and record conversion live below
  `python/yggdryl/records/`; they may inspect typing objects but construct
  native `DataType`/`Field` wrappers - never a parallel schema, never
  PyArrow types created merely to re-import.
- Entry points: `DataType.from_pyhint`, `Field.from_pyhint`.
  `Optional[T]`/`T | None`/unions with `None` supply default nullability; an
  `Annotated` `nullable` option overrides and governs safe input/output at
  every nested boundary; a bare `None` default never changes schema
  nullability and must satisfy the cached Field.
- Field options are exactly `arrow_type`, `nullable`, `metadata`, `id`,
  `dictionary_id`, `dictionary_is_ordered`, as `(key, value)` extras or an
  options mapping (reserved-leading tuples exactly two items). Resolve
  left-to-right, validate the final winner, merge metadata entry-wise, then
  overlay caller/dataclass metadata. A mapping naming one dictionary option
  must name both; tuple extras may split them. A sole physical member
  through Optional/NewType/TypeVar/alias supplies the baseline; outer
  options overlay; a parent `arrow_type` owns its whole subtree.
- `arrow_type` accepts only an actual PyArrow datatype; preserve explicit
  ExtensionTypes through one native Field import; reject conflicting
  `ARROW:extension:*` overlays; extension metadata must be UTF-8. Bare
  `DataType.from_pyhint` applies only `arrow_type`, rejects ExtensionType
  and Field-only options; legacy all-string metadata-only mappings stay
  ignored there.
- `@record` produces a genuine stdlib dataclass; cache one tuple of child
  Fields and one root Field per class, never per instance. Persist
  `python.module`, `python.class`, `python.qualname`, `python.kind` as Field
  metadata. Freeze published cached Fields: metadata mutation raises.
- The records module re-exports the useful `dataclasses` surface; `to_dict`
  and `from_dict` support records and plain dataclasses; generated methods
  delegate to them.
- `safe=False` is the explicit shallow path; `safe=True` recursively
  validates and casts with path-aware errors: exact boolean casting,
  fixed-size-list arity, no temporal truncation. Naive datetimes are UTC
  only for exact `UTC` targets; other zoned targets need aware, zoneless
  need naive; Arrow times reject aware. Error policies are exactly
  `errors="raise"` and `errors="default"` (declared default or factory,
  else raise).
- Resolve inherited/forward annotations once per class; detect recursive
  graphs, preserve declaration order, avoid shared mutable defaults;
  benchmark cached schema access and safe vs shallow separately. Never
  retain a decorator frame: pending records keep annotation-reachable
  bindings only; parameterized aliases use per-conversion binding contexts
  and never publish a specialization as its generic origin's schema.
- Safe output validates existing instances without reconstruction or
  `__init__`/`__post_init__`; thread declared generic hints through nested
  traversal.
- Class factories are exactly
  `Record.from_arrow_field(field, *, class_name=None, module=None)` and
  `Record.from_arrow_schema(...)`: import each Arrow field once through
  `Field.from_arrow`, cache those exact Fields, derive casting annotations
  from the cached graph (a language view only - never regenerate through
  `from_pyhint`, which can erase widths, layout, dictionary state,
  metadata). Assemble roots with `DataType.from_fields`, never via
  `pa.struct` round trips or a second type table. Struct roots contribute
  children; scalar roots one column. Require unique, valid, non-keyword
  Python identifiers; never rename silently. Naming: explicit args, then
  valid `python.class`/`python.module` metadata, then root-name/
  `ArrowRecord` and `__main__`. Replace only the four generated identity
  keys; keep other metadata. Preserve Schema metadata on the root and
  project back via `into_arrow_schema`; accept UTF-8 byte pairs, reject
  non-UTF-8 explicitly. The reserved `yggdryl:ipc:dictionary-ids` key is
  transport state: route through `yggdryl::arrow`, restore IDs once, keep it
  out of root metadata and `into_arrow_schema`; collection outputs may cache
  one transport Schema; never parse the sidecar in Python.
- Collection imports are exactly `from_dicts`, `from_arrow_record_batch`,
  `from_arrow_record_batch_reader`, `from_arrow_table`, `from_arrow`: lazy
  iterators reusing `from_dict` with the same `safe`/`errors` contract, no
  per-row schemas or dictionaries as glue. `from_arrow` accepts RecordBatch,
  Table, RecordBatchReader, `__arrow_c_stream__` exporters, or iterables of
  batches; never invoke arbitrary `to_arrow` methods. Validate schemas at
  the batch/source boundary (names, order, datatypes, nullability
  recursively; transport metadata ignored). When validation is off but a
  mismatch cast is needed, cache the target Arrow type once and call
  `Scalar.cast` directly (not `Field.arrow_scalar` per cell); on output
  normalize only explicit mismatched Scalars through `Field.arrow_scalar`;
  use the same helper for one-cell error localization after a bulk builder
  fails. Output `safe=False` skips annotation validation only - physical
  validity always holds, unsafe overflow never enabled. PyArrow floor is 18
  (correct run-end map and extension scalar paths); do not use
  `maps_as_pydicts` - normalize map pairs in the adapter and reject
  duplicate keys first.
- Exports are exactly `into_arrow_field`, `into_arrow_schema`,
  `into_arrow_record_batch`, `into_arrow_record_batches`,
  `into_arrow_table`, `into_arrow_record_batch_reader`; collection forms are
  classmethods over record iterables; batched forms take positive
  `batch_size` default 65,536, stay lazy and bounded; empty eager outputs
  use the cached schema. Reuse `to_dict`'s projection and `safe` behavior
  but append values directly to Arrow columns (no temporary row map);
  resolve the cached schema once. Output iterables accept only instances of
  the receiving class, expose `safe` not `errors` (mapping rows go through
  `from_dicts` first, where defaults and failure policy belong).
- Regressions must cover empty/one-shot iterables, failures after a
  successful batch, schema-metadata differences, incompatible nesting,
  nullability, dictionary and exact widths, deep structs/lists/maps,
  temporal and decimal values, safe vs shallow. Benchmarks separate class
  materialization, schema validation, row casting, batch/table construction,
  bounded reader iteration; fixtures outside measured loops; no allocation
  claims from timings alone.

## JavaScript extension

- Small inferred factories accept wrappers and strings, then delegate to
  core; camelCase only at the boundary, mapping directly to Rust names.
- `DataType.fromFields(iterable)` is the public Struct assembly boundary;
  typed-field factories may hold private native constructors inside the
  loader, never on public classes or published declarations.
- Provide `toString`, `toJSON`, equality, comparison, stable hashing,
  cloning, child access/iteration, and Map-like metadata (`size`, `get`,
  `set`, `delete`, `has`, `keys`, `values`, `entries`, `update`).
  Convenience protocols live in the loader when Node-API cannot express a
  symbol; the native module stays the source of values and validation.
- Record helpers close over one native struct `Field`, define collision-safe
  getters once, materialize rows through schema-guided Node-API conversion;
  nested Structs stay native Records reusing cached layouts; no per-row
  schemas, name maps, JSON bridges, or a parallel JS value model.
- Arrow JS interop is the standard copied IPC boundary (no C Data consumer):
  parse and validate the native schema before consuming one-shot outputs,
  cache the Arrow JS Schema, validate input schemas once, keep IPC cursors
  bounded and lazy. Public Arrow JS objects show transport-local dictionary
  IDs; the native Record and reserved sidecar keep canonical IDs.
- **`BatchReader` is the record shape**: reads return one, writes consume
  one, one-shot (a second consumer is told, not shown an empty stream). One
  batch crosses as one self-contained IPC stream (schema travels; say that
  per-batch header is the copied boundary's cost). `BatchReader.from` is the
  one inference point (reader, Arrow JS `Table`/`RecordBatch`, array of
  batches, IPC bytes); `toIpc`/`toTable` drain. Never a row-level
  read/write.
- Record and table calls take the settings value or anything naming an
  encoding; the encoding is never an argument (`recordOptions()` derives it;
  `mediaType` is settable). Settings another encoding lacks read `null`;
  foreign codec names cross as parser-accepted text.
- **A table format is a namespace**: `iceberg` in the loader holds `Table`,
  `Catalog`, `PartitionSpec`, `DataFile`, `assignFieldIds`, `canPromote`,
  `schemaFromJson`, `schemaToJson`, nothing else, nowhere else. The
  schema-update builder is reached only through `table.updateSchema()`.
  Compaction reports, snapshots, and manifests arrive as plain objects (they
  record, not behave); 64-bit identifiers cross as `bigint`. Same argument
  order as Python.
- Identifier components are read-only properties, segments an iterable view;
  Windows normalization, suffix mutation, MIME inference, preferred
  extensions stay core-only. Export the same native `MimeType`/`MediaType`
  objects Field metadata uses - no string-union lookalikes or second tables.
- JSON/TOML/YAML facades stay byte-first (`Buffer`/typed bytes), recursive
  conversion through native `Value`; exact `bigint`, bytes, Date, Array,
  plain object, Map, Set semantics; classes only from caller-passed targets.
  Generic operations follow core vocabulary in camelCase and infer only from
  real path suffixes.
- A Node-API-decoded `serde_json::Value` passes directly through the core
  Serde implementation - never re-serialized to a string for a second parse.
  Before such a value sees caller-owned JS, build one iterative bounded
  plain-data snapshot: reject cycles, proxies, accessors, symbols,
  over-limit depth/nodes before NAPI's recursive conversion, and pass only
  the detached snapshot (validate-then-revisit is TOCTOU-vulnerable).
- Reserve `javascript:builtins.<Name>` for genuine built-ins,
  `javascript:<type-or-name>` for application classes, `yggdryl:<Name>` for
  native wrappers; detect by native identity, never `constructor.name`;
  reject application identities entering reserved namespaces.
- Node `maxDepth` default and ceiling stay 48 while recursive N-API
  traversal is used; raising it requires iterative traversal plus a
  subprocess regression proving over-limit data cannot abort Node. Generic
  JSON Lines `from` returns rows, `into` consumes a row iterable - never
  typed as a scalar round trip.

## Required checks

Before handoff run: formatting; warning-free Clippy; workspace tests;
parser/interop/text and JSON/YAML/TOML benchmarks; Rustdoc with warnings
denied; the Rust 1.85 core check (`yggdryl` default features and
`--no-default-features --lib`); Python native and codec tests; Node
native/codec/type tests; `python -m mkdocs build --strict`. Run tests and
Clippy twice - default features and `--features "parquet iceberg"` -
because those features are otherwise never compiled; both extensions build
the core with `arrow`+`parquet`+`iceberg`, so `maturin develop` and
`npm run build` cover that combination. Remove generated targets, `site/`,
virtual environments, native binaries, caches, and `node_modules` after
validation.

## Releases

- **`main` is the trigger, the tag is the receipt.**
  `.github/workflows/release.yml` runs on every push to `main`: an
  already-tagged version costs one preflight job; an untagged one builds,
  publishes to all three registries, then creates the `v<version>` tag and
  GitHub release. Releasing = committing a version bump to `main`. A
  hand-pushed `v*` tag releases the same way; a hand-run of the workflow
  builds and verifies without publishing (rehearsal).
- **Publishes are idempotent; the tag lands last.** A registry that already
  has the version is skipped (crates sparse index, PyPI `skip-existing`,
  `npm view` probe), so a half-dead run is repaired by re-running or by the
  next `main` push. Never create the tag before the registries have the
  version.
- **One version, three manifests**: workspace `Cargo.toml`,
  `python/pyproject.toml`, `node/package.json` must agree, and a pushed tag
  must be `v` plus it; preflight refuses anything else. Bump all three in
  one commit; a taken version is never re-released - bump forward.
- **Surfaces**: `yggdryl` to crates.io; wheels for CPython 3.10-3.14 across
  manylinux/musllinux x86_64+aarch64, macOS x86_64+arm64, Windows x64+arm64
  (arm64 from 3.11, the first CPython there), plus sdist, to PyPI;
  `yggdryl` to npm with every platform's native module in one package
  (the generated loader picks by triple at require time).
- **Credentials are repository configuration**: `CARGO_REGISTRY_TOKEN` and
  `NPM_TOKEN` secrets, PyPI trusted publishing (OIDC) bound to the `pypi`
  environment. The workflow names no repository and survives migration; the
  PyPI trusted-publisher binding names one and must be re-registered after
  migrating. No fourth registry, no stored PyPI password.
- **Nothing publishes untested**: wheels smoke-test an end-to-end table
  round trip on their build platform wherever installable (musl wheels
  cannot install on their glibc builder; platforms without PyArrow wheels
  have nothing to install against - those are marked build-only in the
  workflow, never silently skipped); native modules pass the full Node suite
  on their platform; the npm publish refuses a package missing any binary.
  An artifact that was never imported is not released.
