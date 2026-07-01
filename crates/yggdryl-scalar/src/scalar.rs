//! The [`Scalar`] base trait.

use yggdryl_schema::{Field, Metadata};

/// A single value paired with the schema [`Field`] that describes it, generic over the
/// native value type `T`. It is the value-layer mirror of [`DataType`]`<T>` /
/// [`Field`]`<T>`: it exposes its [`value`](Scalar::value) and [`field`](Scalar::field),
/// and — through the field — its [`name`](Scalar::name), [`dtype`](Scalar::dtype) and
/// [`metadata`](Scalar::metadata).
///
/// ```
/// use yggdryl_scalar::{Int32Scalar, Scalar};
/// use yggdryl_schema::{DataType, DataTypeId};
///
/// let scalar = Int32Scalar::from(7);
/// assert_eq!(*scalar.value(), 7);
/// assert_eq!(scalar.dtype().type_id(), DataTypeId::Int32);
/// assert_eq!(scalar.name(), ""); // unnamed until `with_name`
/// ```
pub trait Scalar<T> {
    /// The concrete field describing this scalar.
    type Field: Field<T>;

    /// The field describing this scalar (name, dtype, nullability, metadata).
    fn field(&self) -> &Self::Field;

    /// The scalar's value.
    fn value(&self) -> &T;

    /// The scalar's name — its field's name.
    fn name<'a>(&'a self) -> &'a str
    where
        Self::Field: 'a,
    {
        self.field().name()
    }

    /// The scalar's data type — its field's data type.
    fn dtype<'a>(&'a self) -> &'a <Self::Field as Field<T>>::DType
    where
        Self::Field: 'a,
    {
        self.field().dtype()
    }

    /// The scalar's metadata — its field's metadata, if any.
    fn metadata<'a>(&'a self) -> Option<&'a Metadata>
    where
        Self::Field: 'a,
    {
        self.field().metadata()
    }
}
