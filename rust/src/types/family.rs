//! What a datatype, a field and a value each owe the root that holds them.
//!
//! [`DataType`], [`Field`] and [`Scalar`] are redirectors: each holds one
//! variant per family, and the family answers which leaf it is. The traits
//! here are the same verbs on the three sides, so a family reads the same
//! whichever side is being asked:
//!
//! | side | trait | widen | narrow |
//! | --- | --- | --- | --- |
//! | datatype | [`DataTypeValue`] | `into_dtype` | `from_dtype` |
//! | field | [`FieldValue`] | `into_field` | `from_field` |
//! | value | [`crate::Value`] | `into_scalar` | `from_scalar` |
//!
//! The roots implement their own trait too - [`DataType`] is a
//! [`DataTypeValue`] and [`Field`] is a `FieldValue<DataType>` - so code that
//! is generic over a family works unchanged on the root that redirects to it.
//!
//! [`Field`]: crate::Field
//! [`Scalar`]: crate::Scalar

use std::fmt;
use std::hash::Hash;

use smol_str::SmolStr;

use crate::{DataType, DataTypeId, DataTypeKind, Field, Metadata, Result, Scalar};

/// One datatype: a family's payload, or the root that redirects to it.
///
/// The implementor is what a [`DataType`] variant holds - an enum over the
/// family's leaves when it has several, one leaf's parameters when it has one,
/// and a parameter-free marker for the variants that carry nothing. Either way
/// [`Self::id`] names the exact leaf, which is what a caller branching on the
/// variant actually wants.
pub trait DataTypeValue:
    Clone + fmt::Debug + fmt::Display + Eq + Hash + Send + Sync + Sized + 'static
{
    /// The family's parameter-free name, as a binding and a refusal spell it.
    const FAMILY: &'static str;

    /// Return the exact identifier of the leaf this payload holds.
    fn id(&self) -> DataTypeId;

    /// Return the category every leaf in this family belongs to.
    fn kind(&self) -> DataTypeKind;

    /// Reject a payload whose parameters cannot describe a column.
    ///
    /// Public variants stay constructible, so a caller can build a payload
    /// that is temporarily invalid; this is where that is caught, before the
    /// value crosses an interoperability boundary.
    fn validate(&self) -> Result<()>;

    /// Widen this payload to the datatype root.
    fn into_dtype(self) -> DataType;

    /// Narrow the datatype root to this family without cloning.
    fn from_dtype(dtype: &DataType) -> Option<&Self>;
}

/// One field: a family's field, or the root that redirects to it.
///
/// A field is its datatype plus the per-column facts a datatype does not
/// carry - the name, the nullability, the metadata. `D` is the datatype the
/// implementor holds, so a leaf field answers with its own leaf datatype and
/// the root answers with [`DataType`]; nothing has to widen to ask.
///
/// [`Self::dtype`] returns an owned datatype rather than a borrow, because a
/// leaf stores the leaf's parameters, not a whole [`DataType`] to lend out.
/// Every implementor's datatype is cheap to produce: the nested ones are one
/// shared pointer, and the parameter-free ones are nothing at all.
pub trait FieldValue<D: DataTypeValue>: Clone + fmt::Debug + fmt::Display + Sized {
    /// Return the physical field name without allocating.
    fn name(&self) -> &str;

    /// Return the datatype this field carries.
    fn dtype(&self) -> D;

    /// Return whether this field admits nulls.
    fn is_nullable(&self) -> bool;

    /// Return the field's metadata without allocating.
    fn metadata(&self) -> &Metadata;

    /// Reject a field whose datatype and options cannot describe a column.
    fn validate(&self) -> Result<()>;

    /// Widen this field to the field root.
    fn into_field(self) -> Field;

    /// Narrow the field root to this family without cloning.
    fn from_field(field: &Field) -> Option<&Self>;
}

/// The root is a datatype like any other family payload.
impl DataTypeValue for DataType {
    const FAMILY: &'static str = "datatype";

    fn id(&self) -> DataTypeId {
        Self::id(self)
    }

    fn kind(&self) -> DataTypeKind {
        Self::kind(self)
    }

    fn validate(&self) -> Result<()> {
        Self::validate(self)
    }

    fn into_dtype(self) -> DataType {
        self
    }

    fn from_dtype(dtype: &DataType) -> Option<&Self> {
        Some(dtype)
    }
}

pub trait NestedValue: crate::Value {
    /// Return the number of direct children.
    fn len(&self) -> usize;
    /// Return whether this value has no direct children.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Iterate over direct sequence values, mapping keys, or record values.
    fn children(&self) -> Children<'_>;
}

/// A borrowed iterator over sequence values, mapping keys or record values.
pub enum Children<'a> {
    /// Sequence values.
    Sequence(std::slice::Iter<'a, Scalar>),
    /// Mapping keys.
    Mapping(std::slice::Iter<'a, (Scalar, Scalar)>),
    /// Record field values in sorted name order.
    Record(std::collections::btree_map::Values<'a, SmolStr, Scalar>),
}

impl<'a> Iterator for Children<'a> {
    type Item = &'a Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next(),
            Self::Mapping(entries) => entries.next().map(|(key, _)| key),
            Self::Record(entries) => entries.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.len();
        (length, Some(length))
    }
}

impl DoubleEndedIterator for Children<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next_back(),
            Self::Mapping(entries) => entries.next_back().map(|(key, _)| key),
            Self::Record(entries) => entries.next_back(),
        }
    }
}

impl ExactSizeIterator for Children<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
            Self::Record(entries) => entries.len(),
        }
    }
}

impl std::iter::FusedIterator for Children<'_> {}






