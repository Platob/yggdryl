# Static values on windows only

This spec supersedes `STATIC_VALUES_SPEC.md`, which is withdrawn: the user asked to "Remove the generic static values, set it only for window, and it should be accessible on generic serie scalar accessors methods". It is layered on `WINDOW_BY_SPEC.md` and `WINDOW_BY_EXTENSION_SPEC.md` (as implemented in commits 3851a88, 373144a and fb510e8), with `SerieSlice` renamed to `WindowSerie` (8973ff9).

## Contract

- **Only windows state static values.** Two kinds of window do:
  - a held `WindowSerie` that `window_by` yielded (`SerieWindows::iter()`);
  - a stream window, the sub-reader `SerieReaderWindows` yields.

  Nothing else states a record: not a `Serie`, a `ChunkedSerie`, a reader that is not a window, a window from `Serie::window(offset, len)`, a narrower window, or a window turned back into a serie.
- **The record** is one required struct, named as the windowed carrier's root (`SerieReader::root_of(field).name()`). Its children, in order:
  1. **kept:** the static values of the window it was cut from, if that is itself a window, minus `windownum` and `rownum`. So the outer key cells come first.
  2. **the key cells,** named as `Projection::name` names them, in selector order. They are declared nullable where the windowed field is nullable.
  3. **`windownum`:** `uint64`, required. The window's place among the windows `window_by` yielded, from 0.
  4. **`rownum`:** `uint64`, nullable. The number this window's first row has in what it was cut from: absolute through windows of windows, null when a sorted gather reordered the rows.
- **One accessor, the same on both kinds:** `static_values(&self) -> Option<FieldScalar<'_>>`. It answers the record as a struct value under its field, and every cell is reached through the generic accessors `FieldScalar` already has (`get`, `get_key_str`, `as_struct`, `into_scalar`, ...).
- **No dedicated path getter, no trait, no `Statics` type, no public setter.** Delete `SerieReader::get_static_value`. Make the in-flight `set_static_values`, `with_static_values` and `clear_static_values` crate-private (`pub(crate)`), because only windowing (and later the media-part seam) states a record. Delete the extension's refusal 6, about a reader stating `windownum` or `rownum` at another datatype: it cannot be reached once no caller states a record. A window field's `rownum` comes from the crate alone.
- **Never identity:** equality, order, hash, `Display` and digests read the rows only.
- **Never on the wire:** `into_serie`, `into_arrow_*`, the C stream export and `IOMedia::write_arrow` drop the record.
- **`ChunkedSerie::window_by`** keeps answering `Vec<(Scalar, ChunkedSerie)>` with no record. The key is the first half of the item, and the position is its place in the `Vec`. Say so in its rustdoc and in the docs.

## Held windows: a lazy record, `WindowSerie` stays `Copy`

```rust
#[derive(Clone, Copy)]
pub struct WindowSerie<'a> {
    serie: &'a Serie,
    offset: usize,
    len: usize,
    /// Set only on a window `SerieWindows::iter` yielded: where its record comes from.
    origin: Option<Origin<'a>>,
}
#[derive(Clone, Copy)]
struct Origin<'a> {
    windows: &'a SerieWindows<'a>,
    index: u32,   // windownum
    key_row: u32, // the key row this window is keyed by
}
```

- **`SerieWindows`** builds three things once, in `new`, at constant cost:
  - `record: Arc<Field>`, the record field;
  - `kept: Box<[Scalar]>`, the kept cells of the windowed carrier if it is a window, else empty;
  - `base: Option<u64>`: the windowed window's `rownum` (`None` when null), else 0.
- **Reading the record.** `WindowSerie::static_values` builds the row on demand: kept, then the key cells at `key_row`, then `windownum`, then `rownum`. That is one `Run` (one allocation), plus an `Arc<str>` per text cell past the inline capacity.
  - `rownum` is `base + (offset − windows.offset)` under `Cuts::Starts`, and null under `Cuts::Gathered`.
  - Iterating windows therefore costs nothing extra: the existing per-window key run is unchanged.
- **What drops the origin:** `window()` narrowing, `into_serie`, and a `WindowSerie` built by `Serie::window`. Pin it.
- **`window_by` on a held window that has an origin** keeps its record cells as `kept` and its `rownum` as `base`. This mirrors what the stream does for a window of a window.
- **Sizes.** `WindowSerie` grows from 24 to 40 B (`Option<Origin>` uses the reference niche). It stays `Copy`. Pin the size with a `const` assert.
- **`WindowSerieMut::window_by`** yields the same windows as the shared form, and `WindowSerieMut` itself states no record.

