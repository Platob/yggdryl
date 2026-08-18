# Prompt: `arrowfs` — an `IOBase` wrapper over foreign Arrow filesystems

Implement a new storage backend for the Yggdryl workspace that wraps **existing
Arrow filesystem implementations** — `pyarrow.fs.FileSystem` (S3, GCS, Azure,
local, subtree, and custom `FileSystemHandler` subclasses) in Python, a
caller-supplied handler object in JavaScript, and any Rust value implementing a
small new vtable trait — behind the crate's one storage abstraction, `IOBase`.
The result: a handle backed by S3 (or any foreign filesystem) reads and writes
folders and files, streams Arrow records, and composes with every existing
wrapper (`Coded`, `Ipc`, `Parquet`, `Media`, Iceberg tables) with **zero
transport code reimplemented in this repository**. Deliver it complete: fully
implemented, edge-case tested, benchmarked against baselines the reader trusts,
and documented with running examples.

Work on branch `claude/iobase-arrow-filesystem-wrapper-d13dzp`; commit and push
there.

---

## 0. Read first (non-negotiable)

1. **`AGENTS.md`, in full.** It is the real spec. The sections that govern this
   task: *Order of work* (line 9), *Source layout and scope* (17), *Storage and
   I/O contract* (101), *Documentation organization* (338), *Exact method
   vocabulary* (390), *Error message contract* (555), *Binding boundary
   contract* (812), *Python extension* (845), *JavaScript extension* (1035),
   *Required checks* (1099).
2. `rust/src/io/mod.rs` — the `IOBase` trait (line 205), the
   `delegate_iobase!` macro (line 94), and the three-method record surface
   (lines 833–1035).
3. `rust/src/io/roles.rs` — the `IOPath` / `IOFolder` / `IOFile` role traits a
   backend implements to inherit the boilerplate.
4. `rust/src/local/` — the reference backend (`path.rs`, `folder.rs`,
   `file.rs`, `tests.rs`). Your module mirrors it one-for-one.
5. `rust/src/io/coding.rs` — `Coded`, the reference *wrapping* handle and the
   in-tree precedent for "this backend cannot seek-write, so bytes are staged
   and published on close, and the comment says why".
6. `rust/src/generic/holder.rs` — the `#[non_exhaustive]` dispatch enum every
   backend must join so `parent`/`child_by`/`ls` can return its handles.
7. `docs/io.md` and `docs/local.md` — the documentation register to match.

The repository has already reserved this slot. `rust/src/io/mod.rs:17`:
*"Anything else - an object store, an Arrow filesystem - implements the same
trait outside the core."* `rust/src/local/mod.rs`: *"a new backend is a sibling
module supplying the same three roles rather than a change to anything here."*
`AGENTS.md:54`: a remote backend is *"a sibling module - never a change to `io`
or `local`."* Honor all three.

---

## 1. Architecture

### 1.1 The foreign vtable (Rust core, dependency-free)

Create sibling module folder `rust/src/arrowfs/` with exactly:
`mod.rs`, `path.rs`, `folder.rs`, `file.rs`, `tests.rs` — plus the one file
`local/` does not have: the vtable definition (put it in `mod.rs` or a
`system.rs` the module keeps private and re-exports from).

Define one small **synchronous** trait modeled on the Arrow C++ `FileSystem`
API surface (the contract `pyarrow.fs`, Arrow C++, and Arrow Java all share),
so a foreign implementation maps onto it method-for-method:

```rust
/// One foreign filesystem: the minimal synchronous surface the three roles
/// need, modeled on Arrow's `FileSystem` API so an existing implementation
/// maps onto it without adaptation logic.
pub trait ArrowFileSystem: Send + Sync {
    /// The filesystem's own name for diagnostics ("s3", "local", "memory").
    fn type_name(&self) -> &str;
    /// What is at `path` right now: kind and size. `NotFound` is a normal
    /// answer, not an error.
    fn file_info(&self, path: &str) -> Result<FileInfo>;
    /// Every entry under `path` (`recursive` descends). A missing directory
    /// lists empty.
    fn list(&self, path: &str, recursive: bool) -> Result<Vec<FileInfo>>;
    /// Bytes `[offset, offset + buffer.len())` of the file at `path` into
    /// `buffer`; short reads at end-of-file return the short count; a missing
    /// file reads 0 bytes.
    fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize>;
    /// Replace the file at `path` with exactly `bytes`, creating it and its
    /// parents. Whole-value replacement is the one write shape every Arrow
    /// filesystem supports (object stores have no random write).
    fn write_full(&self, path: &str, bytes: &[u8]) -> Result<()>;
    /// Create the directory at `path` and its parents; existing is success.
    fn create_dir(&self, path: &str) -> Result<()>;
    /// Remove the file at `path`; missing is success.
    fn delete_file(&self, path: &str) -> Result<()>;
}

/// What a foreign filesystem reports about one path.
pub struct FileInfo {
    pub path: String,        // filesystem-relative, forward slashes
    pub kind: IOKind,        // File | Directory | Unknown (NotFound)
    pub size: u64,           // 0 unless kind == File
}
```

