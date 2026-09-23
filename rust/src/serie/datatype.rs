//! The serie family's datatype: many of one thing, in all five layouts
//! Arrow gives it - and the run a row is.
//!
//! The five `DataType` list variants are the family's leaves, and
//! [`SerieType`] is the view they all answer, so a caller walking a column's
//! item never asks which offset width it was stored under.
//!
//! | leaf | offsets | length |
//! | --- | --- | --- |
//! | [`SerieType::List`] | 32-bit | variable |
//! | [`SerieType::ListView`] | 32-bit, viewed | variable |
//! | [`SerieType::FixedSizeList`] | none | fixed |
//! | [`SerieType::LargeList`] | 64-bit | variable |
//! | [`SerieType::LargeListView`] | 64-bit, viewed | variable |
//!
//! The five differ in how the rows are laid out, never in what a row holds:
//! one item field, the same for all of them. That is why [`Self::item`] is
//! the whole of what most readers need.
//!
//! On the value side the family's value is [`Serie`], and this file holds
//! its schema-free leaf, [`Run`]: what a row canonicalizes to, what a
//! document parses as, what [`Scalar::from_sequence`] builds. Every other
//! leaf of [`Serie`] is a column - the Arrow buffers of one [`Field`] - and
//! the two answer the same verbs at different costs:
//!
//! | ask | [`Run`] | a column |
//! | --- | --- | --- |
//! | `field()` | `None` | the field the rows are typed by |
//! | `as_slice()` | the values, lent | `None`: no value is stored |
//! | `scalar(i)` | one clone | one row built off the buffers |
//! | `null_count()` | a walk | the validity bitmap's count |
//! | `push`, `set`, `splice` | the whole run copied once | the buffers written in place |
//! | `into_arrow_array()` | `None` | the buffers, shared |
//!
//! [`Self::item`]: SerieType::item
//! [`Serie`]: crate::Serie

use std::fmt;
use std::sync::Arc;

use crate::Scalar;
use crate::datatype::validate_non_negative;
use crate::value::DataTypeValue;
use crate::value::Value;
use crate::value::{Children, NestedValue};
use crate::{DataType, DataTypeId, DataTypeKind, Field, Result, Serie};
use serde::{Deserialize, Serialize};

/// The serie family's datatype view.
///
/// Each leaf holds the one item field its rows repeat; the fixed-size leaf
/// also holds how many times.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[non_exhaustive]
pub enum SerieType {
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

impl SerieType {
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

impl DataTypeValue for SerieType {
    const FAMILY: &'static str = "sequence";

    type Sidecar = ();

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
        match self {
            Self::List(item) => DataType::List(item),
            Self::ListView(item) => DataType::ListView(item),
            Self::FixedSizeList(item, length) => DataType::FixedSizeList(item, length),
            Self::LargeList(item) => DataType::LargeList(item),
            Self::LargeListView(item) => DataType::LargeListView(item),
        }
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        dtype.as_serie_type()
    }
}

impl fmt::Display for SerieType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<SerieType> for DataType {
    fn from(value: SerieType) -> Self {
        DataTypeValue::into_dtype(value)
    }
}

impl DataType {
    /// Creates a 32-bit variable list.
    pub fn list(item: Field) -> Self {
        Self::List(Arc::new(item))
    }

    /// Creates a 32-bit variable list-view.
    pub fn list_view(item: Field) -> Self {
        Self::ListView(Arc::new(item))
    }

    /// Creates a fixed-size list after validating its element count.
    pub fn fixed_size_list(item: Field, length: i32) -> Result<Self> {
        validate_non_negative("FixedSizeList", "length", length)?;
        Ok(Self::FixedSizeList(Arc::new(item), length))
    }

    /// Creates a 64-bit variable list.
    pub fn large_list(item: Field) -> Self {
        Self::LargeList(Arc::new(item))
    }

    /// Creates a 64-bit variable list-view.
    pub fn large_list_view(item: Field) -> Self {
        Self::LargeListView(Arc::new(item))
    }

