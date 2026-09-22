# Testing

Every check runs from the repository root, which owns the Cargo workspace.

## Full pass

=== "Rust"

    ```bash
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo test --workspace --all-targets --all-features
    cargo test --workspace --doc --all-features
    ```

=== "Python"

    ```bash
    cd python
    .venv/bin/python -m maturin develop
    .venv/bin/python -m pytest
    .venv/bin/python -m mypy --strict yggdryl tests/typing_bindings.py tests/typing_fields.py
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
| `--all-features`, both bindings | Rust 1.94 or newer |

## By entry

`rust/tests/` mirrors `rust/src/` file for file, and one target declares each
top-level entry: a source folder is declared by `rust/tests/<folder>.rs`, and
the files the crate root holds by `rust/tests/root.rs`. Five targets stand
outside the mirror because what they pin is a cost or an exchange rather than a
file. `--all-features` is what turns `internals` on, and with it a target runs
everything it declares.

=== "Rust"

    ```bash
    cargo test -p yggdryl --all-features --test root
    cargo test -p yggdryl --all-features --test arrow
    cargo test -p yggdryl --all-features --test avro
    cargo test -p yggdryl --all-features --test charset
    cargo test -p yggdryl --all-features --test coding
    cargo test -p yggdryl --all-features --test expression
    cargo test -p yggdryl --all-features --test fix
    cargo test -p yggdryl --all-features --test fs
    cargo test -p yggdryl --all-features --test graph
    cargo test -p yggdryl --all-features --test hashing
    cargo test -p yggdryl --all-features --test holder
    cargo test -p yggdryl --all-features --test iceberg
    cargo test -p yggdryl --all-features --test iobase
    cargo test -p yggdryl --all-features --test ipc
    cargo test -p yggdryl --all-features --test json
    cargo test -p yggdryl --all-features --test local
    cargo test -p yggdryl --all-features --test media
    cargo test -p yggdryl --all-features --test media_type
    cargo test -p yggdryl --all-features --test metadata
    cargo test -p yggdryl --all-features --test mime_type
    cargo test -p yggdryl --all-features --test parquet
    cargo test -p yggdryl --all-features --test s3
    cargo test -p yggdryl --all-features --test text
    cargo test -p yggdryl --all-features --test toml
    cargo test -p yggdryl --all-features --test txhash
    cargo test -p yggdryl --all-features --test uri
    cargo test -p yggdryl --all-features --test value
    cargo test -p yggdryl --all-features --test xxhash
    cargo test -p yggdryl --all-features --test yaml
    cargo test -p yggdryl --all-features --test zip
    cargo test -p yggdryl --all-features --test allocations     # the counting allocator
    cargo test -p yggdryl --all-features --test iobase_calls    # the pinned `IOBase` call counts
    cargo test -p yggdryl --all-features --test benchmark_mode  # every benchmark at its smoke corpus
    cargo test -p yggdryl --all-features --test docs_index      # the landing-page example
    cargo test -p yggdryl --all-features --test interop         # the exchanges with an outside implementation
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py   # one source file
    python/.venv/bin/python -m pytest python/tests/text               # one source folder
    python/.venv/bin/python -m pytest python/tests                    # the whole binding
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/datatype.test.js      # one source file
    node --test "node/tests/text/*.test.js"      # one source folder
    npm test --prefix node                       # the whole binding, and `tsc --noEmit`
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
python scripts/check_object_interop.py
python scripts/check_azure_interop.py
python scripts/check_gcs_interop.py
python scripts/check_iceberg_interop.py
python scripts/setup_spark_interop.py
python -m pytest python/tests -m spark_interop
AVRO_FUZZ_ITERATIONS=200000 cargo test -p yggdryl --features internals --test avro -- mod_::fuzz_lite
```

| Script | Exchanges |
| --- | --- |
| `check_avro_interop.py` | Avro containers with fastavro both ways, logical types included, plus the `apache-avro` crate |
| `check_object_interop.py` | S3 objects with boto3 both ways against MinIO: awkward keys, ranged reads, and a multipart upload verified from the outside |
| `check_azure_interop.py` | Blobs with azure-storage-blob both ways against Azurite, which recomputes the Shared Key signature itself: awkward names, ranged reads, and a block-list upload verified from the outside |
| `check_gcs_interop.py` | Objects with google-cloud-storage both ways against fake-gcs-server: names escaped into one path segment, and a resumable upload verified from the outside. The emulator accepts any token, so this proves the dialect and not the identity |
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
| `rust/tests/<entry>.rs` + `rust/tests/<entry>/` | One target per top-level source entry, declaring the mirror of each of its files |
| `rust/tests/root/<name>.rs` | The file `rust/src/<name>.rs` holds, pinned; a folder's own `mod.rs` is pinned by `mod_.rs` |
| `rust/tests/support/` | Fixtures several targets declare - a counting allocator, an in-process S3 |
| `rust/tests/allocations.rs`, `iobase_calls.rs`, `benchmark_mode.rs`, `docs_index.rs`, `interop/` | What is pinned as a cost or an exchange rather than as a file |
| `python/tests/**/test_<name>.py` | The mirror of `python/yggdryl/` and `python/src/`, which share one shape |
| `node/tests/**/<name>.test.js` | The mirror of `node/src/` and the JavaScript beside it, plus `tsc --noEmit` over the `.types.ts` files |
| `*/benchmarks/<theme>*` | [Benchmarks](benchmarks.md), which stay grouped by a caller's vocabulary |
