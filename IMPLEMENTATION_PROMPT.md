# Prompt: the catalog as a service hierarchy, Iceberg v3 in full, and Apache Doris 4.1 as the outside implementation that checks both

This prompt has three parts, in this order, each complete work on its own if
the task stops there. Each one is the ground the next stands on.

**Part one** (section 1) refactors `iceberg::Catalog` into a real service
hierarchy - `catalogs`/`catalog`, `namespaces`/`namespace`, `tables`/`table`,
one shape at every level, so `catalog.namespaces[...].tables[...]` addresses a
table - and carries two workspace-wide sweeps with it: **every
probe-before-act is deleted** in favor of acting and handling the failure, and
**every listing becomes a lazy iterator** instead of a collected `Vec`, so a
folder or a catalog larger than memory can be listed at all. One mechanical
rename, `IOBase::child_by` → `child_by_path`, rides along. It is where every
later part addresses a table from, so it lands first.

**Part two** (section 2) makes Iceberg format version 3 fully readable and
writable - deletion vectors over a new `rust/src/puffin/`, row lineage,
`variant`, `geometry`/`geography`, nanosecond timestamps, column defaults, and
multi-argument transforms - under one rule: **every feature is built on a
mechanism this workspace already owns, and the ones that are not are named
before a line is written.** It also fixes something worse than a gap: the scan
skips delete manifests today (`iceberg/scan.rs:305`), so a v2 table with
deletes already returns too many rows.

**Part three** (section 3 onward) implements `rust/src/doris/`: the module that
lets Yggdryl speak Apache Doris 4.1 - its type system, its DDL, its Stream Load
wire, its export layout, its table-value functions, and its Iceberg catalog -
and then **uses Doris as the outside implementation** that proves the Parquet
and Iceberg read/write protocols this workspace already ships are correct
against an engine nobody here wrote. Today the workspace validates Iceberg
against PyIceberg and Avro against fastavro; Parquet is checked against
PyArrow, which is the same Rust/C++ lineage the `parquet` crate came from.
Doris is a genuinely independent C++ reader and writer with its own Parquet and
Iceberg implementation, so a round trip through it is the strongest
correctness signal available: if Doris reads every row and every nested value
of what `rust/src/parquet/` and `rust/src/iceberg/` wrote, and Yggdryl reads
back every row of what Doris wrote, both protocols are settled.

Deliver all three complete: fully implemented, edge-case tested - on every
Doris type including the deeply nested ones, and on every v3 feature including
the delete shapes - benchmarked against implementations the reader already
trusts, interop-checked in both directions against PyIceberg *and* a real
Doris, and documented with running examples.

Work on branch `claude/catalog-v3-doris`; commit and push there.

---

## How to work this prompt

- **One part at a time, in order.** Do not open part two until part one's
  checks are green. The parts are written so each stops as complete work; a
  half-finished part underneath a new one is the failure mode this ordering
  prevents.
- **Use the fast loop while working, the matrix only before handoff.**
  `docs/testing.md` lists the narrow commands -
  `cargo test --features "parquet iceberg" -p yggdryl --lib iceberg::`,
  `… --lib io::`, `cargo test --test iceberg_interop`, and the rest. Section 12
  is the pre-handoff matrix, not the inner loop.
- **Commit in coherent steps**, listed at the end of each part. A mechanical
  rename is its own commit; a rename and a behavior change never share one.
  Commit messages say what changed and why, and carry the measurement when the
  change claims one.
- **Every fact in this prompt was transcribed and may be wrong.** Field ids,
  magic bytes, header names, version numbers, line numbers: check each against
  the source named beside it. **When the source disagrees, the source wins** -
  implement what the source says and note the correction in the commit message.
- **When something external is unavailable** - no Docker for the Doris server,
  no PyIceberg build with v3 - the affected interop half prints `SKIPPED` and
  its driver fails on that word. Never fake a pass, never delete a check, never
  soften an assertion to get green. Say in the handoff exactly which half did
  not run.
- **A change that needs a new dependency is the wrong change.** Re-read section
  13 and find the mechanism that already exists; that is what most of this
  prompt is about.
- **Measure before you claim.** No "optimized", "fast", or "zero-copy" in a
  comment, a doc line, or a commit message without a number from a release
  build behind it.
- **Leave the tree clean**: `target/`, `site/`, virtual environments,
  `node_modules`, native binaries, and Docker containers and volumes all go
  after validation.

---

## 0. Read first (non-negotiable)

1. **`AGENTS.md`, in full.** It is the real spec. The sections that govern this
   task: *Order of work* (line 9), *Source layout and scope* (17), *Storage and
   I/O contract* (109), **_Existence and creation contract_ (183)**,
   **_Listing and iteration contract_ (226)**, *Media implementation standard*
   (265), *Table format contract* (310), *Documentation organization* (435),
   *Exact method vocabulary* (491), *Error message contract* (660), *Native
   value behavior* (686), *Parser contract* (708), *Arrow and allocation
   contract* (789), *Binding boundary contract* (923), *Python extension*
   (956), *JavaScript extension* (1146), *Required checks* (1210).
2. `rust/src/iceberg/catalog.rs` — what section 1 refactors. Read
   `Namespaces` (line 343), `Namespace` (488), and `Tables` (597) beside the
   flat `Catalog` methods above them (125-260), and count how many storage
   round trips each operation makes today. That count is the point of part one.
3. `rust/src/iceberg/` — the model for this module's shape. One folder, one
   file per concern (`types.rs`, `schema.rs`, `partition.rs`, `metadata.rs`,
   `options.rs`, `catalog.rs`, `scan.rs`, `table.rs`), a non-default feature,
   no dependency for the format itself, and `iceberg::IcebergOptions` as the
   single place every knob is resolved (explicit → table property → default).
   `doris/` is built exactly this way. Read `iceberg/options.rs` before writing
   `doris/options.rs`.
4. `rust/src/parquet/` and `rust/src/ipc/` — the two record encodings Doris
   will read and write. The three record methods
   (`read_arrow_batch_reader`, `write_arrow_batch_reader`,
   `append_arrow_batch_reader`) are the only decode and encode entry points; a
   Stream Load body is produced *through* them, never by a private writer.
5. `rust/src/expression/` — `Expression`, `Bound`, `pushdown.rs`. The predicate
   Doris receives in a TVF `WHERE` clause is this expression rendered to Doris
   SQL, and the residual split is the one already implemented. There is no
   second filter representation.
6. `rust/src/datatype/compatibility.rs` (`to_scheme_compat`, line 112) and
   `rust/src/enums/scheme.rs` (`COMPATIBILITY_TARGETS`, line 91). Doris becomes
   the sixth compatibility target; the walker is the one generic recursive
   walker with a per-target scalar matrix - never a fork.
7. `rust/src/metadata.rs` and the `Field` protocol views (`Field::iceberg`,
   `Field::parquet_field_id`, `Field::mysql`, `Field::spark`). Doris state is
   inert `doris:*` string properties reached through a `Field::doris` view.
8. `rust/src/json/` and `rust/src/text/` — the Stream Load response is JSON and
   is decoded through the shared `Value`, never `serde_json` directly, never a
   hand-rolled scan.
9. `rust/src/iceberg/scan.rs` — `plan`, and line 305, where every manifest
   whose content is not `Data` is skipped. Understand what that costs before
   section 2 asks you to fix it.
10. **The Iceberg table spec and the Puffin spec, from the source**, not from a
   summary: `format/spec.md` and `format/puffin-spec.md` in `apache/iceberg`.
   Every field id, magic byte, and framing rule quoted in section 2 was
   transcribed and must be re-verified against those two documents before it is
   relied on.
11. `scripts/check_iceberg_interop.py` and `rust/tests/iceberg_interop.rs` — the
   exact interop harness pattern to copy, including the `SKIPPED` word the
   driver fails on so a skipped half can never read as a pass.
12. `docs/iceberg.md`, `docs/parquet.md`, `docs/io.md`, `docs/benchmarks.md` —
   the documentation register to match.

---

## 1. Part one: the catalog becomes a service hierarchy, and existence stops being pre-checked

`doris/catalog.rs` leans on `iceberg::Catalog`, and what it would lean on is a
good idea left half-finished. The collection views already exist -
`Namespaces` (`iceberg/catalog.rs:343`), `Namespace` (488), `Tables` (597) -
but `Catalog` still carries a whole flat surface *over* them, the three levels
do not share a shape, nothing addresses a catalog above a warehouse, no level
but the table carries properties, **every level probes storage before it
acts**, and **every listing collects into a `Vec`**. The last two are
workspace-wide, not catalog-local: the existence audit of 1.4 and the listing
sweep of 1.5 touch `io`, `local`, `arrowfs`, and `generic` as much as
`iceberg`. One mechanical rename (1.6) rides along.

Fix all of it first, in its own commits, before anything new leans on it. This
section is complete work on its own if the task stops here (`AGENTS.md:9`).

### 1.1 Three levels, one shape

Split `rust/src/iceberg/catalog.rs` into the folder it already has a `tests.rs`
in - modules own real implementation, never empty shells around a monolith
(`AGENTS.md:17`):

| file | owns |
| --- | --- |
| `catalog/mod.rs` | the shared `Resource` / `Collection` shape, name resolution, re-exports |
| `catalog/catalogs.rs` | `Catalogs` and `Catalog` |
| `catalog/namespaces.rs` | `Namespaces` and `Namespace` |
| `catalog/tables.rs` | `Tables` (the `Table` value stays in `iceberg/table.rs`) |
| `catalog/tests.rs` | the module's edge cases |

Three levels, one shape, learned once and reused twice. A **collection** is a
lazy map-oriented view whose construction touches nothing, and it has exactly
this vocabulary at every level - no level invents a verb (`AGENTS.md:491`):

```
names()  len()  is_empty()  contains(name)  get(name)  create(name, ..)
open_or_create(name, ..)  iter()  IntoIterator
```

A **resource** is one addressed thing, and it has exactly this one:

```
name()  parent()  url()  properties()  get_property()  set_property()
remove_property()  clear_properties()  <child collection>()
```

So the cascade reads the same at every depth, and a namespace nests:

```rust
let orders = catalog
    .namespaces().get("sales")?
    .namespaces().get("eu")?
    .tables().get("orders")?;
```