## Stream windows

- **Unchanged:** the sub-reader's static values are the window record built at open time (`Walk::step`), and `SerieReader::static_values` answers it.
- **One change to the field:** `rownum` becomes nullable, so both kinds of window have one shape. A stream never gathers, so its `rownum` is never null.
- `Walk.base` keeps reading a window reader's own `rownum` for a window of a window. Its checked arithmetic stays.

## Correspondence pin

For `s = false`, and for `s = true` over keys already in order, `rows.window_by(by, s)?.iter()` and `SerieReader::from_serie(rows)?.window_by(by, s)` state the same record field and the same record values, window by window. This includes a window of a window: windowing the first stream window again, and windowing the first held window again.

For `s = true` over keys out of order, the held windows state `rownum` as null.

## Rust API changes

| Item | Change |
| --- | --- |
| `WindowSerie::static_values(&self) -> Option<FieldScalar<'a>>` | New, with a rustdoc example that reads `get_key_str("venue")` |
| `SerieWindows::static_field(&self) -> &Field` | New: the record field every window states, before any is walked |
| `SerieReader::static_values` | Kept; its rustdoc example states the record through `window_by` |
| `SerieReader::get_static_value` | Deleted |
| `SerieReader::{set_, with_, clear_}static_values` | Crate-private |
| The extension's refusal 6 | Deleted, with its test |

`SerieReaderWindows::static_field` stays.

## Tests (mirror files; refusals first)

- **`rust/tests/root/window_serie.rs`:**
  - `a_window_by_window_states_its_key_windownum_and_rownum`;
  - `a_gathered_window_states_a_null_rownum`;
  - `a_plain_or_narrowed_window_states_none`;
  - `a_window_of_a_window_keeps_the_outer_cells_and_an_absolute_rownum`;
  - `serie_windows_names_its_record_field_before_any_window`;
  - `the_record_is_read_through_the_generic_scalar_accessors`;
  - the size pin.
- **`rust/tests/serie/arrow.rs`:**
  - the correspondence pin (held against stream);
  - every test that stated a record through the public setters is rewritten to reach the record through `window_by`;
  - the overflow test goes through `internals`, or is dropped if no caller can reach it. Keep the checked arithmetic either way.
- **`rust/tests/allocations.rs`:**
  - `a_held_window_record_costs_one_row_only_when_read`: drain cost unchanged; one row per `static_values()` call;
  - the stream pins stand.

## Bindings

Both bindings carry the base and extension work as well.

- **Python:**
  - `window_by(by, sorted=False)` on `Serie`, `WindowSerie` and `ChunkedSerie`. `sorted=None` clears to `False`, and `...` (omitted) means `False`.
  - `SerieReader.window_by(by, sorted=False) -> SerieReaderWindows`, an iterator of `SerieReader` sub-readers; every pull runs in `py.detach`.
  - `WindowSerie.static_values` and `SerieReader.static_values` are getters returning `Scalar | None`: the struct value, read with `Scalar`'s own generic accessors.
  - `_native.pyi`, `typing_bindings.py`, `mypy --strict`, parity tests and boundary benchmarks.
- **Node:**
  - `windowBy(by, sorted)` on `Serie`, `WindowSerie` and `ChunkedSerie`. `undefined` is skipped and `null` means `false`.
  - `SerieReader.windowBy(by, sorted) -> SerieReaderWindows`, iterable of sub-readers.
  - `staticValues: Scalar | null` on `WindowSerie` and `SerieReader`.
  - Regenerate `index.{js,d.ts}`, and add `*.types.ts`, tests and benchmarks.
- `.api-bindings.txt`.

## Docs

`docs/types/serie.md` ("Windows by key"), `docs/types/window-serie.md`, `docs/types/chunked-serie.md` and `docs/arrow/readers.md` ("Windows of a stream": lazy sub-readers, the in-order rule, sorted verification, window records) all need updates.
- Every example uses Rust, Python and JavaScript tabs and asserts its result.
- The pending items in `DOCS_PENDING.md` beside this file are in scope.
- Also `skills/yggdryl-arrow/**`, `AGENTS.md` (Layout rows for `window_serie.rs`, `serie/arrow.rs` and `chunked_serie.rs`; the Serie-is-the-collection table; Zero copy), `.api-inventory.txt`, and rustdoc on every new public item.
- Delete `.handoff/` in the last commit.
