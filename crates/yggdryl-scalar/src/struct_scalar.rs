//! The [`Struct`] nested scalar value.

use crate::{Any, Scalar};

/// A struct value — an array of [`Any`] scalars, so a struct nests scalars
/// recursively. Build it from a **collection** of any [`Scalar`]s (natives, [`Any`],
/// or nested `Struct`s), which each promote to an [`Any`] child.
///
/// ```
/// use yggdryl_scalar::{Any, Scalar, Struct};
/// use yggdryl_schema::DataTypeId;
///
/// let row = Struct::from_scalars([1i32, 2i32]);
/// assert_eq!(row.len(), 2);
/// assert_eq!(row.get(0), Some(&Any::Int32(1)));
/// assert_eq!(row.type_id(), DataTypeId::Struct);
/// // A struct is itself a scalar, so structs nest.
/// let nested = Struct::new(vec![row.to_any()]);
/// assert!(nested.get(0).unwrap().is_struct());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Struct(Vec<Any>);

impl Struct {
    /// A struct value from its ordered [`Any`] child values.
    pub fn new(values: Vec<Any>) -> Self {
        Self(values)
    }

    /// A struct value from a collection of any [`Scalar`]s, each promoted to an [`Any`].
    pub fn from_scalars<I>(scalars: I) -> Self
    where
        I: IntoIterator,
        I::Item: Scalar,
    {
        Self(scalars.into_iter().map(|s| s.to_any()).collect())
    }

    /// The child values, in order.
    pub fn values(&self) -> &[Any] {
        &self.0
    }

    /// The child value at `index`, if any.
    pub fn get(&self, index: usize) -> Option<&Any> {
        self.0.get(index)
    }

    /// The number of child values.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the struct holds no child values.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<Any>> for Struct {
    fn from(values: Vec<Any>) -> Self {
        Self(values)
    }
}

impl Scalar for Struct {
    fn type_id(&self) -> yggdryl_schema::DataTypeId {
        yggdryl_schema::DataTypeId::Struct
    }

    fn to_any(&self) -> Any {
        Any::Struct(self.clone())
    }
}
