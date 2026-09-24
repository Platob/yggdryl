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

use std::sync::Arc;

use napi::bindgen_prelude::{Buffer, ClassInstance, Either, Result, Uint8Array};
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl::{ArrowCastOptions, ChunkedSerie, Field as CoreField, FieldPath, SerieReader};

use crate::datatype::JsDataType;
use crate::field::JsField;
use crate::iomedia::JsBatchReader;
use crate::napi_error;
use crate::serie::{JsSerie, JsSerieIterator, arrow_arrays_ipc, arrow_batches, position};
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

/// The three cast answers JavaScript spells separately, as one native value.
fn options_of(
    safe: Option<bool>,
    nullability: Option<String>,
    representation: Option<String>,
) -> Result<ArrowCastOptions> {
    crate::cast_options(safe, nullability.as_deref(), representation.as_deref())
}

/// A row count as the number JavaScript reads.
fn count(value: usize) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let value = value as f64;
    value
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
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, nullability, representation)?;
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
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, nullability, representation)?;
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
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, nullability, representation)?;
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
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, nullability, representation)?;
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
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<()> {
        let options = options_of(safe, nullability, representation)?;
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
        nullability: Option<String>,
        representation: Option<String>,
    ) -> Result<Self> {
        let options = options_of(safe, nullability, representation)?;
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
        let root = SerieReader::root_of(self.inner.field()).map_err(napi_error)?;
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
}
