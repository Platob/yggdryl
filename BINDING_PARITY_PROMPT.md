# Prompt: close the binding gap — every Rust-only surface reaches Python and JavaScript

`AGENTS.md:832` states the goal plainly: *"every reachable core module should be
reachable from both languages; a Rust-only module is a gap with a `Rust only`
docs note until closed."* There are **26 `Rust only` notes plus one `Rust first`** in the
documentation today. This task closes the ones that are gaps, corrects the ones that are
stale, and keeps — with a written reason — the ones that are decisions.

Every surface this task adds ships complete: implementation, tests, typed
declarations (`.pyi` / `.d.ts`), documentation tabs that actually run, and a
benchmark of the boundary it crosses (`AGENTS.md:855`).

This is a *bindings* task. The Rust core is already proven; if a binding needs
something the core does not have, the core gets it first — implementation,
edge-case tests, docs — and only then the binding (`AGENTS.md:9`). A binding
never reimplements a core rule.

> The expression engine's own bindings are specified separately, in
> `IMPLEMENTATION_PROMPT.md` §6. This prompt covers everything that was
> Rust-only before it. The two can land in either order; where they touch the
> same file, whichever lands second rebases.

Work on branch `claude/generic-expression-filtering-fm76gl`; commit and push
there. Do not open a pull request.

---

## 0. Read first (non-negotiable)

1. **`AGENTS.md`, in full.** Governing sections: *Order of work* (line 9),
   *Binding boundary contract* (829), *Python extension* (862), *JavaScript
   extension* (1052), *Documentation organization* (345), *Exact method
   vocabulary* (397), *Error message contract* (566), *Required checks* (1116).
2. `.api-bindings.txt` and `.api-inventory.txt` — the generated inventories.
   They are the ground truth for what each language exposes; regenerate them
   with their generator, never edit them by hand.
3. `python/src/*.rs` and `node/src/*.rs` — what each binding already does, and
   the house patterns: `declared_by` duck typing (`python/src/record.rs:193`),
   the loader-side conveniences in `node/index.js`, `node/values.js`,
   `node/records.js`.
4. `docs/extensions/python.md` and `docs/extensions/javascript.md` — the two
   pages that document a boundary and nothing else.
5. Every page carrying a `!!! note "Rust only"`: `avro.md`, `generic.md`,
   `gzip.md`, `local.md`, `zlib.md`, `zstd.md`, `io.md` (5 notes), `ipc.md` (2),
   `parquet.md` (2), `iceberg.md` (7 plus the one "Rust first"), `text.md` (2),
   `uri.md`, `field.md`.

---

## 1. Phase 0 — the audit, and it comes first

Before a line of binding code: walk every `Rust only` / `Rust first` note and
classify it into exactly one of three buckets. Write the result as a table in
the pull-request-less commit message and as a short section in
`docs/extensions/python.md` / `javascript.md` (the reader deserves to know what
is deliberate).

- **Stale** — the note is simply no longer true. Fix it *in the same commit as
  the audit*, before any new code: a wrong note costs a reader more than a
  missing feature, because they stop looking. Candidates found while writing
  this prompt, each to be verified rather than trusted:
  - `docs/local.md:5` says the packages "do not expose this module yet", but a
    local file is reached today through `IOBase` with a path or URL. The honest
    note names the constructor instead of claiming absence.
  - `docs/io.md:1278` says neither binding can add a backend — but
    `IOBase.from_arrow_fs` / `IOBase.fromArrowFs` is exactly how one is added.
  - `docs/io.md:321` says the bindings expose no adapters over positional reads,
    while Python already registers a cursor class (`PyIOCursor`).
  - `docs/gzip.md:5`, `zlib.md:5`, `zstd.md:5` say the module is not exposed,
    while Python already ships `gzip_loads`/`gzip_dumps` and the rest
    (`python/src/codings.rs`). The note should say what is missing — streams and
    the transparent handle — not that everything is.
