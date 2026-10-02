# window_by extension: sorted windows, stream windows, static values and nested keys

This extension is layered on `.handoff/window-by/WINDOW_BY_SPEC.md` (the base). Every base section not named here holds unchanged. Where this spec changes a base name or pin, it says so in "What the base becomes".

**Basis.** Design 2 is the base; all three judgements ranked it best. Grafted onto it:
- **A flat window static record**, `{<the reader's own static values>, <key cells>, windownum, rownum}`, from D1/D3, in place of D2's nested `key` struct. It has the same shape as a partition tuple. A colliding name is refused at the call.
- **D3's `lies_where` rule** for the direct key arm, with one shared `child_position` fold used by `segment_array`, path typing and `lies_where`.
- **Static values stored as `(Arc<Field>, Scalar)`.** They are set from a proven `FieldRecord`, so `with_`, `set_` and `clear_` are infallible (D3). They are read as an allocation-free `FieldScalar` and through `get_static_value(&FieldPath)` (D2).
- **`SerieReaderWindows::field()` and `static_field()`**, both known before the first pull (D3).
- **An O(W) window table after a gather**, so no n-entry order is retained (D3).
- **In-order reading of windows.** A window the walk skipped rows of is refused by name. A window whose rows were all served answers `None` (D1).
- **The base names are kept:** `window_key`, `apply_serie`, `bind_key`, `window_starts`, `Compare::opens` (D1).
- **Locking rules:** the GIL is released around every pull, a poisoned walk lock is a typed error, and locks are taken inner then outer (D3).
- **D3's narrow-struct idea, simplified.** Computed keys run the same `projected` kernel over a batch of only the columns they read.
- **Pins:** sorted over sorted keys costs what unsorted costs (D1); cost is constant in record width (D3); the direct arm equals the engine arm (D2); a foreign NaN at a batch edge moves no boundary (D3).

**Defects fixed relative to the designs:**
- D2's nested `key` record is now flat.
- `compare_row` could not live on `Compare`; it is a `Serie` method that reads cells. An edge costs 0 only for inline cells, and that is stated.
- The unvalidated `StructSerie::new` gets an exact `lies_where` condition, an invariant and a parity test.
- `rownum` now has one fixed meaning, so a caller's value cannot be misread.
- A descent no longer refuses the whole batch: the rows before it are delivered.
- A computed key no longer rebuilds the whole record.
- Python cannot bind before `take()`. Only parsing precedes it, and a refused key spends the reader, as a refused cast plan does.
- The static names `index`/`offset` are dropped for `windownum`/`rownum`.
- `FieldRecord::from_checked` does not exist, so reads go through the existing `FieldScalar::from_checked`.

## What the base becomes

| Base item | Extension |
| --- | --- |
| `window_by(by)` on the five owners (SerieReader is new) | `window_by(by, sorted: bool)`; no `sorted_window_by` anywhere |
| `SerieWindows<'a>`, an `Iterator` | An owner over `Cow<'a, Serie>` that lends `(Scalar, SerieSlice<'_>)` through `iter()` and `&SerieWindows: IntoIterator`; `SerieWindowsIter<'s>` is the iterator. `impl Iterator/ExactSizeIterator/FusedIterator for SerieWindows` is deleted |
| `SerieWindows::new(serie, offset, keys)` | `SerieWindows::new(holder, offset, keys, sorted) -> Result<Self>` |
| `Serie::window_starts(&self) -> BooleanBuffer` | `Serie::window_starts(&self, regroup: bool) -> Result<WindowCut>`: the same single pass, now also answering the first descent and, when asked and needed, the regrouping |
| `Serie::same_key` | Deleted; `Serie::compare_to_row` compares a held key to a row in place |
| `BoundSelector::apply_serie` (one arm, `Proof::Unproven`) | Same signature, three arms (direct, narrow, whole), the key plan hoisted into `bind_key`; plus `apply_serie_window` |
| `SerieSlice::window_by` keys over `self.into_serie()` | Keys over `apply_serie_window(holder, offset, len)`, never `into_serie` |
| `Compare` rungs `Buffers`, `Values` | Adds a `Record` rung |
| Base phases 5–8 (bindings, docs, chain) | Folded into X5–X8, run once against the extension's signatures |

## Semantics

### One verb, one flag

`window_by(by, sorted)` is the one verb on `Serie`, `SerieSlice`, `SerieSliceMut`, `ChunkedSerie` and `SerieReader`. `by` is read exactly as in the base.

- **Rust:** a trailing `sorted: bool`. Rust has no default arguments, and the crate already takes a trailing bool flag: `IOBase::remove(recursive)`, `IOBase::children_where(filters, include_private)`, `EventIterator::new(elements, sorted)`.
- **Python:** `window_by(by, sorted=False)`.
- **JavaScript:** `windowBy(by, sorted)`, where `sorted?: boolean | null`.

**`sorted = false`** is the base cut, unchanged: maximal runs of equal adjacent keys, and a key that comes back opens a new window.

**`sorted = true`** answers each distinct key exactly once, in key order: ascending, absent keys last. That is `SortOptions::default()`, the default of the plan's `order by` and of DuckDB.
- **Cut and verdict.** Equality is the base's and has no direction, so the windows are cut where `sorted = false` cuts them. `sorted` adds the order verdict, read in the same comparator pass: `Greater` between adjacent rows is a descent.
- **Held owners** (`Serie`, `SerieSlice`, `SerieSliceMut`):
  - With no descent, the answer is exactly the `sorted = false` windows over the same holder, at the same cost (`std::ptr::eq(windows.serie(), &quotes)`).
  - With a descent, the runs (not the rows) are sorted stably by key, adjacent runs of one key are merged, and the rows are gathered once into key order, into a holder the windows value owns. Rows of one key keep their arrival order.
  - The gather alone requires `u32`-indexable rows. Past `u32::MAX` it is refused by `require_indexable`, naming the serie. This check runs after the key pass, and only when a gather is needed.
- **`ChunkedSerie`:**
  - With no descent inside a chunk or at an edge, the answer is the `sorted = false` windows.
  - With a descent, the runs are regrouped stably by key. Each window is a `ChunkedSerie` of its runs' zero-copy pieces, in arrival order. No row is copied, and the chunks are never joined.
- **`SerieReader`** verifies the order and never reorders. A stream is sorted only by holding it; the plan's `order by` is the crate's one collecting door (`expression/plan.rs:1684`).
  - A window start whose key orders before the open window's key is refused. The refusal names the batch, the row in the batch, the stream row and both keys, and gives the two ways out:
    - `sorted = false`, which is already exact for keys grouped in any order;
    - holding the stream with `ChunkedSerie::from_serie_reader`, then a held `window_by(by, true)`.
  - Every window before the descent is delivered whole: the open window ends at the descent row, because its key changed. The walk's next step then yields the refusal once, and every reader fuses.
- **The word.** `window_by(.., sorted)` means "answer each key once, in key order". `EventIterator::new(.., sorted)` means "the input arrives sorted". Both docs say so once.
- **Direction.** `descending` and `nulls_first` are not offered (Open decision 4).

### Any serie, any key

- **Windowed rows** may be any layout, as in the base.
- **Keys** follow the base rules, and nested keys are first-class:
  - a path through records (`order.venue`, `order.leg.mic`);
  - a whole record cell (`order`), a list cell or a map cell, each compared item by item;
  - a list or map step (`legs[0].venue`, `attrs['desk']`), through the evaluator's kernels;
  - a union or variant cell, on the values' order.
- **Which key arm and which comparator rung** is chosen per call or per batch, as described in Algorithm and cost.
- **Runs.** A schema-free run stays refused as a key holder (base refusal 2).

### Stream windows

`SerieReader::window_by(self, by, sorted)` answers `SerieReaderWindows`, a lazy, fused iterator of sub-readers.
- **What a sub-reader is.** Each sub-reader is an ordinary `SerieReader` whose rows are that window's rows, as they stand in the stream. Its `field()` is the parent's root, and its static values state the window.
- **Nothing pulled at construction.** Every refusal of the key is raised by `window_by` before a batch is pulled.
- **Read in order.** A window is read before the next one is taken.
  - Taking the next window skips the open window's unread rows: it pulls and drops them, holding nothing.
  - A sub-reader read after its walk skipped rows of it answers once, `window {n} was passed by its walk with rows unread; read each window before taking the next`, then fuses. So collecting the walk first (`list(walk)`, `.collect()`) is loud, never a silent loss.
  - A sub-reader whose rows were all served answers `None`, even if the walk moved on before it said so.
  - A dropped sub-reader costs nothing beyond pulling its rows.
  - A sub-reader may outlive the walk value: it shares the walk and reads its window to the end.