Design rules for the vtable:

- **No new dependency.** The trait is plain Rust; the core never links an
  object-store or Arrow-filesystem crate. Because it adds no dependency it is
  **unconditional** (no Cargo feature) — like the Avro value codec. The record
  surface it inherits is already gated by the existing `arrow` feature.
- **Sync only.** `IOBase` is `Send` and blocking-positional. `pyarrow.fs` is
  synchronous, so the Python binding needs no bridge. A future async Rust
  backend owns its own blocking bridge inside its `ArrowFileSystem` impl —
  outside this task's scope; do not add a runtime.
- Errors: implementations report failures as `crate::Error` (transport
  failures wrap as `Error::Io`; `std::io::Error::other(...)` carries a foreign
  message with its source chain intact). Downstream record errors flow through
  the existing `yggdryl::arrow::Error::External` channel. **No third error
  enum** (`AGENTS.md:574`).
- Reuse `IOKind` from `rust/src/enums/io_kind.rs` — never a local kind enum.
- Follow the naming vocabulary exactly (`AGENTS.md:390`): `from_*` validating
  constructors, `as_*` borrowed views, `is_*` predicates.

### 1.2 The three roles over the vtable

`arrowfs::Path`, `arrowfs::Folder`, `arrowfs::File` mirror
`local::{Path,Folder,File}` structurally:

- Each holds `Arc<dyn ArrowFileSystem>` plus a canonical `Url` (the handle's
  identity: `s3://bucket/key`, or whatever scheme the caller's URL carries —
  `Scheme::is_storage` already admits `s3`/`gs`/`az`/`file`). The
  filesystem-relative path string handed to the vtable is derived from the URL
  once and cached.
- `Path` reports `IOKind` from `file_info` and routes to `Folder` or `File`
  (same `resolved: Mutex<Option<...>>` lazy-resolution shape as
  `local/path.rs`; note its line-47 comment — a role never holds a `Holder`).
- Implement `IOPath` / `IOFolder` / `IOFile` from `rust/src/io/roles.rs`, then
  `IOBase`, so glob, partitions, `children_where`, compression, and the three
  record methods are all inherited — write none of them yourself.
- **Handles are lazy** (`AGENTS.md:107`): construction never calls the vtable;
  `pread` on a missing file yields 0 bytes and `size` reports 0; writes create
  the file and parents on first use; `media_type` derives from the URL's
  suffixes and is re-derived after bytes change.
- **Reads map straight through**: `File::pread` → `read_range` (an S3 range
  GET; never a whole-object download to serve one range). `size` →
  `file_info`. `ls`/`child_by` → `list`/`file_info` returning `Holder`s.
- **Writes are staged.** Arrow filesystems replace whole files; `IOBase::pwrite`
  is positional. So `File` stages mutations in an in-memory `Buffer` — loaded
  from the remote value on first write when one exists — and publishes with one
  `write_full` on `flush`/`close`. `open` materializes, `close` publishes and
  releases, exactly the `open`/`close` contract at `AGENTS.md:118`, and a
  comment on the staging field says **why** it is held (`AGENTS.md:212`:
  whatever must be buffered says why). Guard the stage with the shared
  size-budget check (`io::oversized`, `rust/src/io/mod.rs:1229`) so an
  over-budget staged write is a typed refusal, not an OOM.
- `truncate`, `reserve`, `clear`, `append` operate on the stage;
  `capacity` reports the stage's capacity when open, else `size`.
