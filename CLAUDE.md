# yggdryl — contributor & agent instructions

> yggdryl is built around an **Apache Arrow-centralized** data model, one workspace
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
   behaviour added or changed anywhere is immediately replicated in the **Python and
   Node** bindings, adapting only to idioms (Python dunders / keyword defaults, JS
   camelCase / `Option<T>` defaults). A change is never half-applied: the same commit
   that adds or changes a binding-visible surface updates **both** bindings and their
   tests, and every task ends with a **coherence check** confirming the three
   surfaces match method-for-method and behave identically (the binding test suites
   are the executable proof). A core item may stay **Rust-only** only when it cannot
   cross the FFI boundary cleanly: the two-resource streams (`pread_raw_io` /
   `pwrite_raw_io` and the typed `pread_typed_io` / `pwrite_typed_io`, which borrow
   two resources at once) and the typed `IOCursor` / `IOSlice` adapters (no binding
   resource implements `IOBase` yet). The raw `RawIOCursor` / `RawIOSlice`
   adapters, though generic in the core, **are** replicated — as concrete per-buffer
   wrappers (`ByteBufferCursor`, `ByteBufferSlice`, and the `BitBuffer` variants). Any
   such omission is stated in **both** binding module docs and on the docs site, so
   "not replicated" is always a documented, deliberate choice rather than drift.
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
   use the chunked streams — `pread_raw_io` / `pwrite_raw_io` by bytes, or
   `pread_typed_io` / `pwrite_typed_io` by items — rather than materializing whole
   copies.
7. **Value types have value semantics.** Every serializable value type (rule 5)
   also implements equality and hashing, so values compare by content and work as
   map/set keys — and the two agree with rule 5: two values are equal **iff** their
   `serialize_bytes()` are equal, and equal values hash equal. The Rust core derives
   `PartialEq` + `Eq` + `Hash` (add `PartialOrd` + `Ord` only where a total order is
   natural, e.g. widths/levels); the bindings mirror it, adapting to idioms — Python
   `__eq__` / `__hash__` (and `__reduce__` from rule 5), Node `equals()` /
   `hashCode()` (an `i32`, Java-style). The live/stream resources exempt from rule 5
   are exempt here too, since they carry no comparable value.
8. **The three source trees mirror each other.** The Rust core's module tree and
   both bindings' `src/` trees stay structurally identical: a concern is the
   **same-named module/namespace at the matching path** in `crates/<crate>/src/`,
   `bindings/python/src/`, and `bindings/node/src/`, so a reader who learns one
   language's layout knows the others'. The core may split a module into one file
   per type (including Rust-only traits); each binding carries the matching module
   holding exactly the concrete types it replicates (rule 4), and a module with only
   Rust-only items simply has no binding counterpart. A module is never added,
   renamed, or moved in one language without the same change in the others in the
   **same commit**. Published benchmark reports under `benchmarks/` mirror the source
   path of the code they measure (`benchmarks/<crate>/<area>/<type>.md`).
9. **Bypass the FFI copy whenever possible.** A binding must not copy bulk data
   across the boundary when it can hand over or fill memory in place. Prefer, in
   order: exposing the underlying allocation zero-copy (Python buffer protocol /
   `numpy.frombuffer`; Node external `Buffer`), then a **fill-into** method that
   writes into a caller-provided, reusable buffer with no per-call allocation
   (`pread_into(bytearray)` / `preadInto(Buffer)`), and only last a freshly-allocated
   return. Any bulk `pread`/`pwrite`-style op ships its allocation-free counterpart,
   and a benchmark proves the win. Where a zero-copy path is blocked (e.g. the Python
   buffer protocol is absent from the abi3 limited API), say so in the binding doc.
10. **`uv` is the Python toolchain — always, everywhere.** Every Python action, with
    no exception, goes through **`uv`**: creating the venv (`uv venv`), installing
    (`uv pip install`), **building the extension** (`uv run maturin develop` /
    `uv run maturin build`), running tests (`uv run pytest`), benchmarks, and docs
    (`uv run mkdocs …`). This is the default in scripts, CI, docs, examples, and the
    commands you run interactively — a bare `pip`, `python`, `maturin`, `pytest`, or
    `mkdocs` invocation must not appear anywhere. If `uv` itself is missing, install
    it (rule 11) before proceeding.
11. **Provision missing dependencies; never fail on them.** When a build, test, or
    tool step fails because something is absent — `uv`, a C compiler, CMake/Ninja, a
    Python/Node package, a system library, a cargo target — **install or configure it
    and retry**, don't report failure. That is what
    [`scripts/setup-build-deps.py`](scripts/setup-build-deps.py) and the CI setup
    steps already do; extend them rather than adding a manual "install X first" step.
    Prefer, in order: the project's own setup helper, then the platform manager
    (`uv pip`, `winget` / `apt` / `brew`), then a pinned config
    (`.cargo/config.toml`) so the fix is reproducible. Setup helpers and `build.rs`
    auto-provision what they can; when they genuinely cannot (no network, needs
    admin), they exit with the **exact** command to run, never an opaque error.
