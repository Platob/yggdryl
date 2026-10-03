//! JavaScript's native view of the shared [`WindowSerie`] and
//! [`WindowSerieMut`]: a window over a serie that reads and writes through it.
//!
//! [`JsWindowSerie`] holds the parent `Serie` object, an offset and a length,
//! and nothing else. Every call takes the core window over the parent as the
//! parent stands at that call - a read through [`Serie::window`], a write
//! through [`Serie::window_mut`] - so the one class is both the shared and the
//! mutable window: a write through it lands in the parent's buffers, a write
//! on the parent is visible through it, and a window the parent no longer
//! reaches is refused by the core at the call that reads it. Every index is
//! window-relative and checked by the core.
//!
//! A window `windowBy` lent also holds where it was lent: the core's own
//! windows of that call - one value every window of the call shares, holding
//! the rows they were cut over (`SerieWindows::into_owned`, buffers shared,
//! no row copied) - and its place among them, the single owner of its
//! record. Its record - `staticValues` - and a `windowBy` of it redirect to
//! the core window the core lends at that place, so a window of a window
//! keeps the outer cells and an absolute `rownum` exactly as the core's
//! does.

use std::sync::Arc;

use napi::bindgen_prelude::{ClassInstance, Either, Env, JavaScriptClassExt, Reference, Result};
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl::{Serie, SerieWindows, WindowSerie, WindowSerieMut};

use crate::datatype::JsDataType;
use crate::expression::{SelectorInput, selector_from_input};
use crate::field::JsField;
use crate::napi_error;
use crate::serie::{
    JsSerie, JsSerieIterator, OrderingsInput, count, groups, orderings_of, position, rows_of,
    sort_options, static_record,
};
use crate::text::codec::{
    JsScalar, checked_depth, value_to_transport, value_to_transport_with_field,
};

/// A window over a serie: `length` rows from `offset`, read and written
/// through the serie at each call, moving nothing.
///
/// Handed out by `serie.window(offset, length)`. The window holds the serie
/// object it was taken from, so it is both the shared and the mutable window:
/// a read borrows the serie for that call, a write borrows it mutably for that
/// call, and a write never grows or shrinks what the window views. Identity
/// is the window's rows alone, as a serie's is its rows.
#[napi(js_name = "WindowSerie")]
pub struct JsWindowSerie {
    /// The serie object the window reads and writes through.
    serie: Reference<JsSerie>,
    offset: usize,
    len: usize,
    /// Where `windowBy` lent this window - the windows of that call, shared
    /// by every window it lent and holding the rows they were cut over, and
    /// its place among them - or `None` for every other window.
    origin: Option<(Arc<SerieWindows<'static>>, usize)>,
}

impl JsWindowSerie {
    /// The window `offset..offset + len` over `serie`, refused by the core
    /// when it reaches past the end.
    pub(crate) fn new(serie: Reference<JsSerie>, offset: usize, len: usize) -> Result<Self> {
        serie.inner.window(offset, len).map_err(napi_error)?;
        Ok(Self {
            serie,
            offset,
            len,
            origin: None,
        })
    }

    /// Every window `windows` lends, each beside its key: over `holder` -
    /// the serie object whose rows `windowed` holds - or, where `sorted`
    /// gathered the rows into a copy the windows own, over one new serie of
    /// that copy, which every window shares. The windows are held once,
    /// owning what they view, and each window lent keeps them and its place
    /// among them, for its record.
    pub(crate) fn lend(
        env: Env,
        holder: &Reference<JsSerie>,
        windowed: &Serie,
        windows: SerieWindows<'_>,
    ) -> Result<Vec<(JsScalar, Self)>> {
        let gathered = !std::ptr::eq(windows.serie(), windowed);
        let windows = Arc::new(windows.into_owned());
        let serie = if gathered {
            JsSerie::from_core(windows.serie().clone()).into_reference(env)?
        } else {
            holder.clone(env)?
        };
        windows
            .iter()
            .enumerate()
            .map(|(index, (key, window))| {
                Ok((
                    JsScalar::from_core(key),
                    Self {
                        serie: serie.clone(env)?,
                        offset: window.offset(),
                        len: window.len(),
                        origin: Some((Arc::clone(&windows), index)),
                    },
                ))
            })
            .collect()
    }

