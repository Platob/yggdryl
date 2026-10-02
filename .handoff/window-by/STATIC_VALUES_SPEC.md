# Static values on every series: one record, one trait, every carrier

**Layered on** `WINDOW_BY_SPEC.md` (the base) and `WINDOW_BY_EXTENSION_SPEC.md` (the extension), both in `.handoff/window-by/`. Neither is restated here. Every section of those specs that this one does not name still holds. "Changes relative to the extension spec" lists the edits this spec makes on top of whatever the in-flight extension lands.

**In-flight state checked** (read-only, tree 3851a88 plus uncommitted edits):
- X3 has landed a crate-private `struct Statics { field: Arc<Field>, row: Scalar }` with `WINDOWNUM`, `ROWNUM` and `reserved()` (`rust/src/serie/arrow.rs:1468-1506`).
- It also landed the inherent `SerieReader::{static_values, get_static_value, set_/with_/clear_static_values(FieldRecord)}` (`:1797-1877`), and a `window_by` walk that builds a static row with the key as a `Run` view (`:2443-2470`).
- X4 is mid-way: the `x4_probe` test sits in `rust/tests/allocations.rs:3712`.
- Neither binding binds `window_by` or static values yet.

**Basis.** The base is Design 3: `Statics { field, row: Arc<[Scalar]>, shift }`, which makes a cut free; a fixed-shape nullable `rownum`; `from_landed` taking the record; and a window row that replaces the key run. Grafted onto it:
- From Design 1: a sealed `StaticValues` trait, and hooks only at the `Serie` chokepoints, never in the 15 leaf `slice` bodies.
- From Design 2: wire values carry nothing; parts and children state none; writes that move rows withdraw `rownum`; overflow reads as null; `FieldScalar::get_by_path`; windows have no setters.
- From the judges: a reader's batches stay bare, there is no per-batch stamp, and records are equal-or-none across `from_series`.

**Reading confirmed.** Static values become one contract that every series kind answers with one spelling: `Serie`, `SerieSlice`, `SerieSliceMut`, `ChunkedSerie`, `SerieReader`, every `window_by` window, and later `MediaSerie`. A held window states its key cells, `windownum` and `rownum` exactly as a stream window does. Partition values can ride any series. Five corrections:
1. **A schema-free run states none.** `Serie` is 40 B, which is a run's 32 B plus the tag, so there is no room, and a run has no field.
2. **`rownum` is the one positional cell.** A contiguous cut shifts it. A gather, a regroup or a write that moves rows nulls it. Constants and `windownum` survive every verb.
3. **Parts state none.** A reader's batches, a chunked serie's chunks and a record's children carry no record: one owner per fact.
4. **Wire values carry nothing.** Serde, the value stream, pickle and Arrow write rows only. In-process copies keep the record.
5. **`SerieSlice` loses `Copy`.** A `window_by` window owns its record, built in the one allocation its key already cost.

---

## Contract

