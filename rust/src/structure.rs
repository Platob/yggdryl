//! Struct: the named children a row schema is made of.

use std::cmp::Ordering;
use std::fmt;
use std::ops::{Deref, Index};
use std::sync::Arc;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use smol_str::{SmolStr, format_smolstr};

use crate::metadata::FIELD_PARTITION_KEY;

use crate::Scalar;
use crate::enums::EnumType;
use crate::invalid;
use crate::sequence::SequenceType;
use crate::value::DataTypeValue;
use crate::value::Value;
use crate::value::{Children, NestedValue};
use crate::{DataType, DataTypeId, DataTypeKind, Error, Field, FieldPath, FieldSegment, Result};
use std::collections::{BTreeMap, HashSet};

/// One failed borrowed schema traversal, before a public wrapper owns its error.
enum SchemaPathError<'node, 'segment> {
    Missing(&'node DataType),
    Unsupported(&'segment FieldSegment),
}

impl DataType {
    /// Returns the number of direct child fields without allocating.
    pub fn field_len(&self) -> usize {
        match self {
            Self::Sequence(_) | Self::Mapping(_) => 1,
            Self::Struct(structure) => structure.len(),
            Self::Union(fields, _) => fields.len(),
            Self::RunEndEncoded(_) => 2,
            _ => 0,
        }
    }

    /// Returns a direct child field by position without allocating.
    pub fn get_field_at(&self, index: usize) -> Option<&Field> {
        match self {
            Self::Sequence(sequence) => {
                let field = sequence.item();
                (index == 0).then_some(field)
            }
            Self::Struct(structure) => structure.get(index),
            Self::Union(fields, _) => fields.get(index).map(|(_, field)| field),
            Self::Mapping(mapping) => (index == 0).then_some(mapping.entries()),
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
    /// This is the step every named [`FieldSegment`] takes.
    pub(crate) fn get_field_by_name(&self, name: &str) -> Option<&Field> {
        match self {
            Self::Sequence(sequence) => {
                let field = sequence.item();
                (field.name() == name).then_some(field)
            }
            Self::Struct(structure) => structure
                .as_fields()
                .iter()
                .find(|field| field.name() == name),
            Self::Union(fields, _) => fields.get_by_name(name).map(|(_, field)| field),
            Self::Mapping(mapping) => {
                (mapping.entries().name() == name).then_some(mapping.entries())
            }
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

    /// Returns a nested child by the expression path grammar.
    ///
    /// A bare name is one child lookup. A name containing a dot is quoted, so
    /// `"a.b"` is one child and `a.b` is two steps. Every non-bare spelling is
    /// parsed once into [`FieldSegment`]s before this borrowed walk begins.
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from(StructType::from_fields([
    ///     DataType::from(StructType::from_fields([DataType::Float64.required_field("price")])?)
    ///         .required_field("line"),
    /// ])?);
    /// assert_eq!(row.get_field_by_path("line.price").unwrap().name(), "price");
    ///
    /// // A dotted name is quoted, so it is one name rather than two steps.
    /// let dotted = DataType::from(StructType::from_fields([DataType::Int64.required_field("a.b")])?);
    /// assert_eq!(dotted.get_field_by_path("\"a.b\"").unwrap().name(), "a.b");
    ///
    /// // A list is transparent: its item is a step the path need not spell.
    /// let orders = DataType::from_str("struct<orders:array<struct<price:double>>>")?;
    /// assert_eq!(orders.get_field_by_path("orders.price").unwrap().name(), "price");
    /// assert_eq!(orders.get_field_by_path("orders.item.price").unwrap().name(), "price");
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field_by_path(&self, path: &str) -> Option<&Field> {
        FieldPath::with_schema_segments(path, |segments| self.walk_field_by_segments(segments).ok())
            .ok()
            .flatten()
    }

    /// Returns a nested child by position or by path.
    ///
    /// The one lookup a caller reaches for when the key is whichever the data
    /// gave them: an integer is a position, a string is a path.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?);
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
    /// Returns a parse error for an invalid path spelling, a named refusal for
    /// aliases, ranges, and predicates, or an error when no child resolves.
    pub fn field_by_path(&self, path: &str) -> Result<&Field> {
        FieldPath::with_schema_segments(path, |segments| {
            self.walk_field_by_segments(segments)
                .map_err(|error| match error {
                    SchemaPathError::Missing(node) => missing_child(node, path),
                    SchemaPathError::Unsupported(segment) => schema_segment_refusal(path, segment),
                })
        })?
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?);
    /// row.set_field_at(0, DataType::utf8().required_field("id"))?;
    ///
    /// assert_eq!(row["id"].dtype(), &DataType::utf8());
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

    /// Replaces the child `path` resolves to, appending one missing final name.
    ///
    /// The expression grammar is parsed once. A missing final named segment
    /// appends only to the current struct; a missing parent or a non-container
    /// refuses, so a dotted spelling never becomes one literal child name.
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([
    ///     DataType::from(StructType::from_fields([DataType::Int32.required_field("price")])?)
    ///         .required_field("line"),
    /// ])?);
    ///
    /// row.set_field_by_path("line.price", DataType::Float64.required_field("price"))?;
    /// assert_eq!(row["line"]["price"].dtype(), &DataType::Float64);
    ///
    /// // A missing final name appends to this struct.
    /// row.set_field_by_path("venue", DataType::utf8().nullable_field("venue"))?;
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
        FieldPath::with_schema_segments(path, |segments| {
            self.set_field_by_segments(path, segments, child)
        })?
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
        *self = Self::from(StructType::from_fields(children)?);
        Ok(removed)
    }

    /// Removes the child `path` resolves to, returning it.
    ///
    /// # Errors
    ///
    /// Returns an error when the path resolves to no child, or when the
    /// rebuilt datatype does not validate. Failure leaves `self` unchanged.
    pub fn remove_field_by_path(&mut self, path: &str) -> Result<Field> {
        FieldPath::with_schema_segments(path, |segments| {
            self.remove_field_by_segments(path, segments)
        })?
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::from(StructType::from_fields([DataType::Float64.required_field("px")])?)
    ///         .nullable_field("line"),
    /// ])?);
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::list(DataType::Float64.nullable_field("item")).nullable_field("levels"),
    /// ])?);
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
    /// [`StructType::from_fields`] would silently make it a struct, so this refuses
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

    /// The one borrowed walk a schema address takes.
    fn walk_field_by_segments<'node, 'segment>(
        &'node self,
        segments: &'segment [FieldSegment],
    ) -> std::result::Result<&'node Field, SchemaPathError<'node, 'segment>> {
        let Some((segment, rest)) = segments.split_first() else {
            return Err(SchemaPathError::Missing(self));
        };
        if matches!(segment, FieldSegment::Range { .. } | FieldSegment::Where(_)) {
            return Err(SchemaPathError::Unsupported(segment));
        }
        if matches!(self, Self::Mapping(_)) && matches!(segment, FieldSegment::Key(_)) {
            return Err(SchemaPathError::Unsupported(segment));
        }
        let child = if let Some(name) = segment.as_name() {
            match self.get_field_by_name(name) {
                Some(child) => child,
                None => match self.list_item() {
                    // Reading a schema sees through a list item, but writes
                    // name that item explicitly because they rebuild the list.
                    Some(item) => return item.dtype().walk_field_by_segments(segments),
                    None => return Err(SchemaPathError::Missing(self)),
                },
            }
        } else {
            match segment {
                FieldSegment::Index(_) => self.list_item().ok_or(SchemaPathError::Missing(self))?,
                FieldSegment::Key(_) => return Err(SchemaPathError::Unsupported(segment)),
                FieldSegment::Field(_) => unreachable!("named above"),
                FieldSegment::Range { .. } | FieldSegment::Where(_) => {
                    unreachable!("refused above")
                }
            }
        };
        if rest.is_empty() {
            Ok(child)
        } else {
            child.dtype().walk_field_by_segments(rest)
        }
    }

    /// Replaces one parsed child, appending only a missing final named child.
    fn set_field_by_segments(
        &mut self,
        path: &str,
        segments: &[FieldSegment],
        child: Field,
    ) -> Result<()> {
        let Some((segment, rest)) = segments.split_first() else {
            return Err(missing_child(self, path));
        };
        if matches!(segment, FieldSegment::Range { .. } | FieldSegment::Where(_)) {
            return Err(schema_segment_refusal(path, segment));
        }
        if matches!(&*self, Self::Mapping(_)) && matches!(segment, FieldSegment::Key(_)) {
            return Err(schema_segment_refusal(path, segment));
        }
        if rest.is_empty() {
            if let Some(name) = segment.as_name() {
                if let Some(index) = self.index_of_name(name) {
                    let mut children = self.children();
                    children[index] = child.with_name(name);
                    *self = self.with_fields(children)?;
                    return Ok(());
                }
                let mut children = self.require_struct_children()?;
                children.push(child.with_name(name));
                *self = Self::from(StructType::from_fields(children)?);
                return Ok(());
            }
            let Some(index) = self.schema_segment_index(segment) else {
                return Err(missing_child(self, path));
            };
            let mut children = self.children();
            children[index] = child;
            *self = self.with_fields(children)?;
            return Ok(());
        }

        let Some(index) = self.schema_segment_index(segment) else {
            return Err(missing_child(self, path));
        };
        let mut children = self.children();
        let mut nested = children[index].dtype().clone();
        nested.set_field_by_segments(path, rest, child)?;
        children[index].set_dtype(nested)?;
        *self = self.with_fields(children)?;
        Ok(())
    }

    /// Removes one parsed child without making a fixed-arity node grow or shrink.
    fn remove_field_by_segments(&mut self, path: &str, segments: &[FieldSegment]) -> Result<Field> {
        let Some((segment, rest)) = segments.split_first() else {
            return Err(missing_child(self, path));
        };
        if matches!(segment, FieldSegment::Range { .. } | FieldSegment::Where(_)) {
            return Err(schema_segment_refusal(path, segment));
        }
        if matches!(&*self, Self::Mapping(_)) && matches!(segment, FieldSegment::Key(_)) {
            return Err(schema_segment_refusal(path, segment));
        }
        if rest.is_empty() {
            let Some(index) = self.schema_segment_index(segment) else {
                return Err(missing_child(self, path));
            };
            return self.remove_field_at(index);
        }
        let Some(index) = self.schema_segment_index(segment) else {
            return Err(missing_child(self, path));
        };
        let mut children = self.children();
        let mut nested = children[index].dtype().clone();
        let removed = nested.remove_field_by_segments(path, rest)?;
        children[index].set_dtype(nested)?;
        *self = self.with_fields(children)?;
        Ok(removed)
    }

    /// The direct child a write or removal may walk into.
    fn schema_segment_index(&self, segment: &FieldSegment) -> Option<usize> {
        match segment {
            FieldSegment::Field(name) => self.index_of_name(name),
            FieldSegment::Key(key) => key
                .value()
                .as_str()
                .and_then(|name| self.index_of_name(name)),
            FieldSegment::Index(_) => self.list_item().map(|_| 0),
            FieldSegment::Range { .. } | FieldSegment::Where(_) => None,
        }
    }
}

// ------------------------------------------------------------------------
// Nested, dictionary, map, and run-end encoded field markers.
// ------------------------------------------------------------------------

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

impl DataType {
    /// Returns struct children as a borrowed slice, or `None` for other types.
    pub fn as_fields(&self) -> Option<&[Field]> {
        match self {
            Self::Struct(structure) => Some(structure.as_fields()),
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
            Self::Sequence(SequenceType::List(_)) => Self::list(next()),
            Self::Sequence(SequenceType::ListView(_)) => Self::list_view(next()),
            Self::Sequence(SequenceType::FixedSizeList(_, length)) => {
                Self::fixed_size_list(next(), *length)?
            }
            Self::Sequence(SequenceType::LargeList(_)) => Self::large_list(next()),
            Self::Sequence(SequenceType::LargeListView(_)) => Self::large_list_view(next()),
            Self::Struct(_) => Self::from(StructType::from_fields(children)?),
            Self::Union(members, mode) => {
                let ids: Vec<i8> = members.iter().map(|(id, _)| id).collect();
                Self::union(ids.into_iter().zip(children), *mode)?
            }
            Self::Mapping(mapping) => Self::map(next(), mapping.keys_sorted())?,
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
        self.dtype().as_fields().is_some()
    }

    /// Returns the struct children of this field, or an empty slice.
    ///
    /// A struct `Field` is the schema of the rows it describes, so this is the
    /// column list every interop layer projects from.
    pub fn fields(&self) -> &[Field] {
        self.dtype().as_fields().unwrap_or_default()
    }

    /// Returns the number of struct children.
    pub fn field_len(&self) -> usize {
        self.dtype().field_len()
    }

    /// Returns one nested child by position.
    pub fn get_field_at(&self, index: usize) -> Option<&Field> {
        self.dtype().get_field_at(index)
    }

    /// Returns one nested child by the expression path grammar.
    ///
    /// [`DataType::get_field_by_path`] carries the rule, including the one
    /// that makes a list transparent - `orders.price` reaches the price of an
    /// `array<struct>` item; this node's datatype is where it starts.
    pub fn get_field_by_path(&self, path: &str) -> Option<&Field> {
        self.dtype().get_field_by_path(path)
    }

    /// Returns one nested child by position or by path.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let order = DataType::from(StructType::from_fields([
    ///     DataType::from(StructType::from_fields([DataType::Float64.required_field("price")])?)
    ///         .required_field("line"),
    /// ])?)
    /// .required_field("order");
    ///
    /// assert_eq!(order.get_field(0).unwrap().name(), "line");
    /// assert_eq!(order.get_field("line.price").unwrap().name(), "price");
    /// # Ok(())
    /// # }
    /// ```
    pub fn get_field<'key>(&self, key: impl Into<FieldKey<'key>>) -> Option<&Field> {
        self.dtype().get_field(key)
    }

    /// Returns one nested child by position, naming what is there when absent.
    ///
    /// # Errors
    ///
    /// Returns an error when this node has no child at that position.
    pub fn field_at(&self, index: usize) -> Result<&Field> {
        self.dtype().field_at(index)
    }

    /// Returns one nested child by path, naming what is there when absent.
    ///
    /// # Errors
    ///
    /// Returns the parse, selector, or missing-child error its datatype raises.
    pub fn field_by_path(&self, path: &str) -> Result<&Field> {
        self.dtype().field_by_path(path)
    }

    /// Returns one nested child by position or by path, raising when absent.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::field_at`] or [`Self::field_by_path`] raises,
    /// whichever the key selects.
    pub fn field<'key>(&self, key: impl Into<FieldKey<'key>>) -> Result<&Field> {
        self.dtype().field(key)
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
            crate::Widening::upscale(upscale),
            crate::Recode::Allowed,
        )
    }

    /// Returns every leaf under this field, named by its dotted path.
    ///
    /// [`DataType::unnest_fields`] carries the rule; this node's datatype is
    /// where it starts.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let row = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::from(StructType::from_fields([DataType::Float64.required_field("px")])?)
    ///         .nullable_field("line"),
    /// ])?)
    /// .required_field("row");
    ///
    /// let leaves = row.unnest_fields();
    /// let names: Vec<&str> = leaves.iter().map(|field| field.name()).collect();
    /// assert_eq!(names, ["id", "line.px"]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn unnest_fields(&self) -> Vec<Self> {
        self.dtype().unnest_fields()
    }

    /// Returns this field's children with every collection replaced by what it
    /// holds.
    ///
    /// [`DataType::explode_fields`] carries the rule.
    pub fn explode_fields(&self) -> Vec<Self> {
        self.dtype().explode_fields()
    }

    /// Returns this struct root without the named children.
    ///
    /// Names it does not carry are ignored, so a caller can subtract a set
    /// without checking it first. This is what a partitioned write stores: the
    /// schema minus the columns the path already spells out.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("price"),
    ///     DataType::Int32.required_field("year"),
    /// ])?)
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
            DataType::from(StructType::from_fields(kept)?),
            self.is_nullable(),
            self.metadata_iter(),
        )
    }

    /// Returns whether this field carries the values a path spells out.
    ///
    /// A partition field is an ordinary field with the reserved
    /// `FIELD:partition` marker set. Nothing in a batch says which of its
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from(StructType::from_fields([
    ///     DataType::Int32.required_field("year").with_partition(true),
    ///     DataType::Int64.required_field("price"),
    /// ])?)
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
            DataType::from(StructType::from_fields(kept)?),
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let schema = DataType::from(StructType::from_fields([
    ///     DataType::Int32.required_field("year"),
    ///     DataType::Int64.required_field("price"),
    /// ])?)
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
            if self.dtype().get_field_by_name(name).is_none() {
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
            DataType::from(StructType::from_fields(children)?),
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
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    ///     .required_field("row");
    ///
    /// row.set_field_at(0, DataType::utf8().required_field("id"))?;
    /// assert_eq!(row["id"].dtype(), &DataType::utf8());
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
                path: format_smolstr!("$.{}[{index}]", self.name()),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", fields.len()),
                    format_smolstr!("{index}"),
                ),
            });
        }
        fields[index] = child;
        self.set_dtype(DataType::from(StructType::from_fields(fields)?))
    }

    /// Replaces the struct child named `name`, appending an unknown one.
    ///
    /// Dict-like on purpose, and the asymmetry with [`Self::set_field`] is
    /// deliberate: a known name is replaced *in place*, keeping its position,
    /// and a missing final name appends a new child. A position only replaces.
    ///
    /// The child is stored under `name` whatever it calls itself, so
    /// `row.set_field_by_path("price", DataType::Float64.required_field("x"))`
    /// stores a child named `price`.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    ///     .required_field("row");
    ///
    /// // A missing final name appends.
    /// row.set_field_by_path("venue", DataType::utf8().nullable_field("venue"))?;
    /// assert_eq!(row.field_len(), 2);
    ///
    /// // A known one replaces, keeping its position.
    /// row.set_field_by_path("id", DataType::utf8().required_field("id"))?;
    /// assert_eq!(row.field_len(), 2);
    /// assert_eq!(row[0].name(), "id");
    /// assert_eq!(row["id"].dtype(), &DataType::utf8());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when this field is not a struct or the resulting child
    /// set does not validate. Failure leaves `self` unchanged.
    pub fn set_field_by_path(&mut self, path: &str, child: Self) -> Result<()> {
        let mut dtype = self.dtype().clone();
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
                path: format_smolstr!("$.{}[{index}]", self.name()),
                reason: crate::text::expected_got(
                    format_smolstr!("a child position below {}", fields.len()),
                    format_smolstr!("{index}"),
                ),
            });
        }
        let removed = fields.remove(index);
        self.set_dtype(DataType::from(StructType::from_fields(fields)?))?;
        Ok(removed)
    }

    /// Removes the struct child named `name`, returning it and closing the gap.
    ///
    /// ```
    /// use yggdryl::DataType;
    /// use yggdryl::StructType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut row = DataType::from(StructType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::utf8().required_field("venue"),
    /// ])?)
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
        let mut dtype = self.dtype().clone();
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
        match self.dtype().as_fields() {
            Some(fields) => Ok(fields.to_vec()),
            None => Err(Error::InvalidRecord {
                path: format_smolstr!("$.{}", self.name()),
                reason: crate::text::expected_got(
                    "a struct field whose children can be replaced",
                    format_smolstr!("{}", self.dtype()),
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

// ------------------------------------------------------------------------
// The struct datatype: named children in declaration order.
// ------------------------------------------------------------------------

/// The named children of a struct, in declaration order.
///
/// The one collection of fields the crate has: ordered, immutable, and held
/// in one shared allocation, so a clone shares the children rather than
/// copying them and an empty collection holds nothing at all. It is the
/// payload `DataType::Struct(StructType::Struct(..))` carries and the
/// one place a list of children is validated: [`StructType::from_fields`]
/// refuses two children of one name. It dereferences to the slice, so
/// everything a caller does with `&[Field]` reads the same through it, and
/// it serializes as that slice.
///
/// ```
/// use yggdryl::{DataType, StructType};
///
/// # fn main() -> yggdryl::Result<()> {
/// let children = StructType::from_fields([
///     DataType::utf8().required_field("symbol"),
///     DataType::Int64.required_field("quantity"),
/// ])?;
/// assert_eq!(children.len(), 2);
/// assert_eq!(children[1].name(), "quantity");
/// let row = DataType::from(children);
/// assert_eq!(row.field_len(), 2);
/// assert!(
///     StructType::from_fields([
///         DataType::Int64.required_field("id"),
///         DataType::utf8().required_field("id"),
///     ])
///     .is_err()
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Default, Eq, PartialEq, Hash)]
pub struct StructType(pub(crate) Option<Arc<[Field]>>);

impl StructType {
    /// Creates an empty collection without allocating.
    pub const fn new() -> Self {
        Self(None)
    }

    /// Creates a collection and rejects duplicate field names.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidDataType`] when two children share a name or a
    /// child does not validate.
    pub fn from_fields<I>(fields: I) -> Result<Self>
    where
        I: IntoIterator<Item = Field>,
    {
        let fields = fields.into_iter().collect::<Vec<_>>();
        validate_fields(&fields, "StructType")?;
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

    /// The address of the children's storage, which names it for as long as
    /// a clone of this collection is held; zero for no children.
    pub(crate) fn storage_address(&self) -> usize {
        self.0
            .as_ref()
            .map_or(0, |held| Arc::as_ptr(held).cast::<()>() as usize)
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
        reject_duplicate_field_names(&fields, "StructType")?;
        Ok(Self::from_vec(fields))
    }

    /// The struct over children each already valid - read out of a valid
    /// struct, or the fields a registry keeps - so only the names are
    /// checked, once, rather than every child's whole datatype again.
    ///
    /// # Errors
    ///
    /// Returns an error when two children share a name.
    pub(crate) fn from_checked_fields(fields: Vec<Field>) -> Result<Self> {
        reject_duplicate_field_names(&fields, "StructType")?;
        Ok(Self::from_vec(fields))
    }

    /// The struct over children each already valid and named once by
    /// construction - a builder that keeps one slot per name, a row that
    /// replaces a child in place or appends one no child is named as - so
    /// nothing is checked: the caller's construction is the check, and a
    /// debug build asserts it.
    pub(crate) fn from_unique_fields(fields: Vec<Field>) -> Self {
        debug_assert!(
            reject_duplicate_field_names(&fields, "StructType").is_ok(),
            "the children were named once by construction"
        );
        Self::from_vec(fields)
    }
}

impl Deref for StructType {
    type Target = [Field];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl AsRef<[Field]> for StructType {
    fn as_ref(&self) -> &[Field] {
        self.0.as_deref().unwrap_or_default()
    }
}

impl Index<usize> for StructType {
    type Output = Field;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_ref()[index]
    }
}

impl fmt::Debug for StructType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(formatter)
    }
}

impl Ord for StructType {
    fn cmp(&self, other: &Self) -> Ordering {
        if matches!((&self.0, &other.0), (Some(left), Some(right)) if Arc::ptr_eq(left, right)) {
            Ordering::Equal
        } else {
            cmp_field_slices(self, other)
        }
    }
}

impl PartialOrd for StructType {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl IntoIterator for StructType {
    type Item = Field;
    type IntoIter = std::vec::IntoIter<Field>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_fields().into_iter()
    }
}

impl<'a> IntoIterator for &'a StructType {
    type Item = &'a Field;
    type IntoIter = std::slice::Iter<'a, Field>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Serialize for StructType {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StructType {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let fields = Vec::<Field>::deserialize(deserializer)?;
        Self::from_fields(fields).map_err(serde::de::Error::custom)
    }
}

impl DataTypeValue for StructType {
    const FAMILY: &'static str = "struct";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        DataTypeId::Struct
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Nested
    }

    fn validate(&self) -> Result<()> {
        for field in self.iter() {
            field.validate()?;
        }
        Ok(())
    }

    fn into_dtype(self) -> DataType {
        DataType::Struct(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Struct(fields) => Some(fields.clone()),
            _ => None,
        }
    }
}

impl fmt::Display for StructType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id().as_str())
    }
}

impl From<StructType> for DataType {
    fn from(value: StructType) -> Self {
        Self::Struct(value)
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

pub(crate) fn exploded(child: &Field) -> Field {
    let held = match child.dtype() {
        DataType::Sequence(SequenceType::List(item))
        | DataType::Sequence(SequenceType::ListView(item))
        | DataType::Sequence(SequenceType::FixedSizeList(item, _))
        | DataType::Sequence(SequenceType::LargeList(item))
        | DataType::Sequence(SequenceType::LargeListView(item)) => {
            Some((item.dtype().clone(), item.is_nullable()))
        }
        DataType::Mapping(map) => {
            Some((map.entries().dtype().clone(), map.entries().is_nullable()))
        }
        DataType::RunEndEncoded(encoded) => Some((
            encoded.values().dtype().clone(),
            encoded.values().is_nullable(),
        )),
        DataType::Enum(EnumType::Dictionary(dictionary)) => {
            Some((dictionary.value().clone(), false))
        }
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
pub(crate) fn missing_child(node: &DataType, path: &str) -> Error {
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

/// Refuse a selector that cannot name one borrowed schema child.
fn schema_segment_refusal(path: &str, segment: &FieldSegment) -> Error {
    Error::InvalidRecord {
        path: format_smolstr!("$.{path}"),
        reason: format_smolstr!("expected a named child or one list item, got {segment}"),
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

pub(crate) fn validate_fields(fields: &[Field], kind: &'static str) -> Result<()> {
    reject_duplicate_field_names(fields, kind)?;
    fields.iter().try_for_each(Field::validate)
}

pub(crate) fn reject_duplicate_field_names(fields: &[Field], kind: &'static str) -> Result<()> {
    const HASHED_DUPLICATE_CHECK_THRESHOLD: usize = 16;

    if fields.len() > HASHED_DUPLICATE_CHECK_THRESHOLD {
        // Names are short and the set lives for one check, so the hash is
        // the crate's own rather than a keyed one.
        let mut names =
            HashSet::with_capacity_and_hasher(fields.len(), crate::xxhash::Xxh64::new());
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

/// One struct value: its children by name, sorted, so two statements of one
/// value in two orders are one value.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Struct(Arc<BTreeMap<SmolStr, Scalar>>);

impl Struct {
    /// Construct a sorted struct value.
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

impl fmt::Display for Struct {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_map())
    }
}

impl NestedValue for Struct {
    fn len(&self) -> usize {
        self.as_map().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Struct(self.as_map().values())
    }
}

impl<'a> IntoIterator for &'a Scalar {
    type Item = std::borrow::Cow<'a, Scalar>;
    type IntoIter = Children<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Value for Struct {
    fn dtype(&self) -> Result<DataType> {
        Scalar::Struct(self.clone()).dtype()
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Struct(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Struct(value) => Some(value),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// Arrow projection: named children, in the order they are declared.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::{
        DataType as ArrowDataType, FieldRef as ArrowFieldRef, Fields as ArrowFields,
    };

    use super::StructType;
    use crate::value::ArrowFfiParts;
    use crate::{DataType, Field, Result};

    impl StructType {
        /// The Arrow storage these children lay out.
        ///
        /// # Errors
        ///
        /// Returns an error when a child has no Arrow projection.
        pub(crate) fn arrow_storage(&self) -> Result<ArrowDataType> {
            Ok(ArrowDataType::Struct(into_arrow_fields(self.as_fields())?))
        }

        /// The same projection, consuming uniquely held children.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn into_arrow_storage(self) -> Result<ArrowDataType> {
            let fields = self
                .into_fields()
                .into_iter()
                .map(Field::into_arrow_field_ref)
                .collect::<Result<Vec<_>>>()?;
            Ok(ArrowDataType::Struct(fields.into()))
        }

        /// The C Data Interface node these children write.
        ///
        /// # Errors
        ///
        /// [`Self::arrow_storage`] carries the rule.
        pub(crate) fn arrow_ffi_parts(&self) -> Result<ArrowFfiParts> {
            Ok(ArrowFfiParts::nested(
                "+s",
                self.iter()
                    .cloned()
                    .map(Field::into_arrow_field_ffi)
                    .collect::<Result<Vec<_>>>()?,
            ))
        }

        /// The struct datatype one Arrow struct storage imports as.
        ///
        /// # Errors
        ///
        /// Returns an error when a child cannot be imported or the children do
        /// not form a valid struct.
        pub(crate) fn from_arrow_storage_at_depth(
            fields: &ArrowFields,
            depth: usize,
        ) -> Result<DataType> {
            Ok(DataType::Struct(from_arrow_fields_at_depth(fields, depth)?))
        }
    }

    /// Projects one ordered child list as Arrow's own.
    ///
    /// # Errors
    ///
    /// Returns an error when a child has no Arrow projection.
    pub(crate) fn into_arrow_fields(fields: &[Field]) -> Result<ArrowFields> {
        fields
            .iter()
            .cloned()
            .map(Field::into_arrow_field_ref)
            .collect::<Result<Vec<ArrowFieldRef>>>()
            .map(Into::into)
    }

    /// Imports one ordered child list at an existing nesting depth.
    ///
    /// # Errors
    ///
    /// Returns an error when a child cannot be imported or the children do not
    /// form a valid child list.
    pub(crate) fn from_arrow_fields_at_depth(
        fields: &ArrowFields,
        depth: usize,
    ) -> Result<StructType> {
        let fields = fields
            .iter()
            .cloned()
            .map(|field| Field::from_arrow_field_ref_at_depth(field, depth))
            .collect::<Result<Vec<_>>>()?;
        StructType::from_imported_fields(fields)
    }
}