- **Gap** — real, closable, and closed by this task (§2).
- **Decision** — stays Rust-only, and the note is rewritten to say *why* rather
  than "not yet", so nobody re-opens it:
  - the role traits (`IOPath`/`IOFolder`/`IOFile`) and the generic dispatch
    enums (`Holder`, `Media`, `RecordOptions` as enums) — the bindings hold one
    handle class and one settings value, which is the better surface, not a
    lesser one;
  - `Buffer` as a class — `IOBase.from_bytes` is the binding spelling;
  - the Iceberg type-mapping tables (`iceberg.md:2683`, `2793`) — internal
    tables whose *result* is the schema a table already reports;
  - implementing a new backend in the binding language itself (as opposed to
    wrapping a foreign filesystem) — that is a Rust trait impl.

The audit's deliverable is a checklist. Everything below is scoped by it: if the
audit finds a note this prompt calls a gap is actually a decision, say so and
skip it, naming the reason. Do not implement something the audit disproves.

---

## 2. Phase 1–8 — the gaps, one phase each

Each phase is a commit and is complete on its own: Python surface, JavaScript
surface, tests in both, typed declarations, docs tabs replacing the note, one
benchmark. Argument names, order, and meanings are identical across languages;
only the case convention differs (`AGENTS.md:849`).

### Phase 1 — content codings, fully (`docs/gzip.md`, `zlib.md`, `zstd.md`, `io.md:1173`)

What exists: whole-buffer `loads`/`dumps` in Python. What is missing: the level
argument, streaming, the `Codec` vocabulary itself, and the transparent handle.

- **Python**: `yggdryl.gzip` / `zlib` / `zstd` as thin facades beside
  `yggdryl.json` — `load(source)`, `dump(value, dest, level=...)`,
  `reader(fileobj)` and `writer(fileobj)` returning objects implementing the
  standard `io` protocols (`read`, `readinto`, `write`, `close`, context
  manager), so they compose with anything that takes a file object. Plus
  `yggdryl.Codec` with the core's `from_str` / `from_mime_type` /
  `from_media_type` / `from_url` and `Level` as a plain 0–9 int.
- **Transparent handles** are a *method on the one handle class*, not new
  classes: `handle.coded(codec="gzip", level=6)` returns an `IOBase` whose bytes
  are the decoded ones — the binding shape for `Coded`, `Gzip`, `Zlib`, `Zstd`.
  Say in the docs that reading a `.json.gz` handle needs none of this, because
  the media type already decodes it; this is for the case where the caller names
  the coding themselves.
- **JavaScript**: `gzip`/`zlib`/`zstd` namespaces with `load`/`dump` over
  `Buffer`, `Codec` parsing, and `handle.coded({ codec, level })`. Node's own
  `zlib` streams already exist, so do not ship a second stream implementation —
  say that in the JS tab and point at `handle.coded` for the composing case.
- Errors surface unchanged; a level outside 0–9 is the core's typed refusal.

### Phase 2 — Avro as a value codec (`docs/avro.md`)

The record path already reads and writes `.avro` through the three record
methods. What is missing is the **`Value`-level codec** and schema resolution.

- **Python**: `yggdryl.avro` mirroring the `json`/`yaml`/`toml` facades exactly
  — `load`, `loads`, `load_all`, `dump`, `dumps`, `dump_all`, byte-first
  (`Buffer`/`Readable`/`Writable` sources and destinations), plus
  `avro.schema_from_field(field)` and `avro.field_from_schema(schema)` for the
  schema half, and the resolution entry point the core exposes.
- **JavaScript**: the same namespace, byte-first over `Buffer`, values crossing
  through the existing native `Value` conversion — exact `bigint`, bytes,
  `Date`, `Map`, `Set` semantics, never a JSON bridge (`AGENTS.md:1104`).
- Round-trip tests against the **outside implementation** the repo already
  drives: extend `scripts/check_avro_interop.py` so the binding halves are
  covered, keeping the `SKIPPED`-never-reads-as-a-pass rule.

### Phase 3 — `TypedValue` and the typed markers (`docs/text.md:421,458`, `docs/generic.md`)

A value paired with the datatype it belongs to is a core value both languages
should hold.