`Catalogs` is the new level the hierarchy was missing: a view over a folder of
warehouses, so `catalogs.get("lake")?.namespaces()...` works and a deployment
with more than one warehouse stops being a caller-side convention.

**Dotted names are resolved in one place.** A collection's `get`, `create`, and
`open_or_create` accept a dotted identifier and descend - `namespaces.get(
"sales.eu")`, `tables.get("sales.eu.orders")` - so the resolution rule lives in
the collection and not in five call sites. `split_dotted` disappears from the
flat surface.

**The flat surface collapses.** `Catalog` keeps exactly two dotted entry points,
because a dotted identifier is a real Iceberg spelling and deserves one call:
`Catalog::table` and `Catalog::namespace`. Delete `create_table`, `has_table`,
`open_or_create_table`, `append`, `append_with`, `overwrite`, `overwrite_with`,
`list_namespaces`, and `list_tables` from `Catalog`, and the matching pass-
throughs from `Namespace`. Two spellings of one operation is the disease
`AGENTS.md:491` names; a one-line delegate is still a second spelling. This is
a breaking change to a pre-1.0 surface: make it in one commit, and say so.

Rust keeps `get -> Result` and does **not** implement `Index`: panic-on-missing
is normal for an in-memory child lookup and is not normal for a storage lookup
(`AGENTS.md:686`). Say that sentence in the docs, then give Python and
JavaScript the map spelling their readers expect (2.5).

### 1.2 Every level holds metadata

A catalog, a namespace, and a table each carry properties. The table already
does, through `TableMetadata`. The other two get one small document each,
written through `rust/src/json/` and never `serde_json` directly:
`metadata/catalog.json` under the warehouse, `metadata/namespace.json` under
the namespace folder. Absent means empty properties - never an error, never a
missing-file failure a caller has to catch.

That document also retires today's marker trick. Right now an empty namespace
is spelled by an empty `truncate(0)` on a folder handle; from here, a namespace
*exists* when its folder is there, and `namespace.json` is what makes an empty
one durable and what carries its properties. One artifact, two jobs it was
always doing separately.

Property access goes through the inert-property API already in the workspace,
with `iceberg:` reserved (`AGENTS.md:17`) - never a second metadata model.
Writes are transactional: a failure leaves the value unchanged
(`AGENTS.md:686`).

### 1.3 A table create makes its namespaces, by writing

`tables.create(name, schema)` **never creates a namespace in advance and never
checks for one.** Writing the table's first metadata document creates every
missing ancestor folder, because `IOBase` already creates the resource and its
parents on first write (`AGENTS.md:109`). The lazy auto-create asked for here
is therefore mostly the *removal* of code, not the addition of it.

Verify that claim on every backend - `local`, `arrowfs`, `Buffer`, `Coded` -
with a test per backend that creates `a.b.c.orders` into an empty warehouse and
asserts the table opens. Where a backend genuinely cannot create parents, fix
**the backend**, not the catalog. Only if a backend cannot be fixed does the
absence error trigger one repair and one retry, per the existence contract
(`AGENTS.md:183`) - and never a pre-walk of the ancestry.

`namespaces.create` stays, for a caller who wants an empty namespace or wants
properties on it before any table exists. It writes `namespace.json`, which is
a write, which creates the ancestry. Same mechanism, no second path.

Deletion stays absent, with the `AGENTS.md:310` reason restated where a reader
will look for it: the storage contract has no delete, and emulating one would
be worse than saying so.

### 1.4 The existence audit: remove every pre-check, everywhere

This is the global half, and it is not confined to Iceberg. `AGENTS.md:183` is
new and normative; sweep the workspace against it and remove every
probe-before-act. Known sites, as a starting point and not the whole list:

- `Namespaces::get` calls `contains` and then builds the view - two listings
  for one answer, and the answer is stale between them.
- `Namespaces::create` probes with `child_by` / `is_container` /
  `Table::locate`, then calls `open_or_create`, which performs the same probe
  again - three round trips to create a folder.
- `Tables::create` calls `contains` - a full `Table::locate` - and then
  `create_at` locates again.
- `Tables::open_or_create` resolves the dotted name twice and the handle twice.
- `local/file.rs:123`: `if !create && !path.exists()`.
- `io/roles.rs:157`: `if self.file_exists()`.

Each becomes one shape: **act; on typed absence, repair once and retry once; on
typed conflict, absorb or raise as the verb requires.** `get` raises absence,
`create` raises conflict, `open_or_create` absorbs both - three spellings, one
round trip each, one code path.

This needs absence and conflict to be **typed and branchable**, so add them to
`yggdryl::Error` with `is_absent()` / `is_conflict()` predicates - typed
variants with `expected`/`actual`/`path` fields, not an interpolated string a
caller has to match, and **no third enum** (`AGENTS.md:660`). Every backend
normalizes its own spelling into them once, at its boundary:
`std::io::ErrorKind::NotFound` and `AlreadyExists`, Arrow's and the object
stores' equivalents, and Doris's own `Label Already Exists` status when the
`LoadReport` of 3.9 lands.

**Prove the reduction, do not describe it.** A counting mock handle asserts the
exact number of backend calls: `tables.get` on an existing table, `tables.get`
on a missing one, `tables.create` into a three-level missing namespace,
`open_or_create` on both branches. The numbers go in the test as constants and
in the commit message as a before/after. The expression module's selector cost
test set this precedent - the number of backend calls is a behavior, not an
implementation detail.

### 1.5 Every listing is an iterator, not a collected list

The workspace collects everything it lists today, and the inventory is short
enough to fix in one pass (`AGENTS.md:226`):

| site | today | becomes |
| --- | --- | --- |
| `IOBase::ls` / `glob` / `children_where` (`io/mod.rs:428`, `459`, `604`), the `delegate_iobase!` macro (`io/mod.rs:241`), and every implementor - `local/folder.rs`, `arrowfs/{folder,file,path}.rs`, `generic/{holder,codec,media}.rs` | `Result<Vec<Holder>>` | a lazy iterator of `Result<Holder>` |
| `ArrowFileSystem::list` (`arrowfs/system.rs:47`) | `Result<Vec<FileInfo>>` | the same, at the backend seam |
| `Catalog::list_namespaces` / `list_tables` / `list_children`, `Namespaces::names`, `Tables::names` (`iceberg/catalog.rs:229`, `250`, `310`, `364`, `619`) | `Result<Vec<String>>` | the collection `iter()` of 1.1 |
| `Table::data_files` (`iceberg/table.rs:569`), `read_manifest_list` (`manifest.rs:385`), the plan's files | `Result<Vec<_>>` | iterators - a plan over ten thousand files must not build ten thousand entries to keep three |

**What stays owned, and why.** The contract's own distinction decides it:
`expire_snapshots`'s returned ids and a compaction's counts are *reports of
what an act did*, bounded by the act; `IOBase::partitions` is bounded by one
URL's path depth. Each keeps its `Vec` **with a comment naming the bound** - an
exemption with no stated bound is precisely what is being removed.

**One named iterator type per item kind.** `IOBase` is object-safe and stays
so: no `impl Iterator` in the trait, no bare `Box<dyn Iterator<…>>` in a public
signature. Note that `Children` is already taken (`text::Children`, re-exported
at the crate root) - pick another name rather than shadowing it.

**Laziness has to be real, not a `Vec` wrapped in `into_iter()`.** `ls`
recursive holds its frontier and not its result; `glob` descends fixed prefixes
and lists only what survives them; `children_where` and `children_matching`
prune a losing directory *before* listing under it, which they already do and
must keep doing. Prove it with the counting mock of 1.4: taking three entries
from a folder of ten thousand performs the backend calls three entries need,
and `.next()` on a glob whose first prefix loses touches nothing beneath it.

**Bindings get the language's lazy protocol, never a list.** Python `__iter__`
and generators, with `iterdir`, `glob`, and `rglob` lazy exactly as `pathlib`
is (`AGENTS.md:956`); JavaScript iterables and `for...of` (`AGENTS.md:1146`).
Neither collects on the way across the boundary.

**Benchmark time to first entry beside the full drain.** A listing benchmark
that only drains hides the property this change exists for. Add both to
`rust/benchmarks/io.rs` over a synthetic wide folder and report them together.

### 1.6 `IOBase::child_by` becomes `child_by_path`

`child_by` takes a **path**, not a name - `child_by(&segments.join("/"))` is how
the catalog reaches a nested namespace today - and the name does not say so.
That is the exact ambiguity `AGENTS.md:491` exists to prevent, and it is worth
one mechanical commit.

Rename it across the trait, the `delegate_iobase!` macro (`io/mod.rs:235`),
every implementor, and all ~100 call sites, including the doc examples and the
generated notebooks - edit the blocks and regenerate with
`python scripts/build_docs_notebooks.py`, never the notebooks themselves. It is
Rust-only: the bindings call it to implement `joinpath` and `/` and expose no
name of their own, so no binding signature changes and `.api-bindings.txt` must
come back **byte-identical**. Land it alone, separate from anything that
changes behavior, so the diff reviews at a glance.

### 1.7 Tests, benchmarks, bindings, and docs for part one

- **Tests** (`iceberg/catalog/tests.rs`): the cascade to depth; a dotted name
  and its cascaded form returning equal values; a table created into three
  missing namespace levels; two threads creating the same table converging or
  raising one typed conflict; properties round-tripping at all three levels; an
  empty namespace surviving a reopen; absence and conflict typed at every
  level; the call-count assertions from 1.4; and error messages naming the
  **full dotted path**, which `AGENTS.md:660` requires wherever recursion
  reaches more than one node.
