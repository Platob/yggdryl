//! Byte-first JSON, YAML, and TOML adapters for JavaScript values.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use napi::JsGlobal;
use napi::bindgen_prelude::{
    Array, ArrayBuffer, BigInt, Buffer, ClassInstance, Either, Env, Function, JavaScriptClassExt,
    JsObjectValue, JsValue, Object, Result, TypedArray, TypedArrayType, Uint8Array,
    Uint8ClampedArray, Unknown, ValueType,
};
use napi_derive::napi;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use yggdryl::text::{Float, Format, Limits, Value};
use yggdryl::{TimeUnit, json, text, toml, yaml};

use crate::{JsDataType, JsField, JsUri, JsUrl, JsUrn, napi_error};

const DEFAULT_JS_DEPTH: usize = 48;
const MAX_JS_DEPTH: usize = 48;
const MAX_JS_DOCUMENTS: u32 = 1_024;
const TRANSPORT_KEY: &str = "__yggdryl_codec__";
const JS_SAFE_INTEGER: i128 = 9_007_199_254_740_991;
const JS_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
/// The longest prefix of an out-of-range bigint an error message repeats.
const MAX_REPORTED_DIGITS: usize = 40;
/// The widest instant a JavaScript `Date` holds, in milliseconds either way.
const MAX_DATE_MILLISECONDS: u64 = 8_640_000_000_000_000;

/// One native codec value: the pivot every JavaScript value crosses.
///
/// A `Value` is what the core actually stores, so it is also the honest answer
/// to "what did my JavaScript become". `fromJs` builds one from any JavaScript
/// value and `asJs` reads it back; a load and a dump are those two conversions
/// with bytes on the far side, and they run the same code.
///
/// It is also the JavaScript spelling of the values JavaScript has none of: an
/// exact decimal, a date, a time of day, a duration, and any timestamp whose
/// resolution or zone a `Date` cannot hold.
#[napi(js_name = "Value")]
pub struct JsCodecValue {
    pub(crate) inner: Value,
}

impl JsCodecValue {
    /// Wrap one native value for JavaScript.
    pub(crate) const fn from_core(inner: Value) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsCodecValue {
    /// Convert one JavaScript value into the native value it becomes.
    #[napi(factory, js_name = "_fromJsNative", skip_typescript)]
    pub fn from_js_native(
        env: Env,
        value: Unknown<'_>,
        max_depth: Option<u32>,
        native_wrapper_prototypes: Array<'_>,
        native_intrinsics: Array<'_>,
    ) -> Result<Self> {
        encode_js_value(
            env,
            value,
            checked_depth(max_depth)?,
            &native_wrapper_prototypes,
            &native_intrinsics,
        )
        .map(|inner| Self { inner })
    }

    /// Project this value into the transport the JavaScript loader completes.
    #[napi(js_name = "_asJsNative", skip_typescript)]
    pub fn as_js_native(&self, max_depth: Option<u32>) -> Result<JsonValue> {
        value_to_transport(&self.inner, 0, checked_depth(max_depth)?)
    }

    /// Build an exact decimal: `unscaled` times ten to the minus `scale`.
    #[napi(factory)]
    pub fn decimal(unscaled: BigInt, scale: f64) -> Result<Self> {
        let scale = crate::exact_i8(scale, "scale")?;
        Ok(Self {
            inner: Value::decimal(exact_i128(&unscaled, "unscaled")?, scale),
        })
    }

    /// Build a timestamp: a count of `unit` since the Unix epoch.
    #[napi(factory)]
    pub fn timestamp(count: BigInt, unit: String, zone: Option<String>) -> Result<Self> {
        let inner = Value::timestamp(
            exact_i64(&count, "count")?,
            time_unit(&unit)?,
            zone.as_deref(),
        )
        .map_err(napi_error)?;
        Ok(Self { inner })
    }

    /// Build a date: a count of days since the Unix epoch.
    #[napi(factory)]
    pub fn date(days: f64) -> Result<Self> {
        Ok(Self {
            inner: Value::date(crate::exact_i32(days, "days")?),
        })
    }

    /// Build a time of day: a count of `unit` since midnight.
    #[napi(factory)]
    pub fn time(count: BigInt, unit: String) -> Result<Self> {
        Ok(Self {
            inner: Value::time(exact_i64(&count, "count")?, time_unit(&unit)?),
        })
    }

    /// Build a duration: an elapsed count of `unit`.
    #[napi(factory)]
    pub fn duration(count: BigInt, unit: String) -> Result<Self> {
        Ok(Self {
            inner: Value::duration(exact_i64(&count, "count")?, time_unit(&unit)?),
        })
    }

    /// The canonical vocabulary name for this kind, such as `timestamp`.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().to_owned()
    }

    /// The count a temporal holds, in days for a date, or `null`.
    #[napi(getter)]
    pub fn count(&self) -> Option<BigInt> {
        if let Some(days) = self.inner.as_date() {
            return Some(BigInt::from(i64::from(days)));
        }
        let count = self
            .inner
            .as_timestamp()
            .map(|(count, _, _)| count)
            .or_else(|| self.inner.as_time().map(|(count, _)| count))
            .or_else(|| self.inner.as_duration().map(|(count, _)| count))?;
        Some(BigInt::from(count))
    }

    /// The resolution a time, timestamp, or duration counts in, or `null`.
    ///
    /// A date has none: its count is always days, which is what Arrow stores.
    #[napi(getter)]
    pub fn unit(&self) -> Option<String> {
        let unit = self
            .inner
            .as_timestamp()
            .map(|(_, unit, _)| unit)
            .or_else(|| self.inner.as_time().map(|(_, unit)| unit))
            .or_else(|| self.inner.as_duration().map(|(_, unit)| unit))?;
        Some(unit.as_str().to_owned())
    }

    /// The zone a timestamp reads in, or `null` for a naive one.
    #[napi(getter)]
    pub fn zone(&self) -> Option<String> {
        self.inner
            .as_timestamp()
            .and_then(|(_, _, zone)| zone)
            .map(ToOwned::to_owned)
    }

    /// The unscaled coefficient of an exact decimal, or `null`.
    #[napi(getter)]
    pub fn unscaled(&self) -> Option<BigInt> {
        self.inner
            .as_decimal()
            .map(|(unscaled, _)| BigInt::from(unscaled))
    }

    /// The scale of an exact decimal, or `null`.
    #[napi(getter)]
    pub fn scale(&self) -> Option<i32> {
        self.inner.as_decimal().map(|(_, scale)| i32::from(scale))
    }

