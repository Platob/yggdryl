//! Shared child collections and validated nested datatype construction.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::ops::{Deref, Index};
use std::sync::Arc;

use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::enums::UnionMode;
use crate::{Error, Field, Result};

use super::DataType;
use super::scalar::{invalid, validate_non_negative};

/// An ordered, immutable collection of fields stored in one shared allocation.
#[derive(Clone, Default, Eq, PartialEq, Hash)]
pub struct Fields(pub(super) Option<Arc<[Field]>>);

impl Fields {
    /// Creates an empty collection without allocating.
    pub const fn new() -> Self {
        Self(None)
    }

    /// Creates a collection and rejects duplicate field names.
    pub fn from_fields<I>(fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        let fields = fields.into_iter().collect::<Vec<_>>();
        validate_fields(&fields, "Fields")?;
        Ok(Self::from_vec(fields))
    }

    /// Returns the number of fields.
    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns whether no fields are present.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Returns all fields as a borrowed slice without allocating.
    pub fn as_fields(&self) -> &[Field] {
        self.as_ref()
    }

    /// Returns the field at `index`.
    pub fn get(&self, index: usize) -> Option<&Field> {
        self.as_ref().get(index)
    }

    /// Finds the first field whose case-sensitive name equals `name`.
    pub fn get_by_name(&self, name: &str) -> Option<&Field> {
        self.iter().find(|field| field.name() == name)
    }

    /// Iterates in schema order without allocating.
    pub fn iter(&self) -> std::slice::Iter<'_, Field> {
        self.as_ref().iter()
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Consumes the collection and returns owned fields.
    pub fn into_fields(self) -> Vec<Field> {
        self.as_ref().to_vec()
    }

    fn from_vec(fields: Vec<Field>) -> Self {
        if fields.is_empty() {
            Self::new()
        } else {
            Self(Some(fields.into()))
        }
    }

    pub(super) fn from_imported_fields(fields: Vec<Field>) -> Result<Self> {
        reject_duplicate_field_names(&fields, "Fields")?;
        Ok(Self::from_vec(fields))
    }
}

impl AsRef<[Field]> for Fields {
    fn as_ref(&self) -> &[Field] {
        self.0.as_deref().unwrap_or_default()
    }
}

impl Deref for Fields {
    type Target = [Field];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl Index<usize> for Fields {
    type Output = Field;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_ref()[index]
    }
}

/// Subscripting a datatype reaches a nested **child**, never metadata.
///
/// The same semantic [`Field`] carries, so a caller walking a schema gets a
/// child from every node in the graph. See
/// [`Index<&str> for Field`](Field#impl-Index<%26str>-for-Field) for the rule in
/// full; [`DataType::get_field_by_name`] is the non-panicking form.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
/// assert_eq!(row["id"].data_type(), &DataType::Int64);
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this datatype has no child with that name - including when it is
/// not a nested datatype at all, which has no children by definition.
impl Index<&str> for DataType {
    type Output = Field;

    fn index(&self, name: &str) -> &Self::Output {
        self.get_field_by_name(name)
            .unwrap_or_else(|| panic!("{name:?} is not a child of the datatype {self}"))
    }
}

/// Subscripting a datatype by position reaches that nested child.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let items = DataType::list(DataType::Utf8.nullable_field("item"));
/// assert_eq!(items[0].name(), "item");
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this datatype has no child at that position.
impl Index<usize> for DataType {
    type Output = Field;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_field(index).unwrap_or_else(|| {
            panic!(
                "the datatype {self} has {} children, so position {index} is out of range",
                self.field_len()
            )
        })
    }
}

impl fmt::Debug for Fields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(formatter)
    }
}

impl Ord for Fields {
    fn cmp(&self, other: &Self) -> Ordering {
        if matches!((&self.0, &other.0), (Some(left), Some(right)) if Arc::ptr_eq(left, right)) {
            Ordering::Equal
        } else {
            cmp_field_slices(self, other)
        }
    }
}

impl PartialOrd for Fields {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl IntoIterator for Fields {
    type Item = Field;
    type IntoIter = std::vec::IntoIter<Field>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_fields().into_iter()
    }
}

