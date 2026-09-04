# Benchmarks

This page lists commands and measurement rules. Results stay beside the methods
they measure:

- [Arrow schema projection](arrow.md#what-schema-projection-costs) and
  [cached Field access](field.md#what-cached-field-access-costs);
- [plain-text records](text.md#measuring-the-boundary) and
  [record write dispatch](io.md#canonical-record-write-signatures);
- [placeholder overhead](text.md#jinja-style-placeholders);
- [native Scalar and Arrow boundaries](text.md#scalar-and-arrow-boundary-costs);
- [natural codec boundaries](text.md#natural-codec-boundary-costs);
- [streamed byte reads](io.md#measured-streamed-byte-behavior);
- [structured handle values](io.md#structured-values);
- [gzip](gzip.md#against-the-standard-library),
  [zlib](zlib.md#against-the-standard-library), and
  [zstd](zstd.md#against-the-standard-library);
- [IPC](ipc.md#against-pyarrow), [Parquet](parquet.md#against-pyarrow),
  [Avro](avro.md#against-fastavro-and-pyiceberg-on-identical-bytes), and their
  opened-session metadata sections;
- [expressions](expression.md#against-the-raw-arrow-kernels),
  [Arrow filesystem](arrowfs.md#what-the-wrapper-costs), and
  [page buffering](buffered.md#what-the-cache-buys-and-what-it-costs);
- [digests](xxhash.md#benchmarks).

## Running the benchmarks

=== "Rust"

    ```console
    cargo bench --bench datatype
    cargo bench --bench field
    cargo bench --bench fix
    cargo bench --bench enums
    cargo bench --bench uri
    cargo bench --bench text
    cargo bench --bench json
    cargo bench --bench toml
    cargo bench --bench yaml
    cargo bench --bench avro
    cargo bench --bench xxhash
    cargo bench --bench io --features "parquet"
    cargo bench --bench io --features "parquet" -- io_pstream --noplot
    cargo bench --bench io --features "parquet" -- io_value --noplot
    cargo bench --bench io --features "parquet" -- io_buffered
    cargo bench --bench arrowfs --features "parquet"
    cargo bench --bench iceberg --features "parquet iceberg"
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python benchmarks/fields.py --iterations 10000
    .venv/Scripts/python benchmarks/fields_arrow.py --iterations 10000
    .venv/Scripts/python benchmarks/values.py --iterations 10000
    .venv/Scripts/python benchmarks/records_io.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/text.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/codecs.py --iterations 10000
    .venv/Scripts/python benchmarks/compression.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/digests.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/iceberg.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/arrowfs.py --min-time 0.2 --repeat 7
    ```

    Build a release wheel with `maturin build --release` before timing.

=== "JavaScript"

    ```console
    npm run --prefix node bench:schema
    npm run --prefix node bench:codec
    npm run --prefix node bench:defaults
    npm run --prefix node bench:io
    npm run --prefix node bench:text
    npm run --prefix node bench:records
    npm run --prefix node bench:arrowfs
    npm run --prefix node bench:xxhash
    ```

Run on a quiet machine and compare identical release toolchains. Published
runs use one containerized x86_64 Linux host (Intel Xeon @ 2.10 GHz, 4 cores,
16 GiB; rustc 1.94.1 with thin LTO; CPython 3.11.15, PyArrow 25.0.1; Node
22.22.2). Regenerate numbers on the deployment host before drawing conclusions.

## What each target isolates

| target | isolates |
| --- | --- |
| `datatype` | parsing, defaults, compatibility, Arrow, temporal and decimal construction |
| `field` | construction, mutation, typed views, cached field access, Arrow schemas |
| `enums`, `uri` | media inference and URI parsing/component access |
| `text` | Scalar construction, natural format inference, Field-directed parsing, placeholders, line projection and record shape |
| `json`, `yaml`, `toml` | natural whole-value/streaming encode and decode, including exact Field recovery |
| `avro` | type families, block sizing, projection, resolution plans and varints |
| `io` | byte streams, structured values, record round trips, dimensions, opened caches, write dispatch and projection pushdown |
| `text` (Python) | generic plain-text record projection against equivalent CPython `re` plus PyArrow |
| `text` (JavaScript) | generic plain-text Arrow and object records at the copied-IPC boundary |
| `arrowfs` | foreign filesystem boundary beside the native handle |
| `iceberg` | planning, metadata, manifests, partitioning, merge, compaction and commits |
| `compression` (Python) | byte codings beside the standard library on the same wire |
| `xxhash` | digest throughput per algorithm and size, wrapper overhead over the protocol crate, constant-memory handle reads, the canonical value feed, and Arrow row digests |
| `digests` (Python) | the digest boundary beside C `libxxhash` on the same payload, with the conversion cost of each buffer shape visible |

## Rules

- Keep fixtures outside timed loops.
- Keep group identifiers stable so history remains comparable.
- State exactly what each result measures; separate setup from the operation.
- Put a benchmark in its owning domain, beside existing cases.
- Compare with a trusted external implementation on the same payload and wire
  whenever one exists: stdlib, PyArrow, CPython `re`, fastavro, or raw Arrow.
- Regenerate published output; never edit measured numbers by hand.
- Binding benchmarks measure the boundary. Recursive conversion and validation
  belong to the Rust target that implements them.