- **Python**: `yggdryl.TypedValue(value, dtype)` / `TypedValue.from_value(v)`,
  with `.data_type`, `.value`, `.as_py()`, `.to_arrow()` →
  `pyarrow.Array`/`Scalar` through the existing C Data Interface path,
  `TypedValue.from_arrow(...)`, rich comparison, `__hash__`, `__repr__`,
  pickle, JSON. The typed markers already have field factories in
  `yggdryl.fields`; the value side narrows through the same names.
- **JavaScript**: `TypedValue` with `dataType`, `value`, `asJs()`, `toArrow()`
  (the copied IPC boundary), `fromJs`, `toString`, `toJSON`, `equals`,
  `stableHash`.
- No second value model on either side: conversion is the core's, always.

### Phase 4 — Parquet footer statistics (`docs/parquet.md:793`)

Newly worth having: the read path now prunes row groups by these numbers, so a
caller who cannot see them cannot explain their own read.

- **Python**: `handle.read_statistics(options=None)` → a plain object with row
  count, uncompressed and compressed sizes, split offsets, and per-row-group,
  per-column `null_count` / `min` / `max` — bounds crossing as canonical
  `Value`s (a date is a `datetime.date`, a decimal a `Decimal`), never raw
  bytes.
- **JavaScript**: `handle.readStatistics()`, 64-bit counts as `bigint`.
- Documented next to the read plan, because together they answer "why was this
  read fast".

### Phase 5 — Iceberg leveling (`docs/iceberg.md:295,357,558,867,1744,2847`)

- **The scan planner** (867): `table.plan(filter)` / `table.plan_at(snapshot_id,
  filter)` returning a plain object — files read, files skipped, manifests
  skipped, records, and per-task partition tuple and location. This is the value
  that makes pruning visible in both languages, and it is exactly what the
  expression work makes worth showing.
- **`IcebergOptions`** (1744): one settings value with the same keys in Iceberg's
  own spellings, `table.set_options(...)` / `table.setOptions(...)`, resolved
  explicit → table property → default by the core. No key parsing in the
  binding.
- **`PartitionSpec` transforms and path rendering** (558): expose the transform
  vocabulary as its canonical strings and the path a partition tuple renders to,
  reading both off the core — no second renderer, and the write-side refusal of
  a non-invertible transform keeps its message.
- **The metadata document** (295, 357): `table.metadata()` as a read-only plain
  object (or its JSON), so a caller can inspect format version, properties,
  schemas, specs, sort orders, and snapshots without a Rust program. Updates
  stay through the existing typed vocabulary — never a writable dict.
- **Writer settings** (2847): folded into `IcebergOptions`; nothing separate.
- JavaScript keeps its namespace rule (`AGENTS.md:1092`): everything above lives
  under `iceberg`, `bigint` for 64-bit ids, same argument order as Python.

### Phase 6 — handle surface leveling (`docs/io.md:321`, `local.md`)

- **Python**: whatever the audit shows missing from the pathlib-shaped surface,
  plus a file-object view — `handle.open_binary()` returning an object
  implementing `io.RawIOBase` (`readinto`, `write`, `seek`, `tell`, `close`,
  context manager) over the positional core, so a yggdryl handle can be passed
  to any library that takes a file object. This is the *idiomatic* answer to the
  `std::io` adapters, not a port of them.
- **JavaScript**: `handle.createReadStream()` / `createWriteStream()` returning
  Node `Readable` / `Writable` backed by the same positional calls, bounded and
  lazy. Same reasoning.
- `docs/local.md` gains real Python/JS tabs showing the local backend reached
  through `IOBase`, replacing the note that says it cannot be.

### Phase 7 — the small leftovers (`docs/uri.md:656`, `docs/field.md:311`)

`Uri`/`Url`: `default_port`, `is_local`, `join_path`, `local_mime_type`.
`Field`: `set_init` / `is_init` / `with_init`. Mechanical, both languages, with
tests and the notes deleted.

### Phase 8 — cross-language symmetry

Diff the two inventories in `.api-bindings.txt` column against column. **Every
asymmetry is either closed or recorded with its reason** in the extension docs —
including the ones this prompt did not predict. Known starting point: the
content-coding functions exist in Python and not in JavaScript (Phase 1 closes
that); the Iceberg `Namespace`, `Snapshot`, `ManifestFile`, `PartitionField`,
and `Compaction` classes exist in Python and not in JavaScript. Python-only by
design — and stated as such — are the annotation-driven `records` helpers, which
are a Python language feature, not a core surface.

