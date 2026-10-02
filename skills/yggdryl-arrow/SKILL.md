---
name: yggdryl-arrow
description: Moves columns, tables and streams across the Apache Arrow boundary with yggdryl's Serie (one column), ChunkedSerie (chunked arrays and tables kept apart) and SerieReader (a stream under one compiled plan), and casts them with ArrowCastPlan and ArrowCastOptions (safe, representation). Use when landing arrow-rs arrays / RecordBatches / readers, pyarrow or Arrow JS (apache-arrow) tables and vectors under a Field (from_arrow_array / fromArrowArray, from_arrow_batch, SerieReader.from_arrow_reader), taking pyarrow, pandas, polars or NumPy in via Serie.from_, reading typed buffers (as_int64().values()), casting a column or a stream, sorting, deduplicating, grouping or windowing a column (sort_indices / into_sorted, into_unique, partition_by, window), cutting a column or stream into windows of equal keys (window_by / windowBy, static_values), or handing data back (into_arrow_array / intoArrowTable, into_pandas / into_polars). Covers Rust, Python and Node.js.
---

# yggdryl Arrow: Serie, ChunkedSerie, SerieReader, casts

Every value that crosses Apache Arrow lands in one of three holders, and
every cast runs through them:

- **`Serie`** - many values: the Arrow buffers of one `Field` (a column; a
  record column is a table), or a schema-free run. It is the one type that
  reads a cell, casts, or builds an Arrow array or batch.
- **`ChunkedSerie`** - `Serie` columns of one field held apart: a chunked
  array, or a table of one batch per chunk. Nothing is concatenated until
  `into_serie`.
- **`SerieReader`** - a stream: one record `Serie` per batch, every batch cast
  by one `ArrowCastPlan` compiled from the stream's schema before the first
  pull. At most one source batch is held.

The **field is the cast target**, never the source. An input already laid
out as the field's projection is the identity plan: its buffers are shared
and no row is read (except rows of a leaf whose layout is narrower than its
datatype - a code, a sized string, a decimal, `date64`, a `duration32`, a
URL, a map - each read once). Any other layout is cast by one plan. Whether a value may be absent is
the target field's nullability; `safe` only decides whether a present value
that fails to convert becomes null.

