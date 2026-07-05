//! The [`RecordScalar`] scalar: the struct row as an array of atomic scalars.

use crate::{AnyScalar, AnySerie, Scalar, TypedScalar};
use arrow_array::ArrayRef;
use yggdryl_dtype::{DataError, DataType, Struct, StructType};

/// A single, possibly-null `struct` **row atom**: an array of one
/// [`AnyScalar`](crate::AnyScalar) per field, sharing one [`StructType`].
///
/// Where [`StructScalar`](crate::StructScalar) is the column-oriented struct scalar
/// (one one-element serie per field), `RecordScalar` is the **row-oriented** atom —
/// the struct value materialized field-by-field as the crate's own atomic scalars, so
/// [`any_scalar_at`](RecordScalar::any_scalar_at) /
/// [`any_scalar_by`](RecordScalar::any_scalar_by)
/// hand back a field's [`AnyScalar`](crate::AnyScalar) directly (integer fields
/// decomposed to their concrete scalars, anything else a one-element Arrow value), and
/// the [`NestedSerie`](crate::NestedSerie) child access mirrors it. A present row holds
/// a non-null array of one scalar per field; the Arrow forms are reconstituted on
/// demand (reference-count bumps only), and [`as_struct`](Scalar::as_struct) on any
/// struct scalar materializes this row.
///
/// ```
/// use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataType};
/// use yggdryl_scalar::{AnyScalar, Int64Scalar, NestedSerie, RecordScalar, Scalar};
///
/// let point = dtype::StructType::new(arrow_schema::Fields::from(vec![
///     arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
///     arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
/// ]));
/// let row = RecordScalar::new(
///     point,
///     vec![
///         AnyScalar::from(Int64Scalar::new(1)),
///         AnyScalar::from(Int64Scalar::new(2)),
///     ],
/// )
/// .unwrap();
/// assert_eq!(row.data_type().name(), "struct");
/// assert_eq!(row.child_serie_count(), 2);
///
/// // Generic per-field scalar access, by position and by field name.
/// assert_eq!(row.any_scalar_by("y").unwrap(), AnyScalar::from(Int64Scalar::new(2)));
///
/// // ...or unwrapped straight to a concrete scalar (typed, Rust-only).
/// assert_eq!(row.scalar_at::<Int64Scalar>(0), Some(Int64Scalar::new(1)));
/// assert_eq!(row.scalar_by::<Int64Scalar>("y"), Some(Int64Scalar::new(2)));
///
/// // The Arrow round trip preserves the row.
/// assert_eq!(RecordScalar::from_arrow(row.to_arrow_scalar().as_ref()).unwrap(), row);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct RecordScalar {
    data_type: StructType,
    scalars: Option<Vec<AnyScalar>>,
}

impl Eq for RecordScalar {}

impl RecordScalar {
    /// A record of `data_type` holding the row `scalars` — one atomic scalar per
    /// field, in field order. A scalar count differing from the field count, or a
    /// scalar of a different Arrow type than its field, errors with an actionable
    /// [`DataError`].
    pub fn new(data_type: StructType, scalars: Vec<AnyScalar>) -> Result<Self, DataError> {
        let fields = data_type.fields();
        if scalars.len() != fields.len() {
            return Err(DataError::IncompatibleArrowType {
                expected: format!("{} scalar(s), one per struct field", fields.len()),
                got: format!("{} scalar(s)", scalars.len()),
            });
        }
        for (scalar, field) in scalars.iter().zip(fields.iter()) {
            if &scalar.data_type() != field.data_type() {
                return Err(DataError::IncompatibleArrowType {
                    expected: format!(
                        "a {} value for field \"{}\"",
                        field.data_type(),
                        field.name()
                    ),
                    got: scalar.data_type().to_string(),
                });
            }
        }
        Ok(Self {
            data_type,
            scalars: Some(scalars),
        })
    }

    /// The null record of `data_type`.
    pub fn null(data_type: StructType) -> Self {
        Self {
            data_type,
            scalars: None,
        }
    }

    /// A record over already-validated field `scalars` (a struct scalar's decomposed
    /// row), shared zero-copy — no re-validation.
    pub(crate) fn from_parts(data_type: StructType, scalars: Option<Vec<AnyScalar>>) -> Self {
        Self { data_type, scalars }
    }

    /// The field scalar at `index`, or `None` when the record is null or `index` is
    /// out of bounds.
    pub fn any_scalar_at(&self, index: usize) -> Option<AnyScalar> {
        self.scalars.as_ref()?.get(index).cloned()
    }

