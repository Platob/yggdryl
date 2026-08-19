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
    cargo bench --bench io --features "parquet" -- io_buffered
    cargo bench --bench text -- lines_gzip                    # ~12 min alone
    cargo bench --bench arrowfs --features "parquet"
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
    .venv/Scripts/python benchmarks/arrowfs.py --min-time 0.2 --repeat 7
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
    npm run --prefix node bench:arrowfs
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
| `text` | Value construction, format inference, display and elision helpers, the `{{ }}` placeholder guard against the substitution it guards, the text-record Arrow projection and its hash, record shape (batch bounds, timestamp detection against the equivalent pattern, zones, trimming), and (`lines_gzip`) a million-record rotated gzip log folder: content coding, folder shape, typed captures, and a scale sweep |
| `json`, `toml`, `yaml` | Whole-value and streaming encode and decode |
| `avro` | Container decode and encode by type family, codec x block-size sweep, projection skips, resolution plans, the varint floor |
| `io` | Record round-trips over handles, projection pushdown, and (`io_buffered`) the page cache against the handles it wraps |
| `log_lines_bulk` (Python) | The same rotated gzip corpus at the binding, plus peak RSS per corpus size - the residency claim Criterion cannot report |
| `lines` (JavaScript) | The same corpus at the copied-IPC boundary, with median, best, and spread |
| `arrowfs` | What wrapping a foreign Arrow filesystem costs: bytes, ranged reads, record round trips, and listing, each beside the native handle holding the same bytes |
| `arrowfs` (Python) | The same boundary against PyArrow's own calls on the same filesystem - the implementation the wrapper delegates to |
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
- `python/benchmarks/arrowfs.py` times a handle over a `pyarrow.fs.LocalFileSystem` beside
  PyArrow's own calls against that same filesystem. The baseline is the implementation the wrapper
  delegates to, so the difference is the seven-method boundary and nothing else.
- `benchmarks/expression.rs` carries the raw `arrow-ord` / `arrow-select` call as its baseline: the
  same predicate written by hand against the kernels, with no expression involved. The
  `expression_mask` group and the `kernel_mask` group share their case IDs, so the two are read side
  by side and the gap between them is the price of the grammar.

Indicative expression numbers from one containerized x86_64 Linux run, 65,536 rows,
`--measurement-time 1`:

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

`log_lines.py --min-time 0.2 --repeat 5`, 100,000 log lines, 8,400,000 decoded bytes:

```text
read_arrow_lines plain          523.349 ms   191,077 rows/s
read_arrow_lines gzip           535.663 ms   186,684 rows/s
python re loop plain            338.324 ms   295,575 rows/s
python re loop gzip             364.064 ms   274,677 rows/s
```

CPython's `re` is a C engine and wins on raw regex throughput; the projection's cost sits almost
entirely in the pinned dependency-free `regex-lite` engine, not in hashing or Arrow assembly. The
Rust `text` target's `lines_arrow` group splits that claim into numbers on the same corpus and
machine: the whole parse runs at ~15.7 MiB/s (`parse/plain` 509.6 ms) with gzip adding ~1%
(`parse/gzip` 515.8 ms), the grouping stage alone (`group/plain`, `read_lines` with a pattern and no
Arrow projection) is 237.7 ms of that, and hashing every message (`hash/corpus`) is 3.45 ms -
0.7% of the whole read - with the FNV-1a micro-benchmarks (`lines_hash/fnv1a/*`) folding 100-byte
to 2-KiB messages at 780-1000 MiB/s. Those numbers are why the stable FNV-1a contract stays and no
hash dependency was added: the hash is noise, the regex engine is the bottleneck, and what the
projection buys for the difference is streaming decode, multi-line records, offsets, typed
timestamp columns, and a schema Iceberg accepts as declared.

The reading either way: both DEFLATE encodes and decodes outrun the stdlib's by a wide margin,
and the Parquet path sits within ~15% of PyArrow's own writer and reader on the same rows. A table like this is regenerated by running the benchmark and pasting
its output; treat any copy older than the release it names as stale.

## What wrapping a foreign filesystem costs

[`arrowfs`](arrowfs.md) puts an existing Arrow filesystem behind `IOBase`, so the only honest
question is what the wrapper adds to the transport underneath it. Every row below is the same
payload landing in the same place twice: once through an `arrowfs` handle, once through the native
handle holding those same bytes.

One containerized x86_64 Linux run (Intel Xeon @ 2.10GHz, 4 cores, 16 GiB; rustc 1.94.1,
`cargo bench --bench arrowfs --features "parquet"`, Criterion medians, 512 KiB payloads and 65,536
rows). Run it on your own hardware before drawing conclusions.