    /// The serie family's view of any of the five list layouts, `None`
    /// for every other datatype: a shared-pointer clone of the item field.
    #[must_use]
    pub fn as_serie_type(&self) -> Option<SerieType> {
        match self {
            Self::List(item) => Some(SerieType::List(Arc::clone(item))),
            Self::ListView(item) => Some(SerieType::ListView(Arc::clone(item))),
            Self::FixedSizeList(item, length) => {
                Some(SerieType::FixedSizeList(Arc::clone(item), *length))
            }
            Self::LargeList(item) => Some(SerieType::LargeList(Arc::clone(item))),
            Self::LargeListView(item) => Some(SerieType::LargeListView(Arc::clone(item))),
            _ => None,
        }
    }

    /// Move a sequence or mapping `value` into the variant this datatype
    /// declares: a `large_list` value is a [`Scalar::LargeList`], a sorted
    /// map's a [`Scalar::SortedMap`]. The rows are untouched; any other
    /// value, or a value of another family, is answered as it is.
    #[must_use]
    pub fn declared_layout(&self, value: Scalar) -> Scalar {
        match value {
            Scalar::List(serie)
            | Scalar::ListView(serie)
            | Scalar::FixedSizeList(serie)
            | Scalar::LargeList(serie)
            | Scalar::LargeListView(serie) => match self {
                Self::ListView(_) => Scalar::ListView(serie),
                Self::FixedSizeList(..) => Scalar::FixedSizeList(serie),
                Self::LargeList(_) => Scalar::LargeList(serie),
                Self::LargeListView(_) => Scalar::LargeListView(serie),
                _ => Scalar::List(serie),
            },
            Scalar::Map(entries) | Scalar::SortedMap(entries) => match self {
                Self::SortedMap(_) => Scalar::SortedMap(entries),
                _ => Scalar::Map(entries),
            },
            other => other,
        }
    }

    /// Returns the item field of any of the five list layouts, borrowed.
    #[must_use]
    pub fn list_item(&self) -> Option<&Field> {
        match self {
            Self::List(item)
            | Self::ListView(item)
            | Self::FixedSizeList(item, _)
            | Self::LargeList(item)
            | Self::LargeListView(item) => Some(item),
            _ => None,
        }
    }
}

/// A schema-free ordered run of values: what a row canonicalizes to.
///
/// One shared slice, built in one allocation and lent as it is. It declares
/// no field, so it accepts every value and agrees its datatype back out of
/// its rows; [`Serie`] holds it as its one schema-free leaf, beside the
/// columns that carry a field.
///
/// A run is written by copying it once - `Arc<[Scalar]>` cannot grow in
/// place - so building one a `push` at a time is quadratic;
/// [`Scalar::from_sequence`] and [`Serie::new`] build one from values in
/// hand.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Run(Arc<[Scalar]>);

impl Run {
    /// Construct an ordered run.
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

impl fmt::Display for Run {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_slice())
    }
}

impl NestedValue for Run {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.as_slice().iter())
    }
}

impl Value for Run {
    fn dtype(&self) -> Result<DataType> {
        Scalar::List(Serie::Run(self.clone())).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::List(Serie::Run(self))
    }

    /// Answers only a run: a column is the same variant and not this leaf.
    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::List(Serie::Run(value))
            | Scalar::ListView(Serie::Run(value))
            | Scalar::FixedSizeList(Serie::Run(value))
            | Scalar::LargeList(Serie::Run(value))
            | Scalar::LargeListView(Serie::Run(value)) => Some(value),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// Arrow projection: five layouts over one item field.
// ------------------------------------------------------------------------

mod arrow {
    use std::sync::Arc;

    use arrow_schema::DataType as ArrowDataType;
    use smol_str::format_smolstr;

    use super::SerieType;
    use crate::field::arrow_field_ref_from_shared;
    use crate::value::ArrowFfiParts;
    use crate::{DataType, Field, Result};
    use crate::{invalid, validate_non_negative};