impl<'a> IntoIterator for &'a Fields {
    type Item = &'a Field;
    type IntoIter = std::slice::Iter<'a, Field>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Serialize for Fields {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Fields {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = Vec::<Field>::deserialize(deserializer)?;
        Self::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

/// Union members paired with their non-negative Arrow type IDs.
#[derive(Clone, Default, Eq, PartialEq, Hash)]
pub struct UnionFields(pub(super) Option<Arc<[(i8, Field)]>>);

impl UnionFields {
    /// Builds union members and rejects duplicate or negative type IDs.
    pub fn from_fields<I>(values: I) -> Result<Self>
    where
        I: IntoIterator<Item = (i8, Field)>,
    {
        let values = values.into_iter().collect::<Vec<_>>();
        validate_union_values(&values, true)?;
        Ok(Self::from_vec(values))
    }

    /// Returns union members in their declared order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (i8, &Field)> {
        self.as_ref()
            .iter()
            .map(|(type_id, field)| (*type_id, field))
    }

    /// Returns the number of members.
    pub fn len(&self) -> usize {
        self.as_ref().len()
    }

    /// Returns whether the union has no members.
    pub fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// Returns the member at `index`.
    pub fn get(&self, index: usize) -> Option<(i8, &Field)> {
        self.as_ref().get(index).map(|(id, field)| (*id, field))
    }

    /// Finds a member by exact field name.
    pub fn get_by_name(&self, name: &str) -> Option<(i8, &Field)> {
        self.iter().find(|(_, field)| field.name() == name)
    }

    /// Returns the shared member slice without allocating.
    pub fn as_fields(&self) -> &[(i8, Field)] {
        self.as_ref()
    }

    pub(crate) fn shares_storage_with(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Consumes the collection and returns owned type-ID/field pairs.
    pub fn into_fields(self) -> Vec<(i8, Field)> {
        self.as_ref().to_vec()
    }

    fn as_ref(&self) -> &[(i8, Field)] {
        self.0.as_deref().unwrap_or_default()
    }

    fn from_vec(values: Vec<(i8, Field)>) -> Self {
        if values.is_empty() {
            Self(None)
        } else {
            Self(Some(values.into()))
        }
    }

    pub(super) fn from_imported_fields(values: Vec<(i8, Field)>) -> Result<Self> {
        validate_union_values(&values, false)?;
        Ok(Self::from_vec(values))
    }
}

impl fmt::Debug for UnionFields {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl Ord for UnionFields {
    fn cmp(&self, other: &Self) -> Ordering {
        if matches!((&self.0, &other.0), (Some(left), Some(right)) if Arc::ptr_eq(left, right)) {
            return Ordering::Equal;
        }
        let mut left = self.as_ref().iter();
        let mut right = other.as_ref().iter();
        loop {
            match (left.next(), right.next()) {
                (Some((left_id, left_field)), Some((right_id, right_field))) => {
                    let order = left_id
                        .cmp(right_id)
                        .then_with(|| cmp_fields(left_field, right_field));
                    if order != Ordering::Equal {
                        return order;
                    }
                }
                (None, None) => return Ordering::Equal,
                (None, Some(_)) => return Ordering::Less,
                (Some(_), None) => return Ordering::Greater,
            }
        }
    }
}

impl PartialOrd for UnionFields {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Index<usize> for UnionFields {
    type Output = (i8, Field);

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_ref()[index]
    }
}

impl IntoIterator for UnionFields {
    type Item = (i8, Field);
    type IntoIter = std::vec::IntoIter<(i8, Field)>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_fields().into_iter()
    }
}

impl<'a> IntoIterator for &'a UnionFields {
    type Item = (i8, &'a Field);
    type IntoIter =
        std::iter::Map<std::slice::Iter<'a, (i8, Field)>, fn(&(i8, Field)) -> (i8, &Field)>;

    fn into_iter(self) -> Self::IntoIter {
        fn borrow_member((type_id, field): &(i8, Field)) -> (i8, &Field) {
            (*type_id, field)
        }
        self.as_ref().iter().map(borrow_member)
    }
}

impl Serialize for UnionFields {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for UnionFields {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = Vec::<(i8, Field)>::deserialize(deserializer)?;
        Self::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

/// Shared dictionary key and value types.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Ord, PartialOrd, Serialize)]
pub struct DictionaryType {
    pub(super) key: DataType,
    pub(super) value: DataType,
}

impl DictionaryType {
    /// Returns the integer key type without allocating.
    pub const fn key(&self) -> &DataType {
        &self.key
    }

