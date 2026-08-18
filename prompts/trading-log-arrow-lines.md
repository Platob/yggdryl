# Implementation prompt: trading-log lines to Arrow (`into_arrow_lines`)

Copy everything below into a fresh Claude session on this repository.

---

Implement a streaming **log-lines → Arrow** surface in Yggdryl, built on the
existing line-reading ecosystem. Read `AGENTS.md` first and follow it
throughout: Rust first and fully (implementation, edge tests, docs with
running examples, benchmarks), only then the Python and Node bindings.

## What already exists — build on it, do not duplicate it

- `IOBase::read_lines`, `read_lines_matching(pattern)`, `into_read_lines`,
  `into_read_lines_matching` in `rust/src/io/mod.rs` (~line 718) stream
  decoded text lines through a fixed buffer, peeling the media type's content
  codings as streaming decoders — a `trades.log.gz` already reads its lines
  without holding the decompressed value. `LineRecords` groups lines into
  multi-line records opened by a `regex_lite::Regex` match (a log entry plus
  its stack trace arrives as one string; lines before the first match form
  the first record).
- `crate::arrow::BatchReader = Box<dyn RecordBatchReader + Send>`
  (`rust/src/arrow/mod.rs:290`) and the `arrow::batch_reader` helper.
- The stable-hash contract: `stable_hash_display` in
  `rust/src/text/display.rs` is the project's deterministic 64-bit FNV-1a;
  every core value's `stable_hash()` routes through it.
- A flexible ISO datetime parser in `rust/src/generic/iso.rs`:
  `parse_datetime` already accepts `T`, `t`, or a space between date and
  clock, and an optional `.fraction` of 1–9 digits mapped onto the shared
  `TimeUnit`.
- Shared record settings (`batch_size`, declared schema, cast strictness) in
  `rust/src/generic/options.rs`.
- `regex-lite` is the pinned regex engine; do not add a second one.

## The task

### 1. Core surface (`rust/src/io/lines.rs`, new module wired from `io/mod.rs`)

Add an Arrow projection of matched line records:

- `IOBase::read_arrow_lines(&self, options: &LineRecordOptions) -> Result<BatchReader>`
  and the consuming `IOBase::into_arrow_lines(self, options) -> Result<BatchReader>`
  (mirroring the `read_lines` / `into_read_lines` pair; `into_*` is the
  shape the bindings hand across FFI). This is a text-line projection like
  `read_lines`, **not** a fourth record method — the three-method record
  surface (`read/write/append_arrow_batch_reader`) is untouched, and say so
  in the module docs.
- **Folder-or-file detection, and absence reads as empty.** The handle's
  `IOKind` decides the shape — never a second existence check, per
  `AGENTS.md` (`is_container` derives from `IOKind`; handles are lazy and a
  read on a missing resource skips):
  - `IOKind::File` / `Memory`: parse that one resource's lines.
  - `IOKind::Directory`: stream across the leaf files beneath it (the same
    posture as the record surface, where a container reads across its
    leaves) — enumerate with the existing container walk
    (`children_where`/`ls`), deterministic name-sorted order, leaves
    concatenated into one `BatchReader`. Each leaf contributes its own
    canonical `Url` in the `url` column and restarts `rownum` at 1 (the
    `(url, rownum)` pair stays a record identity), and each leaf's content
    codings are peeled from *its own* media type, so a folder mixing
    `a.log` and `b.log.gz` reads uniformly. Leaves are opened lazily, one
    at a time, as the reader advances — never enumerated-and-buffered.
  - `IOKind::Unknown` (the resource does not exist): return an **empty**
    `BatchReader` — zero batches, schema still answered — exactly as
    `read_lines` yields no lines and `pread` reads zero bytes. Absence is
    never an error on the read path.
