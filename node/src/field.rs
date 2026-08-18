//! Node.js view of the native Field domain.

use std::collections::HashMap;

use napi::bindgen_prelude::{
    BigInt, Buffer, ClassInstance, Either, Either4, Env, Error, Reference, Result, Uint8Array,
    Unknown,
};
use napi_derive::napi;
use yggdryl::{Field as CoreField, ProtocolMetadata as CoreProtocolMetadata, Scheme as CoreScheme};

use crate::{
    JsDifferenceIterator,
    datatype::{JsDataType, data_type_from_input},
    exact_i32,
    media::{
        JsMediaType, JsMimeType, MediaTypeInput, MimeTypeInput, media_type_from_input,
        mime_type_from_input,
    },
    napi_error, ordering_value,
    record::arrow_scalar_to_ipc,
    record::{field_value_schema, field_value_to_js},
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

type MetadataInput = Either<Vec<MetadataEntry>, HashMap<String, String>>;

fn metadata_pairs(value: MetadataInput) -> HashMap<String, String> {
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
        data_type: Option<Either<ClassInstance<'_, JsDataType>, String>>,
        nullable: Option<bool>,
        metadata: Option<Either<Vec<MetadataEntry>, HashMap<String, String>>>,
    ) -> Result<Self> {
        let should_override_nullable = data_type.is_none();
        let mut field = match (value, data_type) {
            (Either::A(field), None) => field.inner.clone(),
            (Either::A(_), Some(_)) => {
                return Err(Error::from_reason(
                    "a cloned Field cannot be combined with a DataType",
                ));
            }
            (Either::B(value), None) => CoreField::from_str(&value).map_err(napi_error)?,
            (Either::B(name), Some(data_type)) => {
                let field = CoreField::new(
                    name,
                    data_type_from_input(data_type)?,
                    nullable.unwrap_or(true),
                );
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
        let schema = yggdryl::arrow::schema_from_field(&self.inner).map_err(napi_error)?;
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

    /// Deserialize the native structural JSON representation.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: serde_json::Value) -> Result<Self> {
        serde_json::from_value(value)
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
    pub fn data_type(&self) -> JsDataType {
        JsDataType::from_core(self.inner.data_type().clone())
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

    /// Shared catalog name stored in Arrow-compatible metadata.
    #[napi(getter)]
    pub fn catalog_name(&self) -> Option<String> {
        self.inner.catalog_name().map(ToOwned::to_owned)
    }

    /// Shared schema name stored in Arrow-compatible metadata.
    #[napi(getter)]
    pub fn schema_name(&self) -> Option<String> {
        self.inner.schema_name().map(ToOwned::to_owned)
    }

    /// Shared table name stored in Arrow-compatible metadata.
    #[napi(getter)]
    pub fn table_name(&self) -> Option<String> {
        self.inner.table_name().map(ToOwned::to_owned)
    }

    /// Arrow/Parquet signed 32-bit field identifier stored in metadata.
    #[napi(getter)]
    pub fn parquet_field_id(&self) -> Result<Option<i32>> {
        self.inner.parquet_field_id().map_err(napi_error)
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
        self.inner.accept().map(ToOwned::to_owned)
    }

    /// Raw HTTP Accept-Encoding field value.
    #[napi(getter)]
    pub fn accept_encoding(&self) -> Option<String> {
        self.inner.accept_encoding().map(ToOwned::to_owned)
    }

    /// Raw HTTP Accept-Language field value.
    #[napi(getter)]
    pub fn accept_language(&self) -> Option<String> {
        self.inner.accept_language().map(ToOwned::to_owned)
    }

    /// Raw HTTP Accept-Ranges field value.
    #[napi(getter)]
    pub fn accept_ranges(&self) -> Option<String> {
        self.inner.accept_ranges().map(ToOwned::to_owned)
    }

    /// Raw HTTP Cache-Control field value.
    #[napi(getter)]
    pub fn cache_control(&self) -> Option<String> {
        self.inner.cache_control().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Disposition field value.
    #[napi(getter)]
    pub fn content_disposition(&self) -> Option<String> {
        self.inner.content_disposition().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Encoding field value.
    #[napi(getter)]
    pub fn content_encoding(&self) -> Option<String> {
        self.inner.content_encoding().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Language field value.
    #[napi(getter)]
    pub fn content_language(&self) -> Option<String> {
        self.inner.content_language().map(ToOwned::to_owned)
    }

    /// Exact HTTP Content-Length value.
    #[napi(getter)]
    pub fn content_length(&self) -> Result<Option<BigInt>> {
        self.inner
            .content_length()
            .map(|value| value.map(BigInt::from))
            .map_err(napi_error)
    }

    /// Raw HTTP Content-Location field value.
    #[napi(getter)]
    pub fn content_location(&self) -> Option<String> {
        self.inner.content_location().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Range field value.
    #[napi(getter)]
    pub fn content_range(&self) -> Option<String> {
        self.inner.content_range().map(ToOwned::to_owned)
    }

    /// Raw HTTP Content-Type field value, including parameters.
    #[napi(getter)]
    pub fn content_type(&self) -> Option<String> {
        self.inner.content_type().map(ToOwned::to_owned)
    }

    /// Typed base MIME value derived from Content-Type.
    #[napi(getter)]
    pub fn mime_type(&self) -> Result<JsMimeType> {
        self.inner
            .mime_type()
            .map(JsMimeType::from_core)
            .map_err(napi_error)
    }

    /// Typed media value derived from Content-Type and Content-Encoding.
    #[napi(getter)]
    pub fn media_type(&self) -> Result<JsMediaType> {
        self.inner
            .media_type()
            .map(JsMediaType::from_core)
            .map_err(napi_error)
    }

    /// Raw HTTP `ETag` field value.
    #[napi(getter)]
    pub fn etag(&self) -> Option<String> {
        self.inner.etag().map(ToOwned::to_owned)
    }

    /// Raw HTTP Expires field value.
    #[napi(getter)]
    pub fn expires(&self) -> Option<String> {
        self.inner.expires().map(ToOwned::to_owned)
    }

    /// Raw HTTP Last-Modified field value.
    #[napi(getter)]
    pub fn last_modified(&self) -> Option<String> {
        self.inner.last_modified().map(ToOwned::to_owned)
    }

    /// Typed absolute HTTP Location URL.
    #[napi(getter)]
    pub fn http_location(&self) -> Result<Option<JsUrl>> {
        self.inner
            .http_location()
            .map(|value| value.map(JsUrl::from_core))
            .map_err(napi_error)
    }

    /// Raw HTTP Range field value.
    #[napi(getter)]
    pub fn range(&self) -> Option<String> {
        self.inner.range().map(ToOwned::to_owned)
    }

    /// Raw HTTP Vary field value.
    #[napi(getter)]
    pub fn vary(&self) -> Option<String> {
        self.inner.vary().map(ToOwned::to_owned)
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
    pub fn set_data_type(
        &mut self,
        data_type: Either<ClassInstance<'_, JsDataType>, String>,
    ) -> Result<()> {
        self.inner
            .set_data_type(data_type_from_input(data_type)?)
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

    /// Set the shared catalog name.
    #[napi]
    pub fn set_catalog_name(&mut self, value: String) -> Result<()> {
        self.inner.set_catalog_name(value).map_err(napi_error)
    }

    /// Remove and return the shared catalog name.
    #[napi]
    pub fn remove_catalog_name(&mut self) -> Option<String> {
        self.inner.remove_catalog_name()
    }

    /// Set the shared schema name.
    #[napi]
    pub fn set_schema_name(&mut self, value: String) -> Result<()> {
        self.inner.set_schema_name(value).map_err(napi_error)
    }

    /// Remove and return the shared schema name.
    #[napi]
    pub fn remove_schema_name(&mut self) -> Option<String> {
        self.inner.remove_schema_name()
    }

    /// Set the shared table name.
    #[napi]
    pub fn set_table_name(&mut self, value: String) -> Result<()> {
        self.inner.set_table_name(value).map_err(napi_error)
    }

    /// Remove and return the shared table name.
    #[napi]
    pub fn remove_table_name(&mut self) -> Option<String> {
        self.inner.remove_table_name()
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
        self.inner.set_accept(value).map_err(napi_error)
    }

    /// Remove raw HTTP Accept metadata.
    #[napi]
    pub fn remove_accept(&mut self) -> Option<String> {
        self.inner.remove_accept()
    }

    /// Set raw HTTP Accept-Encoding metadata.
    #[napi]
    pub fn set_accept_encoding(&mut self, value: String) -> Result<()> {
        self.inner.set_accept_encoding(value).map_err(napi_error)
    }

    /// Remove raw HTTP Accept-Encoding metadata.
    #[napi]
    pub fn remove_accept_encoding(&mut self) -> Option<String> {
        self.inner.remove_accept_encoding()
    }

    /// Set raw HTTP Accept-Language metadata.
    #[napi]
    pub fn set_accept_language(&mut self, value: String) -> Result<()> {
        self.inner.set_accept_language(value).map_err(napi_error)
    }

    /// Remove raw HTTP Accept-Language metadata.
    #[napi]
    pub fn remove_accept_language(&mut self) -> Option<String> {
        self.inner.remove_accept_language()
    }

    /// Set raw HTTP Accept-Ranges metadata.
    #[napi]
    pub fn set_accept_ranges(&mut self, value: String) -> Result<()> {
        self.inner.set_accept_ranges(value).map_err(napi_error)
    }

    /// Remove raw HTTP Accept-Ranges metadata.
    #[napi]
    pub fn remove_accept_ranges(&mut self) -> Option<String> {
        self.inner.remove_accept_ranges()
    }

    /// Set raw HTTP Cache-Control metadata.
    #[napi]
    pub fn set_cache_control(&mut self, value: String) -> Result<()> {
        self.inner.set_cache_control(value).map_err(napi_error)
    }

    /// Remove raw HTTP Cache-Control metadata.
    #[napi]
    pub fn remove_cache_control(&mut self) -> Option<String> {
        self.inner.remove_cache_control()
    }

    /// Set raw HTTP Content-Disposition metadata.
    #[napi]
    pub fn set_content_disposition(&mut self, value: String) -> Result<()> {
        self.inner
            .set_content_disposition(value)
            .map_err(napi_error)
    }

    /// Remove raw HTTP Content-Disposition metadata.
    #[napi]
    pub fn remove_content_disposition(&mut self) -> Option<String> {
        self.inner.remove_content_disposition()
    }

    /// Set raw HTTP Content-Encoding metadata.
    #[napi]
    pub fn set_content_encoding(&mut self, value: String) -> Result<()> {
        self.inner.set_content_encoding(value).map_err(napi_error)
    }

    /// Remove raw HTTP Content-Encoding metadata.
    #[napi]
    pub fn remove_content_encoding(&mut self) -> Option<String> {
        self.inner.remove_content_encoding()
    }

    /// Set raw HTTP Content-Language metadata.
    #[napi]
    pub fn set_content_language(&mut self, value: String) -> Result<()> {
        self.inner.set_content_language(value).map_err(napi_error)
    }

    /// Remove raw HTTP Content-Language metadata.
    #[napi]
    pub fn remove_content_language(&mut self) -> Option<String> {
        self.inner.remove_content_language()
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
        self.inner.set_content_length(value);
        Ok(())
    }

    /// Remove exact HTTP Content-Length metadata.
    #[napi]
    pub fn remove_content_length(&mut self) -> Result<Option<BigInt>> {
        self.inner
            .remove_content_length()
            .map(|value| value.map(BigInt::from))
            .map_err(napi_error)
    }

    /// Set raw HTTP Content-Location metadata.
    #[napi]
    pub fn set_content_location(&mut self, value: String) -> Result<()> {
        self.inner.set_content_location(value).map_err(napi_error)
    }

    /// Remove raw HTTP Content-Location metadata.
    #[napi]
    pub fn remove_content_location(&mut self) -> Option<String> {
        self.inner.remove_content_location()
    }

    /// Set raw HTTP Content-Range metadata.
    #[napi]
    pub fn set_content_range(&mut self, value: String) -> Result<()> {
        self.inner.set_content_range(value).map_err(napi_error)
    }

    /// Remove raw HTTP Content-Range metadata.
    #[napi]
    pub fn remove_content_range(&mut self) -> Option<String> {
        self.inner.remove_content_range()
    }

    /// Set raw HTTP Content-Type metadata.
    #[napi]
    pub fn set_content_type(&mut self, value: String) -> Result<()> {
        self.inner.set_content_type(value).map_err(napi_error)
    }

    /// Remove raw HTTP Content-Type metadata.
    #[napi]
    pub fn remove_content_type(&mut self) -> Option<String> {
        self.inner.remove_content_type()
    }

    /// Set a bare typed MIME value while preserving Content-Encoding.
    #[napi]
    pub fn set_mime_type(&mut self, value: MimeTypeInput<'_>) -> Result<()> {
        self.inner.set_mime_type(mime_type_from_input(value)?);
        Ok(())
    }

    /// Remove and return the prior typed MIME value.
    #[napi]
    pub fn remove_mime_type(&mut self) -> Result<Option<JsMimeType>> {
        self.inner
            .remove_mime_type()
            .map(|value| value.map(JsMimeType::from_core))
            .map_err(napi_error)
    }

    /// Atomically project a typed media value to both HTTP content headers.
    #[napi]
    pub fn set_media_type(&mut self, value: MediaTypeInput<'_>) -> Result<()> {
        self.inner
            .set_media_type(media_type_from_input(value)?)
            .map_err(napi_error)
    }

    /// Atomically remove and return both prior typed HTTP media headers.
    #[napi]
    pub fn remove_media_type(&mut self) -> Result<Option<JsMediaType>> {
        self.inner
            .remove_media_type()
            .map(|value| value.map(JsMediaType::from_core))
            .map_err(napi_error)
    }

    /// Set raw HTTP `ETag` metadata.
    #[napi]
    pub fn set_etag(&mut self, value: String) -> Result<()> {
        self.inner.set_etag(value).map_err(napi_error)
    }

    /// Remove raw HTTP `ETag` metadata.
    #[napi]
    pub fn remove_etag(&mut self) -> Option<String> {
        self.inner.remove_etag()
    }

    /// Set raw HTTP Expires metadata.
    #[napi]
    pub fn set_expires(&mut self, value: String) -> Result<()> {
        self.inner.set_expires(value).map_err(napi_error)
    }

    /// Remove raw HTTP Expires metadata.
    #[napi]
    pub fn remove_expires(&mut self) -> Option<String> {
        self.inner.remove_expires()
    }

    /// Set raw HTTP Last-Modified metadata.
    #[napi]
    pub fn set_last_modified(&mut self, value: String) -> Result<()> {
        self.inner.set_last_modified(value).map_err(napi_error)
    }

    /// Remove raw HTTP Last-Modified metadata.
    #[napi]
    pub fn remove_last_modified(&mut self) -> Option<String> {
        self.inner.remove_last_modified()
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
        self.inner.set_http_location(url_from_input(value)?);
        Ok(())
    }

    /// Remove and return typed absolute HTTP Location metadata.
    #[napi]
    pub fn remove_http_location(&mut self) -> Result<Option<JsUrl>> {
        self.inner
            .remove_http_location()
            .map(|value| value.map(JsUrl::from_core))
            .map_err(napi_error)
    }

    /// Set raw HTTP Range metadata.
    #[napi]
    pub fn set_range(&mut self, value: String) -> Result<()> {
        self.inner.set_range(value).map_err(napi_error)
    }

    /// Remove raw HTTP Range metadata.
    #[napi]
    pub fn remove_range(&mut self) -> Option<String> {
        self.inner.remove_range()
    }

    /// Set raw HTTP Vary metadata.
    #[napi]
    pub fn set_vary(&mut self, value: String) -> Result<()> {
        self.inner.set_vary(value).map_err(napi_error)
    }

    /// Remove raw HTTP Vary metadata.
    #[napi]
    pub fn remove_vary(&mut self) -> Option<String> {
        self.inner.remove_vary()
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
    ) -> Result<JsProtocolMetadata> {
        let scheme = CoreScheme::from_str(&scheme).map_err(napi_error)?;
        Ok(JsProtocolMetadata::new(reference, scheme))
    }

    /// The live HTTP and HTTPS representation property view.
    #[napi(getter)]
    pub fn http(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::HTTP)
    }

    /// The live file protocol property view.
    #[napi(getter)]
    pub fn file(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::FILE)
    }

    /// The live uniform resource name property view.
    #[napi(getter)]
    pub fn urn(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::URN)
    }

    /// The live short-spelling `PostgreSQL` property view.
    #[napi(getter)]
    pub fn postgres(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::POSTGRES)
    }

    /// The live long-spelling `PostgreSQL` property view.
    #[napi(getter)]
    pub fn postgresql(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::POSTGRESQL)
    }

    /// The live `MySQL` property view.
    #[napi(getter)]
    pub fn mysql(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::MYSQL)
    }

    /// The live Arrow property view.
    #[napi(getter)]
    pub fn arrow(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::ARROW)
    }

    /// The live generic SQL property view.
    #[napi(getter)]
    pub fn sql(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::SQL)
    }

    /// The live AWS Glue property view.
    #[napi(getter)]
    pub fn glue(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::GLUE)
    }

    /// The live Apache Iceberg property view.
    #[napi(getter)]
    pub fn iceberg(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::ICEBERG)
    }

    /// The live Financial Information eXchange property view.
    #[napi(getter)]
    pub fn fix(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::FIX)
    }

    /// The live Yggdryl field property view.
    #[napi(getter)]
    pub fn field(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::FIELD)
    }

    /// The live Yggdryl datatype property view.
    #[napi(getter)]
    pub fn dtype(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::DTYPE)
    }

    /// The live Amazon S3 property view.
    #[napi(getter)]
    pub fn s3(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::S3)
    }

    /// The live Google Cloud Storage property view.
    #[napi(getter)]
    pub fn gs(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::GS)
    }

    /// The live Azure Blob Storage property view.
    #[napi(getter)]
    pub fn az(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::AZ)
    }

    /// The live Apache Spark property view.
    #[napi(getter)]
    pub fn spark(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::SPARK)
    }

    /// The live Polars property view.
    #[napi(getter)]
    pub fn polars(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::POLARS)
    }

    /// The live pandas property view.
    #[napi(getter)]
    pub fn pandas(&self, reference: Reference<JsField>) -> JsProtocolMetadata {
        JsProtocolMetadata::new(reference, CoreScheme::PANDAS)
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

    /// Deterministic FNV-1a hash of canonical native display text.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Materialize the bounded canonical Field default through Record's exact
    /// schema-guided JavaScript scalar projection.
    #[napi(js_name = "_defaultJSValueNative", skip_typescript)]
    pub fn default_js_value_native<'env>(
        &self,
        env: &'env Env,
        schema: &JsField,
    ) -> Result<Unknown<'env>> {
        let value = self.inner.default_value().map_err(napi_error)?;
        field_value_to_js(env, &self.inner, &value, &schema.inner)
    }

    /// Internal shared one-column schema for repeated exact-Field JavaScript
    /// default projection.
    #[napi(js_name = "_defaultJSValueSchemaNative", skip_typescript)]
    pub fn default_js_value_schema_native(&self) -> Result<JsField> {
        field_value_schema(&self.inner).map(JsField::from_core)
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
    #[napi(js_name = "toSchemeCompat", skip_typescript)]
    pub fn to_scheme_compat(&self, target: String) -> Result<Self> {
        let target = CoreScheme::from_str(&target).map_err(napi_error)?;
        self.inner
            .to_scheme_compat(&target)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Make a cheap clone preserving shared nested state and Arrow cache.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Return canonical syntax accepted losslessly by `fromString`.
    #[napi]
    pub fn to_string(&self) -> String {
        self.inner.to_string()
    }

    /// Serialize to version-independent structural JSON.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        serde_json::to_value(&self.inner).map_err(napi_error)
    }
}

/// One protocol's properties on a field, read and written by bare name.
///
/// The view is live: it holds the `Field` it was taken from and answers every
/// call through it, so a write through the view is visible on the field and a
/// write on the field is visible through the view. Nothing is snapshotted, and
/// the `scheme:` prefix is applied once, by the view.
#[napi(js_name = "ProtocolMetadata")]
pub struct JsProtocolMetadata {
    field: Reference<JsField>,
    scheme: CoreScheme,
}

impl JsProtocolMetadata {
    fn new(field: Reference<JsField>, scheme: CoreScheme) -> Self {
        Self { field, scheme }
    }

    /// Borrow the core read view, which reads the live field's metadata.
    fn view(&self) -> CoreProtocolMetadata<'_> {
        self.field.inner.protocol(&self.scheme)
    }
}

#[napi]
impl JsProtocolMetadata {
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
    #[napi]
    pub fn to_string(&self) -> String {
        self.view().to_string()
    }

    /// Serialize this protocol's bare names as a JSON object.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Object(
            self.view()
                .iter()
                .map(|(name, value)| (name.to_owned(), serde_json::Value::String(value.to_owned())))
                .collect(),
        )
    }
}