    /// Returns the encoded value type without allocating.
    pub const fn value(&self) -> &DataType {
        &self.value
    }
}

impl<'de> Deserialize<'de> for DictionaryType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            key: DataType,
            value: DataType,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_dictionary_key(&repr.key).map_err(serde::de::Error::custom)?;
        Ok(Self {
            key: repr.key,
            value: repr.value,
        })
    }
}

/// Shared Arrow map parameters.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct MapType {
    pub(super) entries: Field,
    pub(super) keys_sorted: bool,
}

impl MapType {
    /// Returns the non-null entries struct field without allocating.
    pub const fn entries(&self) -> &Field {
        &self.entries
    }

    /// Returns whether map keys are ordered.
    pub const fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }
}

impl Ord for MapType {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_fields(&self.entries, &other.entries)
            .then_with(|| self.keys_sorted.cmp(&other.keys_sorted))
    }
}

impl PartialOrd for MapType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'de> Deserialize<'de> for MapType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            entries: Field,
            keys_sorted: bool,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_map_entries(&repr.entries).map_err(serde::de::Error::custom)?;
        Ok(Self {
            entries: repr.entries,
            keys_sorted: repr.keys_sorted,
        })
    }
}

/// Shared run-end encoding child fields.
#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize)]
pub struct RunEndEncodedType {
    pub(super) run_ends: Field,
    pub(super) values: Field,
}

impl RunEndEncodedType {
    /// Returns the non-null signed integer run-end field.
    pub const fn run_ends(&self) -> &Field {
        &self.run_ends
    }

    /// Returns the encoded values field.
    pub const fn values(&self) -> &Field {
        &self.values
    }
}

impl Ord for RunEndEncodedType {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_fields(&self.run_ends, &other.run_ends)
            .then_with(|| cmp_fields(&self.values, &other.values))
    }
}

