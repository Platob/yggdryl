//! The serie family's datatype: many of one thing, in all five layouts
//! Arrow gives it - and the run a row is.
//!
//! The five `DataType` serie variants are the family's leaves, and
//! [`SerieType`] is the view they all answer, so a caller walking a column's
//! item never asks which offset width it was stored under.
//!
//! | leaf | offsets | length |
//! | --- | --- | --- |
//! | [`SerieType::Serie`] | 32-bit | variable |
//! | [`SerieType::SerieView`] | 32-bit, viewed | variable |
//! | [`SerieType::FixedSizeSerie`] | none | fixed |
//! | [`SerieType::LargeSerie`] | 64-bit | variable |
//! | [`SerieType::LargeSerieView`] | 64-bit, viewed | variable |
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
//! | `slice()` | the same values, offsets summed | an Arrow slice |
//! | `push`, `set`, `splice` | the window copied once | the buffers written in place |
//! | `into_arrow_array()` | `None` | the buffers, shared |
//!
//! [`Self::item`]: SerieType::item
//! [`Serie`]: crate::Serie

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use crate::Scalar;
use crate::datatype::validate_non_negative;
use crate::value::DataTypeValue;
use crate::value::Value;
use crate::value::{Children, NestedValue};
use crate::{DataType, DataTypeId, Field, Result, Serie};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The typed field's payload over the five serie layouts.
///
/// Each leaf holds the one item field its rows repeat; the fixed-size leaf
/// also holds how many times.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[non_exhaustive]
pub enum SerieType {
    /// Variable length, 32-bit offsets.
    Serie(Arc<Field>),
    /// Variable length, 32-bit offsets, viewed.
    SerieView(Arc<Field>),
    /// Exactly `length` items per row.
    FixedSizeSerie(Arc<Field>, i32),
    /// Variable length, 64-bit offsets.
    LargeSerie(Arc<Field>),
    /// Variable length, 64-bit offsets, viewed.
    LargeSerieView(Arc<Field>),
}

impl SerieType {
    /// Returns the item field every leaf repeats.
    ///
    /// The five layouts hold exactly one child, and a dotted path treats that
    /// child as a step it need not spell; this is what makes the layouts
    /// interchangeable to a reader walking children.
    pub fn item(&self) -> &Field {
        match self {
            Self::Serie(item)
            | Self::SerieView(item)
            | Self::FixedSizeSerie(item, _)
            | Self::LargeSerie(item)
            | Self::LargeSerieView(item) => item,
        }
    }

    /// Returns the shared item field without cloning the field itself.
    pub fn item_ref(&self) -> &Arc<Field> {
        match self {
            Self::Serie(item)
            | Self::SerieView(item)
            | Self::FixedSizeSerie(item, _)
            | Self::LargeSerie(item)
            | Self::LargeSerieView(item) => item,
        }
    }

    /// Returns how many items each row holds, or `None` when it varies.
    pub const fn fixed_length(&self) -> Option<i32> {
        match self {
            Self::FixedSizeSerie(_, length) => Some(*length),
            _ => None,
        }
    }

    /// Returns whether rows are stored as views into a shared buffer.
    pub const fn is_view(&self) -> bool {
        matches!(self, Self::SerieView(_) | Self::LargeSerieView(_))
    }

    /// Returns whether rows carry 64-bit offsets.
    pub const fn is_large(&self) -> bool {
        matches!(self, Self::LargeSerie(_) | Self::LargeSerieView(_))
    }

    /// Returns this leaf with its item field replaced.
    pub fn with_item(&self, item: Field) -> Self {
        let item = Arc::new(item);
        match self {
            Self::Serie(_) => Self::Serie(item),
            Self::SerieView(_) => Self::SerieView(item),
            Self::FixedSizeSerie(_, length) => Self::FixedSizeSerie(item, *length),
            Self::LargeSerie(_) => Self::LargeSerie(item),
            Self::LargeSerieView(_) => Self::LargeSerieView(item),
        }
    }