- **Listing tests** (`io/tests.rs` and each backend's): an iterator that stops
  after three entries touches the backend three entries' worth; an entry that
  fails mid-listing yields one `Err` at that entry and then ends; the same
  listing over the same state yields the same order twice; a glob whose first
  prefix loses lists nothing beneath it; a recursive walk over a synthetic deep
  tree holds a bounded frontier - assert the peak, do not claim it. Every
  remaining `Vec` return in a listing position has a test *or* a comment naming
  its bound; neither means it was missed.
- **Rename verification**: after 1.6, `.api-bindings.txt` is byte-identical and
  `git diff --stat` for that commit shows no `python/` or `node/` signature
  change.
- **Benchmarks**: `rust/benchmarks/iceberg.rs` gains a `catalog_resolve` group
  - resolve by cascade, resolve by dotted name, and create into a missing
  ancestry - reporting **backend calls** beside wall time, since removing a
  probe is a round-trip saving before it is a CPU saving.
  `rust/benchmarks/io.rs` gains `listing`: **time to first entry** and full
  drain, over a folder of 10, 1 000, and 100 000 entries, flat and recursive,
  so the shape of the change is visible rather than asserted. Keep the existing
  Criterion group IDs stable (`AGENTS.md:435`).
- **Python**: `catalog.namespaces["sales"].tables["orders"]`,
  `catalog.namespaces["sales.eu"]`, `in`, `len`, iteration, `.keys()`,
  `.values()`, `.items()` - the mapping dunders `Field` metadata already uses
  (`AGENTS.md:956`). `__delitem__` is **not** bound, because removal is absent;
  say why rather than emulate it. `len` on a collection drains the iterator, so
  it costs a full listing - say that in the docstring rather than letting a
  reader assume it is free.
- **JavaScript**: Map-like inside the existing `iceberg` loader namespace and
  nowhere else (`AGENTS.md:1146`):
  `catalog.namespaces.get('sales').tables.get('orders')`, plus `has`, `size`,
  `keys`, `values`, `entries`, and `for...of`. No operator sugar; JS has none,
  and the docs say that instead of emulating it.
- **Docs**: rewrite `docs/iceberg.md`'s catalog section around the cascade in
  Rust → Python → JavaScript tabs; update every `ls`/`glob`/`children_where`
  example in `docs/io.md`, `docs/local.md`, `docs/generic.md`, and
  `docs/buffered.md` to the iterator shape; add the resource/collection shape
  to `docs/architecture.md`; point `docs/contributing.md` at the two new
  contracts in one line each.

**Definition of done, part one**: a user writes
`catalog.namespaces["sales.eu"].tables.create("orders", schema)` against an
empty warehouse in any of the three languages; the namespaces come into being
because the metadata document was written, not because anything checked for
them; the same table is reachable as `catalog.table("sales.eu.orders")`;
listing a folder of a hundred thousand entries and taking three costs three
entries' worth of backend calls; and the counting tests show fewer calls than
before, with the numbers in the commit message.

## 2. Part two: Iceberg v3, read and written in full, on the mechanisms that already exist

Some of v3 is already here. `FormatVersion::V3` parses and renders
(`iceberg/metadata.rs:36`), `next-row-id` and `first-row-id` are plumbed
through metadata and manifests (`metadata.rs:221`, `manifest.rs:172`),
`timestamp_ns` / `timestamptz_ns` / `unknown` are in the primitive type
(`types.rs:200`), and `initial-default` / `write-default` round-trip as schema
properties (`schema.rs:34`). What is missing is everything that makes those
useful, and one thing that is worse than missing:

> **The scan drops delete files on the floor.** `iceberg/scan.rs:305` skips
> every manifest whose content is not `Data`. A table with any deletes - v2
> position deletes, v2 equality deletes, v3 deletion vectors - currently reads
> **too many rows**, silently. That is a correctness bug in v2, not only a gap
> in v3, and it is the first thing this part fixes.

Deletion vectors are the v3 spelling of a thing the reader cannot do at all
yet, so the work is: build the delete path once, then let v3 be one of the
three sources feeding it.

### 2.1 The rule for this part: name the mechanism before writing the feature

Every v3 feature below is implemented by *reusing* something the workspace
already owns. Write this table into `docs/iceberg.md`, and treat it as a
review gate: **a large commit against a row that says "nothing new" means
something was re-implemented.**

| v3 feature | reuses | genuinely new |
| --- | --- | --- |
| Deletion vectors | `IOBase` + `Buffer`, the `MimeType`/`MediaType` registry, `rust/src/json/`, `rust/src/zstd/` | the Puffin container and the portable Roaring codec |
| Applying any delete | `expression::Bound::filter_reader` and the shared selection kernel | nothing - a delete becomes a `BooleanArray` the existing path consumes |
| Equality deletes | `Expression` - an equality delete file *is* a predicate | nothing |
| Position deletes (v2 read) | `rust/src/parquet/` through the three record methods | nothing |
| Row lineage | `Field` reserved ids, `io/partition.rs`'s inherited-column precedent, the existing retrying commit gate | the id assignment arithmetic |
| `variant` | the one shared `Value` tree and `TypedValue` | the Parquet Variant binary encoding, once, for three callers |
| `geometry` / `geography` | `DataType::Binary` plus `iceberg:` protocol-view properties, the existing bounds extraction | WKB bounding-box computation |
| `timestamp_ns` / `timestamptz_ns` | `TimeUnit` already owns nanoseconds; already mapped | nothing - finish the statistics and partition paths |
| `unknown` | `DataType::Null` | nothing - enforce the rules |
| Column defaults | `Field::default_value` and `IORecordOptions::cast_arrow_batch`'s completion step | nothing - one wire |
| Multi-argument transforms | `PartitionSpec::partition_field`'s existing stamp | arity in `Transform` |

### 2.2 Deletes at last: one mask, three sources

Planning **yields** each data file with the delete files that apply to it - the
spec's rule is sequence-number and partition based, so a delete applies when
its data sequence number is at or above the data file's. Per `AGENTS.md:226`
the plan is an iterator, so a table with ten thousand files and a predicate
matching three does not build ten thousand entries first; what the delete side
must hold - the delete manifests for the partitions still in play - is bounded
and says so in a comment. Then **three sources
produce one `BooleanArray`, and that mask goes through the selection path the
expression layer already owns** - `filter_reader` and the shared kernel, never
a hand-rolled compaction (`AGENTS.md:789`):

1. **Deletion vector (v3)** - a Puffin blob addressed by `referenced_data_file`
   / `content_offset` / `content_size_in_bytes` on the delete entry (field ids
   `143`, `144`, `145`; verify against the spec). Positions arrive sorted, so
   the mask is one linear pass.
3. **Position delete file (v2, read only)** - a Parquet file of
   `(file_path, pos)`, read through the existing Parquet reader. A v3 writer
   must never produce one; enforce that in 3.7.
4. **Equality delete file** - the elegant one. An equality delete file *is a
   predicate*: each row becomes an `And` of `Compare(Eq)` over the columns
   `equality_ids` names, the rows `Or` together, and the whole thing is
   negated. Hand that `Expression` to the layer that already binds, vectorizes,
   and gets three-valued null semantics right. **No new evaluation code at
   all** - and if this ends up longer than a screen, it was written wrong.

The delete mask composes with the scan's existing predicate residual by `and`,
in that order, so a row filtered by either is filtered once. The mask is built
once per data file, never per batch.

### 2.3 Puffin is a container module, not an Iceberg detail

`rust/src/puffin/` is a **sibling** of `ipc`, `avro`, and `parquet`, not a file
inside `iceberg/`. Puffin is a container format; the workspace's rule is one
module per encoding, and a table format sits on the encodings and never becomes
one (`AGENTS.md:17`, `AGENTS.md:310`). Iceberg sits on Puffin exactly as it
already sits on Avro.

| file | owns |
| --- | --- |
| `puffin/mod.rs` | `Puffin<H: IOBase>`, the wrapping handle, and the blob surface |
| `puffin/format.rs` | magic, footer framing, flags, the payload document |
| `puffin/blob.rs` | `BlobMetadata`, the blob types this build knows |
| `puffin/bitmap.rs` | the portable Roaring bitmap codec and its CRC framing |
| `puffin/tests.rs` | the module's edge cases |

It costs no dependency - bytes, `rust/src/json/`, and `rust/src/zstd/` are all
it needs - so it is unconditional the way Avro's codec is, and the docs say
why.

`Puffin<H>` is **a wrapping handle first** (`AGENTS.md:265`): it mirrors bytes
via `delegate_iobase!` so the file can be copied or handed to a foreign reader
unwrapped, and it caches its footer *only between `open` and `close`*. It is
**not** a record media and does not answer the three record methods - those are
for row encodings, and a blob container is not one. Say that sentence in the
module docs rather than leaving the next reader to wonder why the trait is not
implemented.

The format, exactly:

- Magic `PFA1` (`0x50 0x46 0x41 0x31`) at the head and at both ends of the
  footer. Layout: `Magic | Blob… | Footer`, footer
  `Magic | FooterPayload | FooterPayloadSize | Flags | Magic`.
- `FooterPayloadSize` is a 4-byte little-endian signed integer; `Flags` is 4
  bytes, and bit 0 of byte 0 says the payload is LZ4-compressed.
- The payload is UTF-8 JSON `{"blobs": [...], "properties": {...}}`, decoded
  through `rust/src/json/` and never `serde_json` directly. `BlobMetadata` is
  `type`, `fields`, `snapshot-id`, `sequence-number`, `offset`, `length`, plus
  optional `compression-codec` and `properties`.
- **LZ4 is not a coding this workspace owns.** A compressed footer or an
  `lz4` blob is refused with a typed error naming the codec - never a new
  dependency, never a silent skip. `zstd` goes through `rust/src/zstd/`.
- The `deletion-vector-v1` blob: a 4-byte big-endian combined length, the
  4-byte magic `D1 D3 39 64`, the portable Roaring bitmap little-endian, and a
  4-byte big-endian CRC-32. Required blob properties `referenced-data-file` and
  `cardinality`; compression is not permitted; `snapshot-id` and
  `sequence-number` are `-1`. Every one of those is a validation, not a
  comment.
- CRC-32 comes from `flate2`, which is already pinned and already exposes one.
  Hand-rolling a CRC when a pinned crate has it is the kind of specific
  implementation this part exists to avoid - check first, and only hand-roll
  with a note saying why.

The portable Roaring bitmap is 64-bit: a sorted map of 32-bit high keys to
32-bit Roaring bitmaps in the standard portable serialization, with array,
bitset, and run containers. Implement encode and decode, choose the container
type the format requires rather than always writing one kind, and test against
the official Roaring test vectors plus round trips of every shape - empty, a
single position, dense, sparse, run-heavy, spanning several high keys, and the
maximum position.

### 2.4 The v3 types, each on a mechanism that exists

**`variant` - name the collision before writing a line.** `DataType::variant`
in this workspace is dense-union sugar (`datatype/nested.rs:529`,
`AGENTS.md:708`) and has nothing whatever to do with the Iceberg/Parquet
VARIANT binary encoding. Do not overload it and do not rename it. The Iceberg
variant is one *encoding* of the shared `Value` tree, so it lands as
`rust/src/variant/`: `Value` ⇄ the Parquet Variant binary form (metadata buffer
plus value buffer). **One implementation, three callers** - Iceberg v3 columns,
Parquet's variant logical type, and the Doris `VARIANT` of section 3 - which is
precisely the rule this part is written under. On the Arrow side it is `Binary`
(or the two-field struct Parquet spells) carrying `iceberg:` metadata that
names it; never a new `DataType` variant, because the value model already has
one and it is `Value`.

**`geometry(C)` / `geography(C, A)`** are `Binary` holding WKB, with the CRS
and the edge algorithm in `iceberg:` protocol-view properties. Their bounds are
the spec's geospatial bounding box, computed from the WKB in the module that
already extracts bounds - not in a new one.

**`timestamp_ns` / `timestamptz_ns`** are already parsed and already mapped
(`types.rs:161`). What is missing is the rest of the path: single-value
serialization, statistics bounds, partition values, and the
`to_scheme_compat(&Scheme::ICEBERG)` widening. Finish those; add nothing.

**`unknown`** is already `DataType::Null`. The rules to enforce: it is always
optional, always defaults to null, and is never stored in a data file.

### 2.5 Row lineage, on the commit gate that exists

`_row_id` (`2147483540`) and `_last_updated_sequence_number` (`2147483539`) are
reserved metadata field ids; `first_row_id` is `142` on the data file and `520`
on the manifest list; a v3 snapshot carries `first-row-id` and `added-rows`;
the table carries `next-row-id`. Verify every one of those ids against the spec
before relying on it - they are transcribed here, not derived.

**Reading is inheritance, not storage.** A null `_row_id` resolves to the
manifest's `first_row_id`, plus the data file's, plus the row's position. The
workspace already has the precedent for materializing a column the file does
not carry: `io/partition.rs` restores partition columns from what the layout
knows. Row lineage is the same shape, and belongs beside it rather than in a
new mechanism.

**Writing goes through the retrying commit gate that already exists**
(`AGENTS.md:310`). `append` and `commit_changes` already *rebase* - the intent
re-applies against the winner's document - so assigning row ids against the
winner's `next-row-id` is a new **intent**, not a new gate. Do not add a second
commit path.

Test it where it can actually break: a v3 table appended, merged, and compacted,
with every row's inherited `_row_id` stable across all three; and two
concurrent appends producing disjoint ranges.

### 2.6 Defaults and multi-argument transforms are each one wire

`initial-default` and `write-default` already round-trip (`schema.rs:34`). What
is missing is *using* them: a column absent from a data file must read as its
`initial-default`, and filling a missing column is already the completion step
of `IORecordOptions::cast_arrow_batch` (`AGENTS.md:789`). Feed the Iceberg
default into `Field::default_value` and let the cast do what it already does.
One wire, not a feature - and if it turns into a feature, it went wrong.

Multi-argument transforms: `PartitionField` gains `source-ids` beside the
singular `source-id` it keeps for v1 and v2, and `Transform` gains the arity it
applies over. `PartitionSpec::partition_field` already stamps transform and
source onto tuple children - extend that stamp. A transform this build cannot
invert still refuses writes by name, exactly as `bucket` and `truncate` already
do.

### 2.7 What v3 forbids, enforced rather than documented

Extend the `validate` that already runs on load and before every commit, so a
broken document reads but never writes. Each of these is a typed error naming
both sides, and each is a test:

- a **position delete file written by a v3 writer** - deletion vectors only;
- **more than one deletion vector per data file**;
- a partition spec carrying a transform this build does not know;
- a **non-null default** on an `unknown`, `variant`, `geometry`, or
  `geography` column;
- a default value nested inside a struct field's default;
- a v3 snapshot or manifest **without `first-row-id`**;
- a row id assigned outside the range the spec reserves.

### 2.8 Out of scope for v3, named not emulated

- **Table encryption** (`encryption-keys`) - it needs a crypto dependency this
  workspace will not take. Name it as future work.
- **Puffin sketch blobs** (`apache-datasketches-theta-v1`) - the container
  reads and *skips them losslessly*, preserving them across a rewrite, but
  producing one needs a sketch implementation out of all proportion here.
- **LZ4** - refused by name, per 3.3.

### 2.9 Tests, benchmarks, bindings, and docs for v3

- **Interop is the bar, and it is two independent readers.** Extend
  `scripts/check_iceberg_interop.py` with a v3 half: PyIceberg writes a v3
  table carrying deletion vectors and reads ours back. Pin the PyIceberg
  version that actually writes v3 deletion vectors in the driver and name it in
  the output, so a green run says *which* implementation agreed; an older
  PyIceberg silently writing v2 is a pass that proves nothing. Then the Doris
  interop of section 6 reads the same v3 table, because Doris 4.1 reads V3
  Puffin deletion vectors. A v3 table that two unrelated engines agree on is settled;
  one that only PyIceberg agrees with is not.
- **Benchmarks**: `rust/benchmarks/iceberg.rs` gains `v3_deletes` - building a
  mask from a deletion vector, from a position delete file, and from an
  equality delete file, at 1%, 50%, and 99% deleted, against the kernel
  baseline of applying a hand-built `BooleanArray`. Report what a deletion
  vector saves over a position delete file in **bytes and in time**, since that
  saving is the entire reason v3 has them. A new `rust/benchmarks/puffin.rs`
  measures bitmap encode and decode across container shapes and footer
  read/write.
- **Bindings**: `FormatVersion.V3` reachable from both, `table.plan()`
  reporting deletes applied beside files read and skipped (pruning is already a
  testable number - deletes should be one too), and the Puffin reader exposed
  only where `.api-bindings.txt` says the shape fits; otherwise a
  `!!! note "Rust only"`.
- **Docs**: a v3 section in `docs/iceberg.md` built around the reuse table of
  3.1; a new `docs/puffin.md` and, if `rust/src/variant/` lands as its own
  module, a `docs/variant.md` - one page per core module folder is not optional
  (`AGENTS.md:435`), and both need nav entries.

**Definition of done, part two**: a table written here at format version 3 -
with deletion vectors, assigned row ids, a variant column, a geospatial column,
nanosecond timestamps, a defaulted column added after the fact, and a
multi-argument partition transform - is read back identically by PyIceberg and
by Apache Doris 4.1; a v2 table carrying position and equality deletes finally
returns the right rows; and the benchmark table says what a deletion vector
costs against the raw Arrow kernel and saves against a position delete file.

## 3. Part three: the Doris module, in architecture

### 3.1 Feature and module layout

New non-default feature in `rust/Cargo.toml`:

```toml
# Apache Doris 4.1 interoperability. Not default: it is an engine target on
# top of the record encodings, and a schema-only consumer never reaches it.
doris = ["arrow", "parquet"]
```

The Iceberg bridge inside it is `#[cfg(all(feature = "doris", feature =
"iceberg"))]`, so `--features doris` alone still compiles and still ships the
Parquet half. `--features "parquet iceberg doris"` is the full build the
extensions compile.

New module `rust/src/doris/`, categorized the way `iceberg/` is - modules own
real implementation, never empty shells around a monolith (`AGENTS.md:17`):

| file | owns |
| --- | --- |
| `mod.rs` | the `Doris` namespace value, shared state, re-exports, the module's one-paragraph statement of what is and is not in scope |
| `types.rs` | `DorisType`: the closed Doris 4.1 type enum, its grammar, its `Display`, and the two-way mapping against `DataType` |
| `schema.rs` | `Field` ↔ Doris table schema: the key model, column order, comments, nullability, defaults, and the `doris:*` field properties |
| `variant.rs` | `VARIANT` and `JSON`: schema templates, subcolumn projection, the `DataType` a variant path resolves to |
| `ddl.rs` | `CREATE TABLE`, `CREATE CATALOG`, `DESCRIBE`/`SHOW CREATE TABLE` - rendered and parsed |
| `sql.rs` | `Expression` → Doris SQL text: quoting, literal rendering, precedence, and the refusal list |
| `tvf.rs` | `S3()` / `HDFS()` / `FILE()` / `HTTP()` table-value-function text from a `Url`, a `RecordOptions`, and a predicate |
| `load.rs` | Stream Load: the header set, the body encoded through the three record methods, and `LoadReport` decoded from the JSON response |
| `export.rs` | reading what `EXPORT`, `SELECT INTO OUTFILE`, and `INSERT INTO ... SELECT FROM tvf()` left on storage |
| `catalog.rs` | the Doris external-catalog bridge: Iceberg and Hive catalog text, and the type-mapping check |
| `options.rs` | `DorisOptions`: every knob, resolved explicit → table property → default |
| `tests.rs` | the module's edge cases |

`Doris`, `DorisType`, `DorisOptions`, `StreamLoad`, `LoadReport` are re-exported
from `rust/src/lib.rs` behind the feature, beside the Iceberg exports.

### 3.2 What this module is, and what it is emphatically not

Say this in the module docs, in one short paragraph, so the next reader does
not re-open it:

**In scope.** Everything that is a *value*: the type system, the schema, the
DDL text, the predicate text, the wire *body*, the wire *headers*, the response
*document*, and the on-storage layout Doris reads and writes.

**Out of scope, named not emulated** (`AGENTS.md:310` sets this precedent for
the REST catalog and non-`main` branch writes):

- **No HTTP client.** `StreamLoad` produces a method, a URL, a header map, and
  a body handle. It never opens a socket. An HTTP `IOBase` backend is a sibling
  module and future work; when it exists, `StreamLoad` gains a one-line
  `send` that goes through it. Say that sentence in the docs.
- **No MySQL wire protocol.** Doris's query port is MySQL; that is a network
  client and a second wire format. DDL, catalog, and TVF statements are
  produced as *text* the caller executes with whatever driver it already has.
- **No Arrow Flight SQL client.** Doris 2.1+ serves query results over Flight
  SQL, and it is the fastest way to read from Doris - but it is gRPC, it is
  async, and `IOBase` is neither. Record the measurement it would win
  (published figures put it 20×-100× over the MySQL protocol) and name it as
  future work behind a Flight backend. Do not emulate it, and do not claim a
  number this workspace did not measure.
- **No BE-internal formats.** Segment V3, the tablet layout, and Doris's
  internal indexes are engine internals; the exchange surface is Parquet, ORC,
  CSV, JSON, Arrow IPC, and Iceberg.

### 3.3 `DorisType`: one closed enum, complete for 4.1

`DorisType` is a `#[non_exhaustive]` enum covering **every** type Doris 4.1
spells, grouped and documented as groups, with `Display` canonical and
round-tripping through `FromStr` (`AGENTS.md:686`):

- **Boolean and integers** — `Boolean`, `TinyInt`, `SmallInt`, `Int`,
  `BigInt`, `LargeInt` (16 bytes, signed, range ±2^127).
- **Floating and fixed point** — `Float`, `Double`, `Decimal { precision,
  scale }`.
- **Temporal** — `Date`, `DateTime { precision }` (0..=6, microsecond
  ceiling), `Time { precision }` (query-only: Doris will not store it - the
  mapping must refuse a stored column of it, naming the reason), and
  **`TimestampTz`**, new in 4.1: stored as UTC, converted on read.
- **String and binary** — `Char { length }` (1..=255 bytes),
  `Varchar { length }` (1..=65533 bytes), `String` (default 1 MiB, configurable
  to 2 GiB), `VarBinary` (4.0+, catalog-mapped only - a native Doris table
  cannot declare it, and the mapping says so).
- **Nested, fixed schema** — `Array(Box<DorisType>)`,
  `Map(Box<DorisType>, Box<DorisType>)`,
  `Struct(Arc<[(SmolStr, DorisType)]>)`. Recursive at every level and subject
  to `DataType::PARSE_RECURSION_LIMIT`.
- **Semi-structured** — `Json`, `Variant(Option<VariantTemplate>)` where the
  template is 4.x's `VARIANT<'id': INT, 'tags*': ARRAY<TEXT>>` schema-template
  syntax, wildcards included.
- **Aggregation state** — `Bitmap`, `Hll`, `QuantileState`,
  `AggState(Box<DorisType>)`. These have no logical Arrow shape; they map to
  opaque `Binary` with a `doris:agg-state` property recording the spelling, and
  the docs say plainly that a round trip through Yggdryl preserves the bytes
  and not the semantics.
- **Network** — `Ipv4`, `Ipv6`.

The grammar in `types.rs` follows the parser contract (`AGENTS.md:708`)
exactly: type keywords ASCII case-insensitive, names and quoted values keep
case and Unicode, split only at top-level separators honoring quoting and
escapes, reject trailing tokens and malformed numbers, enforce the recursion
limit, every error carries a byte position and context. It never re-implements
Arrow type parsing - where a Doris type is spelled with an Arrow-compatible
inner type, the text goes to `DataType::from_str`.

### 3.4 The mapping is total, two-way, and honest about what it loses

`types.rs` owns two functions and one table, and nothing else decides a type:

```rust
impl DorisType {
    pub fn to_data_type(&self) -> Result<DataType>;
    pub fn from_data_type(data_type: &DataType) -> Result<Self>;
}
```

The mapping table is written **into the module docs and into
`docs/doris.md` as one table**, generated from the same constant the code uses
so the two cannot drift. Every row states the direction it is lossless in.
The rows that are *not* symmetric are the interesting ones, and each gets a
sentence:

| Doris | `DataType` | note |
| --- | --- | --- |
| `LARGEINT` | `Decimal128(38, 0)` | Arrow has no 128-bit integer; the decimal carries the value exactly up to 38 digits and refuses beyond. `i128::MIN` does **not** fit - reject it by name, do not wrap. |
| `DATETIME(p)` | `Timestamp(unit, None)` | `p` 0..=6 maps to Second/Milli/Micro; Doris has no nanosecond, so a nanosecond timestamp is refused unless `safe` truncation is asked for, and then it is reported. |
| `TIMESTAMPTZ` | `Timestamp(Micro, Some("UTC"))` | Doris stores UTC and converts on read; the timezone is the session's, so a non-UTC Arrow timezone is normalized and the original recorded in `doris:timezone`. |
| `TIME(p)` | `Time64(Micro)` | read-only: Doris 4.1 will not store a `TIME` column. Refuse it on the write path, naming the version. |
| `CHAR(M)` / `VARCHAR(M)` | `Utf8` | `M` is in **bytes**, not characters. A declared length that a UTF-8 payload can exceed is a real failure mode: carry `M` in `doris:length` and validate on the write path. |
| `STRING` | `Utf8` / `LargeUtf8` | the 1 MiB default is a Doris config, not a format limit; name the config key. |
| `VARIANT` | `Utf8` (JSON text) or the template's `Struct` | with a template, the projection is exact and typed; without one, Doris infers - integers become `BIGINT`, decimals become `DOUBLE`, a path with mixed types is promoted to `JSONB`. Say that; do not pretend inference is stable. |
| `BITMAP`/`HLL`/`QUANTILE_STATE`/`AGG_STATE` | `Binary` | bytes preserved, semantics not. |
| `IPV4` / `IPV6` | `FixedSizeBinary(4)` / `FixedSizeBinary(16)` | with `doris:ip` recording which, so the reverse mapping is exact. |
| `ARRAY<T>` | `List(T)` | Doris arrays are always nullable-element; a non-nullable Arrow list element is widened and the widening is reported. |
| `MAP<K,V>` | `Map(K, V)` | Doris map keys are non-null scalars only; a nested or nullable key is refused naming both. |
| `STRUCT<...>` | `Struct(...)` | names are case-insensitive on both sides; an ambiguous fold is refused, never silently picked. |

Arrow types Doris has no home for - `Interval`, `Duration`, `Union`,
`RunEndEncoded`, `Float16`, `Dictionary` of a non-string value, `Decimal256`
beyond 38 digits - are each refused **by name** with `expected X, got Y`
(`AGENTS.md:660`), except where the compatibility walker can widen them
losslessly (see 3.5).

### 3.5 Doris as the sixth compatibility target

Add `Scheme::DORIS` to `COMPATIBILITY_TARGETS` (`enums/scheme.rs:91`) and a
Doris row to the per-target scalar matrix in
`rust/src/datatype/compatibility.rs`. `schema.to_scheme_compat(&Scheme::DORIS)`
returns the schema Doris can actually store, widening exactly what widens
losslessly and refusing the rest:

- `Float16 → Float32`, `Dictionary(_, Utf8) → Utf8`, `RunEndEncoded(_, T) → T`,
  `LargeList → List`, `Utf8View → Utf8`, `Decimal256(p,s)` with `p ≤ 38` →
  `Decimal128(p,s)`;
- `Timestamp(Nano, _) → Timestamp(Micro, _)` **only** when the caller asked for
  it; silently dropping precision is a correctness bug, not a widening;
- `Union`, `Interval`, `Duration`, and `Decimal256` beyond 38 digits are
  refused, naming both sides.

Never fork the walker (`AGENTS.md:789`). Rewrites preserve name, nullability,
and metadata, and invalidate a populated Arrow cache exactly once.

`Field::doris` joins the existing protocol views as the one way to reach
`doris:*` properties - `doris:key-type`, `doris:key`, `doris:aggregate`,
`doris:distribution`, `doris:buckets`, `doris:length`, `doris:ip`,
`doris:agg-state`, `doris:variant-template`, `doris:auto-partition`,
`doris:default`, `doris:comment`. It is a *view* over the one shared snapshot,
not a second map (`AGENTS.md:17`).

### 3.6 `schema.rs` and `ddl.rs`: a `Field` is the table

A struct root `Field` is the schema (`AGENTS.md:17`); `ddl.rs` renders it and
parses it back.

```rust
let sql = doris::create_table(&schema, &DorisOptions::default())?;
let back = doris::schema_from_create_table(&sql)?;
assert_eq!(back, schema);
```

- **Key models**: `DUPLICATE KEY`, `UNIQUE KEY` (merge-on-write), and
  `AGGREGATE KEY` with the per-column aggregation function. The model comes
  from `doris:key-type` on the root and `doris:key` / `doris:aggregate` on the
  columns; absent, it is `DUPLICATE KEY` over the leading columns Doris
  requires, and the docs say which.
- **Distribution**: `DISTRIBUTED BY HASH(...) BUCKETS n` or
  `... BUCKETS AUTO` or `RANDOM`, from `doris:distribution` / `doris:buckets`.
- **Partitioning**: `PARTITION BY RANGE(...)` / `LIST(...)` and 4.x
  `AUTO PARTITION BY RANGE (date_trunc(col, 'day'))`. The partition columns are
  the schema's partition-marked fields - the same marker Iceberg and the Hive
  folder layout already use (`AGENTS.md:109`). One authority on partition
  columns, not a Doris-specific one.
- **Properties**: `replication_num`, `storage_medium`,
  `enable_unique_key_merge_on_write`, `light_schema_change`,
  `variant_max_subcolumns_count`, `variant_enable_flatten_nested`, and the
  4.1 `store_row_column` / DOC-mode keys - each resolved through
  `DorisOptions`, never interpolated ad hoc.
- **Parsing back** is the same recursive grammar discipline: `SHOW CREATE
  TABLE` output and `DESCRIBE` output both round-trip to the same `Field`, and
  every branch gets a round-trip test and an adversarial test
  (`AGENTS.md:708`).

### 3.7 `sql.rs`: the one predicate, rendered for Doris

`Expression` is already the workspace's single filter representation. `sql.rs`
renders a `Bound` predicate as Doris SQL, and renders **nothing else**:

```rust
let predicate: Expression = "ccy = 'EUR' and price > 100 and ts >= timestamp '2026-01-01'".parse()?;
assert_eq!(doris::sql::predicate(&predicate)?, "`ccy` = 'EUR' AND `price` > 100 AND `ts` >= '2026-01-01 00:00:00'");
```

- Identifiers are backtick-quoted with backticks doubled; string literals are
  single-quoted with Doris's escape rules; decimals never become floats;
  temporals render in the exact literal form Doris parses.
- Nodes Doris cannot express (a `&holder.*` selector, a function Doris does not
  have) are **not** rendered - they come back as the *residual* through the
  existing `pushdown.rs` split, exactly as Iceberg's residual already works.
  A caller gets `(pushed_sql, residual_expression)`, applies the first in
  Doris and the second on the batches. Generalize the existing split; do not
  fork it.
- The refusal list is documented: what Doris will be asked, what stays behind,
  and why.

### 3.8 `tvf.rs` and `export.rs`: the round trip that validates the encodings

This is the pair the whole task exists for.

**Out**: Yggdryl writes Parquet (or Iceberg) through the three record methods,
then `tvf.rs` renders the exact statement that makes Doris read it:

```rust
let statement = doris::tvf::select(&url, &options)
    .project(&["ccy", "price"])
    .filter(&predicate)
    .to_string();
// SELECT `ccy`, `price` FROM S3('uri' = 's3://bucket/trades/*.parquet', 'format' = 'parquet', ...)
//  WHERE `ccy` = 'EUR' AND `price` > 100
```

The TVF kind comes from the `Url`'s scheme (`s3`/`oss`/`cos` → `S3()`,
`hdfs` → `HDFS()`, `file` → `LOCAL()`/`FILE()`, `http`/`https` → `HTTP()`,
4.0.2+), the `format` property from the media type via
`RecordOptions::for_media_type` (`AGENTS.md:109` - the encoding is never
guessed and never an argument), and the credential properties from inert
`s3:*` / `hdfs:*` protocol metadata already on the `Field` or `Url`. Path
wildcards (`file_*`, `file_{1..3}`, `file_{a,b}`) come from
`Url::is_glob`/`glob_parts` - never a second glob spelling.

