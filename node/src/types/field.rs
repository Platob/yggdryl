//! Node.js view of the native Field domain.

use std::collections::HashMap;

use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either, Either4, Env, Error, Reference, Result, Uint8Array,
    Unknown,
};
use napi_derive::napi;
use yggdryl::{Field as CoreField, ProtocolField as CoreProtocolField, Scheme as CoreScheme};

use crate::{
    JsDifferenceIterator,
    enums::{
        JsMediaType, JsMimeType, MediaTypeInput, MimeTypeInput, media_type_from_input,
        mime_type_from_input,
    },
    exact_i32,
    fix::{branch_from_js, id_from_js},
    napi_error, napi_type_error, ordering_value,
    types::datatype::{JsAsciiEnum, JsDataType, dtype_from_input},
    types::value::arrow_scalar_to_ipc,
    types::value::field_value_to_js,
    uri::{JsUri, JsUrl, JsUrn, url_from_input},
};

/// One field-metadata key/value pair.
#[napi(object)]
pub struct MetadataEntry {
    /// Metadata key.
    pub key: String,
    /// Metadata value.
    pub value: String,
}

/// Metadata as `[{key, value}]` entries or one plain object.
pub type MetadataInput = Either<Vec<MetadataEntry>, HashMap<String, String>>;

pub(crate) fn metadata_pairs(value: MetadataInput) -> HashMap<String, String> {
    match value {
        Either::A(entries) => {
            let mut values = HashMap::with_capacity(entries.len());
            for entry in entries {
                values.insert(entry.key, entry.value);
            }
            values
        }
        Either::B(values) => values,
    }
}

/// An Arrow field whose metadata and cache invariants are owned by Rust.
#[napi(js_name = "Field")]
pub struct JsField {
    pub(crate) inner: CoreField,
}

impl Clone for JsField {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl JsField {
    pub(crate) fn from_core(inner: CoreField) -> Self {
        Self { inner }
    }

    fn apply_metadata(&mut self, value: MetadataInput) -> Result<()> {
        self.inner
            .update_metadata(metadata_pairs(value))
            .map_err(napi_error)
    }
}

#[napi]
impl JsField {
    /// Parse/clone a `Field`, or construct one from an inferred `DataType`.
    #[napi(constructor)]
    pub fn new(
        value: Either<ClassInstance<'_, JsField>, String>,
        dtype: Option<Either<ClassInstance<'_, JsDataType>, String>>,
        nullable: Option<bool>,
        metadata: Option<Either<Vec<MetadataEntry>, HashMap<String, String>>>,
    ) -> Result<Self> {
        let should_override_nullable = dtype.is_none();
        let mut field = match (value, dtype) {
            (Either::A(field), None) => field.inner.clone(),
            (Either::A(_), Some(_)) => {
                return Err(Error::from_reason(
                    "a cloned Field cannot be combined with a DataType",
                ));
            }
            (Either::B(value), None) => CoreField::from_str(&value).map_err(napi_error)?,
            (Either::B(name), Some(dtype)) => {
                let field =
                    CoreField::new(name, dtype_from_input(dtype)?, nullable.unwrap_or(true));
                field.validate().map_err(napi_error)?;
                field
            }
        };

        if should_override_nullable && let Some(nullable) = nullable {
            field.set_nullable(nullable);
        }
        let mut field = Self::from_core(field);
        if let Some(metadata) = metadata {
            field.apply_metadata(metadata)?;
        }
        Ok(field)
    }

    /// Infer a Field from a native wrapper or field-expression string.
    #[napi(factory, js_name = "from")]
    pub fn from_js(value: Either<ClassInstance<'_, JsField>, String>) -> Result<Self> {
        match value {
            Either::A(value) => Ok(Self::from_core(value.inner.clone())),
            Either::B(value) => CoreField::from_str(&value)
                .map(Self::from_core)
                .map_err(napi_error),
        }
    }

