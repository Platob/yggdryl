//! Nested typed scalar aliases.

use std::ops::Index;

use smol_str::SmolStr;

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

/// Borrowing access shared by every nested value shape.
pub trait NestedValue: crate::ScalarValue {
    /// Return the number of direct children.
    fn len(&self) -> usize;
    /// Return whether this value has no direct children.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Iterate over direct sequence values, mapping keys, or record values.
    fn children(&self) -> Children<'_>;
}

/// A borrowed iterator over sequence values or mapping keys.
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

impl<'a> IntoIterator for &'a Scalar {
    type Item = &'a Scalar;
    type IntoIter = Children<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<Scalar>> for Scalar {
    fn from(value: Vec<Scalar>) -> Self {
        Self::from_sequence(value)
    }
}

impl FromIterator<Scalar> for Scalar {
    fn from_iter<T: IntoIterator<Item = Scalar>>(iter: T) -> Self {
        Self::from_sequence(iter)
    }
}

impl Index<usize> for Scalar {
    type Output = Scalar;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_sequence().expect("value is not a sequence")[index]
    }
}

impl Index<&Scalar> for Scalar {
    type Output = Scalar;

    fn index(&self, key: &Scalar) -> &Self::Output {
        self.get_key(key).expect("mapping key is not present")
    }
}

impl Index<&str> for Scalar {
    type Output = Scalar;

    fn index(&self, key: &str) -> &Self::Output {
        self.get_key_str(key).expect("mapping key is not present")
    }
}

define_scalar_type!(ListScalar, super::List, "list");
define_scalar_type!(ListViewScalar, super::ListView, "list_view");
define_scalar_type!(FixedSizeListScalar, super::FixedSizeList, "fixed_size_list");
define_scalar_type!(LargeListScalar, super::LargeList, "large_list");
define_scalar_type!(LargeListViewScalar, super::LargeListView, "large_list_view");
define_scalar_type!(StructScalar, super::Struct, "struct");
define_scalar_type!(UnionScalar, super::Union, "union");
define_scalar_type!(DictionaryScalar, super::Dictionary, "dictionary");
define_scalar_type!(MapScalar, super::Map, "map");
define_scalar_type!(
    VariantScalar,
    super::Variant,
    "variant",
    crate::DataType::Variant
);
define_scalar_type!(RunEndEncodedScalar, super::RunEndEncoded, "run_end_encoded");
