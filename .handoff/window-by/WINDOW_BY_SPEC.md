# window_by and view flattening: implementation spec

Base: Design 1 (two of the three judgements ranked it best). Grafted from the others:
- `SerieSliceMut::window_mut`;
- one shared key rule `Selector::bind_key`;
- a lazy `SerieWindows` iterator in place of an eager `Vec`;
- `Run::into_inner` deleted;
- the whole-window `slice(0, len)` rule;
- a `SerieSlice` argument resolved through its holder in both bindings;
- the absent-row test on a caller's struct key;
- one boundary predicate `Compare::opens`, shared with `partition_by`.

Defects fixed:
- the key lands nullable;
- the record-window cost is stated relative to `into_serie`, so no wrong `+1`;
- the epoch cost is judged by the benchmark, because its allocation count is constant;
- there is no second equality ladder;
- the chunk edge is compared with no allocation;
- the `*` rule is written down;
- merge's "empty match key" message is kept.

The ask ("ins" read as "is") maps to two parts:
1. `window_by(expressions)` on `Serie`, `SerieSlice`, `SerieSliceMut` and `ChunkedSerie`.
2. Every view of a view resolves to its holder with the offsets summed, so nothing nests and nothing copies. This covers a `Run` slice of a slice, a window of a window, a mutable window of a mutable window, `window_by` over a window, and a window passed back to a binding.

## Semantics

**What `window_by(by)` does.** It cuts the rows into maximal runs of adjacent rows whose keys are equal. It answers one `(key, window)` per run, in row order.
- Windows are non-empty, contiguous and disjoint, and they cover `0..len`.
- Adjacent windows have different keys.
- A key can come back: keys A, A, B, A give three windows, keyed A, B and A. This is a run-length or tumbling cut, unlike `partition_by`, which merges every row of a distinct key in first-occurrence order.
- Over keys already in order, `window_by(by)` and `partition_by` over the same key record give the same keys, the same rows and the same order. A test pins this.

**`by`** is anything `IntoSelector` reads. It is parsed once, at the call.
- Rust: a clause text such as `"venue, minutes(ts, 15) as bucket"`, or a `Selector`, `&Selector`, `Projection`, `Term`, `FieldPath` or `Expression`. A Rust `[&str; N]`, `Vec<&str>` or `&[&str]` is a list of exact column names, as `with_merge_by` and `select` already read it. Multi-term keys are therefore written as one clause text, which reads the same in all three languages.
- Python: `SelectorLike`, read through `selector_from_value`.
- JavaScript: `Selector | Term | string | readonly (Term | string)[]`, read through `selector_from_input`.

**Binding.** The key is bound once, against `SerieReader::root_of(field)`:
- a record column binds against its own field;
- any other column binds as the one child of the `row` root, under its own name, so a bare `ts` column is spelled `minutes(ts, 15)`.

Names resolve as in every expression: ASCII case is folded, and two columns that fold together are refused (quote the one meant). The epoch step must be a literal and cannot be zero. `ChunkedSerie` binds once and evaluates the key chunk by chunk.

**The key rule** is shared with merge through `Selector::bind_key`:
- A key states at least one projection. `*` alone and an empty selector are refused with `expected at least one column to window by, got an empty match key`. Merge keeps its byte-identical `... to merge on, got an empty match key`.
- A `*` beside projections is accepted, and keys by every column it keeps plus the projections. For example, `"* exclude (ts), days(ts)"` windows identical consecutive rows modulo `ts`. This is the rule merge already applies; it is now written down and pinned.
- An `unnest` is refused with `refuse_unnest("in a key")`.

**The key value** is the key record's row at the window's first row.
- It is the run of the projected cells, in selector order. One term gives a one-cell run, the shape `partition_by_paths` already pins. Each cell is named by `Projection::name`: the alias, else the path's last name, else the canonical term text.
- An absent record row keys `Scalar::Null`, the struct-null rule of the expression engine. An absent cell is `[null]`.
- A period term keys its period number since the epoch: `int32`, or `date32` for `days`. The period is epoch-aligned in UTC, whatever zone the column states.

**Equality** is the ordering ladder's own: `Compare` under `SortOptions::default()`, the comparison `partition_by` already cuts with.
- Absent equals absent, so consecutive absent keys form one window.
- Every NaN is one value.
- A nested key compares item by item.
- No options are taken: equality has no direction and no null placement.

**What each owner answers.**
- `Serie`, `SerieSlice` and `SerieSliceMut` answer `SerieWindows<'a>`, a lazy, exact-size, fused iterator of `(Scalar, SerieSlice<'a>)`. Every window views the holder: `window.serie()` is the holding serie and `offset()` is absolute in it.
- On a `SerieSlice`, keys are evaluated over the window's rows only, so rows outside it never influence a cut.
- `ChunkedSerie` answers `Vec<(Scalar, ChunkedSerie)>`, each window being `self.slice(from, len)`. A run that crosses a chunk edge is one window keeping the chunks it reaches.

**Edges.**
- Empty input answers no window, but the bind still runs first, so a refusal is the same at any row count.
- One key on every row answers one window. All keys distinct answer one window per row.

