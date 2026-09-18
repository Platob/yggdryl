//! Generic and datatype-specific Arrow-compatible field values.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::sync::{Arc, OnceLock};

use arrow_schema::Field as ArrowField;
use smol_str::{SmolStr, format_smolstr};

use crate::metadata::{
    ALIAS_KEY, COMMENT_KEY, DESCRIPTION_KEY, DISPLAY_KEY, FIELD_INIT_KEY, FIELD_PARTITION_KEY,
    LOCATION_KEY, MetadataIter, PARQUET_FIELD_ID_KEY, PropertyIter, for_each_well_known_protocol,
    parse_field_id, parse_reserved_bool, property_key, write_json_string as write_quoted,
};
use crate::types::{
    BloombergType, BooleanType, BytesType, CfiType, CountryType, CurrencyType, CusipType,
    Date32Type, Date64Type, DateTime64Type, DecimalType, Duration32Type, Duration64Type, EnumType,
    Float16Type, Float32Type, Float64Type, GeographyType, GeometryType, Int8Type, Int16Type,
    Int32Type, Int64Type, IntervalType, IsinType, MappingType, MediaTypeType, MicType,
    MimeTypeType, NullType, RunEndType, SedolType, SequenceType, SideType, StateType, StringType,
    StructureType, Time32Type, Time64Type, TimeInForceType, TimezoneType, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type, UnionType, UrlType, UuidType, VariantType, VersionType,
};
use crate::types::{DataType, DataTypeValue, FieldValue, preflight_schema_shape};

use crate::{DataTypeId, Error, Metadata, Result, Scheme, Url};

use super::protocol::{self, ProtocolField, ProtocolFieldMut};
use crate::types::FieldSidecar;

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
pub struct FieldOf<D: DataTypeValue> {
    pub(crate) name: SmolStr,
    pub(crate) dtype: D,
    pub(crate) nullable: bool,
    /// The per-column facts only this datatype's fields carry.
    ///
    /// Zero-sized for every datatype but the dictionary-encoded one, so a
    /// field that has no dictionary identifier does not carry room for one.
    pub(crate) sidecar: D::Sidecar,
    pub(crate) metadata: Metadata,
    pub(crate) arrow: OnceLock<FieldRef>,
    /// The leaf's datatype widened to the root, derived on first ask.
    ///
    /// Not a second fact: `dtype` above is the only one, and this is that one
    /// read back in the root's spelling. It is here so a reader can borrow a
    /// `DataType` - the whole crate asks for one - without every ask
    /// rebuilding it. Replacing the datatype rebuilds the field, so this can
    /// never answer for a datatype the field no longer has.
    pub(crate) widened: OnceLock<Box<DataType>>,
}

impl<D: DataTypeValue + Default> FieldOf<D> {
    /// Builds a field of a datatype that carries no parameters.
    ///
    /// There is nothing to pass: the leaf's datatype is the whole of what the
    /// variant says, so naming it again at the call site would say it twice.
    pub fn unit(name: impl Into<SmolStr>, nullable: bool) -> Self {
        Self::new(name, D::default(), nullable)
    }
}

impl<D: DataTypeValue> FieldOf<D> {
    /// Constructs a field with empty metadata.
    pub fn new(name: impl Into<SmolStr>, dtype: D, nullable: bool) -> Self {
        Self {
            name: name.into(),
            dtype,
            nullable,
            sidecar: D::Sidecar::default(),
            metadata: Metadata::new(),
            arrow: OnceLock::new(),
            widened: OnceLock::new(),
        }
    }

    /// Constructs a field around a metadata snapshot that is already valid.
    ///
    /// Record options rebuild their declared root from stored parts on every
    /// ask; moving the shared snapshot in keeps that build free of allocation
    /// and of a second validation of entries the snapshot already checked.
    pub(crate) fn new_with_metadata(
        name: impl Into<SmolStr>,
        dtype: D,
        nullable: bool,
        metadata: Metadata,
    ) -> Self {
        Self {
            name: name.into(),
            dtype,
            nullable,
            sidecar: D::Sidecar::default(),
            metadata,
            arrow: OnceLock::new(),
            widened: OnceLock::new(),
        }
    }