`RecordBatch`, `ArrayRef`, `BatchReader` and their pyarrow / Arrow JS
counterparts are transport, never a second collection API. Install and
cross-language conventions: see the `yggdryl` entry skill.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| Rows in hand -> column | `Serie::from_scalars(field, rows)?` | `Serie.from_scalars(field, rows)` | `Serie.fromScalars(field, rows)` |
| Default / empty column | `Serie::from_default(field, n)?`, `Serie::empty(field)?`, `Serie::with_capacity(field, n)?` | `Serie.from_default(field, rows=1)`, `Serie.empty(field)`, `Serie.with_capacity(field, n)` | `Serie.fromDefault(field, rows?)`, `Serie.empty(field)`, `Serie.withCapacity(field, n)` |
| One Arrow array -> column | `Serie::from_arrow_array(Some(&field), array, options)?` (`None` = its own layout, named `item`) | `Serie.from_arrow_array(array, field=None, *, safe=True, representation="value")` | `Serie.fromArrowArray(vector, field?, { safe, representation }?)` |
| One batch / table -> record column | `Serie::from_arrow_batch(Some(&root), &batch, options)?` | `Serie.from_arrow_batch(batch, root=None, ...)` | `Serie.fromArrowBatch(batchOrTable, root?, options?)` |
| Drain a stream into one column | `Serie::from_arrow_reader(Some(&root), reader, options)?` | `Serie.from_arrow_reader(reader, root=None, ...)` | `Serie.fromArrowReader(batchReader, root?, options?)` |
| Any columnar runtime object -> column | - (use the three doors above) | `Serie.from_(value, field=None, ...)` - pyarrow, pandas, polars, NumPy, Arrow C exporters | - (Arrow JS doors above) |
| Stream lazily, one plan, bounded memory | `SerieReader::from_arrow_reader(Some(&root), reader, options)?` | `SerieReader.from_arrow_reader(reader, root=None, ...)`, `SerieReader.from_(value, root=None, ...)` | `SerieReader.fromArrowReader(batchReader, root?, options?)` |
| Held column / chunks as a stream | `SerieReader::from_serie(serie)?`, `SerieReader::from_chunked(chunked)?` | `SerieReader.from_serie(serie)`, `SerieReader.from_chunked(chunked)` | `SerieReader.fromSerie(serie)`, `SerieReader.fromChunked(chunked)` |
| Re-root a stream under another field | `reader.cast(&target, options)?` | `reader.cast(field, ...)` | `reader.cast(field, options?)` |
| Stream out (transport, never landed) | `reader.into_arrow_reader()` -> `BatchReader` | `reader.into_arrow_reader()` -> `pyarrow.RecordBatchReader` | `reader.intoArrowReader()` -> `BatchReader` |
| Chunked array, chunks kept apart | `ChunkedSerie::from_arrow_arrays(Some(&field), arrays, options)?` | `ChunkedSerie.from_arrow_chunked_array(chunked, field=None, ...)`, `ChunkedSerie.from_(value)` | `ChunkedSerie.fromArrowArray(vector, field?, options?)` (one chunk per `Data`) |
| Table, one chunk per batch | `ChunkedSerie::from_arrow_reader(Some(&root), reader, options)?` | `ChunkedSerie.from_arrow_reader(reader, root=None, ...)`, `ChunkedSerie.from_(table)` | `ChunkedSerie.fromArrowBatch(table, root?, options?)`, `ChunkedSerie.fromArrowReader(batchReader, root?, options?)` |
| Columns in hand as chunks | `ChunkedSerie::from_series(Some(&field), chunks, options)?` | `ChunkedSerie.from_series(chunks, field=None, ...)` | `ChunkedSerie.fromSeries(chunks, field?, options?)` |
| Append one chunk / join all | `push_chunk(serie, options)?` / `into_serie()?` | `push_chunk(serie)` / `into_serie()` | `pushChunk(serie, options?)` / `intoSerie()` |
| Cast a column in hand, once | `serie.cast(&target, options)?` | `serie.cast(field_or_dtype, *, safe, representation)` | `serie.cast(field, options?)` |
| Cast every chunk by one plan | `chunked.cast(&target, options)?` | `chunked.cast(field, ...)` | `chunked.cast(field, options?)` |
| Compile a cast once, apply many | `ArrowCastPlan::compile(&source, &target, options)?` then `apply(&serie)?` / `apply_chunked(&chunked)?` | `ArrowCastPlan(source, target, *, safe=True, representation="value")` then `apply(x)` | `ArrowCastPlan.compile(source, target, options?)` then `apply(x)` |
| Refuse an impossible cast before rows | `plan.preflight()?`, `plan.is_identity()` | `plan.preflight()`, `plan.is_identity` | `plan.preflight()`, `plan.isIdentity` |
| Cast options | `ArrowCastOptions::new().with_safe(false).with_representation(Representation::Bits)` | `safe=False, representation="bits"` keywords | `{ safe: false, representation: 'bits' }` |
| Typed buffer read, per row | `serie.as_int64()` then `values()`, `value(i)`, `nulls()`; `as_utf8()` then `value(i)`, `offsets()`, `payload()` | Rust only: hand the column to pyarrow (`into_arrow_array()`, zero copy) | Rust only: `asJs()` or `intoArrowArray()` once |
| One row / every row | `scalar(i)?`, `get(i)`, `rows()`, `iter()` | `scalar(i)`, `serie[i]`, `get(i)`, `rows()`, `as_py()` | `scalar(i)`, `at(i)`, `rows()`, `[...serie]`, `asJs()` |
| Write rows | `push`, `set`, `insert`, `remove`, `pop`, `extend`, `splice(range, rows)`, `resize`, `truncate`, `clear`, `extend_from_serie` | same names, `splice(start, end, rows)` | `push`, `set`, `insert`, `remove`, `pop`, `extend`, `splice(start, end, rows)`, `resize`, `truncate`, `clear`, `extendFromSerie` |
| Children of a record column | `child(name)`, `children()`, `items()`, `get_child_by_path(&FieldPath)`, `set_child(serie)?`, `set_cell(&path, i, v)?` | `child`, `children()`, `items()`, `get_child_by_path("a.b")`, `set_child`, `set_cell("a.b", i, v)` | `child`, `children()`, `items()`, `getChildByPath`, `setChild`, `setCell` |
| Zero-copy window | `serie.slice(offset, len)?` | `serie.slice(offset, len)`, `serie[a:b]` | `serie.slice(offset, len)` |
| Read or write a stretch where it stands | `serie.window(offset, len)?`, `window_mut(offset, len)?` (`set`, `fill`, `swap`, `as_sorted`, ...) | `serie.window(offset, length)` -> `WindowSerie` | `serie.window(offset, length)` -> `WindowSerie` |
| Sort, order, deduplicate, take, filter (a new serie) | `sort_indices(options)?`, `into_sorted(options)?`, `into_unique()?`, `into_reversed()`, `into_taken(&indices)?`, `into_filtered(&mask)?`, `is_sorted(options)`, `is_unique()`, `unique_count()` | same names; `options` are `descending=False, nulls_first=False` keywords | `sortIndices`, `intoSorted`, `intoUnique`, `intoReversed`, `intoTaken`, `intoFiltered`, `isSorted`, `isUnique`, `uniqueCount`; `options` is `{ descending, nullsFirst }` |
| The same, in place and chained | `serie.as_sorted(options)?.as_unique()?.as_reversed()?`, `as_taken`, `as_filtered` | `serie.as_sorted().as_unique().as_reversed()`, `as_taken`, `as_filtered` | `serie.asSorted().asUnique().asReversed()`, `asTaken`, `asFiltered` |
| Group rows by a key | `partition_by(&keys)?`, `partition_by_paths(&paths)?`; a chunked serie's keys held in chunks: `partition_by_chunked` | `partition_by(keys)`, `partition_by_paths("venue")` | `partitionBy(keys)`, `partitionByPaths('venue')` |
| Rows into windows of equal keys, as views | `serie.window_by("venue", sorted)?` -> `SerieWindows`; `for (key, window) in &windows`, each a `WindowSerie`; also on a `WindowSerie` | `serie.window_by("venue", sorted=False)` -> `[(key, WindowSerie)]` | `serie.windowBy('venue', sorted?)` -> `[[key, WindowSerie]]` |
| The same across chunks, no join | `chunked.window_by("venue", sorted)?` -> `Vec<(Scalar, ChunkedSerie)>` | `chunked.window_by("venue", sorted=False)` | `chunked.windowBy('venue', sorted?)` |
| A stream's windows, lazily, in order | `reader.window_by("venue", sorted)?` -> `SerieReaderWindows` of `SerieReader` | `reader.window_by(...)` -> `SerieReaderWindows` | `reader.windowBy(...)` -> `SerieReaderWindows` |
| The values constant over a window | `window.static_values()` / `reader.static_values()` -> `Option<FieldScalar>`: `get_key_str("venue")`, `windownum`, `rownum` | `window.static_values` -> struct `Scalar` or `None`, `record["venue"]` | `window.staticValues` -> struct `Scalar` or `null`, `record.get('venue')` |
| Bytes a column occupies | `serie.memory_size()` | `serie.memory_size()` | `serie.memorySize()` |
| Column -> Arrow | `into_arrow_array()` (`None` for a run), `require_arrow_array()?`, `into_arrow_batch()?`, `into_arrow_reader()?`, `into_arrow_scalar()?` | `into_arrow_array()`, `into_arrow_batch()`, `into_arrow_table()`, `into_arrow_reader()`, `into_arrow_scalar()`, `into_pandas()`, `into_polars()`, `into_numpy()`; PyCapsule: `pa.array(serie)`, `pa.table(record)` | `intoArrowArray()`, `intoArrowBatch()`, `intoArrowReader()`, `intoArrowScalar()` |
| Chunked -> Arrow | `into_arrow_arrays()`, `into_arrow_reader()?` | `into_arrow_chunked_array()`, `into_arrow_table()`, `into_arrow_reader()` | `intoArrowArray()`, `intoArrowTable()`, `intoArrowReader()` |
| Column as one value, and back | `Scalar::from(serie)`, `value.as_serie()` | `serie.into_scalar()`, `Scalar.from_(serie)`, `value.as_serie()` | `serie.intoScalar()`, `value.asSerie()` |
| Arrow schema <-> root `Field` | `Field::from_arrow_schema("row", &schema)?`, `root.into_arrow_schema()?` | `Field.from_arrow_schema(schema, name="row")`, `root.into_arrow_schema()` | no projection: `BatchReader.from(table).field`, `Serie.fromArrowBatch(table).field` |
| Arrow type / field <-> `DataType` / `Field` | `DataType::from_arrow_datatype(&t)?`, `dtype.into_arrow_datatype()?`, `Field::from_arrow_field(&f)?`, `field.into_arrow_field()?` | `DataType.from_arrow(t)`, `dtype.into_arrow()`, `Field.from_arrow(f)`, `field.into_arrow()` | `DataType.fromArrow(t)`, `Field.fromArrow(f)` (crossed as IPC, nullability included) |
| Build / merge batch streams | `yggdryl::arrow::batch_reader(schema, batches)`, `yggdryl::arrow::combined(left, right)?`, `combined_as(left, right, &root, safe)?` | pyarrow readers directly; `yggdryl.combined(left, right, schema=None, *, safe=True)` | `BatchReader.from(tableOrBatchesOrIpc)`, `BatchReader.fromIpc(bytes)`, `left.combined(right, schema?)`, `intoTable()`, `intoIpc()` |

