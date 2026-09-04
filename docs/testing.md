# Testing

Every check runs from the repository root, which owns the Cargo workspace.

=== "Rust"

    ```console
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --features "parquet iceberg" -- -D warnings
    cargo test --workspace --all-targets --features "parquet iceberg"
    cargo test --workspace --doc --features "parquet iceberg"
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python -m maturin develop
    .venv/Scripts/python -m pytest
    .venv/Scripts/python -m mypy --strict yggdryl tests/typing_bindings.py tests/types/typing_fields.py
    ```

=== "JavaScript"

    ```console
    npm ci --prefix node
    npm run --prefix node build:debug
    npm test --prefix node
    ```

Run the Rust pass twice: once with default features and once with `--features "parquet iceberg"`.
Both are non-default core features. Default and schema-only core checks use
Rust 1.85; the Iceberg pass and both bindings use Rust 1.94 or newer.

## The documentation is tested too

```console
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

The first command compiles every fenced `rust` block under `docs/` as a test, runs every `python`
block under `python/.venv`, and runs every `javascript` block under node with `yggdryl`
rewired to this repository. The second builds the site strictly, which validates every link.

An example that cannot stand alone is tagged `rust,ignore`, `python,ignore`, or
`javascript,ignore`; the checker reports those rather than hiding them.

## Run tests by domain

```console
cargo test --test datatype
cargo test --test enums
cargo test --test field
cargo test --test uri
cargo test --test text
cargo test --test json
cargo test --test toml
cargo test --test yaml
cargo test --features parquet --test batch_cast
cargo test --features parquet --test default_scalar
cargo test --features parquet --test value_bounds
```

Unit tests live beside the code they cover, so a module is exercised through its own path:

```console
cargo test --features "parquet iceberg" -p yggdryl --lib iobase::
cargo test --features "parquet iceberg" -p yggdryl --lib holder::
cargo test --features "parquet iceberg" -p yggdryl --lib types::
cargo test --features "parquet iceberg" -p yggdryl --lib types::cast
cargo test --features "parquet iceberg" -p yggdryl --lib media::ipc::
cargo test --features "parquet iceberg" -p yggdryl --lib media::parquet::
cargo test --features "parquet iceberg" -p yggdryl --lib media::avro::
cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::
cargo test --features "parquet iceberg" -p yggdryl --lib text::codec
```

## Exchange formats meet an outside implementation

```console
python scripts/check_avro_interop.py
python scripts/check_iceberg_interop.py
```

The first exchanges Avro object containers with fastavro in both directions -
logical types included - and round-trips the same container through the
`apache-avro` crate in a scratch project; both are checking tools of the
script, never dependencies of the crate. The second exchanges whole Iceberg
tables with PyIceberg in both directions, covering format versions 1, 2, and
3 where PyIceberg can write them. Each driver fails when a half was skipped,
so a skipped exchange can never read as a pass.

The Iceberg module is also verified against Apache Spark, the format's
reference implementation, over one shared Hadoop warehouse in both
directions:

```console
python scripts/setup_spark_interop.py
python -m pytest python/tests -m spark_interop
```

The suite carries its own pytest marker, is deselected from the default run,
and skips itself - naming what is missing - when Java, `pyspark`, or the
`iceberg-spark-runtime` jar is absent; the setup script provisions the latter
two, and a dedicated CI job runs exactly this suite.

The Avro fuzz sweeps run seeded mutations in the ordinary test pass; a longer
sweep scales the same tests with `AVRO_FUZZ_ITERATIONS`:

```console
AVRO_FUZZ_ITERATIONS=200000 cargo test -p yggdryl --lib media::avro::tests::fuzz_lite
```

## What a test looks like here

A test name states the behaviour: `a_missing_stream_reads_as_empty_rather_than_failing` says what
the contract is, `test_streamed_batch_read_2` says nothing. One behaviour per test, and an assertion
message that carries the case when the test loops.

Adversarial cases sit beside the happy path. The suites that matter most are the ones pinning a
*refusal*: a cast that must fail rather than widen, a schema root that must be non-null, a Parquet
handle that must reject an outer content coding, an allocation budget that must reject before
allocating.

## A claim about allocations is counted, not asserted

The text-record surface rests on records being *views* into a bounded window: reading a million of
them must cost the same allocations as reading a thousand, because none of them belongs to a
record. A comment saying so is not evidence, and neither is a timing - a per-record allocation is
cheap enough to hide inside I/O.

So `rust/tests/allocations.rs` counts them. A pass-through global allocator increments a counter
while armed, and each case drains the same work at two corpus sizes and asserts the two counts are
*equal*: reading records, reading them with a pattern and its captures, and writing them. It is its
own test target because a program has exactly one global allocator.

The shape generalizes. Any claim of the form "this does not scale with N" belongs in that file,
measured the same way, rather than in prose.

## Layout

| Where | What |
| --- | --- |
| `rust/src/**/tests.rs` | Unit tests, beside the module they cover |
| `rust/tests/*.rs` | Integration tests, one file per domain |
| `rust/tests/allocations.rs` | The counting-allocator target: what the hot path must *not* allocate per record |
| `python/tests/` | Python binding and field-decorator tests |
| `node/tests/` | JavaScript binding tests, plus `tsc --noEmit` |
| `*/benchmarks/` | [Benchmarks](benchmarks.md) |
