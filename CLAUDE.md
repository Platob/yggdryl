# yggdryl — contributor & agent instructions

yggdryl is an **Apache Arrow-backed** Rust library with **Python (PyO3/maturin)** and
**Node (napi-rs)** extensions. This is a minimal foundation; features are implemented in
the Rust core first and mirrored, thinly, in both bindings.

## Layout

- `crates/yggdryl-core` — the Rust core, the **single source of truth**. Physical layer:
  `arrow-buffer`.
- `bindings/python` — PyO3 extension, Python module `yggdryl` (built with **maturin**).
- `bindings/node` — napi-rs extension, npm package `yggdryl` (built with **napi**).
- `docs/` + `mkdocs.yml` — the MkDocs (Material) site published to
  `https://platob.github.io/yggdryl/`. `benchmarks/` — time+memory bench reports.
- `.github/workflows/` — `ci.yml` (fmt/clippy/test + strict docs build), `docs.yml`
  (publishes the site to GitHub Pages), `release.yml` (version-bump-gated publish to
  crates.io / PyPI / npm).

Minimal example: `yggdryl_core::version()` → `yggdryl.version()` in **both** Python and Node.

## Adding a feature — the three languages move together

1. **Core first.** Implement in `yggdryl-core` with a `///` doc comment, a runnable
   **doctest**, and a unit test. All logic lives here.
2. **Thin bindings.** Mirror it in **both** extensions — each method is 1–2 lines
   delegating to `yggdryl_core`, **no logic in the binding**. Adapt only to idioms: Python
   dunders / keyword defaults; Node camelCase / `Option<T>` defaults. Error text passes
   through unchanged — the core `Display` becomes a Python `ValueError` and a Node thrown
   `Error`, reading identically.
3. **Test in all three.** Add a test on each surface; the three suites are the executable
   proof the APIs match method-for-method. A binding-visible change updates **both**
   bindings and their tests in the **same commit**.
4. **Document & measure.** Add or extend a `docs/<feature>.md` page with synced
   `=== "Python"` / `=== "Node"` / `=== "Rust"` tabs and list it in `mkdocs.yml` nav —
   `mkdocs build --strict` must stay green. For a performance-sensitive type, add a
   time+memory benchmark and a deterministic allocation check (see `benches/uri.rs`,
   `tests/uri_alloc.rs`, and the report in `benchmarks/yggdryl-core/uri.md`).

## Optimized coding rules

- **Arrow is the physical layer**; **never** expose `arrow-rs` types in a public signature.
- **No lifetime parameters on public types** — the bindings must be able to hold every one.
- **At-most-one-copy discipline.** Prefer zero-copy hand-off; a bulk op ships an
  allocation-free *fill-into* counterpart; **no allocations in hot loops**. When a change
  claims a performance win, prove it (a benchmark on both time and memory).
- **Value types are hashable, serializable, and equatable — everywhere.** Whenever a public
  type carries a *value* (not just an identity), implement all three on it and mirror them in
  **both** bindings, so it works as a map/dict key, in a set, and over a wire in every
  language:
  - *Rust core:* `PartialEq`/`Eq`, `Hash`, and a byte codec `serialize_bytes` /
    `deserialize_bytes` (the exact inverse).
  - *Python:* `__eq__`, `__hash__`, `__reduce__` (pickle), and the same byte codec.
  - *Node:* `equals`, `hashCode`, `serializeBytes` / `deserializeBytes`.

  Keep one identity: **equal iff canonical bytes equal, and equal values hash equal.** Build
  the canonical form **once into a pre-sized buffer** (`String::with_capacity(encoded_len())`)
  and **stream it into the hasher** with a zero-alloc `fmt::Write` adapter — so equality and
  hashing add no per-op allocation (see `io::Uri` / `io::HashWrite`).
- **One file per public type.** A reader should not tell two types apart by the *shape* of
  the code — mirror the nearest neighbour's structure, naming, error style, and doc style.
- **Guided errors.** Every error a user can hit names how to fix it (the missing feature +
  its enable command, the expected range *and* the offending value, or the next step) —
  never an opaque message. Same text across Rust, Python, and Node.
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
