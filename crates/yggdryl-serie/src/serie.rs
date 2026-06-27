//! The [`Serie`] base trait, the typed [`TypedSerie<T>`] trait, the [`SerieRef`]
//! boxed handle, and the [`from_arrow`] / [`from_array`] factory that **redirects** an
//! Arrow array to the right concrete series.

use std::any::Any;
use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, GenericBinaryArray, GenericStringArray};
use arrow_schema::{DataType as ADataType, IntervalUnit as AIntervalUnit};
use yggdryl_schema::{DataType, Field, TypeCategory};

use crate::display::{render as render_display, DisplayOptions};
use crate::error::{SerieError, SerieResult};
#[allow(unused_imports)]
use crate::log_event;
use crate::nested::{ListSerie, MapSerie, StructSerie};
use crate::primitive::{BinarySerie, BooleanSerie, PrimitiveSerie, VarcharSerie};
use crate::scalar::{scalar_at, Scalar};
use crate::temporal::{DatetimeSerie, DurationSerie, TimeSerie};

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

    /// The backing Arrow array. For a materialised column this is a cheap shallow
    /// clone that **shares** the column's buffers (no data copy); a *lazy* column
    /// computes the array on demand.
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

    /// The column's [`DataType`] — the convenient short alias of
    /// [`data_type`](Serie::data_type), reflecting the held [`field`](Serie::field).
    fn dtype(&self) -> &DataType {
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

    /// One metadata value by key, reflecting the held [`field`](Serie::field)'s
    /// metadata — the safe, narrow accessor (the whole map stays encapsulated in the
    /// field).
    fn get_metadata(&self, key: &str) -> Option<&str> {
        self.field().get_metadata(key)
    }

    /// The number of values (including nulls).
    fn len(&self) -> usize {
        self.array().len()
    }

    /// The number of rows — the row-oriented name for [`len`](Serie::len), the
    /// vocabulary a frame uses across all its columns.
    fn num_rows(&self) -> usize {
        self.len()
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

    /// The serie this one was **derived from** (its slice/child source), or `None` for
    /// a root column. Navigational only — a [child](crate::child) slice records its parent so
    /// the graph can be walked upward; [`materialize`](Serie::materialize) detaches it.
    fn parent(&self) -> Option<&SerieRef> {
        None
    }

    /// Whether the column's values are **fully resident in memory** (the normal case).
    /// A *lazy* / computed column (a [range](crate::RangeSerie), a
    /// [date range](crate::DateRangeSerie)) reports `false` — its values are produced
    /// on demand until [`materialize`](Serie::materialize)d.
    fn is_materialized(&self) -> bool {
        true
    }

    /// A fully-materialised, **independent** copy of this column: a lazy column is
    /// computed into a real array, and the parent/graph link is dropped. The default
    /// realises [`array`](Serie::array) into a standalone series.
    fn materialize(&self) -> SerieRef {
        from_arrow(self.field().clone(), self.array()).expect("a serie's array matches its field")
    }

    /// The value at `index` as a type-erased [`Scalar`] (`Null` for a null cell or an
    /// out-of-bounds index). Lazy columns override this to compute the value directly.
    fn value_at(&self, index: usize) -> Scalar {
        if self.is_null(index) {
            Scalar::Null
        } else {
            scalar_at(&self.array(), index)
        }
    }

    /// A zero-copy [`slice`](arrow_array::Array::slice) of `length` values starting at
    /// `offset`, as a new column of the same type. (Use [`child`](crate::child) to keep a link
    /// back to this serie.)
    fn slice(&self, offset: usize, length: usize) -> SerieRef {
        from_arrow(self.field().clone(), self.array().slice(offset, length))
            .expect("a slice has the same type as its source")
    }

    /// A zero-copy slice addressed by a half-open row `range` — the by-range value
    /// accessor companion to [`value_at`](Serie::value_at).
    fn slice_range(&self, range: Range<usize>) -> SerieRef {
        self.slice(range.start, range.len())
    }

    /// A readable, parametrised string view of the column (see [`DisplayOptions`]) —
    /// the building block for a future `Frame`'s table rendering.
    fn display(&self, opts: &DisplayOptions) -> String {
        render_display(self, opts)
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
    // The field is derived *from* the array, so it is consistent by construction —
    // dispatch directly, skipping the explicit-field equality check (which would trip
    // on the schema's documented Arrow normalisations, e.g. a map's `key`/`value`
    // entry-field names vs Arrow's `keys`/`values`).
    let afield = arrow_schema::Field::new(name, array.data_type().clone(), true);
    dispatch(Field::from_arrow(&afield), array)
}

/// Picks the concrete series for an array whose type already matches `field`. Crate-
/// internal: the recursive nested builders call it directly (the array is the source of
/// truth, so the explicit-field check is skipped).
pub(crate) fn dispatch(field: Field, array: ArrayRef) -> SerieResult<SerieRef> {
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

    // Clone the data type so the Timestamp arm can move `array` into the
    // DatetimeSerie (the match scrutinee then borrows the clone, not `array`).
    let data_type = array.data_type().clone();
    let serie = match &data_type {
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
        // temporal — timestamps / times / durations unify into their unit-aware series
        ADataType::Timestamp(_, _) => Arc::new(DatetimeSerie::from_parts(field, array)) as SerieRef,
        ADataType::Time32(_) | ADataType::Time64(_) => {
            Arc::new(TimeSerie::from_parts(field, array)) as SerieRef
        }
        ADataType::Duration(_) => Arc::new(DurationSerie::from_parts(field, array)) as SerieRef,
        ADataType::Date32 => prim!(Date32Type),
        ADataType::Date64 => prim!(Date64Type),
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
        // nested — children build recursively through this same factory
        ADataType::Struct(_) => Arc::new(StructSerie::from_parts(field, array)?) as SerieRef,
        ADataType::List(_) => Arc::new(ListSerie::<i32>::from_parts(field, array)?) as SerieRef,
        ADataType::LargeList(_) => {
            Arc::new(ListSerie::<i64>::from_parts(field, array)?) as SerieRef
        }
        ADataType::Map(_, _) => Arc::new(MapSerie::from_parts(field, array)?) as SerieRef,
        other => {
            return Err(SerieError::Unsupported(format!(
                "no serie backend for arrow type '{other}' yet; nested, view, dictionary \
                 and run-end types are not implemented"
            )))
        }
    };
    Ok(serie)
}
