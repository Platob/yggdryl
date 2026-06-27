//! The concrete per-type scalars — thin typed views over a [`ScalarValue`], each
//! implementing the [`Scalar`] trait. Construct by name (`IntScalar::new(…)`,
//! `DateScalar::from_date(…)`, `StructScalar::from_children(…)`) or via the
//! [`from_value`](crate::from_value) factory; downcast from a [`ScalarRef`] with
//! `as_any().downcast_ref::<…>()`. Each owns a [`ScalarValue`] constrained (by its
//! constructors) to the matching variant, so it shares the value engine while exposing a
//! typed surface.

use std::any::Any;
use std::sync::Arc;

use yggdryl_core::{Date, DateTime, Duration, Time, Timezone};
use yggdryl_schema::Field;

use crate::scalar::{Scalar, ScalarRef, TypedScalar};
use crate::value::{Interval, ScalarValue};

/// Generates a concrete scalar newtype over [`ScalarValue`] plus its [`Scalar`] impl and
/// `From<…> for ScalarValue`.
macro_rules! concrete {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub struct $name(pub(crate) ScalarValue);

        impl Scalar for $name {
            fn value(&self) -> &ScalarValue {
                &self.0
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl From<$name> for ScalarValue {
            fn from(scalar: $name) -> ScalarValue {
                scalar.0
            }
        }

        impl From<$name> for ScalarRef {
            fn from(scalar: $name) -> ScalarRef {
                Arc::new(scalar)
            }
        }
    };
}

concrete!(NullScalar, "A typed null value.");
concrete!(BooleanScalar, "A boolean value.");
concrete!(IntScalar, "An integer value of any width / signedness.");
concrete!(FloatScalar, "A floating-point value of any width.");
concrete!(
    DecimalScalar,
    "A decimal value (precision / scale / storage width)."
);
concrete!(VarcharScalar, "A string value.");
concrete!(BinaryScalar, "An opaque-bytes value.");
concrete!(JsonScalar, "A JSON text value.");
concrete!(BsonScalar, "A BSON document value.");
concrete!(TimezoneScalar, "A timezone value.");
concrete!(DateScalar, "A calendar date value.");
concrete!(TimeScalar, "A time-of-day value.");
concrete!(TimestampScalar, "A timestamp value.");
concrete!(DurationScalar, "A duration (elapsed-time) value.");
concrete!(IntervalScalar, "A calendar interval value.");
concrete!(ListScalar, "A list value with recursive element children.");
concrete!(
    StructScalar,
    "A struct (record) value with recursive field children."
);
concrete!(MapScalar, "A map value with recursive key/value entries.");

// ---- scalar (leaf) constructors + typed access ----

impl NullScalar {
    /// A typed null of `dtype`.
    pub fn new(dtype: yggdryl_schema::DataType) -> Self {
        NullScalar(ScalarValue::null(dtype))
    }
}

impl BooleanScalar {
    /// A boolean scalar.
    pub fn new(value: bool) -> Self {
        BooleanScalar(ScalarValue::boolean(value))
    }
}
impl TypedScalar<bool> for BooleanScalar {
    fn get(&self) -> Option<bool> {
        self.0.as_bool()
    }
}

impl IntScalar {
    /// An integer scalar of `bits` width and the given signedness.
    pub fn new(value: i128, bits: u16, signed: bool) -> Self {
        IntScalar(ScalarValue::int(value, bits, signed))
    }
}
impl TypedScalar<i128> for IntScalar {
    fn get(&self) -> Option<i128> {
        self.0.as_i128()
    }
}

impl FloatScalar {
    /// A floating-point scalar of `bits` width.
    pub fn new(value: f64, bits: u16) -> Self {
        FloatScalar(ScalarValue::float(value, bits))
    }
}
impl TypedScalar<f64> for FloatScalar {
    fn get(&self) -> Option<f64> {
        self.0.as_f64()
    }
}

impl DecimalScalar {
    /// A 128-bit decimal scalar with `(precision, scale)`.
    pub fn new(value: i128, precision: u8, scale: i8) -> Self {
        DecimalScalar(ScalarValue::decimal128(value, precision, scale))
    }
}

impl VarcharScalar {
    /// A string scalar.
    pub fn new(value: impl Into<String>) -> Self {
        VarcharScalar(ScalarValue::utf8(value))
    }
}
impl TypedScalar<String> for VarcharScalar {
    fn get(&self) -> Option<String> {
        self.0.as_str().map(str::to_string)
    }
}

impl BinaryScalar {
    /// An opaque-bytes scalar.
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        BinaryScalar(ScalarValue::binary(value))
    }
}

impl JsonScalar {
    /// A JSON text scalar.
    pub fn new(value: impl Into<String>) -> Self {
        JsonScalar(ScalarValue::json(value))
    }
}

impl BsonScalar {
    /// A BSON document scalar.
    pub fn new(value: impl Into<Vec<u8>>) -> Self {
        BsonScalar(ScalarValue::bson(value))
    }
}

impl TimezoneScalar {
    /// A timezone scalar.
    pub fn new(value: Timezone) -> Self {
        TimezoneScalar(ScalarValue::timezone(value))
    }
}
impl TypedScalar<Timezone> for TimezoneScalar {
    fn get(&self) -> Option<Timezone> {
        self.0.as_timezone().cloned()
    }
}

