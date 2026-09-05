//! Byte-first JSON, YAML, and TOML adapters for JavaScript values.

use std::fs::File;
use std::io::{BufWriter, Cursor, Write};
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, RecordBatch, RecordBatchOptions};
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{Schema, SchemaRef};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use napi::JsGlobal;
use napi::bindgen_prelude::{
    Array, ArrayBuffer, BigInt, Buffer, ClassInstance, Either, Env, Function, Generator,
    JavaScriptClassExt, JsObjectValue, JsValue, Object, Result, TypedArray, TypedArrayType,
    Uint8Array, Uint8ClampedArray, Unknown, ValueType,
};
use napi_derive::napi;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use yggdryl::text::{self, json, toml, yaml};
use yggdryl::text::{Format, Formatting, Indent, Limits, Scalar};
use yggdryl::types::decimal::{Decimal, Decimal32, Decimal64};
use yggdryl::types::integer::Integer;
use yggdryl::types::nested::Nested;
use yggdryl::types::temporal::Temporal;
use yggdryl::{
    ArrowCast, DataType as CoreDataType, DataTypeId, Enum, Field as CoreField,
    Fields as CoreFields, I256, MapType as CoreMapType, TemporalFamily, TimeUnit, Timezone,
};

use crate::types::timezone::{TimezoneInput, timezone_from_input};
use crate::{JsDataType, JsField, JsUri, JsUrl, JsUrn, napi_error};

/// Preserve the core's typed arithmetic failures as JavaScript error classes.
fn arithmetic_error(env: Env, error: yggdryl::Error) -> napi::Error {
    let reason = error.to_string();
    let thrown = match error {
        yggdryl::Error::InvalidArithmetic { .. } => {
            env.throw_type_error(&reason, Some("ERR_YGGDRYL_INVALID_ARITHMETIC"))
        }
        yggdryl::Error::ArithmeticOverflow { .. } => {
            env.throw_range_error(&reason, Some("ERR_YGGDRYL_ARITHMETIC_OVERFLOW"))
        }
        yggdryl::Error::DivisionByZero { .. } => {
            env.throw_range_error(&reason, Some("ERR_YGGDRYL_DIVISION_BY_ZERO"))
        }
        yggdryl::Error::InexactArithmetic { .. } => {
            env.throw_range_error(&reason, Some("ERR_YGGDRYL_INEXACT_ARITHMETIC"))
        }
        _ => env.throw_error(&reason, Some("ERR_YGGDRYL_ARITHMETIC")),
    };
    match thrown {
        Ok(()) => napi::Error::new(napi::Status::PendingException, reason),
        Err(error) => error,
    }
}

pub(crate) const DEFAULT_JS_DEPTH: usize = 48;
const MAX_JS_DEPTH: usize = 48;
const MAX_JS_DOCUMENTS: u32 = 1_024;
const TRANSPORT_KEY: &str = "__yggdryl_codec__";
const JS_SAFE_INTEGER: i128 = 9_007_199_254_740_991;
const JS_SAFE_INTEGER_F64: f64 = 9_007_199_254_740_991.0;
/// The longest prefix of an out-of-range bigint an error message repeats.
const MAX_REPORTED_DIGITS: usize = 40;
/// The widest instant a JavaScript `Date` holds, in milliseconds either way.
const MAX_DATE_MILLISECONDS: u64 = 8_640_000_000_000_000;

/// Nullable parser bounds normalized by the JavaScript codec facade.
#[napi(object)]
pub struct CodecLimitsInput {
    /// Maximum recursive container depth.
    pub max_depth: Option<f64>,
    /// Maximum encoded bytes read by one decoder.
    pub max_input_bytes: Option<f64>,
    /// Maximum decoded nodes per document.
    pub max_nodes: Option<f64>,
    /// Maximum values yielded by a multi-document decoder.
    pub max_documents: Option<f64>,
}

/// One native codec value: the pivot every JavaScript value crosses.
///
/// A `Scalar` is what the core actually stores, so it is also the honest answer
/// to "what did my JavaScript become". `fromJs` builds one from any JavaScript
/// value and `asJs` reads it back; a load and a dump are those two conversions
/// with bytes on the far side, and they run the same code.
///
/// It is also the JavaScript spelling of the values JavaScript has none of: an
/// exact decimal, a date, a time of day, a duration, and any timestamp whose
/// resolution or zone a `Date` cannot hold.
#[napi(js_name = "Scalar")]
#[derive(Clone)]
pub struct JsScalar {
    pub(crate) inner: Scalar,
}

/// An owning iterator over one native value's direct children.
///
/// The iterator snapshots cheap `Scalar` clones. Nested containers retain their
/// shared native storage, so an exact temporal or decimal never crosses the
/// lossy JavaScript projection merely because its parent is iterated.
#[napi(iterator, js_name = "ScalarIterator")]
pub struct JsScalarIterator {
    inner: std::vec::IntoIter<Scalar>,
}

impl Generator for JsScalarIterator {
    type Yield = JsScalar;
    type Next = ();
    type Return = ();

    fn next(&mut self, _value: Option<Self::Next>) -> Option<Self::Yield> {
        self.inner.next().map(JsScalar::from_core)
    }
}

impl JsScalar {
    /// Wrap one native value for JavaScript.
    pub(crate) const fn from_core(inner: Scalar) -> Self {
        Self { inner }
    }
}

#[napi]
impl JsScalar {
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

