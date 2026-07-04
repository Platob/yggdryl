//! The statically-typed [`TypedStructSerie`] scalar: a serie of struct rows.

use std::marker::PhantomData;

use crate::{AnySerie, Scalar, TypedScalar};
use arrow_array::ArrayRef;
// The serie dtype trait is imported anonymously (its `item_field()` accessor is all
// we need) so it does not clash with the scalar types.
use yggdryl_dtype::Serie as _;
use yggdryl_dtype::{DataError, DataType, StructType, TypedSerieType};

/// A single, possibly-null `list<struct>` value: *our array* of struct rows — a
/// sequence whose elements are all the runtime [`StructType`] `item_type`, read back
/// as the row scalar `S`.
///
/// It is the struct counterpart of the generic [`TypedSerie<D, S>`](crate::TypedSerie):
/// the generic one cannot hold a struct element (a [`StructType`] has no compile-time
/// default shape), so this specialized type carries the shape at runtime. The rows
/// live in the crate's own [`AnySerie`] (the struct column, held zero-copy), so
/// [`to_arrow_scalar`](Scalar::to_arrow_scalar) / [`from_arrow`](Scalar::from_arrow)
/// are reference-count bumps; building from row scalars pays the assembly once, at
/// construction. The *row accessors* read a row back:
/// [`get_scalar_at`](TypedStructSerie::get_scalar_at) redirects one element through
/// `S::from_arrow` (an `S` of [`RecordScalar`](crate::RecordScalar) for the row atom,
/// or a [`StructScalar`](crate::StructScalar) for the column form), and the
/// [`NestedSerie`](crate::NestedSerie) children are the struct's *field columns*
/// (each field projected across every row). [`erase`](TypedStructSerie::erase) drops
/// the static row type to a dynamic [`StructSerie`](crate::StructSerie).
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
/// use yggdryl_scalar::{AnyScalar, Int64Scalar, NestedSerie, RecordScalar, Scalar, TypedStructSerie};
///
/// let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
/// ]));
/// let row = |x| RecordScalar::new(point.clone(), vec![AnyScalar::from(Int64Scalar::new(x))]).unwrap();
///
/// let points = TypedStructSerie::new(point.clone(), vec![row(1), row(2)]);
/// assert_eq!(points.len(), 2);
/// assert_eq!(points.data_type().name(), "list");
/// assert_eq!(points.get_scalar_at(1), Some(row(2)));
/// assert_eq!(points.get_scalar_at(2), None); // out of bounds
///
/// // The children are the struct's field columns — "x" across both rows.
/// assert_eq!(points.child_serie_count(), 1);
/// assert_eq!(points.child_serie_by("x").unwrap().len(), 2);
///
/// // The Arrow round trip shares the buffers — no element is copied.
/// let arrow = points.to_arrow_scalar();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(TypedStructSerie::from_arrow(arrow.as_ref()).unwrap(), points);
///
/// let missing: TypedStructSerie<RecordScalar> = TypedStructSerie::null(point);
/// assert!(missing.is_null());
/// ```
#[derive(Debug)]
pub struct TypedStructSerie<S> {
    data_type: TypedSerieType<StructType>,
    values: Option<AnySerie>,
    row: PhantomData<S>,
}

impl<S: Scalar<DataType = StructType>> TypedStructSerie<S> {
    /// A serie holding the struct `rows` of `item_type`, assembled once into one
    /// Arrow struct column (an empty sequence is the empty serie, not null). Each row
    /// must carry `item_type`.
    pub fn new(item_type: StructType, rows: Vec<S>) -> Self {
        let elements = crate::scalar::concat_scalar_arrays(
            rows.iter().map(Scalar::to_arrow_scalar).collect(),
            || DataType::to_arrow(&item_type),
        );
        Self {
            data_type: TypedSerieType::new(item_type),
            values: Some(AnySerie::from_arrow(elements)),
            row: PhantomData,
        }
    }

    /// The null serie scalar of `item_type`.
    pub fn null(item_type: StructType) -> Self {
        Self {
            data_type: TypedSerieType::new(item_type),
            values: None,
            row: PhantomData,
        }
    }

    /// Drop the static row type, returning the dynamic [`StructSerie`](crate::StructSerie)
    /// over the same shared struct column (a reference-count bump, not a copy).
    pub fn erase(&self) -> crate::StructSerie {
        crate::StructSerie::from_parts(self.data_type.erase(), self.values.clone())
    }

    /// The number of rows, `0` when null or empty ([`is_null`](Scalar::is_null)
    /// distinguishes the two).
    pub fn len(&self) -> usize {
        self.values.as_ref().map_or(0, AnySerie::len)
    }

