# Record encodings at a glance

The handle's media type picks the row; nothing else does. Outer codings
(`.gz`, `.zz`, `.zst`) wrap any encoding below except Parquet. Every encoding
answers the same `IOMedia` calls; this table is what differs.

| Encoding | Declared by | Build (Rust) | Reads | Writes |
| --- | --- | --- | --- | --- |
| Arrow IPC stream | `application/vnd.apache.arrow.stream`, `.arrows` | default | batches as stored, re-cut to `batch_row_size` / `batch_byte_size` when set; the stream carries its schema | every Arrow layout, union and dictionary included |
| Arrow IPC file | `application/vnd.apache.arrow.file`, `.arrow`, `.feather`, `.ipc` | default | as the stream | as the stream |
| Parquet | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature | 65,536-row batches (or `batch_row_size`), row groups and columns decoded on every thread, batches in file order | row groups encoded in parallel, byte-identical to a one-thread write; a union column is refused by name |
| Avro container | `application/avro`, `.avro` | default (`snappy` blocks need `parquet`) | blocks decoded in parallel, batches in file order; the container carries its writer schema | blocks of about 1 MB, encoded and compressed in parallel |
| Excel workbook | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | default | one worksheet: the first row of the range names the columns, every cell below is a value - a number a `float64`, a styled serial a `date32`, `time32(ms)`, `datetime64(ms)` or `duration64(ms)` in the workbook's date system - the part streamed row by row; a sheet the workbook lacks reads as the empty stream | one worksheet rendered as the batches arrive, a header row of the column names first; every other part of an opened package is kept, and a `.xlsx.gz` name is refused |
| Plain text | `text/plain`, `.txt`, `.log` | default | one row per line (or per framed chain): 15 event columns, `body`, then one column per `rowheader` capture | writes each row's non-null, non-empty `body` as one line |
| CSV, TSV | `text/csv`, `.csv`; `text/tab-separated-values`, `.tsv` | default | streamed RFC 4180 records: the header names the columns, a declared `field` types every cell or a sample of `infer_row_size` records infers the columns (every one nullable, a later cell that does not fit refused); a ragged record or an unterminated quote is refused by row | the header once, then one record per row, every leaf as the text it reads back from and quoted only where it must be; a write onto a stored document completes onto its header, never its sample, and append writes after the stored tail |
| Iceberg table | a folder with `metadata/` and `data/` | `iceberg` feature (implies `parquet`) | a scan planned from the snapshot's manifests, files decoded side by side | `append`/`overwrite`/`merge` commits of Parquet data files |
| Partitioned folder | a folder of `column=value/` leaves | the leaves' encodings | every leaf, partition columns restored from the path | rows routed to their leaf; the leaf stores only non-partition columns |
| JSON, JSON Lines, YAML, TOML, XML | `.json`, `.jsonl`, `.yaml`, `.toml`, `.xml` | default | only through `read_serie` (one record column) | only through `overwrite_serie` / `write_serie(.., overwrite)` (one document per row, or one document; written whole, so append is refused; of the options only the declared `field`) |

`application/vnd.apache.orc` and any other type answer `record_options()` with
a refusal naming the encodings the build implements.

## Settings each encoding owns

A setting of another encoding reads as `None`/`null`; setting it is an error.

| Encoding | Setting | Default | Spelling |
| --- | --- | --- | --- |
| Parquet | `compression` | `zstd(1)` | `uncompressed`, `snappy`, `gzip(n)`, `brotli(n)`, `lz4`, `lz4_raw`, `zstd(n)` |
| Parquet | `max_row_group_size` | 1,048,576 rows | rows per row group |
| Parquet | `key_value_metadata` | none | footer key/value pairs |
| Avro | `block_codec` | `deflate` | `null`, `deflate`, `snappy`, `zstandard` |
| Avro | `sync_marker` | random per write | 16 bytes, for byte-reproducible files |
| Excel | `sheet` | the first worksheet | a sheet name; missing on read is the empty stream, on write the sheet is added beside the others |
| Excel | `header` | on | whether the range's first row names the columns; off, the columns are named by their letters |
| Excel | `range` | the sheet's used range | `A1:C10`, `A:C`, `3:5`, `A3:F` |
| Plain text | `TextOptions`: `rowheader`, `autotype` (on), `framing`, `lstrip`/`rstrip`, `linesep`, `start_rownum`, `parse_mtime` (on), `leading_fragment`, `max_record_byte_size`, `rename_columns`, `timezone` | 35,840 rows or 64 MiB per batch | named regex captures become columns |
| CSV, TSV | `separator`, `quote`, `escape`, `comment`, `header`, `null_values`, `trim`, `infer_row_size`; Rust also `linesep` | `,` (`\t` under a `.tsv` name), `"`, none, none, on, `[""]`, off, 1,024, `\n` | one ASCII byte per role, never a line break, no two roles one byte; Python and JavaScript spell a byte role as a one-character text and clear `quote`/`escape`/`comment` with `None`/`null` |
| every encoding | `level` | 6 | outer `.gz`/`.zz`/`.zst` level |
| Iceberg | `IcebergOptions`: `read_parallelism`, `write_parallelism`, `max_open_partitions` (`write.max-open-partitions`, 128: past it a write closes and writes the lowest partition; a source sorted on the partition columns closes each as the next arrives), `read_parallel_min_files`, `read_parallel_min_file_size`, `target_file_size`, `commit_retries`, `data_mime_type` | explicit -> table property (`read.parallelism`, ...) -> default | per call (`options=`) or `set_options` per table |