### Static values

A `SerieReader` may state one record of values that are constant over every row it yields: a non-null record `Field` plus its canonical row. The row is proven, because a `FieldRecord` built it. Static values sit beside the root and are never a column, so `field()` is unchanged.

- **Reading them:**
  - `static_values()` lends the record as a `FieldScalar`, the field and the row, with no allocation;
  - `get_static_value(&FieldPath)` lends one value through record steps by exact child name, as `Field::index_of` resolves names. Any other segment answers `None`.
- **Stating them:** `set_static_values(FieldRecord)`, `with_static_values(FieldRecord)` and `clear_static_values()`. These are infallible, because `FieldRecord::new` already refused anything but a row under a non-null record.

| Door | Static values |
| --- | --- |
| `from_arrow_reader`, `from_serie`, `from_chunked` | none |
| `cast` | kept: they describe where the rows come from, not a column the root declares |
| `window_by` | derived for every window, as below |
| `into_arrow_reader`, the bindings' C stream export, `IOMedia::write_arrow` (which calls it) | dropped: transport carries a schema only |
| `ChunkedSerie::from_serie_reader`, `Serie::from_arrow_reader` | dropped: a held table keeps rows only |

**Two reserved names** whose meaning the crate fixes:
- `windownum: uint64` is a window's place among the windows of the reader it was cut from, from 0.
- `rownum: uint64` is the number the reader's first row has in the stream it was cut from. `rownum` is already the crate's word for a row's number (`TextOptions::start_rownum`; the text `seqnum` aliases).

A caller may state either. `window_by` refuses a reader that states either one at any other datatype.

**A window's static values** are a required record, named as the windowed reader's root. Its children, in order:
1. the windowed reader's own static values, except `windownum` and `rownum`;
2. the key cells, named as the key projections are (`Projection::name`), in selector order;
3. `windownum`;
4. `rownum`: the windowed reader's own `rownum` (else 0) plus the rows that reader yielded before the window's first row. It therefore stays absolute through windows of windows.

A key cell whose name collides, under the binder's ASCII fold, with a kept static name or with `windownum` or `rownum` is refused at `window_by`, naming both and asking for an alias. A window of a window keeps the outer key cells and states its own place. A static record built without a declared field is named as the reader's root by the same rule.

**Held windows attach no static values.** Their three facts are already in the item: the key is its first half, the place is its position in `iter()`, and the row is `window.offset()`, absolute in `serie()`. A test pins the correspondence: `SerieReader::from_serie(rows)?.window_by(by, s)` states exactly `(key cells, windownum, rownum)` where `rows.window_by(by, s)` gives `(key, position, offset)`. It runs for `s = false`, and for `s = true` over sorted keys.

### Refusals

**Held owners:** the base list, in the base order, for both flags. Then, on the gather path only, `require_indexable`.

**`SerieReader::window_by`**, before any pull, in this order:
1. the parse error;
2. an empty key or `*` alone;
3. an `unnest`;
4. the binder's refusals;
5. a key cell colliding with a kept static name or a reserved name;
6. a reader stating `windownum` or `rownum` at another datatype.

All are `Error::InvalidRecord` naming the reader's root. There is no run case, because a reader never holds one.

**At run time:**
- **Stream descent.** `Error::InvalidRecord { path: "$[<stream row>]", reason: "window by expects keys in order, ascending with absent keys last: batch <b> row <r> keys <key> after <previous>; window it unsorted, or hold it (ChunkedSerie::from_serie_reader) and window it sorted" }`. Empty batches count in `<b>`. Wrapped in `crate::arrow::Error::Core`.
- **Passed window**, as above.
- **A parent read, plan or key error.** It is the item of whoever pulled it, the walk or the sub-reader, exactly once. Then the walk and every sub-reader fuse, and the parent is dropped, which releases a C stream as `SerieReader` does. An open window is never presented as complete after an error: if the walk was skipping it, it is marked passed.
- **Poisoned walk lock.** `crate::arrow::Error::internal("SerieReaderWindows: a pull panicked while holding its walk")`, then fused.

## The MediaSerie and partition-values seam (described, not implemented)

Nothing below is built in this change, and no name below is added now. It is the design that `static_values` and the sub-reader windows must not block, and the docs state it as a seam.

- **One sub-reader per part.**
  - A later `IOMedia` door answers one `SerieReader` per Hive leaf or per Iceberg file task. Each part's static values are its partition tuple, typed once at the part's boundary.
  - Hive `column=value` text (`Url::hive_partitions_under`) is resolved once against the declared field's `Field::only_partition_fields`, by one plan per stream.
  - Iceberg's `ScanPart.partition: Vec<(Field, Scalar)>` (`iceberg/scan.rs:683`) is already that typed record.
  - A leaf boundary is a window boundary, so no row is compared to find it.
- **Restoring partition columns.** A partition column is restored from a part's static values as one `Serie::from_scalars(cell_field, [cell])` per part, then a `Proof::Proven` `repeat(0, rows)` per batch. That replaces `media::partition::Constant` (a cast plan per leaf and a `take` per batch) and `iceberg::scan::restore_partitions` (a one-row serie rebuilt every batch). Both are deleted in that change, with no back-compat.
- **The same shape everywhere.** Static values are flat and named as the columns they are constants of. A folder stream windowed by its partition columns, and a per-leaf part, therefore state the same record: a writer of parts cannot tell a leaf from a window.
- **Partitioned writes.** When rows arrive grouped, `FolderWriter::split_by_partition` (a `HashMap<Vec<String>, Vec<u32>>` with one rendered `String` per row and partition column, `media/partition.rs:631`) becomes `window_by(<PARTITION:by key>, false)` over the incoming `SerieReader`. Each sub-reader is routed to `leaf_name(partition_text(<its static cells>))`. Ungrouped input keeps the hash grouping, or takes the held `sorted = true` path.
- **Pruning.** `Bounds::with_column(name, v, v, 0)` per static cell feeds `Bound::statistics_prune`, replacing the text pairs of `Bounds::from_partitions`.
- **`MediaSerie`.** A later root file, `media_serie.rs`, holding a `Holder`, the `RecordOptions` and a cached `Arc<Field>`.
  - Reads:
    - `len` is `row_size`;
    - `scalar(i)` and `window(offset, len)` read through the plan's `offset` and `limit` sections pushed into the media, with `iobase_calls` pinned;
    - its parts are the sub-readers above;
    - `window_by` is `read_arrow(..)?.window_by(..)`.
  - Mutations: set and update are `merge_by` upserts scoped to the parts whose static values match. Delete is a partition-scoped overwrite of only the parts it touches, never a whole rewrite. Iceberg has no row-level deletes (`scan.rs:546`), so a delete is a part rewrite.
- **What this change guarantees for the seam.**
  - Static values are typed records, not text.
  - `cast` keeps them.
  - A writer taking a `SerieReader` can read them before `into_arrow_reader` drops them.
  - A sub-reader is a `SerieReader`, which every write door accepts.
  - `rownum` is absolute, so a part's rows are addressable in the stream.

## Rust API (exact signatures, files)

Public, `rust/src/serie/order.rs`:

```rust
impl Serie {
    pub fn window_by(&self, by: impl IntoSelector, sorted: bool) -> Result<SerieWindows<'_>>;
}
```

Its body is `let key = self.window_key(&by.into_selector()?)?; SerieWindows::new(self, 0, key.apply_serie(self)?, sorted)`. The rustdoc example adds `sorted` over XNYS, XNAS, XNYS, XNAS: two windows, XNAS first.

Public, `rust/src/serie_slice.rs`:

```rust
impl<'a> SerieSlice<'a> {
    pub fn window_by(&self, by: impl IntoSelector, sorted: bool) -> Result<SerieWindows<'a>>;
}
impl<'a> SerieSliceMut<'a> {
    pub fn window_by(&self, by: impl IntoSelector, sorted: bool) -> Result<SerieWindows<'_>>;
    // window_mut unchanged
}

#[must_use]
#[derive(Clone, Debug)]
pub struct SerieWindows<'a> {
    holder: Cow<'a, Serie>,
    /// The holder row the keys' first row stands for: the window's offset when
    /// borrowed, 0 when gathered.
    offset: usize,
    /// One key row per row windowed, in arrival order.
    keys: Serie,
    cuts: Cuts,
}
#[derive(Clone, Debug)]
enum Cuts {
    /// Windows in row order: one bit per key row, set where a window opens,
    /// and the number of set bits.
    Starts(BooleanBuffer, usize),
    /// Windows in key order over the gathered holder: per window, the key row
    /// it is keyed by and where it ends in the holder.
    Gathered(Box<[(u32, u32)]>),
}
impl<'a> SerieWindows<'a> {
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    /// The holder every window views: the windowed serie, unless `sorted`
    /// gathered its rows, then that gathered copy, which this value owns.
    pub fn serie(&self) -> &Serie;
    pub fn iter(&self) -> SerieWindowsIter<'_>;
}
impl<'s, 'a: 's> IntoIterator for &'s SerieWindows<'a> {
    type Item = (Scalar, SerieSlice<'s>);
    type IntoIter = SerieWindowsIter<'s>;
}

#[must_use]
#[derive(Clone, Debug)]
pub struct SerieWindowsIter<'s> {
    windows: &'s SerieWindows<'s>,
    front: usize,
    at: usize,
    remaining: usize,
}
impl<'s> Iterator for SerieWindowsIter<'s> { type Item = (Scalar, SerieSlice<'s>); }
impl ExactSizeIterator for SerieWindowsIter<'_> {}
impl FusedIterator for SerieWindowsIter<'_> {}
```

- `SerieWindows<'a>` is covariant in `'a` (`Cow<'a, Serie>`), so `&'s SerieWindows<'a>` lends `SerieSlice<'s>`.
- The `Starts` walk is the base's `next` body, moved.
- A `Gathered` step yields `(proven_row(&keys, key_row), SerieSlice { serie: holder, offset: at, len: end - at })`.
- The type name follows the crate's `MetadataIter` and `HeadersIter`.

Public, `rust/src/chunked_serie.rs`:

```rust
impl ChunkedSerie {
    pub fn window_by(&self, by: impl IntoSelector, sorted: bool) -> crate::Result<Vec<(Scalar, ChunkedSerie)>>;
}
```

It has one return type across both flags:
- `sorted = false`, or keys already in order: each window is `self.slice(from, len)`;
- otherwise: `ChunkedSerie::from_landed(Arc::clone(&self.field), <its runs' chunk pieces, in arrival order>)`.

Public, `rust/src/serie/arrow.rs` (`SerieReader` gains `statics: Option<Statics>`):

```rust
impl SerieReader {
    pub fn window_by(self, by: impl IntoSelector, sorted: bool) -> Result<SerieReaderWindows>; // crate::arrow::Result, beside `cast`
    pub fn static_values(&self) -> Option<FieldScalar<'_>>;
    pub fn get_static_value(&self, path: &FieldPath) -> Option<FieldScalar<'_>>;
    pub fn set_static_values(&mut self, values: FieldRecord<'_>);
    pub fn with_static_values(self, values: FieldRecord<'_>) -> Self;
    pub fn clear_static_values(&mut self);
}

/// The windows of a stream, one lazy sub-reader each; see Streaming.
pub struct SerieReaderWindows {
    walk: Arc<Mutex<Walk>>,
    root: Arc<Field>,
    statics: Arc<Field>,
}
impl SerieReaderWindows {
    /// The record root every window yields.
    pub fn field(&self) -> &Field;
    /// The record every window's static values are typed by.
    pub fn static_field(&self) -> &Field;
}
impl Iterator for SerieReaderWindows { type Item = Result<SerieReader>; } // crate::arrow::Result
impl FusedIterator for SerieReaderWindows {}
impl Debug for SerieReaderWindows {}
```

- `SerieReaderWindows` is `Send + Sync`, because `Walk: Send`. A sub-reader is `Send`, as `SerieReader` is.
- `rust/src/serie.rs:204` becomes `pub use arrow::{SerieReader, SerieReaderWindows};`. `lib.rs:299` already has `pub use serie::*`.
- `rust/src/lib.rs:230` becomes `pub use serie_slice::{SerieSlice, SerieSliceMut, SerieSliceRows, SerieWindows, SerieWindowsIter};`.

Crate-private, `rust/src/serie/order.rs`:

```rust
pub(crate) struct WindowCut {
    /// One bit per row, set where a window opens.
    pub(crate) starts: BooleanBuffer,
    /// The first row whose key orders before its predecessor's under SortOptions::default().
    pub(crate) descent: Option<usize>,
    /// Asked for and needed: the windows in key order.
    pub(crate) regrouped: Option<Regrouped>,
}
pub(crate) struct Regrouped {
    /// Every row, in key order, rows of one key in arrival order.
    pub(crate) order: Vec<u32>,
    /// Per window: the row its key is read at, and where it ends in `order`.
    pub(crate) windows: Box<[(u32, u32)]>,
}
impl Serie {
    pub(crate) fn window_starts(&self, regroup: bool) -> Result<WindowCut>;
    pub(crate) fn compare_to_row(&self, value: &Scalar, row: usize, options: SortOptions) -> Ordering;
}
pub(crate) fn compare_values(left: &Scalar, right: &Scalar, options: SortOptions) -> Ordering; // was private
```

- **`window_starts`** builds one `Compare::new(self, SortOptions::default())` and one `BooleanBuffer::collect_bool`, whose closure records the first `Greater` in a `Cell<Option<usize>>`.
  - With `regroup` and a descent, it reuses that same `Compare`: `require_indexable`, the runs from the bits, a stable `sort_by` of the runs by `compare.cmp(start_a, start_b)`, adjacent equal runs merged, then `order` and `windows`.
- **`Compare::step(&self, index) -> Ordering`** answers `Less` at 0, else `cmp(index - 1, index)`. `opens` becomes `step(index) != Equal`, still the one boundary predicate, and `partition_by` is untouched.
- **A new rung**, `Compare::Record { nulls: Option<NullBuffer>, children: Box<[Compare<'a>]>, options: SortOptions }`.
  - `Compare::new` tries `Buffers` first, then `Record` for a record column with children, then `Values`.
  - `cmp` places an absent record row by `absent_against_present(options)`, then answers the first child `cmp` that is not `Equal`. Each child is `Compare::new(child, options)`, and the result is not re-directed.
  - This is `compare_values`' record arm over equal-arity runs, and it equals Arrow's struct comparator, whose `child_opts` and outer reversal net to the parent's options.
- **`compare_to_row`** compares `value` against row `row` of a key record, cell by cell: `compare_values(&value.row_at(j), &child_j.scalar(row), options)`. An absent row reads as `Null`. A non-record column falls back to `compare_values(value, &proven_row(self, row), options)`. It builds no run, and costs 0 allocations for inline cells.
- **Deleted:** `same_key`.

Crate-private, `rust/src/expression/`:
- **`selector.rs`.** `BoundSelector` gains `key: Option<Arc<KeyPlan>>`, set by `bind_key` only. `bind` and merge's use are unaffected, beyond the extra field merge carries.

  ```rust
  pub(crate) struct KeyPlan {
      root: Arc<Field>,                   // output().with_nullable(true), built once
      schema: SchemaRef,                  // the output's Arrow schema, built once
      resolved: Resolved,                 // land_planned's boxed fields, once
      lies: Box<[Option<Box<[usize]>>]>,  // per projection, Bound::lies_where
      reads: Box<[usize]>,                // the bound root's children any projection reads (Node::column_indices, merged)
      narrow: SchemaRef,                  // those children's Arrow fields, in that order
      proof: Proof,                       // Proof::of_children: Proven where lies, Unproven where computed
  }
  pub(crate) fn apply_serie(&self, serie: &Serie) -> Result<Serie>; // = apply_serie_window(serie, 0, serie.len())
  pub(crate) fn apply_serie_window(&self, serie: &Serie, offset: usize, length: usize) -> Result<Serie>;
  ```

  The arms are described in Algorithm and cost. The `internals` forwarder gains `apply_serie_window`.
- **`bind.rs`.** `pub(crate) fn Bound::lies_where(&self, published: &Field) -> Option<Box<[usize]>>`. It answers the child-position path only when all of these hold:
  - the node is `Kind::Column(i)`, or `Kind::Path(Column(i), steps)` where every step is a `StepKind::Segment` whose `as_name()` names a child of a record field (never a union member or a map);
  - each position is resolved by `child_position`;
  - `published.dtype() == reached.dtype()`, so no declared cast;
  - `published.is_nullable() || !reached.is_nullable()`.