12. **Error messages guide the fix — in all three languages.** Every error a user
    can hit states, whenever the remedy is simple or knowable, **how to resolve it**,
    not merely what went wrong: name the missing feature/dependency *and* its
    enable/install command (`` enable the `gzip` cargo feature ``; ``run
    `uv run maturin develop` ``), the expected input/range *and* the offending value
    (`expected 0..=9, got 12`; `unknown mode "rw+"`), or the concrete next step. Never
    surface a bare or opaque message. The Rust core is the source of truth — its
    `Display` carries the guidance — and the bindings pass that text through unchanged
    (Python `ValueError`/`TypeError`, Node thrown `Error`), so all three read
    identically.
13. **Interpreted bindings infer the type — the caller need not spell it out.** In
    the dynamically-typed bindings (Python, Node), a typed operation that the Rust
    core reaches through an explicit generic (`IOBase<T>` element writes, typed
    buffer/scalar construction) instead **infers the element type from the runtime
    value** and builds the matching buffer/scalar automatically: a Python `int` in
    `int64` range or a JS `number`/`bigint` selects the correct integer width, a
    `float`/`number` the correct float, a `bytes`/`Buffer` the byte buffer, a `bool`
    the bit buffer — so `write(value)` and `buffer(values)` just work without the
    caller naming `Int64` first. Inference is a **convenience layer over**, never a
    replacement for, the explicit-type API: every inferring call has an explicit
    counterpart (`write_i64` / `writeI64`, `Int64Buffer(...)`) the user can reach for
    when a value is ambiguous (e.g. forcing `int32` for a small `int`) or when the
    inferred choice would be wrong, and an ambiguous or out-of-range value raises the
    rule-12 guided error naming the explicit method to call. The inference lives in
    the binding layer only (Rust stays explicitly generic, rule 2); the two bindings
    infer **identically** — same value-to-type mapping, same widths, same overflow
    boundaries — and each binding's module doc and the docs site state the mapping
    table so the auto-typing is a documented contract, not a surprise.

## Workspace layout

One crate per layer; dependencies point strictly downward (a lower layer never
imports an upper one — needing the reverse means the abstraction belongs lower).

- `crates/yggdryl-core` — foundations (streaming byte-IO, shared error types).
  **Apache Arrow's buffers are the core's data holder, not an optional layer**:
  `ByteBuffer`/`ByteCursor` are backed by an Arrow `Buffer`/`MutableBuffer`, the
  bit buffer by an Arrow `BooleanBuffer`, and the wide integers by `arrow_buffer::i256`;
  `arrow-buffer` is therefore a hard (non-optional) dependency, so
  `from_arrow_byte_buffer`/`to_arrow_byte_buffer` (and the bit-buffer equivalents)
  hand the allocation across zero-copy. The compression codecs (`gzip`, `zstd`) remain
  optional cargo features, **on by default** so a plain dependency ships the full
  surface; a `--no-default-features` build drops only those codecs.
- `crates/yggdryl-http` — a dependency-free, generic header map: `Headers` (an ordered
  bytes→bytes map with byte + UTF-8 string accessors, zero-copy in-place value mutation
  via `get_mut`, pre-built `name`/`comment`/`content-type`/`content-encoding` accessors,
  and a deterministic byte codec) plus the `HeadersBased` trait a header-carrying type
  implements. A `Field` and a buffer carry optional `Headers` as their annotations —
  yggdryl-side only, never written into Arrow's `Field`.
- `crates/yggdryl-dtype`, `crates/yggdryl-field`, `crates/yggdryl-scalar` — the
  Arrow data-model layers (data types, then fields, then scalars), one concern per
  crate so the concrete types share one naming convention across the layers
  (`yggdryl_dtype::I64Type` / `yggdryl_field::I64Field` / `yggdryl_scalar::I64Scalar`).
  A `Field` carries optional `yggdryl_http::Headers` via the `HeadersBased` supertrait.
  (Type names use the short primitive form — `I8`/`U8`/`F32`, not `Int8`/`UInt8`/`Float32`
  — matching the buffers.)
