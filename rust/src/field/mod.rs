//! Generic and datatype-specific Arrow-compatible field values.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::sync::{Arc, OnceLock};

use arrow_schema::Field as ArrowField;
use smol_str::{SmolStr, format_smolstr};

use crate::datatype::{
    DataType, MapType, RunEndEncodedType, default_value_for_field, preflight_schema_shape,
};
use crate::metadata::{
    ALIAS_KEY, CATALOG_NAME_KEY, FIELD_INIT_KEY, FIELD_PARTITION_KEY, HTTP_ACCEPT_ENCODING_KEY,
    HTTP_ACCEPT_KEY, HTTP_ACCEPT_LANGUAGE_KEY, HTTP_ACCEPT_RANGES_KEY, HTTP_CACHE_CONTROL_KEY,
    HTTP_CONTENT_DISPOSITION_KEY, HTTP_CONTENT_ENCODING_KEY, HTTP_CONTENT_LANGUAGE_KEY,
    HTTP_CONTENT_LENGTH_KEY, HTTP_CONTENT_LOCATION_KEY, HTTP_CONTENT_RANGE_KEY,
    HTTP_CONTENT_TYPE_KEY, HTTP_ETAG_KEY, HTTP_EXPIRES_KEY, HTTP_LAST_MODIFIED_KEY,
    HTTP_LOCATION_KEY, HTTP_RANGE_KEY, HTTP_VARY_KEY, LOCATION_KEY, MetadataIter,
    PARQUET_FIELD_ID_KEY, PropertyIter, ProtocolMetadata, ProtocolMetadataMut, SCHEMA_NAME_KEY,
    TABLE_NAME_KEY, for_each_well_known_protocol, parse_field_id, parse_reserved_bool,
    property_key, write_json_string as write_quoted,
};
use crate::{
    Error, MediaType, Metadata, MimeType, Result, Scheme, Url, Value, stable_hash_display,
};

/// Emit the borrowed and mutable protocol view accessors of one protocol.
macro_rules! field_protocol_accessors {
    ($name:ident, $mutable:ident, $constant:ident, $label:literal) => {
        #[doc = concat!("Returns the borrowed ", $label, " property view.")]
        ///
        /// This is [`Self::protocol`] with the protocol already chosen.
        pub fn $name(&self) -> ProtocolMetadata<'_> {
            self.protocol(&Scheme::$constant)
        }

        #[doc = concat!("Returns the mutable ", $label, " property view.")]
        ///
        /// This is [`Self::protocol_mut`] with the protocol already chosen.
        pub fn $mutable(&mut self) -> ProtocolMetadataMut<'_> {
            self.protocol_mut(&Scheme::$constant)
        }
    };
}

mod arrow;
pub mod binary;
#[cfg(feature = "arrow")]
pub mod cast;
pub mod decimal;
mod diff;
pub mod floating;
pub mod integer;
pub mod nested;
mod parser;
pub mod scalar;
mod serde;
pub mod temporal;
mod typed;
mod value;

pub(crate) use arrow::arrow_field_to_ffi;
#[cfg(feature = "arrow")]
pub use cast::{ArrowCast, ArrowFieldType};
pub(crate) use diff::push_field_name_path;
pub use diff::{Differences, OwnedDifferences};
pub(crate) use diff::{data_types_equal, show_diff};
pub use typed::{AnyType, FieldType, TypedField, TypedFieldRef};
pub(crate) use value::validate_data_type_value_for;

/// A null-typed field.
pub type NullField = TypedField<scalar::Null>;
/// A Boolean-typed field.
pub type BooleanField = TypedField<scalar::Boolean>;
/// An Int8-typed field.
pub type Int8Field = TypedField<integer::Int8>;
/// An Int16-typed field.
pub type Int16Field = TypedField<integer::Int16>;
/// An Int32-typed field.
pub type Int32Field = TypedField<integer::Int32>;
/// An Int64-typed field.
pub type Int64Field = TypedField<integer::Int64>;
/// A UInt8-typed field.
pub type UInt8Field = TypedField<integer::UInt8>;
/// A UInt16-typed field.
pub type UInt16Field = TypedField<integer::UInt16>;
/// A UInt32-typed field.
pub type UInt32Field = TypedField<integer::UInt32>;
/// A UInt64-typed field.
pub type UInt64Field = TypedField<integer::UInt64>;
/// A Float16-typed field.
pub type Float16Field = TypedField<floating::Float16>;
/// A Float32-typed field.
pub type Float32Field = TypedField<floating::Float32>;
/// A Float64-typed field.
pub type Float64Field = TypedField<floating::Float64>;
/// A timestamp-typed field.
pub type TimestampField = TypedField<temporal::Timestamp>;
/// A Date32-typed field.
pub type Date32Field = TypedField<temporal::Date32>;
/// A Date64-typed field.
pub type Date64Field = TypedField<temporal::Date64>;
/// A Time32-typed field.
pub type Time32Field = TypedField<temporal::Time32>;
/// A Time64-typed field.
pub type Time64Field = TypedField<temporal::Time64>;
/// A duration-typed field.
pub type DurationField = TypedField<temporal::Duration>;
/// An interval-typed field.
pub type IntervalField = TypedField<temporal::Interval>;
/// A variable binary-typed field.
pub type BinaryField = TypedField<binary::Binary>;
/// A fixed-size binary-typed field.
pub type FixedSizeBinaryField = TypedField<binary::FixedSizeBinary>;
/// A large binary-typed field.
pub type LargeBinaryField = TypedField<binary::LargeBinary>;
/// A binary-view-typed field.
pub type BinaryViewField = TypedField<binary::BinaryView>;
/// A UTF-8-typed field.
pub type Utf8Field = TypedField<binary::Utf8>;
/// A large UTF-8-typed field.
pub type LargeUtf8Field = TypedField<binary::LargeUtf8>;
/// A UTF-8-view-typed field.
pub type Utf8ViewField = TypedField<binary::Utf8View>;
/// A list-typed field.
pub type ListField = TypedField<nested::List>;
/// A list-view-typed field.
pub type ListViewField = TypedField<nested::ListView>;
/// A fixed-size-list-typed field.
pub type FixedSizeListField = TypedField<nested::FixedSizeList>;
/// A large-list-typed field.
pub type LargeListField = TypedField<nested::LargeList>;
/// A large-list-view-typed field.
pub type LargeListViewField = TypedField<nested::LargeListView>;
/// A struct-typed field.
pub type StructField = TypedField<nested::Struct>;
/// A union-typed field.
pub type UnionField = TypedField<nested::Union>;
/// A dictionary-typed field.
pub type DictionaryField = TypedField<nested::Dictionary>;
/// A Decimal32-typed field.
pub type Decimal32Field = TypedField<decimal::Decimal32>;
/// A Decimal64-typed field.
pub type Decimal64Field = TypedField<decimal::Decimal64>;
/// A Decimal128-typed field.
pub type Decimal128Field = TypedField<decimal::Decimal128>;
/// A Decimal256-typed field.
pub type Decimal256Field = TypedField<decimal::Decimal256>;
/// A map-typed field.
pub type MapField = TypedField<nested::Map>;
/// A run-end-encoded-typed field.
pub type RunEndEncodedField = TypedField<nested::RunEndEncoded>;

