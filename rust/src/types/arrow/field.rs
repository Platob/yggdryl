//! Arrow field import, cached projection, and conversion traits.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[cfg(feature = "arrow")]
use arrow_schema::Schema;
use arrow_schema::{
    DataType as ArrowDataType, Field as ArrowField,
    extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY},
    ffi::{FFI_ArrowSchema, Flags},
};
use smol_str::{SmolStr, format_smolstr};

use crate::types::{
    ASCII_EXTENSION_NAME, GEOARROW_WKB_EXTENSION_NAME, UUID_EXTENSION_NAME, VARIANT_EXTENSION_NAME,
    VERSION_EXTENSION_NAME, arrow_dtype_to_ffi, arrow_extension_parts, code_for_extension,
    is_variant_storage,
};
use crate::types::{Field, FieldRef};
use crate::{DataType, Error, GeospatialParameters, Metadata, Result};

impl Field {
    /// Imports one complete Arrow schema as a non-null Struct root Field.
    ///
    /// Ordinary schema metadata becomes root metadata. The transport-only
    /// dictionary-ID sidecar is consumed without entering Field metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the Arrow fields cannot form a non-null Struct
    /// root or the dictionary-ID sidecar is invalid.
    #[cfg(feature = "arrow")]
    pub fn from_arrow_schema(name: &str, schema: &Schema) -> crate::arrow::Result<Self> {
        crate::arrow::field_from_arrow_schema(name, schema)
    }

    /// Imports an Arrow field and seeds the projection cache.
    pub fn from_arrow(value: &ArrowField) -> Result<Self> {
        Self::from_arrow_at_depth(value, 0)
    }

    pub(crate) fn from_arrow_at_depth(value: &ArrowField, depth: usize) -> Result<Self> {
        let (dtype, metadata) = imported_parts(value, depth)?;
        let mut field = imported_field(value, dtype, metadata);
        let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
        seed_imported_arrow_cache(&mut field, cacheable, || Arc::new(value.clone()));
        Ok(field)
    }

    /// Imports a shared Arrow field without cloning its projection allocation.
    pub fn from_arrow_ref(value: FieldRef) -> Result<Self> {
        Self::from_arrow_ref_at_depth(value, 0)
    }

    pub(crate) fn from_arrow_ref_at_depth(value: FieldRef, depth: usize) -> Result<Self> {
        let (dtype, metadata) = imported_parts(&value, depth)?;
        let mut field = imported_field(&value, dtype, metadata);
        let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
        seed_imported_arrow_cache(&mut field, cacheable, || value);
        Ok(field)
    }

    pub(crate) fn from_arrow_owned_at_depth(value: ArrowField, depth: usize) -> Result<Self> {
        let (dtype, metadata) = imported_parts(&value, depth)?;
        let mut field = imported_field(&value, dtype, metadata);
        let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
        seed_imported_arrow_cache(&mut field, cacheable, || Arc::new(value));
        Ok(field)
    }

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
    #[cfg(feature = "arrow")]
    pub fn into_arrow_exchange_schema(self) -> crate::arrow::Result<Schema> {
        crate::arrow::arrow_exchange_schema_from_field(&self)
    }

    /// Consumes this Field and projects it as an Arrow schema.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a bounded, non-nullable Struct root.
    #[cfg(feature = "arrow")]
    pub fn into_arrow_schema(self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
        crate::arrow::arrow_schema_from_field(&self)
    }