**Static values** are one record of values that hold for every row a carrier holds or yields. They say where the rows are (a partition tuple, a window's key and place), never what a row holds.
- The record is a required `Struct` `Field` plus its proven row.
- It sits beside the carrier's root and is never a column, so `field()`, the rows and identity never change because of it.

**Owner.** `Statics`, a public value type in the new root file `rust/src/statics.rs`.
- **The one door is `Statics::from_record(FieldRecord)`.** A `FieldRecord` already proved the row under a non-null record. The door also fixes the reserved cells.
- **What `Statics` owns:** validation, the reserved names, path lookup, the shift, the scatter, window records and the merge.
- Carriers only forward.

**Spelling.** A sealed public trait, `StaticValues`, with one required borrow `statics()` and three provided readers: `static_field()`, `static_values()` and `get_static_value(&FieldPath)`.
- Implemented by `Serie`, `SerieSlice`, `SerieSliceMut`, `ChunkedSerie` and `SerieReader`, and later `MediaSerie`.
- Not implemented by the 15 column leaves (records belong to the root) or by `Statics` itself.
- **Setters** are inherent and exist only where a carrier owns a record:
  - `Serie`: `set_`, `try_with_` and `clear_static_values`. `set_` is fallible because a run is refused.
  - `ChunkedSerie` and `SerieReader`: `set_`, `with_` and `clear_static_values`, all infallible.
  - `SerieSlice`: `with_static_values` only, which states a view's own record.
  - `SerieSliceMut`: none.

**Reserved names.** Their fields are fixed, so every record has one shape whatever its data.

| Name | Fixed field | Meaning |
| --- | --- | --- |
| `windownum` | `uint64`, required | A window's place among the windows of the carrier it was cut from, from 0 |
| `rownum` | `uint64`, nullable | Row `i` of this carrier is row `rownum + i` of the carrier it was cut from (for a reader: of the rows it yields). Null where that is not true |

`from_record` resolves the reserved cells:
- A top-level child whose name folds onto a reserved name under the binder's ASCII fold is that reserved child.
- Its cell is restated through the fixed field's own `Field::scalar`, and the child is respelled in lower case and keeps its place.
- What `Field::scalar` refuses is refused with `Error::InvalidRecord { path: "$.<name>", reason }`, naming the child, the fixed field and the actual datatype and value.
- Two children folding onto one reserved name are refused, naming both.
- No other door checks the reserved cells again.

**The algebra.** Each verb applies one of these to the carrier's record. Constants are every cell except `rownum`, so `windownum` is one of them.

| Class | Constants | `rownum` |
| --- | --- | --- |
| keep | kept | kept |
| shift: a contiguous cut at offset `o` | kept | shifted by `o`; 0 allocations; an overflow reads as null |
| scatter: a reorder, a non-contiguous subset, a write that moves rows | kept | null |
| derive: `window_by` | the carrier's record minus the two reserved cells, then the key cells | a fresh `windownum` and `rownum` |
| take: a part moved into a container | moved to the container | moved to the container |
| common: `ChunkedSerie::from_series` | every chunk agrees, or the container states none | kept only when the chunks are contiguous in one origin, else null |
| drop | none | none |

**What is never touched.**
- Identity (equality, order and hash of every carrier and of the `Scalar` holding a serie), `Display`, digests and `memory_size` read only the rows.
- No column, plan or expression reads a record. The binder scope is a seam (see Open decisions).

---

## Storage per carrier

`Statics` is `{ field: Arc<Field>, row: Arc<[Scalar]>, shift: u64 }`, 32 B. `Option<Statics>` is also 32 B through the `Arc` niche. Both are pinned by `const` asserts in `statics.rs`. `Clone` is two `Arc` bumps.

| Carrier | Field | Size effect | Gates |
| --- | --- | --- | --- |
| `Serie`, column | `pub(crate) statics: Option<Statics>` beside `field` in each of the 15 leaf structs: `NullSerie` null.rs:20, `BooleanSerie` boolean.rs:24, `PrimitiveSerie` primitive.rs:126, `ByteSerie`/`ByteViewSerie`/`FixedSerie` bytes.rs:303/518/724, `DictionarySerie` enums.rs:51, `MapSerie` mapping.rs:58, `RunEndEncodedSerie` runend.rs:45, `OffsetSerie`/`OffsetViewSerie`/`FixedSizeSerieSerie` sequence.rs:157/399/717, `StructSerie` structure.rs:43, `UnionSerie` union.rs:56, `VariantSerie` variant.rs:46 | Each boxed leaf grows 32 B (`NullSerie` 16 → 48, a primitive leaf ≈ 88 → 120). Children grow too and always state none | `Serie` stays 40 (serie.rs:611) and `Scalar` 48 (scalar.rs:319); no variant is added, and `column!`, `column_mut!`, `Leaf` and `as_<leaf>` are unchanged |
| `Serie::Run` | none | — | A run states none, and setting a record on one is refused |
| `ChunkedSerie` | `statics: Option<Statics>` | +32 B | Its chunks state none |
| `SerieReader` | `statics: Option<Statics>` (retyped) and `served: u64` (rows yielded so far) | +40 B | Its batches state none |
| `SerieSlice<'a>` | `statics: Option<Statics>`: the holder's record shifted to the window, or the window's own | 24 → 56 B; `Clone`, no longer `Copy`; pinned by `const _: () = assert!(size_of::<SerieSlice<'static>>() == 56)` | — |
| `SerieSliceMut<'a>` | `statics: Option<Statics>`: the holder's record shifted, taken again after a write that scatters | 24 → 56 B | — |
| `SerieWindows<'a>` | `record: Arc<Field>`, `kept: Box<[Scalar]>`, `base: Option<u64>`, built once | Constant | — |
| `SerieReaderWindows` | as in flight; the `rownum` child becomes nullable | — | — |

**Costs.** "≈" means derived from the source, measured once at implementation and pinned with a breakdown sentence.

| Operation | Allocations |
| --- | --- |
| `statics()`, `static_field()`, `static_values()` at shift 0 or with no non-null `rownum`, `get_static_value` | 0. A cell is an `Arc` bump or inline; a derived `rownum` is an inline `UInt64` |
| `static_values()` at shift > 0 over a non-null `rownum` | 1: the derived row |
| A cut: `Serie::slice`, `window`, `window_mut`, `SerieSlice::window`, `ChunkedSerie::slice` | 0: the shift moves and the fresh leaf takes the record through `Arc::get_mut` |
| Keeping across a cast | 0, onto the fresh landed leaf |
| Scatter | +1 row, only when a non-null `rownum` is stated; otherwise an `Arc` clone |
| Stating or clearing a record on a column | 0 on a unique leaf. A shared leaf costs one leaf-struct copy through `Arc::make_mut` (a record's children `Vec` adds 1) |
| `Statics::from_record` | ≈ 2 + F: the row (one allocation through `shared_children`, as `Scalar::from_sequence` builds one), the `Arc<Field>`, and F for the field clone, measured once. A restated reserved child adds the rebuilt record field |
| A held, chunked or stream window | 1 per window: its record row, which replaces the key run. The key is a `Run` view of the row's key cells, at 0 |
| `window_by` build | +S: what `Statics::window_field` builds plus the kept box, constant. The stream already pays it |
| A statics-free series | Unchanged everywhere: no committed pin moves |

`memory_size` counts buffers only and never the record. No `Proof::Proven` site is added, because a record is never rows.

---

## Verbs

A "fresh leaf" is the one a door just landed. Taking the record onto it costs 0 through `Arc::get_mut`.

| Class | Verbs | Rule |
| --- | --- | --- |
| **Keep** | `Clone`; `Scalar::from(serie)` and `Scalar::as_serie`; `SerieValue::into_serie` of a whole leaf; `Serie::slice(0, len)` (`self.clone()`); `Serie::cast` and `ArrowCastPlan::apply`, identity or not (cast.rs:677: the record is taken again after `land_planned`); `apply_chunked` (cast.rs:699); `ChunkedSerie::cast`; `SerieReader::cast` | Same record, same shift. A cast keeps it because it says where the rows are, not what the root declares |
| **Keep: overwrites** | `Serie::set`; a `splice` where `range.len() == rows.len()`; `set_cell`; `set_child` (the entering child is stripped); `SerieSliceMut::{set, fill, copy_from, splice}` | The rows stay where they are, so `rownum` stays true |
| **Shift** | `Serie::slice(o, l)` (serie.rs:2626: `shifted(o)` on the fresh leaf); `Serie::window`/`window_mut` (snapshot); `SerieSlice::window`; `SerieSlice::into_serie` (the slice, then the window's own record); `ChunkedSerie::slice`; `partition_by` groups over sorted keys (they go through `slice`, order.rs:1140) | `rownum += o`, read on demand |
| **Scatter** | `Serie::taken` and `Serie::filtered` (order.rs:987/1071), which wrap both arms, including `taken`'s zero-width fixed-size-serie `from_scalars` return at :1003. Through them: every `into_sorted`/`into_unique`/`into_reversed`/`into_taken`/`into_filtered`, every `as_*` doing `*self = self.into_*()`, `partition_by` groups over unsorted keys, and a held sorted gather. In place: `sort_range_in_place` and `reverse_range_in_place` (order.rs:1557/1577) when they return `true`, so the in-place and replacing forms agree. `SerieSliceMut::{swap, as_sorted, as_reversed, as_taken}`: the holder is scattered and the snapshot taken again. `ChunkedSerie` `into_*`/`as_*`, `partition_by` and `partition_by_chunked` groups | Constants and `windownum` kept, `rownum` null |
| **Scatter: writes that move rows** | `Serie::splice` with `range.len() != rows.len()`, so `push`, `insert`, `remove`, `pop`, `truncate`, `clear` and `extend`; `Serie::write` with a count change (`resize`); `Serie::append` (the `extend_from_serie` fast path); every `get_<leaf>_mut` borrow, through one `leaf_mut`, conservatively, because a typed writer can move rows and states nothing about which; `ChunkedSerie::push_chunk` | One check at each `Serie` door, after the write succeeds. A refused write changes nothing. `extend_from_serie` never reads `other`'s record |
| **Derive** | `window_by` on the five owners | See Windowing |
| **Take** | `ChunkedSerie::from_serie`; `SerieReader::from_serie` (a non-record serie becomes a stripped child in `held_record`, serie/arrow.rs:1527); `SerieReader::from_chunked`; `ChunkedSerie::from_serie_reader` (the reader's record shifted by `served`, read before the drain); `ChunkedSerie::into_serie` (the join, `[one]` or `[]` states the chunked record) | Moved to the container, and the part becomes bare |
| **Common** | `ChunkedSerie::from_series` | The record every chunk states, when all are equal in field and in every cell but `rownum`. `rownum` is kept when each chunk's continues the one before it, else null. A bare chunk or any disagreement gives none. Chunks are stripped. O(chunks) comparisons, at most 1 row |
| **Drop** | `into_run`; `into_arrow_*` on `Serie` and `ChunkedSerie`; `SerieReader::into_arrow_reader`; the C Data and C Stream exports; IPC; `IOMedia::write_arrow` (which calls `into_arrow_reader`, so a part writer reads the record first); `from_arrow_*`, `from_scalars`, `empty`, `with_capacity`, `from_default`, `repeat`; `child`, `children`, `items`, `get_child_by_path`; a typed leaf's `SerieValue::slice` and the other leaf verbs (records belong to the root); the batches a reader or sub-reader yields; serde; the value stream; pickle | The new carrier states none |

**Edges.**
- A run answers `None`, `set_static_values` refuses it with `require_field`'s refusal, and `clear_static_values` on a run is a no-op.
- A failed narrowing through `get_<leaf>_mut` (a leaf of another kind) changes nothing.
- The crate-private key record of `apply_serie`'s direct arm may hold the holder itself as its child 0 (selector.rs:1780). It is never lent, and every window reads its cells by value.
- A static named like a root column is allowed, because a Hive leaf may store its partition column too ("the file wins", media/partition.rs). A collision is refused only where one window record would hold two cells of one name.

---

## Identity and wire

**Identity is unchanged by construction, and pinned.** None of these reads the leaf field:
- `serie_leaf!`'s `compare_rows`/`hash_rows` (serie.rs:149/305/318);
- `Serie`'s `Eq`/`Ord`/`Hash` (serie.rs:3142-3176);
- `ChunkedSerie` (chunked_serie.rs `PartialEq`/`Ord`/`Hash`);
- `SerieSlice` (serie_slice.rs:980-1019);
- `Scalar::Serie` (scalar.rs `Ord`/`Hash`);
- `Display` (`display_column`), `stable_hash`, the xxhash Arrow row digests (which read arrays) and `memory_size`.

`Statics` itself is `Eq + Hash` over its field and its derived cells, with the shift folded into `rownum`. That lets routing and tests compare records.

**`Debug`** prints the record only when one is stated:
- `debug_column` gains a `statics: Option<&Statics>` parameter, which each of the 15 leaf `Debug` impls passes;
- `SerieSlice`, `SerieSliceMut`, `ChunkedSerie` and `SerieReader` (as in flight) print it too;
- `Statics` prints its field name and derived cells.

**Wire rule:** a record lives in this process and travels with in-process copies only.

| Door | Record |
| --- | --- |
| Rust `Clone`; Python `__copy__`/`__deepcopy__` (`inner.clone()`, python/src/serie.rs:~1522); Node `clone()` | kept |
| `Scalar::from(serie)`, `Scalar::as_serie` | kept, ignored by the `Scalar`'s identity |
| serde (`SerieWire { field, rows }`, serie.rs:3971, unchanged; `Scalar`'s structural wire) | dropped; a deserialized serie states none |
| value stream (valuestream.rs:359-376, already rows only) | dropped |
| Python pickles (`Serie.__reduce__` python/src/serie.rs:1514, `ChunkedSerie.__reduce__` chunked_serie.rs:791, tuples unchanged); Node `toJSON` | dropped |
| Arrow arrays, batches and readers, IPC, C Data, C Stream, Node's IPC copies | dropped; no metadata sidecar |

No wire shape changes, so AGENTS' no-back-compat rule is not engaged.

---

## Rust API (exact signatures, files)

**New root file `rust/src/statics.rs`.** `lib.rs` gains `mod statics;` and `pub use statics::{StaticValues, Statics};`.

```rust
/// One record of values constant over every row a carrier holds or yields.
#[derive(Clone)]
pub struct Statics {
    field: Arc<Field>,   // a required record
    row: Arc<[Scalar]>,  // its cells, proven under it once
    shift: u64,          // rows between where the row's rownum is true and this carrier's row 0
}
const _: () = assert!(size_of::<Statics>() == 32);
const _: () = assert!(size_of::<Option<Statics>>() == 32);

impl Statics {
    pub fn from_record(values: FieldRecord<'_>) -> crate::Result<Self>; // the one door; reserved cells restated
    pub fn field(&self) -> &Field;
    pub fn values(&self) -> FieldScalar<'_>;                         // 0 allocations, or 1 for a shifted non-null rownum
    pub fn get(&self, path: &FieldPath) -> Option<FieldScalar<'_>>;  // record steps by exact name; rownum derived
}
impl TryFrom<FieldRecord<'_>> for Statics { type Error = crate::Error; } // = from_record
impl PartialEq for Statics {}  impl Eq for Statics {}  impl Hash for Statics {}  impl fmt::Debug for Statics {}

mod sealed { pub trait Sealed {} } // the graph/operation.rs:29 precedent
pub trait StaticValues: sealed::Sealed {
    fn statics(&self) -> Option<&Statics>;
    fn static_field(&self) -> Option<&Field> { self.statics().map(Statics::field) }
    fn static_values(&self) -> Option<FieldScalar<'_>> { self.statics().map(Statics::values) }
    fn get_static_value(&self, path: &FieldPath) -> Option<FieldScalar<'_>> { self.statics()?.get(path) }
}
```

**Crate-private in `statics.rs`.** `WINDOWNUM`, `ROWNUM` and `reserved()` move here from serie/arrow.rs:1496-1506.

```rust
pub(crate) const WINDOWNUM: &str = "windownum";
pub(crate) const ROWNUM: &str = "rownum";
pub(crate) fn reserved(name: &str) -> Option<&'static str>;
impl Statics {
    pub(crate) fn rownum(&self) -> Option<u64>;                 // derived: None when absent, null or overflowed
    pub(crate) fn base(this: Option<&Self>) -> Option<u64>;     // a window's origin: 0 with no rownum child, None when null
    pub(crate) fn shifted(&self, rows: usize) -> Self;          // 0 allocations; saturating, so an overflow reads null
    pub(crate) fn scattered(&self) -> Self;                     // rownum null: one row only when a non-null rownum is stated
    pub(crate) fn is_same(&self, other: &Self) -> bool;         // pointer and shift, so a restate can be a no-op
    pub(crate) fn kept(this: Option<&Self>) -> (Vec<Field>, Box<[Scalar]>); // minus windownum and rownum
    pub(crate) fn window_field(root: &str, kept: &[Field], keys: &[Field], nullable_keys: bool) -> crate::Result<Arc<Field>>;
        // refuses a key cell folding onto a kept or reserved name, naming both and asking for an alias
    pub(crate) fn window(field: &Arc<Field>, kept: &[Scalar], keys: &Serie, row: usize,
                         windownum: u64, rownum: Option<u64>) -> (Self, Scalar); // one row; the key is a Run view
    pub(crate) fn common<'s>(chunks: impl Iterator<Item = (Option<&'s Self>, usize)>) -> Option<Self>;
}
```

**`rust/src/typed.rs`:**

```rust
impl<'a> FieldScalar<'a> {
    /// Record steps by each child's exact name (`Field::index_of`); the empty path is the whole value;
    /// a folded name, any other segment, or a step past an absent record answers None.
    pub fn get_by_path(&self, path: &FieldPath) -> Option<FieldScalar<'a>>;
}
```

This is the in-flight `SerieReader::get_static_value` body (serie/arrow.rs:1832), moved. `Statics::get` is that walk over the stored row, with the `rownum` step derived.

**`rust/src/serie.rs`:**

```rust
impl StaticValues for Serie {} // column!(self, _run => None, column => column.statics.as_ref())
impl Serie {
    pub fn set_static_values(&mut self, values: Statics) -> Result<()>; // require_field()? first: a run is refused
    pub fn try_with_static_values(self, values: Statics) -> Result<Self>;
    pub fn clear_static_values(&mut self);                              // a run: no-op
    pub(crate) fn restate(&mut self, statics: Option<Statics>);         // no-op when is_same; Arc::get_mut, else make_mut
    pub(crate) fn scatter_statics(&mut self);                           // restate(scattered) when a non-null rownum is stated
    fn leaf_mut<L: Leaf>(&mut self) -> Option<&mut L>;                  // L::narrow(self)?; scatter; L::narrow_mut(self)
}
```

- `slice`, `splice`, `write`, `append` and `set_child` get the hooks listed in Verbs.
- Every `get_<leaf>_mut` body becomes `self.leaf_mut()`: one exact-string sweep of `Leaf::narrow_mut(self)`.
- `debug_column` gains its `statics` parameter.

**The 15 leaf files** (`rust/src/serie/{null,boolean,primitive,bytes,enums,mapping,runend,sequence,structure,union,variant}.rs`):
- the field is added and `new` sets it to `None`;
- the six hand-written `Clone` impls copy it (bytes.rs:466/674/927, primitive.rs:560, sequence.rs:366/658);
- every struct literal, which the compiler finds, sets `None`; no body uses `..` spread syntax, so the compiler forces each one;
- `Debug` passes the field to `debug_column`.

**`rust/src/serie/order.rs`:**
- the bodies of `taken` and `filtered` become `taken_rows` and `filtered_rows`;
- `taken` and `filtered` wrap them with `restate(self.statics().map(Statics::scattered))`;
- `sort_range_in_place` and `reverse_range_in_place` call `scatter_statics()` when they return `true`.

**`rust/src/serie/arrow.rs`:**

```rust
pub struct SerieReader { inner: Option<Source>, root: Arc<Field>, schema: SchemaRef,
                         statics: Option<Statics> /* crate::Statics */, served: u64 }
impl StaticValues for SerieReader {}
impl SerieReader {
    pub fn set_static_values(&mut self, values: Statics);
    pub fn with_static_values(self, values: Statics) -> Self;
    pub fn clear_static_values(&mut self);
    pub(crate) fn unread_statics(&self) -> Option<Statics>; // the record shifted by `served`
}
```

- `Iterator::next` adds each yielded batch's length to `served`, for every source.
- `cast` keeps both `statics` and `served`.

**`rust/src/serie_slice.rs`:**

```rust
#[derive(Clone)] pub struct SerieSlice<'a> { serie: &'a Serie, offset: usize, len: usize, statics: Option<Statics> }
pub struct SerieSliceMut<'a> { serie: &'a mut Serie, offset: usize, len: usize, statics: Option<Statics> }
impl StaticValues for SerieSlice<'_> {}  impl StaticValues for SerieSliceMut<'_> {}
impl<'a> SerieSlice<'a> {
    pub fn with_static_values(self, values: Statics) -> Self; // this window stating its own record; the holder untouched
}
impl SerieWindows<'_> {
    pub fn static_field(&self) -> &Field; // the record every window states, before any is walked
}
```

- `SerieSliceRows` holds a copy of the window without its record.
- `SerieSliceMut::as_window` clones the snapshot.

**`rust/src/chunked_serie.rs`:**

```rust
pub struct ChunkedSerie { field: Arc<Field>, chunks: Vec<Serie>, ends: Vec<usize>, statics: Option<Statics> }
impl StaticValues for ChunkedSerie {}
impl ChunkedSerie {
    pub fn set_static_values(&mut self, values: Statics);
    pub fn with_static_values(self, values: Statics) -> Self;
    pub fn clear_static_values(&mut self);
    pub(crate) fn from_landed(field: Arc<Field>, chunks: Vec<Serie>, statics: Option<Statics>) -> Self;
        // strips a chunk only when it states a record
}
```

Its 19 call sites (chunked_serie.rs plus cast.rs:708) are respelled by one sweep driven by `cargo check -p yggdryl --all-targets --keep-going --message-format=short`. Each site states keep, shift, scatter, common or `None`, as the Verbs table says.

**`rust/src/cast.rs`:** `apply` takes the source's record again after `land_planned`; `apply_chunked` passes `chunked.statics().cloned()`.

**Errors.** Every static-values door is on the `Scalar` side and returns `crate::Result`, or nothing.

**Deleted.**
- The crate-private `struct Statics` in serie/arrow.rs:1475.
- The inherent `SerieReader::static_values` and `get_static_value`. Callers now `use yggdryl::StaticValues`.
- The `FieldRecord`-taking `set_`/`with_static_values`.
- The test-side `// The window is Copy` pin (tests/root/serie_slice.rs:171).

---

## Windowing

**One record shape on every owner.** A window states a required record named `SerieReader::root_of(<the windowed carrier's field>).name()`. Its children, in order:
1. **Kept:** the windowed carrier's record minus `windownum` and `rownum`. For a window of a window, that includes the outer key cells.
2. **The key cells,** named as `Projection::name` names them, in selector order. They are declared nullable when the windowed carrier's field is nullable, because a held record may hold absent rows. A reader's root is required, so its key cells stay as declared.
3. **`windownum`:** `uint64`, required.
4. **`rownum`:** `uint64`, nullable. It is `Statics::base(carrier)` plus the window's first row relative to the carrier's row 0, and null when the base is null or the windows were gathered or regrouped.

**The collision refusal** (the in-flight refusal 5, now `Statics::window_field`) runs at all five owners, after the binder's refusals and before any key row is read. A key named `rownum` or `windownum` is aliased. Every window states a record, even over a bare carrier.

**Held** (`Serie`, `SerieSlice` and `SerieSliceMut::window_by` give `SerieWindows`):
- `SerieWindows::new` builds `record`, `kept` and `base` once (+S). The base is `None` on the `Cuts::Gathered` path, since the gathered holder already went through `taken` and so is scattered.
- Each `SerieWindowsIter` step calls `Statics::window(record, kept, keys, key_row, position, rownum)`: one row (kept, then the key children's cells at `key_row`, then `windownum`, then `rownum`), which replaces `proven_row(&keys, key_row)`.
- The step yields `(key, SerieSlice { serie: holder, offset, len, statics: Some(record) })`.
- `rownum` under `Cuts::Starts` is `base + (offset − windows.offset)`.
- An absent key-record row yields the key `Scalar::Null` and null key cells.
- The per-window cost is unchanged, and `ptr::eq(windows.serie(), &quotes)` holds over keys in order.

**Chunked** (`ChunkedSerie::window_by`):
- **In order** (`sorted = false`, or no descent): each run is pushed with its `Statics::window` row, and its key is the view, so the cost per window is unchanged. `windownum` is the run's index and `rownum` is `base + offset`. Each window is `self.slice(offset, len)`, restated with its record.
- **Regrouped:** after the stable sort and merge, each window's row is built once from its first run's key cells, with `rownum` null. That is +1 per window on this path only, beside the run keys the sort needs. Each window is `from_landed(field, pieces, Some(record))`.

**Stream** (`SerieReader::window_by`): the in-flight walk, with these changes:
- `kept`, `base` and the field come from `Statics::kept`, `Statics::base(self.unread_statics())` and `Statics::window_field`, so a half-read reader still numbers its windows absolutely;
- `Walk.base` becomes `Option<u64>`;
- each opened window's record is `Statics::window(..)`;
- the law `B·10 + W·2 + P·5` and the free whole-batch piece stand;
- pieces are bare, and a sub-reader owns its record;
- `ChunkedSerie::from_serie_reader(sub)` keeps a window's record, which is how a stream window is held with where it came from.

**Correspondence pin** (replaces "Held windows attach no static values"). For `s = false`, and for `s = true` over keys already in order, these three state the same `static_field()` and the same `static_values()`, window by window:
- `rows.window_by(by, s)?.iter()`;
- `SerieReader::from_serie(rows)?.window_by(by, s)` sub-readers;
- `ChunkedSerie::from_serie(rows)?.window_by(by, s)?`.

This holds with `rows` stating nothing, and with `rows` stating `{day, rownum: 1000}`. For `s = true` over keys out of order, the held gather equals the chunked regroup, with `rownum` null, and the stream refuses.

**Seam.** The extension spec's MediaSerie and partition paragraph holds as written. `MediaSerie` implements `StaticValues`, and its parts' records are typed partition tuples.

---

## Bindings

The bindings bind the contract once, in the extension's X5 and X6. Every method parses, redirects and wraps; nothing in a binding implements static-value logic. Statics live in the core value, so the roughly 140 wrap sites, copy/clone, slice, cast, `ArrowCastPlan.apply` and the ordering verbs keep, shift or drop them with no binding edit.

**Python** (shared `pub(crate)` helpers in python/src/serie.rs beside `sort_options`):
- `statics_into_py(&Statics) -> dict[str, Scalar]`: names zipped with cells, in field order.
- `statics_from_py(values, field, root_name) -> PyResult<Option<Statics>>`:
  - `None` clears;
  - otherwise it is `Statics::from_record(FieldRecord::new(field or <Scalar::dtype(values) as a required record named root_name>, Scalar.from_(values)))`;
  - a mapping's names come back sorted, so a field fixes the order;
  - `root_name` is the carrier's field name (`Serie::require_field()?` first, so a run raises the core refusal).

Members:
- `PySerie`, inherited by every leaf subclass: `static_values` (getter, `dict[str, Scalar] | None`), `static_field` (getter, `Field | None`), `get_static_value(path: str | FieldPath) -> Scalar | None` (path parsed by `core_path_from_value`), and `set_static_values(values, field=None) -> None`.
- `PyChunkedSerie`: the same four.
- `PySerieReader`: the same four.
  - It caches `statics: Option<Statics>` beside `field`, captured at `from_inner`, so a spent reader still answers.
  - `set_static_values` writes both the cache and the live reader. On a spent reader it raises the error `take()` raises.
- `PySerieSlice` stays frozen and gains `statics: Option<Statics>`, the window's own record (`Statics` is `Send + Sync`).
  - `read()`, `detached()` and `write()` rebuild `parent.window(o, l)?` and then `.with_static_values(own.clone())` when it has its own record.
  - `None` reads the parent's live record through the core snapshot.
  - It has the three readers and no setter.
  - The docstring line "holds the Serie object, an offset and a length, and nothing else" gains "and a `window_by` window's own record".
- `Serie.window_by` and `SerieSlice.window_by`: the detached closure returns `(Option<Serie>, Vec<(Scalar, usize, usize, Statics)>)`, each record an `Arc`-bump clone.
- `SerieReaderWindows.static_field` is as in flight.
- `_native.pyi`, `typing_bindings.py`, and `mypy --strict`.

**Node:**
- `JsSerie` (on `Serie.prototype`, so the leaf prototypes inherit), `JsChunkedSerie`, `JsSerieReader`:
  - `_staticValuesNative() -> Array<[string, Scalar]> | null` (`skip_typescript`; `binding.js` builds the object through `Object.fromEntries`);
  - `staticField` getter;
  - `getStaticValue(path: string | FieldPath)`;
  - `setStaticValues(values, field?)`: `undefined` is skipped and keeps the record, `null` clears it.
- `JsSerieSlice` gains `statics: Option<Statics>` and gets the three readers.
- `JsSerieReader` caches the record at `from_core`, before any take or end of stream.
- Natives go into the frozen native tables and their delete lists (binding.js:1455-1571, 2101-2146, 2358-2474) and into binding.d.ts. `node/index.{js,d.ts}` are regenerated by `npm run --prefix node build:debug`.
- Arrow crossings drop the record. Node has no `SerieReader` write door, so a JS part writer reads `staticValues` before `intoArrowReader()`; a `SerieReader` IOMedia door is MediaSerie's change, not this one.

**Parity.**
- Identical in all three languages: the names (snake and camel case), the argument order `(values, field)`, `None`/`null` clears, the reserved names, the refusal texts, and windows being read-only.
- Rust only: the `StaticValues` trait, the `Statics` type, `try_with_static_values`, `SerieSliceMut`, `SerieSlice::with_static_values`, `SerieWindows::static_field`, and `FieldScalar::get_by_path`.

**Inventories.** `.api-bindings.txt`:
- Python lines 70, 74 and 78: the four members on Serie, ChunkedSerie and SerieReader; line 80: SerieSlice gets the three readers; the `SerieReaderWindows` entry.
- Node lines 534-568: the camelCase twins.

---

## Tests

Mirror files, refusals first. Each new file opens with its `//!` source line.

- **`rust/tests/root/statics.rs`** (new; `//! rust/src/statics.rs`; declared in `rust/tests/root.rs`):
  - `from_record_refuses_a_reserved_cell_its_fixed_field_cannot_hold`: `windownum` null, `rownum` −1, `rownum` and `RowNum` side by side. The message names the child and both fields.
  - `from_record_restates_reserved_cells_under_their_fixed_fields`: an `int64` `rownum` becomes nullable `uint64`; `RowNum` becomes `rownum` in its place.
  - `get_reaches_record_steps_by_exact_name_only`: a nested cell is reached; a folded name, an index or key segment and a step past an absent record answer `None`; the empty path is the whole record.
  - `a_shift_moves_rownum_alone_and_an_overflow_reads_null`.
  - `a_scatter_nulls_rownum_and_keeps_every_other_cell`.
  - `two_records_are_equal_and_hash_alike_when_fields_and_derived_cells_are`.
  - `every_carrier_answers_one_spelling`: one generic `fn check(c: &impl StaticValues)` over Serie, a plain SerieSlice, SerieSliceMut, ChunkedSerie, SerieReader, a held window, a chunked window and a stream window.
  - `static_values_never_enter_identity`: `==`, `cmp`, hash, a `HashSet` holding one, `Display`, serde JSON, value-stream bytes, `stable_hash` and `memory_size`, stated against bare, for each carrier and for `Scalar::from(serie)`.
  - `every_value_and_byte_door_drops_them`: `into_arrow_array` then `from_arrow_array`, serde, the value stream, `into_run`, `from_scalars`.
- **`rust/tests/root/typed.rs`:** `field_scalar_get_by_path_lends_record_steps_by_exact_name`.
- **`rust/tests/root/serie.rs`:**
  - `a_run_states_no_static_values_and_refuses_them_by_name`;
  - `a_column_sets_and_clears_its_record`;
  - `every_layout_slices_its_record_with_rownum_shifted`: all 15 leaves over the nested and flat fixtures; a leaf's own `SerieValue::slice` states none; a whole slice keeps;
  - `a_write_that_moves_rows_nulls_rownum_and_an_overwrite_keeps_it`: every count-changing verb, both `extend_from_serie` paths, `get_int64_mut().push_value(..)`, a failed narrowing changing nothing, and `set`/`set_cell` keeping;
  - `set_child_strips_the_entering_child_and_children_state_none`;
  - `a_scalar_holding_a_stated_serie_keeps_it_and_compares_by_rows`.
- **`rust/tests/serie/order.rs`:**
  - `a_gather_keeps_constants_and_windownum_and_nulls_rownum`: every `into_*` against its `as_*` twin, the primitive and boolean in-place arms, and `taken`'s zero-width fixed-size branch;
  - `partition_by_shifts_rownum_where_sliced_and_nulls_it_where_taken`;
  - `held_window_by_refuses_a_key_cell_folding_onto_a_kept_or_reserved_name_before_any_row`.
- **`rust/tests/root/serie_slice.rs`:**
  - `a_window_states_its_holders_record_shifted_at_the_cut`, including a window of a window;
  - `a_window_by_window_owns_its_record_and_narrowing_shifts_it`;
  - `into_serie_states_what_the_window_states`;
  - `serie_windows_names_its_static_field_before_any_window`;
  - `held_windows_state_kept_keys_windownum_and_rownum`: `Starts` versus `Gathered` (rownum null), the key is the record's view, and an absent key row gives a `Null` key with null cells;
  - `a_mutable_windows_reorder_nulls_the_holders_rownum_and_its_overwrites_keep_it`;
  - `with_static_values_never_writes_the_holder`.
- **`rust/tests/root/chunked_serie.rs`:**
  - `from_serie_takes_the_record_and_its_chunk_states_none`;
  - `from_series_states_what_every_chunk_agrees_on`: contiguous, broken, bare and disagreeing chunks;
  - `push_chunk_keeps_the_record_nulls_rownum_and_strips_the_chunk`;
  - `into_serie_states_the_record_for_zero_one_and_many_chunks`;
  - `slice_shifts_cast_keeps_and_ordering_verbs_null_rownum`;
  - `window_by_windows_state_their_record_and_a_regroup_nulls_rownum`;
  - `window_by_refuses_a_key_cell_folding_onto_a_kept_name`.
- **`rust/tests/root/cast.rs`:** `a_cast_keeps_the_record_identity_or_not`, over `Serie::cast`, `ArrowCastPlan::apply` and `apply_chunked`.
- **`rust/tests/serie/arrow.rs`:**
  - `from_serie_and_from_chunked_lift_the_record_and_yield_bare_batches`;
  - `from_serie_reader_takes_the_record_shifted_past_what_was_served`;
  - `a_window_of_a_half_read_reader_numbers_its_rows_absolutely`;
  - `the_held_chunked_and_stream_windows_state_one_record`, the correspondence pin;
  - the in-flight `static_values_are_carried_by_cast_and_dropped_at_the_transport_face`, respelled through `StaticValues` and `Statics`;
  - `window_by_on_a_reader_refuses_before_any_pull` loses its "`rownum` static of utf8" case, which `from_record` now owns.
- **Python:**
  - `python/tests/test_serie.py` (Serie and SerieReader): dict in field order, `static_field`, a str and a `FieldPath` path, `None` clears, a run raises `ValueError`, copy keeps and pickle drops, `==` ignores the record, a spent reader still answers, held window records;
  - `test_serie_slice.py` (read-only, `window_by` records, narrowing shifts) and `test_chunked_serie.py`;
  - `typing_bindings.py`.
- **Node:** `node/tests/{serie,serie_slice,chunked_serie}.test.js`, mirroring the Python cases, plus `setStaticValues(undefined)` keeping the record and `setStaticValues(null)` clearing it; `*.types.ts` with `@ts-expect-error setStaticValues` on a `SerieSlice`.

---

## Pins

All in `rust/tests/allocations.rs`, each run at 64 and at 4,096 rows.

**New pins:**
1. `static_values_are_lent_free_on_every_carrier`. It replaces the in-flight `static_values_are_lent_free`. Every reader costs 0 on every carrier; `static_values()` after a shift over a non-null `rownum` costs 1; `get_static_value(rownum)` costs 0.
2. `a_cut_carries_its_record_for_nothing`. Each of these costs the same stating a record as bare: `Serie::slice`, `window`, `SerieSlice::window`, a partial `into_serie`, `ChunkedSerie::slice`, a non-identity `ArrowCastPlan::apply`, `apply_chunked`.
3. `a_gather_nulls_rownum_with_one_row`. Stating `{venue, rownum}` costs bare + 1. Stating `{venue}` costs bare, including the in-place `as_sorted`.
4. `a_write_that_moves_rows_nulls_rownum_once`. The first `push` costs bare + 1, the second costs bare, and `set` costs bare.
5. `stating_a_record_copies_a_leaf_only_when_shared`. On a unique leaf it costs 0; on a shared one, 1 (a record leaf, 2). `from_record` costs ≈ 2 + F.
6. `a_held_window_states_its_record_in_the_row_its_key_cost`. The drain per window is unchanged and the build is + S.
7. `a_chunked_window_states_its_record_in_the_row_its_key_cost`. The in-order path is unchanged per window, with `BIND` + S. A regroup costs + 1 per window.
8. `a_stated_serie_crosses_into_a_scalar_for_nothing`: 0.

**Standing pins:** the stream law `B·10 + W·2 + P·5` and the free whole-batch piece; `iobase_calls`' `windowing_a_read_adds_no_call` (records add no call).

**Pins that move** (re-pinned once, in S5, each with its sentence):
- `WINDOW_BY_COLUMN_KEY`, `WINDOW_BY_PERIOD_KEY` and `WINDOW_BY_VALUES_KEY` gain + S: "the window record field and its kept cells, built once".
- The chunked window_by `BIND` gains + S.
- X4's chunked sorted-regroup pin gains + W: "each merged window's row".

No per-window, per-chunk, per-edge or slice pin moves, and every statics-free pin stays put. A move anywhere else is a defect.

**Size pins:** `size_of::<Statics>() == 32`, `size_of::<Option<Statics>>() == 32`, `size_of::<SerieSlice<'static>>() == 56`. The 40 and 48 B gates are untouched.

**Benchmarks**, direction only with `--quick`:
- `rust/benchmarks/types/datatype/serie.rs`: `serie/static_values/lend`, `serie/slice_stated` against `serie/slice`, `serie/into_sorted_stated`, `window_by/record_read`;
- `python/benchmarks/types/serie.py`: `Serie.static_values`;
- `node/benchmarks/types.js`: `serie/staticValues`.

No Performance table changes.

---

## Docs

- **`docs/types/static-values.md`** (new; mkdocs.yml nav under Types core, after "Serie slice"). It is the one owner of:
  - the contract;
  - `Statics` and `StaticValues`;
  - the reserved names and their fixed fields;
  - carriers and owners;
  - the Verbs table;
  - the wire table ("in process keeps, bytes drop");
  - the cost table;
  - window records and the correspondence;
  - edges: a run states none; writes keep constants, which are the owner's claim, so restate after mixing sources; `rownum` is null after a gather or a move; `use yggdryl::StaticValues`; a leaf's own verbs state none;
  - Rust, Python and JavaScript tabs, each with an assertion: state `{date, venue}`, a slice shifts, a sort nulls `rownum`, `==` ignores the record, and `window_by` windows state `{venue, windownum, rownum}`.
- **`docs/types/serie.md`:**
  - the stale "Size | 24 bytes" row (line 13) becomes 40 B, plus "a stated column's leaf + 32 B";
  - a Static values row in the doors table linking the new page;
  - "Windows by key" says every window states its record.
- **`docs/types/serie-slice.md`:** `SerieSlice` is `Clone` and 56 B; a window states its holder's record shifted, or its own; `with_static_values` is Rust only; windows have no setters.
- **`docs/types/chunked-serie.md`:** one record per chunked serie and chunks state none; the `from_series` agreement rule; `push_chunk`; window records.
- **`docs/arrow/readers.md`:** the extension's "Static values" section shrinks to the stream specifics plus a link: batches are bare; `served`; `from_serie`, `from_chunked` and `from_serie_reader` carry the record; windows state the shared shape.
- **`docs/media/index.md`:** one sentence in the Read and write overview: a part's static values are the partition seam, read before `write_arrow`.
- **`skills/yggdryl-arrow/SKILL.md`** and `references/{rust,python,javascript}.md`:
  - a door row: "values constant over a series (partition, key, place)", spelled `StaticValues::static_values` / `static_values` / `staticValues`;
  - pitfalls: transport and pickle drop the record; a gather or a moving write nulls `rownum`; a run states none; a window has no setter.
- **`AGENTS.md`:**
  - Layout rows: `statics.rs` (Statics, the sealed StaticValues, reserved names, shift, scatter, window records, `common`); `serie.rs + serie/` (every column leaf holds `Option<Statics>` beside its field, a run holds none, the hooks sit at `slice`, `taken`, `filtered`, `splice`, `write`, `append`, `leaf_mut`); `serie_slice.rs` (a reference, an offset, a length and its record; `Clone`); `chunked_serie.rs` (one record, chunks bare); `typed.rs` (`get_by_path`).
  - The DataType/Field/Scalar table gets a row "values constant over the rows | `Statics`, beside the root, never a column, rows-alone identity".
  - "Serie is the collection" gets a Want/Spell row.
  - Zero copy gets a bullet: lending, a cut and a cast carry a record at 0 allocations.
- **`.api-inventory.txt`:**
  - a `statics.rs` section: `Statics` (`from_record`, `field`, `values`, `get`, `TryFrom`) and `StaticValues` (`statics`, `static_field`, `static_values`, `get_static_value`);
  - `set_`/`try_with_`/`clear_static_values` on `Serie`;
  - `set_`/`with_`/`clear_` on `ChunkedSerie` and `SerieReader`, with the inherent `SerieReader` readers deleted;
  - `SerieSlice::with_static_values` and `SerieWindows::static_field`;
  - `FieldScalar::get_by_path`.
- **Rustdoc:** runnable examples on `Statics::from_record`, `StaticValues`, each setter, `SerieWindows::static_field` and `FieldScalar::get_by_path`. The in-flight `SerieReader` examples are respelled with `use yggdryl::{StaticValues, Statics}`.
- **Checks:** `python scripts/check_api_inventory.py`; `python -m mkdocs build --strict --config-file mkdocs.yml`; `python scripts/check_docs_examples.py --lang rust|python|javascript`.

---

## Changes relative to the extension spec

These are edits to apply on top of whatever the in-flight X1a–X4 land. Nothing is aliased or shimmed: every replaced spelling, pin and sentence is deleted in the same commit.

1. **The owner moves.** The crate-private `struct Statics { field, row: Scalar }`, `WINDOWNUM`, `ROWNUM` and `reserved()` in `rust/src/serie/arrow.rs` move to the public root file `rust/src/statics.rs`, retyped as `{ field: Arc<Field>, row: Arc<[Scalar]>, shift: u64 }`.
2. **The reader's accessors.** The inherent `static_values` and `get_static_value` are deleted in favour of `StaticValues`. `set_static_values` and `with_static_values` take a `Statics`, built by `Statics::from_record(FieldRecord)`. `clear_static_values` stays. `SerieReader` gains `served`.
3. **Refusal 6 is deleted** ("a reader stating `windownum` or `rownum` at another datatype"). `from_record` restates or refuses at the one door. Refusals 1–5 keep their order.
4. **`rownum` is nullable** in every window record (serie/arrow.rs:2037 `required_field(ROWNUM)` becomes the fixed nullable field). `Walk.base: u64` becomes `Option<u64>`, from `Statics::base(unread_statics())`, so a half-read reader's windows stay absolute.
5. **The static-values door table:**

   | Door | Extension | Now |
   | --- | --- | --- |
   | `from_serie`, `from_chunked` | none | take the carrier's record; held items are stripped |
   | `ChunkedSerie::from_serie_reader` | dropped | takes the reader's record, shifted by `served` |
   | `Serie::from_arrow_reader` | dropped | unchanged: a `BatchReader` is transport |
   | `cast` | kept | kept |
   | transport face | dropped | dropped |

   New row: a reader's yielded batches are bare.
6. **"Held windows attach no static values" is deleted, with its `(key, position, offset)` pin.**
   - `SerieWindows` gains `record`, `kept`, `base` and `static_field()`.
   - Each item's `SerieSlice` owns its record, built in the row that replaces the key run; the key is a view of that row.
   - The correspondence pin compares records across held, chunked and stream windows, as in Windowing.
7. **`ChunkedSerie::window_by` windows state records.** A regroup builds each merged window's row (+1 per window on that path).
8. **Refusal 5** (the key-cell collision) applies at all five `window_by` owners, through `Statics::window_field`.
9. **`SerieSlice` loses `Copy`.** It grows 24 → 56 B, `SerieSliceRows` holds a copy without the record, and the in-flight item stays `(Scalar, SerieSlice<'s>)`.
10. **Allocation pins:**
    - `static_values_are_lent_free` widens to every carrier;
    - the held builds and the chunked `BIND` gain + S, and the chunked regroup gains + W;
    - the stream law stands.
11. **Bindings (X5 and X6):**
    - the accessors spread from `SerieReader` to `Serie`, `SerieSlice` (read-only) and `ChunkedSerie`;
    - the Python held `window_by` closure returns `(Scalar, usize, usize, Statics)`;
    - Node's `_windowByNative` carries each window's record into `JsSerieSlice`;
    - the `PySerieReader` and `JsSerieReader` caches hold a `Statics`.
12. **Docs (X7):** the Static values contract moves from `docs/arrow/readers.md` to `docs/types/static-values.md`.
13. **The seam paragraph stands.** `MediaSerie` implements `StaticValues`.

---

## Phases with disjoint file sets

**Gate.** X4 settles first, with its own pins re-pinned. X5–X8 are held until S5 settles, then bind and document the generic contract once. Everything lands as one commit, and `cargo fmt --all` runs once, after the last worker returns.

| Phase | Theme | Files | Starts after | Smoke |
| --- | --- | --- | --- | --- |
| S1 owner | root | `rust/src/statics.rs` (new), `rust/src/typed.rs`, `rust/src/lib.rs`, `rust/tests/root/statics.rs` (new), `rust/tests/root.rs`, `rust/tests/root/typed.rs` | X4 | `cargo check -p yggdryl --all-targets --keep-going --message-format=short`; `--test root statics`; `--test root typed get_by_path`; `--doc Statics`; `--doc StaticValues` |
| S2a leaves | `serie` | `rust/src/serie/{null,boolean,primitive,bytes,enums,mapping,runend,sequence,structure,union,variant}.rs` (one compiler-driven sweep: the field, `new`, the six `Clone` impls, struct literals, `Debug`) | S1 | Settled together with S2b |
| S2b serie root | `serie` | `rust/src/serie.rs`, `rust/tests/root/serie.rs` | S1, in parallel with S2a | One `cargo check --keep-going` after both; `--test root serie static` |
| S3 serie verbs | `serie` | `rust/src/serie/order.rs`, `rust/src/serie/arrow.rs`, `rust/tests/serie/{order,arrow}.rs` | S2 | Settled together with S4 |
| S4 carriers | root | `rust/src/serie_slice.rs`, `rust/src/chunked_serie.rs` (the `from_landed` sweep), `rust/src/cast.rs`, `rust/tests/root/{serie_slice,chunked_serie,cast}.rs` | S2, in parallel with S3 | One `cargo check --keep-going` after both; `--test serie order`; `--test serie arrow static`; `--test root serie_slice`; `--test root chunked`; `--test root cast`; `--doc window_by` |
| S5 cost | — | `rust/tests/allocations.rs`, `rust/benchmarks/types/datatype/serie.rs` | S3 and S4 | `--test allocations -- static window_by chunked_window`; `--test iobase_calls windowing`; `cargo bench -p yggdryl --bench types -- static --quick`. The core settles here |
| X5 Python | — | The extension's X5 set (`python/src/{serie,serie_slice,chunked_serie,lib}.rs`, `_native.pyi`, the three test files, `typing_bindings.py`, `benchmarks/types/serie.py`), with the members above | S5 | The extension's X5 smoke |
| X6 Node | — | The extension's X6 set | X5 | The extension's X6 smoke |
| X7 Docs | — | The extension's X7 set plus `docs/types/static-values.md` and `mkdocs.yml` | Foreground, while X8 holds the cargo lock | `mkdocs build --strict`, `check_api_inventory.py` |
| X8 chain | — | One background script, one log | X6 | The extension's X8 chain, then commit, push and read CI |

---

## Open decisions with defaults

| # | Decision | Default | Alternative and why not |
| --- | --- | --- | --- |
| 1 | `SerieSlice` storage | Owns `Option<Statics>`: 56 B, `Clone`, a `window_by` record built in its key's row | Design 2's lazy `cut: &SerieWindows`, `Copy` at 48 B, costs one row per read and a public `get`/`into_owned` on `SerieWindows`. Design 1's eager table costs O(W × width) memory |
| 2 | Writes | Overwrites keep; count-changing writes, reorders and every `get_<leaf>_mut` borrow null `rownum`; constants are always the owner's statement | "Writes keep everything" is simpler but leaves a false `rownum` that a later slice compounds |
| 3 | `ChunkedSerie::from_series` | All chunks agree, else none | Always none loses partition values on a rebuild; per-name intersection silently erases claims |
| 4 | Wire | serde, the value stream, pickle and Arrow drop the record; copies keep it | An optional `SerieWire` key plus a third pickle argument is a new shape, and equal `Scalar`s would serialize differently |
| 5 | Reserved cells | Restated through the fixed field's `Field::scalar` (best effort), refused otherwise | Exact `uint64` only, refusing the rest (the in-flight rule) |
| 6 | `Scalar::from(serie)` | Keeps the record (zero copy); `Scalar` identity ignores it | Stripping costs a leaf copy and is bypassable through a directly built `Scalar::Serie` |
| 7 | A reader's `rownum` | Static, describing its first row; `served` shifts only what is built from its unread rows | Advancing on every batch changes a "static" value while reading |
| 8 | The contract's shape | A sealed `StaticValues` trait plus inherent setters | Mirrored inherent readers per carrier offer no generic bound for later consumers (`MediaSerie`) |
| 9 | Key children over a nullable holder | Declared nullable; an absent key row gives a `Null` key and null cells | A data-dependent record shape |
| 10 | The expression binder scope (statics as `Kind::Literal`, typed `Bounds` from a record) | Not in this change; a seam | It pulls in X2's `bind.rs`, `typing.rs` and `selector.rs` |
| 11 | A Node `SerieReader` IOMedia door | Not in this change; it is MediaSerie's | — |
| 12 | Task #15 (`SerieSlice` → `WindowSerie`) | Lands after this, as its own sweep script, and renames the new members with everything else | — |
| 13 | Typed leaf verbs | A leaf's own `slice` and the like state none; a whole leaf turned back into a serie keeps the record it carried | Hooking 15 leaf bodies, which the compiler does not force |
| 14 | Storage width per leaf | 32 B inline (`Option<Statics>`), so cuts and window records cost 0 extra allocations | `Option<Arc<...>>` plus a shift is 16 B, but adds one allocation per stated record and per window and moves the per-window pins |