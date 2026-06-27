//! The [`Scalar`] enum — one atomic value carrying its full [`DataType`] — plus the
//! [`Interval`] payload and the [`F64`] wrapper that lets the whole type derive
//! [`Hash`] / [`Eq`] (floats hash by a canonical bit pattern). Arrow conversion lives in
//! [`arrow`](crate::arrow); byte serialization in [`bytes`](crate::bytes).

use std::fmt;
use std::hash::{Hash, Hasher};

use arrow_buffer::i256;
use yggdryl_core::{Charset, Date, DateTime, Duration, Time, TimeUnit, Timezone};
use yggdryl_schema::{DataType, Field, IntervalUnit};

use crate::error::{ScalarError, ScalarResult};
#[allow(unused_imports)]
use crate::log_event;

/// An `f64` that is [`Eq`] + [`Hash`]: it compares and hashes by a **canonical bit
/// pattern** (every `NaN` is equal to every other `NaN`, and `-0.0` equals `0.0`), so a
/// [`Float`](Scalar::Float) value can key a map or set. It [`Deref`](std::ops::Deref)s to
/// `f64`, so it reads like one.
///
/// ```
/// use yggdryl_scalar::F64;
/// assert_eq!(F64(f64::NAN), F64(f64::NAN));
/// assert_eq!(F64(-0.0), F64(0.0));
/// assert_eq!(*F64(1.5) + 0.5, 2.0);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct F64(pub f64);

impl F64 {
    /// The canonical bit pattern used for equality and hashing: a single `NaN` pattern,
    /// and `+0.0` for both signed zeros.
    fn canonical_bits(self) -> u64 {
        if self.0.is_nan() {
            f64::NAN.to_bits()
        } else if self.0 == 0.0 {
            0.0f64.to_bits()
        } else {
            self.0.to_bits()
        }
    }
}

impl PartialEq for F64 {
    fn eq(&self, other: &F64) -> bool {
        self.canonical_bits() == other.canonical_bits()
    }
}

impl Eq for F64 {}

impl Hash for F64 {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_bits().hash(state);
    }
}

impl From<f64> for F64 {
    fn from(value: f64) -> F64 {
        F64(value)
    }
}

impl std::ops::Deref for F64 {
    type Target = f64;
    fn deref(&self) -> &f64 {
        &self.0
    }
}

