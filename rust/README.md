# Rust core

This directory is the `yggdryl` core crate. The repository root owns the Cargo
workspace manifest, the shared dependency pins, and the shared lints; its
members are `rust/`, `python/`, and `node/`.

```text
src/datatype/          Categorized datatype implementation
src/field/             Field state, Arrow projection, casting, parsing, diffing
src/enums/             Shared value vocabularies: units, schemes, MIME values
src/metadata.rs        Shared immutable metadata map
src/arrow/             Arrow scalars, arrays, batches, and IPC readers/writers
src/io/                The IOBase storage trait, Buffer, and Coded
src/generic/           Scalar, enums, Holder, Media, and RecordOptions
src/local/             Local Path, Folder, and memory-mapped File
src/{gzip,zlib,zstd}/  Content codings, whole-buffer and streaming
src/{ipc,parquet}/     Record encodings over any handle
src/iceberg/           Apache Iceberg tables over one container handle
src/uri.rs             Identifier domain
src/text/              Structured codecs, dispatch, limits, text utilities
src/{json,yaml,toml}/  Format-specific parsers, streams, emitters
src/codec.rs           Compatibility-only public facade
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

Run the checks from the repository root. The Parquet encoding and the Iceberg
table format over it are behind non-default features, so the test and Clippy
passes run twice: once with default features and once with both enabled.

```console
cargo fmt --manifest-path rust/Cargo.toml
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets -- -D warnings
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg" -- -D warnings
cargo test --manifest-path rust/Cargo.toml
cargo test --manifest-path rust/Cargo.toml --features "parquet iceberg"
```

The core supports Rust 1.85. Building every workspace member requires the newer
toolchain the Node-API and PyO3 dependencies need.

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
the [datatype guide](../docs/datatype.md) before mapping one of these tagged
unions to Iceberg/Parquet Variant or PostgreSQL JSON, which use different
external encodings.

Python development:

```console
python -m venv python/.venv
python/.venv/Scripts/python -m pip install maturin pyarrow pytest mypy
python/.venv/Scripts/python -m maturin develop --release --manifest-path python/Cargo.toml
python/.venv/Scripts/python -m pytest python/tests
python/.venv/Scripts/python -m mypy --strict python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py
```

The field decorator, annotation mapping, dataclass field definitions, and codec
examples are documented in [`python/FIELDS.md`](../python/FIELDS.md) and
[`python/README.md`](../python/README.md).

Node.js development:

```console
npm install --prefix node
npm run --prefix node build
npm test --prefix node
```