- `LineRecordOptions` (in `io/lines.rs`): the header `pattern` (e.g.
  `r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]"`
  for `2024-02-01 10:00:00.000_000 [xx] [xxx]` lines), an optional
  `batch_size` (default 1024, one batch in memory at a time — the reader
  stays streaming end to end), an optional timestamp column name/format
  override, and `custom_fields`: an ordered list of `(name, Value)` constant
  columns appended to every row (how a caller stamps `source`, `session`,
  `venue` onto a file's rows).

### 2. Schema — a non-null Struct `Field`, safe for Iceberg as declared

A struct Field *is* the schema, and every base column must be a datatype the
strict Iceberg codec accepts **unchanged**: `iceberg::PrimitiveType::from_data_type`
(`rust/src/iceberg/types.rs:128`) has no unsigned integers, spells `time`
only at microsecond, and knows nothing of dictionary encodings — so the
schema below is chosen to pass it as-is, not merely after
`to_scheme_compat(&Scheme::ICEBERG)` widening.

Base columns, in this order:

| column    | datatype                 | Iceberg     | meaning |
| --------- | ------------------------ | ----------- | ------- |
| `url`     | `utf8` (plain — no dictionary wrapper; it would not survive the strict codec) | `string` | the handle's canonical `Url` display |
| `rownum`  | `int64`                  | `long`      | 1-based record index within the resource |
| `date`    | `date32`                 | `date`      | the entry's civil date |
| `time`    | `time64(microsecond)`    | `time`      | the entry's clock reading, truncated to micros (Iceberg's `time` has no nanosecond form; full precision lives in `unix`) |
| `unix`    | `int64`                  | `long`      | total nanoseconds since the Unix epoch (naive; document that no zone is applied) |
| `hash`    | `int64`                  | `long`      | stable hash of `message` only — the u64 FNV-1a state reinterpreted as two's-complement i64 (`u64 as i64`, bit pattern preserved, documented), because Iceberg has no unsigned types |
| `header`  | `utf8`                   | `string`    | the exact text the pattern matched |
| `message` | `utf8`                   | `string`    | the record with the header match removed, then trimmed |

Then one nullable `utf8` column per **named capture group** in the pattern,
in group order (`level`, `logger` above) — this is the primary "custom
fields" mechanism and what makes it a good trading-log parser. Then the
constant `custom_fields` columns, typed from their `Value` — validate each
custom column's datatype through `PrimitiveType::from_data_type` at option
construction and reject one Iceberg cannot spell (an unsigned integer, a
non-microsecond time) with the codec's own error, so a caller finds out
before the first batch rather than at table-append time. Reject a name
collision between base, capture, and custom columns with the project's
`expected X, got Y` error shape and a column path.

**Iceberg compatibility is a tested property, not an intention.** Add tests
asserting (a) `Field::to_scheme_compat(&Scheme::ICEBERG)` maps the full
emitted schema (base + capture + custom columns) without error or change,
and (b) — behind the `iceberg` feature, beside the existing iceberg tests —
an end-to-end round trip: assign field IDs with `assign_parquet_field_ids`,
create a table from the schema via `iceberg::Catalog`, `append` the parsed
`BatchReader`, and read the snapshot back row-identical, with the columns
whose types the encodings agree on carrying real bounds statistics.

A record whose opening line the pattern did not match (the preamble record a
rotated file starts with) gets null `date`/`time`/`unix`/`header` and
capture columns, the whole record as `message` — make those five columns
nullable, keep `url`/`rownum`/`hash`/`message` non-null.

### 3. Flexible datetime parsing — extend `rust/src/generic/iso.rs`

Extend the existing parser rather than writing a second one:

- Keep `T`/`t`/space as the date–time separator (already supported).
- Accept an optional fraction with `_` digit-group separators:
  `10:00:00.000_000` ≡ `10:00:00.000000`. Underscores are legal only
  *between* digits (reject leading/trailing/doubled `_`), and the digit
  count after removing them keeps the existing 1–9 rule and `TimeUnit`
  mapping. Add the same acceptance to `parse_time`/`parse_timestamp` since
  they share `parse_clock_at`.
- The Arrow projection normalizes whatever unit the fraction implies to
  nanoseconds for `unix` and truncates to microseconds for `time` (the
  Iceberg-safe `time64(microsecond)` — truncation, never rounding, and the
  docs say a sub-micro reading is only fully recoverable from `unix`).
- New adversarial tests in `rust/src/generic/iso/tests.rs`: `_` variants
  round-tripping to the same counts, and every malformed underscore
  placement rejected with byte position.

### 4. Hashing — decide with a benchmark, keep the stable contract

The hash covers **only** `message` (header stripped, trimmed). Default to
the project's stable FNV-1a contract — expose a `pub(crate)` (or pub, if the
bindings need it) `stable_hash_bytes(&[u8]) -> u64` next to
`stable_hash_display` so the line path hashes without a `Display`
indirection. Before considering any alternative (e.g. an xxHash-family
crate), benchmark FNV-1a on realistic 100 B–2 KiB log messages; add a
dependency only if the benchmark shows FNV-1a is the bottleneck of the whole
read (it will not be — regex + UTF-8 dominate), and record the numbers
either way. Whatever wins must be deterministic and documented as the value
`hash` holds, so it can serve as a dedupe/join key across runs.

### 5. Tests — uncompressed and compressed, IO-based both ways