impl fmt::Display for F64 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Rust's default `f64` formatting is the shortest round-tripping form, so
        // `from_str` recovers the exact value.
        write!(f, "{}", self.0)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for F64 {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_f64(self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for F64 {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<F64, D::Error> {
        f64::deserialize(d).map(F64)
    }
}

/// A calendar [`Interval`](DataType::Interval) value, in one of the three Arrow
/// resolutions. The fields are the raw calendar components, never normalised between
/// resolutions (months, days and sub-day time are independent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Interval {
    /// A whole number of months.
    YearMonth(i32),
    /// Days plus milliseconds.
    DayTime {
        /// Whole days.
        days: i32,
        /// Milliseconds within the day part.
        millis: i32,
    },
    /// Months, days and nanoseconds.
    MonthDayNano {
        /// Whole months.
        months: i32,
        /// Whole days.
        days: i32,
        /// Nanoseconds within the day part.
        nanos: i64,
    },
}

impl Interval {
    /// The [`IntervalUnit`] of this interval.
    pub fn unit(&self) -> IntervalUnit {
        match self {
            Interval::YearMonth(_) => IntervalUnit::YearMonth,
            Interval::DayTime { .. } => IntervalUnit::DayTime,
            Interval::MonthDayNano { .. } => IntervalUnit::MonthDayNano,
        }
    }
}

/// A single atomic value carrying its full [`DataType`]. Every variant pins the type
/// parameters its [`DataType`] needs, so [`data_type`](Scalar::data_type) is exact and
/// the value round-trips losslessly through a [string](Scalar::to_str),
/// [bytes](Scalar::to_bytes) and an [Arrow array](Scalar::to_array).
///
/// Construct with the typed builders ([`int`](Scalar::int), [`utf8`](Scalar::utf8),
/// [`from_datetime`](Scalar::from_datetime), …) or the [`From`] impls; read with the
/// `as_*` accessors or by matching the (public) variants directly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Scalar {
    /// A typed null — a null still knows the [`DataType`] of the column it came from.
    Null(DataType),
    /// A boolean.
    Boolean(bool),
    /// An integer of `bits` width, signed or not (value widened to `i128`).
    Int {
        /// The value, widened to `i128`.
        #[cfg_attr(feature = "serde", serde(with = "serdex::str_i128"))]
        value: i128,
        /// Bit width (8/16/32/64 convert to Arrow).
        bits: u16,
        /// Whether the integer is signed.
        signed: bool,
    },
    /// A floating-point value of `bits` width (value widened to `f64`).
    Float {
        /// The value, widened to `f64`.
        value: F64,
        /// Bit width (16/32/64 convert to Arrow).
        bits: u16,
    },
    /// A decimal with `(precision, scale)` stored in `bits`. The unscaled value is held
    /// as an `i256` (which losslessly covers the 32/64/128/256-bit widths).
    Decimal {
        /// The unscaled value.
        #[cfg_attr(feature = "serde", serde(with = "serdex::str_i256"))]
        value: i256,
        /// Total number of digits.
        precision: u8,
        /// Digits after the decimal point.
        scale: i8,
        /// Storage width: 32, 64, 128 or 256.
        bits: u16,
    },
    /// A string value, with its physical flavour (charset / large / view / fixed size).
    Utf8 {
        /// The text.
        value: String,
        /// The character set.
        charset: Charset,
        /// 64-bit offsets.
        large: bool,
        /// View layout.
        view: bool,
        /// Fixed length, if any.
        size: Option<i32>,
    },
    /// Opaque bytes, with their physical flavour.
    Binary {
        /// The bytes.
        value: Vec<u8>,
        /// 64-bit offsets.
        large: bool,
        /// View layout.
        view: bool,
        /// Fixed width, if any.
        size: Option<i32>,
    },
    /// JSON text (a string-backed logical value).
    Json(String),
    /// A BSON document (a binary-backed logical value).
    Bson(Vec<u8>),
    /// A calendar date, as the physical count (days for `large` = false, milliseconds
    /// for `large` = true).
    Date {
        /// The physical value.
        value: i64,
        /// Millisecond (64-bit) storage instead of day (32-bit).
        large: bool,
    },
    /// A time of day, as a physical count of `unit` since midnight.
    Time {
        /// The physical value.
        value: i64,
        /// Resolution.
        unit: TimeUnit,
    },
    /// A timestamp, as a physical count of `unit` since the epoch, with an optional
    /// display [`Timezone`].
    Timestamp {
        /// The physical value.
        value: i64,
        /// Resolution.
        unit: TimeUnit,
        /// Display timezone, if zoned.
        timezone: Option<Timezone>,
    },
    /// Elapsed time, as a physical count of `unit`.
    Duration {
        /// The physical value.
        value: i64,
        /// Resolution.
        unit: TimeUnit,
    },
    /// A calendar interval.
    Interval(Interval),
    /// A list value: the element [`Field`] and the element values.
    List {
        /// The element values, in order.
        values: Vec<Scalar>,
        /// The element field (its type, name and nullability).
        field: Box<Field>,
        /// 64-bit offsets.
        large: bool,
        /// View layout (not yet supported by Arrow conversion).
        view: bool,
        /// Fixed length, if any.
        size: Option<i32>,
    },
    /// A struct (record) value: the fields and one value per field.
    Struct {
        /// The struct fields, in order.
        fields: Vec<Field>,
        /// One value per field, in field order.
        values: Vec<Scalar>,
    },
    /// A map value: the key/value types and the entries.
    Map {
        /// The key type.
        key: Box<DataType>,
        /// The value type.
        value: Box<DataType>,
        /// Whether the keys are sorted.
        sorted: bool,
        /// The `(key, value)` entries, in order.
        entries: Vec<(Scalar, Scalar)>,
    },
}

