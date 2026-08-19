//! Arrow field import, cached projection, and conversion traits.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use arrow_schema::{
    DataType as ArrowDataType, Field as ArrowField,
    extension::{EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY},
    ffi::{FFI_ArrowSchema, Flags},
};
use smol_str::{SmolStr, format_smolstr};

use super::{Field, FieldRef};
use crate::datatype::{
    GEOARROW_WKB_EXTENSION_NAME, VARIANT_EXTENSION_NAME, arrow_data_type_to_ffi,
    arrow_extension_parts, is_variant_storage,
};
use crate::{DataType, Error, GeospatialType, Metadata, Result};

impl Field {
    /// Imports an Arrow field and seeds the projection cache.
    pub fn from_arrow(value: &ArrowField) -> Result<Self> {
        Self::from_arrow_at_depth(value, 0)
    }

    pub(crate) fn from_arrow_at_depth(value: &ArrowField, depth: usize) -> Result<Self> {
        let (data_type, metadata) = imported_parts(value, depth)?;
        let mut field = imported_field(value, data_type, metadata);
        let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
        seed_imported_arrow_cache(&mut field, cacheable, || Arc::new(value.clone()));
        Ok(field)
    }

    /// Imports a shared Arrow field without cloning its projection allocation.
    pub fn from_arrow_ref(value: FieldRef) -> Result<Self> {
        Self::from_arrow_ref_at_depth(value, 0)
    }

    pub(crate) fn from_arrow_ref_at_depth(value: FieldRef, depth: usize) -> Result<Self> {
        let (data_type, metadata) = imported_parts(&value, depth)?;
        let mut field = imported_field(&value, data_type, metadata);
        let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
        seed_imported_arrow_cache(&mut field, cacheable, || value);
        Ok(field)
    }

