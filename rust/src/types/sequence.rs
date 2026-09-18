//! Sequence: many of one thing, in all five layouts Arrow gives it.
//!
//! One family, one file. Every list-shaped datatype is a leaf here, and
//! [`DataType::Sequence`] is the one variant that holds them, so a caller
//! walking a column's item never asks which offset width it was stored under.
//!
//! | leaf | offsets | length |
//! | --- | --- | --- |
//! | [`SequenceType::List`] | 32-bit | variable |
//! | [`SequenceType::ListView`] | 32-bit, viewed | variable |
//! | [`SequenceType::FixedSizeList`] | none | fixed |
//! | [`SequenceType::LargeList`] | 64-bit | variable |
//! | [`SequenceType::LargeListView`] | 64-bit, viewed | variable |
//!
//! The five differ in how the rows are laid out, never in what a row holds:
//! one item field, the same for all of them. That is why [`Self::item`] is
//! the whole of what most readers need.
//!
//! [`DataType::Sequence`]: crate::DataType::Sequence
//! [`Self::item`]: SequenceType::item

use std::fmt;
use std::sync::Arc;

use crate::types::dtype::validate_non_negative;
use crate::types::family::DataTypeValue;
use crate::types::typed::define_field_types;
use crate::{DataType, DataTypeId, DataTypeKind, Field, Result};
use serde::{Deserialize, Serialize};
use crate::types::family::{Children, NestedValue};
use crate::types::scalar::Value;
use crate::Scalar;

/// The sequence family's datatype payload.
///
/// Each leaf holds the one item field its rows repeat; the fixed-size leaf
/// also holds how many times.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[non_exhaustive]
pub enum SequenceType {
    /// Variable length, 32-bit offsets.
    List(Arc<Field>),
    /// Variable length, 32-bit offsets, viewed.
    ListView(Arc<Field>),
    /// Exactly `length` items per row.
    FixedSizeList(Arc<Field>, i32),
    /// Variable length, 64-bit offsets.
    LargeList(Arc<Field>),
    /// Variable length, 64-bit offsets, viewed.
    LargeListView(Arc<Field>),
}

impl SequenceType {
    /// Returns the item field every leaf repeats.
    ///
    /// The five layouts hold exactly one child, and a dotted path treats that
    /// child as a step it need not spell; this is what makes the layouts
    /// interchangeable to a reader walking children.
    pub fn item(&self) -> &Field {
        match self {
            Self::List(item)
            | Self::ListView(item)
            | Self::FixedSizeList(item, _)
            | Self::LargeList(item)
            | Self::LargeListView(item) => item,
        }
    }

    /// Returns the shared item field without cloning the field itself.
    pub fn item_ref(&self) -> &Arc<Field> {
        match self {
            Self::List(item)
            | Self::ListView(item)
            | Self::FixedSizeList(item, _)
            | Self::LargeList(item)
            | Self::LargeListView(item) => item,
        }
    }

    /// Returns how many items each row holds, or `None` when it varies.
    pub const fn fixed_length(&self) -> Option<i32> {
        match self {
            Self::FixedSizeList(_, length) => Some(*length),
            _ => None,
        }
    }

    /// Returns whether rows are stored as views into a shared buffer.
    pub const fn is_view(&self) -> bool {
        matches!(self, Self::ListView(_) | Self::LargeListView(_))
    }

    /// Returns whether rows carry 64-bit offsets.
    pub const fn is_large(&self) -> bool {
        matches!(self, Self::LargeList(_) | Self::LargeListView(_))
    }

    /// Returns this leaf with its item field replaced.
    pub fn with_item(&self, item: Field) -> Self {
        let item = Arc::new(item);
        match self {
            Self::List(_) => Self::List(item),
            Self::ListView(_) => Self::ListView(item),
            Self::FixedSizeList(_, length) => Self::FixedSizeList(item, *length),
            Self::LargeList(_) => Self::LargeList(item),
            Self::LargeListView(_) => Self::LargeListView(item),
        }
    }