In Python and JavaScript, `field` / `root` accept a `Field`, a field
expression (`"price: int64 not null"`), and in Python a `pyarrow.Field`; in
Rust, build the `Field` explicitly (`Field::from_str(...)` / `.parse()`) and
pass `Some(&field)` - there is no `&str` coercion. A `DataType` passed as a
cast target is its **required** field named `value`. A batch or stream root
is a non-null struct `Field`; with none, the input's own schema is read as
the record `row`.

## Rules for fast, correct use

1. **One plan per stream.** `serie.cast` and every `Serie` Arrow door compile
   a plan per call. A batch loop holds a `SerieReader` or an `ArrowCastPlan`
   instead: 1.7x faster at 1,000 batches of 64 rows, and the gap grows with
   the batch count.
2. **Declare the layout you already have.** An exact layout is the identity
   plan: the same buffers come back (`is_identity` says so) and no row is read
   for layout-is-contract leaves (ints, floats, bool, `utf8`, binary,
   `date32`, datetimes, `duration64`, intervals, `uuid`, and struct, serie,
   union and encoded nestings of them). A `duration32` (Arrow has one
   duration width) and every map (Arrow proves no key uniqueness) are read
   once.
3. **Pick the holder by shape.** Contiguous and held: `Serie`. Chunks or
   batches you want kept apart: `ChunkedSerie` (a clone or slice moves chunk
   pointers, never a row). Larger than memory or read once: `SerieReader`.
   `Serie.from_arrow_reader` / `Serie.from_` drain the whole stream.
