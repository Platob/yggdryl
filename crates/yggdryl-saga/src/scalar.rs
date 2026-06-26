//! The [`Scalar`] literal — a single typed value used in [`Expression`]s and
//! [`Predicate`]s. Its [`cast`](Scalar::cast) is where an untyped
//! ([`Any`](DataType::Any)) or string literal becomes a typed one (e.g. an ISO
//! date string → a `timestamp`), so a comparison can be pushed into typed storage.
//!
//! [`Expression`]: crate::Expression
//! [`Predicate`]: crate::Predicate

use std::fmt;

use crate::cast::{parse_iso, CastError};
#[allow(unused_imports)]
use crate::log_event;
use crate::{DataType, LogicalType, PrimitiveType, TimeUnit};

/// The payload of a [`Scalar`], independent of its declared [`DataType`]. Temporal
/// values are normalised to an integer tick count (and tagged by the scalar's
/// `data_type`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
enum ScalarValue {
    /// A null of the scalar's declared type.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer (also the normalised form of a temporal tick count).
    Int(i64),
    /// A floating-point number.
    Float(f64),
    /// A UTF-8 string (also the raw form of an untyped [`Any`](DataType::Any) value).
    Str(String),
}

/// A single typed literal value.
///
/// A scalar pairs a value with its declared [`DataType`], which may be the dynamic
/// [`Any`](DataType::Any) — the state a freshly-written filter literal starts in
/// before a [`Frame`](crate::Frame) resolves the target column's type.
/// [`cast`](Scalar::cast) then converts it, including the headline case of an ISO
/// date/`timestamp` string becoming a typed temporal value.
///
/// ```
/// use yggdryl_saga::{DataType, Scalar};
///
/// // An untyped literal (the kind a predicate carries) …
/// let raw = Scalar::any("2024-01-01");
/// assert!(raw.data_type().is_any());
///
/// // … cast to the column's timestamp type for pushdown.
/// let ts = DataType::from_str("timestamp(ns, UTC)").unwrap();
/// let typed = raw.cast(&ts).unwrap();
/// assert_eq!(typed.data_type(), &ts);
/// assert_eq!(typed.as_i64(), Some(19723 * 86_400 * 1_000_000_000));
/// ```
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Scalar {
    value: ScalarValue,
    data_type: DataType,
}

impl Scalar {
    /// A typed null.
    pub fn null(data_type: DataType) -> Scalar {
        Scalar {
            value: ScalarValue::Null,
            data_type,
        }
    }

    /// A boolean literal.
    pub fn boolean(value: bool) -> Scalar {
        Scalar {
            value: ScalarValue::Bool(value),
            data_type: PrimitiveType::Boolean.into(),
        }
    }

    /// A 64-bit integer literal.
    pub fn int64(value: i64) -> Scalar {
        Scalar {
            value: ScalarValue::Int(value),
            data_type: PrimitiveType::Int64.into(),
        }
    }

    /// A 64-bit float literal.
    pub fn float64(value: f64) -> Scalar {
        Scalar {
            value: ScalarValue::Float(value),
            data_type: PrimitiveType::Float64.into(),
        }
    }

    /// A UTF-8 string literal.
    pub fn utf8(value: impl Into<String>) -> Scalar {
        Scalar {
            value: ScalarValue::Str(value.into()),
            data_type: PrimitiveType::Utf8.into(),
        }
    }

    /// An **untyped** ([`Any`](DataType::Any)) literal carrying its raw string form
    /// — the state a filter value starts in until cast to a column's type.
    pub fn any(value: impl Into<String>) -> Scalar {
        Scalar {
            value: ScalarValue::Str(value.into()),
            data_type: DataType::Any,
        }
    }

    /// The scalar's declared [`DataType`].
    pub fn data_type(&self) -> &DataType {
        &self.data_type
    }

    /// Whether the value is null.
    pub fn is_null(&self) -> bool {
        matches!(self.value, ScalarValue::Null)
    }

    /// The value as an `i64`, if it is integer-shaped (an integer or a normalised
    /// temporal tick count).
    pub fn as_i64(&self) -> Option<i64> {
        match self.value {
            ScalarValue::Int(i) => Some(i),
            _ => None,
        }
    }