- `crates/yggdryl-buffer` — the typed, Arrow-backed buffers (`I8Buffer` … `F64Buffer`,
  `BooleanBuffer`). It sits **above** field (buffer → field → dtype → core, and depends on
  `yggdryl-http`) so each buffer carries optional `Headers` and hands out its matching
  typed field via `buffer.field(name, nullable)` (`I64Buffer` → `I64Field`); the headers
  are an annotation that does not affect the buffer's byte-content equality. It depends on
  `yggdryl-core` for the io cursors and `arrow-buffer` for the backing store. Core does
  **not** re-export the buffers (that would cycle), so the bindings import them from
  `yggdryl_buffer`.
- Higher layers (logical types, nested types, kernels) and service crates
  (e.g. HTTP) are added as further workspace members, each depending only on the
  layers below it.
- `bindings/python/` (PyO3/maturin) and `bindings/node/` (napi-rs) are **thin
  wrappers**: they translate types/errors and delegate — each method is one or two
  lines calling `self.inner`, `pub(crate)` so sibling modules can convert — and
  contain no logic. Each Rust crate is a submodule of the top-level package
  (`yggdryl-core` → `yggdryl.core`): Python via `sys.modules`; Node via
  `#[napi(namespace = "…")]` where class names are unique, and via the
  hand-written `yggdryl.js` / `yggdryl.d.ts` namespace map over uniquely-prefixed
  native classes (`DtypeI64Type` → `yggdryl.dtype.I64Type`) where they are not — napi
  registers class constructors by JS class name in one addon-global registry, so
  same-named classes across namespaces would collide. The binding source mirrors
  the crate tree (`src/<crate>.rs` or `src/<crate>/` per crate, `lib.rs` wiring
  only). Use `#[pyo3(signature = ...)]` / napi `Option<T>` for defaults.

## Dependencies

Minimal and pinned in the workspace `Cargo.toml`. For Arrow, use only the subset
crates actually needed (`arrow-schema`, `arrow-buffer`, `arrow-array`) — never the
full `arrow` umbrella or `arrow-flight`. Any new dependency carries a code comment
justifying it. Never pull a heavy SDK into a crate that should not depend on it.

## Errors and docs

- One error `enum` per type implementing `Display` + `std::error::Error`, with
  `From` conversions between layers; core errors map to `ValueError` (Python) /
  thrown `Error` (Node).
- **Error messages guide the fix** (rule 12): name the remedy — the missing feature
  (``enable the `gzip` cargo feature``), the expected input *and* the offending value
  (`expected 0..=9, got 12`), the offending value (`unknown mode "rw+"`) — never an
  opaque message, and identical across Rust, Python, and Node.
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

## Benchmarks

Performance-relevant behaviour carries a **throughput benchmark in all three
surfaces**, kept coherent exactly as tests and docs are (rule 4 — the Rust core, the
Python binding, and the Node binding move together). The Rust core bench
(`crates/<crate>/benches/`), the Python script (`bindings/python/benchmarks/`), and
the Node script (`bindings/node/benchmark/`) measure the **same corpus, sizes,
parameters (e.g. compression levels), and operations**, and print the **same shape**
of MB/s output, so the three read side by side. The two binding benchmarks
additionally weigh `yggdryl` against the platform's native codec (Python stdlib,
Node built-ins). A benchmark change is not done until its counterparts in the other
two languages land in the **same commit**. Benchmarks are always built in
release — `cargo bench`, `maturin develop --release`, `napi build --release` —
because a debug extension is ~20× slower and its numbers are meaningless; run them on
demand (they are **not** part of the required pre-commit gate).

**Optimize for both time and memory, and prove it.** A performance-relevant change
states its intended win on **both axes** — throughput (MB/s) *and* memory (bytes
allocated / copied, peak footprint) — and the benchmark measures whatever the change
claims: prefer the allocation-free / zero-copy / in-place path (rules 6 & 9), and when
a benchmark shows a path is allocation- or copy-heavy, note it and the batched or
fill-into alternative in the report. Every published report ends with an
**optimization history** — the wins the benchmark surfaced — so it doubles as a
performance changelog; a change that regresses time or memory is not done until the
regression is fixed or explicitly justified there.

## Required checks before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test
(cd bindings/python && uv run maturin develop && uv run pytest)
(cd bindings/node && npm run build && npm test)
uv run mkdocs build --strict   # when docs/ or mkdocs.yml changed (uv pip install mkdocs-material)
```

All must pass. Then do a final **coherence pass** — required, not optional polish:

1. No redundancy — fold duplicated logic into one place; don't add an API that
   restates another.
2. Cross-language parity — same surface and semantics in the core and both
   bindings.
3. One concern per file, in the right crate/module, mirroring its neighbours.
4. Readability — every public item documented, matching its neighbours.
5. Docs in sync — the matching page reflects the change, with synced language tabs.
6. Benchmarks in sync — a perf-relevant change updates the Rust, Python, and Node
   benchmarks together, each measuring the same thing comparably (see **Benchmarks**).

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
