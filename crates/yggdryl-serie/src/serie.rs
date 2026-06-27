//! The [`Serie`] base trait, the typed [`TypedSerie<T>`] trait, the [`SerieRef`]
//! boxed handle, and the [`from_arrow`] / [`from_array`] factory that **redirects** an
//! Arrow array to the right concrete series.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, GenericBinaryArray, GenericStringArray};
use arrow_schema::{DataType as ADataType, IntervalUnit as AIntervalUnit, TimeUnit as ATimeUnit};
use yggdryl_schema::{DataType, Field, TypeCategory};

use crate::error::{SerieError, SerieResult};
#[allow(unused_imports)]
use crate::log_event;
use crate::primitive::{BinarySerie, BooleanSerie, PrimitiveSerie, VarcharSerie};

/// A reference-counted, type-erased column — the handle a column store and the
/// [factory](from_arrow) hand around.
pub type SerieRef = Arc<dyn Serie>;

/// The object-safe **base** of every column: its [`Field`], the backing Arrow
/// [`array`](Serie::array), the length / null bookkeeping, [`slice`](Serie::slice) and
/// downcasting via [`as_any`](Serie::as_any). Typed value access is added by
/// [`TypedSerie<T>`].
///
/// The default method bodies read everything off [`field`](Serie::field) and
/// [`array`](Serie::array), so a new backend only has to supply those two and
/// [`as_any`](Serie::as_any); concrete series override the length / null methods to
/// read their typed array directly (no `Arc` clone per call).
pub trait Serie: fmt::Debug + Send + Sync {
    /// The column's [`Field`] — its name, [`DataType`], nullability and metadata.
    fn field(&self) -> &Field;

    /// The backing Arrow array (a cheap `Arc` clone of the column's buffers).
    fn array(&self) -> ArrayRef;

    /// Downcast hook — recover the concrete series (e.g. `serie.as_any()
    /// .downcast_ref::<Int32Serie>()`).
    fn as_any(&self) -> &dyn Any;

    /// The column name (its field's name).
    fn name(&self) -> &str {
        self.field().name()
    }

    /// The column's logical [`DataType`].
    fn data_type(&self) -> &DataType {
        self.field().data_type()
    }

    /// The column's [`TypeCategory`].
    fn category(&self) -> TypeCategory {
        self.data_type().category()
    }

    /// Whether the column admits nulls (its field's nullability).
    fn is_nullable(&self) -> bool {
        self.field().is_nullable()
    }

    /// The number of values (including nulls).
    fn len(&self) -> usize {
        self.array().len()
    }

    /// Whether the column has no values.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of null values.
    fn null_count(&self) -> usize {
        self.array().null_count()
    }

    /// Whether the value at `index` is null. Out-of-bounds is treated as null.
    fn is_null(&self, index: usize) -> bool {
        index >= self.len() || self.array().is_null(index)
    }

    /// Whether the value at `index` is present (in bounds and non-null).
    fn is_valid(&self, index: usize) -> bool {
        index < self.len() && !self.is_null(index)
    }

    /// A zero-copy [`slice`](arrow_array::Array::slice) of `length` values starting at
    /// `offset`, as a new column of the same type.
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        from_arrow(self.field().clone(), self.array().slice(offset, length))
            .expect("a slice has the same type as its source")
    }
}

/// Typed value access over a concrete column's native value type `T` (e.g. `i32` for
/// an [`Int32Serie`](crate::Int32Serie), `String` for a [`VarcharSerie`]).
pub trait TypedSerie<T>: Serie {
    /// The value at `index`, or `None` when it is null or out of bounds.
    fn get(&self, index: usize) -> Option<T>;

    /// The value at `index`, panicking when it is null or out of bounds — the
    /// unchecked companion to [`get`](TypedSerie::get).
    fn value(&self, index: usize) -> T {
        self.get(index).expect("non-null value at a valid index")
    }

    /// An iterator over every value as `Option<T>` (null → `None`).
    fn iter(&self) -> Box<dyn Iterator<Item = Option<T>> + '_>;

    /// Collects every value into a `Vec<Option<T>>`.
    fn to_vec(&self) -> Vec<Option<T>> {
        self.iter().collect()
    }
}

/// Builds the right concrete [`Serie`] for `array`, named and typed by `field`.
///
/// The `field`'s [`DataType`] must map to the array's Arrow type (checked up front);
/// the concrete backend is then chosen from the Arrow type. Returns
/// [`SerieError::Unsupported`] for types without a backend yet (the nested and view
/// layouts).
///
/// ```
/// use yggdryl_serie::{from_arrow, Field, DataType};
/// use yggdryl_serie::arrow_array::{ArrayRef, Float64Array};
/// use std::sync::Arc;
///
/// let array: ArrayRef = Arc::new(Float64Array::from(vec![1.0, 2.0]));
/// let serie = from_arrow(Field::new("x", DataType::float(64), true), array).unwrap();
/// assert_eq!(serie.len(), 2);
/// ```
pub fn from_arrow(field: Field, array: ArrayRef) -> SerieResult<SerieRef> {
    log_event!(
        trace,
        "Serie::from_arrow {} {}",
        field.name(),
        array.data_type()
    );
    let expected = field.data_type().to_arrow()?;
    if &expected != array.data_type() {
        return Err(SerieError::TypeMismatch {
            expected: expected.to_string(),
            found: array.data_type().to_string(),
        });
    }
    dispatch(field, array)
}

