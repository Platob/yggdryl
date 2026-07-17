# yggdryl — contributor & agent instructions

yggdryl is a Rust library with **Python (PyO3/maturin)** and **Node (napi-rs)** extensions.
Features are implemented in the Rust core first and mirrored, thinly, in both bindings.

> **Current state (reset).** The core was reset to a **minimal, dependency-free foundation**:
> the abstract memory-access layer (`memory` — the `IOBase` / `IOCursor` / `IOSlice` / `Whence`
> traits, `IoError`, and an in-heap `Bytes` backing) and the `uri` family (`Uri` / `Url` /
> `Authority`). The bindings expose `version()` + `uri`. Everything else described below (the
> typed `io` data layer, the `fixed` / `var` / `nested` / `decimal` / `temporal` families, ops,
> Arrow interop) is the **design north-star to rebuild toward**, not the present surface — the
> prior, fuller implementation lives in git history if a pattern needs reference. `arrow-buffer`
> is **not** a current dependency; the concrete byte backings (heap `Buffer`, mmap) are rebuilt
> against the `memory` traits.

## Project aim — absorb anything, type it once, optimize everything after

yggdryl aims to **ingest as many data types as possible** and to **absorb loosely-typed /
unstructured input at the edge**, assigning each value its **exact internal type in the first
layers** (the `io` type system — `DataTypeId` and the typed `fixed` / `var` / `nested` /
`decimal` / `temporal` families). Once data has crossed that boundary it is **correctly and
precisely typed**, so everything downstream — storage, the byte codec, Arrow interop, compute,
the bindings — operates over a **closed, known type space** and can be **fully optimized**:
zero-copy hand-off, no per-op re-inspection, no defensive re-parsing in the hot path. The
widening / normalization / type-inference happens **once, at ingestion**, never afterwards.
So: prefer **adding a modeled type** (or a lossless / documented closest-fit mapping — see the
`to_arrow` rules below) over leaving data opaque, accept input in **every reasonable native
form** (a value, a scalar, a native-language type, a string, raw bytes, an Arrow array) and
converge it to the one internal representation, and give every type the **full factory surface**
(`from_values` / `from_options` / `from_scalars` / native-language constructors) so callers can
feed data however they hold it. Breadth of ingestion at the top; a single precise type
underneath; speed everywhere after.

## Layout