    /// The value as an `f64`, if it is a float.
    pub fn as_f64(&self) -> Option<f64> {
        match self.value {
            ScalarValue::Float(f) => Some(f),
            _ => None,
        }
    }

    /// The value as a `bool`, if it is boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match self.value {
            ScalarValue::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// The value as a string slice, if it is a string.
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            ScalarValue::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Casts the literal to `target`, returning a new scalar of that type.
    ///
    /// Numbers, booleans and strings interconvert; a string (or untyped
    /// [`Any`](DataType::Any)) value is parsed as ISO-8601 when the target is a
    /// temporal type, and an integer is taken as a raw tick count. Returns
    /// [`CastError`] when the conversion is undefined or the value does not parse.
    pub fn cast(&self, target: &DataType) -> Result<Scalar, CastError> {
        log_event!(trace, "Scalar::cast {:?} -> {target}", self.data_type);
        // A null stays null; a same-type cast is a no-op.
        if self.is_null() {
            return Ok(Scalar::null(target.clone()));
        }
        if &self.data_type == target {
            return Ok(self.clone());
        }
        if !self.data_type.can_cast_to(target) {
            return Err(CastError::Unsupported {
                from: self.data_type.clone(),
                to: target.clone(),
            });
        }

        let value = if target.is_any() {
            self.value.clone()
        } else if target.is_boolean() {
            ScalarValue::Bool(self.to_bool(target)?)
        } else if target.is_temporal() {
            ScalarValue::Int(self.to_temporal(target)?)
        } else if matches!(target, DataType::Primitive(p) if p.is_integer()) || target.is_decimal()
        {
            ScalarValue::Int(self.to_i64(target)?)
        } else if matches!(target, DataType::Primitive(p) if p.is_floating()) {
            ScalarValue::Float(self.to_f64(target)?)
        } else if target.is_string() {
            ScalarValue::Str(self.render())
        } else {
            return Err(CastError::Unsupported {
                from: self.data_type.clone(),
                to: target.clone(),
            });
        };
        Ok(Scalar {
            value,
            data_type: target.clone(),
        })
    }

    /// Renders the value to a string (the source for a string cast).
    fn render(&self) -> String {
        match &self.value {
            ScalarValue::Null => String::new(),
            ScalarValue::Bool(b) => b.to_string(),
            ScalarValue::Int(i) => i.to_string(),
            ScalarValue::Float(f) => f.to_string(),
            ScalarValue::Str(s) => s.clone(),
        }
    }

    fn invalid(&self, target: &DataType) -> CastError {
        CastError::InvalidValue {
            value: self.render(),
            target: target.clone(),
        }
    }

    fn to_bool(&self, target: &DataType) -> Result<bool, CastError> {
        match &self.value {
            ScalarValue::Bool(b) => Ok(*b),
            ScalarValue::Int(i) => Ok(*i != 0),
            ScalarValue::Float(f) => Ok(*f != 0.0),
            ScalarValue::Str(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "t" | "yes" => Ok(true),
                "false" | "0" | "f" | "no" => Ok(false),
                _ => Err(self.invalid(target)),
            },
            ScalarValue::Null => Err(self.invalid(target)),
        }
    }

    fn to_i64(&self, target: &DataType) -> Result<i64, CastError> {
        match &self.value {
            ScalarValue::Int(i) => Ok(*i),
            ScalarValue::Float(f) => Ok(*f as i64),
            ScalarValue::Bool(b) => Ok(*b as i64),
            ScalarValue::Str(s) => s
                .trim()
                .parse::<i64>()
                .or_else(|_| s.trim().parse::<f64>().map(|f| f as i64))
                .map_err(|_| self.invalid(target)),
            ScalarValue::Null => Err(self.invalid(target)),
        }
    }

    fn to_f64(&self, target: &DataType) -> Result<f64, CastError> {
        match &self.value {
            ScalarValue::Float(f) => Ok(*f),
            ScalarValue::Int(i) => Ok(*i as f64),
            ScalarValue::Bool(b) => Ok(*b as i64 as f64),
            ScalarValue::Str(s) => s.trim().parse::<f64>().map_err(|_| self.invalid(target)),
            ScalarValue::Null => Err(self.invalid(target)),
        }
    }

