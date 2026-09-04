//! Shared child collections and validated nested datatype construction.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::ops::Index;
use std::sync::Arc;

use ::serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::types::{invalid, validate_non_negative};
use crate::{DataType, Error, Field, Result, UnionMode};

use super::fields::{FieldKey, Fields};

/// Union members paired with their non-negative Arrow type IDs.
#[derive(Clone, Default, Eq, PartialEq, Hash)]
pub struct UnionFields(pub(crate) Option<Arc<[(i8, Field)]>>);

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

    pub(crate) fn from_imported_fields(values: Vec<(i8, Field)>) -> Result<Self> {
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
    pub(crate) key: DataType,
    pub(crate) value: DataType,
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
    pub(crate) entries: Field,
    pub(crate) keys_sorted: bool,
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
    pub(crate) run_ends: Field,
    pub(crate) values: Field,
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
    /// Creates the self-describing semi-structured Variant type.
    ///
    /// It takes no parameters: shredding is physical layout, while each value
    /// is the ordinary [`crate::Scalar`] tree. Parentheses distinguish the
    /// finite [`Self::dense_union`] input form in the grammar.
    #[must_use]
    pub const fn variant() -> Self {
        Self::Variant
    }

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

    /// Creates a finite dense union with sequential type IDs.
    ///
    /// Members retain their input order and receive IDs `0..`. The result is
    /// the canonical [`DataType::Union`] representation rather than a second
    /// logical datatype, so display, serialization, Arrow projection, and
    /// record materialization all reuse the union contract.
    ///
    /// This used to be spelled `variant`, after the input sugar the parser
    /// still accepts - `variant(a: int32, b: utf8)` remains that sugar, and
    /// its canonical display remains `union(dense, ...)`. The name moved
    /// because bare `variant` is now a datatype of its own - the
    /// self-describing semi-structured value Iceberg v3, Parquet, and Doris
    /// share - and one word cannot name both a union of declared members and
    /// a value that declares itself.
    pub fn dense_union<I>(fields: I) -> Result<Self>
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

    /// Returns a direct child field by position without allocating.
    pub fn get_field_at(&self, index: usize) -> Option<&Field> {
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
    ///
    /// This is the step [`Self::get_field_by_path`] takes before it decomposes
    /// anything, and the whole of it for a name carrying no dot.
    fn get_field_by_name(&self, name: &str) -> Option<&Field> {
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

    /// Returns the item field of a list-shaped datatype.
    ///
    /// The five list layouts hold exactly one child, and a dotted path treats
    /// that child as a step it need not spell; this is the one place they are
    /// recognized as such.
    fn list_item(&self) -> Option<&Field> {
        match self {
            Self::List(field)
            | Self::ListView(field)
            | Self::FixedSizeList(field, _)
            | Self::LargeList(field)
            | Self::LargeListView(field) => Some(field),
            _ => None,
        }
    }

    /// Returns a nested child by path, preferring an exact name at every step.
    ///
    /// A child carrying the whole string as its own name wins outright, so a
    /// name containing a dot stays reachable. Only when nothing carries the
    /// whole string is it decomposed: each `.` is tried as a boundary, left to
    /// right, and a head that names a child is descended into. A descent that
    /// finds nothing falls back to the next boundary, so `"a.b.c"` resolves
    /// even when `a.b` is one child carrying `c`.
    ///
    /// A list-shaped datatype - `List`, `LargeList`, `FixedSizeList`,
    /// `ListView`, `LargeListView` - is transparent to a path: a segment is
    /// matched against the item's own name first, and otherwise resolved
    /// against the item's children, so `orders.price` reaches the price of an
    /// `array<struct>` item the way `orders.item.price` does. A map is not
    /// transparent; its one child is the entries field, addressed by name.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([
    ///     DataType::from_fields([DataType::Float64.required_field("price")])?
    ///         .required_field("line"),
    /// ])?;
    /// assert_eq!(row.get_field_by_path("line.price").unwrap().name(), "price");
    ///
    /// // The whole string first: a dotted name is a name, not a path.
    /// let dotted = DataType::from_fields([DataType::Int64.required_field("a.b")])?;
    /// assert_eq!(dotted.get_field_by_path("a.b").unwrap().name(), "a.b");
    ///
    /// // A list is transparent: its item is a step the path need not spell.
    /// let orders = DataType::from_str("struct<orders:array<struct<price:double>>>")?;
    /// assert_eq!(orders.get_field_by_path("orders.price").unwrap().name(), "price");
    /// assert_eq!(orders.get_field_by_path("orders.item.price").unwrap().name(), "price");
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field_by_path(&self, path: &str) -> Option<&Field> {
        if let Some(field) = self.get_field_by_name(path) {
            return Some(field);
        }
        let mut offset = 0;
        while let Some(at) = path[offset..].find('.') {
            let boundary = offset + at;
            if let Some(child) = self.get_field_by_name(&path[..boundary]) {
                if let Some(found) = child.get_field_by_path(&path[boundary + 1..]) {
                    return Some(found);
                }
            }
            offset = boundary + 1;
        }
        // Nothing here carries the path, so a list hands it to its item: the
        // item's own name was already tried above, and this is the rest of it.
        self.list_item()
            .and_then(|item| item.get_field_by_path(path))
    }

    /// Returns a nested child by position or by path.
    ///
    /// The one lookup a caller reaches for when the key is whichever the data
    /// gave them: an integer is a position, a string is a path.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
    /// assert_eq!(row.get_field(0).unwrap().name(), "id");
    /// assert_eq!(row.get_field("id").unwrap().name(), "id");
    /// assert!(row.get_field("absent").is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&Field> {
        match key.into() {
            FieldKey::Index(index) => self.get_field_at(index),
            FieldKey::Path(path) => self.get_field_by_path(path),
        }
    }

    /// Returns a nested child by position, naming what is there when it is not.
    ///
    /// # Errors
    ///
    /// Returns an error when this datatype has no child at that position,
    /// including when it has no children at all.
    pub fn field_at(&self, index: usize) -> Result<&Field> {
        self.get_field_at(index)
            .ok_or_else(|| Error::InvalidRecord {
                path: format_smolstr!("$[{index}]"),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", self.field_len()),
                    format_smolstr!("{index}"),
                ),
            })
    }

    /// Returns a nested child by path, naming what is there when it is not.
    ///
    /// # Errors
    ///
    /// Returns an error when no child carries that name and no decomposition
    /// of it resolves.
    pub fn field_by_path(&self, path: &str) -> Result<&Field> {
        self.get_field_by_path(path)
            .ok_or_else(|| missing_child(self, path))
    }

    /// Returns a nested child by position or by path, raising when absent.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::field_at`] or [`Self::field_by_path`] raises,
    /// whichever the key selects.
    pub fn field<'key>(&self, key: impl Into<FieldKey<'key>>) -> Result<&Field> {
        match key.into() {
            FieldKey::Index(index) => self.field_at(index),
            FieldKey::Path(path) => self.field_by_path(path),
        }
    }

    /// Replaces the child at `index`, keeping the layout.
    ///
    /// A position replaces only: it never grows the node, which is what
    /// distinguishes it from [`Self::set_field_by_path`].
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from_fields([DataType::Int64.required_field("id")])?;
    /// row.set_field_at(0, DataType::Utf8.required_field("id"))?;
    ///
    /// assert_eq!(row["id"].dtype(), &DataType::Utf8);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end or the rebuilt datatype
    /// does not validate. Failure leaves `self` unchanged.
    pub fn set_field_at(&mut self, index: usize, child: Field) -> Result<()> {
        if index >= self.field_len() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$[{index}]"),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", self.field_len()),
                    format_smolstr!("{index}"),
                ),
            });
        }
        let mut children = self.children();
        children[index] = child;
        *self = self.with_fields(children)?;
        Ok(())
    }

    /// Replaces the child `path` resolves to, appending an unresolved name.
    ///
    /// Resolution is [`Self::get_field_by_path`]'s, so a reader and a writer
    /// never disagree about which child one string names: a child carrying the
    /// whole string is replaced in place, otherwise a resolving head is
    /// descended into and the remainder set there. A string that resolves to
    /// nothing appends one child under that name - which is how a struct gets
    /// built up - so a path whose parents do not exist appends a single child
    /// rather than conjuring the chain.
    ///
    /// The one difference from the reader is deliberate: a list is not
    /// transparent to a write. Reading `orders.price` may reach into a list's
    /// item, but replacing or removing a child is a change to the node that
    /// holds it, so a write addresses the item by its own name -
    /// `orders.item.price` - and a list never grows a second child.
    ///
    /// The child is stored under the name the path ends in, whatever it calls
    /// itself.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from_fields([
    ///     DataType::from_fields([DataType::Int32.required_field("price")])?
    ///         .required_field("line"),
    /// ])?;
    ///
    /// row.set_field_by_path("line.price", DataType::Float64.required_field("price"))?;
    /// assert_eq!(row["line"]["price"].dtype(), &DataType::Float64);
    ///
    /// // An unresolved name appends.
    /// row.set_field_by_path("venue", DataType::Utf8.nullable_field("venue"))?;
    /// assert_eq!(row.field_len(), 2);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when a resolved parent cannot hold children, when an
    /// append lands on a layout with fixed arity, or when the rebuilt datatype
    /// does not validate. Failure leaves `self` unchanged.
    pub fn set_field_by_path(&mut self, path: &str, child: Field) -> Result<()> {
        // Whole string first, exactly as the reader resolves it.
        if let Some(index) = self.index_of_name(path) {
            let mut children = self.children();
            children[index] = child.with_name(path);
            *self = self.with_fields(children)?;
            return Ok(());
        }
        let mut offset = 0;
        while let Some(at) = path[offset..].find('.') {
            let boundary = offset + at;
            let head = &path[..boundary];
            let rest = &path[boundary + 1..];
            if let Some(index) = self.index_of_name(head) {
                let mut children = self.children();
                if children[index]
                    .set_field_by_path(rest, child.clone())
                    .is_ok()
                {
                    *self = self.with_fields(children)?;
                    return Ok(());
                }
            }
            offset = boundary + 1;
        }
        // Nothing resolved, so the whole string names a new child here.
        let mut children = self.require_struct_children()?;
        children.push(child.with_name(path));
        *self = Self::from_fields(children)?;
        Ok(())
    }

    /// Replaces a child by position or by path.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::set_field_at`] or [`Self::set_field_by_path`]
    /// raises, whichever the key selects.
    pub fn set_field<'key>(&mut self, key: impl Into<FieldKey<'key>>, child: Field) -> Result<()> {
        match key.into() {
            FieldKey::Index(index) => self.set_field_at(index, child),
            FieldKey::Path(path) => self.set_field_by_path(path, child),
        }
    }

    /// Removes the child at `index`, returning it and closing the gap.
    ///
    /// # Errors
    ///
    /// Returns an error when `index` is past the end, when the layout has a
    /// fixed arity that removal would break, or when the rebuilt datatype does
    /// not validate. Failure leaves `self` unchanged.
    pub fn remove_field_at(&mut self, index: usize) -> Result<Field> {
        if index >= self.field_len() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$[{index}]"),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", self.field_len()),
                    format_smolstr!("{index}"),
                ),
            });
        }
        let mut children = self.require_struct_children()?;
        let removed = children.remove(index);
        *self = Self::from_fields(children)?;
        Ok(removed)
    }

    /// Removes the child `path` resolves to, returning it.
    ///
    /// # Errors
    ///
    /// Returns an error when the path resolves to no child, or when the
    /// rebuilt datatype does not validate. Failure leaves `self` unchanged.
    pub fn remove_field_by_path(&mut self, path: &str) -> Result<Field> {
        if let Some(index) = self.index_of_name(path) {
            return self.remove_field_at(index);
        }
        let mut offset = 0;
        while let Some(at) = path[offset..].find('.') {
            let boundary = offset + at;
            if let Some(index) = self.index_of_name(&path[..boundary]) {
                let mut children = self.children();
                if let Ok(removed) = children[index].remove_field_by_path(&path[boundary + 1..]) {
                    *self = self.with_fields(children)?;
                    return Ok(removed);
                }
            }
            offset = boundary + 1;
        }
        Err(missing_child(self, path))
    }

    /// Removes a child by position or by path, returning it.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::remove_field_at`] or
    /// [`Self::remove_field_by_path`] raises, whichever the key selects.
    pub fn remove_field<'key>(&mut self, key: impl Into<FieldKey<'key>>) -> Result<Field> {
        match key.into() {
            FieldKey::Index(index) => self.remove_field_at(index),
            FieldKey::Path(path) => self.remove_field_by_path(path),
        }
    }

    /// Returns every leaf under this node, named by its dotted path.
    ///
    /// Struct nesting is flattened all the way down: a child that is a struct
    /// contributes its own leaves rather than itself, each named by the path
    /// that reaches it. Every name this returns is one
    /// [`Self::get_field_by_path`] resolves, so a flattened column list and
    /// the tree it came from address children the same way.
    ///
    /// A leaf under a nullable ancestor is nullable, because a null parent
    /// leaves the leaf with no value to carry.
    ///
    /// Collections are leaves here: a `list` or a `map` contributes itself,
    /// not its element. Unnesting answers what a flat column list looks like,
    /// and a list is one column; [`Self::explode_fields`] is what reaches
    /// inside one.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::from_fields([DataType::Float64.required_field("px")])?
    ///         .nullable_field("line"),
    /// ])?;
    ///
    /// let leaves = row.unnest_fields();
    /// let names: Vec<&str> = leaves.iter().map(|field| field.name()).collect();
    /// assert_eq!(names, ["id", "line.px"]);
    ///
    /// // The nullable parent makes its leaf nullable.
    /// assert!(leaves[1].is_nullable());
    /// assert_eq!(row.get_field_by_path("line.px").unwrap().name(), "px");
    /// # Ok(())
    /// # }
    /// ```
    pub fn unnest_fields(&self) -> Vec<Field> {
        let mut leaves = Vec::with_capacity(self.field_len());
        self.push_leaves("", false, &mut leaves);
        leaves
    }

    /// Collect this node's leaves under an accumulated path.
    fn push_leaves(&self, prefix: &str, nullable: bool, leaves: &mut Vec<Field>) {
        for index in 0..self.field_len() {
            let Some(child) = self.get_field_at(index) else {
                continue;
            };
            let path = if prefix.is_empty() {
                child.name().to_owned()
            } else {
                format!("{prefix}.{}", child.name())
            };
            let nullable = nullable || child.is_nullable();
            match child.dtype().as_fields() {
                // A struct contributes its leaves; anything else is one.
                Some(_) => child.dtype().push_leaves(&path, nullable, leaves),
                None => {
                    let mut leaf = child.clone().with_name(path);
                    leaf.set_nullable(nullable);
                    leaves.push(leaf);
                }
            }
        }
    }

    /// Returns this node's children with every collection replaced by what it
    /// holds.
    ///
    /// A list answers its item, a map its entries, and a dictionary or run-end
    /// node the values it encodes. A child that is not a collection is
    /// returned unchanged, so the result always names the same columns in the
    /// same order - one row's worth of a table whose collections have been
    /// expanded.
    ///
    /// The column keeps its own name rather than the element's, because
    /// exploding does not rename a column, and it is nullable when either the
    /// collection or its element is: an absent list yields no element.
    ///
    /// Only one level is unwrapped, so a list of lists answers a list. Calling
    /// it again reaches the next one, which is what makes the depth the
    /// caller's decision rather than this method's.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
    /// ])?;
    ///
    /// let exploded = row.explode_fields();
    /// assert_eq!(exploded[0].dtype(), &DataType::Int64, "not a collection, unchanged");
    /// assert_eq!(exploded[1].name(), "levels", "the column keeps its name");
    /// assert_eq!(exploded[1].dtype(), &DataType::Float64, "and answers its item");
    /// # Ok(())
    /// # }
    /// ```
    pub fn explode_fields(&self) -> Vec<Field> {
        (0..self.field_len())
            .filter_map(|index| self.get_field_at(index))
            .map(exploded)
            .collect()
    }

    /// Returns the struct children as an owned vector, or a refusal.
    ///
    /// Appending and removing change the child count, and a struct is the only
    /// layout whose arity is not fixed by what it is: a list holds exactly one
    /// child, a run-end node exactly two. Rebuilding one of those through
    /// [`Self::from_fields`] would silently make it a struct, so this refuses
    /// instead.
    fn require_struct_children(&self) -> Result<Vec<Field>> {
        match self.as_fields() {
            Some(fields) => Ok(fields.to_vec()),
            None => Err(Error::InvalidRecord {
                path: SmolStr::new_static("$"),
                reason: crate::text::expected_got(
                    "a struct field whose children can be added or removed",
                    format_smolstr!("{self}"),
                ),
            }),
        }
    }

    /// Returns the direct children as an owned vector, over every layout.
    fn children(&self) -> Vec<Field> {
        (0..self.field_len())
            .filter_map(|index| self.get_field_at(index))
            .cloned()
            .collect()
    }

    /// Returns the position of the direct child with an exact name.
    fn index_of_name(&self, name: &str) -> Option<usize> {
        (0..self.field_len())
            .find(|index| self.get_field_at(*index).is_some_and(|f| f.name() == name))
    }
}

