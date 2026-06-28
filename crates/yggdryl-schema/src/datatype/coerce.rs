//! Type conversion and unification: cast feasibility ([`can_cast_to`](DataType::can_cast_to)),
//! the promotion lattice ([`common_type`](DataType::common_type)) and the schema
//! [`merge`](DataType::merge) [`MergeStrategy`].

use std::fmt;

use super::fixed::FixedKind;
use super::{DataType, SchemaError};
#[allow(unused_imports)]
use crate::log_event;
use crate::Field;
use yggdryl_core::TimeUnit;

/// How [`merge`](DataType::merge) reconciles two types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum MergeStrategy {
    /// The types must be identical (after resolving the [`Any`](DataType::Any) /
    /// [`Null`](DataType::Null) identity elements); any other difference errors.
    Strict,
    /// Widen both to their [`common_type`](DataType::common_type); error if none exists.
    #[default]
    Promote,
    /// Like [`Promote`](MergeStrategy::Promote) but never errors — incompatible
    /// types collapse to the [`Any`](DataType::Any) wildcard. The collapse is at the
    /// **top level**: if a nested leaf (a struct field, a map value) has no common
    /// type the whole container becomes `Any`, rather than collapsing only the leaf.
    Permissive,
}

impl MergeStrategy {
    /// Parses a strategy name (case-insensitive).
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Result<MergeStrategy, SchemaError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" => Ok(MergeStrategy::Strict),
            "promote" | "widen" => Ok(MergeStrategy::Promote),
            "permissive" | "lenient" => Ok(MergeStrategy::Permissive),
            _ => Err(SchemaError::UnknownUnit(value.to_string())),
        }
    }

    /// The lowercase name (`"strict"` / `"promote"` / `"permissive"`).
    pub fn as_str(&self) -> &'static str {
        match self {
            MergeStrategy::Strict => "strict",
            MergeStrategy::Promote => "promote",
            MergeStrategy::Permissive => "permissive",
        }
    }
}

impl fmt::Display for MergeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl DataType {
    /// Whether a value of this type can be cast to `target` — a broad,
    /// Arrow-compatible feasibility check (not every cast is loss-free).
    ///
    /// ```
    /// use yggdryl_schema::DataType;
    /// assert!(DataType::int(32, true).can_cast_to(&DataType::varchar()));
    /// assert!(!DataType::int(32, true).can_cast_to(&DataType::binary()));
    /// ```
    pub fn can_cast_to(&self, target: &DataType) -> bool {
        use DataType::*;
        if self == target || self.is_any() || target.is_any() {
            return true;
        }
        if self.is_null() {
            return true;
        }
        if target.is_null() {
            return false;
        }
        let scalar =
            |d: &DataType| d.is_numeric() || d.is_decimal() || d.is_boolean() || d.is_string();
        match (self, target) {
            (a, b) if scalar(a) && scalar(b) => true,
            (a, b) if a.is_binary() && (b.is_binary() || b.is_string()) => true,
            (a, b) if a.is_string() && b.is_binary() => true,
            (a, b) if a.is_temporal() && (b.is_temporal() || b.is_integer() || b.is_string()) => {
                true
            }
            (a, b) if (a.is_integer() || a.is_string()) && b.is_temporal() => true,
            (Dictionary { value, .. }, b) => value.can_cast_to(b),
            (a, Dictionary { value, .. }) => a.can_cast_to(value),
            // Run-end encoding is transparent to casting, like a dictionary.
            (RunEndEncoded { values, .. }, b) => values.can_cast_to(b),
            (a, RunEndEncoded { values, .. }) => a.can_cast_to(values),
            // Json <-> string, Bson <-> binary and Timezone <-> string cast through
            // their physical type.
            (a, b) if a.is_json() && b.is_string() => true,
            (a, b) if a.is_string() && b.is_json() => true,
            (a, b) if a.is_bson() && b.is_binary() => true,
            (a, b) if a.is_binary() && b.is_bson() => true,
            (a, b) if a.is_timezone() && b.is_string() => true,
            (a, b) if a.is_string() && b.is_timezone() => true,
            (List { item: a, .. }, List { item: b, .. }) => {
                a.data_type().can_cast_to(b.data_type())
            }
            // Casting matches struct fields *positionally* (a layout cast); merging
            // (`common_type`) unions them *by name*. The two have different goals.
            (Struct(a), Struct(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|(x, y)| x.data_type().can_cast_to(y.data_type()))
            }
            (
                Map {
                    key: ak, value: av, ..
                },
                Map {
                    key: bk, value: bv, ..
                },
            ) => ak.can_cast_to(bk) && av.can_cast_to(bv),
            _ => false,
        }
    }