    impl SerieType {
        /// The Arrow storage this layout lays out.
        ///
        /// # Errors
        ///
        /// Returns an error when the fixed length is negative or the item field
        /// has no Arrow projection.
        pub(crate) fn arrow_storage(&self) -> Result<ArrowDataType> {
            Ok(match self {
                Self::List(item) => {
                    ArrowDataType::List(item.as_ref().clone().into_arrow_field_ref()?)
                }
                Self::ListView(item) => {
                    ArrowDataType::ListView(item.as_ref().clone().into_arrow_field_ref()?)
                }
                Self::FixedSizeList(item, length) => {
                    validate_non_negative("FixedSizeList", "length", *length)?;
                    ArrowDataType::FixedSizeList(
                        item.as_ref().clone().into_arrow_field_ref()?,
                        *length,
                    )
                }
                Self::LargeList(item) => {
                    ArrowDataType::LargeList(item.as_ref().clone().into_arrow_field_ref()?)
                }
                Self::LargeListView(item) => {
                    ArrowDataType::LargeListView(item.as_ref().clone().into_arrow_field_ref()?)
                }
            })
        }

        /// The same projection, consuming a uniquely held item field.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn into_arrow_storage(self) -> Result<ArrowDataType> {
            Ok(match self {
                Self::List(item) => ArrowDataType::List(arrow_field_ref_from_shared(item)?),
                Self::ListView(item) => ArrowDataType::ListView(arrow_field_ref_from_shared(item)?),
                Self::FixedSizeList(item, length) => {
                    validate_non_negative("FixedSizeList", "length", length)?;
                    ArrowDataType::FixedSizeList(arrow_field_ref_from_shared(item)?, length)
                }
                Self::LargeList(item) => {
                    ArrowDataType::LargeList(arrow_field_ref_from_shared(item)?)
                }
                Self::LargeListView(item) => {
                    ArrowDataType::LargeListView(arrow_field_ref_from_shared(item)?)
                }
            })
        }

        /// The C Data Interface node this layout writes.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn arrow_ffi_parts(&self) -> Result<ArrowFfiParts> {
            let child = vec![self.item().clone().into_arrow_field_ffi()?];
            Ok(match self {
                Self::List(_) => ArrowFfiParts::nested("+l", child),
                Self::ListView(_) => ArrowFfiParts::nested("+vl", child),
                Self::FixedSizeList(_, length) => {
                    validate_non_negative("FixedSizeList", "length", *length)?;
                    ArrowFfiParts::nested(format!("+w:{length}"), child)
                }
                Self::LargeList(_) => ArrowFfiParts::nested("+L", child),
                Self::LargeListView(_) => ArrowFfiParts::nested("+vL", child),
            })
        }

        /// The sequence datatype one Arrow list storage imports as.
        ///
        /// # Errors
        ///
        /// Returns an error when the item field cannot be imported, when the
        /// fixed length is negative, or when the storage belongs to another
        /// family.
        pub(crate) fn from_arrow_storage_at_depth(
            value: &ArrowDataType,
            depth: usize,
        ) -> Result<DataType> {
            match value {
                ArrowDataType::List(item) => Ok(DataType::list(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                ArrowDataType::ListView(item) => Ok(DataType::list_view(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                ArrowDataType::FixedSizeList(item, length) => DataType::fixed_size_list(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                    *length,
                ),
                ArrowDataType::LargeList(item) => Ok(DataType::large_list(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                ArrowDataType::LargeListView(item) => Ok(DataType::large_list_view(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                other => Err(invalid(
                    "sequence",
                    format_smolstr!("expected a list storage, got {other}"),
                )),
            }
        }

        /// The same import, consuming Arrow's shared item field.
        ///
        /// # Errors
        ///
        /// [`Self::from_arrow_storage_at_depth`] carries the rule.
        pub(crate) fn from_arrow_storage_owned_at_depth(
            value: ArrowDataType,
            depth: usize,
        ) -> Result<DataType> {
            match value {
                ArrowDataType::List(item) => Ok(DataType::list(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                ArrowDataType::ListView(item) => Ok(DataType::list_view(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                ArrowDataType::FixedSizeList(item, length) => DataType::fixed_size_list(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                    length,
                ),
                ArrowDataType::LargeList(item) => Ok(DataType::large_list(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                ArrowDataType::LargeListView(item) => Ok(DataType::large_list_view(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                other => Self::from_arrow_storage_at_depth(&other, depth),
            }
        }
    }
}
