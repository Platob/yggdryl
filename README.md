# yggdryl

> **Project reset.** The implementation has been removed and the project is being
> rebuilt around an **Arrow-centralized** design. Only the buildable skeleton and
> the contributor rules (`CLAUDE.md`) remain.

A Rust-core library with Python and Node.js extensions.

## Layout

- `crates/yggdryl-core` — the Rust core foundations.
- `crates/yggdryl-schema` — the Arrow-compatible schema layer.
- `crates/yggdryl-http` — the HTTP client.
- `bindings/python` — the Python extension (PyO3 / maturin).
- `bindings/node` — the Node.js extension (napi-rs).

See [`CLAUDE.md`](CLAUDE.md) for contributor and agent instructions.

## License

Apache-2.0 — see [`LICENSE`](LICENSE).
