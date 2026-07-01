# yggdryl — contributor & agent instructions

> **Project status: reset.** The implementation has been removed and the project
> is being rebuilt around an **Arrow-centralized** design. What remains is the
> buildable skeleton (the Cargo workspace, the crate and binding manifests, CI,
> and empty stub `lib.rs` files) and these foundational rules. The detailed
> per-module architecture that used to live here was dropped with the code it
> described; reintroduce architecture docs as you build, following the rules
> below.

**Keep all new code uniform.** Before adding anything, read the nearest existing
example and mirror its structure, naming, error handling, and doc style.
Consistency across the Rust core and the two bindings is the top priority — a
reader should not be able to tell which type they are looking at from the shape of
the code.

**Everything must be serializable and hashable.** Do your best to make every value
type round-trip through *all* of: JSON (`serde`, plus `to_json`/`from_json` where a
crate exposes a `json` feature) and **bytes** (`to_bytes`/`from_bytes`), and to
derive (or hand-implement) `Hash` + `Eq` so it can key a map or set. In the bindings
this means `__hash__` + `__reduce__` (pickle) in Python and `toJSON()` + a static
`fromJSON()` in Node. The only exceptions are live/stream resources (IO handles,
HTTP bodies, sessions). When a field cannot be part of a value's identity
(e.g. a navigational `parent` pointer, which would create cycles), exclude it from
`Hash`/`Eq`/`serde` rather than dropping hashability — and document why.

## Workspace layout

The workspace is **three Rust crates plus two thin bindings**, the layers of the
Arrow-centric type system growing back after the reset:

- `crates/yggdryl-core` — the dependency-light foundations every other crate and
  binding builds on. Currently a scaffold exposing only `version()`; reintroduce
  the foundational types here (the zero-copy `Buffer`, the `Io` / `Whence` byte
  abstraction, the `Charset` encodings, the global `JsonParams` + the `Jsonable`
  JSON/BSON trait and the shared error types), one module per concern, with no Arrow
  vocabulary living here.
- `crates/yggdryl-schema` — the Arrow-compatible schema layer (`DataType` / `Field`
  and the schema types), holding the conversion to and from Apache Arrow's
  `arrow-schema` behind its `arrow` feature. The `arrow-schema` SDK is a dependency
  of this crate only. Depends only on `core`.
- `crates/yggdryl-scalar` — the scalar *values*: the `Scalar` trait (a value's
  `dtype` plus its `to_bytes` / `from_bytes` byte form) and the byte-backed
  `Binary` value carrying any binary data type. Depends on `core` + `schema`.
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**. They only translate types/errors and call the crates above; they
  contain no logic. Anything added to a crate must be surfaced in *both* bindings.
  **Each Rust crate is exposed as a submodule of the top-level package**, mirroring
  the crate tree: `yggdryl-core` → `yggdryl.core`, `yggdryl-schema` →
  `yggdryl.schema` (Python submodules registered in `sys.modules`; Node `#[napi(namespace
  = "…")]` exports). The binding source mirrors this too — `src/<crate>.rs` or
  `src/<crate>/` per crate, with `src/lib.rs` only wiring the submodules together.

As the Arrow-centric type system grows back it is **split into one crate per
layer** (data types, then scalar *values*, then fields), each depending only on
the layers below it. Keep the dependency arrows pointing one way: a lower layer
never imports an upper one (a reader needing the other direction means the
abstraction belongs lower). The `Io` trait hands back zero-copy `core::Buffer`
views rather than a higher-layer value, so `core` stays free of the type layers
above it.

Each crate is **one file per type** — each concern is its own module (or module
directory) under `src/`, with `lib.rs` as glue (a crate-local `log_event!` macro,
`mod` declarations, and `pub use` re-exports of every type at the crate root). Each
module owns its concern wholly — do not scatter a concern's logic across modules,
and do not pull a heavy SDK into a crate that should not depend on it.

### One module per type, everywhere

Code is organised the same way in every language: **one file per type**, with a
small glue file tying them together. Don't grow a single big file.

- Rust: one module per concern in each crate.
- Each binding: `src/<type>.rs` per type, with `src/lib.rs` holding only shared
  helpers (error conversion, hashing, encoding free functions) and the module
  registration. Per-type wrappers keep their `inner` field `pub(crate)` so sibling
  modules can convert.

### Cross-language replication rule

The Rust core is the source of truth, but the languages move together. **When you
add or change behaviour in Rust, immediately replicate it in the Python and Node
extensions; when you change an extension, fold the behaviour back into the Rust
core and the other extension.** Adapt to each language's idioms (Python dunders /
keyword defaults, JS camelCase / `Option<bool>` defaults) but keep the surface and
semantics identical, so the three codebases stay coherent and a change is never
half-applied.

