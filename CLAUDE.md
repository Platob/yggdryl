# yggdryl — contributor & agent instructions

yggdryl is a Rust library with **Python (PyO3/maturin)** and **Node (napi-rs)** extensions.
Features are implemented in the Rust core first and mirrored, thinly, in both bindings.

## Scope — one abstract byte/memory-access layer, many sources

The core is the **`io` layer**: a single abstract byte-access contract (`io::memory::IOBase`,
with the concrete `IOCursor` / `IOSlice` wrappers and the in-heap `Heap` source), the root
`uri` family (`Uri` / `Url` / `Authority`) that **addresses** sources, the root `headers`
module (the one metadata map), and the cross-cutting value types at the `io` root (`IOMode`,
`IOKind`, `IoError`, `Whence`, the `Serializable` trait). Everything reads and writes through the one contract, so a new source (memory-mapped,
file, network, compressed, …) is written once and works everywhere.

From here the library scales up: absorb bytes from as many sources as possible behind this
single contract, then grow **typed data serialization** on top — a precise internal type
system, columns, and Arrow interop over these bytes — so ingestion is broad at the edge, the
representation is exact underneath, and everything downstream is fast. `arrow` is **not** a
current dependency.

## Layout — one tree, mirrored everywhere

```text
crates/yggdryl-core/src/             # the core (dependency-free by default; codecs opt-in)
  io/                                # the io layer
    mod.rs                           #   io root: cross-cutting contract + value types
    any.rs                           #   AnyIO + open() — the scheme-dispatching `open()` entry
    meminfo.rs                       #   MemoryInfo — capacity snapshot (RAM/disk/VRAM), one type
    error.rs  whence.rs              #   IoError, Whence (io-wide)
    serializable.rs                  #   the Serializable trait
    mode.rs  kind.rs                 #   IOMode, IOKind
    memory/                          #   byte-access: traits at the module root…
      base.rs cursor.rs slice.rs     #     IOBase (bytes + the graph surface) + wrappers
      heap.rs                        #     …the in-heap source
    local/                           #   the local-filesystem family
      io.rs                          #     LocalIO — the single access point (lazy, self-optimizing)
      mmap.rs                        #     the raw memory-mapped file LocalIO builds on
    gpu/                             #   device memory (feature `gpu`) — organized BY ARCHITECTURE:
      mod.rs device.rs               #     GpuMemory over IOBase + the by-arch device probe
      cpu.rs                         #     CpuHeap — device memory IS our Heap (host RAM)
      amd.rs  cuda.rs                #     AMD Radeon (gpu-amd, live detect) / NVIDIA (gpu-cuda)
  headers.rs                         # Headers — the one metadata map (root module)
  mimetype.rs                        # MimeType + MimeRegistry/MimeCatalog (root module)
  mediatype.rs                       # MediaType — an ordered MimeType list (root module)
  compression.rs                     # Compression trait + feature-gated Gzip/Zlib/Zstd/Lzma
  uri/                               # addressing (root module): Uri/Url/Authority/UriParts + scheme/percent
```

**The same folder tree is mirrored in code, tests, and benchmarks — in the core and in both
extensions.** This is a hard rule: a reader must find the same shape everywhere.