4. **Narrow once, read per row.** In Rust, `as_<leaf>()` once, then
   `values()` / `value(i)` per row: a bounds check and a load, no value built.
   `scalar(i)`, `get`, `rows()` and `iter()` build a `Scalar` per row every
   time and cache nothing - hold the answer rather than asking twice.
   `values()` includes the slots under nulls; read `value(i)` or `nulls()`.
5. **Absence is the target field's nullability, not an option.** A nullable
   column holds null. A required column refuses a null, a missing column and an
   empty text cell by path (`$.symbol`) and never writes its default.
6. **`safe` is about present values only.** `safe=true` (default): a value
   that fails to convert becomes null - but only where the column may hold
   one. A required column refuses that value by name whatever `safe` says.
   `safe=false` refuses it everywhere.
7. **Empty text is no value.** A zero-length text cell entering a non-text
   column is null before `safe` is consulted; a whitespace-only cell is not
   empty but a failed conversion. String and byte columns keep `""`. An
   interval column is not given the null: it parses `""` and fails, so the
   empty cell is null only under `safe` in a nullable column and refused by
   value under `safe=false` or `not null`.
8. **A nested value and text meet through JSON.** A struct, serie or map
   cast into text or bytes spells its compact JSON (struct keys in
   declaration order) and text cast back reads it, at any depth -
   `map<utf8, struct<..>>` and `map<utf8, utf8>` are one cast apart. It is a
   cast's reading (`cast`, `cast_scalar`): `DataType.scalar` / `Field.scalar`
   still refuse a nested value into text. Never hand-roll `json.dumps` per row
   or a string column per struct field.