- **`arrow.rs`.** `pub(crate) fn child_position<'n>(names: impl IntoIterator<Item = &'n str>, name: &str) -> Option<usize>` returns the first `eq_ignore_ascii_case` match. `segment_array`, `path.rs::struct_child_field` and `lies_where` all resolve a record step through it.

Crate-private, `rust/src/serie/arrow.rs`:

```rust
struct Statics { field: Arc<Field>, row: Scalar }
enum Source { Stream(..), Held(..), Window(WindowPart) }
struct WindowPart {
    walk: Arc<Mutex<Walk>>,
    state: Arc<AtomicU8>,                 // Open | Ended | Passed, written under the walk's lock
    index: u64,
    then: Option<Box<ArrowCastPlan>>,     // a sub-reader's cast, applied per piece
}
struct Walk {
    source: Option<SerieReader>, key: BoundSelector, sorted: bool,
    statics: Arc<Field>, kept: Box<[Scalar]>, base: u64,
    held: Option<Cut>, batch: u64, before: u64, opened: u64,
    open: Option<Open>, done: bool,
}
struct Cut { record: Serie, keys: Serie, starts: BooleanBuffer, descent: Option<usize>, edge: Ordering, at: usize }
struct Open { index: u64, key: Scalar /* a Run view of its static row's key cells */, state: Arc<AtomicU8> }
```

Each existing `match` gains the `Window` arm:
- `cast` compiles one plan from the root (or from the existing `then`) into `then`. This is lazy, never the `Held` arm's eager collect.
- `into_arrow_reader` maps each piece through `batch_under`, lazily.
- `next` serves pieces (see Streaming).
- `Debug` prints the statics.

`window_by` binds through `by.into_selector()?.bind_key(&self.root, self.root.name(), "window by")`.

`rust/src/serie_slice.rs`, crate-private: `pub(crate) fn SerieWindows::new(holder: &'a Serie, offset: usize, keys: Serie, sorted: bool) -> Result<Self>`. It runs `keys.window_starts(sorted)`. If it regrouped:
1. `require_indexable(holder)`;
2. shift `order` by `offset` in place;
3. `Cow::Owned(holder.taken(&order)?)`, offset 0, `Cuts::Gathered(windows)`.

The `order` is then dropped. Otherwise the result is `Cow::Borrowed(holder)` with `Cuts::Starts`.

**Re-spelled by one sweep script** driven by `cargo check -p yggdryl --all-targets --keep-going --message-format=short`: every `.window_by(x)` becomes `.window_by(x, false)`, plus `.iter()` wherever an iterator was used. This covers order.rs, serie_slice.rs and chunked_serie.rs, their tests and doc examples, and `allocations.rs:3129-3201, 3376-3418`.

## Algorithm and cost

The allocation counts below are counting-allocator numbers. "Pinned" means asserted in `rust/tests/allocations.rs` today (in-flight, uncommitted). "≈" means derived from the source, measured once on implementation, and pinned with a breakdown sentence.

### Key arms, chosen per call or per batch

1. **Direct.** Taken when every projection `lies`, and no record strictly between the root and a lying leaf holds an absent row in this holder. That check reads the holder's `null_count`, which is O(1) per path step. It is conservative: a null outside a window sends the window to arm 2.
   - Each cell is the landed child, reached by its positions: an `Arc` clone, 0 allocations. A non-record holder is its own child 0.
   - Over a window, each cell is `Serie::slice(offset, len)`: 1 allocation for a flat leaf, and the base's O(window) offsets for a list or map cell. The record's nulls are sliced with 0 allocations.
   - The key is `StructSerie::new(Arc::clone(&plan.root), cells, record_nulls, len)`: one `Vec` and one boxed leaf, so 2 allocations.
   - There is no Arrow round trip, no landing, no proof and no new `Proven` site, because the cells are the landed record's own proven leaves.
   - **Invariant.** This key record names its children through `plan.root` while each cell keeps its landed field. It never leaves the crate: `SerieWindows`, the chunk fold and the walk hold it privately. Its Arrow array is named by the root, and every read of it is positional. The `lies_where` dtype and nullability rule is what keeps `StructSerie::into_arrow_array`'s `expect(ALIGNED)` sound. A parity test pins direct against engine on every layout.
2. **Narrow.** Taken when some projection computes, or a lying path crosses a record with absent rows, and the holder holds no absent record row. Stream batches never do.
   - The batch is `RecordBatch::try_new(plan.narrow, <plan.reads, each child's require_arrow_array(), Arrow-sliced to the window>)`. An Arrow slice of a list is O(1) and needs no rebasing.
   - Then `self.projected(&narrow, Some(&plan.schema))` (the existing kernel), `StructArray::from(batch)` and `land_planned(&plan.resolved, keys, &plan.proof)`.
   - It works because `Context::new` matches columns by name (`expression/arrow.rs:119-138`). Its cost is constant in record width.
3. **Whole.** Taken when the holder holds absent record rows and arm 1 does not apply. This is the base arm: `require_arrow_array` of the record, Arrow-sliced to the window, then `apply_arrow_array` (with `struct_rows` filtering and scattering), then `land_planned(.., &plan.proof)`.

**`plan.proof`** is `Proven` for a lying cell, `Unproven` for a computed one.
- This is **one new reviewed `Proof::Proven` site**: a lying cell is a selection of a landed column. It may gain absence through `segment_array`'s mask fold, but never a value.
- Computed cells are still read under their rule.
- This removes the base's O(rows) re-proof of codes, enums, decimals, ascii text and `Date64` cells (Open decision 3).

### Cut: `window_starts(regroup)`

- **Buffers rung:** one array handle, plus a foreign-NaN scan of float cells only, plus one comparator.
- **Record rung:** each child on its own rung.
  - A Buffers child costs a handle and a comparator.
  - A Values child builds that leaf's rows once: one `Vec` of 48 B per row, and 0 per row for inline cells such as `mic`, `ccy`, `isin` or text up to 23 bytes.
  - No record run is ever built per row. Today one code child sends the whole record to Values (`order.rs:262-273`), which builds one `Arc<[Scalar]>` per row.
- **Values rung:** unchanged, for a list, map or union key cell off the buffers.
- **Bitmap:** 2 allocations of `ceil(len / 8)` bytes. The descent is recorded in the same closure, at 0 cost.

### Held `Serie::window_by`

The quote record (`venue: utf8`, `count: int64`, `ts`) keyed by `venue`:

| | In-flight (pinned) | Extension |
| --- | --- | --- |
| build | `WINDOW_BY_COLUMN_KEY` = 13 bind + 5 record array + 12 project + 10 land + 6 comparator + 2 bitmap = 48 | 13 bind + K key plan + 2 direct + 6 comparator + 2 bitmap ≈ 23 + K, with K the hoisted `KeyPlan` measured once |
| per window | +1 key run | +1 key run |
| `minutes(ts, 15)` | `WINDOW_BY_PERIOD_KEY` = 38 + 5 + 32 + 10 + 6 + 2 | 38 + K′ + narrow batch (≈3) + evaluation + `land_planned` + 6 + 2, equal for a 3-child and a 48-child record |
| through a window | `direct + RECORD_WINDOW_AS_SERIE` (1 + 3 + 1) | `direct + 1`: one slice per key cell or read column |

**`sorted = true`, keys in order:** the same build and the same drain as `sorted = false`, over the caller's serie.

**`sorted = true`, a descent:** build plus `WINDOW_BY_GATHER`:
- the runs `Vec` (1);
- the stable sort's scratch, `s` (0 under about 512 runs, else 1), the rule at `allocations.rs:2226`;
- `order` (1, 4n bytes);
- `windows` (1);
- one `taken`: `order.to_vec()`, the record's array, a take per child, and a `Proven` landing.

That is O(n + W log W) time and one row copy, and constant allocations for a fixed run count and record width. Window keys are read from the original key rows, so no key column is taken.

### `ChunkedSerie::window_by`

- **Bind:** one, with the key plan hoisted (`BIND` = 2 + 13 + 4 in flight, plus K).
- **Per non-empty chunk:** `apply_serie` (direct arm for a bare column: 2), then the comparator (6), then the bitmap (2). In-flight `PER_CHUNK` is 21; the extension is ≈10.
- **Per edge:** `keys.compare_to_row(&last_key, 0, default)`.
  - `Equal` extends the pending window and builds no key.
  - `Greater` under `sorted` records a descent.
  - Anything else pushes a window.
  - In-flight `KEYS_BUILT` is 4 + 1; the extension is 4.
