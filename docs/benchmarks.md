# Benchmarks

Criterion targets for the Rust core and boundary benchmarks for each binding.

=== "Rust"

    ```console
    cargo bench --bench datatype
    cargo bench --bench field
    cargo bench --bench enums
    cargo bench --bench uri
    cargo bench --bench text
    cargo bench --bench json
    cargo bench --bench toml
    cargo bench --bench yaml
    cargo bench --bench avro
    cargo bench --bench io --features "parquet"
    cargo bench --bench io --features "parquet" -- lines_gzip   # ~12 min alone
    cargo bench --bench iceberg --features "parquet iceberg"
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python benchmarks/fields.py --iterations 10000
    .venv/Scripts/python benchmarks/records.py --iterations 100000
    .venv/Scripts/python benchmarks/records_arrow.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/records_io.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/log_lines.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/log_lines_bulk.py --measure-memory
    .venv/Scripts/python benchmarks/codecs.py --iterations 10000
    .venv/Scripts/python benchmarks/compression.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/iceberg.py --min-time 0.2 --repeat 5
    ```

    A binding benchmark measures a **release** build; a debug wheel understates
    the native side by an order of magnitude. Build it with
    `maturin build --release` before timing anything.

=== "JavaScript"

    ```console
    npm run --prefix node bench:schema
    npm run --prefix node bench:codec
    npm run --prefix node bench:defaults
    npm run --prefix node bench:io
    npm run --prefix node bench:lines
    npm run --prefix node bench:records
    ```

Targets live under each member's `benchmarks/` and mirror the core domains. Run them on a quiet
machine against release artifacts, and compare like-for-like toolchains.

## What each target isolates

| Target | Isolates |
| --- | --- |
| `datatype` | Parsing, decimal and time selection, Arrow projection, defaults, compatibility rewriting |
| `field` | Construction, metadata mutation and cache invalidation, comparison, typed views |
| `enums` | MIME and media parsing, compound filename inference, content-coding recovery |
| `uri` | Parsing, component access, path segments, path interop |
| `text` | Value construction, format inference, display and elision helpers |
| `json`, `toml`, `yaml` | Whole-value and streaming encode and decode |
| `avro` | Container decode and encode by type family, codec x block-size sweep, projection skips, resolution plans, the varint floor |
| `io` | Record round-trips over handles, projection pushdown, the line-record Arrow projection and its hash, and (`lines_gzip`) a million-record rotated gzip log folder: content coding, folder shape, typed captures, and a scale sweep |
| `log_lines_bulk` (Python) | The same rotated gzip corpus at the binding, plus peak RSS per corpus size - the residency claim Criterion cannot report |
| `lines` (JavaScript) | The same corpus at the copied-IPC boundary, with median, best, and spread |
| `iceberg` | Plan, metadata, manifests (full against the planning fast path, at scale), partitioning, compaction, merge, contended commits |
| `compression` (Python) | The byte codings against the standard library's, same wire, same payload |

## Rules

**Keep fixtures outside the loop.** A schema, a parsed URL, or an encoded payload rebuilt on every
iteration measures the fixture, not the operation.

**Keep group identifiers stable.** A Criterion group ID is the only thing making two runs
comparable. When a target is split, keep the existing IDs and add new ones beside them; a renamed ID
silently restarts the history it was meant to extend.

**Say what a number means.** A result is evidence for exactly one claim. An encode timing says
nothing about parse allocations, and a cached projection says nothing about a cold one. Cases that
differ in setup belong in different groups.

**Add a benchmark in its own domain.** A field operation goes in the field target, a codec operation
in its format target. Do not create a target for one case, and do not measure two modules in one
group.

For binding benchmarks, measure only the boundary: recursive conversion and validation happen in
Rust and are already measured there.

## Against the native implementations

Two benchmarks carry a baseline from outside the project, so a claim about performance is made
against something a reader already trusts:

- `python/benchmarks/compression.py` times `yggdryl.gzip`/`zlib`/`zstd` beside the standard
  library's `gzip` and `zlib` (and `compression.zstd` on 3.14+) over the same payload - the same
  wire format either way, so the row is an engine comparison.
- `python/benchmarks/records_io.py` carries a PyArrow IPC write baseline over the same batches and
  the same sink.
- `python/benchmarks/log_lines.py` times `read_arrow_lines` beside a plain-Python `re` loop over
  the same log lines - the loop a Python engineer would write without the binding - uncompressed
  and gzip-coded. The baseline hashes with `zlib.crc32` where the binding pays for the stable
  64-bit FNV-1a, so the comparison flatters the baseline slightly.

