//! Casting an Arrow array into the exact array a typed field describes.
//!
//! [`ArrowCast`] answers "make this array fit that field" for any
//! field, and returns an [`ArrayRef`] because any field could be any datatype.
//! A [`TypedField`](crate::TypedField) already knows its variant, so it can answer with the array
//! type itself: [`Int64Field`](crate::types::Int64Field) casts to an
//! [`Int64Array`](arrow_array::Int64Array), and the caller reads values without
//! a downcast of its own.
//!
//! The field is always the *target*: an incoming array is reconciled to the
//! field's datatype and nullability, never the other way around.
//!
//! ```
//! use std::sync::Arc;
//!
//! use arrow_array::{ArrayRef, StringArray};
//! use yggdryl::types::Int64Field;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let field = Int64Field::new("id", false);
//! let source: ArrayRef = Arc::new(StringArray::from(vec!["1", "2"]));
//!
//! // The result is an Int64Array, not an ArrayRef needing a downcast.
//! let ids = field.cast_arrow_array(source, false)?;
//! assert_eq!(ids.values(), &[1, 2]);
//! # Ok(())
//! # }
//! ```
//!
//! A few datatypes carry a parameter that decides their physical array -
//! a timestamp's unit, a dictionary's key type - so those cast to an
//! [`ArrayRef`]. Every other variant casts to its concrete array.

use std::collections::HashMap;
use std::sync::Arc;

use crate::types::ascii::casts::{ingest_ascii_array, ingest_code_array, render_ascii_text};
use crate::types::geospatial::casts::{render_wkt_array, validate_wkb_ingest};
use crate::types::nested::casts::{
    cast_dictionary_planned, cast_run_planned, cast_union_planned, contains_struct, default_array,
    exposed_logical_null_count, fill_nulls, folded_field_mapping, is_logically_null,
    is_reconcilable_nested, list_child, union_mode_matches,
};
use crate::types::temporal::casts::{
    holds_temporal, holds_text, ingest_temporal_text, is_temporal_arrow, render_temporal_text,
};
use crate::types::uuid::casts::{ingest_uuid_array, render_uuid_text};
use crate::types::version::casts::{ingest_version_array, is_text_storage};
use crate::types::{CFI_WIDTH, COUNTRY_WIDTH, CURRENCY_WIDTH, MIC_WIDTH, code_refusal};
use crate::types::{RecognizedExtension, recognized_arrow_extension};
use crate::{DataType, Field};
use arrow_array::{
    Array, ArrayRef, RecordBatch, RecordBatchOptions, Scalar as ArrowScalar, StructArray,
};
use arrow_buffer::BooleanBuffer;
use arrow_cast::can_cast_types;
use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef};

use crate::arrow::{Error, Result};
use crate::types::budget::MaterializationBudget;

mod batch;
mod kernel;
mod typed;

pub use batch::{preflight_arrow_batch_cast, validate_arrow_batch};
pub(crate) use kernel::arrow_cast_exposed;
pub use typed::ArrowFieldType;

/// Arrow array and record-batch casting owned by a canonical Yggdryl schema.
///
/// This extension trait lives in [`crate::arrow`], keeping recursive runtime
/// casts behind Yggdryl's `arrow` feature while retaining method syntax on
/// [`DataType`] and [`Field`].
pub trait ArrowCast {
    /// Casts an Arrow array to this exact physical datatype.
    ///
    /// `safe` is passed directly to Arrow's [`CastOptions`](arrow_cast::CastOptions). With Arrow 59,
    /// supported conversion failures become null when it is true and are
    /// errors when it is false. A non-nullable target Field replaces resulting
    /// nulls with its canonical default; a nullable Field retains them.
    ///
    /// Temporals cross a text boundary through this crate's own spellings, so
    /// a column reads and prints what a row reads and prints; Arrow's kernel
    /// keeps the spellings only it knows.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported cast, an ambiguous case-insensitive
    /// Struct match, or a result that cannot satisfy the target Field.
    fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<ArrayRef>;

    /// Reconciles an Arrow record batch to this Struct schema.
    ///
    /// Struct children are selected in target order by ASCII-case-insensitive
    /// name. Extra source columns are dropped, missing nullable columns are
    /// null-filled, and missing required columns use canonical defaults.
    /// An already exact batch is returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns an error unless this value is a Struct schema, or when a child
    /// cast or missing-column default cannot be materialized.
    fn cast_arrow_batch(&self, batch: RecordBatch, safe: bool) -> Result<RecordBatch>;

    /// Casts a one-row Arrow array to this exact schema, as a scalar.
    ///
    /// A scalar is a one-row array with the row pinned, so the cast is
    /// [`cast_arrow_array`](Self::cast_arrow_array) plus the length check
    /// that makes the pinning honest.
    ///
    /// # Errors
    ///
    /// Returns an error when the array does not hold exactly one row, or any
    /// error the array cast returns.
    fn cast_arrow_scalar(&self, array: ArrayRef, safe: bool) -> Result<ArrowScalar<ArrayRef>> {
        if array.len() != 1 {
            return Err(Error::IncompatibleSchema(format!(
                "a scalar cast takes exactly one row, got {}",
                array.len()
            )));
        }
        Ok(ArrowScalar::new(self.cast_arrow_array(array, safe)?))
    }
}

