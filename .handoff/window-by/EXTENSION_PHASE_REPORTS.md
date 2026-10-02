## x1a:ladder

X1a is implemented, but the real tree does not compile until X1b changes `SerieWindows::new` and removes the two `same_key`/`window_starts()` callers. I smoked it in a scratch copy instead: HEAD plus my two files plus a scratch-only `SerieWindows` built to the spec. The order suite passes there (41 tests).

**Files edited:** `/home/user/yggdryl/rust/src/serie/order.rs` and `/home/user/yggdryl/rust/tests/serie/order.rs`, both run through `rustfmt --edition 2024`.

**What `order.rs` now has:**
- `Serie::window_by(by, sorted: bool)`, with the spec's body: `SerieWindows::new(self, 0, key.apply_serie(self)?, sorted)`. The rustdoc is rewritten: what `sorted` means (and how that differs from `EventIterator::new`), ascending only, the cost of the gather, and the `u32` refusal. The example adds the sorted case over XNYS, XNAS, XNYS, XNAS: two windows, XNAS first.
- New crate-private `WindowCut { starts, descent, regrouped }` and `Regrouped { order: Vec<u32>, windows: Box<[(u32, u32)]> }`.
- `window_starts(regroup) -> Result<WindowCut>`: one `Compare`, one `collect_bool` that also records the first `Greater`. With `regroup` and a descent, it runs `require_indexable(self)?` and then `Compare::regroup` on the same comparator. `regroup` builds the runs from the bits, stable-sorts them by key, merges equal runs and builds `order`. Allocations: runs (1), sort scratch, order (1), windows (1).
- `Compare::step` added; `opens` is now `step(index) != Equal`.
- New `Compare::Record { nulls, children, options }` rung, tried after `Buffers` and before `Values` for a record column with children. Absent rows are placed by the options; each child is compared on its own rung and the result is not reversed again.
- `compare_to_row(value, row, options)`: compares a held key against a key-record row cell by cell, without building the row's run. An absent row reads as Null; a non-record column falls back to `compare_values`.
- `compare_values` and `require_indexable` are now `pub(crate)`. `same_key` is deleted.