Mirror the existing test layout (`rust/src/io/tests.rs` for unit-level,
`rust/tests/` for runtime regressions):

- In-memory `Buffer` and a `local::File` on disk, identical content,
  identical batches out.
- The same content gzip-coded as `events.log.gz` (media type declares the
  coding): batches must be byte-identical to the uncompressed read. Cover at
  least one more coding (`.zst`) since the peeling is generic.
- Multi-line records (stack traces), a preamble before the first match, an
  empty and a missing resource (zero batches, schema still answered), a
  missing *folder* path (also zero batches, not an error), CRLF endings,
- Folder handles: a directory of several log files read as one stream in
  name-sorted order, per-leaf `url` and `rownum` restarting at 1, a folder
  mixing uncompressed and gzip leaves (`a.log` + `b.log.gz`) decoding each
  by its own media type, an empty folder (zero batches), and laziness
  observable — a leaf after the first is not opened until the reader
  reaches it (assert via a leaf that fails to decode: earlier batches must
  still arrive before its error surfaces), `batch_size` boundaries (records % batch_size ≠ 0), a malformed
  timestamp inside a matching header (typed error naming row and byte
  position — never a silent null for a *matched* header), named-capture
  columns, custom constant columns, and the collision rejection.

### 6. Benchmarks — against something the reader trusts

- Rust: extend `rust/benchmarks/io.rs` (keep existing Criterion group IDs;
  the target already requires `parquet`) with a `lines_arrow` group: parse
  throughput (MiB/s and rows/s) over ~100k synthetic trading-log lines,
  uncompressed vs gzip, hash on vs a no-hash variant to isolate its cost,
  and the FNV-1a hashing micro-benchmark from §4.
- Python: add `python/benchmarks/log_lines.py` beside the existing
  `records_io.py`, comparing against a plain-Python `re` + `str.split` loop
  and (if a fair comparison exists) `pyarrow.csv`-style ingestion, on the
  same payload, uncompressed and gzip. Release builds only
  (`maturin build --release`); regenerate — never edit — the affected table
  in `docs/benchmarks.md`, naming machine, interpreter, and profile.

### 7. Bindings and docs (after the core is complete)

- Python (`python/src/io.rs`): extend the existing `read_lines` neighborhood
  with `read_arrow_lines(pattern, *, batch_size=None, custom_fields=None)`
  returning a `pyarrow.RecordBatchReader` over the Arrow C Stream (lazy, as
  the record methods already do). Update `python/yggdryl/_native.pyi`, add
  `python/tests/test_io.py` cases including a gzip file, and a boundary
  benchmark.
- Node: same operation via the standard copied-IPC boundary; never claim
  zero-copy.
- Docs: extend `docs/io.md`'s lines section with the Arrow projection —
  every example in Rust/Python/JavaScript tabs, self-contained with
  assertions, passing `python scripts/check_docs_examples.py` and
  `python -m mkdocs build --strict`.

### 8. Nice-to-haves (implement if cheap, otherwise list them in the PR body)

- `offset` (int64): byte offset of the record's first line in the *decoded*
  stream — the resume/seek key a tailing trading pipeline wants.
- Glob handles: a `logs/2024-02-*/app.log*` URL routed through the existing
  `Url::is_glob` / `IOBase::glob` walk selects its matching leaves with the
  same per-leaf semantics as a folder — no new matching logic.
- `lines` (int32): line count of the record, a free flag for exceptions.
- A `timezone` option: when set, `unix` is interpreted in that zone and
  becomes a zoned timestamp column instead of naive int64 nanos (route
  through the existing `Timezone`/`parse_timestamp` machinery). Default the
  zoned column to `timestamp(microsecond, tz)` — Iceberg's `timestamptz`,
  safe in every format version — and offer `timestamp(nanosecond, tz)`
  (`timestamptz_ns`) only as an explicit opt-in, documented as requiring
  Iceberg format version 3.
- A lenient mode flag: unparseable timestamps in matched headers become
  nulls with a counter surfaced somewhere inspectable, for dirty archives —
  strict stays the default.
- A declared-schema hook: let the caller pass a target Struct `Field` and
  route through the one `cast_arrow_reader` definition so capture columns
  can land typed (`level` as dictionary, a captured `price` as decimal).

## Validation gates (all must pass)

```console
cargo fmt --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg" -- -D warnings
cargo test --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml --features "parquet iceberg"
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

Plus the binding test suites for any binding you touch. Follow the exact
method vocabulary and error contract of `AGENTS.md` (`expected X, got Y`,
byte positions, `Error::InvalidRecord`-style paths); never panic on
caller-controlled input; keep every read one-batch-in-memory streaming.