impl ArrowCast for DataType {
    fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<ArrayRef> {
        let plan = ArrayCastPlan::new_dtype(self, array.data_type(), safe)?;
        let mut budget = MaterializationBudget::default();
        plan.cast(array, &mut budget)
    }

    fn cast_arrow_batch(&self, batch: RecordBatch, safe: bool) -> Result<RecordBatch> {
        Field::new("record", self.clone(), false).cast_arrow_batch(batch, safe)
    }
}

impl ArrowCast for Field {
    fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<ArrayRef> {
        cast_field_array(self, None, array, safe)
    }

    fn cast_arrow_batch(&self, batch: RecordBatch, safe: bool) -> Result<RecordBatch> {
        cast_record_batch(self, batch, safe)
    }
}

/// Cast a record batch by casting the struct array it already is.
///
/// A [`RecordBatch`] is a [`StructArray`] plus a schema, so there is no second
/// cast engine here: the batch becomes an array, goes through the same
/// recursive field cast every array uses - name reconciliation, defaults for
/// missing columns, nested repair - and comes back as a batch.
pub(crate) fn cast_record_batch(
    target: &Field,
    batch: RecordBatch,
    safe: bool,
) -> Result<RecordBatch> {
    target.validate_bounded()?;
    if target.is_nullable() {
        return Err(Error::IncompatibleSchema(
            "record-batch cast target Struct Field must be non-nullable".to_owned(),
        ));
    }
    if target.dtype().as_fields().is_none() {
        return Err(Error::IncompatibleSchema(format!(
            "record-batch cast target {:?} must have a struct datatype",
            target.name()
        )));
    }

    let row_count = batch.num_rows();
    let source = struct_array_from_batch(batch);
    let cast = cast_field_array(target, None, source, safe)?;
    let columns = cast
        .as_any()
        .downcast_ref::<StructArray>()
        .ok_or_else(|| {
            Error::IncompatibleSchema(
                "a struct cast must produce a struct array; the cast table disagrees".to_owned(),
            )
        })?
        .columns()
        .to_vec();

    // A batch carries its row count even with no columns, which a struct array
    // cannot, so the count is restored explicitly.
    let schema = crate::arrow::arrow_schema_from_field(target)?;
    let options = RecordBatchOptions::new().with_row_count(Some(row_count));
    RecordBatch::try_new_with_options(schema, columns, &options).map_err(Into::into)
}

/// View a batch as one struct array, keeping the row count of an empty batch.
fn struct_array_from_batch(batch: RecordBatch) -> ArrayRef {
    if batch.num_columns() == 0 {
        return Arc::new(StructArray::new_empty_fields(batch.num_rows(), None));
    }
    Arc::new(StructArray::from(batch))
}

#[derive(Clone, Copy)]
enum NullPolicy {
    Field,
    DataType,
    Reject,
}

#[derive(Clone, Copy)]
enum StructPolicy {
    Normal,
    MapEntries,
}

pub(crate) struct ArrayCastPlan {
    pub(crate) field: Field,
    pub(crate) source_type: ArrowDataType,
    pub(crate) expected: ArrowDataType,
    pub(crate) safe: bool,
    null_policy: NullPolicy,
    kind: ArrayCastKind,
}

enum ArrayCastKind {
    Exact,
    Kernel,
    /// Bytes entering a geometry or geography: same bytes out, but every
    /// exposed value is validated as WKB on the way in. A non-Binary binary
    /// layout is first cast to the Binary storage through Arrow's kernel.
    GeospatialIngest,
    /// A recognized geospatial source rendering as WKT text.
    GeospatialWkt,
    /// Values entering an ASCII width: every exposed value is validated
    /// under the width's rule and padded into the fixed storage. A fixed
    /// binary of the target width is the same array once validated; any
    /// other source first renders as Utf8 through Arrow's kernel.
    AsciiIngest,
    /// A recognized ASCII source rendering as trimmed text.
    AsciiText,
    /// Values entering a registered code: the same rule as [`Self::AsciiIngest`]
    /// at the width the code fixes, which is a constant, so the validation
    /// and the padding run monomorphized per code rather than reading a
    /// width out of the datatype on every row.
    CodeIngest,
    /// A recognized code source rendering as trimmed text, at its own width.
    CodeText,
    /// Values entering a UUID: every exposed value is validated under the one
    /// UUID rule and stored as its sixteen bytes.
    UuidIngest,
    /// A recognized UUID source rendering as its hyphenated spelling.
    UuidText,
    /// Text entering a version is parsed and rewritten to its canonical text.
    VersionIngest,
    /// Text entering a temporal: every exposed value is read through the
    /// crate's own spellings, which are wider than Arrow's. Arrow's kernel
    /// stays behind them for the spellings only it knows, so the reading is
    /// never narrower than it was.
    TemporalIngest,
    /// A temporal rendering as its classic text, so a column spells what a
    /// row spells - a zoned instant included, which Arrow's own formatter
    /// refuses without its timezone database.
    TemporalText,
    DeferredUnsupported {
        reason: String,
    },
    Struct {
        fields: arrow_schema::Fields,
        columns: Vec<StructColumnPlan>,
    },
    List {
        field: ArrowFieldRef,
        child: Box<ArrayCastPlan>,
        kind: ListPlanKind,
    },
    Map {
        source: Arc<crate::MapType>,
        field: ArrowFieldRef,
        ordered: bool,
        entries: Box<ArrayCastPlan>,
    },
    Dictionary {
        source_key: ArrowDataType,
        values: Box<ArrayCastPlan>,
    },
    Union {
        fields: arrow_schema::UnionFields,
        children: Vec<(i8, ArrayCastPlan)>,
    },
    RunEndEncoded {
        source_run_type: ArrowDataType,
        values: Box<ArrayCastPlan>,
    },
}

