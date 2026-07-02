# yggdryl

**yggdryl** is an Arrow-backboned data toolkit: one Rust core exposed to Python
and Node.js with the **same code patterns**, so data code is written once and
runs identically in all three languages — manipulating huge volumes across data
sources with zero-copy containers and lazy computation, at Rust performance.

- **One core, three languages** — all logic lives in the Rust core; the Python
  (PyO3) and Node (napi-rs) bindings are thin, idiomatic mirrors of the same API.
- **Own containers, Arrow layout** — yggdryl's containers hold `arrow-buffer`
  buffers and validity bitmaps laid out exactly per the Apache Arrow columnar
  spec, so they convert to and from Arrow zero-copy and interoperate with the
  whole Arrow ecosystem.
- **Zero-copy, Rust performance** — buffers are refcounted; slicing and viewing
  never copy, and hot paths never allocate when nothing changed.
- **Cross-datasource** — Arrow as the interchange backbone centralizes data code
  across sources and formats, built for huge volumes and lazy computation.

> **Project status: rebuilding.** The previous implementation was removed; the
> project is being rebuilt layer by layer around this design. See
> [`CLAUDE.md`](CLAUDE.md) for the contributor and agent rules.

## Layout

- `crates/yggdryl-core` — the Rust core foundations.
- `crates/yggdryl-schema` — the Arrow-centralized schema layer (typed data types and fields).
- `bindings/python` — the Python extension (PyO3 / maturin).
- `bindings/node` — the Node.js extension (napi-rs).

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