**Back**: Doris writes with `EXPORT`, `SELECT INTO OUTFILE`, or 4.1's
`INSERT INTO FILE()/S3()`; `export.rs` reads that folder back, streaming its
leaves per `AGENTS.md:226` - an export of ten thousand files is read the same
way as one of three. It is a
`Holder` over a folder and nothing more (`AGENTS.md:109` - a handle plus
`RecordOptions` is the whole surface, no dataset type): the leaves' media type
selects the encoding, the folder's `column=value` layout restores partition
columns through `io/partition.rs`, and the caller gets a `BatchReader`. Doris's
own naming convention (the `<label>_<n>.parquet` suffix, the optional success
marker) is recognized, and anything unrecognized is *reported*, never skipped
silently.

**Symmetry is the assertion**: rows written → rows Doris reads → rows Doris
exports → rows read back must be identical, value for value, including nulls,
decimals to the scale, timestamps to the unit, and every nested level.

### 3.9 `load.rs`: Stream Load, composed not sent

```rust
let load = doris::StreamLoad::new("trades", "orders")
    .with_format(MimeType::PARQUET)
    .with_label("2026-08-19-batch-7")
    .with_merge(MergeType::Merge)
    .with_options(&options);
let (headers, body) = load.compose(reader)?;   // body is an IOBase handle
```

