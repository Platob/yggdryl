# Rust core

This directory is the `yggdryl` core crate. The repository root owns the Cargo
workspace manifest, the shared dependency pins, and the shared lints; its
members are `rust/`, `python/`, and `node/`.

```text
src/datatype/          Categorized datatype implementation
src/field/             Field state, Arrow projection, casting, parsing, diffing
src/metadata.rs        Shared immutable metadata map
src/arrow/             Arrow scalars, arrays, batches, and IPC readers/writers
src/io/                The IOBase storage trait, Buffer, and Coded
src/generic/           Scalar, enums, Holder, Media, and RecordOptions
src/local/             Local Path, Folder, and memory-mapped File
src/{gzip,zlib,zstd}/  Content codings, whole-buffer and streaming
src/{ipc,parquet,avro}/
                       Record encodings over any handle
src/iceberg/           Apache Iceberg tables over one container handle
src/uri.rs             Identifier domain
src/text/              Structured codecs, dispatch, limits, text utilities
src/{json,yaml,toml}/  Format-specific parsers, streams, emitters
tests/{datatype,enums,field,text}/
                       Categorized edge cases
tests/{datatype,enums,field,uri,text,json,toml,yaml}.rs
                       Public test target wiring / edge cases
tests/{batch_cast,default_scalar,value_bounds}.rs
                       Arrow runtime, default, and allocation edge cases
tests/docs_index.rs    The documentation index regression
tests/iceberg_interop.rs
                       The Iceberg exchange with PyIceberg (`iceberg` feature)
benchmarks/{datatype,field,io,json,text,toml,yaml}/
                       Categorized benchmarks
benchmarks/{datatype,field,enums,uri,text,json,toml,yaml,io}.rs
                       Criterion target wiring / baselines
```

Run checks from the repository root. Root Cargo commands select only the Rust
core; `--workspace` explicitly adds the binding crates. Parquet and Iceberg are
non-default core features, so CI checks the default core and the full workspace.

```console
cargo fmt --all -- --check
cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings
cargo test -p yggdryl --all-targets
cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings
cargo test -p yggdryl --all-targets --all-features
cargo check -p yggdryl --profile bench --benches --all-features
```

Development and test profiles retain line-table backtraces but omit full debug
symbols. Use `--profile debugging` when a debugger needs full symbols.

Default and schema-only core builds support Rust 1.85. The optional `iceberg`
feature and both bindings require Rust 1.94 because they include official
Iceberg 0.10.1.

Arrow scalars, arrays, RecordBatch, and IPC live in the core `yggdryl::arrow`
module and are enabled by default. A schema-only consumer may disable the
runtime with `yggdryl = { version = "0.1", default-features = false }` and keep
Arrow schema projection. Both bindings depend directly on `yggdryl` and build it
with `arrow`, `parquet`, and `iceberg`.

Read [OPTIMIZATION.md](OPTIMIZATION.md) before changing schema storage, metadata
updates, Arrow conversion, or extension boundaries.

`DataType::variant(fields)` is the finite-sum convenience constructor. It assigns
declaration-order IDs and returns the canonical dense Arrow Union; the
`variant(...)` parser spelling canonicalizes to the same physical display. See
the [nested datatypes page](../docs/types/nested.md) before mapping one of these tagged
unions to Iceberg/Parquet Variant or PostgreSQL JSON, which use different
external encodings.

Python development:

```console
python -m venv python/.venv
python/.venv/Scripts/python -m pip install maturin pyarrow pytest mypy
python/.venv/Scripts/python -m maturin develop --manifest-path python/Cargo.toml
python/.venv/Scripts/python -m pytest python/tests
python/.venv/Scripts/python -m mypy --config-file python/pyproject.toml --strict python/yggdryl python/tests/typing_bindings.py python/tests/types/typing_fields.py
```

The field decorator, annotation mapping, dataclass field definitions, and codec
examples are documented in [`python/FIELDS.md`](../python/FIELDS.md) and
[`python/README.md`](../python/README.md).

Node.js development:

```console
npm ci --prefix node
npm run --prefix node build:debug
npm test --prefix node
```

Use release builds only for packaging and performance measurements.
