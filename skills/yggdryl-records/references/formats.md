# Record encodings at a glance

The handle's media type picks the row; nothing else does. Outer codings
(`.gz`, `.zz`, `.zst`) wrap any encoding below except Parquet. Every encoding
answers the same `IOMedia` calls; this table is what differs.

| Encoding | Declared by | Build (Rust) | Reads | Writes |
| --- | --- | --- | --- | --- |
| Arrow IPC stream | `application/vnd.apache.arrow.stream`, `.arrows` | default | batches as stored; the stream carries its schema | every Arrow layout, union and dictionary included |
| Arrow IPC file | `application/vnd.apache.arrow.file`, `.arrow`, `.feather`, `.ipc` | default | as the stream | as the stream |
| Parquet | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature | 65,536-row batches (or `batch_row_size`), row groups and columns decoded on every thread, batches in file order | row groups encoded in parallel, byte-identical to a one-thread write; a union column is refused by name |
| Avro container | `application/avro`, `.avro` | default (`snappy` blocks need `parquet`) | blocks decoded in parallel, batches in file order; the container carries its writer schema | blocks of about 1 MB, encoded and compressed in parallel |
| Plain text | `text/plain`, `.txt`, `.log` | default | one row per line (or per framed chain): 16 event columns, `body`, then one column per `rowheader` capture | writes each row's non-null, non-empty `body` as one line |
| Iceberg table | a folder with `metadata/` and `data/` | `iceberg` feature (implies `parquet`) | a scan planned from the snapshot's manifests, files decoded side by side | `append`/`overwrite`/`merge` commits of Parquet data files |
| Partitioned folder | a folder of `column=value/` leaves | the leaves' encodings | every leaf, partition columns restored from the path | rows routed to their leaf; the leaf stores only non-partition columns |
| JSON, JSON Lines, YAML, TOML, XML | `.json`, `.jsonl`, `.yaml`, `.toml`, `.xml` | default | only through `read_arrow` (one record column) | only through `write_arrow` (one document per row, or one document) |

`text/csv` and any other type answer `record_options()` with a refusal naming
the encodings the build implements.

## Settings each encoding owns

A setting of another encoding reads as `None`/`null`; setting it is an error.

| Encoding | Setting | Default | Spelling |
| --- | --- | --- | --- |
| Parquet | `compression` | `zstd(1)` | `uncompressed`, `snappy`, `gzip(n)`, `brotli(n)`, `lz4`, `lz4_raw`, `zstd(n)` |
| Parquet | `max_row_group_size` | 1,048,576 rows | rows per row group |
| Parquet | `key_value_metadata` | none | footer key/value pairs |
| Avro | `block_codec` | `deflate` | `null`, `deflate`, `snappy`, `zstandard` |
| Avro | `sync_marker` | random per write | 16 bytes, for byte-reproducible files |
| Plain text | `TextOptions`: `rowheader`, `autotype` (on), `framing`, `lstrip`/`rstrip`, `linesep`, `start_rownum`, `parse_mtime` (on), `leading_fragment`, `max_record_byte_size`, `rename_columns`, `timezone` | 35,840 rows or 64 MiB per batch | named regex captures become columns |
| every encoding | `level` | 6 | outer `.gz`/`.zz`/`.zst` level |
| Iceberg | `IcebergOptions`: `read_parallelism`, `write_parallelism`, `read_parallel_min_files`, `read_parallel_min_file_size`, `target_file_size`, `commit_retries`, `compact_after_commits`, `data_mime_type` | explicit -> table property (`read.parallelism`, ...) -> default | per call (`options=`) or `set_options` per table |

## Pushdown

| Encoding | `select` / narrower `field` | `filter` | `max_row_size` / `max_byte_size` |
| --- | --- | --- | --- |
| Arrow IPC | skipped columns are never decoded | rows filtered after decode | stops pulling; the boundary batch is sliced |
| Parquet | unprojected column chunks are never fetched (footer-first read above 1 MB; chunks under 1 MB apart share a request) | row groups whose footer statistics rule it out are skipped, then rows filtered; float min/max never prune (NaN), null counts do | decodes lazily, one file at a time, on one thread |
| Avro | skipped fields jumped by their length prefixes | rows filtered after decode | stays on one thread |
| Plain text | applied after the lines become rows | applied after the lines become rows; a `where` may name a capture | stops pulling lines |
| Partitioned folder | per leaf | each filter equality (`partition_pairs()`) prunes leaves by path before listing below them; ranges and `in` lists prune nothing by path | across leaves |
| Iceberg | projection per data file | manifest-list summaries, then partition tuples and column bounds, then rows; the whole expression language prunes | across files |

## Limits and edges

- `max_row_size` counts result rows, `max_byte_size` their uncompressed Arrow bytes; both apply last and stop pulling. `0` is a valid read (schema, no batch); a non-zero byte bound yields at least one row. On a write, a limit truncates the input and never pulls past it.
- `commit_row_size`: unset commits once; `N` publishes every `N` rows then the remainder; `0` is refused. A plain folder publishes each leaf on its own; Iceberg commits a snapshot.
- Merge: keys by Arrow row format (null matches null, last arrival wins); holds only the stored side in memory. Iceberg merge keys are the identity partition columns plus `merge_by`; a table with neither is refused; merge on format v3 is refused.
- Parquet reads copy the bytes they keep into reader-owned memory, so rewriting the file while a reader lives is safe.
- Iceberg: promotions are `int32 -> int64`, `float32 -> float64`, same-scale decimal widening; field IDs are preserved and never reused; append and metadata-only commits rebase on conflict, overwrite/merge/compact restore state and report the conflict.
- Plain text: a blank physical line separates records and never is one; bytes are decoded once in the handle's declared charset, otherwise as UTF-8 with Windows-1252 fallback per invalid byte.

Pages: https://platob.github.io/yggdryl/media/ (per format) and https://platob.github.io/yggdryl/holder/#records (the shared surface).