impl Scalar {
    // ---- constructors ----

    /// A typed null of `dtype`.
    pub fn null(dtype: DataType) -> Scalar {
        Scalar::Null(dtype)
    }

    /// A boolean value.
    pub fn boolean(value: bool) -> Scalar {
        Scalar::Boolean(value)
    }

    /// An integer of `bits` width and the given signedness.
    pub fn int(value: i128, bits: u16, signed: bool) -> Scalar {
        Scalar::Int {
            value,
            bits,
            signed,
        }
    }

    /// A floating-point value of `bits` width.
    pub fn float(value: f64, bits: u16) -> Scalar {
        Scalar::Float {
            value: F64(value),
            bits,
        }
    }

    /// A decimal with `(precision, scale)` stored in `bits` (32/64/128/256).
    pub fn decimal(value: i256, precision: u8, scale: i8, bits: u16) -> Scalar {
        Scalar::Decimal {
            value,
            precision,
            scale,
            bits,
        }
    }

    /// A 128-bit decimal with `(precision, scale)` — the common case.
    pub fn decimal128(value: i128, precision: u8, scale: i8) -> Scalar {
        Scalar::decimal(i256::from_i128(value), precision, scale, 128)
    }

    /// A variable-length UTF-8 string.
    pub fn utf8(value: impl Into<String>) -> Scalar {
        Scalar::Utf8 {
            value: value.into(),
            charset: Charset::Utf8,
            large: false,
            view: false,
            size: None,
        }
    }

    /// Variable-length opaque bytes.
    pub fn binary(value: impl Into<Vec<u8>>) -> Scalar {
        Scalar::Binary {
            value: value.into(),
            large: false,
            view: false,
            size: None,
        }
    }

    /// JSON text.
    pub fn json(value: impl Into<String>) -> Scalar {
        Scalar::Json(value.into())
    }

    /// A BSON document.
    pub fn bson(value: impl Into<Vec<u8>>) -> Scalar {
        Scalar::Bson(value.into())
    }

    /// A day-resolution date from a count of days since the epoch.
    pub fn date(days: i32) -> Scalar {
        Scalar::Date {
            value: days as i64,
            large: false,
        }
    }

    /// A timestamp from a physical count of `unit`, with an optional [`Timezone`].
    pub fn timestamp(value: i64, unit: TimeUnit, timezone: Option<Timezone>) -> Scalar {
        Scalar::Timestamp {
            value,
            unit,
            timezone,
        }
    }

    /// A nanosecond, timezone-naive timestamp from a core [`DateTime`] (clamped to the
    /// `i64` nanosecond range), preserving the instant's display [`Timezone`].
    pub fn from_datetime(value: &DateTime) -> Scalar {
        let nanos = value
            .epoch_nanos()
            .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        Scalar::timestamp(nanos, TimeUnit::Nanosecond, value.timezone().cloned())
    }

    /// A day-resolution date from a core [`Date`].
    pub fn from_date(value: &Date) -> Scalar {
        Scalar::date(value.epoch_days())
    }

    /// A nanosecond time-of-day from a core [`Time`].
    pub fn from_time(value: &Time) -> Scalar {
        Scalar::Time {
            value: value.nanos_of_day() as i64,
            unit: TimeUnit::Nanosecond,
        }
    }

    /// A nanosecond duration from a core [`Duration`] (clamped to the `i64` range).
    pub fn from_duration(value: &Duration) -> Scalar {
        let nanos = value.as_nanos().clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        Scalar::Duration {
            value: nanos,
            unit: TimeUnit::Nanosecond,
        }
    }

    /// A `year_month` interval of `months`.
    pub fn interval_year_month(months: i32) -> Scalar {
        Scalar::Interval(Interval::YearMonth(months))
    }

    // ---- type / null checks ----

