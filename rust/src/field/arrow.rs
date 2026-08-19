//! Arrow field import, cached projection, and conversion traits.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use arrow_schema::{
    Field as ArrowField,
    ffi::{FFI_ArrowSchema, Flags},
};

use super::{Field, FieldRef};
use crate::datatype::arrow_data_type_to_ffi;
use crate::{DataType, Error, Metadata, Result};

impl Field {
    /// Imports an Arrow field and seeds the projection cache.
    pub fn from_arrow(value: &ArrowField) -> Result<Self> {
        Self::from_arrow_at_depth(value, 0)
    }

    pub(crate) fn from_arrow_at_depth(value: &ArrowField, depth: usize) -> Result<Self> {
        let metadata = Metadata::from_arrow(value.metadata())?;
        let data_type = DataType::from_arrow_at_depth(value.data_type(), depth)?;
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
        let metadata = Metadata::from_arrow(value.metadata())?;
        let data_type = DataType::from_arrow_at_depth(value.data_type(), depth)?;
        let mut field = imported_field(&value, data_type, metadata);
        let cacheable = imported_arrow_is_cacheable(&field, value.metadata());
        seed_imported_arrow_cache(&mut field, cacheable, || value);
        Ok(field)
    }

    pub(crate) fn from_arrow_owned_at_depth(value: ArrowField, depth: usize) -> Result<Self> {
        let data_type = DataType::from_arrow_at_depth(value.data_type(), depth)?;
        let metadata = Metadata::from_arrow(value.metadata())?;
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
            self.metadata.to_arrow(),
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
        schema
            .with_metadata(&self.metadata.to_arrow())
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
        Ok(arrow_field_from_parts(
            name.as_str(),
            data_type.into_arrow()?,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            metadata.into_arrow(),
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
        Ok(Arc::new(arrow_field_from_parts(
            name.as_str(),
            data_type.into_arrow()?,
            nullable,
            dictionary_id,
            dictionary_is_ordered,
            metadata.into_arrow(),
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
