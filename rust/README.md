# Rust core

This directory is the `yggdryl` core crate. The repository root owns the Cargo
workspace manifest, the shared dependency pins, and the shared lints; its
members are `rust/`, `rust/market/` (`yggdryl-market`), `rust/fix/`
(`yggdryl-fix`), `python/`, `node/` and `cli/`. The core's manifest excludes
`market/` and `fix/`, each a crate of its own with its own `README.md`.

```text
src/<name>.rs          One type, or one shared trait, enum or value, per root
                       file: a type's datatype, field and scalar together
                       (integer.rs, string.rs, isin.rs, state.rs), the vocabulary
                       beside them (datatype.rs, field.rs, scalar.rs, serie.rs,
                       cast.rs, iobase.rs, market.rs, vocabulary.rs)
src/value/             What a datatype, a field and a value owe the root
src/<implementation>/  One folder per implementation: storage (local, fs, zip,
                       s3, http), media (ipc, parquet, avro, csv, excel, xmla,
                       iceberg), codecs (json, yaml, toml, xml, text), and
                       arrow, expression, graph, logging, warehouse, xxhash
src/{gzip,zlib,zstd}.rs
                       Content codings, one root file each
tests/<entry>.rs       One target per top-level source entry; tests/root.rs
                       declares the files the crate root holds
tests/<entry>/         src/<entry>/<name>.rs is pinned by tests/<entry>/<name>.rs,
                       a folder's mod.rs by mod_.rs; tests/support/ the shared
                       fixtures
benchmarks/            Criterion targets by theme: types, holder, media, text, ...
market/                yggdryl-market: the market kinds, identifiers, the ISIN
                       registry and the market-data graph
fix/                   yggdryl-fix: the FIX dictionary, codec, messages and rows
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

The crate builds on Rust 1.94 and newer, every feature included.

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
VIRTUAL_ENV=python/.venv python/.venv/Scripts/python -m maturin develop --manifest-path python/Cargo.toml
python/.venv/Scripts/python -m pytest python/tests
python/.venv/Scripts/python -m mypy --config-file python/pyproject.toml --strict python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py
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
