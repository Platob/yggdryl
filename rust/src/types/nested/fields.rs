//! Nested, dictionary, map, and run-end encoded field markers.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Deref, Index};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::format_smolstr;

use crate::metadata::FIELD_PARTITION_KEY;
use crate::types::typed::define_field_types;
use crate::{DataType, Error, Field, Result, TypedField};

use super::dtypes::{cmp_field_slices, reject_duplicate_field_names, validate_fields};

/// Either of the two ways a caller names one child.
///
/// [`From`] carries every spelling a caller reaches for, so `field(0)` and
/// `field("line.price")` are one call rather than two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKey<'a> {
    /// A zero-based position among the direct children.
    Index(usize),
    /// A child name, or a dotted path through nested children.
    Path(&'a str),
}

impl From<usize> for FieldKey<'_> {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl<'a> From<&'a str> for FieldKey<'a> {
    fn from(path: &'a str) -> Self {
        Self::Path(path)
    }
}

impl<'a> From<&'a String> for FieldKey<'a> {
    fn from(path: &'a String) -> Self {
        Self::Path(path.as_str())
    }
}

/// An ordered, immutable collection of fields stored in one shared allocation.
#[derive(Clone, Default, Eq, PartialEq, Hash)]
pub struct Fields(pub(crate) Option<Arc<[Field]>>);

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

    pub(crate) fn from_imported_fields(fields: Vec<Field>) -> Result<Self> {
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
/// child from every node in the graph. The string is resolved by
/// [`DataType::get_field_by_path`] - an exact name first, a dotted path after -
/// and that method is the non-panicking form.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
/// assert_eq!(row["id"].dtype(), &DataType::Int64);
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

    fn index(&self, path: &str) -> &Self::Output {
        self.get_field_by_path(path)
            .unwrap_or_else(|| panic!("{path:?} is not a child of the datatype {self}"))
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
        self.get_field_at(index).unwrap_or_else(|| {
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

impl DataType {
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
}

impl Field {
    /// Returns whether this field is a struct, and therefore usable as a
    /// record schema root.
    pub fn is_struct(&self) -> bool {
        self.dtype.as_fields().is_some()
    }

    /// Returns the struct children of this field, or an empty slice.
    ///
    /// A struct `Field` is the schema of the rows it describes, so this is the
    /// column list every interop layer projects from.
    pub fn fields(&self) -> &[Field] {
        self.dtype.as_fields().unwrap_or_default()
    }

    /// Returns the number of struct children.
    pub fn field_len(&self) -> usize {
        self.dtype.field_len()
    }

    /// Returns one nested child by position.
    pub fn get_field_at(&self, index: usize) -> Option<&Field> {
        self.dtype.get_field_at(index)
    }

    /// Returns one nested child by path, an exact name first.
    ///
    /// [`DataType::get_field_by_path`] carries the rule, including the one
    /// that makes a list transparent - `orders.price` reaches the price of an
    /// `array<struct>` item; this node's datatype is where it starts.
    pub fn get_field_by_path(&self, path: &str) -> Option<&Field> {
        self.dtype.get_field_by_path(path)
    }

    /// Returns one nested child by position or by path.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = DataType::from_fields([
    ///     DataType::from_fields([DataType::Float64.required_field("price")])?
    ///         .required_field("line"),
    /// ])?
    /// .required_field("order");
    ///
    /// assert_eq!(order.get_field(0).unwrap().name(), "line");
    /// assert_eq!(order.get_field("line.price").unwrap().name(), "price");
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&Field> {
        self.dtype.get_field(key)
    }

    /// Returns one nested child by position, naming what is there when absent.
    ///
    /// # Errors
    ///
    /// Returns an error when this node has no child at that position.
    pub fn field_at(&self, index: usize) -> Result<&Field> {
        self.dtype.field_at(index)
    }

    /// Returns one nested child by path, naming what is there when absent.
    ///
    /// # Errors
    ///
    /// Returns an error when no child carries that name and no decomposition
    /// of it resolves.
    pub fn field_by_path(&self, path: &str) -> Result<&Field> {
        self.dtype.field_by_path(path)
    }

    /// Returns one nested child by position or by path, raising when absent.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::field_at`] or [`Self::field_by_path`] raises,
    /// whichever the key selects.
    pub fn field<'key>(&self, key: impl Into<FieldKey<'key>>) -> Result<&Field> {
        self.dtype.field(key)
    }

    /// Returns the position of the first struct child with an exact name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.fields().iter().position(|field| field.name() == name)
    }

    /// Returns the field that describes both this one and `other`.
    ///
    /// The datatype is [`DataType::merge_with`]'s answer, so every promotion
    /// rule lives in one place and this adds only what a field carries beyond
    /// a type:
    ///
    /// * the **name** is this field's, because a merge answers in the
    ///   receiver's vocabulary; struct children are paired by name, so the two
    ///   already agree wherever it matters;
    /// * the result is **nullable** when either side is, since a value absent
    ///   from one of two sources is absent from their union;
    /// * **metadata** is the union of both, and this field wins a key they
    ///   disagree on. Reserved keys stay validated by the same path every
    ///   other write uses, so a merge cannot assemble a field that would have
    ///   been refused outright;
    /// * **dictionary options** survive only where both sides encode, for the
    ///   reason a dictionary does not survive a merge with a plain column.
    ///
    /// ```
    /// use yggdryl::{DataType, Field};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let narrow = Field::new("price", DataType::Int32, false);
    /// let wide = Field::new("price", DataType::Int64, true);
    ///
    /// let merged = narrow.merge_with(&wide, true)?;
    /// assert_eq!(merged.dtype(), &DataType::Int64);
    /// assert!(merged.is_nullable(), "either side being nullable carries over");
    ///
    /// // The other direction meets at the tightest type naming both.
    /// assert_eq!(narrow.merge_with(&wide, false)?.dtype(), &DataType::Int32);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when the two datatypes have no meeting point, or when
    /// the merged metadata does not validate. Failure leaves both fields
    /// untouched.
    pub fn merge_with(&self, other: &Self, upscale: bool) -> Result<Self> {
        self.merge(
            other,
            crate::types::Widening::upscale(upscale),
            crate::types::Recode::Allowed,
        )
    }

    /// The recursive worker behind [`Self::merge_with`], shared with the
    /// datatype merge so nested children never take a different path.
    pub(crate) fn merge(
        &self,
        other: &Self,
        how: crate::types::Widening,
        recode: crate::types::Recode,
    ) -> Result<Self> {
        let dtype = self.dtype.merge(&other.dtype, how, recode)?;
        let mut merged = Self::new(self.name.clone(), dtype, self.nullable || other.nullable);
        // One rule, on `Metadata` itself: the union of both, this field
        // winning any key they disagree on.
        merged.set_metadata(self.metadata.merge_with(&other.metadata)?.iter())?;
        if self.dictionary_id != 0 && other.dictionary_id != 0 {
            merged.set_dictionary_options(
                self.dictionary_id,
                self.dictionary_is_ordered && other.dictionary_is_ordered,
            )?;
        }
        Ok(merged)
    }

    /// Returns every leaf under this field, named by its dotted path.
    ///
    /// [`DataType::unnest_fields`] carries the rule; this node's datatype is
    /// where it starts.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::from_fields([DataType::Float64.required_field("px")])?
    ///         .nullable_field("line"),
    /// ])?
    /// .required_field("row");
    ///
    /// let leaves = row.unnest_fields();
    /// let names: Vec<&str> = leaves.iter().map(|field| field.name()).collect();
    /// assert_eq!(names, ["id", "line.px"]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn unnest_fields(&self) -> Vec<Self> {
        self.dtype.unnest_fields()
    }

    /// Returns this field's children with every collection replaced by what it
    /// holds.
    ///
    /// [`DataType::explode_fields`] carries the rule.
    pub fn explode_fields(&self) -> Vec<Self> {
        self.dtype.explode_fields()
    }

    /// Returns this struct root without the named children.
    ///
    /// Names it does not carry are ignored, so a caller can subtract a set
    /// without checking it first. This is what a partitioned write stores: the
    /// schema minus the columns the path already spells out.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from_fields([
    ///     DataType::Int64.required_field("price"),
    ///     DataType::Int32.required_field("year"),
    /// ])?
    /// .required_field("row");
    ///
    /// let stored = schema.without_fields(&["year"])?;
    /// assert_eq!(stored.field_len(), 1);
    /// assert_eq!(stored.name(), "row");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this is not a struct, or when removing the names
    /// would leave a datatype that is not valid.
    pub fn without_fields(&self, names: &[&str]) -> Result<Self> {
        self.require_struct()?;
        let kept: Vec<Self> = self
            .fields()
            .iter()
            .filter(|field| !names.contains(&field.name()))
            .cloned()
            .collect();
        // The root's metadata describes the rows, not the columns, so it stays.
        Self::from_parts(
            self.name(),
            DataType::from_fields(kept)?,
            self.is_nullable(),
            self.metadata_iter(),
        )
    }

    /// Returns whether this field carries the values a path spells out.
    ///
    /// A partition field is an ordinary field with the reserved
    /// `field:partition` marker set. Nothing in a batch says which of its
    /// columns belong in a directory name, so a schema that means to be stored
    /// partitioned has to say so, and this is where it says it. Every
    /// constructor canonicalizes the marker, so an absent one and an explicit
    /// `false` both read as "not a partition field".
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// let year = DataType::Int32.required_field("year").with_partition(true);
    ///
    /// assert!(year.is_partition());
    /// assert!(!DataType::Int64.required_field("price").is_partition());
    /// ```
    pub fn is_partition(&self) -> bool {
        self.get_metadata(FIELD_PARTITION_KEY) == Some("true")
    }

    /// Returns the struct children that partition the rows.
    ///
    /// The iterator borrows the children in declaration order, which is also
    /// the order their directories nest in a path.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from_fields([
    ///     DataType::Int32.required_field("year").with_partition(true),
    ///     DataType::Int64.required_field("price"),
    /// ])?
    /// .required_field("row");
    ///
    /// assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year"]);
    /// assert_eq!(schema.partition_field_len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    pub fn partition_fields(&self) -> PartitionFields<'_> {
        PartitionFields(self.fields().iter())
    }

    /// Returns the names of the struct children that partition the rows.
    pub fn partition_field_names(&self) -> PartitionFieldNames<'_> {
        PartitionFieldNames(self.partition_fields())
    }

    /// Returns how many struct children partition the rows.
    pub fn partition_field_len(&self) -> usize {
        self.partition_fields().count()
    }

    /// Returns whether any struct child partitions the rows.
    pub fn has_partition_fields(&self) -> bool {
        self.partition_fields().next().is_some()
    }

    /// Returns this struct root holding only the columns a path spells out.
    ///
    /// This is the tuple a partitioned layout carries in its directory names,
    /// and the complement of [`Self::without_partition_fields`].
    ///
    /// # Errors
    ///
    /// Returns an error when this is not a struct, or when the remaining
    /// children do not form a valid datatype.
    pub fn only_partition_fields(&self) -> Result<Self> {
        self.require_struct()?;
        let kept: Vec<Self> = self.partition_fields().cloned().collect();
        Self::from_parts(
            self.name(),
            DataType::from_fields(kept)?,
            self.is_nullable(),
            self.metadata_iter(),
        )
    }

    /// Returns this struct root without the columns a path spells out.
    ///
    /// This is what a partitioned write stores in a leaf: the declared schema
    /// minus the columns the directory names already carry.
    ///
    /// # Errors
    ///
    /// Returns an error when this is not a struct, or when removing the
    /// partition children would leave a datatype that is not valid.
    pub fn without_partition_fields(&self) -> Result<Self> {
        self.require_struct()?;
        let names: Vec<&str> = self.partition_field_names().collect();
        if names.is_empty() {
            // Subtracting nothing is the field itself, and a clone of a field
            // shares its metadata, children, and populated Arrow projection.
            return Ok(self.clone());
        }
        self.without_fields(&names)
    }

    /// Returns this struct root with the named children marked as partitions.
    ///
    /// A name this root does not carry is an error rather than a silent
    /// omission: a partition column nobody stores is a layout the writer would
    /// have produced without ever saying which column went missing.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from_fields([
    ///     DataType::Int32.required_field("year"),
    ///     DataType::Int64.required_field("price"),
    /// ])?
    /// .required_field("row")
    /// .with_partition_fields(&["year"])?;
    ///
    /// assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year"]);
    /// assert_eq!(schema.without_partition_fields()?.field_len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this is not a struct or a name is not one of its
    /// children.
    pub fn with_partition_fields(&self, names: &[&str]) -> Result<Self> {
        self.require_struct()?;
        for name in names {
            if self.get_field_by_path(name).is_none() {
                return Err(Error::InvalidRecord {
                    path: format_smolstr!("$.{name}"),
                    reason: crate::text::expected_got(
                        format_args!("a column of {:?} to partition on", self.name()),
                        format_args!("{name:?}"),
                    ),
                });
            }
        }
        let children: Vec<Self> = self
            .fields()
            .iter()
            .map(|child| {
                let partition = names.contains(&child.name());
                if partition == child.is_partition() {
                    child.clone()
                } else {
                    child.clone().with_partition(partition)
                }
            })
            .collect();
        Self::from_parts(
            self.name(),
            DataType::from_fields(children)?,
            self.is_nullable(),
            self.metadata_iter(),
        )
    }

    /// Replaces the struct child at `index`, cache-aware.
    ///
    /// Replacement only: a position past the end is an error rather than a
    /// silent append, because a position is not a name. The whole child set is
    /// revalidated and a populated Arrow cache is invalidated exactly once,
    /// through [`Self::set_dtype`] - which is why child mutation is a named
    /// method here and not an `IndexMut`. Handing out a `&mut Field` would both
    /// bypass that invalidation and force `Arc::make_mut` to clone the shared
    /// child array on every subscript assignment.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from_fields([DataType::Int64.required_field("id")])?
    ///     .required_field("row");
    ///
    /// row.set_field_at(0, DataType::Utf8.required_field("id"))?;
    /// assert_eq!(row["id"].dtype(), &DataType::Utf8);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a struct, when `index` is past
    /// the end, or when the resulting child set does not validate. Failure
    /// leaves `self` unchanged.
    pub fn set_field_at(&mut self, index: usize, child: Self) -> Result<()> {
        let mut fields = self.struct_children()?;
        if index >= fields.len() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}[{index}]", self.name),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", fields.len()),
                    format_smolstr!("{index}"),
                ),
            });
        }
        fields[index] = child;
        self.set_dtype(DataType::from_fields(fields)?)
    }

    /// Replaces the struct child named `name`, appending an unknown one.
    ///
    /// Dict-like on purpose, and the asymmetry with [`Self::set_field`] is
    /// deliberate: a known name is replaced *in place*, keeping its position,
    /// and an unknown name appends a new child - which is the natural way to
    /// build a schema up. A position, by contrast, only ever replaces.
    ///
    /// The child is stored under `name` whatever it calls itself, so
    /// `row.set_field_by_path("price", DataType::Float64.required_field("x"))`
    /// stores a child named `price`.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from_fields([DataType::Int64.required_field("id")])?
    ///     .required_field("row");
    ///
    /// // An unknown name appends.
    /// row.set_field_by_path("venue", DataType::Utf8.nullable_field("venue"))?;
    /// assert_eq!(row.field_len(), 2);
    ///
    /// // A known one replaces, keeping its position.
    /// row.set_field_by_path("id", DataType::Utf8.required_field("id"))?;
    /// assert_eq!(row.field_len(), 2);
    /// assert_eq!(row[0].name(), "id");
    /// assert_eq!(row["id"].dtype(), &DataType::Utf8);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a struct or the resulting child
    /// set does not validate. Failure leaves `self` unchanged.
    pub fn set_field_by_path(&mut self, path: &str, child: Self) -> Result<()> {
        let mut dtype = self.dtype.clone();
        dtype.set_field_by_path(path, child)?;
        self.set_dtype(dtype)
    }

    /// Replaces a nested child by position or by path.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::set_field_at`] or [`Self::set_field_by_path`]
    /// raises, whichever the key selects.
    pub fn set_field<'key>(&mut self, key: impl Into<FieldKey<'key>>, child: Self) -> Result<()> {
        match key.into() {
            FieldKey::Index(index) => self.set_field_at(index, child),
            FieldKey::Path(path) => self.set_field_by_path(path, child),
        }
    }

    /// Removes the struct child at `index`, returning it and closing the gap.
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a struct, when `index` is past
    /// the end, or when the resulting child set does not validate. Failure
    /// leaves `self` unchanged.
    pub fn remove_field_at(&mut self, index: usize) -> Result<Self> {
        let mut fields = self.struct_children()?;
        if index >= fields.len() {
            return Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}[{index}]", self.name),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", fields.len()),
                    format_smolstr!("{index}"),
                ),
            });
        }
        let removed = fields.remove(index);
        self.set_dtype(DataType::from_fields(fields)?)?;
        Ok(removed)
    }

    /// Removes the struct child named `name`, returning it and closing the gap.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::Utf8.required_field("venue"),
    /// ])?
    /// .required_field("row");
    ///
    /// let dropped = row.remove_field_by_path("id")?;
    /// assert_eq!(dropped.name(), "id");
    /// // Positions close up behind it.
    /// assert_eq!(row[0].name(), "venue");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a struct, when no child carries
    /// `name`, or when the resulting child set does not validate. Failure
    /// leaves `self` unchanged.
    pub fn remove_field_by_path(&mut self, path: &str) -> Result<Self> {
        let mut dtype = self.dtype.clone();
        let removed = dtype.remove_field_by_path(path)?;
        self.set_dtype(dtype)?;
        Ok(removed)
    }

    /// Removes a nested child by position or by path, returning it.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::remove_field_at`] or
    /// [`Self::remove_field_by_path`] raises, whichever the key selects.
    pub fn remove_field<'key>(&mut self, key: impl Into<FieldKey<'key>>) -> Result<Self> {
        match key.into() {
            FieldKey::Index(index) => self.remove_field_at(index),
            FieldKey::Path(path) => self.remove_field_by_path(path),
        }
    }

    /// The struct children as an owned vector, or a refusal naming the reason.
    ///
    /// Cloning here is what keeps the shared child array shared everywhere
    /// else: the copy exists only for the duration of one mutation and the
    /// result is rebuilt through [`Self::set_dtype`], so read, clone, and
    /// projection paths never pay for a caller's edit.
    fn struct_children(&self) -> Result<Vec<Self>> {
        match self.dtype.as_fields() {
            Some(fields) => Ok(fields.to_vec()),
            None => Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", self.name),
                reason: crate::text::expected_got(
                    "a struct field whose children can be replaced",
                    format_smolstr!("{}", self.dtype),
                ),
            }),
        }
    }
}

