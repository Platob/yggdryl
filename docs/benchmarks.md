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
    cargo bench --bench io --features "parquet"
    cargo bench --bench iceberg --features "parquet iceberg"
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python benchmarks/fields.py --iterations 10000
    .venv/Scripts/python benchmarks/records.py --iterations 100000
    .venv/Scripts/python benchmarks/records_arrow.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/records_io.py --min-time 0.2 --repeat 7
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
| `expression` | Parse, bind, and display once; mask, filter, row-scan, and prune per batch - each against the raw `arrow-ord` / `arrow-select` call |
| `text` | Value construction, format inference, display and elision helpers |
| `json`, `toml`, `yaml` | Whole-value and streaming encode and decode |
| `io` | Record round-trips over handles, projection pushdown |
| `iceberg` | Plan, metadata, manifests, partitioning, compaction, merge, contended commits |
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
- `benchmarks/expression.rs` carries the raw `arrow-ord` / `arrow-select` call as its baseline: the
  same predicate written by hand against the kernels, with no expression involved. The
  `expression_mask` group and the `kernel_mask` group share their case IDs, so the two are read side
  by side and the gap between them is the price of the grammar.

Indicative numbers from one containerized x86_64 Linux run, 65,536 rows, `--measurement-time 1`:

```text
                       expression   kernel
utf8_equality             181.5 us  177.2 us
int64_range                41.1 us   36.9 us
decimal_range              48.6 us   51.0 us
set_membership            347.3 us  349.1 us
conjunction               277.6 us  271.4 us
```

Parsing and binding are the other half, and they happen once per stream rather than once per batch:
`expression_parse` runs 0.5-2.4 us per predicate and `expression_bind` 0.5-4.5 us. A batch of 65,536
rows therefore pays the grammar back in the first few microseconds of the first batch.

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

The reading either way: both DEFLATE encodes and decodes outrun the stdlib's by a wide margin,
and the Parquet path sits within ~15% of PyArrow's own writer and reader on the same rows. A table like this is regenerated by running the benchmark and pasting
its output; treat any copy older than the release it names as stale.
