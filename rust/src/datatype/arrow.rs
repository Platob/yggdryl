//! Lossless borrowed and consuming Arrow datatype interoperability.

use std::sync::Arc;

use arrow_schema::{
    DataType as ArrowDataType, FieldRef as ArrowFieldRef, Fields as ArrowFields,
    UnionFields as ArrowUnionFields,
    ffi::{FFI_ArrowSchema, Flags},
};
use smol_str::{SmolStr, format_smolstr};

use crate::field::arrow_field_to_ffi;
use crate::{Error, Field, Result};

use super::floating::validate_decimal;
use super::nested::{validate_dictionary_key, validate_map_entries, validate_run_ends};
use super::scalar::{invalid, validate_non_negative};
use super::temporal::{validate_time32_unit, validate_time64_unit};
use super::{DataType, Fields, UnionFields, UnionMode};

impl DataType {
    /// Projects this Struct datatype as an Arrow schema.
    ///
    /// A schema is the columns of a struct, so this is the same projection a
    /// non-null Struct [`crate::Field`] makes, without a name or metadata.
    ///
    /// # Errors
    ///
    /// Returns an error unless this is a bounded Struct datatype.
    // Mirrors the gating on `Field::to_arrow_schema`, which this delegates to:
    // the Arrow array runtime that owns the schema result is optional.
    #[cfg(feature = "arrow")]
    pub fn to_arrow_schema(&self) -> crate::arrow::Result<arrow_schema::SchemaRef> {
        crate::Field::new("row", self.clone(), false).to_arrow_schema()
    }

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
        crate::arrow::default_data_type_scalar_array(self)
    }

    /// Imports an Arrow datatype and validates every nested invariant.
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
            A::Timestamp(unit, timezone) => Self::Timestamp(
                (*unit).into(),
                timezone
                    .as_ref()
                    .map(|value| crate::Timezone::from_smol_str(SmolStr::from(Arc::clone(value))))
                    .transpose()?,
            ),
            A::Date32 => Self::Date32,
            A::Date64 => Self::Date64,
            A::Time32(unit) => Self::time32((*unit).into())?,
            A::Time64(unit) => Self::time64((*unit).into())?,
            A::Duration(unit) => Self::Duration((*unit).into()),
            A::Interval(unit) => Self::Interval((*unit).into()),
            A::Binary => Self::Binary,
            A::FixedSizeBinary(width) => Self::fixed_size_binary(*width)?,
            A::LargeBinary => Self::LargeBinary,
            A::BinaryView => Self::BinaryView,
            A::Utf8 => Self::Utf8,
            A::LargeUtf8 => Self::LargeUtf8,
            A::Utf8View => Self::Utf8View,
            A::List(field) => Self::List(Arc::new(Field::from_arrow_ref_at_depth(
                Arc::clone(field),
                child_depth,
            )?)),
            A::ListView(field) => Self::ListView(Arc::new(Field::from_arrow_ref_at_depth(
                Arc::clone(field),
                child_depth,
            )?)),
            A::FixedSizeList(field, length) => Self::fixed_size_list(
                Field::from_arrow_ref_at_depth(Arc::clone(field), child_depth)?,
                *length,
            )?,
            A::LargeList(field) => Self::LargeList(Arc::new(Field::from_arrow_ref_at_depth(
                Arc::clone(field),
                child_depth,
            )?)),
            A::LargeListView(field) => Self::LargeListView(Arc::new(
                Field::from_arrow_ref_at_depth(Arc::clone(field), child_depth)?,
            )),
            A::Struct(fields) => Self::Struct(from_arrow_fields_at_depth(fields, child_depth)?),
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
            A::Timestamp(unit, timezone) => Self::Timestamp(
                unit.into(),
                timezone
                    .map(|value| crate::Timezone::from_smol_str(SmolStr::from(value)))
                    .transpose()?,
            ),
            A::Date32 => Self::Date32,
            A::Date64 => Self::Date64,
            A::Time32(unit) => Self::time32(unit.into())?,
            A::Time64(unit) => Self::time64(unit.into())?,
            A::Duration(unit) => Self::Duration(unit.into()),
            A::Interval(unit) => Self::Interval(unit.into()),
            A::Binary => Self::Binary,
            A::FixedSizeBinary(width) => Self::fixed_size_binary(width)?,
            A::LargeBinary => Self::LargeBinary,
            A::BinaryView => Self::BinaryView,
            A::Utf8 => Self::Utf8,
            A::LargeUtf8 => Self::LargeUtf8,
            A::Utf8View => Self::Utf8View,
            A::List(field) => Self::List(Arc::new(Field::from_arrow_ref_at_depth(
                field,
                child_depth,
            )?)),
            A::ListView(field) => Self::ListView(Arc::new(Field::from_arrow_ref_at_depth(
                field,
                child_depth,
            )?)),
            A::FixedSizeList(field, length) => {
                Self::fixed_size_list(Field::from_arrow_ref_at_depth(field, child_depth)?, length)?
            }
            A::LargeList(field) => Self::LargeList(Arc::new(Field::from_arrow_ref_at_depth(
                field,
                child_depth,
            )?)),
            A::LargeListView(field) => Self::LargeListView(Arc::new(
                Field::from_arrow_ref_at_depth(field, child_depth)?,
            )),
            A::Struct(fields) => {
                // Arrow's shared field slice has no consuming iterator. Its
                // `FieldRef`s are shallow-cloned and remain the exact cached
                // projections held by the incoming Arrow datatype.
                let values = fields
                    .iter()
                    .cloned()
                    .map(|field| Field::from_arrow_ref_at_depth(field, child_depth))
                    .collect::<Result<Vec<_>>>()?;
                Self::Struct(Fields::from_imported_fields(values)?)
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

    /// Returns an owned Arrow datatype while borrowing this value.
    ///
    /// Nested fields reuse cached Arrow `Arc`s. The outer Arrow enum is newly
    /// constructed because Arrow and Yggdryl intentionally have distinct public
    /// value models.
    pub fn to_arrow(&self) -> Result<ArrowDataType> {
        self.try_into()
    }

    /// Projects this datatype to an owned Arrow C Data Interface schema.
    ///
    /// This uses the same validated Arrow projection as [`Self::to_arrow`]
    /// and preserves datatype flags recursively, including sorted map keys.
    /// Arrow 59's generic Field-to-C-schema conversion overwrites those flags
    /// when adding field flags, so Yggdryl owns the corrected recursive path.
    pub fn to_arrow_ffi(&self) -> Result<FFI_ArrowSchema> {
        native_data_type_to_ffi(self)
    }

    /// Consumes this value and returns its Arrow representation.
    ///
    /// Scalar conversion is allocation-free. Shared nested fields reuse their
    /// Arrow projections; uniquely owned child state can be consumed by
    /// [`Field::into_arrow_ref`].
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
            Self::List(field)
            | Self::ListView(field)
            | Self::FixedSizeList(field, _)
            | Self::LargeList(field)
            | Self::LargeListView(field) => field.arrow_import_is_projection_equivalent(),
            Self::Struct(fields) => fields
                .iter()
                .all(Field::arrow_import_is_projection_equivalent),
            Self::Union(fields, _) => fields
                .iter()
                .all(|(_, field)| field.arrow_import_is_projection_equivalent()),
            Self::Dictionary(dictionary) => {
                dictionary.key().arrow_import_is_projection_equivalent()
                    && dictionary.value().arrow_import_is_projection_equivalent()
            }
            Self::Map(map) => map.entries().arrow_import_is_projection_equivalent(),
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
            R::Timestamp(unit, timezone) => Self::Timestamp(
                unit.into_arrow_time()?,
                timezone
                    .as_ref()
                    .map(|value| Arc::<str>::from(value.as_smol_str().clone())),
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
            R::Duration(unit) => Self::Duration(unit.into_arrow_time()?),
            R::Interval(unit) => Self::Interval(unit.into_arrow_interval()?),
            R::Binary => Self::Binary,
            R::FixedSizeBinary(width) => {
                validate_non_negative("FixedSizeBinary", "width", *width)?;
                Self::FixedSizeBinary(*width)
            }
            R::LargeBinary => Self::LargeBinary,
            R::BinaryView => Self::BinaryView,
            R::Utf8 => Self::Utf8,
            R::LargeUtf8 => Self::LargeUtf8,
            R::Utf8View => Self::Utf8View,
            R::List(field) => Self::List(field.to_arrow_ref()?),
            R::ListView(field) => Self::ListView(field.to_arrow_ref()?),
            R::FixedSizeList(field, length) => {
                validate_non_negative("FixedSizeList", "length", *length)?;
                Self::FixedSizeList(field.to_arrow_ref()?, *length)
            }
            R::LargeList(field) => Self::LargeList(field.to_arrow_ref()?),
            R::LargeListView(field) => Self::LargeListView(field.to_arrow_ref()?),
            R::Struct(fields) => Self::Struct(to_arrow_fields(fields)?),
            R::Union(fields, mode) => {
                let mut type_ids = Vec::with_capacity(fields.len());
                let mut arrow_fields = Vec::with_capacity(fields.len());
                for (type_id, field) in fields.iter() {
                    type_ids.push(type_id);
                    arrow_fields.push(field.to_arrow_ref()?);
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
            R::Map(map) => {
                validate_map_entries(&map.entries)?;
                Self::Map(map.entries.to_arrow_ref()?, map.keys_sorted)
            }
            R::RunEndEncoded(encoded) => {
                validate_run_ends(&encoded.run_ends)?;
                Self::RunEndEncoded(
                    encoded.run_ends.to_arrow_ref()?,
                    encoded.values.to_arrow_ref()?,
                )
            }
            // The Arrow storage of the three extension-typed variants. The
            // extension name and metadata are *field* metadata
            // (`ARROW:extension:name`), so they ride `Field`'s projection;
            // this level answers the storage type Arrow actually lays out:
            // the canonical `arrow.parquet.variant` struct of two required
            // binaries, and WKB bytes for the geospatial pair.
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
            R::Timestamp(unit, timezone) => Self::Timestamp(
                unit.into_arrow_time()?,
                timezone.map(|value| Arc::<str>::from(value.into_smol_str())),
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
            R::Duration(unit) => Self::Duration(unit.into_arrow_time()?),
            R::Interval(unit) => Self::Interval(unit.into_arrow_interval()?),
            R::Binary => Self::Binary,
            R::FixedSizeBinary(width) => {
                validate_non_negative("FixedSizeBinary", "width", width)?;
                Self::FixedSizeBinary(width)
            }
            R::LargeBinary => Self::LargeBinary,
            R::BinaryView => Self::BinaryView,
            R::Utf8 => Self::Utf8,
            R::LargeUtf8 => Self::LargeUtf8,
            R::Utf8View => Self::Utf8View,
            R::List(field) => Self::List(into_arrow_field(field)?),
            R::ListView(field) => Self::ListView(into_arrow_field(field)?),
            R::FixedSizeList(field, length) => {
                validate_non_negative("FixedSizeList", "length", length)?;
                Self::FixedSizeList(into_arrow_field(field)?, length)
            }
            R::LargeList(field) => Self::LargeList(into_arrow_field(field)?),
            R::LargeListView(field) => Self::LargeListView(into_arrow_field(field)?),
            R::Struct(fields) => {
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
                    Box::new(dictionary.key.to_arrow()?),
                    Box::new(dictionary.value.to_arrow()?),
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
            R::Map(map) => match Arc::try_unwrap(map) {
                Ok(map) => {
                    validate_map_entries(&map.entries)?;
                    Self::Map(map.entries.into_arrow_ref()?, map.keys_sorted)
                }
                Err(map) => {
                    validate_map_entries(&map.entries)?;
                    Self::Map(map.entries.to_arrow_ref()?, map.keys_sorted)
                }
            },
            R::RunEndEncoded(encoded) => match Arc::try_unwrap(encoded) {
                Ok(encoded) => {
                    validate_run_ends(&encoded.run_ends)?;
                    Self::RunEndEncoded(
                        encoded.run_ends.into_arrow_ref()?,
                        encoded.values.into_arrow_ref()?,
                    )
                }
                Err(encoded) => Self::RunEndEncoded(
                    encoded.run_ends.to_arrow_ref()?,
                    encoded.values.to_arrow_ref()?,
                ),
            },
            // The Arrow storage of the three extension-typed variants. The
            // extension name and metadata are *field* metadata
            // (`ARROW:extension:name`), so they ride `Field`'s projection;
            // this level answers the storage type Arrow actually lays out:
            // the canonical `arrow.parquet.variant` struct of two required
            // binaries, and WKB bytes for the geospatial pair.
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

fn to_arrow_fields(fields: &Fields) -> Result<ArrowFields> {
    fields
        .iter()
        .map(Field::to_arrow_ref)
        .collect::<Result<Vec<ArrowFieldRef>>>()
        .map(Into::into)
}

fn into_arrow_field(field: Arc<Field>) -> Result<ArrowFieldRef> {
    match Arc::try_unwrap(field) {
        Ok(field) => field.into_arrow_ref(),
        Err(field) => field.to_arrow_ref(),
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

/// Builds a C Data Interface schema without losing nested datatype flags.
pub(crate) fn arrow_data_type_to_ffi(
    data_type: &ArrowDataType,
) -> std::result::Result<FFI_ArrowSchema, arrow_schema::ArrowError> {
    let template = FFI_ArrowSchema::try_from(data_type)?;
    let children = match data_type {
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
    let dictionary = match data_type {
        ArrowDataType::Dictionary(_, value) => Some(arrow_data_type_to_ffi(value)?),
        _ => None,
    };
    let flags = template.flags().unwrap_or_else(Flags::empty);
    FFI_ArrowSchema::try_new(template.format(), children, dictionary)?.with_flags(flags)
}

fn native_data_type_to_ffi(data_type: &DataType) -> Result<FFI_ArrowSchema> {
    let (format, children, dictionary, flags) = match data_type {
        DataType::List(field) => (
            "+l".to_owned(),
            vec![field.to_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::ListView(field) => (
            "+vl".to_owned(),
            vec![field.to_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::FixedSizeList(field, length) => {
            validate_non_negative("FixedSizeList", "length", *length)?;
            (
                format!("+w:{length}"),
                vec![field.to_arrow_ffi()?],
                None,
                Flags::empty(),
            )
        }
        DataType::LargeList(field) => (
            "+L".to_owned(),
            vec![field.to_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::LargeListView(field) => (
            "+vL".to_owned(),
            vec![field.to_arrow_ffi()?],
            None,
            Flags::empty(),
        ),
        DataType::Struct(fields) => (
            "+s".to_owned(),
            fields
                .iter()
                .map(Field::to_arrow_ffi)
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
                children.push(field.to_arrow_ffi()?);
            }
            (format, children, None, Flags::empty())
        }
        DataType::Dictionary(dictionary) => {
            validate_dictionary_key(&dictionary.key)?;
            let key = dictionary.key.to_arrow_ffi()?;
            (
                key.format().to_owned(),
                Vec::new(),
                Some(dictionary.value.to_arrow_ffi()?),
                Flags::empty(),
            )
        }
        DataType::Map(map) => {
            validate_map_entries(&map.entries)?;
            (
                "+m".to_owned(),
                vec![map.entries.to_arrow_ffi()?],
                None,
                if map.keys_sorted {
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
                    encoded.run_ends.to_arrow_ffi()?,
                    encoded.values.to_arrow_ffi()?,
                ],
                None,
                Flags::empty(),
            )
        }
        _ => {
            let arrow = data_type.to_arrow()?;
            return FFI_ArrowSchema::try_from(&arrow).map_err(Error::from);
        }
    };
    FFI_ArrowSchema::try_new(&format, children, dictionary)
        .and_then(|schema| schema.with_flags(flags))
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
