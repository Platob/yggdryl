//! The [`Scalar`] trait — the object-safe, per-type interface every concrete scalar
//! ([`IntScalar`](crate::IntScalar), [`VarcharScalar`](crate::VarcharScalar),
//! [`DateScalar`](crate::DateScalar), [`StructScalar`](crate::StructScalar), …)
//! implements, mirroring the `Serie` trait at the value level. Each concrete is a thin
//! typed view over the shared [`ScalarValue`] engine (which owns the Arrow / serialization
//! logic); [`ScalarRef`] is the boxed handle and [`from_value`] /
//! [`ScalarValue::into_scalar`] wrap a value as the right concrete.

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef};
use yggdryl_schema::DataType;

use crate::concrete::*;
use crate::error::ScalarResult;
use crate::value::ScalarValue;

/// A reference-counted, type-erased atomic value — the value-level analogue of
/// `SerieRef`, the handle the [factory](from_value) and a `Serie` hand around.
pub type ScalarRef = Arc<dyn Scalar>;

/// The object-safe interface of an atomic value. A concrete scalar supplies only its
/// tagged [`value`](Scalar::value) and [`as_any`](Scalar::as_any); every other method
/// defaults off the shared [`ScalarValue`] engine (Arrow conversion, serialization,
/// casting). Typed value access is added by
/// [`TypedScalar<T>`](crate::TypedScalar).
pub trait Scalar: fmt::Debug + Send + Sync {
    /// The tagged value backing this scalar (the shared representation + engine).
    fn value(&self) -> &ScalarValue;

    /// Downcast hook — recover the concrete scalar (e.g.
    /// `s.as_any().downcast_ref::<IntScalar>()`).
    fn as_any(&self) -> &dyn Any;

    /// The exact [`DataType`], as a shared **interned** `Arc` (cheap to attach to many
    /// values).
    fn data_type(&self) -> Arc<DataType> {
        self.value().data_type().interned()
    }

    /// Whether this is a null value.
    fn is_null(&self) -> bool {
        self.value().is_null()
    }

    /// Renders as a length-1 Arrow [`ArrayRef`].
    fn to_array(&self) -> ScalarResult<ArrayRef> {
        self.value().to_array()
    }

    /// Wraps [`to_array`](Scalar::to_array) in an [`arrow_array::Scalar`] broadcast marker.
    fn to_arrow_scalar(&self) -> ScalarResult<arrow_array::Scalar<ArrayRef>> {
        self.value().to_arrow_scalar()
    }

    /// Lossless Arrow-IPC bytes (round-trips via [`from_bytes`](crate::from_bytes)).
    fn to_bytes(&self) -> ScalarResult<Vec<u8>> {
        self.value().to_bytes()
    }

    /// The canonical string (`"42::int64"`).
    fn to_str(&self) -> String {
        self.value().to_str()
    }

    /// The `{type, value}` component map.
    fn to_mapping(&self) -> BTreeMap<String, String> {
        self.value().to_mapping()
    }

    /// Casts to `dtype` by running Arrow's cast kernel over the length-1 array, returning
    /// the right concrete scalar (lossy / narrowing casts yield null on overflow). Nested
    /// types follow the Arrow kernel's own semantics; for a by-name struct cast that fills
    /// missing target columns, cast at the [`Serie`](yggdryl_serie) layer instead.
    fn cast(&self, dtype: &DataType) -> ScalarResult<ScalarRef> {
        Ok(self.value().cast(dtype)?.into_scalar())
    }

    /// `self + rhs`, boxed as a [`ScalarRef`] — the default delegates to
    /// [`ScalarValue::add`], which promotes numeric operands, defines a few temporal
    /// combinations, and raises [`ScalarError::Unsupported`](crate::ScalarError::Unsupported)
    /// for a combination with no defined sum. Every concrete scalar inherits it, so it
    /// either computes or says why it can't.
    fn add(&self, rhs: &dyn Scalar) -> ScalarResult<ScalarRef> {
        Ok(self.value().add(rhs.value())?.into_scalar())
    }

    /// `self - rhs`, boxed as a [`ScalarRef`] (see [`add`](Scalar::add)).
    fn sub(&self, rhs: &dyn Scalar) -> ScalarResult<ScalarRef> {
        Ok(self.value().sub(rhs.value())?.into_scalar())
    }

    /// `self * rhs`, boxed as a [`ScalarRef`] (see [`add`](Scalar::add)).
    fn mul(&self, rhs: &dyn Scalar) -> ScalarResult<ScalarRef> {
        Ok(self.value().mul(rhs.value())?.into_scalar())
    }

    /// `self / rhs`, boxed as a [`ScalarRef`] (see [`add`](Scalar::add); raises on a zero
    /// divisor).
    fn div(&self, rhs: &dyn Scalar) -> ScalarResult<ScalarRef> {
        Ok(self.value().div(rhs.value())?.into_scalar())
    }

    /// `-self`, boxed as a [`ScalarRef`] (see [`add`](Scalar::add)).
    fn neg(&self) -> ScalarResult<ScalarRef> {
        Ok(self.value().neg()?.into_scalar())
    }
}

/// Typed value access over a concrete scalar's native value type `T` (e.g. `i128` for an
/// [`IntScalar`](crate::IntScalar), [`Date`](yggdryl_core::Date) for a
/// [`DateScalar`](crate::DateScalar)).
pub trait TypedScalar<T>: Scalar {
    /// The value, or `None` when it is null / not representable as `T`.
    fn get(&self) -> Option<T>;
}

/// Wraps a tagged [`ScalarValue`] in its concrete [`Scalar`] (the right per-type struct),
/// boxed as a [`ScalarRef`].
pub fn from_value(value: ScalarValue) -> ScalarRef {
    use ScalarValue as V;
    match &value {
        V::Null(_) => Arc::new(NullScalar(value)),
        V::Boolean(_) => Arc::new(BooleanScalar(value)),
        V::Int { .. } => Arc::new(IntScalar(value)),
        V::Float { .. } => Arc::new(FloatScalar(value)),
        V::Decimal { .. } => Arc::new(DecimalScalar(value)),
        V::Utf8 { .. } => Arc::new(VarcharScalar(value)),
        V::Binary { .. } => Arc::new(BinaryScalar(value)),
        V::Json(_) => Arc::new(JsonScalar(value)),
        V::Bson(_) => Arc::new(BsonScalar(value)),
        V::Timezone(_) => Arc::new(TimezoneScalar(value)),
        V::Date { .. } => Arc::new(DateScalar(value)),
        V::Time { .. } => Arc::new(TimeScalar(value)),
        V::Timestamp { .. } => Arc::new(TimestampScalar(value)),
        V::Duration { .. } => Arc::new(DurationScalar(value)),
        V::Interval(_) => Arc::new(IntervalScalar(value)),
        V::List { .. } => Arc::new(ListScalar(value)),
        V::Struct { .. } => Arc::new(StructScalar(value)),
        V::Map { .. } => Arc::new(MapScalar(value)),
    }
}

impl ScalarValue {
    /// Wraps this value as its concrete [`Scalar`], boxed as a [`ScalarRef`].
    pub fn into_scalar(self) -> ScalarRef {
        from_value(self)
    }

    /// Reads an Arrow array cell as a boxed concrete [`Scalar`] (the trait-level
    /// companion to [`from_array`](ScalarValue::from_array)).
    pub fn scalar_at(array: &dyn Array, index: usize) -> ScalarResult<ScalarRef> {
        Ok(ScalarValue::from_array(array, index)?.into_scalar())
    }
}