- **Sorted with a descent:** a stable `sort_by` of the W runs by `compare_values(&a.key, &b.key, default)`, which builds nothing. Adjacent equal keys are merged. Each window is `from_landed` (2) plus each run's `self.slice(from, len)` chunks (2 or 3 per run; 0 for a whole chunk). One piece set per run, no join, no copy. A key that changes on every row makes one-row pieces; the docs advise `into_serie()` first in that case.

### Stream walk

Over quote batches of k = 3 children, keyed by `venue`:
- **Construction:** the bind, plus K, plus the static field, plus `kept`, plus the `Arc<Mutex<Walk>>`. Constant, with 0 batches pulled.
- **Per non-empty batch:** the parent's own landing (not counted here), then the direct arm (2), the comparator (6) and the bitmap (2): ≈10. It is equal for 64-row and 4,096-row batches.
- **Per edge:** `compare_to_row` costs 0 for inline cells. A text cell past 23 bytes costs one `Arc<str>` per comparison.
- **Per window:** the static row, one run built with its length known (≈1), plus the state cell (1). The open key is a `Run` view of the static row (base `Run::slice`, 0). The sub-reader itself is inline: its `Arc`s are clones.
- **Per piece:** 0 when the piece is a whole batch (`slice(0, len)` is the serie itself). A batch a window opens or closes in costs one record slice, k + 2 = 5. Its list or map children rebuild offsets, O(piece) bytes.
- **Law, pinned:** `B·10 + W·2 + P·5` beyond the parent's own cost. It is never per row. Eight batches cost eight times one.

### Pins to add

In `rust/tests/allocations.rs`, each at 64 and 4,096 rows, with the selector parsed outside the closure:
1. **`window_by_sorted_over_keys_in_order_costs_what_unsorted_costs`.** Build and drain are equal for both flags, and `ptr::eq(windows.serie(), &quotes)`.
2. **`window_by_sorted_gathers_the_rows_once_in_key_order`.** 6 runs over 3 venues give build `C + WINDOW_BY_GATHER`, equal at both sizes. `WINDOW_BY_GATHER` is pinned with its breakdown.
3. **`window_by_costs_the_same_whatever_the_record_is_wide`.** It compares `quote` against `wide_quote` (`quote` + `legs: serie<struct<px, qty>>` + `order: struct<venue, mic, ts>` + `tag: dictionary<int32, utf8>` + 40 int64 columns), keyed by `venue`, `order.venue` and `minutes(ts, 15)`:
   - the builds are equal across records and sizes;
   - `order.venue` with an absent `order` row costs `+ MASK_FOLD`, constant (the narrow arm).
4. **`a_record_key_with_a_code_child_cuts_with_a_constant_count`.** `{venue: mic, side: side, count: int64}` keyed by `"venue, side"` costs the same at both sizes. In flight it grows with rows.
5. **`a_chunked_sorted_window_by_copies_no_row`.** The cost is equal at both sizes, and the windows `shares` the source buffers.
6. **`a_windowed_stream_costs_per_batch_and_per_window_never_per_row`.** It runs 1 and 8 batches at both batch sizes with fixed windows per batch, checks the law above, and asserts that a whole-batch piece is `free`.
7. **`a_continuing_window_edge_builds_no_key`:** a stream edge and a chunk edge each cost 0.
8. **`static_values_are_lent_free`:** `static_values()` and `get_static_value` cost 0.

In `rust/tests/iobase_calls.rs`: `windowing_a_read_adds_no_call`. `read_arrow(None)?.window_by("venue", true)?` drained over a `Counted` IPC leaf issues exactly the calls a drained `read_arrow` issues.

### Pins that move, and why

These are the in-flight pins of this same branch, re-pinned in X4. Each move is accounted for exactly in its sentence:
- **`WINDOW_BY_COLUMN_KEY`** moves from `13 + 5 + 12 + 10 + 6 + 2` to `13 + K + 2 + 6 + 2`. The record array, projection and landing (27) become the direct arm (2), and the key root, schema, `Resolved`, lies, reads and proof move into the bind (K).
- **`WINDOW_BY_PERIOD_KEY`** loses its whole-record terms and gains the narrow batch and K′.
- **The through-window relation** moves from `direct + RECORD_WINDOW_AS_SERIE` to `direct + 1`.
- **Chunked:** `BIND` gains K, `PER_CHUNK` moves from 21 to ≈10, and `KEYS_BUILT` from 4 + 1 to 4.

**No committed pin moves.** That covers `partition_by` sorted 6 (2285-2291), `partition_by_paths` 154, the ordering pins (2240-2279), the chunked pins (2853-2933) and every slice or window pin, because all of them are over flat columns or do not reach `apply_serie`. A move elsewhere is a defect. If the whole run shows a code-bearing record pin dropping through the `Record` rung, that is re-pinned on its own row with "one leaf's values built, not one run per row".

## Streaming