- `crates/yggdryl-core` — the Rust core, the **single source of truth**. Currently the `memory`
  (abstract byte-access traits + heap `Bytes`) and `uri` modules, at the crate root; **no external
  dependencies**.
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

   **Make the Python type behave like a native Python value — implement every idiomatic
   dunder the concept supports**, not just the minimum. A value type gets `__eq__`, `__hash__`
   (only if immutable/hashable — a mutable one leaves `__hash__` unset so it is unhashable like
   `dict`/`bytearray`), `__repr__`, `__str__` (when there's a canonical string), and `__reduce__`
   so it **pickles** (reconstruct via the class ctor + args, or the `deserialize_bytes` codec —
   see `io::Uri`). Beyond that, implement the relevant *protocols*: a container is `__len__` +
   `__bool__`; a map is also `__contains__` / `__getitem__` / `__setitem__` / `__delitem__` /
   `__iter__` (like `dict`); a sequence/buffer is `__getitem__` (int **and** slice) + `__iter__`
   + `__bytes__`; anything with a `copy()` also gets `__copy__` / `__deepcopy__`; a totally
   ordered value gets `__lt__` … (rich comparison); an int-like enum gets `__int__`. The Node
   side has no dunders — mirror the same capability as named methods (`equals`, `toString`,
   `get`/`set`/`has`, `toBytes`, …).
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
- **Closest-Arrow fallback — `to_arrow` is total; zero-copy is a capability, not a requirement.**
  A type's `to_arrow()` / `arrow_data_type()` always returns *some* `arrow_schema::DataType`: the
  **exact** primitive when Arrow has one, else the **closest optimized representation** —
  `Decimal128(38,0)` / `Decimal256(76,0)` for wide *signed* integers (a scale-0 decimal is an
  integer), `FixedSizeBinary(N)` for a width Arrow cannot model (`u128`, `u96`/`i96`, `u256`,
  fixed-size utf8), `Float16` for `f16`. Document the mapping as **lossy** where it is
  (`FixedSizeBinary` drops the utf8/integer tag; a scale-0 `Decimal` under-covers the top of the
  integer's range) with a `// DESIGN:` note. Define the mapping **once**, centralized on the id
  (`DataTypeId::to_arrow` / `from_arrow`), so the erased and typed descriptors share it and it
  stays total across the whole type space. **Zero-copy `PrimitiveArray` interop is gated on a
  capability sub-trait** (`ArrowNative`, implemented only for types with a real
  `ArrowPrimitiveType`); a type without it is still a first-class value (full codec,
  `Buffer`/`Serie`, serialization) — it just lacks the shared-`Arc` Arrow round-trip. Never route
  an *integer* through a decimal *array* (semantically wrong), and never key an alignment/realign
  decision off a wrapper's own alignment — use the Arrow native's.
- **Carry the exact logical type in field metadata — a lossy schema mapping must still round-trip.**
  Because `to_arrow` is lossy and *non-injective* (`u96`/`i96`/`FixedUtf8`/… all collapse to
  `FixedSizeBinary(N)`), a `Field` reconstructed by `from_arrow` would otherwise be a *guess*. So
  a field carries string key/value metadata as [`Headers`](crates/yggdryl-core/src/io/headers.rs) —
  the **single, centralized** metadata holder (there is no separate `Metadata` type; `Headers` is
  the one map used for HTTP headers *and* schema metadata, mirroring Arrow's `Field::metadata`) — and
  `to_arrow` records the exact type name under a reserved key (`DataTypeId::METADATA_KEY`) — but
  **only when the plain mapping can't be reversed to it** (exact primitives, and the
  `Decimal`-backed ints that reverse unambiguously, add no tag). `from_arrow` prefers that key to
  recover the precise type, strips it from the user-visible metadata, and **falls back to the safe
  base** (`FixedSizeBinary` → `fixed_binary`, never a guessed wide integer) when it is absent.
  Arrow carries unknown metadata keys through IPC/Parquet, so the discriminator survives external
  round-trips. Every field type (erased + typed) preserves metadata through `erase`/`clone`/`eq`.
- **Bit-canonical value identity — never derive `Eq`/`Hash` off a float.** A value type's
  identity (`PartialEq`/`Eq`/`Hash`) and its `serialize_bytes` are all over the **same canonical
  little-endian bytes**, so they can never disagree. This is mandatory for the float types
  (`f16`/`f32`/`f64`): their `==` is IEEE (`NaN != NaN`, `+0.0 == -0.0`) and they are neither
  `Eq` nor `Hash`, so a `#[derive]` over `T` silently drops `Eq`/`Hash` (breaking "hashable
  everywhere") **and** disagrees with the byte codec. Implement `Scalar`/`Serie` identity from
  `write_le` bytes instead; the result is bit-canonical (`NaN == NaN` by pattern, `+0.0 != -0.0`)
  — document that it deliberately diverges from IEEE `==`. For a fixed-width little-endian integer
  newtype, byte-wise `Eq`/`Hash` is exact (equal value ⇔ equal bytes, negatives included) but
  **omit `Ord`/`PartialOrd`** — little-endian byte order is *not* numeric order.
- **Centralize the type space in one ranged integer id.** The concrete types are enumerated in
  exactly one `#[repr(u16)]` `DataTypeId`, laid out so each category is a **contiguous integer
  range** with reserved gaps for future types; every `is_*` predicate is one/two `u16` range
  checks (no `match`, no `category()` on the hot path), and the coarse `DataTypeCategory` derives
  from it. Width is an id-range property, **not** a category one (a fixed-size binary and a var
  binary share a category but differ in width). Keep each predicate a **bounded** range (never an
  open `>=`, or a future higher category is silently misclassified), lock the load-bearing
  adjacencies with `const` asserts, and decode a `u16` back to an id via a **checked match**
  (`from_u16 -> Option`), never a transmute over a reserved gap.
- **No lifetime parameters on public types** — the bindings must be able to hold every one.
- **At-most-one-copy discipline.** Prefer zero-copy hand-off; a bulk op ships an
  allocation-free *fill-into* counterpart; **no allocations in hot loops**. When a change
  claims a performance win, prove it (a benchmark on both time and memory).
- **In-place mutation — COW-backed `*_mut` / `*_assign` twins; copy is shallow by default.** The
  physical buffers are `Arc`-shared and **copy-on-write**, so `clone` / `copy` is a cheap `Arc`
  bump — **never** a payload copy — and a mutation copies the buffer **once, on write, only if it
  is shared**. A heavy independent payload copy happens **only** on an explicit deep copy
  (`deep_copy()` in the core, `copy(deep=true)` in the bindings); plain `copy()` / `copy(deep=false)`
  stays shallow. So a transform must **not** clone the payload — a value type is passed by `&` and
  mutated through the COW path. Every transform a caller might apply without keeping the original
  ships an **in-place mutating twin** next to its return-new form: the arithmetic `add_assign` /
  `sub_assign` / … (and the scalar broadcast), `fill_null_mut`, `retain` (in-place filter /
  compact), and the setters (`set` / `set_slice` / `set_child`, already in place). The `*_mut` twin
  mutates `self`'s buffer through copy-on-write — **zero allocation when the buffer is uniquely
  owned, one COW when shared** — so a hot loop of ops never allocates a fresh result per step.
  Prove it with a counting-allocator test (owned → 0 allocs, shared → exactly 1). Mirror the twins
  in **both** bindings (Python in-place dunders `__iadd__` / `__isub__` / … + `fill_null_mut` /
  `retain` / `copy(deep=)`; Node camelCase `addAssign` / … + `copy(deep)`).
- **Auto-vectorization — write the numeric hot loops so the compiler emits SIMD.** The
  arithmetic kernels and reductions run on **stable** Rust (no `portable_simd`/nightly, no SIMD
  dependency), so vectorization comes from **LLVM auto-vectorizing a clean loop**. Structure the
  hot path for it: iterate a **contiguous `&[T]`** (the values [`Buffer`], not per-index `get`),
  compute the values **densely and branch-free** over the whole slice (integer arithmetic
  `wrapping_*`; a null slot's placeholder participates harmlessly), and handle **validity
  separately** as a word-at-a-time bitmap combine (`self.validity AND other.validity`) — never a
  per-element `if null` inside the value loop (a branch there defeats the vectorizer). The
  `*_unchecked` tier is where this lives; keep it a single straight `for i in 0..n` (or an
  iterator `zip`) over equal-length slices with the result written into a pre-sized buffer. Prove
  it: a benchmark showing the speedup, and — where it's load-bearing — a note that the loop
  vectorizes (inspect the asm / that removing the inner branch moved the needle). Reductions
  (`sum`/`min`/`max`) fold over the same contiguous slice.
- **Two-tier ops — a fast `*_unchecked` under a checking + casting base.** Every vectorized
  operation (arithmetic between series, a scalar broadcast, a filter, a fill) comes in two
  tiers. The **base** (`add`, `sub`, `filter`, …) is the safe default a caller reaches for: it
  **validates** (length match, mask length) and **casts** the operands to the result type
  (**the result follows the left operand — the right is cast into `self`'s type**). Per the
  "absorb anything" aim it **tries every available conversion** — the whole `Converter` surface,
  numeric *and* the utf8/binary/decimal/temporal bridges — so a convertible operand of *any* type
  (a numeric-string column, a `bool`, a decimal, a wide integer) is coerced in rather than
  rejected; it errors **only** when a value genuinely cannot convert (a non-numeric string into a
  number, an out-of-range magnitude) — always with a guided message. It propagates nulls and
  returns a `Result`. It then
  delegates to a **`*_unchecked`** twin (`add_unchecked`, …) — the tight, allocation-minimal,
  **infallible** hot path that *assumes* normalized inputs (identical element type + width, equal
  length; a scalar already the element type) and only `debug_assert!`s them. So the checks and
  casts happen **once** at the boundary and the inner loop is branch-lean (integer arithmetic
  **wraps**, like Arrow/NumPy; integer div/rem by zero yields a **null**, never a panic;
  documented). Mirror **both** tiers in the core and expose the ergonomic base in both bindings
  (Python operator dunders `__add__`/`__radd__`/… over a serie *or* a native scalar; Node named
  `add`/`sub`/… methods). A nested op recurses to leaves (struct field-wise, list element-wise on
  matching shape, map over values); a temporal op routes through its **backing integer** so
  `date + date` and `timestamp − timestamp` fall out of the same path.
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
- **Ergonomic immutable updates — `copy` + `with_*`.** Whenever a public value type is
  worth mutating, give it a **`copy`** (the cross-language name for a clone) and a
  **`with_<field>`** builder for every settable field — each returns a fresh value so callers
  get one-line, chainable, non-mutating updates (`base.with_host("h").with_port(443)`) instead
  of clone-then-set boilerplate. Keep the in-place `set_<field>` too; `with_*` is the
  1-line-friendly wrapper over it. Where combining two whole values reads naturally, add a
  **`merge_with(other)`** overlay (each field `other` sets wins) and any domain combinator
  (`joinpath` for a path). Implement these in the core and mirror them, thinly, in **both**
  bindings — `copy` becomes Python's `.copy()` / Node's `.copy()`, and `with_*`/`merge_with`
  read identically in all three (see `io::Uri` / `io::Authority`).
- **One file per public type.** A reader should not tell two types apart by the *shape* of
  the code — mirror the nearest neighbour's structure, naming, error style, and doc style.
- **Coherent layering — generics at the root, families below, no sideways deps.** A
  family-agnostic contract lives at the *module root* it spans (`io::DataType`, `io::FieldType`,
  `io::ScalarType`, …, and a shared axis like `io::DataTypeCategory`) — **never** inside one
  concrete family. Each family (`io::fixed`, `io::var`) then adds only its **own** sub-traits
  (`Fixed*` / `Var*`) and concrete implementors, and depends **downward** on the root — a
  family must never import a base trait, enum, or helper from a *sibling* family (if `var`
  reaches into `fixed` for something shared, that something belongs one level up). Mirror the
  sibling family's file layout: one sub-trait + its concrete per file, so `var/scalar.rs` reads
  like `fixed/scalar.rs`. When a family legitimately lacks a peer the sibling has (there is no
  `VarBuffer` for `FixedBuffer`), omit it and leave a `// DESIGN:` line saying why — don't ship
  a dead scaffold.
- **Drill down with predicates, not `match`.** A type carrying a closed set of variants (a
  category, a kind) matches that set in **exactly one place** — a single categorizing method
  (`category()`) — and exposes `is_*` predicates that forward to it (`is_integer`, `is_utf8`,
  `is_fixed_width`, …), mirrored in both bindings. Callers then classify with one cheap,
  inlinable predicate instead of re-matching the whole space at every call site; adding a
  variant touches the one match, not every caller.
- **Minimize `Option`.** Reach for `Option<T>` only when *absence is a real, distinct state a
  caller must handle* (a genuinely nullable value, a lookup that can miss). Do **not** use it
  as a lazy stand-in — prefer a total function with a sensible default, an **empty** collection
  or string over `Option<Vec<_>>` / `Option<String>`, a dedicated null-object constructor
  (`Scalar::null()`) or two named methods over an `Option<bool>` flag argument, and a guided
  `Result` when absence is really an error the user should be told how to fix. An `Option`
  return that a caller almost always `.unwrap()`s is a smell — give them the total method. Each
  `Option` in a public signature should justify its existence; when in doubt, model the state
  explicitly rather than overloading `None`.
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
