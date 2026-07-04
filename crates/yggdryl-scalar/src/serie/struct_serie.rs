//! The dynamic [`StructSerie`] scalar: a serie of struct rows, row type erased.

use crate::{AnySerie, RecordScalar, Scalar};
use arrow_array::ArrayRef;
// The serie dtype trait is imported anonymously (its `item_field()` accessor is all
// we need) so it does not clash with the scalar types.
use yggdryl_dtype::Serie as _;
use yggdryl_dtype::{DataError, DataType, SerieType};

/// The struct field layout of a serie whose item is a `struct` — its ordered, named
/// fields, or the empty list for a non-struct item. The shared reader behind the
/// struct series' [`NestedSerie`](crate::NestedSerie) field-column children.
pub(crate) fn item_fields(serie_type: &impl yggdryl_dtype::Serie) -> arrow_schema::Fields {
    match serie_type.item_field().data_type() {
        arrow_schema::DataType::Struct(fields) => fields.clone(),
        _ => arrow_schema::Fields::empty(),
    }
}

/// The `index`-th field column projected out of the struct column `values` (decomposed
/// into its own serie), or `None` when the serie is null, the item is not a struct, or
/// `index` is out of bounds — the shared projector behind the struct series'
/// field-column children.
pub(crate) fn project_field(values: Option<&AnySerie>, index: usize) -> Option<AnySerie> {
    let column = values?.to_arrow();
    let entries = column.as_any().downcast_ref::<arrow_array::StructArray>()?;
    entries
        .columns()
        .get(index)
        .map(|field_column| AnySerie::from_arrow(field_column.clone()))
}

/// A single, possibly-null `list<struct>` value with its row type erased — *our
/// array* of struct rows, holding them as the crate's own [`AnySerie`] struct column,
/// carrying a dynamic [`SerieType`](yggdryl_dtype::SerieType).
///
/// It is the untyped base of the statically-typed
/// [`TypedStructSerie<S>`](crate::TypedStructSerie): it implements only the base
/// [`Scalar`] surface plus [`len`](StructSerie::len) / [`is_empty`](StructSerie::is_empty),
/// the [`get_row`](StructSerie::get_row) dynamic row accessor, and the
/// [`NestedSerie`](crate::NestedSerie) field-column children (each struct field
/// projected across every row), since the row scalar type is erased — the statically
/// typed row accessor lives on `TypedStructSerie<S>`, which
/// [`erase`](crate::TypedStructSerie::erase)s back to this type. The Arrow forms are
/// reconstituted on demand and decomposed on the way in, reference-count bumps only.
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
/// // A dynamic struct serie is reached by erasing a typed one, or from Arrow.
/// let points = TypedStructSerie::new(point.clone(), vec![row(1), row(2)]).erase();
/// assert!(!points.is_null());
/// assert_eq!(points.len(), 2);
/// assert_eq!(points.data_type().name(), "list");
/// assert_eq!(points.get_row(1), Some(row(2)));
/// assert_eq!(points.child_serie_by("x").unwrap().len(), 2); // the "x" field column
/// assert_eq!(
///     yggdryl_scalar::StructSerie::from_arrow(points.to_arrow_scalar().as_ref()).unwrap(),
///     points
/// );
/// ```
#[derive(Debug, Clone)]
pub struct StructSerie {
    data_type: SerieType,
    values: Option<AnySerie>,
}

impl StructSerie {
    /// A dynamic struct serie over an already-built struct column `values` (shared
    /// zero-copy) of the given dynamic `data_type`, or the null serie for `None`.
    pub(crate) fn from_parts(data_type: SerieType, values: Option<AnySerie>) -> Self {
        Self { data_type, values }
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

    /// The row at `index` as a [`RecordScalar`](crate::RecordScalar) row atom, or
    /// `None` when the serie is null or `index` is out of bounds.
    pub fn get_row(&self, index: usize) -> Option<RecordScalar> {
        let values = self.values.as_ref()?;
        if index >= values.len() {
            return None;
        }
        let element = values.to_arrow().slice(index, 1);
        RecordScalar::from_arrow(element.as_ref()).ok()
    }
}

impl PartialEq for StructSerie {
    // Compared logically, like Arrow arrays: two series are equal when their struct
    // columns are; null is distinct from every present serie.
    fn eq(&self, other: &Self) -> bool {
        self.values == other.values
    }
}

impl Eq for StructSerie {}

impl crate::NestedSerie for StructSerie {
    fn child_serie_count(&self) -> usize {
        item_fields(&self.data_type).len()
    }

    fn child_serie_at(&self, index: usize) -> Option<AnySerie> {
        project_field(self.values.as_ref(), index)
    }

    fn child_serie_name_at(&self, index: usize) -> Option<String> {
        item_fields(&self.data_type)
            .get(index)
            .map(|field| field.name().to_string())
    }
}

impl Scalar for StructSerie {
    type DataType = SerieType;
    type Value = AnySerie;

    fn data_type(&self) -> &SerieType {
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
        // The data type validates the layout; the rows are decomposed into the
        // crate's own struct column, sharing the buffers zero-copy.
        let data_type = SerieType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .expect("a value with a serie data type is a serie array");
        let values = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(AnySerie::from_arrow(array.value(0)))
        };
        Ok(Self { data_type, values })
    }

    fn as_serie(&self) -> Result<crate::Serie, DataError> {
        // The generic dynamic list view over the same struct column.
        Ok(crate::Serie::from_parts(
            self.data_type.clone(),
            self.values.clone(),
        ))
    }
}
