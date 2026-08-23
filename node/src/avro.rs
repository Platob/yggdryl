//! Native JavaScript redirections over the raw Avro `Value` codec.

use std::sync::Arc;

use napi::bindgen_prelude::{Buffer, ClassInstance, Result};
use napi_derive::napi;
use yggdryl::avro::{Block as CoreBlock, Blocks as CoreBlocks, Container, Resolution, Schema};
use yggdryl::io::Buffer as CoreBuffer;
use yggdryl::{Limits, Value};

use crate::codec::JsCodecValue;
use crate::field::MetadataEntry;
use crate::{exact_u64, napi_error};

/// Resource limits shared by every Avro decode entry point.
#[napi(object)]
pub struct AvroDecodeLimitsInput {
    /// Maximum structural nesting in a schema or datum.
    pub max_depth: Option<f64>,
    /// Maximum encoded bytes consumed by one decode.
    pub max_input_bytes: Option<f64>,
    /// Maximum decoded nodes or container rows.
    pub max_nodes: Option<f64>,
}

impl AvroDecodeLimitsInput {
    fn into_core(self) -> Result<Limits> {
        let defaults = Limits::default();
        Ok(Limits::new(
            exact_limit(self.max_depth, "maxDepth", defaults.max_depth())?,
            exact_limit(
                self.max_input_bytes,
                "maxInputBytes",
                defaults.max_input_bytes(),
            )?,
            exact_limit(self.max_nodes, "maxNodes", defaults.max_nodes())?,
            defaults.max_documents(),
        ))
    }
}

fn decode_limits(value: Option<AvroDecodeLimitsInput>) -> Result<Limits> {
    value.map_or_else(|| Ok(Limits::default()), AvroDecodeLimitsInput::into_core)
}

fn exact_limit(value: Option<f64>, name: &str, default: usize) -> Result<usize> {
    value.map_or(Ok(default), |value| {
        usize::try_from(exact_u64(value, name)?)
            .map_err(|_| napi_error(format!("{name} exceeds this platform's usize")))
    })
}

/// One parsed Avro schema backed by the native schema graph.
#[napi(js_name = "AvroSchema")]
pub struct JsAvroSchema {
    inner: Schema,
}