    /// The least type both `self` and `other` can widen to without loss of range,
    /// or `None`. [`Any`](DataType::Any) / [`Null`](DataType::Null) are identity
    /// elements; integers/floats promote, decimals grow, strings/binaries widen,
    /// and nested types recurse (structs union their fields by name).
    ///
    /// ```
    /// use yggdryl_schema::DataType;
    /// assert_eq!(DataType::int(8, true).common_type(&DataType::int(32, true)),
    ///            Some(DataType::int(32, true)));
    /// assert_eq!(DataType::int(32, true).common_type(&DataType::float(32)),
    ///            Some(DataType::float(64)));
    /// ```
    pub fn common_type(&self, other: &DataType) -> Option<DataType> {
        use DataType::*;
        if self == other {
            return Some(self.clone());
        }
        match (self, other) {
            (Any, t) | (t, Any) => Some(t.clone()),
            (Null, t) | (t, Null) => Some(t.clone()),
            (a, b) if a.is_integer() && b.is_integer() => Some(common_integer(a, b)),
            (a, b) if a.is_numeric() && b.is_numeric() => Some(common_numeric(a, b)),
            (a, b) if a.is_decimal() || b.is_decimal() => common_decimal(a, b),
            (
                Varchar {
                    charset: c1,
                    large: l1,
                    view: v1,
                    size: z1,
                },
                Varchar {
                    charset: c2,
                    large: l2,
                    view: v2,
                    size: z2,
                },
            ) if c1 == c2 => Some(Varchar {
                charset: *c1,
                large: *l1 || *l2,
                view: *v1 && *v2,
                // A common fixed size only when both agree, else fall to variable.
                size: if z1 == z2 { *z1 } else { None },
            }),
            (
                Binary {
                    large: l1,
                    view: v1,
                    size: s1,
                },
                Binary {
                    large: l2,
                    view: v2,
                    size: s2,
                },
            ) => Some(Binary {
                large: *l1 || *l2,
                view: *v1 && *v2,
                size: if s1 == s2 { *s1 } else { None },
            }),
            (Date { large: a }, Date { large: b }) => Some(Date { large: *a || *b }),
            (Time { unit: a }, Time { unit: b }) => Some(Time {
                unit: finer(*a, *b),
            }),
            (Duration { unit: a }, Duration { unit: b }) => Some(Duration {
                unit: finer(*a, *b),
            }),
            (
                Timestamp {
                    unit: a,
                    timezone: za,
                },
                Timestamp {
                    unit: b,
                    timezone: zb,
                },
            ) if za == zb => Some(Timestamp {
                unit: finer(*a, *b),
                timezone: za.clone(),
            }),
            (Interval { unit: a }, Interval { unit: b }) => Some(Interval {
                // The only superset carrying both calendar fields is MonthDayNano
                // (months + days + nanos); picking max(unit) would drop components
                // (e.g. YearMonth's months when widened to DayTime).
                unit: if a == b {
                    *a
                } else {
                    crate::IntervalUnit::MonthDayNano
                },
            }),
            (
                List {
                    item: ia,
                    large: la,
                    view: va,
                    size: sa,
                },
                List {
                    item: ib,
                    large: lb,
                    view: vb,
                    size: sb,
                },
            ) => {
                let item = promote_field(ia, ib)?;
                let size = if sa == sb { *sa } else { None };
                Some(DataType::List {
                    item: Box::new(item),
                    large: *la || *lb,
                    view: *va && *vb,
                    size,
                })
            }
            (Struct(a), Struct(b)) => Some(DataType::Struct(merge_struct_fields(a, b)?)),
            (
                Map {
                    key: ak,
                    value: av,
                    sorted: sa,
                },
                Map {
                    key: bk,
                    value: bv,
                    sorted: sb,
                },
            ) => Some(DataType::map(
                ak.common_type(bk)?,
                av.common_type(bv)?,
                *sa && *sb,
            )),
            (Dictionary { value: v1, .. }, Dictionary { value: v2, .. }) => v1.common_type(v2),
            (Dictionary { value, .. }, t) | (t, Dictionary { value, .. }) => value.common_type(t),
            (RunEndEncoded { values: v1, .. }, RunEndEncoded { values: v2, .. }) => {
                v1.common_type(v2)
            }
            (RunEndEncoded { values, .. }, t) | (t, RunEndEncoded { values, .. }) => {
                values.common_type(t)
            }
            // Json / Bson / Timezone widen to their physical string / binary supertype.
            (Json, t) | (t, Json) if t.is_string() => Some(t.clone()),
            (Bson, t) | (t, Bson) if t.is_binary() => Some(t.clone()),
            (Timezone, t) | (t, Timezone) if t.is_string() => Some(t.clone()),
            _ => None,
        }
    }

