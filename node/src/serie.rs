//! JavaScript's native view of the shared [`Serie`]: many values, as a
//! schema-free run or as the Arrow buffers of one field.
//!
//! [`JsSerie`] owns only the core value and redirects every verb to it. The
//! verbs one nested leaf lends - a list's offsets and rows, a map's entries,
//! a record's names - are private natives here; `binding.js` publishes them
//! on `ListSerie`, `LargeListSerie`, `ListViewSerie`, `LargeListViewSerie`,
//! `FixedSizeListSerie`, `MapSerie` and `StructSerie`, the subclasses every
//! serie is handed out as by the leaf `_leafNative` names, so nesting reads
//! typed all the way down. Arrow crosses as copied IPC, as it does for every
//! other value of this binding.
//!
//! [`JsSerieReader`] is the stream beside it: one record serie per batch of a
//! native `BatchReader`, every batch cast by the one plan the core compiled
//! when the reader was built.

use std::borrow::Cow;
use std::ops::Range;

use arrow_array::RecordBatch;
use napi::bindgen_prelude::{Buffer, ClassInstance, Either, Generator, Result, Uint8Array};
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl::{
    ArrowCastOptions, Field as CoreField, FieldPath, Scalar, Serie, SerieReader, SerieValue,
};

use crate::datatype::JsDataType;
use crate::field::JsField;
use crate::iomedia::JsBatchReader;
use crate::napi_error;
use crate::text::codec::{
    JsScalar, arrow_array_ipc, arrow_batches, checked_depth, ensure_one_column, value_to_transport,
    value_to_transport_with_field,
};

/// The invariant `binding.js` keeps: a leaf verb is published only on the
/// class `_leafNative` names, so the leaf is the one it asks for.
const LEAF: &str = "a nested Serie verb is reached only through the class of its leaf";

/// Many values: a schema-free run, or the Arrow buffers of one field.
///
/// A column holds no JavaScript value: a row is built when one is asked
/// for. Writes land in the buffers the column holds, and two series are
/// equal when their rows are.
#[napi(js_name = "Serie")]
#[derive(Clone)]
pub struct JsSerie {
    pub(crate) inner: Serie,
}

/// An owning iterator over a serie's rows, each built once.
#[napi(iterator, js_name = "SerieIterator")]
pub struct JsSerieIterator {
    inner: std::vec::IntoIter<Scalar>,
}

impl Generator for JsSerieIterator {
    type Yield = JsScalar;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        self.inner.next().map(JsScalar::from_core)
    }
}

impl JsSerie {
    /// Wrap one native serie for JavaScript.
    pub(crate) const fn from_core(inner: Serie) -> Self {
        Self { inner }
    }
}

/// Read a JavaScript index as a row position.
fn position(index: f64, name: &str) -> Result<usize> {
    let index = crate::exact_u64(index, name)?;
    usize::try_from(index)
        .map_err(|_| napi_error(format!("{name} {index} exceeds this platform's range")))
}