    /// Apply this schema's metadata-declared columns to one Arrow batch.
    ///
    /// A [`Field`] states more about a batch than its shape: a
    /// [`partition:`](crate::PartitionField::apply_arrow_batch) declaration
    /// says a column is *derived* from another, and a
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
    #[cfg(feature = "arrow")]
    pub fn apply_arrow_batch(
        &self,
        batch: &arrow_array::RecordBatch,
        digest: bool,
        partition: bool,
        cast: bool,
        options: crate::ArrowCastOptions,
    ) -> Result<arrow_array::RecordBatch> {
        AppliedPlan::compile(self, batch.schema(), digest, partition, cast, options)?.apply(batch)
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
    #[cfg(feature = "arrow")]
    pub fn apply_arrow_schema(
        &self,
        schema: arrow_schema::SchemaRef,
        digest: bool,
        partition: bool,
        cast: bool,
        options: crate::ArrowCastOptions,
    ) -> Result<arrow_schema::SchemaRef> {
        Ok(AppliedPlan::compile(self, schema, digest, partition, cast, options)?.schema)
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
    #[cfg(feature = "arrow")]
    pub fn apply_arrow_reader(
        &self,
        inner: crate::arrow::BatchReader,
        digest: bool,
        partition: bool,
        cast: bool,
        options: crate::ArrowCastOptions,
    ) -> Result<crate::arrow::BatchReader> {
        if !digest && !partition && !cast {
            return Ok(inner);
        }
        let plan = AppliedPlan::compile(self, inner.schema(), digest, partition, cast, options)?;
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
    #[cfg(feature = "arrow")]
    pub fn default_arrow_array(&self) -> crate::arrow::Result<arrow_array::ArrayRef> {
        crate::arrow::default_scalar_array(self)
    }

    /// Projects this field to an owned Arrow C Data Interface schema.
    ///
    /// Name, metadata, nullability, dictionary ordering, and nested datatype
    /// flags are preserved in one canonical core conversion.
    pub fn into_arrow_ffi(self) -> Result<FFI_ArrowSchema> {
        if let Some(field) = self.arrow.get() {
            return arrow_field_to_ffi(field).map_err(Error::from);
        }
        let mut schema = self.dtype.clone().into_arrow_ffi()?;
        let mut flags = schema.flags().unwrap_or_else(Flags::empty);
        if self.nullable {
            flags |= Flags::NULLABLE;
        }
        if self.dictionary_is_ordered {
            flags |= Flags::DICTIONARY_ORDERED;
        }
        schema = schema.with_name(self.name())?.with_flags(flags)?;
        // The datatype projection already carries the extension entries for an
        // extension-typed field, but the C interface stores metadata as
        // one buffer, so the replacement map must carry them again.
        schema
            .with_metadata(&projected_arrow_metadata(
                &self.dtype,
                self.metadata.clone().into_arrow(),
            )?)
            .map_err(Error::from)
    }

    /// Consumes this field and returns an owned Arrow field.
    pub fn into_arrow(self) -> Result<ArrowField> {
        let Self {
            name,
            dtype,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            metadata,
            arrow,
        } = self;
        if let Some(field) = arrow.into_inner() {
            return Ok(Arc::try_unwrap(field).unwrap_or_else(|field| field.as_ref().clone()));
        }
        let projected = projected_arrow_metadata(&dtype, metadata.into_arrow())?;
        Ok(arrow_field_from_parts(
            name.as_str(),
            dtype.into_arrow()?,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            projected,
        ))
    }

    /// Consumes this field and returns a shared Arrow projection.
    pub fn into_arrow_ref(self) -> Result<FieldRef> {
        let Self {
            name,
            dtype,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            metadata,
            arrow,
        } = self;
        if let Some(field) = arrow.into_inner() {
            return Ok(field);
        }
        let projected = projected_arrow_metadata(&dtype, metadata.into_arrow())?;
        Ok(Arc::new(arrow_field_from_parts(
            name.as_str(),
            dtype.into_arrow()?,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            projected,
        )))
    }
}

fn seed_imported_arrow_cache(
    field: &mut Field,
    cacheable: bool,
    projection: impl FnOnce() -> FieldRef,
) {
    if cacheable {
        field.arrow = OnceLock::from(projection());
    }
}

fn imported_arrow_is_cacheable(field: &Field, arrow_metadata: &HashMap<String, String>) -> bool {
    field.metadata.matches_arrow(arrow_metadata)
        && field.dtype.arrow_import_is_projection_equivalent()
}

/// The core identity an Arrow field's extension metadata declares, when it
/// declares one of the first-class extension-typed datatypes.
pub(crate) enum RecognizedExtension {
    /// The canonical `arrow.parquet.variant` over its exact storage struct.
    Variant,
    /// The community `geoarrow.wkb` over Binary storage; the parsed GeoArrow
    /// document says whether it is a geometry or a geography.
    Geospatial(GeospatialParameters),
    /// The `yggdryl.ascii` extension: `Binary` for the variable form, and
    /// `FixedSizeBinary(n)` for the width the storage names.
    Ascii(DataType),
    /// A code's own `yggdryl.{country,currency,mic,cfi}` over the
    /// `FixedSizeBinary` width that code fixes.
    ///
    /// It is separate from [`Self::Ascii`] because the identity is the point:
    /// three bytes under `yggdryl.currency` are a currency and three bytes
    /// under `yggdryl.ascii` are an `ascii(3)`, and neither imports as the
    /// other.
    Code(DataType),
    /// The canonical `arrow.uuid` over `FixedSizeBinary(16)`.
    Uuid,
    /// The canonical version text over Utf8.
    Version,
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
            Self::Ascii(dtype) | Self::Code(dtype) => dtype,
            Self::Uuid => DataType::Uuid,
            Self::Version => DataType::Version,
        }
    }
}

/// Recognizes the Arrow extension spellings the first-class datatypes ride:
/// `geoarrow.wkb` over Binary storage, the canonical `arrow.parquet.variant`
/// over its exact storage struct with an empty extension metadata document,
/// `yggdryl.ascii` over `FixedSizeBinary(2 | 3 | 4 | 8 | 12 | 16)`, each
/// registered code's own `yggdryl.{country,currency,mic,cfi}` over the width
/// that code fixes, and the canonical `arrow.uuid` over
/// `FixedSizeBinary(16)`, each with an empty or absent document.
///
/// Any other pairing keeps today's behavior exactly - a foreign extension
/// name, one of ours over a storage it does not spell, a variant or an ASCII
/// width with a non-empty document: the field imports as its storage type
/// with the `ARROW:extension:*` keys as plain metadata.
///
/// # Errors
///
/// Returns an error when a recognized `geoarrow.wkb` field carries a GeoArrow
/// metadata document that does not parse.
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
        // The storage shape alone tells the two ASCII datatypes apart: the
        // variable form is Arrow's `Binary`, and a width is that width's
        // `FixedSizeBinary`.
        ASCII_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
            let (values, key) = encoded_values(storage)?;
            Ok(match values {
                ArrowDataType::Binary => Some(RecognizedExtension::Ascii(re_encoded(
                    DataType::Ascii,
                    key,
                )?)),
                ArrowDataType::FixedSizeBinary(width) => Some(RecognizedExtension::Ascii(
                    re_encoded(DataType::ascii(*width)?, key)?,
                )),
                _ => None,
            })
        }
        UUID_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
            Ok(matches!(storage, ArrowDataType::FixedSizeBinary(16))
                .then_some(RecognizedExtension::Uuid))
        }
        VERSION_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
            Ok(matches!(storage, ArrowDataType::Utf8).then_some(RecognizedExtension::Version))
        }
        code if document.unwrap_or("").is_empty() => {
            let (values, key) = encoded_values(storage)?;
            Ok(match values {
                ArrowDataType::FixedSizeBinary(width) => match code_for_extension(code, *width) {
                    Some(dtype) => Some(RecognizedExtension::Code(re_encoded(dtype, key)?)),
                    None => None,
                },
                _ => None,
            })
        }
        _ => Ok(None),
    }
}