- `Folder::create_folder` → `create_dir`; on object stores a directory is a
  prefix — `folder_exists` is "the prefix has entries or the marker exists",
  matching what `pyarrow.fs` itself reports. Do not invent marker objects the
  foreign filesystem would not create.

### 1.3 One unavoidable edit outside the module: `Holder`

Add `Holder::ArrowPath`, `Holder::ArrowFolder`, `Holder::ArrowFile` variants
(names following the module) in `rust/src/generic/holder.rs`, with the same
mechanical constructors and full `IOBase` delegation the existing variants
have. The enum is `#[non_exhaustive]`, so this is additive. A generic enum
*"delegates the whole contract to its variant and adds no behavior"*
(`AGENTS.md:43`) — keep it a pure dispatch edit. No other file outside
`rust/src/arrowfs/` changes in the core (docs, tests, benches aside).

---

## 2. Order of work (from `AGENTS.md:9` — Rust first, fully)

**Phase 1 — Rust core, complete before any binding**: vtable + roles +
`Holder` variants + in-tree test filesystems + edge-case tests + criterion
benchmarks + `docs/arrowfs.md` with runnable Rust examples (Python/JS tabs
marked `!!! note "Rust first"` until the bindings land). A change that stopped
here would be complete work.

**Phase 2 — Python binding.** **Phase 3 — JavaScript binding.**
**Phase 4 — docs/benchmark/notebook completion.** **Phase 5 — required
checks.** Each phase below.

---

## 3. Phase 1 details: core tests and reference filesystems

Provide two in-tree `ArrowFileSystem` implementations **inside the module**
(exported, documented — they are the test and benchmark substrate and give
Rust users a working backend without any foreign runtime):

- `arrowfs::MemoryFileSystem` — a `Mutex<BTreeMap<String, Vec<u8>>>` plus a
  directory set. This is the "memory" filesystem the task asks for.
- `arrowfs::LocalFileSystem` — thin `std::fs` mapping (read_range via
  `seek`+`read`, write_full via temp-file-then-rename so publication is
  atomic). It exists to prove the vtable against a real OS filesystem and to
  benchmark wrapper overhead against `local::File`; it does **not** replace
  `rust/src/local/` and the docs must say so in one sentence.

Tests:

- `rust/src/arrowfs/tests.rs`: the module's edge cases — lazy construction
  (a constructed handle performs zero vtable calls — prove it with a counting
  mock), missing-file reads yield 0/empty, write-creates-parents, staged
  write publishes once on close and the remote value is unchanged before
  `close`, reopen-after-close refetches (no stale cache, `AGENTS.md:186`),
  range reads at EOF, `ls` recursive and flat, glob descent, hive
  `children_where`, `child_by` on a file is `NotADirectory`, oversized stage
  refused with the typed error, media-type derivation from URL suffixes,
  `Coded` over an `arrowfs::File` round-trips `.json.gz`, IPC and Parquet
  record round trips through the three record methods over both reference
  filesystems, and an Iceberg table over an `arrowfs` folder handle
  (feature-gated like the existing iceberg tests).