/// The core values of already-converted rows.
fn rows_of(rows: &[ClassInstance<'_, JsScalar>]) -> Vec<Scalar> {
    rows.iter().map(|row| row.inner.clone()).collect()
}

/// A range as the `[start, end]` pair JavaScript reads.
fn pair(range: Option<Range<usize>>) -> Option<Vec<f64>> {
    #[allow(clippy::cast_precision_loss)]
    range.map(|range| vec![range.start as f64, range.end as f64])
}

/// An offsets or sizes buffer as the numbers JavaScript reads.
fn numbers<T: Copy + Into<i64>>(values: &[T]) -> Vec<f64> {
    #[allow(clippy::cast_precision_loss)]
    values.iter().map(|value| (*value).into() as f64).collect()
}

/// The column a one-column Arrow IPC stream holds, through the core array
/// door: of its own layout under the `item` field the core names an array
/// by, or cast into `field` under `options`.
///
/// The stream lands once as the record of its batches - its own schema, so
/// nothing is cast there - and its one child's buffers take the array door
/// once: one plan however many batches the vector crossed as. The IPC
/// column's own name and nullability are the bridge's, never the caller's.
pub(crate) fn column_from_ipc(
    bytes: &[u8],
    label: &str,
    field: Option<&CoreField>,
    options: ArrowCastOptions,
) -> Result<Serie> {
    let (schema, batches) = arrow_batches(bytes)?;
    ensure_one_column(&schema, label)?;
    let reader = yggdryl::arrow::batch_reader(schema, batches);
    let records =
        Serie::from_arrow_reader(None, reader, ArrowCastOptions::new()).map_err(napi_error)?;
    let column = records
        .as_struct()
        .and_then(|records| records.child_at(0))
        .ok_or_else(|| napi_error(format!("{label} IPC has no value column")))?;
    let array = column.require_arrow_array().map_err(napi_error)?;
    Serie::from_arrow_array(field, array, options).map_err(napi_error)
}

/// The record column an Arrow IPC stream's batches hold: of its own schema,
/// or cast into `root` under `options`.
pub(crate) fn records_from_ipc(
    bytes: &[u8],
    root: Option<&CoreField>,
    options: ArrowCastOptions,
) -> Result<Serie> {
    let (schema, batches) = arrow_batches(bytes)?;
    let reader = yggdryl::arrow::batch_reader(schema, batches);
    Serie::from_arrow_reader(root, reader, options).map_err(napi_error)
}

#[napi]
impl JsSerie {
    /// A schema-free run of already-converted rows; `Serie.fromScalars` and
    /// the Arrow doors build a column.
    #[napi(constructor)]
    pub fn new(rows: Option<Vec<ClassInstance<'_, JsScalar>>>) -> Self {
        Self::from_core(Serie::new(rows.as_deref().map(rows_of).unwrap_or_default()))
    }

    /// The column `field` types already-converted `rows` into.
    #[napi(factory, js_name = "_fromScalarsNative", skip_typescript)]
    pub fn from_scalars_native(
        field: ClassInstance<'_, JsField>,
        rows: Vec<ClassInstance<'_, JsScalar>>,
    ) -> Result<Self> {
        Serie::from_scalars(field.inner.clone(), rows_of(&rows))
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The empty column of `field`.
    #[napi(factory, js_name = "_emptyNative", skip_typescript)]
    pub fn empty(field: ClassInstance<'_, JsField>) -> Result<Self> {
        Serie::empty(field.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The empty column of `field`, with room for `rows` rows.
    #[napi(factory, js_name = "_withCapacityNative", skip_typescript)]
    pub fn with_capacity(field: ClassInstance<'_, JsField>, rows: f64) -> Result<Self> {
        Serie::with_capacity(field.inner.clone(), position(rows, "rows")?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// `rows` copies of `field`'s canonical default.
    #[napi(factory, js_name = "_fromDefaultNative", skip_typescript)]
    pub fn from_default_native(field: ClassInstance<'_, JsField>, rows: f64) -> Result<Self> {
        Serie::from_default(field.inner.clone(), position(rows, "rows")?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Decode one Arrow JS vector from its one-column IPC bridge, cast into
    /// `field` when one is given.
    #[napi(factory, js_name = "_fromArrowArrayIpcNative", skip_typescript)]
    pub fn from_arrow_array_ipc_native(
        bytes: Uint8Array,
        field: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, nullability.as_deref(), representation.as_deref())?;
        column_from_ipc(
            &bytes,
            "Serie",
            field.as_ref().map(|field| &field.inner),
            options,
        )
        .map(Self::from_core)
    }

    /// Decode one Arrow JS record batch or table as a record column, cast
    /// into `root` when one is given.
    #[napi(factory, js_name = "_fromArrowBatchIpcNative", skip_typescript)]
    pub fn from_arrow_batch_ipc_native(
        bytes: Uint8Array,
        root: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, nullability.as_deref(), representation.as_deref())?;
        records_from_ipc(&bytes, root.as_ref().map(|root| &root.inner), options)
            .map(Self::from_core)
    }

    /// Drain a native `BatchReader` into one record column: of its own
    /// schema, named `row`, or cast into `root`.
    #[napi(factory, js_name = "_fromArrowReaderNative", skip_typescript)]
    pub fn from_arrow_reader(
        mut reader: ClassInstance<'_, JsBatchReader>,
        root: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, nullability.as_deref(), representation.as_deref())?;
        Serie::from_arrow_reader(
            root.as_ref().map(|root| &root.inner),
            reader.take()?,
            options,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// The field a column carries, or `null` for a run.
    #[napi(getter)]
    pub fn field(&self) -> Option<JsField> {
        self.inner.field().cloned().map(JsField::from_core)
    }

    /// `list(<the field named item>)` for a column; agreed out of a run's rows.
    #[napi(getter)]
    pub fn dtype(&self) -> Result<JsDataType> {
        self.inner
            .dtype()
            .map(JsDataType::from_core)
            .map_err(napi_error)
    }

    /// Whether this is a column rather than a schema-free run.
    #[napi(getter)]
    pub fn is_column(&self) -> bool {
        self.inner.is_column()
    }

    /// The row count.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let length = self.inner.len() as f64;
        length
    }

    /// Which nested leaf this is - `list`, `largeList`, `listView`,
    /// `largeListView`, `fixedSizeList`, `map`, `struct` - or `null`.
    #[napi(getter, js_name = "_leafNative", skip_typescript)]
    pub fn leaf_native(&self) -> Option<&'static str> {
        match &self.inner {
            Serie::List(_) => Some("list"),
            Serie::LargeList(_) => Some("largeList"),
            Serie::ListView(_) => Some("listView"),
            Serie::LargeListView(_) => Some("largeListView"),
            Serie::FixedSizeList(_) => Some("fixedSizeList"),
            Serie::Map(_) | Serie::SortedMap(_) => Some("map"),
            Serie::Struct(_) => Some("struct"),
            _ => None,
        }
    }

    /// The rows that are absent.
    #[napi]
    pub fn null_count(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let count = self.inner.null_count() as f64;
        count
    }

    /// Whether the serie holds no row.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Whether row `index` is absent.
    #[napi]
    pub fn is_null(&self, index: f64) -> Result<bool> {
        self.inner
            .is_null(position(index, "index")?)
            .map_err(napi_error)
    }

    /// Row `index`, built as one value.
    #[napi]
    pub fn scalar(&self, index: f64) -> Result<JsScalar> {
        self.inner
            .scalar(position(index, "index")?)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Row `index`, or `null` past the end.
    #[napi]
    pub fn at(&self, index: f64) -> Result<Option<JsScalar>> {
        Ok(self
            .inner
            .get(position(index, "index")?)
            .map(|row| JsScalar::from_core(row.into_owned())))
    }

    /// Every row, each built once and kept by the array alone.
    #[napi]
    pub fn rows(&self) -> Vec<JsScalar> {
        self.inner
            .rows()
            .into_owned()
            .into_iter()
            .map(JsScalar::from_core)
            .collect()
    }

    /// Every row as the transport `asJs` reads, a record under its field.
    #[napi(js_name = "_asJsNative", skip_typescript)]
    pub fn as_js_native(&self, max_depth: Option<u32>) -> Result<JsonValue> {
        let max_depth = checked_depth(max_depth)?;
        let rows = match self.inner.field() {
            Some(field) => self
                .inner
                .iter()
                .map(|row| value_to_transport_with_field(&row, field, 1, max_depth))
                .collect::<Result<Vec<JsonValue>>>()?,
            None => self
                .inner
                .iter()
                .map(|row| value_to_transport(&row, 1, max_depth))
                .collect::<Result<Vec<JsonValue>>>()?,
        };
        Ok(JsonValue::Array(rows))
    }

    /// Iterate the rows, each built once.
    #[napi(js_name = "_iterNative", skip_typescript)]
    pub fn iter_native(&self) -> JsSerieIterator {
        JsSerieIterator {
            inner: self
                .inner
                .iter()
                .map(Cow::into_owned)
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }

    /// This serie as the one value a `Scalar` sequence holds.
    #[napi]
    pub fn into_scalar(&self) -> JsScalar {
        JsScalar::from_core(Scalar::from(self.inner.clone()))
    }

    /// The run of this serie's rows, dropping the field.
    #[napi(js_name = "_intoRunNative", skip_typescript)]
    pub fn into_run(&self) -> Self {
        Self::from_core(Serie::from(self.inner.clone().into_run()))
    }

    /// `length` rows from `offset`, sharing a column's buffers.
    #[napi(js_name = "_sliceNative", skip_typescript)]
    pub fn slice(&self, offset: f64, length: f64) -> Result<Self> {
        self.inner
            .slice(position(offset, "offset")?, position(length, "length")?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// A record column's child named `name`, or `null`.
    #[napi(js_name = "_childNative", skip_typescript)]
    pub fn child(&self, name: String) -> Option<JsSerie> {
        self.inner.child(&name).cloned().map(Self::from_core)
    }

    /// A record column's child, or a union's member, at `index`.
    #[napi(js_name = "_childAtNative", skip_typescript)]
    pub fn child_at(&self, index: f64) -> Result<Option<JsSerie>> {
        Ok(self
            .inner
            .child_at(position(index, "index")?)
            .cloned()
            .map(Self::from_core))
    }

    /// Every child of a record column, or member of a union.
    #[napi(js_name = "_childrenNative", skip_typescript)]
    pub fn children(&self) -> Vec<JsSerie> {
        self.inner
            .children()
            .iter()
            .cloned()
            .map(Self::from_core)
            .collect()
    }

    /// A sequence column's items, a mapping's entries, an encoding's values.
    #[napi(js_name = "_itemsNative", skip_typescript)]
    pub fn items(&self) -> Option<JsSerie> {
        self.inner.items().cloned().map(Self::from_core)
    }

    /// The column `path` reaches, spelled as a field path.
    #[napi(js_name = "_getChildByPathNative", skip_typescript)]
    pub fn get_child_by_path(&self, path: String) -> Result<Option<JsSerie>> {
        let path = FieldPath::from_str(&path).map_err(napi_error)?;
        Ok(self
            .inner
            .get_child_by_path(&path)
            .cloned()
            .map(Self::from_core))
    }

    /// Replace rows `start..end` by already-converted `rows`: the one mutation.
    #[napi(js_name = "_spliceNative", skip_typescript)]
    pub fn splice_native(
        &mut self,
        start: f64,
        end: f64,
        rows: Vec<ClassInstance<'_, JsScalar>>,
    ) -> Result<()> {
        let range = position(start, "start")?..position(end, "end")?;
        self.inner.splice(range, rows_of(&rows)).map_err(napi_error)
    }

    /// Overwrite row `index` with an already-converted value.
    #[napi(js_name = "_setNative", skip_typescript)]
    pub fn set_native(&mut self, index: f64, value: &JsScalar) -> Result<()> {
        self.inner
            .set(position(index, "index")?, value.inner.clone())
            .map_err(napi_error)
    }

    /// Append one already-converted row.
    #[napi(js_name = "_pushNative", skip_typescript)]
    pub fn push_native(&mut self, value: &JsScalar) -> Result<()> {
        self.inner.push(value.inner.clone()).map_err(napi_error)
    }

    /// Insert one already-converted row before `index`.
    #[napi(js_name = "_insertNative", skip_typescript)]
    pub fn insert_native(&mut self, index: f64, value: &JsScalar) -> Result<()> {
        self.inner
            .insert(position(index, "index")?, value.inner.clone())
            .map_err(napi_error)
    }

    /// Remove row `index` and answer it.
    #[napi]
    pub fn remove(&mut self, index: f64) -> Result<JsScalar> {
        self.inner
            .remove(position(index, "index")?)
            .map(JsScalar::from_core)
            .map_err(napi_error)
    }

    /// Remove the last row and answer it, or `null` when empty.
    #[napi]
    pub fn pop(&mut self) -> Result<Option<JsScalar>> {
        self.inner
            .pop()
            .map(|row| row.map(JsScalar::from_core))
            .map_err(napi_error)
    }

    /// Drop every row from `length` on.
    #[napi]
    pub fn truncate(&mut self, length: f64) -> Result<()> {
        self.inner
            .truncate(position(length, "length")?)
            .map_err(napi_error)
    }

    /// Drop every row and keep the field.
    #[napi]
    pub fn clear(&mut self) -> Result<()> {
        self.inner.clear().map_err(napi_error)
    }

    /// Append already-converted `rows`.
    #[napi(js_name = "_extendNative", skip_typescript)]
    pub fn extend_native(&mut self, rows: Vec<ClassInstance<'_, JsScalar>>) -> Result<()> {
        self.inner.extend(rows_of(&rows)).map_err(napi_error)
    }

    /// Append every row of `other`, buffer to buffer where the fields agree.
    #[napi]
    pub fn extend_from_serie(&mut self, other: &JsSerie) -> Result<()> {
        self.inner
            .extend_from_serie(&other.inner)
            .map_err(napi_error)
    }

    /// Truncate to `length`, or grow with clones of an already-converted value.
    #[napi(js_name = "_resizeNative", skip_typescript)]
    pub fn resize_native(&mut self, length: f64, value: &JsScalar) -> Result<()> {
        self.inner
            .resize(position(length, "length")?, value.inner.clone())
            .map_err(napi_error)
    }

    /// Replace a record column's child of `child`'s name, or add it.
    #[napi]
    pub fn set_child(&mut self, child: &JsSerie) -> Result<()> {
        self.inner
            .set_child(child.inner.clone())
            .map_err(napi_error)
    }

    /// Write one cell of row `index`, `path` deep, in place.
    #[napi(js_name = "_setCellNative", skip_typescript)]
    pub fn set_cell_native(&mut self, path: String, index: f64, value: &JsScalar) -> Result<()> {
        let path = FieldPath::from_str(&path).map_err(napi_error)?;
        self.inner
            .set_cell(&path, position(index, "index")?, value.inner.clone())
            .map_err(napi_error)
    }

    /// This column under `target` - a `Field`, or a `DataType` as its required
    /// `value` field - cast once; a run has no layout to cast.
    #[napi(js_name = "_castNative", skip_typescript)]
    pub fn cast_native(
        &self,
        target: Either<ClassInstance<'_, JsField>, ClassInstance<'_, JsDataType>>,
        safe: Option<bool>,
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, nullability.as_deref(), representation.as_deref())?;
        let target = match target {
            Either::A(field) => field.inner.clone(),
            Either::B(dtype) => dtype.inner.clone().required_field("value"),
        };
        self.inner
            .cast(&target, options)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Encode this one-row column as the one-row Arrow IPC a scalar crosses
    /// as; any other length, and a run, is refused.
    #[napi(js_name = "_intoArrowScalarIpcNative", skip_typescript)]
    pub fn into_arrow_scalar_ipc_native(&self) -> Result<Buffer> {
        let field = self.inner.require_field().map_err(napi_error)?;
        let scalar = self.inner.into_arrow_scalar().map_err(napi_error)?;
        arrow_array_ipc(field, scalar.into_inner())
    }

    /// Encode this column as a one-column Arrow IPC array; a run has none.
    #[napi(js_name = "_intoArrowArrayIpcNative", skip_typescript)]
    pub fn into_arrow_array_ipc_native(&self) -> Result<Buffer> {
        let field = self.inner.require_field().map_err(napi_error)?;
        let array = self.inner.require_arrow_array().map_err(napi_error)?;
        arrow_array_ipc(field, array)
    }

    /// Encode this record column as one Arrow IPC record batch.
    #[napi(js_name = "_intoArrowBatchIpcNative", skip_typescript)]
    pub fn into_arrow_batch_ipc_native(&self) -> Result<Buffer> {
        let batch: RecordBatch = self.inner.into_arrow_batch().map_err(napi_error)?;
        let reader = yggdryl::arrow::batch_reader(batch.schema(), [batch]);
        let mut reader = JsBatchReader::from_core(reader, "row");
        reader.into_ipc()
    }

    /// This record column as a native `BatchReader` of one batch.
    #[napi]
    pub fn into_arrow_reader(&self) -> Result<JsBatchReader> {
        let reader = self.inner.into_arrow_reader().map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, "row"))
    }

    /// Whether two series hold equal rows, whichever leaf holds them.
    #[napi]
    pub fn equals(&self, other: &JsSerie) -> bool {
        self.inner == other.inner
    }

    /// Order two series by their rows, as the core defines it.
    #[napi]
    pub fn compare(&self, other: &JsSerie) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// A copy sharing the buffers; writing either copies them once.
    #[napi(js_name = "_cloneNative", skip_typescript)]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// The rows, rendered behind the field's name.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    // ------------------------------------------------------------------
    // The verbs one nested leaf lends, published on its subclass.
    // ------------------------------------------------------------------

    /// A list, list-view or map leaf's offsets.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_offsetsNative", skip_typescript)]
    pub fn offsets_native(&self) -> Vec<f64> {
        let serie = &self.inner;
        if let Some(leaf) = serie.as_list() {
            return numbers(leaf.offsets());
        }
        if let Some(leaf) = serie.as_large_list() {
            return numbers(leaf.offsets());
        }
        if let Some(leaf) = serie.as_list_view() {
            return numbers(leaf.offsets());
        }
        if let Some(leaf) = serie.as_large_list_view() {
            return numbers(leaf.offsets());
        }
        numbers(serie.as_map().expect(LEAF).offsets())
    }

    /// A list-view leaf's sizes.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_sizesNative", skip_typescript)]
    pub fn sizes_native(&self) -> Vec<f64> {
        match self.inner.as_list_view() {
            Some(leaf) => numbers(leaf.sizes()),
            None => numbers(self.inner.as_large_list_view().expect(LEAF).sizes()),
        }
    }

    /// A fixed-size list leaf's width.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_widthNative", skip_typescript)]
    pub fn width_native(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let width = self.inner.as_fixed_size_list().expect(LEAF).width() as f64;
        width
    }

    /// The items or entries row `index` holds, or `null` past the end.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_rangeNative", skip_typescript)]
    pub fn range_native(&self, index: f64) -> Result<Option<Vec<f64>>> {
        let index = position(index, "index")?;
        let serie = &self.inner;
        let range = if let Some(leaf) = serie.as_list() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_large_list() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_list_view() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_large_list_view() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_fixed_size_list() {
            leaf.range(index)
        } else {
            serie.as_map().expect(LEAF).range(index)
        };
        Ok(pair(range))
    }

    /// The items or entries column cut to row `index`, or `null` past the end.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_rowNative", skip_typescript)]
    pub fn row_native(&self, index: f64) -> Result<Option<JsSerie>> {
        let index = position(index, "index")?;
        let serie = &self.inner;
        let row = if let Some(leaf) = serie.as_list() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_large_list() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_list_view() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_large_list_view() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_fixed_size_list() {
            leaf.row(index)
        } else {
            serie.as_map().expect(LEAF).row(index)
        };
        Ok(row.map(Self::from_core))
    }

    /// A map leaf's entries: one record column of the entries field.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_entriesNative", skip_typescript)]
    pub fn entries_native(&self) -> Self {
        Self::from_core(self.inner.as_map().expect(LEAF).entries().clone())
    }

    /// A map leaf's key column.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_keysNative", skip_typescript)]
    pub fn keys_native(&self) -> Self {
        Self::from_core(self.inner.as_map().expect(LEAF).keys().clone())
    }

    /// A map leaf's value column.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_valuesNative", skip_typescript)]
    pub fn values_native(&self) -> Self {
        Self::from_core(self.inner.as_map().expect(LEAF).values().clone())
    }

    /// Whether a map leaf's field declares every row's keys sorted.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_keysSortedNative", skip_typescript)]
    pub fn keys_sorted_native(&self) -> bool {
        self.inner.as_map().expect(LEAF).keys_sorted()
    }

    /// A record leaf's child names, in order.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_namesNative", skip_typescript)]
    pub fn names_native(&self) -> Vec<String> {
        self.inner
            .as_struct()
            .expect(LEAF)
            .children()
            .iter()
            .filter_map(|child| child.field().map(|field| field.name().to_owned()))
            .collect()
    }

    /// A record leaf without the child named `name`, sharing every other.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_withoutChildNative", skip_typescript)]
    pub fn without_child_native(&self, name: String) -> Result<Self> {
        self.inner
            .as_struct()
            .expect(LEAF)
            .without_child(&name)
            .map(|leaf| Self::from_core(leaf.into_serie()))
            .map_err(napi_error)
    }
}

/// One record serie per batch of a native `BatchReader`, each cast by the
/// one plan the core compiled from the stream's schema.
///
/// The reader is a stream, read once: iterating it and `intoArrowReader`
/// both consume it, and a batch's failure surfaces at the pull that read it.
#[napi(js_name = "SerieReader")]
pub struct JsSerieReader {
    /// The undrained core reader, taken by whatever consumes it.
    inner: Option<SerieReader>,
    /// The record every yielded serie is typed by, kept after the reader
    /// is taken.
    root: CoreField,
    /// Whether `intoArrowReader` took the reader rather than draining it here.
    taken: bool,
}

/// The refusal a second consumer of one stream reads.
fn serie_reader_consumed() -> napi::Error {
    napi_error("this SerieReader has already been consumed; a stream is read once")
}

#[napi]
impl JsSerieReader {
    /// Read `reader`'s batches as record series: of its own schema, named
    /// `row`, or cast into `root`. The reader is consumed.
    #[napi(factory, js_name = "_fromArrowReaderNative", skip_typescript)]
    pub fn from_arrow_reader(
        mut reader: ClassInstance<'_, JsBatchReader>,
        root: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, nullability.as_deref(), representation.as_deref())?;
        let inner = SerieReader::from_arrow_reader(
            root.as_ref().map(|root| &root.inner),
            reader.take()?,
            options,
        )
        .map_err(napi_error)?;
        Ok(Self {
            root: inner.field().clone(),
            inner: Some(inner),
            taken: false,
        })
    }

    /// The record every yielded serie is typed by.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.root.clone())
    }

    /// Pull the next batch as its record serie, or `null` at the end.
    ///
    /// The native half of the iteration protocol; the loader wraps it so
    /// `for...of` yields each serie as its leaf's class.
    #[napi(js_name = "_nextNative", skip_typescript)]
    pub fn next_native(&mut self) -> Result<Option<JsSerie>> {
        if self.taken {
            return Err(serie_reader_consumed());
        }
        let Some(reader) = self.inner.as_mut() else {
            return Ok(None);
        };
        if let Some(landed) = reader.next() {
            return landed
                .map(|serie| Some(JsSerie::from_core(serie)))
                .map_err(napi_error);
        }
        self.inner = None;
        Ok(None)
    }

    /// The stream's batches reconciled to the root as a native
    /// `BatchReader`, never landed; the reader is consumed.
    #[napi]
    pub fn into_arrow_reader(&mut self) -> Result<JsBatchReader> {
        let reader = self.inner.take().ok_or_else(serie_reader_consumed)?;
        self.taken = true;
        Ok(JsBatchReader::from_core(
            reader.into_arrow_reader(),
            self.root.name(),
        ))
    }
}
