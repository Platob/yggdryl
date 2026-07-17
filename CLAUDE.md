# yggdryl — contributor & agent instructions

yggdryl is a Rust library with **Python (PyO3/maturin)** and **Node (napi-rs)** extensions.
Features are implemented in the Rust core first and mirrored, thinly, in both bindings.

## Current scope — one abstract byte/memory-access layer, many sources

The core is a **minimal, dependency-free** foundation focused on **byte / memory access**: a
single abstract I/O contract (`memory` — the `IOBase` / `IOCursor` / `IOSlice` / `Whence` traits
and `IoError`) that many concrete **sources** implement. Today the in-heap [`Heap`] is the one
source; a memory-mapped backing and other sources (network, compressed, …) plug in against the
**same traits**. Alongside it is the `uri` family (`Uri` / `Url` / `Authority`) that **addresses**
those sources. Everything reads and writes through the one contract, so a new source is written
once and works everywhere.

The aim scales up from here: **absorb bytes from as many sources as possible behind this single
contract now**, then grow **typed data serialization** on top later — a precise internal type
system, columns, and Arrow interop over these bytes — so ingestion is broad at the edge, the
representation is exact underneath, and everything downstream is fast. That fuller typed layer
was prototyped and now lives in **git history**; it is the north-star to rebuild toward, not the
present surface. `arrow` is **not** a current dependency.

## Layout

- `crates/yggdryl-core` — the Rust core, the **single source of truth**. Currently the `memory`
  (abstract byte-access traits + the `Heap` source) and `uri` modules, at the crate root; **no
  external dependencies**.
- `bindings/python` — PyO3 extension, Python module `yggdryl` (built with **maturin**).
- `bindings/node` — napi-rs extension, npm package `yggdryl` (built with **napi**).
- `docs/` + `mkdocs.yml` — the MkDocs (Material) site published to
  `https://platob.github.io/yggdryl/`. `benchmarks/` — time+memory bench reports.
- `.github/workflows/` — `ci.yml` (fmt/clippy/test + strict docs build), `docs.yml`
  (publishes the site to GitHub Pages), `release.yml` (version-bump-gated publish to
  crates.io / PyPI / npm).

**Mirror the core's module layout in the bindings and their tests/benches.** The core is organised
into modules (`memory`, `uri`); each binding's `src/`, test suite, and benchmarks are split the
same way (a `memory` unit and a `uri` unit), so a reader finds the same shape in all three
languages. Minimal example: `yggdryl_core::version()` → `yggdryl.version()` in **both** bindings.

## Adding a feature — the three languages move together

1. **Core first.** Implement in `yggdryl-core` with a `///` doc comment, a runnable
   **doctest**, and a unit test. All logic lives here.
