# yggdryl

> **Project status: rebuilding.** The old implementation was removed and the
> project is being rebuilt around an **Arrow-centralized** design. The foundational
> `codec` and `compression` layers have landed; more follows, guided by the
> contributor rules in `CLAUDE.md`.

A Rust-core library with Python and Node.js extensions.

📖 **Documentation: <https://platob.github.io/yggdryl/>**

## Layout

- `crates/yggdryl-core` — the Rust core foundations (the `codec`, `compression`, and
  `io` layers so far).
- `bindings/python` — the Python extension (PyO3 / maturin).
- `bindings/node` — the Node.js extension (napi-rs).

See [`CLAUDE.md`](CLAUDE.md) for contributor and agent instructions.

## Develop

### Prerequisites

- **Rust** (stable) via [rustup](https://rustup.rs) — pinned by
  [`rust-toolchain.toml`](rust-toolchain.toml).
- **[uv](https://docs.astral.sh/uv/)** — the Python toolchain for this project
  (venv, installs, building the extension, tests, docs). `uv venv && uv pip install
  maturin`.
- **Node.js ≥ 16** with `npm` — the `@napi-rs/cli` dev-dependency builds the addon.
- **CMake + Ninja + a C compiler** for the bindings — they build the fast
  `gzip-zlib-ng` and `zstd` backends. Install CMake/Ninja with
  `uv run python scripts/setup-build-deps.py`. The pure-Rust core (default `cargo`
  commands, `--no-default-features` for none) needs no C toolchain.

### Rust core

`cargo` with no arguments only touches the pure-Rust core, so no Python or Node
toolchain is needed for the common loop:

```bash
cargo build                 # build the core
cargo test                  # unit, integration, and doctests
cargo bench -p yggdryl-core # run the codec throughput benchmark
```

The core is a library (no binary yet); exercise it through `cargo test`, the
doctests, or the benchmark above.

### Python extension

Built and tested through **[uv](https://docs.astral.sh/uv/)**:

```bash
cd bindings/python
uv run maturin develop      # compile and install into the uv env
uv run python -c "from yggdryl import compression; print(compression.Gzip().name)"
uv run pytest               # run the binding tests
```

### Node extension

```bash
cd bindings/node
npm install                 # once, to fetch @napi-rs/cli
npm run build               # napi build --release -> yggdryl.<triple>.node
node -e "const {compression}=require('.'); console.log(new compression.Gzip().name)"
npm test                    # run the binding tests
```

### Benchmarks

Each surface has a throughput benchmark; the Python and Node scripts compare
`yggdryl` against the platform's native gzip. Build the extensions in **release**
first — a debug build is ~20× slower.

```bash
cargo bench -p yggdryl-core                                   # Rust core
(cd bindings/python && maturin develop --release) && \
  python bindings/python/benchmarks/bench_compression.py       # Python vs stdlib gzip
(cd bindings/node && npm run build) && npm --prefix bindings/node run bench  # Node vs zlib
```

### Before committing

Run the full gate (formatting, lints, tests across all three surfaces, and — when
`docs/` changed — a strict docs build). The authoritative list lives in
[`CLAUDE.md`](CLAUDE.md#required-checks-before-committing):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
mkdocs build --strict       # when docs/ or mkdocs.yml changed
```

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
