# Testing

Every check runs from the repository root, which owns the Cargo workspace.

## Full pass

=== "Rust"

    ```bash
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --features "parquet iceberg" -- -D warnings
    cargo test --workspace --all-targets --features "parquet iceberg"
    cargo test --workspace --doc --features "parquet iceberg"
    ```

=== "Python"

    ```bash
    cd python
    .venv/bin/python -m maturin develop
    .venv/bin/python -m pytest
    .venv/bin/python -m mypy --strict yggdryl tests/typing_bindings.py tests/types/typing_fields.py
    ```

=== "JavaScript"

    ```bash
    npm ci --prefix node
    npm run --prefix node build:debug
    npm test --prefix node
    ```

| Pass | Toolchain |
| --- | --- |
| Default features, schema-only core | Rust 1.85 |
| `--features "parquet iceberg"`, both bindings | Rust 1.94 or newer |

## By layer

Integration targets are one file per layer under `rust/tests/`; unit tests sit beside the module they cover.

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test types
    cargo test --features "parquet iceberg" -p yggdryl --test arrow
    cargo test --features "parquet iceberg" -p yggdryl --test holder
    cargo test --features "parquet iceberg" -p yggdryl --test text
    cargo test --features "parquet iceberg" -p yggdryl --test uri
    cargo test --features "parquet iceberg" -p yggdryl --test fix
    cargo test --features "parquet iceberg" -p yggdryl --test interop
    cargo test --features "parquet iceberg" -p yggdryl --test allocations
    cargo test --features "parquet iceberg" -p yggdryl --lib types::
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::
    cargo test --features "parquet iceberg" -p yggdryl --lib holder::
    cargo test --features "parquet iceberg" -p yggdryl --lib coding::
    cargo test --features "parquet iceberg" -p yggdryl --lib media::
    cargo test --features "parquet iceberg" -p yggdryl --lib text::
    cargo test --features "parquet iceberg" -p yggdryl --lib expression::
    cargo test --features "parquet iceberg" -p yggdryl --lib xxhash::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types
    python/.venv/bin/python -m pytest python/tests/holder
    python/.venv/bin/python -m pytest python/tests/coding
    python/.venv/bin/python -m pytest python/tests/media
    python/.venv/bin/python -m pytest python/tests/text
    python/.venv/bin/python -m pytest python/tests/expression
    python/.venv/bin/python -m pytest python/tests/fix
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/types/*.test.js"
    node --test "node/tests/holder/*.test.js"
    node --test "node/tests/media/*.test.js"
    node --test "node/tests/text/*.test.js"
    node --test "node/tests/uri/*.test.js"
    node --test "node/tests/expression/*.test.js"
    node --test "node/tests/xxhash/*.test.js"
    node --test "node/tests/fix/*.test.js"
    ```

## The documentation is tested too

```bash
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

The first command compiles every `rust` block under `docs/` as a test, runs every `python` block under `python/.venv`, and every `javascript` block under node with `yggdryl` rewired to this checkout. The second builds the site strictly, which validates every link.

A block that cannot stand alone is tagged `{ .rust .ignore }`, `{ .python .ignore }`, or `{ .javascript .ignore }`; the checker reports those instead of hiding them.

## Exchange formats meet an outside implementation

```bash
python scripts/check_avro_interop.py
python scripts/check_iceberg_interop.py
python scripts/setup_spark_interop.py
python -m pytest python/tests -m spark_interop
AVRO_FUZZ_ITERATIONS=200000 cargo test -p yggdryl --lib media::avro::tests::fuzz_lite
```

| Script | Exchanges |
| --- | --- |
| `check_avro_interop.py` | Avro containers with fastavro both ways, logical types included, plus the `apache-avro` crate |
| `check_iceberg_interop.py` | Whole Iceberg tables with PyIceberg both ways, format versions 1 to 3 |
| `setup_spark_interop.py` + the `spark_interop` marker | One Hadoop warehouse shared with Apache Spark, both directions |
| `AVRO_FUZZ_ITERATIONS` | Seeded Avro mutations; the ordinary pass runs a short sweep |

A skipped half fails its driver, so a skipped exchange never reads as a pass.

## What a test looks like here

- A name states the behaviour: `a_missing_stream_reads_as_empty_rather_than_failing`.
- One behaviour per test, and an assertion message that carries the case when the test loops.
- Refusals sit beside the happy path: a cast that must fail, a root that must be non-null, a Parquet handle that must reject an outer coding, a budget that must refuse before allocating.
- A claim that "this does not scale with N" is counted, not asserted: `rust/tests/allocations.rs` drains the same work at two corpus sizes under a counting allocator and asserts equal counts.

## Layout

| Where | What |
| --- | --- |
| `rust/src/**/tests.rs` | Unit tests beside the module they cover |
| `rust/tests/<layer>.rs` + `rust/tests/<layer>/` | Integration tests, one target per layer |
| `rust/tests/allocations.rs` | The counting-allocator target |
| `python/tests/<layer>/` | Python binding and field-decorator tests |
| `node/tests/<layer>/` | JavaScript binding tests, plus `tsc --noEmit` |
| `*/benchmarks/<layer>*` | [Benchmarks](benchmarks.md) |