    /// Whether two native values are the same value.
    ///
    /// One instant spelled in two resolutions is one value, and so are two
    /// spellings of one exact decimal, because the core compares what a value
    /// names rather than how it was written.
    #[napi]
    pub fn equals(&self, other: &JsCodecValue) -> bool {
        self.inner == other.inner
    }
}

fn time_unit(value: &str) -> Result<TimeUnit> {
    TimeUnit::from_str(value).map_err(napi_error)
}

fn exact_i64(value: &BigInt, name: &str) -> Result<i64> {
    let (value, lossless) = value.get_i64();
    if !lossless {
        return Err(napi_error(format!(
            "{name} must fit in a signed 64-bit integer"
        )));
    }
    Ok(value)
}

fn exact_i128(value: &BigInt, name: &str) -> Result<i128> {
    let (value, lossless) = value.get_i128();
    if !lossless {
        return Err(napi_error(format!(
            "{name} must fit in a signed 128-bit integer"
        )));
    }
    Ok(value)
}

/// Assemble the read-side options from the boundary's two switches.
///
/// Substitution is on when the caller supplied variables *or* asked for the
/// environment, and off - meaning no value walk and no environment access at
/// all - when they did neither. The two are separate because a document that
/// resolves `{{ AWS_SECRET_ACCESS_KEY }}` into a value that is then dumped has
/// leaked it, so reaching the process environment is its own decision.
fn loading_from(
    limits: yggdryl::text::Limits,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: bool,
) -> Result<yggdryl::text::Loading> {
    let loading = yggdryl::text::Loading::new().with_limits(limits);
    if placeholders.is_none() && !environment {
        return Ok(loading);
    }
    let variables = match placeholders {
        Some(mapping) => {
            yggdryl::text::Placeholders::from_variables(&mapping.inner).map_err(napi_error)?
        }
        None => yggdryl::text::Placeholders::new(),
    };
    Ok(loading.with_placeholders(variables.with_environment(environment)))
}

/// Decode one JSON value without generic format parsing or dispatch.
#[napi(js_name = "jsonLoadsNative", skip_typescript)]
pub fn json_loads_native(
    input: Either<Buffer, String>,
    max_depth: Option<u32>,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: Option<bool>,
) -> Result<serde_json::Value> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let loading = loading_from(limits, placeholders, environment.unwrap_or(false))?;
    let value = match &input {
        Either::A(bytes) => text::from_slice_with(bytes.as_ref(), Format::Json, &loading),
        Either::B(value) => text::from_str_with(value, Format::Json, &loading),
    }
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Decode one YAML value without generic format parsing or dispatch.
#[napi(js_name = "yamlLoadsNative", skip_typescript)]
pub fn yaml_loads_native(
    input: Either<Buffer, String>,
    max_depth: Option<u32>,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: Option<bool>,
) -> Result<serde_json::Value> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let loading = loading_from(limits, placeholders, environment.unwrap_or(false))?;
    let value = match &input {
        Either::A(bytes) => text::from_slice_with(bytes.as_ref(), Format::Yaml, &loading),
        Either::B(value) => text::from_str_with(value, Format::Yaml, &loading),
    }
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Decode one TOML value without generic format parsing or dispatch.
#[napi(js_name = "tomlLoadsNative", skip_typescript)]
pub fn toml_loads_native(
    input: Either<Buffer, String>,
    max_depth: Option<u32>,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: Option<bool>,
) -> Result<serde_json::Value> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let loading = loading_from(limits, placeholders, environment.unwrap_or(false))?;
    let value = match &input {
        Either::A(bytes) => text::from_slice_with(bytes.as_ref(), Format::Toml, &loading),
        Either::B(value) => text::from_str_with(value, Format::Toml, &loading),
    }
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Decode strict JSON Lines without generic format parsing or dispatch.
#[napi(js_name = "jsonLinesLoadsNative", skip_typescript)]
pub fn json_lines_loads_native(
    input: Either<Buffer, String>,
    max_depth: Option<u32>,
) -> Result<Vec<serde_json::Value>> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let values = match &input {
        Either::A(bytes) => json::from_lines_slice_with_limits(bytes.as_ref(), limits),
        Either::B(value) => text::from_str_all_with_limits(value, Format::JsonLines, limits),
    }
    .map_err(napi_error)?;
    values_to_transport(&values, limits)
}

/// Decode every YAML document without generic format parsing or dispatch.
#[napi(js_name = "yamlLoadsAllNative", skip_typescript)]
pub fn yaml_loads_all_native(
    input: Either<Buffer, String>,
    max_depth: Option<u32>,
) -> Result<Vec<serde_json::Value>> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let values = match &input {
        Either::A(bytes) => yaml::from_slice_all_with_limits(bytes.as_ref(), limits),
        Either::B(value) => yaml::from_str_all_with_limits(value, limits),
    }
    .map_err(napi_error)?;
    values_to_transport(&values, limits)
}

/// Encode one JavaScript value directly to JSON bytes.
#[napi(js_name = "jsonDumpsNative", skip_typescript)]
pub fn json_dumps_native(
    env: Env,
    value: Unknown<'_>,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    json::to_vec(&value).map(Buffer::from).map_err(napi_error)
}

/// Encode one JavaScript value directly to YAML bytes.
#[napi(js_name = "yamlDumpsNative", skip_typescript)]
pub fn yaml_dumps_native(
    env: Env,
    value: Unknown<'_>,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    yaml::to_vec(&value).map(Buffer::from).map_err(napi_error)
}