**Refusals**, in this order, before any row is read. All are typed `Error::InvalidRecord` and name the serie:
1. the text's own parse error;
2. a schema-free run: `a schema-free run windows by no term`, via `Serie::not_a_record`;
3. an empty key or `*` alone (the shared key rule);
4. an `unnest`;
5. an unknown or ambiguous column, a duplicate published name, or a non-literal or zero epoch step: the binder's or the typer's own message.

**Run view.** A `Run` is a window over one shared `Arc<[Scalar]>`.
- `Serie::slice` of a run shares the holder with the offsets summed, so a slice of a slice is one level over the original allocation.
- A zero-length slice answers the shared empty run, which holds no holder.
- `slice(0, len)` of any serie is the serie itself, a pointer bump.
- Identity is the window's rows alone: `Eq`, `Ord`, `Hash`, `Debug` (`Run([...])`), `Display` and serde all match today's output for the same rows.
- A write goes in place when the run holds its holder alone. Otherwise only the window is copied, which releases the holder. A shared write never clones rows outside the window.
- A live view keeps its whole holder alive, as an Arrow slice keeps its buffers. `memory_size` reports the window.

## Rust API (exact signatures, files)

Public:
- `rust/src/serie/order.rs`: `impl Serie { pub fn window_by(&self, by: impl IntoSelector) -> Result<SerieWindows<'_>> }`. The rows cut into windows of equal adjacent keys that `by` computes, each a view over this serie, in row order.
- `rust/src/serie_slice.rs`:
  ```rust
  #[must_use]
  #[derive(Clone, Debug)]
  pub struct SerieWindows<'a> { serie: &'a Serie, offset: usize, keys: Serie, starts: BooleanBuffer, front: usize, remaining: usize }
  impl<'a> Iterator for SerieWindows<'a> { type Item = (Scalar, SerieSlice<'a>); }
  impl ExactSizeIterator for SerieWindows<'_> {}
  impl FusedIterator for SerieWindows<'_> {}
  ```
- `rust/src/serie_slice.rs`: `impl<'a> SerieSlice<'a> { pub fn window_by(&self, by: impl IntoSelector) -> Result<SerieWindows<'a>> }`. The window's rows cut by keys evaluated over the window alone. Every window is over the same serie at absolute offsets.
- `rust/src/serie_slice.rs`:
  ```rust
  impl<'a> SerieSliceMut<'a> {
      pub fn window_mut(&mut self, offset: usize, length: usize) -> Result<SerieSliceMut<'_>>;
      pub fn window_by(&self, by: impl IntoSelector) -> Result<SerieWindows<'_>>;
  }
  ```
  `window_mut` is a narrower mutable window over the same `&mut Serie`: `require_window(self.name(), offset, length, self.len)`, then `offset: self.offset + offset`. `window_by` is `self.as_window().window_by(by)`.
- `rust/src/chunked_serie.rs`: `impl ChunkedSerie { pub fn window_by(&self, by: impl IntoSelector) -> crate::Result<Vec<(Scalar, ChunkedSerie)>> }`. The same runs across the chunks under one bind. A run crossing a chunk edge is one window, a zero-copy `slice` keeping the chunks it reaches.
- `rust/src/lib.rs:230`: `pub use serie_slice::{SerieSlice, SerieSliceMut, SerieSliceRows, SerieWindows};`
- `rust/src/serie/datatype.rs`:
  ```rust
  #[derive(Clone)]
  pub struct Run { values: Arc<[Scalar]>, start: usize, len: usize }
  ```
  - `#[repr(transparent)]` and `#[serde(transparent)]` are dropped.
  - Written by hand:
    - `Default`: the one shared empty run. The static `OnceLock<Arc<[Scalar]>>` moves here from `Scalar::empty_sequence` (scalar.rs:1643), which then answers `Run::default()`.
    - `Debug`: `debug_tuple("Run").field(&self.as_slice())`.
    - `PartialEq`/`Eq`: an `Arc::ptr_eq && start == start && len == len` shortcut, then the slices compared.
    - `PartialOrd`/`Ord`: over the slice.
    - `Hash`: `self.as_slice().hash(state)`, the bytes `<[Scalar]>::hash` writes, so `hash_rows` still matches it.
    - `Serialize`: the slice as a sequence.
    - `Deserialize`: `Vec<Scalar>` then `Run::new`.
  - `Display`, `NestedValue` and `Value` are unchanged.
  - `pub fn new(values: impl Into<Arc<[Scalar]>>) -> Self` keeps its signature: the whole slice, start 0. It is still a move.
  - `pub fn as_slice(&self) -> &[Scalar]` keeps its signature and now answers `&self.values[self.start..self.start + self.len]`.
  - **Deleted:** `pub fn into_inner(self) -> Arc<[Scalar]>`. Its only callers are `rust/tests/serie/datatype.rs:27-35, 52-55`, and handing out the holder would expose rows outside the window.
- `rust/src/serie.rs` `Serie::slice(&self, offset, length) -> Result<Self>` keeps its signature with a new body:
  1. `require_window`;
  2. if `offset == 0 && length == self.len()`, answer `Ok(self.clone())`;
  3. the run arm answers `Ok(Self::Run(run.slice(offset, length)))`;
  4. the column arm is unchanged.

  New doc: "Zero copy for every leaf: a column is an Arrow slice, a run shares its values; a slice of a slice reaches the holder with the offsets summed; the whole serie is the serie itself; a zero-length run slice holds nothing."