    /// Parse canonical, Arrow, SQL, Hive, or Spark field syntax.
    #[napi(factory)]
    pub fn from_string(value: String) -> Result<Self> {
        CoreField::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the textual representation of an Arrow-compatible JS field.
    #[napi(factory, js_name = "fromArrowString", skip_typescript)]
    pub fn from_arrow(value: String) -> Result<Self> {
        CoreField::from_str(&value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Cast one Arrow IPC stream to this exact Field, batch by batch.
    ///
    /// The loader wraps this as `castArrow`/`cast`, which hand over whatever
    /// Arrow JS holds and read the result back as a `Table`.
    #[napi(js_name = "_castArrowIpc", skip_typescript)]
    pub fn cast_arrow_ipc(&self, bytes: Uint8Array, safe: Option<bool>) -> Result<Buffer> {
        use arrow_ipc::reader::StreamReader;
        use arrow_ipc::writer::StreamWriter;
        use yggdryl::ArrowCast;

        let safe = safe.unwrap_or(true);
        let reader = StreamReader::try_new(std::io::Cursor::new(bytes.to_vec()), None)
            .map_err(napi_error)?;
        let schema = self.inner.clone().into_arrow_schema().map_err(napi_error)?;
        let mut writer = StreamWriter::try_new(Vec::new(), schema.as_ref()).map_err(napi_error)?;
        for batch in reader {
            let cast = self
                .inner
                .cast_arrow_batch(batch.map_err(napi_error)?, safe)
                .map_err(napi_error)?;
            writer.write(&cast).map_err(napi_error)?;
        }
        writer.finish().map_err(napi_error)?;
        Ok(Buffer::from(writer.into_inner().map_err(napi_error)?))
    }

    /// Build an empty native reader carrying exactly this Field's Arrow schema.
    ///
    /// The JavaScript records adapter captures and removes this private bridge;
    /// it lets an empty record iterable publish a declared schema without a
    /// second datatype-to-Arrow implementation in JavaScript.
    #[napi(js_name = "_emptyArrowReaderNative", skip_typescript)]
    pub fn empty_arrow_reader(&self) -> Result<crate::iomedia::JsBatchReader> {
        let schema = self.inner.clone().into_arrow_schema().map_err(napi_error)?;
        let reader = yggdryl::arrow::batch_reader(schema, []);
        Ok(crate::iomedia::JsBatchReader::from_core(
            reader,
            self.inner.name(),
        ))
    }

    /// Deserialize the structural JSON representation.
    ///
    /// One entry point for the three shapes a caller already has: the document
    /// as a string, the same document as the bytes it was read from, or the
    /// object `JSON.parse` already turned it into.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        crate::json_document(value)
            .and_then(serde_json::from_value)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Deserialize the structural JSON representation from bytes.
    ///
    /// The reading half of `toJSONBytes`. JavaScript names the byte reader
    /// rather than folding it into `fromJSON` because napi validates a union
    /// arm by type and cannot tell a typed array from any other object there,
    /// so one entry point taking both would silently take neither.
    #[napi(factory, js_name = "fromJSONBytes")]
    pub fn from_json_bytes(bytes: Uint8Array) -> Result<Self> {
        serde_json::from_slice(&bytes)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Physical field name.
    #[napi(getter)]
    pub fn name(&self) -> String {
        self.inner.name().to_owned()
    }

    /// Logical native datatype.
    #[napi(getter)]
    pub fn dtype(&self) -> JsDataType {
        JsDataType::from_core(self.inner.dtype().clone())
    }

    /// Return the field that describes both this one and `other`.
    ///
    /// The datatype is `DataType.mergeWith`'s answer; this adds the name
    /// (kept from the receiver), nullability (either side being nullable
    /// carries over), and metadata (the union, this field winning a clash).
    #[napi]
    pub fn merge_with(
        &self,
        other: ClassInstance<'_, JsField>,
        upscale: Option<bool>,
    ) -> Result<JsField> {
        self.inner
            .merge_with(&other.inner, upscale.unwrap_or(true))
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Every leaf under this node, named by its dotted path.
    ///
    /// Struct nesting flattens all the way down, and a leaf under a nullable
    /// ancestor is nullable. Collections are leaves: a list or a map is one
    /// column, and `explodeFields` is what reaches inside one. Every name this
    /// answers is one `fieldByPath` resolves.
    #[napi]
    pub fn unnest_fields(&self) -> Vec<JsField> {
        self.inner
            .unnest_fields()
            .into_iter()
            .map(JsField::from_core)
            .collect()
    }

    /// This node's children with every collection replaced by what it holds.
    ///
    /// A list answers its item, a map its entries, a dictionary or run-end
    /// node the values it encodes, and anything else itself - so the result
    /// names the same columns in the same order. One level only, so the depth
    /// is the caller's decision.
    #[napi]
    pub fn explode_fields(&self) -> Vec<JsField> {
        self.inner
            .explode_fields()
            .into_iter()
            .map(JsField::from_core)
            .collect()
    }

    /// Number of direct child fields.
    #[napi(getter)]
    pub fn field_len(&self) -> u32 {
        u32::try_from(self.inner.field_len()).unwrap_or(u32::MAX)
    }

    /// Return the child at an Array-compatible index, or `null`.
    #[napi]
    pub fn get_field_at(&self, index: i32) -> Option<JsField> {
        self.dtype().get_field_at(index)
    }

    /// Return the child a path names, or `null`.
    ///
    /// A child carrying the whole string wins before the string is decomposed
    /// on `.`, so a name containing a dot stays reachable.
    #[napi]
    pub fn get_field_by_path(&self, path: String) -> Option<JsField> {
        self.inner
            .get_field_by_path(&path)
            .cloned()
            .map(Self::from_core)
    }

    /// Return the child a position or a path names, or `null`.
    #[napi]
    pub fn get_field(&self, key: Either<i32, String>) -> Option<JsField> {
        match key {
            Either::A(index) => self.get_field_at(index),
            Either::B(path) => self.get_field_by_path(path),
        }
    }

    /// Return the child at an Array-compatible index, or throw.
    #[napi]
    pub fn field_at(&self, index: i32) -> Result<JsField> {
        self.dtype().field_at(index)
    }

    /// Return the child a path names, or throw.
    #[napi]
    pub fn field_by_path(&self, path: String) -> Result<JsField> {
        self.inner
            .field_by_path(&path)
            .cloned()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Return the child a position or a path names, or throw.
    #[napi]
    pub fn field(&self, key: Either<i32, String>) -> Result<JsField> {
        match key {
            Either::A(index) => self.field_at(index),
            Either::B(path) => self.field_by_path(path),
        }
    }

    /// Replace the child at an Array-compatible index.
    #[napi]
    pub fn set_field_at(&mut self, index: i32, child: ClassInstance<'_, JsField>) -> Result<()> {
        let len = i64::from(self.field_len());
        let at = i64::from(index);
        let resolved = if at < 0 { len + at } else { at };
        let position = usize::try_from(resolved)
            .ok()
            .filter(|at| *at < self.inner.field_len())
            .ok_or_else(|| napi_error(format_args!("no child at position {index}")))?;
        self.inner
            .set_field_at(position, child.inner.clone())
            .map_err(napi_error)
    }

    /// Replace the child a path names, appending an unresolved name.
    #[napi]
    pub fn set_field_by_path(
        &mut self,
        path: String,
        child: ClassInstance<'_, JsField>,
    ) -> Result<()> {
        self.inner
            .set_field_by_path(&path, child.inner.clone())
            .map_err(napi_error)
    }

    /// Replace the child a position or a path names.
    #[napi]
    pub fn set_field(
        &mut self,
        key: Either<i32, String>,
        child: ClassInstance<'_, JsField>,
    ) -> Result<()> {
        match key {
            Either::A(index) => self.set_field_at(index, child),
            Either::B(path) => self.set_field_by_path(path, child),
        }
    }

    /// Remove and return the child at an Array-compatible index.
    #[napi]
    pub fn remove_field_at(&mut self, index: i32) -> Result<JsField> {
        let len = i64::from(self.field_len());
        let at = i64::from(index);
        let resolved = if at < 0 { len + at } else { at };
        let position = usize::try_from(resolved)
            .ok()
            .filter(|at| *at < self.inner.field_len())
            .ok_or_else(|| napi_error(format_args!("no child at position {index}")))?;
        self.inner
            .remove_field_at(position)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Remove and return the child a path names.
    #[napi]
    pub fn remove_field_by_path(&mut self, path: String) -> Result<JsField> {
        self.inner
            .remove_field_by_path(&path)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Remove and return the child a position or a path names.
    #[napi]
    pub fn remove_field(&mut self, key: Either<i32, String>) -> Result<JsField> {
        match key {
            Either::A(index) => self.remove_field_at(index),
            Either::B(path) => self.remove_field_by_path(path),
        }
    }

    /// Whether values may be null.
    #[napi(getter)]
    pub fn nullable(&self) -> bool {
        self.inner.is_nullable()
    }

    /// Arrow IPC dictionary identifier, or `null` for non-dictionary fields.
    #[napi(getter)]
    pub fn dictionary_id(&self) -> Option<BigInt> {
        self.inner.dictionary_id().map(BigInt::from)
    }

    /// Arrow dictionary ordering flag, or `null` for non-dictionary fields.
    #[napi(getter)]
    pub fn dictionary_is_ordered(&self) -> Option<bool> {
        self.inner.dictionary_is_ordered()
    }

    /// Shared logical alias stored in Arrow-compatible metadata.
    #[napi(getter)]
    pub fn alias(&self) -> Option<String> {
        self.inner.alias().map(ToOwned::to_owned)
    }

    /// Shared human-readable comment stored in Arrow-compatible metadata.
    ///
    /// The one straight description a field carries, belonging to no protocol.
    /// Every protocol view falls back to it.
    #[napi(getter)]
    pub fn comment(&self) -> Option<String> {
        self.inner.comment().map(ToOwned::to_owned)
    }

    /// Shared human-readable display name stored in Arrow-compatible metadata.
    ///
    /// The label a reader is shown in place of the physical name, belonging to
    /// no protocol. Every protocol view falls back to it.
    #[napi(getter)]
    pub fn display(&self) -> Option<String> {
        self.inner.display().map(ToOwned::to_owned)
    }

    /// Arrow/Parquet signed 32-bit field identifier stored in metadata.
    #[napi(getter)]
    pub fn parquet_field_id(&self) -> Result<Option<i32>> {
        self.inner.parquet_field_id().map_err(napi_error)
    }

    /// The enum this field's ASCII values name, `null` when it declares none.
    ///
    /// The declaration is one `field:enum` document, so it reaches Arrow, a
    /// file, and another runtime as ordinary field metadata and comes back the
    /// enum that was written.
    #[napi(getter)]
    pub fn ascii_enum(&self) -> Result<Option<JsAsciiEnum>> {
        self.inner
            .ascii_enum()
            .map(|value| value.map(JsAsciiEnum::from_core))
            .map_err(napi_error)
    }

    /// Typed location URL stored canonically in Arrow-compatible metadata.
    #[napi(getter)]
    pub fn location(&self) -> Result<Option<JsUrl>> {
        self.inner
            .location()
            .map(|value| value.map(JsUrl::from_core))
            .map_err(napi_error)
    }

    /// Raw HTTP Accept field value.
    #[napi(getter)]
    pub fn accept(&self) -> Option<String> {
        self.inner.as_http().accept().map(ToOwned::to_owned)
    }

    /// Raw HTTP Accept-Encoding field value.
    #[napi(getter)]
    pub fn accept_encoding(&self) -> Option<String> {
        self.inner
            .as_http()
            .accept_encoding()
            .map(ToOwned::to_owned)
    }

    /// Raw HTTP Accept-Language field value.
    #[napi(getter)]
    pub fn accept_language(&self) -> Option<String> {
        self.inner
            .as_http()
            .accept_language()
            .map(ToOwned::to_owned)
    }

    /// Raw HTTP Accept-Ranges field value.
    #[napi(getter)]
    pub fn accept_ranges(&self) -> Option<String> {
        self.inner.as_http().accept_ranges().map(ToOwned::to_owned)
    }

    /// Raw HTTP Cache-Control field value.
    #[napi(getter)]
    pub fn cache_control(&self) -> Option<String> {
        self.inner.as_http().cache_control().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Disposition field value.
    #[napi(getter)]
    pub fn content_disposition(&self) -> Option<String> {
        self.inner
            .as_http()
            .content_disposition()
            .map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Encoding field value.
    #[napi(getter)]
    pub fn content_encoding(&self) -> Option<String> {
        self.inner
            .as_http()
            .content_encoding()
            .map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Language field value.
    #[napi(getter)]
    pub fn content_language(&self) -> Option<String> {
        self.inner
            .as_http()
            .content_language()
            .map(ToOwned::to_owned)
    }

    /// Exact HTTP Content-Length value.
    #[napi(getter)]
    pub fn content_length(&self) -> Result<Option<BigInt>> {
        self.inner
            .as_http()
            .content_length()
            .map(|value| value.map(BigInt::from))
            .map_err(napi_error)
    }

    /// Raw HTTP Content-Location field value.
    #[napi(getter)]
    pub fn content_location(&self) -> Option<String> {
        self.inner
            .as_http()
            .content_location()
            .map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Range field value.
    #[napi(getter)]
    pub fn content_range(&self) -> Option<String> {
        self.inner.as_http().content_range().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Type field value, including parameters.
    #[napi(getter)]
    pub fn content_type(&self) -> Option<String> {
        self.inner.as_http().content_type().map(ToOwned::to_owned)
    }

    /// Typed base MIME value derived from Content-Type.
    #[napi(getter)]
    pub fn mime_type(&self) -> Result<JsMimeType> {
        self.inner
            .as_http()
            .mime_type()
            .map(JsMimeType::from_core)
            .map_err(napi_error)
    }

    /// Typed media value derived from Content-Type and Content-Encoding.
    #[napi(getter)]
    pub fn media_type(&self) -> Result<JsMediaType> {
        self.inner
            .as_http()
            .media_type()
            .map(JsMediaType::from_core)
            .map_err(napi_error)
    }

    /// Raw HTTP `ETag` field value.
    #[napi(getter)]
    pub fn etag(&self) -> Option<String> {
        self.inner.as_http().etag().map(ToOwned::to_owned)
    }

    /// Raw HTTP Expires field value.
    #[napi(getter)]
    pub fn expires(&self) -> Option<String> {
        self.inner.as_http().expires().map(ToOwned::to_owned)
    }

    /// Raw HTTP Last-Modified field value.
    #[napi(getter)]
    pub fn last_modified(&self) -> Option<String> {
        self.inner.as_http().last_modified().map(ToOwned::to_owned)
    }

    /// Typed absolute HTTP Location URL.
    #[napi(getter)]
    pub fn http_location(&self) -> Result<Option<JsUrl>> {
        self.inner
            .as_http()
            .location()
            .map(|value| value.map(JsUrl::from_core))
            .map_err(napi_error)
    }

    /// Raw HTTP Range field value.
    #[napi(getter)]
    pub fn range(&self) -> Option<String> {
        self.inner.as_http().range().map(ToOwned::to_owned)
    }

    /// Raw HTTP Vary field value.
    #[napi(getter)]
    pub fn vary(&self) -> Option<String> {
        self.inner.as_http().vary().map(ToOwned::to_owned)
    }

    /// Number of metadata entries.
    #[napi(getter, js_name = "size")]
    pub fn metadata_len(&self) -> u32 {
        u32::try_from(self.inner.metadata_len()).unwrap_or(u32::MAX)
    }

    /// Change the physical name through the native cache-aware setter.
    #[napi]
    pub fn set_name(&mut self, name: String) {
        self.inner.set_name(name);
    }

    /// Change the datatype from a native wrapper or parsed expression.
    #[napi]
    pub fn set_dtype(
        &mut self,
        dtype: Either<ClassInstance<'_, JsDataType>, String>,
    ) -> Result<()> {
        self.inner
            .set_dtype(dtype_from_input(dtype)?)
            .map_err(napi_error)
    }

    /// Change nullability through the native validated setter.
    #[napi]
    pub fn set_nullable(&mut self, nullable: bool) {
        self.inner.set_nullable(nullable);
    }

    /// Change Arrow IPC dictionary options through the validated core setter.
    #[napi]
    pub fn set_dictionary_options(&mut self, id: BigInt, is_ordered: bool) -> Result<()> {
        let (id, lossless) = id.get_i64();
        if !lossless {
            return Err(Error::from_reason(
                "dictionary ID must fit in a signed 64-bit integer",
            ));
        }
        self.inner
            .set_dictionary_options(id, is_ordered)
            .map_err(napi_error)
    }

    /// Set the shared logical alias.
    #[napi]
    pub fn set_alias(&mut self, value: String) -> Result<()> {
        self.inner.set_alias(value).map_err(napi_error)
    }

    /// Remove and return the shared logical alias.
    #[napi]
    pub fn remove_alias(&mut self) -> Option<String> {
        self.inner.remove_alias()
    }

    /// Set the shared comment.
    #[napi]
    pub fn set_comment(&mut self, value: String) -> Result<()> {
        self.inner.set_comment(value).map_err(napi_error)
    }

    /// Remove and return the shared comment.
    #[napi]
    pub fn remove_comment(&mut self) -> Option<String> {
        self.inner.remove_comment()
    }

    /// Set the shared display name.
    #[napi]
    pub fn set_display(&mut self, value: String) -> Result<()> {
        self.inner.set_display(value).map_err(napi_error)
    }

    /// Remove and return the shared display name.
    #[napi]
    pub fn remove_display(&mut self) -> Option<String> {
        self.inner.remove_display()
    }

    /// Set the canonical Arrow/Parquet signed 32-bit field identifier.
    #[napi]
    pub fn set_parquet_field_id(&mut self, id: f64) -> Result<()> {
        self.inner.set_parquet_field_id(exact_i32(id, "field ID")?);
        Ok(())
    }

    /// Remove and return the Arrow/Parquet signed 32-bit field identifier.
    #[napi]
    pub fn remove_parquet_field_id(&mut self) -> Result<Option<i32>> {
        self.inner.remove_parquet_field_id().map_err(napi_error)
    }

    /// Declare the enum this field's ASCII values name.
    #[napi]
    pub fn set_ascii_enum(&mut self, value: &JsAsciiEnum) -> Result<()> {
        self.inner
            .set_ascii_enum(value.as_core())
            .map_err(napi_error)
    }

    /// Remove the declaration and return the enum it held.
    #[napi]
    pub fn remove_ascii_enum(&mut self) -> Result<Option<JsAsciiEnum>> {
        self.inner
            .remove_ascii_enum()
            .map(|value| value.map(JsAsciiEnum::from_core))
            .map_err(napi_error)
    }

    /// Set a typed location from any native identifier wrapper or URL string.
    #[napi]
    pub fn set_location(
        &mut self,
        value: Either4<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            String,
        >,
    ) -> Result<()> {
        self.inner.set_location(url_from_input(value)?);
        Ok(())
    }

    /// Remove and return the typed location URL.
    #[napi]
    pub fn remove_location(&mut self) -> Result<Option<JsUrl>> {
        self.inner
            .remove_location()
            .map(|value| value.map(JsUrl::from_core))
            .map_err(napi_error)
    }

    /// Set raw HTTP Accept metadata.
    #[napi]
    pub fn set_accept(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_accept(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Accept metadata.
    #[napi]
    pub fn remove_accept(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_accept()
    }

    /// Set raw HTTP Accept-Encoding metadata.
    #[napi]
    pub fn set_accept_encoding(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_accept_encoding(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Accept-Encoding metadata.
    #[napi]
    pub fn remove_accept_encoding(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_accept_encoding()
    }

    /// Set raw HTTP Accept-Language metadata.
    #[napi]
    pub fn set_accept_language(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_accept_language(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Accept-Language metadata.
    #[napi]
    pub fn remove_accept_language(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_accept_language()
    }

    /// Set raw HTTP Accept-Ranges metadata.
    #[napi]
    pub fn set_accept_ranges(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_accept_ranges(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Accept-Ranges metadata.
    #[napi]
    pub fn remove_accept_ranges(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_accept_ranges()
    }

    /// Set raw HTTP Cache-Control metadata.
    #[napi]
    pub fn set_cache_control(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_cache_control(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Cache-Control metadata.
    #[napi]
    pub fn remove_cache_control(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_cache_control()
    }

    /// Set raw HTTP Content-Disposition metadata.
    #[napi]
    pub fn set_content_disposition(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_content_disposition(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Disposition metadata.
    #[napi]
    pub fn remove_content_disposition(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_content_disposition()
    }

    /// Set raw HTTP Content-Encoding metadata.
    #[napi]
    pub fn set_content_encoding(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_content_encoding(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Encoding metadata.
    #[napi]
    pub fn remove_content_encoding(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_content_encoding()
    }

    /// Set raw HTTP Content-Language metadata.
    #[napi]
    pub fn set_content_language(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_content_language(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Language metadata.
    #[napi]
    pub fn remove_content_language(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_content_language()
    }

    /// Set exact unsigned HTTP Content-Length metadata.
    #[napi]
    pub fn set_content_length(&mut self, value: BigInt) -> Result<()> {
        let (negative, value, lossless) = value.get_u64();
        if negative || !lossless {
            return Err(Error::from_reason(
                "content length must fit in an unsigned 64-bit integer",
            ));
        }
        self.inner.as_http_mut().set_content_length(value);
        Ok(())
    }

    /// Remove exact HTTP Content-Length metadata.
    #[napi]
    pub fn remove_content_length(&mut self) -> Result<Option<BigInt>> {
        self.inner
            .as_http_mut()
            .remove_content_length()
            .map(|value| value.map(BigInt::from))
            .map_err(napi_error)
    }

    /// Set raw HTTP Content-Location metadata.
    #[napi]
    pub fn set_content_location(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_content_location(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Location metadata.
    #[napi]
    pub fn remove_content_location(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_content_location()
    }

    /// Set raw HTTP Content-Range metadata.
    #[napi]
    pub fn set_content_range(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_content_range(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Range metadata.
    #[napi]
    pub fn remove_content_range(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_content_range()
    }

    /// Set raw HTTP Content-Type metadata.
    #[napi]
    pub fn set_content_type(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_content_type(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Type metadata.
    #[napi]
    pub fn remove_content_type(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_content_type()
    }

    /// Set a bare typed MIME value while preserving Content-Encoding.
    #[napi]
    pub fn set_mime_type(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_mime_type(mime_type_from_input(value)?);
        Ok(())
    }

    /// Remove and return the prior typed MIME value.
    #[napi]
    pub fn remove_mime_type(&mut self) -> Result<Option<JsMimeType>> {
        self.inner
            .as_http_mut()
            .remove_mime_type()
            .map(|value| value.map(JsMimeType::from_core))
            .map_err(napi_error)
    }

    /// Atomically project a typed media value to both HTTP content headers.
    #[napi]
    pub fn set_media_type(&mut self, value: MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_media_type(media_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Atomically remove and return both prior typed HTTP media headers.
    #[napi]
    pub fn remove_media_type(&mut self) -> Result<Option<JsMediaType>> {
        self.inner
            .as_http_mut()
            .remove_media_type()
            .map(|value| value.map(JsMediaType::from_core))
            .map_err(napi_error)
    }

    /// Set raw HTTP `ETag` metadata.
    #[napi]
    pub fn set_etag(&mut self, value: String) -> Result<()> {
        self.inner.as_http_mut().set_etag(value).map_err(napi_error)
    }

    /// Remove raw HTTP `ETag` metadata.
    #[napi]
    pub fn remove_etag(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_etag()
    }

    /// Set raw HTTP Expires metadata.
    #[napi]
    pub fn set_expires(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_expires(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Expires metadata.
    #[napi]
    pub fn remove_expires(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_expires()
    }

    /// Set raw HTTP Last-Modified metadata.
    #[napi]
    pub fn set_last_modified(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_last_modified(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Last-Modified metadata.
    #[napi]
    pub fn remove_last_modified(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_last_modified()
    }

    /// Set typed absolute HTTP Location metadata.
    #[napi]
    pub fn set_http_location(
        &mut self,
        value: Either4<
            ClassInstance<'_, JsUrl>,
            ClassInstance<'_, JsUri>,
            ClassInstance<'_, JsUrn>,
            String,
        >,
    ) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_location(url_from_input(value)?);
        Ok(())
    }

    /// Remove and return typed absolute HTTP Location metadata.
    #[napi]
    pub fn remove_http_location(&mut self) -> Result<Option<JsUrl>> {
        self.inner
            .as_http_mut()
            .remove_location()
            .map(|value| value.map(JsUrl::from_core))
            .map_err(napi_error)
    }

    /// Set raw HTTP Range metadata.
    #[napi]
    pub fn set_range(&mut self, value: String) -> Result<()> {
        self.inner
            .as_http_mut()
            .set_range(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Range metadata.
    #[napi]
    pub fn remove_range(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_range()
    }

    /// Set raw HTTP Vary metadata.
    #[napi]
    pub fn set_vary(&mut self, value: String) -> Result<()> {
        self.inner.as_http_mut().set_vary(value).map_err(napi_error)
    }

    /// Remove raw HTTP Vary metadata.
    #[napi]
    pub fn remove_vary(&mut self) -> Option<String> {
        self.inner.as_http_mut().remove_vary()
    }

    /// Read one `scheme:name` protocol property.
    #[napi]
    pub fn get_property(&self, scheme: String, name: String) -> Result<Option<String>> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        Ok(self
            .inner
            .get_property(&scheme, &name)
            .map(ToOwned::to_owned))
    }

    /// Test whether one `scheme:name` protocol property exists.
    #[napi]
    pub fn has_property(&self, scheme: String, name: String) -> Result<bool> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        Ok(self.inner.has_property(&scheme, &name))
    }

    /// Insert or replace one `scheme:name` protocol property.
    #[napi]
    pub fn set_property(
        &mut self,
        scheme: String,
        name: String,
        value: String,
    ) -> Result<Option<String>> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        self.inner
            .set_property(&scheme, &name, value)
            .map_err(napi_error)
    }

    /// Remove and return one `scheme:name` protocol property.
    #[napi]
    pub fn remove_property(&mut self, scheme: String, name: String) -> Result<Option<String>> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        Ok(self.inner.remove_property(&scheme, &name))
    }

    /// Protocol property suffix/value entries in deterministic lexical order.
    #[napi]
    pub fn property_iter(&self, scheme: String) -> Result<Vec<MetadataEntry>> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        Ok(self
            .inner
            .property_iter(&scheme)
            .map(|(key, value)| MetadataEntry {
                key: key.to_owned(),
                value: value.to_owned(),
            })
            .collect())
    }

    /// Remove every property for one protocol without affecting shared keys.
    #[napi]
    pub fn clear_properties(&mut self, scheme: String) -> Result<()> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        self.inner.clear_properties(&scheme);
        Ok(())
    }

    /// Return a live view of one protocol's properties on this field.
    ///
    /// The scheme accepts every spelling `getProperty` does, and the view keeps
    /// reading and writing this same field.
    #[napi]
    pub fn protocol(
        &self,
        reference: Reference<JsField>,
        scheme: String,
    ) -> Result<JsProtocolField> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        Ok(JsProtocolField::new(reference, scheme))
    }

    /// The live HTTP and HTTPS representation property view.
    #[napi(getter)]
    pub fn http(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::HTTP)
    }

    /// The live file protocol property view.
    #[napi(getter)]
    pub fn file(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::FILE)
    }

    /// The live uniform resource name property view.
    #[napi(getter)]
    pub fn urn(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::URN)
    }

    /// The live short-spelling `PostgreSQL` property view.
    #[napi(getter)]
    pub fn postgres(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::POSTGRES)
    }

    /// The live long-spelling `PostgreSQL` property view.
    #[napi(getter)]
    pub fn postgresql(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::POSTGRESQL)
    }

    /// The live `MySQL` property view.
    #[napi(getter)]
    pub fn mysql(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::MYSQL)
    }

    /// The live Arrow property view.
    #[napi(getter)]
    pub fn arrow(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::ARROW)
    }

    /// The live generic SQL property view.
    #[napi(getter)]
    pub fn sql(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::SQL)
    }

    /// The live AWS Glue property view.
    #[napi(getter)]
    pub fn glue(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::GLUE)
    }

    /// The live Apache Iceberg property view.
    #[napi(getter)]
    pub fn iceberg(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::ICEBERG)
    }

    /// The live Financial Information eXchange property view.
    #[napi(getter)]
    pub fn fix(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::FIX)
    }

    /// The live Yggdryl field property view.
    ///
    /// Named for the namespace it exposes rather than plain `field`, which on
    /// a schema node reaches a nested child.
    #[napi(getter)]
    pub fn field_properties(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::FIELD)
    }

    /// The live Amazon S3 property view.
    #[napi(getter)]
    pub fn s3(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::S3)
    }

    /// The live Google Cloud Storage property view.
    #[napi(getter)]
    pub fn gs(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::GS)
    }

    /// The live Azure Blob Storage property view.
    #[napi(getter)]
    pub fn az(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::AZ)
    }

    /// The live Apache Spark property view.
    #[napi(getter)]
    pub fn spark(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::SPARK)
    }

    /// The live Polars property view.
    #[napi(getter)]
    pub fn polars(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::POLARS)
    }

    /// The live pandas property view.
    #[napi(getter)]
    pub fn pandas(&self, reference: Reference<JsField>) -> JsProtocolField {
        JsProtocolField::new(reference, CoreScheme::PANDAS)
    }

    /// Whether this field carries the values a path spells out.
    #[napi(getter)]
    pub fn is_partition(&self) -> bool {
        self.inner.is_partition()
    }

    /// Mark or unmark this field as one a path spells out.
    #[napi]
    pub fn set_partition(&mut self, partition: bool) {
        self.inner.set_partition(partition);
    }

    /// Return a copy of this field with the partition marker set.
    #[napi]
    pub fn with_partition(&self, partition: bool) -> Self {
        Self::from_core(self.inner.clone().with_partition(partition))
    }

    /// The struct children that partition the rows, in declaration order.
    #[napi]
    pub fn partition_fields(&self) -> Vec<JsField> {
        self.inner
            .partition_fields()
            .cloned()
            .map(Self::from_core)
            .collect()
    }

    /// The names of the struct children that partition the rows.
    #[napi]
    pub fn partition_field_names(&self) -> Vec<String> {
        self.inner
            .partition_field_names()
            .map(ToOwned::to_owned)
            .collect()
    }

    /// Number of struct children that partition the rows.
    #[napi(getter)]
    pub fn partition_field_len(&self) -> u32 {
        u32::try_from(self.inner.partition_field_len()).unwrap_or(u32::MAX)
    }

    /// Whether any struct child partitions the rows.
    #[napi(getter)]
    pub fn has_partition_fields(&self) -> bool {
        self.inner.has_partition_fields()
    }

    /// Return this struct root holding only the columns a path spells out.
    #[napi]
    pub fn only_partition_fields(&self) -> Result<Self> {
        self.inner
            .only_partition_fields()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Return this struct root without the columns a path spells out.
    #[napi]
    pub fn without_partition_fields(&self) -> Result<Self> {
        self.inner
            .without_partition_fields()
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Return this struct root with the named children marked as partitions.
    #[napi]
    pub fn with_partition_fields(&self, names: Vec<String>) -> Result<Self> {
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        self.inner
            .with_partition_fields(&names)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Read one metadata value without materializing the metadata collection.
    #[napi]
    pub fn get(&self, key: String) -> Option<String> {
        self.inner.get_metadata(&key).map(ToOwned::to_owned)
    }

    /// Insert or replace one metadata value through the native Field API.
    #[napi]
    pub fn set(&mut self, key: String, value: String) -> Result<()> {
        self.inner
            .insert_metadata(key, value)
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Remove one metadata key and report whether it existed.
    #[napi]
    pub fn delete(&mut self, key: String) -> bool {
        self.inner.remove_metadata(&key).is_some()
    }

    /// Test whether a metadata key exists.
    #[napi]
    pub fn has(&self, key: String) -> bool {
        self.inner.has_metadata(&key)
    }

    /// Metadata keys in deterministic lexical order.
    #[napi]
    pub fn keys(&self) -> Vec<String> {
        self.inner
            .metadata_iter()
            .map(|(key, _)| key.to_owned())
            .collect()
    }

    /// Metadata values in deterministic lexical-key order.
    #[napi]
    pub fn values(&self) -> Vec<String> {
        self.inner
            .metadata_iter()
            .map(|(_, value)| value.to_owned())
            .collect()
    }

    /// Metadata entries in deterministic lexical-key order.
    #[napi]
    pub fn entries(&self) -> Vec<MetadataEntry> {
        self.inner
            .metadata_iter()
            .map(|(key, value)| MetadataEntry {
                key: key.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    }

    /// Bulk-overlay metadata from an object, entry array, or loader-adapted Map.
    #[napi]
    pub fn update(
        &mut self,
        values: Either<Vec<MetadataEntry>, HashMap<String, String>>,
    ) -> Result<()> {
        self.apply_metadata(values)
    }

    /// Remove all metadata without allocating.
    #[napi]
    pub fn clear(&mut self) {
        self.inner.clear_metadata();
    }

    /// Recursive native equality, optionally ignoring Field metadata.
    #[napi]
    pub fn equals(&self, other: &JsField, with_metadata: Option<bool>) -> bool {
        self.inner
            .equals(&other.inner, with_metadata.unwrap_or(true))
    }

    /// Return an iterator over stable recursive difference lines.
    #[napi(js_name = "_showDiffs", skip_typescript)]
    pub fn show_diffs_native(
        &self,
        other: &JsField,
        with_metadata: Option<bool>,
        return_equal: Option<bool>,
    ) -> JsDifferenceIterator {
        JsDifferenceIterator::from_fields(
            &self.inner,
            &other.inner,
            with_metadata.unwrap_or(true),
            return_equal.unwrap_or(false),
        )
    }

    /// Join recursive differences, or return `✓ equal`.
    #[napi]
    pub fn show_diff(
        &self,
        other: &JsField,
        with_metadata: Option<bool>,
        return_equal: Option<bool>,
    ) -> String {
        self.inner.show_diff(
            &other.inner,
            with_metadata.unwrap_or(true),
            return_equal.unwrap_or(true),
        )
    }

    /// Total native ordering: `-1`, `0`, or `1`.
    #[napi]
    pub fn compare(&self, other: &JsField) -> i32 {
        ordering_value(self.inner.cmp(&other.inner))
    }

    /// Deterministic XXH3-64 hash of canonical native display text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Materialize the bounded canonical Field default through the exact native
    /// schema-guided JavaScript scalar projection.
    #[napi(js_name = "_defaultJSValueNative", skip_typescript)]
    pub fn default_js_value_native<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        let value = self.inner.default_value().map_err(napi_error)?;
        field_value_to_js(env, &self.inner, &value)
    }

    /// Internal one-row copied IPC projection for Apache Arrow JS scalar
    /// materialization.
    #[napi(js_name = "_defaultArrowScalarIpcNative", skip_typescript)]
    pub fn default_arrow_scalar_ipc_native(&self) -> Result<napi::bindgen_prelude::Buffer> {
        let array = self.inner.default_arrow_array().map_err(napi_error)?;
        arrow_scalar_to_ipc(&self.inner, array)
    }

    /// Recursively normalize this exact Field for one closed compatibility
    /// target without changing the current wrapper.
    #[napi(js_name = "intoSchemeCompat", skip_typescript)]
    pub fn into_scheme_compat(&self, target: String) -> Result<Self> {
        let target = CoreScheme::from_str(&target).map_err(napi_error)?;
        self.inner
            .clone()
            .into_scheme_compat(&target)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Make a cheap clone preserving shared nested state and Arrow cache.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }

    /// Serialize to structural JSON bytes.
    ///
    /// The same document `toJSON` renders, encoded rather than decoded, for a
    /// caller writing it straight to a file or a socket. `fromJSON` reads
    /// these bytes back without being told which shape it got.
    #[napi(js_name = "toJSONBytes")]
    pub fn js_json_bytes(&self) -> Result<Buffer> {
        serde_json::to_vec(&self.inner)
            .map(Buffer::from)
            .map_err(napi_error)
    }
}

/// One protocol's properties on a field, read and written by bare name.
///
/// The view is live: it holds the `Field` it was taken from and answers every
/// call through it, so a write through the view is visible on the field and a
/// write on the field is visible through the view. Nothing is snapshotted, and
/// the `scheme:` prefix is applied once, by the view.
#[napi(js_name = "ProtocolField")]
pub struct JsProtocolField {
    field: Reference<JsField>,
    scheme: CoreScheme,
}

impl JsProtocolField {
    fn new(field: Reference<JsField>, scheme: CoreScheme) -> Self {
        Self { field, scheme }
    }

    /// Borrow the core read view, which reads the live field's metadata.
    fn view(&self) -> CoreProtocolField<'_> {
        self.field.inner.protocol(&self.scheme)
    }

    /// Refuse a typed FIX property on a view of another protocol.
    ///
    /// The typed vocabulary belongs to one protocol, so it is answered only by
    /// the view `field.fix` returns; every other scheme reads and writes its
    /// own properties through the Map-like surface above.
    fn require_fix(&self, env: Env, property: &str) -> Result<()> {
        if self.scheme == CoreScheme::FIX {
            return Ok(());
        }
        Err(napi_type_error(
            env,
            format!(
                "{property} is a fix property, and this is a {} view",
                self.scheme.as_str()
            ),
        ))
    }
}

#[napi]
impl JsProtocolField {
    /// The protocol this view remembers, in its canonical lowercase spelling.
    #[napi(getter)]
    pub fn scheme(&self) -> String {
        self.scheme.as_str().to_owned()
    }

    /// The canonical key prefix this view applies.
    #[napi(getter)]
    pub fn prefix(&self) -> String {
        self.view().prefix().to_owned()
    }

    /// The full metadata key one bare property name is stored under.
    #[napi]
    pub fn key(&self, name: String) -> String {
        self.view().key(&name)
    }

    /// Number of properties this protocol holds.
    #[napi(getter, js_name = "size")]
    pub fn property_len(&self) -> u32 {
        u32::try_from(self.view().len()).unwrap_or(u32::MAX)
    }

    /// Read one property value by its bare name.
    #[napi]
    pub fn get(&self, name: String) -> Option<String> {
        self.view().get(&name).map(ToOwned::to_owned)
    }

    /// Test whether one property exists.
    #[napi]
    pub fn has(&self, name: String) -> bool {
        self.view().contains_key(&name)
    }

    /// Insert or replace one property through the live field.
    #[napi]
    pub fn set(&mut self, name: String, value: String) -> Result<()> {
        self.field
            .inner
            .protocol_mut(&self.scheme)
            .insert(&name, value)
            .map(|_| ())
            .map_err(napi_error)
    }

    /// Remove one property and report whether it existed.
    #[napi]
    pub fn delete(&mut self, name: String) -> bool {
        self.field
            .inner
            .protocol_mut(&self.scheme)
            .remove(&name)
            .is_some()
    }

    /// Bare property names in deterministic lexical order.
    #[napi]
    pub fn keys(&self) -> Vec<String> {
        self.view()
            .iter()
            .map(|(name, _)| name.to_owned())
            .collect()
    }

    /// Property values in deterministic lexical-name order.
    #[napi]
    pub fn values(&self) -> Vec<String> {
        self.view()
            .iter()
            .map(|(_, value)| value.to_owned())
            .collect()
    }

    /// Bare name/value entries in deterministic lexical order.
    #[napi]
    pub fn entries(&self) -> Vec<MetadataEntry> {
        self.view()
            .iter()
            .map(|(name, value)| MetadataEntry {
                key: name.to_owned(),
                value: value.to_owned(),
            })
            .collect()
    }

    /// This protocol's comment, falling back to the field's straight one.
    ///
    /// `get`, iteration and `length` stay literal about what this protocol
    /// carries; the fallback lives here so a view never reports a property
    /// that iterating it would not yield.
    #[napi(getter)]
    pub fn comment(&self) -> Option<String> {
        self.view().comment().map(ToOwned::to_owned)
    }

    /// This protocol's display name, falling back to the field's straight one.
    ///
    /// The fallback lives here for the reason [`Self::comment`]'s does.
    #[napi(getter)]
    pub fn display(&self) -> Option<String> {
        self.view().display().map(ToOwned::to_owned)
    }

    /// The dictionary this field belongs to, on the `fix` view.
    ///
    /// A branch crosses as text: `'standard'` is the FIX specification's own
    /// dictionary and what an absent `fix:branch` means, and assigning it
    /// removes the key rather than storing it. A spelling that is not a branch
    /// throws the native parse failure, and a refusal - a tag the specification
    /// assigns cannot move to another dictionary - leaves the field unchanged.
    #[napi(getter)]
    pub fn branch(&self, env: Env) -> Result<String> {
        self.require_fix(env, "branch")?;
        self.field
            .inner
            .as_fix()
            .branch()
            .map(|branch| branch.as_str().to_owned())
            .map_err(napi_error)
    }

    /// Record the dictionary this field belongs to.
    #[napi(setter)]
    pub fn set_branch(&mut self, env: Env, value: String) -> Result<()> {
        self.require_fix(env, "branch")?;
        let branch = branch_from_js(&value)?;
        self.field
            .inner
            .as_fix_mut()
            .set_branch(&branch)
            .map_err(napi_error)
    }

    /// This field's identity, `branch:tag`, on the `fix` view.
    ///
    /// Derived from the branch and the canonical tag on every read and never
    /// stored, so it is `null` exactly when `fix:tag` is absent. Assigning one
    /// moves both halves at once, which is the only ordering-safe way to move a
    /// field between dictionaries.
    #[napi(getter)]
    pub fn id(&self, env: Env) -> Result<Option<String>> {
        self.require_fix(env, "id")?;
        Ok(self
            .field
            .inner
            .as_fix()
            .id()
            .map_err(napi_error)?
            .map(|id| id.to_string()))
    }

    /// Record both halves of this field's identity at once.
    #[napi(setter)]
    pub fn set_id(&mut self, env: Env, value: String) -> Result<()> {
        self.require_fix(env, "id")?;
        let id = id_from_js(&value)?;
        self.field
            .inner
            .as_fix_mut()
            .set_id(&id)
            .map_err(napi_error)
    }

    /// The canonical FIX tag, on the `fix` view.
    ///
    /// Reads and writes `fix:tag` through the core's own typed accessors, so
    /// the property name is never spelled at a call site. `view.delete('tag')`
    /// removes it, the way every other property is removed. A tag below
    /// `fix.STANDARD_TAG_LIMIT` is the FIX specification's own, so a field in
    /// another branch cannot claim it.
    #[napi(getter)]
    pub fn tag(&self, env: Env) -> Result<Option<i32>> {
        self.require_fix(env, "tag")?;
        self.field.inner.as_fix().tag().map_err(napi_error)
    }

    /// Record the canonical FIX tag, rejecting anything but an exact `i32`.
    #[napi(setter)]
    pub fn set_tag(&mut self, env: Env, value: f64) -> Result<()> {
        self.require_fix(env, "tag")?;
        let tag = exact_i32(value, "tag")?;
        self.field
            .inner
            .as_fix_mut()
            .set_tag(tag)
            .map_err(napi_error)
    }

    /// The alternate tags, highest priority first.
    ///
    /// An absent property is an empty array, and assigning an empty one
    /// removes it: a field states alternate tags only when it has them.
    #[napi(getter)]
    pub fn tags(&self, env: Env) -> Result<Vec<i32>> {
        self.require_fix(env, "tags")?;
        self.field.inner.as_fix().tags().map_err(napi_error)
    }

    /// Record the alternate tags; an empty array removes the property.
    #[napi(setter)]
    pub fn set_tags(&mut self, env: Env, values: Vec<f64>) -> Result<()> {
        self.require_fix(env, "tags")?;
        let tags = values
            .into_iter()
            .map(|value| exact_i32(value, "tags"))
            .collect::<Result<Vec<i32>>>()?;
        self.field
            .inner
            .as_fix_mut()
            .set_tags(&tags)
            .map_err(napi_error)
    }

    /// The alternate names, highest priority first.
    ///
    /// Assigning an empty array removes the property.
    #[napi(getter)]
    pub fn aliases(&self, env: Env) -> Result<Vec<String>> {
        self.require_fix(env, "aliases")?;
        Ok(self
            .field
            .inner
            .as_fix()
            .aliases()
            .map(ToOwned::to_owned)
            .collect())
    }

    /// Record the aliases; an empty array removes the property.
    #[napi(setter)]
    pub fn set_aliases(&mut self, env: Env, values: Vec<String>) -> Result<()> {
        self.require_fix(env, "aliases")?;
        self.field
            .inner
            .as_fix_mut()
            .set_aliases(values)
            .map_err(napi_error)
    }

    /// The specification's own wording for this field.
    #[napi(getter)]
    pub fn description(&self, env: Env) -> Result<Option<String>> {
        self.require_fix(env, "description")?;
        Ok(self
            .field
            .inner
            .as_fix()
            .description()
            .map(ToOwned::to_owned))
    }

    /// Record the specification's own wording for this field.
    #[napi(setter)]
    pub fn set_description(&mut self, env: Env, value: String) -> Result<()> {
        self.require_fix(env, "description")?;
        self.field
            .inner
            .as_fix_mut()
            .set_description(value)
            .map_err(napi_error)
    }

    /// Merge another protocol view's properties into this one, in place.
    ///
    /// A name this view already carries keeps its value, so the merge only
    /// ever adds. Properties of other protocols are untouched.
    #[napi]
    pub fn merge_with(&mut self, other: &JsProtocolField) -> Result<()> {
        // Read both sides before writing: `other` may view this same field.
        let additions: Vec<(String, String)> = {
            let held = self.view();
            other
                .view()
                .iter()
                .filter(|(name, _)| held.get(name).is_none())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect()
        };
        self.field
            .inner
            .protocol_mut(&self.scheme)
            .update(additions)
            .map_err(napi_error)
    }

    /// Overlay several properties atomically, keeping the ones not named.
    #[napi]
    pub fn update(
        &mut self,
        values: Either<Vec<MetadataEntry>, HashMap<String, String>>,
    ) -> Result<()> {
        self.field
            .inner
            .protocol_mut(&self.scheme)
            .update(metadata_pairs(values))
            .map_err(napi_error)
    }

    /// Remove every property of this protocol, leaving shared keys alone.
    #[napi]
    pub fn clear(&mut self) {
        self.field.inner.protocol_mut(&self.scheme).clear();
    }

    /// Render this protocol's bare names as the native JSON object text.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> String {
        self.view().to_string()
    }

    /// Serialize this protocol's bare names as a JSON object.
    #[napi(js_name = "toJSON")]
    pub fn js_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.view()
                .iter()
                .map(|(name, value)| (name.to_owned(), serde_json::Value::String(value.to_owned())))
                .collect(),
        )
    }
}