2. **Thin bindings.** Mirror it in **both** extensions — each method is 1–2 lines
   delegating to `yggdryl_core`, **no logic in the binding**. Adapt only to idioms: Python
   dunders / keyword defaults; Node camelCase / `Option<T>` defaults. Error text passes
   through unchanged — the core `Display` becomes a Python `ValueError` and a Node thrown
   `Error`, reading identically.

   **Make the Python type behave like a native Python value — implement every idiomatic
   dunder the concept supports**, not just the minimum. A value type gets `__eq__`, `__hash__`
   (only if immutable/hashable — a mutable one leaves `__hash__` unset so it is unhashable like
   `dict`/`bytearray`), `__repr__`, `__str__` (when there's a canonical string), and `__reduce__`
   so it **pickles** (reconstruct via the class ctor + args, or the `deserialize_bytes` codec —
   see `uri::Uri`). Beyond that, implement the relevant *protocols*: a container is `__len__` +
   `__bool__`; a map is also `__contains__` / `__getitem__` / `__setitem__` / `__delitem__` /
   `__iter__` (like `dict`); a sequence/buffer is `__getitem__` (int **and** slice) + `__iter__`
   + `__bytes__`; anything with a `copy()` also gets `__copy__` / `__deepcopy__`. The Node side
   has no dunders — mirror the same capability as named methods (`equals`, `toString`,
   `get`/`set`/`has`, `toBytes`, …).
3. **Test in all three.** Add a test on each surface; the three suites are the executable
   proof the APIs match method-for-method. A binding-visible change updates **both**
   bindings and their tests in the **same commit**.
4. **Document & measure.** Add or extend a `docs/<feature>.md` page with synced
   `=== "Python"` / `=== "Node"` / `=== "Rust"` tabs and list it in `mkdocs.yml` nav —
   `mkdocs build --strict` must stay green. For a performance-sensitive type, add a
   time+memory benchmark and a deterministic allocation check (see `benches/heap.rs`,
   `tests/memory_heap_alloc.rs`, and the report under `benchmarks/yggdryl-core/`).

## Coding rules

- **Explicit type names in the Rust core; generic, type-inferring entry points in the bindings.**
  A core builder / typed accessor **names the concrete type it works over** —
  `parse_str`, `pread_i32` / `pwrite_i32`, `pread_byte` / `pread_byte_array`, `read_u64`,
  `from_bytes` — never an overloaded bare `parse` / `read` that hides which representation it
  takes. The bindings then expose **one generic method per concept** (`parse`, `read`, `from`)
  that **infers the input's runtime type** (Python / Node duck typing) and **redirects to the
  matching explicit core builder** — so a caller writes `Uri.parse(x)` / `heap.read(x)` and the
  binding dispatches (`str` → `parse_str`, `bytes` → `deserialize_bytes`, an integer width → the
  typed reader). Explicit and unambiguous underneath; ergonomic and generic on top.
- **No lifetime parameters on public types** — the bindings must be able to hold every one.
- **Coherent layering — the contract at the module root, sources below.** The family-agnostic
  contract (the `IOBase` / `IOCursor` / `IOSlice` traits, `Whence`, `IoError`) lives at the
  `memory` root; each concrete **source** (`Heap`, a future `Mmap`) is one file implementing those
  traits and adding only its own inherent methods (constructors, capacity). A source depends
  **downward** on the root traits and never sideways on a sibling source. Prefer default trait
  methods so a new source implements the few primitives and inherits the rest.
- **Value types are hashable, serializable, and equatable — everywhere.** Whenever a public
  type carries a *value* (not just an identity), implement all three on it and mirror them in
  **both** bindings, so it works as a map/dict key, in a set, and over a wire in every language:
  - *Rust core:* `PartialEq`/`Eq`, `Hash`, and a byte codec `serialize_bytes` /
    `deserialize_bytes` (the exact inverse).
  - *Python:* `__eq__`, `__hash__`, `__reduce__` (pickle), and the same byte codec.
  - *Node:* `equals`, `hashCode`, `serializeBytes` / `deserializeBytes`.

  Keep **one identity: equal iff canonical bytes equal, and equal values hash equal.** Build the
  canonical form **once into a pre-sized buffer** (`String::with_capacity(encoded_len())`) and
  **stream it into the hasher** with a zero-alloc `fmt::Write` adapter, so equality and hashing
  add no per-op allocation (see `uri::Uri` / `uri::HashWrite`).
- **Ergonomic immutable updates — `copy` + `with_*`.** Whenever a public value type is worth
  mutating, give it a **`copy`** (the cross-language name for a clone) and a **`with_<field>`**
  builder for every settable field — each returns a fresh value so callers get one-line,
  chainable, non-mutating updates (`base.with_host("h").with_port(443)`). Keep the in-place
  `set_<field>` too; `with_*` is the 1-line-friendly wrapper over it. Where combining two whole
  values reads naturally, add a **`merge_with(other)`** overlay and any domain combinator
  (`joinpath` for a path). Mirror these thinly in **both** bindings (see `uri::Uri` /
  `uri::Authority`).
- **At-most-one-copy discipline.** Prefer zero-copy hand-off; a bulk op ships an allocation-free
  *fill-into* / *read-into* counterpart (`pread_into`, `pread_vec`); **no allocations in hot
  loops**. Maintain capacity like `Vec` (`with_capacity` / `capacity` / `reserve`) so a growing
  source amortises its allocations. When a change claims a performance win, **prove it** — a
  benchmark on both time and memory, plus a deterministic allocation test.
- **One file per public type.** A reader should not tell two types apart by the *shape* of the
  code — mirror the nearest neighbour's structure, naming, error style, and doc style.
- **Minimize `Option`.** Reach for `Option<T>` only when *absence is a real, distinct state a
  caller must handle* (a genuinely nullable value, a lookup that can miss). Prefer a total
  function with a sensible default, an **empty** collection/string over `Option<Vec<_>>` /
  `Option<String>`, two named methods over an `Option<bool>` flag, and a **guided `Result`** when
  absence is really an error the user should be told how to fix. Each `Option` in a public
  signature must justify its existence.
- **Guided errors.** Every error a user can hit names how to fix it (the expected range *and* the
  offending value, or the next step) — never an opaque message. Same text across Rust, Python,
  and Node.
- Mark underdetermined decisions with a `// DESIGN:` comment.

## Toolchain (this environment is Windows)

- cargo at `%USERPROFILE%\.cargo\bin` (on the PowerShell PATH); node at
  `C:\Program Files\nodejs`. Use **`uv`** for every Python action (venv, build, test).

## Gate before committing

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test                                    # default-members = core only (no Python/Node headers)
(cd bindings/python && uv run maturin develop && uv run pytest)
(cd bindings/node && npm run build && npm test)
uv run --no-project --with mkdocs-material mkdocs build --strict   # docs check
```

All must pass. Work on a **branch**; commit/push only when asked.

**Releasing** is by version bump: `release.yml` runs on every push to `main`, and whenever
`[workspace.package].version` has **no matching `v<version>` tag** it publishes to
crates.io / PyPI / npm and creates the `v<version>` tag + GitHub Release. So bump the
version **only** when you intend to release; keep it pinned during ordinary changes so the
auto-publish never fires mid-change.