- Extend `rust/src/io/tests.rs` ("behavior every `IOBase` implementation must
  share") so the shared conformance battery also runs over
  `arrowfs::MemoryFileSystem`-backed handles.
- Error messages follow the contract: `expected X, got Y`, located by URL;
  foreign failures keep their source chain; never a panic on caller input.

Benchmarks (`rust/benchmarks/arrowfs.rs`, dispatcher pattern
`#[path = "arrowfs/mod.rs"] mod benchmarks;` like `rust/benchmarks/io.rs`;
stable criterion group IDs):

- wrapper overhead: `pread`/`pwrite`/`read_all` through
  `arrowfs::MemoryFileSystem` vs the native `Buffer`, and through
  `arrowfs::LocalFileSystem` vs `local::File` — the baselines the reader
  trusts, on the same payloads;
- record round trip: IPC and Parquet (the parquet-feature bench leg) over
  `arrowfs` vs over `Buffer`;
- listing/glob over a populated tree.

---

## 4. Phase 2: Python binding — `pyarrow.fs` transparently

The point of the feature: any `pyarrow.fs.FileSystem` — `S3FileSystem`,
`GcsFileSystem`, `LocalFileSystem`, `SubTreeFileSystem`, and **custom**
filesystems via `pyarrow.fs.PyFileSystem(FileSystemHandler)` (which is also
how fsspec arrives) — becomes a Yggdryl handle with no per-backend code.

- In `python/src/io.rs`, implement `ArrowFileSystem` once over a held
  `Py<PyAny>` filesystem object. Each vtable method acquires the GIL and calls
  the stable `pyarrow.fs.FileSystem` API: `get_file_info` (path and
  `FileSelector`), `open_input_file(...).read_at(nbytes, offset)` (fall back
  to `seek`+`read` only if `read_at` is absent), `open_output_stream` +
  `write` + `close` for `write_full`, `create_dir`, `delete_file`. Python
  exceptions cross as `Error::Io(std::io::Error::other(...))` preserving the
  message unchanged (`AGENTS.md:580`). `Py<PyAny>` is `Send + Sync`; take the
  GIL per call, never hold it across a call boundary.
- Detect a filesystem argument with the non-importing `declared_by(value,
  "pyarrow.fs", ...)` / duck-typing pattern from `python/src/record.rs:193` —
  accept any object whose class derives from `pyarrow.fs.FileSystem`; never
  import pyarrow at module load.
- Boundary: widen the existing inference in `PyIOBase::new`
  (`python/src/io.rs`, the `#[new]` that today dead-ends in `url.to_path()`)
  and add the explicit classmethod. Surface, matching house style
  (`IOBase.from_bytes` precedent; `filesystem` naming pyarrow's own keyword):

  ```python
  IOBase(fs, "bucket/path")                    # inference: fs first, path second
  IOBase.from_arrow_fs(fs, "bucket/path")      # the explicit spelling
  handle = IOBase.from_arrow_fs(S3FileSystem(region=...), "bucket/key.parquet")
  ```

  The returned handle is the same `IOBase` class — pathlib-shaped surface,
  `with` binding `open`/`close`, the three record methods — nothing
  filesystem-specific leaks into the Python API. `iterdir`, `glob`, `/`,
  `parent` return handles that still carry the filesystem.
- Update `python/yggdryl/_native.pyi` and `__init__.pyi`; keep
  `mypy --strict` green.
- Tests in `python/tests/test_arrowfs.py`, house style (fixture + plain-English
  test classes with docstrings):
  - `pyarrow.fs.LocalFileSystem` over `tmp_path`: bytes round trip, folder
    listing/glob, parquet + ipc record round trips, `with` publish semantics,
    interop both directions (pyarrow writes / yggdryl reads, and the reverse,
    byte-identical) — this is the outside-implementation check
    (`AGENTS.md:12`) run inside pytest;
  - a **custom in-memory `pyarrow.fs.FileSystemHandler`** subclass wrapped in
    `PyFileSystem` — proves the "custom pyarrow filesystems" and "memory"
    requirements with no new test dependency;
  - `SubTreeFileSystem` — proves transparency over a wrapped/prefixed store
    (the S3-shaped case without a network);
  - error surfacing: a handler that raises shows the original message in the
    raised exception; a non-filesystem object is refused naming what was
    expected and what arrived.
- Benchmark `python/benchmarks/arrowfs.py`: read/write and parquet round trip
  through the wrapper vs **pyarrow's own** `LocalFileSystem` +
  `pyarrow.parquet` on the same payload — the trusted baseline —, release
  build only (`maturin build --release`), numbers regenerated into
  `docs/benchmarks.md`, never edited (`AGENTS.md:369`).

---

## 5. Phase 3: JavaScript binding — a handler object

Arrow JS ships no filesystem, so the JS boundary accepts what the ecosystem
has: **a caller-supplied handler object** implementing the vtable in camelCase
— the same contract, documented as mirroring `pyarrow.fs`:

```javascript
const fs = {
  typeName: 'memory',
  fileInfo(path) { return { path, kind: 'file', size: 12n } },   // or kind: 'not-found'
  list(path, recursive) { return [...] },
  readRange(path, offset, length) { return Buffer },
  writeFull(path, bytes) {},
  createDir(path) {},
  deleteFile(path) {},
}
const handle = IOBase.fromArrowFs(fs, 'bucket/key.arrows')
```

- In `node/src/io.rs`, implement `ArrowFileSystem` over stored
  `napi::Ref`s to the handler's methods, called synchronously on the JS
  thread (the binding is sync like the rest of the surface; do not introduce
  threadsafe-function plumbing — if a stored handler cannot be invoked
  synchronously off-thread in napi 3, say so in the doc page's JS tab note
  and keep handler-backed handles main-thread-only, named limitation over
  emulation, per house style).
- Widen `LocationInput` inference in `node/src/io.rs` plus the explicit
  `IOBase.fromArrowFs`. 64-bit sizes cross as `bigint`; kinds cross as the
  `IOKind` strings the core already exports; errors surface the native
  message via `napi_error` unchanged.
- Tests `node/tests/arrowfs.test.js` + `arrowfs.types.ts` (node:test +
  tsc --noEmit pair): an in-memory JS handler (map-backed) and a `node:fs`
  handler over a temp dir; byte + record round trips; a throwing handler's
  message surfaces; type-level checks for the handler interface.
- Benchmark `node/benchmarks/arrowfs.js` (wired as `npm run bench:arrowfs`):
  wrapper vs direct `node:fs` on the same payload, release build
  (`napi build --release`).

---

## 6. Phase 4: documentation

- New page `docs/arrowfs.md` — one H1, exactly one opening sentence, then
  example-first sections: construct from a foreign filesystem; bytes; folders
  and glob; records (IPC/Parquet); composing with `Coded` and Iceberg; the
  staged-write semantics stated plainly (a write publishes on `close`; an
  object store has no random write — say it, don't hide it); the two in-tree
  Rust filesystems; how to bring your own (`ArrowFileSystem` impl /
  `FileSystemHandler` / JS handler).
- Every example in **Rust → Python → JavaScript tabs, in that order**, each
  idiomatic, each self-contained with at least one assertion, all passing
  `python scripts/check_docs_examples.py`. Check `.api-bindings.txt` before
  showing a language do anything; use the S3 constructor only in a
  `<lang>,ignore`-tagged block (no network in doc checks) with the runnable
  examples on local/memory filesystems.
- Add the page to the `Storage:` nav block in `mkdocs.yml` beside
  `io / generic / local`; update `docs/io.md`'s backend sentence and
  `docs/architecture.md`; regenerate notebooks with
  `python scripts/build_docs_notebooks.py` (edit blocks, never notebooks);
  update the README layout table line for `rust/src/arrowfs/`; touch
  `docs/extensions/{python,javascript}.md` only for their boundary.
- Update `docs/benchmarks.md` with the regenerated tables, naming machine,
  interpreter, and build profile.
- `python -m mkdocs build --strict` stays green.

---

## 7. Phase 5: required checks (all must pass before handoff)

Per `AGENTS.md:1099`: `cargo fmt --check`; warning-free
`cargo clippy --locked --workspace --all-targets -- -D warnings` **twice**
(default features and `--features "parquet iceberg"`); workspace tests twice
the same way; `cargo doc` with `RUSTDOCFLAGS="-D warnings"`; the Rust 1.85
core check (default features and `--no-default-features --lib` — the new
module must compile without `arrow`, with its record surface inherited, not
duplicated); `cargo bench --benches --no-run`; maturin develop + pytest +
`mypy --strict`; `npm run test:package` + `npm test`;
`python scripts/check_docs_examples.py`; `python -m mkdocs build --strict`.
Clean generated targets, venvs, and `node_modules` after validation.

---

## 8. Hard constraints, restated

- Never a second storage trait, never a change to `rust/src/io/` semantics or
  `rust/src/local/` (the only `io` edit is additive: the shared conformance
  tests; the only `generic` edit is the mechanical `Holder` variants).
- The record surface stays exactly the three methods; the wrapper inherits
  them — no `arrowfs`-specific record entry points.
- No new runtime dependency in any of the three manifests; no async runtime;
  no network calls in any test, benchmark, or doc example.
- Names follow the exact method vocabulary; bindings map `from_arrow_fs` ↔
  `fromArrowFs`; argument names and order identical across languages.
- Error messages: native message crosses unchanged; typed, located,
  `expected X, got Y`; a foreign failure keeps its source chain.
- Anything staged in memory carries a comment saying why; no silent caps —
  the size budget refuses loudly.
- Commit in coherent steps (core, tests, benches, python, node, docs) with
  descriptive messages; push the branch; do not open a PR.

**Definition of done**: a user hands `IOBase.from_arrow_fs(S3FileSystem(...),
"bucket/table")` to `yggdryl.iceberg.Table` and it just works — and every
line of transport code involved was written by the Arrow project, not this
one.