    /// Build an identity-preserving member of a core enum.
    #[napi(factory)]
    pub fn from_enum(kind: String, value: String) -> Result<Self> {
        Enum::from_parts(&kind, &value)
            .map(Scalar::from)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Build one floating scalar at 16, 32, or 64 bits.
    #[napi(factory)]
    pub fn float(value: f64, width: Option<f64>) -> Result<Self> {
        Scalar::from_float(value, crate::exact_u8(width.unwrap_or(64.0), "width")?)
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Build the narrowest exact decimal that holds the coefficient.
    #[napi(factory)]
    pub fn decimal(coefficient: BigInt, scale: Option<f64>) -> Result<Self> {
        Ok(Self::from_core(Scalar::from_decimal(
            exact_i256(&coefficient, "coefficient")?,
            crate::exact_i8(scale.unwrap_or(0.0), "scale")?,
        )))
    }

    /// Build the exact date width selected by its unit.
    #[napi(factory)]
    pub fn date(
        count: Either<BigInt, f64>,
        unit: Option<String>,
        timezone: Option<TimezoneInput<'_>>,
    ) -> Result<Self> {
        Scalar::from_date(
            exact_i64_input(count, "count")?,
            time_unit(unit.as_deref().unwrap_or("d"))?,
            timezone_or_naive(timezone)?,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Build the exact time-of-day width selected by its unit.
    #[napi(factory)]
    pub fn time(
        count: Either<BigInt, f64>,
        unit: String,
        timezone: Option<TimezoneInput<'_>>,
    ) -> Result<Self> {
        Scalar::from_time(
            exact_i64_input(count, "count")?,
            time_unit(&unit)?,
            timezone_or_naive(timezone)?,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Build an epoch or wall-clock datetime with a non-null timezone.
    #[napi(factory)]
    pub fn datetime(
        count: Either<BigInt, f64>,
        unit: String,
        timezone: Option<TimezoneInput<'_>>,
    ) -> Result<Self> {
        Scalar::from_datetime(
            exact_i64_input(count, "count")?,
            time_unit(&unit)?,
            timezone_or_naive(timezone)?,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Build the narrowest duration width that holds the count.
    #[napi(factory)]
    pub fn duration(
        count: Either<BigInt, f64>,
        unit: String,
        timezone: Option<TimezoneInput<'_>>,
    ) -> Result<Self> {
        Scalar::from_duration(
            exact_i64_input(count, "count")?,
            time_unit(&unit)?,
            timezone_or_naive(timezone)?,
        )
        .map(Self::from_core)
        .map_err(napi_error)
    }

    /// Rehydrate one exact decimal variant for the private JavaScript transport.
    #[napi(factory, js_name = "_fromDecimalPartsNative", skip_typescript)]
    pub fn from_decimal_parts_native(id: String, unscaled: BigInt, scale: f64) -> Result<Self> {
        let unscaled = exact_i256(&unscaled, "unscaled")?;
        let scale = crate::exact_i8(scale, "scale")?;
        let inner = match DataTypeId::from_str(&id).map_err(napi_error)? {
            DataTypeId::Decimal32 => Scalar::Decimal(Decimal::D32(Decimal32::new(
                unscaled
                    .as_i128()
                    .and_then(|value| i32::try_from(value).ok())
                    .ok_or_else(|| napi_error("decimal32 coefficient must fit signed 32 bits"))?,
                scale,
            ))),
            DataTypeId::Decimal64 => Scalar::Decimal(Decimal::D64(Decimal64::new(
                unscaled
                    .as_i128()
                    .and_then(|value| i64::try_from(value).ok())
                    .ok_or_else(|| napi_error("decimal64 coefficient must fit signed 64 bits"))?,
                scale,
            ))),
            DataTypeId::Decimal128 => Scalar::d128(
                unscaled
                    .as_i128()
                    .ok_or_else(|| napi_error("decimal128 coefficient must fit signed 128 bits"))?,
                scale,
            ),
            DataTypeId::Decimal256 => Scalar::d256(unscaled, scale),
            _ => return Err(napi_error(format!("{id:?} is not an exact decimal id"))),
        };
        Ok(Self::from_core(inner))
    }

    /// Rehydrate one exact temporal variant for the private JavaScript transport.
    #[napi(factory, js_name = "_fromTemporalPartsNative", skip_typescript)]
    pub fn from_temporal_parts_native(
        id: String,
        count: BigInt,
        unit: String,
        timezone: Option<TimezoneInput<'_>>,
    ) -> Result<Self> {
        let count = exact_i64(&count, "count")?;
        let unit = time_unit(&unit)?;
        let zone = timezone_or_naive(timezone)?;
        let inner = match DataTypeId::from_str(&id).map_err(napi_error)? {
            DataTypeId::Date32 => {
                Scalar::date32_in(i32::try_from(count).map_err(napi_error)?, unit, zone)
            }
            DataTypeId::Date64 => Scalar::date64_in(count, unit, zone),
            DataTypeId::Time32 => {
                Scalar::time32(i32::try_from(count).map_err(napi_error)?, unit, zone)
            }
            DataTypeId::Time64 => Scalar::time64(count, unit, zone),
            DataTypeId::DateTime64 => Scalar::datetime64(count, unit, zone),
            DataTypeId::Duration32 => {
                Scalar::duration32_in(i32::try_from(count).map_err(napi_error)?, unit, zone)
            }
            DataTypeId::Duration64 => Scalar::duration64_in(count, unit, zone),
            _ => return Err(napi_error(format!("{id:?} is not an exact temporal id"))),
        }
        .map_err(napi_error)?;
        Ok(Self::from_core(inner))
    }

    /// The canonical width-specific vocabulary name.
    #[napi(getter)]
    pub fn kind(&self) -> String {
        self.inner.kind().to_owned()
    }

    /// The enum vocabulary name, when this scalar is an enum.
    #[napi(getter)]
    pub fn enum_kind(&self) -> Option<String> {
        self.inner.as_enum().map(|value| value.kind().to_owned())
    }

    /// The canonical enum member spelling, when this scalar is an enum.
    #[napi(getter)]
    pub fn enum_value(&self) -> Option<String> {
        self.inner.as_enum().map(|value| value.as_str().to_owned())
    }

    /// The compact zero-based member index, when this scalar is an enum.
    #[napi(getter)]
    pub fn enum_ordinal(&self) -> Option<u8> {
        self.inner.as_enum().map(|value| value.ordinal())
    }

    /// The number of direct sequence children, mapping entries, or record fields.
    #[napi(getter)]
    pub fn length(&self) -> f64 {
        #[allow(clippy::cast_precision_loss)]
        let length = self.inner.len() as f64;
        length
    }

    /// Whether this is an empty sequence, mapping, or record.
    #[napi]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up one non-negative sequence index without projecting its value.
    #[napi]
    pub fn at(&self, index: f64) -> Result<Option<JsScalar>> {
        let index = crate::exact_u64(index, "index")?;
        let index = usize::try_from(index)
            .map_err(|_| napi_error(format!("index {index} exceeds this platform's range")))?;
        Ok(self.inner.get(index).cloned().map(Self::from_core))
    }

    /// Look up a dotted mapping/record key and sequence-index path.
    #[napi]
    pub fn path(&self, path: String) -> Option<JsScalar> {
        self.inner.path(&path).cloned().map(Self::from_core)
    }

    /// Iterate direct children under the core convention.
    ///
    /// Sequences yield values, mappings yield keys, and records yield values
    /// in deterministic field-name order.
    #[napi(js_name = "_iterNative", skip_typescript)]
    pub fn iter_native(&self) -> JsScalarIterator {
        JsScalarIterator {
            inner: self.inner.iter().cloned().collect::<Vec<_>>().into_iter(),
        }
    }

    /// Look up a mapping key or record name without a JavaScript projection.
    #[napi(js_name = "_getNative", skip_typescript)]
    pub fn get_native(&self, key: &JsScalar) -> Option<JsScalar> {
        let value = match &self.inner {
            Scalar::Nested(Nested::Record(_)) => key
                .inner
                .as_str()
                .and_then(|name| self.inner.get_key_str(name)),
            Scalar::Nested(Nested::Mapping(_)) => self.inner.get_key(&key.inner),
            _ => None,
        };
        value.cloned().map(Self::from_core)
    }

    /// Return this mapping or record with one persistent replacement.
    #[napi(js_name = "_setNative", skip_typescript)]
    pub fn set_native(&self, key: &JsScalar, value: &JsScalar) -> Result<Self> {
        let rebuilt = match &self.inner {
            Scalar::Nested(Nested::Mapping(_)) => {
                self.inner.with_key(key.inner.clone(), value.inner.clone())
            }
            Scalar::Nested(Nested::Record(_)) => {
                let name = key
                    .inner
                    .as_str()
                    .ok_or_else(|| napi_error("record field names must be strings"))?;
                self.inner.with_field(name, value.inner.clone())
            }
            _ => {
                return Err(napi_error(format!(
                    "expected a mapping or record to set a value on, got {}",
                    self.inner.kind()
                )));
            }
        };
        rebuilt.map(Self::from_core).map_err(napi_error)
    }

    /// Return this mapping or record without one string key.
    #[napi(js_name = "_removeNative", skip_typescript)]
    pub fn remove_native(&self, key: &JsScalar) -> Result<Self> {
        let name = key
            .inner
            .as_str()
            .ok_or_else(|| napi_error("remove requires a string key"))?;
        let rebuilt = match &self.inner {
            Scalar::Nested(Nested::Mapping(_)) => self.inner.without_key(name),
            Scalar::Nested(Nested::Record(_)) => self.inner.without_field(name),
            _ => {
                return Err(napi_error(format!(
                    "expected a mapping or record to remove a value from, got {}",
                    self.inner.kind()
                )));
            }
        };
        rebuilt.map(Self::from_core).map_err(napi_error)
    }

    /// The count a temporal holds, or `null`.
    #[napi(getter)]
    pub fn count(&self) -> Option<BigInt> {
        self.inner
            .as_temporal()
            .map(|value| BigInt::from(value.count()))
    }

    /// The unit carried by a temporal, or `null`.
    #[napi(getter)]
    pub fn unit(&self) -> Option<String> {
        self.inner
            .as_temporal()
            .map(|value| value.unit().as_str().to_owned())
    }

    /// The non-null timezone marker carried by a temporal, or `null`.
    #[napi(getter)]
    pub fn zone(&self) -> Option<String> {
        self.inner.temporal_timezone().map(|zone| zone.to_string())
    }

    /// The unscaled coefficient of an exact decimal, or `null`.
    #[napi(getter)]
    pub fn unscaled(&self) -> Option<BigInt> {
        self.inner
            .as_decimal()
            .map(|(unscaled, _)| bigint_from_i256(unscaled))
    }

    /// The scale of an exact decimal, or `null`.
    #[napi(getter)]
    pub fn scale(&self) -> Option<i32> {
        self.inner.as_decimal().map(|(_, scale)| i32::from(scale))
    }

    /// Infer the exact native datatype this value names.
    #[napi(getter)]
    pub fn dtype(&self) -> Result<JsDataType> {
        self.inner
            .dtype()
            .map(JsDataType::from_core)
            .map_err(napi_error)
    }

    /// Infer the exact native Field for this scalar value.
    #[napi]
    pub fn into_field(&self) -> Result<JsField> {
        self.inner
            .inferred_scalar_field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Infer the exact item Field for this non-empty outer Sequence.
    #[napi]
    pub fn into_array_field(&self) -> Result<JsField> {
        self.inner
            .inferred_array_field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Infer a non-null Struct root from named Record rows.
    #[napi]
    pub fn into_struct_field(&self) -> Result<JsField> {
        self.inner
            .inferred_struct_field()
            .map(JsField::from_core)
            .map_err(napi_error)
    }

    /// Return deterministic hash bits shared with Rust and Python.
    #[napi]
    pub fn stable_hash(&self) -> u64 {
        self.inner.stable_hash()
    }

    /// Digest this value's canonical byte representation.
    ///
    /// Equal values answer equal digests across integer, float, decimal, and
    /// temporal widths, because the feed writes each family's canonical form
    /// rather than its storage width.
    #[napi]
    pub fn digest(&self, algorithm: Option<String>) -> Result<crate::xxhash::JsDigest> {
        let algorithm = match algorithm.as_deref() {
            Some(algorithm) => crate::xxhash::algorithm_from_str(algorithm)?,
            None => yggdryl::DigestAlgorithm::Xxh3_64,
        };
        Ok(crate::xxhash::JsDigest::from_core(
            self.inner.digest(algorithm),
        ))
    }

    /// Compare two native values by the core's total value order.
    ///
    /// Numeric widths compare by value, as equality does: `i8(1)` and
    /// `u64(1)` compare equal, and exact decimals are normalized first.
    #[napi]
    pub fn compare(&self, other: &JsScalar) -> i32 {
        crate::ordering_value(self.inner.cmp(&other.inner))
    }

    /// Make a cheap native clone without projecting through JavaScript.
    #[napi(js_name = "clone")]
    pub fn clone_js(&self) -> Self {
        self.clone()
    }

    /// Add two already-inferred native numeric values.
    #[napi(js_name = "_addNative", skip_typescript)]
    pub fn add_native(&self, env: Env, other: &JsScalar) -> Result<Self> {
        self.inner
            .checked_add(&other.inner)
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Subtract two already-inferred native numeric values.
    #[napi(js_name = "_subtractNative", skip_typescript)]
    pub fn subtract_native(&self, env: Env, other: &JsScalar) -> Result<Self> {
        self.inner
            .checked_sub(&other.inner)
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Multiply two already-inferred native numeric values.
    #[napi(js_name = "_multiplyNative", skip_typescript)]
    pub fn multiply_native(&self, env: Env, other: &JsScalar) -> Result<Self> {
        self.inner
            .checked_mul(&other.inner)
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Divide two already-inferred native numeric values.
    #[napi(js_name = "_divideNative", skip_typescript)]
    pub fn divide_native(&self, env: Env, other: &JsScalar) -> Result<Self> {
        self.inner
            .checked_div(&other.inner)
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Take the remainder of two already-inferred native numeric values.
    #[napi(js_name = "_remainderNative", skip_typescript)]
    pub fn remainder_native(&self, env: Env, other: &JsScalar) -> Result<Self> {
        self.inner
            .checked_rem(&other.inner)
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Negate one already-inferred native numeric value.
    #[napi(js_name = "_negateNative", skip_typescript)]
    pub fn negate_native(&self, env: Env) -> Result<Self> {
        self.inner
            .checked_neg()
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Return the absolute value of one already-inferred native number.
    #[napi(js_name = "_absoluteNative", skip_typescript)]
    pub fn absolute_native(&self, env: Env) -> Result<Self> {
        self.inner
            .checked_abs()
            .map(Self::from_core)
            .map_err(|error| arithmetic_error(env, error))
    }

    /// Borrow binary content as a Buffer, or `null` for another kind.
    #[napi]
    pub fn as_bytes(&self) -> Option<Buffer> {
        self.inner.as_bytes().map(|value| value.to_vec().into())
    }

    /// Borrow string content, or `null` for another kind.
    #[napi]
    pub fn as_utf8(&self) -> Option<String> {
        self.inner.as_utf8().map(ToOwned::to_owned)
    }

    /// Encode this value as natural compact JSON bytes.
    #[napi]
    pub fn as_json_bytes(&self) -> Result<Buffer> {
        self.inner
            .as_json_bytes()
            .map(Into::into)
            .map_err(napi_error)
    }

    /// Encode this value as natural compact JSON UTF-8.
    #[napi]
    pub fn as_json_utf8(&self) -> Result<String> {
        self.inner.as_json_utf8().map_err(napi_error)
    }

    /// Natural compact JSON for the standard JavaScript string protocol.
    #[napi(js_name = "toString")]
    pub fn js_string(&self) -> Result<String> {
        self.as_json_utf8()
    }

    /// Whether two native values are the same value.
    ///
    /// One instant spelled in two resolutions is one value, and so are two
    /// spellings of one exact decimal, because the core compares what a value
    /// names rather than how it was written.
    #[napi]
    pub fn equals(&self, other: &JsScalar) -> bool {
        self.inner == other.inner
    }

    /// Decode one Arrow JS scalar from its one-column IPC bridge.
    #[napi(factory, js_name = "_fromArrowScalarIpcNative", skip_typescript)]
    pub fn from_arrow_scalar_ipc_native(
        bytes: Uint8Array,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Self> {
        let (schema, batches) = arrow_batches(&bytes)?;
        ensure_one_column(&schema, "Arrow scalar")?;
        let rows = batches.iter().map(RecordBatch::num_rows).sum::<usize>();
        if rows != 1 {
            return Err(napi_error(format!(
                "Arrow scalar IPC must contain exactly one row, got {rows}"
            )));
        }
        let inferred =
            CoreField::from_arrow_ref(Arc::clone(&schema.fields()[0])).map_err(napi_error)?;
        let field = field
            .as_ref()
            .map_or_else(|| inferred.clone(), |field| field.inner.clone());
        let array = batches
            .into_iter()
            .find(|batch| batch.num_rows() != 0)
            .and_then(|batch| batch.columns().first().cloned())
            .ok_or_else(|| napi_error("Arrow scalar IPC has no value column"))?;
        let array = if field == inferred {
            array
        } else {
            field.cast_arrow_array(array, true).map_err(napi_error)?
        };
        yggdryl::arrow::scalar_value(&field, array.as_ref())
            .map(Self::from_core)
            .map_err(napi_error)
    }

    /// Decode one Arrow JS vector from its one-column IPC bridge.
    #[napi(factory, js_name = "_fromArrowArrayIpcNative", skip_typescript)]
    pub fn from_arrow_array_ipc_native(
        bytes: Uint8Array,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Self> {
        let (schema, batches) = arrow_batches(&bytes)?;
        ensure_one_column(&schema, "Arrow array")?;
        let inferred =
            CoreField::from_arrow_ref(Arc::clone(&schema.fields()[0])).map_err(napi_error)?;
        let field = field
            .as_ref()
            .map_or_else(|| inferred.clone(), |field| field.inner.clone());
        let mut values = Vec::new();
        for batch in batches {
            let array = batch
                .columns()
                .first()
                .cloned()
                .ok_or_else(|| napi_error("Arrow array IPC has no value column"))?;
            let array = if field == inferred {
                array
            } else {
                field.cast_arrow_array(array, true).map_err(napi_error)?
            };
            let decoded =
                yggdryl::arrow::array_to_value(&field, array.as_ref()).map_err(napi_error)?;
            values.extend_from_slice(decoded.as_sequence().ok_or_else(|| {
                napi_error("native Arrow array decode did not return a sequence")
            })?);
        }
        Ok(Self::from_core(Scalar::from_sequence(values)))
    }

    /// Decode Arrow JS record batches from standard IPC.
    #[napi(factory, js_name = "_fromArrowBatchIpcNative", skip_typescript)]
    pub fn from_arrow_batch_ipc_native(
        bytes: Uint8Array,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Self> {
        Self::from_arrow_batches_ipc(&bytes, field)
    }

    /// Decode an Arrow JS table from standard IPC.
    #[napi(factory, js_name = "_fromArrowTableIpcNative", skip_typescript)]
    pub fn from_arrow_table_ipc_native(
        bytes: Uint8Array,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Self> {
        Self::from_arrow_batches_ipc(&bytes, field)
    }

    /// Encode this value as a one-row, one-column Arrow IPC scalar.
    #[napi(js_name = "_intoArrowScalarIpcNative", skip_typescript)]
    pub fn into_arrow_scalar_ipc_native(
        &self,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Buffer> {
        let field = field
            .as_ref()
            .map(|field| field.inner.clone())
            .map_or_else(
                || self.inner.inferred_scalar_field().map_err(napi_error),
                Ok,
            )?;
        let array = yggdryl::arrow::scalar_array(&field, &self.inner).map_err(napi_error)?;
        arrow_array_ipc(&field, array)
    }

    /// Encode this sequence as a one-column Arrow IPC array.
    #[napi(js_name = "_intoArrowArrayIpcNative", skip_typescript)]
    pub fn into_arrow_array_ipc_native(
        &self,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Buffer> {
        let field = field
            .as_ref()
            .map(|field| field.inner.clone())
            .map_or_else(|| self.inner.inferred_array_field().map_err(napi_error), Ok)?;
        let array = yggdryl::arrow::array_from_value(&field, &self.inner).map_err(napi_error)?;
        arrow_array_ipc(&field, array)
    }

    /// Encode record values as one Arrow IPC record batch.
    #[napi(js_name = "_intoArrowBatchIpcNative", skip_typescript)]
    pub fn into_arrow_batch_ipc_native(
        &self,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Buffer> {
        self.arrow_batches_ipc(field)
    }

    /// Encode record values as one Arrow IPC table.
    #[napi(js_name = "_intoArrowTableIpcNative", skip_typescript)]
    pub fn into_arrow_table_ipc_native(
        &self,
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Buffer> {
        self.arrow_batches_ipc(field)
    }
}

impl JsScalar {
    fn from_arrow_batches_ipc(
        bytes: &[u8],
        field: Option<ClassInstance<'_, JsField>>,
    ) -> Result<Self> {
        let (schema, batches) = arrow_batches(bytes)?;
        let inferred = CoreField::from_arrow_schema("row", schema.as_ref()).map_err(napi_error)?;
        let field = field
            .as_ref()
            .map_or_else(|| inferred.clone(), |field| field.inner.clone());
        field.validate_struct_root().map_err(napi_error)?;
        let mut rows = Vec::new();
        for batch in batches {
            let batch = if field == inferred {
                batch
            } else {
                field.cast_arrow_batch(batch, true).map_err(napi_error)?
            };
            let decoded = yggdryl::arrow::batch_to_value(&batch).map_err(napi_error)?;
            rows.extend_from_slice(
                decoded
                    .as_sequence()
                    .ok_or_else(|| napi_error("native Arrow batch decode did not return rows"))?,
            );
        }
        Ok(Self::from_core(Scalar::from_sequence(rows)))
    }

    fn arrow_batches_ipc(&self, field: Option<ClassInstance<'_, JsField>>) -> Result<Buffer> {
        let field = field
            .as_ref()
            .map(|field| field.inner.clone())
            .map_or_else(
                || self.inner.inferred_struct_field().map_err(napi_error),
                Ok,
            )?;
        let batch = yggdryl::arrow::batch_from_value(&field, &self.inner).map_err(napi_error)?;
        arrow_batches_into_ipc(batch.schema(), [batch])
    }
}

pub(crate) fn arrow_batches(bytes: &[u8]) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    if bytes.is_empty() {
        return Err(napi_error("Arrow IPC input is empty and has no schema"));
    }
    let mut reader =
        StreamReader::try_new(Cursor::new(bytes.to_vec()), None).map_err(napi_error)?;
    let schema = reader.schema();
    let batches = reader
        .by_ref()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(napi_error)?;
    Ok((schema, batches))
}

pub(crate) fn ensure_one_column(schema: &SchemaRef, label: &str) -> Result<()> {
    if schema.fields().len() == 1 {
        Ok(())
    } else {
        Err(napi_error(format!(
            "{label} IPC must contain exactly one column, got {}",
            schema.fields().len()
        )))
    }
}

pub(crate) fn arrow_array_ipc(field: &CoreField, array: ArrayRef) -> Result<Buffer> {
    let schema = Arc::new(Schema::new([field
        .clone()
        .into_arrow_ref()
        .map_err(napi_error)?]));
    let options = RecordBatchOptions::new().with_row_count(Some(array.len()));
    let batch =
        RecordBatch::try_new_with_options(schema, vec![array], &options).map_err(napi_error)?;
    arrow_batches_into_ipc(batch.schema(), [batch])
}

fn arrow_batches_into_ipc(
    schema: SchemaRef,
    batches: impl IntoIterator<Item = RecordBatch>,
) -> Result<Buffer> {
    let mut writer = StreamWriter::try_new(Vec::new(), schema.as_ref()).map_err(napi_error)?;
    for batch in batches {
        writer.write(&batch).map_err(napi_error)?;
    }
    writer.finish().map_err(napi_error)?;
    Ok(writer.into_inner().map_err(napi_error)?.into())
}

fn time_unit(value: &str) -> Result<TimeUnit> {
    TimeUnit::from_str(value).map_err(napi_error)
}

fn timezone_or_naive(value: Option<TimezoneInput<'_>>) -> Result<Timezone> {
    value.map_or(Ok(Timezone::NAIVE), timezone_from_input)
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

fn exact_i64_input(value: Either<BigInt, f64>, name: &str) -> Result<i64> {
    match value {
        Either::A(value) => exact_i64(&value, name),
        Either::B(value) => crate::exact_i64(value, name),
    }
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
    field: Option<&ClassInstance<'_, JsField>>,
    placeholders: Option<ClassInstance<'_, JsScalar>>,
    environment: bool,
) -> Result<yggdryl::text::Loading> {
    let loading = yggdryl::text::Loading::new().with_limits(limits);
    let loading = match field {
        Some(field) => loading.with_field(field.inner.clone()),
        None => loading,
    };
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
///
/// No placeholder parameters: `{{ }}` substitution is a YAML and TOML
/// feature, and the core refuses it for JSON by name.
#[napi(js_name = "jsonLoadsNative", skip_typescript)]
pub fn json_loads_native(
    input: Either<Buffer, String>,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let value = match (&input, field.as_ref()) {
        (Either::A(bytes), Some(field)) => text::from_bytes_with_field_and_limits(
            bytes.as_ref(),
            Format::Json,
            &field.inner,
            limits,
        ),
        (Either::B(value), Some(field)) => {
            text::from_utf8_with_field_and_limits(value, Format::Json, &field.inner, limits)
        }
        (Either::A(bytes), None) => {
            text::from_bytes_with_limits(bytes.as_ref(), Format::Json, limits)
        }
        (Either::B(value), None) => text::from_utf8_with_limits(value, Format::Json, limits),
    }
    .map_err(napi_error)?;
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
}

/// Decode one YAML value without generic format parsing or dispatch.
#[napi(js_name = "yamlLoadsNative", skip_typescript)]
pub fn yaml_loads_native(
    input: Either<Buffer, String>,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    placeholders: Option<ClassInstance<'_, JsScalar>>,
    environment: Option<bool>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let loading = loading_from(
        limits,
        field.as_ref(),
        placeholders,
        environment.unwrap_or(false),
    )?;
    let value = match &input {
        Either::A(bytes) => text::from_bytes_with(bytes.as_ref(), Format::Yaml, &loading),
        Either::B(value) => text::from_utf8_with(value, Format::Yaml, &loading),
    }
    .map_err(napi_error)?;
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
}

/// Decode one TOML value without generic format parsing or dispatch.
#[napi(js_name = "tomlLoadsNative", skip_typescript)]
pub fn toml_loads_native(
    input: Either<Buffer, String>,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    placeholders: Option<ClassInstance<'_, JsScalar>>,
    environment: Option<bool>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let loading = loading_from(
        limits,
        field.as_ref(),
        placeholders,
        environment.unwrap_or(false),
    )?;
    let value = match &input {
        Either::A(bytes) => text::from_bytes_with(bytes.as_ref(), Format::Toml, &loading),
        Either::B(value) => text::from_utf8_with(value, Format::Toml, &loading),
    }
    .map_err(napi_error)?;
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
}

/// Decode strict JSON Lines without generic format parsing or dispatch.
#[napi(js_name = "jsonLinesLoadsNative", skip_typescript)]
pub fn json_lines_loads_native(
    input: Either<Buffer, String>,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Vec<Either<JsScalar, JsonValue>>> {
    let limits = checked_limits(limits)?;
    let values = match (&input, field.as_ref()) {
        (Either::A(bytes), Some(field)) => text::from_bytes_all_with_field_and_limits(
            bytes.as_ref(),
            Format::JsonLines,
            &field.inner,
            limits,
        ),
        (Either::B(value), Some(field)) => text::from_utf8_all_with_field_and_limits(
            value,
            Format::JsonLines,
            &field.inner,
            limits,
        ),
        (Either::A(bytes), None) => json::from_lines_bytes_with_limits(bytes.as_ref(), limits),
        (Either::B(value), None) => {
            text::from_utf8_all_with_limits(value, Format::JsonLines, limits)
        }
    }
    .map_err(napi_error)?;
    decoded_values_for_field(
        values,
        field.as_ref().map(|field| &field.inner),
        limits,
        native_scalar.unwrap_or(false),
    )
}

/// Decode every YAML document without generic format parsing or dispatch.
#[napi(js_name = "yamlLoadsAllNative", skip_typescript)]
pub fn yaml_loads_all_native(
    input: Either<Buffer, String>,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Vec<Either<JsScalar, JsonValue>>> {
    let limits = checked_limits(limits)?;
    let values = match (&input, field.as_ref()) {
        (Either::A(bytes), Some(field)) => {
            yaml::from_bytes_all_with_field_and_limits(bytes.as_ref(), &field.inner, limits)
        }
        (Either::B(value), Some(field)) => {
            yaml::from_utf8_all_with_field_and_limits(value, &field.inner, limits)
        }
        (Either::A(bytes), None) => yaml::from_bytes_all_with_limits(bytes.as_ref(), limits),
        (Either::B(value), None) => yaml::from_utf8_all_with_limits(value, limits),
    }
    .map_err(napi_error)?;
    decoded_values_for_field(
        values,
        field.as_ref().map(|field| &field.inner),
        limits,
        native_scalar.unwrap_or(false),
    )
}

/// Encode one JavaScript value directly to JSON bytes.
#[napi(js_name = "jsonDumpsNative", skip_typescript)]
pub fn json_dumps_native(
    env: Env,
    value: Unknown<'_>,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let formatting = checked_formatting(&indent)?;
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    json::into_bytes_with_formatting(&value, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}

fn exact_i256(value: &BigInt, name: &str) -> Result<I256> {
    if value.words.len() > 4 && value.words[4..].iter().any(|word| *word != 0) {
        return Err(napi_error(format!(
            "{name} must fit in a signed 256-bit integer"
        )));
    }
    let mut bytes = [0_u8; 32];
    for (index, word) in value.words.iter().take(4).enumerate() {
        bytes[index * 8..index * 8 + 8].copy_from_slice(&word.to_le_bytes());
    }
    if value.sign_bit {
        let exceeds_minimum =
            bytes[31] > 0x80 || (bytes[31] == 0x80 && bytes[..31].iter().any(|byte| *byte != 0));
        if exceeds_minimum {
            return Err(napi_error(format!(
                "{name} must fit in a signed 256-bit integer"
            )));
        }
        if bytes.iter().any(|byte| *byte != 0) {
            for byte in &mut bytes {
                *byte = !*byte;
            }
            for byte in &mut bytes {
                let (next, carry) = byte.overflowing_add(1);
                *byte = next;
                if !carry {
                    break;
                }
            }
        }
    } else if bytes[31] & 0x80 != 0 {
        return Err(napi_error(format!(
            "{name} must fit in a signed 256-bit integer"
        )));
    }
    Ok(I256::from_le_bytes(bytes))
}

fn bigint_from_i256(value: I256) -> BigInt {
    let sign_bit = value.is_negative();
    let mut bytes = value.into_le_bytes();
    if sign_bit {
        for byte in &mut bytes {
            *byte = !*byte;
        }
        for byte in &mut bytes {
            let (next, carry) = byte.overflowing_add(1);
            *byte = next;
            if !carry {
                break;
            }
        }
    }
    let mut words = bytes
        .chunks(8)
        .map(|word| u64::from_le_bytes(word.try_into().unwrap_or([0; 8])))
        .collect::<Vec<_>>();
    while words.len() > 1 && words.last() == Some(&0) {
        words.pop();
    }
    BigInt { sign_bit, words }
}

/// Encode one JavaScript value directly to YAML bytes.
#[napi(js_name = "yamlDumpsNative", skip_typescript)]
pub fn yaml_dumps_native(
    env: Env,
    value: Unknown<'_>,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let formatting = checked_formatting(&indent)?;
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    yaml::into_bytes_with_formatting(&value, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode one JavaScript value directly to TOML bytes.
#[napi(js_name = "tomlDumpsNative", skip_typescript)]
pub fn toml_dumps_native(
    env: Env,
    value: Unknown<'_>,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let formatting = checked_formatting(&indent)?;
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
    toml::into_bytes_with_formatting(&value, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode JavaScript values directly as JSON Lines.
#[napi(js_name = "jsonLinesDumpAllNative", skip_typescript)]
pub fn json_lines_dump_all_native(
    env: Env,
    values: Array<'_>,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let formatting = checked_formatting(&indent)?;
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    json::into_bytes_all_with_formatting(&values, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Encode JavaScript values directly as YAML documents.
#[napi(js_name = "yamlDumpAllNative", skip_typescript)]
pub fn yaml_dump_all_native(
    env: Env,
    values: Array<'_>,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<Buffer> {
    let formatting = checked_formatting(&indent)?;
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    yaml::into_bytes_all_with_formatting(&values, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Decode one JSON value from a path through the native reader boundary.
///
/// No placeholder parameters: `{{ }}` substitution is a YAML and TOML
/// feature, and the core refuses it for JSON by name.
#[napi(js_name = "jsonLoadPathNative", skip_typescript)]
pub fn json_load_path_native(
    path: String,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let reader = open_path(&path, limits.max_input_bytes())?;
    let value = match field.as_ref() {
        Some(field) => {
            text::from_reader_with_field_and_limits(reader, Format::Json, &field.inner, limits)
        }
        None => text::from_reader_with_limits(reader, Format::Json, limits),
    }
    .map_err(napi_error)?;
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
}

/// Decode one YAML value from a path through the native reader boundary.
#[napi(js_name = "yamlLoadPathNative", skip_typescript)]
pub fn yaml_load_path_native(
    path: String,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    placeholders: Option<ClassInstance<'_, JsScalar>>,
    environment: Option<bool>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let loading = loading_from(
        limits,
        field.as_ref(),
        placeholders,
        environment.unwrap_or(false),
    )?;
    let value = text::from_reader_with(
        open_path(&path, limits.max_input_bytes())?,
        Format::Yaml,
        &loading,
    )
    .map_err(napi_error)?;
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
}

/// Decode one TOML value from a path through the native reader boundary.
#[napi(js_name = "tomlLoadPathNative", skip_typescript)]
pub fn toml_load_path_native(
    path: String,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    placeholders: Option<ClassInstance<'_, JsScalar>>,
    environment: Option<bool>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let loading = loading_from(
        limits,
        field.as_ref(),
        placeholders,
        environment.unwrap_or(false),
    )?;
    let value = text::from_reader_with(
        open_path(&path, limits.max_input_bytes())?,
        Format::Toml,
        &loading,
    )
    .map_err(napi_error)?;
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
}

/// Decode strict JSON Lines from a path through the native reader boundary.
#[napi(js_name = "jsonLinesLoadPathNative", skip_typescript)]
pub fn json_lines_load_path_native(
    path: String,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Vec<Either<JsScalar, JsonValue>>> {
    let limits = checked_limits(limits)?;
    let reader = open_path(&path, limits.max_input_bytes())?;
    let values = match field.as_ref() {
        Some(field) => text::from_reader_all_with_field_and_limits(
            reader,
            Format::JsonLines,
            &field.inner,
            limits,
        ),
        None => json::from_lines_reader_with_limits(reader, limits),
    }
    .map_err(napi_error)?;
    decoded_values_for_field(
        values,
        field.as_ref().map(|field| &field.inner),
        limits,
        native_scalar.unwrap_or(false),
    )
}

/// Decode every YAML document from a path through the native reader boundary.
#[napi(js_name = "yamlLoadAllPathNative", skip_typescript)]
pub fn yaml_load_all_path_native(
    path: String,
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Vec<Either<JsScalar, JsonValue>>> {
    let limits = checked_limits(limits)?;
    let reader = open_path(&path, limits.max_input_bytes())?;
    let values = match field.as_ref() {
        Some(field) => yaml::from_reader_all_with_field_and_limits(reader, &field.inner, limits),
        None => yaml::from_reader_all_with_limits(reader, limits),
    }
    .map_err(napi_error)?;
    decoded_values_for_field(
        values,
        field.as_ref().map(|field| &field.inner),
        limits,
        native_scalar.unwrap_or(false),
    )
}

/// Encode one JavaScript value directly to a JSON file writer.
#[napi(js_name = "jsonDumpPathNative", skip_typescript)]
pub fn json_dump_path_native(
    env: Env,
    value: Unknown<'_>,
    path: String,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let formatting = checked_formatting(&indent)?;
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    json::into_writer_with_formatting(&value, &mut writer, formatting).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode one JavaScript value directly to a YAML file writer.
#[napi(js_name = "yamlDumpPathNative", skip_typescript)]
pub fn yaml_dump_path_native(
    env: Env,
    value: Unknown<'_>,
    path: String,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let formatting = checked_formatting(&indent)?;
    let value = encode_js_value(
        env,
        value,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    yaml::into_writer_with_formatting(&value, &mut writer, formatting).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode one JavaScript value directly to a TOML file writer.
#[napi(js_name = "tomlDumpPathNative", skip_typescript)]
pub fn toml_dump_path_native(
    env: Env,
    value: Unknown<'_>,
    path: String,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let formatting = checked_formatting(&indent)?;
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
    toml::into_writer_with_formatting(&value, &mut writer, formatting).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode JavaScript values directly to a JSON Lines file writer.
#[napi(js_name = "jsonLinesDumpPathNative", skip_typescript)]
pub fn json_lines_dump_path_native(
    env: Env,
    values: Array<'_>,
    path: String,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let formatting = checked_formatting(&indent)?;
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    json::into_writer_all_with_formatting(&values, &mut writer, formatting).map_err(napi_error)?;
    writer.flush().map_err(napi_error)
}

/// Encode JavaScript values directly to a YAML document file writer.
#[napi(js_name = "yamlDumpAllPathNative", skip_typescript)]
pub fn yaml_dump_all_path_native(
    env: Env,
    values: Array<'_>,
    path: String,
    max_depth: Option<u32>,
    indent: String,
    native_wrapper_prototypes: Array<'_>,
    native_intrinsics: Array<'_>,
) -> Result<()> {
    let formatting = checked_formatting(&indent)?;
    let values = encode_js_values(
        env,
        &values,
        checked_depth(max_depth)?,
        &native_wrapper_prototypes,
        &native_intrinsics,
    )?;
    let mut writer = create_path(&path)?;
    yaml::into_writer_all_with_formatting(&values, &mut writer, formatting).map_err(napi_error)?;
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
    limits: Option<CodecLimitsInput>,
    field: Option<ClassInstance<'_, JsField>>,
    native_scalar: Option<bool>,
) -> Result<Either<JsScalar, JsonValue>> {
    let limits = checked_limits(limits)?;
    let (_, value) = match &input {
        Either::A(bytes) => text::from_bytes_inferred_with_limits(bytes.as_ref(), limits),
        Either::B(value) => text::from_utf8_inferred_with_limits(value, limits),
    }
    .map_err(napi_error)?;
    let value = match field.as_ref() {
        Some(field) => field.inner.from_natural_value(value).map_err(napi_error)?,
        None => value,
    };
    decoded_value_for_field(
        value,
        field.as_ref().map(|field| &field.inner),
        limits.max_depth(),
        native_scalar.unwrap_or(false),
    )
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
) -> Result<Scalar> {
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
) -> Result<Vec<Scalar>> {
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

fn decoded_values_for_field(
    values: Vec<Scalar>,
    field: Option<&CoreField>,
    limits: Limits,
    native_scalar: bool,
) -> Result<Vec<Either<JsScalar, JsonValue>>> {
    values
        .into_iter()
        .map(|value| decoded_value_for_field(value, field, limits.max_depth(), native_scalar))
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

fn checked_limit(value: Option<f64>, name: &str, default: usize) -> Result<usize> {
    let Some(value) = value else {
        return Ok(default);
    };
    let value = crate::exact_u64(value, name)?;
    usize::try_from(value)
        .map_err(|_| napi_error(format!("{name} {value} exceeds this platform's range")))
}

fn checked_limits(input: Option<CodecLimitsInput>) -> Result<Limits> {
    let defaults = Limits::default();
    let input = input.unwrap_or(CodecLimitsInput {
        max_depth: None,
        max_input_bytes: None,
        max_nodes: None,
        max_documents: None,
    });
    let max_depth = checked_limit(input.max_depth, "maxDepth", DEFAULT_JS_DEPTH)?;
    if !(1..=MAX_JS_DEPTH).contains(&max_depth) {
        return Err(napi_error(format!(
            "maxDepth must be between 1 and {MAX_JS_DEPTH}"
        )));
    }
    Ok(Limits::new(
        max_depth,
        checked_limit(
            input.max_input_bytes,
            "maxInputBytes",
            defaults.max_input_bytes(),
        )?,
        checked_limit(input.max_nodes, "maxNodes", defaults.max_nodes())?,
        checked_limit(
            input.max_documents,
            "maxDocuments",
            defaults.max_documents(),
        )?,
    ))
}

fn checked_formatting(indent: &str) -> Result<Formatting> {
    match indent {
        "default" => Ok(Formatting::default()),
        "none" => Ok(Formatting::compact()),
        "tabs" => Ok(Formatting::default().with_indent(Indent::Tabs)),
        _ => {
            let width = indent
                .strip_prefix("spaces:")
                .ok_or_else(|| napi_error(format!("invalid native indent mode {indent:?}")))?
                .parse::<u8>()
                .map_err(|_| napi_error(format!("invalid native indent width {indent:?}")))?;
            Ok(Formatting::default().with_indent(Indent::Spaces(width)))
        }
    }
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

    fn encode(&mut self, value: Unknown<'env>) -> Result<Scalar> {
        self.encode_at(value, 0)
    }

    fn encode_at(&mut self, value: Unknown<'env>, depth: usize) -> Result<Scalar> {
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
            ValueType::Undefined | ValueType::Null => Ok(Scalar::Null),
            ValueType::Boolean => value.coerce_to_bool().map(Scalar::from),
            ValueType::Number => Self::encode_number(value),
            ValueType::String => value
                .coerce_to_string()?
                .into_utf8()?
                .into_owned()
                .map(Scalar::from),
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
    fn encode_number(value: Unknown<'env>) -> Result<Scalar> {
        let value = value.coerce_to_number()?.get_double()?;
        if value == 0.0 && value.is_sign_negative() {
            return Ok(Scalar::from(value));
        }
        if value.is_finite()
            && value.fract() == 0.0
            && (-JS_SAFE_INTEGER_F64..=JS_SAFE_INTEGER_F64).contains(&value)
        {
            return Ok(Scalar::from(value as i64));
        }
        Ok(Scalar::from(value))
    }

    /// Encode a `bigint` as the narrowest native integer that holds it exactly.
    ///
    /// A `bigint` is an exact integer, so it belongs in an integer variant
    /// rather than in a wrapper naming its JavaScript type. Beyond 128 bits the
    /// core has no exact storage for one, and refusing there keeps a rounded or
    /// re-spelled number from being written as though it were the original.
    fn encode_bigint(value: Unknown<'env>) -> Result<Scalar> {
        let digits = value.coerce_to_string()?.into_utf8()?.into_owned()?;
        if let Ok(value) = digits.parse::<i64>() {
            return Ok(Scalar::from(value));
        }
        if let Ok(value) = digits.parse::<u64>() {
            return Ok(Scalar::from(value));
        }
        if let Ok(value) = digits.parse::<i128>() {
            return Ok(Scalar::from(value));
        }
        if let Ok(value) = digits.parse::<u128>() {
            return Ok(Scalar::from(value));
        }
        Err(napi_error(format!(
            "bigint {} exceeds the exact 128-bit integer range this codec stores",
            truncated(&digits)
        )))
    }

    fn encode_object(&mut self, value: Unknown<'env>, depth: usize) -> Result<Scalar> {
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

    fn encode_object_inner(&mut self, value: Unknown<'env>, depth: usize) -> Result<Scalar> {
        let object = value.coerce_to_object()?;
        if self.buffer_is_buffer.call(object)? {
            let bytes: Buffer = self.extract_object(&object)?;
            return Ok(Scalar::from(bytes.as_ref().to_vec()));
        }
        if value.is_arraybuffer()? {
            if !self.has_exact_prototype(&object, self.array_buffer_prototype)? {
                return Err(napi_error(
                    "ArrayBuffer subclasses are not serialized implicitly; convert to ArrayBuffer",
                ));
            }
            let bytes: ArrayBuffer<'_> = self.extract_object(&object)?;
            return Ok(Scalar::from(bytes.to_vec()));
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

    /// Encode a `Date` as the UTC millisecond instant it already is.
    fn encode_date(&self, object: &Object<'env>) -> Result<Scalar> {
        let millis = self.date_get_time.apply(object, ())?;
        if !millis.is_finite() {
            return Err(napi_error(
                "an invalid Date has no instant; encode a valid Date or null",
            ));
        }
        #[allow(clippy::cast_possible_truncation)]
        Scalar::datetime64(millis as i64, TimeUnit::Millisecond, Timezone::UTC).map_err(napi_error)
    }

    fn encode_branded_object(
        &mut self,
        value: Unknown<'env>,
        object: &Object<'env>,
        depth: usize,
    ) -> Result<Option<Scalar>> {
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
            return Ok(Some(Scalar::from(self.url_to_string.apply(object, ())?)));
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
            return Ok(Some(Scalar::from(format!("/{source}/{flags}"))));
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

        // Schema wrappers use the core structural Scalar conversion. Location
        // wrappers use their canonical cross-runtime text.
        native_wrapper!(JsScalar, "Scalar", 0, Clone::clone);
        native_wrapper!(JsDataType, "DataType", 1, Scalar::from);
        native_wrapper!(JsField, "Field", 2, Scalar::from);
        native_wrapper!(JsUri, "Uri", 3, |inner| Scalar::from(ToString::to_string(
            inner
        )));
        native_wrapper!(JsUrl, "Url", 4, |inner| Scalar::from(ToString::to_string(
            inner
        )));
        native_wrapper!(JsUrn, "Urn", 5, |inner| Scalar::from(ToString::to_string(
            inner
        )));
        Ok(None)
    }

    fn encode_typed_array(&mut self, value: Unknown<'env>, depth: usize) -> Result<Scalar> {
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
            return Ok(Scalar::from(bytes.as_ref().to_vec()));
        }
        if constructor_name == "Uint8ClampedArray" {
            let bytes: Uint8ClampedArray = self.extract_object(&object)?;
            return Ok(Scalar::from(bytes.as_ref().to_vec()));
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
        Ok(Scalar::from_sequence(values))
    }

    fn encode_array(&mut self, value: Unknown<'env>, depth: usize) -> Result<Scalar> {
        let object = value.coerce_to_object()?;
        let length = object.get::<u32>("length")?.unwrap_or(0);
        let mut values = Vec::with_capacity(length as usize);
        for index in 0..length {
            match object.get::<Unknown<'_>>(&index.to_string())? {
                Some(item) => values.push(self.encode_at(item, depth + 1)?),
                None => values.push(Scalar::Null),
            }
        }
        Ok(Scalar::from_sequence(values))
    }

    /// Encode a `Map` as a native mapping over its own arbitrary keys.
    ///
    /// The core mapping already takes any value as a key, so a Map needs no
    /// wrapper. Two distinct JavaScript keys can still encode to one native
    /// key, and that collision is refused rather than silently collapsed.
    fn encode_map(&mut self, object: &Object<'env>, depth: usize) -> Result<Scalar> {
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
        Scalar::from_mapping(values).map_err(napi_error)
    }

    fn encode_set(&mut self, object: &Object<'env>, depth: usize) -> Result<Scalar> {
        let entries = Self::iterator_values(object, self.set_values)?;
        let mut values = Vec::with_capacity(entries.len());
        for entry in entries {
            values.push(match entry {
                Some(entry) => self.encode_at(entry, depth + 1)?,
                None => Scalar::Null,
            });
        }
        Ok(Scalar::from_sequence(values))
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

    fn encode_properties(&mut self, object: &Object<'env>, depth: usize) -> Result<Scalar> {
        let keys = Object::keys(object)?;
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            let value = match object.get::<Unknown<'_>>(&key)? {
                Some(value) => self.encode_at(value, depth + 1)?,
                None => Scalar::Null,
            };
            entries.push((key, value));
        }
        Scalar::from_record(entries).map_err(napi_error)
    }

    fn encode_required_property(
        &mut self,
        object: &Object<'env>,
        name: &str,
        depth: usize,
        context: &str,
    ) -> Result<Scalar> {
        if !object.has_named_property(name)? {
            return Err(napi_error(format!("{context} property is missing")));
        }
        match object.get::<Unknown<'_>>(name)? {
            Some(value) => self.encode_at(value, depth),
            None => Ok(Scalar::Null),
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
        required_array_object(values, 0, "Scalar")?,
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

#[allow(clippy::too_many_lines)] // One exhaustive family match keeps the boundary auditable.
fn value_to_transport(value: &Scalar, depth: usize, max_depth: usize) -> Result<JsonValue> {
    if depth > max_depth {
        return Err(napi_error(format!(
            "decoded value exceeds maxDepth {max_depth}"
        )));
    }
    match value {
        Scalar::Null => Ok(JsonValue::Null),
        Scalar::Boolean(value) => Ok(JsonValue::Bool(value.get())),
        Scalar::Integer(value) => match value {
            Integer::I8(value) => integer_transport(i128::from(value.get())),
            Integer::I16(value) => integer_transport(i128::from(value.get())),
            Integer::I32(value) => integer_transport(i128::from(value.get())),
            Integer::I64(value) => integer_transport(i128::from(value.get())),
            Integer::U8(value) => unsigned_transport(u128::from(value.get())),
            Integer::U16(value) => unsigned_transport(u128::from(value.get())),
            Integer::U32(value) => unsigned_transport(u128::from(value.get())),
            Integer::U64(value) => unsigned_transport(u128::from(value.get())),
            Integer::I128(value) => integer_transport(value.get()),
            Integer::U128(value) => unsigned_transport(value.get()),
            _ => Err(napi_error("unsupported native integer representation")),
        },
        Scalar::Floating(_) => float_transport(
            value
                .as_float()
                .ok_or_else(|| napi_error("invalid native float"))?
                .as_f64(),
        ),
        Scalar::Text(value) => Ok(JsonValue::String(value.as_str().to_owned())),
        Scalar::Ascii(value) => Ok(JsonValue::String(value.as_str().to_owned())),
        Scalar::Guid(value) => Ok(JsonValue::String(value.to_string())),
        Scalar::Enum(value) => Ok(JsonValue::String(value.as_str().to_owned())),
        // A geometry has no JavaScript binding surface yet, so its WKB crosses
        // as its plain shape: the bytes transport that becomes a Buffer.
        Scalar::Bytes(value) => Ok(marker(
            "bytes",
            [("value", JsonValue::String(BASE64.encode(value.as_bytes())))],
        )),
        Scalar::Geospatial(value) => Ok(marker(
            "bytes",
            [("value", JsonValue::String(BASE64.encode(value.as_bytes())))],
        )),
        Scalar::Nested(Nested::Sequence(values)) => values
            .as_slice()
            .iter()
            .map(|value| value_to_transport(value, depth + 1, max_depth))
            .collect::<Result<Vec<_>>>()
            .map(JsonValue::Array),
        // A count is carried as text because a nanosecond instant needs more
        // than the 53 bits a JSON number keeps exactly; the JavaScript side
        // reads it as a bigint.
        Scalar::Decimal(decimal) => {
            let (unscaled, scale) = value
                .as_decimal()
                .ok_or_else(|| napi_error("invalid native decimal"))?;
            let id = match decimal {
                Decimal::D32(_) => "decimal32",
                Decimal::D64(_) => "decimal64",
                Decimal::D128(_) => "decimal128",
                Decimal::D256(_) => "decimal256",
                _ => return Err(napi_error("unsupported native decimal representation")),
            };
            Ok(marker(
                id,
                [
                    ("value", JsonValue::String(unscaled.to_string())),
                    ("scale", JsonValue::Number(JsonNumber::from(scale))),
                ],
            ))
        }
        Scalar::Temporal(Temporal::Interval(interval)) => Ok(JsonValue::Array(vec![
            integer_transport(i128::from(interval.months()))?,
            integer_transport(i128::from(interval.days()))?,
            integer_transport(i128::from(interval.nanoseconds()))?,
        ])),
        Scalar::Temporal(_) => {
            let temporal = value
                .as_temporal()
                .ok_or_else(|| napi_error("invalid native temporal"))?;
            let id = match (temporal.family(), temporal.bit_width()) {
                (TemporalFamily::Date, 32) => "date32",
                (TemporalFamily::Date, 64) => "date64",
                (TemporalFamily::Time, 32) => "time32",
                (TemporalFamily::Time, 64) => "time64",
                (TemporalFamily::DateTime, 64) => "datetime64",
                (TemporalFamily::Duration, 32) => "duration32",
                (TemporalFamily::Duration, 64) => "duration64",
                _ => return Err(napi_error("invalid native temporal family width")),
            };
            let date = (temporal.family() == TemporalFamily::DateTime
                && temporal.timezone().is_utc())
            .then(|| value.temporal_count_at(TimeUnit::Millisecond))
            .flatten()
            .filter(|millis| millis.unsigned_abs() <= MAX_DATE_MILLISECONDS);
            Ok(temporal_transport(
                id,
                temporal.count(),
                temporal.unit(),
                temporal.timezone(),
                date,
            ))
        }
        Scalar::Nested(Nested::Mapping(entries)) => {
            mapping_transport(entries.as_slice(), depth, max_depth)
        }
        Scalar::Nested(Nested::Record(entries)) => {
            record_transport(entries.as_map(), depth, max_depth)
        }
        _ => Err(napi_error("unsupported native Scalar representation")),
    }
}

/// Project a typed core value into JavaScript while restoring struct names.
///
/// Core rows stay ordered nested `Sequence` values. At the language boundary
/// a declared Struct becomes the object JavaScript uses for named records;
/// nested structs, lists, maps, unions, and dictionary values follow the same
/// field recursively.
fn struct_transport_with_field(
    value: &Scalar,
    fields: &CoreFields,
    depth: usize,
    max_depth: usize,
) -> Result<JsonValue> {
    let values = match value {
        Scalar::Nested(Nested::Sequence(values)) if values.as_slice().len() == fields.len() => {
            fields.iter().zip(values.as_slice()).collect::<Vec<_>>()
        }
        Scalar::Nested(Nested::Record(values)) => fields
            .iter()
            .map(|field| {
                values
                    .as_map()
                    .get(field.name())
                    .map(|value| (field, value))
                    .ok_or_else(|| {
                        napi_error(format!("typed record is missing field {:?}", field.name()))
                    })
            })
            .collect::<Result<Vec<_>>>()?,
        _ => {
            return Err(napi_error(format!(
                "expected {} typed struct values, got {}",
                fields.len(),
                value.kind()
            )));
        }
    };
    let entries = values
        .into_iter()
        .map(|(field, value)| {
            Ok((
                field.name(),
                value_to_transport_with_field(value, field, depth + 1, max_depth)?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    if entries
        .iter()
        .any(|(name, _)| matches!(*name, TRANSPORT_KEY | "__proto__"))
    {
        let pairs = entries
            .into_iter()
            .map(|(name, value)| JsonValue::Array(vec![JsonValue::String(name.to_owned()), value]))
            .collect();
        return Ok(marker("record", [("value", JsonValue::Array(pairs))]));
    }
    let mut object = JsonMap::with_capacity(entries.len());
    for (name, value) in entries {
        object.insert(name.to_owned(), value);
    }
    Ok(JsonValue::Object(object))
}

fn map_transport_with_field(
    value: &Scalar,
    map: &CoreMapType,
    depth: usize,
    max_depth: usize,
) -> Result<JsonValue> {
    let [key_field, value_field] = map.entries().fields() else {
        return Err(napi_error("typed map entries need key and value fields"));
    };
    let entries = value
        .as_mapping()
        .ok_or_else(|| napi_error(format!("expected a typed mapping, got {}", value.kind())))?;
    let pairs = entries
        .iter()
        .map(|(key, value)| {
            Ok(JsonValue::Array(vec![
                value_to_transport_with_field(key, key_field, depth + 1, max_depth)?,
                value_to_transport_with_field(value, value_field, depth + 1, max_depth)?,
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(marker("mapping", [("value", JsonValue::Array(pairs))]))
}

pub(crate) fn value_to_transport_with_field(
    value: &Scalar,
    field: &CoreField,
    depth: usize,
    max_depth: usize,
) -> Result<JsonValue> {
    if depth > max_depth {
        return Err(napi_error(format!(
            "decoded value exceeds maxDepth {max_depth}"
        )));
    }
    if value.is_null() {
        return value_to_transport(value, depth, max_depth);
    }
    match field.dtype() {
        CoreDataType::Struct(fields) => {
            struct_transport_with_field(value, fields, depth, max_depth)
        }
        CoreDataType::List(child)
        | CoreDataType::ListView(child)
        | CoreDataType::FixedSizeList(child, _)
        | CoreDataType::LargeList(child)
        | CoreDataType::LargeListView(child) => value
            .as_sequence()
            .ok_or_else(|| napi_error(format!("expected a typed list, got {}", value.kind())))?
            .iter()
            .map(|value| value_to_transport_with_field(value, child, depth + 1, max_depth))
            .collect::<Result<Vec<_>>>()
            .map(JsonValue::Array),
        CoreDataType::Map(map) => map_transport_with_field(value, map, depth, max_depth),
        CoreDataType::Union(fields, _) => {
            let Some([type_id, payload]) = value.as_sequence() else {
                return Err(napi_error(
                    "typed union value must contain its type id and payload",
                ));
            };
            let type_id = type_id
                .as_i128()
                .and_then(|value| i8::try_from(value).ok())
                .ok_or_else(|| napi_error("typed union id must fit i8"))?;
            let branch = fields
                .iter()
                .find_map(|(candidate, branch)| (candidate == type_id).then_some(branch))
                .ok_or_else(|| napi_error("typed union id is not declared"))?;
            value_to_transport_with_field(payload, branch, depth, max_depth)
        }
        CoreDataType::Dictionary(dictionary) => value_to_transport_with_field(
            value,
            &CoreField::new(
                field.name(),
                dictionary.value().clone(),
                field.is_nullable(),
            ),
            depth,
            max_depth,
        ),
        CoreDataType::RunEndEncoded(encoded) => {
            value_to_transport_with_field(value, encoded.values(), depth, max_depth)
        }
        _ => value_to_transport(value, depth, max_depth),
    }
}

/// Cross one decoded core value without lowering exact variants through JS.
pub(crate) fn decoded_value_for_field(
    value: Scalar,
    field: Option<&CoreField>,
    max_depth: usize,
    native_scalar: bool,
) -> Result<Either<JsScalar, JsonValue>> {
    if native_scalar {
        return Ok(Either::A(JsScalar::from_core(value)));
    }
    value_to_transport_for_field(&value, field, max_depth).map(Either::B)
}

pub(crate) fn value_to_transport_for_field(
    value: &Scalar,
    field: Option<&CoreField>,
    max_depth: usize,
) -> Result<JsonValue> {
    field.map_or_else(
        || value_to_transport(value, 0, max_depth),
        |field| value_to_transport_with_field(value, field, 0, max_depth),
    )
}

fn temporal_transport(
    kind: &str,
    count: i64,
    unit: TimeUnit,
    zone: Timezone,
    date: Option<i64>,
) -> JsonValue {
    marker(
        kind,
        [
            ("value", JsonValue::String(count.to_string())),
            ("unit", JsonValue::String(unit.as_str().to_owned())),
            ("zone", JsonValue::String(zone.as_str().to_owned())),
            (
                "date",
                date.map_or(JsonValue::Null, |millis| {
                    JsonValue::Number(JsonNumber::from(millis))
                }),
            ),
        ],
    )
}

fn record_transport<K: AsRef<str> + Ord>(
    entries: &std::collections::BTreeMap<K, Scalar>,
    depth: usize,
    max_depth: usize,
) -> Result<JsonValue> {
    let reserved = entries
        .keys()
        .any(|key| matches!(key.as_ref(), TRANSPORT_KEY | "__proto__"));
    if reserved {
        let pairs = entries
            .iter()
            .map(|(key, value)| {
                Ok(JsonValue::Array(vec![
                    JsonValue::String(key.as_ref().to_owned()),
                    value_to_transport(value, depth + 1, max_depth)?,
                ]))
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(marker("record", [("value", JsonValue::Array(pairs))]));
    }
    let mut object = JsonMap::with_capacity(entries.len());
    for (key, value) in entries {
        object.insert(
            key.as_ref().to_owned(),
            value_to_transport(value, depth + 1, max_depth)?,
        );
    }
    Ok(JsonValue::Object(object))
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
    entries: &[(Scalar, Scalar)],
    depth: usize,
    max_depth: usize,
) -> Result<JsonValue> {
    let pairs = entries
        .iter()
        .map(|(key, value)| {
            Ok(JsonValue::Array(vec![
                value_to_transport(key, depth + 1, max_depth)?,
                value_to_transport(value, depth + 1, max_depth)?,
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(marker("mapping", [("value", JsonValue::Array(pairs))]))
}

fn marker<const N: usize>(kind: &str, fields: [(&str, JsonValue); N]) -> JsonValue {
    let mut object = JsonMap::with_capacity(N + 1);
    object.insert(TRANSPORT_KEY.to_owned(), JsonValue::String(kind.to_owned()));
    for (key, value) in fields {
        object.insert(key.to_owned(), value);
    }
    JsonValue::Object(object)
}