- `rust/src/serie.rs:606-608`: the assert becomes `const _: () = assert!(size_of::<Serie>() == 40);`, with the comment "a run's window - one shared `Arc<[Scalar]>`, where it starts, how long - inline beside a discriminant; every column leaf is one thin pointer". `Scalar` stays 48 (scalar.rs:319, root/string.rs:31, root/bytes.rs:346).

Crate-private:
- `rust/src/serie/datatype.rs`:
  - `pub(crate) fn Run::slice(&self, offset: usize, length: usize) -> Run`: caller-proven. Same `Arc`, `start + offset`. `Run::default()` when `length == 0`.
  - `pub(crate) fn Run::make_mut(&mut self) -> &mut [Scalar]`: if `Arc::get_mut(&mut self.values)` is `None`, then `*self = Run::new(Arc::<[Scalar]>::from(self.as_slice()))`, copying only the window. Then `&mut Arc::make_mut(&mut self.values)[self.start..self.start + self.len]`, which is in place because the holder is now unique. No `unwrap` or `expect`.
- `rust/src/expression/selector.rs`: `pub(crate) fn Selector::bind_key(&self, root: &Field, path: &str, verb: &str) -> Result<BoundSelector>`:
  - empty projections are refused with `path`, `expected at least one column to {verb}, got an empty match key`;
  - then `refuse_unnest("in a key")`;
  - then `bind(root)`.

  `media/merge.rs::key_selector` (162-173) is deleted. Its caller writes `merge_by.bind_key(field, "$.merge_by", "merge on")`.
- `rust/src/expression/selector.rs` (`mod arrow`): `pub(crate) fn BoundSelector::apply_serie(&self, serie: &Serie) -> Result<Serie>`:
  - `array` is the record column's own `require_arrow_array()`, or, for any other column, `StructArray::try_new(<root fields>, vec![serie.require_arrow_array()?], None)`;
  - then `self.apply_arrow_array(&array)`. That keeps the struct null mask, returns the array untouched for an identity selector, and reuses a bare column's `ArrayRef`;
  - then `land(Arc::new(self.output().clone().with_nullable(true)), keys, &Proof::Unproven)`.
  - The root must be nullable: `root_of` forces a record root to `not null`, `bind_with` keeps it on `output()`, and `require_present` (serie/arrow.rs:63-86) would refuse a key with absent rows.
  - This adds no `Proof::Proven` site.
- `rust/src/serie/order.rs`:
  - `pub(crate) fn Serie::window_key(&self, by: Selector) -> Result<BoundSelector>`: a run gives `self.not_a_record("windows by no term")`; otherwise `by.bind_key(&SerieReader::root_of(field)?, self.name(), "window by")`.
  - `pub(crate) fn Serie::window_starts(&self) -> BooleanBuffer`: one `Compare::new(self, SortOptions::default())`, then `BooleanBuffer::collect_bool(len, |index| compare.opens(index))`.
  - `fn Compare::opens(&self, index: usize) -> bool`: `index == 0 || self.cmp(index - 1, index) != Ordering::Equal`. This is the one boundary predicate. `partition_by`'s sorted branch (order.rs:1012) now reads `index == len || compare.opens(index)`, so its pinned count does not move.
  - `pub(crate) fn same_key(left: &Scalar, right: &Scalar) -> bool`: `compare_values(left, right, SortOptions::default()) == Ordering::Equal`, used for the chunk-edge test.
- `rust/src/serie_slice.rs`: `pub(crate) fn SerieWindows::new(serie: &'a Serie, offset: usize, keys: Serie, starts: BooleanBuffer) -> Self`. Sets `remaining = starts.count_set_bits()` and `front = 0`.

## Algorithm and cost

**Key plan, once per call.**
1. `by.into_selector()?` parses the text. A `&Selector` clone is Arc bumps.
2. `window_key` refuses a run, applies the key rule, and binds once against `root_of`. The binder types every term once.
3. `apply_serie` builds the key record, which lands once.

**Cut.** `keys.window_starts()`:
- On the Buffers rung (utf8, integers, temporals and records of them), it costs one array handle plus one Arrow comparator, and builds no row.
- On the Values rung (codes, windows-1252, versions, URLs, unions, foreign NaN), each key row is built once, the same documented cost as `partition_by`.
- Either way: one bitmap of `ceil(len / 8)` bytes, whatever the window count. That is one indirect compare per row and no allocation per row.

**Walk.** `SerieWindows::next`:
1. `start = front`.
2. `end` is the next set bit after `start`, found with `arrow_buffer::bit_iterator::BitIndexIterator::new(starts.values(), starts.offset() + start + 1, len - start - 1)`, or `len` if there is none.
3. Set `front = end` and decrement `remaining`.
4. Yield `(proven_row(&keys, start), SerieSlice { serie, offset: self.offset + start, len: end - start })`.

Allocations:
- the window: zero;
- the key: one `Arc<[Scalar]>` for the key run (none for an absent record row), plus one `Arc<str>` per text cell past `INLINE_CAPACITY`.

**`SerieSlice::window_by`.**
1. Refuse through `self.serie.window_key(..)`, which names the holder.
2. `keys = key.apply_serie(&self.into_serie())`.
3. `SerieWindows::new(self.serie, self.offset, keys, starts)`. Windows are never built over the temporary.