/// The storage one extension describes, and the dictionary key it sits under.
///
/// Arrow's `Dictionary` carries a bare datatype for its values rather than a
/// field, so a dictionary-encoded extension column has nowhere but the field
/// itself to declare its identity. Peeling here is what lets a caller's own
/// `dictionary(int32, currency)` import as itself rather than as anonymous
/// bytes; no datatype here *is* a dictionary - a code is its own fixed
/// binary - so this is only about not losing what a caller composed.
///
/// # Errors
///
/// Returns an error when the dictionary key is not an Arrow type this crate
/// imports.
fn encoded_values(storage: &ArrowDataType) -> Result<(&ArrowDataType, Option<DataType>)> {
    match storage {
        ArrowDataType::Dictionary(key, value) => {
            Ok((value.as_ref(), Some(DataType::from_arrow(key.as_ref())?)))
        }
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
        return Ok((recognized.into_dtype(), Metadata::from_arrow(&stripped)?));
    }
    let metadata = Metadata::from_arrow(value.metadata())?;
    let dtype = DataType::from_arrow_at_depth(value.data_type(), depth)?;
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
    let Some((name, document)) = arrow_extension_parts(dtype) else {
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
    Field {
        name: value.name().into(),
        dtype,
        nullable: value.is_nullable(),
        dictionary_id,
        dictionary_is_ordered: value.dict_is_ordered().unwrap_or_default(),
        metadata,
        arrow: OnceLock::new(),
    }
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
        value.clone().into_arrow()
    }
}