    /// Returns whether both payloads are the same leaf over one allocation.
    pub fn shares_storage_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Serie(left), Self::Serie(right))
            | (Self::SerieView(left), Self::SerieView(right))
            | (Self::LargeSerie(left), Self::LargeSerie(right))
            | (Self::LargeSerieView(left), Self::LargeSerieView(right)) => Arc::ptr_eq(left, right),
            (Self::FixedSizeSerie(left, left_len), Self::FixedSizeSerie(right, right_len)) => {
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
            Self::Serie(_) => DataTypeId::Serie,
            Self::SerieView(_) => DataTypeId::SerieView,
            Self::FixedSizeSerie(..) => DataTypeId::FixedSizeSerie,
            Self::LargeSerie(_) => DataTypeId::LargeSerie,
            Self::LargeSerieView(_) => DataTypeId::LargeSerieView,
        }
    }

    fn validate(&self) -> Result<()> {
        if let Self::FixedSizeSerie(_, length) = self {
            validate_non_negative("FixedSizeSerie", "length", *length)?;
        }
        self.item().validate()
    }

    fn into_dtype(self) -> DataType {
        match self {
            Self::Serie(item) => DataType::Serie(item),
            Self::SerieView(item) => DataType::SerieView(item),
            Self::FixedSizeSerie(item, length) => DataType::FixedSizeSerie(item, length),
            Self::LargeSerie(item) => DataType::LargeSerie(item),
            Self::LargeSerieView(item) => DataType::LargeSerieView(item),
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
    /// Creates a 32-bit variable serie.
    pub fn serie(item: Field) -> Self {
        Self::Serie(Arc::new(item))
    }

    /// Creates a 32-bit variable serie view.
    pub fn serie_view(item: Field) -> Self {
        Self::SerieView(Arc::new(item))
    }

    /// Creates a fixed-size serie after validating its element count.
    pub fn fixed_size_serie(item: Field, length: i32) -> Result<Self> {
        validate_non_negative("FixedSizeSerie", "length", length)?;
        Ok(Self::FixedSizeSerie(Arc::new(item), length))
    }

    /// Creates a 64-bit variable serie.
    pub fn large_serie(item: Field) -> Self {
        Self::LargeSerie(Arc::new(item))
    }

    /// Creates a 64-bit variable serie view.
    pub fn large_serie_view(item: Field) -> Self {
        Self::LargeSerieView(Arc::new(item))
    }

    /// The typed field's payload over any of the five serie layouts, `None`
    /// for every other datatype: a shared-pointer clone of the item field.
    #[must_use]
    pub fn as_serie_type(&self) -> Option<SerieType> {
        match self {
            Self::Serie(item) => Some(SerieType::Serie(Arc::clone(item))),
            Self::SerieView(item) => Some(SerieType::SerieView(Arc::clone(item))),
            Self::FixedSizeSerie(item, length) => {
                Some(SerieType::FixedSizeSerie(Arc::clone(item), *length))
            }
            Self::LargeSerie(item) => Some(SerieType::LargeSerie(Arc::clone(item))),
            Self::LargeSerieView(item) => Some(SerieType::LargeSerieView(Arc::clone(item))),
            _ => None,
        }
    }

    /// Move a sequence or mapping `value` into the variant this datatype
    /// declares: a `large_serie` value is a [`Scalar::LargeSerie`], a sorted
    /// map's a [`Scalar::SortedMap`]. The rows are untouched; any other
    /// value, or a value of another family, is answered as it is.
    #[must_use]
    pub fn declared_layout(&self, value: Scalar) -> Scalar {
        match value {
            Scalar::Serie(serie)
            | Scalar::SerieView(serie)
            | Scalar::FixedSizeSerie(serie)
            | Scalar::LargeSerie(serie)
            | Scalar::LargeSerieView(serie) => match self {
                Self::SerieView(_) => Scalar::SerieView(serie),
                Self::FixedSizeSerie(..) => Scalar::FixedSizeSerie(serie),
                Self::LargeSerie(_) => Scalar::LargeSerie(serie),
                Self::LargeSerieView(_) => Scalar::LargeSerieView(serie),
                _ => Scalar::Serie(serie),
            },
            Scalar::Map(entries) | Scalar::SortedMap(entries) => match self {
                Self::SortedMap(_) => Scalar::SortedMap(entries),
                _ => Scalar::Map(entries),
            },
            other => other,
        }
    }

    /// Returns the item field of any of the five serie layouts, borrowed.
    #[must_use]
    pub fn serie_item(&self) -> Option<&Field> {
        match self {
            Self::Serie(item)
            | Self::SerieView(item)
            | Self::FixedSizeSerie(item, _)
            | Self::LargeSerie(item)
            | Self::LargeSerieView(item) => Some(item),
            _ => None,
        }
    }
}

/// A schema-free ordered run of values: what a row canonicalizes to.
///
/// A window over one shared slice: built in one allocation, sliced in none.
/// [`Serie::slice`] of a run shares the slice with the offsets summed, so a
/// slice of a slice is one level over the original allocation, and an empty
/// slice is the one shared empty run, holding nothing. It declares no field,
/// so it accepts every value and agrees its datatype back out of its rows;
/// [`Serie`] holds it as its one schema-free leaf, beside the columns that
/// carry a field.
///
/// Identity is the window's rows alone: equality, order, hash, `Debug`,
/// `Display` and serde read [`Self::as_slice`] and never a row around it. A
/// live window keeps its whole slice alive, as an Arrow slice keeps its
/// buffers.
///
/// A sort or a reverse rewrites the window in place when this run holds its
/// slice alone. Any other write copies the window once - `Arc<[Scalar]>`
/// cannot grow in place - and a shared slice is never copied past the
/// window, the copy letting go of it; building one a `push` at a time is
/// quadratic, so [`Scalar::from_sequence`] and [`Serie::new`] build one from
/// values in hand.
///
/// ```
/// use yggdryl::{Run, Scalar, Serie};
///
/// let prices = Serie::new(vec![
///     Scalar::from(125_i64),
///     Scalar::from(126_i64),
///     Scalar::from(127_i64),
///     Scalar::from(128_i64),
/// ]);
/// let inner = prices.slice(1, 3)?.slice(1, 2)?;
/// let (Some(whole), Some(window)) = (prices.as_run(), inner.as_run()) else {
///     panic!("both are runs");
/// };
/// // A slice of a slice is a window over the one slice the run was built in.
/// assert!(std::ptr::eq(&whole.as_slice()[2], &window.as_slice()[0]));
/// assert_eq!(window, &Run::new(vec![Scalar::from(127_i64), Scalar::from(128_i64)]));
/// # Ok::<(), yggdryl::Error>(())
/// ```
#[derive(Clone)]
pub struct Run {
    values: Arc<[Scalar]>,
    start: usize,
    len: usize,
}

impl Run {
    /// Construct an ordered run over the whole of `values`.
    pub fn new(values: impl Into<Arc<[Scalar]>>) -> Self {
        let values = values.into();
        let len = values.len();
        Self {
            values,
            start: 0,
            len,
        }
    }

    /// Borrow the ordered values.
    pub fn as_slice(&self) -> &[Scalar] {
        &self.values[self.start..self.start + self.len]
    }

    /// The window `offset..offset + length` of this one, over the same
    /// slice; the caller has proven it lies inside. An empty window is the
    /// shared empty run, so it keeps nothing alive.
    pub(crate) fn slice(&self, offset: usize, length: usize) -> Self {
        if length == 0 {
            return Self::default();
        }
        Self {
            values: Arc::clone(&self.values),
            start: self.start + offset,
            len: length,
        }
    }

    /// Borrow the values to rewrite them where they stand: in place when
    /// this run holds its slice alone, else the window alone copied once,
    /// which lets go of the slice.
    pub(crate) fn make_mut(&mut self) -> &mut [Scalar] {
        if Arc::get_mut(&mut self.values).is_none() {
            *self = Self::new(Arc::<[Scalar]>::from(self.as_slice()));
        }
        let window = self.start..self.start + self.len;
        &mut Arc::make_mut(&mut self.values)[window]
    }
}

impl Default for Run {
    /// The one shared empty run, which every empty sequence answers with.
    fn default() -> Self {
        static EMPTY: OnceLock<Arc<[Scalar]>> = OnceLock::new();
        Self::new(Arc::clone(EMPTY.get_or_init(|| Arc::from([]))))
    }
}

impl fmt::Debug for Run {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("Run")
            .field(&self.as_slice())
            .finish()
    }
}

