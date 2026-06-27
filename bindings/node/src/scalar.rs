//! The `Scalar` napi class — a single atomic value carrying its full data type, with
//! lossless Arrow round-tripping. A thin wrapper over [`yggdryl_scalar`]'s `Scalar`; all
//! logic lives in the core, so the Node and Python bindings behave identically.

use std::collections::HashMap;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde_json::Value as JsonValue;
use yggdryl_scalar::{from_bytes, i256, Interval, ScalarValue as CoreScalar};
use yggdryl_schema::DataType as CoreDataType;

use crate::datatype::DataType;
use crate::{err, to_mapping};

/// A single, type-erased value that knows its own `DataType` and round-trips losslessly
/// to and from Apache Arrow. Build one from a JS value (`new Scalar(42)`,
/// `new Scalar(5, "int32")`), a canonical string (`Scalar.fromStr`), bytes
/// (`Scalar.fromBytes`) or a component map (`Scalar.fromMapping`); read its native
/// `value`, its `dataType`, and serialise it through `toStr` / `toBytes`.
#[napi]
pub struct Scalar {
    pub(crate) inner: CoreScalar,
}

fn wrap(inner: CoreScalar) -> Scalar {
    Scalar { inner }
}

/// Resolves a `DataType` class **or** a type string to a core [`CoreDataType`].
fn resolve_dtype(dtype: Either<&DataType, String>) -> Result<CoreDataType> {
    match dtype {
        Either::A(dt) => Ok(dt.inner.clone()),
        Either::B(text) => CoreDataType::from_str(&text).map_err(err),
    }
}

/// Builds a scalar of an explicit `dtype` from a JS value (`null` → a typed null). The
/// primitive / string / JSON families build directly; bytes use `Scalar.binary`, and the
/// richer types (decimal, temporal, nested) go through `fromStr` / `fromBytes`.
fn build_typed(value: &JsonValue, dtype: &CoreDataType) -> Result<CoreScalar> {
    use CoreDataType as D;
    if value.is_null() {
        return Ok(CoreScalar::null(dtype.clone()));
    }
    Ok(match dtype {
        D::Boolean => {
            CoreScalar::boolean(value.as_bool().ok_or_else(|| err("expected a boolean"))?)
        }
        D::Int { bits, signed } => {
            let n = value.as_i64().ok_or_else(|| err("expected an integer"))?;
            CoreScalar::int(n as i128, *bits, *signed)
        }
        D::Float { bits } => CoreScalar::float(
            value.as_f64().ok_or_else(|| err("expected a number"))?,
            *bits,
        ),
        D::Varchar {
            charset,
            large,
            view,
            size,
        } => CoreScalar::Utf8 {
            value: value
                .as_str()
                .ok_or_else(|| err("expected a string"))?
                .to_string(),
            charset: *charset,
            large: *large,
            view: *view,
            size: *size,
        },
        D::Json => CoreScalar::json(
            value
                .as_str()
                .ok_or_else(|| err("expected a string"))?
                .to_string(),
        ),
        other => {
            return Err(err(format!(
                "construct a '{}' scalar via Scalar.fromStr / fromBytes / fromMapping \
                 (or Scalar.binary for bytes)",
                other.to_str()
            )))
        }
    })
}

/// Infers a scalar from a JS value: `boolean` → bool, `number` → int64 if integral else
/// float64, `string` → utf8.
fn infer(value: &JsonValue) -> Result<CoreScalar> {
    Ok(match value {
        JsonValue::Bool(b) => CoreScalar::boolean(*b),
        JsonValue::Number(n) => match n.as_i64() {
            Some(i) => CoreScalar::int(i as i128, 64, true),
            None => CoreScalar::float(n.as_f64().unwrap_or(0.0), 64),
        },
        JsonValue::String(s) => CoreScalar::utf8(s.clone()),
        JsonValue::Null => return Err(err("cannot infer a scalar from null; pass a dtype")),
        _ => return Err(err("unsupported scalar value; pass an explicit dtype")),
    })
}

/// Renders an unscaled decimal value as its scaled decimal string.
fn decimal_string(value: i256, scale: i8) -> String {
    let digits = value.to_string();
    if scale <= 0 {
        return format!("{digits}{}", "0".repeat((-(scale as i32)) as usize));
    }
    let scale = scale as usize;
    let (sign, digits) = match digits.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", digits.as_str()),
    };
    if digits.len() > scale {
        let point = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..point], &digits[point..])
    } else {
        format!("{sign}0.{}{}", "0".repeat(scale - digits.len()), digits)
    }
}

