//! JavaScript's native view of the shared [`Serie`]: many values, as a
//! schema-free run or as the Arrow buffers of one field.
//!
//! [`JsSerie`] owns only the core value and redirects every verb to it. The
//! verbs one nested leaf lends - a serie's offsets and rows, a map's entries,
//! a record's names - are private natives here; `binding.js` publishes them
//! on `SerieSerie`, `LargeSerieSerie`, `SerieViewSerie`, `LargeSerieViewSerie`,
//! `FixedSizeSerieSerie`, `MapSerie` and `StructSerie`, the subclasses every
//! serie is handed out as by the leaf `_leafNative` names, so nesting reads
//! typed all the way down. Arrow crosses as copied IPC, as it does for every
//! other value of this binding.
//!
//! [`JsSerieReader`] is the stream beside it: one record serie per batch of a
//! native `BatchReader`, every batch cast by the one plan the core compiled
//! when the reader was built, the one record serie a held column is, or one
//! per chunk of a held chunked column - and, cast into another root, the
//! reader the core hands back with every record cast by one plan more.
//! [`JsSerieReaderWindows`] is the core's walk over a stream's windows, one
//! lazy `SerieReader` per window.

use std::borrow::Cow;
use std::io::Cursor;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, RecordBatchOptions};
use arrow_ipc::reader::StreamReader;
use arrow_schema::{Schema, SchemaRef};
use napi::bindgen_prelude::{
    Buffer, ClassInstance, Either, Either3, Env, Generator, Reference, Result, Uint8Array,
};
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl::expression::{IntoOrderings as _, Ordering};
use yggdryl::{
    ArrowCastOptions, Field as CoreField, FieldPath, FieldScalar, JoinSource, Scalar, Serie,
    SerieReader, SerieReaderWindows, SerieValue, SortOptions,
};

use crate::chunked_serie::JsChunkedSerie;
use crate::datatype::JsDataType;
use crate::expression::{JsSelector, SelectorInput, selector_from_input};
use crate::field::JsField;
use crate::iomedia::{JsBatchReader, encoded};
use crate::join::{JoinOptionsInput, join_kind, join_options};
use crate::napi_error;
use crate::spill::{JsSpillOptions, spill_bound};
use crate::text::codec::{
    JsScalar, checked_depth, value_to_transport, value_to_transport_with_field,
};
use crate::text::line::JsFieldPath;
use crate::window_serie::JsWindowSerie;

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

impl JsSerieIterator {
    /// Iterate rows already built, in order.
    pub(crate) fn from_rows(rows: Vec<Scalar>) -> Self {
        Self {
            inner: rows.into_iter(),
        }
    }
}

impl JsSerie {
    /// Wrap one native serie for JavaScript.
    pub(crate) const fn from_core(inner: Serie) -> Self {
        Self { inner }
    }
}

/// Read a JavaScript index as a row position.
pub(crate) fn position(index: f64, name: &str) -> Result<usize> {
    let index = crate::exact_u64(index, name)?;
    usize::try_from(index)
        .map_err(|_| napi_error(format!("{name} {index} exceeds this platform's range")))
}

