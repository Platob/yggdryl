//! Native key layouts, their payloads, and held or streamed collections.

use crate::chunked_serie::JsChunkedSerie;
use crate::expression::{JsStreamSerie, SelectorInput, selector_from_input};
use crate::field::JsField;
use crate::napi_error;
use crate::serie::{
    JsSerie, JsStreamChunkedSerie, PartitionOptionsInput, count, partition_options, position,
};
use crate::text::codec::JsScalar;
use crate::text::line::JsFieldPath;
use crate::window_serie::JsWindowSerie;
use napi::bindgen_prelude::{ClassInstance, Either3, Either8, Result};
use napi_derive::napi;
use yggdryl::{Field, FieldPath, IntoKeyBy, KeyBy, KeySerie, KeySeries, Serie, StreamKeySerie};

/// Every native source enters a record write without an Arrow crossing.
pub(crate) type SerieInput<'a> = Either8<
    ClassInstance<'a, JsSerie>,
    ClassInstance<'a, JsChunkedSerie>,
    ClassInstance<'a, JsStreamChunkedSerie>,
    ClassInstance<'a, JsStreamSerie>,
    ClassInstance<'a, JsKeySerie>,
    ClassInstance<'a, JsKeySeries>,
    ClassInstance<'a, JsStreamKeySerie>,
    ClassInstance<'a, JsWindowSerie>,
>;

pub(crate) fn serie_source(value: SerieInput<'_>) -> Result<Serie> {
    Ok(match value {
        Either8::A(value) => value.inner.clone(),
        Either8::B(value) => value.inner.clone().into(),
        Either8::C(mut value) => value.take()?.into(),
        Either8::D(mut value) => value.take()?.into(),
        Either8::E(value) => value.inner.clone().into(),
        Either8::F(value) => value.inner.clone().into(),
        Either8::G(mut value) => value.take()?.into(),
        Either8::H(value) => value.into_serie_native()?.inner.clone(),
    })
}

pub(crate) type KeyInput<'a> =
    Either3<SelectorInput<'a>, Vec<ClassInstance<'a, JsFieldPath>>, SerieInput<'a>>;
pub(crate) fn key_by(value: KeyInput<'_>) -> Result<KeyBy> {
    match value {
        Either3::A(value) => selector_from_input(value)?
            .into_key_by()
            .map_err(napi_error),
        Either3::B(paths) => paths
            .into_iter()
            .map(|path| path.inner.clone())
            .collect::<Vec<_>>()
            .into_key_by()
            .map_err(napi_error),
        Either3::C(value) => serie_source(value).map(KeyBy::External),
    }
}
pub(crate) fn row_bound(value: Option<f64>) -> Result<Option<usize>> {
    value.map(|value| position(value, "rowSize")).transpose()
}
pub(crate) fn byte_bound(value: Option<f64>) -> Result<Option<u64>> {
    value
        .map(|value| crate::exact_u64(value, "byteSize"))
        .transpose()
}

/// One immutable key context and its native payload.
#[napi(js_name = "KeySerie")]
pub struct JsKeySerie {
    pub(crate) inner: KeySerie,
}
impl JsKeySerie {
    pub(crate) const fn from_core(inner: KeySerie) -> Self {
        Self { inner }
    }
}
/// A repeatable collection of native key items sharing one layout.
#[napi(js_name = "KeySeries")]
pub struct JsKeySeries {
    pub(crate) inner: KeySeries,
}
impl JsKeySeries {
    pub(crate) const fn from_core(inner: KeySeries) -> Self {
        Self { inner }
    }
}
/// A lazy, single-use walk of native key items.
#[napi(js_name = "StreamKeySerie")]
pub struct JsStreamKeySerie {
    inner: Option<StreamKeySerie>,
    field: Field,
    key_field: Field,
    serie_field: Field,
    key_paths: Vec<Option<FieldPath>>,
}
impl JsStreamKeySerie {
    pub(crate) fn from_core(inner: StreamKeySerie) -> Self {
        Self {
            field: inner.field().clone(),
            key_field: inner.key_field().clone(),
            serie_field: inner.serie_field().clone(),
            key_paths: inner.key_paths().to_vec(),
            inner: Some(inner),
        }
    }
    pub(crate) fn take(&mut self) -> Result<StreamKeySerie> {
        self.inner
            .take()
            .ok_or_else(|| napi_error("StreamKeySerie was already consumed"))
    }
}