/// Maps an [`Interval`] to a JSON object of its calendar components.
fn interval_to_json(interval: &Interval) -> JsonValue {
    let mut obj = serde_json::Map::new();
    match interval {
        Interval::YearMonth(months) => {
            obj.insert("months".into(), JsonValue::from(*months));
        }
        Interval::DayTime { days, millis } => {
            obj.insert("days".into(), JsonValue::from(*days));
            obj.insert("millis".into(), JsonValue::from(*millis));
        }
        Interval::MonthDayNano {
            months,
            days,
            nanos,
        } => {
            obj.insert("months".into(), JsonValue::from(*months));
            obj.insert("days".into(), JsonValue::from(*days));
            obj.insert("nanos".into(), JsonValue::from(*nanos));
        }
    }
    JsonValue::Object(obj)
}

/// Maps a core [`CoreScalar`] to a JSON value (which napi converts to the JS value):
/// primitives become native values, temporal scalars an ISO string, decimals / intervals
/// a string / object, and the nested types an array / object (recursively).
fn value_to_json(scalar: &CoreScalar) -> JsonValue {
    use CoreScalar as S;
    match scalar {
        S::Null(_) => JsonValue::Null,
        S::Boolean(b) => JsonValue::Bool(*b),
        S::Int { value, .. } => i64::try_from(*value)
            .map(JsonValue::from)
            .unwrap_or_else(|_| JsonValue::String(value.to_string())),
        S::Float { value, .. } => serde_json::Number::from_f64(value.0)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        S::Utf8 { value, .. } => JsonValue::String(value.clone()),
        S::Json(v) => JsonValue::String(v.clone()),
        S::Binary { value, .. } => {
            JsonValue::Array(value.iter().map(|x| JsonValue::from(*x)).collect())
        }
        S::Bson(v) => JsonValue::Array(v.iter().map(|x| JsonValue::from(*x)).collect()),
        S::Timezone(tz) => JsonValue::String(tz.name()),
        S::Decimal { value, scale, .. } => JsonValue::String(decimal_string(*value, *scale)),
        S::Date { .. } => {
            JsonValue::String(scalar.as_date().map(|d| d.to_string()).unwrap_or_default())
        }
        S::Time { .. } => scalar
            .as_time()
            .map(|t| JsonValue::String(t.to_string()))
            .unwrap_or(JsonValue::Null),
        S::Timestamp { .. } => JsonValue::String(
            scalar
                .as_datetime()
                .map(|d| d.to_string())
                .unwrap_or_default(),
        ),
        S::Duration { .. } => JsonValue::String(
            scalar
                .as_duration()
                .map(|d| d.to_string())
                .unwrap_or_default(),
        ),
        S::Interval(interval) => interval_to_json(interval),
        S::List { values, .. } => JsonValue::Array(values.iter().map(value_to_json).collect()),
        S::Struct { fields, values } => {
            let mut obj = serde_json::Map::new();
            for (field, value) in fields.iter().zip(values) {
                obj.insert(field.name().to_string(), value_to_json(value));
            }
            JsonValue::Object(obj)
        }
        S::Map { entries, .. } => JsonValue::Array(
            entries
                .iter()
                .map(|(k, v)| JsonValue::Array(vec![value_to_json(k), value_to_json(v)]))
                .collect(),
        ),
    }
}

/// Lowercase-hex encoding of the IPC bytes used by `toJSON` / `fromJSON`.
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Decodes the lowercase-hex string produced by `toJSON`.
fn from_hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return Err(err("invalid scalar hex: odd length"));
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|_| err("invalid scalar hex")))
        .collect()
}

#[napi]
impl Scalar {
    /// Build a scalar from a JS value. Without `dtype` the type is inferred (`boolean` →
    /// bool, `number` → int64 if integral else float64, `string` → utf8); pass `dtype`
    /// (a `DataType` or type string) to build a specific type, and pass `null` with a
    /// `dtype` for a typed null. Use `Scalar.binary` for bytes.
    #[napi(constructor)]
    pub fn new(value: JsonValue, dtype: Option<Either<&DataType, String>>) -> Result<Self> {
        let inner = match dtype {
            Some(dtype) => build_typed(&value, &resolve_dtype(dtype)?)?,
            None => infer(&value)?,
        };
        Ok(wrap(inner))
    }