Indicative numbers from one containerized x86_64 Linux run (Python 3.11.15, PyArrow 25.0.1,
yggdryl 0.1.1 **release** wheel; run the commands above on your own hardware before drawing
conclusions - a container's CPU budget and cache behavior shift these substantially).

`compression.py --min-time 0.2 --repeat 5`, over 1,080,000 bytes of JSON lines:

```text
gzip encode (yggdryl)      0.362 ms   2848.0 MiB/s
gzip encode (stdlib gzip)  3.003 ms    343.0 MiB/s
gzip decode (yggdryl)      0.253 ms   4065.4 MiB/s
gzip decode (stdlib gzip)  0.396 ms   2597.8 MiB/s
zlib encode (yggdryl)      0.344 ms   2998.3 MiB/s
zlib encode (stdlib zlib)  3.177 ms    324.2 MiB/s
zlib decode (yggdryl)      0.234 ms   4401.1 MiB/s
zlib decode (stdlib zlib)  0.484 ms   2127.9 MiB/s
zstd encode (yggdryl)     14.879 ms     69.2 MiB/s
zstd decode (yggdryl)      0.358 ms   2877.3 MiB/s
```

The DEFLATE engine is `zlib-rs`, which is what puts the encodes 8-9x ahead of the standard
library's; its level 6 trades a little ratio for that speed on highly repetitive payloads, so a
caller optimizing for size raises the level.

`records_io.py --min-time 0.1 --repeat 3`, 65,536 rows, 4 columns, 8 batches:

```text
ipc write reader                 1.133 ms   57.9M rows/s
PyArrow IPC write baseline       1.607 ms   40.8M rows/s
parquet write reader             6.932 ms    9.5M rows/s
PyArrow parquet write baseline   6.495 ms   10.1M rows/s
parquet read whole               2.620 ms   25.0M rows/s
PyArrow parquet read baseline    2.195 ms   29.9M rows/s
```

`log_lines.py --min-time 0.2 --repeat 5`, 100,000 log lines, 8,400,000 decoded bytes:

```text
read_arrow_lines plain          523.349 ms   191,077 rows/s
read_arrow_lines gzip           535.663 ms   186,684 rows/s
python re loop plain            338.324 ms   295,575 rows/s
python re loop gzip             364.064 ms   274,677 rows/s
```

CPython's `re` is a C engine and wins on raw regex throughput; the projection's cost sits almost
entirely in the pinned dependency-free `regex-lite` engine, not in hashing or Arrow assembly. The
Rust `io` target's `lines_arrow` group splits that claim into numbers on the same corpus and
machine: the whole parse runs at ~15.7 MiB/s (`parse/plain` 509.6 ms) with gzip adding ~1%
(`parse/gzip` 515.8 ms), the grouping stage alone (`group/plain`, `read_lines_matching` with no
Arrow projection) is 237.7 ms of that, and hashing every message (`hash/corpus`) is 3.45 ms -
0.7% of the whole read - with the FNV-1a micro-benchmarks (`lines_hash/fnv1a/*`) folding 100-byte
to 2-KiB messages at 780-1000 MiB/s. Those numbers are why the stable FNV-1a contract stays and no
hash dependency was added: the hash is noise, the regex engine is the bottleneck, and what the
projection buys for the difference is streaming decode, multi-line records, offsets, typed
timestamp columns, and a schema Iceberg accepts as declared.

The reading either way: both DEFLATE encodes and decodes outrun the stdlib's by a wide margin,
and the Parquet path sits within ~15% of PyArrow's own writer and reader on the same rows. A table like this is regenerated by running the benchmark and pasting
its output; treat any copy older than the release it names as stale.

## Avro against fastavro and PyIceberg, on identical bytes

`python scripts/bench_avro_baseline.py` has the Rust half write one deterministic
ten-thousand-entry Iceberg manifest (112,246 bytes, statistics included) and then times three
implementations over that exact file - so the rows below are readers of identical bytes, not of
similar fixtures. From one containerized x86_64 Linux run (rustc stable **release** build,
CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1; run it on your own hardware before drawing
conclusions):

```text
fastavro 1.12.2:                    67,719 entries/s best (147.7 ms best of 7)
pyiceberg 0.11.1:                   46,790 entries/s best (213.7 ms best of 7)
yggdryl full (release):            101,937 entries/s best ( 98.1 ms best of 7)
yggdryl plan_stats (release):      203,252 entries/s best ( 49.2 ms best of 7)
yggdryl plan_identity (release):   438,596 entries/s best ( 22.8 ms best of 7)
```

`full` is `read_manifest`, decoding every field the way the other two readers do.
`plan_stats` is `read_manifest_for_plan(handle, true)` - what a *filtered* scan runs - which keeps
the value counts, null counts, and bounds that pruning consults and skips the rest as bytes.
`plan_identity` is the unfiltered planning read, which keeps only file identity, partition tuple,
and sizes. The gap between the three is the measured worth of the skip routines: on this manifest
the planning path is 2.1x the full decode with statistics kept and 4.4x without, and the ratios
hold at 1,000 and 100,000 entries (`manifest/decode_full`, `manifest/decode_plan_with_stats`,
`manifest/decode_plan_identity_only` in the `iceberg` target).

From the same machine, the `avro` target's own groups
(`cargo bench --bench avro --features "parquet iceberg"`):

- **Types** (`codec/avro_types`, 10,000 rows each): primitives decode at ~2.9M rows/s and encode
  at ~3.5M rows/s; two-string rows at ~3.3M rows/s decode; 18-digit decimals at ~6.7M rows/s;
  the deeply nested family (array of records of maps) at ~620K rows/s. The single-object varint
  floor sits at ~57-65 ns per framed datum.
- **Codec x block size** (`codec/avro_blocks`, 65,536 three-column rows): decode throughput is
  nearly flat from 1,024 to 65,536 rows per block for every codec - the sweet spot is "anything
  above ~1,000 rows"; below that the per-block header and sync overhead starts to show. On this
  payload snappy decodes at ~20 MiB/s of encoded bytes, deflate at ~12, zstandard at ~9.6, and
  the null codec at ~38 (its bytes are bigger, so the row rate is what to compare).
- **Projection** (`codec/avro_projection`, 40 columns, null codec so the skip itself is visible):
  reading 3 of 40 columns takes 6.4 ms against 9.4 ms for all 40 over 8,192 rows. Avro
  interleaves columns per record, so a projection can never skip *reading* a row - the saving is
  the decode and allocation of the 37 skipped columns, jumped by their length prefixes.
- **Resolution** (`codec/avro_resolution`): compiling a five-field plan costs ~533 ns once;
  executing it per row is *cheaper* than the direct decode on this shape (4.12 ms against 4.74 ms
  for 10,000 rows) because the plan skips two writer columns the reader never wanted - the
  per-record cost of resolving is not just near zero, it can be negative.