/// One child with its collection, if it is one, replaced by what it holds.
fn exploded(child: &Field) -> Field {
    let held = match child.dtype() {
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::FixedSizeList(item, _)
        | DataType::LargeList(item)
        | DataType::LargeListView(item) => Some((item.dtype().clone(), item.is_nullable())),
        DataType::Map(map) => Some((map.entries().dtype().clone(), map.entries().is_nullable())),
        DataType::RunEndEncoded(encoded) => Some((
            encoded.values().dtype().clone(),
            encoded.values().is_nullable(),
        )),
        DataType::Dictionary(dictionary) => Some((dictionary.value().clone(), false)),
        _ => None,
    };
    match held {
        Some((dtype, element_nullable)) => {
            let mut exploded =
                Field::new(child.name(), dtype, child.is_nullable() || element_nullable);
            // The column's own annotations describe the column, not the
            // collection layout, so they survive the expansion.
            let _ = exploded.set_metadata(child.metadata_iter());
            exploded
        }
        None => child.clone(),
    }
}

/// Report a path that names no child, and the names that exist beside it.
fn missing_child(node: &DataType, path: &str) -> Error {
    let names: Vec<&str> = (0..node.field_len())
        .filter_map(|index| node.get_field_at(index))
        .map(Field::name)
        .collect();
    Error::InvalidRecord {
        path: format_smolstr!("$.{path}"),
        reason: crate::text::expected_got(
            format_smolstr!("a child among {names:?}"),
            format_smolstr!("{path:?}"),
        ),
    }
}

