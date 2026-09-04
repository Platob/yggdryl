//! Generic and datatype-specific Arrow-compatible field values.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::sync::{Arc, OnceLock};

use arrow_schema::Field as ArrowField;
use smol_str::{SmolStr, format_smolstr};

use crate::metadata::{
    ALIAS_KEY, COMMENT_KEY, DISPLAY_KEY, FIELD_INIT_KEY, FIELD_PARTITION_KEY, LOCATION_KEY,
    MetadataIter, PARQUET_FIELD_ID_KEY, PropertyIter, for_each_well_known_protocol, parse_field_id,
    parse_reserved_bool, property_key, write_json_string as write_quoted,
};
use crate::types::{DataType, preflight_schema_shape};
use crate::{Error, Metadata, Result, Scheme, Url};

use super::protocol::{self, ProtocolField, ProtocolFieldMut};

/// Emit the borrowed and mutable named views of one protocol on a field.
macro_rules! field_protocol_accessors {
    ($name:ident, $mutable:ident, $constant:ident, $view:ident, $view_mut:ident, $label:literal) => {
        #[doc = concat!("Returns this field borrowed as its ", $label, " protocol.")]
        ///
        /// This is [`Self::protocol`] with the protocol and its type already
        /// chosen. The result dereferences to `Field`.
        pub fn $name(&self) -> protocol::$view<'_> {
            protocol::$view::new(self)
        }

        #[doc = concat!("Returns this field mutably borrowed as its ", $label, " protocol.")]
        ///
        /// This is [`Self::protocol_mut`] with the protocol and its type
        /// already chosen. Every write routes through this field's own
        /// cache-aware mutation.
        pub fn $mutable(&mut self) -> protocol::$view_mut<'_> {
            protocol::$view_mut::new(self)
        }
    };
}

/// A shared Arrow field projection.
pub type FieldRef = Arc<ArrowField>;

/// An allocation-conscious Arrow field with field-owned metadata.
///
/// Scalar traits ignore the projection cache. Clones share metadata, nested
/// datatype state, and a populated Arrow projection until an effective change
/// invalidates the cache.
pub struct Field {
    pub(crate) name: SmolStr,
    pub(crate) dtype: DataType,
    pub(crate) nullable: bool,
    pub(crate) dictionary_id: i64,
    pub(crate) dictionary_is_ordered: bool,
    pub(crate) metadata: Metadata,
    pub(crate) arrow: OnceLock<FieldRef>,
}

impl Field {
    /// Constructs a field with empty metadata.
    pub fn new(name: impl Into<SmolStr>, dtype: DataType, nullable: bool) -> Self {
        Self {
            name: name.into(),
            dtype,
            nullable,
            dictionary_id: 0,
            dictionary_is_ordered: false,
            metadata: Metadata::new(),
            arrow: OnceLock::new(),
        }
    }

    /// Constructs a field around a metadata snapshot that is already valid.
    ///
    /// Record options rebuild their declared root from stored parts on every
    /// ask; moving the shared snapshot in keeps that build free of allocation
    /// and of a second validation of entries the snapshot already checked.
    pub(crate) fn new_with_metadata(
        name: impl Into<SmolStr>,
        dtype: DataType,
        nullable: bool,
        metadata: Metadata,
    ) -> Self {
        Self {
            name: name.into(),
            dtype,
            nullable,
            dictionary_id: 0,
            dictionary_is_ordered: false,
            metadata,
            arrow: OnceLock::new(),
        }
    }