    /// The exact [`DataType`] this value belongs to.
    pub fn data_type(&self) -> DataType {
        use Scalar::*;
        match self {
            Null(dt) => dt.clone(),
            Boolean(_) => DataType::Boolean,
            Int { bits, signed, .. } => DataType::Int {
                bits: *bits,
                signed: *signed,
            },
            Float { bits, .. } => DataType::Float { bits: *bits },
            Decimal {
                precision,
                scale,
                bits,
                ..
            } => DataType::Decimal {
                precision: *precision,
                scale: *scale,
                bits: *bits,
            },
            Utf8 {
                charset,
                large,
                view,
                size,
                ..
            } => DataType::Varchar {
                charset: *charset,
                large: *large,
                view: *view,
                size: *size,
            },
            Binary {
                large, view, size, ..
            } => DataType::Binary {
                large: *large,
                view: *view,
                size: *size,
            },
            Json(_) => DataType::Json,
            Bson(_) => DataType::Bson,
            Date { large, .. } => DataType::Date { large: *large },
            Time { unit, .. } => DataType::Time { unit: *unit },
            Timestamp { unit, timezone, .. } => DataType::Timestamp {
                unit: *unit,
                timezone: timezone.clone(),
            },
            Duration { unit, .. } => DataType::Duration { unit: *unit },
            Interval(iv) => DataType::Interval { unit: iv.unit() },
            List {
                field,
                large,
                view,
                size,
                ..
            } => DataType::List {
                item: Box::new((**field).clone()),
                large: *large,
                view: *view,
                size: *size,
            },
            Struct { fields, .. } => DataType::Struct(fields.clone()),
            Map {
                key, value, sorted, ..
            } => DataType::Map {
                key: key.clone(),
                value: value.clone(),
                sorted: *sorted,
            },
        }
    }

    /// Whether this is a [`Null`](Scalar::Null) value.
    pub fn is_null(&self) -> bool {
        matches!(self, Scalar::Null(_))
    }

    // ---- value accessors ----

    /// The value as a `bool`, when it is a [`Boolean`](Scalar::Boolean).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Scalar::Boolean(v) => Some(*v),
            _ => None,
        }
    }

    /// The value as an `i128`, when it is an [`Int`](Scalar::Int).
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            Scalar::Int { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// The value as an `f64`, when it is a [`Float`](Scalar::Float).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Scalar::Float { value, .. } => Some(value.0),
            _ => None,
        }
    }

    /// The unscaled value as an `i256`, when it is a [`Decimal`](Scalar::Decimal).
    pub fn as_decimal(&self) -> Option<i256> {
        match self {
            Scalar::Decimal { value, .. } => Some(*value),
            _ => None,
        }
    }

    /// The value as a `&str`, when it is a [`Utf8`](Scalar::Utf8) or [`Json`](Scalar::Json).
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Scalar::Utf8 { value, .. } => Some(value),
            Scalar::Json(value) => Some(value),
            _ => None,
        }
    }

    /// The value as bytes, when it is a [`Binary`](Scalar::Binary) or [`Bson`](Scalar::Bson).
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Scalar::Binary { value, .. } => Some(value),
            Scalar::Bson(value) => Some(value),
            _ => None,
        }
    }

    /// The value as a core [`DateTime`], when it is a [`Timestamp`](Scalar::Timestamp).
    pub fn as_datetime(&self) -> Option<DateTime> {
        match self {
            Scalar::Timestamp {
                value,
                unit,
                timezone,
            } => {
                let nanos = (*value as i128) * (unit.nanos() as i128);
                Some(DateTime::from_epoch_nanos(nanos, timezone.clone()))
            }
            _ => None,
        }
    }

    /// The value as a core [`Date`], when it is a [`Date`](Scalar::Date).
    pub fn as_date(&self) -> Option<Date> {
        match self {
            Scalar::Date { value, large } => {
                let days = if *large {
                    // Date64 is milliseconds; reduce to whole days.
                    (*value).div_euclid(86_400_000) as i32
                } else {
                    *value as i32
                };
                Some(Date::from_epoch_days(days))
            }
            _ => None,
        }
    }

    /// The value as a core [`Time`], when it is a [`Time`](Scalar::Time) (and the
    /// physical value is a valid time of day).
    pub fn as_time(&self) -> Option<Time> {
        match self {
            Scalar::Time { value, unit } => {
                let nanos = (*value as i128) * (unit.nanos() as i128);
                Time::from_nanos_of_day(u64::try_from(nanos).ok()?).ok()
            }
            _ => None,
        }
    }

    /// The value as a core [`Duration`], when it is a [`Duration`](Scalar::Duration).
    pub fn as_duration(&self) -> Option<Duration> {
        match self {
            Scalar::Duration { value, unit } => Some(Duration::from_nanos(
                (*value as i128) * (unit.nanos() as i128),
            )),
            _ => None,
        }
    }
}

