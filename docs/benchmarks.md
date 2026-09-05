# Benchmarks

This page lists commands and measurement rules. Results stay beside the methods
they measure:

- [Arrow schema projection](arrow.md#what-schema-projection-costs) and
  [cached Field access](types.md#what-cached-field-access-costs);
- [plain-text records](media.md#measuring-the-boundary) and
  [record write dispatch](holder.md#canonical-record-write-signatures);
- [placeholder overhead](text.md#jinja-style-placeholders);
- [native Scalar and Arrow boundaries](text.md#scalar-and-arrow-boundary-costs);
- [natural codec boundaries](text.md#natural-codec-boundary-costs);
- [streamed byte reads](holder.md#measured-streamed-byte-behavior);
- [structured handle values](holder.md#structured-values);
- [gzip](coding.md#against-the-standard-library),
  [zlib](coding.md#against-the-standard-library), and
  [zstd](coding.md#against-the-standard-library);
- [IPC](media.md#against-pyarrow), [Parquet](media.md#against-pyarrow),
  [Avro](media.md#against-fastavro-and-pyiceberg-on-identical-bytes), and their
  opened-session metadata sections;
- [expressions](expression.md#against-the-raw-arrow-kernels),
  [Arrow filesystem](holder.md#performance-gates), and
  [page buffering](holder.md#what-the-cache-buys-and-what-it-costs);
- [digests](xxhash.md#benchmarks).

## Running the benchmarks

=== "Rust"

    ```console
    cargo bench --bench types
    cargo bench --bench fix
    cargo bench --bench holder --features "parquet"
    cargo bench --bench coding
    cargo bench --bench media --features "parquet iceberg"
    cargo bench --bench text
    cargo bench --bench uri
    cargo bench --bench expression
    cargo bench --bench xxhash
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python benchmarks/types.py --iterations 10000
    .venv/Scripts/python benchmarks/types/arrow.py --iterations 10000
    .venv/Scripts/python benchmarks/holder.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/coding.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/media.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/text.py --min-time 0.2 --repeat 7
    .venv/Scripts/python benchmarks/uri.py --iterations 2000
    .venv/Scripts/python benchmarks/xxhash.py --min-time 0.2 --repeat 5
    .venv/Scripts/python benchmarks/fix.py --iterations 2000
    ```

    Build a release wheel with `maturin build --release` before timing.

=== "JavaScript"

    ```console
    npm run --prefix node bench:types
    npm run --prefix node bench:holder
    npm run --prefix node bench:coding
    npm run --prefix node bench:media
    npm run --prefix node bench:text
    npm run --prefix node bench:xxhash
    npm run --prefix node bench:fix
    ```

Run on a quiet machine and compare identical release toolchains. Published
runs use one containerized x86_64 Linux host (Intel Xeon @ 2.10 GHz, 4 cores,
16 GiB; rustc 1.94.1 with thin LTO; CPython 3.11.15, PyArrow 25.0.1; Node
22.22.2). Regenerate numbers on the deployment host before drawing conclusions.

## What each target isolates

| target | isolates |
| --- | --- |
| `types` | parsing, construction, validation, mutation, cached access, and Arrow schemas |
| `holder` | byte streams, listings, buffering, and foreign-filesystem boundaries |
| `coding` | content codings beside their standard-library baselines on the same wire |
| `media` | record round trips, text projection, Avro, Parquet, Iceberg, and pushdown |
| `text` | natural whole-value and streaming codecs, field-directed parsing, and placeholders |
| `uri` | URI parsing and component access |
| `expression` | binding, row and Arrow evaluation, and statistics pushdown |
| `fix` | registry lookup, mutation, storage, and binding crossings |
| `xxhash` | digest throughput per algorithm and size, wrapper overhead over the protocol crate, constant-memory handle reads, the canonical value feed, and Arrow row digests |

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
