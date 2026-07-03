//! The [`StructScalar`] scalar of the [`StructType`](yggdryl_dtype::StructType) data
//! type.

use crate::{Scalar, TypedScalar};
use arrow_array::ArrayRef;
use yggdryl_dtype::{DataError, DataType, Struct, StructType};

/// A single, possibly-null `struct` value: one row, held as one one-element Arrow
/// column per child field.
///
/// Like its data type ([`StructType`](yggdryl_dtype::StructType)), it is dynamic —
/// the children are only known at runtime — so its [`Value`](Scalar::Value) is the
/// borrowed slice of column [`ArrayRef`]s (each of length one), and construction
/// validates the columns against the declared fields with actionable errors. Being
/// dynamic, it has no [`ScalarFactory`](crate::ScalarFactory).
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
/// use yggdryl_scalar::{arrow_array, Scalar, StructScalar};
///
/// let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
///     arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
/// ]));
///
/// let row = StructScalar::new(
///     point.clone(),
///     vec![
///         std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([1])),
///         std::sync::Arc::new(arrow_array::Int64Array::from_iter_values([2])),
///     ],
/// )
/// .unwrap();
/// assert!(!row.is_null());
/// assert_eq!(row.value().map(<[_]>::len), Some(2));
///
/// // The Arrow round trip preserves the row.
/// let arrow = row.to_arrow();
/// assert_eq!(arrow.len(), 1);
/// assert_eq!(StructScalar::from_arrow(arrow.as_ref()).unwrap(), row);
///
/// assert!(StructScalar::null(point).is_null());
/// ```
#[derive(Debug, Clone)]
pub struct StructScalar {
    data_type: StructType,
    columns: Option<Vec<ArrayRef>>,
}

impl StructScalar {
    /// A scalar holding one row of `data_type`: one one-element column per child
    /// field, in field order. A column count, length or type mismatch errors with
    /// an actionable [`DataError`].
    pub fn new(data_type: StructType, columns: Vec<ArrayRef>) -> Result<Self, DataError> {
        let fields = Struct::fields(&data_type);
        if columns.len() != fields.len() {
            return Err(DataError::IncompatibleArrowType {
                expected: format!("{} column(s), one per child field", fields.len()),
                got: format!("{} column(s)", columns.len()),
            });
        }
        for (field, column) in fields.iter().zip(&columns) {
            let length = arrow_array::Array::len(column.as_ref());
            if length != 1 {
                return Err(DataError::InvalidScalarLength { got: length });
            }
            if column.data_type() != field.data_type() {
                return Err(DataError::IncompatibleArrowType {
                    expected: field.data_type().to_string(),
                    got: column.data_type().to_string(),
                });
            }
            if !field.is_nullable() && arrow_array::Array::logical_null_count(column.as_ref()) > 0 {
                return Err(DataError::IncompatibleArrowType {
                    expected: format!("a non-null value for the non-nullable \"{}\"", field.name()),
                    got: "a null".to_string(),
                });
            }
        }
        Ok(Self {
            data_type,
            columns: Some(columns),
        })
    }

    /// The null struct scalar of `data_type`.
    pub fn null(data_type: StructType) -> Self {
        Self {
            data_type,
            columns: None,
        }
    }
}

impl PartialEq for StructScalar {
    // Column `ArrayRef`s compare by value through the `Array` `PartialEq` on their
    // data, so two rows are equal when their types and values are.
    fn eq(&self, other: &Self) -> bool {
        self.data_type == other.data_type
            && match (&self.columns, &other.columns) {
                (None, None) => true,
                (Some(left), Some(right)) => {
                    left.len() == right.len()
                        && left
                            .iter()
                            .zip(right)
                            .all(|(a, b)| a.as_ref() == b.as_ref())
                }
                _ => false,
            }
    }
}

impl Scalar<StructType> for StructScalar {
    type Value = [ArrayRef];

    fn data_type(&self) -> &StructType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.columns.is_none()
    }

    fn value(&self) -> Option<&[ArrayRef]> {
        self.columns.as_deref()
    }

    fn to_arrow(&self) -> ArrayRef {
        let fields = Struct::fields(&self.data_type);
        let Some(columns) = &self.columns else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        let array =
            arrow_array::StructArray::try_new_with_length(fields.clone(), columns.clone(), None, 1)
                .expect("validated one-element columns assemble into a one-row struct");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        let data_type = DataType::from_arrow(arrow_array::Array::data_type(array))?;
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
                    .map(|column| arrow_array::Array::slice(column.as_ref(), 0, 1))
                    .collect(),
            )
        };
        Ok(Self { data_type, columns })
    }
}

impl TypedScalar<StructType, [ArrayRef]> for StructScalar {}