**Tests:** every existing `window_by` call now passes `false`, and `cuts` takes `&SerieWindows`. The refusal matrix became a helper run by the existing test and by the new `window_by_sorted_refuses_what_unsorted_refuses_before_any_row`. The renamed test is `a_record_with_a_value_ordered_leaf_compares_child_by_child_and_agrees_with_its_run`. New tests:
- `window_by_sorted_over_keys_in_order_answers_the_unsorted_windows_over_the_serie` (includes `ptr::eq`, NaN and ±0.0, absent cell and absent row)
- `window_by_sorted_gathers_the_rows_once_in_stable_key_order` (the spec's 5-row case, an absent row, a two-cell key)
- `window_by_cuts_every_nested_key_as_its_values_do` (`nested_columns`, union, map, run-end, a `{venue: mic, px, qty}` record with foreign NaNs, and the `order`/`order.venue`/`order.px` keys, under both flags)
- `the_record_rung_agrees_with_its_run_under_every_ordering`

Because `compare_values` is crate-private, the tests carry their own copy of that order (`value_order`) as the reference.

**Deviations:**
- The descent is recorded in the `FnMut` closure's captured `Option` instead of a `Cell`.
- The doc links name `SortOptions::ascending`, which is the default value.

**Smoke results:**
- Real tree, `cargo check -p yggdryl --all-targets --keep-going --message-format=short`: 6 errors, all in the expected places:
  - `rust/src/window_serie.rs:1043`, `:1047` and `:1049` (old `window_starts()` and `SerieWindows::new`);
  - `rust/src/chunked_serie.rs:1071` (`same_key`);
  - `rust/src/serie/order.rs:1375`, two errors, both from calling the 4-argument `SerieWindows::new` that X1b has not written yet.
- Scratch copy, `cargo test -p yggdryl --test serie order`: 41 passed. Whole `--test serie`: 226 passed.
- Seen failing: flipping the Record rung's absence placement and disabling the run merge made the three new behaviour tests fail; restoring the code made them pass again.
- Scratch clippy (`--lib --test serie --no-deps`): one warning, `WindowCut::descent` never read. That clears once X1b's chunked code or X3's stream walk reads it.
- Scratch `--test allocations`: one pin moved, covered below. Every other pin passed, including `partition_by`, the ordering pins and the chunked pins. Four FIX pins failed only because the scratch copy lacked `config/`, and passed once I added it.
- Not run validly: `--doc window_by`. In scratch the doctest linked a stale library because of the artifact sharing below, so X1b's `--doc window_by` smoke is the first real run.

**Build hazard:** the scratch workspace shared `target-w2` with the same relative package path, so it wrote over the real tree's `yggdryl` build files. I then touched my two files so the next real-tree build rebuilds from source, and deleted the scratch copy. If a smoke run since about 11:30 UTC behaved oddly, rerun it.

**Needs other owners:**
- **`rust/src/serie.rs` (X3):** beside `mod order;` (line 216) add `pub(crate) use order::{compare_values, require_indexable};`. Without it, `window_serie.rs` cannot reach `require_indexable` and `chunked_serie.rs` cannot reach `compare_values`. The `WindowCut` and `Regrouped` fields are reachable without naming the types.
- **X1b:**
  - Write `SerieWindows::new(holder, offset, keys, sorted) -> Result<Self>` per the spec.
  - In `chunked_serie.rs`, replace `same_key` with `keys.compare_to_row(&last_key, 0, SortOptions::default())`.
  - Call `window_starts(false)?`, or `window_starts(sorted)?`, wherever the old no-argument form is used.
- **X4 (`allocations.rs`):** `window_by_over_value_ordered_keys_builds_each_key_row_once` moves from build/walk `WINDOW_BY_VALUES_KEY + rows` / `+ rows + 3` to `WINDOW_BY_VALUES_KEY + 1` / `+ 1 + 3`. That is 36/39 at both 64 and 4,096 rows. It is the code-bearing record pin the spec expects to drop through the `Record` rung. Suggested reason sentence: "one leaf's values built, not one run per row; the one is the Record rung's boxed child comparators".

## x2:key-door

X2 (the key door) is in place in my five files, and all four smoke commands pass. I ran them in a scratch worktree, because the main tree's lib can't compile until X1b lands. X1a has changed `window_starts` and deleted `same_key`, so `window_serie.rs:1043-1049` and `chunked_serie.rs:1071` fail to build. The worktree was a detached checkout at HEAD 3851a88 with only my five files copied in, using the same `CARGO_TARGET_DIR`; I've removed it since. Nothing was committed or pushed.

**Files edited**
- `rust/src/expression/selector.rs`
- `rust/src/expression/bind.rs`
- `rust/src/expression/arrow.rs`
- `rust/src/expression/path.rs`
- `rust/tests/expression/selector.rs`

**What was implemented**
- **`KeyPlan`**: `bind_key` builds it, and it replaces the `key_root` OnceLock. `bind_with` sets `key: None`, so merge carries the plan but never reads it.
- **Three arms** in `apply_serie_window`, with `apply_serie` delegating to it:
  - **Direct**: every cell lies and no record between the keyed record and a lying cell has an absent row (checked by `null_count`). The key is `StructSerie::new(root, <landed cells sliced to the window>, <the keyed record's nulls sliced>, len)`. Each cell is checked against its published field (same dtype, no non-null over nullable). A column laid out differently from what the key was bound against is refused by name, so `into_arrow_array`'s `expect(ALIGNED)` stays sound.
  - **Narrow**: a batch of only the read columns, Arrow-sliced, then `projected(.., Some(&schema))` → `StructArray::from` → `land_planned`.
  - **Whole**: the window holds absent rows of the keyed record. Same flow, via `struct_rows` and `rebuilt_struct`.
- **`Bound::lies_where`** (`bind.rs:351`), plus a shared `published_as` rule (`bind.rs:404`).
- **`child_position`** (`arrow.rs:842`), now used by `segment_array`, `struct_child_field`, `struct_child`'s sequence arm and `lies_where`.
- **The new `Proof::Proven` site** is commented for review at `selector.rs:1499`.
- **Internals forwarders**: `apply_serie_window`, and `lying_cells`, which `a_key_plan_is_bound_once_and_names_its_lying_cells` needs. `python3 scripts/generate_internals.py` printed "lib.rs: internals re-exports 112 module(s)", left `lib.rs` unchanged, and `--check` reports "yggdryl::internals is current".
- **Tests**: `a_key_plan_is_bound_once_and_names_its_lying_cells` and `the_direct_key_arm_equals_the_engine_arm_on_every_nested_layout`. The parity test covers 15 keys, four absence variants (including a foreign `StructArray` with values left under absent parents) and four windows. I also added two refusal tests: a window past the end, and a column the key wasn't bound against. With the cell guard disabled, the second one returned `Ok` with a mismatched key, so the guard does real work.

**Deviations and why**
1. **The plan is split in two.** `KeyPlan` holds `root` and `lies` plus an optional `KeyEngine` (schema, resolved, reads, narrow, proof). The engine half is built only when a cell is computed or lies below a record step. A key made only of the record's own top-level columns always takes the direct arm, so building the engine for it was wasted: this cut the cost of the plan itself (the spec's K) from about 21 allocations to about 8.
2. **The whole arm uses the read columns only.** It builds a narrow struct with the record's nulls instead of the whole record. That gives the spec's own cost line, O(rows × read columns), and leaves columns the key doesn't read untouched.
3. **The narrow schema comes from each read column's cached projection**, not the root's projection, so the plan's cost stays constant in record width.
4. **I fixed an existing panic in `scattered`** (`arrow.rs:558`). The parity test found it: Arrow's `take` panics on a dense union, or a struct or fixed-size list holding one, at a null scatter index when the first member is empty. You can hit it through `Selector::apply_arrow_array` on a record with absent rows. Those rows now take row 0 instead (the record's own nulls already mark them absent), or are laid out null when there is no row at all.
5. **`Node::collect_columns` is now `pub(crate)`**, so the plan collects read columns without allocating a vector per term.

