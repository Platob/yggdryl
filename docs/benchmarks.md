# Benchmarks

Results live beside the method they measure. Each page's Performance section names its host and toolchain and ends with the command that regenerates it.

## Where the numbers are

| Tab | Page | Measures |
| --- | --- | --- |
| Arrow | [Schema](arrow/schema.md) | The `types` Criterion target times only the three Struct-root methods over one nested fixture built outside... |
| Expression | [Evaluate](expression/evaluate.md) | `benchmarks/expression.rs` writes each predicate by hand against `arrow-ord` / `arrow-select`, and `express... |
| FIX | [FIX](fix/index.md) | Field setters and the `FixId` codec, including `fix/ulbridge/step/set_bloombergcode` for synchronizing a normalized Bloomberg identifier into `secaltids` |
| FIX | [Arrow](fix/arrow.md) | Historical `fix/pipeline` release estimates and a current debug counting-allocator ULBridge profile for row and Arrow-batch requests; the latter makes no CPU or throughput claim |
| FIX | [Message](fix/message.md) | `fix/ulbridge/step/set_bloombergcode` measures one `FixMsg` identifier setter including `secaltids` synchronization |
| FIX | [Registry](fix/registry.md) | Lookups and mutations over the tracked seed: the Rust column one release run of the Criterion target on a Linux x86_64 container, the Python and Node columns an earlier Windows run, so a row compares a language against its own boundary |
| FIX | [Store](fix/store.md) | Folder loads, snapshots and writes over the tracked seed: the Rust column one release run of the Criterion target on a Linux x86_64 container, the Python and Node columns an earlier Windows run, so a row compares a language against its own boundary |
| Hashing | [Hashing](hashing.md) | The `hashing` Criterion target, `python/benchmarks/digest.py` and `txhash.py`, and `node/benchmarks/hashing/`: digest throughput per algorithm and size, handle reads and write-through, the value feed, Arrow row digests, the coupling beside the digest it wraps, and coupled column and holder costs, with both bindings; containerized x86_64 Linux runs on one host |
| Holder | [Buffered](holder/index.md#buffered-performance) | `io_buffered` runs three workloads over one 16 MiB fixture and every shipped handle: one containerized x86_... |
| Holder | [Filesystems](holder/index.md#filesystems-performance) | The benchmark times the wrapper against direct PyArrow, local, or native local operations; gates rather than published medians |
| Holder | [Object stores](holder/index.md#object-stores-performance) | Both clients against one in-process store over a real socket: reads, writes under either payload policy, and listings, beside `object_store` 0.13.2 |
| Holder | [Bytes](holder/index.md#bytes-performance) | Criterion measured medians on one 8 MiB decoded fixture: Windows 11 x86_64, AMD Ryzen 5 150 (6 cores/12 thr... |
| Holder | [Records](holder/index.md#records-performance) | Write-mode dispatch, 4,096 rows, one local Windows x86_64 release run (Criterion point estimates; regenerat... |
| Holder | [Values](holder/index.md#values-performance) | Criterion measured one 16,384-record JSON value through `IOBase`; each compressed case includes coding and... |
| Holder | [Call counts](holder/index.md#call-counts-performance) | One run of each operation over a 4 MiB in-memory value, wall clock beside the `IOBase` calls it makes |
| Holder | [ZIP](holder/index.md#zip-performance) | `io_zip`: positional, whole and streamed member reads and writes, restart strides and a 2,000-member archive; one containerized x86_64 Linux release run |
| Media | [gzip](media/index.md#gzip-performance) | One containerized x86_64 Linux run of the Python binding against the standard library's `gzip`, over 1,080,... |
| Media | [zlib](media/index.md#zlib-performance) | `python/benchmarks/coding.py` times `zlib-rs` beside the standard library's zlib over 1,080,000 bytes of JS... |
| Media | [zstd](media/index.md#zstd-performance) | One containerized x86_64 Linux run of the Python binding (CPython 3.11) over 1,080,000 bytes of JSON lines |
| Media | [Arrow IPC](media/index.md#arrow-ipc-performance) | Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150, rustc 1.96.1 (2026..., and the `Media` enum over its IPC variant |
| Media | [Parquet](media/index.md#parquet-performance) | Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (... |
| Media | [Parquet against PyArrow](media/index.md#streaming-against-pyarrow) | `python/benchmarks/media/parquet.py`: files PyArrow wrote, read whole and streamed both ways, and tables written both ways, from 64K to 4M rows; one containerized x86_64 Linux run |
| Media | [Parquet footer statistics](media/index.md#footer-statistics) | Local release-build spot-check of the Python and JavaScript binding boundary; fixtures differ, so rows are... |
| Media | [Avro](media/index.md#avro-performance) | Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (... |
| Media | [Avro against polars and fastavro](media/index.md#against-polars-and-fastavro) | `python/benchmarks/media/avro.py`: containers fastavro wrote in Java-default blocks, read three ways under every codec, and tables written three ways; one containerized x86_64 Linux run |
| Media | [JSON](media/index.md#json-performance), [YAML](media/index.md#yaml-performance), [TOML](media/index.md#toml-performance), [XML](media/index.md#xml-performance) | One Windows x86_64 release run of `python/benchmarks/text.py` and `node/benchmarks/text.js`: the natural-codec boundary per format; XML measured, no table yet |
| Media | [YAML placeholders](media/index.md#placeholders) | 256-entry YAML documents, feature off and on; containerized x86_64 Linux, Criterion medians with 95% intervals |
| Media | [Iceberg](media/index.md#iceberg-performance) | Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1 |
| Media | [Iceberg against PyIceberg](media/index.md#against-pyiceberg) | `python/benchmarks/media/iceberg.py`: appends, opens and four scans of a 1M-row table, unpartitioned and in eight partitions, beside PyIceberg's SQLite catalog; one containerized x86_64 Linux run |
| Types | [Cast](types/cast.md) | One compiled `ArrowCastPlan` against planning per batch, over 1, 10 and 1,000 batches of 64 rows; one con... |
| Types | [Field](types/field.md) | Rust times both consuming typed accessors, construction outside the timer; the bindings hold the cached val... |
| Types | [Scalar](types/scalar.md) | The value model's own boundaries - enum, inference, and the `Scalar`/Arrow crossings - in release builds, Windows x86_64, AMD Ryzen 5 150, rustc 1.96.1, CPython 3.12.13, Node 24.18... |

## Running every target

=== "Rust"

    ```bash
    cargo bench --bench types
    cargo bench --bench arrow
    cargo bench --bench uri
    cargo bench --bench expression
    cargo bench --bench text
    cargo bench --bench charset
    cargo bench --bench coding
    cargo bench --bench hashing
    cargo bench --bench fix
    cargo bench --bench holder --features "parquet s3"
    cargo bench --bench media --features "parquet iceberg"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python python/benchmarks/datatypes.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    python/.venv/bin/python python/benchmarks/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/holder/io.py --iterations 10000
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/media.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/media/text.py --min-time 0.05 --repeat 3
    python/.venv/bin/python python/benchmarks/media/parquet.py --repeat 7
    python/.venv/bin/python python/benchmarks/media/avro.py --repeat 5
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> python/.venv/bin/python python/benchmarks/media/s3tables.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    python/.venv/bin/python python/benchmarks/digest.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/txhash.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    python/.venv/bin/python scripts/bench_avro_baseline.py
    ```

    Build a release wheel with `maturin develop --release` before timing. The Iceberg comparison needs `pyiceberg[pyarrow,sql-sqlite]` and reports `SKIPPED` without it. The S3 Tables run needs `pyiceberg` and `boto3` installed and a table bucket of your own to write into; without either it reports `SKIPPED` and names what is missing.

=== "JavaScript"

    ```bash
    npm run --prefix node bench:types
    npm run --prefix node bench:types:defaults
    npm run --prefix node bench:holder
    npm run --prefix node bench:holder:io
    npm run --prefix node bench:coding
    npm run --prefix node bench:media
    npm run --prefix node bench:media:text
    npm run --prefix node bench:text
    npm run --prefix node bench:hashing:xxhash
    npm run --prefix node bench:hashing:txhash
    npm run --prefix node bench:fix
    ```

## What each Rust target isolates

| Target | Isolates |
| --- | --- |
| `types` | parsing, construction, validation, mutation, cached access, and Arrow schemas |
| `arrow` | `Serie` construction from an array, a batch and a stream, the reader funnel to the first batch and to the last, collapsing a stream, casting against the bare call it wraps, and structured text beside the native `Scalar` pair |
| `holder` | byte streams, listings, buffering, and foreign-filesystem boundaries |
| `charset` | the borrow an all-ASCII payload answers with, the transcode a mixed one pays for, and the three streaming doors |
| `coding` | content codings beside their standard-library baselines on the same wire |
| `media` | record round trips, text projection, Avro, Parquet, Iceberg, and pushdown |
| `text` | natural whole-value and streaming codecs, field-directed parsing, and placeholders |
| `uri` | URI parsing and component access |
| `expression` | binding, row and Arrow evaluation, and statistics pushdown |
| `fix` | registry lookup, mutation, storage, and binding crossings |
| `hashing` | digest throughput per algorithm and size, wrapper overhead, handle reads, the value feed, Arrow row digests, the time coupling beside the digest it wraps, the value's projections, instant intake, coupled columns, and the coupled holder fill |

## Rules

- Keep fixtures outside timed loops, and keep group identifiers stable so history stays comparable.
- State exactly what a result measures, and separate setup from the operation.
- Put a benchmark in its owning domain, beside the existing cases.
- Compare a trusted external implementation on the same payload and wire where one exists: stdlib, PyArrow, CPython `re`, fastavro, or raw Arrow.
- Regenerate published output; never edit a measured number by hand.
- Binding benchmarks measure the boundary. Recursive conversion and validation belong to the Rust target that implements them.
- Run on a quiet machine and compare identical release toolchains.