pub(crate) enum StructColumnPlan {
    Source { index: usize, cast: ArrayCastPlan },
    Missing(Field),
}

#[derive(Clone, Copy)]
pub(crate) enum ListPlanKind {
    List,
    LargeList,
    ListView,
    LargeListView,
    FixedSize { size: i32 },
}

impl ArrayCastPlan {
    fn new_dtype(dtype: &DataType, source_type: &ArrowDataType, safe: bool) -> Result<Self> {
        dtype.validate_bounded()?;
        Self::new_nested_validated(
            &Field::new("value", dtype.clone(), false),
            source_type,
            safe,
            NullPolicy::DataType,
        )
    }

    fn new_nested_validated(
        field: &Field,
        source_type: &ArrowDataType,
        safe: bool,
        null_policy: NullPolicy,
    ) -> Result<Self> {
        Self::new_validated_with(
            field,
            source_type,
            None,
            safe,
            null_policy,
            StructPolicy::Normal,
            true,
        )
    }

    /// Plans a nested cast whose source is a complete Arrow field, so the
    /// source's extension identity (a variant, a geometry, a geography)
    /// participates in the declared cast rules rather than being erased to
    /// its storage type.
    fn new_nested_from_arrow_field(
        field: &Field,
        source_field: &ArrowFieldRef,
        safe: bool,
        null_policy: NullPolicy,
    ) -> Result<Self> {
        Self::new_validated_with(
            field,
            source_field.data_type(),
            Some(source_field.metadata()),
            safe,
            null_policy,
            StructPolicy::Normal,
            true,
        )
    }