    /// Returns whether both payloads are the same leaf over one allocation.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::List(left), Self::List(right))
            | (Self::ListView(left), Self::ListView(right))
            | (Self::LargeList(left), Self::LargeList(right))
            | (Self::LargeListView(left), Self::LargeListView(right)) => Arc::ptr_eq(left, right),
            (Self::FixedSizeList(left, left_len), Self::FixedSizeList(right, right_len)) => {
                left_len == right_len && Arc::ptr_eq(left, right)
            }
            _ => false,
        }
    }
}

impl DataTypeValue for SequenceType {
    const FAMILY: &'static str = "sequence";

    fn id(&self) -> DataTypeId {
        match self {
            Self::List(_) => DataTypeId::List,
            Self::ListView(_) => DataTypeId::ListView,
            Self::FixedSizeList(..) => DataTypeId::FixedSizeList,
            Self::LargeList(_) => DataTypeId::LargeList,
            Self::LargeListView(_) => DataTypeId::LargeListView,
        }
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Nested
    }

    fn validate(&self) -> Result<()> {
        if let Self::FixedSizeList(_, length) = self {
            validate_non_negative("FixedSizeList", "length", *length)?;
        }
        self.item().validate()
    }

    fn into_dtype(self) -> DataType {
        DataType::Sequence(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<&Self> {
        match dtype {
            DataType::Sequence(family) => Some(family),
            _ => None,
        }
    }
}

impl fmt::Display for SequenceType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<SequenceType> for DataType {
    fn from(value: SequenceType) -> Self {
        Self::Sequence(value)
    }
}

impl DataType {
    /// Creates a 32-bit variable list.
    pub fn list(item: Field) -> Self {
        Self::Sequence(SequenceType::List(Arc::new(item)))
    }

    /// Creates a 32-bit variable list-view.
    pub fn list_view(item: Field) -> Self {
        Self::Sequence(SequenceType::ListView(Arc::new(item)))
    }

    /// Creates a fixed-size list after validating its element count.
    pub fn fixed_size_list(item: Field, length: i32) -> Result<Self> {
        validate_non_negative("FixedSizeList", "length", length)?;
        Ok(Self::Sequence(SequenceType::FixedSizeList(
            Arc::new(item),
            length,
        )))
    }

    /// Creates a 64-bit variable list.
    pub fn large_list(item: Field) -> Self {
        Self::Sequence(SequenceType::LargeList(Arc::new(item)))
    }

    /// Creates a 64-bit variable list-view.
    pub fn large_list_view(item: Field) -> Self {
        Self::Sequence(SequenceType::LargeListView(Arc::new(item)))
    }

    /// Returns the sequence family payload of a sequence datatype.
    pub fn as_sequence_type(&self) -> Option<&SequenceType> {
        SequenceType::from_dtype(self)
    }

    /// Returns the item field of a list-shaped datatype.
    pub(crate) fn list_item(&self) -> Option<&Field> {
        self.as_sequence_type().map(SequenceType::item)
    }
}

define_field_types!(
    ListTypeMarker,
    List,
    crate::DataType::Sequence(crate::types::SequenceType::List(_))
);

define_field_types!(
    ListViewTypeMarker,
    ListView,
    crate::DataType::Sequence(crate::types::SequenceType::ListView(_))
);

define_field_types!(
    LargeListTypeMarker,
    LargeList,
    crate::DataType::Sequence(crate::types::SequenceType::LargeList(_))
);

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

impl NestedValue for Sequence {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.as_slice().iter())
    }
}

define_field_types!(
    FixedSizeListType,
    FixedSizeList,
    crate::DataType::Sequence(crate::types::SequenceType::FixedSizeList(..))
);

define_field_types!(
    LargeListViewType,
    LargeListView,
    crate::DataType::Sequence(crate::types::SequenceType::LargeListView(_))
);

impl Value for Sequence {
    fn dtype(&self) -> Result<DataType> {
        Scalar::Sequence(self.clone()).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Sequence(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Sequence(value) => Some(value),
            _ => None,
        }
    }
}