impl PartialOrd for RunEndEncodedType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'de> Deserialize<'de> for RunEndEncodedType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            run_ends: Field,
            values: Field,
        }
        let repr = Repr::deserialize(deserializer)?;
        validate_run_ends(&repr.run_ends).map_err(serde::de::Error::custom)?;
        Ok(Self {
            run_ends: repr.run_ends,
            values: repr.values,
        })
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

    /// Creates a struct after rejecting duplicate field names.
    pub fn from_fields<I>(fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        Ok(Self::Struct(Fields::from_fields(fields)?))
    }

    /// Creates a union after validating field names and type IDs.
    pub fn union<I>(fields: I, mode: UnionMode) -> Result<Self>
    where
        I: IntoIterator<Item = (i8, Field)>,
    {
        Ok(Self::Union(UnionFields::from_fields(fields)?, mode))
    }

    /// Creates a finite Variant as a dense union with sequential type IDs.
    ///
    /// Members retain their input order and receive IDs `0..`. The result is
    /// the canonical [`DataType::Union`] representation rather than a second
    /// logical datatype, so display, serialization, Arrow projection, and
    /// record materialization all reuse the union contract.
    pub fn variant<I>(fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        let fields = fields
            .into_iter()
            .enumerate()
            .map(|(index, field)| {
                i8::try_from(index)
                    .map(|type_id| (type_id, field))
                    .map_err(|_| {
                        invalid("Variant", "a variant cannot contain more than 128 members")
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        Self::union(fields, UnionMode::Dense)
    }

    /// Creates a dictionary and validates its integer key type.
    pub fn dictionary(key: Self, value: Self) -> Result<Self> {
        validate_dictionary_key(&key)?;
        Ok(Self::Dictionary(Arc::new(DictionaryType { key, value })))
    }

    /// Creates a map after validating Arrow's entries-field shape.
    pub fn map(entries: Field, keys_sorted: bool) -> Result<Self> {
        validate_map_entries(&entries)?;
        Ok(Self::Map(Arc::new(MapType {
            entries,
            keys_sorted,
        })))
    }

    /// Creates a map from logical key and value types using Arrow names.
    pub fn map_of(key: Self, value: Self, keys_sorted: bool) -> Result<Self> {
        Self::map(
            Field::new(
                "entries",
                Self::from_fields([
                    Field::new("key", key, false),
                    Field::new("value", value, true),
                ])?,
                false,
            ),
            keys_sorted,
        )
    }

    /// Creates a run-end encoded type after validating its run-end field.
    pub fn run_end_encoded(run_ends: Field, values: Field) -> Result<Self> {
        validate_run_ends(&run_ends)?;
        Ok(Self::RunEndEncoded(Arc::new(RunEndEncodedType {
            run_ends,
            values,
        })))
    }

    /// Returns struct children as a borrowed slice, or `None` for other types.
    pub fn as_fields(&self) -> Option<&[Field]> {
        match self {
            Self::Struct(fields) => Some(fields.as_fields()),
            _ => None,
        }
    }

    /// Returns this datatype with its direct children replaced.
    ///
    /// The layout is kept exactly - a list stays a list, a map stays a map with
    /// the same key ordering, a union keeps its type IDs and mode - and only
    /// the children change. This is the write side of [`Self::get_field`]: one
    /// generic walk can rebuild any nested datatype without a match per
    /// caller.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let list = DataType::list(DataType::Int32.nullable_field("item"));
    /// let widened = list.with_fields([DataType::Int64.nullable_field("item")])?;
    ///
    /// assert_eq!(widened, DataType::list(DataType::Int64.nullable_field("item")));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the number of children does not match the layout,
    /// or when the rebuilt datatype is not valid.
    pub fn with_fields<I>(&self, fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        let children: Vec<Field> = fields.into_iter().collect();
        let expected = self.field_len();
        if children.len() != expected {
            return Err(Error::InvalidDataType {
                kind: "DataType",
                reason: crate::text::expected_got(
                    format_args!("{expected} children for a {}", self.name()),
                    format_args!("{}", children.len()),
                ),
            });
        }
        let mut children = children.into_iter();
        let mut next = || {
            children
                .next()
                .expect("a child of the arity this layout declares")
        };
        Ok(match self {
            Self::List(_) => Self::list(next()),
            Self::ListView(_) => Self::list_view(next()),
            Self::FixedSizeList(_, length) => Self::fixed_size_list(next(), *length)?,
            Self::LargeList(_) => Self::large_list(next()),
            Self::LargeListView(_) => Self::large_list_view(next()),
            Self::Struct(_) => Self::from_fields(children)?,
            Self::Union(members, mode) => {
                let ids: Vec<i8> = members.iter().map(|(id, _)| id).collect();
                Self::union(ids.into_iter().zip(children), *mode)?
            }
            Self::Map(map) => Self::map(next(), map.keys_sorted())?,
            Self::RunEndEncoded(_) => {
                let run_ends = next();
                Self::run_end_encoded(run_ends, next())?
            }
            // A layout with no children is returned as it is, which is what
            // matching zero children against zero children means.
            scalar => scalar.clone(),
        })
    }

    /// Returns the number of direct child fields without allocating.
    pub fn field_len(&self) -> usize {
        match self {
            Self::List(_)
            | Self::ListView(_)
            | Self::FixedSizeList(..)
            | Self::LargeList(_)
            | Self::LargeListView(_)
            | Self::Map(_) => 1,
            Self::Struct(fields) => fields.len(),
            Self::Union(fields, _) => fields.len(),
            Self::RunEndEncoded(_) => 2,
            _ => 0,
        }
    }

    /// Returns a direct child field by positional index without allocating.
    pub fn get_field(&self, index: usize) -> Option<&Field> {
        match self {
            Self::List(field)
            | Self::ListView(field)
            | Self::FixedSizeList(field, _)
            | Self::LargeList(field)
            | Self::LargeListView(field) => (index == 0).then_some(field),
            Self::Struct(fields) => fields.get(index),
            Self::Union(fields, _) => fields.get(index).map(|(_, field)| field),
            Self::Map(map) => (index == 0).then_some(&map.entries),
            Self::RunEndEncoded(encoded) => match index {
                0 => Some(&encoded.run_ends),
                1 => Some(&encoded.values),
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns a direct child field by exact name without allocating.
    pub fn get_field_by_name(&self, name: &str) -> Option<&Field> {
        match self {
            Self::List(field)
            | Self::ListView(field)
            | Self::FixedSizeList(field, _)
            | Self::LargeList(field)
            | Self::LargeListView(field) => (field.name() == name).then_some(field),
            Self::Struct(fields) => fields.get_by_name(name),
            Self::Union(fields, _) => fields.get_by_name(name).map(|(_, field)| field),
            Self::Map(map) => (map.entries.name() == name).then_some(&map.entries),
            Self::RunEndEncoded(encoded) => {
                if encoded.run_ends.name() == name {
                    Some(&encoded.run_ends)
                } else if encoded.values.name() == name {
                    Some(&encoded.values)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

pub(super) fn cmp_field_slices(left: &[Field], right: &[Field]) -> Ordering {
    let mut left = left.iter();
    let mut right = right.iter();
    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) => {
                let order = cmp_fields(left, right);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

pub(super) fn cmp_fields(left: &Field, right: &Field) -> Ordering {
    left.cmp(right)
}

pub(super) fn validate_map_entries(entries: &Field) -> Result<()> {
    if entries.is_nullable() {
        return Err(invalid("Map", "entries field must be non-null"));
    }
    let DataType::Struct(children) = entries.data_type() else {
        return Err(invalid("Map", "entries field must contain a struct"));
    };
    if children.len() != 2 {
        return Err(invalid(
            "Map",
            "entries struct must contain exactly key and value fields",
        ));
    }
    if children[0].is_nullable() {
        return Err(invalid("Map", "key field must be non-null"));
    }
    Ok(())
}

pub(super) fn validate_dictionary_key(key: &DataType) -> Result<()> {
    if is_valid_dictionary_key(key) {
        Ok(())
    } else {
        Err(invalid(
            "Dictionary",
            format_smolstr!(
                "expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        ))
    }
}

pub(super) fn validate_run_ends(run_ends: &Field) -> Result<()> {
    // Two independent rules; report the one that actually fired so a caller
    // fixes the right half.
    if run_ends.is_nullable() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a non-null run_ends field, got nullable field {:?}",
                run_ends.name()
            ),
        ));
    }
    if !run_ends.data_type().is_run_ends_type() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a run_ends datatype of int16, int32, or int64, got {}",
                run_ends.data_type()
            ),
        ));
    }
    Ok(())
}

fn is_valid_dictionary_key(key: &DataType) -> bool {
    key.is_integer()
}

pub(super) fn validate_fields(fields: &[Field], kind: &'static str) -> Result<()> {
    reject_duplicate_field_names(fields, kind)?;
    fields.iter().try_for_each(Field::validate)
}

pub(super) fn validate_union_fields(fields: &UnionFields) -> Result<()> {
    validate_union_values(fields.as_fields(), true)
}

pub(super) fn validate_union_values(values: &[(i8, Field)], validate_children: bool) -> Result<()> {
    let mut seen = 0_u128;
    for (index, (type_id, field)) in values.iter().enumerate() {
        if *type_id < 0 {
            return Err(invalid(
                "Union",
                format_smolstr!("type id must be non-negative: {type_id}"),
            ));
        }
        let mask = 1_u128 << *type_id;
        if seen & mask != 0 {
            return Err(invalid(
                "Union",
                format_smolstr!("duplicate type id: {type_id}"),
            ));
        }
        seen |= mask;
        if values[..index]
            .iter()
            .any(|(_, previous)| previous.name() == field.name())
        {
            return Err(invalid(
                "Union",
                format_smolstr!("duplicate field name {:?}", field.name()),
            ));
        }
        if validate_children {
            field.validate()?;
        }
    }
    Ok(())
}

pub(super) fn reject_duplicate_field_names(fields: &[Field], kind: &'static str) -> Result<()> {
    const HASHED_DUPLICATE_CHECK_THRESHOLD: usize = 16;

    if fields.len() > HASHED_DUPLICATE_CHECK_THRESHOLD {
        let mut names = HashSet::with_capacity(fields.len());
        for field in fields {
            if !names.insert(field.name()) {
                return Err(invalid(
                    kind,
                    format_smolstr!("duplicate field name {:?}", field.name()),
                ));
            }
        }
        return Ok(());
    }

    for (index, field) in fields.iter().enumerate() {
        if fields[..index]
            .iter()
            .any(|previous| previous.name() == field.name())
        {
            return Err(invalid(
                kind,
                format_smolstr!("duplicate field name {:?}", field.name()),
            ));
        }
    }
    Ok(())
}