// ---- ergonomic `From` constructors ----

impl From<bool> for Scalar {
    fn from(value: bool) -> Scalar {
        Scalar::Boolean(value)
    }
}

impl From<i64> for Scalar {
    fn from(value: i64) -> Scalar {
        Scalar::int(value as i128, 64, true)
    }
}

impl From<i32> for Scalar {
    fn from(value: i32) -> Scalar {
        Scalar::int(value as i128, 32, true)
    }
}

impl From<f64> for Scalar {
    fn from(value: f64) -> Scalar {
        Scalar::float(value, 64)
    }
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Scalar {
        Scalar::utf8(value)
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Scalar {
        Scalar::utf8(value)
    }
}

impl From<Vec<u8>> for Scalar {
    fn from(value: Vec<u8>) -> Scalar {
        Scalar::binary(value)
    }
}

impl From<&DateTime> for Scalar {
    fn from(value: &DateTime) -> Scalar {
        Scalar::from_datetime(value)
    }
}

// ---- canonical string / mapping ----

impl Scalar {
    /// Renders the canonical, round-tripping string `"<payload>::<datatype>"` — e.g.
    /// `42::int64`, `'hi'::utf8`, `0x00ff::binary`, `null::int64`. The temporal types
    /// render their **physical** value (a count of the type's unit). Nested values
    /// (list / struct / map) render readably but are **not** parsed back by
    /// [`from_str`](Scalar::from_str) — use [`from_bytes`](crate::from_bytes) or
    /// [`from_json`](Scalar::from_json) for those. The inverse of
    /// [`from_str`](Scalar::from_str) for the atomic types.
    pub fn to_str(&self) -> String {
        format!("{}::{}", self.payload_str(), self.data_type().to_str())
    }

