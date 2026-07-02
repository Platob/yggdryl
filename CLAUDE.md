# yggdryl — contributor & agent instructions

> **Project status: rebuilding.** The old implementation was removed; the project is
> being rebuilt around an **Apache Arrow-centralized** data model, one workspace
> crate per layer. The goal: one Rust core exposed to Python and Node with the
> **same code patterns**, so data code is written once and runs identically in all
> three languages — manipulating huge volumes across data sources through
> zero-copy, Arrow-backboned containers and lazy computation, at Rust performance.
> This file holds only the **cross-cutting rules**; each crate documents its own
> design in its module doc comments and README, so this file stays small as the
> workspace grows.

Before adding anything, read the nearest existing example and mirror its structure,
naming, error handling, and doc style. A reader should not be able to tell which
type they are looking at from the shape of the code.

## Hard rules (apply to every crate, every language)

1. **One file per public type.** Each concern is its own module
   (`src/datatype/int_width.rs` holds one type), re-exported from its `mod.rs`;
   `lib.rs` is glue only (`mod` declarations, `pub use` re-exports, shared helpers
   such as error conversion and the `log_event!` macro). Never grow one big file.
2. **Typed construction only.** Validation happens in typed constructors; invalid
   states are unrepresentable after construction. There is no lenient mode and no
   `safe` flag. **No string-parsing constructors** — `from_str` / `parse*` are
   legacy and must not be (re)added. `Display` impls are render-only diagnostics
   with no parsing counterpart and no round-trip contract.
3. **FFI-clean public surface.** No lifetime parameters on any public type — the
   bindings must be able to hold every one of them. Temporary borrows appear only
   on `&self` accessor methods and never escape. All logic lives in the Rust core;
   bindings stay thin.
4. **Append-only public API.** Once merged, the public surface only grows: mark
   public enums `#[non_exhaustive]`, never repurpose or remove a published item.
5. **At-most-one-copy.** Buffers are refcounted; slicing/viewing never copies. A
   value extracted from a larger container is a zero-copy slice holding a refcount
   on the parent buffer; a standalone value is the same type over a fresh buffer —
   same type, two provenances. Never copy speculatively; for any hot path ask
   "does this allocate when nothing changed?" and add a cheap up-front check if so.
6. **Numeric semantics never lie.** Widening reads are fine (an i8 readable via
   `as_i64()`); silent truncation is not — return `None` or a typed error.
   Cross-type comparisons return `None`, never panic.
7. **Everything is serializable and hashable** except live/stream resources (IO
   handles, HTTP bodies, sessions). Value types round-trip through bytes
   (`to_bytes` / `from_bytes`) and JSON (off-by-default `serde` feature;
   `to_json` / `from_json` where a `json` feature exists) and implement `Hash` +
   `Eq`. Bindings mirror this: Python `__hash__` + `__reduce__` (pickle), Node
   `toJSON()` + static `fromJSON()`. A field that cannot be part of identity
   (e.g. a cyclic `parent` pointer) is excluded from `Hash`/`Eq`/serde with a
   comment saying why, rather than dropping hashability.
8. **The three languages move together.** The Rust core is the source of truth;
   behaviour added or changed anywhere is immediately replicated in the other two,
   adapting only to idioms (Python dunders / keyword defaults, JS camelCase /
   `Option<T>` defaults). A change is never half-applied.
9. **Own the containers, keep Arrow's layout.** yggdryl defines its own data
   containers (`Scalar`, the `Array` implementations, and friends) rather than
   wrapping `arrow-array` types — the move polars made with arrow2/polars-arrow.
   A container holds
   `arrow_buffer::Buffer`s plus a validity bitmap, laid out exactly per the Arrow
   columnar format spec, so `to_arrow` / `from_arrow` are zero-copy buffer
   handoffs. All data manipulation is implemented on our own containers;
   `arrow-array` appears only at the interop boundary, never as the storage or
   compute representation.

## Workspace layout

One crate per layer; dependencies point strictly downward (a lower layer never
imports an upper one — needing the reverse means the abstraction belongs lower).

- `crates/yggdryl-core` — dependency-light foundations (streaming byte-IO, shared
  error types). **No Arrow vocabulary in core.** Byte sources implement the single
  IO abstraction and hand back zero-copy views; byte consumers take an IO/reader,
  never a pre-collected `Vec`.
- `crates/yggdryl-schema` — the schema layer (`DataType`, `DataTypeId`, the
  erased `AnyDataType`, the abstract `Field` base with its generic `TypedField`
  implementation, and the category subtraits `PrimitiveType` / `LogicalType` /
  `NestedType`): the typed vocabulary every upper layer and binding shares.
- `crates/yggdryl-scalar` — the scalar container layer (`Scalar<T>`, the
  `ScalarType` layout contract): one typed value over an `arrow-buffer`
  `Buffer`, validated at construction and extracted from arrays as a zero-copy
  slice.
- `crates/yggdryl-array` — the array container layer (the abstract `Array`
  base, `PrimitiveArray<T>`, further array types as they land): typed columns
  over `arrow-buffer` buffers plus `NullBuffer` validity bitmaps in the Arrow
  columnar layout; slicing and scalar extraction never copy.
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

## Arrow interop

- Every yggdryl type that maps to Arrow exposes `to_arrow()` / `from_arrow(...)`,
  and the mapping is **total and reversible** for the supported subset — losslessly
  round-trippable, and **property-tested** as such. Where Arrow lacks a physical
  type, anchor on a compatible physical type plus metadata that restores the
  semantics, and document the rationale in the doc comment.
- `from_arrow` is the **only inbound conversion** and validates fully; unknown
  `ygg.*` metadata values are rejected with a typed error.
- All `ygg.*` metadata keys are namespaced constants defined in **one module**
  (the single source of truth) — no string literals scattered through the code.

## Naming conventions (identical across languages; JS uses camelCase)

| Concept | Name |
| --- | --- |
| Construct from explicit parts | `from_parts(...)` |
| Typed conversions | `from_<type>(...)` / `to_<repr>()` (e.g. `from_i8`, `from_le_bytes`, `to_arrow`) |
| Serialize to / from bytes | `to_bytes()` / `from_bytes(bytes)` |
| JSON (where a `json` feature exists) | `to_json()` / `from_json(value)` |
| Checked read accessor | `as_<type>()` on `&self`, `Option`-returning |
| Independent / overriding copy | `copy(...)` — every field optional, omitted fields come from `self` |
| Single-field functional update | `with_<field>(value)` returns a new value |
| Clear an optional field | `without_<field>()` |

- `from_*` names take **typed** inputs and validate (length-check byte inputs
  against the type's width); the word "parse" never appears in the public API.
- `with_*` / `without_*` / `copy` are **non-mutating** and return a new value.
  `copy` is the one primitive that rebuilds the value with selected fields
  overridden; every `with_*` / `without_*` is a one-line delegation to it. Design
  trait signatures so implementors can satisfy them in one line; expand to
  multi-line bodies only when the logic genuinely needs it.
- Shared handles are `Arc` type aliases named `<Type>Ref` (e.g.
  `pub type TypedFieldRef<T> = Arc<TypedField<T>>`); the Arc clone IS the cheap sharing mechanism —
  no view/borrowed variants.

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

1. No redundancy — fold duplicated logic into one place; a new `from_*` delegates
   to an existing one; don't add an API that restates another.
2. Cross-language parity — same surface and semantics in the core and both
   bindings.
3. One concern per file, in the right crate/module, mirroring its neighbours.
4. Readability — names match the conventions table; every public item documented.
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