The extra cost is exactly `costs(self.into_serie())`:
- 1 boxed leaf for a flat column;
- 2 plus one leaf per child for a record (`Vec<Serie>`, one leaf per child, the root leaf);
- 0 for a whole window.

**`ChunkedSerie::window_by`.**
1. Bind once against `root_of(&self.field)` with `bind_key(.., self.field.name(), "window by")`.
2. For each non-empty chunk, at running row `base`:
   - `keys = key.apply_serie(chunk)?` and `starts = keys.window_starts()`;
   - walk the set bits as above;
   - a chunk's first window whose key satisfies `same_key(&last.0, &key)` extends the pending window (`last.2 += len`). The equality is the Values rung's own, which the Buffers rung is gated to agree with. A test pins this.
   - otherwise push `(key, base + start, len)`.
3. Empty chunks contribute no bit and are skipped, as `is_sorted` skips them.
4. Finally map each window to `self.slice(from, len)?`.

Cost:
- one bind;
- per non-empty chunk: one `apply_serie`, one comparator and one bitmap;
- per window: one key plus the pinned `ChunkedSerie::slice` (2 for whole chunks, 3 inside one), plus `Vec` growth;
- per edge: at most one key built and dropped;
- never `compare_across` (which costs three allocations per edge).

**Per call, `Serie::window_by`:** parse plus bind plus `apply_serie` plus comparator plus bitmap. This is constant in rows, and the counting allocator pins it at two sizes with a fixed window count.

**Epoch terms** (`days`, `minutes(ts, n)`, ...) take `expression/arrow.rs::fallback` (1288-1315):
- allocations are constant: one row `Vec`, each read column landed once, one answers `Vec::with_capacity(rows)` and one `from_scalars`. Per-row `node.eval` allocates nothing (eval.rs:239-252);
- time is O(rows), and there is a transient of 48 bytes per row.

The allocation row therefore cannot reveal this cost. The `window_by/epoch_key` and `window_by/column_key` benchmarks are the evidence (Open decision 2).

**Pins to add**, in `rust/tests/allocations.rs`, at 64 and 4096 rows, with the selector parsed outside the closure:
1. `a_run_slice_shares_its_values_and_merges_onto_its_holder`. These cost nothing:
   - `run.slice(1, n - 2)`;
   - that slice sliced again;
   - `run.slice(0, n)` and `run.slice(0, 0)`;
   - `run.window(1, n - 2).into_serie()`.

   For each of the 11 `SerieSlice` ordering verbs over a run window, `costs(window.verb()) == costs(sliced.verb())`, where `sliced = run.slice(1, n - 2)` is built outside. `partition_by` over a run with sorted run keys costs the same at both sizes for a fixed group count.
2. `a_slice_of_the_whole_serie_is_the_serie`: `column.slice(0, n)` and `column.window(0, n).into_serie()` cost nothing.
3. `window_by_costs_one_plan_per_call_one_key_per_window_and_nothing_per_row`. The fixture is a record `quote{venue: utf8 (inline), count: int64, ts: datetime64(ns, UTC)}` laid out as 3 venue runs and 4 `minutes(ts, 15)` buckets at both sizes.
   - The build of `window_by(&venue)` costs C, equal at both sizes. Build plus drain costs C + 3.
   - `window_by(&"minutes(ts, 15)")`: the build is equal at both sizes, and the drain adds exactly 4.
   - `window.window_by(&venue) == sliced.window_by(&venue) + costs(window.into_serie())`.
   - `serie.window_mut(1, n - 2)?.window_mut(1, 1)` costs nothing.

   C is pinned once, with a sentence breaking it down into per-call and per-window parts.
4. `a_chunked_window_by_costs_one_bind_per_call_and_the_pinned_slice_per_window`. The fixture has 4 chunks with fixed cuts and one run crossing an edge. The cost is equal at both sizes.

## View flattening

Each site lists its change and the pin that holds it.