impl PartialEq for Run {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.values, &other.values)
            && self.start == other.start
            && self.len == other.len)
            || self.as_slice() == other.as_slice()
    }
}

impl Eq for Run {}

impl PartialOrd for Run {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Run {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_slice().cmp(other.as_slice())
    }
}

impl Hash for Run {
    /// The bytes `<[Scalar]>::hash` writes, which a column of the same rows
    /// writes too.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_slice().hash(state);
    }
}

impl Serialize for Run {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.as_slice().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Run {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Vec::<Scalar>::deserialize(deserializer).map(Self::new)
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
        Scalar::Serie(Serie::Run(self.clone())).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Serie(Serie::Run(self))
    }

    /// Answers only a run: a column is the same variant and not this leaf.
    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Serie(Serie::Run(value))
            | Scalar::SerieView(Serie::Run(value))
            | Scalar::FixedSizeSerie(Serie::Run(value))
            | Scalar::LargeSerie(Serie::Run(value))
            | Scalar::LargeSerieView(Serie::Run(value)) => Some(value),
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
            // The item is a shared box, so its projection is borrowed and
            // cached where every holder of the box finds it.
            Ok(match self {
                Self::Serie(item) => ArrowDataType::List(Arc::clone(item.as_arrow_field_ref()?)),
                Self::SerieView(item) => {
                    ArrowDataType::ListView(Arc::clone(item.as_arrow_field_ref()?))
                }
                Self::FixedSizeSerie(item, length) => {
                    validate_non_negative("FixedSizeSerie", "length", *length)?;
                    ArrowDataType::FixedSizeList(Arc::clone(item.as_arrow_field_ref()?), *length)
                }
                Self::LargeSerie(item) => {
                    ArrowDataType::LargeList(Arc::clone(item.as_arrow_field_ref()?))
                }
                Self::LargeSerieView(item) => {
                    ArrowDataType::LargeListView(Arc::clone(item.as_arrow_field_ref()?))
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
                Self::Serie(item) => ArrowDataType::List(arrow_field_ref_from_shared(item)?),
                Self::SerieView(item) => {
                    ArrowDataType::ListView(arrow_field_ref_from_shared(item)?)
                }
                Self::FixedSizeSerie(item, length) => {
                    validate_non_negative("FixedSizeSerie", "length", length)?;
                    ArrowDataType::FixedSizeList(arrow_field_ref_from_shared(item)?, length)
                }
                Self::LargeSerie(item) => {
                    ArrowDataType::LargeList(arrow_field_ref_from_shared(item)?)
                }
                Self::LargeSerieView(item) => {
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
                Self::Serie(_) => ArrowFfiParts::nested("+l", child),
                Self::SerieView(_) => ArrowFfiParts::nested("+vl", child),
                Self::FixedSizeSerie(_, length) => {
                    validate_non_negative("FixedSizeSerie", "length", *length)?;
                    ArrowFfiParts::nested(format!("+w:{length}"), child)
                }
                Self::LargeSerie(_) => ArrowFfiParts::nested("+L", child),
                Self::LargeSerieView(_) => ArrowFfiParts::nested("+vL", child),
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
                ArrowDataType::List(item) => Ok(DataType::serie(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                ArrowDataType::ListView(item) => Ok(DataType::serie_view(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                ArrowDataType::FixedSizeList(item, length) => DataType::fixed_size_serie(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                    *length,
                ),
                ArrowDataType::LargeList(item) => Ok(DataType::large_serie(
                    Field::from_arrow_field_ref_at_depth(Arc::clone(item), depth)?,
                )),
                ArrowDataType::LargeListView(item) => Ok(DataType::large_serie_view(
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
                ArrowDataType::List(item) => Ok(DataType::serie(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                ArrowDataType::ListView(item) => Ok(DataType::serie_view(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                ArrowDataType::FixedSizeList(item, length) => DataType::fixed_size_serie(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                    length,
                ),
                ArrowDataType::LargeList(item) => Ok(DataType::large_serie(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                ArrowDataType::LargeListView(item) => Ok(DataType::large_serie_view(
                    Field::from_arrow_field_ref_at_depth(item, depth)?,
                )),
                other => Self::from_arrow_storage_at_depth(&other, depth),
            }
        }
    }
}
