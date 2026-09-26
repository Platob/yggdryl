---
name: yggdryl-records
description: Reads and writes rows and Arrow batches on any yggdryl handle - Arrow IPC, Parquet, Avro, plain-text logs, Iceberg tables and hive-partitioned folders - with streamed readers, pushdown, append/merge (upsert) and commit cadence. Use when calling read_arrow_reader / readArrowReader, overwrite_arrow_* / append_* / merge_*, write_arrow_* with a mode, *_records / readRecords, read_arrow / write_arrow (SerieReader), RecordOptions (field, safe, select, filter, merge_by, max_row_size, commit_row_size, plan), TextOptions rowheader, iceberg Table create/append/merge/scan, partition pruning, or picking an encoding by suffix. Covers Rust, Python and Node.js.
---

# Records

`IOMedia` is the record surface every handle answers: one streamed read
(`read_arrow_reader`) and three explicit write intents (overwrite, append,
merge) in three input shapes (a batch reader, one batch or table, native
rows). The handle's media type - its suffix or declared `MediaType` - picks
the encoding; one `RecordOptions` carries every setting, and no call takes a
format argument. A schema is a non-null Struct `Field`; a read hands back a
stream in which only the current batch is alive.

Hold one model: **the options are the query.** `field` declares and casts,
`select` projects, `filter` prunes (row groups, partition leaves, Iceberg
manifests) and then filters rows, `max_row_size` stops pulling, `merge_by`
keys an upsert, `commit_row_size` bounds a write. Put them on the call and the
medium does the work before a byte is decoded.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| options for this handle's encoding | `handle.record_options()?` | `handle.record_options()` | `handle.recordOptions()` |
| options from a name or type, no handle | `RecordOptions::for_media_type(&url.media_type())?`, `RecordOptions::for_mime_type(&MimeType::PARQUET)?` | `RecordOptions("trades.parquet")` | `RecordOptions.from('trades.parquet')`, `RecordOptions.forMimeType(MimeType.ARROW_STREAM)` |
| stored schema, no rows decoded | `read_arrow_field(&options)?` | `read_arrow_field()` | `readArrowField()` |
| row and column counts from metadata | `row_size()?`, `column_size()?` | `.row_size`, `.column_size` | `.rowSize`, `.columnSize` |
| stream batches out | `read_arrow_reader(&options)?` -> `arrow::BatchReader` | `read_arrow_reader()` -> `pyarrow.RecordBatchReader` | `readArrowReader()` -> `BatchReader` of Arrow JS batches |
| stream record columns out | `read_arrow(Some(&options))?` -> `SerieReader` | `read_arrow()` -> `SerieReader` | not bound |
| rows out as native values | `read_arrow` + `serie.child(name)` / `scalar(i)` | `read_records()`, `read_records(Cls)` | `readRecords()`, `readRecords(Cls)` |
| replace | `overwrite_arrow_reader(reader, &options)?`, `overwrite_arrow_batch` | `overwrite_arrow_reader`, `_table`, `_batch` | `overwriteArrowReader(BatchReader.from(x))`, `overwriteArrowTable`, `overwriteArrowBatch` |
| append | `append_arrow_reader`, `append_arrow_batch` | `append_arrow_reader`, `_table`, `_batch` | `appendArrowReader`, `appendArrowTable`, `appendArrowBatch` |
| upsert by key | `merge_arrow_reader(r, &options.with_merge_by(["id"])?)?` | `merge_arrow_table(t, merge_by=["id"])` | `mergeArrowTable(t, { mergeBy: ['id'] })` |
| mode chosen at run time | `write_arrow_reader(r, IOMode::Append, &options)?`, `write_arrow_batch`, `write_records` | `write_arrow_table(t, "append")`, `write_arrow_reader`, `write_arrow_batch`, `write_records` | `writeArrowTable(t, 'append')`, `writeArrowReader`, `writeArrowBatch`, `writeRecords` |
| native rows in | `overwrite_records(rows, &options)?` (rows `Into<Scalar>`) | `overwrite_records([dict or @scalar instance])` | `overwriteRecords([object], { field })` |
| a `SerieReader` or held column in (also JSON/YAML/TOML/XML rows) | `write_arrow(SerieReader::from_serie(s)?, IOMode::Overwrite, None)?` (a document handle takes `Overwrite` only) | `write_arrow(value, "overwrite")` (document handle: overwrite only) | not bound |
| refuse values the declared field cannot convert | `options.with_field(root).with_safe(false)` | `read_arrow_reader(field=f, safe=False)` | `readArrowReader({ field, safe: false })` |
| one setting for one call | `options.clone().with_select(["id"])?.with_filter("id > 3")?` | `read_arrow_reader(select=["id"], filter="id > 3")` | `readArrowReader({ select: ['id'], filter: 'id > 3' })` |
| sections as one plan | `options.with_plan("select id where x > 1 limit 5")?` | `options.plan = "select ..."` | `options.withPlan('select ...')` |
| Parquet page codec | `options.set_parquet_compression_name("zstd(3)")?`, `ParquetOptions::new().with_compression(..)` | `compression="zstd(3)"` | `{ compression: 'zstd(3)' }`, `withCompression` |
| Parquet footer statistics | `read_parquet_statistics()?` | `read_parquet_statistics()` | `readParquetStatistics()` |
| Avro bytes with a reader schema | `avro::read_container_resolved(&h, &schema)?` | `avro.loads(data, reader_schema=...)` | `avro.loads(data, { readerSchema })` |
| log lines as typed rows | `handle.into_text_with(TextOptions)` | `IOBase(p).into_text(TextOptions())`, or `read_arrow_reader(rowheader=...)` | `new IOBase(p).intoText(opts)`, or `readArrowReader({ rowheader })` |
| write a partitioned folder | `Holder::folder(&root)?.overwrite_arrow_reader(r, &options)?` | `IOBase(dir).overwrite_arrow_batch(b, options=o)` | `new IOBase(dir).overwriteArrowTable(t, options)` |
| leaves of one partition | `children_where(&[("year", "2024")], false)?` | `children_where({"year": "2024"})` | `childrenWhere({ year: '2024' })` |
| derived partition column | `field.as_partition_mut().set_transform(Function::Year)?`, `root.as_transform().apply_arrow_batch(&b)?` | `field.partition.transform = "year"`, `root.partition.apply_arrow_batch(b)` | not bound |
| Iceberg table | `Table::create(LocalFolder::new(p)?, FormatVersion::V2, schema, spec)?` | `Table.create(IOBase(p), schema, ["venue"])` | `iceberg.Table.create(p, schema, ['venue'])` |
| Iceberg write | `commit_append(r)?`, `commit_overwrite`, `commit_merge(r, &sel, safe)?` | `append(t)`, `overwrite`, `merge(t, ["id"])` | `append(t)`, `overwrite`, `merge(t, ['id'])` |
| Iceberg filtered scan | `scan_matching("px > 1", None)?`, `plan_matching(..)?` | `scan_matching("px > 1")`, `plan_matching(..)` | `scanMatching('px > 1')`, `planMatching(..)` |
| Iceberg time travel | `scan_at(snapshot_id, &[], None)?` | `scan_at(snapshot_id)` | `scanAt(snapshotId)` |
| Iceberg schema change | `SchemaUpdate::from_metadata(..)?` + `evolve_schema(field)?` | `update_schema().add_column("", f).commit()` | `updateSchema().addColumn('', f).commit()` |
| lazy engine scan | - | `scan_polars()`, `scan_arrow()` | - |
| SQL-like write/read plan | `"select ...".parse::<Plan>()?.execute()?`, `apply_arrow_reader` | `Plan("insert into ...").apply_arrow_batch(b)`, `.execute()` | `new Plan('...').applyArrowBatch(b)`, `.execute()` |

