//! JavaScript's native view of the shared [`ChunkedSerie`]: many columns
//! under one field, held apart - what an Apache Arrow JS vector of several
//! `Data` is, and, when the field is a record, a table of one batch per
//! chunk.
//!
//! [`JsChunkedSerie`] owns only the core value and redirects every verb to
//! it. The verbs answering a `Serie` - a chunk, every chunk, the join - are
//! private natives here, because `binding.js` hands each serie out as its
//! leaf's class. Arrow crosses as copied IPC, one batch per chunk, and
//! nothing is joined but by `intoSerie`.

use crate::key_serie::{JsKeySeries, KeyInput, key_by};
use std::sync::Arc;

use napi::bindgen_prelude::{Buffer, ClassInstance, Either, Result, Uint8Array};
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl::{ArrowCastOptions, ChunkedSerie, Field as CoreField, FieldPath, StreamChunkedSerie};

use crate::datatype::JsDataType;
use crate::field::JsField;
use crate::iomedia::JsBatchReader;
use crate::join::{JoinOptionsInput, join_kind, join_options};
use crate::napi_error;
use crate::serie::{
    JsSerie, JsSerieIterator, OrderingsInput, arrow_arrays_ipc, arrow_batches, count, orderings_of,
    position, sort_options, spelled_orderings,
};
use crate::spill::{JsSpillOptions, spill_bound};
use crate::text::codec::{JsScalar, checked_depth, value_to_transport_with_field};

/// Many columns under one field, held apart: a chunked array, or a table.
///
/// Every chunk is a column of exactly this field. A row is read out of the
/// chunk that holds it, and two chunked series - or a chunked serie and a
/// serie - are equal when their rows are, however the rows are cut.
#[napi(js_name = "ChunkedSerie")]
#[derive(Clone)]
pub struct JsChunkedSerie {
    pub(crate) inner: ChunkedSerie,
}

impl JsChunkedSerie {
    /// Wrap one native chunked serie for JavaScript.
    pub(crate) const fn from_core(inner: ChunkedSerie) -> Self {
        Self { inner }
    }
}

/// The chunks a one-column Arrow IPC stream carries - each batch's one
/// column one array of the core array door - of their own layout under the
/// field named `item`, or cast into `field` under `options`.
///
/// The IPC column's own name and nullability are the bridge's, never the
/// caller's.
fn chunked_from_ipc(
    bytes: &[u8],
    field: Option<&CoreField>,
    options: ArrowCastOptions,
) -> Result<ChunkedSerie> {
    let (schema, batches) = arrow_batches(bytes)?;
    if schema.fields().len() != 1 {
        return Err(napi_error(format!(
            "ChunkedSerie IPC must contain exactly one column, got {}",
            schema.fields().len()
        )));
    }
    let arrays = batches.iter().map(|batch| Arc::clone(batch.column(0)));
    ChunkedSerie::from_arrow_arrays(field, arrays, options).map_err(napi_error)
}

/// The two cast answers JavaScript spells separately, as one native value.
fn options_of(safe: Option<bool>, representation: Option<String>) -> Result<ArrowCastOptions> {
    crate::cast_options(safe, representation.as_deref())
}