---

## 3. What every phase must ship

- **Implementation** in `python/src/<domain>.rs` / `node/src/<domain>.rs`
  mirroring core domains; each `lib.rs` stays boundary helpers, exports, and
  registration only.
- **Tests in that extension**: `python/tests/test_<domain>.py` in house style
  (fixtures, plain-English test classes with docstrings) and
  `node/tests/<domain>.test.js` + `<domain>.types.ts` (node:test +
  `tsc --noEmit` pair). Cover the happy path, every error message crossing
  unchanged, and the boundary's own edge cases (empty input, huge input against
  the shared limits, a value the other language cannot hold).
- **Typed declarations**: `python/yggdryl/_native.pyi` and `__init__.pyi` kept
  exact, `mypy --strict` green; `node/index.d.ts` / `binding.d.ts` kept exact,
  `tsc --noEmit` green.
- **Documentation**: the `!!! note "Rust only"` is **deleted and replaced by
  real Python and JavaScript tabs** on the same examples — same operation, each
  idiomatic, each self-contained with at least one assertion, all passing
  `python scripts/check_docs_examples.py`. A note that stays is rewritten to
  give its reason. Notebooks regenerated with
  `python scripts/build_docs_notebooks.py`.
- **A benchmark of the boundary crossed**, release build only
  (`maturin build --release`, `napi build --release`), against a baseline the
  reader trusts — the stdlib codecs for Phase 1, PyArrow for Phases 3–4,
  PyIceberg for Phase 5, `node:fs` for Phase 6 — with numbers regenerated into
  `docs/benchmarks.md`, never edited (`AGENTS.md:376`).
- **Inventories regenerated** by their generator.

---

## 4. Hard constraints

- **Rust first, always.** A binding that needs a core capability gets the core
  change first, with its own tests and docs, in its own commit. A binding never
  computes what the core can compute, never parses what the core parses, never
  validates what the core validates.
- **Infer at the boundary, compute in Rust**: generic entry points coerce once
  through core `from_*` and redirect; never stringify an arbitrary object as a
  fallback.
- **One value model, one schema model, one error family.** No binding-side
  cache, no second parser, no parallel value tree. Native error messages cross
  unchanged, mapped to idiomatic exception types.
- **Idiomatic, not identical**: Python uses its protocols (`len`, `in`, mapping
  dunders, `with`, real keyword defaults, `pathlib` shape); JavaScript uses
  `from` constructors, `Map`, iterables, spread, `bigint` for 64-bit. Same
  operation, same argument names and order, different syntax.
- **No new runtime dependency** in either package; pandas, polars, pyarrow, and
  friends stay imported only where a value of theirs actually appears.
- **No fabricated documentation tab.** If a language genuinely cannot do
  something, the note stays and says why. Never show a language doing something
  `.api-bindings.txt` does not list.
- Nothing in `rust/src/` changes in this task except a core addition a binding
  provably needs — and that addition arrives Rust-first, complete.

---

## 5. Required checks

`cargo fmt --check`; warning-free
`cargo clippy --locked --workspace --all-targets -- -D warnings` twice (default
features and `--features "parquet iceberg"`); workspace tests twice the same
way; `cargo doc` with `RUSTDOCFLAGS="-D warnings"`; `maturin develop` + `pytest`
+ `mypy --strict`; `npm run test:package` + `npm test` + `tsc --noEmit`;
`python scripts/check_docs_examples.py`; `python scripts/check_avro_interop.py`;
`python scripts/check_iceberg_interop.py`; `python -m mkdocs build --strict`.
Clean generated targets, `site/`, virtual environments, native binaries, caches,
and `node_modules` afterwards.

---

**Definition of done**: `grep -rn 'Rust only' docs/` returns only notes that
explain a deliberate decision, every one of them accurate; both inventories list
the same capabilities under each language's own spelling; and a Python or
JavaScript user can read an Avro container, compress a stream, hold a typed
value, see why a Parquet read was fast, and plan an Iceberg scan without being
told to write Rust.