    fn new_validated_with(
        field: &Field,
        source_type: &ArrowDataType,
        source_metadata: Option<&HashMap<String, String>>,
        safe: bool,
        null_policy: NullPolicy,
        struct_policy: StructPolicy,
        may_be_fully_hidden: bool,
    ) -> Result<Self> {
        let source_extension = match source_metadata {
            Some(metadata) => recognized_arrow_extension(metadata, source_type)?,
            None => None,
        };
        check_extension_source(field, source_extension.as_ref())?;
        let expected = field.clone().into_arrow_ref()?.data_type().clone();
        // A geospatial target validates WKB on the way in and an ASCII
        // target validates text, so an exact storage source must still take
        // the planned path - unless it is a recognized ASCII source of the
        // same width, already validated when it was written.
        let ingest_validated = match field.dtype() {
            DataType::Geometry(_) | DataType::Geography(_) => true,
            DataType::Ascii | DataType::FixedAscii(_) => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::Ascii(source)) if source == field.dtype()
            ),
            // The same rule for a code, over its own extension: a currency
            // column written as a currency is already validated, and one
            // written as three anonymous bytes is not.
            DataType::Country | DataType::Currency | DataType::Mic | DataType::Cfi => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::Code(source)) if source == field.dtype()
            ),
            DataType::Uuid => !matches!(source_extension.as_ref(), Some(RecognizedExtension::Uuid)),
            DataType::Version => !matches!(
                source_extension.as_ref(),
                Some(RecognizedExtension::Version)
            ),
            _ => false,
        };
        let kind = if source_type == &expected
            && !is_reconcilable_nested(field.dtype())
            && !ingest_validated
        {
            ArrayCastKind::Exact
        } else {
            Self::nested_kind(
                field,
                source_type,
                source_extension.as_ref(),
                &expected,
                safe,
                struct_policy,
                may_be_fully_hidden,
            )?
        };
        Ok(Self {
            field: field.clone(),
            source_type: source_type.clone(),
            expected,
            safe,
            null_policy,
            kind,
        })
    }

    #[allow(clippy::too_many_lines)] // Exhaustive Arrow wrapper dispatch is clearest together.
    fn nested_kind(
        field: &Field,
        source_type: &ArrowDataType,
        source_extension: Option<&RecognizedExtension>,
        expected: &ArrowDataType,
        safe: bool,
        struct_policy: StructPolicy,
        may_be_fully_hidden: bool,
    ) -> Result<ArrayCastKind> {
        let dtype = field.dtype();
        let kind = match (dtype, source_type) {
            // The extension-typed variants follow declared rules, never the
            // positional kernel: WKB is validated entering a geospatial
            // column, text is validated entering an ASCII width, WKT needs a
            // parser this workspace deliberately lacks, and a variant's
            // binary encoding lands with the Iceberg v3 layer, so only the
            // identity works until then.
            (DataType::Geometry(_) | DataType::Geography(_), source) => match source {
                ArrowDataType::Binary
                | ArrowDataType::LargeBinary
                | ArrowDataType::BinaryView
                | ArrowDataType::FixedSizeBinary(_) => ArrayCastKind::GeospatialIngest,
                ArrowDataType::Utf8 | ArrowDataType::LargeUtf8 | ArrowDataType::Utf8View => {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "casting text to {} would need the WKT parser this workspace \
                             deliberately does not have yet",
                            dtype.name()
                        ),
                    });
                }
                other => {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a binary column of WKB payloads to cast into {}, got {other:?}",
                            dtype.name()
                        ),
                    });
                }
            },
            (DataType::Variant, source) => {
                if crate::types::is_variant_storage(source) {
                    // The identity: physically the same two required binary
                    // children, reconciled to the canonical child spelling.
                    ArrayCastKind::Kernel
                } else {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "casting {source:?} to variant goes through the variant codec, \
                             which lands with the Iceberg v3 layer"
                        ),
                    });
                }
            }
            // An ASCII width takes fixed binary directly and everything the
            // kernel renders as text through one Utf8 temporary; the width
            // rule is checked per value either way.
            (DataType::Ascii | DataType::FixedAscii(_), source) => {
                if matches!(source, ArrowDataType::FixedSizeBinary(_))
                    || can_cast_types(source, &ArrowDataType::Utf8)
                {
                    ArrayCastKind::AsciiIngest
                } else {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a fixed binary or a column Arrow renders as utf8 to cast \
                             into {}, got {source:?}",
                            dtype.name()
                        ),
                    });
                }
            }
            // A code takes the same two sources as a width, and refuses the
            // same third, at the width its own type fixes.
            (DataType::Country | DataType::Currency | DataType::Mic | DataType::Cfi, source) => {
                if matches!(source, ArrowDataType::FixedSizeBinary(_))
                    || can_cast_types(source, &ArrowDataType::Utf8)
                {
                    ArrayCastKind::CodeIngest
                } else {
                    return Err(Error::Unsupported {
                        kind: dtype.name(),
                        reason: format!(
                            "expected a fixed binary or a column Arrow renders as utf8 to cast \
                             into {}, got {source:?}",
                            dtype.name()
                        ),
                    });
                }
            }
            (DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View, ArrowDataType::Binary)
                if matches!(source_extension, Some(RecognizedExtension::Geospatial(_))) =>
            {
                ArrayCastKind::GeospatialWkt
            }
            (
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
                ArrowDataType::Binary | ArrowDataType::FixedSizeBinary(_),
            ) if matches!(source_extension, Some(RecognizedExtension::Ascii(_))) => {
                ArrayCastKind::AsciiText
            }
            (
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
                ArrowDataType::FixedSizeBinary(_),
            ) if matches!(source_extension, Some(RecognizedExtension::Code(_))) => {
                ArrayCastKind::CodeText
            }
            // A UUID takes its sixteen bytes directly and every text spelling
            // through one Utf8 temporary; the one UUID rule runs per value
            // either way.
            (DataType::Uuid, source) => {
                if matches!(source, ArrowDataType::FixedSizeBinary(16))
                    || can_cast_types(source, &ArrowDataType::Utf8)
                {
                    ArrayCastKind::UuidIngest
                } else {
                    ArrayCastKind::DeferredUnsupported {
                        reason: format!("casting {source:?} to uuid is not supported"),
                    }
                }
            }
            (DataType::Version, source) if is_text_storage(source) => ArrayCastKind::VersionIngest,
            (DataType::Version, source) => ArrayCastKind::DeferredUnsupported {
                reason: format!("casting {source:?} to version is not supported"),
            },
            (
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
                ArrowDataType::FixedSizeBinary(16),
            ) if matches!(source_extension, Some(RecognizedExtension::Uuid)) => {
                ArrayCastKind::UuidText
            }
            (DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View, source)
                if is_temporal_arrow(source) =>
            {
                ArrayCastKind::TemporalText
            }
            // A temporal reads text with this crate's spellings rather than
            // Arrow's: a grouped fraction, an hour past the end of the day, a
            // bracketed zone name, and a duration in either spelling all read
            // here, and Arrow reads nothing into a duration at all. An
            // encoded column reads its values the same way and is encoded
            // afterwards, because the encoding is a layout, not a reading.
            (target, source) if holds_temporal(target) && holds_text(source) => {
                ArrayCastKind::TemporalIngest
            }
            (DataType::Struct(fields), ArrowDataType::Struct(source_fields)) => {
                let ArrowDataType::Struct(target_fields) = expected else {
                    return Err(internal_target_error("struct"));
                };
                let mapping = folded_field_mapping(source_fields, fields)?;
                let mut columns = Vec::with_capacity(fields.len());
                for (target_index, (target, source_index)) in fields.iter().zip(mapping).enumerate()
                {
                    let column = match source_index {
                        Some(index) => {
                            let null_policy = if matches!(struct_policy, StructPolicy::MapEntries)
                                && target_index == 0
                            {
                                NullPolicy::Reject
                            } else {
                                NullPolicy::Field
                            };
                            StructColumnPlan::Source {
                                index,
                                cast: Self::new_validated_with(
                                    target,
                                    source_fields[index].data_type(),
                                    Some(source_fields[index].metadata()),
                                    safe,
                                    null_policy,
                                    StructPolicy::Normal,
                                    true,
                                )?,
                            }
                        }
                        None if matches!(struct_policy, StructPolicy::MapEntries)
                            && target_index == 0 =>
                        {
                            return Err(Error::IncompatibleSchema(
                                "map key field is missing from the source entries Struct"
                                    .to_owned(),
                            ));
                        }
                        None => StructColumnPlan::Missing(target.clone()),
                    };
                    columns.push(column);
                }
                ArrayCastKind::Struct {
                    fields: target_fields.clone(),
                    columns,
                }
            }
            (DataType::List(child), ArrowDataType::List(source_child)) => ArrayCastKind::List {
                field: list_child(expected)?,
                child: Box::new(Self::new_nested_from_arrow_field(
                    child,
                    source_child,
                    safe,
                    NullPolicy::Field,
                )?),
                kind: ListPlanKind::List,
            },
            (
                DataType::LargeList(child) | DataType::List(child),
                ArrowDataType::LargeList(source_child),
            ) => ArrayCastKind::List {
                field: list_child(expected)?,
                child: Box::new(Self::new_nested_from_arrow_field(
                    child,
                    source_child,
                    safe,
                    NullPolicy::Field,
                )?),
                kind: ListPlanKind::LargeList,
            },
            (DataType::LargeList(child), ArrowDataType::List(source_child)) => {
                ArrayCastKind::List {
                    field: list_child(expected)?,
                    child: Box::new(Self::new_nested_from_arrow_field(
                        child,
                        source_child,
                        safe,
                        NullPolicy::Field,
                    )?),
                    kind: ListPlanKind::List,
                }
            }
            (
                DataType::ListView(child) | DataType::LargeListView(child),
                ArrowDataType::ListView(source_child),
            ) => ArrayCastKind::List {
                field: list_child(expected)?,
                child: Box::new(Self::new_nested_from_arrow_field(
                    child,
                    source_child,
                    safe,
                    NullPolicy::Field,
                )?),
                kind: ListPlanKind::ListView,
            },
            (
                DataType::LargeListView(child) | DataType::ListView(child),
                ArrowDataType::LargeListView(source_child),
            ) => ArrayCastKind::List {
                field: list_child(expected)?,
                child: Box::new(Self::new_nested_from_arrow_field(
                    child,
                    source_child,
                    safe,
                    NullPolicy::Field,
                )?),
                kind: ListPlanKind::LargeListView,
            },
            (
                DataType::FixedSizeList(child, size),
                ArrowDataType::FixedSizeList(source_child, source_size),
            ) if size == source_size => ArrayCastKind::List {
                field: list_child(expected)?,
                child: Box::new(Self::new_nested_from_arrow_field(
                    child,
                    source_child,
                    safe,
                    NullPolicy::Field,
                )?),
                kind: ListPlanKind::FixedSize { size: *size },
            },
            (DataType::Map(map), ArrowDataType::Map(source_entries, _)) => {
                let ArrowDataType::Map(target_entries, ordered) = expected else {
                    return Err(internal_target_error("map"));
                };
                let DataType::Map(source) = DataType::from_arrow(source_type)? else {
                    return Err(Error::IncompatibleSchema(
                        "source Arrow Map did not import as a Map datatype".to_owned(),
                    ));
                };
                ArrayCastKind::Map {
                    source,
                    field: Arc::clone(target_entries),
                    ordered: *ordered,
                    entries: Box::new(Self::new_validated_with(
                        map.entries(),
                        source_entries.data_type(),
                        Some(source_entries.metadata()),
                        safe,
                        NullPolicy::Reject,
                        StructPolicy::MapEntries,
                        true,
                    )?),
                }
            }
            (
                DataType::Dictionary(dictionary),
                ArrowDataType::Dictionary(source_key, source_value),
            ) => ArrayCastKind::Dictionary {
                source_key: source_key.as_ref().clone(),
                values: Box::new(Self::new_nested_validated(
                    &Field::new("values", dictionary.value().clone(), true),
                    source_value,
                    safe,
                    NullPolicy::Field,
                )?),
            },
            (DataType::Union(fields, mode), ArrowDataType::Union(source_fields, source_mode))
                if union_mode_matches(*mode, *source_mode) =>
            {
                if fields.len() != source_fields.len()
                    || source_fields
                        .iter()
                        .any(|(id, _)| !fields.iter().any(|(target_id, _)| target_id == id))
                {
                    return Err(Error::IncompatibleSchema(
                        "source and target union type-ID sets must match exactly".to_owned(),
                    ));
                }
                let ArrowDataType::Union(target_fields, _) = expected else {
                    return Err(internal_target_error("union"));
                };
                let mut children = Vec::with_capacity(fields.len());
                for (type_id, target) in fields.iter() {
                    let source = source_fields
                        .iter()
                        .find_map(|(id, field)| (id == type_id).then_some(field))
                        .ok_or_else(|| {
                            Error::IncompatibleSchema(format!(
                                "source union is missing target type ID {type_id}"
                            ))
                        })?;
                    children.push((
                        type_id,
                        Self::new_nested_from_arrow_field(target, source, safe, NullPolicy::Field)?,
                    ));
                }
                ArrayCastKind::Union {
                    fields: target_fields.clone(),
                    children,
                }
            }
            (
                DataType::RunEndEncoded(encoded),
                ArrowDataType::RunEndEncoded(source_runs, source_values),
            ) if source_runs.data_type()
                == encoded.run_ends().clone().into_arrow_ref()?.data_type() =>
            {
                ArrayCastKind::RunEndEncoded {
                    source_run_type: source_runs.data_type().clone(),
                    values: Box::new(Self::new_nested_from_arrow_field(
                        encoded.values(),
                        source_values,
                        safe,
                        NullPolicy::Field,
                    )?),
                }
            }
            _ if contains_struct(dtype) => {
                return Err(Error::Unsupported {
                    kind: dtype.name(),
                    reason: "a wrapper/layout change around Struct values is not supported because positional Arrow casting would bypass case-insensitive name reconciliation".to_owned(),
                });
            }
            // Anything Arrow's own kernel can cast, it casts - including the
            // wrapper and layout changes around non-Struct values: a list to
            // a view list, a fixed-size list to a variable one, a dictionary
            // encoded or decoded, a run-end array expanded. The Struct guard
            // above already refused the shapes where positional casting would
            // bypass name reconciliation, and the reservation walks nested
            // layouts, so the kernel's materialization stays budgeted.
            _ if can_cast_types(source_type, expected) => ArrayCastKind::Kernel,
            _ if may_be_fully_hidden => ArrayCastKind::DeferredUnsupported {
                reason: format!(
                    "Arrow cannot cast source datatype {source_type:?} to target datatype {expected:?}"
                ),
            },
            _ => {
                return Err(Error::Unsupported {
                    kind: dtype.name(),
                    reason: format!(
                        "Arrow cannot cast source datatype {source_type:?} to target datatype {expected:?}"
                    ),
                });
            }
        };
        Ok(kind)
    }

    fn cast(&self, array: ArrayRef, budget: &mut MaterializationBudget) -> Result<ArrayRef> {
        self.cast_exposed(array, None, budget)
    }

    #[allow(clippy::too_many_lines)] // Recursive Arrow dispatch and final null policy stay aligned.
    pub(crate) fn cast_exposed(
        &self,
        array: ArrayRef,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        if array.data_type() != &self.source_type {
            return Err(Error::IncompatibleSchema(format!(
                "array changed datatype after cast planning: expected {:?}, got {:?}",
                self.source_type,
                array.data_type()
            )));
        }
        if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
            return Err(Error::IncompatibleSchema(
                "nested Arrow exposure mask length differs from its child array".to_owned(),
            ));
        }
        let mut cast = match &self.kind {
            ArrayCastKind::Exact => array,
            ArrayCastKind::Kernel => arrow_cast_exposed(
                &array,
                &self.expected,
                self.safe,
                exposure,
                &self.field,
                budget,
            )?,
            ArrayCastKind::GeospatialIngest => {
                let binary = if array.data_type() == &ArrowDataType::Binary {
                    array
                } else {
                    arrow_cast_exposed(
                        &array,
                        &ArrowDataType::Binary,
                        self.safe,
                        exposure,
                        &self.field,
                        budget,
                    )?
                };
                validate_wkb_ingest(binary.as_ref(), &self.field, exposure)?;
                binary
            }
            ArrayCastKind::GeospatialWkt => {
                render_wkt_array(&array, &self.expected, &self.field, exposure, budget)?
            }
            ArrayCastKind::AsciiIngest => ingest_ascii_array(
                &array,
                &self.expected,
                self.safe,
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::UuidIngest => ingest_uuid_array(
                &array,
                &self.expected,
                self.safe,
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::UuidText => {
                render_uuid_text(&array, &self.expected, &self.field, exposure, budget)?
            }
            ArrayCastKind::VersionIngest => {
                ingest_version_array(&array, &self.field, exposure, budget)?
            }
            ArrayCastKind::AsciiText => {
                render_ascii_text(&array, &self.expected, &self.field, exposure, budget)?
            }
            // One match per array selects the code's width; every row after
            // it runs against a constant.
            ArrayCastKind::CodeIngest => match self.field.dtype() {
                DataType::Country => ingest_code_array::<COUNTRY_WIDTH>(
                    &array,
                    self.safe,
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Currency => ingest_code_array::<CURRENCY_WIDTH>(
                    &array,
                    self.safe,
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Mic => ingest_code_array::<MIC_WIDTH>(
                    &array,
                    self.safe,
                    &self.field,
                    exposure,
                    budget,
                )?,
                DataType::Cfi => ingest_code_array::<CFI_WIDTH>(
                    &array,
                    self.safe,
                    &self.field,
                    exposure,
                    budget,
                )?,
                other => return Err(code_refusal(other).into()),
            },
            // Rendering reads bytes out and never pads, so a code shares the
            // width's one implementation: the storage says the width, and
            // the recognizer already agreed it is the code's own.
            ArrayCastKind::CodeText => {
                render_ascii_text(&array, &self.expected, &self.field, exposure, budget)?
            }
            ArrayCastKind::TemporalText => {
                render_temporal_text(&array, self.safe, &self.field, exposure, budget)?
            }
            ArrayCastKind::TemporalIngest => ingest_temporal_text(
                &array,
                &self.expected,
                self.safe,
                &self.field,
                exposure,
                budget,
            )?,
            ArrayCastKind::DeferredUnsupported { reason } => {
                let source_type = DataType::from_arrow(array.data_type())?;
                let exposed = exposure.map_or(array.len(), BooleanBuffer::count_set_bits);
                let logical_nulls =
                    exposed_logical_null_count(array.as_ref(), &source_type, exposure)?;
                let has_visible_value = exposed > logical_nulls;
                if has_visible_value {
                    return Err(Error::Unsupported {
                        kind: self.field.dtype().name(),
                        reason: reason.clone(),
                    });
                }
                default_array(&self.field, array.len(), exposure, budget)?
            }
            ArrayCastKind::Struct { fields, columns } => {
                self.cast_struct_array(array, fields, columns, exposure, budget)?
            }
            ArrayCastKind::List { field, child, kind } => {
                self.cast_list_array(array, field, child, *kind, exposure, budget)?
            }
            ArrayCastKind::Map {
                source,
                field,
                ordered,
                entries,
            } => self.cast_map_array(array, source, field, *ordered, entries, exposure, budget)?,
            ArrayCastKind::Dictionary { source_key, values } => {
                cast_dictionary_planned(source_key, self, array, values, exposure, budget)?
            }
            ArrayCastKind::Union { fields, children } => {
                cast_union_planned(fields, array, children, exposure, budget)?
            }
            ArrayCastKind::RunEndEncoded {
                source_run_type,
                values,
            } => cast_run_planned(
                source_run_type,
                &self.expected,
                array,
                values,
                exposure,
                budget,
            )?,
        };
        if cast.data_type() != &self.expected {
            cast = arrow_cast_exposed(
                &cast,
                &self.expected,
                self.safe,
                exposure,
                &self.field,
                budget,
            )?;
        }
        let null_count = exposed_logical_null_count(cast.as_ref(), self.field.dtype(), exposure)?;
        match self.null_policy {
            NullPolicy::Reject if null_count != 0 => {
                return Err(Error::IncompatibleSchema(format!(
                    "field {:?} contains {null_count} logical null values",
                    self.field.name()
                )));
            }
            NullPolicy::DataType if null_count != 0 => {
                cast = fill_nulls(&self.field, cast, true, exposure, budget)?;
            }
            NullPolicy::Field if !self.field.is_nullable() && null_count != 0 => {
                cast = fill_nulls(&self.field, cast, false, exposure, budget)?;
            }
            NullPolicy::Field | NullPolicy::DataType | NullPolicy::Reject => {}
        }
        if cast.data_type() != &self.expected {
            return Err(Error::IncompatibleSchema(format!(
                "cast result datatype {:?} differs from target {:?}",
                cast.data_type(),
                self.expected
            )));
        }
        let remaining_nulls =
            if matches!(self.null_policy, NullPolicy::Field | NullPolicy::DataType)
                && (matches!(self.null_policy, NullPolicy::DataType) || !self.field.is_nullable())
            {
                exposed_logical_null_count(cast.as_ref(), self.field.dtype(), exposure)?
            } else {
                0
            };
        if remaining_nulls != 0 {
            let default = if matches!(self.null_policy, NullPolicy::DataType) {
                self.field.dtype().default_arrow_array()?
            } else {
                self.field.default_arrow_array()?
            };
            if !is_logically_null(default.as_ref(), 0) {
                return Err(Error::IncompatibleSchema(format!(
                    "required field {:?} still contains {remaining_nulls} logical null values after default filling",
                    self.field.name(),
                )));
            }
        }
        Ok(cast)
    }
}

/// Casts an array into the shape a field declares.
///
/// `source_metadata` is the Arrow metadata of the field the array came from,
/// when the caller has one: it carries the extension identity, so a
/// recognized ASCII width or geospatial column follows the declared rules
/// exactly as a batch column does. A bare array casts as its storage.
pub(crate) fn cast_field_array(
    field: &Field,
    source_metadata: Option<&HashMap<String, String>>,
    array: ArrayRef,
    safe: bool,
) -> Result<ArrayRef> {
    field.validate_bounded()?;
    let plan = ArrayCastPlan::new_validated_with(
        field,
        array.data_type(),
        source_metadata,
        safe,
        NullPolicy::Field,
        StructPolicy::Normal,
        true,
    )?;
    let mut budget = MaterializationBudget::default();
    plan.cast(array, &mut budget)
}

/// Enforces the declared rules a recognized extension source adds to a cast.
///
/// A variant crosses only to a variant until the codec lands with the
/// Iceberg v3 layer, and a geospatial source refuses a CRS or an
/// edge-interpretation change by name: both are value transformations, not
/// schema casts. A matching geospatial pair, and every plain-storage target
/// (bytes stay bytes, text renders as WKT), passes through to the planned
/// arms. An ASCII source is validated text and crosses to every target:
/// another width re-validates, text trims, bytes keep the padding.
fn check_extension_source(target: &Field, source: Option<&RecognizedExtension>) -> Result<()> {
    let Some(source) = source else {
        return Ok(());
    };
    match (target.dtype(), source) {
        (DataType::Variant, RecognizedExtension::Variant) => Ok(()),
        (_, RecognizedExtension::Ascii(_) | RecognizedExtension::Code(_)) => Ok(()),
        // A UUID source is sixteen bytes: a UUID target re-validates them,
        // text renders them, and bytes keep them.
        (_, RecognizedExtension::Uuid) => Ok(()),
        (
            DataType::Version | DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View,
            RecognizedExtension::Version,
        ) => Ok(()),
        (other, RecognizedExtension::Version) => Err(Error::Unsupported {
            kind: "version",
            reason: format!("casting version to {} is not supported", other.name()),
        }),
        (other, RecognizedExtension::Variant) => Err(Error::Unsupported {
            kind: "variant",
            reason: format!(
                "casting variant to {} goes through the variant codec, which lands \
                 with the Iceberg v3 layer",
                other.name()
            ),
        }),
        (
            DataType::Geometry(target_geospatial) | DataType::Geography(target_geospatial),
            RecognizedExtension::Geospatial(source_geospatial),
        ) => {
            let target_kind = target.dtype().name();
            match (target_geospatial.algorithm(), source_geospatial.algorithm()) {
                (None, Some(algorithm)) => Err(Error::Unsupported {
                    kind: target_kind,
                    reason: format!(
                        "expected planar geometry edges, got geography {algorithm} edges; \
                         the edge interpretation change is a value transformation, not a \
                         schema cast"
                    ),
                }),
                (Some(algorithm), None) => Err(Error::Unsupported {
                    kind: target_kind,
                    reason: format!(
                        "expected geography {algorithm} edges, got planar geometry edges; \
                         the edge interpretation change is a value transformation, not a \
                         schema cast"
                    ),
                }),
                (Some(target_algorithm), Some(source_algorithm))
                    if target_algorithm != source_algorithm =>
                {
                    Err(Error::Unsupported {
                        kind: target_kind,
                        reason: format!(
                            "expected geography {target_algorithm} edges, got \
                             {source_algorithm} edges; the edge interpretation change is \
                             a value transformation, not a schema cast"
                        ),
                    })
                }
                _ if target_geospatial.crs() != source_geospatial.crs() => {
                    Err(Error::Unsupported {
                        kind: target_kind,
                        reason: format!(
                            "expected CRS {:?}, got CRS {:?}; a CRS change is a coordinate \
                             transformation, not a schema cast",
                            target_geospatial.crs(),
                            source_geospatial.crs()
                        ),
                    })
                }
                _ => Ok(()),
            }
        }
        (_, RecognizedExtension::Geospatial(_)) => Ok(()),
    }
}

/// Names the field and the row on a refused cell.
pub(crate) fn named_cell<T>(field: &Field, index: usize, read: crate::Result<T>) -> Result<T> {
    read.map_err(|error| {
        let reason = match error {
            crate::Error::InvalidRecord { reason, .. } => reason.to_string(),
            other => other.to_string(),
        };
        Error::IncompatibleSchema(format!("field {:?} row {index}: {reason}", field.name()))
    })
}

pub(crate) fn downcast<T: Array + 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "Arrow array implementation does not match datatype {:?}",
            array.data_type()
        ))
    })
}

pub(crate) fn internal_target_error(kind: &'static str) -> Error {
    Error::Unsupported {
        kind,
        reason: "validated target projected an unexpected Arrow datatype".to_owned(),
    }
}

#[cfg(test)]
mod tests;