### Serialization — a cross-cutting optional concern

Every value type is **serializable**, but the mechanism is idiomatic per language
(adapt to each, keep the semantics identical). In Rust it is the off-by-default
`serde` feature: value types `derive` a structural `Serialize` / `Deserialize`. The
bindings surface the same: **Python**
implements `__reduce__` (so `pickle` / `copy` reconstruct through the existing
constructors), **Node** implements `toJSON()` + a static `fromJSON()` (used by
`JSON.stringify`). Live/stream resources (IO handles, an HTTP body, a session) are
**not** serialised. When you add a type, add its serde impl and replicate the
pickle / `toJSON` surface in both bindings.

## Naming conventions (cross-language)

These names are identical in Rust, Python and JS (JS uses camelCase):

| Concept | Name |
| --- | --- |
| Construct from any supported input | `from_` (Rust trait `FromInput`) |
| Construct from explicit parts | `from_parts(...)` |
| Serialize to / from bytes | `to_bytes()` / `from_bytes(bytes)` |
| JSON (where a `json` feature exists) | `to_json()` / `from_json(value)` |
| Independent / overriding copy | `copy(...)` — every field optional, omitted fields come from `self` |
| Single-field functional update | `with_<field>(value)` returns a new value |
| Clear an optional field | `without_<field>()` |
| Type conversions | `to_<type>` / `from_<type>` |

Rules:
- Parsing entry points are `from_*`, never `parse*` (the public API does not use
  the word "parse").
- Parsing always validates and returns an error / raises on malformed input;
  there is no lenient mode and no `safe` flag.
- `with_*` / `without_*` / `copy` are **non-mutating** and return a new value.
- URL-safe `percent_encode` / `percent_decode` are the only encoding helpers;
  modifiers that build query strings percent-encode their inputs.

## Patterns to mirror

- **Errors**: one `enum` per type (e.g. `UriError`) implementing `Display` +
  `std::error::Error`, with `From` conversions between layers. Core errors map to
  `ValueError` (Python) / thrown `Error` (Node).
  **Make error messages actionable**: when the fix is knowable, say it in the
  message — name the missing feature (`enable the \`gzip\` cargo feature`), the
  expected input (`expected 0, 1 or 2`), or the offending value (`unknown mode
  "rw+"`). A reader should learn *how to fix it* from the message, not just that it
  failed.
- **Docs**: every public item has a `///` doc comment; types carry a runnable
  doctest. Match the existing terse style.
- **Bindings**: each wrapper method is one or two lines delegating to
  `self.inner`. Use `#[pyo3(signature = ...)]` / napi `Option<T>` for defaults.
- **One-line functional updates**: write the non-mutating helpers as a single
  expression. `copy` is the one primitive that rebuilds the value with selected
  fields overridden (omitted ones taken from `self`); every `with_<field>` /
  `without_<field>` is a one-line delegation to it — e.g.
  `fn with_name(&self, name: String) -> Self { self.copy(Some(name), None, None, None) }`.
  Favour concise functional one-liners wherever they stay readable, and define the
  trait method signatures so an implementor can satisfy them on one line; only
  expand to a multi-line body when the logic genuinely needs it.

## Performance: zero-copy with checks

Prefer **borrowing over copying**. A function that returns string data should hand
back a borrow (`&str`) or a `Cow` and allocate **only when the data must actually
change** — guarded by a cheap up-front check:

- Decode/validate paths check for the trigger byte (e.g. `%`) first and return the
  input untouched when it is absent — no allocation, no second scan.
- Encode paths scan for the first byte that needs escaping; if there is none they
  return `Cow::Borrowed`, otherwise they allocate once and copy the already-valid
  prefix verbatim before encoding the rest.
- Single-key lookups scan for the one key instead of building the whole map, and
  compare the raw bytes without allocating unless an escape forces a decode.

When you add a hot path, ask "does this allocate when nothing changed?" — if so,
add the check and borrow. Never copy speculatively; never re-scan what a single
pass can decide.

**Prefer view types by default.** When nothing forces a particular layout, default
to the *view* variant of the binary/string types (`BinaryViewType` / `StringViewType`
and their `Large*` siblings) over the offset-backed `BinaryType` / `StringType`.
View values share their bytes through the zero-copy `Buffer`, so cloning, slicing and
casting them never deep-copy. A constructor picking a default type, a binding
exposing one, or a doc example that just needs "some bytes" / "a string" should reach
for the view type; choose a non-view variant only when an external format, an
offset-width requirement, or a fixed-/max-size cap demands it.