    /// Constructs and validates a field with a complete metadata snapshot.
    pub fn from_parts<I, K, V>(
        name: impl Into<SmolStr>,
        dtype: DataType,
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
            dtype,
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
    pub const fn dtype(&self) -> &DataType {
        &self.dtype
    }

    /// Returns whether values may be null.
    pub const fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Returns the Arrow IPC dictionary identifier for dictionary fields.
    pub const fn dictionary_id(&self) -> Option<i64> {
        if matches!(self.dtype, DataType::Dictionary(_)) {
            Some(self.dictionary_id)
        } else {
            None
        }
    }

    /// Returns Arrow's dictionary ordering flag for dictionary fields.
    pub const fn dictionary_is_ordered(&self) -> Option<bool> {
        if matches!(self.dtype, DataType::Dictionary(_)) {
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

    /// Returns the shared human-readable comment stored in metadata.
    ///
    /// [`Metadata::comment`] carries what it is and who reads it.
    pub fn comment(&self) -> Option<&str> {
        self.metadata.comment()
    }

    /// Returns the shared human-readable display name stored in metadata.
    ///
    /// [`Metadata::display`] carries what it is and who reads it.
    pub fn display(&self) -> Option<&str> {
        self.metadata.display()
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
        (0..self.dtype.field_len())
            .filter_map(|index| self.dtype.get_field(index))
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
        for index in 0..self.dtype.field_len() {
            let Some(child) = self.dtype.get_field(index) else {
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
        let count = self.dtype.field_len();
        if count == 0 {
            return Ok(());
        }
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let Some(child) = self.dtype.get_field(index) else {
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
        self.set_dtype(self.dtype.with_fields(children)?)
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
    /// it borrows this field and reads out of the same metadata tree. A
    /// runtime scheme cannot select a compile-time type, so this is the
    /// dynamic half of the contract the `as_<protocol>` accessors type.
    ///
    /// ```
    /// use yggdryl::{DataType, Scheme};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut field = DataType::Int64.required_field("price");
    /// field.set_property(&Scheme::ICEBERG, "doc", "closing price")?;
    ///
    /// assert_eq!(field.protocol(&Scheme::ICEBERG).get("doc"), Some("closing price"));
    /// assert_eq!(field.as_iceberg().get("doc"), Some("closing price"));
    /// # Ok(())
    /// # }
    /// ```
    pub fn protocol(&self, scheme: &Scheme) -> ProtocolField<'_> {
        ProtocolField::new(self, scheme.clone())
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
    pub fn protocol_mut(&mut self, scheme: &Scheme) -> ProtocolFieldMut<'_> {
        ProtocolFieldMut::new(self, scheme.clone())
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
    pub fn set_dtype(&mut self, dtype: DataType) -> Result<()> {
        dtype.validate()?;
        if self.dtype != dtype {
            self.dtype = dtype;
            if !matches!(self.dtype, DataType::Dictionary(_)) {
                self.dictionary_id = 0;
                self.dictionary_is_ordered = false;
            }
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent copy with a validated datatype.
    pub fn try_with_dtype(mut self, dtype: DataType) -> Result<Self> {
        self.set_dtype(dtype)?;
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
        if !matches!(self.dtype, DataType::Dictionary(_)) {
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

    /// Sets a validated comment.
    ///
    /// # Errors
    ///
    /// Returns an error when the value fails the validation reserved text
    /// goes through.
    pub fn set_comment(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(COMMENT_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated comment.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::set_comment`] raises.
    pub fn try_with_comment(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_comment(value)?;
        Ok(self)
    }

    /// Removes and returns the comment.
    pub fn remove_comment(&mut self) -> Option<String> {
        self.remove_metadata(COMMENT_KEY)
    }

    /// Sets a validated display name.
    ///
    /// # Errors
    ///
    /// Returns an error when the value fails the validation reserved text
    /// goes through.
    pub fn set_display(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(DISPLAY_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated display name.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::set_display`] raises.
    pub fn try_with_display(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_display(value)?;
        Ok(self)
    }

    /// Removes and returns the display name.
    pub fn remove_display(&mut self) -> Option<String> {
        self.remove_metadata(DISPLAY_KEY)
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
        self.dtype.validate()?;
        if !matches!(self.dtype, DataType::Dictionary(_))
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
        preflight_schema_shape(self.dtype(), "Field")?;
        self.validate()
    }

    pub(crate) fn invalidate_arrow(&mut self) {
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
            dtype: self.dtype.clone(),
            nullable: self.nullable,
            dictionary_id: self.dictionary_id,
            dictionary_is_ordered: self.dictionary_is_ordered,
            metadata: self.metadata.clone(),
            arrow,
        }
    }
}

impl fmt::Debug for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Field")
            .field("name", &self.name)
            .field("dtype", &self.dtype)
            .field("nullable", &self.nullable)
            .field("dictionary_id", &self.dictionary_id)
            .field("dictionary_is_ordered", &self.dictionary_is_ordered)
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl fmt::Display for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `{:#}` is the readable, indented rendering; the plain form is the
        // compact constructor spelling, which round-trips through `from_str`
        // and is what `__repr__`, the errors, and the docs depend on.
        if formatter.alternate() {
            return fmt::Display::fmt(&self.pretty(), formatter);
        }
        formatter.write_str("field(")?;
        write_quoted(formatter, &self.name)?;
        write!(
            formatter,
            ",{},{}",
            self.dtype,
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
                && self.dtype == other.dtype
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
            &self.dtype,
            self.nullable,
            self.dictionary_id,
            self.dictionary_is_ordered,
            &self.metadata,
        )
            .cmp(&(
                &other.name,
                &other.dtype,
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
        self.dtype.hash(state);
        self.nullable.hash(state);
        self.dictionary_id.hash(state);
        self.dictionary_is_ordered.hash(state);
        self.metadata.hash(state);
    }
}

/// Subscripting a schema node reaches a nested **child**, never metadata.
///
/// Item access on a [`Field`] or a [`DataType`] means one thing and only one
/// thing: descend the schema. Metadata is reached through its own view -
/// [`Field::metadata_iter`] and [`Field::get_metadata`] - because a view whose keys
/// *are* keys is where item syntax legitimately means "a key". Before this,
/// `field["level"]` was a metadata lookup while `dtype["level"]` was a
/// child, so a caller walking one object graph got two unrelated things from
/// identical syntax.
///
/// Chained subscripts are the nesting story: `field["order"]["price"]` descends
/// two levels, because each subscript returns a node that subscripts again.
/// There is no dotted-string or tuple path form.
///
/// Panics when the name is not a child, as [`Index`] idiomatically does;
/// [`Field::get_field_by_path`] is the non-panicking form.
///
/// ```
/// use yggdryl::{DataType, Field};
///
/// # fn main() -> yggdryl::Result<()> {
/// let order = DataType::from_fields([
///     DataType::Int64.required_field("id"),
///     DataType::from_fields([DataType::Float64.required_field("price")])?
///         .required_field("line"),
/// ])?
/// .required_field("order");
///
/// assert_eq!(order["id"].dtype(), &DataType::Int64);
/// // Each subscript answers a node that subscripts again.
/// assert_eq!(order["line"]["price"].dtype(), &DataType::Float64);
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this node has no child with that name.
impl Index<&str> for Field {
    type Output = Self;

    fn index(&self, name: &str) -> &Self::Output {
        self.get_field_by_path(name)
            .unwrap_or_else(|| panic!("{:?} is not a child of the field {:?}", name, self.name()))
    }
}

/// Subscripting a schema node by position reaches that nested child.
///
/// The positional companion of [`Index<&str>`], matching how
/// [`Fields`](crate::types::Fields) already indexes.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let order = DataType::from_fields([
///     DataType::Int64.required_field("id"),
///     DataType::Utf8.required_field("venue"),
/// ])?
/// .required_field("order");
///
/// assert_eq!(order[0].name(), "id");
/// assert_eq!(order[1].name(), "venue");
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this node has no child at that position.
impl Index<usize> for Field {
    type Output = Self;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_field(index).unwrap_or_else(|| {
            panic!(
                "the field {:?} has {} children, so position {index} is out of range",
                self.name(),
                self.field_len()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    use super::Field;
    use crate::{AsciiEnum, DataType, Error};

    #[test]
    fn a_field_declares_the_enum_its_ascii_values_name() {
        let side = AsciiEnum::from_members("Side", [("BUY", "B"), ("SELL", "S")]).unwrap();
        let field = Field::new("side", DataType::FixedAscii(4), false)
            .try_with_ascii_enum(&side)
            .unwrap();

        // One reserved document, readable through the `field:` protocol view
        // and through the typed accessor that owns it.
        assert_eq!(
            field.get_metadata("field:enum"),
            Some(r#"{"members":{"BUY":"B","SELL":"S"},"name":"Side"}"#)
        );
        assert_eq!(
            field.as_field_properties().get("enum"),
            field.get_metadata("field:enum")
        );
        assert_eq!(field.ascii_enum().unwrap(), Some(side.clone()));
        assert_eq!(
            field.as_metadata().as_field_properties().get("enum"),
            field.get_metadata("field:enum")
        );

        // The members carry the packed codes of this field's own width.
        assert_eq!(
            side.into_members(field.dtype()).unwrap(),
            [("BUY".into(), 0x4200_0000), ("SELL".into(), 0x5300_0000)]
        );

        // Metadata canonicalizes the document, so one enum is one stored text
        // whichever spelling reached the field.
        let restated = Field::new("side", DataType::FixedAscii(4), false)
            .try_with_metadata(
                "field:enum",
                r#"{"name":"Side","members":{"SELL":"S","BUY":"B"}}"#,
            )
            .unwrap();
        assert_eq!(
            restated.get_metadata("field:enum"),
            field.get_metadata("field:enum")
        );
        assert_eq!(restated.stable_hash(), field.stable_hash());

        // A declaration the width could not store is refused whole.
        let wide = AsciiEnum::from_members("Venue", [("LONG", "EUREX")]).unwrap();
        let mut narrow = Field::new("venue", DataType::FixedAscii(4), false);
        let refused = narrow.set_ascii_enum(&wide).unwrap_err().to_string();
        assert!(refused.contains("at most 4 bytes"), "{refused}");
        assert_eq!(narrow.ascii_enum().unwrap(), None);
        assert!(
            Field::new("venue", DataType::Utf8, false)
                .set_ascii_enum(&wide)
                .is_err()
        );

        // A stored document that is not one is refused where it is written.
        let refused = Field::new("side", DataType::FixedAscii(4), false)
            .try_with_metadata("field:enum", "[]")
            .unwrap_err()
            .to_string();
        assert!(refused.contains("field:enum"), "{refused}");

        let mut removed = field.clone();
        assert_eq!(removed.remove_ascii_enum().unwrap(), Some(side));
        assert_eq!(removed.remove_ascii_enum().unwrap(), None);
        assert_eq!(removed, Field::new("side", DataType::FixedAscii(4), false));
    }

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
        assert_eq!(
            Field::from_json(&field.clone().into_json().unwrap()).unwrap(),
            field
        );
        let arrow = field.clone().into_arrow_ref().unwrap();
        assert_eq!(arrow, field.clone().into_arrow_ref().unwrap());
        assert_eq!(Field::from_arrow(arrow.as_ref()).unwrap(), field);
    }

    #[test]
    fn sql_hive_and_wrapped_forms_parse() {
        assert_eq!(
            Field::from_str("id bigint not null").unwrap().dtype(),
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
        let field = Field::from_arrow(&arrow).unwrap();
        assert_eq!(Field::from_str(&arrow.to_string()).unwrap(), field);
        assert_eq!(Field::from_str(&field.to_string()).unwrap(), field);
        assert_eq!(field.dictionary_id(), Some(42));
        assert_eq!(field.dictionary_is_ordered(), Some(true));

        let cached = Arc::new(field.clone().into_arrow().unwrap());
        let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_ref().unwrap()
        ));
        field.set_dictionary_options(42, true).unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_ref().unwrap()
        ));
        field.set_dictionary_options(7, false).unwrap();
        assert!(!Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_ref().unwrap()
        ));
        field.set_dictionary_options(42, true).unwrap();

        field.set_name("renamed");
        let rebuilt = field.into_arrow().unwrap();
        assert_eq!(rebuilt.dict_id(), Some(42));
        assert_eq!(rebuilt.dict_is_ordered(), Some(true));

        let shared = Arc::new(arrow);
        let imported = Field::from_arrow_ref(Arc::clone(&shared)).unwrap();
        assert!(Arc::ptr_eq(&shared, &imported.into_arrow_ref().unwrap()));
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
        let cached = Arc::new(field.clone().into_arrow().unwrap());
        let mut field = Field::from_arrow_ref(Arc::clone(&cached)).unwrap();
        field.insert_metadata("a", "first").unwrap();
        assert!(Arc::ptr_eq(
            &cached,
            &field.clone().into_arrow_ref().unwrap()
        ));
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
        second.clone().into_arrow_ref().unwrap();
        assert_eq!(before, second.stable_hash());
    }
}
