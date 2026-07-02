# yggdryl — contributor & agent instructions

> **Project status: reset.** The implementation was removed; the project is being
> rebuilt around an **Arrow-centralized** design. What remains is the buildable
> skeleton — the Cargo workspace, `yggdryl-core`, the two binding manifests, CI,
> and a minimal `version()` / `hello()` example that round-trips all three
> languages — plus these rules. Reintroduce architecture docs as you build.

## Core principles

1. **Uniformity first.** Before adding anything, read the nearest existing example
   and mirror its structure, naming, error handling, and doc style. A reader should
   not be able to tell which type they are looking at from the shape of the code.
2. **The three languages move together.** The Rust core is the source of truth, but
   any behaviour added or changed in one language is immediately replicated in the
   other two, adapting only to each language's idioms (Python dunders / keyword
   defaults, JS camelCase / `Option<T>` defaults). A change is never half-applied.
3. **Everything is serializable and hashable.** Every value type round-trips through
   JSON (off-by-default `serde` feature; `to_json` / `from_json` where a crate has a
   `json` feature) and bytes (`to_bytes` / `from_bytes`), and implements `Hash` +
   `Eq`. In the bindings: `__hash__` + `__reduce__` (pickle) in Python, `toJSON()` +
   static `fromJSON()` in Node. Only live/stream resources (IO handles, HTTP bodies,
   sessions) are exempt. A field that cannot be part of a value's identity (e.g. a
   cyclic `parent` pointer) is excluded from `Hash`/`Eq`/serde — with a comment
   saying why — rather than dropping hashability.
4. **One file per type, in every language.** Each concern is its own module; never
   grow one big file. `lib.rs` is glue only: `mod` declarations, `pub use`
   re-exports, and shared helpers (error conversion, the `log_event!` macro; in
   bindings also hashing/encoding free functions). Binding wrappers keep their
   `inner` field `pub(crate)` so sibling modules can convert.

## Workspace layout

- `crates/yggdryl-core` — the dependency-light foundations everything else builds
  on (currently only `version()` / `hello()`). Reintroduce the foundational types
  here: the zero-copy `Buffer`, the byte-IO abstraction, the `Charset` encodings,
  the global `JsonParams` + the `Jsonable` JSON/BSON trait, and the shared error
  types. **No Arrow vocabulary in core.**
- Grow the Arrow type system back as **one crate per layer** (schema data types →
  scalar values → fields), each added to the workspace members and depending only
  on the layers below it. Dependency arrows point one way: a lower layer never
  imports an upper one; the byte-IO layer returns `core::Buffer` views, never a
  higher-layer value.
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**: they translate types/errors and delegate — each method is one or two
  lines calling `self.inner` — and contain no logic. Each Rust crate is exposed as
  a submodule of the top-level package (`yggdryl-core` → `yggdryl.core`; Python via
  `sys.modules`, Node via `#[napi(namespace = "…")]`), and the binding source
  mirrors the crate tree (`src/<crate>.rs` or `src/<crate>/` per crate, `lib.rs`
  wiring only). Use `#[pyo3(signature = ...)]` / napi `Option<T>` for defaults.

## Naming conventions (identical across languages; JS uses camelCase)

| Concept | Name |
| --- | --- |
| Construct from explicit parts | `from_parts(...)` |
| Serialize to / from bytes | `to_bytes()` / `from_bytes(bytes)` |
| JSON (where a `json` feature exists) | `to_json()` / `from_json(value)` |
| Independent / overriding copy | `copy(...)` — every field optional, omitted fields come from `self` |
| Single-field functional update | `with_<field>(value)` returns a new value |
| Clear an optional field | `without_<field>()` |

- Parsing entry points are `from_*`, never `parse*`; parsing always validates and
  errors/raises on malformed input — no lenient mode, no `safe` flag.
- `with_*` / `without_*` / `copy` are **non-mutating** and return a new value.
  `copy` is the one primitive that rebuilds the value with selected fields
  overridden; every `with_*` / `without_*` is a one-line delegation to it, e.g.
  `fn with_name(&self, name: String) -> Self { self.copy(Some(name), None, None, None) }`.
  Design trait signatures so implementors can satisfy them in one line; expand to
  multi-line bodies only when the logic genuinely needs it.
- URL-safe `percent_encode` / `percent_decode` are the only encoding helpers;
  modifiers that build query strings percent-encode their inputs.

## Errors and docs

- One error `enum` per type (e.g. `UriError`) implementing `Display` +
  `std::error::Error`, with `From` conversions between layers; core errors map to
  `ValueError` (Python) / thrown `Error` (Node).
- **Error messages are actionable**: name the fix — the missing feature (``enable
  the `gzip` cargo feature``), the expected input (`expected 0, 1 or 2`), the
  offending value (`unknown mode "rw+"`).
- Every public item has a `///` doc comment; types carry a runnable doctest. Match
  the existing terse style.

## Performance: zero-copy with checks

Prefer **borrowing over copying** — return `&str` / `Cow` and allocate only when
the data must actually change, guarded by a cheap up-front check:

- Decode paths check for the trigger byte (e.g. `%`) first and return the input
  untouched when absent; encode paths scan for the first byte needing escaping and
  return `Cow::Borrowed` when there is none, else allocate once and copy the valid
  prefix verbatim.
- Single-key lookups scan for the one key instead of building the whole map, and
  compare raw bytes without allocating unless an escape forces a decode.
- For any hot path ask "does this allocate when nothing changed?" — if so, add the
  check and borrow. Never copy speculatively; never re-scan what one pass decides.
- **Prefer view types by default** (`BinaryViewType` / `Large*` sibling) over
  offset-backed `BinaryType` / `LargeBinaryType` — view values share bytes through
  the zero-copy `Buffer`, so clone/slice/cast never deep-copy. Pick a non-view
  variant only when an external format, offset width, or size cap demands it.
- **One IO abstraction for all byte access.** Every byte source (memory, file,
  cloud object, HTTP body) implements the single IO trait, overriding the
  zero-copy hook when memory-resident. Byte consumers (JSON, compression, codecs,
  HTTP bodies) take an IO/reader, never a pre-collected `Vec` — including the
  bindings, which accept our IO instances rather than serialized `bytes`, so large
  data streams through Rust and is never materialised in the host language.

## Logging

An optional, **off-by-default** `log` cargo feature, used only through the
crate-local `log_event!(level, …)` macro (compiles to nothing when off). Never call
`log::` directly; keep the `log` dependency `optional`. Levels:

- `trace` — per-call detail; `debug` — a routine action being performed;
- `info` — an important action completed, especially a change to global/shared
  state; `warn` — an input skipped or a fallback defaulted.

A code path that skips, defaults, or mutates shared state must log it; the feature
must compile and pass `clippy -D warnings` both on and off.

## Documentation

User-facing docs are a **MkDocs Material** site in `docs/` (config: `mkdocs.yml`,
published to GitHub Pages; removed in the reset — recreate as code lands). The docs
tree mirrors the code tree: one page per module, added to `nav` when the module is
added. Docs follow the replication rule — a change is not done until the matching
page is updated in the same commit.

Every code example is a synced language-tab block, in this order with these exact
labels: `=== "Python"` then `=== "Node"` then `=== "Rust"` (4-space-indented fenced
block under each). Never write sequential per-language sections. Keep examples
accurate to the current API and copy-runnable.

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