**Centralise byte/memory access behind one IO abstraction.** A new byte source
(memory buffer, local file, cloud object, HTTP body) should implement that single
trait and override the zero-copy hook when it is memory-resident, so positional
reads, copies, JSON parsing and media-sniffing all light up the zero-copy path for
free. Operations that consume bytes (JSON, compression, codecs, HTTP bodies) take an
IO/reader, never a pre-collected `Vec`, so the data is read once and copied at most
once. This extends to the **bindings**: a Python/JS wrapper that needs bytes should
accept and pass our IO instances, not serialized `bytes`, so a large body or upload
streams through Rust and is never materialised in the host language.

## Logging

The Rust crates carry an optional, **off-by-default** `log` feature, emitted only
through a crate-local `log_event!(level, …)` macro (which compiles to nothing when
the feature is off, so the crates stay dependency-free and pay no runtime cost).
Never call `log::` directly, and keep the `log` dependency `optional`.

When you add or change behaviour, instrument it at the right level:

- `trace` — very frequent, per-call detail (e.g. each parse entry).
- `debug` — a routine **action being performed** (e.g. inferring a media type).
- `info` — an **important action that completed**, especially a change to global or
  shared state (e.g. a registry `register` / `unregister` / `reset`).
- `warn` — a **skipped** input or a **defaulted** fallback was applied (e.g. an
  unknown extension dropped, a missing scheme defaulted).

A new code path that skips, defaults, or mutates shared state must log it; the `log`
feature must compile and pass `clippy -D warnings` both on and off.

## Documentation

User-facing docs live in **`docs/`** as a **MkDocs Material** site (config:
`mkdocs.yml`), published to GitHub Pages. **The docs tree mirrors the code tree** —
one page per concern/module, so code and documentation map 1:1 and a reader can
find the doc for any type by its module. (The `docs/` tree and `mkdocs.yml` were
removed in the reset; recreate them as the Arrow-centralized code lands.)

Rules (treat them like the cross-language replication rule — a change is not done
until the docs match):

- **When you add or change behaviour, update the matching doc page** in the same
  commit. A new module/type gets a new page added to the `nav` in `mkdocs.yml`
  mirroring its code location.
- **Every code example is a synced language-tab block**, in this order and with
  these exact labels (so Material's linked tabs switch the whole page at once):
  `=== "Python"` then `=== "Node"` then `=== "Rust"` (4-space-indented fenced block
  under each). Never write raw, one-after-another per-language sections.
- Keep examples **accurate to the current API**; prefer copy-runnable snippets.
- **Doc build check** (add it to the gate when you touched docs):
  `pip install mkdocs-material && mkdocs build --strict` must pass (strict catches
  broken links and missing nav pages).

## Required checks before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && maturin develop && pytest)
(cd bindings/node && npm run build && npm test)
mkdocs build --strict   # when docs/ or mkdocs.yml changed (pip install mkdocs-material)
```

All must pass.

## Releasing

The workspace `version` under `[workspace.package]` in the root `Cargo.toml` is the
single source of truth. To cut a release, bump it and merge to `main`: the
`Release` workflow detects the new version (no matching `v<version>` tag yet), runs
the gate, publishes to crates.io / PyPI / npm, then creates the tag and a GitHub
Release. Inter-crate dependencies use caret ranges, so a `0.1.x` bump only touches
the version line (the Python wheels inherit it via `version.workspace = true`; the
npm `package.json` is synced from it at publish time — keep it in sync locally too).
Never re-use a published version number; crates.io/npm reject re-uploads.

The Python extension is built against PyO3's **stable ABI** (`abi3-py37`), so one
`cp37-abi3` wheel per OS/arch covers every CPython from **3.7** up
(`requires-python = ">=3.7"`) — don't build a wheel per interpreter version. Keep
new binding code within the limited API (the PyO3 `*_bound` helpers already are).

## Code-coherence review (after every implementation)

Once the change compiles and the checks pass, do a final coherence pass before
committing — treat it as a required step, not an optional polish:

1. **No redundancy** — fold duplicated logic into one place; a new `from_*` should
   delegate to an existing one rather than re-implement it. Don't add a second API
   that merely restates an existing one.
2. **Cross-language parity** — the same surface and semantics exist in the Rust core
   and *both* bindings (adapting only to each language's idioms); a change is never
   half-applied.
3. **One concern per file/type** — the new code lives in the right crate/module and
   mirrors the structure of its neighbours (naming, error handling, doc style,
   terseness).
4. **Readability** — names match the conventions table, every public item has a
   `///` doc, and a reader cannot tell which type they are looking at from the shape
   of the code.
5. **Docs in sync** — the matching `docs/` page reflects the new/changed behaviour,
   with synced Python/Node/Rust language tabs.

If any point fails, fix it before committing.
