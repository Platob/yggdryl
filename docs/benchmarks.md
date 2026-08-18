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
    cargo bench --bench text --features "parquet iceberg" -- lines_gzip   # ~12 min alone
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
| `text` | Value construction, format inference, display and elision helpers, the `{{ }}` placeholder guard against the substitution it guards, the text-record Arrow projection and its hash, record shape (batch bounds, timestamp detection against the equivalent pattern, zones, trimming), and (`lines_gzip`) a million-record rotated gzip log folder: content coding, folder shape, typed captures, and a scale sweep |
| `json`, `toml`, `yaml` | Whole-value and streaming encode and decode |
| `avro` | Container decode and encode by type family, codec x block-size sweep, projection skips, resolution plans, the varint floor |
| `io` | Record round-trips over handles and projection pushdown |
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

From one containerized x86_64 Linux run, each target run alone (`cargo bench --bench text
--features "parquet iceberg" -- lines_gzip`; Criterion, 10 samples, medians with 95% intervals):

```text
lines_gzip/folder/gzip    5.4928 s   17.045 MiB/s   [5.4374 s 5.5504 s]
lines_gzip/folder/plain   5.3714 s   17.430 MiB/s   [5.3328 s 5.4104 s]
lines_gzip/single/gzip    5.4684 s   17.121 MiB/s   [5.4099 s 5.5341 s]
lines_gzip/casts/typed    5.4964 s   17.034 MiB/s   [5.4164 s 5.5767 s]
lines_gzip/casts/text     5.3721 s   17.428 MiB/s   [5.3386 s 5.4045 s]
lines_gzip/scale/125k   699.47 ms    16.731 MiB/s
lines_gzip/scale/250k     1.4050 s   16.659 MiB/s
lines_gzip/scale/500k     2.7508 s   17.017 MiB/s
lines_gzip/scale/1m       5.5704 s   16.807 MiB/s
```

**The headline is 93.6 MiB of compressed log text parsed into typed Arrow batches in 5.49 s -
182,057 rows/s.** What the other rows are for:

- **Nothing is quadratic.** The sweep holds 16.66-17.02 MiB/s while the corpus grows eight-fold,
  and the times double cleanly: 699 ms, 1.405 s, 2.751 s, 5.570 s (2.01x, 1.96x, 2.02x). Flat
  per-byte throughput is evidence about *shape*, not about residency - a reader that buffered
  the whole decoded corpus before emitting a batch would look just as flat - which is why the
  memory claim is measured separately, below.
- **Storing the logs gzip-coded costs about 2%** (5.4928 s against 5.3714 s; the intervals just
  clear each other). Read that as the *net* trade, not as the inflate in isolation: the coded
  leaves move a fifth of the bytes off storage and then inflate them, and the two effects pull
  in opposite directions. For inflate alone, both sides must move identical source bytes, which
  is what the older `lines_arrow/parse/{plain,gzip}` pair does with in-memory handles (~1%).
  Two percent of wall clock for 5.6x less storage is the trade a production reader is making.
- **The rotation is free.** Eight leaves against one (5.4928 s against 5.4684 s) is within the
  intervals: per-leaf open, media-type inference, and the batch boundary forced at every leaf
  cost nothing measurable. A folder of a thousand daily logs reads like one file.
- **Typed captures cost about 2%.** `casts/typed` against `casts/text` (5.4964 s against
  5.3721 s) is the strict native cast on two captures for every record - 2,000,000 values -
  which works out near 62 ns a value. Both rows only count rows, so nothing but the cast
  differs. That is the price of `latency_us` arriving as `int64` instead of text.

### The binding boundaries, on the same corpus

`log_lines_bulk.py --measure-memory` (release wheel, Python 3.11.15, PyArrow 25.0.1):

```text
read folder gzip            5476.949 ms   182,583 rows/s   17.1 MiB/s decoded
read folder plain           5519.981 ms   181,160 rows/s   17.0 MiB/s decoded
read single gzip            5538.659 ms   180,549 rows/s   16.9 MiB/s decoded
read folder utf8            5539.805 ms   180,512 rows/s   16.9 MiB/s decoded
typed accessors             6067.945 ms   164,800 rows/s   15.4 MiB/s decoded
text captures + py cast     5694.085 ms   175,621 rows/s   16.4 MiB/s decoded
```

**The Python binding is free.** 182,583 rows/s against the Rust core's 182,057 is the same
number twice: the reader crosses as an Arrow C Stream, pulled one batch at a time with no copy,
so a Python caller pays for the parse and nothing else. Note also what this target *cannot*
settle: its own run-to-run spread is a percent or two, so the coding, rotation, and cast deltas
- all around 2% - flip sign between runs here. They are quoted from Criterion's intervals above
and nowhere else.

The memory table is what Criterion cannot report, and it is the reason the projection is a
reader rather than a function returning a table. Each size is measured in its own process
(`ru_maxrss` is a whole-process high-water mark), reading a corpus the parent already wrote:

```text
probe                   records   decoded MiB   peak RSS MiB   over floor MiB
floor (no corpus)             0           0.0           68.5             0.0
scale 1/8               125,000          11.7           74.9             6.4
scale 1/4               250,000          23.4           75.1             6.6
scale 1/2               500,000          46.8           75.7             7.2
scale 1/1             1,000,000          93.6           76.5             8.0
```

**Eight times the corpus costs 1.25x the memory.** Reading 93.6 MiB of decoded text needs 8.0
MiB above the bare interpreter - one batch at a time, content codings decoded as streams, the
whole file never resident. That is the streaming contract as a measurement rather than a
promise, and it is what lets a log directory larger than memory parse at all.

`npm run --prefix node bench:lines` (release addon, **250,000 records** - a quarter of the Rust
and Python corpus, because the boundary is dearer):

```text
readArrowLines folder gzip    1476.144 ms   169,360 rows/s   15.9 MiB/s    7.4% spread
readArrowLines folder plain   1460.570 ms   171,166 rows/s   16.0 MiB/s    5.2% spread
readArrowLines single gzip    1440.573 ms   173,542 rows/s   16.2 MiB/s    4.4% spread
readArrowLines folder utf8    1420.733 ms   175,965 rows/s   16.5 MiB/s    6.0% spread
typed accessors               1564.626 ms   159,783 rows/s   15.0 MiB/s   17.3% spread
text captures + js cast       1564.922 ms   159,752 rows/s   15.0 MiB/s    9.1% spread
```

Arrow JS has no Arrow C Data consumer, so this binding hands every batch across as its own
copied Arrow IPC stream - encoded natively, decoded again in JavaScript - which the other two
never pay. **It costs about 7%** (169,360 rows/s against the core's 182,057): the parse is
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
    let mut table = catalog.create_table("logs.app", marked)?;

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
    table = catalog.create_table("logs.app", marked)

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
    const table = catalog.createTable('logs.app', marked)

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

## Placeholder substitution

`{{ }}` substitution is a feature almost no document uses, so the question that matters is what it
costs the documents that do *not*. `codec/placeholder` answers it directly: the same 256-entry JSON
document parsed with the feature off and on, at three placeholder densities. From one containerized
x86_64 Linux run (`cargo bench --features "parquet iceberg" --bench text -- codec/placeholder`;
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