    /// Merges this type with `other` under the chosen [`MergeStrategy`].
    ///
    /// ```
    /// use yggdryl_schema::{DataType, MergeStrategy};
    /// assert_eq!(DataType::int(8, true).merge(&DataType::int(64, true), MergeStrategy::Promote).unwrap(),
    ///            DataType::int(64, true));
    /// assert_eq!(DataType::int(8, true).merge(&DataType::varchar(), MergeStrategy::Permissive).unwrap(),
    ///            DataType::Any);
    /// ```
    pub fn merge(
        &self,
        other: &DataType,
        strategy: MergeStrategy,
    ) -> Result<DataType, SchemaError> {
        let incompatible = || SchemaError::Incompatible {
            left: self.to_str(),
            right: other.to_str(),
        };
        match strategy {
            MergeStrategy::Strict => {
                if self == other || other.is_any() || other.is_null() {
                    Ok(self.clone())
                } else if self.is_any() || self.is_null() {
                    Ok(other.clone())
                } else {
                    Err(incompatible())
                }
            }
            MergeStrategy::Promote => self.common_type(other).ok_or_else(incompatible),
            MergeStrategy::Permissive => Ok(self.common_type(other).unwrap_or(DataType::Any)),
        }
    }
}

/// Unions two struct field lists by name (promote semantics): a field present on
/// only one side becomes nullable; shared fields are merged. `None` if any shared
/// field has no common type.
fn merge_struct_fields(a: &[Field], b: &[Field]) -> Option<Vec<Field>> {
    let mut fields = Vec::with_capacity(a.len().max(b.len()));
    for f in a {
        match b.iter().find(|x| x.name() == f.name()) {
            Some(other) => fields.push(promote_field(f, other)?),
            None => fields.push(f.clone().with_nullable(true)),
        }
    }
    for other in b {
        if !a.iter().any(|x| x.name() == other.name()) {
            fields.push(other.clone().with_nullable(true));
        }
    }
    Some(fields)
}

/// Promotes two fields to a common field (merged type, nullable if either is,
/// keeping the first's metadata then folding in the second's — matching
/// [`Field::merge`](crate::Field::merge), without its name-equality check so list
/// items with differing element names can still promote).
fn promote_field(a: &Field, b: &Field) -> Option<Field> {
    let data_type = a.data_type().common_type(b.data_type())?;
    let mut metadata = a.metadata().clone();
    for (key, value) in b.metadata() {
        metadata.entry(key.clone()).or_insert_with(|| value.clone());
    }
    // Build the merged field directly (keeping only `a`'s name) rather than cloning the
    // whole of `a` and overwriting its type / nullability / metadata.
    Some(
        Field::new(a.name(), data_type, a.is_nullable() || b.is_nullable()).with_metadata(metadata),
    )
}

/// `(bit width, is signed)` of an integer type (via its fixed descriptor).
fn int_meta(dt: &DataType) -> (u32, bool) {
    match dt.fixed() {
        Some(t) => (t.bits() as u32, t.signed()),
        None => (64, true),
    }
}

