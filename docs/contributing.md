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
| `rust/src/types/` | `rust/tests/types/` | [Types](types/index.md) |
| `rust/src/holder/`, `rust/src/iobase*` | `rust/tests/holder/` | [Holder](holder/index.md) |
| `rust/src/coding/` | `rust/src/coding/tests.rs` | [Coding](coding/index.md) |
| `rust/src/charset.rs`, `rust/src/charset/` | `rust/src/charset/tests.rs`, `rust/tests/charset/` | [Charset](charset/index.md) |
| `rust/src/media/` | `rust/src/media/tests.rs`, `rust/tests/interop/` | [Media](media/index.md) |
| `rust/src/text/` | `rust/tests/text/` | [Structured documents](media/structured.md) |
| `rust/src/uri/` | `rust/tests/uri/` | [URI](uri/index.md) |
| `rust/src/arrow/` | `rust/tests/arrow/` | [Arrow](arrow/index.md) |
| `rust/src/expression/` | `rust/src/expression/tests.rs` | [Expression](expression/index.md) |
| `rust/src/graph/` | `rust/tests/graph/` | [Graph](graph.md) |
| `rust/src/hashing/` | `rust/src/hashing/xxhash/tests.rs`, `rust/src/hashing/txhash/tests.rs` | [Hashing](hashing.md) |
| `rust/src/fix/` | `rust/tests/fix/` | [FIX](fix/index.md) |

Each shared trait, enum, or value owns one root `rust/src/<name>.rs`; each layer owns one folder. Python and Node mirror the layer names without reimplementing core behavior: `yggdryl.hashing.xxhash` and `yggdryl.hashing.txhash` in Python, `hashing.xxhash` and `hashing.txhash` in JavaScript. Runnable examples live in the documentation, never in an `examples/` directory.

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
- Every example appears in Rust, Python, and JavaScript unless it carries the "Rust only" line, and every block runs under `python scripts/check_docs_examples.py`.
- A benchmark table lives on the page that owns the measured method, names host and toolchain, and ends with its regenerate command.
- Adding or renaming a page updates `mkdocs.yml` and every link to it in the same change; `mkdocs build --strict` fails otherwise.

## Bindings

Python and JavaScript are views. They may infer inputs at the boundary (a string where a datatype expression is expected, a path-like where a URL is expected) and must then call the core. A native error message crosses unchanged, and each binding stays idiomatic: mapping dunders and keyword arguments in Python, `from` constructors and `Map` protocols in JavaScript.
