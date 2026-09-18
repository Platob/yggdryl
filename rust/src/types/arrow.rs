//! Lossless Arrow datatype interoperability.

use std::sync::Arc;

use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef, Fields as ArrowFields, UnionFields as ArrowUnionFields, ffi::{FFI_ArrowSchema, Flags}};
pub(crate) use field::arrow_field_to_ffi;
pub(crate) use field::{RecognizedExtension, recognized_arrow_extension};
use smol_str::{SmolStr, format_smolstr};

use crate::types::structure::Fields;
use crate::types::union::UnionFields;
use crate::{Error, Field, Result};
use super::bytes::BYTES_EXTENSION_NAME;
use super::code::code_extension_name;
use super::decimal::validate_decimal;
use super::geospatial::{GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME};
use super::media_type::MEDIATYPE_EXTENSION_NAME;
use super::mime_type::MIMETYPE_EXTENSION_NAME;
use super::nested::validate_dictionary_key;
use super::nested::validate_map_entries;
use super::nested::validate_run_ends;
use super::string::{STRING_EXTENSION_NAME, needs_extension};
use super::temporal::{validate_duration_unit, validate_time32_unit, validate_time64_unit};
use super::timezone::TIMEZONE_EXTENSION_NAME;
use super::url::URL_EXTENSION_NAME;
use super::uuid::UUID_EXTENSION_NAME;
use super::version::VERSION_EXTENSION_NAME;
use super::{DataType, UnionMode, invalid, validate_non_negative};
use crate::types::sequence::SequenceType;