    /// The value half of [`to_str`](Scalar::to_str) (no `::<datatype>` suffix).
    fn payload_str(&self) -> String {
        use Scalar::*;
        match self {
            Null(_) => "null".to_string(),
            Boolean(v) => v.to_string(),
            Int { value, .. } => value.to_string(),
            Float { value, .. } => value.to_string(),
            Decimal { value, .. } => value.to_string(),
            Utf8 { value, .. } => quote_str(value),
            Json(v) => quote_str(v),
            Binary { value, .. } => hex_str(value),
            Bson(v) => hex_str(v),
            Date { value, .. }
            | Time { value, .. }
            | Timestamp { value, .. }
            | Duration { value, .. } => value.to_string(),
            Interval(iv) => match iv {
                self::Interval::YearMonth(m) => format!("ym:{m}"),
                self::Interval::DayTime { days, millis } => format!("dt:{days},{millis}"),
                self::Interval::MonthDayNano {
                    months,
                    days,
                    nanos,
                } => format!("mdn:{months},{days},{nanos}"),
            },
            List { values, .. } => {
                let inner = values
                    .iter()
                    .map(Scalar::payload_str)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("[{inner}]")
            }
            Struct { fields, values } => {
                let inner = fields
                    .iter()
                    .zip(values)
                    .map(|(f, v)| format!("{}={}", f.name(), v.payload_str()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{inner}}}")
            }
            Map { entries, .. } => {
                let inner = entries
                    .iter()
                    .map(|(k, v)| format!("{}={}", k.payload_str(), v.payload_str()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{{{inner}}}")
            }
        }
    }

    /// Parses the canonical string of [`to_str`](Scalar::to_str). Atomic types
    /// (boolean / integer / float / decimal / string / binary / temporal / interval and
    /// a typed `null`) round-trip; a nested type returns
    /// [`ScalarError::Unsupported`] (use [`from_bytes`](crate::from_bytes) /
    /// [`from_json`](Scalar::from_json)).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(input: &str) -> ScalarResult<Scalar> {
        log_event!(trace, "Scalar::from_str {input:?}");
        let (payload, type_str) = input
            .rsplit_once("::")
            .ok_or_else(|| ScalarError::Invalid(format!("missing '::<type>' in {input:?}")))?;
        let dtype = DataType::from_str(type_str.trim())?;
        Scalar::from_payload(payload.trim(), &dtype)
    }

    /// Builds a [`Scalar`] from a component `BTreeMap` with `type` and `value` keys (as
    /// produced by [`to_mapping`](Scalar::to_mapping)).
    pub fn from_mapping(
        fields: &std::collections::BTreeMap<String, String>,
    ) -> ScalarResult<Scalar> {
        let type_str = fields
            .get("type")
            .ok_or_else(|| ScalarError::Invalid("mapping missing 'type'".into()))?;
        let payload = fields
            .get("value")
            .ok_or_else(|| ScalarError::Invalid("mapping missing 'value'".into()))?;
        let dtype = DataType::from_str(type_str)?;
        Scalar::from_payload(payload, &dtype)
    }

    /// Renders to a component `BTreeMap` with `type` (the canonical type string) and
    /// `value` (the [payload](Scalar::payload_str)) keys.
    pub fn to_mapping(&self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::from([
            ("type".to_string(), self.data_type().to_str()),
            ("value".to_string(), self.payload_str()),
        ])
    }

    /// Parses a `payload` string against an already-parsed `dtype` (shared by
    /// [`from_str`](Scalar::from_str) and [`from_mapping`](Scalar::from_mapping)).
    fn from_payload(payload: &str, dtype: &DataType) -> ScalarResult<Scalar> {
        use DataType::*;
        let invalid = || ScalarError::Invalid(format!("{payload:?} is not a {}", dtype.to_str()));
        // A typed null literal is valid for any type.
        if payload == "null" {
            return Ok(Scalar::Null(dtype.clone()));
        }
        Ok(match dtype {
            Null => Scalar::Null(Null),
            Boolean => Scalar::Boolean(payload.parse().map_err(|_| invalid())?),
            Int { bits, signed } => {
                Scalar::int(payload.parse().map_err(|_| invalid())?, *bits, *signed)
            }
            Float { bits } => Scalar::float(payload.parse().map_err(|_| invalid())?, *bits),
            Decimal {
                precision,
                scale,
                bits,
            } => Scalar::decimal(
                payload.parse().map_err(|_| invalid())?,
                *precision,
                *scale,
                *bits,
            ),
            Varchar {
                charset,
                large,
                view,
                size,
            } => Scalar::Utf8 {
                value: unquote_str(payload).ok_or_else(invalid)?,
                charset: *charset,
                large: *large,
                view: *view,
                size: *size,
            },
            Json => Scalar::Json(unquote_str(payload).ok_or_else(invalid)?),
            Binary { large, view, size } => Scalar::Binary {
                value: unhex_str(payload).ok_or_else(invalid)?,
                large: *large,
                view: *view,
                size: *size,
            },
            Bson => Scalar::Bson(unhex_str(payload).ok_or_else(invalid)?),
            Date { large } => Scalar::Date {
                value: payload.parse().map_err(|_| invalid())?,
                large: *large,
            },
            Time { unit } => Scalar::Time {
                value: payload.parse().map_err(|_| invalid())?,
                unit: *unit,
            },
            Timestamp { unit, timezone } => Scalar::Timestamp {
                value: payload.parse().map_err(|_| invalid())?,
                unit: *unit,
                timezone: timezone.clone(),
            },
            Duration { unit } => Scalar::Duration {
                value: payload.parse().map_err(|_| invalid())?,
                unit: *unit,
            },
            Interval { .. } => Scalar::Interval(parse_interval(payload).ok_or_else(invalid)?),
            other => {
                return Err(ScalarError::Unsupported(format!(
                    "cannot parse a '{}' scalar from a string; use from_bytes / from_json",
                    other.to_str()
                )))
            }
        })
    }
}

impl fmt::Display for Scalar {
    /// The canonical string (see [`to_str`](Scalar::to_str)).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_str())
    }
}

/// Single-quotes a string, escaping `\` and `'`.
fn quote_str(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\\' || ch == '\'' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('\'');
    out
}

/// Reverses [`quote_str`]: a `'`-quoted, `\`-escaped string, or `None` if not quoted.
fn unquote_str(value: &str) -> Option<String> {
    let inner = value.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut out = String::with_capacity(inner.len());
    let mut escaped = false;
    for ch in inner.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

/// Renders bytes as `0x`-prefixed lowercase hex.
fn hex_str(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Reverses [`hex_str`]: a `0x`-prefixed even-length hex string, or `None`.
fn unhex_str(value: &str) -> Option<Vec<u8>> {
    let hex = value.strip_prefix("0x")?;
    if hex.len() % 2 != 0 {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
}

/// Parses an interval payload (`ym:M` / `dt:D,MS` / `mdn:MO,D,N`).
fn parse_interval(payload: &str) -> Option<Interval> {
    let (tag, rest) = payload.split_once(':')?;
    let nums: Vec<&str> = rest.split(',').collect();
    match tag {
        "ym" if nums.len() == 1 => Some(Interval::YearMonth(nums[0].parse().ok()?)),
        "dt" if nums.len() == 2 => Some(Interval::DayTime {
            days: nums[0].parse().ok()?,
            millis: nums[1].parse().ok()?,
        }),
        "mdn" if nums.len() == 3 => Some(Interval::MonthDayNano {
            months: nums[0].parse().ok()?,
            days: nums[1].parse().ok()?,
            nanos: nums[2].parse().ok()?,
        }),
        _ => None,
    }
}

/// `serde(with)` helpers that string-encode the wide integers, so JSON (and any other
/// `serde` format) carries their full range losslessly.
#[cfg(feature = "serde")]
mod serdex {
    /// An `i128` serialized as its decimal string.
    pub mod str_i128 {
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(value: &i128, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&value.to_string())
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i128, D::Error> {
            String::deserialize(d)?
                .parse()
                .map_err(serde::de::Error::custom)
        }
    }

    /// An `i256` serialized as its decimal string.
    pub mod str_i256 {
        use std::str::FromStr;

        use arrow_buffer::i256;
        use serde::{Deserialize, Deserializer, Serializer};

        pub fn serialize<S: Serializer>(value: &i256, s: S) -> Result<S::Ok, S::Error> {
            s.serialize_str(&value.to_string())
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<i256, D::Error> {
            i256::from_str(&String::deserialize(d)?).map_err(serde::de::Error::custom)
        }
    }
}

#[cfg(feature = "json")]
impl Scalar {
    /// Serialises to a lossless structural JSON string (carrying the exact logical type,
    /// unlike the Arrow-normalised [`to_bytes`](Scalar::to_bytes)). Requires `json`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Scalar serialises")
    }

    /// Parses a [`Scalar`] from the structural JSON of [`to_json`](Scalar::to_json).
    pub fn from_json(json: &str) -> ScalarResult<Scalar> {
        serde_json::from_str(json).map_err(|e| ScalarError::Invalid(e.to_string()))
    }
}
