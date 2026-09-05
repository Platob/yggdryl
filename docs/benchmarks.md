# Benchmarks

Results live beside the method they measure. Each page's Performance section names its host and toolchain and ends with the command that regenerates it.

## Where the numbers are

| Tab | Page | Measures |
| --- | --- | --- |
| Arrow | [Schema](arrow/schema.md) | The `types` Criterion target times only the three Struct-root methods over one nested fixture built outside... |
| Coding | [gzip](coding/gzip.md) | One containerized x86_64 Linux run of the Python binding against the standard library's `gzip`, over 1,080,... |
| Coding | [zlib](coding/zlib.md) | `python/benchmarks/coding.py` times `zlib-rs` beside the standard library's zlib over 1,080,000 bytes of JS... |
| Coding | [zstd](coding/zstd.md) | One containerized x86_64 Linux run of the Python binding (CPython 3.11) over 1,080,000 bytes of JSON lines |
| Expression | [Evaluate](expression/evaluate.md) | `benchmarks/expression.rs` writes each predicate by hand against `arrow-ord` / `arrow-select`, and `express... |
| FIX | [FIX](fix/index.md) | Field setters and the `FixId` codec: one local Windows x86_64 release run of the Criterion target, point es... |
| FIX | [Message](fix/message.md) | Binding rows only; the Criterion target carries no `FixMsg` case |
| FIX | [Registry](fix/registry.md) | One local Windows x86_64 release run of the Criterion target, point estimates, over the tracked seed of 34... |
| FIX | [Store](fix/store.md) | One local Windows x86_64 release run of the Criterion target, point estimates, over the tracked seed of 34... |
| Holder | [Buffered](holder/backends/buffered.md) | `io_buffered` runs three workloads over one 16 MiB fixture and every shipped handle: one containerized x86_... |
| Holder | [Filesystems](holder/backends/filesystems.md) | The benchmark times the wrapper against direct PyArrow, local, or native local operations; gates rather than published medians |
| Holder | [Bytes](holder/iobase/bytes.md) | Criterion measured medians on one 8 MiB decoded fixture: Windows 11 x86_64, AMD Ryzen 5 150 (6 cores/12 thr... |
| Holder | [Records](holder/iobase/records.md) | Write-mode dispatch, 4,096 rows, one local Windows x86_64 release run (Criterion point estimates; regenerat... |
| Holder | [Values](holder/iobase/values.md) | Criterion measured one 16,384-record JSON value through `IOBase`; each compressed case includes coding and... |
| Media | [Apache Avro](media/avro.md) | Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (... |
| Media | [Iceberg](media/iceberg/index.md) | Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1 |
| Media | [Media](media/index.md) | `io_write_stateful/media_ipc` drives the enum over its IPC variant with a 4,096-row, four-column fixture |
| Media | [Arrow IPC](media/ipc.md) | Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150, rustc 1.96.1 (2026... |
| Media | [Parquet footer](media/parquet-footer.md) | Local release-build spot-check of the Python and JavaScript binding boundary; fixtures differ, so rows are... |
| Media | [Apache Parquet](media/parquet.md) | Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (... |
| Text | [Structured text](text/index.md) | Windows x86_64 release smoke runs, Criterion group `value` in `--bench types` and `python/benchmarks/types/... |
| Text | [Placeholders](text/placeholders.md) | 256-entry YAML documents, feature off and on; containerized x86_64 Linux, Criterion medians with 95% intervals |
| Types | [Field](types/field.md) | Rust times both consuming typed accessors, construction outside the timer; the bindings hold the cached val... |
| Types | [Scalar](types/scalar.md) | Enum boundary in release builds, Windows x86_64, AMD Ryzen 5 150, rustc 1.96.1, CPython 3.12.13, Node 24.18... |
| xxHash | [Handles](xxhash/handles.md) | One containerized x86_64 Linux run (benchmarks): Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 relea... |
| xxHash | [xxHash](xxhash/index.md) | `rust/benchmarks/xxhash.rs`, `python/benchmarks/xxhash.py`, and `node/benchmarks/xxhash.js` measure one pro... |
| xxHash | [Values](xxhash/values.md) | One containerized x86_64 Linux run (Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 release with thin... |

## Running every target

=== "Rust"

    ```bash
    cargo bench --bench types
    cargo bench --bench uri
    cargo bench --bench expression
    cargo bench --bench text
    cargo bench --bench coding
    cargo bench --bench xxhash
    cargo bench --bench fix
    cargo bench --bench holder --features "parquet"
    cargo bench --bench media --features "parquet iceberg"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/arrow.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/holder/io.py --iterations 10000
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/media.py --min-time 0.2 --repeat 7
    python/.venv/bin/python python/benchmarks/media/text.py --min-time 0.05 --repeat 3
    python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/text.py --iterations 10000
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    python/.venv/bin/python python/benchmarks/xxhash.py --min-time 0.2 --repeat 5
    python/.venv/bin/python python/benchmarks/fix.py --iterations 2000
    python/.venv/bin/python scripts/bench_avro_baseline.py
    ```

    Build a release wheel with `maturin develop --release` before timing.

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
    npm run --prefix node bench:xxhash
    npm run --prefix node bench:fix
    ```

## What each Rust target isolates

| Target | Isolates |
| --- | --- |
| `types` | parsing, construction, validation, mutation, cached access, and Arrow schemas |
| `holder` | byte streams, listings, buffering, and foreign-filesystem boundaries |
| `coding` | content codings beside their standard-library baselines on the same wire |
| `media` | record round trips, text projection, Avro, Parquet, Iceberg, and pushdown |
| `text` | natural whole-value and streaming codecs, field-directed parsing, and placeholders |
| `uri` | URI parsing and component access |
| `expression` | binding, row and Arrow evaluation, and statistics pushdown |
| `fix` | registry lookup, mutation, storage, and binding crossings |
| `xxhash` | digest throughput per algorithm and size, wrapper overhead, handle reads, the value feed, and Arrow row digests |

## Rules

- Keep fixtures outside timed loops, and keep group identifiers stable so history stays comparable.
- State exactly what a result measures, and separate setup from the operation.
- Put a benchmark in its owning domain, beside the existing cases.
- Compare a trusted external implementation on the same payload and wire where one exists: stdlib, PyArrow, CPython `re`, fastavro, or raw Arrow.
- Regenerate published output; never edit a measured number by hand.
- Binding benchmarks measure the boundary. Recursive conversion and validation belong to the Rust target that implements them.
- Run on a quiet machine and compare identical release toolchains.