```text
                                  arrowfs      native handle
bytes read_all   (memory)         22.99 us     23.68 us   Buffer
bytes write_all  (memory)         67.00 us     24.85 us   Buffer
bytes pread 4KiB (memory)         63.91 ns     35.05 ns   Buffer
bytes read_all   (local)          25.82 us     23.51 us   local::File
bytes write_all  (local)         212.43 us      1.11 ms   local::File
ipc     write                      4.08 ms      4.44 ms   Buffer
ipc     read                       1.49 ms      1.30 ms   Buffer
parquet write                     18.16 ms     17.84 ms   Buffer
parquet read                       5.17 ms      5.01 ms   Buffer
ls recursive     (local)          61.49 us     92.84 us   local::Folder
```

The ranged read is the row that matters most, and it is the one stated in nanoseconds. Serving
4 KiB out of a 512 KiB value costs 64 ns, not the 23 us a whole-value read costs, so the handle
serves a range without materializing the value. The vtable itself is the 29 ns difference against
`Buffer`: one dynamic call plus a bounds check. This measures the handle, not any reader above it -
[`parquet`](parquet.md) still fetches its value whole, as that page says.

Whole-value writes are where staging shows. An Arrow filesystem replaces files rather than writing
ranges, so a write is buffered and published once - 67 us against `Buffer`'s 25 us for 512 KiB,
which is the copy the publication costs. Against the memory-mapped `local::File` the same write is
**five times faster** (212 us against 1.11 ms), because publishing a whole file through a temporary
and a rename beats remapping and resizing a mapping. Neither number makes one backend better than
the other; they measure different write shapes, which is exactly why both exist.

Records are within a few percent either way, because the encoding dominates and the wrapper only
moves the finished bytes. Listing is faster than the local backend's because one `list` call
answers a recursive walk that `std::fs::read_dir` has to make per directory.

`glob` over the same tree shows the descent the contract promises: expanding `**/*.parquet` across
a 16-leaf lake costs 57 us, while `year=2024/**/*.parquet` costs 23 us, because a fixed prefix is
descended rather than listed and filtered.

At the Python boundary the baseline is PyArrow's own calls against the same
`pyarrow.fs.LocalFileSystem` - the implementation the wrapper delegates to - so the difference is
the vtable crossing and nothing else. `arrowfs.py --min-time 0.2 --repeat 7`, release wheel,
medians:

```text
                        wrapper      PyArrow
bytes write            400.0 us     244.3 us
bytes read             115.2 us      17.6 us
range read (4 KiB)      14.5 us       2.9 us
parquet write            8.16 ms      6.19 ms
parquet read             2.00 ms      1.95 ms
listing (16 entries)    85.5 us      34.4 us
```

A Parquet read is at parity, because the decode dominates and the boundary moves only finished
bytes. Everything smaller is dominated by the crossing itself: each vtable call acquires the GIL
and makes a handful of PyArrow calls, which is roughly 12 us of fixed cost, so the 4 KiB range read
costs 14.5 us against PyArrow's 2.9 us. That cost is per *call*, not per byte - the ranged read
still reads 4 KiB rather than the 512 KiB object, which is the property that matters on an object
store, and it is why the read is 14.5 us rather than the 115 us a whole-value read takes.

A write costs more than PyArrow's because it is a different operation: the wrapper stages the value
and publishes it once, which is what makes a positional `pwrite` API work over a filesystem that
only replaces whole files.

JavaScript pays the same shape of cost against `node:fs`, with the handler crossing the boundary on
every call rather than only the handle. `bench:arrowfs`, release build:

```text
                            wrapper       node:fs
handle from path         107,101/s     257,632/s
write bytes                4,912/s      11,235/s
read bytes                13,193/s      58,399/s
read range (4 KiB)       124,733/s     227,893/s
list children             58,181/s     264,682/s
glob *.parquet             9,372/s      16,900/s
read records            15.6M rows/s   11.6M rows/s (local handle)
write records           10.2M rows/s   22.1M rows/s (local handle)
```

The ranged read is again the row that carries the claim: it is the *fastest* byte operation of the
three, not the slowest, because it fetches 4 KiB rather than the whole payload. Records read
faster than through the local handle because the staged value is already in memory once the first
read has fetched it, and slower to write for the same reason a Python write is - the value is
staged and published once.

## Big gzip log files, end to end

A production log directory is not one file: it is a rotation of gzip-coded leaves, and the
interesting question is what a *season* of them costs. Three targets - `lines_gzip` in the Rust
`text` bench, `log_lines_bulk.py`, and `bench:lines` - read one corpus generated byte-for-byte
identically in all three languages, so their rows describe the same work.

The corpus is **1,000,000 synthetic trading-log records across eight rotated `app-N.log.gz`
leaves**: 98,172,480 decoded bytes (93.6 MiB) from 17,590,854 bytes on disk (16.8 MiB, a 5.6x
coding ratio). Every fiftieth record is a three-line stack trace, so 1,040,000 text lines fold
into exactly 1,000,000 rows - and every corpus is proven to yield that row count over that line
count before a timer starts, because a parser that split the multi-line records, or dropped
their continuation lines while still being charged for the bytes, would otherwise just look
fast.