    /// Whether the serie holds no rows (also `true` when null).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The row at `index` as the row scalar `S`, or `None` when the serie is null or
    /// `index` is out of bounds.
    pub fn get_scalar_at(&self, index: usize) -> Option<S> {
        let values = self.values.as_ref()?;
        if index >= values.len() {
            return None;
        }
        let element = values.to_arrow().slice(index, 1);
        S::from_arrow(element.as_ref()).ok()
    }

    /// An iterator over the rows as [`RecordScalar`](crate::RecordScalar) row atoms,
    /// in order (a null row is the record's null; a null serie yields nothing) —
    /// independent of the row scalar type `S`, the rows always read back as records.
    /// The struct column is reconstituted **once**, and each step slices one row from
    /// it — linear, unlike a [`get_scalar_at`](TypedStructSerie::get_scalar_at) loop.
    /// The iterator owns a reference-counted view of the column, borrowing nothing,
    /// and is [`ExactSizeIterator`] / [`DoubleEndedIterator`].
    ///
    /// ```
    /// use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema};
    /// use yggdryl_scalar::{AnyScalar, Int64Scalar, RecordScalar, Scalar, TypedStructSerie};
    ///
    /// let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
    ///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
    /// ]));
    /// let row = |x| RecordScalar::new(point.clone(), vec![AnyScalar::from(Int64Scalar::new(x))]).unwrap();
    ///
    /// let points = TypedStructSerie::new(point.clone(), vec![row(1), row(2)]);
    /// let rows: Vec<RecordScalar> = points.iter_records().collect();
    /// assert_eq!(rows, vec![row(1), row(2)]);
    /// assert_eq!(points.iter_records().len(), 2); // exact size, no walk
    /// ```
    pub fn iter_records(
        &self,
    ) -> impl ExactSizeIterator<Item = crate::RecordScalar> + DoubleEndedIterator {
        super::struct_serie::iter_records(self.values.as_ref(), self.len())
    }
}

impl<S: Scalar<DataType = StructType>> Clone for TypedStructSerie<S> {
    // Cloning bumps the struct column's reference count — no row is copied.
    fn clone(&self) -> Self {
        Self {
            data_type: self.data_type.clone(),
            values: self.values.clone(),
            row: PhantomData,
        }
    }
}

impl<S> PartialEq for TypedStructSerie<S> {
    // The struct columns compare by value through `AnySerie` equality, so two series
    // are equal when their rows are; null is distinct from every present serie.
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl<S> Eq for TypedStructSerie<S> {}

impl<S: Scalar<DataType = StructType>> Scalar for TypedStructSerie<S> {
    type DataType = TypedSerieType<StructType>;
    type Value = AnySerie;

    fn data_type(&self) -> &TypedSerieType<StructType> {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.values.is_none()
    }

    fn value(&self) -> Option<&AnySerie> {
        self.values.as_ref()
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let Some(values) = &self.values else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The struct column is reconstituted into the one-element list — a
        // reference-count bump, not a copy.
        let array = arrow_array::ListArray::try_new(
            self.data_type.item_field(),
            arrow_buffer::OffsetBuffer::from_lengths([values.len()]),
            values.to_arrow(),
            None,
        )
        .expect("a one-element serie of the struct item is valid");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the serie-of-struct layout; the rows are shared
        // zero-copy as the struct column.
        let data_type =
            TypedSerieType::<StructType>::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a serie data type is a serie array");
        let values = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(AnySerie::from_arrow(array.value(0)))
        };
        Ok(Self {
            data_type,
            values,
            row: PhantomData,
        })
    }

    fn as_serie(&self) -> Result<crate::Serie, DataError> {
        // The generic dynamic list view (element type erased to struct); the
        // struct-aware form is `erase()` → `StructSerie`.
        Ok(crate::Serie::from_parts(
            self.data_type.erase(),
            self.values.clone(),
        ))
    }
}

impl<S: Scalar<DataType = StructType>> crate::NestedSerie for TypedStructSerie<S> {
    fn child_serie_count(&self) -> usize {
        super::struct_serie::item_fields(&self.data_type).len()
    }

    fn child_serie_at(&self, index: usize) -> Option<AnySerie> {
        super::struct_serie::project_field(self.values.as_ref(), index)
    }

    fn child_serie_name_at(&self, index: usize) -> Option<String> {
        super::struct_serie::item_fields(&self.data_type)
            .get(index)
            .map(|field| field.name().to_string())
    }
}

impl<S: Scalar<DataType = StructType>>
    TypedScalar<TypedSerieType<StructType>, AnySerie, arrow_array::ListArray>
    for TypedStructSerie<S>
{
}
