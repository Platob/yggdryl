//! What a datatype family owes the root that redirects to it.
//!
//! [`DataType`], [`Field`] and [`Scalar`] each hold one variant per *family*,
//! never one per leaf. A family owns the leaves that share a shape - every
//! mapping, every sequence - and answers which leaf it is holding. The root is
//! then a redirector: it matches one variant and hands the question down.
//!
//! The three traits here are the same three verbs on the three sides, so a
//! family reads the same whichever side is being asked:
//!
//! | side | trait | widen | narrow |
//! | --- | --- | --- | --- |
//! | datatype | [`FamilyType`] | `into_dtype` | `from_dtype` |
//! | field | [`FamilyField`] | `into_field` | `from_field` |
//! | value | [`crate::Value`] | `into_scalar` | `from_scalar` |
//!
//! [`Field`]: crate::Field
//! [`Scalar`]: crate::Scalar

use std::fmt;
use std::hash::Hash;

use crate::{DataType, DataTypeId, DataTypeKind, Field, Result};
use smol_str::SmolStr;
use crate::Scalar;

/// One datatype family's payload.
///
/// The implementor is what a [`DataType`] variant holds: an enum over the
/// family's leaves when it has several, a single leaf's parameters when it has
/// one. Either way [`Self::id`] names the exact leaf, which is what a caller
/// branching on the variant actually wants.
pub trait FamilyType: Clone + fmt::Debug + Eq + Hash + Sized {
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

/// One field family's payload.
///
/// A family's field is its datatype plus the per-column facts a datatype does
/// not carry - the name, the nullability, the metadata. It validates on
/// construction, so holding one is the proof that the field really is of this
/// family.
pub trait FamilyField: Clone + fmt::Debug + Sized {
    /// The family this field's datatype belongs to.
    type Type: FamilyType;

    /// Check a generic field and take ownership of it.
    fn try_from_field(field: Field) -> Result<Self>;

    /// Borrow the generic field without allocating.
    fn as_field(&self) -> &Field;

    /// Return the family payload of this field's datatype.
    fn family_type(&self) -> &Self::Type;

    /// Widen this payload back to the generic field.
    fn into_field(self) -> Field;
}

// ------------------------------------------------------------------------
// Nested values and typed scalar aliases.
// ------------------------------------------------------------------------

/// Borrowing access shared by every nested value shape.
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