/// A shared Arrow field projection.
pub type FieldRef = Arc<ArrowField>;

/// An allocation-conscious Arrow field with field-owned metadata.
///
/// Value traits ignore the projection cache. Clones share metadata, nested
/// datatype state, and a populated Arrow projection until an effective change
/// invalidates the cache.
pub struct Field {
    name: SmolStr,
    data_type: DataType,
    nullable: bool,
    dictionary_id: i64,
    dictionary_is_ordered: bool,
    metadata: Metadata,
    arrow: OnceLock<FieldRef>,
}

impl Field {
    /// Constructs a field with empty metadata.
    pub fn new(name: impl Into<SmolStr>, data_type: DataType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
            dictionary_id: 0,
            dictionary_is_ordered: false,
            metadata: Metadata::new(),
            arrow: OnceLock::new(),
        }
    }

    /// Materializes this field's bounded canonical scalar default.
    ///
    /// Nullable fields prefer logical null. Union and run-end layouts encode
    /// that null through a physically nullable logical child when possible.
    pub fn default_value(&self) -> Result<Value> {
        default_value_for_field(self)
    }

    /// Checks this field's datatype and returns an allocation-free typed view.
    pub fn try_as_typed<K: FieldType>(&self) -> Result<TypedFieldRef<'_, K>> {
        TypedFieldRef::try_from_field(self)
    }

    /// Checks this field's datatype and consumes it into a typed field.
    pub fn try_into_typed<K: FieldType>(self) -> Result<TypedField<K>> {
        TypedField::try_from_field(self)
    }

    /// Constructs and validates a field with a complete metadata snapshot.
    pub fn from_parts<I, K, V>(
        name: impl Into<SmolStr>,
        data_type: DataType,
        nullable: bool,
        metadata: I,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let field = Self {
            name: name.into(),
            data_type,
            nullable,
            dictionary_id: 0,
            dictionary_is_ordered: false,
            metadata: Metadata::from_entries(metadata)?,
            arrow: OnceLock::new(),
        };
        field.validate()?;
        Ok(field)
    }

    /// Returns the physical field name without allocating.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the logical datatype without allocating.
    pub const fn data_type(&self) -> &DataType {
        &self.data_type
    }

    /// Returns whether values may be null.
    pub const fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Returns whether this field is a struct, and therefore usable as a
    /// record schema root.
    pub fn is_struct(&self) -> bool {
        self.data_type.as_fields().is_some()
    }

    /// Returns the struct children of this field, or an empty slice.
    ///
    /// A struct `Field` is the schema of the rows it describes, so this is the
    /// column list every interop layer projects from.
    pub fn fields(&self) -> &[Field] {
        self.data_type.as_fields().unwrap_or_default()
    }

    /// Returns the number of struct children.
    pub fn field_len(&self) -> usize {
        self.data_type.field_len()
    }

    /// Returns one struct child by position.
    pub fn get_field(&self, index: usize) -> Option<&Field> {
        self.data_type.get_field(index)
    }

    /// Returns the first struct child with an exact name.
    pub fn get_field_by_name(&self, name: &str) -> Option<&Field> {
        self.data_type.get_field_by_name(name)
    }

    /// Returns the position of the first struct child with an exact name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.fields().iter().position(|field| field.name() == name)
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
            if self.get_field_by_name(name).is_none() {
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

    /// Validates one row value against this struct root.
    ///
    /// The row is an ordered [`Value::Sequence`] with one entry per struct
    /// child. Validation is schema-directed and reports the dot/bracket path
    /// of the first value that does not fit.
    ///
    /// # Errors
    ///
    /// Returns an error when the root is not a struct, the row has the wrong
    /// arity, or any value violates its field's datatype or nullability.
    pub fn validate_value(&self, value: &Value) -> Result<()> {
        // Only a struct is required here. Nullability is a media-root concern:
        // a nullable struct is a perfectly good row schema, and a null row is
        // representable in Arrow even though a tabular resource forbids it.
        self.require_struct()?;
        value::validate_row(self, value)
    }

    /// Rewrites one row value into the exact representation this root declares.
    ///
    /// Integers, floats, and nested containers are narrowed to the declared
    /// datatype. A row that already matches is returned untouched, so this is
    /// free for values that were built correctly.
    ///
    /// # Errors
    ///
    /// Returns an error when a value cannot be represented by its field.
    pub fn canonicalize_value(&self, value: Value) -> Result<Value> {
        self.require_struct()?;
        value::canonicalize_row(self, value)
    }

    /// Validates that this field is a struct, without a nullability opinion.
    ///
    /// # Errors
    ///
    /// Returns an error when the datatype is not a struct.
    pub fn require_struct(&self) -> Result<()> {
        self.validate()?;
        if !self.is_struct() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new("$"),
                reason: format_smolstr!(
                    "expected a struct root, got field {:?} of {}",
                    self.name(),
                    self.data_type()
                ),
            });
        }
        Ok(())
    }

    /// Validates that this field can serve as a record schema root.
    ///
    /// A root must be a non-null struct: nullable roots would make an entire
    /// row logically absent, which no row-oriented reader can represent.
    ///
    /// # Errors
    ///
    /// Returns an error naming what the field is when it is not a usable root.
    pub fn validate_struct_root(&self) -> Result<()> {
        self.validate()?;
        if self.is_nullable() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new("$"),
                reason: format_smolstr!(
                    "expected a non-null struct root, got nullable field {:?}",
                    self.name()
                ),
            });
        }
        if !self.is_struct() {
            return Err(Error::InvalidRecord {
                path: SmolStr::new("$"),
                reason: format_smolstr!(
                    "expected a struct root, got field {:?} of {}",
                    self.name(),
                    self.data_type()
                ),
            });
        }
        Ok(())
    }

    /// Returns the Arrow IPC dictionary identifier for dictionary fields.
    pub const fn dictionary_id(&self) -> Option<i64> {
        if matches!(self.data_type, DataType::Dictionary(_)) {
            Some(self.dictionary_id)
        } else {
            None
        }
    }

    /// Returns Arrow's dictionary ordering flag for dictionary fields.
    pub const fn dictionary_is_ordered(&self) -> Option<bool> {
        if matches!(self.data_type, DataType::Dictionary(_)) {
            Some(self.dictionary_is_ordered)
        } else {
            None
        }
    }

    /// Returns the number of metadata entries.
    pub fn metadata_len(&self) -> usize {
        self.metadata.len()
    }

    /// Returns the immutable shared metadata snapshot without allocating.
    pub const fn as_metadata(&self) -> &Metadata {
        &self.metadata
    }

    /// Returns whether the field has no metadata.
    pub fn is_metadata_empty(&self) -> bool {
        self.metadata.is_empty()
    }

    /// Iterates over metadata in lexical key order without allocating.
    pub fn metadata_iter(&self) -> MetadataIter<'_> {
        self.metadata.iter()
    }

    /// Returns the first metadata entry after `after_key`, or the first for `None`.
    pub fn next_metadata_entry(&self, after_key: Option<&str>) -> Option<(&str, &str)> {
        self.metadata.next_entry(after_key)
    }

    /// Looks up a metadata value without materializing a map.
    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key)
    }

    /// Returns whether a metadata key exists.
    pub fn has_metadata(&self, key: &str) -> bool {
        self.metadata.contains_key(key)
    }

    /// Returns the shared logical alias stored in metadata.
    pub fn alias(&self) -> Option<&str> {
        self.get_metadata(ALIAS_KEY)
    }

    /// Returns the shared catalog name stored in metadata.
    pub fn catalog_name(&self) -> Option<&str> {
        self.get_metadata(CATALOG_NAME_KEY)
    }

    /// Returns the shared schema name stored in metadata.
    pub fn schema_name(&self) -> Option<&str> {
        self.get_metadata(SCHEMA_NAME_KEY)
    }

    /// Returns the shared table name stored in metadata.
    pub fn table_name(&self) -> Option<&str> {
        self.get_metadata(TABLE_NAME_KEY)
    }

    /// Parses the Arrow/Parquet field identifier stored in metadata.
    ///
    /// Generic metadata construction validates and canonicalizes the
    /// `PARQUET:field_id` value, so an error can only originate from externally
    /// corrupted serialized state.
    pub fn parquet_field_id(&self) -> Result<Option<i32>> {
        self.get_metadata(PARQUET_FIELD_ID_KEY)
            .map(parse_field_id)
            .transpose()
    }

    /// Returns the field anywhere in this tree carrying one Arrow/Parquet
    /// field identifier.
    ///
    /// The walk is over every child a datatype has - struct and union members,
    /// a list's item, a map's entries, a run-end layout's two - because an
    /// identifier is unique across a whole schema and not only across one
    /// level of it.
    ///
    /// ```
    /// use yggdryl::DataType;
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut schema = DataType::from_fields([
    ///     DataType::Int64.required_field("id"),
    ///     DataType::list(DataType::Utf8.nullable_field("item")).nullable_field("tags"),
    /// ])?
    /// .required_field("row");
    ///
    /// assert_eq!(schema.assign_parquet_field_ids(1)?, 4);
    /// assert_eq!(
    ///     schema.field_by_parquet_field_id(3).map(yggdryl::Field::name),
    ///     Some("item"),
    /// );
    /// assert_eq!(schema.max_parquet_field_id()?, Some(3));
    /// # Ok(())
    /// # }
    /// ```
    pub fn field_by_parquet_field_id(&self, id: i32) -> Option<&Self> {
        if self.parquet_field_id().ok().flatten() == Some(id) {
            return Some(self);
        }
        (0..self.data_type.field_len())
            .filter_map(|index| self.data_type.get_field(index))
            .find_map(|child| child.field_by_parquet_field_id(id))
    }

    /// Returns the highest Arrow/Parquet field identifier anywhere in this
    /// tree.
    ///
    /// A schema evolution numbers above it, so an identifier is never reused
    /// for a different column.
    ///
    /// # Errors
    ///
    /// Returns an error when a stored identifier is not a canonical integer,
    /// which externally corrupted serialized state can produce.
    pub fn max_parquet_field_id(&self) -> Result<Option<i32>> {
        let mut highest = self.parquet_field_id()?;
        for index in 0..self.data_type.field_len() {
            let Some(child) = self.data_type.get_field(index) else {
                continue;
            };
            if let Some(id) = child.max_parquet_field_id()? {
                highest = Some(highest.map_or(id, |current: i32| current.max(id)));
            }
        }
        Ok(highest)
    }

    /// Numbers every field in this tree that does not already carry an
    /// Arrow/Parquet field identifier, and returns the next unused one.
    ///
    /// Children are numbered depth first in declaration order, which is the
    /// order every format that stores identifiers assigns them in. A field that
    /// already carries one keeps it, so numbering an evolved schema leaves the
    /// columns that already existed alone.
    ///
    /// # Errors
    ///
    /// Returns an error when the tree is not valid or an identifier would
    /// overflow.
    pub fn assign_parquet_field_ids(&mut self, start: i32) -> Result<i32> {
        self.validate()?;
        let mut next = start;
        self.assign_child_ids(&mut next)?;
        Ok(next)
    }

    /// Number one level of children, then each of their trees.
    fn assign_child_ids(&mut self, next: &mut i32) -> Result<()> {
        let count = self.data_type.field_len();
        if count == 0 {
            return Ok(());
        }
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let Some(child) = self.data_type.get_field(index) else {
                continue;
            };
            let mut child = child.clone();
            if child.parquet_field_id()?.is_none() {
                child.set_parquet_field_id(*next);
                *next = next.checked_add(1).ok_or_else(|| Error::InvalidRecord {
                    path: format_smolstr!("$.{}", child.name()),
                    reason: crate::text::expected_got(
                        format_args!("a field identifier below {}", i32::MAX),
                        format_args!("an overflow"),
                    ),
                })?;
            }
            child.assign_child_ids(next)?;
            children.push(child);
        }
        self.set_data_type(self.data_type.with_fields(children)?)
    }

    /// Returns whether this field participates in caller-side initialization.
    ///
    /// The reserved `field:init` metadata key is absent for an ordinary field,
    /// which reports `true`. Set it to `false` to mark a field that a schema
    /// still declares but a constructor must not accept, such as a value
    /// derived after construction.
    ///
    /// # Errors
    ///
    /// Generic metadata construction validates this reserved key, so an error
    /// can only originate from externally corrupted serialized state.
    pub fn is_init(&self) -> Result<bool> {
        self.get_metadata(FIELD_INIT_KEY)
            .map_or(Ok(true), |value| parse_reserved_bool(FIELD_INIT_KEY, value))
    }

    /// Parses the canonical location metadata as a typed URL.
    ///
    /// Generic metadata construction validates this reserved key, so an error
    /// can only originate from externally corrupted serialized state.
    pub fn location(&self) -> Result<Option<Url>> {
        self.get_metadata(LOCATION_KEY)
            .map(Url::from_str)
            .transpose()
    }

    /// Returns the raw HTTP `Accept` field value.
    pub fn accept(&self) -> Option<&str> {
        self.metadata.accept()
    }

    /// Returns the raw HTTP `Accept-Encoding` field value.
    pub fn accept_encoding(&self) -> Option<&str> {
        self.metadata.accept_encoding()
    }

    /// Returns the raw HTTP `Accept-Language` field value.
    pub fn accept_language(&self) -> Option<&str> {
        self.metadata.accept_language()
    }

    /// Returns the raw HTTP `Accept-Ranges` field value.
    pub fn accept_ranges(&self) -> Option<&str> {
        self.metadata.accept_ranges()
    }

    /// Returns the raw HTTP `Cache-Control` field value.
    pub fn cache_control(&self) -> Option<&str> {
        self.metadata.cache_control()
    }

    /// Returns the raw HTTP `Content-Disposition` field value.
    pub fn content_disposition(&self) -> Option<&str> {
        self.metadata.content_disposition()
    }

    /// Returns the raw HTTP `Content-Encoding` field value.
    pub fn content_encoding(&self) -> Option<&str> {
        self.metadata.content_encoding()
    }

    /// Returns the raw HTTP `Content-Language` field value.
    pub fn content_language(&self) -> Option<&str> {
        self.metadata.content_language()
    }

    /// Parses the canonical HTTP `Content-Length` field value.
    pub fn content_length(&self) -> Result<Option<u64>> {
        self.metadata.content_length()
    }

    /// Returns the raw HTTP `Content-Location` field value.
    pub fn content_location(&self) -> Option<&str> {
        self.metadata.content_location()
    }

    /// Returns the raw HTTP `Content-Range` field value.
    pub fn content_range(&self) -> Option<&str> {
        self.metadata.content_range()
    }

    /// Returns the raw HTTP `Content-Type` field value, including parameters.
    pub fn content_type(&self) -> Option<&str> {
        self.metadata.content_type()
    }

    /// Parses the base MIME type from HTTP `Content-Type`.
    pub fn mime_type(&self) -> Result<MimeType> {
        self.metadata.mime_type()
    }

    /// Parses HTTP `Content-Type` and `Content-Encoding` as one media value.
    pub fn media_type(&self) -> Result<MediaType> {
        self.metadata.media_type()
    }

    /// Returns the raw HTTP `ETag` field value.
    pub fn etag(&self) -> Option<&str> {
        self.metadata.etag()
    }

    /// Returns the raw HTTP `Expires` field value.
    pub fn expires(&self) -> Option<&str> {
        self.metadata.expires()
    }

    /// Returns the raw HTTP `Last-Modified` field value.
    pub fn last_modified(&self) -> Option<&str> {
        self.metadata.last_modified()
    }

    /// Parses HTTP `Location` as an absolute URL.
    pub fn http_location(&self) -> Result<Option<Url>> {
        self.metadata.http_location()
    }

    /// Returns the raw HTTP `Range` field value.
    pub fn range(&self) -> Option<&str> {
        self.metadata.range()
    }

    /// Returns the raw HTTP `Vary` field value.
    pub fn vary(&self) -> Option<&str> {
        self.metadata.vary()
    }

    /// Looks up one canonical `scheme:name` property without allocating.
    pub fn get_property(&self, scheme: &Scheme, name: &str) -> Option<&str> {
        self.metadata.get_property(scheme, name)
    }

    /// Returns whether one canonical `scheme:name` property exists.
    pub fn has_property(&self, scheme: &Scheme, name: &str) -> bool {
        self.metadata.has_property(scheme, name)
    }

    /// Iterates over a protocol's property suffixes and values without allocating.
    pub fn property_iter<'field, 'scheme>(
        &'field self,
        scheme: &'scheme Scheme,
    ) -> PropertyIter<'field, 'scheme> {
        self.metadata.property_iter(scheme)
    }

    /// Returns the first protocol property after `after_name`.
    pub fn next_property_entry<'field>(
        &'field self,
        scheme: &Scheme,
        after_name: Option<&str>,
    ) -> Option<(&'field str, &'field str)> {
        self.metadata.next_property_entry(scheme, after_name)
    }

    /// Returns a borrowed view of one protocol's properties.
    ///
    /// The view remembers the protocol, so a caller spells the bare property
    /// name and never assembles a `scheme:name` key itself. Nothing is copied:
    /// it borrows this field's metadata and reads out of the same tree.
    ///
    /// ```
    /// use yggdryl::{DataType, Scheme};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut field = DataType::Int64.required_field("price");
    /// field.set_property(&Scheme::ICEBERG, "doc", "closing price")?;
    ///
    /// assert_eq!(field.protocol(&Scheme::ICEBERG).get("doc"), Some("closing price"));
    /// assert_eq!(field.iceberg().get("doc"), Some("closing price"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn protocol(&self, scheme: &Scheme) -> ProtocolMetadata<'_> {
        self.metadata.protocol(scheme)
    }

    /// Returns a mutable view of one protocol's properties.
    ///
    /// Every write goes through this field's own cache-aware mutation, so a
    /// protocol write invalidates a populated Arrow projection exactly as a
    /// direct metadata write does.
    ///
    /// ```
    /// use yggdryl::{DataType, Scheme};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut field = DataType::Int64.required_field("price");
    ///
    /// field.protocol_mut(&Scheme::ICEBERG).insert("doc", "closing price")?;
    ///
    /// assert_eq!(field.get_metadata("iceberg:doc"), Some("closing price"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn protocol_mut(&mut self, scheme: &Scheme) -> ProtocolMetadataMut<'_> {
        ProtocolMetadataMut::new(self, scheme.clone())
    }

    for_each_well_known_protocol!(field_protocol_accessors);

    /// Changes the field name and invalidates a populated Arrow cache once.
    pub fn set_name(&mut self, name: impl Into<SmolStr>) {
        let name = name.into();
        if self.name != name {
            self.name = name;
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent copy with a different name.
    pub fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.set_name(name);
        self
    }

    /// Validates and replaces the datatype, leaving `self` unchanged on error.
    pub fn set_data_type(&mut self, data_type: DataType) -> Result<()> {
        data_type.validate()?;
        if self.data_type != data_type {
            self.data_type = data_type;
            if !matches!(self.data_type, DataType::Dictionary(_)) {
                self.dictionary_id = 0;
                self.dictionary_is_ordered = false;
            }
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent copy with a validated datatype.
    pub fn try_with_data_type(mut self, data_type: DataType) -> Result<Self> {
        self.set_data_type(data_type)?;
        Ok(self)
    }

    /// Changes nullability.
    pub fn set_nullable(&mut self, nullable: bool) {
        if self.nullable != nullable {
            self.nullable = nullable;
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent copy with different nullability.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.set_nullable(nullable);
        self
    }

    /// Replaces Arrow IPC dictionary options on a dictionary-typed field.
    pub fn set_dictionary_options(&mut self, id: i64, is_ordered: bool) -> Result<()> {
        if !matches!(self.data_type, DataType::Dictionary(_)) {
            return Err(Error::InvalidDataType {
                kind: "Field",
                reason: "dictionary options require a dictionary datatype".into(),
            });
        }
        if self.dictionary_id != id || self.dictionary_is_ordered != is_ordered {
            self.dictionary_id = id;
            self.dictionary_is_ordered = is_ordered;
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent field with different Arrow IPC dictionary options.
    pub fn try_with_dictionary_options(mut self, id: i64, is_ordered: bool) -> Result<Self> {
        self.set_dictionary_options(id, is_ordered)?;
        Ok(self)
    }

    /// Inserts or replaces one metadata entry.
    pub fn insert_metadata(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Option<String>> {
        let key = key.into();
        let value = value.into();
        let (previous, changed) = self.metadata.insert(key, value)?;
        if changed {
            self.invalidate_arrow();
        }
        Ok(previous)
    }

    /// Replaces all metadata atomically and validates duplicates before change.
    pub fn set_metadata<I, K, V>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let metadata = Metadata::from_entries(values)?;
        if self.metadata != metadata {
            self.metadata = metadata;
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Overlays validated metadata atomically through copy-on-write storage.
    pub fn update_metadata<I, K, V>(&mut self, values: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let overlay = Metadata::from_entries(values)?;
        if self.metadata.update(overlay) {
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent copy containing one metadata entry.
    pub fn try_with_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self> {
        self.insert_metadata(key, value)?;
        Ok(self)
    }

    /// Returns a persistent copy after a bulk metadata overlay.
    pub fn try_with_metadata_entries<I, K, V>(mut self, values: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.update_metadata(values)?;
        Ok(self)
    }

    /// Removes one metadata key, returning its prior value.
    pub fn remove_metadata(&mut self, key: &str) -> Option<String> {
        let previous = self.metadata.remove(key);
        if previous.is_some() {
            self.invalidate_arrow();
        }
        previous
    }

    /// Returns a persistent copy without one metadata key.
    pub fn with_metadata_removed(mut self, key: &str) -> Self {
        self.remove_metadata(key);
        self
    }

    /// Removes all metadata without allocating.
    pub fn clear_metadata(&mut self) {
        if self.metadata.clear() {
            self.invalidate_arrow();
        }
    }

    /// Sets a validated logical alias.
    pub fn set_alias(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(ALIAS_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated logical alias.
    pub fn try_with_alias(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_alias(value)?;
        Ok(self)
    }

    /// Removes and returns the logical alias.
    pub fn remove_alias(&mut self) -> Option<String> {
        self.remove_metadata(ALIAS_KEY)
    }

    /// Sets a validated catalog name.
    pub fn set_catalog_name(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(CATALOG_NAME_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated catalog name.
    pub fn try_with_catalog_name(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_catalog_name(value)?;
        Ok(self)
    }

    /// Removes and returns the catalog name.
    pub fn remove_catalog_name(&mut self) -> Option<String> {
        self.remove_metadata(CATALOG_NAME_KEY)
    }

    /// Sets a validated schema name.
    pub fn set_schema_name(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(SCHEMA_NAME_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated schema name.
    pub fn try_with_schema_name(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_schema_name(value)?;
        Ok(self)
    }

    /// Removes and returns the schema name.
    pub fn remove_schema_name(&mut self) -> Option<String> {
        self.remove_metadata(SCHEMA_NAME_KEY)
    }

    /// Sets a validated table name.
    pub fn set_table_name(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(TABLE_NAME_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated table name.
    pub fn try_with_table_name(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_table_name(value)?;
        Ok(self)
    }

    /// Removes and returns the table name.
    pub fn remove_table_name(&mut self) -> Option<String> {
        self.remove_metadata(TABLE_NAME_KEY)
    }

    /// Sets the canonical Arrow/Parquet signed 32-bit field identifier.
    pub fn set_parquet_field_id(&mut self, id: i32) {
        let (_, changed) = self
            .metadata
            .insert_validated(PARQUET_FIELD_ID_KEY.to_owned(), id.to_string());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field with an Arrow/Parquet field identifier.
    pub fn with_parquet_field_id(mut self, id: i32) -> Self {
        self.set_parquet_field_id(id);
        self
    }

    /// Records whether this field participates in caller-side initialization.
    ///
    /// Setting `true` removes the reserved key rather than storing a redundant
    /// default, so an ordinary field never carries metadata it does not need.
    pub fn set_init(&mut self, init: bool) {
        if init {
            self.remove_metadata(FIELD_INIT_KEY);
            return;
        }
        let (_, changed) = self
            .metadata
            .insert_validated(FIELD_INIT_KEY.to_owned(), "false".to_owned());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field with its initialization participation set.
    pub fn with_init(mut self, init: bool) -> Self {
        self.set_init(init);
        self
    }

    /// Marks or unmarks this field as one a path spells out.
    ///
    /// An ordinary field carries no marker at all, so unmarking removes the
    /// reserved key rather than storing the default it already means. That
    /// keeps two schemas that partition the same way exactly equal.
    pub fn set_partition(&mut self, partition: bool) {
        if !partition {
            self.remove_metadata(FIELD_PARTITION_KEY);
            return;
        }
        let (_, changed) = self
            .metadata
            .insert_validated(FIELD_PARTITION_KEY.to_owned(), "true".to_owned());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field marked or unmarked as a partition column.
    pub fn with_partition(mut self, partition: bool) -> Self {
        self.set_partition(partition);
        self
    }

    /// Removes and parses the prior Arrow/Parquet field identifier.
    pub fn remove_parquet_field_id(&mut self) -> Result<Option<i32>> {
        self.remove_metadata(PARQUET_FIELD_ID_KEY)
            .map(|value| parse_field_id(&value))
            .transpose()
    }

    /// Sets the canonical typed location URL.
    pub fn set_location(&mut self, location: Url) {
        let (_, changed) = self
            .metadata
            .insert_validated(LOCATION_KEY.to_owned(), location.to_string());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field with a canonical typed location URL.
    pub fn with_location(mut self, location: Url) -> Self {
        self.set_location(location);
        self
    }

    /// Removes and parses the prior typed location URL.
    pub fn remove_location(&mut self) -> Result<Option<Url>> {
        self.remove_metadata(LOCATION_KEY)
            .map(|value| Url::from_str(&value))
            .transpose()
    }

    /// Sets a validated raw HTTP `Accept` field value.
    pub fn set_accept(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_ACCEPT_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept` field value.
    pub fn remove_accept(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_ACCEPT_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Encoding` field value.
    pub fn set_accept_encoding(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_ACCEPT_ENCODING_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Encoding` field value.
    pub fn remove_accept_encoding(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_ACCEPT_ENCODING_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Language` field value.
    pub fn set_accept_language(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_ACCEPT_LANGUAGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Language` field value.
    pub fn remove_accept_language(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_ACCEPT_LANGUAGE_KEY)
    }

    /// Sets a validated raw HTTP `Accept-Ranges` field value.
    pub fn set_accept_ranges(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_ACCEPT_RANGES_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Accept-Ranges` field value.
    pub fn remove_accept_ranges(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_ACCEPT_RANGES_KEY)
    }

    /// Sets a validated raw HTTP `Cache-Control` field value.
    pub fn set_cache_control(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CACHE_CONTROL_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Cache-Control` field value.
    pub fn remove_cache_control(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CACHE_CONTROL_KEY)
    }

    /// Sets a validated raw HTTP `Content-Disposition` field value.
    pub fn set_content_disposition(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CONTENT_DISPOSITION_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Disposition` field value.
    pub fn remove_content_disposition(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CONTENT_DISPOSITION_KEY)
    }

    /// Sets a validated raw HTTP `Content-Encoding` field value.
    pub fn set_content_encoding(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CONTENT_ENCODING_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Encoding` field value.
    pub fn remove_content_encoding(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CONTENT_ENCODING_KEY)
    }

    /// Sets a validated raw HTTP `Content-Language` field value.
    pub fn set_content_language(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CONTENT_LANGUAGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Language` field value.
    pub fn remove_content_language(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CONTENT_LANGUAGE_KEY)
    }

    /// Sets canonical HTTP `Content-Length` metadata.
    pub fn set_content_length(&mut self, value: u64) {
        let (_, changed) = self
            .metadata
            .insert_validated(HTTP_CONTENT_LENGTH_KEY.to_owned(), value.to_string());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field with canonical HTTP `Content-Length` metadata.
    pub fn with_content_length(mut self, value: u64) -> Self {
        self.set_content_length(value);
        self
    }

    /// Removes and parses the prior HTTP `Content-Length` value.
    pub fn remove_content_length(&mut self) -> Result<Option<u64>> {
        let previous = self.content_length()?;
        if previous.is_some() {
            self.remove_metadata(HTTP_CONTENT_LENGTH_KEY);
        }
        Ok(previous)
    }

    /// Sets a validated raw HTTP `Content-Location` field value.
    pub fn set_content_location(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CONTENT_LOCATION_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Location` field value.
    pub fn remove_content_location(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CONTENT_LOCATION_KEY)
    }

    /// Sets a validated raw HTTP `Content-Range` field value.
    pub fn set_content_range(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CONTENT_RANGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Range` field value.
    pub fn remove_content_range(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CONTENT_RANGE_KEY)
    }

    /// Sets a validated raw HTTP `Content-Type` field value.
    pub fn set_content_type(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_CONTENT_TYPE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Content-Type` field value.
    pub fn remove_content_type(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_CONTENT_TYPE_KEY)
    }

    /// Sets the bare HTTP `Content-Type` MIME value and preserves encodings.
    pub fn set_mime_type(&mut self, value: MimeType) {
        let (_, changed) = self
            .metadata
            .insert_validated(HTTP_CONTENT_TYPE_KEY.to_owned(), value.to_string());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field with a bare HTTP `Content-Type` MIME value.
    pub fn with_mime_type(mut self, value: MimeType) -> Self {
        self.set_mime_type(value);
        self
    }

    /// Removes and parses the prior HTTP `Content-Type` MIME value.
    ///
    /// Existing `Content-Encoding` metadata is deliberately preserved.
    pub fn remove_mime_type(&mut self) -> Result<Option<MimeType>> {
        let Some(content_type) = self.content_type() else {
            return Ok(None);
        };
        let previous = MimeType::from_content_type(content_type)?;
        self.remove_metadata(HTTP_CONTENT_TYPE_KEY);
        Ok(Some(previous))
    }

    /// Atomically projects a media value to HTTP content headers.
    ///
    /// File encodings without registered HTTP coding tokens are rejected
    /// before either metadata key or the Arrow projection cache is changed.
    pub fn set_media_type(&mut self, value: MediaType) -> Result<()> {
        let content_type = value.base().to_string();
        let mut content_encoding = String::new();
        for encoding in value.encodings() {
            let coding = encoding
                .content_coding()
                .ok_or_else(|| Error::InvalidMetadataValue {
                    key: SmolStr::new_static(HTTP_CONTENT_ENCODING_KEY),
                    reason: SmolStr::new_static(
                        "media encoding has no registered HTTP Content-Encoding token",
                    ),
                })?;
            if !content_encoding.is_empty() {
                content_encoding.push_str(", ");
            }
            content_encoding.push_str(coding);
        }

        let mut metadata = self.metadata.clone();
        metadata.insert_validated(HTTP_CONTENT_TYPE_KEY.to_owned(), content_type);
        if content_encoding.is_empty() {
            metadata.remove(HTTP_CONTENT_ENCODING_KEY);
        } else {
            metadata.insert_validated(HTTP_CONTENT_ENCODING_KEY.to_owned(), content_encoding);
        }
        if metadata != self.metadata {
            self.metadata = metadata;
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent field with atomically projected HTTP media headers.
    pub fn try_with_media_type(mut self, value: MediaType) -> Result<Self> {
        self.set_media_type(value)?;
        Ok(self)
    }

    /// Removes both HTTP media header keys after parsing their prior value.
    ///
    /// If either stored header is malformed, this field remains unchanged.
    pub fn remove_media_type(&mut self) -> Result<Option<MediaType>> {
        if self.content_type().is_none() && self.content_encoding().is_none() {
            return Ok(None);
        }
        let previous = self.media_type()?;
        let mut metadata = self.metadata.clone();
        metadata.remove(HTTP_CONTENT_TYPE_KEY);
        metadata.remove(HTTP_CONTENT_ENCODING_KEY);
        self.metadata = metadata;
        self.invalidate_arrow();
        Ok(Some(previous))
    }

    /// Sets a validated raw HTTP `ETag` field value.
    pub fn set_etag(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_ETAG_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `ETag` field value.
    pub fn remove_etag(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_ETAG_KEY)
    }

    /// Sets a validated raw HTTP `Expires` field value.
    pub fn set_expires(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_EXPIRES_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Expires` field value.
    pub fn remove_expires(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_EXPIRES_KEY)
    }

    /// Sets a validated raw HTTP `Last-Modified` field value.
    pub fn set_last_modified(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_LAST_MODIFIED_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Last-Modified` field value.
    pub fn remove_last_modified(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_LAST_MODIFIED_KEY)
    }

    /// Sets typed absolute HTTP `Location` metadata.
    pub fn set_http_location(&mut self, value: Url) {
        let (_, changed) = self
            .metadata
            .insert_validated(HTTP_LOCATION_KEY.to_owned(), value.to_string());
        if changed {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field with typed absolute HTTP `Location` metadata.
    pub fn with_http_location(mut self, value: Url) -> Self {
        self.set_http_location(value);
        self
    }

    /// Removes and parses the prior typed HTTP `Location` URL.
    pub fn remove_http_location(&mut self) -> Result<Option<Url>> {
        let previous = self.http_location()?;
        if previous.is_some() {
            self.remove_metadata(HTTP_LOCATION_KEY);
        }
        Ok(previous)
    }

    /// Sets a validated raw HTTP `Range` field value.
    pub fn set_range(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_RANGE_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Range` field value.
    pub fn remove_range(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_RANGE_KEY)
    }

    /// Sets a validated raw HTTP `Vary` field value.
    pub fn set_vary(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(HTTP_VARY_KEY, value)?;
        Ok(())
    }

    /// Removes and returns the raw HTTP `Vary` field value.
    pub fn remove_vary(&mut self) -> Option<String> {
        self.remove_metadata(HTTP_VARY_KEY)
    }

    /// Sets one validated canonical `scheme:name` property.
    pub fn set_property(
        &mut self,
        scheme: &Scheme,
        name: &str,
        value: impl Into<String>,
    ) -> Result<Option<String>> {
        self.insert_metadata(property_key(scheme, name), value)
    }

    /// Returns a persistent field with one validated protocol property.
    pub fn try_with_property(
        mut self,
        scheme: &Scheme,
        name: &str,
        value: impl Into<String>,
    ) -> Result<Self> {
        self.set_property(scheme, name, value)?;
        Ok(self)
    }

    /// Removes and returns one canonical protocol property.
    pub fn remove_property(&mut self, scheme: &Scheme, name: &str) -> Option<String> {
        self.remove_metadata(&property_key(scheme, name))
    }

    /// Removes every property for one protocol without affecting shared keys.
    pub fn clear_properties(&mut self, scheme: &Scheme) {
        if self.metadata.remove_properties(scheme) {
            self.invalidate_arrow();
        }
    }

    /// Returns a persistent field without properties for one protocol.
    pub fn with_properties_cleared(mut self, scheme: &Scheme) -> Self {
        self.clear_properties(scheme);
        self
    }

    /// Validates the complete recursive datatype.
    pub fn validate(&self) -> Result<()> {
        self.data_type.validate()?;
        if !matches!(self.data_type, DataType::Dictionary(_))
            && (self.dictionary_id != 0 || self.dictionary_is_ordered)
        {
            return Err(Error::InvalidDataType {
                kind: "Field",
                reason: "dictionary options require a dictionary datatype".into(),
            });
        }
        Ok(())
    }

    /// Validates a caller-built Field after a bounded iterative shape walk.
    ///
    /// Foreign projection boundaries use this method so arbitrarily nested
    /// public datatype variants cannot exhaust the stack before returning a
    /// normal validation error.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema exceeds hard shape limits or this
    /// Field contains an invalid datatype/dictionary-option combination.
    #[doc(hidden)]
    pub fn validate_bounded(&self) -> Result<()> {
        preflight_schema_shape(self.data_type(), "Field")?;
        self.validate()
    }

    /// Compares fields, optionally including metadata at every nesting level.
    ///
    /// With `with_metadata = true`, this is exactly [`PartialEq`]. With
    /// `with_metadata = false`, metadata is ignored recursively while field
    /// names, nullability, dictionary state, and datatype parameters remain
    /// significant.
    pub fn equals(&self, other: &Self, with_metadata: bool) -> bool {
        diff::fields_equal(self, other, with_metadata)
    }

    /// Lazily yields stable, UTF-8 lines describing every difference.
    ///
    /// `return_equal` decides what an equal comparison yields: `false`
    /// yields nothing, and `true` yields one equal line so a caller
    /// rendering a full report never shows an empty section.
    pub fn show_diffs<'schema>(
        &'schema self,
        other: &'schema Self,
        with_metadata: bool,
        return_equal: bool,
    ) -> Differences<'schema> {
        Differences::from_fields(self, other, with_metadata, return_equal)
    }

    /// Returns all formatted differences joined with newlines.
    ///
    /// Equal fields produce `✓ equal`.
    pub fn show_diff(&self, other: &Self, with_metadata: bool, return_equal: bool) -> String {
        diff::show_diff(self.show_diffs(other, with_metadata, return_equal))
    }

    /// Compares nested layout while deliberately ignoring all metadata.
    pub fn layout_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.name == other.name
                && self.nullable == other.nullable
                && data_type_layout_eq(&self.data_type, &other.data_type)
    }

    /// Returns a deterministic cross-language hash of canonical display output.
    pub fn stable_hash(&self) -> u64 {
        stable_hash_display(self)
    }

    /// Returns a deterministic hash of name, datatype, and nullability only.
    ///
    /// This is intended for runtimes that expose mutable metadata on otherwise
    /// fixed field wrappers. Exact-equal fields always share this hash, while a
    /// metadata update cannot invalidate a runtime hash-table entry.
    pub fn stable_layout_hash(&self) -> u64 {
        stable_hash_display(&FieldLayoutDisplay(self))
    }

    fn invalidate_arrow(&mut self) {
        self.arrow.take();
    }

    /// Reports whether this imported field still owns its exact Arrow projection.
    ///
    /// Arrow import uses an empty cache to propagate that validation
    /// canonicalized this field or one of its descendants. Keeping this check
    /// private to the import pipeline avoids rebuilding or comparing an Arrow
    /// subtree merely to decide whether its parent projection is reusable.
    pub(crate) fn arrow_import_is_projection_equivalent(&self) -> bool {
        self.arrow.get().is_some()
    }
}

impl Clone for Field {
    fn clone(&self) -> Self {
        let arrow = OnceLock::new();
        if let Some(cached) = self.arrow.get() {
            let _ = arrow.set(Arc::clone(cached));
        }
        Self {
            name: self.name.clone(),
            data_type: self.data_type.clone(),
            nullable: self.nullable,
            dictionary_id: self.dictionary_id,
            dictionary_is_ordered: self.dictionary_is_ordered,
            metadata: self.metadata.clone(),
            arrow,
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

impl fmt::Debug for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Field")
            .field("name", &self.name)
            .field("data_type", &self.data_type)
            .field("nullable", &self.nullable)
            .field("dictionary_id", &self.dictionary_id)
            .field("dictionary_is_ordered", &self.dictionary_is_ordered)
            .field("metadata", &self.metadata)
            .finish()
    }
}

struct FieldLayoutDisplay<'a>(&'a Field);

impl fmt::Display for FieldLayoutDisplay<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("field(")?;
        write_quoted(formatter, &self.0.name)?;
        write!(
            formatter,
            ",{},nullable={})",
            self.0.data_type, self.0.nullable
        )
    }
}

impl fmt::Display for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("field(")?;
        write_quoted(formatter, &self.name)?;
        write!(
            formatter,
            ",{},{}",
            self.data_type,
            if self.nullable {
                "nullable=true"
            } else {
                "nullable=false"
            }
        )?;
        if self.dictionary_id != 0 {
            write!(formatter, ",dictionary_id={}", self.dictionary_id)?;
        }
        if self.dictionary_is_ordered {
            formatter.write_str(",dictionary_is_ordered=true")?;
        }
        formatter.write_str(",")?;
        formatter.write_str("metadata={")?;
        for (index, (key, value)) in self.metadata.iter().enumerate() {
            if index != 0 {
                formatter.write_str(",")?;
            }
            write_quoted(formatter, key)?;
            formatter.write_str(":")?;
            write_quoted(formatter, value)?;
        }
        formatter.write_str("})")
    }
}

impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.name == other.name
                && self.data_type == other.data_type
                && self.nullable == other.nullable
                && self.dictionary_id == other.dictionary_id
                && self.dictionary_is_ordered == other.dictionary_is_ordered
                && self.metadata == other.metadata
    }
}

impl Eq for Field {}

impl PartialOrd for Field {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Field {
    fn cmp(&self, other: &Self) -> Ordering {
        if std::ptr::eq(self, other) {
            return Ordering::Equal;
        }
        (
            &self.name,
            &self.data_type,
            self.nullable,
            self.dictionary_id,
            self.dictionary_is_ordered,
            &self.metadata,
        )
            .cmp(&(
                &other.name,
                &other.data_type,
                other.nullable,
                other.dictionary_id,
                other.dictionary_is_ordered,
                &other.metadata,
            ))
    }
}

impl Hash for Field {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.data_type.hash(state);
        self.nullable.hash(state);
        self.dictionary_id.hash(state);
        self.dictionary_is_ordered.hash(state);
        self.metadata.hash(state);
    }
}

impl Index<&str> for Field {
    type Output = str;

    fn index(&self, key: &str) -> &Self::Output {
        self.get_metadata(key)
            .unwrap_or_else(|| panic!("metadata key {key:?} is not present"))
    }
}

fn field_layout_eq(left: &Field, right: &Field) -> bool {
    left.layout_eq(right)
}

#[allow(clippy::too_many_lines)]
fn data_type_layout_eq(left: &DataType, right: &DataType) -> bool {
    if std::ptr::eq(left, right) {
        return true;
    }
    use DataType as D;
    match (left, right) {
        (D::List(left), D::List(right))
        | (D::ListView(left), D::ListView(right))
        | (D::LargeList(left), D::LargeList(right))
        | (D::LargeListView(left), D::LargeListView(right)) => field_layout_eq(left, right),
        (D::FixedSizeList(left, left_size), D::FixedSizeList(right, right_size)) => {
            left_size == right_size && field_layout_eq(left, right)
        }
        (D::Struct(left), D::Struct(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| field_layout_eq(left, right))
        }
        (D::Union(left, left_mode), D::Union(right, right_mode)) => {
            left_mode == right_mode
                && left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|((left_id, left), (right_id, right))| {
                        left_id == right_id && field_layout_eq(left, right)
                    })
        }
        (D::Dictionary(left), D::Dictionary(right)) => {
            data_type_layout_eq(left.key(), right.key())
                && data_type_layout_eq(left.value(), right.value())
        }
        (D::Map(left), D::Map(right)) => map_layout_eq(left, right),
        (D::RunEndEncoded(left), D::RunEndEncoded(right)) => run_layout_eq(left, right),
        _ => left == right,
    }
}

fn map_layout_eq(left: &MapType, right: &MapType) -> bool {
    left.keys_sorted() == right.keys_sorted() && field_layout_eq(left.entries(), right.entries())
}

fn run_layout_eq(left: &RunEndEncodedType, right: &RunEndEncodedType) -> bool {
    field_layout_eq(left.run_ends(), right.run_ends())
        && field_layout_eq(left.values(), right.values())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    use super::Field;
    use crate::{DataType, Error};

    #[test]
    fn canonical_display_json_and_arrow_round_trip() {
        let field = Field::new(
            "items",
            DataType::list(Field::new("item", DataType::Utf8, true)),
            false,
        )
        .try_with_metadata("source", "a, b")
        .unwrap();

        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
        assert_eq!(Field::from_json(&field.to_json().unwrap()).unwrap(), field);
        let arrow = field.to_arrow_ref().unwrap();
        assert!(Arc::ptr_eq(&arrow, &field.to_arrow_ref().unwrap()));
        assert_eq!(Field::from_arrow(arrow.as_ref()).unwrap(), field);
    }

    #[test]
    fn sql_hive_and_wrapped_forms_parse() {
        assert_eq!(
            Field::from_str("id bigint not null").unwrap().data_type(),
            &DataType::Int64
        );
        assert!(!Field::from_str("id bigint not null").unwrap().is_nullable());
        assert_eq!(
            Field::from_str("['events': array<struct<id:bigint,name:string>>]")
                .unwrap()
                .name(),
            "events"
        );
        assert!(
            !Field::from_str("id bigint  NOT \t NULL")
                .unwrap()
                .is_nullable()
        );
        assert_eq!(Field::from_str("'it''s': string").unwrap().name(), "it's");
        assert_eq!(Field::from_str(r#""a""b": string"#).unwrap().name(), "a\"b");
        assert_eq!(Field::from_str("[a]]b] string").unwrap().name(), "a]b");
    }

    #[test]
    #[allow(deprecated)]
    fn arrow_display_and_dictionary_state_round_trip_after_cache_invalidation() {
        let arrow = ArrowField::new_dict(
            "codes",
            ArrowDataType::Dictionary(
                Box::new(ArrowDataType::Int16),
                Box::new(ArrowDataType::Utf8),
            ),
            true,
            42,
            true,
        )
        .with_metadata(std::collections::HashMap::from([(
            "source".to_owned(),
            "ipc".to_owned(),
        )]));
        let mut field = Field::from_arrow(&arrow).unwrap();
        assert_eq!(Field::from_str(&arrow.to_string()).unwrap(), field);
        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
        assert_eq!(field.dictionary_id(), Some(42));
        assert_eq!(field.dictionary_is_ordered(), Some(true));

        let cached = field.to_arrow_ref().unwrap();
        field.set_dictionary_options(42, true).unwrap();
        assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));
        field.set_dictionary_options(7, false).unwrap();
        assert!(!Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));
        field.set_dictionary_options(42, true).unwrap();

        field.set_name("renamed");
        let rebuilt = field.to_arrow().unwrap();
        assert_eq!(rebuilt.dict_id(), Some(42));
        assert_eq!(rebuilt.dict_is_ordered(), Some(true));

        let shared = Arc::new(arrow);
        let imported = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
        assert!(Arc::ptr_eq(&shared, &imported.to_arrow_ref().unwrap()));
    }

    #[test]
    fn wrappers_are_bounded_and_nested_errors_use_field_offsets() {
        let accepted = format!(
            "{}id:int64{}",
            "(".repeat(DataType::PARSE_RECURSION_LIMIT),
            ")".repeat(DataType::PARSE_RECURSION_LIMIT)
        );
        assert_eq!(Field::from_str(&accepted).unwrap().name(), "id");
        let rejected_depth = DataType::PARSE_RECURSION_LIMIT + 1;
        let rejected = format!(
            "{}id:int64{}",
            "(".repeat(rejected_depth),
            ")".repeat(rejected_depth)
        );
        assert!(Field::from_str(&rejected).is_err());

        let error = Field::from_str("id: struct<x: definitely_bad>").unwrap_err();
        assert!(matches!(
            error,
            Error::Parse {
                target: "field",
                position: 3..,
                ..
            }
        ));
    }

    #[test]
    fn metadata_updates_are_sorted_atomic_and_cache_aware() {
        let mut field = Field::new("id", DataType::Int64, false);
        field
            .update_metadata([("z", "last"), ("a", "first")])
            .unwrap();
        assert_eq!(
            field.metadata_iter().collect::<Vec<_>>(),
            vec![("a", "first"), ("z", "last")]
        );
        let cached = field.to_arrow_ref().unwrap();
        field.insert_metadata("a", "first").unwrap();
        assert!(Arc::ptr_eq(&cached, &field.to_arrow_ref().unwrap()));
        assert!(field.update_metadata([("", "bad")]).is_err());
        assert_eq!(field.metadata_len(), 2);
    }

    #[test]
    #[allow(clippy::mutable_key_type)]
    fn native_order_hash_and_stable_hash_ignore_cache() {
        let first = Field::new("a", DataType::Int64, false);
        let second = Field::new("b", DataType::Int64, false);
        let mut ordered = BTreeSet::new();
        ordered.insert(second.clone());
        ordered.insert(first.clone());
        assert_eq!(ordered.into_iter().next().unwrap(), first);
        let mut hashed = HashSet::new();
        hashed.insert(second.clone());
        assert!(hashed.contains(&second));
        let before = second.stable_hash();
        second.to_arrow_ref().unwrap();
        assert_eq!(before, second.stable_hash());
    }
}