| Site | Today | Change | Pin |
| --- | --- | --- | --- |
| `Serie::slice` run arm, serie.rs:2603-2606 | copies the window; a slice of a slice is a copy chain | `Run::slice`: same `Arc`, offsets summed; zero length answers the shared empty | datatype.rs pointer tests; allocations row 1 |
| `Serie::slice(0, len)`, every leaf | boxes a new leaf, or copies a run | `self.clone()` | root/serie.rs; allocations row 2 |
| `SerieSlice::into_serie` (296-300) and every ordering verb on a window (304-376, `dtype` 185, `memory_size` 275) | a run window copied first | inherits both rules: a run window and a whole window cost nothing | allocations rows 1 and 2; the column `through == direct + 1` at 2644-2763 is unchanged |
| `partition_by` sorted groups, order.rs:1014 | a copy per group over a run | views, unchanged code | allocations row 1 |
| `ChunkedSerie` mask and keys per chunk, chunked_serie.rs:918 and 961 | run argument copied per chunk | views, unchanged code | existing chunked tests stay green |
| `[a:b]` and `slice()` over a run row: scalar.rs:2288-2294, expression/path.rs:296-303, eval.rs:747-763, fix/market.rs:1273 | copy | views, unchanged code | whole run; any drop must equal the removed copy |
| `Run::make_mut` (callers order.rs:1236, 1252) | `Arc::make_mut` clones the whole shared holder | in place if unique, else the window alone copied | datatype.rs write test |
| `splice_run`, serie.rs:2275 | copies the whole run | copies the window alone, which releases the holder; no code change | datatype.rs write test |
| `SerieSlice::window` (285-292) | already flat | unchanged | allocations.rs:2330 stays free |
| `SerieSliceMut` mutable sub-window | not expressible | `window_mut`, offsets summed over one `&mut Serie` | root/serie_slice.rs; allocations row 3 |
| `window_by` on a `SerieSlice` | n/a | windows over `self.serie` at `self.offset + start`, never over the `into_serie` temporary | `std::ptr::eq(w.serie(), &holder)` |
| `ChunkedSerie::window_by` | n/a | `self.slice(from, len)` over the original chunks; an edge-crossing run merged into one window | root/chunked_serie.rs `shares` |
| Python `columnar()`, python/src/serie.rs:435 | a `SerieSlice` iterated into a field-less run (keys, indices, mask, `extend`, `splice`, `Serie.from_`, `Scalar.from_`) | a `PySerieSlice` arm returns `Columnar::Held(window.into_core_serie(py)?)`: the live holder re-windowed, zero copy, field kept | test_serie_slice.py |
| Node `serieArgument` (binding.js:1602) and `comparedRows` (2495) | a `SerieSlice` iterated into a run; `serie.equals(window)` refused | a `SerieSlice` resolves by `_intoSerieNative`; `Serie.equals` and `compare` accept a window | serie_slice.test.js |
| Binding `window_by` | n/a | windows over the same holder object (`slf.clone().unbind()`, `self.serie.clone_ref(py)`, `Reference<JsSerie>`, `self.serie.clone(env)?`) at the core's absolute offsets | `w.serie is quotes` / `strictEqual(w.serie, quotes)` |

Not changed, stated in the docs:
- nested column slices (sequence.rs:317 and 609, mapping.rs:231, structure.rs:342) stay flat but rebuild their offsets or children at each level, which is O(window);
- `ChunkedSerie::slice` keeps interior empty chunks;
- adjacent Arrow views are never coalesced, since Arrow cannot widen a buffer view;
- `SerieReader.from_serie` still takes a `Serie` only.

## Bindings

**Python.** Each method only parses with an existing helper, then redirects to the core.
- `python/src/serie.rs`: `#[pyo3(signature = (by))] fn window_by(slf: &Bound<'_, Self>, by: &Bound<'_, PyAny>) -> PyResult<Vec<(PyScalar, PySerieSlice)>>`.
  1. `selector_from_value(by)` under the GIL.
  2. `Self::detached(slf, move |serie| Ok(serie.window_by(selector)?.map(|(k, w)| (k, w.offset(), w.len())).collect::<Vec<_>>()))`.
  3. `(PyScalar::from_inner(k), PySerieSlice::new(slf.clone().unbind(), offset, len))`.
- `python/src/serie_slice.rs`:
  - `fn window_by(&self, py, by) -> PyResult<Vec<(PyScalar, PySerieSlice)>>` runs the same through `self.detached`, building each window as `PySerieSlice::new(self.serie.clone_ref(py), offset, len)`.
  - `pub(crate) fn into_core_serie(&self, py) -> PyResult<Serie>` is `self.read(py, |w| Ok(w.into_serie()))`.
- `python/src/chunked_serie.rs`: `fn window_by(&self, py, by) -> PyResult<Vec<(PyScalar, Py<PyAny>)>>`, the windows going through the existing chunked group describer.
- `columnar()` gets the `PySerieSlice` arm described above.
- `python/yggdryl/_native.pyi`:
  - `Serie` (beside `window`) and `SerieSlice`: `def window_by(self, by: SelectorLike) -> list[tuple[Scalar, SerieSlice]]: ...`;
  - `ChunkedSerie`: `-> list[tuple[Scalar, ChunkedSerie]]`.
- Refusals map to `ValueError` with the core message; a value that is not a selector is a `TypeError`. `SerieSliceMut`, `window_mut` and `SerieWindows` stay Rust-only.

**Node.** Natives are `skip_typescript` and are deleted from the prototype.
- `node/src/serie.rs`: `#[napi(js_name = "_windowByNative", skip_typescript)] pub fn window_by_native(&self, env: Env, reference: Reference<JsSerie>, by: SelectorInput) -> Result<Vec<(JsScalar, JsSerieSlice)>>`. It calls `selector_from_input`, then the core, then builds windows over `reference.clone(env)?` at the absolute offsets.
- `node/src/serie_slice.rs`: `_windowByNative(env, by)`, building windows over `self.serie.clone(env)?`.
- `node/src/chunked_serie.rs`: `_windowByNative(by) -> Vec<(JsScalar, JsChunkedSerie)>`.
- `node/binding.js`:
  - public `windowBy(by)` on `Serie`, `SerieSlice` and `ChunkedSerie`; ChunkedSerie windows are re-prototyped as its partition groups are;
  - the natives go on the delete-lists;
  - `serieArgument` and `comparedRows` get the `SerieSlice` arm.