impl TryFrom<Field> for ArrowField {
    type Error = Error;

    fn try_from(value: Field) -> Result<Self> {
        value.into_arrow()
    }
}

impl TryFrom<&ArrowField> for Field {
    type Error = Error;

    fn try_from(value: &ArrowField) -> Result<Self> {
        Self::from_arrow(value)
    }
}

impl TryFrom<ArrowField> for Field {
    type Error = Error;

    fn try_from(value: ArrowField) -> Result<Self> {
        Self::from_arrow_owned_at_depth(value, 0)
    }
}

/// One root's declarations, compiled once against one source schema.
///
/// A reader applies the same three steps to every batch, and all three answer
/// from the schemas alone: which columns the cast reconciles, which columns
/// each protocol declares, and what the applied shape therefore is. Compiling
/// that once is what lets a stream pay for it once.
#[cfg(feature = "arrow")]
pub(crate) struct AppliedPlan {
    root: Field,
    cast: Option<crate::types::cast::ArrowCastPlan>,
    partition: bool,
    digest: bool,
    /// The strict re-check over the finished batch, compiled only when a
    /// protocol was allowed to leave a hole for itself, or when no cast ran.
    verify: Option<crate::types::cast::ArrowCastPlan>,
    /// The schema every applied batch carries.
    schema: arrow_schema::SchemaRef,
}

#[cfg(feature = "arrow")]
impl AppliedPlan {
    /// Compiles the three steps and derives the applied schema, without rows.
    pub(crate) fn compile(
        root: &Field,
        source: arrow_schema::SchemaRef,
        digest: bool,
        partition: bool,
        cast: bool,
        options: crate::ArrowCastOptions,
    ) -> Result<Self> {
        use crate::types::cast::{ArrowCastPlan, Deferred};

        let cast = if cast {
            Some(ArrowCastPlan::compile_deferring(
                &source,
                root,
                options,
                Deferred { partition, digest },
            )?)
        } else {
            None
        };
        // The applied shape is a property of the two schemas, so it is read off
        // an empty batch: nothing is decoded, and a declaration that cannot be
        // satisfied fails here rather than on the first batch.
        let empty = arrow_array::RecordBatch::new_empty(source);
        let applied = Self::stages(root, cast.as_ref(), partition, digest, &empty)?;
        let schema = applied.schema();
        // A cast with no protocol behind it already refused every hole, so the
        // re-check exists only where something could still have left one.
        let verify = if options.nullability().is_strict() && (partition || digest || cast.is_none())
        {
            Some(ArrowCastPlan::compile(schema.as_ref(), root, options)?)
        } else {
            None
        };
        Ok(Self {
            root: root.clone(),
            cast,
            partition,
            digest,
            verify,
            schema,
        })
    }