**Smoke commands** (env `CARGO_TARGET_DIR=/home/user/target-w2 CARGO_INCREMENTAL=0`)

| Command | Result |
| --- | --- |
| `cargo test -p yggdryl --test expression selector` | ok, 22 passed |
| `cargo test -p yggdryl --features internals --test expression selector` | ok, 35 passed |
| `cargo test -p yggdryl --test media merge` | ok, 19 passed |
| `cargo test -p yggdryl --features "parquet iceberg internals" --test iceberg merge` | ok, 15 passed (not skipped; disk was 6.9 GB free) |
| `--test expression` (whole), `--test serie window`, `--test root window` (widened) | ok: 175, 10 and 16 passed |
| clippy `--lib --test expression` with internals, and `--lib`, both `-D warnings` | clean |

**Allocation counts for X4** (I didn't edit `allocations.rs`; measured at HEAD plus my files, the same at 64 and 4,096 rows):

| Pin | In-flight | Now |
| --- | --- | --- |
| `WINDOW_BY_COLUMN_KEY` build / walk | 48 / 51 | 31 / 34, so K = 8 |
| `WINDOW_BY_PERIOD_KEY` build / walk | 93 / 97 | 95 / 99 |
| `WINDOW_BY_VALUES_KEY` | 35 + rows | 24 + rows |
| chunked `window_by` | 105 | 80 |

The period key goes up by 2 at 3 children; X4's wide-record pin should confirm it stays constant in width. The through-window case is still `direct + 5` (39 against 34), because `WindowSerie::window_by` still keys through `into_serie`. It becomes `direct + 1` once X1b calls `apply_serie_window`.

**Left for other owners**
- **X1b** must use `key.apply_serie_window(self.serie, self.offset, self.len)` in `WindowSerie::window_by` and fix the broken call sites above.
- **X4** re-pins the numbers in the table.
- **Possible engine defect, untouched** (arrow.rs, but outside X2's spec): `Context::new` matches batch columns by the first case-insensitive name, so two root columns whose names differ only in case can read the wrong column. Exact-name-first matching would fix it.
- **Disk**: the scratch builds left a few hundred MB to a couple of GB of yggdryl artifacts in `/home/user/target-w2` that I can't separate from other workers' by name. 7.0 GB is free now.

## x1b:held

X1b is done. My files compile, and every smoke I own passes in the real tree, not a scratch copy. `cargo check --all-targets` still fails, but only in `allocations.rs` and the `types` bench. Both are X4's, and both just need the new `window_by(by, sorted)` spelling (sites listed at the end).

**Files edited** (all under `/home/user/yggdryl/`; each ran through `rustfmt --edition 2024` except `lib.rs`, whose changed line is 97 chars):
- `rust/src/window_serie.rs`
- `rust/src/chunked_serie.rs`
- `rust/src/lib.rs`
- `rust/tests/root/window_serie.rs`
- `rust/tests/root/chunked_serie.rs`

**What was implemented**
- **`SerieWindows<'a>`** is now an owner.
  - It holds the windowed serie (borrowed, or the gathered copy it owns), the key offset, the keys, and either the row-order start bits or, after a gather, the key-order window table.
  - It has `len`, `is_empty`, `serie`, `iter`, and `&SerieWindows` works in a `for`. Its `Iterator` impls are deleted.
  - `SerieWindowsIter<'s>` is the walk, exact-size and fused; the row-order step is the base's `next` body, moved.
  - `lib.rs` exports `SerieWindowsIter`.
- **`SerieWindows::new(holder, offset, keys, sorted) -> Result<Self>`** runs `keys.window_starts(sorted)?`.
  - On a regroup: `require_indexable(holder)?`, shift `order` by `offset` in place, take the rows once into an owned holder, keep the window table. `order` is then dropped.
  - Otherwise it borrows the holder.
- **`WindowSerie::window_by(by, sorted)`** keys through `apply_serie_window(self.serie, self.offset, self.len)`, never `into_serie`. `WindowSerieMut::window_by(by, sorted)` goes through `as_window()`.
- **`ChunkedSerie::window_by(by, sorted)`**:
  - Binds once. Per non-empty chunk: `apply_serie`, then `window_starts(false)`, reading its `descent`.
  - At each chunk edge, `keys.compare_to_row(&pending_key, 0, SortOptions::default())`: `Equal` extends the pending window and builds no key; `Greater` records a descent.
  - Unsorted, or no descent: each window is `self.slice(..)`, as before.
  - Sorted with a descent: a stable `sort_by` of the runs on `compare_values`, adjacent equal keys merged. Each window is `from_landed` over its runs' zero-copy pieces in arrival order, keyed by its first run's key. No join, no row copy.
- **Two small helpers:**
  - `pub(crate) fn window_end(starts, start)` in `window_serie.rs`, shared by the walk and the chunk fold.
  - `ChunkedSerie::extend_pieces`, a private split of `slice`'s body. `slice` keeps its allocation count: one pieces `Vec`, now sized with `reserve_exact`, plus `ends`.
- **Tests:**
  - **window_serie:** the existing window tests and the run/empty/`*` refusals now run under both flags. New: `serie_windows_lends_its_windows_again_and_again` (replaces `serie_windows_is_an_exact_fused_walk`), `serie_windows_owns_a_gathered_holder_and_borrows_otherwise`, `a_window_windows_sorted_over_its_own_rows`.
  - **chunked:** the refusals run under both flags. New: `window_by_sorted_regroups_runs_across_chunks_with_no_row_copied` (`[B, A] [A, B]`, a descent only at an edge, absent keys last across an edge, `shares`, equal to the joined `window_by(.., true)`) and `window_by_compares_chunk_edges_in_place` (NaN payloads and ±0.0 at edges, both flags). The NaN case moved there out of the merge test.

**Deviations**
- A sorted chunked window costs less than the spec's count. Each run adds its pieces to the window's vector through `extend_pieces`, rather than building a full `self.slice(from, len)` per run. That saves the two temporary vectors per run; the cost is still constant in rows.
- The spec's `Cut`/`descent` cell became a plain local: the chunk fold reads `cut.descent` and the edge `Greater` into one `descent` flag.
- I wrote no scratch build: I waited until X3's `serie.rs` re-export (`compare_values`, `require_indexable`) landed and the shared tree compiled.

**Smoke** (env `CARGO_TARGET_DIR=/home/user/target-w2 CARGO_INCREMENTAL=0`)

| Command | Result |
| --- | --- |
| `cargo check -p yggdryl --all-targets --keep-going --message-format=short` | lib and `root` test clean; 28 errors left, all in `rust/tests/allocations.rs` and `rust/benchmarks/types/datatype/serie.rs` (old `window_by(x)` spelling) |
| `cargo test -p yggdryl --test root window_serie` | ok, 12 passed |
| `cargo test -p yggdryl --test root chunked` | ok, 48 passed |
| `cargo test -p yggdryl --doc window_by` | ok, 5 passed (includes X3's `SerieReader::window_by`) |
| `--doc window_serie` | ok, 9 passed |
| Widened: `--test serie order` | ok, 41 passed |
| Widened: whole `--test root` | ok, 1293 passed |
| `cargo clippy -p yggdryl --lib --test root --no-deps` | nothing from my files; one warning in X3's `rust/src/serie/arrow.rs:2424` (collapsible `if`). X1a's unused-`descent` warning is gone |

Four mutations made the new tests fail, and the code was restored after each:
- dropping the offset shift;
- keying a gathered window by its position instead of its key row;
- not recording an edge descent;
- merging on `Less` instead of `Equal`.

**Left for X4** (`rust/tests/allocations.rs`, `rust/benchmarks/types/datatype/serie.rs`): every `.window_by(x)` becomes `.window_by(x, false)`, adding `.iter()` before `.map(black_box)`. Where `.count()` reads an owner, use `.len()`.
- `allocations.rs`:
  - 3143, 3144, 3250: `.window_by(&k).expect("windows").count()` becomes `.window_by(&k, false).expect("windows").len()`.
  - 3145, 3157, 3251, 3468: `.window_by(&k, false)`.
  - 3150, 3162, 3174, 3182, 3256: `.window_by(&k, false).expect("windows").iter().map(black_box).count()`.
  - 3450: `chunked.window_by(&venue, false)`.
- The bench file:
  - 586, 595, 606: `.window_by(&k, false).expect("windows").iter().map(black_box).count()`.
  - 616: `.window_by(&venue, false)`.
- The through-window pin should now move from `direct + RECORD_WINDOW_AS_SERIE` to `direct + 1`, because the slice keys through `apply_serie_window`. The chunked `KEYS_BUILT` should drop from 4 + 1 to 4, since the edge builds no key. I did not run `--test allocations`.

No other window_by callers exist yet in the bindings, docs or skills.

## x3:stream

I've implemented X3 in my three files: `SerieReader` static values, `SerieReader::window_by` and `SerieReaderWindows`. My smokes pass: `--test serie arrow` (70 tests), `--doc SerieReader` (4) and the whole `--test serie` (236). The all-targets check still fails, but only in `rust/tests/allocations.rs` and `rust/benchmarks/types/datatype/serie.rs`, which still use the old one-argument `window_by`. Those files belong to X4 and the X1b sweep.

**Files edited**
- `/home/user/yggdryl/rust/src/serie/arrow.rs`
- `/home/user/yggdryl/rust/src/serie.rs`: `pub use arrow::{SerieReader, SerieReaderWindows};`, plus the `pub(crate) use order::{compare_values, require_indexable};` that X1a asked for and X1b's files now import.
- `/home/user/yggdryl/rust/tests/serie/arrow.rs`

All three were formatted with `rustfmt --edition 2024`, and nothing was committed.

**What's implemented**
- **`SerieReader` static values:**
  - Stored as `Statics { field: Arc<Field>, row: Scalar }` in a new `statics` field.
  - `static_values()` and `get_static_value(&FieldPath)` lend the values with no allocation. Paths step through records by exact name only; any other segment answers `None`, and the empty path answers the whole record.
  - `set_static_values`, `with_static_values` and `clear_static_values` are infallible and take a `FieldRecord`.
  - `cast` keeps them. `into_arrow_reader` drops them, as do the held doors (`from_arrow_reader`, `from_serie`, `from_chunked`).
  - `Debug` prints them.
- **`Source::Window(WindowPart)`:**
  - A cast on a window compiles one plan lazily (chained over an earlier one) and applies it to each piece outside the walk's lock.
  - `into_arrow_reader` turns each piece into one batch through `batch_under`, lazily.
- **`SerieReader::window_by(self, by, sorted)`:**
  - Every refusal comes before any pull, in the spec's order: parse, empty or `*`-only key, unnest, binder, key-cell collision, reserved name at the wrong type.
  - It builds the flat static record `{kept statics, key cells, windownum, rownum}`, named as the reader's root and required.
- **`SerieReaderWindows`:** `field()`, `static_field()`, `Iterator`, `FusedIterator` and `Debug`. It is `Send + Sync`, which a test asserts.
- **The walk:** follows the spec's step and pull rules, including the in-order rule and the passed-window refusal. The sorted descent refusal is `InvalidRecord`, wrapped in `Core`, naming the batch (empty batches counted), the row, the stream row and both keys. Every error is answered once by whoever pulled it, then everything fuses and the parent stream is dropped.
- **Tests:** the nine the spec lists for `rust/tests/serie/arrow.rs`, plus `a_pull_that_panics_poisons_the_walk_into_one_internal_error`.
- **Tests seen failing:** two rounds of deliberate changes to the walk (ignoring skipped rows, dropping the edge-descent check, dropping `before` from `rownum`, ignoring the edge, not marking a window passed on error, not dropping the stream on refusal) made the matching tests fail. The code was restored afterwards.

**Deviations**
1. `Walk` has no `statics` field and `Open` has no `index`; `SerieReaderWindows` also holds `schema` so it can build sub-readers.
2. The reserved names are matched with the binder's ASCII fold, so a stated `RowNum: uint64` counts as `rownum`.
3. The descent path is `$[base + before + row]`, the absolute row number the window's `rownum` uses. For a top-level reader that is the plain stream row.
4. Keys in the descent message are printed with the serie's `Display`, cut to 64 bytes with `elide_display`. `Scalar` itself has no `Display`.
5. The passed-window refusal's path is the root's name, since the spec gave none.
6. The next-window search reuses X1b's `crate::window_serie::window_end` rather than repeating it.
7. A poisoned lock is recovered with `into_inner` and `clear_poison`. The first puller to hit it fails the walk, marks the open window passed and gets the internal error; every pull after that ends.

**Smoke results** (with `CARGO_TARGET_DIR=/home/user/target-w2 CARGO_INCREMENTAL=0`)

| Command | Result |
| --- | --- |
| `cargo check -p yggdryl --all-targets --keep-going --message-format=short` | lib and every other target clean. 21 errors in `rust/tests/allocations.rs` and 7 in the types benchmark, all the old `window_by` signature or "`SerieWindows` is not an iterator" |
| `cargo test -p yggdryl --test serie arrow` | ok, 70 passed |
| `cargo test -p yggdryl --test serie` | ok, 236 passed |
| `cargo test -p yggdryl --doc SerieReader` | ok, 4 passed |
| `cargo clippy -p yggdryl --lib --test serie --no-deps -- -D warnings` | clean |
| `RUSTDOCFLAGS="-D warnings" cargo doc -p yggdryl --no-deps` | clean |

Disk is at 6.9 GB free.

**Left for other owners**
- **X4:** add `, false` to the `window_by` calls and `.iter()` to the `SerieWindows` uses in `allocations.rs` and `benchmarks/types/datatype/serie.rs`, then add the stream pins. From the code I expect, per non-empty batch, the key (2) plus the comparator plus the bitmap. Per window: the static row (1) and the state cell (1). Per piece: 0 for a whole batch, else one record slice. Per continuing edge: 0 for inline cells. `static_values` and `get_static_value` should cost 0 on inline cells. None of these are measured yet.
- **X7:** add to `.api-inventory.txt`:
  - `SerieReader`: `window_by`, `static_values`, `get_static_value`, `set_static_values`, `with_static_values`, `clear_static_values`;
  - `SerieReaderWindows`: `field`, `static_field`.
- **Spec owner:** the internal error the spec asks for (`Error::internal`) renders as "... this is a bug in yggdryl", which is misleading when the user's own stream is what panicked. Rewording it means changing `Error::Internal`'s display in `rust/src/arrow/mod.rs`, which is outside my files.