impl IntervalScalar {
    /// An interval scalar.
    pub fn new(value: Interval) -> Self {
        IntervalScalar(ScalarValue::Interval(value))
    }
}

// ---- temporal constructors + typed access (Stage 1 delegates to the core types) ----

impl DateScalar {
    /// A day-resolution date scalar from a count of days since the epoch.
    pub fn new(days: i32) -> Self {
        DateScalar(ScalarValue::date(days))
    }
    /// A date scalar from a core [`Date`].
    pub fn from_date(value: &Date) -> Self {
        DateScalar(ScalarValue::from_date(value))
    }
}
impl TypedScalar<Date> for DateScalar {
    fn get(&self) -> Option<Date> {
        self.0.as_date()
    }
}

impl TimeScalar {
    /// A nanosecond time-of-day scalar from a core [`Time`].
    pub fn from_time(value: &Time) -> Self {
        TimeScalar(ScalarValue::from_time(value))
    }
}
impl TypedScalar<Time> for TimeScalar {
    fn get(&self) -> Option<Time> {
        self.0.as_time()
    }
}

impl TimestampScalar {
    /// A nanosecond timestamp scalar from a core [`DateTime`].
    pub fn from_datetime(value: &DateTime) -> Self {
        TimestampScalar(ScalarValue::from_datetime(value))
    }
}
impl TypedScalar<DateTime> for TimestampScalar {
    fn get(&self) -> Option<DateTime> {
        self.0.as_datetime()
    }
}

impl DurationScalar {
    /// A nanosecond duration scalar from a core [`Duration`].
    pub fn from_duration(value: &Duration) -> Self {
        DurationScalar(ScalarValue::from_duration(value))
    }
}
impl TypedScalar<Duration> for DurationScalar {
    fn get(&self) -> Option<Duration> {
        self.0.as_duration()
    }
}

// ---- nested constructors + recursive child access ----

impl ListScalar {
    /// A list scalar from its element children and element [`Field`].
    pub fn from_children(field: Field, values: Vec<ScalarRef>) -> Self {
        ListScalar(ScalarValue::List {
            values: values.iter().map(|s| s.value().clone()).collect(),
            field: Box::new(field),
            large: false,
            view: false,
            size: None,
        })
    }

    /// The element values, each boxed as a [`ScalarRef`].
    pub fn values(&self) -> Vec<ScalarRef> {
        match &self.0 {
            ScalarValue::List { values, .. } => {
                values.iter().map(|v| v.clone().into_scalar()).collect()
            }
            _ => Vec::new(),
        }
    }

    /// The number of elements.
    pub fn len(&self) -> usize {
        match &self.0 {
            ScalarValue::List { values, .. } => values.len(),
            _ => 0,
        }
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl StructScalar {
    /// A struct scalar from its fields and one value per field.
    pub fn from_children(fields: Vec<Field>, values: Vec<ScalarRef>) -> Self {
        StructScalar(ScalarValue::Struct {
            fields,
            values: values.iter().map(|s| s.value().clone()).collect(),
        })
    }

    /// The child value at `index`, boxed as a [`ScalarRef`].
    pub fn child(&self, index: usize) -> Option<ScalarRef> {
        match &self.0 {
            ScalarValue::Struct { values, .. } => {
                values.get(index).map(|v| v.clone().into_scalar())
            }
            _ => None,
        }
    }

    /// The child value for field `name`, boxed as a [`ScalarRef`].
    pub fn child_named(&self, name: &str) -> Option<ScalarRef> {
        match &self.0 {
            ScalarValue::Struct { fields, values } => fields
                .iter()
                .position(|f| f.name() == name)
                .and_then(|i| values.get(i))
                .map(|v| v.clone().into_scalar()),
            _ => None,
        }
    }

    /// All child values, in field order, each boxed as a [`ScalarRef`].
    pub fn children(&self) -> Vec<ScalarRef> {
        match &self.0 {
            ScalarValue::Struct { values, .. } => {
                values.iter().map(|v| v.clone().into_scalar()).collect()
            }
            _ => Vec::new(),
        }
    }
}

impl MapScalar {
    /// A map scalar from key/value types and its entries.
    pub fn from_entries(
        key: yggdryl_schema::DataType,
        value: yggdryl_schema::DataType,
        sorted: bool,
        entries: Vec<(ScalarRef, ScalarRef)>,
    ) -> Self {
        MapScalar(ScalarValue::Map {
            key: Box::new(key),
            value: Box::new(value),
            sorted,
            entries: entries
                .iter()
                .map(|(k, v)| (k.value().clone(), v.value().clone()))
                .collect(),
        })
    }

    /// The `(key, value)` entries, each boxed as a [`ScalarRef`].
    pub fn entries(&self) -> Vec<(ScalarRef, ScalarRef)> {
        match &self.0 {
            ScalarValue::Map { entries, .. } => entries
                .iter()
                .map(|(k, v)| (k.clone().into_scalar(), v.clone().into_scalar()))
                .collect(),
            _ => Vec::new(),
        }
    }
}