    /// The core window `windowBy` lent this window as - the window at its
    /// place among the windows of that call, stating its record - or `None`
    /// for every other window; reached in constant time
    /// ([`SerieWindows::get`]).
    fn lent(&self) -> Option<WindowSerie<'_>> {
        let (windows, index) = self.origin.as_ref()?;
        windows.get(*index).map(|(_, window)| window)
    }

    /// The core window over the serie as it stands now.
    fn window(&self) -> Result<WindowSerie<'_>> {
        self.serie
            .inner
            .window(self.offset, self.len)
            .map_err(napi_error)
    }

    /// The core mutable window over the serie as it stands now.
    fn window_mut(&mut self) -> Result<WindowSerieMut<'_>> {
        let (offset, len) = (self.offset, self.len);
        self.serie.inner.window_mut(offset, len).map_err(napi_error)
    }
}

#[napi]
impl JsWindowSerie {
    /// The number of rows the window holds.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        count(self.len)
    }

    /// The serie row the window starts at.
    #[napi(getter)]
    pub fn offset(&self) -> f64 {
        count(self.offset)
    }

    /// The whole serie the window reads and writes through: the very object
    /// `window` was called on.
    #[napi(getter)]
    pub fn serie(&self, env: Env) -> Result<Reference<JsSerie>> {
        self.serie.clone(env)
    }

    /// The values constant over this window's rows, where `windowBy` lent
    /// it: one struct value, read with `Scalar`'s own accessors - the cells
    /// of the record the windowed window states but `windownum` and
    /// `rownum`, the key cells as the key's projections name them,
    /// `windownum` (the window's place among the windows, from 0) and
    /// `rownum` (the number its first row has in what was windowed,
    /// absolute through windows of windows, `null` where `sorted` gathered
    /// the rows out of their order) - read off the rows as they stood when
    /// `windowBy` cut them. `null` for every other window: one `window`
    /// takes, and a narrower one. Never the window's identity, and never
    /// carried by `intoSerie` or an Arrow array of it.
    #[napi(getter)]
    pub fn static_values(&self) -> Result<Option<JsScalar>> {
        let record = self.lent().and_then(|window| window.static_values());
        Ok(static_record(record)?.map(JsScalar::from_core))
    }

    /// The field every row is typed by, or `null` for a window over a run.
    #[napi(getter)]
    pub fn field(&self) -> Result<Option<JsField>> {
        Ok(self.window()?.field().cloned().map(JsField::from_core))
    }

    /// The serie's datatype: `serie(<the field named item>)` for a column,
    /// agreed out of the window's rows for a run.
    #[napi(getter)]
    pub fn dtype(&self) -> Result<JsDataType> {
        self.window()?
            .dtype()
            .map(JsDataType::from_core)
            .map_err(napi_error)
    }

    /// Whether the window holds no row.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The window's absent rows, counted off the validity bits with no row
    /// built.
    #[napi]
    pub fn null_count(&self) -> Result<f64> {
        Ok(count(self.window()?.null_count()))
    }

    /// Whether window row `index` is absent.
    #[napi]
    pub fn is_null(&self, index: f64) -> Result<bool> {
        self.window()?
            .is_null(position(index, "index")?)
            .map_err(napi_error)
    }

    /// Window row `index`, built as one value.
    #[napi]
    pub fn scalar(&self, index: f64) -> Result<JsScalar> {
        self.window()?
            .scalar(position(index, "index")?)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Window row `index`, or `null` past the window.
    #[napi]
    pub fn at(&self, index: f64) -> Result<Option<JsScalar>> {
        Ok(self
            .window()?
            .get(position(index, "index")?)
            .map(|row| JsScalar::from_core(row.into_owned())))
    }

    /// Every row of the window, each built once.
    #[napi]
    pub fn rows(&self) -> Result<Vec<JsScalar>> {
        Ok(self
            .window()?
            .rows()
            .into_owned()
            .into_iter()
            .map(JsScalar::from_core)
            .collect())
    }

    /// Every row as the transport `asJs` reads, a record under its field.
    #[napi(js_name = "_asJsNative", skip_typescript)]
    pub fn as_js_native(&self, max_depth: Option<u32>) -> Result<JsonValue> {
        let max_depth = checked_depth(max_depth)?;
        let window = self.window()?;
        let rows = match window.field() {
            Some(field) => window
                .iter()
                .map(|row| value_to_transport_with_field(&row, field, 1, max_depth))
                .collect::<Result<Vec<JsonValue>>>()?,
            None => window
                .iter()
                .map(|row| value_to_transport(&row, 1, max_depth))
                .collect::<Result<Vec<JsonValue>>>()?,
        };
        Ok(JsonValue::Array(rows))
    }

    /// Iterate the window's rows, each built once.
    #[napi(js_name = "_iterNative", skip_typescript)]
    pub fn iter_native(&self) -> Result<JsSerieIterator> {
        Ok(JsSerieIterator::from_rows(
            self.window()?.rows().into_owned(),
        ))
    }

    /// The bytes the window's rows occupy, as its own slice counts them.
    #[napi]
    pub fn memory_size(&self) -> Result<f64> {
        Ok(count(self.window()?.memory_size()))
    }

    /// The bytes the window's rows occupy in memory, read through the serie
    /// it views: a window is never spilled on its own - spill the serie.
    #[napi]
    pub fn resident_size(&self) -> Result<f64> {
        Ok(count(self.window()?.resident_size()))
    }

    /// Whether the window's rows lie in a spill file: the serie's own
    /// answer over the rows it views.
    #[napi]
    pub fn is_spilled(&self) -> Result<bool> {
        Ok(self.window()?.is_spilled())
    }

    /// Whether the window's rows are in sorted order under the options.
    #[napi(js_name = "_isSortedNative", skip_typescript)]
    pub fn is_sorted_native(
        &self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<bool> {
        Ok(self
            .window()?
            .is_sorted(sort_options(descending, nulls_first)))
    }

    /// Whether no two window rows hold one value; two absent rows are a
    /// repeat.
    #[napi]
    pub fn is_unique(&self) -> Result<bool> {
        Ok(self.window()?.is_unique())
    }

    /// How many distinct values the window's rows hold, an absent row one of
    /// them.
    #[napi]
    pub fn unique_count(&self) -> Result<f64> {
        Ok(count(self.window()?.unique_count()))
    }

    /// The window-relative row positions in sorted order, as a `uint32`
    /// column named `index`.
    #[napi(js_name = "_sortIndicesNative", skip_typescript)]
    pub fn sort_indices_native(
        &self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<JsSerie> {
        self.window()?
            .sort_indices(sort_options(descending, nulls_first))
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The window-relative row positions in the order the `order by` keys
    /// of `by` state, as a `uint32` column named `index`: the keys computed
    /// over the window's rows alone.
    #[napi(js_name = "_sortIndicesByNative", skip_typescript)]
    pub fn sort_indices_by_native(&self, by: OrderingsInput<'_>) -> Result<JsSerie> {
        let by = orderings_of(&by)?;
        self.window()?
            .sort_indices_by(by)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The window's rows in the order the `order by` keys of `by` state, as
    /// a new serie declaring them.
    #[napi(js_name = "_intoSortByNative", skip_typescript)]
    pub fn into_sort_by_native(&self, by: OrderingsInput<'_>) -> Result<JsSerie> {
        let by = orderings_of(&by)?;
        self.window()?
            .into_sort_by(by)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// A narrower window, window-relative, over the same serie.
    #[napi(js_name = "_windowNative", skip_typescript)]
    pub fn window_native(&self, env: Env, offset: f64, length: f64) -> Result<Self> {
        let (offset, len) = {
            let narrower = self
                .window()?
                .window(position(offset, "offset")?, position(length, "length")?)
                .map_err(napi_error)?;
            (narrower.offset(), narrower.len())
        };
        Ok(Self {
            serie: self.serie.clone(env)?,
            offset,
            len,
            origin: None,
        })
    }

    /// The windows `by` cuts the window's rows into, as the serie's own
    /// `windowBy` cuts them: each over the serie at its own offset, or over
    /// one new serie of the rows `sorted` gathered, at offset 0. A window
    /// `windowBy` lent is windowed as the core window it is - the rows as
    /// they stood when it was lent, its record's cells kept and its `rownum`
    /// the base; any other as the rows the serie holds now.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(
        &self,
        env: Env,
        by: SelectorInput<'_>,
        sorted: Option<bool>,
    ) -> Result<Vec<(JsScalar, Self)>> {
        let by = selector_from_input(by)?;
        let sorted = sorted.unwrap_or(false);
        let window = match self.lent() {
            Some(window) => window,
            None => self.window()?,
        };
        let windows = window.window_by(by, sorted).map_err(napi_error)?;
        Self::lend(env, &self.serie, window.serie(), windows)
    }

    /// The window's rows as a serie: `slice`, sharing a column's buffers.
    #[napi(js_name = "_intoSerieNative", skip_typescript)]
    pub fn into_serie_native(&self) -> Result<JsSerie> {
        Ok(JsSerie::from_core(self.window()?.into_serie()))
    }

    /// The window's rows in sorted order, as a new serie.
    #[napi(js_name = "_intoSortedNative", skip_typescript)]
    pub fn into_sorted_native(
        &self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<JsSerie> {
        self.window()?
            .into_sorted(sort_options(descending, nulls_first))
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The first occurrence of every value in the window, as a new serie.
    #[napi(js_name = "_intoUniqueNative", skip_typescript)]
    pub fn into_unique_native(&self) -> Result<JsSerie> {
        self.window()?
            .into_unique()
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The window's rows in reverse order, as a new serie.
    #[napi(js_name = "_intoReversedNative", skip_typescript)]
    pub fn into_reversed_native(&self) -> Result<JsSerie> {
        Ok(JsSerie::from_core(self.window()?.into_reversed()))
    }

    /// The window rows an integer serie of window-relative positions names.
    #[napi(js_name = "_intoTakenNative", skip_typescript)]
    pub fn into_taken_native(&self, indices: &JsSerie) -> Result<JsSerie> {
        self.window()?
            .into_taken(&indices.inner)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The window rows a boolean serie as long as the window keeps.
    #[napi(js_name = "_intoFilteredNative", skip_typescript)]
    pub fn into_filtered_native(&self, mask: &JsSerie) -> Result<JsSerie> {
        self.window()?
            .into_filtered(&mask.inner)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The window's rows grouped by a key serie as long as the window.
    #[napi(js_name = "_partitionByNative", skip_typescript)]
    pub fn partition_by_native(&self, keys: &JsSerie) -> Result<Vec<(JsScalar, JsSerie)>> {
        self.window()?
            .partition_by(&keys.inner)
            .map(groups)
            .map_err(napi_error)
    }

    /// Whether the window's rows equal another window's, or a serie's.
    #[napi(js_name = "_equalsNative", skip_typescript)]
    pub fn equals_native(
        &self,
        other: Either<ClassInstance<'_, JsWindowSerie>, ClassInstance<'_, JsSerie>>,
    ) -> Result<bool> {
        let window = self.window()?;
        Ok(match other {
            Either::A(slice) => window == slice.window()?,
            Either::B(serie) => window == serie.inner,
        })
    }

    /// The window's rows, rendered behind the serie's name.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> Result<String> {
        Ok(self.window()?.to_string())
    }

    // ------------------------------------------------------------------
    // Writes: each borrows the serie mutably for the call and goes through
    // `Serie::splice` or `Serie::set` on the rebased range.
    // ------------------------------------------------------------------

    /// Overwrite window row `index` with an already-converted value.
    #[napi(js_name = "_setNative", skip_typescript)]
    pub fn set_native(&mut self, index: f64, value: &JsScalar) -> Result<()> {
        let index = position(index, "index")?;
        self.window_mut()?
            .set(index, value.inner.clone())
            .map_err(napi_error)
    }

    /// Overwrite every window row with an already-converted value.
    #[napi(js_name = "_fillNative", skip_typescript)]
    pub fn fill_native(&mut self, value: &JsScalar) -> Result<()> {
        self.window_mut()?
            .fill(value.inner.clone())
            .map_err(napi_error)
    }

    /// Swap window rows `left` and `right`.
    #[napi]
    pub fn swap(&mut self, left: f64, right: f64) -> Result<()> {
        let (left, right) = (position(left, "left")?, position(right, "right")?);
        self.window_mut()?.swap(left, right).map_err(napi_error)
    }

    /// Overwrite the window, row for row, with another window's rows or a
    /// whole serie's.
    #[napi(js_name = "_copyFromNative", skip_typescript)]
    pub fn copy_from_native(
        &self,
        env: Env,
        other: Either<ClassInstance<'_, JsWindowSerie>, ClassInstance<'_, JsSerie>>,
    ) -> Result<()> {
        // The source is held apart before the serie is borrowed mutably, so
        // a window copied from the serie it views reads the rows as they
        // were: the shared buffers are copied once by the write.
        let (source, offset, len): (Serie, usize, usize) = match other {
            Either::A(slice) => (slice.serie.inner.clone(), slice.offset, slice.len),
            Either::B(serie) => {
                let source = serie.inner.clone();
                let len = source.len();
                (source, 0, len)
            }
        };
        let source = source.window(offset, len).map_err(napi_error)?;
        let mut serie = self.serie.clone(env)?;
        serie
            .inner
            .window_mut(self.offset, self.len)
            .map_err(napi_error)?
            .copy_from(&source)
            .map_err(napi_error)
    }

    /// Replace window rows `start..end` by as many already-converted rows.
    #[napi(js_name = "_spliceNative", skip_typescript)]
    pub fn splice_native(
        &mut self,
        start: f64,
        end: f64,
        rows: Vec<ClassInstance<'_, JsScalar>>,
    ) -> Result<()> {
        let range = position(start, "start")?..position(end, "end")?;
        self.window_mut()?
            .splice(range, rows_of(&rows))
            .map_err(napi_error)
    }

    /// Sort the window's rows in place under the options.
    #[napi(js_name = "_asSortedNative", skip_typescript)]
    pub fn as_sorted_native(
        &mut self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<()> {
        self.window_mut()?
            .as_sorted(sort_options(descending, nulls_first))
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Sort the window's rows in place in the order the `order by` keys of
    /// `by` state, written back over the window's own range: every row
    /// outside it untouched, and a refusal leaves the serie as it was.
    #[napi(js_name = "_asSortByNative", skip_typescript)]
    pub fn as_sort_by_native(&mut self, by: OrderingsInput<'_>) -> Result<()> {
        let by = orderings_of(&by)?;
        self.window_mut()?
            .as_sort_by(by)
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Reverse the window's rows in place.
    #[napi(js_name = "_asReversedNative", skip_typescript)]
    pub fn as_reversed_native(&mut self) -> Result<()> {
        self.window_mut()?
            .as_reversed()
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Rearrange the window's rows as an integer serie of window-relative
    /// positions, exactly as many as the window holds, names them.
    #[napi(js_name = "_asTakenNative", skip_typescript)]
    pub fn as_taken_native(&mut self, indices: &JsSerie) -> Result<()> {
        let indices = indices.inner.clone();
        self.window_mut()?
            .as_taken(&indices)
            .map(|_| ())
            .map_err(napi_error)
    }
}