#[napi]
impl JsChunkedSerie {
    /// The chunked serie of no chunks under `field`.
    #[napi(factory, js_name = "_emptyNative", skip_typescript)]
    pub fn empty(field: ClassInstance<'_, JsField>) -> Result<Self> {
        ChunkedSerie::empty(field.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// One held column as its one chunk, sharing its buffers; a run is
    /// refused.
    #[napi(factory, js_name = "_fromSerieNative", skip_typescript)]
    pub fn from_serie(serie: &JsSerie) -> Result<Self> {
        ChunkedSerie::from_serie(serie.inner.clone())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Held columns as chunks under one field: the first chunk's, nullable
    /// where any chunk's is, or `field`, every other layout cast into it.
    #[napi(factory, js_name = "_fromSeriesNative", skip_typescript)]
    pub fn from_series(
        chunks: Vec<ClassInstance<'_, JsSerie>>,
        field: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, representation)?;
        ChunkedSerie::from_series(
            field.as_ref().map(|field| &field.inner),
            chunks.iter().map(|chunk| chunk.inner.clone()),
            options,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Decode one Arrow JS vector from its one-column IPC bridge, one chunk
    /// per `Data` it holds, cast into `field` when one is given.
    #[napi(factory, js_name = "_fromArrowArrayIpcNative", skip_typescript)]
    pub fn from_arrow_array_ipc_native(
        bytes: Uint8Array,
        field: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, representation)?;
        chunked_from_ipc(&bytes, field.as_ref().map(|field| &field.inner), options)
            .map(Self::from_core)
    }

    /// Decode one Arrow JS table or record batch, one chunk per batch: of
    /// its own schema, named `row`, or cast into `root`.
    #[napi(factory, js_name = "_fromArrowBatchIpcNative", skip_typescript)]
    pub fn from_arrow_batch_ipc_native(
        bytes: Uint8Array,
        root: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, representation)?;
        let (schema, batches) = arrow_batches(&bytes)?;
        ChunkedSerie::from_arrow_reader(
            root.as_ref().map(|root| &root.inner),
            yggdryl::arrow::batch_reader(schema, batches),
            options,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Drain a native `BatchReader`, one chunk per batch: of its own schema,
    /// named `row`, or cast into `root`. The reader is consumed.
    #[napi(factory, js_name = "_fromArrowReaderNative", skip_typescript)]
    pub fn from_arrow_reader(
        mut reader: ClassInstance<'_, JsBatchReader>,
        root: Option<ClassInstance<'_, JsField>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, representation)?;
        ChunkedSerie::from_arrow_reader(
            root.as_ref().map(|root| &root.inner),
            reader.take()?,
            options,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// The field every chunk is typed by.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.field().clone())
    }

    /// `serie(<the field named item>)`: what a column of this field answers.
    #[napi(getter)]
    pub fn dtype(&self) -> JsDataType {
        JsDataType::from_core(self.inner.dtype())
    }

    /// How many chunks the rows are cut into.
    #[napi(getter)]
    pub fn num_chunks(&self) -> f64 {
        count(self.inner.num_chunks())
    }

    /// The row count across every chunk.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        count(self.inner.len())
    }

    /// Every chunk, in order, each sharing its buffers.
    #[napi(js_name = "_chunksNative", skip_typescript)]
    pub fn chunks_native(&self) -> Vec<JsSerie> {
        self.inner
            .chunks()
            .iter()
            .cloned()
            .map(JsSerie::from_core)
            .collect()
    }

    /// Chunk `index`, or `null` past the last.
    #[napi(js_name = "_chunkNative", skip_typescript)]
    pub fn chunk_native(&self, index: f64) -> Result<Option<JsSerie>> {
        Ok(self
            .inner
            .chunk(position(index, "index")?)
            .cloned()
            .map(JsSerie::from_core))
    }

    /// The rows that are absent, one read per chunk.
    #[napi]
    pub fn null_count(&self) -> f64 {
        count(self.inner.null_count())
    }

    /// Whether no chunk holds a row.
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

    /// Row `index`, built as one value out of the chunk holding it.
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
            .map(JsScalar::from_core))
    }

    /// Every row, chunk after chunk, each built once.
    #[napi]
    pub fn rows(&self) -> Vec<JsScalar> {
        self.inner.iter().map(JsScalar::from_core).collect()
    }

    /// Every row as the transport `asJs` reads, each under the field.
    #[napi(js_name = "_asJsNative", skip_typescript)]
    pub fn as_js_native(&self, max_depth: Option<u32>) -> Result<JsonValue> {
        let max_depth = checked_depth(max_depth)?;
        let field = self.inner.field();
        let rows = self
            .inner
            .iter()
            .map(|row| value_to_transport_with_field(&row, field, 1, max_depth))
            .collect::<Result<Vec<JsonValue>>>()?;
        Ok(JsonValue::Array(rows))
    }

    /// Iterate the rows, chunk after chunk, each built once.
    #[napi(js_name = "_iterNative", skip_typescript)]
    pub fn iter_native(&self) -> JsSerieIterator {
        JsSerieIterator::from_rows(self.inner.rows())
    }

    /// The window `offset..offset + length`: the chunks it reaches, the two
    /// at its edges sliced, nothing copied.
    #[napi]
    pub fn slice(&self, offset: f64, length: f64) -> Result<Self> {
        self.inner
            .slice(position(offset, "offset")?, position(length, "length")?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// A record's child named `name` in every chunk - a table's column - or
    /// `null`.
    #[napi]
    pub fn child(&self, name: String) -> Option<JsChunkedSerie> {
        self.inner.child(&name).map(Self::from_core)
    }

    /// A record's child, or a union's member, at `index` in every chunk.
    #[napi]
    pub fn child_at(&self, index: f64) -> Result<Option<JsChunkedSerie>> {
        Ok(self
            .inner
            .child_at(position(index, "index")?)
            .map(Self::from_core))
    }

    /// Every child of a record, or member of a union, chunk by chunk.
    #[napi]
    pub fn children(&self) -> Vec<JsChunkedSerie> {
        self.inner
            .children()
            .into_iter()
            .map(Self::from_core)
            .collect()
    }

    /// A sequence's items, a mapping's entries, an encoding's values, in
    /// every chunk; `null` elsewhere.
    #[napi]
    pub fn items(&self) -> Option<JsChunkedSerie> {
        self.inner.items().map(Self::from_core)
    }

    /// The chunked column `path` reaches in every chunk, spelled as a field
    /// path.
    #[napi]
    pub fn get_child_by_path(&self, path: String) -> Result<Option<JsChunkedSerie>> {
        let path = FieldPath::from_str(&path).map_err(napi_error)?;
        Ok(self.inner.get_child_by_path(&path).map(Self::from_core))
    }

    /// Append one column as a chunk: under the field as it stands, any other
    /// cast into it; a refusal leaves the chunks as they were.
    #[napi(js_name = "_pushChunkNative", skip_typescript)]
    pub fn push_chunk_native(
        &mut self,
        chunk: &JsSerie,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<()> {
        let options = options_of(safe, representation)?;
        self.inner
            .push_chunk(chunk.inner.clone(), options)
            .map_err(napi_error)
    }

    /// Every row as one column: the one join.
    #[napi(js_name = "_intoSerieNative", skip_typescript)]
    pub fn into_serie_native(&self) -> Result<JsSerie> {
        self.inner
            .into_serie()
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// Every chunk under `target` - a `Field`, or a `DataType` as its
    /// required `value` field - by one plan.
    #[napi(js_name = "_castNative", skip_typescript)]
    pub fn cast_native(
        &self,
        target: Either<ClassInstance<'_, JsField>, ClassInstance<'_, JsDataType>>,
        safe: Option<bool>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, representation)?;
        let target = match target {
            Either::A(field) => field.inner.clone(),
            Either::B(dtype) => dtype.inner.clone().required_field("value"),
        };
        self.inner
            .cast(&target, options)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Encode every chunk as one batch of a one-column Arrow IPC stream
    /// under the field, which Arrow JS reads as one vector of one `Data`
    /// per chunk.
    #[napi(js_name = "_intoArrowArrayIpcNative", skip_typescript)]
    pub fn into_arrow_array_ipc_native(&self) -> Result<Buffer> {
        arrow_arrays_ipc(self.inner.field(), self.inner.into_arrow_arrays())
    }

    /// Every chunk as one batch of a native `BatchReader`: a record's chunks
    /// the batches they are, any other field's the one column of a `row`
    /// root. A record chunk holding an absent row is refused.
    #[napi]
    pub fn into_arrow_reader(&self) -> Result<JsBatchReader> {
        let reader = self.inner.into_arrow_reader().map_err(napi_error)?;
        let root = StreamChunkedSerie::root_of(self.inner.field()).map_err(napi_error)?;
        Ok(JsBatchReader::from_core(reader, root.name()))
    }

    /// Whether the rows equal another chunked serie's, or a serie's.
    #[napi(js_name = "_equalsNative", skip_typescript)]
    pub fn equals_native(
        &self,
        other: Either<ClassInstance<'_, JsChunkedSerie>, ClassInstance<'_, JsSerie>>,
    ) -> bool {
        match other {
            Either::A(chunked) => self.inner == chunked.inner,
            Either::B(serie) => self.inner == serie.inner,
        }
    }

    /// Order the rows against another chunked serie's, or a serie's, as the
    /// core orders a column's, however the rows are cut.
    #[napi(js_name = "_compareNative", skip_typescript)]
    pub fn compare_native(
        &self,
        other: Either<ClassInstance<'_, JsChunkedSerie>, ClassInstance<'_, JsSerie>>,
    ) -> Result<i32> {
        // The core orders a chunked serie against a column by the rows, as it
        // does two chunked series, so no chunk is joined here.
        let ordering = match other {
            Either::A(chunked) => Some(self.inner.cmp(&chunked.inner)),
            Either::B(serie) => self.inner.partial_cmp(&serie.inner),
        };
        ordering
            .map(crate::ordering_value)
            .ok_or_else(|| napi_error("the rows of a chunked serie and a serie have no order"))
    }

    /// A copy sharing every chunk's buffers.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// The rows, rendered behind the field's name as the column of them
    /// would be.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    // ------------------------------------------------------------------
    // Ordering, uniqueness and grouping across the chunks: each one core
    // verb, chunk by chunk where a chunk alone can answer and through the
    // one join where the rows must be seen together. The loader validates
    // the options and coerces indices, masks and keys.
    // ------------------------------------------------------------------

    /// The row positions in sorted order across the chunks, as a `uint32`
    /// column named `index`.
    #[napi(js_name = "_sortIndicesNative", skip_typescript)]
    pub fn sort_indices_native(
        &self,
        descending: Option<bool>,
        nulls_first: Option<bool>,
    ) -> Result<JsSerie> {
        self.inner
            .sort_indices(sort_options(descending, nulls_first))
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// Whether the rows are in sorted order across the chunks: each chunk,
    /// then every chunk edge, with no join.
    #[napi(js_name = "_isSortedNative", skip_typescript)]
    pub fn is_sorted_native(&self, descending: Option<bool>, nulls_first: Option<bool>) -> bool {
        self.inner.is_sorted(sort_options(descending, nulls_first))
    }

    /// Whether no two rows across the chunks hold one value; two absent rows
    /// are a repeat.
    #[napi]
    pub fn is_unique(&self) -> bool {
        self.inner.is_unique()
    }

    /// How many distinct values the rows hold across the chunks, an absent
    /// row one of them.
    #[napi]
    pub fn unique_count(&self) -> f64 {
        count(self.inner.unique_count())
    }

    /// The bytes the rows occupy: every chunk's, as its own slice counts
    /// them.
    #[napi]
    pub fn memory_size(&self) -> f64 {
        count(self.inner.memory_size())
    }

    /// The bytes the rows occupy in memory: every chunk's `residentSize`,
    /// summed.
    #[napi]
    pub fn resident_size(&self) -> f64 {
        count(self.inner.resident_size())
    }

    /// Whether every chunk's rows lie in a spill file: no byte resident, and
    /// some bytes. A chunked serie of no chunk is never spilled.
    #[napi]
    pub fn is_spilled(&self) -> bool {
        self.inner.is_spilled()
    }

    /// Move chunks to disk until the resident bytes are under the bound
    /// `options` states - the process default where it is `undefined` or
    /// `null` - the heaviest chunks whole first, so a chunked serie under the
    /// bound is untouched and one over it keeps its lightest chunks
    /// resident. A refused folder is named, the chunks spilled so far kept
    /// mapped.
    #[napi]
    pub fn spill(&mut self, options: Option<ClassInstance<'_, JsSpillOptions>>) -> Result<()> {
        let bound = spill_bound(options.as_deref())?;
        self.inner.spill(bound).map_err(napi_error)
    }

    /// Spill chunks in place under the bound `options` states, as `spill`
    /// does; the loader answers this chunked serie.
    #[napi(js_name = "_asSpilledNative", skip_typescript)]
    pub fn as_spilled_native(
        &mut self,
        options: Option<ClassInstance<'_, JsSpillOptions>>,
    ) -> Result<()> {
        let bound = spill_bound(options.as_deref())?;
        self.inner.as_spilled(bound).map(|_| ()).map_err(napi_error)
    }

    /// A copy of this chunked serie spilled under the bound `options`
    /// states, this one untouched: the chunks the bound leaves resident are
    /// shared, the rest written once and mapped.
    #[napi(js_name = "_intoSpilledNative", skip_typescript)]
    pub fn into_spilled_native(
        &self,
        options: Option<ClassInstance<'_, JsSpillOptions>>,
    ) -> Result<Self> {
        let bound = spill_bound(options.as_deref())?;
        self.inner
            .into_spilled(bound)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The `order by` keys this chunked record's field declares its rows
    /// keep across every chunk, each as the key grammar spells it; `null`
    /// where it declares none or is no record.
    #[napi]
    pub fn declared_order(&self) -> Result<Option<Vec<String>>> {
        self.inner
            .declared_order()
            .map(spelled_orderings)
            .map_err(napi_error)
    }

    /// The rows in sorted order, the chunks sorted on their own and merged
    /// into chunks of at most the record batch row size, with no join.
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

    /// The row positions in the order the `order by` keys of `by` state,
    /// over every chunk, as a `uint32` column named `index`: the keys read
    /// first, then the one join.
    #[napi(js_name = "_sortIndicesByNative", skip_typescript)]
    pub fn sort_indices_by_native(&self, by: OrderingsInput<'_>) -> Result<JsSerie> {
        self.inner
            .sort_indices_by(orderings_of(&by)?)
            .map(JsSerie::from_core)
            .map_err(napi_error)
    }

    /// The rows in the order the `order by` keys of `by` state, merged as
    /// `intoSorted` merges them, with no join; the field declares the keys.
    #[napi(js_name = "_intoSortByNative", skip_typescript)]
    pub fn into_sort_by_native(&self, by: OrderingsInput<'_>) -> Result<Self> {
        self.inner
            .into_sort_by(orderings_of(&by)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// This chunked serie joined with `other` on the keys `by` holds, under
    /// the kind `how` names - `inner` where it names none - and `options`:
    /// the output batches kept apart as chunks, each settled.
    #[napi(js_name = "_joinWithNative", skip_typescript)]
    pub fn join_with_native(
        &self,
        other: &JsChunkedSerie,
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

    /// The first occurrence of every value across the chunks, with no
    /// join: each chunk filtered by its own first occurrences and kept
    /// apart, a chunk left with no row dropped.
    #[napi]
    pub fn into_unique(&self) -> Result<Self> {
        self.inner
            .into_unique()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The rows in reverse order: the chunks reversed, each reversed, kept
    /// apart.
    #[napi]
    pub fn into_reversed(&self) -> Self {
        Self::from_core(self.inner.into_reversed())
    }

    /// The rows an integer serie of positions across the chunks names, as a
    /// chunked serie of one chunk.
    #[napi(js_name = "_intoTakenNative", skip_typescript)]
    pub fn into_taken_native(&self, indices: &JsSerie) -> Result<Self> {
        self.inner
            .into_taken(&indices.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// The rows a boolean serie as long as the whole keeps, chunk by chunk
    /// and kept apart.
    #[napi(js_name = "_intoFilteredNative", skip_typescript)]
    pub fn into_filtered_native(&self, mask: &JsSerie) -> Result<Self> {
        self.inner
            .into_filtered(&mask.inner)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Groups equal keys under the same key layout, retaining native payloads.
    #[napi(js_name = "_partitionByNative", skip_typescript)]
    pub fn partition_by_native(&self, by: KeyInput<'_>) -> Result<JsKeySeries> {
        self.inner
            .partition_by(key_by(by)?)
            .map(JsKeySeries::from_core)
            .map_err(napi_error)
    }

    /// Cuts adjacent equal keys under one selector, path, or typed external-key layout.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(&self, by: KeyInput<'_>, sorted: Option<bool>) -> Result<JsKeySeries> {
        self.inner
            .window_by(key_by(by)?, sorted.unwrap_or(false))
            .map(JsKeySeries::from_core)
            .map_err(napi_error)
    }

    /// Sort the rows in place under the options: the merged chunks replace
    /// the chunks.
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

    /// Keep the first occurrence of every value in place, each chunk its
    /// own first occurrences, kept apart.
    #[napi(js_name = "_asUniqueNative", skip_typescript)]
    pub fn as_unique_native(&mut self) -> Result<()> {
        self.inner.as_unique().map(|_| ()).map_err(napi_error)
    }

    /// Sort the rows in place in the order the `order by` keys of `by`
    /// state: the merged chunks replace the chunks; a refusal leaves them as
    /// they were.
    #[napi(js_name = "_asSortByNative", skip_typescript)]
    pub fn as_sort_by_native(&mut self, by: OrderingsInput<'_>) -> Result<()> {
        let by = orderings_of(&by)?;
        self.inner.as_sort_by(by).map(|_| ()).map_err(napi_error)
    }

    /// Reverse the rows in place: the chunks reversed, each where it stands.
    #[napi(js_name = "_asReversedNative", skip_typescript)]
    pub fn as_reversed_native(&mut self) -> Result<()> {
        self.inner.as_reversed().map(|_| ()).map_err(napi_error)
    }

    /// Keep the rows an integer serie of positions names, in place.
    #[napi(js_name = "_asTakenNative", skip_typescript)]
    pub fn as_taken_native(&mut self, indices: &JsSerie) -> Result<()> {
        self.inner
            .as_taken(&indices.inner)
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Keep the rows a boolean serie keeps, in place and chunk by chunk.
    #[napi(js_name = "_asFilteredNative", skip_typescript)]
    pub fn as_filtered_native(&mut self, mask: &JsSerie) -> Result<()> {
        self.inner
            .as_filtered(&mask.inner)
            .map(|_| ())
            .map_err(napi_error)
    }
}

#[napi]
impl JsChunkedSerie {
    /// Moves these values into a native scalar-row stream.
    #[napi]
    pub fn into_stream(&self) -> Result<crate::expression::JsStreamSerie> {
        self.inner
            .clone()
            .into_stream()
            .map(crate::expression::JsStreamSerie::from_core)
            .map_err(napi_error)
    }
    /// Moves these values into native chunks under optional row and byte bounds.
    #[napi]
    pub fn into_chunked_stream(
        &self,
        row_size: Option<f64>,
        byte_size: Option<f64>,
    ) -> Result<crate::serie::JsStreamChunkedSerie> {
        self.inner
            .clone()
            .into_chunked_stream(
                crate::key_serie::row_bound(row_size)?,
                crate::key_serie::byte_bound(byte_size)?,
            )
            .map_err(napi_error)
            .map(crate::serie::JsStreamChunkedSerie::from_core)
    }
}
