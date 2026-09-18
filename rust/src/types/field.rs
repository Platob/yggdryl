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
use crate::types::{DataType, DataTypeValue, FieldValue, preflight_schema_shape};
use crate::types::{
    NullType, BooleanType, Int8Type, Int16Type,
    Int32Type, Int64Type, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type, Float16Type, Float32Type,
    Float64Type, DateTime64Type, Date32Type, Date64Type,
    Time32Type, Time64Type, Duration32Type, Duration64Type,
    IntervalType, BytesType, StringType, CountryType,
    CurrencyType, MicType, CfiType, IsinType,
    SideType, StateType, TimeInForceType, UuidType,
    VersionType, UrlType, SequenceType, StructureType,
    UnionType, EnumType, Decimal32Type, Decimal64Type,
    Decimal128Type, Decimal256Type, MappingType, RunEndType,
    VariantType, GeometryType, GeographyType, TimezoneType,
    MimeTypeType, MediaTypeType, CusipType, SedolType,
    BloombergType,
};

use crate::{DataTypeId, Error, Metadata, Result, Scheme, Url};

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
pub struct FieldOf<D: DataTypeValue> {
    pub(crate) name: SmolStr,
    pub(crate) dtype: D,
    pub(crate) nullable: bool,
    pub(crate) dictionary_id: i64,
    pub(crate) dictionary_is_ordered: bool,
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
            dictionary_id: 0,
            dictionary_is_ordered: false,
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
            dictionary_id: 0,
            dictionary_is_ordered: false,
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
            dictionary_id: 0,
            dictionary_is_ordered: false,
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
        if matches!(self.dtype.id(), DataTypeId::Dictionary) {
            Some(self.dictionary_id)
        } else {
            None
        }
    }

    /// Returns Arrow's dictionary ordering flag for dictionary fields.
    pub fn dictionary_is_ordered(&self) -> Option<bool> {
        if matches!(self.dtype.id(), DataTypeId::Dictionary) {
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
        field.set_dictionary_options_unchecked(self.dictionary_id, self.dictionary_is_ordered);
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
            let keeps_dictionary_options =
                matches!(dtype.id(), crate::DataTypeId::Dictionary);
            self.dtype = dtype;
            if !keeps_dictionary_options {
                self.dictionary_id = 0;
                self.dictionary_is_ordered = false;
            }
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
        if !matches!(self.dtype.id(), DataTypeId::Dictionary) {
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
            dictionary_id: self.dictionary_id,
            dictionary_is_ordered: self.dictionary_is_ordered,
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
            .field("dictionary_id", &self.dictionary_id)
            .field("dictionary_is_ordered", &self.dictionary_is_ordered)
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
            return fmt::Display::fmt(&self.to_field().pretty(), formatter);
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

impl<D: DataTypeValue> PartialEq for FieldOf<D> {
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

impl<D: DataTypeValue> Hash for FieldOf<D> {
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

impl<D: DataTypeValue> FieldOf<D> {

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
                match self {
                    $(Self::$variant(field) => {
                        field.dictionary_id = id;
                        field.dictionary_is_ordered = is_ordered;
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
            pub fn as_arrow_ref(&self) -> Result<&FieldRef> {
                match self { $(Self::$variant(field) => field.as_arrow_ref(),)+ }
            }

            /// Consumes this field and returns an owned Arrow field.
            pub fn into_arrow(self) -> Result<arrow_schema::Field> {
                match self { $(Self::$variant(field) => field.into_arrow(),)+ }
            }

            /// Consumes this field and returns a shared Arrow field.
            pub fn into_arrow_ref(self) -> Result<FieldRef> {
                match self { $(Self::$variant(field) => field.into_arrow_ref(),)+ }
            }

            /// Consumes this field and returns its C schema.
            pub fn into_arrow_ffi(self) -> Result<arrow_schema::ffi::FFI_ArrowSchema> {
                match self { $(Self::$variant(field) => field.into_arrow_ffi(),)+ }
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
    Decimal32 => Decimal32Field / Decimal32Type,
    Decimal64 => Decimal64Field / Decimal64Type,
    Decimal128 => Decimal128Field / Decimal128Type,
    Decimal256 => Decimal256Field / Decimal256Type,
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
        let mut merged = Self::new(self.name(), dtype, self.is_nullable() || other.is_nullable());
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

impl Index<&str> for Field {
    type Output = Self;

    fn index(&self, name: &str) -> &Self::Output {
        self.get_field_by_path(name)
            .unwrap_or_else(|| panic!("{:?} is not a child of the field {:?}", name, self.name()))
    }
}
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