From one containerized x86_64 Linux run, each target run alone (`cargo bench --bench text --
lines_gzip`; Criterion, 10 samples, medians with 95% intervals):

```text
lines_gzip/folder/gzip    7.3296 s   12.773 MiB/s   [7.3010 s 7.3577 s]
lines_gzip/folder/plain   7.2334 s   12.943 MiB/s   [7.2015 s 7.2765 s]
lines_gzip/single/gzip    8.0862 s   11.578 MiB/s   [8.0155 s 8.1522 s]
lines_gzip/casts/typed    7.3877 s   12.673 MiB/s   [7.3146 s 7.4974 s]
lines_gzip/casts/text     7.1908 s   13.020 MiB/s   [7.1627 s 7.2229 s]
lines_gzip/scale/125k   917.96 ms    12.749 MiB/s
lines_gzip/scale/250k     1.8404 s   12.718 MiB/s
lines_gzip/scale/500k     3.9324 s   11.904 MiB/s
lines_gzip/scale/1m       8.0716 s   11.599 MiB/s
```

**The headline is 93.6 MiB of compressed log text parsed into typed Arrow batches in 7.33 s -
136,433 rows/s.**

**Read the structural deltas in that table as nothing.** `scale/1m` re-reads the *same corpus
with the same options* as `folder/gzip`, at the end of the run instead of the start, and lands
10% slower (8.0716 s against 7.3296 s) - so this container's own drift across a fourteen-minute
run is larger than every structural difference the group is trying to isolate. The coding, the
rotation, and the cast are all sub-5% effects; none of them is separable here, and quoting one
would be reading a number that says nothing. Two rows that survive the drift because they are
not sub-5%:

- **Nothing is quadratic.** The sweep holds 11.90-12.75 MiB/s while the corpus grows eight-fold,
  and the times double cleanly: 918 ms, 1.840 s, 3.932 s, 8.072 s (2.00x, 2.14x, 2.05x). Flat
  per-byte throughput is evidence about *shape*, not about residency - a reader that buffered
  the whole decoded corpus before emitting a batch would look just as flat - which is why the
  memory claim is measured separately, below.
- **Inflate itself is free**, which the coded-against-plain rows cannot show through the drift:
  `lines_arrow/parse/{plain,gzip}` moves identical source bytes through in-memory handles and
  reports 594.70 ms against 595.82 ms - indistinguishable. The coded leaves also move a fifth of
  the bytes off storage, so the folder rows measure a net trade rather than the inflate.

**Against the implementation this surface replaced**, on this same container and the same corpus,
the pre-refactor line reader measures 7.0888 s where this one measures 7.3296 s - about 3% - and
the stages localize it: `lines_arrow/group/plain` (splitting records, no projection) is
identical at 231.78 ms against 237.87 ms, and `lines_arrow/hash/corpus` is identical at 3.34 ms.
The difference is in the Arrow projection, where the old code ran one match and read everything
straight out of it while a `TextLine` resolves the same fields through lazily cached accessors -
which is what buys the borrowed view, the write side, log mode, zones, and trimming. Removing
the per-record allocations recovered 4% of it; the rest is that indirection, and it is stated
here rather than left for a reader to discover.

### What the record shape costs

The `lines_shape` group prices the extractor's own options against one corpus whose record sizes
swing by two orders of magnitude - a ~4 KB stack trace next to ~40-byte neighbours - so the batch
bounds have something to disagree about:

```text
lines_shape/batching/rows       122.34 ms   29.213 MiB/s   20 batches, rows 544..=1024
lines_shape/batching/bytes      123.67 ms   28.899 MiB/s    4 batches, rows 3220..=5594
lines_shape/detect/timestamp     33.33 ms  107.22 MiB/s
lines_shape/detect/regex         74.84 ms   47.751 MiB/s
lines_shape/zone/naive          125.70 ms   28.432 MiB/s
lines_shape/zone/fixed          125.20 ms   28.544 MiB/s
lines_shape/zone/named          123.05 ms   29.044 MiB/s
lines_shape/strip/whitespace    123.72 ms   28.887 MiB/s
lines_shape/strip/none          122.94 ms   29.070 MiB/s
```

- **Timestamp detection is 2.2x faster than the equivalent pattern** (33.33 ms against 74.84 ms).
  Log mode opens a record where `parse_datetime_prefix` succeeds at the line's start, behind a
  cheap first-byte guard, and never runs a regex per line; `^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*`
  over the same corpus is the row beneath it. Both find the same record boundaries. Non-ISO
  timestamp shapes still need a pattern - detection is not a general date parser - but for the
  shapes it accepts, it is the faster default as well as the one needing no configuration.
- **The two batch bounds are the same work, split differently.** Row-bounded closes 20 batches of
  544-1024 rows; byte-bounded closes 4 of 3220-5594. The wall clock is within a percent either
  way, so choosing between them is about the batch *shape* a downstream consumer wants, not about
  throughput - which is the point of both defaulting and the first to trip winning.