- `node/binding.d.ts`: `windowBy(by: Selector | Term | string | readonly (Term | string)[]): Array<[Scalar, SerieSlice]>`; ChunkedSerie answers `Array<[Scalar, ChunkedSerie]>`. `serie.equals` and `compare` accept a `SerieSlice`.
- `node/index.js` and `node/index.d.ts` are regenerated by `npm run --prefix node build:debug`.

**Parity.** Argument order, refusal text and key shape are the same in all three languages. The one divergence is pre-existing: a Rust list of `&str` is exact column names, while a Python or JS list is projection texts.

## Tests

Refusals come first. Each new test file opens with its `//!` source line, per the mirror rule.

- `rust/tests/serie/order.rs`:
  - `window_by_refuses_a_run_an_empty_key_an_unnest_and_a_term_naming_no_column`:
    - a parse error;
    - a run: `a schema-free run windows by no term`;
    - `"*"` and `Selector::new(Vec::new())`: path is the serie's name, the message contains `empty match key`;
    - `"unnest(items)"`: `in a key`;
    - `"tier"`: the unknown column, named;
    - columns `a` and `A` keyed by `"a"`: ambiguity;
    - `"minutes(ts, 0)"` and `"minutes(ts, count)"`: the step refusal.

    Each case is also run over a zero-row serie, to show the bind runs first.
  - `window_by_cuts_runs_of_equal_adjacent_keys_in_row_order`:
    - XNAS, XNAS, XNYS, XNAS give three windows (0..2, 2..3, 3..4);
    - `std::ptr::eq(w.serie(), &quotes)`;
    - windows are disjoint and cover; adjacent keys differ;
    - `len()` equals the drain count;
    - a one-term key is a one-cell run;
    - `"venue, minutes(ts, 15) as bucket"` gives a two-cell key in that order;
    - `minutes` keys are `int32`, `days` keys are `date32`;
    - `"VENUE"` folds.
  - `window_by_keys_consecutive_absent_rows_as_one_null_window`:
    - absent record rows give one window keyed `Scalar::Null`; an absent cell gives `[null]`;
    - an identity selector over a record landed from a foreign `StructArray`, whose children hold different values under absent parents, cuts one window over those rows.
  - `window_by_over_sorted_keys_answers_what_partition_by_paths_answers`, including NaN and ±0.0 keys, which agree with `partition_by`.
  - `a_non_record_column_windows_by_its_own_name`: `ts.window_by("days(ts)")`; window offsets carried to an aligned serie through `window(offset, len)`; zero rows give no window.
  - `a_star_beside_terms_keys_every_column_it_keeps`: `"* exclude (ts), days(ts)"`.
  - `a_rust_list_of_texts_names_columns`: `window_by(["minutes(ts, 15)"])` is refused as the column `minutes(ts, 15)`.
- `rust/tests/root/serie_slice.rs`:
  - `a_window_windows_by_its_own_rows_over_the_serie`: a window over rows 1..5; every `serie()` is the holder; offsets are absolute, the first being 1; a run starting before the window is cut at its edge; the results equal the holder's windows restricted to the window; a whole window answers what `serie.window_by` answers; `SerieSliceMut::window_by` answers the same; a window over a run is refused.
  - `a_mutable_window_narrows_onto_the_same_serie`: three nested `window_mut` levels sum to one offset; writes land at the summed row; a sub-window past its parent is refused naming the serie and both counts.
  - `serie_windows_is_an_exact_fused_walk`: `size_hint`, `None` after the end, `Clone`.
- `rust/tests/root/chunked_serie.rs`, `window_by_merges_a_run_across_a_chunk_edge`:
  - chunks `[XNAS, XNAS] [XNAS, XNYS] [] [XNYS]` give (`[XNAS]`, rows 0..3, two chunks) and (`[XNYS]`, rows 3..5, crossing the empty chunk);
  - `shares` the source buffers;
  - keys and rows equal `into_serie()?.window_by(..)`, including a NaN and ±0.0 at an edge;
  - an empty key is refused even with zero chunks.
- `rust/tests/serie/datatype.rs`. Replace the `into_inner` tests at 14-36 and 38-57 with:
  - `a_run_slice_is_a_view_of_its_holder_and_a_slice_of_a_slice_merges_onto_it`: `Serie::new(8 rows).slice(2, 4)?.slice(1, 2)?` has its `as_slice().as_ptr()` at the holder's plus 3;
  - `a_run_view_is_its_window_alone_to_eq_ord_hash_serde_and_debug`: a view and `Run::new` of the same rows compare equal, hash equal, and serialize and Debug-print identically; the hash also equals a column of those rows;
  - `a_write_through_a_shared_run_view_copies_the_window_alone`: the holder's rows are unchanged after a write and the view no longer points into it; a uniquely held run sorts in place with its pointer unchanged;
  - `an_empty_run_slice_holds_no_holder`: its pointer is `Run::default()`'s.
- `rust/tests/root/serie.rs:892`: rename the test to `a_slice_is_zero_copy_for_a_column_and_a_run`, so the run half asserts the holder pointer. Add `a_slice_of_the_whole_serie_is_the_serie` for both leaves.
- Untouched, and must stay green: `rust/tests/media/merge.rs`, `rust/tests/iceberg/mod_.rs:8264`, `node/tests/iceberg.test.js:1320` (`empty match key`).
- Python:
  - `python/tests/test_serie.py` `test_window_by_*` mirrors the order.rs names;
  - `test_serie_slice.py` adds `test_a_window_windows_by_its_own_rows_over_the_serie` (`w.serie is serie`, absolute offsets) and `test_a_window_passed_as_keys_keeps_its_field`;
  - `test_chunked_serie.py` adds the edge merge;
  - `typing_bindings.py` adds `list[tuple[Scalar, SerieSlice]]`.