/// The integer type of the given width and signedness.
fn int_of(bits: u32, signed: bool) -> DataType {
    DataType::int(bits as u16, signed)
}

/// The smallest integer type holding both inputs, falling back to `float64` when a
/// signed type cannot cover an unsigned 64-bit range.
fn common_integer(a: &DataType, b: &DataType) -> DataType {
    let (ab, asg) = int_meta(a);
    let (bb, bsg) = int_meta(b);
    if asg == bsg {
        return int_of(ab.max(bb), asg);
    }
    let (sbits, ubits) = if asg { (ab, bb) } else { (bb, ab) };
    let needed = match ubits {
        8 => 16,
        16 => 32,
        32 => 64,
        _ => {
            // uint64 has no signed integer superset; float64 is the lossy fallback.
            log_event!(
                warn,
                "common_integer: uint64 vs signed widened to float64 (range-lossy)"
            );
            return DataType::float(64);
        }
    };
    int_of(sbits.max(needed).min(64), true)
}

/// The bit width of a float type (via its fixed descriptor).
fn float_bits(dt: &DataType) -> u32 {
    dt.fixed().map(|t| t.bits() as u32).unwrap_or(64)
}

/// The float that safely holds an integer type's range.
fn float_for_int(dt: &DataType) -> u32 {
    match int_meta(dt) {
        (bits, _) if bits <= 16 => 32,
        _ => 64,
    }
}

/// The common type of a numeric pair where at least one side is a float.
fn common_numeric(a: &DataType, b: &DataType) -> DataType {
    let fa = if a.is_floating() {
        float_bits(a)
    } else {
        float_for_int(a)
    };
    let fb = if b.is_floating() {
        float_bits(b)
    } else {
        float_for_int(b)
    };
    DataType::float(fa.max(fb) as u16)
}

/// Treats an integer or decimal as `(precision, scale)`, or `None`.
fn as_decimal(dt: &DataType) -> Option<(i16, i16)> {
    if let Some((p, s)) = dt.decimal_parts() {
        return Some((p as i16, s as i16));
    }
    if !dt.is_integer() {
        return None;
    }
    let (bits, signed) = int_meta(dt);
    let digits = match (bits, signed) {
        (8, _) => 3,
        (16, _) => 5,
        (32, _) => 10,
        (64, true) => 19,
        _ => 20,
    };
    Some((digits, 0))
}

/// The decimal storage width (bits) for a precision, at least `min_bits`.
fn decimal_bits_for(precision: u8, min_bits: u16) -> u16 {
    let needed = match precision {
        0..=9 => 32,
        10..=18 => 64,
        19..=38 => 128,
        _ => 256,
    };
    needed.max(min_bits)
}

/// The common type of a pair involving at least one decimal.
fn common_decimal(a: &DataType, b: &DataType) -> Option<DataType> {
    if a.is_floating() || b.is_floating() {
        return Some(DataType::float(64));
    }
    let (p1, s1) = as_decimal(a)?;
    let (p2, s2) = as_decimal(b)?;
    let scale = s1.max(s2);
    let lead = (p1 - s1).max(p2 - s2);
    let total = lead + scale;
    // A decimal tops out at 76 digits; clamping `total` would silently drop integer
    // digits, so widen to float64 instead (consistent with the decimal+float case).
    if total > 76 {
        log_event!(
            warn,
            "common_decimal: {total} digits exceed decimal's 76, widening to float64"
        );
        return Some(DataType::float(64));
    }
    let precision = total.clamp(1, 76) as u8;
    let min_bits = decimal_bits_of(a).max(decimal_bits_of(b));
    Some(DataType::decimal_with(
        precision,
        scale.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
        decimal_bits_for(precision, min_bits),
    ))
}

/// The storage bits of a decimal type, or `0` if not a decimal.
fn decimal_bits_of(dt: &DataType) -> u16 {
    match dt.fixed() {
        Some(t) if t.kind() == FixedKind::Decimal => t.bits(),
        _ => 0,
    }
}

/// The higher-resolution of two time units.
fn finer(a: TimeUnit, b: TimeUnit) -> TimeUnit {
    if a >= b {
        a
    } else {
        b
    }
}