    pub(crate) fn from_arrow_owned_at_depth(value: ArrowField, depth: usize) -> Result<Self> {
        let (data_type, metadata) = imported_parts(&value, depth)?;
        let mut field = imported_field(&value, data_type, metadata);
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
    #[cfg(feature = "arrow")]
    pub fn to_arrow_schema(&self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
        crate::arrow::schema_from_field(self)
    }

    /// Consumes this Field and projects it as an Arrow schema.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a bounded, non-nullable Struct root.
    #[cfg(feature = "arrow")]
    pub fn into_arrow_schema(self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
        self.to_arrow_schema()
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

    /// Returns a cached shared Arrow field projection.
    pub fn to_arrow_ref(&self) -> Result<FieldRef> {
        if let Some(field) = self.arrow.get() {
            return Ok(Arc::clone(field));
        }
        let field = Arc::new(arrow_field_from_parts(
            self.name.as_str(),
            self.data_type.to_arrow()?,
            self.nullable,
            self.dictionary_id,
            self.dictionary_is_ordered,
            projected_arrow_metadata(&self.data_type, self.metadata.to_arrow())?,
        ));
        if self.arrow.set(Arc::clone(&field)).is_ok() {
            Ok(field)
        } else {
            Ok(self.arrow.get().map_or(field, Arc::clone))
        }
    }

    /// Returns an owned Arrow field, sharing nested Arrow state where possible.
    pub fn to_arrow(&self) -> Result<ArrowField> {
        Ok(self.to_arrow_ref()?.as_ref().clone())
    }

    /// Projects this field to an owned Arrow C Data Interface schema.
    ///
    /// Name, metadata, nullability, dictionary ordering, and nested datatype
    /// flags are preserved in one canonical core conversion.
    pub fn to_arrow_ffi(&self) -> Result<FFI_ArrowSchema> {
        if let Some(field) = self.arrow.get() {
            return arrow_field_to_ffi(field).map_err(Error::from);
        }
        let mut schema = self.data_type.to_arrow_ffi()?;
        let mut flags = schema.flags().unwrap_or_else(Flags::empty);
        if self.nullable {
            flags |= Flags::NULLABLE;
        }
        if self.dictionary_is_ordered {
            flags |= Flags::DICTIONARY_ORDERED;
        }
        schema = schema.with_name(self.name())?.with_flags(flags)?;
        // The datatype projection already carries the extension entries for a
        // variant or geospatial field, but the C interface stores metadata as
        // one buffer, so the replacement map must carry them again.
        schema
            .with_metadata(&projected_arrow_metadata(
                &self.data_type,
                self.metadata.to_arrow(),
            )?)
            .map_err(Error::from)
    }

    /// Consumes this field and returns an owned Arrow field.
    pub fn into_arrow(self) -> Result<ArrowField> {
        let Self {
            name,
            data_type,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            metadata,
            arrow,
        } = self;
        if let Some(field) = arrow.into_inner() {
            return Ok(Arc::try_unwrap(field).unwrap_or_else(|field| field.as_ref().clone()));
        }
        let projected = projected_arrow_metadata(&data_type, metadata.into_arrow())?;
        Ok(arrow_field_from_parts(
            name.as_str(),
            data_type.into_arrow()?,
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
            data_type,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            metadata,
            arrow,
        } = self;
        if let Some(field) = arrow.into_inner() {
            return Ok(field);
        }
        let projected = projected_arrow_metadata(&data_type, metadata.into_arrow())?;
        Ok(Arc::new(arrow_field_from_parts(
            name.as_str(),
            data_type.into_arrow()?,
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
        && field.data_type.arrow_import_is_projection_equivalent()
}

/// The core identity an Arrow field's extension metadata declares, when it
/// declares one of the three first-class extension-typed datatypes.
pub(crate) enum RecognizedExtension {
    /// The canonical `arrow.parquet.variant` over its exact storage struct.
    Variant,
    /// The community `geoarrow.wkb` over Binary storage; the parsed GeoArrow
    /// document says whether it is a geometry or a geography.
    Geospatial(GeospatialType),
}

impl RecognizedExtension {
    /// The first-class datatype this recognized extension imports as.
    pub(crate) fn into_data_type(self) -> DataType {
        match self {
            Self::Variant => DataType::Variant,
            Self::Geospatial(geospatial) => {
                if geospatial.algorithm().is_some() {
                    DataType::Geography(Arc::new(geospatial))
                } else {
                    DataType::Geometry(Arc::new(geospatial))
                }
            }
        }
    }
}

/// Recognizes the two Arrow extension spellings the three first-class
/// datatypes ride: `geoarrow.wkb` over Binary storage, and the canonical
/// `arrow.parquet.variant` over its exact storage struct with an empty
/// extension metadata document.
///
/// Any other pairing - a foreign extension name, one of ours over a storage
/// it does not spell, a variant with a non-empty document - keeps today's
/// behavior exactly: the field imports as its storage type with the
/// `ARROW:extension:*` keys as plain metadata.
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
            let geospatial = GeospatialType::from_geoarrow_json(document).map_err(|error| {
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
        _ => Ok(None),
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
        return Ok((
            recognized.into_data_type(),
            Metadata::from_arrow(&stripped)?,
        ));
    }
    let metadata = Metadata::from_arrow(value.metadata())?;
    let data_type = DataType::from_arrow_at_depth(value.data_type(), depth)?;
    Ok((data_type, metadata))
}

/// Completes a projected Arrow metadata map with the extension identity of a
/// variant or geospatial datatype, refusing a user-set extension key.
///
/// The two `ARROW:extension:*` entries belong to the datatype: letting a
/// caller's own value ride along would let one field name two extensions, so
/// a conflict is refused naming both spellings rather than silently picking.
fn projected_arrow_metadata(
    data_type: &DataType,
    mut metadata: HashMap<String, String>,
) -> Result<HashMap<String, String>> {
    let Some((name, document)) = arrow_extension_parts(data_type) else {
        return Ok(metadata);
    };
    for key in [EXTENSION_TYPE_NAME_KEY, EXTENSION_TYPE_METADATA_KEY] {
        if let Some(existing) = metadata.get(key) {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(key),
                reason: format_smolstr!(
                    "expected no caller-set Arrow extension entry on a {} field, \
                     got {existing:?}; the datatype itself projects {name:?}",
                    data_type.name()
                ),
            });
        }
    }
    metadata.insert(EXTENSION_TYPE_NAME_KEY.to_owned(), name.to_owned());
    metadata.insert(EXTENSION_TYPE_METADATA_KEY.to_owned(), document);
    Ok(metadata)
}

fn imported_field(value: &ArrowField, data_type: DataType, metadata: Metadata) -> Field {
    #[allow(deprecated)]
    let dictionary_id = value.dict_id().unwrap_or_default();
    Field {
        name: value.name().into(),
        data_type,
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
    let mut schema = arrow_data_type_to_ffi(field.data_type())?;
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
    data_type: arrow_schema::DataType,
    nullable: bool,
    dictionary_id: i64,
    dictionary_is_ordered: bool,
    metadata: HashMap<String, String>,
) -> ArrowField {
    ArrowField::new_dict(
        name,
        data_type,
        nullable,
        dictionary_id,
        dictionary_is_ordered,
    )
    .with_metadata(metadata)
}

impl TryFrom<&Field> for ArrowField {
    type Error = Error;

    fn try_from(value: &Field) -> Result<Self> {
        value.to_arrow()
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
        let arrow = field.to_arrow().unwrap();
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
        let arrow = field.to_arrow().unwrap();
        assert_eq!(
            arrow.extension_type_metadata(),
            Some(r#"{"crs":"EPSG:4326","edges":"vincenty"}"#)
        );
        assert_eq!(Field::from_arrow(&arrow).unwrap(), field);
    }

    #[test]
    fn a_variant_field_projects_the_canonical_struct_and_reimports_itself() {
        let field = Field::new("payload", DataType::variant(), true);
        let arrow = field.to_arrow().unwrap();
        assert_eq!(arrow.data_type(), &variant_storage());
        assert_eq!(arrow.extension_type_name(), Some("arrow.parquet.variant"));
        assert_eq!(arrow.extension_type_metadata(), Some(""));

        let imported = Field::from_arrow(&arrow).unwrap();
        assert_eq!(imported.data_type(), &DataType::Variant);
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
        assert_eq!(imported.data_type(), &DataType::geometry(None).unwrap());
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
        assert_eq!(imported.data_type(), &DataType::Binary);
        assert_eq!(
            imported.get_metadata(EXTENSION_TYPE_NAME_KEY),
            Some("someorg.blob")
        );
        assert_eq!(imported.to_arrow().unwrap(), arrow);
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
        assert_eq!(imported.data_type(), &DataType::LargeBinary);
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
        let refused = field.to_arrow().unwrap_err().to_string();
        assert!(refused.contains("someorg.other"), "{refused}");
        assert!(refused.contains("geoarrow.wkb"), "{refused}");

        let variant = Field::from_parts(
            "payload",
            DataType::variant(),
            true,
            [(EXTENSION_TYPE_METADATA_KEY, "shredded")],
        )
        .unwrap();
        let refused = variant.to_arrow().unwrap_err().to_string();
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
        assert!(matches!(imported.data_type(), DataType::Struct(_)));

        let swapped = ArrowDataType::Struct(arrow_schema::Fields::from(vec![
            ArrowField::new("value", ArrowDataType::Binary, false),
            ArrowField::new("metadata", ArrowDataType::Binary, false),
        ]));
        let swapped = ArrowField::new("payload", swapped, true).with_metadata(HashMap::from([(
            EXTENSION_TYPE_NAME_KEY.to_owned(),
            "arrow.parquet.variant".to_owned(),
        )]));
        let imported = Field::from_arrow(&swapped).unwrap();
        assert!(matches!(imported.data_type(), DataType::Struct(_)));
    }
}