- Node:
  - `node/tests/serie.test.js`, `serie_slice.test.js` (`strictEqual(w.serie, quotes)`, `serie.equals(window)`, natives private) and `chunked_serie.test.js` mirror the same cases;
  - `*.types.ts` have `windowBy` and `@ts-expect-error windowBy(3)`.

## Benchmarks

- `rust/benchmarks/types/datatype/serie.rs`:
  - `window_by/column_key`, `window_by/epoch_key`, `window_by/through_window`, `chunked/window_by`;
  - `run/slice`, `run/row_at`, `run/window_is_sorted`. `run/row_at` must show the two extra bounds checks in `as_slice` flat before phase 1 settles.
  - Run `cargo bench -p yggdryl --bench types -- "window_by|run/" --quick` (direction only).
- `python/benchmarks/types/serie.py`: rows `Serie.window_by, minutes(ts, 15)`, `SerieSlice.window_by`, `ChunkedSerie.window_by`.
- `node/benchmarks/types.js`: `serie/windowBy`, `serie_slice/windowBy`, `chunked_serie/windowBy`.
- No Performance table on any page changes, since no release number is stated.

## Docs and skills

- `docs/types/serie.md`:
  - line 10: a run holds a window over one shared slice;
  - line 13: Size is 40 bytes, `Scalar` still 48;
  - lines 110 and 154: `slice` is zero copy for both leaves, the whole serie is itself, a slice of a slice reaches the holder;
  - a new `window_by` row beside line 124, and a cost row beside line 170;
  - a "Windows by key" section with Rust, Python and JS tabs over `"venue, minutes(ts, 15) as bucket"`;
  - line 535: the key crosses as a selector;
  - edges: a returning key opens a new window, the one-cell run key, UTC buckets, the `*` rule, and a view pinning its holder;
  - lines 1788 and 1800: writes copy the window alone.
- `docs/types/serie-slice.md`:
  - Doors row: `window_by` gives windows of the holder at absolute offsets; `window_mut` is Rust-only;
  - line 12 and the cost table at 160-166: `into_serie` is zero copy for a run and for a whole window;
  - Bindings row: windows hold the same serie object;
  - the Edges line at 186 is replaced: keys are read over the window alone, and a run window is refused.
- `docs/types/chunked-serie.md`:
  - lines 272-280: `window_by`, and the edge merge;
  - lines 756-766: drop the "column mask/keys" qualifier, since a run argument now slices free.
- `skills/yggdryl-arrow/SKILL.md`: a door-table row "windows of equal consecutive keys", and the pitfalls (not `partition_by`, the run key, a Rust list is column names). Recipes go in `references/rust.md`, `python.md` and `javascript.md`.
- `AGENTS.md`:
  - Layout rows:
    - `serie.rs + serie/`: `Run` is "a window over one shared `Arc<[Scalar]>`, one allocation to build and none to slice"; `order.rs` lists `window_by`;
    - `serie_slice.rs`: `SerieWindows`, `window_by`, `window_mut`;
    - `chunked_serie.rs`: `window_by`;
    - `expression/`: `apply_serie` and `bind_key`.
  - Zero copy: a run's slice shares its values and a slice of a slice reaches the holder; the whole serie is itself.
  - The "Serie is the collection" table: a row "rows cut where the key changes".
- Rustdoc: the `serie_slice.rs` module doc (lines 1-17), `Serie::slice`, `SerieSlice::into_serie`, the `Run` doc, and the size rationale at serie.rs:430-432.
- `.api-inventory.txt`: the `Run` block (about 5159-5176, `into_inner` retired), `Serie::slice` (5661), `SerieSlice::into_serie` (4252), the four `window_by` entries, `SerieWindows`, and `SerieSliceMut::window_mut`.
- `.api-bindings.txt`: Python lines 70-71, 74-75 and 80-81; Node lines 537, 542 and 559-561.
- Checks: `python scripts/check_api_inventory.py`, `python -m mkdocs build --strict --config-file mkdocs.yml`, and `python scripts/check_docs_examples.py --lang rust|python|javascript`.

## Pins that move and why

- `serie.rs:608` moves from 24 to 40. `Run` carries its window inline (a fat `Arc` plus start and len, usize).
- `rust/tests/serie/datatype.rs:14-57` is rewritten, because `into_inner` is deleted.
- `rust/tests/root/serie.rs:892` is renamed and its run half inverted, because a run slice no longer copies.
- `partition_by`'s sorted branch is re-spelled through `Compare::opens` with the same comparator and the same loop, so the 6-allocation pin at 2285-2291 stays.
- No existing allocation count moves by design. Every window and slice pin is over a column or a partial window: 2326-2331, 2344-2350, 2644-2763, 3027-3089, 2008-2040 and 4521+ stay, and `partition_by_paths`'s 154 stays.
- If the whole run shows a pin that slices a run or a whole serie (an expression range, `fix/market.rs:1273`, the fix allocation benches), it may drop only by exactly the removed copy or boxed leaf. It is then re-pinned on its own row, with "a run's slice shares its values" or "the whole serie is the serie". Any other move is a defect.
- Docs and inventory lines that state "a run copies its window".