9. **`representation="bits"` is a preference for same-width pairs** (`uint64`
   <-> `int64` <-> `float64` <-> `fixed_size_binary(8)`): the value buffer is
   shared, `u64::MAX` reads `-1`. Different widths or rule-governed targets
   (codes, UUIDs, fixed strings) convert as usual.
10. **Missing required columns fail at compile time; nulls fail at the pull.**
   `ArrowCastPlan` / `SerieReader` constructors refuse what the two schemas
   alone refuse; a batch's null or bad value surfaces when that batch is
   pulled, and the reader is fused after the first failure.
11. **A stream is never a `Scalar`.** A held column becomes one value with
    `Scalar::from(serie)` / `into_scalar()` at no copy; `Scalar.from_(reader)`
    drains the stream. `SerieReader`s are one-shot: iterating,
    `into_arrow_reader` and `cast` each consume them.
12. **Python is zero copy; JavaScript is copied IPC.** Python crosses the Arrow
    C Data Interface and PyCapsule interface, sharing buffers, and lands,
    joins and casts off the GIL; `into_numpy` copies (a null becomes `nan`).
    Every JavaScript door serializes one self-contained IPC stream per array,
    batch or table: cross whole tables or readers, never a row at a time.
13. **Write in bulk.** `splice` is the one mutation and a refusal leaves the
    column unchanged. `from_scalars` / `extend` build in one pass; `push` on a
    schema-free run copies the run (quadratic). A byte-leaf `set` rebuilds
    from that row on. The first write to a shared or foreign buffer copies it
    once. `extend_from_serie(other)` appends a column whose datatype agrees
    and whose nullability fits buffer to buffer with no row read; any other
    column is read row by row through `Field::scalar` (no cast options, no
    `safe`), so cast `other` onto the field first when its layout differs and
    you want the cast rules.
14. **`into_serie` joins the chunks**: no chunk is the empty column (no
    buffers), one chunk is itself (shared, zero copy), and only two or more
    chunks are concatenated into new buffers. `into_arrow_batch`,
    `into_arrow_reader` and `SerieReader.from_serie` refuse a record column
    holding a null row, because a batch states no row validity.
15. **Schema-changing loops compile one plan per distinct source schema.** A
    plan refuses an input of another layout by name; key your plans by the
    source schema rather than recompiling per batch.