pub(crate) fn cmp_field_slices(left: &[Field], right: &[Field]) -> Ordering {
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

pub(crate) fn cmp_fields(left: &Field, right: &Field) -> Ordering {
    left.cmp(right)
}

pub(crate) fn validate_map_entries(entries: &Field) -> Result<()> {
    if entries.is_nullable() {
        return Err(invalid("Map", "entries field must be non-null"));
    }
    let DataType::Struct(children) = entries.dtype() else {
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

pub(crate) fn validate_dictionary_key(key: &DataType) -> Result<()> {
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

pub(crate) fn validate_run_ends(run_ends: &Field) -> Result<()> {
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
    if !run_ends.dtype().is_run_ends_type() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a run_ends datatype of int16, int32, or int64, got {}",
                run_ends.dtype()
            ),
        ));
    }
    Ok(())
}

fn is_valid_dictionary_key(key: &DataType) -> bool {
    key.is_integer()
}

pub(crate) fn validate_fields(fields: &[Field], kind: &'static str) -> Result<()> {
    reject_duplicate_field_names(fields, kind)?;
    fields.iter().try_for_each(Field::validate)
}

pub(crate) fn validate_union_fields(fields: &UnionFields) -> Result<()> {
    validate_union_values(fields.as_fields(), true)
}

pub(crate) fn validate_union_values(values: &[(i8, Field)], validate_children: bool) -> Result<()> {
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

pub(crate) fn reject_duplicate_field_names(fields: &[Field], kind: &'static str) -> Result<()> {
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