- **A zone is free, and so is trimming.** `zone/named` resolves a DST-observing zone through the
  registry and lands inside the naive row's interval, because the offset is cached rather than
  resolved per row - a sorted log resolves it once or twice per file. `strip/none` against the
  default `whitespace` is likewise indistinguishable: trimming moves the record's own start and
  end rather than copying anything, so the option that does nothing saves nothing.

### The binding boundaries, on the same corpus

`log_lines_bulk.py --measure-memory` (release wheel, Python 3.11.15, PyArrow 25.0.1):

```text
read folder gzip            7659.096 ms   130,564 rows/s   12.2 MiB/s decoded
read folder plain           7615.238 ms   131,316 rows/s   12.3 MiB/s decoded
read single gzip            7602.393 ms   131,538 rows/s   12.3 MiB/s decoded
read folder utf8            7523.360 ms   132,919 rows/s   12.4 MiB/s decoded
typed accessors             7694.314 ms   129,966 rows/s   12.2 MiB/s decoded
text captures + py cast     7674.664 ms   130,299 rows/s   12.2 MiB/s decoded
```

**The Python binding is free.** 130,564 rows/s against the Rust core's 136,433 is the same
number twice once the container's own drift is allowed for: the reader crosses as an Arrow C
Stream, pulled one batch at a time with no copy, so a Python caller pays for the parse and
nothing else. Note what this target *cannot* settle either - its rows sit inside a 2% band, and
the coding, rotation, and cast effects are all smaller than that, so none of them is a result
here.

The memory table is what Criterion cannot report, and it is the reason the projection is a
reader rather than a function returning a table. Each size is measured in its own process
(`ru_maxrss` is a whole-process high-water mark), reading a corpus the parent already wrote:

```text
probe                   records   decoded MiB   peak RSS MiB   over floor MiB
floor (no corpus)             0           0.0           68.6             0.0
scale 1/8               125,000          11.7           85.1            16.5
scale 1/4               250,000          23.4           96.3            27.7
scale 1/2               500,000          46.8          120.7            52.1
scale 1/1             1,000,000          93.6          122.9            54.3
```

**Residency is bounded by the batch, not by the corpus.** Doubling from 500,000 records to
1,000,000 - and from 46.8 MiB of text to 93.6 MiB - moves the peak by 2.2 MiB, because what is
resident is one batch and its builders, never the file. That is the streaming contract as a
measurement rather than a promise, and it is what lets a log directory larger than memory parse
at all.

**The level it settles at is a default, and it is a choice.** 54 MiB above the bare interpreter
is what `batch_size` 65,536 and `byte_size` 8 MiB cost on this record shape - roughly eight
times the input bytes a batch holds, in offsets, validity, and builder headroom across a dozen
columns. Batches that size are what a vectorized consumer wants; a caller who would rather have
the footprint sets `batch_size` and gets it back proportionally. The row bound is the one that
trips first here, at about 6.4 MiB of input per batch.

`npm run --prefix node bench:lines` (release addon, **250,000 records** - a quarter of the Rust
and Python corpus, because the boundary is dearer):

```text
readArrowLines folder gzip    2023.843 ms   123,527 rows/s   11.6 MiB/s    6.3% spread
readArrowLines folder plain   2174.462 ms   114,971 rows/s   10.8 MiB/s   16.2% spread
readArrowLines single gzip    2206.057 ms   113,324 rows/s   10.6 MiB/s    7.1% spread
readArrowLines folder utf8    2147.227 ms   116,429 rows/s   10.9 MiB/s    2.3% spread
typed accessors               2078.320 ms   120,289 rows/s   11.3 MiB/s    7.6% spread
text captures + js cast       2036.664 ms   122,750 rows/s   11.5 MiB/s    1.0% spread
```