## Phases

The phases follow AGENTS order. File sets are disjoint per worker, and `cargo fmt` runs once after the last worker.

1. **Core, Run view** (theme `serie`). Worker A owns `rust/src/serie/datatype.rs`, `rust/src/serie.rs`, `rust/src/scalar.rs`, `rust/tests/serie/datatype.rs` and `rust/tests/root/serie.rs`. Smoke:
   - `cargo check -p yggdryl --all-targets --keep-going --message-format=short`;
   - `cargo test -p yggdryl --test serie datatype`;
   - `--test root serie`;
   - `--test root string`, `--test root bytes` (the 48-byte gate).
2. **Core, key door** (theme `expression`, plus the merge caller). Worker B owns `rust/src/expression/selector.rs` and `rust/src/media/merge.rs`. Smoke:
   - `cargo test -p yggdryl --test media merge`;
   - `cargo test -p yggdryl --features iceberg --test iceberg merge`.

   Phases 1 and 2 share no file and run in parallel; one `cargo check` follows both.
3. **Core, window_by.**
   - 3a (theme `serie`): `rust/src/serie/order.rs` and `rust/tests/serie/order.rs`. Smoke `--test serie order`.
   - 3b (root): `rust/src/serie_slice.rs`, `rust/src/chunked_serie.rs`, `rust/src/lib.rs`, `rust/tests/root/serie_slice.rs` and `rust/tests/root/chunked_serie.rs`. Smoke `--test root serie_slice`, `--test root chunked`, `--doc window_by`.
   - Sequential: 3b reads `window_starts`.
4. **Cost.** `rust/tests/allocations.rs` and `rust/benchmarks/types/datatype/serie.rs`. Smoke `--test allocations -- run_slice whole_serie window_by chunked_window_by`, then the `--quick` bench. The core settles here.
5. **Python.** `python/src/{serie,serie_slice,chunked_serie}.rs`, `python/yggdryl/_native.pyi`, `python/tests/{test_serie,test_serie_slice,test_chunked_serie}.py`, `python/tests/typing_bindings.py`, `python/benchmarks/types/serie.py`. Smoke:
   - `cargo check --workspace --all-targets --keep-going --message-format=short`;
   - `maturin develop`;
   - `pytest python/tests/test_serie_slice.py -x -q`;
   - `mypy --strict`.
6. **Node.** `node/src/{serie,serie_slice,chunked_serie}.rs`, `node/binding.js`, `node/binding.d.ts`, generated `node/index.js` and `index.d.ts`, `node/tests/{serie,serie_slice,chunked_serie}.test.js`, `node/tests/*.types.ts`, `node/benchmarks/types.js`. Smoke: `npm run --prefix node build:debug`, then `node --test node/tests/serie_slice.test.js`.
7. **Docs** (the foreground, while the chain below holds the cargo lock). `docs/types/{serie,serie-slice,chunked-serie}.md`, `skills/yggdryl-arrow/**`, `AGENTS.md`, `.api-inventory.txt`, `.api-bindings.txt`. Smoke: `mkdocs build --strict`.
8. **The chain**, in the background with one log:
   1. `cargo test --all-targets --all-features --no-fail-fast`;
   2. clippy;
   3. `--doc`;
   4. the §3 and §4 pre-push blocks;
   5. `build_docs_playground.js --check` and `build_docs_fix.js --check`;
   6. the example runner per language.

   Then one commit, push, and read CI.

## Open decisions

1. **Run view** (Serie grows from 24 to 40 bytes, and a view pins its holder). This is what makes "detect the slice, get the holder, merge" hold for runs. Each `Vec<Serie>` entry grows 16 bytes. A `u32` start and length would give 32 bytes, but needs a refusal inside the infallible `Run::new`.
   - **Default: take it, with `usize`, gated by the `run/row_at` bench.**
   - Fallback: drop phase 1 and merge only at the view level (SerieSlice, `window_mut`, ChunkedSerie, bindings). Then a partial run window's `into_serie` keeps copying.
2. **A vectorized epoch arm** in `expression/arrow.rs` (`PrimitiveArray::unary_opt`, with one shared `EpochPeriod` narrowing owner, a hoisted `i64` divisor, and a row-tier equivalence test). It also speeds up `select`, `where`, `TRANSFORM`, `PARTITION` and `DIGEST`.
   - **Default: not in this change.** Report the `window_by/epoch_key` and `window_by/column_key` benchmark numbers, and open it as its own change.
3. **A key-in-hand door** `window_by_keys(&Serie)`, the only way to window a schema-free run or keys held as a run.
   - **Default: not added.** Keys held as a column are windowed with `keys.window_by("<its name>")`, then each window is carried over with `rows.window(w.offset(), w.len())`.
4. **`partition_by_paths`** keeps exact names, and keys an absent record row by its masked children, where `window_by` folds names and keys that row `Null`.
   - **Default: leave it unchanged and document the difference.** Aligning it is a separate no-back-compat change.