/// Arrow field import, cached projection, and conversion traits.
mod field {
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
        BYTES_EXTENSION_NAME, BytesParameters, GEOARROW_WKB_EXTENSION_NAME, MEDIATYPE_EXTENSION_NAME,
        MIMETYPE_EXTENSION_NAME, STRING_EXTENSION_NAME, StringParameters, TIMEZONE_EXTENSION_NAME,
        URL_EXTENSION_NAME, UUID_EXTENSION_NAME, VARIANT_EXTENSION_NAME, VERSION_EXTENSION_NAME,
        arrow_dtype_to_ffi, arrow_extension_parts, code_for_extension, is_variant_storage,
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
        #[cfg(feature = "arrow")]
        pub fn apply_arrow_batch(
            &self,
            batch: &arrow_array::RecordBatch,
            digest: bool,
            transform: bool,
            cast: bool,
            options: crate::ArrowCastOptions,
        ) -> Result<arrow_array::RecordBatch> {
            AppliedPlan::compile(self, batch.schema(), digest, transform, cast, options)?.apply(batch)
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
        #[cfg(feature = "arrow")]
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
            let plan = AppliedPlan::compile(self, inner.schema(), digest, transform, cast, options)?;
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

        /// Borrows this field's Arrow projection, building it once.
        ///
        /// [`Self::into_arrow_ref`] consumes the field, so a caller that still
        /// needs it has to clone first and pays for the projection every time.
        /// This borrows, and fills the same cache an Arrow import seeds and a
        /// clone shares - so a field exported more than once is projected once.
        /// Any effective change clears the cache, exactly as it does today.
        ///
        /// # Errors
        ///
        /// Returns an error when the datatype or its metadata has no valid Arrow
        /// projection.
        pub fn as_arrow_ref(&self) -> Result<&FieldRef> {
            if let Some(field) = self.arrow.get() {
                return Ok(field);
            }
            let projected = projected_arrow_metadata(&self.dtype, self.metadata.clone().into_arrow())?;
            let built = Arc::new(arrow_field_from_parts(
                self.name.as_str(),
                self.dtype.clone().into_arrow()?,
                self.nullable,
                self.dictionary_id,
                self.dictionary_is_ordered,
                projected,
            ));
            let _ = self.arrow.set(built);
            Ok(self
                .arrow
                .get()
                .expect("the projection was just placed in the cache"))
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
        /// The canonical `arrow.uuid` over `FixedSizeBinary(16)`.
        Uuid,
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
                Self::Uuid => DataType::Uuid,
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
                let parameters = StringParameters::from_extension_json(document).map_err(|error| {
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
                let parameters = BytesParameters::from_extension_json(document).map_err(|error| {
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
                    .then_some(RecognizedExtension::Uuid))
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
            MEDIATYPE_EXTENSION_NAME if document.unwrap_or("").is_empty() => {
                Ok(matches!(storage, ArrowDataType::Utf8).then_some(RecognizedExtension::MediaType))
            }
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
            // The extension describes the values; the encoding it was found under
            // is the caller's, and goes back on.
            let (_, key) = encoded_values(value.data_type())?;
            return Ok((
                re_encoded(recognized.into_dtype(), key)?,
                Metadata::from_arrow(&stripped)?,
            ));
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
        transform: bool,
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
            let verify = if options.nullability().is_strict() && (transform || digest || cast.is_none())
            {
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
}

impl DataType {
    /// Projects this Struct datatype as an Arrow schema.
    ///
    /// A schema is the columns of a struct, so this is the same projection a
    /// non-null Struct [`crate::Field`] makes, without a name or metadata.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a bounded Struct datatype.
    /// Consumes this Struct datatype and projects it as an Arrow schema.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a bounded Struct datatype.
    #[cfg(feature = "arrow")]
    pub fn into_arrow_schema(self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
        crate::Field::new("row", self, false).into_arrow_schema()
    }

    /// Materializes [`DataType::default_value`] as an exact one-row array.
    ///
    /// The bounded core default planner selects the value, so
    /// [`DataType::Null`] and transparent logical wrappers with a null-only
    /// canonical default materialize as logical null; every other datatype
    /// materializes its present zero/empty default.
    ///
    /// # Errors
    ///
    /// Returns an error when no physically valid default exists or Arrow
    /// cannot materialize the datatype.
    #[cfg(feature = "arrow")]
    pub fn default_arrow_array(&self) -> crate::arrow::Result<arrow_array::ArrayRef> {
        crate::arrow::default_dtype_scalar_array(self)
    }

    /// Imports an Arrow datatype and validates every nested invariant.
    ///
    /// An Arrow datatype carries no metadata, so an extension type arrives as
    /// the storage it is written over: `fixed_size_binary(3)` and not
    /// `currency`, `binary` and not `geometry`. The identity lives on the
    /// field - [`Field::from_arrow`](crate::Field::from_arrow) reads it, and
    /// [`Self::into_arrow_ffi`] projects a node that carries it - so a schema
    /// round trip keeps every first-class datatype and only this bare pair
    /// answers storage.
    pub fn from_arrow(value: &ArrowDataType) -> Result<Self> {
        Self::from_arrow_at_depth(value, 0)
    }

    /// Imports Arrow state at an existing datatype nesting depth.
    ///
    /// Field import paths use this entry point to preserve one shared depth
    /// budget across alternating Arrow datatype and field nodes.
    pub(crate) fn from_arrow_at_depth(value: &ArrowDataType, depth: usize) -> Result<Self> {
        check_arrow_import_depth(depth)?;
        let child_depth = depth + 1;
        use ArrowDataType as A;
        Ok(match value {
            A::Null => Self::Null,
            A::Boolean => Self::Boolean,
            A::Int8 => Self::Int8,
            A::Int16 => Self::Int16,
            A::Int32 => Self::Int32,
            A::Int64 => Self::Int64,
            A::UInt8 => Self::UInt8,
            A::UInt16 => Self::UInt16,
            A::UInt32 => Self::UInt32,
            A::UInt64 => Self::UInt64,
            A::Float16 => Self::Float16,
            A::Float32 => Self::Float32,
            A::Float64 => Self::Float64,
            A::Timestamp(unit, timezone) => Self::datetime64(
                (*unit).into(),
                timezone
                    .as_ref()
                    .map_or(Ok(crate::Timezone::NAIVE), |value| {
                        crate::Timezone::from_smol_str(SmolStr::from(Arc::clone(value)))
                    })?,
            )?,
            A::Date32 => Self::Date32,
            A::Date64 => Self::Date64,
            A::Time32(unit) => Self::time32((*unit).into())?,
            A::Time64(unit) => Self::time64((*unit).into())?,
            A::Duration(unit) => Self::duration64((*unit).into())?,
            A::Interval(unit) => Self::Interval((*unit).into()),
            A::Binary => Self::binary(),
            A::FixedSizeBinary(width) => Self::fixed_size_binary(arrow_fixed_width(*width)?)?,
            A::LargeBinary => Self::large_binary(),
            A::BinaryView => Self::binary_view(),
            A::Utf8 => Self::utf8(),
            A::LargeUtf8 => Self::large_utf8(),
            A::Utf8View => Self::utf8_view(),
            A::List(field) => Self::list(Field::from_arrow_ref_at_depth(
                Arc::clone(field),
                child_depth,
            )?),
            A::ListView(field) => Self::list_view(Field::from_arrow_ref_at_depth(
                Arc::clone(field),
                child_depth,
            )?),
            A::FixedSizeList(field, length) => Self::fixed_size_list(
                Field::from_arrow_ref_at_depth(Arc::clone(field), child_depth)?,
                *length,
            )?,
            A::LargeList(field) => Self::large_list(Field::from_arrow_ref_at_depth(
                Arc::clone(field),
                child_depth,
            )?),
            A::LargeListView(field) => Self::large_list_view(
                Field::from_arrow_ref_at_depth(Arc::clone(field), child_depth)?,
            ),
            A::Struct(fields) => {
                Self::Structure(from_arrow_fields_at_depth(fields, child_depth)?.into())
            }
            A::Union(fields, mode) => {
                let values = fields
                    .iter()
                    .map(|(type_id, field)| {
                        Ok((
                            type_id,
                            Field::from_arrow_ref_at_depth(Arc::clone(field), child_depth)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Self::Union(UnionFields::from_imported_fields(values)?, (*mode).into())
            }
            A::Dictionary(key, value) => Self::dictionary(
                Self::from_arrow_at_depth(key, child_depth)?,
                Self::from_arrow_at_depth(value, child_depth)?,
            )?,
            A::Decimal32(precision, scale) => Self::decimal32(*precision, *scale)?,
            A::Decimal64(precision, scale) => Self::decimal64(*precision, *scale)?,
            A::Decimal128(precision, scale) => Self::decimal128(*precision, *scale)?,
            A::Decimal256(precision, scale) => Self::decimal256(*precision, *scale)?,
            A::Map(entries, keys_sorted) => Self::map(
                Field::from_arrow_ref_at_depth(Arc::clone(entries), child_depth)?,
                *keys_sorted,
            )?,
            A::RunEndEncoded(run_ends, values) => Self::run_end_encoded(
                Field::from_arrow_ref_at_depth(Arc::clone(run_ends), child_depth)?,
                Field::from_arrow_ref_at_depth(Arc::clone(values), child_depth)?,
            )?,
        })
    }

    fn from_arrow_owned_at_depth(value: ArrowDataType, depth: usize) -> Result<Self> {
        check_arrow_import_depth(depth)?;
        let child_depth = depth + 1;
        use ArrowDataType as A;
        Ok(match value {
            A::Null => Self::Null,
            A::Boolean => Self::Boolean,
            A::Int8 => Self::Int8,
            A::Int16 => Self::Int16,
            A::Int32 => Self::Int32,
            A::Int64 => Self::Int64,
            A::UInt8 => Self::UInt8,
            A::UInt16 => Self::UInt16,
            A::UInt32 => Self::UInt32,
            A::UInt64 => Self::UInt64,
            A::Float16 => Self::Float16,
            A::Float32 => Self::Float32,
            A::Float64 => Self::Float64,
            A::Timestamp(unit, timezone) => Self::datetime64(
                unit.into(),
                timezone.map_or(Ok(crate::Timezone::NAIVE), |value| {
                    crate::Timezone::from_smol_str(SmolStr::from(value))
                })?,
            )?,
            A::Date32 => Self::Date32,
            A::Date64 => Self::Date64,
            A::Time32(unit) => Self::time32(unit.into())?,
            A::Time64(unit) => Self::time64(unit.into())?,
            A::Duration(unit) => Self::duration64(unit.into())?,
            A::Interval(unit) => Self::Interval(unit.into()),
            A::Binary => Self::binary(),
            A::FixedSizeBinary(width) => Self::fixed_size_binary(arrow_fixed_width(width)?)?,
            A::LargeBinary => Self::large_binary(),
            A::BinaryView => Self::binary_view(),
            A::Utf8 => Self::utf8(),
            A::LargeUtf8 => Self::large_utf8(),
            A::Utf8View => Self::utf8_view(),
            A::List(field) => Self::list(Field::from_arrow_ref_at_depth(field, child_depth)?),
            A::ListView(field) => {
                Self::list_view(Field::from_arrow_ref_at_depth(field, child_depth)?)
            }
            A::FixedSizeList(field, length) => {
                Self::fixed_size_list(Field::from_arrow_ref_at_depth(field, child_depth)?, length)?
            }
            A::LargeList(field) => {
                Self::large_list(Field::from_arrow_ref_at_depth(field, child_depth)?)
            }
            A::LargeListView(field) => {
                Self::large_list_view(Field::from_arrow_ref_at_depth(field, child_depth)?)
            }
            A::Struct(fields) => {
                // Arrow's shared field slice has no consuming iterator. Its
                // `FieldRef`s are shallow-cloned and remain the exact cached
                // projections held by the incoming Arrow datatype.
                let values = fields
                    .iter()
                    .cloned()
                    .map(|field| Field::from_arrow_ref_at_depth(field, child_depth))
                    .collect::<Result<Vec<_>>>()?;
                Self::Structure(Fields::from_imported_fields(values)?.into())
            }
            A::Union(fields, mode) => {
                let values = fields
                    .iter()
                    .map(|(type_id, field)| {
                        Ok((
                            type_id,
                            Field::from_arrow_ref_at_depth(Arc::clone(field), child_depth)?,
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                Self::Union(UnionFields::from_imported_fields(values)?, mode.into())
            }
            A::Dictionary(key, value) => Self::dictionary(
                Self::from_arrow_owned_at_depth(*key, child_depth)?,
                Self::from_arrow_owned_at_depth(*value, child_depth)?,
            )?,
            A::Decimal32(precision, scale) => Self::decimal32(precision, scale)?,
            A::Decimal64(precision, scale) => Self::decimal64(precision, scale)?,
            A::Decimal128(precision, scale) => Self::decimal128(precision, scale)?,
            A::Decimal256(precision, scale) => Self::decimal256(precision, scale)?,
            A::Map(entries, keys_sorted) => Self::map(
                Field::from_arrow_ref_at_depth(entries, child_depth)?,
                keys_sorted,
            )?,
            A::RunEndEncoded(run_ends, values) => Self::run_end_encoded(
                Field::from_arrow_ref_at_depth(run_ends, child_depth)?,
                Field::from_arrow_ref_at_depth(values, child_depth)?,
            )?,
        })
    }

    /// Projects this datatype to an owned Arrow C Data Interface schema.
    ///
    /// This uses the same validated Arrow projection as [`Self::into_arrow`]
    /// and preserves datatype flags recursively, including sorted map keys.
    /// Arrow 59's generic Field-to-C-schema conversion overwrites those flags
    /// when adding field flags, so Yggdryl owns the corrected recursive path.
    ///
    /// A C schema is a field node, so unlike [`Self::into_arrow`] this keeps
    /// an extension identity - a code, a UUID, a version, a variant, a
    /// geospatial parameter set - including under a dictionary encoding,
    /// where the entries belong to the outer node.
    pub fn into_arrow_ffi(self) -> Result<FFI_ArrowSchema> {
        native_dtype_to_ffi(&self)
    }

    /// Consumes this value and returns its Arrow representation.
    ///
    /// Scalar conversion is allocation-free. Shared nested fields reuse their
    /// Arrow projections; uniquely owned child state can be consumed by
    /// [`Field::into_arrow_ref`].
    ///
    /// An extension type projects as its storage: an Arrow datatype has
    /// nowhere to carry the `ARROW:extension:*` entries, which is what
    /// [`Self::into_arrow_ffi`] and the field projections do carry.
    pub fn into_arrow(self) -> Result<ArrowDataType> {
        ArrowDataType::try_from(self)
    }

    /// Reports whether an imported datatype can reuse its enclosing Arrow field.
    ///
    /// Scalar and parameter-only variants import without canonicalization.
    /// Nested fields retain their incoming projection only when their complete
    /// subtree is equivalent, so inspecting each direct child propagates that
    /// result through the tree in one pass without allocating an Arrow copy.
    pub(crate) fn arrow_import_is_projection_equivalent(&self) -> bool {
        match self {
            Self::Sequence(sequence) => {
                let field = sequence.item();
                field.arrow_import_is_projection_equivalent()
            }
            Self::Structure(fields) => fields
                .iter()
                .all(Field::arrow_import_is_projection_equivalent),
            Self::Union(fields, _) => fields
                .iter()
                .all(|(_, field)| field.arrow_import_is_projection_equivalent()),
            Self::Dictionary(dictionary) => {
                dictionary.key().arrow_import_is_projection_equivalent()
                    && dictionary.value().arrow_import_is_projection_equivalent()
            }
            Self::Mapping(mapping) => mapping.entries().arrow_import_is_projection_equivalent(),
            Self::RunEndEncoded(encoded) => {
                encoded.run_ends().arrow_import_is_projection_equivalent()
                    && encoded.values().arrow_import_is_projection_equivalent()
            }
            _ => true,
        }
    }
}

impl From<UnionMode> for arrow_schema::UnionMode {
    fn from(value: UnionMode) -> Self {
        match value {
            UnionMode::Sparse => Self::Sparse,
            UnionMode::Dense => Self::Dense,
        }
    }
}

impl From<arrow_schema::UnionMode> for UnionMode {
    fn from(value: arrow_schema::UnionMode) -> Self {
        match value {
            arrow_schema::UnionMode::Sparse => Self::Sparse,
            arrow_schema::UnionMode::Dense => Self::Dense,
        }
    }
}

impl TryFrom<&DataType> for ArrowDataType {
    type Error = Error;

    #[allow(clippy::too_many_lines)]
    fn try_from(value: &DataType) -> Result<Self> {
        use DataType as R;
        Ok(match value {
            R::Null => Self::Null,
            R::Boolean => Self::Boolean,
            R::Int8 => Self::Int8,
            R::Int16 => Self::Int16,
            R::Int32 => Self::Int32,
            R::Int64 => Self::Int64,
            R::UInt8 => Self::UInt8,
            R::UInt16 => Self::UInt16,
            R::UInt32 => Self::UInt32,
            R::UInt64 => Self::UInt64,
            R::Float16 => Self::Float16,
            R::Float32 => Self::Float32,
            R::Float64 => Self::Float64,
            R::DateTime64 { unit, timezone } => Self::Timestamp(
                unit.into_arrow_time()?,
                (!timezone.is_naive()).then(|| Arc::<str>::from(timezone.as_smol_str().clone())),
            ),
            R::Date32 => Self::Date32,
            R::Date64 => Self::Date64,
            R::Time32(unit) => {
                validate_time32_unit(*unit)?;
                Self::Time32(unit.into_arrow_time()?)
            }
            R::Time64(unit) => {
                validate_time64_unit(*unit)?;
                Self::Time64(unit.into_arrow_time()?)
            }
            R::Duration32(unit) => {
                validate_duration_unit("Duration32", *unit)?;
                Self::Duration(unit.into_arrow_time()?)
            }
            R::Duration64(unit) => {
                validate_duration_unit("Duration64", *unit)?;
                Self::Duration(unit.into_arrow_time()?)
            }
            R::Interval(unit) => Self::Interval(unit.into_arrow_interval()?),
            R::Bytes(parameters) => super::bytes::arrow_storage(*parameters)?,
            R::String(parameters) => super::string::arrow_storage(*parameters)?,
            // A code is the ASCII text it is, so it rides Arrow's own text
            // layout and the extension name beside it carries the identity.
            R::Country
            | R::Currency
            | R::Mic
            | R::Cfi
            | R::Isin
            | R::Cusip
            | R::Sedol
            | R::Bloomberg
            | R::Side
            | R::State
            | R::TimeInForce
            | R::Version
            | R::Url
            | R::Timezone
            | R::MimeType
            | R::MediaType => Self::Utf8,
            R::Uuid => Self::FixedSizeBinary(16),
            R::Sequence(SequenceType::List(field)) => {
                Self::List(field.as_ref().clone().into_arrow_ref()?)
            }
            R::Sequence(SequenceType::ListView(field)) => {
                Self::ListView(field.as_ref().clone().into_arrow_ref()?)
            }
            R::Sequence(SequenceType::FixedSizeList(field, length)) => {
                validate_non_negative("FixedSizeList", "length", *length)?;
                Self::FixedSizeList(field.as_ref().clone().into_arrow_ref()?, *length)
            }
            R::Sequence(SequenceType::LargeList(field)) => {
                Self::LargeList(field.as_ref().clone().into_arrow_ref()?)
            }
            R::Sequence(SequenceType::LargeListView(field)) => {
                Self::LargeListView(field.as_ref().clone().into_arrow_ref()?)
            }
            R::Structure(structure) => {
                Self::Struct(into_arrow_fields(&structure.clone().into_fields())?)
            }
            R::Union(fields, mode) => {
                let mut type_ids = Vec::with_capacity(fields.len());
                let mut arrow_fields = Vec::with_capacity(fields.len());
                for (type_id, field) in fields.iter() {
                    type_ids.push(type_id);
                    arrow_fields.push(field.clone().into_arrow_ref()?);
                }
                Self::Union(
                    ArrowUnionFields::try_new(type_ids, arrow_fields)?,
                    (*mode).into(),
                )
            }
            R::Dictionary(dictionary) => {
                validate_dictionary_key(&dictionary.key)?;
                Self::Dictionary(
                    Box::new((&dictionary.key).try_into()?),
                    Box::new((&dictionary.value).try_into()?),
                )
            }
            R::Decimal32 { precision, scale } => {
                validate_decimal("Decimal32", *precision, *scale, 9)?;
                Self::Decimal32(*precision, *scale)
            }
            R::Decimal64 { precision, scale } => {
                validate_decimal("Decimal64", *precision, *scale, 18)?;
                Self::Decimal64(*precision, *scale)
            }
            R::Decimal128 { precision, scale } => {
                validate_decimal("Decimal128", *precision, *scale, 38)?;
                Self::Decimal128(*precision, *scale)
            }
            R::Decimal256 { precision, scale } => {
                validate_decimal("Decimal256", *precision, *scale, 76)?;
                Self::Decimal256(*precision, *scale)
            }
            R::Mapping(mapping) => {
                validate_map_entries(mapping.entries())?;
                Self::Map(
                    mapping.entries().clone().into_arrow_ref()?,
                    mapping.keys_sorted(),
                )
            }
            R::RunEndEncoded(encoded) => {
                validate_run_ends(&encoded.run_ends)?;
                Self::RunEndEncoded(
                    encoded.run_ends.clone().into_arrow_ref()?,
                    encoded.values.clone().into_arrow_ref()?,
                )
            }
            // The Arrow storage of the extension-typed variants. The
            // extension name and metadata are *field* metadata
            // (`ARROW:extension:name`), so they ride `Field`'s projection;
            // this level answers the storage type Arrow actually lays out:
            // the canonical `arrow.parquet.variant` struct of two required
            // binaries, and WKB bytes for the geospatial pair. The codes lay
            // out as the text their arms above name.
            R::Variant => Self::Struct(arrow_schema::Fields::from(vec![
                arrow_schema::Field::new("metadata", Self::Binary, false),
                arrow_schema::Field::new("value", Self::Binary, false),
            ])),
            R::Geometry(_) | R::Geography(_) => Self::Binary,
        })
    }
}

impl TryFrom<DataType> for ArrowDataType {
    type Error = Error;

    #[allow(clippy::too_many_lines)]
    fn try_from(value: DataType) -> Result<Self> {
        use DataType as R;
        Ok(match value {
            R::Null => Self::Null,
            R::Boolean => Self::Boolean,
            R::Int8 => Self::Int8,
            R::Int16 => Self::Int16,
            R::Int32 => Self::Int32,
            R::Int64 => Self::Int64,
            R::UInt8 => Self::UInt8,
            R::UInt16 => Self::UInt16,
            R::UInt32 => Self::UInt32,
            R::UInt64 => Self::UInt64,
            R::Float16 => Self::Float16,
            R::Float32 => Self::Float32,
            R::Float64 => Self::Float64,
            R::DateTime64 { unit, timezone } => Self::Timestamp(
                unit.into_arrow_time()?,
                (!timezone.is_naive()).then(|| Arc::<str>::from(timezone.into_smol_str())),
            ),
            R::Date32 => Self::Date32,
            R::Date64 => Self::Date64,
            R::Time32(unit) => {
                validate_time32_unit(unit)?;
                Self::Time32(unit.into_arrow_time()?)
            }
            R::Time64(unit) => {
                validate_time64_unit(unit)?;
                Self::Time64(unit.into_arrow_time()?)
            }
            R::Duration32(unit) => {
                validate_duration_unit("Duration32", unit)?;
                Self::Duration(unit.into_arrow_time()?)
            }
            R::Duration64(unit) => {
                validate_duration_unit("Duration64", unit)?;
                Self::Duration(unit.into_arrow_time()?)
            }
            R::Interval(unit) => Self::Interval(unit.into_arrow_interval()?),
            R::Bytes(parameters) => super::bytes::arrow_storage(parameters)?,
            R::String(parameters) => super::string::arrow_storage(parameters)?,
            // A code is the ASCII text it is, so it rides Arrow's own text
            // layout and the extension name beside it carries the identity.
            R::Country
            | R::Currency
            | R::Mic
            | R::Cfi
            | R::Isin
            | R::Cusip
            | R::Sedol
            | R::Bloomberg
            | R::Side
            | R::State
            | R::TimeInForce
            | R::Version
            | R::Url
            | R::Timezone
            | R::MimeType
            | R::MediaType => Self::Utf8,
            R::Uuid => Self::FixedSizeBinary(16),
            R::Sequence(SequenceType::List(field)) => Self::List(into_arrow_field(field)?),
            R::Sequence(SequenceType::ListView(field)) => {
                Self::ListView(into_arrow_field(field)?)
            }
            R::Sequence(SequenceType::FixedSizeList(field, length)) => {
                validate_non_negative("FixedSizeList", "length", length)?;
                Self::FixedSizeList(into_arrow_field(field)?, length)
            }
            R::Sequence(SequenceType::LargeList(field)) => {
                Self::LargeList(into_arrow_field(field)?)
            }
            R::Sequence(SequenceType::LargeListView(field)) => {
                Self::LargeListView(into_arrow_field(field)?)
            }
            R::Structure(fields) => {
                let fields = fields
                    .into_fields()
                    .into_iter()
                    .map(Field::into_arrow_ref)
                    .collect::<Result<Vec<_>>>()?;
                Self::Struct(fields.into())
            }
            R::Union(fields, mode) => {
                let mut type_ids = Vec::with_capacity(fields.len());
                let mut arrow_fields = Vec::with_capacity(fields.len());
                for (type_id, field) in fields.into_fields() {
                    type_ids.push(type_id);
                    arrow_fields.push(field.into_arrow_ref()?);
                }
                Self::Union(
                    ArrowUnionFields::try_new(type_ids, arrow_fields)?,
                    mode.into(),
                )
            }
            R::Dictionary(dictionary) => match Arc::try_unwrap(dictionary) {
                Ok(dictionary) => {
                    validate_dictionary_key(&dictionary.key)?;
                    Self::Dictionary(
                        Box::new(dictionary.key.into_arrow()?),
                        Box::new(dictionary.value.into_arrow()?),
                    )
                }
                Err(dictionary) => Self::Dictionary(
                    Box::new(dictionary.key.clone().into_arrow()?),
                    Box::new(dictionary.value.clone().into_arrow()?),
                ),
            },
            R::Decimal32 { precision, scale } => {
                validate_decimal("Decimal32", precision, scale, 9)?;
                Self::Decimal32(precision, scale)
            }
            R::Decimal64 { precision, scale } => {
                validate_decimal("Decimal64", precision, scale, 18)?;
                Self::Decimal64(precision, scale)
            }
            R::Decimal128 { precision, scale } => {
                validate_decimal("Decimal128", precision, scale, 38)?;
                Self::Decimal128(precision, scale)
            }
            R::Decimal256 { precision, scale } => {
                validate_decimal("Decimal256", precision, scale, 76)?;
                Self::Decimal256(precision, scale)
            }
            R::Mapping(mapping) => {
                let keys_sorted = mapping.keys_sorted();
                match Arc::try_unwrap(mapping.into_parameters()) {
                    Ok(parameters) => {
                        validate_map_entries(&parameters.entries)?;
                        Self::Map(parameters.entries.into_arrow_ref()?, keys_sorted)
                    }
                    Err(parameters) => {
                        validate_map_entries(&parameters.entries)?;
                        Self::Map(parameters.entries.clone().into_arrow_ref()?, keys_sorted)
                    }
                }
            }
            R::RunEndEncoded(encoded) => match Arc::try_unwrap(encoded) {
                Ok(encoded) => {
                    validate_run_ends(&encoded.run_ends)?;
                    Self::RunEndEncoded(
                        encoded.run_ends.into_arrow_ref()?,
                        encoded.values.into_arrow_ref()?,
                    )
                }
                Err(encoded) => Self::RunEndEncoded(
                    encoded.run_ends.clone().into_arrow_ref()?,
                    encoded.values.clone().into_arrow_ref()?,
                ),
            },
            // The Arrow storage of the extension-typed variants. The
            // extension name and metadata are *field* metadata
            // (`ARROW:extension:name`), so they ride `Field`'s projection;
            // this level answers the storage type Arrow actually lays out:
            // the canonical `arrow.parquet.variant` struct of two required
            // binaries, and WKB bytes for the geospatial pair. The codes lay
            // out as the text their arms above name.
            R::Variant => Self::Struct(arrow_schema::Fields::from(vec![
                arrow_schema::Field::new("metadata", Self::Binary, false),
                arrow_schema::Field::new("value", Self::Binary, false),
            ])),
            R::Geometry(_) | R::Geography(_) => Self::Binary,
        })
    }
}

impl TryFrom<&ArrowDataType> for DataType {
    type Error = Error;

    fn try_from(value: &ArrowDataType) -> Result<Self> {
        Self::from_arrow_at_depth(value, 0)
    }
}

impl TryFrom<ArrowDataType> for DataType {
    type Error = Error;

    fn try_from(value: ArrowDataType) -> Result<Self> {
        Self::from_arrow_owned_at_depth(value, 0)
    }
}

fn into_arrow_fields(fields: &Fields) -> Result<ArrowFields> {
    fields
        .iter()
        .cloned()
        .map(Field::into_arrow_ref)
        .collect::<Result<Vec<ArrowFieldRef>>>()
        .map(Into::into)
}

fn into_arrow_field(field: Arc<Field>) -> Result<ArrowFieldRef> {
    match Arc::try_unwrap(field) {
        Ok(field) => field.into_arrow_ref(),
        Err(field) => field.as_ref().clone().into_arrow_ref(),
    }
}

fn from_arrow_fields_at_depth(fields: &ArrowFields, depth: usize) -> Result<Fields> {
    let fields = fields
        .iter()
        .cloned()
        .map(|field| Field::from_arrow_ref_at_depth(field, depth))
        .collect::<Result<Vec<_>>>()?;
    Fields::from_imported_fields(fields)
}

/// The width an Arrow fixed binary declares, as the count this crate bounds
/// bytes in. Arrow's field is signed; a negative width is no width.
fn arrow_fixed_width(width: i32) -> Result<u32> {
    u32::try_from(width).map_err(|_| {
        invalid(
            "bytes",
            format_smolstr!("width must be non-negative: {width}"),
        )
    })
}

fn check_arrow_import_depth(depth: usize) -> Result<()> {
    if depth >= DataType::PARSE_RECURSION_LIMIT {
        Err(invalid(
            "ArrowImport",
            format_smolstr!(
                "datatype nesting exceeds the limit of {}",
                DataType::PARSE_RECURSION_LIMIT
            ),
        ))
    } else {
        Ok(())
    }
}

/// The Arrow extension name and metadata an extension-typed datatype
/// projects, `None` for every other datatype.
///
/// A dictionary projects what its values would. Arrow's `Dictionary` holds a
/// bare datatype for its values rather than a field, so a dictionary-encoded
/// currency has nowhere but the field itself to carry its identity - and a
/// dictionary-encoded code is the ordinary case, not an edge one.
pub(crate) fn arrow_extension_parts(dtype: &DataType) -> Option<(&'static str, String)> {
    match dtype {
        DataType::Dictionary(dictionary) => arrow_extension_parts(dictionary.value()),
        DataType::Variant => Some((VARIANT_EXTENSION_NAME, String::new())),
        DataType::Geometry(geospatial) | DataType::Geography(geospatial) => {
            Some((GEOARROW_WKB_EXTENSION_NAME, geospatial.geoarrow_json()))
        }
        // A charset, a length bound, and which of the two view layouts this
        // is: three facts Arrow has nowhere to put, so they ride here when
        // the string declares any of them; plain UTF-8 is Arrow's own.
        DataType::String(parameters) if needs_extension(*parameters) => {
            Some((STRING_EXTENSION_NAME, parameters.extension_json()))
        }
        // A maximum on a variable layout is the one fact about bytes Arrow
        // has nowhere to put; the four layouts and a fixed width are its own.
        DataType::Bytes(parameters) if super::bytes::needs_extension(*parameters) => {
            Some((BYTES_EXTENSION_NAME, parameters.extension_json()))
        }
        DataType::Uuid => Some((UUID_EXTENSION_NAME, String::new())),
        DataType::Version => Some((VERSION_EXTENSION_NAME, String::new())),
        DataType::Url => Some((URL_EXTENSION_NAME, String::new())),
        DataType::Timezone => Some((TIMEZONE_EXTENSION_NAME, String::new())),
        DataType::MimeType => Some((MIMETYPE_EXTENSION_NAME, String::new())),
        DataType::MediaType => Some((MEDIATYPE_EXTENSION_NAME, String::new())),
        // A code carries its own name, so the identity survives Arrow: three
        // bytes under `yggdryl.currency` read back a currency.
        code => code_extension_name(code).map(|name| (name, String::new())),
    }
}

/// Reports whether an Arrow datatype is exactly the canonical variant
/// storage: a struct of a non-nullable `metadata` Binary followed by a
/// non-nullable `value` Binary.
///
/// Child field metadata does not participate - it is transport, not
/// identity - but the order is fixed because Arrow's own struct casting is
/// positional and would silently relabel swapped children.
pub(crate) fn is_variant_storage(dtype: &ArrowDataType) -> bool {
    let ArrowDataType::Struct(fields) = dtype else {
        return false;
    };
    fields.len() == 2
        && fields[0].name() == "metadata"
        && fields[1].name() == "value"
        && fields
            .iter()
            .all(|field| field.data_type() == &ArrowDataType::Binary && !field.is_nullable())
}

/// Builds a C Data Interface schema without losing nested datatype flags.
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

fn native_dtype_to_ffi(dtype: &DataType) -> Result<FFI_ArrowSchema> {
    let (format, children, dictionary, flags) = match dtype {
        DataType::Sequence(SequenceType::List(field)) => (
            "+l".to_owned(),
            vec![field.as_ref().clone().into_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::Sequence(SequenceType::ListView(field)) => (
            "+vl".to_owned(),
            vec![field.as_ref().clone().into_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::Sequence(SequenceType::FixedSizeList(field, length)) => {
            validate_non_negative("FixedSizeList", "length", *length)?;
            (
                format!("+w:{length}"),
                vec![field.as_ref().clone().into_arrow_ffi()?],
                None,
                Flags::empty(),
            )
        }
        DataType::Sequence(SequenceType::LargeList(field)) => (
            "+L".to_owned(),
            vec![field.as_ref().clone().into_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::Sequence(SequenceType::LargeListView(field)) => (
            "+vL".to_owned(),
            vec![field.as_ref().clone().into_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::Structure(fields) => (
            "+s".to_owned(),
            fields
                .iter()
                .cloned()
                .map(Field::into_arrow_ffi)
                .collect::<Result<Vec<_>>>()?,
            None,
            Flags::empty(),
        ),
        DataType::Union(fields, mode) => {
            let prefix = match mode {
                UnionMode::Dense => "+ud:",
                UnionMode::Sparse => "+us:",
            };
            let mut format = String::with_capacity(prefix.len() + fields.len() * 4);
            format.push_str(prefix);
            let mut children = Vec::with_capacity(fields.len());
            for (index, (type_id, field)) in fields.iter().enumerate() {
                if index != 0 {
                    format.push(',');
                }
                push_i8_decimal(&mut format, type_id);
                children.push(field.clone().into_arrow_ffi()?);
            }
            (format, children, None, Flags::empty())
        }
        DataType::Dictionary(dictionary) => {
            validate_dictionary_key(&dictionary.key)?;
            let key = dictionary.key.clone().into_arrow_ffi()?;
            // An encoded extension declares its identity once, on the node
            // the field is: Arrow's dictionary values are a bare datatype, so
            // an importer reads the outer entries and never the ones a values
            // projection would carry. The tail below writes them there.
            let mut values = dictionary.value.clone().into_arrow_ffi()?;
            if arrow_extension_parts(&dictionary.value).is_some() {
                values = values.with_metadata::<[(&str, &str); 0], _>([])?;
            }
            (
                key.format().to_owned(),
                Vec::new(),
                Some(values),
                Flags::empty(),
            )
        }
        DataType::Mapping(mapping) => {
            validate_map_entries(mapping.entries())?;
            (
                "+m".to_owned(),
                vec![mapping.entries().clone().into_arrow_ffi()?],
                None,
                if mapping.keys_sorted() {
                    Flags::MAP_KEYS_SORTED
                } else {
                    Flags::empty()
                },
            )
        }
        DataType::RunEndEncoded(encoded) => {
            validate_run_ends(&encoded.run_ends)?;
            (
                "+r".to_owned(),
                vec![
                    encoded.run_ends.clone().into_arrow_ffi()?,
                    encoded.values.clone().into_arrow_ffi()?,
                ],
                None,
                Flags::empty(),
            )
        }
        // The extension identity of an extension-typed variant is metadata,
        // and a C schema is a field, so the storage projection carries the
        // two `ARROW:extension:*` entries here. `Field::into_arrow_ffi`
        // merges the same entries with the field's own metadata.
        //
        // Which datatypes those are is asked of the function that answers it
        // rather than re-listed here. The list this used to spell had drifted
        // several datatypes behind - `side`, `state`, `timeinforce` and
        // every `string(...)` - and each of them fell to
        // the plain arm below and crossed the C Data Interface as anonymous
        // storage, which is exactly what this arm exists to prevent.
        dtype if arrow_extension_parts(dtype).is_some() => {
            let arrow = dtype.clone().into_arrow()?;
            let schema = FFI_ArrowSchema::try_from(&arrow)?;
            let Some((name, metadata)) = arrow_extension_parts(dtype) else {
                return Err(invalid(
                    "ArrowExtension",
                    format_smolstr!("expected an extension projection for {dtype}, got none"),
                ));
            };
            return schema
                .with_metadata([
                    (arrow_schema::extension::EXTENSION_TYPE_NAME_KEY, name),
                    (
                        arrow_schema::extension::EXTENSION_TYPE_METADATA_KEY,
                        metadata.as_str(),
                    ),
                ])
                .map_err(Error::from);
        }
        _ => {
            let arrow = dtype.clone().into_arrow()?;
            return FFI_ArrowSchema::try_from(&arrow).map_err(Error::from);
        }
    };
    let schema = FFI_ArrowSchema::try_new(&format, children, dictionary)
        .and_then(|schema| schema.with_flags(flags))?;
    // A dictionary-encoded extension is still that extension, and the C
    // schema is the field that says so: the entries the plain projection
    // writes above ride here for the encoded shape too.
    let Some((name, metadata)) = arrow_extension_parts(dtype) else {
        return Ok(schema);
    };
    schema
        .with_metadata([
            (arrow_schema::extension::EXTENSION_TYPE_NAME_KEY, name),
            (
                arrow_schema::extension::EXTENSION_TYPE_METADATA_KEY,
                metadata.as_str(),
            ),
        ])
        .map_err(Error::from)
}

fn push_i8_decimal(output: &mut String, value: i8) {
    let magnitude = value.unsigned_abs();
    if value < 0 {
        output.push('-');
    }
    if magnitude >= 100 {
        output.push(char::from(b'0' + magnitude / 100));
    }
    if magnitude >= 10 {
        output.push(char::from(b'0' + magnitude / 10 % 10));
    }
    output.push(char::from(b'0' + magnitude % 10));
}