    /// A typed null of `dtype` (a `DataType` or type string).
    #[napi(factory)]
    pub fn null(dtype: Either<&DataType, String>) -> Result<Self> {
        Ok(wrap(CoreScalar::null(resolve_dtype(dtype)?)))
    }

    /// A binary scalar from a `Buffer`.
    #[napi(factory)]
    pub fn binary(value: Buffer) -> Self {
        wrap(CoreScalar::binary(value.to_vec()))
    }

    /// Parse a scalar from its canonical string (`"42::int64"`, `"'hi'::utf8"`).
    #[napi(factory, js_name = "fromStr")]
    pub fn from_str_js(value: String) -> Result<Self> {
        CoreScalar::from_str(&value).map(wrap).map_err(err)
    }

    /// Reconstruct a scalar from its Arrow-IPC `toBytes` form.
    #[napi(factory, js_name = "fromBytes")]
    pub fn from_bytes_js(data: Buffer) -> Result<Self> {
        from_bytes(&data).map(wrap).map_err(err)
    }

    /// Build a scalar from a `{ type, value }` component map.
    #[napi(factory, js_name = "fromMapping")]
    pub fn from_mapping(mapping: HashMap<String, String>) -> Result<Self> {
        CoreScalar::from_mapping(&to_mapping(mapping))
            .map(wrap)
            .map_err(err)
    }

    /// The scalar's `DataType`.
    #[napi(getter, js_name = "dataType")]
    pub fn data_type(&self) -> DataType {
        DataType {
            inner: self.inner.data_type(),
        }
    }

    /// Whether this is a null value.
    #[napi(getter, js_name = "isNull")]
    pub fn is_null(&self) -> bool {
        self.inner.is_null()
    }

    /// The native JS value (`null` for a null; an ISO string for temporals; an array /
    /// object for the nested types).
    #[napi(getter)]
    pub fn value(&self) -> JsonValue {
        value_to_json(&self.inner)
    }

    /// The value as a boolean, or `null`.
    #[napi(js_name = "asBool")]
    pub fn as_bool(&self) -> Option<bool> {
        self.inner.as_bool()
    }

    /// The value as an integer (`null` if not an integer or out of the `i64` range).
    #[napi(js_name = "asInt")]
    pub fn as_int(&self) -> Option<i64> {
        self.inner.as_i128().and_then(|v| i64::try_from(v).ok())
    }

    /// The value as a number, or `null`.
    #[napi(js_name = "asFloat")]
    pub fn as_float(&self) -> Option<f64> {
        self.inner.as_f64()
    }

    /// The value as a string, or `null`.
    #[napi(js_name = "asStr")]
    pub fn as_str(&self) -> Option<String> {
        self.inner.as_str().map(|s| s.to_string())
    }

    /// The value as a `Buffer`, or `null`.
    #[napi(js_name = "asBytes")]
    pub fn as_bytes(&self) -> Option<Buffer> {
        self.inner.as_bytes().map(|b| Buffer::from(b.to_vec()))
    }

    /// The canonical string (`"42::int64"`).
    #[napi(js_name = "toStr")]
    pub fn to_str(&self) -> String {
        self.inner.to_str()
    }

    /// The `{ type, value }` component map.
    #[napi(js_name = "toMapping")]
    pub fn to_mapping_js(&self) -> HashMap<String, String> {
        self.inner.to_mapping().into_iter().collect()
    }

    /// Serialise to lossless Arrow-IPC bytes (round-trips via `fromBytes`).
    #[napi(js_name = "toBytes")]
    pub fn to_bytes(&self) -> Result<Buffer> {
        self.inner.to_bytes().map(Buffer::from).map_err(err)
    }

    /// Value equality (same type and value).
    #[napi]
    pub fn equals(&self, other: &Scalar) -> bool {
        self.inner == other.inner
    }

    #[napi(js_name = "toString")]
    pub fn to_string_js(&self) -> String {
        self.inner.to_str()
    }

    /// Serialise to a lossless string (hex of the Arrow-IPC bytes) for `JSON.stringify`.
    #[napi(js_name = "toJSON")]
    pub fn to_json(&self) -> Result<String> {
        self.inner.to_bytes().map(|b| to_hex(&b)).map_err(err)
    }

    /// Reconstruct from the string produced by `toJSON`.
    #[napi(factory, js_name = "fromJSON")]
    pub fn from_json(value: String) -> Result<Self> {
        from_bytes(&from_hex(&value)?).map(wrap).map_err(err)
    }
}
