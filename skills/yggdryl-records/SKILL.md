---
name: yggdryl-records
description: Reads and writes rows and Arrow batches on any yggdryl handle - Arrow IPC/Feather, Parquet, Avro, CSV/TSV, Excel workbooks (.xlsx, one worksheet as records and the Workbook/Sheet/Cell random-access model), plain-text logs, Iceberg tables and hive-partitioned folders - with streamed readers, pushdown, append/merge (upsert) and commit cadence. Use when calling read_arrow_reader / readArrowReader, overwrite_arrow_* / append_* / merge_*, write_arrow_* with a mode, *_records / readRecords, read_arrow / write_arrow (SerieReader), read_arrow_field / row_size (a file's schema or row count without reading it), pandas or polars frames to and from a file (read_polars_frame, overwrite_pandas_frame, scan_polars), RecordOptions (field, safe, select, filter, merge_by, max_row_size, row_offset, commit_batch_num, plan), TextOptions rowheader, iceberg Table create/append/merge/scan, partition pruning, or picking an encoding by suffix. Covers Rust, Python and Node.js.
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
keys an upsert, `commit_batch_num` bounds a write. Put them on the call and the
medium does the work before a byte is decoded.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| options for this handle's encoding | `handle.record_options()?` | `handle.record_options()` | `handle.recordOptions()` |
| options from a name or type, no handle | `RecordOptions::for_media_type(&url.media_type())?`, `RecordOptions::for_mime_type(&MimeType::PARQUET)?` | `RecordOptions("trades.parquet")` | `RecordOptions.from('trades.parquet')`, `RecordOptions.forMimeType(MimeType.ARROW_STREAM)` |
| stored schema, no rows decoded | `read_arrow_field(&options)?` | `read_arrow_field()` | `readArrowField()` |
| row and column counts from metadata | `row_size()?`, `column_size()?` | `.row_size()`, `.column_size()` | `.rowSize()`, `.columnSize()` |
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
| Excel worksheet, header row and range on a `.xlsx` handle | `options.set_excel_sheet(Some("Trades"))?`, `set_header(false)?`, `set_excel_range(Some("A2:D".parse()?))?` | `read_arrow_reader(sheet="Trades", header=False, range="A2:D")` | `readArrowReader({ sheet: 'Trades', header: false, range: 'A2:D' })` |
| any cell of a workbook, a sheet as a column set | `Workbook::open(h)?.sheet_mut("Trades")?.set_cell("B2".parse()?, 2.5)?`, `sheet.into_serie(Some(&field), true, Default::default())?`, `Sheet::from_serie("Notes", &serie, true)?` | `Workbook.open(p)["Trades"]["B2"]`, `sheet["B2"] = 2.5`, `sheet.into_serie(field)`, `Sheet.from_serie("Notes", table)` | `Workbook.open(p).sheet('Trades').cell('B2')`, `sheet.setCell('B2', 2.5)`, `sheet.intoSerie(field)`, `Sheet.fromSerie('Notes', table)` |
| log lines as typed rows | `handle.into_text_with(TextOptions)` | `IOBase(p).into_text(TextOptions())`, or `read_arrow_reader(rowheader=...)` | `new IOBase(p).intoText(opts)`, or `readArrowReader({ rowheader })` |
| a CSV dialect for one call | `options.set_csv_separator(b';')?`, `set_csv_quote(None)?`, `set_header(false)?`, `set_csv_null_values(["NA"])?`, `set_csv_trim(true)?`, `set_csv_comment(Some(b'#'))?`, `set_csv_infer_row_size(64)?` | `read_records(separator=";")`, `quote=None`, `header=False`, `null_values=["NA"]`, `trim=True`, `comment="#"`, `infer_row_size=64` | `readRecords({ separator: ';' })`, `{ quote: null, header: false, nullValues: ['NA'], trim: true, comment: '#', inferRowSize: 64 }` |
| CSV options in hand | `CsvOptions::new().with_separator(b';')?`, `RecordOptions::for_mime_type(&MimeType::CSV)?` | `RecordOptions("trades.csv")`, `options.separator = ";"` | `RecordOptions.from('trades.csv').withSeparator(';')` |
| a TSV | `CsvOptions::tsv()`, or a `.tsv` name | `IOBase("trades.tsv")`, `RecordOptions("trades.tsv")` | `new IOBase('trades.tsv')`, `RecordOptions.from('trades.tsv')` |
| a compressed CSV | `Holder::local("trades.csv.gz")?.into_declared_media()` | `IOBase("trades.csv.gz")` | `new IOBase('trades.csv.gz')` |
| write a partitioned folder | `Holder::folder(&root)?.overwrite_arrow_reader(r, &options)?` | `IOBase(dir).overwrite_arrow_batch(b, options=o)` | `new IOBase(dir).overwriteArrowTable(t, options)` |
| leaves of one partition | `children_where(&[("year", "2024")], false)?` | `children_where({"year": "2024"})` | `childrenWhere({ year: '2024' })` |
| derived partition column | `root.with_partition_by(["year(event) as year".parse()?])?`, `root.as_transform().apply_arrow_batch(&b)?` | `root.with_partition_by(["year(event) as year"])`, `root.transform.apply_arrow_batch(b)` | `root.withPartitionBy(['year(event) as year'])`, `Selector.fromField(root).applyArrowBatch(b)` |
| Iceberg table | `Table::create(LocalFolder::new(p)?, FormatVersion::V2, schema, PartitionSpec::from_schema(1, &schema)?)?` | `Table.create(IOBase(p), schema, ["venue", "minutes(ts, 15)"])` | `iceberg.Table.create(p, schema, ['venue', 'minutes(ts, 15)'])` |
| Iceberg write | `commit_append(r)?`, `commit_overwrite`, `commit_merge(r, &sel, safe)?` | `append(t)`, `overwrite`, `merge(t, ["id"])` | `append(t)`, `overwrite`, `merge(t, ['id'])` |
| Iceberg filtered scan | `scan_matching("px > 1", None)?`, `plan_matching(..)?` | `scan_matching("px > 1")`, `plan_matching(..)` | `scanMatching('px > 1')`, `planMatching(..)` |
| Iceberg time travel | `scan_at(snapshot_id, &[], None)?` | `scan_at(snapshot_id)` | `scanAt(snapshotId)` |
| Iceberg schema change | `SchemaUpdate::from_metadata(..)?` + `update_schema(&update)?` | `update_schema().add_column("", f).commit()` | `updateSchema().addColumn('', f).commit()` |
| lazy engine scan | - | `scan_polars()`, `scan_arrow()` | - |
| pandas / polars frames to and from a file | - | `read_pandas_frame()`, `read_polars_frame()` (whole), `read_pandas()` / `read_polars()` (lazy iterator of frames), `overwrite_pandas_frame(df)`, `append_polars_frame(df)`, `merge_pandas_frame(df, merge_by=[...])`, `write_polars(frames, mode)` | - |
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
5. **Bound memory with `commit_batch_num`.** It counts whole batches, never
   cutting one: `N` publishes every `N` batches and the committed prefix
   survives a later failure; `0` is refused before any input is pulled. Unset
   is the destination's own cadence - a file or folder publishes once at the
   end, an Iceberg table each time its held batches reach the target file
   size. Native rows are cut into batches by `batch_row_size`.
6. **`row_size`/`column_size`/`read_arrow_field` read metadata only.** They
   answer from a footer, a stream header or the manifests; `open()` caches the
   answer until `close()`. In Python and JavaScript they - and `size()`,
   `kind()` - are methods, never properties: call them.
7. **The name picks the encoding and the outer coding.** `.arrows` (IPC
   stream), `.arrow`/`.feather`/`.ipc` (IPC file), `.parquet`, `.avro`,
   `.csv`, `.tsv`, `.xlsx` (one worksheet of a workbook), `.txt`/`.log`, a table folder;
   `.gz`, `.zz`, `.zst` wrap the bytes. Parquet and a workbook compress
   internally, so `.parquet.gz` and `.xlsx.gz` are refused before a byte is
   written - set `compression` on Parquet instead. A glob's suffix names the
   encoding of the leaves it matches and no outer coding: `lake/**/*.parquet`
   reads as Parquet over only its `.parquet` leaves (not `_SUCCESS`, not a
   note beside them), `logs/*.log.gz` as plain text, each leaf decoded on its
   own; a folder or a path ending in `/` finds the encoding beneath it. Dot
   names - `.venv`, `.config` - and their trees are skipped by default.
8. **Parquet and Iceberg are Cargo features in Rust** (`parquet`, `iceberg`
   implies `parquet`); the Python and Node packages carry both.
9. **Absent is empty; never probe first.** An absent resource reads as no
   batches and the first write creates it and its parents. Do not guard with
   `exists`; an encoding the build lacks (e.g. `application/vnd.apache.orc`)
   is named by `record_options()`.
10. **Properties by name are copies.** `read_arrow_reader(rowheader=...)` /
    `readArrowReader({ rowheader })` set that property on a copy of the
    handle's (or the given) options; the handle's options are unchanged. Left
    out (`...` / `undefined`) keeps the default; `None` / `null` clears. A
    name no setter owns (a typo, a read-only `mime_type`, another encoding's
    setting) is skipped with an `UnknownPropertyWarning` naming the closest
    property - escalate it with `warnings.simplefilter("error",
    UnknownPropertyWarning)` in tests; in Node it is a process warning with
    `code: 'YGGDRYL_UNKNOWN_PROPERTY'`.
11. **JavaScript crosses as copied IPC.** One self-contained IPC stream per
    batch; write whole tables or readers, never rows in a loop. Python crosses
    the C Data Interface without copying.
12. **One plan per stream.** A declared field or plan is compiled once per
    read or write session; build the options once and reuse them across
    calls instead of re-parsing a filter per batch.
13. **Pick the input shape that is already in hand.** A reader streams; a
    table or batch is wrapped into one; native rows (`*_records`) cross the
    value contract row by row - a row that omits a required column is refused,
   not defaulted - and cost the most (4,096 rows: about 0.1 ms as
    a batch, 3 ms as records, on the docs' reference machine). Keep rows for
    small or hand-built data, batches for everything else.
14. **Plain text has a fixed shape.** A text read answers the fifteen event
    columns (`currunix` first, `state` last), then `body`, then one column per
    named `rowheader` capture - the row header is the only thing that lifts a
    column out of a line. `autotype` settles each capture's datatype from the
    regex before a byte is read. An object's lines are one chain: a line whose
    own `creaunix` capture states none takes the earliest `currunix` the read
    has dated a line of its object by so far; a `creaunix` capture stands. Each
    line also states, as `prevunix`, the `currunix` the read dated the line
    before it by - none for an object's first line or after an undated one, a
    `prevunix` capture standing, each object of a folder or glob starting
    again - and never a `prevuuid`. A line's `currhashcode` is the XXH3-64
    of its `body` alone, so byte-identical bodies share it; the instant, the
    row number and the cross code tell them apart through `curruuid`. A
    folder, a
    path ending in `/` or a glob reads leaf by leaf through
    `read_text_lines`, `row_size` and the record reads alike: every text leaf,
    each its own cross code, time, coding and row numbers, a last line ending
    with its leaf. A write consumes each row's non-empty `body`.
15. **CSV is typed by its header and a sample, or by the declared `field`.**
    The first record names the columns (`header=False`: `column_1`, ...),
    a sample of `infer_row_size` records (1,024) types each column - boolean,
    `int64`, `float64`, `date32`, `datetime64(ns, UTC)`, else `utf8` - and
    every inferred column is nullable; a later cell its column cannot read
    is refused, naming `infer_row_size`; declare the `field` to read every
    cell under a contract. A write onto a stored document completes onto its
    header, never the sample: the rows are written as they are, a header
    column they lack is empty, and a column the header lacks is refused. An
    unquoted cell spelling one of `null_values` (the empty cell, by default)
    is null and `""` the empty text, a record with the
    wrong number of cells is refused by row, a `.tsv` name is the same medium
    under a tab, and the dialect is a set of option properties -
    `separator`, `quote`, `escape`, `comment`, `header`, `null_values`,
    `trim`, `infer_row_size` - never a format argument. Compression and the
    charset are the handle's (`trades.csv.gz`, `;charset=windows-1252`).
16. **Iceberg commits are snapshots.** `append` and metadata-only commits
    rebase on a concurrent commit; `overwrite`, `merge` and `compact` report a
    conflict instead. A merge keys on the identity partition columns plus
    `merge_by`, so a row only ever updates its own partition. A table is
    created from the partitioning its schema declares (`PARTITION:by`: `venue`,
    `days(ts)`, `minutes(ts, 15)`, `truncate(name, 4) as prefix`) unless
    `partition_by` / `partitionBy` states entries, or `[]` / `null` states
    none, and from its `SORT:by` as the default sort order; `minutes[n]`,
    `week` and `quarter` are this crate's own transforms, which other Iceberg
    readers do not prune by. A scan decodes qualifying files side by side
    (`read.parallelism`) and hands batches back in plan order.
17. **Folders read by their layout.** A stored `column=value` layout is
    authoritative; with none on disk, the schema's partition-marked fields
    decide where rows go (`with_partition_fields`, or `with_partition_by` for
    derived entries such as `years(ts)`). The first batch to reach a
    leaf performs the write's operation; later ones append.

## Pitfalls

- Reading then filtering in the host (`[r for r in rows if r["id"] > 3]`,
  `table.filter(...)`) - pass `filter="id > 3"` so the medium prunes.
- `merge_*` with no `merge_by` - it raises; pass `merge_by=["id"]` /
  `{ mergeBy: ['id'] }` / `with_merge_by(["id"])?`.
- Naming a file `trades.parquet.gz` - refused ("parquet compresses"); use
  `compression="zstd(3)"` on a plain `.parquet`.
- Skipping rows in the host after the read: `row_offset` (`rowOffset`,
  `with_row_offset`) skips leading rows before `max_row_size` counts, and a
  plan's `offset` lands there too (`options.plan`, `withPlan`, `with_plan`,
  `execute`). A batch the skip covers whole is never handed on; a merge with
  a `row_offset` is refused.
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
- Reading a `.xlsx` inferred and expecting integers: the file stores every
  number as a double, so an inferred number column is `float64` and a styled
  serial a `date32`/`time32(ms)`/`datetime64(ms)`/`duration64(ms)`. Declare
  the `field` to read `1` back as the `int64` it was written as.
- JavaScript plain-object rows with strings are inferred as
  `dictionary(int32,utf8)`; every encoding stores them (Avro as the plain
  values), but pass `{ field }` to state the column rather than inherit Arrow
  JS's guess.
- Python: `read_arrow_reader(options)` positionally is a `TypeError`; options
  are keyword-only (`options=`).
- Reading one leaf of a partitioned folder and expecting the partition
  columns: they live in the path; read the folder.
- Reusing an Iceberg `Table` object after writing through another handle: it
  caches metadata; open the table again.
- Collecting a Parquet read to count rows or learn the schema: `row_size`
  and `read_arrow_field` answer from the footer.
- `commit_batch_num` on an Iceberg write: every commit is a snapshot, so a
  small `N` leaves many snapshots; expire them (`expire_snapshots`) or leave
  it unset to commit per target file size.
- Building an Arrow JS `Int64` vector from `number`s: use `bigint` (`1n`).
  Native row writes unify `number` and `bigint` rows of one column instead:
  `[{ id: 1 }, { id: 2n }]` is one `int64` column, the integral `number` read
  as `bigint`; only a fraction beside `bigint` rows is refused, naming the
  column and the value.
- Reading a `;` or `|` document under the default CSV dialect: the header is
  one column. Pass `separator=';'` / `{ separator: ';' }` /
  `set_csv_separator(b';')?` on the read - and on the write, or the file
  reads back one-columned under the default.
- Trusting an inferred CSV column: the sample's datatypes are a reading,
  so a later cell that does not fit is refused - `$[row].<column>: expected
  int64, the datatype the first 1024 records (infer_row_size) infer, got
  "x" ...` - partway through the read. Declare the `field` (or widen
  `infer_row_size`) for a contract; a declared nullable column takes such a
  cell as null under `safe`.

## Language references

- `references/rust.md` - read for Rust: traits to import, `RecordOptions` builders, batch readers, Iceberg `Table`.
- `references/python.md` - read for Python: keyword properties, pyarrow readers, dataclass rows, lazy scans.
- `references/javascript.md` - read for Node.js: property objects, Arrow JS tables, `bigint`, copied IPC.
- `references/formats.md` - per-encoding table: media type and suffix, settings, pushdown, feature gate, limits.

## Deeper

- Records surface (signatures, pushdown, limits, append and merge, commit cadence, lazy scans): https://platob.github.io/yggdryl/holder/#records
- Partitions (pruning, partition columns, derived columns): https://platob.github.io/yggdryl/holder/#partitions
- Media overview and options: https://platob.github.io/yggdryl/media/#read-and-write
- Per format: https://platob.github.io/yggdryl/media/#arrow-ipc, https://platob.github.io/yggdryl/media/#parquet, https://platob.github.io/yggdryl/media/#avro, https://platob.github.io/yggdryl/media/#excel, https://platob.github.io/yggdryl/media/#csv, https://platob.github.io/yggdryl/media/#plain-text, https://platob.github.io/yggdryl/media/#iceberg
- Required columns and the cast rule: https://platob.github.io/yggdryl/types/cast/
- Plans and write verbs: https://platob.github.io/yggdryl/expression/plans/
- Sibling skills: `yggdryl-storage` (handles, backends, codings), `yggdryl-arrow` (`Serie`, `SerieReader`, casts), `yggdryl-expressions` (filter/select grammar, `Plan`), `yggdryl-uri` (hive paths, globs), `yggdryl-types` (fields, dataclasses), `yggdryl-documents` (JSON/YAML/TOML/XML).