impl JsAvroSchema {
    fn from_core(inner: Schema) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsAvroSchema {
    /// Parse a schema from one native `Value`.
    #[napi(factory, js_name = "_fromValueNative", skip_typescript)]
    pub fn from_value_native(
        value: &JsCodecValue,
        limits: Option<AvroDecodeLimitsInput>,
    ) -> Result<Self> {
        Schema::from_json_with_limits(&value.inner, decode_limits(limits)?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the JSON UTF-8 spelling of an Avro schema.
    #[napi(factory, js_name = "_fromUtf8Native", skip_typescript)]
    pub fn from_utf8_native(value: String, limits: Option<AvroDecodeLimitsInput>) -> Result<Self> {
        let limits = decode_limits(limits)?;
        let document = yggdryl::json::from_utf8_with_limits(&value, limits).map_err(napi_error)?;
        Schema::from_json_with_limits(&document, limits)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Parse the JSON bytes spelling of an Avro schema.
    #[napi(factory, js_name = "_fromBytesNative", skip_typescript)]
    pub fn from_bytes_native(value: Buffer, limits: Option<AvroDecodeLimitsInput>) -> Result<Self> {
        let limits = decode_limits(limits)?;
        let document = yggdryl::json::from_bytes_with_limits(&value, limits).map_err(napi_error)?;
        Schema::from_json_with_limits(&document, limits)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Return the exact schema document as one native `Value`.
    #[napi(js_name = "_intoValueNative", skip_typescript)]
    pub fn into_value_native(&self) -> JsCodecValue {
        JsCodecValue::from_core(self.inner.clone().into_json())
    }

    /// Return the root Avro kind.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().to_owned()
    }

    /// Return the Avro parsing canonical form.
    #[napi(getter)]
    pub fn canonical_form(&self) -> String {
        self.inner.clone().into_canonical_form()
    }

    /// Return the CRC-64-AVRO fingerprint.
    #[napi(getter)]
    pub fn fingerprint(&self) -> u64 {
        self.inner.fingerprint()
    }

    /// Return whether two schemas retain the same behavior-affecting document.
    #[napi]
    pub fn equals(&self, other: &JsAvroSchema) -> bool {
        self.inner == other.inner
    }

    /// Compare two schemas by the core's complete retained-schema order.
    #[napi]
    pub fn compare(&self, other: &JsAvroSchema) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Return deterministic hash bits for the complete retained schema.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Make a cheap clone sharing the parsed schema graph.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        Self::from_core(self.inner.clone())
    }

    /// Encode one value with Avro single-object framing.
    #[napi(js_name = "_intoSingleObjectNative", skip_typescript)]
    pub fn into_single_object_native(&self, value: &JsCodecValue) -> Result<Buffer> {
        yggdryl::avro::into_single_object_vec(&self.inner, &value.inner)
            .map(Into::into)
            .map_err(napi_error)
    }

    /// Decode one value from Avro single-object framing.
    #[napi(js_name = "_fromSingleObjectNative", skip_typescript)]
    pub fn from_single_object_native(
        &self,
        input: Buffer,
        limits: Option<AvroDecodeLimitsInput>,
    ) -> Result<JsCodecValue> {
        yggdryl::avro::from_single_object_slice_with_limits(
            &input,
            &self.inner,
            decode_limits(limits)?,
        )
        .map(JsCodecValue::from_core)
        .map_err(napi_error)
    }
}

/// One compressed block yielded by an owning lazy Avro iterator.
#[napi(js_name = "AvroBlock")]
pub struct JsAvroBlock {
    inner: CoreBlock,
    resolution: Option<Arc<Resolution>>,
}

#[napi]
impl JsAvroBlock {
    /// Return the row count declared by the block header.
    #[napi(getter)]
    pub fn count(&self) -> u64 {
        self.inner.count()
    }

    /// Return the compressed payload size in bytes.
    #[napi(getter)]
    pub fn size(&self) -> u64 {
        self.inner.size() as u64
    }

    /// Decode this block only, through the optional compiled resolution plan.
    #[napi(js_name = "_rowsNative", skip_typescript)]
    pub fn rows_native(&self) -> Result<JsCodecValue> {
        let rows = match self.resolution.as_deref() {
            Some(resolution) => self.inner.rows_resolved(resolution),
            None => self.inner.rows(),
        }
        .map_err(napi_error)?;
        Ok(JsCodecValue::from_core(Value::from_sequence(rows)))
    }
}

/// A fused lazy iterator over compressed Avro container blocks.
#[napi(js_name = "AvroBlocks")]
pub struct JsAvroBlocks {
    inner: CoreBlocks<'static, CoreBuffer>,
    resolution: Option<Arc<Resolution>>,
    done: bool,
}

#[napi]
impl JsAvroBlocks {
    /// Return the writer schema carried by the container header.
    #[napi(getter)]
    pub fn schema(&self) -> JsAvroSchema {
        JsAvroSchema::from_core(self.inner.schema().clone())
    }

    /// Return metadata as entries so every string key remains an own key.
    #[napi(js_name = "_metadataNative", skip_typescript)]
    pub fn metadata_native(&self) -> Vec<MetadataEntry> {
        self.inner
            .metadata()
            .iter()
            .map(|(key, value)| MetadataEntry {
                key: key.to_string(),
                value: value.to_string(),
            })
            .collect()
    }

    /// Return one header metadata value by key.
    #[napi]
    pub fn get(&self, key: String) -> Option<String> {
        self.inner.get(&key).map(str::to_owned)
    }

    /// Return the next still-compressed block, or `null` after EOF.
    #[napi(js_name = "next")]
    pub fn next_js(&mut self) -> Result<Option<JsAvroBlock>> {
        if self.done {
            return Ok(None);
        }
        match self.inner.next_block() {
            Ok(Some(inner)) => Ok(Some(JsAvroBlock {
                inner,
                resolution: self.resolution.clone(),
            })),
            Ok(None) => {
                self.done = true;
                Ok(None)
            }
            Err(error) => {
                self.done = true;
                Err(napi_error(error))
            }
        }
    }
}

/// Decode a whole Avro object container into the shared native `Value`.
#[napi(js_name = "avroLoadsNative", skip_typescript)]
pub fn avro_loads_native(
    input: Buffer,
    reader_schema: Option<ClassInstance<'_, JsAvroSchema>>,
    limits: Option<AvroDecodeLimitsInput>,
) -> Result<JsCodecValue> {
    let handle = CoreBuffer::from_bytes(input.to_vec());
    let limits = decode_limits(limits)?;
    let container = match reader_schema.as_ref() {
        Some(reader) => {
            yggdryl::avro::read_container_resolved_with_limits(&handle, &reader.inner, limits)
        }
        None => yggdryl::avro::read_container_with_limits(&handle, limits),
    }
    .map_err(napi_error)?;
    container_into_value(container)
        .map(JsCodecValue::from_core)
        .map_err(napi_error)
}

/// Open a fused lazy iterator over a container's still-compressed blocks.
#[napi(js_name = "avroBlocksNative", skip_typescript)]
pub fn avro_blocks_native(
    input: Buffer,
    reader_schema: Option<ClassInstance<'_, JsAvroSchema>>,
    limits: Option<AvroDecodeLimitsInput>,
) -> Result<JsAvroBlocks> {
    let limits = decode_limits(limits)?;
    let inner = yggdryl::avro::read_blocks_owned_with_limits(
        CoreBuffer::from_bytes(input.to_vec()),
        limits,
    )
    .map_err(napi_error)?;
    let resolution = reader_schema
        .as_ref()
        .map(|reader| Resolution::from_schemas(inner.schema(), &reader.inner).map(Arc::new))
        .transpose()
        .map_err(napi_error)?;
    Ok(JsAvroBlocks {
        inner,
        resolution,
        done: false,
    })
}

/// Encode a whole Avro object container from shared native values.
#[napi(js_name = "avroDumpsNative", skip_typescript)]
pub fn avro_dumps_native(
    schema: &JsAvroSchema,
    rows: &JsCodecValue,
    metadata: Vec<MetadataEntry>,
) -> Result<Buffer> {
    let rows = rows
        .inner
        .as_sequence()
        .ok_or_else(|| napi_error("Avro container rows must be a sequence"))?;
    let owned_metadata = metadata
        .into_iter()
        .map(|entry| (entry.key, entry.value))
        .collect::<Vec<_>>();
    let borrowed_metadata = owned_metadata
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect::<Vec<_>>();
    let mut handle = CoreBuffer::new();
    yggdryl::avro::write_container(
        &mut handle,
        &schema.inner.clone().into_json(),
        &borrowed_metadata,
        rows,
    )
    .map_err(napi_error)?;
    Ok(handle.into_bytes().into())
}

fn container_into_value(container: Container) -> yggdryl::Result<Value> {
    let metadata = container
        .metadata
        .into_iter()
        .map(|(key, value)| {
            Value::from_record([
                ("key", Value::from(key.as_str())),
                ("value", Value::from(value.as_str())),
            ])
        })
        .collect::<yggdryl::Result<Vec<_>>>()?;
    Value::from_record([
        ("schema", container.schema.into_json()),
        ("metadata", Value::from_sequence(metadata)),
        ("rows", Value::from_sequence(container.rows)),
    ])
}