- *Core tests/benches* (flat by cargo's design) mirror by **path-derived names**:
  `src/io/memory/heap.rs` → `tests/io_memory_heap.rs` (+ `_alloc`) → `benches/io_memory_heap.rs`
  → `benchmarks/yggdryl-core/io/memory/heap.md`; `src/io/local/` → `tests/io_local_io.rs` +
  `tests/io_local_mmap.rs` (+ `_alloc`) → `benches/io_local_io.rs` + `benches/io_local_mmap.rs`
  → `benchmarks/yggdryl-core/io/local/{io.md,mmap.md}`; `src/uri/` → `tests/uri*.rs` →
  `benches/uri.rs` → `benchmarks/yggdryl-core/uri.md`; `src/headers.rs` → `tests/headers.rs`.
- *Bindings* mirror with **real folders**: `bindings/*/src/io/{memory.rs,local.rs,mod.rs,…}` +
  `bindings/*/src/{headers.rs,uri.rs}`,
  `bindings/python/tests/{io/test_memory.py,io/test_local.py,test_uri.py,test_headers.py}`,
  `bindings/node/test/{io/memory.test.js,io/local.test.js,uri.test.js}`, and the same under
  `benchmarks/` / `benchmark/`.
- *Public namespaces* mirror the **leaf modules identically in both bindings** —
  `yggdryl.memory`, `yggdryl.local`, `yggdryl.uri`, `yggdryl.headers`, and `yggdryl.io` for the
  io-root types — adapting only to
  platform nesting limits (napi namespaces are single-level, so both bindings stay flat and
  therefore identical).
- `docs/` pages mirror too (`docs/io/memory.md`, `docs/io/local.md`, `docs/uri.md`,
  `docs/headers.md`), each with synced
  `=== "Python"` / `=== "Node"` / `=== "Rust"` tabs, listed in `mkdocs.yml` nav.

Other top-level dirs: `.github/workflows/` — `ci.yml` (fmt/clippy/test + strict docs),
`docs.yml` (GitHub Pages), `release.yml` (version-bump-gated publish to crates.io/PyPI/npm).

## Adding a feature — the three languages move together

1. **Core first.** Implement in `yggdryl-core` with a `///` doc comment, a runnable
   **doctest**, and a unit test. All logic lives here.
2. **Thin bindings.** Mirror it in **both** extensions — each method is 1–2 lines delegating
   to `yggdryl_core`, **no logic in the binding**. Adapt only to idioms: Python dunders /
   keyword defaults; Node camelCase / `Option<T>` defaults. Error text passes through
   unchanged — the core `Display` becomes a Python `ValueError` and a Node thrown `Error`,
   reading identically.

   **Implement the most Python dunders the concept supports — always.** A value type gets
   `__eq__`, `__hash__` (only if immutable — a mutable one leaves `__hash__` unset, like
   `bytearray`), `__repr__`, `__str__` (when there's a canonical string), `__reduce__` so it
   pickles, and `__copy__` / `__deepcopy__` alongside `copy()`. Then every relevant
   *protocol*: a container is `__len__` + `__bool__`; anything map-like is `__contains__` /
   `__getitem__` / `__setitem__` / `__delitem__` / `__iter__` (like `dict` — including a type
   that *contains* a map, e.g. `Uri` over its params); a sequence/buffer is `__getitem__`
   (int **and** slice) + `__iter__` + `__bytes__`; an int-like enum is `__int__` + `__index__`.
   When in doubt, add the dunder. The Node side has no dunders — mirror the same capability as
   named methods (`equals`, `toString`, `get`/`set`/`has`/`delete`, `toBytes`, …).
3. **Test in all three.** Add a test on each surface; the three suites are the executable
   proof the APIs match method-for-method. A binding-visible change updates **both** bindings
   and their tests in the **same commit**.
4. **Document & measure.** Add or extend the mirrored `docs/io/<module>.md` page (synced
   three-language tabs; `mkdocs build --strict` stays green). For a performance-sensitive
   type, add a time+memory benchmark and a deterministic allocation check (see
   `benches/io_memory_heap.rs`, `tests/io_memory_heap_alloc.rs`, and the report under
   `benchmarks/yggdryl-core/`).

## Coding rules

- **Explicit type names in the Rust core; generic, type-inferring entry points in the
  bindings.** A core builder / typed accessor **names the concrete type it works over** —
  `parse_str`, `pread_i32` / `pwrite_i32`, `pread_byte_array`, `pread_utf8`, `IOMode::parse_str`
  — never an overloaded bare `parse` / `read` that hides which representation it takes. The
  bindings then expose **one generic method per concept** (`parse`, `read`, `copy`) that
  **infers the input's runtime type** and redirects to the matching explicit core builder.
- **Serializable, hashable, equatable — whenever possible.** Every public type that can carry
  a value implements the `io::Serializable` trait (`serialize_bytes` → `Vec<u8>` /
  `deserialize_bytes` — the exact inverse) plus `PartialEq`/`Eq` and `Hash` (skip `Hash` only
  for genuinely mutable buffers, which stay equatable by content). Mirror all three in **both**
  bindings — Python `__eq__` / `__hash__` / `__reduce__` (pickle) + the byte codec; Node
  `equals` / `hashCode` / `serializeBytes` / `deserializeBytes` — so every value works as a
  map key, in a set, and over a wire in every language. Keep **one identity: equal iff
  canonical bytes equal, and equal values hash equal**; build the canonical form once into a
  pre-sized buffer and stream it into the hasher (see `uri::Uri` / `HashWrite`).
- **Centralize metadata in `headers::Headers`.** There is exactly **one** metadata/annotation map
  type in the project: `Headers` (ordered, case-insensitive, multi-value, byte-capable). HTTP
  headers, schema/field metadata, source annotations — all of it is a `Headers`; never
  introduce a second map type or an ad-hoc `HashMap<String, String>` in a public signature.
  Every `IOBase` carries one (`headers()` / `headers_mut()`).
- **Least reallocation, fewest copies — in every action.** Prefer zero-copy hand-off; never
  clone what a borrow can serve; pre-size every buffer you build (`with_capacity` /
  `encoded_len`); a bulk op ships an allocation-free *fill-into* / *read-into* counterpart
  (`pread_into`, `pread_i32_array`); bulk kernels stage through **fixed stack chunks**, not
  per-call heap buffers; **no allocations in hot loops**. Constructors take capacity when the
  caller knows it (`with_capacity` on every growable type, including via the `IOBase` trait).
  When a change claims a performance win, **prove it** — a benchmark on both time and memory,
  plus a deterministic allocation test.
- **Bulk operations are vectorized.** Typed bulk reads/writes (`pread_i32_array` /
  `pwrite_i64_array`, …) and repeated-value fills (`pwrite_i32_repeat`, …) run as **dense,
  branch-free loops over contiguous slices** so LLVM auto-vectorizes them on stable Rust (no
  SIMD dependency) — and a fill never materializes the full array. New sources inherit these
  from `IOBase`'s default methods; override only with something measurably faster.
- **Cross-platform first, platform-optimized underneath.** Every public API behaves
  **identically on every OS**; the same code runs on Windows, macOS, and Linux. Where a
  platform offers a faster route, **redirect to it behind `#[cfg(...)]`** (as `Mmap` does —
  `mmap`/`munmap` on unix, `CreateFileMappingW`/`MapViewOfFile` on windows — under one
  cross-platform surface), never fork the public behavior. A `#[cfg]` block always has an arm
  for **every** target (a portable `std` fallback is the last arm), and CI cross-checks unix on
  `x86_64-unknown-linux-gnu`. Paths are POSIX-normalized (`uri`), temp/home roots resolved from
  the environment — nothing hardcodes a separator or an absolute root.
- **Resolve shared instances once — never construct per call.** A registry, catalog, codec, or
  parsed constant that does not depend on the call's inputs is built **once** into a
  process-wide `LazyLock` static and reused (the `default_catalog()` mime registry, the
  `DEFAULT_URI`, the `stage_*` kernels). In the **bindings** this matters most: expose module
  singletons / cached factories so Python and Node do not re-instantiate a codec or re-seed a
  catalog on every call — resolve from the shared static and hand back a thin handle.
- **Content-changing io keeps its metadata in sync — optimally.** Any operation that changes a
  source's bytes (write past the end, `truncate`, in-place `compress`/`decompress`, a
  cross-source copy) **updates the affected `Headers`** in the same pass: `Content-Length` to
  the new byte size, `Content-Type` when the media changes (compress/decompress), and
  `mtime` (epoch µs) to now. Do it **without extra passes or allocations** — set the small
  header values inline (the alloc-free `set_mtime` render), only when the value actually
  changed, and never re-read the source to recompute what the operation already knows.
- **Metadata reads prefer the cached header.** Size / media-type / mtime accessors read the
  `Headers` value when present (it is authoritative and free) before probing the backing —
  a mapped `byte_size` is cheap, but a directory tree sum or a network `HEAD` is not, so a
  populated `Content-Length` short-circuits it.
- **A move is a copy that consumes its source — streamed, then removed.** `move_into(dst)`
  relocates a source's bytes into another `IOBase` and **removes the source at the end**,
  leveraging the same abstraction a cross-source copy does — **never** a re-read or an extra
  full-size buffer. It is a **no-op when source and destination address the same `uri`** (a
  move onto itself neither copies nor deletes). Prefer a **streamed** move — transfer in
  bounded chunks and, where the source can shrink cheaply (a `Heap`/`Mmap`/`LocalIO` that
  `truncate`s), **drop each chunk from the tail as it lands** so peak memory is one chunk, not
  the whole payload — then `rm` whatever backing remains. A source with no removable backing
  (a bare `Heap`) still moves its bytes and simply clears to empty.
- **Reads never fail on a missing source — they return empty.** A positioned byte read of a
  node that does not exist yet (a lazy `LocalIO` over an absent path, a `Heap` past its end)
  returns **zero bytes**, never an error — laziness means "not there yet", not "broken". Only
  the *typed* helpers with a hard fill contract (`pread_i32`, `pread_exact`) surface the guided
  `UnexpectedEof`, because they cannot fabricate the missing bytes. Every filesystem family
  inherits this: probing/navigating an absent node touches nothing and reads empty.
- **Every error names the fix.** An `IoError` (or any guided error) states the offending value,
  the expected range/tokens, **and** a short, concrete next step to fix it ("read fewer bytes
  or extend the data first", "enable the `compression` feature", "seek to a non-negative
  position"). Keep the fix hint short and imperative; the **same text** surfaces as a Python
  `ValueError` and a Node `Error`, so it must read well with no code around it.
- **`IOBase` is the central access path — bytes, address, and graph in one contract.** There
  is no separate path/graph trait: `IOBase` itself carries the graph surface — `ls` /
  `ls_recursive` **stream children of the same source type** (`Children` / `Walk` associated
  types; a leaf source declares `NoChildren`), `name` / `parent` navigate, `children` is the
  collected convenience, and `rm` / `rmfile` / `rmdir` remove (leaf default: a guided
  refusal). Discovery is **streamed** (iterators, never a pre-collected tree).
- **A container node is a memory tree.** A directory (or an object-store prefix) serves the
  *byte* contract too, through the generic `tree_*` defaults on `IOBase` — `tree_byte_size`
  (the lazy, streamed, uncached subtree sum), `tree_pread_byte_array` /
  `tree_pwrite_byte_array` (reads/writes routed across **name-sorted child blocks** as one
  contiguous region; child containers recurse; a middle block never grows — only the last
  block absorbs bytes past the end). The pattern is written **once** on `IOBase`; every
  filesystem family (local today, s3/azure/network later) wires its `byte_size`/`pread`/
  `pwrite` container branches to these same defaults so behavior is identical everywhere.
- **One access point per filesystem — lazy, auto-creating, self-optimizing.** Each
  filesystem family exposes exactly **one** handle type (`LocalIO` for local): a **lazy**
  node over any path — constructing/probing/navigating touches nothing, reads on a missing
  node are empty, and the handle **decides per call** how to serve I/O (ad-hoc positioned
  reads before any write; the first **write auto-creates** the missing parent folders + the
  file, memory-maps it, and keeps the mapping so later access runs at memory speed with zero
  allocations). Never split the surface into separate file/folder/path types — `mkdir`
  covers the folder-as-goal case and `close()` releases the optimized backing.
- **No lifetime parameters on public types** — the bindings must hold every one.
- **Coherent layering — the contract at the module root, implementations below.** Cross-cutting
  value types and traits (`IoError`, `Whence`, `Headers`, `IOMode`, `IOKind`, `Serializable`)
  live at the `io` root; the byte contract (`IOBase` + wrappers) at the `memory` root; each
  concrete **source** (`Heap`, `Mmap`) is one file below, implementing the trait's few
  required methods and inheriting the rest. A source depends **downward**, never sideways on a
  sibling source.
- **Ergonomic updates — `copy(**fields)` + `set_*` + `with_*`.** Every mutable public value
  type gets the trio: a `copy` that (where the idiom allows) takes an optional argument per
  settable field defaulting to the current value (Python kwargs / Node options object — the
  clone-with-overrides front door); an in-place `set_<field>`; a chainable `with_<field>`.
  Where combining whole values reads naturally, add `merge_with(other)` and domain combinators
  (`joinpath`). In the Rust core `copy` stays a plain clone and overrides chain via `with_*`.
- **One file per public type.** Mirror the nearest neighbour's structure, naming, error style,
  and doc style.
- **Minimize `Option`.** Only when absence is a real, distinct state a caller must handle.
  Prefer a total method with a sensible default (`Url::host()` → `""`), an empty collection
  over `Option<Vec<_>>`, two named methods over an `Option<bool>` flag, and a guided `Result`
  when absence is an error. Each `Option` in a public signature must justify itself.
- **Guided errors.** Every error names how to fix it (the expected range/tokens *and* the
  offending value, or the next step). Same text across Rust, Python, and Node.
- **Naming: `query` is the raw string; `params` is the map.** On `Uri`/`Url`, `query()` /
  `set_query()` address the raw query **string**, while `param` / `params` / `set_param` /
  `has_param` / … address the parsed key-value **map**. Apply the same split anywhere a raw
  form and a parsed map coexist.
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
crates.io / PyPI / npm and creates the `v<version>` tag + GitHub Release. So bump the version
**only** when you intend to release; keep it pinned during ordinary changes so the
auto-publish never fires mid-change.