## Rules for fast, correct use

1. **Keep the reader a reader.** Iterate `read_arrow_reader`, or hand it
   straight to another handle's `overwrite_arrow_reader`; only the current
   batch is alive. `read_all()` / `intoTable()` / `collect` only when the
   caller asked for a whole table.
2. **Push `select`, `filter` and limits into the options.** Parquet skips
   unprojected column chunks and row groups whose footer statistics rule the
   filter out; IPC skips decoding unprojected columns; a folder skips leaves
   whose `column=value` path contradicts a filter equality; Iceberg skips
   manifests and files. Filtering rows after the read throws all of that away.
3. **Declare the `field` to cast once.** A narrower field is a projection; a
   wider one fills missing nullable columns with nulls; the cast runs in the
   same pass as the decode. A `not null` column refuses a value, a null or a
   missing column by name - it never stores a default. A nullable declared
   column takes a value it cannot convert as null while `safe` holds, and
   `safe` is on by default - so a bad value vanishes silently. Pass
   `safe=False` / `{ safe: false }` / `.with_safe(false)` (or declare the
   column `not null`) when an unconvertible value must be refused.
4. **`merge_by` is required for merge.** Keys use Arrow's row format: null
   matches null and the last arrival wins. Merge holds only the stored side in
   memory; `merge_by` absent is a refusal, never an overwrite.