    /// The field scalar of the field named `name`, or `None` when the record is null
    /// or no field carries the name.
    pub fn any_scalar_by(&self, name: &str) -> Option<AnyScalar> {
        let index = self
            .data_type
            .fields()
            .iter()
            .position(|field| field.name() == name)?;
        self.any_scalar_at(index)
    }

    /// The field scalar at `index` unwrapped to the concrete scalar `S`, or `None` when
    /// the record is null, `index` is out of bounds, or the field is not an `S` — the
    /// typed counterpart of [`any_scalar_at`](RecordScalar::any_scalar_at).
    ///
    /// It is generic over the target scalar, so it stays **Rust-only**: the bindings
    /// read a field through the record's native-value accessors (`get` / `to_pyvalue`
    /// / `to_js_value`).
    pub fn scalar_at<S: Scalar>(&self, index: usize) -> Option<S> {
        self.any_scalar_at(index)
            .and_then(|scalar| scalar.unwrap::<S>().ok())
    }

    /// The field scalar named `name` unwrapped to the concrete scalar `S`, or `None`
    /// when the record is null, no field carries the name, or the field is not an `S`
    /// — the typed counterpart of [`any_scalar_by`](RecordScalar::any_scalar_by).
    ///
    /// Generic over the target scalar, so it stays **Rust-only** (like
    /// [`scalar_at`](RecordScalar::scalar_at)).
    pub fn scalar_by<S: Scalar>(&self, name: &str) -> Option<S> {
        self.any_scalar_by(name)
            .and_then(|scalar| scalar.unwrap::<S>().ok())
    }
}

impl From<crate::StructScalar> for RecordScalar {
    /// The same row materialized field-by-field — shared zero-copy.
    fn from(scalar: crate::StructScalar) -> Self {
        scalar.as_struct().expect("a struct scalar is a record")
    }
}

impl crate::NestedSerie for RecordScalar {
    fn child_serie_count(&self) -> usize {
        self.data_type.fields().len()
    }

    fn child_serie_at(&self, index: usize) -> Option<AnySerie> {
        // The field scalar handed out as its one-element column (rehydrate with the
        // matching scalar's `from_arrow`) — the NestedSerie contract is column-shaped.
        self.any_scalar_at(index)
            .map(|scalar| AnySerie::from_arrow(scalar.to_arrow_scalar()))
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
    type Value = [AnyScalar];

    fn data_type(&self) -> &StructType {
        &self.data_type
    }

    fn is_null(&self) -> bool {
        self.scalars.is_none()
    }

    fn value(&self) -> Option<&[AnyScalar]> {
        self.scalars.as_deref()
    }

    // A transposed `field | value` table — a wide row still fits the screen.
    fn display_with(&self, options: crate::DisplayOptions) -> String {
        crate::display::render_record(self, options)
    }

    fn to_arrow_scalar(&self) -> ArrayRef {
        let fields = Struct::fields(&self.data_type);
        let Some(scalars) = &self.scalars else {
            return arrow_array::new_null_array(&DataType::to_arrow(&self.data_type), 1);
        };
        // Each field scalar is reconstituted into its one-element column, assembled
        // into the one-element struct row — reference-count bumps, not copies.
        let array = arrow_array::StructArray::try_new_with_length(
            fields.clone(),
            scalars.iter().map(AnyScalar::to_arrow_scalar).collect(),
            None,
            1,
        )
        .expect("one field scalar per declared field assembles into the row");
        std::sync::Arc::new(array)
    }

    fn from_arrow(array: &dyn arrow_array::Array) -> Result<Self, DataError> {
        let length = arrow_array::Array::len(array);
        if length != 1 {
            return Err(DataError::InvalidScalarLength { got: length });
        }
        // The data type validates the layout; every column is decomposed into the
        // crate's own atomic scalar, sharing the buffers zero-copy.
        let data_type = StructType::from_arrow(arrow_array::Array::data_type(array))?;
        let array = array
            .as_any()
            .downcast_ref::<arrow_array::StructArray>()
            .expect("a value with a struct data type is a struct array");
        let scalars = if arrow_array::Array::is_null(array, 0) {
            None
        } else {
            Some(
                array
                    .columns()
                    .iter()
                    .map(|column| AnyScalar::from_arrow(column.clone()))
                    .collect(),
            )
        };
        Ok(Self { data_type, scalars })
    }

    fn as_struct(&self) -> Result<RecordScalar, DataError> {
        Ok(self.clone())
    }
}

impl TypedScalar<StructType, [AnyScalar], arrow_array::StructArray> for RecordScalar {}