    /// Run cast, then partition, then digest - the order their answers depend
    /// on, and the order this crate publishes.
    fn stages(
        root: &Field,
        cast: Option<&crate::types::cast::ArrowCastPlan>,
        partition: bool,
        digest: bool,
        batch: &arrow_array::RecordBatch,
    ) -> Result<arrow_array::RecordBatch> {
        let mut applied = match cast {
            Some(plan) => plan.apply(batch.clone())?,
            None => batch.clone(),
        };
        if partition {
            applied = root.as_partition().apply_arrow_batch(&applied)?;
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
            self.partition,
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
#[cfg(feature = "arrow")]
struct AppliedReader {
    inner: crate::arrow::BatchReader,
    plan: AppliedPlan,
}

#[cfg(feature = "arrow")]
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

#[cfg(feature = "arrow")]
impl arrow_array::RecordBatchReader for AppliedReader {
    fn schema(&self) -> arrow_schema::SchemaRef {
        Arc::clone(&self.plan.schema)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    use crate::{DataType, EdgeAlgorithm, Field};

    use super::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY};

    fn variant_storage() -> ArrowDataType {
        ArrowDataType::Struct(arrow_schema::Fields::from(vec![
            ArrowField::new("metadata", ArrowDataType::Binary, false),
            ArrowField::new("value", ArrowDataType::Binary, false),
        ]))
    }

    #[test]
    fn a_geometry_field_projects_the_geoarrow_extension_and_reimports_itself() {
        let field = Field::new("shape", DataType::geometry(None).unwrap(), true);
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(arrow.data_type(), &ArrowDataType::Binary);
        assert_eq!(arrow.extension_type_name(), Some("geoarrow.wkb"));
        assert_eq!(
            arrow.extension_type_metadata(),
            Some(r#"{"crs":"OGC:CRS84"}"#)
        );

        let imported = Field::from_arrow(&arrow).unwrap();
        assert_eq!(imported, field);
        // The extension keys are transport: they never reach Field metadata.
        assert!(imported.as_metadata().is_empty());
    }

    #[test]
    fn a_geography_projection_carries_the_edge_algorithm_and_round_trips() {
        let field = Field::new(
            "region",
            DataType::geography(Some("EPSG:4326"), Some(EdgeAlgorithm::Vincenty)).unwrap(),
            false,
        );
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(
            arrow.extension_type_metadata(),
            Some(r#"{"crs":"EPSG:4326","edges":"vincenty"}"#)
        );
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field);
    }

    #[test]
    fn a_variant_field_projects_the_canonical_struct_and_reimports_itself() {
        let field = Field::new("payload", DataType::variant(), true);
        let arrow = field.clone().into_arrow().unwrap();
        assert_eq!(arrow.data_type(), &variant_storage());
        assert_eq!(arrow.extension_type_name(), Some("arrow.parquet.variant"));
        assert_eq!(arrow.extension_type_metadata(), Some(""));

        let imported = Field::from_arrow(&arrow).unwrap();
        assert_eq!(imported.dtype(), &DataType::Variant);
        assert!(imported.as_metadata().is_empty());
        assert_eq!(imported, field);
    }

    #[test]
    fn a_bare_geoarrow_document_imports_as_the_default_geometry() {
        let arrow =
            ArrowField::new("shape", ArrowDataType::Binary, true).with_metadata(HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "geoarrow.wkb".to_owned(),
            )]));
        let imported = Field::from_arrow(&arrow).unwrap();
        assert_eq!(imported.dtype(), &DataType::geometry(None).unwrap());
        let shared = Field::from_arrow_ref(Arc::new(arrow.clone())).unwrap();
        assert_eq!(shared, imported);
        let owned = Field::try_from(arrow).unwrap();
        assert_eq!(owned, imported);
    }

    #[test]
    fn an_unknown_extension_name_keeps_todays_import_exactly() {
        let arrow =
            ArrowField::new("raw", ArrowDataType::Binary, true).with_metadata(HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "someorg.blob".to_owned(),
                ),
                (EXTENSION_TYPE_METADATA_KEY.to_owned(), "{}".to_owned()),
            ]));
        let imported = Field::from_arrow(&arrow).unwrap();
        assert_eq!(imported.dtype(), &DataType::Binary);
        assert_eq!(
            imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
            Some("someorg.blob")
        );
        assert_eq!(imported.into_arrow().unwrap(), arrow);
    }

    #[test]
    fn our_extension_name_over_a_foreign_storage_keeps_todays_import() {
        let arrow = ArrowField::new("shape", ArrowDataType::LargeBinary, true).with_metadata(
            HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "geoarrow.wkb".to_owned(),
            )]),
        );
        let imported = Field::from_arrow(&arrow).unwrap();
        assert_eq!(imported.dtype(), &DataType::LargeBinary);
        assert_eq!(
            imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
            Some("geoarrow.wkb")
        );
    }

    #[test]
    fn a_malformed_geoarrow_document_is_refused_naming_the_key() {
        let arrow =
            ArrowField::new("shape", ArrowDataType::Binary, true).with_metadata(HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "geoarrow.wkb".to_owned(),
                ),
                (
                    EXTENSION_TYPE_METADATA_KEY.to_owned(),
                    r#"{"crs":7}"#.to_owned(),
                ),
            ]));
        let refused = Field::from_arrow(&arrow).unwrap_err().to_string();
        assert!(refused.contains(EXTENSION_TYPE_METADATA_KEY), "{refused}");
        assert!(refused.contains("crs"), "{refused}");
    }

    #[test]
    fn a_caller_set_extension_key_on_an_extension_typed_field_is_refused_naming_both() {
        let field = Field::from_parts(
            "shape",
            DataType::geometry(None).unwrap(),
            false,
            [(EXTENSION_TYPE_NAME_KEY, "someorg.other")],
        )
        .unwrap();
        let refused = field.into_arrow().unwrap_err().to_string();
        assert!(refused.contains("someorg.other"), "{refused}");
        assert!(refused.contains("geoarrow.wkb"), "{refused}");

        let variant = Field::from_parts(
            "payload",
            DataType::variant(),
            true,
            [(EXTENSION_TYPE_METADATA_KEY, "shredded")],
        )
        .unwrap();
        let refused = variant.into_arrow().unwrap_err().to_string();
        assert!(refused.contains("shredded"), "{refused}");
        assert!(refused.contains("arrow.parquet.variant"), "{refused}");
    }

    #[test]
    fn a_variant_with_a_nonempty_document_or_foreign_shape_keeps_todays_import() {
        let shredded =
            ArrowField::new("payload", variant_storage(), true).with_metadata(HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "arrow.parquet.variant".to_owned(),
                ),
                (
                    EXTENSION_TYPE_METADATA_KEY.to_owned(),
                    "shredded".to_owned(),
                ),
            ]));
        let imported = Field::from_arrow(&shredded).unwrap();
        assert!(matches!(imported.dtype(), DataType::Struct(_)));

        let swapped = ArrowDataType::Struct(arrow_schema::Fields::from(vec![
            ArrowField::new("value", ArrowDataType::Binary, false),
            ArrowField::new("metadata", ArrowDataType::Binary, false),
        ]));
        let swapped = ArrowField::new("payload", swapped, true).with_metadata(HashMap::from([(
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            "arrow.parquet.variant".to_owned(),
        )]));
        let imported = Field::from_arrow(&swapped).unwrap();
        assert!(matches!(imported.dtype(), DataType::Struct(_)));
    }

    #[test]
    fn an_ascii_field_projects_the_yggdryl_extension_and_reimports_itself() {
        // The storage shape carries the whole identity: `Binary` is the
        // variable form and every `FixedSizeBinary(n)` is that width, so no
        // width is special and none is excluded.
        for (dtype, storage) in [
            (DataType::Ascii, ArrowDataType::Binary),
            (DataType::FixedAscii(1), ArrowDataType::FixedSizeBinary(1)),
            (DataType::FixedAscii(3), ArrowDataType::FixedSizeBinary(3)),
            (DataType::FixedAscii(5), ArrowDataType::FixedSizeBinary(5)),
            (DataType::FixedAscii(16), ArrowDataType::FixedSizeBinary(16)),
            (DataType::FixedAscii(64), ArrowDataType::FixedSizeBinary(64)),
        ] {
            let field = Field::new("ccy", dtype, false);
            let arrow = field.clone().into_arrow().unwrap();
            assert_eq!(arrow.data_type(), &storage);
            assert_eq!(arrow.extension_type_name(), Some("yggdryl.ascii"));
            assert_eq!(arrow.extension_type_metadata(), Some(""));

            let imported = Field::from_arrow(&arrow).unwrap();
            assert_eq!(imported, field);
            assert!(imported.as_metadata().is_empty());
        }
    }

    #[test]
    fn an_ascii_extension_over_other_storage_or_a_document_keeps_todays_import() {
        // The extension names a value rule over one of two storage shapes.
        // Any other storage is not that rule, so the name stays metadata.
        let large = ArrowField::new("ccy", ArrowDataType::LargeBinary, true).with_metadata(
            HashMap::from([(
                EXTENSION_TYPE_NAME_KEY.to_owned(),
                "yggdryl.ascii".to_owned(),
            )]),
        );
        let imported = Field::from_arrow(&large).unwrap();
        assert_eq!(imported.dtype(), &DataType::LargeBinary);
        assert_eq!(
            imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
            Some("yggdryl.ascii")
        );

        let documented = ArrowField::new("ccy", ArrowDataType::FixedSizeBinary(4), true)
            .with_metadata(HashMap::from([
                (
                    EXTENSION_TYPE_NAME_KEY.to_owned(),
                    "yggdryl.ascii".to_owned(),
                ),
                (EXTENSION_TYPE_METADATA_KEY.to_owned(), "{}".to_owned()),
            ]));
        let imported = Field::from_arrow(&documented).unwrap();
        assert_eq!(imported.dtype(), &DataType::FixedSizeBinary(4));
        assert_eq!(imported.into_arrow().unwrap(), documented);
    }
}