5. **Bound memory with `commit_row_size`.** Unset publishes once at the end;
   `N` publishes every `N` rows and the committed prefix survives a later
   failure; `0` is refused before any input is pulled.
6. **`row_size`/`column_size`/`read_arrow_field` read metadata only.** They
   answer from a footer, a stream header or the manifests; `open()` caches the
   answer until `close()`.
7. **The name picks the encoding and the outer coding.** `.arrows` (IPC
   stream), `.arrow`/`.feather`/`.ipc` (IPC file), `.parquet`, `.avro`,
   `.txt`/`.log`, a table folder; `.gz`, `.zz`, `.zst` wrap the bytes.
   Parquet compresses internally, so `.parquet.gz` is refused before a byte is
   written - set `compression` instead.
8. **Parquet and Iceberg are Cargo features in Rust** (`parquet`, `iceberg`
   implies `parquet`); the Python and Node packages carry both.
9. **Absent is empty; never probe first.** An absent resource reads as no
   batches and the first write creates it and its parents. Do not guard with
   `exists`; an encoding the build lacks (e.g. `text/csv`) is named by
   `record_options()`.
10. **Properties by name are copies.** `read_arrow_reader(rowheader=...)` /
    `readArrowReader({ rowheader })` set that property on a copy of the
    handle's (or the given) options; the handle's options are unchanged. Left
    out (`...` / `undefined`) keeps the default; `None` / `null` clears.
11. **JavaScript crosses as copied IPC.** One self-contained IPC stream per
    batch; write whole tables or readers, never rows in a loop. Python crosses
    the C Data Interface without copying.
12. **One plan per stream.** A declared field or plan is compiled once per
    read or write session; build the options once and reuse them across
    calls instead of re-parsing a filter per batch.
