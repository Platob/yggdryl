//! Nested values and typed scalar aliases.

use std::collections::BTreeMap;
use std::fmt;
use std::ops::Index;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;
use crate::{AnyType, DataType, DataTypeId, DataTypeKind, Result, ScalarFamily, ScalarValue};

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

/// One ordered sequence of scalar children.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Sequence(Arc<[Scalar]>);

impl Sequence {
    /// Construct an ordered sequence.
    pub fn new(values: impl Into<Arc<[Scalar]>>) -> Self {
        Self(values.into())
    }

    /// Borrow the ordered values.
    pub fn as_slice(&self) -> &[Scalar] {
        self.0.as_ref()
    }

    /// Consume this value and return its shared children.
    pub fn into_inner(self) -> Arc<[Scalar]> {
        self.0
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_slice())
    }
}

/// One insertion-ordered mapping with arbitrary scalar keys.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Mapping(Arc<[(Scalar, Scalar)]>);

impl Mapping {
    /// Construct a mapping from already unique entries.
    pub fn new(entries: impl Into<Arc<[(Scalar, Scalar)]>>) -> Self {
        Self(entries.into())
    }

    /// Borrow the ordered entries.
    pub fn as_slice(&self) -> &[(Scalar, Scalar)] {
        self.0.as_ref()
    }

    /// Consume this value and return its shared entries.
    pub fn into_inner(self) -> Arc<[(Scalar, Scalar)]> {
        self.0
    }
}

impl fmt::Display for Mapping {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_slice())
    }
}

/// One deterministic record sorted by field name.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Record(Arc<BTreeMap<SmolStr, Scalar>>);

impl Record {
    /// Construct a sorted record.
    pub fn new(values: impl Into<Arc<BTreeMap<SmolStr, Scalar>>>) -> Self {
        Self(values.into())
    }

    /// Borrow the sorted fields.
    pub fn as_map(&self) -> &BTreeMap<SmolStr, Scalar> {
        self.0.as_ref()
    }

    /// Consume this value and return its shared fields.
    pub fn into_inner(self) -> Arc<BTreeMap<SmolStr, Scalar>> {
        self.0
    }
}

impl fmt::Display for Record {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_map())
    }
}

/// One schema-free nested value shape.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[non_exhaustive]
pub enum Nested {
    /// Ordered values, whose field decides list or struct semantics.
    Sequence(Sequence),
    /// Ordered arbitrary-key entries.
    Mapping(Mapping),
    /// Name-sorted record entries.
    Record(Record),
}

impl Nested {
    /// Return the number of immediate children.
    pub fn len(&self) -> usize {
        match self {
            Self::Sequence(value) => value.as_slice().len(),
            Self::Mapping(value) => value.as_slice().len(),
            Self::Record(value) => value.as_map().len(),
        }
    }

    /// Whether this nested value has no children.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for Nested {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sequence(value) => value.fmt(formatter),
            Self::Mapping(value) => value.fmt(formatter),
            Self::Record(value) => value.fmt(formatter),
        }
    }
}

const _: () = assert!(std::mem::size_of::<Nested>() == 24);

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

macro_rules! nested_value {
    ($leaf:ident, $variant:ident, $id:ident) => {
        impl ScalarValue for $leaf {
            type Family = Nested;
            type Type = AnyType;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Nested;

            fn dtype(&self) -> Result<DataType> {
                Scalar::Nested(Nested::$variant(self.clone())).dtype()
            }

            fn into_family(self) -> Self::Family {
                Nested::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Nested::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Nested(Nested::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Nested(Nested::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

nested_value!(Sequence, Sequence, List);
nested_value!(Mapping, Mapping, Map);
nested_value!(Record, Record, Struct);

impl NestedValue for Sequence {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.as_slice().iter())
    }
}

impl NestedValue for Mapping {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Mapping(self.as_slice().iter())
    }
}

impl NestedValue for Record {
    fn len(&self) -> usize {
        self.as_map().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Record(self.as_map().values())
    }
}

impl ScalarFamily for Nested {
    const KIND: DataTypeKind = DataTypeKind::Nested;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Sequence(_) => DataTypeId::List,
            Self::Mapping(_) => DataTypeId::Map,
            Self::Record(_) => DataTypeId::Struct,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        self.clone().into_scalar().dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Nested(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Nested(value) => Some(value),
            _ => None,
        }
    }
}

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

define_scalar_type!(ListScalar, super::ListType, "list");
define_scalar_type!(ListViewScalar, super::ListViewType, "list_view");
define_scalar_type!(
    FixedSizeListScalar,
    super::FixedSizeListType,
    "fixed_size_list"
);
define_scalar_type!(LargeListScalar, super::LargeListType, "large_list");
define_scalar_type!(
    LargeListViewScalar,
    super::LargeListViewType,
    "large_list_view"
);
define_scalar_type!(StructScalar, super::StructType, "struct");
define_scalar_type!(UnionScalar, super::UnionType, "union");
define_scalar_type!(DictionaryScalar, super::DictionaryTypeMarker, "dictionary");
define_scalar_type!(MapScalar, super::MapTypeMarker, "map");
define_scalar_type!(
    VariantScalar,
    super::VariantType,
    "variant",
    crate::DataType::Variant
);
define_scalar_type!(
    RunEndEncodedScalar,
    super::RunEndEncodedTypeMarker,
    "run_end_encoded"
);