**Adaptor.** `SerieReader::window_by(self, by, sorted)` binds the key once against `self.field()`, which is already a non-null record root. It builds the static field once, collects `kept` (the reader's own static cells, minus `windownum` and `rownum`), and reads `base` (its `rownum`, else 0). Then it wraps everything in `Arc<Mutex<Walk>>`.
- A raw `BatchReader` enters only through `SerieReader::from_arrow_reader`, because `BatchReader` is transport.
- `IOMedia::read_arrow(..)?.window_by(..)` composes with no new media door. A structured-text document is one held batch, and its windows are pieces of it.

**Held state and its bound.** The walk holds:
- the parent reader, which holds at most one batch, its own bound;
- the one batch the cursor stands in, with its key record (the direct arm's cells are that batch's own children), its bitmap of `ceil(len / 8)` bytes, and its first descent;
- the open window's static row and state cell;
- counters.

**Reason for the bound:** the walk holds exactly what a `SerieReader` holds, plus one bit per row and one static row. No window is ever accumulated, so no new constant and no bound refusal exist. A key held for the whole stream is streamed with one batch held. Pieces the caller keeps pin their parent batches by the caller's choice. `memory_size` reports a slice's own extent (`arrow/size.rs:1-9`), and the docs say so.

**Walk step** (`SerieReaderWindows::next`, under the lock):
1. If `done`, answer `None`.
2. **Skip the open window's unread rows.** From the cursor, go to the next set bit.
   - At the batch end, pull the next non-empty batch and test its row 0 against the open key with `compare_to_row`; while it continues, keep skipping. Nothing skipped is sliced.
   - Mark the state `Passed` if any row was skipped, else `Ended`.
3. **Find the next start.** If the cursor is at the batch end, pull the next non-empty batch:
   - land its key (`apply_serie`) and cut it (`window_starts(false)`);
   - set `edge = compare_to_row(open_key, 0, default)`, which is `Less` for the first batch;
   - empty batches are skipped and counted in `batch`;
   - at end of stream, drop the source, set `done`, and answer `None`.
4. **Under `sorted`,** if this start is `(at == 0 && edge == Greater)` or `Some(at) == cut.descent`, answer the descent refusal once, drop the source and set `done`. Descent rows are starts, and the walk visits every start in order, so the first descent is always met.
5. **Open the window.**
   - `index = opened++`.
   - The static row is `kept ++ <key cells at at> ++ [index, base + before + at]`.
   - `open.key = Scalar::Serie(row.slice(kept.len(), k))`.
   - The state is a new `Open` cell.
6. **Answer** `SerieReader { inner: Some(Source::Window(WindowPart { walk: Arc::clone, state: Arc::clone, index, then: None })), root, schema, statics: Some(Statics { field: Arc::clone(&statics), row }) }`.

**Sub-reader pull** (`Source::Window`, under the lock):
1. A state of `Ended` answers `None`. A state of `Passed` answers the passed refusal once. Either way the sub-reader is fused.
2. At the batch end, pull the next non-empty batch: its key, its cut, its `edge`.
   - At end of stream: set the state to `Ended`, drop the source, set `done`, and answer `None`.
   - On an error: answer it, set `done`, and drop the source.
   - If `edge != Equal`: the window ended. Set `Ended` and answer `None`. The batch is kept for the walk, and a `Greater` becomes the walk's step-4 refusal.
3. Serve `end` = the next set bit after `at`, else `len`. The piece is `record.slice(at, end - at)`; set `at = end`. If `end < len`, set the state to `Ended`. Apply `then` if a cast was taken.

**Failure and fusing.**
- A parent, plan or key error is the item of whoever pulled it, once. A sub-reader that did not pull it is either already ended or passed (the walk was skipping it), so a partial window is never presented as complete.
- The walk, the open sub-reader and the parent all fuse, and the parent is dropped where the error arrives.
- Construction pulls nothing.

**Schema change.** There is one key plan per stream.
- The parent compiled its plan once. A batch whose layout drifts is reconciled upstream (the media's `PlanCache`, the folder's per-leaf part reader) or refused by the parent's plan, naming it. That refusal is the walk's item, and the walk fuses.
- Every landed record has the root's exact layout, so the direct arm never meets a second layout.
- The narrow and whole arms keep `ColumnCast`'s own cache for a cell's storage drift (dictionary, view).
- The walk adds no `PlanCache`.

**Composition.** A sub-reader is an ordinary `SerieReader`.
- `cast` is lazy per piece, and `into_arrow_reader` yields `batch_under` per piece.
- `IOMedia::write_arrow(sub, mode, options)` streams one window with one batch held.
- `sub.window_by(..)` nests a walk. Its pulls lock inner then outer, a fixed order that cannot cycle.
- **GIL rule:** bindings take the walk lock only with the GIL released (every pull inside `py.detach`), so a Python-backed C stream pulled under the lock can reacquire the GIL.

## Nested

**O(1), zero copy, or constant in rows:**
- A held window: a `SerieSlice`, 0 allocations at any depth.
- A lying key cell (bare column, record path, a column `*` keeps): an `Arc` clone with no proof, unless a record on its path holds an absent row.
- A record key over ordered leaves: one Arrow struct comparator. Over mixed leaves, the `Record` rung: only the value-ordered children build their leaf rows (one `Vec`, 0 per row inline), and only float cells are NaN-scanned.
- An edge: 0 for inline cells.
- A whole-batch stream piece: 0.
- A key through a window: one slice per key cell, never the record's `into_serie`.
- The cost is constant in record width (arms 1 and 2 never touch a column the key does not read).

**Not O(1), and why** (each stated in the docs):
- **A partial stream piece of a record:** k + 2 allocations. Its list and map children rebuild offsets, O(piece) bytes. This is the rebased-cut invariant (`sequence.rs:12-15`), unchanged.
- **A held gather:** one take of every column, O(n × width) bytes. It is the only row copy in the design.
- **A Values-rung cell:** O(rows) transient memory per call or batch (48 B per row per cell). A list, map or union key cell off the buffers builds one run per row, which is one allocation per row.
- **Epoch and computed terms:** O(rows) time through the base's row-tier fallback (base Open decision 2).
- **Arm 2 with a nullable record on a path:** `segment_array`'s mask union, O(rows / 64).
- **Arm 3:** `struct_rows`' filter and scatter, O(rows × read columns), only for a held record with absent rows.
- **A dictionary key cell:** `DictionaryArray::try_new` scans its keys once per comparator (O(rows)). A dictionary column the key does not read costs nothing in arms 1 and 2. Arm 3 still rebuilds it.
- **A run-end key cell:** O(log runs) per compare.
- **A foreign-NaN float cell:** one O(rows) scan per call or batch, as in the base.

**Pins:**
- the wide-record row and the code-child row in Algorithm and cost;
- `the_direct_key_arm_equals_the_engine_arm_on_every_nested_layout`;
- `the_record_rung_agrees_with_its_run_under_every_ordering`;
- `a_foreign_nan_at_a_batch_edge_moves_no_window_boundary`.

## Bindings

### Python

Each method only parses, redirects and wraps. Every pull runs in `py.detach`.

**Held owners:**
- `Serie.window_by(by, sorted=False)` and `SerieSlice.window_by(by, sorted=False)` return `list[tuple[Scalar, SerieSlice]]`.
  - The signature is `#[pyo3(signature = (by, sorted = false))]`, positional or keyword, in Rust's order. A non-bool `sorted` (including `None`) is a `TypeError`, matching `EventIterator(items, sorted=True)`.
  - The detached closure returns `(Option<Serie>, Vec<(Scalar, usize, usize)>)`. The first half is `Some(windows.serie().clone())` (an `Arc` bump) only when `!ptr::eq(windows.serie(), serie)`.
  - Windows wrap the caller's own object (`w.serie is quotes`), or one new `Serie` shared by every window.
- `ChunkedSerie.window_by(by, sorted=False)` returns `list[tuple[Scalar, ChunkedSerie]]` through the chunked group describer.

**Streams:**
- `SerieReader.window_by(by, sorted=False)` returns `SerieReaderWindows`. The selector is parsed under the GIL before `take()`. A bind refusal spends the reader, as a refused cast plan does, and the docs say so.
- `SerieReaderWindows` is a new pyclass holding the core value; no extra lock is needed, because it is `Sync`. It has `__iter__`, `__next__ -> SerieReader` (each wrapped by `PySerieReader::from_inner`), `field`, `static_field`, `__repr__`, and `__hash__ = None`.
- A sub-reader is a `SerieReader`. It is itself a Python iterator of `Serie`, it has `cast`, `into_arrow_reader` and `__arrow_c_stream__` (static values dropped), and it can be passed to `IOBase.write_arrow`.

**`SerieReader` static values:**
- `PySerieReader` caches `statics` beside `field`, so they stay readable after `into_arrow_reader`.
- `static_values -> dict[str, Scalar] | None`: names zipped with cells, in field order.
- `static_field -> Field | None`.
- `get_static_value(path: str | FieldPath) -> Scalar | None`.
- `set_static_values(values, field=None) -> None`:
  - `values` is a mapping, a `Scalar` or a sequence. With `field`, it is `FieldRecord::new(field, Scalar.from_(values))`.
  - Without `field`, the field is the value's own `Scalar::dtype` as a required record named as the root. A mapping's names come back sorted (`Scalar::Struct`), so a field fixes the order.
  - `values=None` clears.

Refusals are a `ValueError` carrying the core message. `_native.pyi` gains `class SerieReaderWindows(Iterator[SerieReader])` and the signatures above, and `typing_bindings.py` covers them. Registration goes in `python/src/lib.rs` beside `PySerieReader`.

### Node

Natives are `skip_typescript` and are deleted from the prototypes.

**Held owners:**
- `_windowByNative(env, reference, by: SelectorInput, sorted: Option<bool>)` on `JsSerie` and `JsSerieSlice`, and `_windowByNative(by, sorted)` on `JsChunkedSerie`.
- `undefined` is skipped and `null` clears to the default `false`, the `SortOptions` field precedent (`binding.d.ts:4396-4402`).
- Windows sit over `reference.clone(env)?`, or over one new `JsSerie` of the gathered copy: `strictEqual(w.serie, quotes)` holds when nothing was gathered.
- `binding.js` adds `windowBy(by, sorted)` on `Serie`, `SerieSlice` and `ChunkedSerie`.

**Streams:**
- `SerieReader.prototype.windowBy(by, sorted)` consumes the reader ("a stream is read once"). `selector_from_input` runs before the take.
- It returns `SerieReaderWindows`, a native class with `_nextNative(): JsSerieReader | null` and the getters `field` and `staticField`. `binding.js` makes it `Iterable<SerieReader>`: `[Symbol.iterator]` is a generator looping `_nextNative` until `null`, as `SerieReader`'s own is (binding.js:2393).
- Each sub-reader is a `SerieReader` instance, itself `Iterable<Serie>`. Pulls are synchronous and no lock is held across an await.

**`SerieReader` static values:**
- `staticValues: { [name: string]: Scalar } | null`, in field order;
- `staticField: Field | null`;
- `getStaticValue(path: string | FieldPath): Scalar | null`;
- `setStaticValues(values: unknown, field?: Field | string | null): void`. `undefined` is skipped and keeps the values; `null` clears.

`binding.d.ts`:
- `windowBy(by: Selector | Term | string | readonly (Term | string)[], sorted?: boolean | null)`;
- `Array<[Scalar, SerieSlice]>` on `Serie` and `SerieSlice`, `Array<[Scalar, ChunkedSerie]>` on `ChunkedSerie`, and `SerieReaderWindows` on `SerieReader`.

`node/index.js` and `node/index.d.ts` are regenerated by `npm run --prefix node build:debug`.

### Parity

These are identical in all three languages:
- argument order `(by, sorted)`, default `false`;
- refusal texts;
- the key shape (a one-cell run for one term);
- static names `windownum`, `rownum` and the key cells;
- the in-order rule and the passed-window refusal.

Divergences:
- Rust only: setting is `set_`/`with_`/`clear_` over a `FieldRecord`. The bindings have one `set_static_values` / `setStaticValues` whose `None` / `null` clears.
- Rust-only types: `SerieWindowsIter` and `SerieSliceMut`.
- Pre-existing: a Rust list of `&str` is column names.

`.api-bindings.txt`:
- Python lines 70-81: `window_by(by, sorted=False)` on `Serie`, `SerieSlice` and `ChunkedSerie`. `SerieReader` gains `get_static_value`, `set_static_values`, `static_field`, `static_values`, `window_by`. A new `SerieReaderWindows` entry.
- Node lines 537-568: the camelCase twins.

## Tests

Refusals come first. Each new test file opens with its `//!` source line.

- **`rust/tests/serie/order.rs`** (mirrors `serie/order.rs`):
  - `window_by_sorted_refuses_what_unsorted_refuses_before_any_row`: the base matrix with `sorted = true`, at 0 and 4 rows.
  - `window_by_sorted_over_keys_in_order_answers_the_unsorted_windows_over_the_serie`: the same keys, offsets and lengths; `ptr::eq`; NaN and ±0.0 keys.
  - `window_by_sorted_gathers_the_rows_once_in_stable_key_order`: XNYS, XNAS, XNYS, absent, XNAS gives XNAS (rows 1 and 4, in that order), XNYS, absent last; it equals `into_taken(stable order)` cut by `sorted = false`.
  - `window_by_cuts_every_nested_key_as_its_values_do`: over `nested_columns()` (`order.rs:502`) plus a union, a map, a run-end column and `{venue: mic, px: float64 with a foreign NaN and ±0.0, qty}`, under both flags, against a reference cut by `compare_values` over `rows()`.
  - `the_record_rung_agrees_with_its_run_under_every_ordering`: extends `agrees_with_its_run` and `a_nested_absence_goes_where_the_options_put_a_top_level_one_on_every_rung` (`order.rs:883`) under the four `ORDERINGS`. The test at `order.rs:973` is renamed `a_record_with_a_value_ordered_leaf_compares_child_by_child_and_agrees_with_its_run`.
- **`rust/tests/root/serie_slice.rs`:**
  - `serie_windows_owns_a_gathered_holder_and_borrows_otherwise`;
  - `serie_windows_lends_its_windows_again_and_again`: `iter()` twice, `&windows` in a `for`, `len`, `is_empty`, an exact and fused iterator;
  - `a_window_windows_sorted_over_its_own_rows`: borrowed offsets are absolute, a gather takes the window's rows only at offset 0, and `SerieSliceMut::window_by` answers the same.
- **`rust/tests/root/chunked_serie.rs`:**
  - `window_by_sorted_regroups_runs_across_chunks_with_no_row_copied`: `[B, A] [A, B]` gives A (one edge-merged run) and B (two pieces); `shares`; it equals `into_serie()?.window_by(by, true)`;
  - `window_by_compares_chunk_edges_in_place`: a NaN and ±0.0 at an edge.
- **`rust/tests/expression/selector.rs`:**
  - `a_key_plan_is_bound_once_and_names_its_lying_cells`: through `internals`. Column, path, `*` expansion and alias lie; a declared cast, a list step, a union member and a `not null` over a nullable column do not.
  - `the_direct_key_arm_equals_the_engine_arm_on_every_nested_layout`: its own fixtures (record, nullable struct `order` with absent rows, a list child, a dictionary child, a code child), with `apply_serie` compared against `Selector::apply_arrow_array` then a landing, over whole and windowed ranges. It includes the fallback with an absent `order` and a foreign `StructArray` whose children differ under absent parents.
- **`rust/tests/serie/arrow.rs`** (mirrors `serie/arrow.rs`):
  - `window_by_on_a_reader_refuses_before_any_pull`: using the pull counter at `:1127`. Covers parse, empty, `*` alone, `unnest`, an unknown column, `minutes(ts, 0)`, a key named `rownum`, a key colliding with a stated static value, and a `rownum` static of `utf8`.
  - `a_reader_windows_lazily_holding_one_batch`: `quote_stream` batches `[A A] [A B] [] [B C]`. The pull count at each piece is pinned, a whole interior batch is the landed batch itself, and the empty batch is skipped.
  - `a_window_states_its_key_windownum_and_rownum`: the flat static record, the held correspondence, and a window of a window (`{day, venue, windownum, rownum}` with `rownum` absolute).
  - `a_sorted_reader_refuses_a_key_going_backwards_naming_batch_and_row`: one descent inside a batch and one at an edge. Rows and windows before it are delivered, the message is checked, the walk fuses, and a drop flag proves the parent was dropped.
  - `a_window_passed_by_the_walk_refuses_to_be_read`: skipped versus fully served (`None`) versus dropped; a sub-reader outliving its walk.
  - `a_reader_error_mid_window_is_the_pullers_item_and_fuses`.
  - `a_foreign_nan_at_a_batch_edge_moves_no_window_boundary`.
  - `static_values_are_carried_by_cast_and_dropped_at_the_transport_face`: also `clear_`, `with_`, `get_static_value` exact names, and a folded name answering `None`.
  - `a_window_sub_reader_casts_and_crosses_as_batches_lazily`.
- **`rust/tests/allocations.rs`, `rust/tests/iobase_calls.rs`:** the rows in Algorithm and cost.
- **Untouched, and must stay green:** `rust/tests/media/merge.rs`, `iceberg/mod_.rs:8264`, `node/tests/iceberg.test.js:1320`.
- **Python:**
  - `python/tests/test_serie.py` (`SerieReader` lives in `src/serie.rs`): `test_window_by_sorted_*` and `test_reader_window_by_*` mirroring the Rust names, including `w.serie is quotes` only over sorted keys, `static_values` as a dict, a passed window raising `ValueError`, `for w in walk: pass` raising nothing, and `sorted=None` raising `TypeError`;
  - `test_serie_slice.py`, `test_chunked_serie.py`;
  - `typing_bindings.py`, plus `mypy --strict`.
- **Node:**
  - `node/tests/serie.test.js`, `serie_slice.test.js` and `chunked_serie.test.js`: the same cases, plus `windowBy(by)` deep-equal to `windowBy(by, false)` and `windowBy(by, null)`;
  - `*.types.ts` with `@ts-expect-error windowBy('venue', 'yes')`.

## Benchmarks

- `rust/benchmarks/types/datatype/serie.rs`: `window_by/sorted_in_order`, `window_by/sorted_gather` (W runs against n rows), `window_by/record_code_key`, `window_by/wide_record` (3 against 48 children; `venue`, `order.venue`, `minutes(ts, 15)`), `chunked/window_by_sorted`.
- `rust/benchmarks/arrow.rs`: `serie_reader/window_by/{column_key, path_key, code_key, period_key, sorted}`, measuring time to the first window and the full drain, against `serie_reader/drain` as the baseline.
- `python/benchmarks/types/serie.py`: `Serie.window_by sorted`, `SerieReader.window_by`.
- `node/benchmarks/types.js`: `serie/windowBy sorted`, `serie_reader/windowBy`.
- Direction only: `cargo bench -p yggdryl --bench types -- window_by --quick` and `--bench arrow -- serie_reader/window_by --quick`. No Performance table changes.

## Docs and skills

- **`docs/types/serie.md`, "Windows by key":**
  - `window_by(by, sorted)`: each key once, in key order; zero copy when already in order, else one gather;
  - `SerieWindows` as an owner, with `iter()` and `&windows`, and `serie()`;
  - nested keys (path, struct, list or map cell, union);
  - a cost table: per call, per window, the comparison alone per row, and the Not-O(1) list;
  - edges: the `sorted` word, a direction other than ascending uses `sorted = false`, and the u32 bound on the gather.
- **`docs/types/serie-slice.md`:** the Doors row `window_by(by, sorted)`. The cost row changes from `+ into_serie` to `+1 per key cell`.
- **`docs/types/chunked-serie.md`:** sorted runs regrouped as pieces, no join; edges compared in place.
- **`docs/arrow/readers.md`:**
  - new "Windows of a stream": lazy sub-readers; one batch held whatever the window length; read a window before taking the next; passed versus served versus dropped; `sorted` verifies and refuses; windows of windows; a three-language example of `read_arrow`, then `window_by('venue', true)`, then each window's `static_values`, then `write_arrow` to its own file;
  - new "Static values": the contract table, the reserved names, and the seam paragraph;
  - Contract and Edges rows.
- **`docs/media/index.md`**, Read and write overview: one sentence and a link saying that a read windows as it streams.
- **`skills/yggdryl-arrow/SKILL.md`:**
  - door rows "windows of equal consecutive keys" (`window_by(by, false)`), "each key once in key order" (`window_by(by, true)`), "a stream cut into readers" and "a reader's static values";
  - pitfalls: read a window before the next; a sorted stream must arrive sorted; static values vanish at the transport face; bind `SerieWindows` before iterating; a Rust list of `&str` is column names;
  - recipes in `references/rust.md`, `python.md` and `javascript.md`.
- **`AGENTS.md`:**
  - Layout rows:
    - `serie.rs + serie/`: `order.rs` gains the `Record` rung and `window_starts`' descent and regrouping; `arrow.rs` gains `SerieReader` static values and `SerieReaderWindows`;
    - `serie_slice.rs`: `SerieWindows` owner, `SerieWindowsIter`;
    - `chunked_serie.rs`: sorted regrouping;
    - `expression/`: the key plan and its three arms, and `child_position`.
  - The `SerieReader` row of the `DataType`/`Field`/`Scalar` table: a reader states its constants as one record, never a column.
  - "Serie is the collection": rows for `window_by(by, sorted)` and a stream's windows.
  - Zero copy: in-order sorted keys cost nothing extra, and a whole stream batch is handed on as itself.
- **Rustdoc:** every new or changed item, with runnable examples; `--doc window_by`.
- **`.api-inventory.txt`:** the five `window_by` entries; `SerieWindows` (`len`, `is_empty`, `serie`, `iter`, no `Iterator`); `SerieWindowsIter`; `SerieReader` around line 5616 (`window_by`, `static_values`, `get_static_value`, `set_static_values`, `with_static_values`, `clear_static_values`); `SerieReaderWindows` (`field`, `static_field`).
- **Checks:** `python scripts/check_api_inventory.py`, `mkdocs build --strict`, `check_docs_examples.py --lang rust|python|javascript`.

## Phases

**Gate.** Base phases 1–4 run exactly as the base specifies. Extension work starts when base phase 4 is green: `cargo check --all-targets` is clean and `--test allocations -- run_slice whole_serie window_by chunked_window_by` passes. Base phases 5–8 do not run separately: X5–X8 subsume them and bind the final signatures once.

| Phase | Theme | Files, disjoint per worker | Starts after | Smoke |
| --- | --- | --- | --- | --- |
| X1a ladder | `serie` | `rust/src/serie/order.rs`, `rust/tests/serie/order.rs` | gate | settled with X1b: `cargo check --all-targets --keep-going --message-format=short`, then `--test serie order` |
| X2 key door | `expression` | `rust/src/expression/{selector,bind,arrow,path}.rs`, `rust/tests/expression/selector.rs` | gate, in parallel with X1a | `--test expression selector`; `--features internals --test expression selector`; `--test media merge`; `--features iceberg --test iceberg merge` |
| X1b held surface | root | `rust/src/serie_slice.rs`, `rust/src/chunked_serie.rs`, `rust/src/lib.rs`, `rust/tests/root/{serie_slice,chunked_serie}.rs`, plus the sweep script over their doc examples | X1a and X2 (reads `window_starts` and `apply_serie_window`) | `--test root serie_slice`, `--test root chunked`, `--doc window_by` |
| X3 stream | `serie` | `rust/src/serie/arrow.rs`, `rust/src/serie.rs`, `rust/tests/serie/arrow.rs` | X1a and X2, in parallel with X1b | `--test serie arrow`, `--doc SerieReader` |
| X4 cost | — | `rust/tests/allocations.rs`, `rust/tests/iobase_calls.rs`, `rust/benchmarks/types/datatype/serie.rs`, `rust/benchmarks/arrow.rs` | X1b and X3 | `--test allocations -- window_by static_values windowed_stream chunked_sorted`, `--test iobase_calls windowing`, the two `--quick` benches. The core settles here |
| X5 Python | — | `python/src/{serie,serie_slice,chunked_serie,lib}.rs`, `python/yggdryl/_native.pyi`, `python/tests/{test_serie,test_serie_slice,test_chunked_serie}.py`, `python/tests/typing_bindings.py`, `python/benchmarks/types/serie.py` | X4 | `cargo check --workspace --all-targets --keep-going --message-format=short`, `maturin develop`, `pytest python/tests/test_serie.py -x -q`, `mypy --strict` |
| X6 Node | — | `node/src/{serie,serie_slice,chunked_serie}.rs`, `node/binding.js`, `node/binding.d.ts`, the generated `node/index.{js,d.ts}`, `node/tests/{serie,serie_slice,chunked_serie}.test.js`, `node/tests/*.types.ts`, `node/benchmarks/types.js` | X5 | `npm run --prefix node build:debug`, `node --test node/tests/serie.test.js` |
| X7 Docs | — | `docs/types/{serie,serie-slice,chunked-serie}.md`, `docs/arrow/readers.md`, `docs/media/index.md`, `skills/yggdryl-arrow/**`, `AGENTS.md`, `.api-inventory.txt`, `.api-bindings.txt` | foreground, while X8 holds the cargo lock | `mkdocs build --strict`, `check_api_inventory.py` |
| X8 chain | — | one background script, one log | X6 | `cargo test --all-targets --all-features --no-fail-fast`, clippy, `--doc`, the §3 and §4 pre-push blocks, `build_docs_playground.js --check`, `build_docs_fix.js --check`, the example runner per language; then one commit, push, and read CI |

The `Proof::Proven` site that X2 adds is reviewed before the commit. `cargo fmt --all` runs once, after the last worker returns.

## Open decisions

1. **The Rust spelling of the flag.**
   - **Default:** a plain `sorted: bool`, on the crate's trailing-bool precedent.
   - **Rejected:** `Option<SortOptions>` and a one-bit options type. The first states two facts in one argument and publishes directions that a stream can only verify. The second is a second vocabulary for one bit.
2. **`SerieWindows` as an owner.** It breaks the in-flight iterator (bind the value, then `iter()`).
   - **Default:** take it. It is the only shape that keeps zero-copy `SerieSlice` items and one return type over both a borrowed holder and a gathered one.
   - **Fallback:** owned `(Scalar, Serie)` items, at k + 2 per record window, losing holder identity and absolute offsets.
3. **Key arms 2 and 3 landing `plan.proof`.** This is one new `Proven` site, for lying cells.
   - **Default:** take it, with the AGENTS review.
   - **If the review refuses:** land `Unproven` as the base does. A mixed key with a code or enum cell then re-proves that cell O(rows) per call or batch, and the docs say so. Arm 1 is unaffected.
4. **`descending` and `nulls_first`.**
   - **Default:** not offered. Keys grouped in any order already yield each key once under `sorted = false`.
   - **Later:** a later need replaces the bool with `SortOptions`, with no shim. That churns three bindings.
5. **Stream windows: lazy sub-readers against held windows.**
   - **Default:** lazy, with the in-order rule and the named passed refusal.
   - **Rejected:** held windows, which would need a new bound constant and pin a whole window.
6. **An unsorted stream under `sorted = true`.**
   - **Default:** refuse at the first descent, naming its way out.
   - **Rejected:** an opt-in collect-and-sort, which would be a second collecting door beside the plan's `order by`.
7. **Static names and `rownum`.**
   - **Default:** `windownum` (relative to its reader) and `rownum` (absolute), both reserved `uint64` names. A collision is refused, never shadowed.
8. **The vectorized epoch arm.**
   - **Default:** still the base's Open decision 2, not in this change. The `serie_reader/window_by/period_key` and `window_by/wide_record` benchmarks report it.
9. **Chunked regrouping, one piece per run.**
   - **Default:** keep it, with no heuristic. The docs advise `into_serie()` first for keys that change on every row.
10. **Absence in the bindings.**
    - **Default:** Python's `sorted: bool` refuses `None` with a `TypeError` (`EventIterator` precedent). Node's `undefined` is skipped and `null` clears to `false` (`SortOptions` precedent). Both are written down once, in `docs/types/serie.md`.
11. **The intermediate-null check in arm 1.**
    - **Default:** the holder's `null_count` (O(1), conservative, so a null outside a window sends that window to arm 2).
    - **Alternative:** a per-window null count, at O(window / 64).