## Pushdown

| Encoding | `select` / narrower `field` | `filter` | `row_offset`, `max_row_size` / `max_byte_size` |
| --- | --- | --- | --- |
| Arrow IPC | skipped columns are never decoded | rows filtered after decode | stops pulling; the boundary batch is sliced |
| Parquet | unprojected column chunks are never fetched (footer-first read above 1 MB; chunks under 1 MB apart share a request) | row groups whose footer statistics rule it out are skipped, then rows filtered; float min/max never prune (NaN), null counts do | decodes lazily, one file at a time, on one thread |
| Avro | skipped fields jumped by their length prefixes | rows filtered after decode | stays on one thread |
| Excel | applied after the cells become rows | applied after the cells become rows | stops pulling rows; the row count is a pass over the part with no value read |
| Plain text | applied after the lines become rows | applied after the lines become rows; a `where` may name a capture | stops pulling lines |
| CSV | a declared `field` types only the columns it names; `select` applied after the records become rows | applied after the records become rows | stops pulling records |
| Partitioned folder | per leaf | each filter equality (`partition_pairs()`) prunes leaves by path before listing below them; ranges and `in` lists prune nothing by path | across leaves |
| Iceberg | projection per data file | manifest-list summaries, then partition tuples and column bounds, then rows; the whole expression language prunes | across files |

## Limits and edges

- `row_offset` skips leading result rows first; `max_row_size` then counts result rows, `max_byte_size` their uncompressed Arrow bytes; all three apply last and stop pulling. `0` is a valid read (schema, no batch); a non-zero byte bound yields at least one row. On a write, a limit truncates the input and never pulls past it.
- `commit_batch_num`: counts whole batches and never cuts one; `N` publishes every `N` batches then the remainder; `0` is refused. Unset is the destination's cadence: a leaf, a plain folder and an Iceberg table commit once, the table holding every partition's rows under the process spill bound until the source ends (`write.target-file-size-bytes` cuts files, never commits). A plain folder publishes each leaf on its own; Iceberg commits a snapshot. What a cadence holds between publications is held under the process spill bound, heaviest batches spilled first.
- `num_threads`: how many parts a write of several parts runs at once - an Iceberg commit's partition groups, each sorted whole by the table's sort order unless already in it - over the table's `write.parallelism`, else `read.parallelism`, else the host; `0` is refused naming `$.num_threads`.
- Merge: keys by Arrow row format (null matches null, last arrival wins); holds only the stored side in memory; replaces a stored row only where the last arrival for its key differs from it, every column compared - a layout the row format does not encode, a float zero of the other sign or a NaN of other bits counting as changed - so a merge changing nothing leaves a leaf's bytes and modification time and commits no Iceberg snapshot, while its `IOResult` counts every row pulled as written. `merge_by=True` (Python; `with_merge_by_scalar(&Scalar::from(true))` in Rust) is the destination's own key, the empty key `None` states; `False` is refused at `$.merge_by`. Iceberg merge keys are the identity partition columns plus `merge_by`, else the columns the schema's `identifier-field-ids` names, else the partition alone; an unpartitioned table stating no identifier is refused; a keyed merge on format v3 is refused.
- Iceberg append to a table stating `identifier-field-ids`: only the rows whose key - the identity partition columns, then the identifier columns - neither the table nor an earlier row of the write holds are written, the first arrival kept and the rest counted in `IOResult.skipped_rows`; no stored file is rewritten, an append keeping nothing commits nothing, on format v2 and v3 alike; a table stating no identifier appends every row. Each partition group holds its key bytes in memory - the stored keys of the files whose statistics may hold an incoming key, and the incoming keys kept - outside the spill bound, as a merge's key index is.
- Parquet reads copy the bytes they keep into reader-owned memory, so rewriting the file while a reader lives is safe.
- Iceberg: promotions are `int32 -> int64`, `float32 -> float64`, same-scale decimal widening, and v3's `unknown` to any type; field IDs are preserved and never reused; the version document is created exclusively (`create_bytes`), so racing writers never overwrite one another on local storage or an object store, an append to a table stating no identifier and metadata-only commits rebase on conflict, overwrite/merge/compact and a keyed append restore state and report the conflict.
- CSV: `1,` is null and `1,""` the empty text; a quoted cell may hold the separator, a line break and a doubled quote; `\n` and `\r\n` both end a record and a `\r` alone ends nothing; a blank record and a comment record are skipped; an empty document declares no schema (`read_arrow_field` refused at `$.csv`) and reads as no rows; a read under another dialect reads another shape, so a `;` document under the default dialect is one column.
- Excel: the grid is 1,048,576 rows by 16,384 columns and a cell holds at most 32,767 characters; text is escaped as ECMA-376 spells it (`_xHHHH_`), which Excel reads back and openpyxl leaves as written; an inferred required column is one every row states.
- Plain text: a blank physical line separates records and never is one; bytes are decoded once in the handle's declared charset, otherwise as UTF-8 with Windows-1252 fallback per invalid byte.

Pages: https://platob.github.io/yggdryl/media/ (the overview, one page per format beneath it) and https://platob.github.io/yggdryl/holder/#records (the shared surface).