/// A borrowed iterator over the struct children that partition the rows.
#[derive(Clone)]
pub struct PartitionFields<'field>(std::slice::Iter<'field, Field>);

impl<'field> Iterator for PartitionFields<'field> {
    type Item = &'field Field;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.find(|field| field.is_partition())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Every remaining child may or may not be marked, and the marker is a
        // metadata read rather than a count kept beside the children.
        (0, Some(self.0.len()))
    }
}

impl DoubleEndedIterator for PartitionFields<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.rfind(|field| field.is_partition())
    }
}

impl std::iter::FusedIterator for PartitionFields<'_> {}

/// A borrowed iterator over the names of the partition children.
#[derive(Clone)]
pub struct PartitionFieldNames<'field>(PartitionFields<'field>);

impl<'field> Iterator for PartitionFieldNames<'field> {
    type Item = &'field str;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(Field::name)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl DoubleEndedIterator for PartitionFieldNames<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.0.next_back().map(Field::name)
    }
}

impl std::iter::FusedIterator for PartitionFieldNames<'_> {}

define_field_types!(ListType, "list", crate::DataType::List(_));
define_field_types!(ListViewType, "list_view", crate::DataType::ListView(_));
define_field_types!(
    FixedSizeListType,
    "fixed_size_list",
    crate::DataType::FixedSizeList(..)
);
define_field_types!(LargeListType, "large_list", crate::DataType::LargeList(_));
define_field_types!(
    LargeListViewType,
    "large_list_view",
    crate::DataType::LargeListView(_)
);
define_field_types!(StructType, "struct", crate::DataType::Struct(_));
define_field_types!(UnionType, "union", crate::DataType::Union(..));
// The variant lives with the nested family: it is the self-describing
// sibling of the union whose grammar it shares (`variant` bare, `variant(...)`
// as dense-union sugar), and its Arrow storage is a struct of two binaries.
define_field_types!(VariantType, "variant", crate::DataType::Variant);
define_field_types!(
    DictionaryTypeMarker,
    "dictionary",
    crate::DataType::Dictionary(_)
);
define_field_types!(MapTypeMarker, "map", crate::DataType::Map(_));
define_field_types!(
    RunEndEncodedTypeMarker,
    "run_end_encoded",
    crate::DataType::RunEndEncoded(_)
);

/// A list-typed field.
pub type ListField = TypedField<ListType>;
/// A list-view-typed field.
pub type ListViewField = TypedField<ListViewType>;
/// A fixed-size-list-typed field.
pub type FixedSizeListField = TypedField<FixedSizeListType>;
/// A large-list-typed field.
pub type LargeListField = TypedField<LargeListType>;
/// A large-list-view-typed field.
pub type LargeListViewField = TypedField<LargeListViewType>;
/// A struct-typed field.
pub type StructField = TypedField<StructType>;
/// A union-typed field.
pub type UnionField = TypedField<UnionType>;
/// A dictionary-typed field.
pub type DictionaryField = TypedField<DictionaryTypeMarker>;
/// A map-typed field.
pub type MapField = TypedField<MapTypeMarker>;
/// A run-end-encoded-typed field.
pub type RunEndEncodedField = TypedField<RunEndEncodedTypeMarker>;
/// A variant-typed field.
pub type VariantField = TypedField<VariantType>;