Arrow JS has no Arrow C Data consumer, so this binding hands every batch across as its own
copied Arrow IPC stream - encoded natively, decoded again in JavaScript - which the other two
never pay. **It costs under 10%** (123,527 rows/s against the core's 136,433): the parse is
expensive enough that a per-batch copy barely shows. Every structural delta in that table is
smaller than the spread printed beside it, so none of them is a result; the target prints the
spread precisely so nobody reads one.

### The pipeline those numbers measure

The same read, small enough to run, and carried through to a table: a folder of rotated leaves -
one gzip, one plain, one zstd - parsed into batches, combined with a second parse whose schema is
one column short, and appended to an Iceberg table in one commit. Nothing names a codec or a format
anywhere in it, nothing checks whether a resource exists, and nothing is collected: this is the
shape that carries a season of logs larger than memory.

The stages, and what each one proves:

1. **The table exists before the first record does.** `schema_from_pattern` derives the projection's
   root from the extractor configuration alone - no resource, no reader - and its
   [partition marks](field.md#a-field-can-be-a-partition-column) become the table's spec.
2. **One handle reads mixed codings.** `incoming/` holds `app-0.log.gz`, `app-1.log`, and
   `app-2.log.zst`; each leaf is decoded by its *own* media type, in name-sorted order, one at a
   time, and no line here says gzip or zstd.
3. **The extractor is the parse and half the transform.** `took=(?<latency_us>\d+)` infers `int64`
   from its own sub-pattern, so the arithmetic below is on a number column rather than on per-row
   string parsing; `custom_fields` stamps the constant `source` column; `byte_size` closes each
   batch on decoded input bytes.
4. **The combine is the other half, and it is lazy.** The archived leaves are parsed by an older
   extractor that has no `thread_id` capture. `combined` derives the root the two schemas merge
   into - from the schemas alone, which a reader answers without pulling a batch - so the archived
   rows arrive with a null `thread_id` and neither side is drained to find that out.
5. **The append is one commit** of that lazy reader, and the read-back asserts on the *table*
   rather than on anything the example still holds in memory.

A record spanning a stack trace stays in the fixture, because multi-line grouping is the case that
makes the line surface worth having.

=== "Rust"

    ```rust
    use arrow_array::{Array, Int64Array};
    use yggdryl::iceberg::Catalog;
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::text::TextLineOptions;
    use yggdryl::Value;

    let pattern = r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\] \[(?<thread_id>\d+)\] took=(?<latency_us>\d+)";
    // The older extractor: same records, no thread column.
    let archived = r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S* \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\] \[\d+\] took=(?<latency_us>\d+)";

    let root = std::env::temp_dir().join(format!("yggdryl-docs-pipeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let incoming = root.join("incoming");
    let archive = root.join("archive");
    std::fs::create_dir_all(&incoming)?;
    std::fs::create_dir_all(&archive)?;

    // Three rotated leaves in three codings; the second record spans a stack trace.
    let first = concat!(
        "2024-02-01 10:00:00.000000 [ii] [engine] [3] took=120 fill 100 SYMB-0001\n",
        "2024-02-01 10:00:01.000000 [ee] [engine] [4] took=980 fill 101 SYMB-0002\n",
        "    at engine::match(order.rs:118)\n",
        "    at engine::step(order.rs:64)\n",
        "2024-02-01 10:00:02.000000 [ww] [router] [5] took=240 fill 102 SYMB-0003\n",
    );
    std::fs::write(incoming.join("app-0.log.gz"), yggdryl::gzip::dump(first.as_bytes())?)?;
    std::fs::write(
        incoming.join("app-1.log"),
        b"2024-02-01 10:00:03.000000 [ee] [ledger] [6] took=770 fill 103 SYMB-0004\n",
    )?;
    std::fs::write(
        incoming.join("app-2.log.zst"),
        yggdryl::zstd::dump(b"2024-02-01 10:00:04.000000 [ii] [feed] [7] took=100 fill 104 SYMB-0005\n")?,
    )?;
    std::fs::write(
        archive.join("app-9.log.gz"),
        yggdryl::gzip::dump(b"2024-01-31 23:59:59.000000 [ee] [engine] [2] took=310 fill 099 SYMB-0000\n")?,
    )?;

    // 1. The extractor, and the table it implies - both before anything is read.
    let options = TextLineOptions::with_pattern(pattern)?
        .try_with_custom_fields([("source", Value::from("gateway"))])?
        .with_byte_size(8 * 1024 * 1024);
    let marked = options.schema().with_partition_fields(&["level"])?;

    let catalog = Catalog::new(Folder::new(root.join("warehouse"))?);
    let mut table = catalog.tables().create("logs.app", marked)?;

    // 2, 3, 4. One handle per folder, and one lazy combine over the two.
    let older = TextLineOptions::with_pattern(archived)?
        .try_with_custom_fields([("source", Value::from("archive"))])?;
    let stream = yggdryl::arrow::combined(
        Folder::new(&incoming)?.into_arrow_lines(&options)?,
        Folder::new(&archive)?.into_arrow_lines(&older)?,
    )?;

    // 5. One commit, handed the reader itself - never a Vec of batches.
    table.append(stream)?;

    // The read-back asserts on the table, not on anything held in memory.
    let mut rows = 0_usize;
    let mut latency = 0_i64;
    let mut threadless = 0_usize;
    for batch in table.scan(None)? {
        let batch = batch?;
        rows += batch.num_rows();
        let took = batch
            .column_by_name("latency_us")
            .expect("the typed capture")
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("int64 by inference, not by declaration");
        latency += (0..batch.num_rows()).map(|row| took.value(row)).sum::<i64>();
        threadless += batch
            .column_by_name("thread_id")
            .expect("the merged column")
            .null_count();
    }
    // Five live records - not the seven lines they occupy - and one archived.
    assert_eq!(rows, 6);
    assert_eq!(latency, 120 + 980 + 240 + 770 + 100 + 310);
    assert_eq!(threadless, 1, "the archived extractor had no thread column");

    let reopened = catalog.table("logs.app")?;
    assert_eq!(reopened.metadata().default_spec()?.fields[0].name, "level");
    assert!(reopened.schema()?.get_field_by_name("source").is_some());

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa
    import pyarrow.compute as pc

    from yggdryl import IOBase, combined, schema_from_pattern, zstd
    from yggdryl.iceberg import Catalog

    pattern = (
        r"^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\S*"
        r" \[(?<level>[^\]]+)\] \[(?<logger>[^\]]+)\]"
        r" \[(?<thread_id>\d+)\] took=(?<latency_us>\d+)"
    )
    # The older extractor: same records, no thread column.
    archived_pattern = pattern.replace(r"\[(?<thread_id>\d+)\]", r"\[\d+\]")

    extractor = {
        "pattern": pattern,
        "byte_size": 8 * 1024 * 1024,
        "custom_fields": {"source": "gateway"},
    }
    older = {"pattern": archived_pattern, "custom_fields": {"source": "archive"}}

    root = pathlib.Path(tempfile.mkdtemp(prefix="yggdryl-doc-"))
    incoming = root / "incoming"
    archive = root / "archive"
    incoming.mkdir()
    archive.mkdir()

    # Three rotated leaves in three codings; the second record spans a stack trace.
    (incoming / "app-0.log.gz").write_bytes(
        gzip.compress(
            b"2024-02-01 10:00:00.000000 [ii] [engine] [3] took=120 fill 100 SYMB-0001\n"
            b"2024-02-01 10:00:01.000000 [ee] [engine] [4] took=980 fill 101 SYMB-0002\n"
            b"    at engine::match(order.rs:118)\n"
            b"    at engine::step(order.rs:64)\n"
            b"2024-02-01 10:00:02.000000 [ww] [router] [5] took=240 fill 102 SYMB-0003\n"
        )
    )
    (incoming / "app-1.log").write_bytes(
        b"2024-02-01 10:00:03.000000 [ee] [ledger] [6] took=770 fill 103 SYMB-0004\n"
    )
    (incoming / "app-2.log.zst").write_bytes(
        zstd.dumps(b"2024-02-01 10:00:04.000000 [ii] [feed] [7] took=100 fill 104 SYMB-0005\n")
    )
    (archive / "app-9.log.gz").write_bytes(
        gzip.compress(
            b"2024-01-31 23:59:59.000000 [ee] [engine] [2] took=310 fill 099 SYMB-0000\n"
        )
    )

    # 1. The table exists before the first record does.
    marked = schema_from_pattern(options=extractor).with_partition_fields(["level"])
    catalog = Catalog(root / "warehouse")
    table = catalog.tables.create("logs.app", marked)

    # 2, 3, 4. One handle per folder, and one lazy combine over the two: both
    # schemas are answered without pulling a batch, so nothing is read yet.
    stream = combined(
        IOBase(incoming).read_arrow_lines(options=extractor),
        IOBase(archive).read_arrow_lines(options=older),
    )

    # 5. One commit, handed the reader itself - never a list of batches.
    table.append(stream)

    # The read-back asserts on the table, not on anything held in memory.
    rows = table.scan().read_all()
    # Five live records - not the seven lines they occupy - and one archived.
    assert rows.num_rows == 6
    assert pc.sum(rows.column("latency_us")).as_py() == 120 + 980 + 240 + 770 + 100 + 310
    assert rows.schema.field("latency_us").type == pa.int64()
    assert rows.column("thread_id").null_count == 1

    reopened = catalog.table("logs.app")
    assert [field.name for field in reopened.spec.fields] == ["level"]
    assert set(rows.column("source").to_pylist()) == {"gateway", "archive"}

    shutil.rmtree(root)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase, iceberg, schemaFromPattern, zstd } = require('yggdryl')

    const pattern =
      '^\\d{4}-\\d{2}-\\d{2} \\d{2}:\\d{2}:\\d{2}\\S*' +
      ' \\[(?<level>[^\\]]+)\\] \\[(?<logger>[^\\]]+)\\]' +
      ' \\[(?<thread_id>\\d+)\\] took=(?<latency_us>\\d+)'
    // The older extractor: same records, no thread column.
    const archivedPattern = pattern.replace('\\[(?<thread_id>\\d+)\\]', '\\[\\d+\\]')

    const extractor = {
      pattern,
      byteSize: 8 * 1024 * 1024,
      customFields: { source: 'gateway' },
    }
    const older = { pattern: archivedPattern, customFields: { source: 'archive' } }

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-'))
    const incoming = path.join(root, 'incoming')
    const archive = path.join(root, 'archive')
    fs.mkdirSync(incoming)
    fs.mkdirSync(archive)

    // Three rotated leaves in three codings; the second record spans a stack trace.
    fs.writeFileSync(
      path.join(incoming, 'app-0.log.gz'),
      zlib.gzipSync(
        Buffer.from(
          '2024-02-01 10:00:00.000000 [ii] [engine] [3] took=120 fill 100 SYMB-0001\n' +
            '2024-02-01 10:00:01.000000 [ee] [engine] [4] took=980 fill 101 SYMB-0002\n' +
            '    at engine::match(order.rs:118)\n' +
            '    at engine::step(order.rs:64)\n' +
            '2024-02-01 10:00:02.000000 [ww] [router] [5] took=240 fill 102 SYMB-0003\n',
        ),
      ),
    )
    fs.writeFileSync(
      path.join(incoming, 'app-1.log'),
      '2024-02-01 10:00:03.000000 [ee] [ledger] [6] took=770 fill 103 SYMB-0004\n',
    )
    fs.writeFileSync(
      path.join(incoming, 'app-2.log.zst'),
      zstd.dumps(
        Buffer.from('2024-02-01 10:00:04.000000 [ii] [feed] [7] took=100 fill 104 SYMB-0005\n'),
      ),
    )
    fs.writeFileSync(
      path.join(archive, 'app-9.log.gz'),
      zlib.gzipSync(
        Buffer.from('2024-01-31 23:59:59.000000 [ee] [engine] [2] took=310 fill 099 SYMB-0000\n'),
      ),
    )

    // 1. The table exists before the first record does.
    const marked = schemaFromPattern(extractor).withPartitionFields(['level'])
    const catalog = new iceberg.Catalog(path.join(root, 'warehouse'))
    const table = catalog.tables.create('logs.app', marked)

    // 2, 3, 4. One handle per folder, and one lazy combine over the two.
    const stream = new IOBase(incoming)
      .readArrowLines(extractor)
      .combined(new IOBase(archive).readArrowLines(older))

    // 5. One commit, handed the reader itself - never an array of batches.
    table.append(stream)

    // The read-back asserts on the table, not on anything held in memory.
    const rows = table.scan().toTable()
    // Five live records - not the seven lines they occupy - and one archived.
    assert.equal(rows.numRows, 6)
    const latency = [...rows.getChild('latency_us')].reduce((total, took) => total + took, 0n)
    assert.equal(latency, 120n + 980n + 240n + 770n + 100n + 310n)
    assert.equal(rows.getChild('thread_id').nullCount, 1)
    assert.deepEqual(
      new Set(rows.getChild('source')),
      new Set(['gateway', 'archive']),
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

## What a page cache buys, and what it costs

`io_buffered` runs three read workloads over one 16 MiB fixture and every handle the core
ships, plus a fourth workload over a compressed one. The handles fall into two families, and
that split is the whole result:

- **Already memory.** An in-memory `Buffer`, a memory-mapped `local::File`, and an
  [`arrowfs`](arrowfs.md) handle over `MemoryFileSystem`. A read is a `memcpy` - out of a
  `Vec`, out of the mapping the kernel already caches, or out of the vtable's own map.
- **A fetch per read.** An `arrowfs` handle over `LocalFileSystem`, where every `pread` is an
  `open`, a `seek` and a `read`, and every `size` is a `stat`. That is the shape of every
  object store, and the only such backend the core ships.

`random` reads 512 bytes at a time inside a 4 MiB hot region that fits the 8 MiB budget;
`sequential` scans the whole 16 MiB in 8 KiB steps, twice the budget, so every page is
fetched once and evicted before it is wanted again; `footer` reads both ends, sweeps 12 MiB
of the middle, and reads both ends again.

From one containerized x86_64 Linux run with the group run alone
(`cargo bench --bench io --features "parquet" -- io_buffered`; Intel Xeon 2.10 GHz, 4 cores,
Rust 1.97.1, `--release` with thin LTO; Criterion, 100 samples, medians). This box's
run-to-run spread is wide - a case can move 15% between runs - so read the multiples, never
the percentages:

```text
                                     random        sequential        footer
buffer                             10.041 µs        606.58 µs      439.58 µs
file                               28.037 µs        517.80 µs      367.66 µs
buffered  (over file)              75.362 µs      1.9495 ms        507.49 µs
arrowfs_memory                     46.739 µs        734.30 µs      392.83 µs
arrowfs_memory_buffered            77.633 µs      2.0068 ms        503.69 µs
arrowfs_local                     1.0832 ms       2.7574 ms       2.0924 ms
arrowfs_local_buffered             73.668 µs      2.3841 ms        532.18 µs
```

**Over a backend that is already memory, the cache is a cost; over one that fetches, it is
worth 4x to 15x.** The same code, the same page table, the same pinning:

| Workload | `arrowfs_local` | with the cache | |
| --- | --- | --- | --- |
| `random` (a hot region, re-read) | 1.0832 ms | 73.668 µs | **14.7x faster** |
| `footer` (both ends, big middle) | 2.0924 ms | 532.18 µs | **3.9x faster** |
| `sequential` (one pass, nothing re-read) | 2.7574 ms | 2.3841 ms | **1.2x faster** |

The ordering of those three is the useful part. A cache pays where reads *repeat* - a hot
region, or the two ends of a footer-first container - and barely pays where every byte is
read exactly once, because a one-pass scan copies each byte twice and reuses none of it. The
`sequential` row is the honest floor: 8 KiB reads through 64 KiB pages, so the cache still
turns eight `open`/`seek`/`read` triples into one, and that is worth 16%.

Over the memory-like handles the same cache costs 2.7x (`random`, against `file`), because
there was no fetch to remove: a hit is a clock read, a lock, a map lookup and a copy against
a `memcpy` that was going to happen anyway. That is the price of not knowing what you were
handed, and it is why the wrapper is opt-in rather than automatic.

**Two changes during this work moved these numbers, and both are in the diff:**

- *A hit asks the handle for nothing.* `read_at` used to call `size()` on every read, for the
  end-of-value bound and the pin. On `arrowfs_local` that is a `stat` per read - the cache
  paying exactly the cost it exists to remove. The size is now remembered beside the pages and
  re-asked only when a read runs past what the cache knows. `random/arrowfs_local_buffered`
  went **597.88 µs to 73.668 µs** and `sequential` turned from a 1.2x *loss* into a win.
- *Dense page indexes hash with a multiply and a rotation* rather than SipHash, and the offset
  arithmetic shifts rather than divides - which is what the power-of-two page size is for.
  Together, −20% on the hit case.

**What pinning buys cannot be timed over a backend whose re-read is a `memcpy`**, so the
target asserts it as a count before any timer starts, over a counting handle: after a 12 MiB
middle scan four times wider than the whole budget, re-reading the head and the tail costs
**zero** inner fetches. On `arrowfs_local`, where a fetch is real, that count is what the
`footer` row's 3.9x is made of.

### Over a compressed handle

A content coding is not seekable, so [`Coded`](io.md) answers a positional read by decoding
the value, and *which* decode it pays depends on whether the handle is open. The `coded`
cases read a 256 KiB gzip value in 64 reads of 4 KiB:

```text
io_buffered/coded/closed     18.673 ms    13.389 MiB/s
io_buffered/coded/open        4.2057 µs   58.050 GiB/s
io_buffered/coded/buffered    7.8713 µs   31.017 GiB/s
```

- **`closed` is the trap.** Nothing may be cached as a side effect of an ordinary read, so a
  coded handle nobody opened decodes the **whole payload for every `pread`**: 64 reads, 64
  decodes, 18.7 ms to read 256 KiB.
- **`open` is the cure the coding already ships** - and it got **~100x faster in this diff**.
  `Coded::pread` used to reach the materialized value through a helper that *cloned* it, so an
  open handle copied the entire payload to serve four bytes; `size()` cloned it just to read
  a length. Both now borrow. Measured on the same case: **420.40 µs to 4.2057 µs**.
- **`buffered` is what the page cache is worth when the handle is not opened** - the case a
  caller who does not know what they were handed is in. It turns one decode per read into one
  decode per page miss: **2,372x faster than `closed`**, within 2x of the open path.

The order of wrapping is the useful one: `Buffered<Coded<_>>` caches the *decoded* bytes.
`Coded<Buffered<_>>` would cache the compressed bytes and still decode on every read.

## Placeholder substitution

`{{ }}` substitution is a feature almost no document uses, so the question that matters is what it
costs the documents that do *not*. `codec/placeholder` answers it directly: the same 256-entry JSON
document parsed with the feature off and on, at three placeholder densities. From one containerized
x86_64 Linux run (`cargo bench --bench text -- codec/placeholder`;
Criterion medians with 95% intervals):

```text
codec/placeholder/none/off   88.786 us   [88.029 us 89.799 us]
codec/placeholder/none/on    91.827 us   [91.692 us 91.978 us]
codec/placeholder/few/off    90.048 us   [89.508 us 90.662 us]
codec/placeholder/few/on    130.400 us   [129.45 us 131.43 us]
codec/placeholder/most/off   88.717 us   [87.809 us 88.717 us]
codec/placeholder/most/on   219.970 us   [219.66 us 220.30 us]
```

**The guard costs about 3 us on a 7.6 KiB document - 3.4% of the parse, ~0.4 ns a byte.** That is
the whole delta between `none/off` and `none/on`: one linear scan for `{{`, and when it finds
nothing the parsed value is returned untouched - no walk, no allocation, no per-scalar inspection.
It is small, and it is not zero, so it is stated rather than rounded away. It is written as a
single-byte search that checks its neighbour rather than as a two-byte window comparison, which
measured worse; a SIMD byte-search dependency would shrink it further and has not been taken for
3 us.

The other two rows are the work itself, not the guard: eight placeholders in 256 scalars costs
40 us (`few`), and 256 of them cost 131 us - about 0.5 us a substituted scalar either way, which is
the fresh string each one builds. Only string scalars are visited, and only the ones that actually
contain a placeholder are rebuilt.

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

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/benchmarks-rust.ipynb){ download },
[Python](notebooks/benchmarks-python.ipynb){ download },
[JavaScript](notebooks/benchmarks-javascript.ipynb){ download }.

<!-- /notebooks -->