/// Encode one JavaScript value directly to TOML bytes.
#[napi(js_name = "tomlDumpsNative", skip_typescript)]
pub fn toml_dumps_native(
    env: Env,
    value: Unknown<'_>,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let max_depth = checked_depth(max_depth)?;
    let value = encode_js_value(
        env,
        value,
        max_depth,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    toml::validate_for_write_with_limits(&value, limits_with_depth(max_depth))
        .map_err(napi_error)?;
    toml::to_vec(&value).map(Buffer::from).map_err(napi_error)
}

/// Encode JavaScript values directly as JSON Lines.
#[napi(js_name = "jsonLinesDumpAllNative", skip_typescript)]
pub fn json_lines_dump_all_native(
    env: Env,
    values: Array<'_>,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    json::to_vec_all(&values)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode JavaScript values directly as YAML documents.
#[napi(js_name = "yamlDumpAllNative", skip_typescript)]
pub fn yaml_dump_all_native(
    env: Env,
    values: Array<'_>,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    yaml::to_vec_all(&values)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Decode one JSON value from a path through the native reader boundary.
#[napi(js_name = "jsonLoadPathNative", skip_typescript)]
pub fn json_load_path_native(
    path: String,
    max_depth: Option<u32>,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: Option<bool>,
) -> Result<serde_json::Value> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let loading = loading_from(limits, placeholders, environment.unwrap_or(false))?;
    let value = text::from_reader_with(
        open_path(&path, limits.max_input_bytes())?,
        Format::Json,
        &loading,
    )
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Decode one YAML value from a path through the native reader boundary.
#[napi(js_name = "yamlLoadPathNative", skip_typescript)]
pub fn yaml_load_path_native(
    path: String,
    max_depth: Option<u32>,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: Option<bool>,
) -> Result<serde_json::Value> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let loading = loading_from(limits, placeholders, environment.unwrap_or(false))?;
    let value = text::from_reader_with(
        open_path(&path, limits.max_input_bytes())?,
        Format::Yaml,
        &loading,
    )
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Decode one TOML value from a path through the native reader boundary.
#[napi(js_name = "tomlLoadPathNative", skip_typescript)]
pub fn toml_load_path_native(
    path: String,
    max_depth: Option<u32>,
    placeholders: Option<ClassInstance<'_, JsCodecValue>>,
    environment: Option<bool>,
) -> Result<serde_json::Value> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let loading = loading_from(limits, placeholders, environment.unwrap_or(false))?;
    let value = text::from_reader_with(
        open_path(&path, limits.max_input_bytes())?,
        Format::Toml,
        &loading,
    )
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Decode strict JSON Lines from a path through the native reader boundary.
#[napi(js_name = "jsonLinesLoadPathNative", skip_typescript)]
pub fn json_lines_load_path_native(
    path: String,
    max_depth: Option<u32>,
) -> Result<Vec<serde_json::Value>> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let values =
        json::from_lines_reader_with_limits(open_path(&path, limits.max_input_bytes())?, limits)
            .map_err(napi_error)?;
    values_to_transport(&values, limits)
}

/// Decode every YAML document from a path through the native reader boundary.
#[napi(js_name = "yamlLoadAllPathNative", skip_typescript)]
pub fn yaml_load_all_path_native(
    path: String,
    max_depth: Option<u32>,
) -> Result<Vec<serde_json::Value>> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let values =
        yaml::from_reader_all_with_limits(open_path(&path, limits.max_input_bytes())?, limits)
            .map_err(napi_error)?;
    values_to_transport(&values, limits)
}

/// Encode one JavaScript value directly to a JSON file writer.
#[napi(js_name = "jsonDumpPathNative", skip_typescript)]
pub fn json_dump_path_native(
    env: Env,
    value: Unknown<'_>,
    path: String,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    json::to_writer(&mut writer, &value).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode one JavaScript value directly to a YAML file writer.
#[napi(js_name = "yamlDumpPathNative", skip_typescript)]
pub fn yaml_dump_path_native(
    env: Env,
    value: Unknown<'_>,
    path: String,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    yaml::to_writer(&mut writer, &value).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode one JavaScript value directly to a TOML file writer.
#[napi(js_name = "tomlDumpPathNative", skip_typescript)]
pub fn toml_dump_path_native(
    env: Env,
    value: Unknown<'_>,
    path: String,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let max_depth = checked_depth(max_depth)?;
    let value = encode_js_value(
        env,
        value,
        max_depth,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    toml::validate_for_write_with_limits(&value, limits_with_depth(max_depth))
        .map_err(napi_error)?;
    let mut writer = create_path(&path)?;
    toml::to_writer(&mut writer, &value).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode JavaScript values directly to a JSON Lines file writer.
#[napi(js_name = "jsonLinesDumpPathNative", skip_typescript)]
pub fn json_lines_dump_path_native(
    env: Env,
    values: Array<'_>,
    path: String,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    json::to_writer_all(&mut writer, &values).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode JavaScript values directly to a YAML document file writer.
#[napi(js_name = "yamlDumpAllPathNative", skip_typescript)]
pub fn yaml_dump_all_path_native(
    env: Env,
    values: Array<'_>,
    path: String,
    max_depth: Option<u32>,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    yaml::to_writer_all(&mut writer, &values).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Infer a codec format through the native extension rules.
#[napi(js_name = "codecInferFormat")]
pub fn codec_infer_format(path: String) -> Result<String> {
    Format::from_path(path)
        .map(|format| format.as_str().to_owned())
        .map_err(napi_error)
}

/// Infer and decode retained content once through the core parser rules.
#[napi(js_name = "codecLoadsInferredNative", skip_typescript)]
pub fn codec_loads_inferred_native(
    input: Either<Buffer, String>,
    max_depth: Option<u32>,
) -> Result<JsonValue> {
    let limits = limits_with_depth(checked_depth(max_depth)?);
    let (_, value) = match &input {
        Either::A(bytes) => text::from_slice_inferred_with_limits(bytes.as_ref(), limits),
        Either::B(value) => text::from_str_inferred_with_limits(value, limits),
    }
    .map_err(napi_error)?;
    value_to_transport(&value, 0, limits.max_depth())
}

/// Parse a format alias and return its stable native spelling.
#[napi(js_name = "codecNormalizeFormat")]
pub fn codec_normalize_format(format: String) -> Result<String> {
    parse_format(&format).map(|format| format.as_str().to_owned())
}

fn parse_format(format: &str) -> Result<Format> {
    Format::from_str(format).map_err(napi_error)
}

fn encode_js_value<'env>(
    env: Env,
    value: Unknown<'env>,
    max_depth: usize,
    native_wrapper_prototypes: &Array<'env>,
    native_intrinsics: &Array<'env>,
) -> Result<Value> {
    JsEncoder::new(env, max_depth, native_wrapper_prototypes, native_intrinsics)
        .map_err(|error| napi_error(format!("codec encoder initialization failed: {error}")))?
        .encode(value)
        .map_err(|error| napi_error(format!("JavaScript value encoding failed: {error}")))
}

fn encode_js_values<'env>(
    env: Env,
    values: &Array<'env>,
    max_depth: usize,
    native_wrapper_prototypes: &Array<'env>,
    native_intrinsics: &Array<'env>,
) -> Result<Vec<Value>> {
    if values.len() > MAX_JS_DOCUMENTS {
        return Err(napi_error(format!(
            "codec collection exceeds the {MAX_JS_DOCUMENTS}-document limit"
        )));
    }
    let mut encoder = JsEncoder::new(env, max_depth, native_wrapper_prototypes, native_intrinsics)
        .map_err(|error| napi_error(format!("codec encoder initialization failed: {error}")))?;
    let mut encoded_values = Vec::with_capacity(values.len() as usize);
    for index in 0..values.len() {
        let value = values
            .get::<Unknown<'_>>(index)?
            .ok_or_else(|| napi_error(format!("missing value at stream index {index}")))?;
        encoded_values.push(encoder.encode(value)?);
    }
    Ok(encoded_values)
}

fn values_to_transport(values: &[Value], limits: Limits) -> Result<Vec<serde_json::Value>> {
    values
        .iter()
        .map(|value| value_to_transport(value, 0, limits.max_depth()))
        .collect()
}

fn open_path(path: &str, max_input_bytes: usize) -> Result<File> {
    let file = File::open(Path::new(path)).map_err(napi_error)?;
    let max_file_bytes = u64::try_from(max_input_bytes).unwrap_or(u64::MAX);
    if file.metadata().map_err(napi_error)?.len() > max_file_bytes {
        return Err(napi_error(format!(
            "input exceeds the {max_input_bytes}-byte input limit"
        )));
    }
    Ok(file)
}

fn create_path(path: &str) -> Result<BufWriter<File>> {
    File::create(Path::new(path))
        .map(BufWriter::new)
        .map_err(napi_error)
}

fn checked_depth(max_depth: Option<u32>) -> Result<usize> {
    let max_depth = max_depth.map_or(DEFAULT_JS_DEPTH, |value| value as usize);
    if !(1..=MAX_JS_DEPTH).contains(&max_depth) {
        return Err(napi_error(format!(
            "maxDepth must be between 1 and {MAX_JS_DEPTH}"
        )));
    }
    Ok(max_depth)
}

fn limits_with_depth(max_depth: usize) -> Limits {
    let defaults = Limits::default();
    Limits::new(
        max_depth,
        defaults.max_input_bytes(),
        defaults.max_nodes(),
        defaults.max_documents(),
    )
}

struct JsEncoder<'env> {
    env: Env,
    active: Object<'env>,
    array_prototype: Object<'env>,
    array_buffer_prototype: Object<'env>,
    buffer_is_buffer: Function<'env, Object<'env>, bool>,
    date_get_time: Function<'env, (), f64>,
    date_prototype: Object<'env>,
    map_constructor: Function<'env, (), Unknown<'env>>,
    map_entries: Function<'env, (), Unknown<'env>>,
    map_is_map: Function<'env, Object<'env>, bool>,
    map_prototype: Object<'env>,
    native_wrapper_prototypes: [Object<'env>; 6],
    regexp_constructor: Function<'env, (), Unknown<'env>>,
    regexp_flags_getter: Function<'env, Object<'env>, String>,
    regexp_is_regexp: Function<'env, Object<'env>, bool>,
    regexp_prototype: Object<'env>,
    regexp_source_getter: Function<'env, Object<'env>, String>,
    set_constructor: Function<'env, (), Unknown<'env>>,
    set_prototype: Object<'env>,
    set_is_set: Function<'env, Object<'env>, bool>,
    set_values: Function<'env, (), Unknown<'env>>,
    typed_array_prototypes: [Object<'env>; 11],
    url_constructor: Function<'env, (), Unknown<'env>>,
    url_prototype: Object<'env>,
    url_to_string: Function<'env, (), String>,
    max_depth: usize,
}

impl<'env> JsEncoder<'env> {
    fn new(
        env: Env,
        max_depth: usize,
        native_wrapper_prototypes: &Array<'env>,
        native_intrinsics: &Array<'env>,
    ) -> Result<Self> {
        let global = env.get_global()?;
        let weak_set: Function<'_, (), Unknown<'_>> = global.get_named_property("WeakSet")?;
        let active = weak_set.new_instance(())?.coerce_to_object()?;
        let array_constructor: Function<'_, (), Unknown<'_>> =
            global.get_named_property("Array")?;
        let array_prototype = array_constructor.get_named_property("prototype")?;
        let array_buffer_constructor: Function<'_, (), Unknown<'_>> =
            global.get_named_property("ArrayBuffer")?;
        let array_buffer_prototype = array_buffer_constructor.get_named_property("prototype")?;
        let buffer: Function<'_, (), Unknown<'_>> = global.get_named_property("Buffer")?;
        let buffer_is_buffer: Function<'_, Object<'_>, bool> =
            buffer.get_named_property("isBuffer")?;
        let date_constructor: Function<'_, (), Unknown<'_>> = global.get_named_property("Date")?;
        let date_prototype: Object<'_> = date_constructor.get_named_property("prototype")?;
        // Read the instant off the prototype's own method rather than off the
        // instance, so an own `getTime` property cannot dictate what is written.
        let date_get_time: Function<'_, (), f64> = date_prototype.get_named_property("getTime")?;
        let map_constructor: Function<'_, (), Unknown<'_>> = global.get_named_property("Map")?;
        let map_prototype: Object<'_> = map_constructor.get_named_property("prototype")?;
        let map_entries: Function<'_, (), Unknown<'_>> =
            map_prototype.get_named_property("entries")?;
        if native_intrinsics.len() != 5 {
            return Err(napi_error(
                "native codec intrinsic table must contain exactly five entries",
            ));
        }
        let map_is_map = required_array_predicate(native_intrinsics, 0, "Map")?;
        let set_is_set = required_array_predicate(native_intrinsics, 1, "Set")?;
        let regexp_is_regexp = required_array_predicate(native_intrinsics, 2, "RegExp")?;
        let regexp_source_getter =
            required_array_string_function(native_intrinsics, 3, "RegExp source")?;
        let regexp_flags_getter =
            required_array_string_function(native_intrinsics, 4, "RegExp flags")?;
        let native_wrapper_prototypes = wrapper_prototypes(native_wrapper_prototypes)?;
        let regexp_constructor: Function<'_, (), Unknown<'_>> =
            global.get_named_property("RegExp")?;
        let regexp_prototype: Object<'_> = regexp_constructor.get_named_property("prototype")?;
        let set_constructor: Function<'_, (), Unknown<'_>> = global.get_named_property("Set")?;
        let set_prototype: Object<'_> = set_constructor.get_named_property("prototype")?;
        let set_values: Function<'_, (), Unknown<'_>> =
            set_prototype.get_named_property("values")?;
        let typed_array_prototypes = [
            constructor_prototype(&global, "Int8Array")?,
            constructor_prototype(&global, "Uint8Array")?,
            constructor_prototype(&global, "Uint8ClampedArray")?,
            constructor_prototype(&global, "Int16Array")?,
            constructor_prototype(&global, "Uint16Array")?,
            constructor_prototype(&global, "Int32Array")?,
            constructor_prototype(&global, "Uint32Array")?,
            constructor_prototype(&global, "Float32Array")?,
            constructor_prototype(&global, "Float64Array")?,
            constructor_prototype(&global, "BigInt64Array")?,
            constructor_prototype(&global, "BigUint64Array")?,
        ];
        let url_constructor: Function<'_, (), Unknown<'_>> = global.get_named_property("URL")?;
        let url_prototype: Object<'_> = url_constructor.get_named_property("prototype")?;
        let url_to_string: Function<'_, (), String> =
            url_prototype.get_named_property("toString")?;
        Ok(Self {
            env,
            active,
            array_prototype,
            array_buffer_prototype,
            buffer_is_buffer,
            date_get_time,
            date_prototype,
            map_constructor,
            map_entries,
            map_is_map,
            map_prototype,
            native_wrapper_prototypes,
            regexp_constructor,
            regexp_flags_getter,
            regexp_is_regexp,
            regexp_prototype,
            regexp_source_getter,
            set_constructor,
            set_prototype,
            set_is_set,
            set_values,
            typed_array_prototypes,
            url_constructor,
            url_prototype,
            url_to_string,
            max_depth,
        })
    }

    fn encode(&mut self, value: Unknown<'env>) -> Result<Value> {
        self.encode_at(value, 0)
    }

    fn encode_at(&mut self, value: Unknown<'env>, depth: usize) -> Result<Value> {
        if depth > self.max_depth {
            return Err(napi_error(format!(
                "JavaScript value exceeds maxDepth {}",
                self.max_depth
            )));
        }
        match value.get_type()? {
            // Nothing and no value are one absence on the wire, so `undefined`
            // reads back as `null`. Keeping them apart needed a tag, and a tag
            // that only says "this was undefined" is not worth a vocabulary.
            ValueType::Undefined | ValueType::Null => Ok(Value::Null),
            ValueType::Boolean => value.coerce_to_bool().map(Value::Bool),
            ValueType::Number => Self::encode_number(value),
            ValueType::String => value
                .coerce_to_string()?
                .into_utf8()?
                .into_owned()
                .map(Value::from),
            ValueType::BigInt => Self::encode_bigint(value),
            ValueType::Object => self.encode_object(value, depth),
            ValueType::Function => Err(napi_error(
                "functions are not serializable; encode their data explicitly",
            )),
            ValueType::Symbol => Err(napi_error(
                "symbols are not serializable; encode their description explicitly",
            )),
            ValueType::External | ValueType::Unknown => {
                Err(napi_error("unsupported external JavaScript value"))
            }
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn encode_number(value: Unknown<'env>) -> Result<Value> {
        let value = value.coerce_to_number()?.get_double()?;
        if value == 0.0 && value.is_sign_negative() {
            return Ok(Value::F64(Float::from_f64(value)));
        }
        if value.is_finite()
            && value.fract() == 0.0
            && (-JS_SAFE_INTEGER_F64..=JS_SAFE_INTEGER_F64).contains(&value)
        {
            return Ok(Value::I64(value as i64));
        }
        Ok(Value::F64(Float::from_f64(value)))
    }

    /// Encode a `bigint` as the narrowest native integer that holds it exactly.
    ///
    /// A `bigint` is an exact integer, so it belongs in an integer variant
    /// rather than in a wrapper naming its JavaScript type. Beyond 128 bits the
    /// core has no exact storage for one, and refusing there keeps a rounded or
    /// re-spelled number from being written as though it were the original.
    fn encode_bigint(value: Unknown<'env>) -> Result<Value> {
        let digits = value.coerce_to_string()?.into_utf8()?.into_owned()?;
        if let Ok(value) = digits.parse::<i64>() {
            return Ok(Value::I64(value));
        }
        if let Ok(value) = digits.parse::<u64>() {
            return Ok(Value::U64(value));
        }
        if let Ok(value) = digits.parse::<i128>() {
            return Ok(Value::I128(value));
        }
        if let Ok(value) = digits.parse::<u128>() {
            return Ok(Value::U128(value));
        }
        Err(napi_error(format!(
            "bigint {} exceeds the exact 128-bit integer range this codec stores",
            truncated(&digits)
        )))
    }

    fn encode_object(&mut self, value: Unknown<'env>, depth: usize) -> Result<Value> {
        let object = value.coerce_to_object()?;
        let has: Function<'_, Object<'env>, bool> = required_property(&self.active, "has")?;
        if has.apply(self.active, object)? {
            return Err(napi_error("cyclic JavaScript value is not serializable"));
        }
        let add: Function<'_, Object<'env>, Unknown<'_>> = required_property(&self.active, "add")?;
        add.apply(self.active, object)?;
        let result = self.encode_object_inner(value, depth);
        let remove: Function<'_, Object<'env>, bool> = required_property(&self.active, "delete")?;
        remove.apply(self.active, object)?;
        result
    }

    fn encode_object_inner(&mut self, value: Unknown<'env>, depth: usize) -> Result<Value> {
        let object = value.coerce_to_object()?;
        if self.buffer_is_buffer.call(object)? {
            let bytes: Buffer = self.extract_object(&object)?;
            return Ok(Value::from(bytes.as_ref().to_vec()));
        }
        if value.is_arraybuffer()? {
            if !self.has_exact_prototype(&object, self.array_buffer_prototype)? {
                return Err(napi_error(
                    "ArrayBuffer subclasses are not serialized implicitly; convert to ArrayBuffer",
                ));
            }
            let bytes: ArrayBuffer<'_> = self.extract_object(&object)?;
            return Ok(Value::from(bytes.to_vec()));
        }
        if value.is_typedarray()? {
            return self.encode_typed_array(value, depth);
        }
        if value.is_array()? {
            if !self.has_exact_prototype(&object, self.array_prototype)? {
                return Err(napi_error(
                    "Array subclasses are not serialized implicitly; convert to Array",
                ));
            }
            return self.encode_array(value, depth);
        }
        if value.is_date()? {
            if !self.has_exact_prototype(&object, self.date_prototype)? {
                return Err(napi_error(
                    "Date subclasses are not serialized implicitly; convert to Date",
                ));
            }
            return self.encode_date(&object);
        }

        if let Some(value) = self.encode_branded_object(value, &object, depth)? {
            return Ok(value);
        }

        // A class instance has no shape of its own on the wire: its own
        // enumerable properties are a mapping, and the class name that used to
        // travel beside them is gone. Reading one back therefore yields a plain
        // object, not an instance.
        self.encode_properties(&object, depth)
    }

    /// Encode a `Date` as the millisecond timestamp it already is.
    fn encode_date(&self, object: &Object<'env>) -> Result<Value> {
        let millis = self.date_get_time.apply(object, ())?;
        if !millis.is_finite() {
            return Err(napi_error(
                "an invalid Date has no instant; encode a valid Date or null",
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(Value::timestamp_in(
            millis as i64,
            TimeUnit::Millisecond,
            None,
        ))
    }

    fn encode_branded_object(
        &mut self,
        value: Unknown<'env>,
        object: &Object<'env>,
        depth: usize,
    ) -> Result<Option<Value>> {
        if value.instanceof(self.map_constructor)? {
            if !self.has_exact_prototype(object, self.map_prototype)? {
                return Err(napi_error(
                    "Map subclasses are not serialized implicitly; convert to Map",
                ));
            }
            return self.encode_map(object, depth).map(Some);
        }
        if self
            .map_is_map
            .call(*object)
            .map_err(|error| napi_error(format!("Map brand check failed: {error}")))?
        {
            return Err(napi_error(
                "cross-realm Map values are not serialized implicitly; copy into the current realm Map",
            ));
        }
        if value.instanceof(self.set_constructor)? {
            if !self.has_exact_prototype(object, self.set_prototype)? {
                return Err(napi_error(
                    "Set subclasses are not serialized implicitly; convert to Set",
                ));
            }
            return self.encode_set(object, depth).map(Some);
        }
        if self
            .set_is_set
            .call(*object)
            .map_err(|error| napi_error(format!("Set brand check failed: {error}")))?
        {
            return Err(napi_error(
                "cross-realm Set values are not serialized implicitly; copy into the current realm Set",
            ));
        }
        if value.instanceof(self.url_constructor)? {
            if !self.has_exact_prototype(object, self.url_prototype)? {
                return Err(napi_error(
                    "URL subclasses are not serialized implicitly; convert to URL",
                ));
            }
            // A URL is its href, which is what every other runtime stores too.
            return Ok(Some(Value::from(self.url_to_string.apply(object, ())?)));
        }
        if value.instanceof(self.regexp_constructor)? {
            if !self.has_exact_prototype(object, self.regexp_prototype)? {
                return Err(napi_error(
                    "RegExp subclasses are not serialized implicitly; convert to RegExp",
                ));
            }
            // The literal spelling carries both halves of a RegExp in one
            // string, so no second field is needed to keep the flags.
            let source = self.regexp_source_getter.call(*object)?;
            let flags = self.regexp_flags_getter.call(*object)?;
            return Ok(Some(Value::from(format!("/{source}/{flags}"))));
        }
        if self
            .regexp_is_regexp
            .call(*object)
            .map_err(|error| napi_error(format!("RegExp brand check failed: {error}")))?
        {
            return Err(napi_error(
                "cross-realm RegExp values are not serialized implicitly; copy into the current realm RegExp",
            ));
        }

        macro_rules! native_wrapper {
            ($wrapper:ty, $name:literal, $prototype_index:literal, $encode:expr) => {
                if <$wrapper>::instance_of(&self.env, &value)? {
                    if !self.has_exact_prototype(
                        object,
                        self.native_wrapper_prototypes[$prototype_index],
                    )? {
                        return Err(napi_error(concat!(
                            $name,
                            " subclasses are not serialized implicitly; convert to the native ",
                            $name,
                            " value"
                        )));
                    }
                    let wrapper: ClassInstance<'_, $wrapper> = self.extract_object(object)?;
                    #[allow(clippy::redundant_closure_call)]
                    return Ok(Some($encode(&wrapper.inner)));
                }
            };
        }

        // A Value is already a native value, so it crosses as itself. Every
        // other wrapper crosses as the canonical text it round-trips through,
        // because that is the one spelling every runtime can read back.
        native_wrapper!(JsCodecValue, "Value", 0, Clone::clone);
        native_wrapper!(JsDataType, "DataType", 1, |inner| Value::from(
            ToString::to_string(inner)
        ));
        native_wrapper!(JsField, "Field", 2, |inner| Value::from(
            ToString::to_string(inner)
        ));
        native_wrapper!(JsUri, "Uri", 3, |inner| Value::from(ToString::to_string(
            inner
        )));
        native_wrapper!(JsUrl, "Url", 4, |inner| Value::from(ToString::to_string(
            inner
        )));
        native_wrapper!(JsUrn, "Urn", 5, |inner| Value::from(ToString::to_string(
            inner
        )));
        Ok(None)
    }

    fn encode_typed_array(&mut self, value: Unknown<'env>, depth: usize) -> Result<Value> {
        let object = value.coerce_to_object()?;
        let typed: TypedArray<'_> = self.extract_object(&object)?;
        let (constructor_name, prototype_index) = match typed.typed_array_type {
            TypedArrayType::Int8 => ("Int8Array", 0),
            TypedArrayType::Uint8 => ("Uint8Array", 1),
            TypedArrayType::Uint8Clamped => ("Uint8ClampedArray", 2),
            TypedArrayType::Int16 => ("Int16Array", 3),
            TypedArrayType::Uint16 => ("Uint16Array", 4),
            TypedArrayType::Int32 => ("Int32Array", 5),
            TypedArrayType::Uint32 => ("Uint32Array", 6),
            TypedArrayType::Float32 => ("Float32Array", 7),
            TypedArrayType::Float64 => ("Float64Array", 8),
            TypedArrayType::BigInt64 => ("BigInt64Array", 9),
            TypedArrayType::BigUint64 => ("BigUint64Array", 10),
            _ => {
                return Err(napi_error("unsupported JavaScript typed-array kind"));
            }
        };
        if !self.has_exact_prototype(&object, self.typed_array_prototypes[prototype_index])? {
            return Err(napi_error(format!(
                "{constructor_name} subclasses are not serialized implicitly; convert to {constructor_name}"
            )));
        }
        // A byte-wide typed array is bytes; every wider one is a sequence of
        // the numbers it holds, which is the shape its elements already have.
        if constructor_name == "Uint8Array" {
            let bytes: Uint8Array = self.extract_object(&object)?;
            return Ok(Value::from(bytes.as_ref().to_vec()));
        }
        if constructor_name == "Uint8ClampedArray" {
            let bytes: Uint8ClampedArray = self.extract_object(&object)?;
            return Ok(Value::from(bytes.as_ref().to_vec()));
        }
        let length = object.get::<u32>("length")?.unwrap_or(0);
        let mut values = Vec::with_capacity(length as usize);
        for index in 0..length {
            let item = object
                .get::<Unknown<'_>>(&index.to_string())?
                .ok_or_else(|| {
                    napi_error(format!("missing typed-array element at index {index}"))
                })?;
            values.push(self.encode_at(item, depth + 1)?);
        }
        Ok(Value::from_sequence(values))
    }

    fn encode_array(&mut self, value: Unknown<'env>, depth: usize) -> Result<Value> {
        let object = value.coerce_to_object()?;
        let length = object.get::<u32>("length")?.unwrap_or(0);
        let mut values = Vec::with_capacity(length as usize);
        for index in 0..length {
            match object.get::<Unknown<'_>>(&index.to_string())? {
                Some(item) => values.push(self.encode_at(item, depth + 1)?),
                None => values.push(Value::Null),
            }
        }
        Ok(Value::from_sequence(values))
    }

    /// Encode a `Map` as a native mapping over its own arbitrary keys.
    ///
    /// The core mapping already takes any value as a key, so a Map needs no
    /// wrapper. Two distinct JavaScript keys can still encode to one native
    /// key, and that collision is refused rather than silently collapsed.
    fn encode_map(&mut self, object: &Object<'env>, depth: usize) -> Result<Value> {
        let entries = Self::iterator_values(object, self.map_entries)?;
        let mut values = Vec::with_capacity(entries.len());
        for entry in entries {
            let entry = entry.ok_or_else(|| napi_error("Map iterator yielded undefined"))?;
            let pair = entry.coerce_to_object()?;
            values.push((
                self.encode_required_property(&pair, "0", depth + 1, "Map key")?,
                self.encode_required_property(&pair, "1", depth + 1, "Map value")?,
            ));
        }
        Value::from_mapping(values).map_err(napi_error)
    }

    fn encode_set(&mut self, object: &Object<'env>, depth: usize) -> Result<Value> {
        let entries = Self::iterator_values(object, self.set_values)?;
        let mut values = Vec::with_capacity(entries.len());
        for entry in entries {
            values.push(match entry {
                Some(entry) => self.encode_at(entry, depth + 1)?,
                None => Value::Null,
            });
        }
        Ok(Value::from_sequence(values))
    }

    fn iterator_values(
        object: &Object<'env>,
        method: Function<'env, (), Unknown<'env>>,
    ) -> Result<Vec<Option<Unknown<'env>>>> {
        let iterator = method.apply(object, ())?.coerce_to_object()?;
        let next: Function<'_, (), Unknown<'_>> = required_property(&iterator, "next")?;
        let mut values = Vec::new();
        loop {
            let step = next.apply(iterator, ())?.coerce_to_object()?;
            if step.get::<bool>("done")?.unwrap_or(false) {
                break;
            }
            if !step.has_named_property("value")? {
                return Err(napi_error("iterator result has no `value` property"));
            }
            values.push(step.get::<Unknown<'_>>("value")?);
        }
        Ok(values)
    }

    fn encode_properties(&mut self, object: &Object<'env>, depth: usize) -> Result<Value> {
        let keys = Object::keys(object)?;
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            let value = match object.get::<Unknown<'_>>(&key)? {
                Some(value) => self.encode_at(value, depth + 1)?,
                None => Value::Null,
            };
            entries.push((Value::from(key), value));
        }
        Value::from_mapping(entries).map_err(napi_error)
    }

    fn encode_required_property(
        &mut self,
        object: &Object<'env>,
        name: &str,
        depth: usize,
        context: &str,
    ) -> Result<Value> {
        if !object.has_named_property(name)? {
            return Err(napi_error(format!("{context} property is missing")));
        }
        match object.get::<Unknown<'_>>(name)? {
            Some(value) => self.encode_at(value, depth),
            None => Ok(Value::Null),
        }
    }

    fn has_exact_prototype(&self, object: &Object<'env>, expected: Object<'env>) -> Result<bool> {
        self.env.strict_equals(object.get_prototype()?, expected)
    }

    fn extract_object<T>(&self, object: &Object<'env>) -> Result<T>
    where
        T: napi::bindgen_prelude::FromNapiValue,
    {
        let mut holder = Object::new(&self.env)?;
        holder.set("value", object)?;
        holder
            .get::<T>("value")?
            .ok_or_else(|| napi_error("failed to access JavaScript binary value"))
    }
}

fn constructor_prototype<'env>(global: &JsGlobal<'env>, name: &str) -> Result<Object<'env>> {
    let constructor: Function<'_, (), Unknown<'_>> = global.get_named_property(name)?;
    constructor.get_named_property("prototype")
}

fn wrapper_prototypes<'env>(values: &Array<'env>) -> Result<[Object<'env>; 6]> {
    if values.len() != 6 {
        return Err(napi_error(
            "native wrapper prototype table must contain exactly six entries",
        ));
    }
    Ok([
        required_array_object(values, 0, "Value")?,
        required_array_object(values, 1, "DataType")?,
        required_array_object(values, 2, "Field")?,
        required_array_object(values, 3, "Uri")?,
        required_array_object(values, 4, "Url")?,
        required_array_object(values, 5, "Urn")?,
    ])
}

fn required_array_object<'env>(
    values: &Array<'env>,
    index: u32,
    name: &str,
) -> Result<Object<'env>> {
    values
        .get::<Object<'env>>(index)?
        .ok_or_else(|| napi_error(format!("missing native {name} prototype")))
}

fn required_array_predicate<'env>(
    values: &Array<'env>,
    index: u32,
    name: &str,
) -> Result<Function<'env, Object<'env>, bool>> {
    values
        .get::<Function<'env, Object<'env>, bool>>(index)?
        .ok_or_else(|| napi_error(format!("missing native {name} brand check")))
}

fn required_array_string_function<'env>(
    values: &Array<'env>,
    index: u32,
    name: &str,
) -> Result<Function<'env, Object<'env>, String>> {
    values
        .get::<Function<'env, Object<'env>, String>>(index)?
        .ok_or_else(|| napi_error(format!("missing native {name} accessor")))
}

fn required_property<T>(object: &Object<'_>, name: &str) -> Result<T>
where
    T: napi::bindgen_prelude::FromNapiValue,
{
    object
        .get::<T>(name)?
        .ok_or_else(|| napi_error(format!("missing required JavaScript property `{name}`")))
}

/// Shorten a caller-supplied digit string before an error message repeats it.
fn truncated(digits: &str) -> String {
    if digits.len() <= MAX_REPORTED_DIGITS {
        return digits.to_owned();
    }
    format!("{}…", &digits[..MAX_REPORTED_DIGITS])
}

fn value_to_transport(value: &Value, depth: usize, max_depth: usize) -> Result<JsonValue> {
    if depth > max_depth {
        return Err(napi_error(format!(
            "decoded value exceeds maxDepth {max_depth}"
        )));
    }
    match value {
        // A record crosses as the plain object its field names spell.
        Value::Record(..) => value_to_transport(&value.record_to_mapping(), depth, max_depth),
        Value::Null => Ok(JsonValue::Null),
        Value::Bool(value) => Ok(JsonValue::Bool(*value)),
        Value::I8(value) => integer_transport(i128::from(*value)),
        Value::I16(value) => integer_transport(i128::from(*value)),
        Value::I32(value) => integer_transport(i128::from(*value)),
        Value::I64(value) => integer_transport(i128::from(*value)),
        Value::U8(value) => unsigned_transport(u128::from(*value)),
        Value::U16(value) => unsigned_transport(u128::from(*value)),
        Value::U32(value) => unsigned_transport(u128::from(*value)),
        Value::U64(value) => unsigned_transport(u128::from(*value)),
        Value::I128(value) => integer_transport(*value),
        Value::U128(value) => unsigned_transport(*value),
        Value::F32(value) => float_transport(value.as_f64()),
        Value::F64(value) => float_transport(value.as_f64()),
        Value::String(value) => Ok(JsonValue::String(value.to_string())),
        // A geometry has no JavaScript binding surface yet, so its WKB crosses
        // as its plain shape: the bytes transport that becomes a Buffer.
        Value::Bytes(value) | Value::Geospatial(value) => Ok(marker(
            "bytes",
            [("value", JsonValue::String(BASE64.encode(value)))],
        )),
        Value::Sequence(values) => values
            .iter()
            .map(|value| value_to_transport(value, depth + 1, max_depth))
            .collect::<Result<Vec<_>>>()
            .map(JsonValue::Array),
        // A count is carried as text because a nanosecond instant needs more
        // than the 53 bits a JSON number keeps exactly; the JavaScript side
        // reads it as a bigint.
        Value::Decimal(unscaled, scale) => Ok(marker(
            "decimal",
            [
                ("value", JsonValue::String(unscaled.to_string())),
                ("scale", JsonValue::Number(JsonNumber::from(*scale))),
            ],
        )),
        Value::Date(days) => Ok(marker(
            "date",
            [("value", JsonValue::Number(JsonNumber::from(*days)))],
        )),
        Value::Time(count, unit) => Ok(marker(
            "time",
            [
                ("value", JsonValue::String(count.to_string())),
                ("unit", JsonValue::String(unit.as_str().to_owned())),
            ],
        )),
        Value::Timestamp(count, unit, zone) => Ok(marker(
            "timestamp",
            [
                ("value", JsonValue::String(count.to_string())),
                ("unit", JsonValue::String(unit.as_str().to_owned())),
                ("zone", JsonValue::String(zone.as_str().to_owned())),
                ("date", JsonValue::Null),
            ],
        )),
        Value::DateTime(count, unit) => {
            // A Date is a naive instant counted in whole milliseconds and no
            // wider than this range. A naive reading that is exactly one
            // crosses as a Date; every other resolution crosses as itself,
            // because rounding an instant to fit would change it.
            let date = value
                .temporal_count_at(TimeUnit::Millisecond)
                .filter(|millis| millis.unsigned_abs() <= MAX_DATE_MILLISECONDS);
            Ok(marker(
                "timestamp",
                [
                    ("value", JsonValue::String(count.to_string())),
                    ("unit", JsonValue::String(unit.as_str().to_owned())),
                    ("zone", JsonValue::Null),
                    (
                        "date",
                        date.map_or(JsonValue::Null, |millis| {
                            JsonValue::Number(JsonNumber::from(millis))
                        }),
                    ),
                ],
            ))
        }
        Value::Duration(count, unit) => Ok(marker(
            "duration",
            [
                ("value", JsonValue::String(count.to_string())),
                ("unit", JsonValue::String(unit.as_str().to_owned())),
            ],
        )),
        Value::Mapping(entries) => mapping_transport(entries, depth, max_depth),
    }
}

fn integer_transport(value: i128) -> Result<JsonValue> {
    if (-JS_SAFE_INTEGER..=JS_SAFE_INTEGER).contains(&value) {
        let value = i64::try_from(value).map_err(napi_error)?;
        return Ok(JsonValue::Number(JsonNumber::from(value)));
    }
    Ok(marker(
        "bigint",
        [("value", JsonValue::String(value.to_string()))],
    ))
}

fn unsigned_transport(value: u128) -> Result<JsonValue> {
    if value <= JS_SAFE_INTEGER as u128 {
        let value = u64::try_from(value).map_err(napi_error)?;
        return Ok(JsonValue::Number(JsonNumber::from(value)));
    }
    Ok(marker(
        "bigint",
        [("value", JsonValue::String(value.to_string()))],
    ))
}

fn float_transport(value: f64) -> Result<JsonValue> {
    if value.is_finite() && !(value == 0.0 && value.is_sign_negative()) {
        return JsonNumber::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| napi_error("invalid finite floating-point value"));
    }
    let spelling = if value.is_nan() {
        "nan"
    } else if value.is_sign_positive() {
        "infinity"
    } else if value == 0.0 {
        "-0"
    } else {
        "-infinity"
    };
    Ok(marker(
        "float",
        [("value", JsonValue::String(spelling.to_owned()))],
    ))
}

fn mapping_transport(
    entries: &[(Value, Value)],
    depth: usize,
    max_depth: usize,
) -> Result<JsonValue> {
    let all_strings = entries
        .iter()
        .all(|(key, _)| matches!(key, Value::String(_)));
    let has_reserved_key = entries.iter().any(|(key, _)| {
        matches!(key, Value::String(value) if value.as_str() == TRANSPORT_KEY || value.as_str() == "__proto__")
    });
    if all_strings && !has_reserved_key {
        let mut object = JsonMap::with_capacity(entries.len());
        for (key, value) in entries {
            let Value::String(key) = key else {
                return Err(napi_error("internal string-key mapping mismatch"));
            };
            object.insert(
                key.to_string(),
                value_to_transport(value, depth + 1, max_depth)?,
            );
        }
        return Ok(JsonValue::Object(object));
    }
    let pairs = entries
        .iter()
        .map(|(key, value)| {
            Ok(JsonValue::Array(vec![
                value_to_transport(key, depth + 1, max_depth)?,
                value_to_transport(value, depth + 1, max_depth)?,
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(marker(
        if all_strings { "object" } else { "mapping" },
        [("value", JsonValue::Array(pairs))],
    ))
}

fn marker<const N: usize>(kind: &str, fields: [(&str, JsonValue); N]) -> JsonValue {
    let mut object = JsonMap::with_capacity(N + 1);
    object.insert(TRANSPORT_KEY.to_owned(), JsonValue::String(kind.to_owned()));
    for (key, value) in fields {
        object.insert(key.to_owned(), value);
    }
    JsonValue::Object(object)
}