    /// Converts the value to the integer tick count of a temporal `target`. A
    /// string is parsed as ISO-8601; an integer is taken as an already-scaled tick
    /// count.
    fn to_temporal(&self, target: &DataType) -> Result<i64, CastError> {
        let logical = match target {
            DataType::Logical(l) => l,
            _ => return Err(self.invalid(target)),
        };
        match &self.value {
            // An already-numeric value is a raw tick count in the target unit.
            ScalarValue::Int(i) => Ok(*i),
            ScalarValue::Float(f) => Ok(*f as i64),
            ScalarValue::Str(s) => {
                let instant = parse_iso(s).ok_or_else(|| self.invalid(target))?;
                Ok(match logical {
                    LogicalType::Date32 => instant.days,
                    LogicalType::Date64 => instant.epoch(TimeUnit::Millisecond),
                    LogicalType::Timestamp(unit, _) => instant.epoch(*unit),
                    LogicalType::Time32(unit) | LogicalType::Time64(unit) => {
                        instant.time_of_day(*unit)
                    }
                    LogicalType::Duration(unit) => instant.epoch(*unit),
                    _ => return Err(self.invalid(target)),
                })
            }
            _ => Err(self.invalid(target)),
        }
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.value {
            ScalarValue::Null => write!(f, "null"),
            other => write!(f, "{}", DisplayValue(other)),
        }
    }
}

struct DisplayValue<'a>(&'a ScalarValue);
impl fmt::Display for DisplayValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            ScalarValue::Null => write!(f, "null"),
            ScalarValue::Bool(b) => write!(f, "{b}"),
            ScalarValue::Int(i) => write!(f, "{i}"),
            ScalarValue::Float(x) => write!(f, "{x}"),
            ScalarValue::Str(s) => write!(f, "{s:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DataType {
        DataType::from_str("timestamp(ns, UTC)").unwrap()
    }

    #[test]
    fn iso_string_casts_to_timestamp() {
        let typed = Scalar::any("2024-01-01").cast(&ts()).unwrap();
        assert_eq!(typed.data_type(), &ts());
        assert_eq!(typed.as_i64(), Some(19723 * 86_400 * 1_000_000_000));
    }

    #[test]
    fn iso_string_casts_to_date32() {
        let date32 = DataType::from_str("date32").unwrap();
        let typed = Scalar::utf8("2024-01-01").cast(&date32).unwrap();
        assert_eq!(typed.as_i64(), Some(19723));
    }

    #[test]
    fn numeric_and_bool_and_string_interconvert() {
        assert_eq!(
            Scalar::int64(42)
                .cast(&PrimitiveType::Float64.into())
                .unwrap()
                .as_f64(),
            Some(42.0)
        );
        assert_eq!(
            Scalar::utf8("3.5")
                .cast(&PrimitiveType::Float64.into())
                .unwrap()
                .as_f64(),
            Some(3.5)
        );
        assert_eq!(
            Scalar::utf8("true")
                .cast(&PrimitiveType::Boolean.into())
                .unwrap()
                .as_bool(),
            Some(true)
        );
        assert_eq!(
            Scalar::int64(7)
                .cast(&PrimitiveType::Utf8.into())
                .unwrap()
                .as_str(),
            Some("7")
        );
    }

    #[test]
    fn integer_is_a_raw_tick_count() {
        // An int epoch passes through unchanged as the tick count.
        let typed = Scalar::int64(1_000).cast(&ts()).unwrap();
        assert_eq!(typed.as_i64(), Some(1_000));
    }

    #[test]
    fn null_and_errors() {
        let n = Scalar::null(PrimitiveType::Int64.into());
        assert!(n.cast(&ts()).unwrap().is_null());
        // A non-ISO string cannot become a timestamp.
        assert!(matches!(
            Scalar::utf8("not-a-date").cast(&ts()),
            Err(CastError::InvalidValue { .. })
        ));
        // A struct target is unsupported.
        let struct_ty = DataType::from_str("struct<a: int64>").unwrap();
        assert!(matches!(
            Scalar::int64(1).cast(&struct_ty),
            Err(CastError::Unsupported { .. })
        ));
    }
}
