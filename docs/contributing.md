# Contributing

Smoke what you changed while you are changing it, then push and let CI run the matrix. The loop is the narrowest command that executes the new code - one test target, one filter, a debug build - and it is seconds; the commands below are what to run once before pushing. `AGENTS.md` is the normative version of everything here.

=== "Rust"

    ```bash
    cargo test -p yggdryl --test <theme> <filter>   # the loop, while you write
    cargo fmt --all
    cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings
    cargo test -p yggdryl --all-targets
    cargo test -p yggdryl --doc
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
    node --test node/tests/<area>/<file>.test.js   # the loop, while you write
    npm run --prefix node test:package:debug
    npm test --prefix node
    ```

CI runs the rest on the pushed branch: both feature lanes, the 1.85 and 1.94 MSRVs, the exchanges with MinIO, Azurite, fake-gcs-server, `zipfile`, fastavro, PyIceberg and Spark, both pyarrow legs, and every documentation example in three languages. Two checks have no job and stay local - `python scripts/generate_charset_tables.py --check` and `python scripts/check_charset_interop.py` - as does any benchmark whose number a page states.

## Where things go

| Source | Tests | Docs tab |
| --- | --- | --- |
| `rust/src/datatype.rs`, `field.rs`, `scalar.rs`, `cast.rs`, `typed.rs`, `protocol.rs`, `metadata.rs` and one root file per type - `string.rs`, `bytes.rs`, `integer.rs`, `decimal.rs` with `int256.rs`, the five temporal files with `temporal.rs`, `timezone.rs`, `uuid.rs`, `geospatial.rs`, `code.rs` with the twelve codes including `figi_code.rs`, `mime_type/datatype.rs`, `media_type/datatype.rs` | `rust/tests/types/`, `rust/src/metadata/tests.rs` | [Types](types/index.md) |
| `rust/src/iobase.rs`, `rust/src/iobase/`, the `rust/src/io*.rs` roles, `rust/src/holder/`, and one root folder per backend: `rust/src/local/`, `fs/`, `zip/`, `object/` | `rust/tests/holder/`, `rust/tests/iobase/`, `rust/src/holder/buffered/tests.rs`, `rust/src/{local,zip}/tests.rs`, `rust/src/object/tests/` | [Holder](holder/index.md) |
| `rust/src/codec.rs`, `rust/src/coding/`, `rust/src/gzip.rs`, `zlib.rs`, `zstd.rs` | `rust/tests/coding/` | [Coding](coding/index.md) |
| `rust/src/charset.rs`, `rust/src/charset/`, `rust/src/utf8.rs`, `ascii.rs`, `cp1252.rs` | `rust/tests/charset/`, `rust/src/charset/reader/tests.rs`, `rust/src/utf8/tests.rs`, `rust/tests/interop/charset.rs` | [Charset](charset/index.md) |
| `rust/src/media_type.rs`, `mime_type.rs`, `rust/src/media/`, and one root folder per medium: `rust/src/ipc/`, `parquet/`, `avro/`, `iceberg/`, `text/` | `rust/tests/media/`, `rust/tests/interop/`, `rust/src/{ipc,parquet,avro,iceberg}/tests.rs`, `rust/src/media/*/tests.rs` | [Media](media/index.md) |
| `rust/src/json/`, `toml/`, `yaml/` over the codec machinery in `rust/src/text/` | `rust/tests/text/`, `rust/src/toml/wire/tests.rs` | [Structured documents](media/structured.md) |
| `rust/src/uri/` | `rust/tests/uri/`, `rust/src/uri/pattern/tests.rs` | [URI](uri/index.md) |
| `rust/src/arrow/` | `rust/tests/arrow/` | [Arrow](arrow/index.md) |
| `rust/src/expression/` | `rust/tests/expression/`, `rust/src/expression/eval/tests.rs` | [Expression](expression/index.md) |
| `rust/src/graph/` | `rust/tests/graph/` | [Graph](graph.md) |
| `rust/src/digest.rs`, `rust/src/hashing/`, `rust/src/xxhash/`, `rust/src/txhash/` | `rust/tests/hashing/`, `rust/src/xxhash/tests.rs`, `rust/src/txhash/tests.rs` | [Hashing](hashing.md) |
| `rust/src/fix/` | `rust/tests/fix/`, `rust/src/fix/tests.rs` | [FIX](fix/index.md) |

Each shared trait, enum, value or type owns one root `rust/src/<name>.rs`; each implementation owns a root folder or file of its own name; a parent folder holds only what its implementations share. The Python package is laid out the same way and reimplements nothing: one module per type at the package root, one module or package per implementation - `yggdryl.avro`, `yggdryl.iceberg`, `yggdryl.json`, `yggdryl.gzip`, `yggdryl.xxhash`, `yggdryl.txhash` - and a package only where its implementations share something, `media/`, `text/`, `coding/`, `holder/`, `charset/`, `enums/`. Every type is re-exported from `yggdryl` itself, as the crate re-exports each of its root files. JavaScript keeps `hashing.xxhash` and `hashing.txhash` over the same root `xxhash/` and `txhash/`. Runnable examples live in the documentation, never in an `examples/` directory.

## What a change must satisfy

- **One concept, one home.** Two modules that need one behaviour share it from the module below them.
- **A trait says what; an enum says which.** A new backend or encoding implements the trait and adds the variant; no parallel dispatch.
- **Errors state expected, got, and where.** `expected int64, got utf8` with a path or byte offset.
- **Wide in, typed through.** A boundary accepts every documented spelling of an input, resolves it once into a `DataType`, `Field`, or `Scalar`, and the interior works on the resolved type - no re-parsing, re-validating, or branching on a string per row.
- **Refusals are features.** A silent widening cast, a nullable root, or a double-compressed handle is worse than an error; test the refusal.
- **Laziness is a contract.** Constructing a handle touches nothing, reading something absent yields nothing, writing creates.
- **Never pre-check.** No `exists` before a read, no `mkdir` before a write; act, branch on the typed absence or conflict, repair once, retry once. A caller's own `exists` or `is_dir` stays public.
- **Every listing is an iterator.** `ls`, `glob`, and the predicate listings yield `Result` items and fuse at the first failure.

## Documentation is part of the change

- One page per family, one H1, one sentence, then the [page skeleton](architecture.md): Contract, Use, feature sections, Edges, Commands, Performance.
- One page per type under its family's folder, in the order its core file is written: Contract, DataType, Field, Scalar, Arrow storage, features, Edges, Commands.
- One folder per media scheme: `index.md`, `read.md`, `write.md`, then a page per feature. A read or a write page shows native scalars first and Arrow batches second.
- Every example appears in Rust, Python, and JavaScript unless it carries the "Rust only" line, and every block runs under `python scripts/check_docs_examples.py`.
- A benchmark table lives on the page that owns the measured method, names host and toolchain, and ends with its regenerate command.
- Adding or renaming a page updates `mkdocs.yml` and every link to it in the same change; `mkdocs build --strict` fails otherwise.

## Bindings

Python and JavaScript are views. They may infer inputs at the boundary (a string where a datatype expression is expected, a path-like where a URL is expected) and must then call the core. A native error message crosses unchanged, and each binding stays idiomatic: mapping dunders and keyword arguments in Python, `from` constructors and `Map` protocols in JavaScript.