#[napi]
impl JsKeySerie {
    /// The complete field: key columns followed by payload columns.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.field().clone())
    }
    /// The field of the explicit key context.
    #[napi(getter)]
    pub fn key_field(&self) -> JsField {
        JsField::from_core(self.inner.key_field().clone())
    }
    /// The payload field, without columns moved into the key.
    #[napi(getter)]
    pub fn serie_field(&self) -> JsField {
        JsField::from_core(self.inner.serie_field().clone())
    }
    /// Source paths for moved columns; computed and external keys have no path.
    #[napi(getter)]
    pub fn key_paths(&self) -> Vec<Option<JsFieldPath>> {
        self.inner
            .key_paths()
            .iter()
            .map(|path| path.clone().map(JsFieldPath::from_core))
            .collect()
    }
    /// The key as a scalar record in key-field order.
    #[napi(getter)]
    pub fn key(&self) -> JsScalar {
        JsScalar::from_core(self.inner.key().clone())
    }
    /// The absolute start of an adjacent window, absent for a partition or gather.
    #[napi(getter)]
    pub fn rownum(&self) -> Option<f64> {
        self.inner
            .rownum()
            .map(|value| count(usize::try_from(value).unwrap_or(usize::MAX)))
    }
    /// The native payload; reading it retains the source layout and ownership.
    #[napi(getter, js_name = "_rowsNative", skip_typescript)]
    pub fn rows(&self) -> JsSerie {
        JsSerie::from_core(self.inner.rows().clone())
    }
    /// The context and payload as separate native values.
    #[napi]
    pub fn into_parts(&self) -> (JsScalar, JsSerie) {
        let (key, rows) = self.inner.clone().into_parts();
        (JsScalar::from_core(key), JsSerie::from_core(rows))
    }
    /// The bytes currently held, without pulling a stream.
    #[napi]
    pub fn memory_size(&self) -> f64 {
        count(self.inner.memory_size())
    }
    /// Cuts adjacent equal keys under one selector, path, or typed external-key layout.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(&self, by: KeyInput<'_>, sorted: Option<bool>) -> Result<JsKeySeries> {
        self.inner
            .window_by(key_by(by)?, sorted.unwrap_or(false))
            .map(JsKeySeries::from_core)
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
    /// Moves these values into a native scalar-row stream.
    #[napi]
    pub fn into_stream(&self) -> Result<JsStreamSerie> {
        self.inner
            .clone()
            .into_stream()
            .map(JsStreamSerie::from_core)
            .map_err(napi_error)
    }
    /// Moves these values into native chunks under optional row and byte bounds.
    #[napi]
    pub fn into_chunked_stream(
        &self,
        row_size: Option<f64>,
        byte_size: Option<f64>,
    ) -> Result<JsStreamChunkedSerie> {
        self.inner
            .clone()
            .into_chunked_stream(row_bound(row_size)?, byte_bound(byte_size)?)
            .map_err(napi_error)
            .map(JsStreamChunkedSerie::from_core)
    }
}
#[napi]
impl JsKeySeries {
    /// The complete field: key columns followed by payload columns.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.inner.field().clone())
    }
    /// The field of the explicit key context.
    #[napi(getter)]
    pub fn key_field(&self) -> JsField {
        JsField::from_core(self.inner.key_field().clone())
    }
    /// The payload field, without columns moved into the key.
    #[napi(getter)]
    pub fn serie_field(&self) -> JsField {
        JsField::from_core(self.inner.serie_field().clone())
    }
    /// Source paths for moved columns; computed and external keys have no path.
    #[napi(getter)]
    pub fn key_paths(&self) -> Vec<Option<JsFieldPath>> {
        self.inner
            .key_paths()
            .iter()
            .map(|path| path.clone().map(JsFieldPath::from_core))
            .collect()
    }
    /// The number of held key items.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        count(self.inner.len())
    }
    /// The held item at this position, absent outside the collection.
    #[napi]
    pub fn get(&self, index: f64) -> Result<Option<JsKeySerie>> {
        Ok(self
            .inner
            .get(position(index, "index")?)
            .cloned()
            .map(JsKeySerie::from_core))
    }
    /// The bytes currently held, without pulling a stream.
    #[napi]
    pub fn memory_size(&self) -> f64 {
        count(self.inner.memory_size())
    }
    /// Cuts adjacent equal keys under one selector, path, or typed external-key layout.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(&self, by: KeyInput<'_>, sorted: Option<bool>) -> Result<Self> {
        self.inner
            .window_by(key_by(by)?, sorted.unwrap_or(false))
            .map(Self::from_core)
            .map_err(napi_error)
    }
    /// Groups equal keys under the same key layout, retaining native payloads.
    #[napi(js_name = "_partitionByNative", skip_typescript)]
    pub fn partition_by_native(&self, by: KeyInput<'_>) -> Result<Self> {
        self.inner
            .partition_by(key_by(by)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }
    /// Moves these values into a native scalar-row stream.
    #[napi]
    pub fn into_stream(&self) -> Result<JsStreamSerie> {
        self.inner
            .clone()
            .into_stream()
            .map(JsStreamSerie::from_core)
            .map_err(napi_error)
    }
    /// Moves these values into native chunks under optional row and byte bounds.
    #[napi]
    pub fn into_chunked_stream(
        &self,
        row_size: Option<f64>,
        byte_size: Option<f64>,
    ) -> Result<JsStreamChunkedSerie> {
        self.inner
            .clone()
            .into_chunked_stream(row_bound(row_size)?, byte_bound(byte_size)?)
            .map_err(napi_error)
            .map(JsStreamChunkedSerie::from_core)
    }
}
#[napi]
impl JsStreamKeySerie {
    /// The complete field: key columns followed by payload columns.
    #[napi(getter)]
    pub fn field(&self) -> JsField {
        JsField::from_core(self.field.clone())
    }
    /// The field of the explicit key context.
    #[napi(getter)]
    pub fn key_field(&self) -> JsField {
        JsField::from_core(self.key_field.clone())
    }
    /// The payload field, without columns moved into the key.
    #[napi(getter)]
    pub fn serie_field(&self) -> JsField {
        JsField::from_core(self.serie_field.clone())
    }
    /// Source paths for moved columns; computed and external keys have no path.
    #[napi(getter)]
    pub fn key_paths(&self) -> Vec<Option<JsFieldPath>> {
        self.key_paths
            .iter()
            .map(|path| path.clone().map(JsFieldPath::from_core))
            .collect()
    }
    /// Pulls the next key item, or returns null after the stream ends.
    #[napi(js_name = "_nextNative", skip_typescript)]
    pub fn next_native(&mut self) -> Result<Option<JsKeySerie>> {
        self.inner
            .as_mut()
            .and_then(Iterator::next)
            .transpose()
            .map(|item| item.map(JsKeySerie::from_core))
            .map_err(napi_error)
    }
    /// Cuts adjacent equal keys under one selector, path, or typed external-key layout.
    #[napi(js_name = "_windowByNative", skip_typescript)]
    pub fn window_by_native(&mut self, by: KeyInput<'_>, sorted: Option<bool>) -> Result<Self> {
        let by = key_by(by)?;
        self.take()?
            .window_by(by, sorted.unwrap_or(false))
            .map(Self::from_core)
            .map_err(napi_error)
    }
    /// Groups equal keys under the same key layout, retaining native payloads.
    #[napi(js_name = "_partitionByNative", skip_typescript)]
    pub fn partition_by_native(
        &mut self,
        by: KeyInput<'_>,
        options: Option<PartitionOptionsInput>,
    ) -> Result<Self> {
        let by = key_by(by)?;
        let options = partition_options(options)?;
        self.take()?
            .partition_by(by, options)
            .map(Self::from_core)
            .map_err(napi_error)
    }
    /// Moves these values into a native scalar-row stream.
    #[napi]
    pub fn into_stream(&mut self) -> Result<JsStreamSerie> {
        self.take()?
            .into_stream()
            .map(JsStreamSerie::from_core)
            .map_err(napi_error)
    }
    /// Moves these values into native chunks under optional row and byte bounds.
    #[napi]
    pub fn into_chunked_stream(
        &mut self,
        row_size: Option<f64>,
        byte_size: Option<f64>,
    ) -> Result<JsStreamChunkedSerie> {
        self.take()?
            .into_chunked_stream(row_bound(row_size)?, byte_bound(byte_size)?)
            .map_err(napi_error)
            .map(JsStreamChunkedSerie::from_core)
    }
}