13. **Pick the input shape that is already in hand.** A reader streams; a
    table or batch is wrapped into one; native rows (`*_records`) cross the
    value contract row by row and cost the most (4,096 rows: about 0.1 ms as
    a batch, 3 ms as records, on the docs' reference machine). Keep rows for
    small or hand-built data, batches for everything else.
14. **Plain text has a fixed shape.** A text read answers the sixteen event
    columns (`currunix` first, `state` last), then `body`, then one column per
    named `rowheader` capture - the row header is the only thing that lifts a
    column out of a line. `autotype` settles each capture's datatype from the
    regex before a byte is read. A write consumes each row's non-empty `body`.
15. **Iceberg commits are snapshots.** `append` and metadata-only commits
    rebase on a concurrent commit; `overwrite`, `merge` and `compact` report a
    conflict instead. A merge keys on the identity partition columns plus
    `merge_by`, so a row only ever updates its own partition. A scan decodes
    qualifying files side by side (`read.parallelism`) and hands batches back
    in plan order.
16. **Folders read by their layout.** A stored `column=value` layout is
    authoritative; with none on disk, the schema's partition-marked fields
    decide where rows go (`with_partition_fields`). The first batch to reach a
    leaf performs the write's operation; later ones append.

## Pitfalls

- Reading then filtering in the host (`[r for r in rows if r["id"] > 3]`,
  `table.filter(...)`) - pass `filter="id > 3"` so the medium prunes.
- `merge_*` with no `merge_by` - it raises; pass `merge_by=["id"]` /
  `{ mergeBy: ['id'] }` / `with_merge_by(["id"])?`.
- Naming a file `trades.parquet.gz` - refused ("parquet compresses"); use
  `compression="zstd(3)"` on a plain `.parquet`.
- Expecting `offset` from `RecordOptions`: there is none, and a plan's
  `offset` given through `options.plan` / `withPlan` / `with_plan` is dropped
  silently. Apply the `Plan` itself (`Plan.apply_arrow_reader`, `execute`).
- Calling `overwrite_records` / `read_arrow_reader` on a `.json`, `.jsonl`,
  `.yaml`, `.toml` or `.xml` handle - refused ("expected a record encoding
  this build implements"); use
  `write_arrow` / `read_arrow` (Rust, Python) or the codecs in
  `yggdryl-documents`.
- Appending rows to a `.json`/`.jsonl`/`.yaml`/`.toml`/`.xml` handle with
  `write_arrow(..., append)` - refused: a structured document is written
  whole, so only `overwrite` is accepted; to accumulate rows use an
  `.arrows`, `.parquet` or `.avro` handle, or read, extend and overwrite.
- Declaring `int32` over a column holding `"x"` and trusting the read: under
  the default `safe` a nullable column reads it as null. Pass `safe=False` /
  `{ safe: false }` / `.with_safe(false)` to be told.
- JavaScript plain-object rows with strings are inferred as
  `dictionary(int32,utf8)`, which Avro cannot store - pass `{ field }`.
- Python: `read_arrow_reader(options)` positionally is a `TypeError`; options
  are keyword-only (`options=`).
- Reading one leaf of a partitioned folder and expecting the partition
  columns: they live in the path; read the folder.
- Reusing an Iceberg `Table` object after writing through another handle: it
  caches metadata; open the table again.
- Collecting a Parquet read to count rows or learn the schema: `row_size`
  and `read_arrow_field` answer from the footer.
- `commit_row_size` on an Iceberg write: every commit is a snapshot, so a
  small `N` leaves many snapshots; expire them (`expire_snapshots`) or commit
  once.
- Building an Arrow JS `Int64` vector from `number`s: use `bigint` (`1n`).
  Record writes under a declared field accept either `number` or `bigint`,
  but not both in one call: Arrow JS infers the rows from the first row
  before the declared field applies, so `[{ id: 1 }, { id: 2n }]` throws a
  `TypeError` - keep one numeric kind per column.
- Looking for a CSV reader: `text/csv` is not a record encoding; read lines
  with a `rowheader` regex, or convert upstream.

## Language references

- `references/rust.md` - read for Rust: traits to import, `RecordOptions` builders, batch readers, Iceberg `Table`.
- `references/python.md` - read for Python: keyword properties, pyarrow readers, dataclass rows, lazy scans.
- `references/javascript.md` - read for Node.js: property objects, Arrow JS tables, `bigint`, copied IPC.
- `references/formats.md` - per-encoding table: media type and suffix, settings, pushdown, feature gate, limits.

## Deeper

- Records surface (signatures, pushdown, limits, append and merge, commit cadence, lazy scans): https://platob.github.io/yggdryl/holder/#records
- Partitions (pruning, partition columns, derived columns): https://platob.github.io/yggdryl/holder/#partitions
- Media overview and options: https://platob.github.io/yggdryl/media/#read-and-write
- Per format: https://platob.github.io/yggdryl/media/#arrow-ipc, https://platob.github.io/yggdryl/media/#parquet, https://platob.github.io/yggdryl/media/#avro, https://platob.github.io/yggdryl/media/#plain-text, https://platob.github.io/yggdryl/media/#iceberg
- Required columns and the cast rule: https://platob.github.io/yggdryl/types/cast/
- Plans and write verbs: https://platob.github.io/yggdryl/expression/plans/
- Sibling skills: `yggdryl-storage` (handles, backends, codings), `yggdryl-arrow` (`Serie`, `SerieReader`, casts), `yggdryl-expressions` (filter/select grammar, `Plan`), `yggdryl-uri` (hive paths, globs), `yggdryl-types` (fields, dataclasses), `yggdryl-documents` (JSON/YAML/TOML/XML).