- The URL shape is `PUT /api/{db}/{table}/_stream_load`, with the FE→BE
  307 redirect documented (the FE picks a BE round-robin as coordinator).
- The header set is a typed value, not a string map the caller fills: `label`,
  `format` (`csv`, `csv_with_names`, `csv_with_names_and_types`, `json`,
  `parquet`, `orc`, `arrow`), `column_separator`, `line_delimiter`, `columns`,
  `jsonpaths`, `json_root`, `strip_outer_array`, `where`, `partitions`,
  `max_filter_ratio`, `strict_mode`, `timezone`, `timeout`, `merge_type`,
  `delete`, `compress_type` (**including 4.1.3's zstd**), `enclose`, `escape`,
  `skip_lines`, `trim_double_quotes`, `hidden_columns`,
  `function_column.sequence_col`, `unique_key_update_mode`,
  `partial_update_new_key_behavior`, `two_phase_commit`, `group_commit`, and
  4.x's `compute_group`. Each is one field, each resolved through
  `DorisOptions` (explicit → property → default), each with a typed error
  naming key and value when unparseable - never a silent default
  (`AGENTS.md:310` states this rule for Iceberg's size key; it holds here).
- The **body is produced through the three record methods**, into a `Buffer`
  or any other `IOBase` handle. `format: arrow` writes an Arrow IPC stream
  through `rust/src/ipc/`; `format: parquet` through `rust/src/parquet/`;
  `format: json`/`csv` through the shared text tier. There is no fourth
  writer, and nothing is collected that could stream: the body handle is
  written by a `BatchReader` and read back by the caller in chunks, so a load
  larger than memory is expressible (`AGENTS.md:109`).
- `LoadReport::from_json` decodes the response document through
  `rust/src/json/` into typed fields - `TxnId`, `Label`, `Status`
  (`Success` | `Publish Timeout` | `Label Already Exists` | `Fail`),
  `ExistingJobStatus`, `Message`, `NumberTotalRows`, `NumberLoadedRows`,
  `NumberFilteredRows`, `NumberUnselectedRows`, `LoadBytes`, `LoadTimeMs`,
  and the per-phase timings, plus `ErrorURL`. A non-`Success` status is a typed
  error carrying the message and the error URL, so a caller can fix the input
  without reading source.

### 3.10 `catalog.rs`: the Iceberg bridge

Behind `#[cfg(all(feature = "doris", feature = "iceberg"))]`:

- `doris::catalog::create_catalog(&spec)` renders the `CREATE CATALOG`
  statement for the catalog types Doris 4.1 supports - `hms`, `rest`,
  `hadoop`, `glue`, `dlf`, `s3tables`, and 4.1.0's experimental `jdbc`
  (PostgreSQL/MySQL/SQLite) - from the same inert protocol metadata the
  `iceberg::Catalog` already carries. A warehouse folder Yggdryl created is
  addressable as a `hadoop` catalog with no further configuration; say that,
  and show it.
- `doris::catalog::check_readable(&table)` walks a committed
  `iceberg::Table`'s schema through `DorisType::from_data_type` and reports,
  per column, whether Doris 4.1 can read it - so a table is known unreadable
  *before* an interop run says so. It never mutates the table.
- The support matrix is documented as fact, not aspiration: Doris 4.1 reads
  and writes Iceberg **V1 and V2** fully (`INSERT INTO`, `INSERT OVERWRITE`,
  `UPDATE`, `DELETE`, `MERGE INTO`, `CTAS`), and reads **V3** including
  Puffin-format deletion vectors, with V3 write support arriving through the
  same. Position deletes and equality deletes are both read. Time travel is
  `FOR TIME AS OF` / `FOR VERSION AS OF`, and branches and tags are
  `table@branch(name)` / `table@tag(name)`.
- **After part two, Yggdryl writes V3**, so the interop exercises V2 *and* V3
  in both directions - and the V3 half is the most valuable test in this whole
  prompt, because Doris reads Puffin deletion vectors with a C++ implementation
  that shares no code with ours and none with PyIceberg's. A deletion vector
  three implementations agree on is a deletion vector that is right. Where
  Doris's V3 write support does not yet cover something, say which thing rather
  than implying the round trip was complete.

---

## 4. Order of work (`AGENTS.md:9` — Rust first, fully)

**Phase 0** — section 1: the catalog hierarchy and the existence audit, Rust
then both bindings then docs, landed and green. → **Phase 1** — section 2: the
delete path first, then Puffin, then the v3 types, lineage, defaults, and
transforms, with the PyIceberg v3 interop green. → **Phase 2** research note
for Doris → **Phase 3** Rust core of `doris/` complete (types, mapping, compat
target, schema, DDL, variant, SQL, TVF, load, export, catalog, options, tests,
interop, benches, docs) → **Phase 4** optimization pass → **Phase 5** Python →
**Phase 6** JavaScript → **Phase 7** docs and benchmark tables → **Phase 8**
required checks.

Phases 0, 1, and 3 each stop as complete work. Do not run them out of order or
in parallel: the Doris bridge is the first new caller of the catalog hierarchy
and the second reader of v3, so it should be written against shapes that are
staying. In particular, **do not write the Doris interop before v3 writing
works** - the strongest thing that interop can prove is that Doris reads the
v3 deletion vectors this workspace produced.

---

## 5. Phase 2: pin the target, from the sources

Before writing the enum, spend one pass on the primary sources and write
`docs/doris.md`'s design section from it - short, cited, opinionated, not a
survey. **Verify every number below against the live docs; they are what this
prompt was written from, in August 2026, and they are what you must confirm or
correct:**

- **The version.** Target **Apache Doris 4.1**, latest patch **4.1.3**
  (2026-07-13; 4.1.0 was 2026-04-21, 4.1.1 2026-05-24, 4.1.2 2026-06-17). The
  4.0 line is still receiving patches (4.0.8, 2026-08-14) - support 4.0 as the
  floor and gate 4.1-only spellings (`TIMESTAMPTZ`, `MERGE INTO`, `UNNEST`,
  variant DOC mode, `INSERT INTO FILE()`, zstd stream-load compression) behind
  a declared version so a 4.0 cluster gets a typed refusal instead of a syntax
  error from the server.
  <https://doris.apache.org/releases/core/>
- **What 4.1 changed that this module must know**: `TIMESTAMPTZ`; Segment V3
  (external column metadata separation, sparse-column sharding, DOC mode for
  deferred JSON materialization); full Iceberg V2/V3 with `MERGE INTO`;
  Iceberg sorted write and manifest cache; a Parquet page cache reported at
  >20% on scans; `UNNEST`; recursive CTE; `ASOF JOIN`; JDBC Iceberg catalog;
  vector indexing (out of scope, but do not let the enum forget the column
  types it introduces).
  <https://doris.apache.org/releases/v4.1/release-4.1.0/>
- **The type system**, every type and every parameter:
  <https://doris.apache.org/docs/4.x/sql-manual/basic-element/sql-data-types/data-type-overview>
- **VARIANT**, including schema templates, `variant_max_subcolumns_count`
  (default 2048, practical ceiling ~10 000), sparse-column sharding, the
  BIGINT/DOUBLE/JSONB inference rules, and DOC mode:
  <https://doris.apache.org/docs/4.x/sql-manual/basic-element/sql-data-types/semi-structured/VARIANT>
- **Stream Load**, every header and every response field:
  <https://doris.apache.org/docs/4.x/data-operate/import/import-way/stream-load-manual>
- **EXPORT / SELECT INTO OUTFILE**, formats and compression (Parquet: SNAPPY
  default, plus GZIP, BROTLI, ZSTD, LZ4, PLAIN; ORC: ZLIB default, plus PLAIN,
  SNAPPY, ZSTD; `max_file_size` 5 MiB..2 GiB, default 1 GiB):
  <https://doris.apache.org/docs/4.x/sql-manual/sql-statements/data-modification/load-and-export/EXPORT>
- **Table value functions**, syntax, properties, wildcards, and 4.1.0's
  `INSERT INTO tvf()` export:
  <https://doris.apache.org/docs/4.x/lakehouse/file-analysis>
- **The Iceberg catalog**, catalog types and the operation matrix:
  <https://doris.apache.org/docs/4.x/lakehouse/catalogs/iceberg-catalog>
- **Arrow Flight SQL**, for the future-work note and nothing else:
  <https://doris.apache.org/docs/4.x/db-connect/arrow-flight-sql-connect>

Read the **Doris source** where the docs are ambiguous - `apache/doris` on
GitHub - specifically the Parquet reader (`be/src/vec/exec/format/parquet/`)
and the Iceberg reader, because the questions this task actually answers are
"which Parquet logical types does Doris's own C++ reader accept" and "which
Iceberg V2 delete shapes does it apply". A doc sentence is weaker evidence
than the reader.

**The deliverable of this phase is a decision list in the docs**: which Doris
types map losslessly, which map lossily and how, which are refused, which
spellings are 4.1-only, and which surfaces (Flight SQL, MySQL wire, Segment
V3) are deliberately out of scope with the reason.

---

## 6. Phase 3 details: tests

`rust/src/doris/tests.rs` plus per-file test modules where the existing modules
keep them, and the interop target below. Cover, at minimum:

### 6.1 Every type, exhaustively

A single table-driven test walks **every** `DorisType` variant and asserts, for
each: `Display` round-trips through `FromStr`; `to_data_type` then
`from_data_type` returns the original or a documented widening; the error
message for a refused mapping names both sides. A `#[test]` that iterates a
`const ALL: [DorisType; N]` and fails when a new variant is added without a
case - so the exhaustiveness is enforced by the compiler and the test, not by
review.

Parameters are boundary-tested, not sampled: `CHAR(1)`, `CHAR(255)`,
`CHAR(256)` refused; `VARCHAR(1)`, `VARCHAR(65533)`, `VARCHAR(65534)` refused;
`DECIMAL(1,0)`, `DECIMAL(38,38)`, `DECIMAL(39,0)` refused; `DATETIME(0)`,
`DATETIME(6)`, `DATETIME(7)` refused; `LARGEINT` at ±(2^127−1) and the refusal
at `i128::MIN`.

### 6.2 Deep nesting

The nesting tests are not decoration - they are where a mapping fails in
production. Build and round-trip, in **both** directions and through **both**
Parquet and Iceberg:

- `ARRAY<ARRAY<ARRAY<INT>>>` — three levels;
- `MAP<STRING, ARRAY<STRUCT<a: INT, b: MAP<STRING, DECIMAL(18,4)>>>>` — the
  four-way alternation that breaks naive mappers;
- `STRUCT` nested to the recursion limit, and one past it (the error carries
  the byte position and the path);
- a struct whose children are every scalar type, inside a list, inside a map
  value — so one fixture exercises the whole scalar matrix at depth;
- nulls at **every** level: a null map, a map with a null value, a list with
  null elements, a struct with all-null children, a non-null struct containing
  a null list containing non-null structs. Wrapper exposure must not make
  hidden child nulls observable (`AGENTS.md:789`).
- `VARIANT` with and without a template, including a path whose type changes
  between rows (the JSONB promotion), a path count over
  `variant_max_subcolumns_count`, and a nested object inside an array (which
  Doris flattens differently - assert what it actually does, do not assume).

### 6.3 DDL and SQL

Round trip `Field` → `CREATE TABLE` → `Field` for every key model, every
distribution, range/list/auto partitioning, and every property. Parse real
`SHOW CREATE TABLE` and `DESCRIBE` output captured from a live 4.1 into
fixtures. Adversarial: unbalanced backticks, a comment containing a backtick, a
default value containing a quote, a duplicate column name, a partition column
not in the schema, trailing tokens. Every error carries a byte position.

Predicate rendering: an assertion table of `Expression` → Doris SQL for every
node kind, plus the residual split for every node Doris cannot take.

### 6.4 Stream Load and export

Header composition for every format and every option combination, with a
golden-file assertion. Body bytes for `parquet`, `arrow`, `json`, and `csv`
from the same `BatchReader`, each decoded back through the matching Yggdryl
reader and compared row for row. `LoadReport` decoding for every `Status`,
including a malformed document and a document missing a field. A body larger
than the batch size streams: assert peak retained batches is one, do not claim
it (`AGENTS.md:789`).

Export reading: a folder of Doris-named Parquet files, a folder of ORC, a
folder of CSV with and without the header variants, a Hive-partitioned export
whose partition columns must come back typed, an empty export, and a folder
containing one unrecognized file (reported, not skipped).

### 6.5 Interop, both directions — `rust/tests/doris_interop.rs`

Copy the Iceberg harness pattern exactly (`AGENTS.md:310` - exchange formats
are validated against an outside implementation):

- `scripts/check_doris_interop.py` is the driver. It brings up Apache Doris
  **4.1.3** in Docker (FE + BE, the official `apache/doris` images), waits for
  readiness, and runs both halves.
- **Half one, Yggdryl → Doris.** The cargo target writes, into
  `target/doris-interop/from-rust`: a Parquet file with every scalar type; a
  Parquet file with the deep-nested fixtures from 6.2; a partitioned Parquet
  folder; and an Iceberg V2 table with an append and a key-matched merge. The
  driver then makes Doris read each one - the Parquet through
  `SELECT ... FROM LOCAL()/S3()`, the Iceberg through a `hadoop` catalog it
  creates from the warehouse folder - and compares **every row and every
  nested value** against what was written, not just counts.
- **Half two, Doris → Yggdryl.** Doris writes the same rows out with
  `INSERT INTO FILE()` (Parquet, ORC, CSV) and commits an Iceberg V2 table with
  `INSERT INTO`, an `UPDATE`, a `DELETE`, and a `MERGE INTO` - so the table
  carries position deletes and equality deletes. The cargo target reads all of
  it back and asserts the rows.
- **Half three, the wire.** The driver sends a real Stream Load with the body
  and headers `load.rs` composed, for `parquet`, `arrow`, `json`, and `csv`,
  and asserts the returned `LoadReport` matches what `LoadReport::from_json`
  decodes and that `NumberLoadedRows` equals the rows written.
- Run alone, the Rust target prints **`SKIPPED`** when the external artifacts
  are absent, and the driver fails on that word - so a skipped half can never
  read as a pass.
- A CI job runs the driver; it is not part of `cargo test`, because it needs a
  server. `docs/testing.md` says how to run it locally.

---

## 7. Phase 3 benchmarks — where the protocols get validated

`rust/benchmarks/doris.rs` with the dispatcher pattern
(`#[path = "doris/mod.rs"] mod benchmarks;`, stable Criterion group IDs), plus
the interop-driven measurements in `scripts/check_doris_interop.py`. The point
of this benchmark is **not** to show Yggdryl is fast at rendering SQL. It is to
put a number on the read/write protocols against an engine that did not come
from this workspace.

**In-process Criterion groups** (no server):

- `doris_types` — mapping and grammar: `DorisType::from_str` on a scalar, on
  the four-way nested type, on a variant template; `to_data_type` /
  `from_data_type` both directions. Baseline: the existing
  `DataType::from_str` groups, so the reader sees this grammar costs the same
  order as the schema grammar.
- `doris_ddl` — `CREATE TABLE` render and parse against a 10-column, a
  200-column, and a deeply-nested schema.
- `doris_sql` — predicate rendering and the residual split, against the
  `expression_bind` groups.
- `doris_load` — body composition per format for a 1e6-row batch, reported as
  **throughput in rows and in bytes-on-the-wire**, so the four formats are
  directly comparable. This is the number that decides which format a caller
  should use, and it is the first thing the docs table shows.
- `doris_export` — reading a Doris-shaped export folder, against reading the
  same bytes as a plain Parquet folder, so the layout handling's cost is
  visible as a delta and not a total.

**Server-driven measurements** (the driver, release build, numbers into
`docs/benchmarks.md`, regenerated never edited, naming machine, Doris version,
and build profile):

1. **Write protocol.** Yggdryl writes N rows to Parquet; Doris reads them via
   TVF. Report Yggdryl's write time, the file bytes, and Doris's scan time.
   Baseline: the **same rows written by PyArrow**, read by the same Doris query.
   If Doris scans Yggdryl's Parquet slower than PyArrow's, that is a real
   finding about row-group sizing or encodings - chase it, do not report it and
   move on.
2. **Read protocol.** Doris exports N rows to Parquet and ORC; Yggdryl reads
   them. Baseline: PyArrow reading the identical files. Same rule.
3. **Iceberg both directions.** Yggdryl commits a partitioned V2 table; Doris
   plans and scans it - report files read vs skipped, which the plan already
   reports as a testable number (`AGENTS.md:310`). Then Doris commits with
   `MERGE INTO`; Yggdryl plans and scans that. Baseline: PyIceberg on both
   tables, which the workspace already runs.
5. **Stream Load formats.** The same 1e6 rows loaded four ways -
   `parquet`, `arrow`, `json`, `csv` - reporting bytes on the wire, server-side
   `LoadTimeMs`, `ReadDataTimeMs`, and `WriteDataTimeMs` from the real
   `LoadReport`. Plus zstd `compress_type` (4.1.3) against uncompressed.
6. **Pushdown.** The same query with and without the projection and predicate
   the TVF renderer emits, reporting bytes Doris actually read. Pushdown that
   does not reduce bytes read is pushdown that does not work.

Python and JavaScript benchmarks compare the boundary crossing against
implementations the reader trusts on the same payload: `pyarrow.parquet` and
`pyarrow.dataset`, and - if available in the bench environment without becoming
a package dependency - the `mysql-connector-python` and ADBC paths a Doris user
would otherwise take.

---

## 8. Phase 4: the optimization pass (find them, measure them, then land them)

This is a distinct phase with a distinct rule from `AGENTS.md:789`: **measure
before claiming any optimization**, and an optimization that changes observable
behavior is a bug. Work the list below; for each item, land it with a
benchmark delta, or record in `docs/doris.md` that it was tried and did not pay
- a refused optimization with a number beside it is a real deliverable.

1. **Parquet writer settings Doris's reader actually likes.** Sweep row-group
   size, page size, dictionary encoding, compression (SNAPPY / ZSTD / LZ4), and
   statistics granularity, measured by *Doris's* scan time, not ours. Doris
   4.1 added a Parquet page cache reported at >20%; find the page size that
   cooperates with it. Land the winning defaults in `DorisOptions` with the
   measurement in the commit message.
2. **Page-level pruning metadata, which nothing here writes deliberately
   today.** The pinned `parquet` crate can emit the column and offset index
   (page-level statistics) and, per column, a **bloom filter**. Doris's reader
   uses both to skip pages and row groups a predicate cannot match, so this is
   the optimization with the most leverage in the list: measure Doris's scan
   time and bytes read for an equality predicate on a high-cardinality column
   with the bloom filter on and off, and for a range predicate with the page
   index present and absent. Land what pays as `IORecordOptions` settings -
   flattened fields like every other setting - and record what did not. Check
   what the crate writes by default before assuming anything is missing.
3. **Format choice for Stream Load.** Rank `arrow`, `parquet`, `json`, `csv` by
   bytes-on-the-wire and by server-side load time. Arrow IPC should win on CPU
   (no serialization on either side) and lose on bytes; prove which matters at
   which row count and document the crossover.
4. **Streaming body with a bounded buffer.** The composed body must never
   require holding the whole load. Use chunked transfer encoding, hold one
   batch, and assert the peak.
5. **zstd stream-load compression** (4.1.3): measure the bytes/CPU trade
   against uncompressed on the same payload.
6. **Projection and predicate pushdown into the TVF text**, measured as bytes
   Doris read - see benchmark 6 above.
7. **Iceberg write alignment.** Doris 4.1 added Iceberg sorted write and a
   manifest cache. Check whether Yggdryl's `write.target-file-size-bytes`
   default and its manifest granularity cooperate with Doris's planner; a
   manifest layout that defeats Doris's cache is a real cost with a real
   number.
8. **Allocation on the hot paths.** DDL rendering, predicate rendering, and
   header composition must not allocate per column or per node beyond the
   single output buffer; core scalar construction, getters, and iteration setup
   must not allocate at all (`AGENTS.md:789`). Prove it with an allocation
   baseline the way the codec benchmarks already do.
9. **One mapping table, computed once.** The Doris↔Arrow matrix is a `const`
   consulted by index, never a match chain re-walked per column, and never a
   per-record map.

Every landed optimization is invisible to a caller and stated in a comment; a
silent cap - a top-N, a sampling, a truncation - is worse than none.

---

## 9. Phase 5: Python binding

`python/src/doris.rs` exposing a `yggdryl.doris` namespace over the native
module - no Python-side type mapping, no Python-side SQL rendering
(`AGENTS.md:923`):

- `doris.DorisType.parse("map<string, array<struct<a:int>>>")`,
  `.to_data_type()`, `DorisType.from_data_type(dt)`, `str(...)` canonical.
- `doris.create_table(schema, **options) -> str`,
  `doris.schema_from_create_table(sql) -> Field`.
- `doris.tvf(url, **options).project([...]).filter("price > 100") -> str`,
  accepting `str | Expression` for the filter exactly as the existing filter
  arguments do.
- `doris.StreamLoad(db, table, format=..., label=...)` with
  `.compose(reader) -> (dict[str, str], IOBase)`, and
  `doris.LoadReport.from_json(...)`. The docstring says, in one line, that
  nothing is sent.
- `doris.check_readable(table)` for the Iceberg bridge.
- `schema.to_scheme_compat("doris")` works through the existing method with no
  new spelling; `Field.doris` joins the protocol views.
- `_native.pyi` and `__init__.pyi` updated; `mypy --strict` green.
- `python/tests/test_doris.py` in house style: the full type matrix, the deep
  nesting fixtures, DDL round trips, and - as the outside check - a live-Doris
  test marked `@pytest.mark.doris` that skips loudly when no server is
  configured, exercising Stream Load and the TVF round trip against the same
  container the interop driver uses.

## 10. Phase 6: JavaScript binding

`node/src/doris.rs`, mirroring the Python surface with camelCase names in a
`doris` loader namespace, the way `iceberg` already is a namespace
(`AGENTS.md:1146`): `DorisType.parse`, `dorisType.toDataType()`,
`doris.createTable(schema, options)`, `doris.schemaFromCreateTable(sql)`,
`doris.tvf(url, options).project([...]).filter('price > 100')`,
`doris.StreamLoad`, `doris.LoadReport.fromJson`. 64-bit values cross as
`bigint`; `LARGEINT` crosses as a decimal string and the docs say why. Errors
surface the native message unchanged. `node/tests/doris.test.js` +
`doris.types.ts` (node:test + `tsc --noEmit`): the type matrix, nesting, DDL
round trips, and type-level checks for the builder.

---

## 11. Phase 7: documentation

- New page `docs/doris.md` — one H1, exactly one opening sentence, then
  example-first sections: map a schema; the full type table (generated, not
  hand-kept); create a table; render a TVF read; compose a Stream Load; read an
  export; the Iceberg bridge; the decision list from Phase 2; the scope note
  from 3.2 saying plainly what is not here and why; the optimization findings
  from Phase 4.
- Every example in **Rust → Python → JavaScript tabs, in that order**, each
  idiomatic, self-contained, with at least one assertion, all passing
  `python scripts/check_docs_examples.py`. Check `.api-bindings.txt` before
  showing a language do anything; a surface the bindings do not expose shows
  Rust alone under `!!! note "Rust only"`.
- Add the page to `mkdocs.yml` beside `iceberg`, and state in the commit why
  that slot. Update `docs/iceberg.md` (the Doris bridge), `docs/parquet.md`
  (Doris as an outside reader), `docs/io.md` (the export layout),
  `docs/testing.md` (how to run the interop driver), and
  `docs/architecture.md`. Regenerate notebooks with
  `python scripts/build_docs_notebooks.py` (edit blocks, never notebooks).
  Update the README layout table for `rust/src/doris/`.
- `docs/benchmarks.md` regenerated from real runs, release builds only, naming
  the Doris version the numbers came from.
- `python -m mkdocs build --strict` stays green.

---

## 12. Phase 8: required checks (all must pass before handoff)

Per `AGENTS.md:1210`: `cargo fmt --check`; warning-free
`cargo clippy --locked --workspace --all-targets -- -D warnings` **twice**
(default features and `--features "parquet iceberg doris"`); workspace tests
twice the same way; `cargo doc` with `RUSTDOCFLAGS="-D warnings"`; the Rust
1.85 core check (default features and `--no-default-features --lib`);
`cargo bench --benches --no-run`; `python scripts/check_iceberg_interop.py`
(the catalog refactor must not change what PyIceberg sees, and its v3 half must
be green);
`python scripts/check_doris_interop.py`;
maturin develop + pytest + `mypy --strict`; `npm run test:package` +
`npm test`; `python scripts/check_docs_examples.py`;
`python -m mkdocs build --strict`. Clean generated targets, venvs, Docker
containers and volumes, and `node_modules` after validation.

---

## 13. Hard constraints, restated

- **No new dependency** in any of the three manifests. No HTTP client, no
  MySQL driver, no gRPC or Flight stack, no `serde_json` beyond what is already
  pinned, no Doris crate. The Docker image and the Python `requests` used by
  the interop driver are test-only tooling in `requirements-docs.txt`'s
  sibling, never a runtime dependency.
- **One representation, everywhere.** The filter is `Expression`. The schema is
  a struct `Field`. The encoding comes from the media type. Partition columns
  come from the partition marker. Errors are `crate::Error`. Doris does not get
  a private copy of any of them.
- **Never a second parser, a second cast, or a second error enum.** Arrow type
  text goes to `DataType::from_str`; value conversion goes through
  `field/cast`; the compatibility rewrite goes through the one walker.
- **The record surface stays three methods.** A Stream Load body and an export
  read go through `read_arrow_batch_reader` /
  `write_arrow_batch_reader` / `append_arrow_batch_reader`, never a private
  encoder. Nothing that could stream is collected.
- **Never a pre-check.** `AGENTS.md:183` is normative for the whole
  workspace after this task: no `exists` before a read, no `contains` before a
  `get`, no `mkdir` before a write, no "ensure" step anywhere. Act, branch on
  the typed absence or conflict, repair once, retry once. A review finding a
  probe on the way to doing something else treats it as a bug.
- **Name the mechanism before writing the feature.** Every v3 capability is
  built on something the workspace already owns, per the table in 2.1. A
  private re-implementation of masking, casting, defaulting, parsing, or
  evaluation is a review failure even when it works.
- **One shape per hierarchy level.** A collection and a resource read
  identically at `catalogs`, `namespaces`, and `tables`; a level that invents a
  verb is the thing this refactor exists to remove.
- **Total or refused.** A type mapping either round-trips, or widens with the
  widening documented, or errors naming both sides. There is no third outcome
  and no silent coercion.
- **Measure before claiming.** Every optimization in Phase 4 carries a number
  or a note saying it did not pay.
- Method names follow the exact vocabulary (`AGENTS.md:491`); Rust
  `create_table`/`schema_from_create_table`/`check_readable` ↔ Python the same
  ↔ JS `createTable`/`schemaFromCreateTable`/`checkReadable`; argument names and
  order identical across languages.
- Commit in coherent steps (types, mapping, compat target, schema, DDL,
  variant, SQL, TVF, load, export, catalog, options, interop, benches,
  optimizations, python, node, docs) with descriptive messages; push the
  branch; do not open a PR.

**Definition of done, part three**: a user writes a schema with every Doris 4.1
type in it, nested four levels deep, in any of the three languages; Yggdryl
renders the `CREATE TABLE`, writes the rows as Parquet and commits them as a
**v3** Iceberg table with deletion vectors and assigned row ids, and composes
the Stream Load; a real Apache Doris 4.1.3 reads all three
and returns every value unchanged; Doris then writes the same rows back out
through `MERGE INTO` and `INSERT INTO FILE()`, Yggdryl reads them, and the rows
are identical again - and `docs/benchmarks.md` shows what each protocol cost,
beside PyArrow and PyIceberg on the same payload.
