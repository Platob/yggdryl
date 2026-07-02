# yggdryl — contributor & agent instructions

> **Project status: rebuilding.** The old implementation was removed; the project is
> being rebuilt around an **Apache Arrow-centralized** data model, one workspace
> crate per layer, targeting dataframe-style workloads. This file holds only the
> **cross-cutting rules**; each crate documents its own design (type shapes, naming,
> API surface) in its module doc comments and README, so this file stays small as
> the workspace grows.

Before adding anything, read the nearest existing example and mirror its structure,
naming, error handling, and doc style. A reader should not be able to tell which
type they are looking at from the shape of the code.

## Hard rules (apply to every crate, every language)

1. **One file per public type.** Each concern is its own module
   (`src/datatype/int_width.rs` holds one type), re-exported from its `mod.rs`;
   `lib.rs` is glue only (`mod` declarations, `pub use` re-exports, shared helpers
   such as error conversion and the `log_event!` macro). Never grow one big file.
2. **FFI-clean public surface.** No lifetime parameters on any public type — the
   bindings must be able to hold every one of them. Temporary borrows appear only
   on `&self` accessor methods and never escape. All logic lives in the Rust core;
   bindings stay thin.
3. **Append-only public API.** Once merged, the public surface only grows: mark
   public enums `#[non_exhaustive]`, never repurpose or remove a published item.
4. **The three languages move together.** The Rust core is the source of truth;
   behaviour added or changed anywhere is immediately replicated in the other two,
   adapting only to idioms (Python dunders / keyword defaults, JS camelCase /
   `Option<T>` defaults). A change is never half-applied.
5. **Serializable to and from bytes whenever possible.** Every value type
   round-trips through bytes via `serialize_bytes()` / `deserialize_bytes(bytes)`;
   the only exceptions are live/stream resources (IO handles, HTTP bodies,
   sessions), which carry no serializable value. `deserialize_bytes` validates
   fully (length-check against the type's width) and is the exact inverse of
   `serialize_bytes`. Bindings mirror the pair, adapting to idioms (Node camelCase
   `serializeBytes()` / `deserializeBytes()`; Python `__reduce__` so pickle
   round-trips too).
6. **IO goes through the core IO traits.** Anything that reads or writes a
   sequence of bytes or bits implements the positioned-IO surface in
   `yggdryl-core` (`Seekable` + `RawIOBase`; typed element writes via
   `IOBase<T>`), and generic transfer/streaming code is written against those
   traits — never against a concrete buffer type or an ad-hoc `Vec<u8>`
   parameter. The one sanctioned byte-slice surface is rule 5's per-type
   `serialize_bytes` / `deserialize_bytes` codec. Transfers between two resources
   use the chunked `pread_io` / `pwrite_io` streams rather than materializing
   whole copies.

## Workspace layout

One crate per layer; dependencies point strictly downward (a lower layer never
imports an upper one — needing the reverse means the abstraction belongs lower).

- `crates/yggdryl-core` — dependency-light foundations (streaming byte-IO, shared
  error types). **No Arrow vocabulary in core.**
- `crates/yggdryl-data` — the Arrow data-model layer, built on `arrow-buffer`
  buffers.
- Higher layers (logical types, nested types, kernels) and service crates
  (e.g. HTTP) are added as further workspace members, each depending only on the
  layers below it.
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**: they translate types/errors and delegate — each method is one or two
  lines calling `self.inner`, `pub(crate)` so sibling modules can convert — and
  contain no logic. Each Rust crate is a submodule of the top-level package
  (`yggdryl-core` → `yggdryl.core`; Python via `sys.modules`, Node via
  `#[napi(namespace = "…")]`), and the binding source mirrors the crate tree
  (`src/<crate>.rs` or `src/<crate>/` per crate, `lib.rs` wiring only). Use
  `#[pyo3(signature = ...)]` / napi `Option<T>` for defaults.

## Dependencies

Minimal and pinned in the workspace `Cargo.toml`. For Arrow, use only the subset
crates actually needed (`arrow-schema`, `arrow-buffer`, `arrow-array`) — never the
full `arrow` umbrella or `arrow-flight`. Any new dependency carries a code comment
justifying it. Never pull a heavy SDK into a crate that should not depend on it.

## Errors and docs

- One error `enum` per type implementing `Display` + `std::error::Error`, with
  `From` conversions between layers; core errors map to `ValueError` (Python) /
  thrown `Error` (Node).
- **Error messages are actionable**: name the fix — the missing feature (``enable
  the `gzip` cargo feature``), the expected input (`expected 0, 1 or 2`), the
  offending value (`unknown mode "rw+"`).
- Every public item has a `///` doc comment; types carry a runnable doctest. Match
  the existing terse style.

## Logging

An optional, **off-by-default** `log` cargo feature, used only through the
crate-local `log_event!(level, …)` macro (compiles to nothing when off). Never call
`log::` directly; keep the `log` dependency `optional`. Levels: `trace` per-call
detail; `debug` a routine action; `info` an important action completed (especially
global/shared-state changes); `warn` an input skipped or a fallback defaulted. A
code path that skips, defaults, or mutates shared state must log it; the feature
must compile and pass `clippy -D warnings` both on and off.

## Documentation

User-facing docs are a **MkDocs Material** site in `docs/` (config: `mkdocs.yml`,
published to GitHub Pages; recreate as code lands). The docs tree mirrors the code
tree: one page per module, added to `nav` when the module is added. Docs follow the
replication rule — a change is not done until the matching page is updated in the
same commit. Every code example is a synced language-tab block, in this order with
these exact labels: `=== "Python"` then `=== "Node"` then `=== "Rust"`
(4-space-indented fenced block under each); never sequential per-language sections.
Keep examples accurate to the current API and copy-runnable.

## Required checks before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
mkdocs build --strict   # when docs/ or mkdocs.yml changed (pip install mkdocs-material)
```

All must pass. Then do a final **coherence pass** — required, not optional polish:

1. No redundancy — fold duplicated logic into one place; don't add an API that
   restates another.
2. Cross-language parity — same surface and semantics in the core and both
   bindings.
3. One concern per file, in the right crate/module, mirroring its neighbours.
4. Readability — every public item documented, matching its neighbours.
5. Docs in sync — the matching page reflects the change, with synced language tabs.

Fix any failure before committing.

## Releasing

The workspace `version` in the root `Cargo.toml` (`[workspace.package]`) is the
single source of truth: bump it and merge to `main`, and the `Release` workflow
(seeing no matching `v<version>` tag) runs the gate, publishes to crates.io / PyPI /
npm, then tags and creates a GitHub Release. Inter-crate deps use caret ranges, so a
`0.1.x` bump only touches the version line; Python wheels inherit it via
`version.workspace = true`, the npm `package.json` is synced at publish time — keep
it in sync locally too. Never re-use a published version number.

The Python extension targets PyO3's **stable ABI** (`abi3-py37`): one `cp37-abi3`
wheel per OS/arch covers CPython ≥ 3.7 — don't build per-interpreter wheels, and
keep binding code within the limited API (the PyO3 `*_bound` helpers already are).