/// The core values of already-converted rows.
pub(crate) fn rows_of(rows: &[ClassInstance<'_, JsScalar>]) -> Vec<Scalar> {
    rows.iter().map(|row| row.inner.clone()).collect()
}

/// The direction and nulls placement JavaScript states as two optional
/// booleans, as the one native value: an absent or `null` answer is the
/// default, ascending with nulls last.
pub(crate) const fn sort_options(
    descending: Option<bool>,
    nulls_first: Option<bool>,
) -> SortOptions {
    let options = match descending {
        Some(true) => SortOptions::descending(),
        _ => SortOptions::ascending(),
    };
    options.with_nulls_first(matches!(nulls_first, Some(true)))
}

/// The `order by` keys a sort verb reads: a `Selector`, every projection
/// ascending with nulls last, or the one `Scalar` the loader converted the
/// caller's keys into - the clause's text, a list of key texts, or a list of
/// `{ term, descending, nulls_first }` records.
pub(crate) type OrderingsInput<'a> =
    Either<ClassInstance<'a, JsSelector>, ClassInstance<'a, JsScalar>>;

/// The keys `by` stands for, read once by the core's own rule
/// ([`yggdryl::expression::IntoOrderings`]) before any row is.
pub(crate) fn orderings_of(by: &OrderingsInput<'_>) -> Result<Vec<Ordering>> {
    match by {
        Either::A(selector) => (&selector.inner).into_orderings(),
        Either::B(keys) => (&keys.inner).into_orderings(),
    }
    .map_err(napi_error)
}

/// The keys a record declares its rows keep, each as the `order by` key
/// grammar spells it - `venue`, `price desc nulls first` - or `None` where
/// it declares none.
pub(crate) fn spelled_orderings(keys: Option<Vec<Ordering>>) -> Option<Vec<String>> {
    keys.map(|keys| keys.iter().map(ToString::to_string).collect())
}

/// A row or item count as the number JavaScript reads.
pub(crate) fn count(value: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let value = value as f64;
    value
}

/// The groups a partition answers, each its key and its rows.
pub(crate) fn groups(groups: Vec<(Scalar, Serie)>) -> Vec<(JsScalar, JsSerie)> {
    groups
        .into_iter()
        .map(|(key, rows)| (JsScalar::from_core(key), JsSerie::from_core(rows)))
        .collect()
}

/// The record a window states, as the struct value JavaScript reads with
/// `Scalar`'s own accessors - `get('venue')`, `path('.rownum')`: each cell
/// under the name the record's field gives it. `None` for a serie or reader
/// that is no window.
pub(crate) fn static_record(record: Option<FieldScalar<'_>>) -> Result<Option<Scalar>> {
    let Some(record) = record else {
        return Ok(None);
    };
    let cells = record
        .value()
        .sequence_rows()
        .ok_or_else(|| napi_error("a window's static values are one record row"))?;
    Scalar::from_struct(
        record
            .field()
            .fields()
            .iter()
            .map(CoreField::name)
            .zip(cells.iter().cloned()),
    )
    .map(Some)
    .map_err(napi_error)
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

/// The schema and batches of one Arrow IPC stream; an empty stream names no
/// schema and is refused.
///
/// The stream is drained here, so the decoder reads the caller's bytes in
/// place: each message body is copied once into the batch that owns it, and
/// the payload is never copied whole first.
pub(crate) fn arrow_batches(bytes: &[u8]) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    if bytes.is_empty() {
        return Err(napi_error("Arrow IPC input is empty and has no schema"));
    }
    let mut reader = StreamReader::try_new(Cursor::new(bytes), None).map_err(napi_error)?;
    let schema = reader.schema();
    let batches = reader
        .by_ref()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(napi_error)?;
    Ok((schema, batches))
}

/// One column under `field` as the one-column Arrow IPC stream Arrow JS
/// reads a vector from.
fn arrow_array_ipc(field: &CoreField, array: ArrayRef) -> Result<Buffer> {
    arrow_arrays_ipc(field, vec![array])
}

/// Columns under `field` as the one-column Arrow IPC stream Arrow JS reads
/// one vector from, one batch - one `Data` of the vector - per column.
pub(crate) fn arrow_arrays_ipc(field: &CoreField, arrays: Vec<ArrayRef>) -> Result<Buffer> {
    let schema = Arc::new(Schema::new([field
        .clone()
        .into_arrow_field_ref()
        .map_err(napi_error)?]));
    let batches = arrays
        .into_iter()
        .map(|array| {
            let options = RecordBatchOptions::new().with_row_count(Some(array.len()));
            RecordBatch::try_new_with_options(Arc::clone(&schema), vec![array], &options)
                .map_err(napi_error)
        })
        .collect::<Result<Vec<RecordBatch>>>()?;
    encoded(&schema, &batches)
}

/// The column a one-column Arrow IPC stream holds, through the core array
/// door: of its own layout under the `item` field the core names an array
/// by, or cast into `field` under `options`.
///
/// The stream lands once as the record of its batches - its own schema, so
/// nothing is cast there - and its one child's buffers take the array door
/// once: one plan however many batches the vector crossed as. The IPC
/// column's own name and nullability are the bridge's, never the caller's.
fn column_from_ipc(
    bytes: &[u8],
    field: Option<&CoreField>,
    options: ArrowCastOptions,
) -> Result<Serie> {
    let (schema, batches) = arrow_batches(bytes)?;
    if schema.fields().len() != 1 {
        return Err(napi_error(format!(
            "Serie IPC must contain exactly one column, got {}",
            schema.fields().len()
        )));
    }
    let reader = yggdryl::arrow::batch_reader(schema, batches);
    let records =
        Serie::from_arrow_reader(None, reader, ArrowCastOptions::new()).map_err(napi_error)?;
    let column = records
        .as_struct()
        .and_then(|records| records.child_at(0))
        .ok_or_else(|| napi_error("Serie IPC has no value column"))?;
    let array = column.require_arrow_array().map_err(napi_error)?;
    Serie::from_arrow_array(field, array, options).map_err(napi_error)
}

/// The record column an Arrow IPC stream's batches hold: of its own schema,
/// or cast into `root` under `options`.
fn records_from_ipc(
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
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, representation.as_deref())?;
        column_from_ipc(&bytes, field.as_ref().map(|field| &field.inner), options)
            .map(Self::from_core)
    }

    /// Decode one Arrow JS record batch or table as a record column, cast
    /// into `root` when one is given.
    #[napi(factory, js_name = "_fromArrowBatchIpcNative", skip_typescript)]
    pub fn from_arrow_batch_ipc_native(
        bytes: Uint8Array,
        root: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, representation.as_deref())?;
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
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, representation.as_deref())?;
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

    /// `serie(<the field named item>)` for a column; agreed out of a run's rows.
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

    /// Which nested leaf this is - `serie`, `largeSerie`, `serieView`,
    /// `largeSerieView`, `fixedSizeSerie`, `map`, `struct` - or `null`.
    #[napi(getter, js_name = "_leafNative", skip_typescript)]
    pub fn leaf_native(&self) -> Option<&'static str> {
        match &self.inner {
            Serie::Serie(_) => Some("serie"),
            Serie::LargeSerie(_) => Some("largeSerie"),
            Serie::SerieView(_) => Some("serieView"),
            Serie::LargeSerieView(_) => Some("largeSerieView"),
            Serie::FixedSizeSerie(_) => Some("fixedSizeSerie"),
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
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, representation.as_deref())?;
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
        let root = SerieReader::root_of(self.inner.require_field().map_err(napi_error)?)
            .map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, root.name()))
    }

    /// Whether the rows equal another serie's, or a chunked serie's,
    /// whichever leaf or cut holds them.
    #[napi(js_name = "_equalsNative", skip_typescript)]
    pub fn equals_native(
        &self,
        other: Either<ClassInstance<'_, JsSerie>, ClassInstance<'_, JsChunkedSerie>>,
    ) -> bool {
        match other {
            Either::A(serie) => self.inner == serie.inner,
            Either::B(chunked) => self.inner == chunked.inner,
        }
    }

    /// Order the rows against another serie's, or a chunked serie's, as the
    /// core defines it.
    #[napi(js_name = "_compareNative", skip_typescript)]
    pub fn compare_native(
        &self,
        other: Either<ClassInstance<'_, JsSerie>, ClassInstance<'_, JsChunkedSerie>>,
    ) -> Result<i32> {
        let ordering = match other {
            Either::A(serie) => Some(self.inner.cmp(&serie.inner)),
            Either::B(chunked) => self.inner.partial_cmp(&chunked.inner),
        };
        ordering
            .map(crate::ordering_value)
            .ok_or_else(|| napi_error("the rows of a serie and a chunked serie have no order"))
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
    // Ordering, uniqueness and grouping: the reads answer a new serie and
    // leave this one as it was, the `_as*Native` writes bring it into the
    // state in place. Each is one core verb; the loader validates the
    // options, coerces indices, masks and keys, and hands every answered
    // serie out as its leaf's class.
    // ------------------------------------------------------------------

    /// The row positions in sorted order, as a `uint32` column named `index`.
    #[napi(js_name = "_sortIndicesNative", skip_typescript)]
    pub fn sort_indices_native(
        &self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<Self> {
        self.inner
            .sort_indices(sort_options(descending, nulls_first))
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Whether the rows are in sorted order under the options.
    #[napi(js_name = "_isSortedNative", skip_typescript)]
    pub fn is_sorted_native(&self, descending: Option<bool>, nulls_first: Option<bool>) -> bool {
        self.inner.is_sorted(sort_options(descending, nulls_first))
    }

    /// Whether no two rows hold one value; two absent rows are a repeat.
    ///
    /// One hash set over the rows, stopping at the first repeat.
    #[napi]
    pub fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    /// How many distinct values the rows hold, an absent row one of them.
    #[napi]
    pub fn unique_count(&self) -> f64 {
        count(self.inner.unique_count())
    }

    /// The bytes the rows occupy: a column's buffers as its own slice counts
    /// them, a run's values as the row estimator charges them.
    #[napi]
    pub fn memory_size(&self) -> f64 {
        count(self.inner.memory_size())
    }

    /// The bytes the rows occupy in memory: `memorySize` less what lies in a
    /// spill file's mapping, read off the buffers.
    #[napi]
    pub fn resident_size(&self) -> f64 {
        count(self.inner.resident_size())
    }

    /// Whether the rows lie in a spill file: some bytes, none of them
    /// resident. A run and an empty column are never spilled.
    #[napi]
    pub fn is_spilled(&self) -> bool {
        self.inner.is_spilled()
    }

    /// Move the rows to disk until the resident bytes are under the bound
    /// `options` states - the process default (`SpillOptions.fromEnv()`)
    /// where it is `undefined` or `null` - the heaviest leaves first, each
    /// written once to a private file and mapped back read-only, so every
    /// later read reaches the mapping and a write copies the buffer it
    /// touches back once. A run spills nothing; a refused folder is named
    /// and leaves the serie as it was.
    #[napi]
    pub fn spill(&mut self, options: Option<ClassInstance<'_, JsSpillOptions>>) -> Result<()> {
        let bound = spill_bound(options.as_deref())?;
        self.inner.spill(bound).map_err(napi_error)
    }

    /// The `order by` keys this record's root declares its rows keep, most
    /// significant first, each as the key grammar spells it (`price desc`):
    /// `SORT:by` on the root, a proven order the sorts write and the writes
    /// that break it clear. `null` for a run, a column that is no record,
    /// and a root declaring none.
    #[napi]
    pub fn declared_order(&self) -> Result<Option<Vec<String>>> {
        self.inner
            .declared_order()
            .map(spelled_orderings)
            .map_err(napi_error)
    }

    /// The rows in sorted order, this serie untouched.
    #[napi(js_name = "_intoSortedNative", skip_typescript)]
    pub fn into_sorted_native(
        &self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<Self> {
        self.inner
            .into_sorted(sort_options(descending, nulls_first))
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The row positions in the order the `order by` keys of `by` state, as
    /// a `uint32` column named `index`: stable, a record's terms bound once
    /// against its root and a plain column keyed as itself.
    #[napi(js_name = "_sortIndicesByNative", skip_typescript)]
    pub fn sort_indices_by_native(&self, by: OrderingsInput<'_>) -> Result<Self> {
        self.inner
            .sort_indices_by(orderings_of(&by)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The rows in the order the `order by` keys of `by` state, this serie
    /// untouched; the root declares the keys it was sorted by.
    #[napi(js_name = "_intoSortByNative", skip_typescript)]
    pub fn into_sort_by_native(&self, by: OrderingsInput<'_>) -> Result<Self> {
        self.inner
            .into_sort_by(orderings_of(&by)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// This serie joined with `other` on the keys `by` holds, under the kind
    /// `how` names - `inner` where it names none - and `options`: one record
    /// column of the left columns, then the right.
    #[napi(js_name = "_joinWithNative", skip_typescript)]
    pub fn join_with_native(
        &self,
        other: &JsSerie,
        by: &JsScalar,
        how: Option<String>,
        options: Option<JoinOptionsInput<'_>>,
    ) -> Result<Self> {
        let how = join_kind(how)?;
        let options = join_options(options)?;
        self.inner
            .join_with(&other.inner, &by.inner, how, &options)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The first occurrence of every value, in order of first occurrence.
    #[napi(js_name = "_intoUniqueNative", skip_typescript)]
    pub fn into_unique_native(&self) -> Result<Self> {
        self.inner
            .into_unique()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The rows in reverse order, this serie untouched.
    #[napi(js_name = "_intoReversedNative", skip_typescript)]
    pub fn into_reversed_native(&self) -> Self {
        Self::from_core(self.inner.into_reversed())
    }

    /// The rows an integer serie of positions names, in that order.
    #[napi(js_name = "_intoTakenNative", skip_typescript)]
    pub fn into_taken_native(&self, indices: &JsSerie) -> Result<Self> {
        self.inner
            .into_taken(&indices.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The rows a boolean serie of the same length keeps.
    #[napi(js_name = "_intoFilteredNative", skip_typescript)]
    pub fn into_filtered_native(&self, mask: &JsSerie) -> Result<Self> {
        self.inner
            .into_filtered(&mask.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The rows grouped by a key serie of the same length, one `[key, rows]`
    /// pair per distinct key in order of first occurrence.
    #[napi(js_name = "_partitionByNative", skip_typescript)]
    pub fn partition_by_native(&self, keys: &JsSerie) -> Result<Vec<(JsScalar, Self)>> {
        self.inner
            .partition_by(&keys.inner)
            .map(groups)
            .map_err(napi_error)
    }

    /// The windows `by` cuts the rows into: one `[key, window]` pair per run
    /// of equal adjacent keys, in row order - or, `sorted`, each key once in
    /// key order - every window over this very serie object, or, where
    /// `sorted` gathered the rows into key order, over one new serie of the
    /// gathered copy they all share. Each window states its record as its
    /// `staticValues`, read off the rows as they stood at this call: the
    /// windows hold them - buffers shared, no row copied - so a write on this
    /// serie while they are alive copies the written leaf away from them
    /// once. `sorted` absent or `null` is `false`.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(
        &self,
        env: Env,
        reference: Reference<JsSerie>,
        by: SelectorInput<'_>,
        sorted: Option<bool>,
    ) -> Result<Vec<(JsScalar, JsWindowSerie)>> {
        let windows = self
            .inner
            .window_by(selector_from_input(by)?, sorted.unwrap_or(false))
            .map_err(napi_error)?;
        JsWindowSerie::lend(env, &reference, &self.inner, windows)
    }

    /// A record column's rows grouped by the cells the field paths reach,
    /// keyed by the run of those cells; a path is a `FieldPath` or its text.
    #[napi(js_name = "_partitionByPathsNative", skip_typescript)]
    pub fn partition_by_paths_native(
        &self,
        paths: Vec<Either<String, ClassInstance<'_, JsFieldPath>>>,
    ) -> Result<Vec<(JsScalar, Self)>> {
        let paths = paths
            .iter()
            .map(|path| match path {
                Either::A(text) => FieldPath::from_str(text).map_err(napi_error),
                Either::B(path) => Ok(path.inner.clone()),
            })
            .collect::<Result<Vec<FieldPath>>>()?;
        self.inner
            .partition_by_paths(&paths)
            .map(groups)
            .map_err(napi_error)
    }

    /// Sort the rows in place under the options.
    #[napi(js_name = "_asSortedNative", skip_typescript)]
    pub fn as_sorted_native(
        &mut self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<()> {
        self.inner
            .as_sorted(sort_options(descending, nulls_first))
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Keep the first occurrence of every value, in place.
    #[napi(js_name = "_asUniqueNative", skip_typescript)]
    pub fn as_unique_native(&mut self) -> Result<()> {
        self.inner.as_unique().map(|_| ()).map_err(napi_error)
    }

    /// Reverse the rows in place.
    #[napi(js_name = "_asReversedNative", skip_typescript)]
    pub fn as_reversed_native(&mut self) -> Result<()> {
        self.inner.as_reversed().map(|_| ()).map_err(napi_error)
    }

    /// Sort the rows in place in the order the `order by` keys of `by`
    /// state; a refusal leaves the serie as it was.
    #[napi(js_name = "_asSortByNative", skip_typescript)]
    pub fn as_sort_by_native(&mut self, by: OrderingsInput<'_>) -> Result<()> {
        let by = orderings_of(&by)?;
        self.inner.as_sort_by(by).map(|_| ()).map_err(napi_error)
    }

    /// Keep the rows an integer serie of positions names, in place.
    #[napi(js_name = "_asTakenNative", skip_typescript)]
    pub fn as_taken_native(&mut self, indices: &JsSerie) -> Result<()> {
        let indices = indices.inner.clone();
        self.inner
            .as_taken(&indices)
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Keep the rows a boolean serie of the same length keeps, in place.
    #[napi(js_name = "_asFilteredNative", skip_typescript)]
    pub fn as_filtered_native(&mut self, mask: &JsSerie) -> Result<()> {
        let mask = mask.inner.clone();
        self.inner
            .as_filtered(&mask)
            .map(|_| ())
            .map_err(napi_error)
    }

    /// The window `offset..offset + length` over this serie, read and
    /// written through it at each call.
    #[napi(js_name = "_windowNative", skip_typescript)]
    pub fn window_native(
        &self,
        reference: Reference<JsSerie>,
        offset: f64,
        length: f64,
    ) -> Result<JsWindowSerie> {
        JsWindowSerie::new(
            reference,
            position(offset, "offset")?,
            position(length, "length")?,
        )
    }

    // ------------------------------------------------------------------
    // The verbs one nested leaf lends, published on its subclass.
    // ------------------------------------------------------------------

    /// A serie, serie-view or map leaf's offsets.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_offsetsNative", skip_typescript)]
    pub fn offsets_native(&self) -> Vec<f64> {
        let serie = &self.inner;
        if let Some(leaf) = serie.as_serie() {
            return numbers(leaf.offsets());
        }
        if let Some(leaf) = serie.as_large_serie() {
            return numbers(leaf.offsets());
        }
        if let Some(leaf) = serie.as_serie_view() {
            return numbers(leaf.offsets());
        }
        if let Some(leaf) = serie.as_large_serie_view() {
            return numbers(leaf.offsets());
        }
        numbers(serie.as_map().expect(LEAF).offsets())
    }

    /// A serie-view leaf's sizes.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_sizesNative", skip_typescript)]
    pub fn sizes_native(&self) -> Vec<f64> {
        match self.inner.as_serie_view() {
            Some(leaf) => numbers(leaf.sizes()),
            None => numbers(self.inner.as_large_serie_view().expect(LEAF).sizes()),
        }
    }

    /// A fixed-size serie leaf's width.
    ///
    /// # Panics
    ///
    /// Never: the binding hands this method only to the leaf class it
    /// reads.
    #[napi(js_name = "_widthNative", skip_typescript)]
    pub fn width_native(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let width = self.inner.as_fixed_size_serie().expect(LEAF).width() as f64;
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
        let range = if let Some(leaf) = serie.as_serie() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_large_serie() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_serie_view() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_large_serie_view() {
            leaf.range(index)
        } else if let Some(leaf) = serie.as_fixed_size_serie() {
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
        let row = if let Some(leaf) = serie.as_serie() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_large_serie() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_serie_view() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_large_serie_view() {
            leaf.row(index)
        } else if let Some(leaf) = serie.as_fixed_size_serie() {
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
/// one plan the core compiled from the stream's schema, or the one record
/// serie a held column is.
///
/// The reader is a stream, read once: iterating it, `cast` and
/// `intoArrowReader` each consume it, and a batch's failure surfaces at the
/// pull that read it.
#[napi(js_name = "SerieReader")]
pub struct JsSerieReader {
    /// The undrained core reader, taken by whatever consumes it.
    inner: Option<SerieReader>,
    /// The record every yielded serie is typed by, kept after the reader
    /// is taken.
    root: CoreField,
    /// The struct value of the record a window states, kept after the
    /// reader is taken; `None` for a reader that is no window.
    statics: Option<Scalar>,
    /// Whether `intoArrowReader` took the reader rather than draining it here.
    taken: bool,
}

/// The refusal a second consumer of one stream reads.
fn serie_reader_consumed() -> napi::Error {
    napi_error("this SerieReader has already been consumed; a stream is read once")
}

impl JsSerieReader {
    /// Wrap one undrained core reader, keeping the root it names and the
    /// record it states.
    fn from_core(inner: SerieReader) -> Result<Self> {
        Ok(Self {
            root: inner.field().clone(),
            statics: static_record(inner.static_values())?,
            inner: Some(inner),
            taken: false,
        })
    }
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
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, representation.as_deref())?;
        let inner = SerieReader::from_arrow_reader(
            root.as_ref().map(|root| &root.inner),
            reader.take()?,
            options,
        )
        .map_err(napi_error)?;
        Self::from_core(inner)
    }

    /// Read one held column as a stream of one record serie: a record column
    /// as it stands, any other column as the one child of a record named
    /// `row`. Nothing is cast or copied; a run, and a record column holding
    /// an absent row, are refused.
    #[napi(factory, js_name = "_fromSerieNative", skip_typescript)]
    pub fn from_serie(serie: &JsSerie) -> Result<Self> {
        SerieReader::from_serie(serie.inner.clone())
            .map_err(napi_error)
            .and_then(Self::from_core)
    }

    /// Read a held chunked column as the stream of its chunks, one record
    /// serie per chunk: a record's chunks as they stand, any other field's
    /// each the one child of a record named `row`. Nothing is cast or
    /// copied; a record chunk holding an absent row is refused.
    #[napi(factory, js_name = "_fromChunkedNative", skip_typescript)]
    pub fn from_chunked(chunked: &JsChunkedSerie) -> Result<Self> {
        SerieReader::from_chunked(chunked.inner.clone())
            .map_err(napi_error)
            .and_then(Self::from_core)
    }

    /// The record every yielded serie is typed by.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.root.clone())
    }

    /// The values constant over every row this reader yields, where it is a
    /// window `windowBy` cut: one struct value, read with `Scalar`'s own
    /// accessors - the cells of the record the windowed reader states but
    /// `windownum` and `rownum`, the key cells, `windownum` (the window's
    /// place, from 0) and `rownum` (the number its first row has in the
    /// stream). `null` for every other reader. Kept once the reader is
    /// consumed; never a column, and dropped at the Arrow face.
    #[napi(getter)]
    pub fn static_values(&self) -> Option<JsScalar> {
        self.statics.clone().map(JsScalar::from_core)
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

    /// Every record this reader yields cast into `target` - a `Field`, or a
    /// `DataType` as its required `value` field - by one plan: a record
    /// root, or a column as the one child of a record named `row`. A target
    /// that is this reader's own root answers the reader as it stands; held
    /// records are cast here, once each, and a stream's batches as they are
    /// pulled. The reader is consumed.
    #[napi(js_name = "_castNative", skip_typescript)]
    pub fn cast_native(
        &mut self,
        target: Either<ClassInstance<'_, JsField>, ClassInstance<'_, JsDataType>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = crate::cast_options(safe, representation.as_deref())?;
        let target = match target {
            Either::A(field) => field.inner.clone(),
            Either::B(dtype) => dtype.inner.clone().required_field("value"),
        };
        let reader = self.inner.take().ok_or_else(serie_reader_consumed)?;
        self.taken = true;
        reader
            .cast(&target, options)
            .map_err(napi_error)
            .and_then(Self::from_core)
    }

    /// Cut the stream into windows of equal adjacent keys, one lazy reader
    /// each, in the order they arrive: `by` bound against the root before
    /// any batch is pulled, `sorted` verifying that the keys arrive in key
    /// order. The reader is consumed, a refused key included.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(
        &mut self,
        by: SelectorInput<'_>,
        sorted: Option<bool>,
    ) -> Result<JsSerieReaderWindows> {
        let by = selector_from_input(by)?;
        let reader = self.inner.take().ok_or_else(serie_reader_consumed)?;
        self.taken = true;
        reader
            .window_by(by, sorted.unwrap_or(false))
            .map(|inner| JsSerieReaderWindows { inner })
            .map_err(napi_error)
    }

    /// The bytes the records this reader holds occupy in memory: the held
    /// records still to yield, or the batch a window's walk stands in; a
    /// stream holds no landed batch between pulls, and a consumed reader
    /// nothing, and both answer zero.
    #[napi]
    pub fn resident_size(&self) -> f64 {
        count(self.inner.as_ref().map_or(0, SerieReader::resident_size))
    }

    /// Whether every record this reader holds lies in a spill file: held
    /// records only, never a stream, which holds none.
    #[napi]
    pub fn is_spilled(&self) -> bool {
        self.inner.as_ref().is_some_and(SerieReader::is_spilled)
    }

    /// Move the records this reader holds to disk under the bound `options`
    /// states - the process default where it is `undefined` or `null` -
    /// each held record as `Serie.spill` moves it; a stream holds none and
    /// is untouched. Refused once the reader was taken.
    #[napi]
    pub fn spill(&mut self, options: Option<ClassInstance<'_, JsSpillOptions>>) -> Result<()> {
        if self.taken {
            return Err(serie_reader_consumed());
        }
        let bound = spill_bound(options.as_deref())?;
        match self.inner.as_mut() {
            Some(reader) => reader.spill(bound).map_err(napi_error),
            None => Ok(()),
        }
    }

    /// Every record this reader yields in sorted order under the options:
    /// the stream drained into its chunks, each settled as it lands, merged,
    /// and read back as the held stream of the merged chunks. The root and
    /// the record a window states are kept; the reader is consumed.
    #[napi(js_name = "_intoSortedNative", skip_typescript)]
    pub fn into_sorted_native(
        &mut self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<Self> {
        let reader = self.inner.take().ok_or_else(serie_reader_consumed)?;
        self.taken = true;
        reader
            .into_sorted(sort_options(descending, nulls_first))
            .map_err(napi_error)
            .and_then(Self::from_core)
    }

    /// Every record this reader yields in the order the `order by` keys of
    /// `by` state, bound against the root before any batch is pulled, then
    /// drained and merged as `intoSorted` is. The reader is consumed, a
    /// refused key included.
    #[napi(js_name = "_intoSortByNative", skip_typescript)]
    pub fn into_sort_by_native(&mut self, by: OrderingsInput<'_>) -> Result<Self> {
        let by = orderings_of(&by)?;
        let reader = self.inner.take().ok_or_else(serie_reader_consumed)?;
        self.taken = true;
        reader
            .into_sort_by(by)
            .map_err(napi_error)
            .and_then(Self::from_core)
    }

    /// This stream joined with `other` - a held serie, a chunked one, or
    /// another stream, consumed too - on the keys `by` holds, under the kind
    /// `how` names and `options`: a stream of the output, one probe batch
    /// joined at a time, the held side built first. The kind and the options
    /// are read before anything is consumed; this reader is consumed, a
    /// refused key included.
    #[napi(js_name = "_joinWithNative", skip_typescript)]
    pub fn join_with_native(
        &mut self,
        other: Either3<
            ClassInstance<'_, JsSerie>,
            ClassInstance<'_, JsChunkedSerie>,
            ClassInstance<'_, JsSerieReader>,
        >,
        by: &JsScalar,
        how: Option<String>,
        options: Option<JoinOptionsInput<'_>>,
    ) -> Result<Self> {
        let how = join_kind(how)?;
        let options = join_options(options)?;
        if self.taken || self.inner.is_none() {
            return Err(serie_reader_consumed());
        }
        let other = match other {
            Either3::A(serie) => JoinSource::from(serie.inner.clone()),
            Either3::B(chunked) => JoinSource::from(chunked.inner.clone()),
            Either3::C(mut reader) => {
                let taken = reader.inner.take().ok_or_else(serie_reader_consumed)?;
                reader.taken = true;
                JoinSource::from(taken)
            }
        };
        let reader = self.inner.take().ok_or_else(serie_reader_consumed)?;
        self.taken = true;
        reader
            .join_with(other, &by.inner, how, &options)
            .map_err(napi_error)
            .and_then(Self::from_core)
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

/// The windows of a stream, one lazy `SerieReader` per run of equal adjacent
/// keys, in the order they arrive.
///
/// Every window is pulled through one walk holding at most one batch of the
/// stream, so windows are read in order: taking the next window drops the
/// unread rows of the one before, and a window read after the walk passed
/// rows of it refuses once, naming it. Each window states its record as its
/// `staticValues`.
#[napi(js_name = "SerieReaderWindows")]
pub struct JsSerieReaderWindows {
    inner: SerieReaderWindows,
}

#[napi]
impl JsSerieReaderWindows {
    /// The record root every window yields: the windowed reader's own.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.field().clone())
    }

    /// The record every window's `staticValues` is typed by, known before
    /// the first pull: the windowed reader's own but `windownum` and
    /// `rownum`, the key cells, `windownum` and `rownum`.
    #[napi(getter)]
    pub fn static_field(&self) -> JsField {
        JsField::from_core(self.inner.static_field().clone())
    }

    /// Open the next window as its lazy reader, or `null` after the last.
    ///
    /// The native half of the iteration protocol; the loader wraps it so
    /// `for...of` yields each window's `SerieReader`.
    #[napi(js_name = "_nextNative", skip_typescript)]
    pub fn next_native(&mut self) -> Result<Option<JsSerieReader>> {
        match self.inner.next() {
            None => Ok(None),
            Some(window) => window
                .map_err(napi_error)
                .and_then(JsSerieReader::from_core)
                .map(Some),
        }
    }
}
