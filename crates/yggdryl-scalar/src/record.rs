//! The [`RecordScalar`] scalar: the generic struct-row accessor.

use crate::{AnySerie, Scalar, TypedScalar};
use arrow_array::ArrayRef;
use yggdryl_dtype::{DataError, DataType, Struct, StructType};

/// A single, possibly-null `struct` row with **generic per-child scalar access**: an
/// array of one-element child series sharing one [`StructType`].
///
/// Arrow models a scalar as a one-element array, so each child scalar is held as a
/// one-element [`AnySerie`] column — the crate's own zero-copy holder — and the
/// shared [`StructType`] names the fields. Where [`StructScalar`](crate::StructScalar)
/// is the plain row value, `RecordScalar` adds the accessor surface:
/// [`scalar_at`](RecordScalar::scalar_at) / [`scalar_by`](RecordScalar::scalar_by)
/// hand back a child's one-element serie (rehydrate it with the matching scalar's
/// `from_arrow`), and the [`NestedSerie`](crate::NestedSerie) child access mirrors
/// it. The Arrow forms are reconstituted on demand, reference-count bumps only.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
/// use yggdryl_scalar::{Int64Scalar, NestedSerie, RecordScalar, Scalar};
///
/// let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
///     arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
/// ]));
/// let row = RecordScalar::new(
///     point,
///     vec![
///         Int64Scalar::new(1).to_arrow_scalar().into(),
///         Int64Scalar::new(2).to_arrow_scalar().into(),
///     ],
/// )
/// .unwrap();
/// assert_eq!(row.data_type().name(), "struct");
/// assert_eq!(row.child_serie_count(), 2);
///
/// // Generic child access, by position and by field name.
/// let y = row.scalar_by("y").unwrap();
/// assert_eq!(Int64Scalar::from_arrow(y.to_arrow().as_ref()).unwrap(), Int64Scalar::new(2));
///
/// // The Arrow round trip preserves the row.
/// assert_eq!(RecordScalar::from_arrow(row.to_arrow_scalar().as_ref()).unwrap(), row);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RecordScalar {
    data_type: StructType,
    columns: Option<Vec<AnySerie>>,
}

impl Eq for RecordScalar {}

impl RecordScalar {
    /// A record of `data_type` holding the row `columns` — one one-element serie
    /// per field, shared zero-copy. A column count differing from the field count,
    /// a column that is not one element long, or a column of a different Arrow
    /// type than its field errors with an actionable [`DataError`].
    pub fn new(data_type: StructType, columns: Vec<AnySerie>) -> Result<Self, DataError> {
        let fields = data_type.fields();
        if columns.len() != fields.len() {
            return Err(DataError::IncompatibleArrowType {
                expected: format!("{} column(s), one per struct field", fields.len()),
                got: format!("{} column(s)", columns.len()),
            });
        }
        for (column, field) in columns.iter().zip(fields.iter()) {
            if column.len() != 1 {
                return Err(DataError::InvalidScalarLength { got: column.len() });
            }
            if &column.data_type() != field.data_type() {
                return Err(DataError::IncompatibleArrowType {
                    expected: format!(
                        "a {} column for field \"{}\"",
                        field.data_type(),
                        field.name()
                    ),
                    got: column.data_type().to_string(),
                });
            }
        }
        Ok(Self {
            data_type,
            columns: Some(columns),
        })
    }

    /// The null record of `data_type`.
    pub fn null(data_type: StructType) -> Self {
        Self {
            data_type,
            columns: None,
        }
    }

    /// A record over already-validated one-element `columns` (the struct scalars'
    /// own storage), shared zero-copy — no re-validation.
    pub(crate) fn from_parts(data_type: StructType, columns: Option<Vec<AnySerie>>) -> Self {
        Self { data_type, columns }
    }

    /// The child scalar at `index` as its one-element serie (a zero-copy handle —
    /// rehydrate it with the matching scalar's `from_arrow`), or `None` when the
    /// record is null or `index` is out of bounds.
    pub fn scalar_at(&self, index: usize) -> Option<AnySerie> {
        self.columns.as_ref()?.get(index).cloned()
    }

    /// The child scalar of the field named `name` as its one-element serie, or
    /// `None` when the record is null or no field carries the name.
    pub fn scalar_by(&self, name: &str) -> Option<AnySerie> {
        let index = self
            .data_type
            .fields()
            .iter()
            .position(|field| field.name() == name)?;
        self.scalar_at(index)
    }
}

impl From<crate::StructScalar> for RecordScalar {
    /// The same row with the generic accessor surface — shared zero-copy.
    fn from(scalar: crate::StructScalar) -> Self {
        scalar.as_struct().expect("a struct scalar is a record")
    }
}

impl crate::NestedSerie for RecordScalar {
    fn child_serie_count(&self) -> usize {
        self.data_type.fields().len()
    }

    fn child_serie_at(&self, index: usize) -> Option<AnySerie> {
        self.scalar_at(index)
    }

    fn child_serie_name_at(&self, index: usize) -> Option<String> {
        self.data_type
            .fields()
            .get(index)
            .map(|field| field.name().to_string())
    }
}

impl Scalar for RecordScalar {
    type DataType = StructType;
    type Value = [AnySerie];

    fn data_type(&self) -> &StructType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.columns.is_none()
    }

    fn value(&self) -> Option<&[AnySerie]> {
        self.columns.as_deref()
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let fields = Struct::fields(&self.data_type);
        let Some(columns) = &self.columns else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // The columns are reconstituted into the one-element struct row —
        // reference-count bumps, not copies.
        let array = arrow_array::StructArray::try_new_with_length(
            fields.clone(),
            columns.iter().map(AnySerie::to_arrow).collect(),
            None,
            1,
        )
        .expect("one-element columns of the declared fields assemble into the row");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout; every column is decomposed into the
        // crate's own serie, sharing the buffers zero-copy.
        let data_type = StructType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .expect("a value with a struct data type is a struct array");
        let columns = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(
                array
                    .columns()
                    .iter()
                    .map(|column| AnySerie::from_arrow(column.clone()))
                    .collect(),
            )
        };
        Ok(Self { data_type, columns })
    }

    fn as_struct(&self) -> Result<RecordScalar, DataError> {
        Ok(self.clone())
    }
}

impl TypedScalar<StructType, [AnySerie], arrow_array::StructArray> for RecordScalar {}
