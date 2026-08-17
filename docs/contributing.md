# Contributing

Run the checks for whatever you changed, and make code, tests, and documentation agree before handing off.

=== "Rust"

    ```console
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --features "parquet iceberg" -- -D warnings
    cargo test --workspace --all-targets --features "parquet iceberg"
    python scripts/check_docs_examples.py
    python -m mkdocs build --strict
    ```

=== "Python"

    ```console
    cd python
    .venv/Scripts/python -m maturin develop
    .venv/Scripts/python -m pytest
    .venv/Scripts/python -m mypy --strict yggdryl tests/typing_bindings.py tests/typing_records.py
    ```

=== "JavaScript"

    ```console
    npm ci --prefix node
    npm run --prefix node build:debug
    npm test --prefix node
    ```

Start with the repository-wide `AGENTS.md`, which is the normative version of everything below.

## Where things go

```text
rust/src/enums/      shared vocabulary     -> rust/tests/enums.rs       -> docs/core/enums.md
rust/src/datatype/   logical types         -> rust/tests/datatype/      -> docs/core/datatype.md
rust/src/field/      schema and casting    -> rust/tests/field/         -> docs/core/field.md
rust/src/arrow/      scalars, projection   -> rust/tests/*.rs           -> docs/core/arrow.md
rust/src/io/         IOBase, Buffer, roles -> rust/src/io/tests.rs      -> docs/core/io.md
rust/src/generic/    enums and Value       -> rust/src/generic/**       -> docs/core/generic.md
rust/src/local/      Path, Folder, File    -> rust/src/local/tests.rs   -> docs/core/local.md
rust/src/gzip|zlib|zstd/ content codings   -> beside each module        -> docs/core/<name>.md
rust/src/ipc|parquet|iceberg/ encodings    -> beside each module        -> docs/core/<name>.md
rust/src/uri.rs      identifiers           -> rust/tests/uri.rs         -> docs/core/uri.md
rust/src/text/       Value and text codecs -> rust/tests/text/          -> docs/core/text.md
rust/src/json|yaml|toml/ formats           -> rust/tests/<format>.rs    -> docs/core/<format>.md

python/src/*.rs      PyO3 views of core values
python/yggdryl/      Python-only facades: json/toml/yaml I/O and records
node/src/*.rs        Node-API views of core values
node/*.js            JavaScript facade: loader, defaults, fields, values
```

`rust/src/lib.rs` holds exports and shared error plumbing, not a second implementation home.
Benchmarks live in each member's `benchmarks/`; there are no `examples/` directories, because every
runnable example lives in the documentation where all three languages appear side by side.

## What a change must satisfy

**One concept, one home.** A behaviour belongs to exactly one module. If two modules need it, it
moves down to the one they share rather than being copied.

**A trait says what; an enum says which.** Adding a backend or an encoding means implementing the
trait and adding the variant to the [generic enum](core/generic.md), never adding a parallel
dispatch.

**Errors state expected, got, and where.** `expected int64, got utf8` with a path or byte offset. A
message that only names a rule is incomplete.

**Refusals are features.** A cast that widens silently, a schema that accepts a nullable root, or a
handle that double-compresses is worse than an error. Test the refusal.

**Laziness is a contract.** Constructing a handle touches nothing; reading something absent yields
nothing; writing creates. New backends inherit this by implementing the
[role traits](core/local.md).

## Documentation is part of the change

Every page mirrors one module folder, opens with one H1 and exactly one sentence, and shows every
example in all three languages unless the module carries the Rust-only note. Every block is executed
by `python scripts/check_docs_examples.py`, so a renamed method breaks the docs exactly as it breaks
the code.

That same run generates the notebooks under `docs/notebooks/` and the `Notebooks` section it links
them from, between the two comment markers at the foot of the page. Both are outputs: change the
blocks, not the notebook, and never hand-write inside the markers.

When you add or rename a page, update the `mkdocs.yml` navigation and every link to it in the same
change; `mkdocs build --strict` fails otherwise.

## Bindings

Python and JavaScript are views. They may infer inputs at the boundary - a string where a datatype
expression is expected, a path-like where a URL is expected - and must then call the core. A native
error message crosses the boundary unchanged, and each binding stays idiomatic in its own language:
mapping dunders and keyword arguments in Python, generic `from` constructors and `Map` protocols in
JavaScript.