16. **One order for every leaf.** `sort_indices`, `into_sorted`, `into_unique`
    and `partition_by` answer `Scalar`'s total order: absent values go last
    (nulls first is an option, and the opposite of Arrow's own default), every
    NaN is one value above every number, and a column and the run of its rows
    agree. `into_*` leaves the serie as it was; `as_*` rewrites it in place -
    a primitive or boolean column held alone sorts where it stands - and a
    refusal leaves it unchanged. A `ChunkedSerie` joins for `sort_indices`,
    `into_sorted`, `into_unique` and `into_taken`, and keeps its chunks apart
    for `into_reversed`, `into_filtered` and `partition_by`.
16. **Window by key instead of grouping by hand.** `window_by(by, sorted)`
    takes a selector (`"venue, minutes(ts, 15) as bucket"`), computes the key
    once and lends each window as a view - one key column and one bit per
    row, then each window costs its key. With
    `sorted=false` a window is a run of equal adjacent keys, so a key that
    comes back opens another; with `sorted=true` each key comes once, in
    key order: keys already in order copy nothing, a held serie gathers its
    rows once only where the keys descend, a `ChunkedSerie` regroups its
    runs as zero-copy pieces, and a stream verifies the order and refuses a
    key going backwards. Every window a `Serie`, a `WindowSerie` or a
    `SerieReader` cuts states its record - the key cells, `windownum` and
    `rownum` (null after a gather) - as `static_values`; a chunked window
    states none, its key and index being its record. A key cell named
    `windownum` or `rownum` is refused wherever a record is stated: alias it.

## Pitfalls

- **Wrong:** `for batch in reader: Serie.from_arrow_batch(batch, root)` (or
  `serie.cast(...)` per batch). **Right:** `for s in
  SerieReader.from_arrow_reader(reader, root)`, or compile
  `ArrowCastPlan(schema, root)` once and `plan.apply(batch)`.
- **Wrong:** `Serie.from_arrow_reader(huge_reader)` to process a large file.
  **Right:** `SerieReader` (one batch held), or
  `ChunkedSerie.from_arrow_reader` when you need random access without a join.
- **Wrong:** summing `serie.scalar(i)` in a loop. **Right:** Rust
  `serie.as_int64().ok_or("not an int64 column")?.values()`; Python `pyarrow.compute.sum(serie.into_arrow_array())`
  (zero copy); JavaScript `asJs()` once.
- **Wrong:** `safe=False` to "keep nulls out" or `safe=True` to "let nulls
  into a required column". **Right:** declare the field nullable or
  `not null`; `safe` never changes what absence means.
- **Wrong:** expecting a required field to fill a missing column or a null
  with `0` / `""`. **Right:** it is refused (`required Arrow field $.x holds 1
  null values`); make the field nullable, or fill upstream.
- **Wrong:** `serie.cast(DataType("int64"))` on a column with nulls, then
  surprise at `$.value holds 1 null values`. **Right:** a `DataType` target is
  the required `value` field; pass a nullable `Field` to keep nulls.
- **Wrong:** `Serie.from_(pa.chunked_array(...))` when the chunks should stay
  apart. **Right:** `ChunkedSerie.from_(...)` - `Serie.from_` joins the chunks
  (a copy once there are 2+; a single chunk is shared, not copied).
- **Wrong (JS):** `Serie.fromArrowReader(arrowTable)`. **Right:**
  `Serie.fromArrowBatch(table)` or `Serie.fromArrowReader(BatchReader.from(table))`
  - `fromArrowReader` takes only a native `BatchReader`, and consumes it.
- **Wrong (JS):** `new ChunkedSerie()`. **Right:** a static:
  `ChunkedSerie.fromSeries`, `fromArrowArray`, `fromArrowBatch`, `empty`.
- **Wrong:** reusing a `SerieReader` after `into_arrow_reader()`, `cast` or a
  full iteration. **Right:** build a new one; a stream is read once.
- **Wrong:** `Serie([1, 2]).cast(field)` / `Serie::new(rows).cast(..)`. A
  schema-free run lays out no buffers and is refused. **Right:**
  `Serie.from_scalars(field, rows)`.
- **Wrong (Rust):** `serie.as_serie()` to get the column a `Scalar` holds.
  **Right:** `Scalar::as_serie` borrows the whole `Serie`; `Serie::as_serie`
  narrows a column to its `SerieSerie` (list) leaf.
- **Wrong:** trusting an `ARROW:extension:name` label (`yggdryl.url`) to skip
  validation. **Right:** a label is not proof; those rows are read once and
  the first bad one is named (`$[0].u`).
- **Wrong:** `chunked.push_chunk(foreign)` in a loop of foreign chunks (a plan
  per call). **Right:** `ChunkedSerie.from_series(chunks, field)` - one plan
  per run of chunks of one source layout.
- **Wrong:** `windows = list(reader.window_by("venue"))`, then reading
  them. **Right:** read each stream window before taking the next; a window
  its walk passed is refused once (`window 0 was passed by its walk`). Hold
  the stream as a `ChunkedSerie` first when windows must be revisited.
- **Wrong:** `partition_by` to find runs of a key, or `window_by(.., sorted=True)`
  over a stream whose keys arrive out of order. **Right:** `window_by(..,
  sorted=False)` cuts runs; a stream is never reordered, so window it
  unsorted or hold it (`ChunkedSerie.from_(reader)`) and window that sorted.
- **Wrong:** mutating a child column to edit a nested cell. **Right:** no
  child is handed out mutably; use `set_cell("a.b", i, v)` or replace a whole
  child with `set_child`.

## Language references

- `references/rust.md` - read when writing Rust: `arrow-array` /
  `arrow-schema` 59 interop, typed leaf narrowings, typed writers.
- `references/python.md` - read when writing Python: pyarrow, pandas,
  polars, NumPy intake through `from_`, PyCapsule out, zero-copy checks.
- `references/javascript.md` - read when writing Node.js: Apache Arrow JS
  doors, `BatchReader`, copied IPC.
- `references/cast-rules.md` - the cast contract as tables: `safe`,
  `representation`, required columns, empty text, struct reconciliation, what
  a landing proves, when failures surface.

## Deeper

- Serie: https://platob.github.io/yggdryl/types/serie/ - leaves, costs,
  [every columnar runtime in](https://platob.github.io/yggdryl/types/serie/#arrow-every-columnar-runtime-in)
  [sorting, uniqueness and partitions](https://platob.github.io/yggdryl/types/serie/#sorting-uniqueness-and-partitions),
  [windows](https://platob.github.io/yggdryl/types/window-serie/),
  [windows by key](https://platob.github.io/yggdryl/types/serie/#windows-by-key),
  [windows of a stream](https://platob.github.io/yggdryl/arrow/readers/#windows-of-a-stream)
- ChunkedSerie: https://platob.github.io/yggdryl/types/chunked-serie/
- Cast: https://platob.github.io/yggdryl/types/cast/ -
  [required columns](https://platob.github.io/yggdryl/types/cast/#required-columns),
  [compiled plans](https://platob.github.io/yggdryl/types/cast/#compiled-plans),
  [eager and lazy](https://platob.github.io/yggdryl/types/cast/#eager-and-lazy)
- Arrow overview and defaults: https://platob.github.io/yggdryl/arrow/
- Schema projection: https://platob.github.io/yggdryl/arrow/schema/
- Batch readers and `combined`: https://platob.github.io/yggdryl/arrow/readers/
- Sibling skills: `yggdryl-types` (building `DataType`, `Field`, `Scalar`),
  `yggdryl-records` (reading and writing files: `read_arrow`,
  `write_arrow`, `read_arrow_reader`), `yggdryl-expressions` (filters,
  selectors and plans over Arrow batches).