    /// Constructs and validates a field with a complete metadata snapshot.
    pub fn from_parts<I, K, V>(
        name: impl Into<SmolStr>,
        dtype: D,
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
            sidecar: D::Sidecar::default(),
            metadata: Metadata::from_entries(metadata)?,
            arrow: OnceLock::new(),
            widened: OnceLock::new(),
        };
        field.validate()?;
        Ok(field)
    }

    /// Returns the physical field name without allocating.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns this field's datatype.
    ///
    /// Always [`DataType`], whichever leaf the field is, so a caller reading a
    /// datatype off a field never has to know which one it holds. Derived from
    /// the leaf's own datatype on the first ask and kept for the rest.
    pub fn dtype(&self) -> &DataType {
        self.widened
            .get_or_init(|| Box::new(self.dtype.clone().into_dtype()))
    }

    /// Returns this field's datatype in its own type, without allocating.
    ///
    /// The dedicated accessor: a `StructField` answers with a `StructType`,
    /// which is what a caller that already knows the leaf came for.
    /// [`Self::dtype`] is the generic reading of the same fact.
    pub const fn typed_dtype_ref(&self) -> &D {
        &self.dtype
    }

    /// Returns the parameter-free identifier of this field's shape.
    ///
    /// The identifier is the whole answer wherever a caller branches on the
    /// variant or names it for a binding, and it is the cheap one: reading it
    /// never touches the nested state a datatype holds. [`Scalar::id`] is the
    /// same verb on the value side.
    ///
    /// [`Scalar::id`]: crate::types::Scalar::id
    pub fn id(&self) -> DataTypeId {
        self.dtype.id()
    }

    /// Returns whether values may be null.
    pub const fn is_nullable(&self) -> bool {
        self.nullable
    }

    /// Returns the Arrow IPC dictionary identifier for dictionary fields.
    pub fn dictionary_id(&self) -> Option<i64> {
        self.sidecar.dictionary_id()
    }

    /// Returns Arrow's dictionary ordering flag for dictionary fields.
    pub fn dictionary_is_ordered(&self) -> Option<bool> {
        self.sidecar.dictionary_is_ordered()
    }

    /// Returns the number of metadata entries.
    pub fn metadata_len(&self) -> usize {
        self.metadata.len()
    }

    /// Returns the immutable shared metadata snapshot without allocating.
    /// Builds a leaf from a root datatype, refusing one of another family.
    pub fn try_new(name: impl Into<SmolStr>, dtype: DataType, nullable: bool) -> Result<Self> {
        let payload = D::from_dtype(&dtype).ok_or_else(|| Error::InvalidDataType {
            kind: D::FAMILY,
            reason: format_smolstr!("expected a {} datatype, got {dtype}", D::FAMILY),
        })?;
        Ok(Self::new(name, payload, nullable))
    }

    /// Checks a field and narrows it to this leaf.
    pub fn try_from_field(field: Field) -> Result<Self> {
        let dtype = field.dtype().clone();
        let payload = D::from_dtype(&dtype).ok_or_else(|| Error::InvalidDataType {
            kind: D::FAMILY,
            reason: format_smolstr!("expected a {} field, got {dtype}", D::FAMILY),
        })?;
        let mut leaf = Self::new(field.name(), payload, field.is_nullable());
        leaf.metadata = field.as_metadata().clone();
        Ok(leaf)
    }

    /// Widens this leaf to the field root, keeping every per-column fact.
    pub fn to_field(&self) -> Field {
        let mut field = Field::new_with_metadata(
            self.name.clone(),
            self.dtype().clone(),
            self.nullable,
            self.metadata.clone(),
        );
        field.set_dictionary_options_unchecked(
            self.dictionary_id().unwrap_or_default(),
            self.dictionary_is_ordered().unwrap_or_default(),
        );
        field
    }

    /// Returns the field's metadata mutably.
    ///
    /// Crate-internal: a write through it has to invalidate the Arrow
    /// projection, which every caller here does.
    pub(crate) fn metadata_mut(&mut self) -> &mut Metadata {
        &mut self.metadata
    }

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

    /// Returns the shared description of what this field holds.
    ///
    /// [`Metadata::description`] carries what it is and who reads it. A
    /// protocol that publishes a definition - FIX's own wording, an Iceberg
    /// doc, a column comment in a SQL dialect - publishes this one, because a
    /// field has one meaning however many catalogs quote it.
    pub fn description(&self) -> Option<&str> {
        self.metadata.description()
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
    pub fn set_dtype(&mut self, dtype: D) -> Result<()> {
        dtype.validate()?;
        if self.dtype != dtype {
            // The leaf cannot change family, so whether it carries dictionary
            // options cannot change either: the sidecar's type says so.
            self.dtype = dtype;
            self.invalidate_arrow();
        }
        Ok(())
    }

    /// Returns a persistent copy with a validated datatype.
    pub fn try_with_dtype(mut self, dtype: D) -> Result<Self> {
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
        if self.sidecar.set_dictionary_options(id, is_ordered)? {
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

    /// Sets a validated description of what this field holds.
    ///
    /// # Errors
    ///
    /// Returns an error when the value fails the validation reserved text
    /// goes through.
    pub fn set_description(&mut self, value: impl Into<String>) -> Result<()> {
        self.insert_metadata(DESCRIPTION_KEY, value)?;
        Ok(())
    }

    /// Returns a persistent field with a validated description.
    ///
    /// # Errors
    ///
    /// Returns the error [`Self::set_description`] raises.
    pub fn try_with_description(mut self, value: impl Into<String>) -> Result<Self> {
        self.set_description(value)?;
        Ok(self)
    }

    /// Removes and returns the description.
    pub fn remove_description(&mut self) -> Option<String> {
        self.remove_metadata(DESCRIPTION_KEY)
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
        if !matches!(self.dtype.id(), DataTypeId::Dictionary)
            && (self.dictionary_id().is_some_and(|id| id != 0)
                || self.dictionary_is_ordered().unwrap_or_default())
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

impl<D: DataTypeValue> Clone for FieldOf<D> {
    fn clone(&self) -> Self {
        let arrow = OnceLock::new();
        if let Some(cached) = self.arrow.get() {
            let _ = arrow.set(Arc::clone(cached));
        }
        // The derived datatype rides along: it is this field's own datatype
        // read back, so a clone that already paid for it should not pay again.
        let widened = OnceLock::new();
        if let Some(derived) = self.widened.get() {
            let _ = widened.set(derived.clone());
        }
        Self {
            name: self.name.clone(),
            dtype: self.dtype.clone(),
            nullable: self.nullable,
            sidecar: self.sidecar.clone(),
            metadata: self.metadata.clone(),
            arrow,
            widened,
        }
    }
}

impl<D: DataTypeValue> fmt::Debug for FieldOf<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Field")
            .field("name", &self.name)
            .field("dtype", self.dtype())
            .field("nullable", &self.nullable)
            .field("dictionary_id", &self.dictionary_id())
            .field("dictionary_is_ordered", &self.dictionary_is_ordered())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl<D: DataTypeValue> fmt::Display for FieldOf<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `{:#}` is the readable, indented rendering; the plain form is the
        // compact constructor spelling, which round-trips through `from_str`
        // and is what `__repr__`, the errors, and the docs depend on.
        if formatter.alternate() {
            return fmt::Display::fmt(&self.to_field().pretty_view(), formatter);
        }
        formatter.write_str("field(")?;
        write_quoted(formatter, &self.name)?;
        write!(
            formatter,
            ",{},{}",
            self.dtype(),
            if self.nullable {
                "nullable=true"
            } else {
                "nullable=false"
            }
        )?;
        if let Some(id) = self.dictionary_id().filter(|id| *id != 0) {
            write!(formatter, ",dictionary_id={id}")?;
        }
        if self.dictionary_is_ordered().unwrap_or_default() {
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

impl<D: DataTypeValue> PartialEq for FieldOf<D> {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.name == other.name
                && self.dtype == other.dtype
                && self.nullable == other.nullable
                && self.sidecar == other.sidecar
                && self.metadata == other.metadata
    }
}

impl<D: DataTypeValue> Eq for FieldOf<D> {}

impl<D: DataTypeValue> PartialOrd for FieldOf<D> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<D: DataTypeValue> Ord for FieldOf<D> {
    fn cmp(&self, other: &Self) -> Ordering {
        if std::ptr::eq(self, other) {
            return Ordering::Equal;
        }
        (
            &self.name,
            &self.dtype,
            self.nullable,
            &self.sidecar,
            &self.metadata,
        )
            .cmp(&(
                &other.name,
                &other.dtype,
                other.nullable,
                &other.sidecar,
                &other.metadata,
            ))
    }
}

impl<D: DataTypeValue> Hash for FieldOf<D> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.dtype.hash(state);
        self.nullable.hash(state);
        self.dictionary_id().hash(state);
        self.dictionary_is_ordered().hash(state);
        self.metadata.hash(state);
    }
}

// ------------------------------------------------------------------------
// The field leaves, and the enum that redirects to them.
// ------------------------------------------------------------------------

/// Emit every field leaf and the enum over them.
///
/// A leaf is [`FieldOf`] carrying that variant's own datatype, so a
/// `StructField` holds a `StructType` and nothing has to agree with anything
/// else. The enum is the redirector: it matches one variant and hands the
/// question down.
/// The payload of a variant that was just matched is always there.
///
/// One cold function rather than an `expect` in each of the fifty-three arms:
/// a panic site per arm is a stack frame per arm in a debug build, and these
/// constructors sit on the recursive walk of a nested schema.
#[cold]
#[inline(never)]
fn variant_payload_missing() -> ! {
    unreachable!("the datatype variant was just matched")
}

macro_rules! field_leaves {
    ($($variant:ident => $leaf:ident / $payload:ty,)+) => {
        $(
            #[doc = concat!(
                "A field whose datatype is [`DataType::",
                stringify!($variant),
                "`](crate::DataType::",
                stringify!($variant),
                ")."
            )]
            pub type $leaf = FieldOf<$payload>;
        )+

        /// One field, under the leaf its datatype names.
        ///
        /// The variant is not a second fact beside the datatype: each leaf
        /// stores its own datatype and nothing else, so there is no second
        /// copy to disagree with. [`Self::dtype`] reads it back as the
        /// generic [`DataType`] whichever leaf answers.
        #[derive(Clone)]
        #[non_exhaustive]
        pub enum Field {
            $(
                #[doc = concat!("A field whose datatype is [`DataType::", stringify!($variant), "`].")]
                $variant($leaf),
            )+
        }

        impl Field {
            /// Constructs a field, choosing the leaf its datatype names.
            pub fn new(name: impl Into<SmolStr>, dtype: DataType, nullable: bool) -> Self {
                let name = name.into();
                match &dtype {
                    $(
                        DataType::$variant { .. } => {
                            let payload = <$payload as DataTypeValue>::from_dtype(&dtype)
                                .unwrap_or_else(|| variant_payload_missing());
                            Self::$variant(FieldOf::new(name, payload, nullable))
                        }
                    )+
                }
            }

            pub(crate) fn new_with_metadata(
                name: impl Into<SmolStr>,
                dtype: DataType,
                nullable: bool,
                metadata: Metadata,
            ) -> Self {
                let name = name.into();
                match &dtype {
                    $(
                        DataType::$variant { .. } => {
                            let payload = <$payload as DataTypeValue>::from_dtype(&dtype)
                                .unwrap_or_else(|| variant_payload_missing());
                            Self::$variant(FieldOf::new_with_metadata(
                                name, payload, nullable, metadata,
                            ))
                        }
                    )+
                }
            }

            /// Constructs and validates a field with a metadata snapshot.
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
                let field = Self::new_with_metadata(
                    name,
                    dtype,
                    nullable,
                    Metadata::from_entries(metadata)?,
                );
                field.validate()?;
                Ok(field)
            }

            /// Returns this field's datatype.
            pub fn dtype(&self) -> &DataType {
                match self { $(Self::$variant(field) => field.dtype(),)+ }
            }

            /// Replaces the datatype, moving the field to the matching leaf.
            pub fn set_dtype(&mut self, dtype: DataType) -> Result<()> {
                dtype.validate()?;
                // The dictionary sidecar belongs to a dictionary field, so it
                // rides along only when the new datatype is still one - which
                // is the rule the in-place setter has always applied.
                let keeps_sidecar = matches!(dtype.id(), DataTypeId::Dictionary);
                let sidecar = (
                    self.dictionary_id().unwrap_or_default(),
                    self.dictionary_is_ordered().unwrap_or_default(),
                );
                let mut rebuilt = Self::new(self.name(), dtype, self.is_nullable());
                rebuilt.set_metadata_snapshot(self.as_metadata().clone());
                if keeps_sidecar {
                    rebuilt.set_dictionary_options_unchecked(sidecar.0, sidecar.1);
                }
                *self = rebuilt;
                Ok(())
            }

            /// Returns this field with a different datatype.
            pub fn try_with_dtype(mut self, dtype: DataType) -> Result<Self> {
                self.set_dtype(dtype)?;
                Ok(self)
            }

            /// Sets the dictionary sidecar without re-validating it.
            ///
            /// The Arrow importer has already read both halves off the schema
            /// it is reading, so they are not a caller's claim to check.
            pub(crate) fn set_dictionary_options_unchecked(
                &mut self,
                id: i64,
                is_ordered: bool,
            ) {
                // A leaf that carries no dictionary options has nothing to
                // store, and its sidecar says so by refusing: that refusal is
                // the whole check, so there is nothing to report here.
                match self {
                    $(Self::$variant(field) => {
                        let _ = field.sidecar.set_dictionary_options(id, is_ordered);
                    })+
                }
            }

            /// Seeds the Arrow projection cache of whichever leaf this is.
            pub(crate) fn seed_arrow_cache(&mut self, projection: FieldRef) {
                match self {
                    $(Self::$variant(field) => {
                        field.arrow = OnceLock::from(projection);
                    })+
                }
            }

            fn set_metadata_snapshot(&mut self, metadata: Metadata) {
                match self { $(Self::$variant(field) => field.metadata = metadata,)+ }
            }

            /// Returns this field with a different name.
            pub fn with_name(self, name: impl Into<SmolStr>) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_name(name)),)+ }
            }

            /// Changes the field name.
            pub fn set_name(&mut self, name: impl Into<SmolStr>) {
                match self { $(Self::$variant(field) => field.set_name(name),)+ }
            }

            #[doc = concat!("Delegates to [`FieldOf::", stringify!(name), "`].")]
            pub fn name(&self) -> &str {
                match self { $(Self::$variant(field) => field.name(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(id), "`].")]
            pub fn id(&self) -> DataTypeId {
                match self { $(Self::$variant(field) => field.id(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(is_nullable), "`].")]
            pub fn is_nullable(&self) -> bool {
                match self { $(Self::$variant(field) => field.is_nullable(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(dictionary_id), "`].")]
            pub fn dictionary_id(&self) -> Option<i64> {
                match self { $(Self::$variant(field) => field.dictionary_id(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(dictionary_is_ordered), "`].")]
            pub fn dictionary_is_ordered(&self) -> Option<bool> {
                match self { $(Self::$variant(field) => field.dictionary_is_ordered(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(metadata_len), "`].")]
            pub fn metadata_len(&self) -> usize {
                match self { $(Self::$variant(field) => field.metadata_len(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(as_metadata), "`].")]
            pub fn as_metadata(&self) -> &Metadata {
                match self { $(Self::$variant(field) => field.as_metadata(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(is_metadata_empty), "`].")]
            pub fn is_metadata_empty(&self) -> bool {
                match self { $(Self::$variant(field) => field.is_metadata_empty(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(metadata_iter), "`].")]
            pub fn metadata_iter(&self) -> MetadataIter<'_> {
                match self { $(Self::$variant(field) => field.metadata_iter(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(get_metadata), "`].")]
            pub fn get_metadata(&self, key: &str) -> Option<&str> {
                match self { $(Self::$variant(field) => field.get_metadata(key),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(has_metadata), "`].")]
            pub fn has_metadata(&self, key: &str) -> bool {
                match self { $(Self::$variant(field) => field.has_metadata(key),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(alias), "`].")]
            pub fn alias(&self) -> Option<&str> {
                match self { $(Self::$variant(field) => field.alias(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(comment), "`].")]
            pub fn comment(&self) -> Option<&str> {
                match self { $(Self::$variant(field) => field.comment(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(display), "`].")]
            pub fn display(&self) -> Option<&str> {
                match self { $(Self::$variant(field) => field.display(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(description), "`].")]
            pub fn description(&self) -> Option<&str> {
                match self { $(Self::$variant(field) => field.description(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(parquet_field_id), "`].")]
            pub fn parquet_field_id(&self) -> Result<Option<i32>> {
                match self { $(Self::$variant(field) => field.parquet_field_id(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(is_init), "`].")]
            pub fn is_init(&self) -> Result<bool> {
                match self { $(Self::$variant(field) => field.is_init(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(location), "`].")]
            pub fn location(&self) -> Result<Option<Url>> {
                match self { $(Self::$variant(field) => field.location(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(get_property), "`].")]
            pub fn get_property(&self, scheme: &Scheme, name: &str) -> Option<&str> {
                match self { $(Self::$variant(field) => field.get_property(scheme, name),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(has_property), "`].")]
            pub fn has_property(&self, scheme: &Scheme, name: &str) -> bool {
                match self { $(Self::$variant(field) => field.has_property(scheme, name),)+ }
            }

            /// Delegates to [`FieldOf::insert_metadata`].
            pub fn insert_metadata(
                &mut self,
                key: impl Into<String>,
                value: impl Into<String>,
            ) -> Result<Option<String>> {
                match self { $(Self::$variant(field) => field.insert_metadata(key, value),)+ }
            }

            /// Delegates to [`FieldOf::set_metadata`].
            pub fn set_metadata<I, K, V>(&mut self, values: I) -> Result<()>
            where
                I: IntoIterator<Item = (K, V)>,
                K: Into<String>,
                V: Into<String>,
            {
                match self { $(Self::$variant(field) => field.set_metadata(values),)+ }
            }

            /// Delegates to [`FieldOf::update_metadata`].
            pub fn update_metadata<I, K, V>(&mut self, values: I) -> Result<()>
            where
                I: IntoIterator<Item = (K, V)>,
                K: Into<String>,
                V: Into<String>,
            {
                match self { $(Self::$variant(field) => field.update_metadata(values),)+ }
            }

            /// Delegates to [`FieldOf::try_with_metadata_entries`], keeping the leaf.
            pub fn try_with_metadata_entries<I, K, V>(self, values: I) -> Result<Self>
            where
                I: IntoIterator<Item = (K, V)>,
                K: Into<String>,
                V: Into<String>,
            {
                match self {
                    $(Self::$variant(field) => {
                        field.try_with_metadata_entries(values).map(Self::$variant)
                    })+
                }
            }

            /// Delegates to [`FieldOf::set_description`].
            pub fn set_description(&mut self, value: impl Into<String>) -> Result<()> {
                match self { $(Self::$variant(field) => field.set_description(value),)+ }
            }

            /// Delegates to [`FieldOf::set_display`].
            pub fn set_display(&mut self, value: impl Into<String>) -> Result<()> {
                match self { $(Self::$variant(field) => field.set_display(value),)+ }
            }

            /// Delegates to [`FieldOf::set_property`].
            pub fn set_property(
                &mut self,
                scheme: &Scheme,
                name: &str,
                value: impl Into<String>,
            ) -> Result<Option<String>> {
                match self { $(Self::$variant(field) => field.set_property(scheme, name, value),)+ }
            }

            /// Delegates to [`FieldOf::property_iter`].
            pub fn property_iter<'field, 'scheme>(
                &'field self,
                scheme: &'scheme Scheme,
            ) -> PropertyIter<'field, 'scheme> {
                match self { $(Self::$variant(field) => field.property_iter(scheme),)+ }
            }

            /// Delegates to [`FieldOf::next_property_entry`].
            pub fn next_property_entry<'field>(
                &'field self,
                scheme: &Scheme,
                after_name: Option<&str>,
            ) -> Option<(&'field str, &'field str)> {
                match self {
                    $(Self::$variant(field) => field.next_property_entry(scheme, after_name),)+
                }
            }

            /// Borrows the Arrow projection, building it on the first ask.
            pub fn as_arrow_field_ref(&self) -> Result<&FieldRef> {
                match self { $(Self::$variant(field) => field.as_arrow_field_ref(),)+ }
            }

            /// Consumes this field and returns an owned Arrow field.
            pub fn into_arrow_field(self) -> Result<arrow_schema::Field> {
                match self { $(Self::$variant(field) => field.into_arrow_field(),)+ }
            }

            /// Consumes this field and returns a shared Arrow field.
            pub fn into_arrow_field_ref(self) -> Result<FieldRef> {
                match self { $(Self::$variant(field) => field.into_arrow_field_ref(),)+ }
            }

            /// Consumes this field and returns its C schema.
            pub fn into_arrow_field_ffi(self) -> Result<arrow_schema::ffi::FFI_ArrowSchema> {
                match self { $(Self::$variant(field) => field.into_arrow_field_ffi(),)+ }
            }

            pub(crate) fn metadata_mut(&mut self) -> &mut Metadata {
                match self { $(Self::$variant(field) => field.metadata_mut(),)+ }
            }

            fn compare_leaf(&self, other: &Self) -> Ordering {
                match (self, other) {
                    $((Self::$variant(left), Self::$variant(right)) => left.cmp(right),)+
                    // Different leaves mean different datatypes, and a field
                    // has always ordered by name first and datatype next.
                    _ => self
                        .name()
                        .cmp(other.name())
                        .then_with(|| self.dtype().cmp(other.dtype())),
                }
            }

            fn hash_leaf<H: Hasher>(&self, state: &mut H) {
                match self { $(Self::$variant(field) => field.hash(state),)+ }
            }

            fn display_leaf(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Self::$variant(field) => fmt::Display::fmt(field, formatter),)+ }
            }

            fn debug_leaf(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self { $(Self::$variant(field) => fmt::Debug::fmt(field, formatter),)+ }
            }

            pub(crate) fn invalidate_arrow(&mut self) {
                match self { $(Self::$variant(field) => field.invalidate_arrow(),)+ }
            }

            pub(crate) fn arrow_import_is_projection_equivalent(&self) -> bool {
                match self {
                    $(Self::$variant(field) => field.arrow_import_is_projection_equivalent(),)+
                }
            }


            /// Delegates to [`FieldOf::set_comment`].
            pub fn set_comment(&mut self, value: impl Into<String>) -> Result<()> {
                match self { $(Self::$variant(field) => field.set_comment(value),)+ }
            }

            /// Delegates to [`FieldOf::set_alias`].
            pub fn set_alias(&mut self, value: impl Into<String>) -> Result<()> {
                match self { $(Self::$variant(field) => field.set_alias(value),)+ }
            }

            /// Delegates to [`FieldOf::try_with_metadata`], keeping the leaf.
            pub fn try_with_metadata(
                self,
                key: impl Into<String>,
                value: impl Into<String>,
            ) -> Result<Self> {
                match self {
                    $(Self::$variant(field) => {
                        field.try_with_metadata(key, value).map(Self::$variant)
                    })+
                }
            }

            /// Delegates to [`FieldOf::try_with_property`], keeping the leaf.
            pub fn try_with_property(
                self,
                scheme: &Scheme,
                name: &str,
                value: impl Into<String>,
            ) -> Result<Self> {
                match self {
                    $(Self::$variant(field) => {
                        field.try_with_property(scheme, name, value).map(Self::$variant)
                    })+
                }
            }

            /// Delegates to [`FieldOf::try_with_alias`], keeping the leaf.
            pub fn try_with_alias(self, value: impl Into<String>) -> Result<Self> {
                match self {
                    $(Self::$variant(field) => field.try_with_alias(value).map(Self::$variant),)+
                }
            }

            /// Delegates to [`FieldOf::try_with_comment`], keeping the leaf.
            pub fn try_with_comment(self, value: impl Into<String>) -> Result<Self> {
                match self {
                    $(Self::$variant(field) => field.try_with_comment(value).map(Self::$variant),)+
                }
            }

            /// Delegates to [`FieldOf::try_with_display`], keeping the leaf.
            pub fn try_with_display(self, value: impl Into<String>) -> Result<Self> {
                match self {
                    $(Self::$variant(field) => field.try_with_display(value).map(Self::$variant),)+
                }
            }

            /// Delegates to [`FieldOf::try_with_description`], keeping the leaf.
            pub fn try_with_description(self, value: impl Into<String>) -> Result<Self> {
                match self {
                    $(Self::$variant(field) => {
                        field.try_with_description(value).map(Self::$variant)
                    })+
                }
            }

            /// Returns a view of one protocol's properties.
            pub fn protocol(&self, scheme: &Scheme) -> ProtocolField<'_> {
                ProtocolField::new(self, scheme.clone())
            }

            for_each_well_known_protocol!(field_protocol_accessors);
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(validate), "`].")]
            pub fn validate(&self) -> Result<()> {
                match self { $(Self::$variant(field) => field.validate(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(validate_bounded), "`].")]
            pub fn validate_bounded(&self) -> Result<()> {
                match self { $(Self::$variant(field) => field.validate_bounded(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(next_metadata_entry), "`].")]
            pub fn next_metadata_entry(&self, after_key: Option<&str>) -> Option<(&str, &str)> {
                match self { $(Self::$variant(field) => field.next_metadata_entry(after_key),)+ }
            }
            /// Returns a mutable view of one protocol's properties.
            pub fn protocol_mut(&mut self, scheme: &Scheme) -> ProtocolFieldMut<'_> {
                ProtocolFieldMut::new(self, scheme.clone())
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(set_nullable), "`].")]
            pub fn set_nullable(&mut self, nullable: bool) -> () {
                match self { $(Self::$variant(field) => field.set_nullable(nullable),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(set_dictionary_options), "`].")]
            pub fn set_dictionary_options(&mut self, id: i64, is_ordered: bool) -> Result<()> {
                match self { $(Self::$variant(field) => field.set_dictionary_options(id, is_ordered),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_metadata), "`].")]
            pub fn remove_metadata(&mut self, key: &str) -> Option<String> {
                match self { $(Self::$variant(field) => field.remove_metadata(key),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(clear_metadata), "`].")]
            pub fn clear_metadata(&mut self) -> () {
                match self { $(Self::$variant(field) => field.clear_metadata(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_alias), "`].")]
            pub fn remove_alias(&mut self) -> Option<String> {
                match self { $(Self::$variant(field) => field.remove_alias(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_comment), "`].")]
            pub fn remove_comment(&mut self) -> Option<String> {
                match self { $(Self::$variant(field) => field.remove_comment(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_display), "`].")]
            pub fn remove_display(&mut self) -> Option<String> {
                match self { $(Self::$variant(field) => field.remove_display(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_description), "`].")]
            pub fn remove_description(&mut self) -> Option<String> {
                match self { $(Self::$variant(field) => field.remove_description(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(set_parquet_field_id), "`].")]
            pub fn set_parquet_field_id(&mut self, id: i32) -> () {
                match self { $(Self::$variant(field) => field.set_parquet_field_id(id),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(set_init), "`].")]
            pub fn set_init(&mut self, init: bool) -> () {
                match self { $(Self::$variant(field) => field.set_init(init),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(set_partition), "`].")]
            pub fn set_partition(&mut self, partition: bool) -> () {
                match self { $(Self::$variant(field) => field.set_partition(partition),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_parquet_field_id), "`].")]
            pub fn remove_parquet_field_id(&mut self) -> Result<Option<i32>> {
                match self { $(Self::$variant(field) => field.remove_parquet_field_id(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(set_location), "`].")]
            pub fn set_location(&mut self, location: Url) -> () {
                match self { $(Self::$variant(field) => field.set_location(location),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_location), "`].")]
            pub fn remove_location(&mut self) -> Result<Option<Url>> {
                match self { $(Self::$variant(field) => field.remove_location(),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(remove_property), "`].")]
            pub fn remove_property(&mut self, scheme: &Scheme, name: &str) -> Option<String> {
                match self { $(Self::$variant(field) => field.remove_property(scheme, name),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(clear_properties), "`].")]
            pub fn clear_properties(&mut self, scheme: &Scheme) -> () {
                match self { $(Self::$variant(field) => field.clear_properties(scheme),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_nullable), "`], keeping the leaf.")]
            pub fn with_nullable(self, nullable: bool) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_nullable(nullable)),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(try_with_dictionary_options), "`], keeping the leaf.")]
            pub fn try_with_dictionary_options(self, id: i64, is_ordered: bool) -> Result<Self> {
                match self { $(Self::$variant(field) => field.try_with_dictionary_options(id, is_ordered).map(Self::$variant),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_metadata_removed), "`], keeping the leaf.")]
            pub fn with_metadata_removed(self, key: &str) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_metadata_removed(key)),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_parquet_field_id), "`], keeping the leaf.")]
            pub fn with_parquet_field_id(self, id: i32) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_parquet_field_id(id)),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_init), "`], keeping the leaf.")]
            pub fn with_init(self, init: bool) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_init(init)),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_partition), "`], keeping the leaf.")]
            pub fn with_partition(self, partition: bool) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_partition(partition)),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_location), "`], keeping the leaf.")]
            pub fn with_location(self, location: Url) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_location(location)),)+ }
            }
            #[doc = concat!("Delegates to [`FieldOf::", stringify!(with_properties_cleared), "`], keeping the leaf.")]
            pub fn with_properties_cleared(self, scheme: &Scheme) -> Self {
                match self { $(Self::$variant(field) => Self::$variant(field.with_properties_cleared(scheme)),)+ }
            }
        }

        impl FieldValue<DataType> for Field {
            fn name(&self) -> &str {
                Self::name(self)
            }

            fn dtype(&self) -> DataType {
                Self::dtype(self).clone()
            }

            fn typed_dtype(&self) -> DataType {
                Self::dtype(self).clone()
            }

            fn is_nullable(&self) -> bool {
                Self::is_nullable(self)
            }

            fn metadata(&self) -> &Metadata {
                self.as_metadata()
            }

            fn validate(&self) -> Result<()> {
                Self::validate(self)
            }

            fn into_field(self) -> Field {
                self
            }

            fn from_field(field: &Field) -> Option<&Self> {
                Some(field)
            }
        }

        $(
            impl From<$leaf> for Field {
                fn from(value: $leaf) -> Self {
                    Self::$variant(value)
                }
            }

            impl FieldValue<$payload> for $leaf {
                fn name(&self) -> &str {
                    FieldOf::name(self)
                }

                fn dtype(&self) -> DataType {
                    FieldOf::dtype(self).clone()
                }

                fn typed_dtype(&self) -> $payload {
                    self.typed_dtype_ref().clone()
                }

                fn is_nullable(&self) -> bool {
                    FieldOf::is_nullable(self)
                }

                fn metadata(&self) -> &Metadata {
                    self.as_metadata()
                }

                fn validate(&self) -> Result<()> {
                    FieldOf::validate(self)
                }

                fn into_field(self) -> Field {
                    Field::$variant(self)
                }

                fn from_field(field: &Field) -> Option<&Self> {
                    match field {
                        Field::$variant(leaf) => Some(leaf),
                        _ => None,
                    }
                }
            }
        )+
    };
}

field_leaves! {
    Null => NullField / NullType,
    Boolean => BooleanField / BooleanType,
    Int8 => Int8Field / Int8Type,
    Int16 => Int16Field / Int16Type,
    Int32 => Int32Field / Int32Type,
    Int64 => Int64Field / Int64Type,
    UInt8 => UInt8Field / UInt8Type,
    UInt16 => UInt16Field / UInt16Type,
    UInt32 => UInt32Field / UInt32Type,
    UInt64 => UInt64Field / UInt64Type,
    Float16 => Float16Field / Float16Type,
    Float32 => Float32Field / Float32Type,
    Float64 => Float64Field / Float64Type,
    DateTime64 => DateTime64Field / DateTime64Type,
    Date32 => Date32Field / Date32Type,
    Date64 => Date64Field / Date64Type,
    Time32 => Time32Field / Time32Type,
    Time64 => Time64Field / Time64Type,
    Duration32 => Duration32Field / Duration32Type,
    Duration64 => Duration64Field / Duration64Type,
    Interval => IntervalField / IntervalType,
    Bytes => BytesField / BytesType,
    String => StringField / StringType,
    Country => CountryField / CountryType,
    Currency => CurrencyField / CurrencyType,
    Mic => MicField / MicType,
    Cfi => CfiField / CfiType,
    Isin => IsinField / IsinType,
    Side => SideField / SideType,
    State => StateField / StateType,
    TimeInForce => TimeInForceField / TimeInForceType,
    Uuid => UuidField / UuidType,
    Version => VersionField / VersionType,
    Url => UrlField / UrlType,
    Sequence => SequenceField / SequenceType,
    Structure => StructureField / StructureType,
    Union => UnionField / UnionType,
    Enum => EnumField / EnumType,
    Decimal => DecimalField / DecimalType,
    Mapping => MappingField / MappingType,
    RunEndEncoded => RunEndEncodedField / RunEndType,
    Variant => VariantField / VariantType,
    Geometry => GeometryField / GeometryType,
    Geography => GeographyField / GeographyType,
    Timezone => TimezoneField / TimezoneType,
    MimeType => MimeTypeField / MimeTypeType,
    MediaType => MediaTypeField / MediaTypeType,
    Cusip => CusipField / CusipType,
    Sedol => SedolField / SedolType,
    Bloomberg => BloombergField / BloombergType,
}

// A field compares and hashes as the leaf it holds. Two fields of different
// leaves have different datatypes, so they are never equal.
impl PartialEq for Field {
    fn eq(&self, other: &Self) -> bool {
        self.compare_leaf(other) == Ordering::Equal
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
        self.compare_leaf(other)
    }
}

impl Hash for Field {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash_leaf(state);
    }
}

impl fmt::Debug for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.debug_leaf(formatter)
    }
}

impl fmt::Display for Field {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.display_leaf(formatter)
    }
}

impl Field {
    /// The recursive worker behind [`Self::merge_with`], shared with the
    /// datatype merge so nested children never take a different path.
    pub(crate) fn merge(
        &self,
        other: &Self,
        how: crate::types::Widening,
        recode: crate::types::Recode,
    ) -> Result<Self> {
        let dtype = self.dtype().merge(other.dtype(), how, recode)?;
        let mut merged = Self::new(
            self.name(),
            dtype,
            self.is_nullable() || other.is_nullable(),
        );
        // One rule, on `Metadata` itself: the union of both, this field
        // winning any key they disagree on.
        merged.set_metadata(self.as_metadata().merge_with(other.as_metadata())?.iter())?;
        if self.dictionary_id().is_some() && other.dictionary_id().is_some() {
            merged.set_dictionary_options(
                self.dictionary_id().unwrap_or_default(),
                self.dictionary_is_ordered().unwrap_or_default()
                    && other.dictionary_is_ordered().unwrap_or_default(),
            )?;
        }
        Ok(merged)
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
    ///     DataType::list(DataType::utf8().nullable_field("item")).nullable_field("tags"),
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
        (0..self.dtype().field_len())
            .filter_map(|index| self.dtype().get_field(index))
            .find_map(|child| child.field_by_parquet_field_id(id))
    }
    /// Number one level of children, then each of their trees.
    fn assign_child_ids(&mut self, next: &mut i32) -> Result<()> {
        let count = self.dtype().field_len();
        if count == 0 {
            return Ok(());
        }
        let mut children = Vec::with_capacity(count);
        for index in 0..count {
            let Some(child) = self.dtype().get_field(index) else {
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
        let widened = self.dtype().clone().with_fields(children)?;
        self.set_dtype(widened)
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
        for index in 0..self.dtype().field_len() {
            let Some(child) = self.dtype().get_field(index) else {
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
///     DataType::utf8().required_field("venue"),
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

// ------------------------------------------------------------------------
// Arrow projection and import: the field node, and the plan a root applies.
// ------------------------------------------------------------------------

/// Arrow field import, cached projection, and conversion traits.
mod arrow {
    use crate::types::FieldSidecar;
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_schema::Schema;
    use arrow_schema::{
        DataType as ArrowDataType, Field as ArrowField,
        extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY},
        ffi::{FFI_ArrowSchema, Flags},
    };
    use smol_str::{SmolStr, format_smolstr};

    use crate::types::{
        BYTES_EXTENSION_NAME, BytesType, GEOARROW_WKB_EXTENSION_NAME, MEDIATYPE_EXTENSION_NAME,
        MIMETYPE_EXTENSION_NAME, STRING_EXTENSION_NAME, StringType, TIMEZONE_EXTENSION_NAME,
        URL_EXTENSION_NAME, UUID_EXTENSION_NAME, UUID_VERSION_EXTENSION_NAME, UuidType,
        VARIANT_EXTENSION_NAME, VERSION_EXTENSION_NAME, code_for_extension, is_variant_storage,
    };
    use crate::types::{Field, FieldRef};
    use crate::{DataType, Error, GeospatialParameters, Metadata, Result};

    impl<D: crate::types::DataTypeValue> crate::types::FieldOf<D> {
        /// Projects this field to an owned Arrow C Data Interface schema.
        ///
        /// Name, metadata, nullability, dictionary ordering, and nested datatype
        /// flags are preserved in one canonical core conversion.
        pub fn into_arrow_field_ffi(self) -> Result<FFI_ArrowSchema> {
            if let Some(field) = self.arrow.get() {
                return arrow_field_to_ffi(field).map_err(Error::from);
            }
            let mut schema = self.dtype().clone().into_arrow_datatype_ffi()?;
            let mut flags = schema.flags().unwrap_or_else(Flags::empty);
            if self.nullable {
                flags |= Flags::NULLABLE;
            }
            if self.dictionary_is_ordered().unwrap_or_default() {
                flags |= Flags::DICTIONARY_ORDERED;
            }
            schema = schema.with_name(self.name())?.with_flags(flags)?;
            // The datatype projection already carries the extension entries for an
            // extension-typed field, but the C interface stores metadata as
            // one buffer, so the replacement map must carry them again.
            schema
                .with_metadata(&projected_arrow_metadata(
                    self.dtype(),
                    self.metadata.clone().into_arrow_metadata(),
                )?)
                .map_err(Error::from)
        }

        /// Consumes this field and returns an owned Arrow field.
        pub fn into_arrow_field(self) -> Result<ArrowField> {
            let Self {
                name,
                dtype,
                nullable,
                metadata,
                arrow,
                widened: _,
                sidecar,
            } = self;
            // The leaf holds its own datatype; the Arrow projection speaks the
            // root's, so widen it once here.
            let dtype = <D as crate::types::DataTypeValue>::into_dtype(dtype);
            if let Some(field) = arrow.into_inner() {
                return Ok(Arc::try_unwrap(field).unwrap_or_else(|field| field.as_ref().clone()));
            }
            let projected = projected_arrow_metadata(&dtype, metadata.into_arrow_metadata())?;
            Ok(arrow_field_from_parts(
                name.as_str(),
                dtype.into_arrow_datatype()?,
                nullable,
                sidecar.dictionary_id().unwrap_or_default(),
                sidecar.dictionary_is_ordered().unwrap_or_default(),
                projected,
            ))
        }

        /// Borrows this field's Arrow projection, building it once.
        ///
        /// [`Self::into_arrow_field_ref`] consumes the field, so a caller that still
        /// needs it has to clone first and pays for the projection every time.
        /// This borrows, and fills the same cache an Arrow import seeds and a
        /// clone shares - so a field exported more than once is projected once.
        /// Any effective change clears the cache, exactly as it does today.
        ///
        /// # Errors
        ///
        /// Returns an error when the datatype or its metadata has no valid Arrow
        /// projection.
        pub fn as_arrow_field_ref(&self) -> Result<&FieldRef> {
            if let Some(field) = self.arrow.get() {
                return Ok(field);
            }
            let projected = projected_arrow_metadata(
                self.dtype(),
                self.metadata.clone().into_arrow_metadata(),
            )?;
            let built = Arc::new(arrow_field_from_parts(
                self.name.as_str(),
                self.dtype().clone().into_arrow_datatype()?,
                self.nullable,
                self.dictionary_id().unwrap_or_default(),
                self.dictionary_is_ordered().unwrap_or_default(),
                projected,
            ));
            let _ = self.arrow.set(built);
            Ok(self
                .arrow
                .get()
                .expect("the projection was just placed in the cache"))
        }

        /// Consumes this field and returns a shared Arrow projection.
        pub fn into_arrow_field_ref(self) -> Result<FieldRef> {
            let Self {
                name,
                dtype,
                nullable,
                metadata,
                arrow,
                widened: _,
                sidecar,
            } = self;
            // The leaf holds its own datatype; the Arrow projection speaks the
            // root's, so widen it once here.
            let dtype = <D as crate::types::DataTypeValue>::into_dtype(dtype);
            if let Some(field) = arrow.into_inner() {
                return Ok(field);
            }
            let projected = projected_arrow_metadata(&dtype, metadata.into_arrow_metadata())?;
            Ok(Arc::new(arrow_field_from_parts(
                name.as_str(),
                dtype.into_arrow_datatype()?,
                nullable,
                sidecar.dictionary_id().unwrap_or_default(),
                sidecar.dictionary_is_ordered().unwrap_or_default(),
                projected,
            )))
        }
    }

    impl Field {
        /// Projects this non-null Struct root Field as an Arrow schema.
        ///
        /// This is the schema an Arrow batch, an IPC stream, or a Parquet file
        /// carries: one Arrow field per child, with the root's metadata as the
        /// schema metadata, so field identifiers reach the file.
        ///
        /// # Errors
        ///
        /// Returns an error unless this is a bounded, non-nullable Struct root.
        /// Projects this non-null Struct root for Arrow runtime exchange.
        ///
        /// Unlike [`Field::into_arrow_schema`], this owned schema carries the
        /// transport-only dictionary-ID sidecar needed when crossing the Arrow C
        /// Data Interface. The sidecar never becomes logical Field metadata.
        ///
        /// # Errors
        ///
        /// Returns an error unless this is a bounded, non-null Struct root or when
        /// caller metadata uses the transport-reserved sidecar key.
        pub fn into_arrow_exchange_schema(self) -> crate::arrow::Result<Schema> {
            crate::arrow::arrow_exchange_schema_from_field(&self)
        }
        /// Consumes this Field and projects it as an Arrow schema.
        ///
        /// # Errors
        ///
        /// Returns an error unless this is a bounded, non-nullable Struct root.
        pub fn into_arrow_schema(self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
            crate::arrow::arrow_schema_from_field(&self)
        }
        /// Apply this schema's metadata-declared columns to one Arrow batch.
        ///
        /// A [`Field`] states more about a batch than its shape: a
        /// [`transform:`](crate::TransformField::apply_arrow_batch) declaration
        /// says a column is *derived* from others, and a
        /// [`digest:`](crate::DigestField::apply_arrow_batch) role says a column
        /// *holds* the row's hash. Each protocol owns how it answers, including
        /// how far down it walks, and this is the one entry point that asks them
        /// all in the order their answers depend on.
        ///
        /// That order is the argument order reversed, because a later step reads
        /// what an earlier one wrote:
        ///
        /// - `cast` reconciles the batch to this root first - the columns it
        ///   declares, in its order and its types, missing ones materialized as
        ///   their canonical defaults.
        /// - `partition` fills the derived columns, so a value computed from
        ///   another column exists before anything hashes it.
        /// - `digest` fills the holders last, over the rows as they finally stand.
        ///
        /// Each protocol leaves a column holding anything but its canonical
        /// [default](Self::default_value) alone, so applying twice changes nothing
        /// the first pass already wrote. A digest fill reconciles to this root for
        /// itself whatever `cast` says, because a holder is addressed by position.
        ///
        /// `options` carries the cast policy the first step runs under, and
        /// under [`Nullability::Strict`](crate::Nullability::Strict) it also holds after the protocols have
        /// run: a field an enabled protocol materializes may arrive absent or
        /// holding its canonical default, because closing that hole is the
        /// protocol's job, but the applied batch is checked again once every
        /// protocol is done, so what a protocol left null is refused by path.
        ///
        /// ```
        /// use std::sync::Arc;
        ///
        /// use arrow_array::{ArrayRef, Date32Array, RecordBatch};
        /// use yggdryl::expression::Function;
        /// use yggdryl::{ArrowCastOptions, DataType};
        ///
        /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
        /// let mut year = DataType::Int32.nullable_field("year");
        /// year.as_partition_mut().set_sources(["event"])?;
        /// year.as_partition_mut().set_transform(Function::Year)?;
        /// let mut stored = DataType::UInt64.nullable_field("row_digest");
        /// stored.as_digest_mut().set_holder()?;
        /// let root = DataType::from_fields([
        ///     DataType::Date32.required_field("event"),
        ///     year,
        ///     stored,
        /// ])?
        /// .required_field("row");
        ///
        /// let batch = RecordBatch::try_from_iter([(
        ///     "event",
        ///     Arc::new(Date32Array::from(vec![19_723])) as ArrayRef,
        /// )])?;
        ///
        /// let applied = root.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;
        ///
        /// assert_eq!(applied.num_columns(), 3);
        /// // The digest saw the derived column, because the partition step ran first.
        /// assert_eq!(applied.column(2).null_count(), 0);
        ///
        /// // Applying again changes nothing: every column now holds a written value.
        /// assert_eq!(
        ///     root.apply_arrow_batch(&applied, true, true, true, ArrowCastOptions::new())?,
        ///     applied,
        /// );
        /// # Ok(())
        /// # }
        /// ```
        ///
        /// # Errors
        ///
        /// Returns an error when this is not a Struct root, when the batch cannot
        /// be cast to it, when either protocol refuses a declaration it carries, or
        /// when the applied batch leaves a declared non-null field null under
        /// [`Nullability::Strict`](crate::Nullability::Strict).
        pub fn apply_arrow_batch(
            &self,
            batch: &arrow_array::RecordBatch,
            digest: bool,
            transform: bool,
            cast: bool,
            options: crate::ArrowCastOptions,
        ) -> Result<arrow_array::RecordBatch> {
            AppliedPlan::compile(self, batch.schema(), digest, transform, cast, options)?
                .apply(batch)
        }
        /// Answer the schema [`Self::apply_arrow_batch`] produces, with no rows.
        ///
        /// A reader has to report its schema before it yields anything, and a
        /// caller planning a write needs the same answer. Both get it here: the
        /// declarations name every column they add, so the applied shape is a
        /// property of two schemas and never of the data. The batch is the empty
        /// one, so nothing is decoded and no column is materialized beyond its
        /// zero-length arrays - and a declaration that cannot be satisfied, a
        /// source column the reader does not carry above all, fails here rather
        /// than on the first batch.
        ///
        /// ```
        /// use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
        /// use yggdryl::expression::Function;
        /// use yggdryl::{ArrowCastOptions, DataType};
        ///
        /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
        /// let mut year = DataType::Int32.nullable_field("year");
        /// year.as_partition_mut().set_sources(["event"])?;
        /// year.as_partition_mut().set_transform(Function::Year)?;
        /// let root = DataType::from_fields([DataType::Date32.required_field("event"), year])?
        ///     .required_field("row");
        ///
        /// let stored = Schema::new(vec![ArrowField::new(
        ///     "event",
        ///     ArrowDataType::Date32,
        ///     false,
        /// )]);
        ///
        /// let applied =
        ///     root.apply_arrow_schema(stored.into(), true, true, true, ArrowCastOptions::new())?;
        ///
        /// assert_eq!(applied.fields().len(), 2);
        /// assert_eq!(applied.field(1).name(), "year");
        /// # Ok(())
        /// # }
        /// ```
        ///
        /// # Errors
        ///
        /// [`Self::apply_arrow_batch`] carries the rule.
        pub fn apply_arrow_schema(
            &self,
            schema: arrow_schema::SchemaRef,
            digest: bool,
            transform: bool,
            cast: bool,
            options: crate::ArrowCastOptions,
        ) -> Result<arrow_schema::SchemaRef> {
            Ok(AppliedPlan::compile(self, schema, digest, transform, cast, options)?.schema)
        }
        /// Wrap a reader so every batch it yields has this schema applied.
        ///
        /// The stream form of [`Self::apply_arrow_batch`], and a reader for the
        /// reason every streaming shape in this crate is one: a lake being read
        /// into a lake being written should not have to be held in memory to gain
        /// its derived columns. The applied plan - the cast, the applied schema,
        /// and the strict re-check - is compiled once from the reader's schema, so
        /// the returned reader answers that schema before the first batch is
        /// pulled and no batch is planned for twice.
        ///
        /// # Errors
        ///
        /// Returns an error when the applied schema cannot be derived. A failure
        /// on one batch surfaces as that batch's `Err`, and the reader is not
        /// fused after it: it yields whatever the inner reader yields next.
        pub fn apply_arrow_reader(
            &self,
            inner: crate::arrow::BatchReader,
            digest: bool,
            transform: bool,
            cast: bool,
            options: crate::ArrowCastOptions,
        ) -> Result<crate::arrow::BatchReader> {
            if !digest && !transform && !cast {
                return Ok(inner);
            }
            let plan =
                AppliedPlan::compile(self, inner.schema(), digest, transform, cast, options)?;
            Ok(Box::new(AppliedReader { inner, plan }))
        }
        /// Materializes [`Field::default_value`] as an exact one-row array.
        ///
        /// The bounded core default planner selects the value under this Field's
        /// own nullability policy: a nullable Field materializes logical null and
        /// a non-nullable one its datatype's present default.
        ///
        /// # Errors
        ///
        /// Returns an error when no physically valid default exists or Arrow
        /// cannot materialize the datatype.
        pub fn default_arrow_array(&self) -> crate::arrow::Result<arrow_array::ArrayRef> {
            crate::arrow::default_scalar_array(self)
        }
        /// Imports one complete Arrow schema as a non-null Struct root Field.
        ///
        /// Ordinary schema metadata becomes root metadata. The transport-only
        /// dictionary-ID sidecar is consumed without entering Field metadata.
        ///
        /// # Errors
        ///
        /// Returns an error when the Arrow fields cannot form a non-null Struct
        /// root or the dictionary-ID sidecar is invalid.
        pub fn from_arrow_schema(name: &str, schema: &Schema) -> crate::arrow::Result<Self> {
            crate::arrow::field_from_arrow_schema(name, schema)
        }
        /// Imports an Arrow field and seeds the projection cache.
        pub fn from_arrow_field(value: &ArrowField) -> Result<Self> {
            Self::from_arrow_field_at_depth(value, 0)
        }
        pub(crate) fn from_arrow_field_at_depth(value: &ArrowField, depth: usize) -> Result<Self> {
            let (dtype, metadata) = imported_parts(value, depth)?;
            let mut field = imported_field(value, dtype, metadata);
            let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
            seed_imported_arrow_cache(&mut field, cacheable, || Arc::new(value.clone()));
            Ok(field)
        }
        /// Imports a shared Arrow field without cloning its projection allocation.
        pub fn from_arrow_field_ref(value: FieldRef) -> Result<Self> {
            Self::from_arrow_field_ref_at_depth(value, 0)
        }
        pub(crate) fn from_arrow_field_ref_at_depth(value: FieldRef, depth: usize) -> Result<Self> {
            let (dtype, metadata) = imported_parts(&value, depth)?;
            let mut field = imported_field(&value, dtype, metadata);
            let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
            seed_imported_arrow_cache(&mut field, cacheable, || value);
            Ok(field)
        }
        pub(crate) fn from_arrow_field_owned_at_depth(
            value: ArrowField,
            depth: usize,
        ) -> Result<Self> {
            let (dtype, metadata) = imported_parts(&value, depth)?;
            let mut field = imported_field(&value, dtype, metadata);
            let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
            seed_imported_arrow_cache(&mut field, cacheable, || Arc::new(value));
            Ok(field)
        }
    }

    fn seed_imported_arrow_cache(
        field: &mut Field,
        cacheable: bool,
        projection: impl FnOnce() -> FieldRef,
    ) {
        if cacheable {
            field.seed_arrow_cache(projection());
        }
    }

    fn imported_arrow_is_cacheable(
        field: &Field,
        arrow_metadata: &HashMap<String, String>,
    ) -> bool {
        field.as_metadata().matches_arrow(arrow_metadata)
            && field.dtype().arrow_import_is_projection_equivalent()
    }

    /// The core identity an Arrow field's extension metadata declares, when it
    /// declares one of the first-class extension-typed datatypes.
    pub(crate) enum RecognizedExtension {
        /// The canonical `arrow.parquet.variant` over its exact storage struct.
        Variant,
        /// The community `geoarrow.wkb` over Binary storage; the parsed GeoArrow
        /// document says whether it is a geometry or a geography.
        Geospatial(GeospatialParameters),
        /// A code's own `yggdryl.{country,currency,mic,cfi}` over Utf8.
        ///
        /// It is separate from [`Self::String`] because the identity is the
        /// point: text under `yggdryl.currency` is a currency and the same text
        /// under `yggdryl.string` is a bounded ASCII string, and neither imports
        /// as the other.
        Code(DataType),
        /// The `yggdryl.string` extension: a layout, a charset and a bound over
        /// the Arrow storage that layout and charset lay out.
        String(DataType),
        /// The `yggdryl.bytes` extension: a layout and a bound over the Arrow
        /// storage that layout is.
        Bytes(DataType),
        /// An identifier over `FixedSizeBinary(16)`: the canonical
        /// `arrow.uuid`, or `yggdryl.uuid` with the version it admits.
        Uuid(UuidType),
        /// The canonical version text over Utf8.
        Version,
        /// The canonical URL text over Utf8.
        Url,
        /// The canonical time zone name over Utf8.
        Timezone,
        /// The canonical MIME type over Utf8.
        MimeType,
        /// The canonical media type over Utf8.
        MediaType,
    }

    impl RecognizedExtension {
        /// The first-class datatype this recognized extension imports as.
        pub(crate) fn into_dtype(self) -> DataType {
            match self {
                Self::Variant => DataType::Variant,
                Self::Geospatial(geospatial) => {
                    if geospatial.algorithm().is_some() {
                        DataType::Geography(Arc::new(geospatial))
                    } else {
                        DataType::Geometry(Arc::new(geospatial))
                    }
                }
                Self::Code(dtype) | Self::String(dtype) | Self::Bytes(dtype) => dtype,
                Self::Uuid(family) => DataType::Uuid(family),
                Self::Version => DataType::Version,
                Self::Url => DataType::Url,
                Self::Timezone => DataType::Timezone,
                Self::MimeType => DataType::MimeType,
                Self::MediaType => DataType::MediaType,
            }
        }
    }

    /// Recognizes the Arrow extension spellings the first-class datatypes ride:
    /// `geoarrow.wkb` over Binary storage, the canonical `arrow.parquet.variant`
    /// over its exact storage struct with an empty extension metadata document,
    /// `yggdryl.string` and `yggdryl.bytes` over the storage their documents lay
    /// out, each registered code's own `yggdryl.{country,currency,mic,cfi}` over
    /// Utf8, and the canonical `arrow.uuid` over `FixedSizeBinary(16)`, each with
    /// an empty or absent document.
    ///
    /// The answer is what the extension describes, which is the *values* of a
    /// dictionary-encoded column: [`encoded_values`] peels the encoding here and
    /// [`imported_parts`] puts it back, so no extension is recognized in one
    /// layout and lost in the other.
    ///
    /// Any other pairing keeps today's behavior exactly - a foreign extension
    /// name, one of ours over a storage it does not spell, a variant or a code
    /// with a non-empty document: the field imports as its storage type with the
    /// `ARROW:extension:*` keys as plain metadata. A code, a version and a URL
    /// all ride Utf8, so it is the *name* that separates them, and a name none of
    /// them claims leaves the column the plain text it is.
    ///
    /// # Errors
    ///
    /// Returns an error when a recognized `geoarrow.wkb` field carries a GeoArrow
    /// metadata document that does not parse, or when a dictionary key is not an
    /// Arrow type this crate imports.
    pub(crate) fn recognized_arrow_extension(
        metadata: &HashMap<String, String>,
        storage: &ArrowDataType,
    ) -> Result<Option<RecognizedExtension>> {
        let Some(name) = metadata.get(EXTENSION_TYPE_NAME_KEY) else {
            return Ok(None);
        };
        let document = metadata
            .get(EXTENSION_TYPE_METADATA_KEY)
            .map(String::as_str);
        let (storage, _) = encoded_values(storage)?;
        match name.as_str() {
            GEOARROW_WKB_EXTENSION_NAME if storage == &ArrowDataType::Binary => {
                let geospatial =
                    GeospatialParameters::from_geoarrow_json(document).map_err(|error| {
                        Error::InvalidMetadataValue {
                            key: SmolStr::new_static(EXTENSION_TYPE_METADATA_KEY),
                            reason: format_smolstr!("{error}"),
                        }
                    })?;
                Ok(Some(RecognizedExtension::Geospatial(geospatial)))
            }
            VARIANT_EXTENSION_NAME
                if is_variant_storage(storage) && document.unwrap_or("").is_empty() =>
            {
                Ok(Some(RecognizedExtension::Variant))
            }
            // A string declares everything about itself in its document, so the
            // storage is checked against what that document lays out: our name
            // over a storage it does not describe is a foreign field wearing it,
            // and that imports as its storage rather than as a string.
            STRING_EXTENSION_NAME => {
                let Some(document) = document else {
                    return Ok(None);
                };
                let parameters = StringType::from_extension_json(document).map_err(|error| {
                    Error::InvalidMetadataValue {
                        key: SmolStr::new_static(EXTENSION_TYPE_METADATA_KEY),
                        reason: format_smolstr!("{error}"),
                    }
                })?;
                if !crate::types::string::describes_storage(parameters, storage)? {
                    return Ok(None);
                }
                Ok(Some(RecognizedExtension::String(DataType::string(
                    parameters,
                )?)))
            }
            // The same rule for bytes: the document names the layout, and the
            // storage must be that layout.
            BYTES_EXTENSION_NAME => {
                let Some(document) = document else {
                    return Ok(None);
                };
                let parameters = BytesType::from_extension_json(document).map_err(|error| {
                    Error::InvalidMetadataValue {
                        key: SmolStr::new_static(EXTENSION_TYPE_METADATA_KEY),
                        reason: format_smolstr!("{error}"),
                    }
                })?;
                if !crate::types::bytes::describes_storage(parameters, storage)? {
                    return Ok(None);
                }
                Ok(Some(RecognizedExtension::Bytes(DataType::bytes(
                    parameters,
                )?)))
            }
            UUID_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
                Ok(matches!(storage, ArrowDataType::FixedSizeBinary(16))
                    .then_some(RecognizedExtension::Uuid(UuidType::Uuid)))
            }
            // A version is what the document states, and the storage must be
            // the sixteen bytes every identifier is: our name over anything
            // else is a foreign field wearing it, and imports as its storage.
            UUID_VERSION_EXTENSION_NAME => {
                let Some(document) = document else {
                    return Ok(None);
                };
                let Some(family) = UuidType::from_extension_json(document) else {
                    return Ok(None);
                };
                Ok(matches!(storage, ArrowDataType::FixedSizeBinary(16))
                    .then_some(RecognizedExtension::Uuid(family)))
            }
            VERSION_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
                Ok(matches!(storage, ArrowDataType::Utf8).then_some(RecognizedExtension::Version))
            }
            URL_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
                Ok(matches!(storage, ArrowDataType::Utf8).then_some(RecognizedExtension::Url))
            }
            TIMEZONE_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
                Ok(matches!(storage, ArrowDataType::Utf8).then_some(RecognizedExtension::Timezone))
            }
            MIMETYPE_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
                Ok(matches!(storage, ArrowDataType::Utf8).then_some(RecognizedExtension::MimeType))
            }
            MEDIATYPE_EXTENSION_NAME if document.unwrap_or("").is_empty() => Ok(matches!(
                storage,
                ArrowDataType::Utf8
            )
            .then_some(RecognizedExtension::MediaType)),
            code if document.unwrap_or("").is_empty() && matches!(storage, ArrowDataType::Utf8) => {
                Ok(code_for_extension(code).map(RecognizedExtension::Code))
            }
            _ => Ok(None),
        }
    }

    /// The storage one extension describes, and the dictionary key it sits under.
    ///
    /// Arrow's `Dictionary` carries a bare datatype for its values rather than a
    /// field, so a dictionary-encoded extension column has nowhere but the field
    /// itself to declare its identity. Peeling here is what lets a caller's own
    /// `dictionary(int32, currency)` - or `dictionary(int32, uuid)`, or any other
    /// extension - import as itself rather than as anonymous storage; no datatype
    /// this crate recognizes *is* a dictionary - a code is its own fixed binary -
    /// so this is only about not losing what a caller composed.
    ///
    /// # Errors
    ///
    /// Returns an error when the dictionary key is not an Arrow type this crate
    /// imports.
    fn encoded_values(storage: &ArrowDataType) -> Result<(&ArrowDataType, Option<DataType>)> {
        match storage {
            ArrowDataType::Dictionary(key, value) => Ok((
                value.as_ref(),
                Some(DataType::from_arrow_datatype(key.as_ref())?),
            )),
            other => Ok((other, None)),
        }
    }

    /// One recognized values type, put back under the key it was found beneath.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is not an integer a dictionary may use.
    fn re_encoded(values: DataType, key: Option<DataType>) -> Result<DataType> {
        match key {
            Some(key) => DataType::dictionary(key, values),
            None => Ok(values),
        }
    }

    /// Imports the datatype and metadata halves of one Arrow field.
    ///
    /// A recognized extension imports as its first-class datatype, and the two
    /// `ARROW:extension:*` keys are stripped: they are transport, exactly like
    /// the reserved `PARQUET:field_id` spelling, and the projection back to Arrow
    /// re-derives them from the datatype. Every other field imports unchanged.
    fn imported_parts(value: &ArrowField, depth: usize) -> Result<(DataType, Metadata)> {
        if let Some(recognized) = recognized_arrow_extension(value.metadata(), value.data_type())? {
            let stripped: HashMap<String, String> = value
                .metadata()
                .iter()
                .filter(|(key, _)| {
                    key.as_str() != EXTENSION_TYPE_NAME_KEY
                        && key.as_str() != EXTENSION_TYPE_METADATA_KEY
                })
                .map(|(key, held)| (key.clone(), held.clone()))
                .collect();
            // The extension describes the values; the encoding it was found under
            // is the caller's, and goes back on.
            let (_, key) = encoded_values(value.data_type())?;
            return Ok((
                re_encoded(recognized.into_dtype(), key)?,
                Metadata::from_arrow_metadata(&stripped)?,
            ));
        }
        let metadata = Metadata::from_arrow_metadata(value.metadata())?;
        let dtype = DataType::from_arrow_datatype_at_depth(value.data_type(), depth)?;
        Ok((dtype, metadata))
    }

    /// Completes a projected Arrow metadata map with the extension identity of an
    /// extension-typed datatype, refusing a user-set extension key.
    ///
    /// The two `ARROW:extension:*` entries belong to the datatype: letting a
    /// caller's own value ride along would let one field name two extensions, so
    /// a conflict is refused naming both spellings rather than silently picking.
    fn projected_arrow_metadata(
        dtype: &DataType,
        mut metadata: HashMap<String, String>,
    ) -> Result<HashMap<String, String>> {
        let Some((name, document)) = dtype.arrow_extension() else {
            return Ok(metadata);
        };
        for key in [EXTENSION_TYPE_NAME_KEY, EXTENSION_TYPE_METADATA_KEY] {
            if let Some(existing) = metadata.get(key) {
                return Err(Error::InvalidMetadataValue {
                    key: SmolStr::new_static(key),
                    reason: format_smolstr!(
                        "expected no caller-set Arrow extension entry on a {} field, \
                         got {existing:?}; the datatype itself projects {name:?}",
                        dtype.name()
                    ),
                });
            }
        }
        metadata.insert(EXTENSION_TYPE_NAME_KEY.to_owned(), name.to_owned());
        metadata.insert(EXTENSION_TYPE_METADATA_KEY.to_owned(), document);
        Ok(metadata)
    }

    fn imported_field(value: &ArrowField, dtype: DataType, metadata: Metadata) -> Field {
        #[allow(deprecated)]
        let dictionary_id = value.dict_id().unwrap_or_default();
        let mut field =
            Field::new_with_metadata(value.name(), dtype, value.is_nullable(), metadata);
        field.set_dictionary_options_unchecked(
            dictionary_id,
            value.dict_is_ordered().unwrap_or_default(),
        );
        field
    }

    /// Builds a field C schema while retaining flags already owned by its datatype.
    pub(crate) fn arrow_field_to_ffi(
        field: &ArrowField,
    ) -> std::result::Result<FFI_ArrowSchema, arrow_schema::ArrowError> {
        let mut schema = arrow_dtype_to_ffi(field.data_type())?;
        let mut flags = schema.flags().unwrap_or_else(Flags::empty);
        if field.is_nullable() {
            flags |= Flags::NULLABLE;
        }
        if field.dict_is_ordered() == Some(true) {
            flags |= Flags::DICTIONARY_ORDERED;
        }
        schema = schema.with_name(field.name())?.with_flags(flags)?;
        schema.with_metadata(field.metadata())
    }

    #[allow(deprecated)]
    fn arrow_field_from_parts(
        name: &str,
        dtype: arrow_schema::DataType,
        nullable: bool,
        dictionary_id: i64,
        dictionary_is_ordered: bool,
        metadata: HashMap<String, String>,
    ) -> ArrowField {
        ArrowField::new_dict(name, dtype, nullable, dictionary_id, dictionary_is_ordered)
            .with_metadata(metadata)
    }

    impl TryFrom<&Field> for ArrowField {
        type Error = Error;

        fn try_from(value: &Field) -> Result<Self> {
            value.clone().into_arrow_field()
        }
    }

    impl TryFrom<Field> for ArrowField {
        type Error = Error;

        fn try_from(value: Field) -> Result<Self> {
            value.into_arrow_field()
        }
    }

    impl TryFrom<&ArrowField> for Field {
        type Error = Error;

        fn try_from(value: &ArrowField) -> Result<Self> {
            Self::from_arrow_field(value)
        }
    }

    impl TryFrom<ArrowField> for Field {
        type Error = Error;

        fn try_from(value: ArrowField) -> Result<Self> {
            Self::from_arrow_field_owned_at_depth(value, 0)
        }
    }

    /// One root's declarations, compiled once against one source schema.
    ///
    /// A reader applies the same three steps to every batch, and all three answer
    /// from the schemas alone: which columns the cast reconciles, which columns
    /// each protocol declares, and what the applied shape therefore is. Compiling
    /// that once is what lets a stream pay for it once.
    pub(crate) struct AppliedPlan {
        root: Field,
        cast: Option<crate::types::cast::ArrowCastPlan>,
        transform: bool,
        digest: bool,
        /// The strict re-check over the finished batch, compiled only when a
        /// protocol was allowed to leave a hole for itself, or when no cast ran.
        verify: Option<crate::types::cast::ArrowCastPlan>,
        /// The schema every applied batch carries.
        schema: arrow_schema::SchemaRef,
    }

    impl AppliedPlan {
        /// Compiles the three steps and derives the applied schema, without rows.
        pub(crate) fn compile(
            root: &Field,
            source: arrow_schema::SchemaRef,
            digest: bool,
            transform: bool,
            cast: bool,
            options: crate::ArrowCastOptions,
        ) -> Result<Self> {
            use crate::types::cast::{ArrowCastPlan, Deferred};

            // A protocol is asked whether it declares anything before it is
            // planned: both walk every batch, and the digest fill casts one a
            // second time to materialize the holder columns. A root that declares
            // neither is the ordinary schema, and applying it must cost exactly
            // the cast. The question is answered on the declaration, so it reads
            // no row - but only after `require_struct`, because a root the
            // protocols cannot run on at all is refused rather than skipped.
            if transform || digest {
                root.require_struct()?;
            }
            let transform = transform && root.as_transform().declares_derivation();
            let digest = digest && root.as_digest().declares_holder();

            let cast = if cast {
                Some(ArrowCastPlan::compile_deferring(
                    &source,
                    root,
                    options,
                    Deferred { transform, digest },
                )?)
            } else {
                None
            };
            // The applied shape is a property of the two schemas, so it is read off
            // an empty batch: nothing is decoded, and a declaration that cannot be
            // satisfied fails here rather than on the first batch.
            let empty = arrow_array::RecordBatch::new_empty(source);
            let applied = Self::stages(root, cast.as_ref(), transform, digest, &empty)?;
            let schema = applied.schema();
            // A cast with no protocol behind it already refused every hole, so the
            // re-check exists only where something could still have left one.
            let verify =
                if options.nullability().is_strict() && (transform || digest || cast.is_none()) {
                    Some(ArrowCastPlan::compile(schema.as_ref(), root, options)?)
                } else {
                    None
                };
            Ok(Self {
                root: root.clone(),
                cast,
                transform,
                digest,
                verify,
                schema,
            })
        }

        /// Run cast, then transform, then digest - the order their answers depend
        /// on, and the order this crate publishes.
        fn stages(
            root: &Field,
            cast: Option<&crate::types::cast::ArrowCastPlan>,
            transform: bool,
            digest: bool,
            batch: &arrow_array::RecordBatch,
        ) -> Result<arrow_array::RecordBatch> {
            let mut applied = match cast {
                Some(plan) => plan.apply(batch.clone())?,
                None => batch.clone(),
            };
            if transform {
                applied = root.as_transform().apply_arrow_batch(&applied)?;
            }
            if digest {
                applied = root.as_digest().apply_arrow_batch(&applied)?;
            }
            Ok(applied)
        }

        /// Apply the compiled declarations to one batch of the source schema.
        pub(crate) fn apply(
            &self,
            batch: &arrow_array::RecordBatch,
        ) -> Result<arrow_array::RecordBatch> {
            let applied = Self::stages(
                &self.root,
                self.cast.as_ref(),
                self.transform,
                self.digest,
                batch,
            )?;
            if let Some(verify) = &self.verify {
                // The applied batch is already the declared shape, so this is a
                // zero-copy pass whose only product is the refusal it may raise.
                verify.apply(applied.clone())?;
            }
            Ok(applied)
        }
    }

    /// A reader applying one root's declarations to every batch it yields.
    ///
    /// The schema is the applied one from the start, so a consumer reads the shape
    /// it will get rather than the shape the inner reader stores.
    struct AppliedReader {
        inner: crate::arrow::BatchReader,
        plan: AppliedPlan,
    }

    impl Iterator for AppliedReader {
        type Item = std::result::Result<arrow_array::RecordBatch, arrow_schema::ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            let batch = match self.inner.next()? {
                Ok(batch) => batch,
                Err(error) => return Some(Err(error)),
            };
            Some(self.plan.apply(&batch).map_err(|error| {
                arrow_schema::ArrowError::ComputeError(format!(
                    "the declared columns could not be applied: {error}"
                ))
            }))
        }
    }

    impl arrow_array::RecordBatchReader for AppliedReader {
        fn schema(&self) -> arrow_schema::SchemaRef {
            Arc::clone(&self.plan.schema)
        }
    }
    /// Builds a C Data Interface schema without losing nested datatype flags.
    ///
    /// Arrow's own `FFI_ArrowSchema` conversion rebuilds a nested node from its
    /// format string alone and drops the flags its children own, so the walk is
    /// this crate's: each child is written by [`arrow_field_to_ffi`] and the
    /// parent keeps the flags its template declared.
    pub(crate) fn arrow_dtype_to_ffi(
        dtype: &ArrowDataType,
    ) -> std::result::Result<FFI_ArrowSchema, arrow_schema::ArrowError> {
        let template = FFI_ArrowSchema::try_from(dtype)?;
        let children = match dtype {
            ArrowDataType::List(field)
            | ArrowDataType::ListView(field)
            | ArrowDataType::FixedSizeList(field, _)
            | ArrowDataType::LargeList(field)
            | ArrowDataType::LargeListView(field)
            | ArrowDataType::Map(field, _) => vec![arrow_field_to_ffi(field)?],
            ArrowDataType::Struct(fields) => fields
                .iter()
                .map(|field| arrow_field_to_ffi(field))
                .collect::<std::result::Result<Vec<_>, _>>()?,
            ArrowDataType::Union(fields, _) => fields
                .iter()
                .map(|(_, field)| arrow_field_to_ffi(field))
                .collect::<std::result::Result<Vec<_>, _>>()?,
            ArrowDataType::RunEndEncoded(run_ends, values) => {
                vec![arrow_field_to_ffi(run_ends)?, arrow_field_to_ffi(values)?]
            }
            _ => Vec::new(),
        };
        let dictionary = match dtype {
            ArrowDataType::Dictionary(_, value) => Some(arrow_dtype_to_ffi(value)?),
            _ => None,
        };
        let flags = template.flags().unwrap_or_else(Flags::empty);
        FFI_ArrowSchema::try_new(template.format(), children, dictionary)?.with_flags(flags)
    }

    /// Projects a shared child field, consuming the handle when it is the last.
    ///
    /// A nested datatype holds its children behind `Arc`, and a datatype being
    /// consumed usually holds the only handle: unwrapping it moves the field into
    /// the projection rather than cloning a whole subtree to throw it away.
    ///
    /// # Errors
    ///
    /// Returns an error when the field has no Arrow projection.
    pub(crate) fn arrow_field_ref_from_shared(field: Arc<Field>) -> Result<FieldRef> {
        match Arc::try_unwrap(field) {
            Ok(field) => field.into_arrow_field_ref(),
            Err(field) => field.as_ref().clone().into_arrow_field_ref(),
        }
    }
}

pub(crate) use arrow::{
    RecognizedExtension, arrow_field_ref_from_shared, recognized_arrow_extension,
};