/// Builds a [`Serie`] from `array`, deriving the [`Field`] (nullable) from the array's
/// Arrow type and naming it `name` — the quick path when there is no explicit field.
///
/// ```
/// use yggdryl_serie::from_array;
/// use yggdryl_serie::arrow_array::{ArrayRef, StringArray};
/// use std::sync::Arc;
///
/// let array: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "c"]));
/// let serie = from_array("name", array).unwrap();
/// assert_eq!(serie.data_type(), &yggdryl_serie::DataType::varchar());
/// ```
pub fn from_array(name: impl Into<String>, array: ArrayRef) -> SerieResult<SerieRef> {
    let afield = arrow_schema::Field::new(name, array.data_type().clone(), true);
    from_arrow(Field::from_arrow(&afield), array)
}

/// Picks the concrete series for an array whose type already matches `field`.
fn dispatch(field: Field, array: ArrayRef) -> SerieResult<SerieRef> {
    use arrow_array::types::*;

    /// Downcasts `array` to its `PrimitiveArray<$ty>` and boxes a `PrimitiveSerie`.
    macro_rules! prim {
        ($ty:ty) => {{
            let typed = array
                .as_any()
                .downcast_ref::<arrow_array::PrimitiveArray<$ty>>()
                .expect("data type matched the primitive array")
                .clone();
            Arc::new(PrimitiveSerie::<$ty>::from_parts(field, typed)) as SerieRef
        }};
    }

    let serie = match array.data_type() {
        // integers
        ADataType::Int8 => prim!(Int8Type),
        ADataType::Int16 => prim!(Int16Type),
        ADataType::Int32 => prim!(Int32Type),
        ADataType::Int64 => prim!(Int64Type),
        ADataType::UInt8 => prim!(UInt8Type),
        ADataType::UInt16 => prim!(UInt16Type),
        ADataType::UInt32 => prim!(UInt32Type),
        ADataType::UInt64 => prim!(UInt64Type),
        // floats
        ADataType::Float16 => prim!(Float16Type),
        ADataType::Float32 => prim!(Float32Type),
        ADataType::Float64 => prim!(Float64Type),
        // decimals
        ADataType::Decimal128(_, _) => prim!(Decimal128Type),
        ADataType::Decimal256(_, _) => prim!(Decimal256Type),
        // temporal
        ADataType::Date32 => prim!(Date32Type),
        ADataType::Date64 => prim!(Date64Type),
        ADataType::Time32(ATimeUnit::Second) => prim!(Time32SecondType),
        ADataType::Time32(ATimeUnit::Millisecond) => prim!(Time32MillisecondType),
        ADataType::Time64(ATimeUnit::Microsecond) => prim!(Time64MicrosecondType),
        ADataType::Time64(ATimeUnit::Nanosecond) => prim!(Time64NanosecondType),
        ADataType::Timestamp(ATimeUnit::Second, _) => prim!(TimestampSecondType),
        ADataType::Timestamp(ATimeUnit::Millisecond, _) => prim!(TimestampMillisecondType),
        ADataType::Timestamp(ATimeUnit::Microsecond, _) => prim!(TimestampMicrosecondType),
        ADataType::Timestamp(ATimeUnit::Nanosecond, _) => prim!(TimestampNanosecondType),
        ADataType::Duration(ATimeUnit::Second) => prim!(DurationSecondType),
        ADataType::Duration(ATimeUnit::Millisecond) => prim!(DurationMillisecondType),
        ADataType::Duration(ATimeUnit::Microsecond) => prim!(DurationMicrosecondType),
        ADataType::Duration(ATimeUnit::Nanosecond) => prim!(DurationNanosecondType),
        ADataType::Interval(AIntervalUnit::YearMonth) => prim!(IntervalYearMonthType),
        ADataType::Interval(AIntervalUnit::DayTime) => prim!(IntervalDayTimeType),
        ADataType::Interval(AIntervalUnit::MonthDayNano) => prim!(IntervalMonthDayNanoType),
        // boolean
        ADataType::Boolean => {
            let typed = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("data type matched the boolean array")
                .clone();
            Arc::new(BooleanSerie::from_parts(field, typed)) as SerieRef
        }
        // strings
        ADataType::Utf8 => {
            let typed = array
                .as_any()
                .downcast_ref::<GenericStringArray<i32>>()
                .expect("data type matched the string array")
                .clone();
            Arc::new(VarcharSerie::<i32>::from_parts(field, typed)) as SerieRef
        }
        ADataType::LargeUtf8 => {
            let typed = array
                .as_any()
                .downcast_ref::<GenericStringArray<i64>>()
                .expect("data type matched the large string array")
                .clone();
            Arc::new(VarcharSerie::<i64>::from_parts(field, typed)) as SerieRef
        }
        // binary
        ADataType::Binary => {
            let typed = array
                .as_any()
                .downcast_ref::<GenericBinaryArray<i32>>()
                .expect("data type matched the binary array")
                .clone();
            Arc::new(BinarySerie::<i32>::from_parts(field, typed)) as SerieRef
        }
        ADataType::LargeBinary => {
            let typed = array
                .as_any()
                .downcast_ref::<GenericBinaryArray<i64>>()
                .expect("data type matched the large binary array")
                .clone();
            Arc::new(BinarySerie::<i64>::from_parts(field, typed)) as SerieRef
        }
        other => {
            return Err(SerieError::Unsupported(format!(
                "no serie backend for arrow type '{other}' yet; nested, view, dictionary \
                 and run-end types are not implemented"
            )))
        }
    };
    Ok(serie)
}
