//! Schema-directed Arrow array and record-batch reconciliation.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{DataType, Field, UnionMode, Value};
use arrow_array::types::{
    ArrowDictionaryKeyType, Int8Type, Int16Type, Int32Type, Int64Type, RunEndIndexType, UInt8Type,
    UInt16Type, UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BinaryArray, BinaryViewArray, BooleanArray, Decimal256Array, DictionaryArray,
    FixedSizeBinaryArray, FixedSizeListArray, Float16Array, Float32Array, Float64Array,
    Int16RunArray, Int32RunArray, Int64RunArray, LargeBinaryArray, LargeListArray,
    LargeListViewArray, LargeStringArray, ListArray, ListViewArray, MapArray, PrimitiveArray,
    RecordBatch, RecordBatchOptions, RunArray, Scalar, StringArray, StringViewArray, StructArray,
    UInt32Array, UnionArray, make_array, new_null_array,
};
use arrow_buffer::{ArrowNativeType, BooleanBuffer, BooleanBufferBuilder, i256};
use arrow_cast::display::{ArrayFormatter, FormatOptions};
use arrow_cast::{CastOptions, can_cast_types, cast_with_options};
use arrow_ord::ord::{DynComparator, make_comparator};
use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef, SortOptions};
use arrow_select::{concat::concat, take::take, zip::zip};

use crate::arrow::value::MaterializationBudget;
use crate::arrow::{DefaultArrowScalar, Error, Result};

/// Arrow array and record-batch casting owned by a canonical Yggdryl schema.
///
/// This extension trait lives in [`crate::arrow`], keeping recursive runtime
/// casts behind Yggdryl's `arrow` feature while retaining method syntax on
/// [`DataType`] and [`Field`].
pub trait ArrowCast {
    /// Casts an Arrow array to this exact physical datatype.
    ///
    /// `safe` is passed directly to Arrow's [`CastOptions`]. With Arrow 59,
    /// supported conversion failures become null when it is true and are
    /// errors when it is false. A non-nullable target Field replaces resulting
    /// nulls with its canonical default; a nullable Field retains them.
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
}

impl ArrowCast for DataType {
    fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<ArrayRef> {
        let plan = ArrayCastPlan::new_data_type(self, array.data_type(), safe)?;
        let mut budget = MaterializationBudget::default();
        plan.cast(array, &mut budget)
    }

    fn cast_arrow_batch(&self, batch: RecordBatch, safe: bool) -> Result<RecordBatch> {
        Field::new("record", self.clone(), false).cast_arrow_batch(batch, safe)
    }
}

impl ArrowCast for Field {
    fn cast_arrow_array(&self, array: ArrayRef, safe: bool) -> Result<ArrayRef> {
        self.validate_bounded()?;
        cast_field_array(self, array, safe)
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
    if target.data_type().as_fields().is_none() {
        return Err(Error::IncompatibleSchema(format!(
            "record-batch cast target {:?} must have a struct datatype",
            target.name()
        )));
    }

    let row_count = batch.num_rows();
    let source = struct_array_from_batch(batch);
    let cast = cast_field_array(target, source, safe)?;
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
    let schema = crate::arrow::schema_from_field(target)?;
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

struct ArrayCastPlan {
    field: Field,
    source_type: ArrowDataType,
    expected: ArrowDataType,
    safe: bool,
    null_policy: NullPolicy,
    kind: ArrayCastKind,
}

enum ArrayCastKind {
    Exact,
    Kernel,
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

enum StructColumnPlan {
    Source { index: usize, cast: ArrayCastPlan },
    Missing(Field),
}

#[derive(Clone, Copy)]
enum ListPlanKind {
    List,
    LargeList,
    ListView,
    LargeListView,
    FixedSize { size: i32 },
}

impl ArrayCastPlan {
    fn new_data_type(
        data_type: &DataType,
        source_type: &ArrowDataType,
        safe: bool,
    ) -> Result<Self> {
        data_type.validate_bounded()?;
        Self::new_nested_validated(
            &Field::new("value", data_type.clone(), false),
            source_type,
            safe,
            NullPolicy::DataType,
        )
    }

    fn new(field: &Field, source_type: &ArrowDataType, safe: bool) -> Result<Self> {
        field.validate_bounded()?;
        Self::new_nested_validated(field, source_type, safe, NullPolicy::Field)
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
            safe,
            null_policy,
            StructPolicy::Normal,
            true,
        )
    }

    fn new_validated_with(
        field: &Field,
        source_type: &ArrowDataType,
        safe: bool,
        null_policy: NullPolicy,
        struct_policy: StructPolicy,
        may_be_fully_hidden: bool,
    ) -> Result<Self> {
        let expected = field.to_arrow_ref()?.data_type().clone();
        let kind = if source_type == &expected && !is_reconcilable_nested(field.data_type()) {
            ArrayCastKind::Exact
        } else {
            Self::nested_kind(
                field,
                source_type,
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
        expected: &ArrowDataType,
        safe: bool,
        struct_policy: StructPolicy,
        may_be_fully_hidden: bool,
    ) -> Result<ArrayCastKind> {
        let data_type = field.data_type();
        let kind = match (data_type, source_type) {
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
                                cast: Self::new_nested_validated(
                                    target,
                                    source_fields[index].data_type(),
                                    safe,
                                    null_policy,
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
                child: Box::new(Self::new_nested_validated(
                    child,
                    source_child.data_type(),
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
                child: Box::new(Self::new_nested_validated(
                    child,
                    source_child.data_type(),
                    safe,
                    NullPolicy::Field,
                )?),
                kind: ListPlanKind::LargeList,
            },
            (DataType::LargeList(child), ArrowDataType::List(source_child)) => {
                ArrayCastKind::List {
                    field: list_child(expected)?,
                    child: Box::new(Self::new_nested_validated(
                        child,
                        source_child.data_type(),
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
                child: Box::new(Self::new_nested_validated(
                    child,
                    source_child.data_type(),
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
                child: Box::new(Self::new_nested_validated(
                    child,
                    source_child.data_type(),
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
                child: Box::new(Self::new_nested_validated(
                    child,
                    source_child.data_type(),
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
                        Self::new_nested_validated(
                            target,
                            source.data_type(),
                            safe,
                            NullPolicy::Field,
                        )?,
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
            ) if source_runs.data_type() == encoded.run_ends().to_arrow_ref()?.data_type() => {
                ArrayCastKind::RunEndEncoded {
                    source_run_type: source_runs.data_type().clone(),
                    values: Box::new(Self::new_nested_validated(
                        encoded.values(),
                        source_values.data_type(),
                        safe,
                        NullPolicy::Field,
                    )?),
                }
            }
            _ if contains_struct(data_type) => {
                return Err(Error::Unsupported {
                    kind: data_type.name(),
                    reason: "a wrapper/layout change around Struct values is not supported because positional Arrow casting would bypass case-insensitive name reconciliation".to_owned(),
                });
            }
            _ if can_cast_types(source_type, expected)
                && generic_kernel_materialization_is_bounded(source_type, data_type)? =>
            {
                ArrayCastKind::Kernel
            }
            _ if may_be_fully_hidden => ArrayCastKind::DeferredUnsupported {
                reason: format!(
                    "Arrow cannot cast source datatype {source_type:?} to target datatype {expected:?}"
                ),
            },
            _ => {
                return Err(Error::Unsupported {
                    kind: data_type.name(),
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
    fn cast_exposed(
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
            ArrayCastKind::DeferredUnsupported { reason } => {
                let source_type = DataType::from_arrow(array.data_type())?;
                let exposed = exposure.map_or(array.len(), BooleanBuffer::count_set_bits);
                let logical_nulls =
                    exposed_logical_null_count(array.as_ref(), &source_type, exposure)?;
                let has_visible_value = exposed > logical_nulls;
                if has_visible_value {
                    return Err(Error::Unsupported {
                        kind: self.field.data_type().name(),
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
        let null_count =
            exposed_logical_null_count(cast.as_ref(), self.field.data_type(), exposure)?;
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
                exposed_logical_null_count(cast.as_ref(), self.field.data_type(), exposure)?
            } else {
                0
            };
        if remaining_nulls != 0 {
            let default = if matches!(self.null_policy, NullPolicy::DataType) {
                self.field.data_type().default_arrow_scalar()?
            } else {
                self.field.default_arrow_scalar()?
            };
            if !is_logically_null(default.array().as_ref(), 0) {
                return Err(Error::IncompatibleSchema(format!(
                    "required field {:?} still contains {remaining_nulls} logical null values after default filling",
                    self.field.name(),
                )));
            }
        }
        Ok(cast)
    }

    fn cast_struct_array(
        &self,
        array: ArrayRef,
        fields: &arrow_schema::Fields,
        columns: &[StructColumnPlan],
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source = downcast::<StructArray>(&array)?;
        let child_exposure = visible_array_exposure(source, exposure, budget)?;
        let mut output = Vec::with_capacity(columns.len());
        let mut unchanged = self.source_type == self.expected;
        for column in columns {
            output.push(match column {
                StructColumnPlan::Source { index, cast } => {
                    let source_column = source.column(*index);
                    let output = cast.cast_exposed(
                        Arc::clone(source_column),
                        child_exposure.as_ref(),
                        budget,
                    )?;
                    unchanged &=
                        output.len() == source_column.len() && Arc::ptr_eq(&output, source_column);
                    output
                }
                StructColumnPlan::Missing(field) => {
                    unchanged = false;
                    default_array(field, source.len(), child_exposure.as_ref(), budget)?
                }
            });
        }
        if unchanged {
            return Ok(array);
        }
        Ok(Arc::new(StructArray::try_new_with_length(
            fields.clone(),
            output,
            source.nulls().cloned(),
            source.len(),
        )?))
    }

    #[allow(clippy::too_many_lines)] // Keep the five Arrow list layouts behaviorally aligned.
    fn cast_list_array(
        &self,
        array: ArrayRef,
        field: &ArrowFieldRef,
        child: &ArrayCastPlan,
        kind: ListPlanKind,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        Ok(match kind {
            ListPlanKind::List => {
                let source = downcast::<ListArray>(&array)?;
                let child_exposure = range_exposure(
                    source.values().len(),
                    source.len(),
                    exposure,
                    |row| source.is_valid(row),
                    |row| {
                        let offsets = source.value_offsets();
                        offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))
                    },
                    budget,
                )?;
                let values = child.cast_exposed(
                    Arc::clone(source.values()),
                    child_exposure.as_ref(),
                    budget,
                )?;
                let values = ensure_list_child_physical(&child.field, values, budget)?;
                if self.source_type == self.expected && Arc::ptr_eq(&values, source.values()) {
                    return Ok(array);
                }
                Arc::new(ListArray::try_new(
                    Arc::clone(field),
                    source.offsets().clone(),
                    values,
                    source.nulls().cloned(),
                )?) as ArrayRef
            }
            ListPlanKind::LargeList => {
                let source = downcast::<LargeListArray>(&array)?;
                let child_exposure = range_exposure(
                    source.values().len(),
                    source.len(),
                    exposure,
                    |row| source.is_valid(row),
                    |row| {
                        let offsets = source.value_offsets();
                        offset_pair(offsets[row], offsets[row + 1])
                    },
                    budget,
                )?;
                let values = child.cast_exposed(
                    Arc::clone(source.values()),
                    child_exposure.as_ref(),
                    budget,
                )?;
                let values = ensure_list_child_physical(&child.field, values, budget)?;
                if self.source_type == self.expected && Arc::ptr_eq(&values, source.values()) {
                    return Ok(array);
                }
                Arc::new(LargeListArray::try_new(
                    Arc::clone(field),
                    source.offsets().clone(),
                    values,
                    source.nulls().cloned(),
                )?)
            }
            ListPlanKind::ListView => {
                let source = downcast::<ListViewArray>(&array)?;
                let child_exposure = range_exposure(
                    source.values().len(),
                    source.len(),
                    exposure,
                    |row| source.is_valid(row),
                    |row| {
                        offset_size(
                            i64::from(source.value_offsets()[row]),
                            i64::from(source.value_sizes()[row]),
                        )
                    },
                    budget,
                )?;
                let values = child.cast_exposed(
                    Arc::clone(source.values()),
                    child_exposure.as_ref(),
                    budget,
                )?;
                let values = ensure_list_child_physical(&child.field, values, budget)?;
                if self.source_type == self.expected && Arc::ptr_eq(&values, source.values()) {
                    return Ok(array);
                }
                Arc::new(ListViewArray::try_new(
                    Arc::clone(field),
                    source.offsets().clone(),
                    source.sizes().clone(),
                    values,
                    source.nulls().cloned(),
                )?)
            }
            ListPlanKind::LargeListView => {
                let source = downcast::<LargeListViewArray>(&array)?;
                let child_exposure = range_exposure(
                    source.values().len(),
                    source.len(),
                    exposure,
                    |row| source.is_valid(row),
                    |row| offset_size(source.value_offsets()[row], source.value_sizes()[row]),
                    budget,
                )?;
                let values = child.cast_exposed(
                    Arc::clone(source.values()),
                    child_exposure.as_ref(),
                    budget,
                )?;
                let values = ensure_list_child_physical(&child.field, values, budget)?;
                if self.source_type == self.expected && Arc::ptr_eq(&values, source.values()) {
                    return Ok(array);
                }
                Arc::new(LargeListViewArray::try_new(
                    Arc::clone(field),
                    source.offsets().clone(),
                    source.sizes().clone(),
                    values,
                    source.nulls().cloned(),
                )?)
            }
            ListPlanKind::FixedSize { size } => {
                let source = downcast::<FixedSizeListArray>(&array)?;
                let child_exposure = range_exposure(
                    source.values().len(),
                    source.len(),
                    exposure,
                    |row| source.is_valid(row),
                    |row| offset_size(i64::from(source.value_offset(row)), i64::from(size)),
                    budget,
                )?;
                let values = child.cast_exposed(
                    Arc::clone(source.values()),
                    child_exposure.as_ref(),
                    budget,
                )?;
                let values = ensure_list_child_physical(&child.field, values, budget)?;
                if self.source_type == self.expected && Arc::ptr_eq(&values, source.values()) {
                    return Ok(array);
                }
                Arc::new(FixedSizeListArray::try_new_with_length(
                    Arc::clone(field),
                    size,
                    values,
                    source.nulls().cloned(),
                    source.len(),
                )?)
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn cast_map_array(
        &self,
        array: ArrayRef,
        source_map: &crate::MapType,
        field: &ArrowFieldRef,
        ordered: bool,
        entries: &ArrayCastPlan,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source = downcast::<MapArray>(&array)?;
        validate_map_invariants(source_map, source, exposure, budget)?;
        if self.source_type == self.expected
            && !(0..source.len()).any(|row| is_exposed(exposure, row) && source.is_valid(row))
        {
            return Ok(array);
        }
        let entry_exposure = range_exposure(
            source.entries().len(),
            source.len(),
            exposure,
            |row| source.is_valid(row),
            |row| {
                let offsets = source.value_offsets();
                offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))
            },
            budget,
        )?;
        let source_entries = Arc::new(source.entries().clone()) as ArrayRef;
        let entries =
            entries.cast_exposed(Arc::clone(&source_entries), entry_exposure.as_ref(), budget)?;
        let unchanged = self.source_type == self.expected && Arc::ptr_eq(&entries, &source_entries);
        let output = if unchanged {
            array
        } else {
            let entries = downcast::<StructArray>(&entries)?.clone();
            Arc::new(MapArray::try_new(
                Arc::clone(field),
                source.offsets().clone(),
                entries,
                source.nulls().cloned(),
                ordered,
            )?) as ArrayRef
        };
        let DataType::Map(target_map) = self.field.data_type() else {
            return Err(internal_target_error("map"));
        };
        if !unchanged || source_map != target_map.as_ref() {
            validate_map_invariants(target_map, output.as_ref(), exposure, budget)?;
        }
        Ok(output)
    }
}

fn cast_field_array(field: &Field, array: ArrayRef, safe: bool) -> Result<ArrayRef> {
    let plan = ArrayCastPlan::new(field, array.data_type(), safe)?;
    let mut budget = MaterializationBudget::default();
    plan.cast(array, &mut budget)
}

fn validate_map_invariants(
    map: &crate::MapType,
    array: &dyn Array,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let array = downcast::<MapArray>(array)?;
    let phase = budget.mark();
    let keys = array.entries().column(0);
    let Some([key_field, _]) = map.entries().data_type().as_fields() else {
        return Err(Error::IncompatibleSchema(
            "map entries must contain key and value fields".to_owned(),
        ));
    };
    let compare = make_yggdryl_key_comparator(key_field.data_type(), keys, budget)?;
    let offsets = array.value_offsets();

    let mut maximum_row_len = 0usize;
    for row in 0..array.len() {
        if !is_exposed(exposure, row) || array.is_null(row) {
            continue;
        }
        let (start, end) = offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))?;
        maximum_row_len = maximum_row_len.max(end.saturating_sub(start));
    }

    // Small maps are faster with direct comparisons. Wide rows share one
    // allocation across the complete array instead of creating a map, schema,
    // and Record for every logical row.
    let mut ordered_indices = Vec::new();
    if maximum_row_len > 16 {
        budget.add_array(&DataType::UInt64, maximum_row_len)?;
        ordered_indices
            .try_reserve_exact(maximum_row_len)
            .map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "map-key validation scratch allocation failed: {error}"
                ))
            })?;
    }
    for row in 0..array.len() {
        if !is_exposed(exposure, row) || array.is_null(row) {
            continue;
        }
        let (start, end) = offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1]))?;
        for index in start..end {
            if logical_null_at(keys.as_ref(), key_field.data_type(), index)? {
                return Err(Error::IncompatibleSchema(format!(
                    "map row {row} has a null key at entry {}",
                    index - start
                )));
            }
        }
        if end - start <= 16 {
            for index in (start + 1)..end {
                if (start..index).any(|previous| compare(previous, index) == Ordering::Equal) {
                    return Err(Error::IncompatibleSchema(format!(
                        "map row {row} has a duplicate key at entry {}",
                        index - start
                    )));
                }
            }
        } else {
            ordered_indices.clear();
            ordered_indices.extend(start..end);
            ordered_indices.sort_unstable_by(|left, right| compare(*left, *right));
            if ordered_indices
                .windows(2)
                .any(|pair| compare(pair[0], pair[1]) == Ordering::Equal)
            {
                return Err(Error::IncompatibleSchema(format!(
                    "map row {row} has duplicate keys"
                )));
            }
        }

        if map.keys_sorted()
            && (start..end.saturating_sub(1))
                .any(|index| compare(index, index + 1) == Ordering::Greater)
        {
            return Err(Error::IncompatibleSchema(format!(
                "map row {row} declares sorted keys but values are not ordered"
            )));
        }
    }
    budget.restore(phase);
    Ok(())
}

fn requires_yggdryl_key_comparator(data_type: &DataType) -> bool {
    match data_type {
        DataType::Float16
        | DataType::Float32
        | DataType::Float64
        | DataType::Decimal256 { .. }
        | DataType::Union(..)
        | DataType::Dictionary(_)
        | DataType::RunEndEncoded(_) => true,
        DataType::List(child)
        | DataType::ListView(child)
        | DataType::FixedSizeList(child, _)
        | DataType::LargeList(child)
        | DataType::LargeListView(child) => requires_yggdryl_key_comparator(child.data_type()),
        DataType::Struct(fields) => fields
            .iter()
            .any(|field| requires_yggdryl_key_comparator(field.data_type())),
        DataType::Map(map) => requires_yggdryl_key_comparator(map.entries().data_type()),
        _ => false,
    }
}

fn has_derived_logical_nulls(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Null | DataType::Dictionary(_) | DataType::Union(..) | DataType::RunEndEncoded(_)
    )
}

fn wrap_yggdryl_nulls(
    left: &ArrayRef,
    right: &ArrayRef,
    data_type: &DataType,
    compare: DynComparator,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    if has_derived_logical_nulls(data_type) {
        budget.add_bitmap(left.len())?;
        if !Arc::ptr_eq(left, right) {
            budget.add_bitmap(right.len())?;
        }
    }
    let left_nulls = left.logical_nulls();
    let right_nulls = if Arc::ptr_eq(left, right) {
        left_nulls.clone()
    } else {
        right.logical_nulls()
    };
    if left_nulls.is_none() && right_nulls.is_none() {
        return Ok(compare);
    }
    Ok(Box::new(move |left, right| {
        let left_null = left_nulls.as_ref().is_some_and(|nulls| nulls.is_null(left));
        let right_null = right_nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(right));
        match (left_null, right_null) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => compare(left, right),
        }
    }))
}

struct DecimalText {
    bytes: [u8; 78],
    start: usize,
}

impl DecimalText {
    fn new(value: i256) -> Self {
        let negative = value.is_negative();
        let mut raw = value.to_le_bytes();
        if negative {
            let mut carry = true;
            for byte in &mut raw {
                *byte = !*byte;
                if carry {
                    let (next, overflow) = byte.overflowing_add(1);
                    *byte = next;
                    carry = overflow;
                }
            }
        }
        let mut limbs = [0_u64; 4];
        for (limb, chunk) in limbs.iter_mut().zip(raw.chunks_exact(8)) {
            let mut bytes = [0_u8; 8];
            bytes.copy_from_slice(chunk);
            *limb = u64::from_le_bytes(bytes);
        }

        let mut bytes = [0_u8; 78];
        let mut start = bytes.len();
        loop {
            let mut remainder = 0_u128;
            for limb in limbs.iter_mut().rev() {
                let value = (remainder << 64) | u128::from(*limb);
                *limb = u64::try_from(value / 10).unwrap_or(u64::MAX);
                remainder = value % 10;
            }
            start -= 1;
            bytes[start] = b'0' + u8::try_from(remainder).unwrap_or(0);
            if limbs.iter().all(|limb| *limb == 0) {
                break;
            }
        }
        if negative {
            start -= 1;
            bytes[start] = b'-';
        }
        Self { bytes, start }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[self.start..]
    }
}

fn dictionary_key_comparator<K: ArrowDictionaryKeyType>(
    left: &ArrayRef,
    right: &ArrayRef,
    dictionary: &crate::DictionaryType,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    let left_source = downcast::<DictionaryArray<K>>(left.as_ref())?;
    let right_source = downcast::<DictionaryArray<K>>(right.as_ref())?;
    let left_values = Arc::clone(left_source.values());
    let right_values = Arc::clone(right_source.values());
    let value_compare =
        make_yggdryl_comparator(dictionary.value(), &left_values, &right_values, budget)?;
    let left_keys = left_source.keys().values().clone();
    let right_keys = right_source.keys().values().clone();
    let left_nulls = left_source.keys().nulls().cloned();
    let right_nulls = right_source.keys().nulls().cloned();
    Ok(Box::new(move |left, right| {
        let left_null = left_nulls.as_ref().is_some_and(|nulls| nulls.is_null(left));
        let right_null = right_nulls
            .as_ref()
            .is_some_and(|nulls| nulls.is_null(right));
        match (left_null, right_null) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => {
                value_compare(left_keys[left].as_usize(), right_keys[right].as_usize())
            }
        }
    }))
}

fn run_key_comparator<R: RunEndIndexType>(
    left: &ArrayRef,
    right: &ArrayRef,
    encoded: &crate::RunEndEncodedType,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    let left_source = downcast::<RunArray<R>>(left.as_ref())?;
    let right_source = downcast::<RunArray<R>>(right.as_ref())?;
    let left_values = Arc::clone(left_source.values());
    let right_values = Arc::clone(right_source.values());
    let value_compare = make_yggdryl_comparator(
        encoded.values().data_type(),
        &left_values,
        &right_values,
        budget,
    )?;
    let left_run_ends = left_source.run_ends().clone();
    let right_run_ends = right_source.run_ends().clone();
    Ok(Box::new(move |left, right| {
        value_compare(
            left_run_ends.get_physical_index(left),
            right_run_ends.get_physical_index(right),
        )
    }))
}

#[allow(clippy::too_many_lines)] // Recurses only where native Value ordering differs from Arrow.
fn make_yggdryl_key_comparator(
    data_type: &DataType,
    array: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    make_yggdryl_comparator(data_type, array, array, budget)
}

#[allow(clippy::too_many_lines)] // Recurses only where native Value ordering differs from Arrow.
fn make_yggdryl_comparator(
    data_type: &DataType,
    left: &ArrayRef,
    right: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    if !requires_yggdryl_key_comparator(data_type) {
        if has_derived_logical_nulls(data_type) {
            budget.add_bitmap(left.len())?;
            if !Arc::ptr_eq(left, right) {
                budget.add_bitmap(right.len())?;
            }
        }
        return make_comparator(left.as_ref(), right.as_ref(), SortOptions::default())
            .map_err(Into::into);
    }

    let compare: DynComparator = match data_type {
        DataType::Float16 => {
            let left_values = downcast::<Float16Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Float16Array>(right.as_ref())?.values().clone();
            Box::new(move |left, right| {
                crate::Float::from_f64(left_values[left].to_f64())
                    .cmp(&crate::Float::from_f64(right_values[right].to_f64()))
            })
        }
        DataType::Float32 => {
            let left_values = downcast::<Float32Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Float32Array>(right.as_ref())?.values().clone();
            Box::new(move |left, right| {
                crate::Float::from_f64(f64::from(left_values[left]))
                    .cmp(&crate::Float::from_f64(f64::from(right_values[right])))
            })
        }
        DataType::Float64 => {
            let left_values = downcast::<Float64Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Float64Array>(right.as_ref())?.values().clone();
            Box::new(move |left, right| {
                crate::Float::from_f64(left_values[left])
                    .cmp(&crate::Float::from_f64(right_values[right]))
            })
        }
        DataType::Decimal256 { .. } => {
            let left_values = downcast::<Decimal256Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Decimal256Array>(right.as_ref())?
                .values()
                .clone();
            Box::new(move |left, right| {
                DecimalText::new(left_values[left])
                    .as_bytes()
                    .cmp(DecimalText::new(right_values[right]).as_bytes())
            })
        }
        DataType::List(child) => {
            let left_source = downcast::<ListArray>(left.as_ref())?;
            let right_source = downcast::<ListArray>(right.as_ref())?;
            let left_offsets = left_source.offsets().clone();
            let right_offsets = right_source.offsets().clone();
            let left_values = Arc::clone(left_source.values());
            let right_values = Arc::clone(right_source.values());
            let child_compare =
                make_yggdryl_comparator(child.data_type(), &left_values, &right_values, budget)?;
            Box::new(move |left, right| {
                let left = left_offsets[left].as_usize()..left_offsets[left + 1].as_usize();
                let right = right_offsets[right].as_usize()..right_offsets[right + 1].as_usize();
                for (left, right) in left.clone().zip(right.clone()) {
                    let ordering = child_compare(left, right);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                left.len().cmp(&right.len())
            })
        }
        DataType::LargeList(child) => {
            let left_source = downcast::<LargeListArray>(left.as_ref())?;
            let right_source = downcast::<LargeListArray>(right.as_ref())?;
            let left_offsets = left_source.offsets().clone();
            let right_offsets = right_source.offsets().clone();
            let left_values = Arc::clone(left_source.values());
            let right_values = Arc::clone(right_source.values());
            let child_compare =
                make_yggdryl_comparator(child.data_type(), &left_values, &right_values, budget)?;
            Box::new(move |left, right| {
                let left = left_offsets[left].as_usize()..left_offsets[left + 1].as_usize();
                let right = right_offsets[right].as_usize()..right_offsets[right + 1].as_usize();
                for (left, right) in left.clone().zip(right.clone()) {
                    let ordering = child_compare(left, right);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                left.len().cmp(&right.len())
            })
        }
        DataType::ListView(child) => {
            let left_source = downcast::<ListViewArray>(left.as_ref())?;
            let right_source = downcast::<ListViewArray>(right.as_ref())?;
            let left_offsets = left_source.offsets().clone();
            let right_offsets = right_source.offsets().clone();
            let left_sizes = left_source.sizes().clone();
            let right_sizes = right_source.sizes().clone();
            let left_values = Arc::clone(left_source.values());
            let right_values = Arc::clone(right_source.values());
            let child_compare =
                make_yggdryl_comparator(child.data_type(), &left_values, &right_values, budget)?;
            Box::new(move |left, right| {
                let left_start = left_offsets[left].as_usize();
                let right_start = right_offsets[right].as_usize();
                let left_len = left_sizes[left].as_usize();
                let right_len = right_sizes[right].as_usize();
                for offset in 0..left_len.min(right_len) {
                    let ordering = child_compare(left_start + offset, right_start + offset);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                left_len.cmp(&right_len)
            })
        }
        DataType::LargeListView(child) => {
            let left_source = downcast::<LargeListViewArray>(left.as_ref())?;
            let right_source = downcast::<LargeListViewArray>(right.as_ref())?;
            let left_offsets = left_source.offsets().clone();
            let right_offsets = right_source.offsets().clone();
            let left_sizes = left_source.sizes().clone();
            let right_sizes = right_source.sizes().clone();
            let left_values = Arc::clone(left_source.values());
            let right_values = Arc::clone(right_source.values());
            let child_compare =
                make_yggdryl_comparator(child.data_type(), &left_values, &right_values, budget)?;
            Box::new(move |left, right| {
                let left_start = left_offsets[left].as_usize();
                let right_start = right_offsets[right].as_usize();
                let left_len = left_sizes[left].as_usize();
                let right_len = right_sizes[right].as_usize();
                for offset in 0..left_len.min(right_len) {
                    let ordering = child_compare(left_start + offset, right_start + offset);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                left_len.cmp(&right_len)
            })
        }
        DataType::FixedSizeList(child, size) => {
            let left_values = Arc::clone(downcast::<FixedSizeListArray>(left.as_ref())?.values());
            let right_values = Arc::clone(downcast::<FixedSizeListArray>(right.as_ref())?.values());
            let child_compare =
                make_yggdryl_comparator(child.data_type(), &left_values, &right_values, budget)?;
            let size = usize::try_from(*size).map_err(|_| {
                Error::IncompatibleSchema("map key fixed-list size is negative".to_owned())
            })?;
            Box::new(move |left, right| {
                let left = left * size;
                let right = right * size;
                for offset in 0..size {
                    let ordering = child_compare(left + offset, right + offset);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                Ordering::Equal
            })
        }
        DataType::Struct(fields) => {
            let left_source = downcast::<StructArray>(left.as_ref())?;
            let right_source = downcast::<StructArray>(right.as_ref())?;
            let comparators = fields
                .iter()
                .zip(left_source.columns())
                .zip(right_source.columns())
                .map(|((field, left), right)| {
                    make_yggdryl_comparator(field.data_type(), left, right, budget)
                })
                .collect::<Result<Vec<_>>>()?;
            Box::new(move |left, right| {
                comparators
                    .iter()
                    .map(|compare| compare(left, right))
                    .find(|ordering| *ordering != Ordering::Equal)
                    .unwrap_or(Ordering::Equal)
            })
        }
        DataType::Map(map) => {
            let left_source = downcast::<MapArray>(left.as_ref())?;
            let right_source = downcast::<MapArray>(right.as_ref())?;
            let left_offsets = left_source.offsets().clone();
            let right_offsets = right_source.offsets().clone();
            let left_entries: ArrayRef = Arc::new(left_source.entries().clone());
            let right_entries: ArrayRef = Arc::new(right_source.entries().clone());
            let entry_compare = make_yggdryl_comparator(
                map.entries().data_type(),
                &left_entries,
                &right_entries,
                budget,
            )?;
            Box::new(move |left, right| {
                let left = left_offsets[left].as_usize()..left_offsets[left + 1].as_usize();
                let right = right_offsets[right].as_usize()..right_offsets[right + 1].as_usize();
                for (left, right) in left.clone().zip(right.clone()) {
                    let ordering = entry_compare(left, right);
                    if ordering != Ordering::Equal {
                        return ordering;
                    }
                }
                left.len().cmp(&right.len())
            })
        }
        DataType::Dictionary(dictionary) => {
            return match dictionary.key() {
                DataType::Int8 => {
                    dictionary_key_comparator::<Int8Type>(left, right, dictionary, budget)
                }
                DataType::Int16 => {
                    dictionary_key_comparator::<Int16Type>(left, right, dictionary, budget)
                }
                DataType::Int32 => {
                    dictionary_key_comparator::<Int32Type>(left, right, dictionary, budget)
                }
                DataType::Int64 => {
                    dictionary_key_comparator::<Int64Type>(left, right, dictionary, budget)
                }
                DataType::UInt8 => {
                    dictionary_key_comparator::<UInt8Type>(left, right, dictionary, budget)
                }
                DataType::UInt16 => {
                    dictionary_key_comparator::<UInt16Type>(left, right, dictionary, budget)
                }
                DataType::UInt32 => {
                    dictionary_key_comparator::<UInt32Type>(left, right, dictionary, budget)
                }
                DataType::UInt64 => {
                    dictionary_key_comparator::<UInt64Type>(left, right, dictionary, budget)
                }
                _ => Err(Error::IncompatibleSchema(
                    "map key dictionary index is not an integer".to_owned(),
                )),
            };
        }
        DataType::Union(fields, _) => {
            let left_source = downcast::<UnionArray>(left.as_ref())?.clone();
            let right_source = downcast::<UnionArray>(right.as_ref())?.clone();
            let mut comparators = HashMap::with_capacity(fields.len());
            for (type_id, field) in fields {
                comparators.insert(
                    type_id,
                    make_yggdryl_comparator(
                        field.data_type(),
                        left_source.child(type_id),
                        right_source.child(type_id),
                        budget,
                    )?,
                );
            }
            Box::new(move |left, right| {
                let left_id = left_source.type_id(left);
                let right_id = right_source.type_id(right);
                match left_id.cmp(&right_id) {
                    Ordering::Equal => {
                        comparators
                            .get(&left_id)
                            .map_or(Ordering::Equal, |compare| {
                                compare(
                                    left_source.value_offset(left),
                                    right_source.value_offset(right),
                                )
                            })
                    }
                    ordering => ordering,
                }
            })
        }
        DataType::RunEndEncoded(encoded) => {
            return match encoded.run_ends().data_type() {
                DataType::Int16 => run_key_comparator::<Int16Type>(left, right, encoded, budget),
                DataType::Int32 => run_key_comparator::<Int32Type>(left, right, encoded, budget),
                DataType::Int64 => run_key_comparator::<Int64Type>(left, right, encoded, budget),
                _ => Err(Error::IncompatibleSchema(
                    "map key run-end type is invalid".to_owned(),
                )),
            };
        }
        _ => {
            return make_comparator(left.as_ref(), right.as_ref(), SortOptions::default())
                .map_err(Into::into);
        }
    };
    // Union's native representation is always a present `[id, payload]`
    // sequence. Every other sensitive wrapper follows ordinary Value nulls.
    if matches!(data_type, DataType::Union(..)) {
        Ok(compare)
    } else {
        wrap_yggdryl_nulls(left, right, data_type, compare, budget)
    }
}

fn cast_dictionary_planned(
    source_key: &ArrowDataType,
    plan: &ArrayCastPlan,
    array: ArrayRef,
    values: &ArrayCastPlan,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let expected = &plan.expected;
    macro_rules! rebuild {
        ($key:ty) => {{
            let source = downcast::<DictionaryArray<$key>>(&array)?;
            let source_values = source.values();
            if array.data_type() == expected
                && !(0..source.len())
                    .any(|row| is_exposed(exposure, row) && source.keys().is_valid(row))
            {
                // An exact dictionary with no reachable key can retain its
                // opaque vocabulary. Building a dense value-exposure bitmap
                // here would expand compact REE or view-backed vocabularies
                // solely to describe an empty selection.
                return Ok(array);
            }
            let value_exposure = selected_index_exposure(
                source_values.len(),
                source.len(),
                exposure,
                |row| {
                    source
                        .keys()
                        .is_valid(row)
                        .then(|| source.keys().value(row).try_into().ok())
                        .flatten()
                },
                budget,
            )?;
            let values =
                values.cast_exposed(Arc::clone(source_values), value_exposure.as_ref(), budget)?;
            if array.data_type() == expected && Arc::ptr_eq(&values, source_values) {
                return Ok(array);
            }
            Arc::new(DictionaryArray::<$key>::try_new(
                source.keys().clone(),
                values,
            )?) as ArrayRef
        }};
    }
    let rebuilt = match source_key {
        ArrowDataType::Int8 => rebuild!(Int8Type),
        ArrowDataType::Int16 => rebuild!(Int16Type),
        ArrowDataType::Int32 => rebuild!(Int32Type),
        ArrowDataType::Int64 => rebuild!(Int64Type),
        ArrowDataType::UInt8 => rebuild!(UInt8Type),
        ArrowDataType::UInt16 => rebuild!(UInt16Type),
        ArrowDataType::UInt32 => rebuild!(UInt32Type),
        ArrowDataType::UInt64 => rebuild!(UInt64Type),
        _ => {
            return arrow_cast_exposed(&array, expected, plan.safe, exposure, &plan.field, budget);
        }
    };
    if rebuilt.data_type() == expected {
        Ok(rebuilt)
    } else {
        arrow_cast_exposed(&rebuilt, expected, plan.safe, exposure, &plan.field, budget)
    }
}

fn cast_union_planned(
    fields: &arrow_schema::UnionFields,
    array: ArrayRef,
    plans: &[(i8, ArrayCastPlan)],
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let source = downcast::<UnionArray>(&array)?;
    let source_mode = if source.offsets().is_some() {
        arrow_schema::UnionMode::Dense
    } else {
        arrow_schema::UnionMode::Sparse
    };
    let mut unchanged = array.data_type() == &ArrowDataType::Union(fields.clone(), source_mode);
    let mut children = Vec::with_capacity(plans.len());
    for (type_id, plan) in plans {
        let source_child = source.child(*type_id);
        let child_exposure = selected_index_exposure(
            source_child.len(),
            source.len(),
            exposure,
            |row| (source.type_id(row) == *type_id).then(|| source.value_offset(row)),
            budget,
        )?;
        let child = plan.cast_exposed(Arc::clone(source_child), child_exposure.as_ref(), budget)?;
        unchanged &= Arc::ptr_eq(&child, source_child);
        children.push(child);
    }
    if unchanged {
        return Ok(array);
    }
    Ok(Arc::new(UnionArray::try_new(
        fields.clone(),
        source.type_ids().clone(),
        source.offsets().cloned(),
        children,
    )?))
}

fn run_value_exposure<R: RunEndIndexType>(
    source: &RunArray<R>,
    parent: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<Option<BooleanBuffer>> {
    if parent.is_some_and(|parent| parent.len() != source.len()) {
        return Err(Error::IncompatibleSchema(
            "run-end exposure has the wrong logical length".to_owned(),
        ));
    }
    if source.is_empty() {
        if source.values().is_empty() {
            return Ok(None);
        }
        budget.add_bitmap(source.values().len())?;
        return Ok(Some(BooleanBuffer::new_unset(source.values().len())));
    }
    let first_physical = source.get_start_physical_index();
    let last_physical = source.get_end_physical_index();
    if parent.is_none()
        && first_physical == 0
        && last_physical.checked_add(1) == Some(source.values().len())
    {
        return Ok(None);
    }

    budget.add_bitmap(source.values().len())?;
    let mut builder = BooleanBufferBuilder::new(source.values().len());
    builder.append_n(source.values().len(), false);
    let mut start = 0usize;
    let mut selected = 0usize;
    for (offset, end) in source.run_ends().sliced_values().enumerate() {
        let end = end.as_usize();
        let visible = parent.is_none_or(|parent| {
            end > start && parent.slice(start, end - start).count_set_bits() != 0
        });
        if visible {
            let physical = first_physical + offset;
            if physical >= source.values().len() {
                return Err(Error::IncompatibleSchema(
                    "run-end value index exceeds its values array".to_owned(),
                ));
            }
            builder.set_bit(physical, true);
            selected += 1;
        }
        start = end;
    }
    if selected == source.values().len() {
        Ok(None)
    } else {
        Ok(Some(builder.build()))
    }
}

fn cast_run_planned(
    source_run_type: &ArrowDataType,
    expected: &ArrowDataType,
    array: ArrayRef,
    values: &ArrayCastPlan,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    macro_rules! rebuild {
        ($array:ty, $key:ty) => {{
            let source = downcast::<$array>(&array)?;
            let source_values = source.values();
            let value_exposure = run_value_exposure(source, exposure, budget)?;
            let values =
                values.cast_exposed(Arc::clone(source_values), value_exposure.as_ref(), budget)?;
            if array.data_type() == expected && Arc::ptr_eq(&values, source_values) {
                return Ok(array);
            }
            if array.offset() != 0 {
                return Err(Error::Unsupported {
                    kind: "run_end_encoded",
                    reason: "nested casting of a sliced run-end encoded array is not supported"
                        .to_owned(),
                });
            }
            let run_ends = PrimitiveArray::<$key>::new(source.run_ends().inner().clone(), None);
            Arc::new(<$array>::try_new(&run_ends, values.as_ref())?) as ArrayRef
        }};
    }
    let rebuilt = match source_run_type {
        ArrowDataType::Int16 => rebuild!(Int16RunArray, Int16Type),
        ArrowDataType::Int32 => rebuild!(Int32RunArray, Int32Type),
        ArrowDataType::Int64 => rebuild!(Int64RunArray, Int64Type),
        _ => {
            return Err(Error::IncompatibleSchema(
                "run-end type must be Int16, Int32, or Int64".to_owned(),
            ));
        }
    };
    if rebuilt.data_type() == expected {
        return Ok(rebuilt);
    }
    reserve_to_data_scratch(&rebuilt, budget)?;
    let data = rebuilt
        .to_data()
        .into_builder()
        .data_type(expected.clone())
        .build()?;
    Ok(make_array(data))
}

fn contains_struct(data_type: &DataType) -> bool {
    match data_type {
        DataType::Struct(_) | DataType::Map(_) => true,
        DataType::List(field)
        | DataType::ListView(field)
        | DataType::FixedSizeList(field, _)
        | DataType::LargeList(field)
        | DataType::LargeListView(field) => contains_struct(field.data_type()),
        DataType::Union(fields, _) => fields
            .iter()
            .any(|(_, field)| contains_struct(field.data_type())),
        DataType::Dictionary(dictionary) => contains_struct(dictionary.value()),
        DataType::RunEndEncoded(encoded) => contains_struct(encoded.values().data_type()),
        _ => false,
    }
}

fn is_reconcilable_nested(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::List(_)
            | DataType::ListView(_)
            | DataType::FixedSizeList(_, _)
            | DataType::LargeList(_)
            | DataType::LargeListView(_)
            | DataType::Struct(_)
            | DataType::Union(_, _)
            | DataType::Dictionary(_)
            | DataType::Map(_)
            | DataType::RunEndEncoded(_)
    )
}

fn generic_kernel_materialization_is_bounded(
    source_type: &ArrowDataType,
    target_type: &DataType,
) -> Result<bool> {
    let source_type = DataType::from_arrow(source_type)?;
    if !is_reconcilable_nested(&source_type) && !is_reconcilable_nested(target_type) {
        return Ok(true);
    }
    // String kernels expose Arrow's own ArrayFormatter, which provides an
    // exact allocation-free byte-counting prepass even for nested wrappers.
    // Other generic wrapper changes do not expose a shape plan; matched
    // List/Map/Dictionary/Union/REE layouts recurse through dedicated plans.
    Ok(matches!(
        target_type,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
    ))
}

fn is_logically_null(array: &dyn Array, index: usize) -> bool {
    array
        .logical_nulls()
        .is_some_and(|nulls| nulls.is_null(index))
}

fn is_exposed(exposure: Option<&BooleanBuffer>, index: usize) -> bool {
    exposure.is_none_or(|exposure| exposure.value(index))
}

fn dictionary_logical_null_at<K: ArrowDictionaryKeyType>(
    array: &dyn Array,
    dictionary: &crate::DictionaryType,
    index: usize,
) -> Result<bool> {
    let array = downcast::<DictionaryArray<K>>(array)?;
    if array.keys().is_null(index) {
        return Ok(true);
    }
    let value_index = array.keys().value(index).as_usize();
    if value_index >= array.values().len() {
        return Err(Error::IncompatibleSchema(
            "dictionary key points outside its values array".to_owned(),
        ));
    }
    logical_null_at(array.values().as_ref(), dictionary.value(), value_index)
}

fn run_logical_null_at<R: RunEndIndexType>(
    array: &dyn Array,
    encoded: &crate::RunEndEncodedType,
    index: usize,
) -> Result<bool> {
    let array = downcast::<RunArray<R>>(array)?;
    logical_null_at(
        array.values().as_ref(),
        encoded.values().data_type(),
        array.get_physical_index(index),
    )
}

fn logical_null_at(array: &dyn Array, data_type: &DataType, index: usize) -> Result<bool> {
    if index >= array.len() {
        return Err(Error::IncompatibleSchema(
            "logical-null index exceeds its Arrow array".to_owned(),
        ));
    }
    match data_type {
        DataType::Null => Ok(true),
        DataType::Dictionary(dictionary) => match dictionary.key() {
            DataType::Int8 => dictionary_logical_null_at::<Int8Type>(array, dictionary, index),
            DataType::Int16 => dictionary_logical_null_at::<Int16Type>(array, dictionary, index),
            DataType::Int32 => dictionary_logical_null_at::<Int32Type>(array, dictionary, index),
            DataType::Int64 => dictionary_logical_null_at::<Int64Type>(array, dictionary, index),
            DataType::UInt8 => dictionary_logical_null_at::<UInt8Type>(array, dictionary, index),
            DataType::UInt16 => dictionary_logical_null_at::<UInt16Type>(array, dictionary, index),
            DataType::UInt32 => dictionary_logical_null_at::<UInt32Type>(array, dictionary, index),
            DataType::UInt64 => dictionary_logical_null_at::<UInt64Type>(array, dictionary, index),
            key => Err(Error::Unsupported {
                kind: key.name(),
                reason: format!(
                    "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
                ),
            }),
        },
        DataType::Union(fields, _) => {
            let array = downcast::<UnionArray>(array)?;
            let type_id = array.type_id(index);
            let (_, field) = fields
                .iter()
                .find(|(candidate, _)| *candidate == type_id)
                .ok_or_else(|| {
                    Error::IncompatibleSchema(format!("unknown union type id {type_id}"))
                })?;
            logical_null_at(
                array.child(type_id).as_ref(),
                field.data_type(),
                array.value_offset(index),
            )
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().data_type() {
            DataType::Int16 => run_logical_null_at::<Int16Type>(array, encoded, index),
            DataType::Int32 => run_logical_null_at::<Int32Type>(array, encoded, index),
            DataType::Int64 => run_logical_null_at::<Int64Type>(array, encoded, index),
            _ => Err(Error::IncompatibleSchema(
                "run-end type is not a supported signed integer".to_owned(),
            )),
        },
        _ => Ok(array.is_null(index)),
    }
}

fn run_exposed_logical_null_count<R: RunEndIndexType>(
    array: &dyn Array,
    encoded: &crate::RunEndEncodedType,
    exposure: Option<&BooleanBuffer>,
) -> Result<usize> {
    let array = downcast::<RunArray<R>>(array)?;
    if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
        return Err(Error::IncompatibleSchema(
            "run-end null-count exposure has the wrong length".to_owned(),
        ));
    }
    if array.is_empty() {
        return Ok(0);
    }
    let first_physical = array.get_start_physical_index();
    let mut start = 0usize;
    let mut null_count = 0usize;
    for (offset, end) in array.run_ends().sliced_values().enumerate() {
        let end = end.as_usize();
        if logical_null_at(
            array.values().as_ref(),
            encoded.values().data_type(),
            first_physical + offset,
        )? {
            let visible = exposure.map_or(end - start, |exposure| {
                exposure.slice(start, end - start).count_set_bits()
            });
            null_count = null_count.checked_add(visible).ok_or_else(|| {
                Error::IncompatibleSchema("logical null count exceeds usize".to_owned())
            })?;
        }
        start = end;
    }
    Ok(null_count)
}

fn exposed_logical_null_count(
    array: &dyn Array,
    data_type: &DataType,
    exposure: Option<&BooleanBuffer>,
) -> Result<usize> {
    if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
        return Err(Error::IncompatibleSchema(
            "logical-null exposure has the wrong length".to_owned(),
        ));
    }
    if let DataType::RunEndEncoded(encoded) = data_type {
        return match encoded.run_ends().data_type() {
            DataType::Int16 => {
                run_exposed_logical_null_count::<Int16Type>(array, encoded, exposure)
            }
            DataType::Int32 => {
                run_exposed_logical_null_count::<Int32Type>(array, encoded, exposure)
            }
            DataType::Int64 => {
                run_exposed_logical_null_count::<Int64Type>(array, encoded, exposure)
            }
            _ => Err(Error::IncompatibleSchema(
                "run-end type is not a supported signed integer".to_owned(),
            )),
        };
    }
    let mut null_count = 0usize;
    for index in 0..array.len() {
        if is_exposed(exposure, index) && logical_null_at(array, data_type, index)? {
            null_count += 1;
        }
    }
    Ok(null_count)
}

fn logical_validity_buffer(
    array: &dyn Array,
    data_type: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<arrow_buffer::NullBuffer> {
    budget.add_bitmap(array.len())?;
    let mut builder = BooleanBufferBuilder::new(array.len());
    for index in 0..array.len() {
        builder.append(!logical_null_at(array, data_type, index)?);
    }
    Ok(arrow_buffer::NullBuffer::new(builder.build()))
}

fn visible_array_exposure(
    array: &dyn Array,
    parent: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<Option<BooleanBuffer>> {
    if array.null_count() == 0
        && parent.is_none_or(|parent| parent.count_set_bits() == parent.len())
    {
        return Ok(None);
    }
    budget.add_bitmap(array.len())?;
    let exposure = BooleanBuffer::collect_bool(array.len(), |index| {
        is_exposed(parent, index) && array.is_valid(index)
    });
    Ok((exposure.count_set_bits() != exposure.len()).then_some(exposure))
}

fn offset_pair(start: i64, end: i64) -> Result<(usize, usize)> {
    let start = usize::try_from(start).map_err(|_| {
        Error::IncompatibleSchema("nested Arrow offset is negative or exceeds usize".to_owned())
    })?;
    let end = usize::try_from(end).map_err(|_| {
        Error::IncompatibleSchema("nested Arrow offset is negative or exceeds usize".to_owned())
    })?;
    Ok((start, end))
}

fn offset_size(start: i64, size: i64) -> Result<(usize, usize)> {
    let (start, size) = offset_pair(start, size)?;
    let end = start.checked_add(size).ok_or_else(|| {
        Error::IncompatibleSchema("nested Arrow offset plus size exceeds usize".to_owned())
    })?;
    Ok((start, end))
}

fn range_exposure<Valid, Range>(
    child_len: usize,
    parent_len: usize,
    parent: Option<&BooleanBuffer>,
    mut is_valid: Valid,
    mut range: Range,
    budget: &mut MaterializationBudget,
) -> Result<Option<BooleanBuffer>>
where
    Valid: FnMut(usize) -> bool,
    Range: FnMut(usize) -> Result<(usize, usize)>,
{
    if parent.is_some_and(|parent| parent.len() != parent_len) {
        return Err(Error::IncompatibleSchema(
            "nested Arrow range exposure has the wrong parent length".to_owned(),
        ));
    }
    let mut next = 0usize;
    let mut full_coverage = true;
    for row in 0..parent_len {
        if !is_exposed(parent, row) || !is_valid(row) {
            full_coverage = false;
            break;
        }
        let (start, end) = range(row)?;
        if start > end || end > child_len {
            return Err(Error::IncompatibleSchema(
                "nested Arrow offsets select values outside their child array".to_owned(),
            ));
        }
        if start != next {
            full_coverage = false;
            break;
        }
        next = end;
    }
    if full_coverage && next == child_len {
        return Ok(None);
    }
    budget.add_bitmap(child_len)?;
    let mut builder = BooleanBufferBuilder::new(child_len);
    builder.append_n(child_len, false);
    let mut selected = 0usize;
    for row in 0..parent_len {
        if !is_exposed(parent, row) || !is_valid(row) {
            continue;
        }
        let (start, end) = range(row)?;
        if start > end || end > child_len {
            return Err(Error::IncompatibleSchema(
                "nested Arrow offsets select values outside their child array".to_owned(),
            ));
        }
        for index in start..end {
            if !builder.get_bit(index) {
                builder.set_bit(index, true);
                selected += 1;
            }
        }
    }
    if selected == child_len {
        Ok(None)
    } else {
        Ok(Some(builder.build()))
    }
}

fn selected_index_exposure<Index>(
    child_len: usize,
    parent_len: usize,
    parent: Option<&BooleanBuffer>,
    mut index: Index,
    budget: &mut MaterializationBudget,
) -> Result<Option<BooleanBuffer>>
where
    Index: FnMut(usize) -> Option<usize>,
{
    if parent.is_some_and(|parent| parent.len() != parent_len) {
        return Err(Error::IncompatibleSchema(
            "nested Arrow selection exposure has the wrong parent length".to_owned(),
        ));
    }
    if parent.is_none()
        && parent_len == child_len
        && (0..parent_len).all(|row| index(row) == Some(row))
    {
        return Ok(None);
    }
    budget.add_bitmap(child_len)?;
    let mut builder = BooleanBufferBuilder::new(child_len);
    builder.append_n(child_len, false);
    let mut selected = 0usize;
    for row in 0..parent_len {
        if !is_exposed(parent, row) {
            continue;
        }
        let Some(index) = index(row) else {
            continue;
        };
        if index >= child_len {
            return Err(Error::IncompatibleSchema(
                "nested Arrow selection points outside its child array".to_owned(),
            ));
        }
        if !builder.get_bit(index) {
            builder.set_bit(index, true);
            selected += 1;
        }
    }
    if selected == child_len {
        Ok(None)
    } else {
        Ok(Some(builder.build()))
    }
}

fn union_mode_matches(core: UnionMode, arrow: arrow_schema::UnionMode) -> bool {
    matches!(
        (core, arrow),
        (UnionMode::Sparse, arrow_schema::UnionMode::Sparse)
            | (UnionMode::Dense, arrow_schema::UnionMode::Dense)
    )
}

fn arrow_cast(array: &ArrayRef, expected: &ArrowDataType, safe: bool) -> Result<ArrayRef> {
    let options = CastOptions {
        safe,
        ..CastOptions::default()
    };
    cast_with_options(array.as_ref(), expected, &options).map_err(Into::into)
}

#[derive(Clone, Copy)]
enum SourceSelection<'a> {
    Indices(&'a [u32]),
    Ranges(&'a [(usize, usize)]),
}

impl SourceSelection<'_> {
    fn row_count(self, upper_bound: usize) -> Result<usize> {
        let mut rows = 0usize;
        self.try_for_each(upper_bound, |_| {
            rows = rows.checked_add(1).ok_or_else(|| {
                Error::IncompatibleSchema(
                    "masked Arrow source selection length exceeds usize".to_owned(),
                )
            })?;
            Ok(())
        })?;
        Ok(rows)
    }

    fn try_for_each(
        self,
        upper_bound: usize,
        mut visit: impl FnMut(usize) -> Result<()>,
    ) -> Result<()> {
        match self {
            Self::Indices(indices) => {
                for index in indices {
                    let index = usize::try_from(*index).map_err(|_| {
                        Error::IncompatibleSchema(
                            "masked Arrow source index exceeds usize".to_owned(),
                        )
                    })?;
                    if index >= upper_bound {
                        return Err(Error::IncompatibleSchema(
                            "masked Arrow source index exceeds its array length".to_owned(),
                        ));
                    }
                    visit(index)?;
                }
            }
            Self::Ranges(ranges) => {
                for &(start, end) in ranges {
                    if start > end || end > upper_bound {
                        return Err(Error::IncompatibleSchema(
                            "masked Arrow source range exceeds its array length".to_owned(),
                        ));
                    }
                    for index in start..end {
                        visit(index)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn reserve_vec_bytes<T>(budget: &mut MaterializationBudget, capacity: usize) -> Result<()> {
    let bytes = std::mem::size_of::<T>()
        .checked_mul(capacity)
        .ok_or_else(|| {
            Error::IncompatibleSchema(
                "masked Arrow source scratch allocation exceeds usize".to_owned(),
            )
        })?;
    budget.add_bytes(bytes)
}

fn scratch_vec<T>(
    budget: &mut MaterializationBudget,
    capacity: usize,
    purpose: &str,
) -> Result<Vec<T>> {
    reserve_vec_bytes::<T>(budget, capacity)?;
    let mut values = Vec::new();
    values.try_reserve_exact(capacity).map_err(|error| {
        Error::IncompatibleSchema(format!("masked Arrow {purpose} allocation failed: {error}"))
    })?;
    Ok(values)
}

fn selected_child_ranges<Valid, Range>(
    selection: SourceSelection<'_>,
    parent_len: usize,
    child_len: usize,
    is_valid: Valid,
    range: Range,
    budget: &mut MaterializationBudget,
) -> Result<Vec<(usize, usize)>>
where
    Valid: Fn(usize) -> bool,
    Range: Fn(usize) -> Result<(usize, usize)>,
{
    let mut range_count = 0usize;
    let mut previous_end = None;
    selection.try_for_each(parent_len, |row| {
        if !is_valid(row) {
            return Ok(());
        }
        let (start, end) = range(row)?;
        if start > end || end > child_len {
            return Err(Error::IncompatibleSchema(
                "masked Arrow nested source range exceeds its child array".to_owned(),
            ));
        }
        if start != end {
            if previous_end != Some(start) {
                range_count = range_count.checked_add(1).ok_or_else(|| {
                    Error::IncompatibleSchema(
                        "masked Arrow nested range count exceeds usize".to_owned(),
                    )
                })?;
            }
            previous_end = Some(end);
        }
        Ok(())
    })?;

    let mut ranges = scratch_vec::<(usize, usize)>(budget, range_count, "range scratch")?;
    selection.try_for_each(parent_len, |row| {
        if !is_valid(row) {
            return Ok(());
        }
        let (start, end) = range(row)?;
        if start == end {
            return Ok(());
        }
        if let Some(previous) = ranges.last_mut() {
            if previous.1 == start {
                previous.1 = end;
                return Ok(());
            }
        }
        ranges.push((start, end));
        Ok(())
    })?;
    Ok(ranges)
}

fn reserve_byte_payload(
    selection: SourceSelection<'_>,
    array_len: usize,
    mut value_len: impl FnMut(usize) -> usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let mut bytes = 0usize;
    selection.try_for_each(array_len, |index| {
        bytes = bytes.checked_add(value_len(index)).ok_or_else(|| {
            Error::IncompatibleSchema(
                "masked Arrow selected source payload exceeds usize".to_owned(),
            )
        })?;
        Ok(())
    })?;
    budget.add_bytes(bytes)
}

fn reserve_run_source_take<R: RunEndIndexType>(
    source: &RunArray<R>,
    encoded: &crate::RunEndEncodedType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(source.len())?;
    // Arrow's run take first expands every selected logical index to usize.
    reserve_vec_bytes::<usize>(budget, selected_count)?;

    let mut run_count = 0usize;
    let mut previous = None;
    selection.try_for_each(source.len(), |row| {
        let physical = source.get_physical_index(row);
        if previous != Some(physical) {
            run_count += 1;
            previous = Some(physical);
        }
        Ok(())
    })?;
    budget.add_array(encoded.run_ends().data_type(), run_count)?;
    budget.add_array_layout(&DataType::UInt32, run_count)?;

    let mut value_indices = Vec::new();
    value_indices
        .try_reserve_exact(run_count)
        .map_err(|error| {
            Error::IncompatibleSchema(format!(
                "masked Arrow run-value index allocation failed: {error}"
            ))
        })?;
    previous = None;
    selection.try_for_each(source.len(), |row| {
        let physical = source.get_physical_index(row);
        if previous != Some(physical) {
            value_indices.push(u32::try_from(physical).map_err(|_| {
                Error::IncompatibleSchema("masked Arrow run-value index exceeds UInt32".to_owned())
            })?);
            previous = Some(physical);
        }
        Ok(())
    })?;
    reserve_source_selection(
        source.values().as_ref(),
        encoded.values().data_type(),
        SourceSelection::Indices(&value_indices),
        budget,
    )
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow's exhaustive take dispatch and sharing rules.
fn reserve_source_selection(
    array: &dyn Array,
    source_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(array.len())?;
    budget.add_array_layout(source_type, selected_count)?;
    reserve_source_children_and_payload(array, source_type, selection, budget)
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow's exhaustive take dispatch and sharing rules.
fn reserve_source_children_and_payload(
    array: &dyn Array,
    source_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let selected_count = selection.row_count(array.len())?;
    match source_type {
        DataType::Binary => {
            let array = downcast::<BinaryArray>(array)?;
            reserve_byte_payload(
                selection,
                array.len(),
                |index| {
                    if array.is_valid(index) {
                        array.value(index).len()
                    } else {
                        0
                    }
                },
                budget,
            )?;
        }
        DataType::LargeBinary => {
            let array = downcast::<LargeBinaryArray>(array)?;
            reserve_byte_payload(
                selection,
                array.len(),
                |index| {
                    if array.is_valid(index) {
                        array.value(index).len()
                    } else {
                        0
                    }
                },
                budget,
            )?;
        }
        DataType::Utf8 => {
            let array = downcast::<StringArray>(array)?;
            reserve_byte_payload(
                selection,
                array.len(),
                |index| {
                    if array.is_valid(index) {
                        array.value(index).len()
                    } else {
                        0
                    }
                },
                budget,
            )?;
        }
        DataType::LargeUtf8 => {
            let array = downcast::<LargeStringArray>(array)?;
            reserve_byte_payload(
                selection,
                array.len(),
                |index| {
                    if array.is_valid(index) {
                        array.value(index).len()
                    } else {
                        0
                    }
                },
                budget,
            )?;
        }
        DataType::List(child) => {
            let array = downcast::<ListArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |row| array.is_valid(row),
                |row| offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1])),
                budget,
            )?;
            reserve_source_selection(
                array.values().as_ref(),
                child.data_type(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::LargeList(child) => {
            let array = downcast::<LargeListArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |row| array.is_valid(row),
                |row| offset_pair(offsets[row], offsets[row + 1]),
                budget,
            )?;
            reserve_source_selection(
                array.values().as_ref(),
                child.data_type(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::FixedSizeList(child, size) => {
            let array = downcast::<FixedSizeListArray>(array)?;
            let size = usize::try_from(*size).map_err(|_| {
                Error::IncompatibleSchema(
                    "masked Arrow fixed-size-list width is negative".to_owned(),
                )
            })?;
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.values().len(),
                |_| true,
                |row| {
                    let start = row.checked_mul(size).ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "masked Arrow fixed-size-list offset exceeds usize".to_owned(),
                        )
                    })?;
                    let end = start.checked_add(size).ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "masked Arrow fixed-size-list range exceeds usize".to_owned(),
                        )
                    })?;
                    Ok((start, end))
                },
                budget,
            )?;
            let child_count = SourceSelection::Ranges(&ranges).row_count(array.values().len())?;
            // Arrow expands fixed-list rows into a UInt32 child-index buffer.
            budget.add_array_layout(&DataType::UInt32, child_count)?;
            reserve_source_selection(
                array.values().as_ref(),
                child.data_type(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Struct(fields) => {
            let array = downcast::<StructArray>(array)?;
            if fields.len() != array.num_columns() {
                return Err(Error::IncompatibleSchema(
                    "masked Arrow source Struct child count changed after planning".to_owned(),
                ));
            }
            for (field, child) in fields.iter().zip(array.columns()) {
                reserve_source_selection(child.as_ref(), field.data_type(), selection, budget)?;
            }
        }
        DataType::Map(map) => {
            let array = downcast::<MapArray>(array)?;
            let offsets = array.value_offsets();
            let ranges = selected_child_ranges(
                selection,
                array.len(),
                array.entries().len(),
                |row| array.is_valid(row),
                |row| offset_pair(i64::from(offsets[row]), i64::from(offsets[row + 1])),
                budget,
            )?;
            reserve_source_selection(
                array.entries(),
                map.entries().data_type(),
                SourceSelection::Ranges(&ranges),
                budget,
            )?;
        }
        DataType::Union(fields, UnionMode::Sparse) => {
            let array = downcast::<UnionArray>(array)?;
            for (type_id, field) in fields {
                reserve_source_selection(
                    array.child(type_id).as_ref(),
                    field.data_type(),
                    selection,
                    budget,
                )?;
            }
        }
        DataType::Union(fields, UnionMode::Dense) => {
            let array = downcast::<UnionArray>(array)?;
            // Dense take keeps one row-sized mask and filtered-offset scratch
            // alive at a time while retaining every already-built child.
            budget.add_bitmap(selected_count)?;
            budget.add_array_layout(&DataType::UInt32, selected_count)?;
            let mut branch = Vec::new();
            branch.try_reserve_exact(selected_count).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "masked Arrow dense-union index allocation failed: {error}"
                ))
            })?;
            for (type_id, field) in fields {
                branch.clear();
                selection.try_for_each(array.len(), |row| {
                    if array.type_id(row) == type_id {
                        branch.push(u32::try_from(array.value_offset(row)).map_err(|_| {
                            Error::IncompatibleSchema(
                                "masked Arrow dense-union offset exceeds UInt32".to_owned(),
                            )
                        })?);
                    }
                    Ok(())
                })?;
                reserve_source_selection(
                    array.child(type_id).as_ref(),
                    field.data_type(),
                    SourceSelection::Indices(&branch),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().data_type() {
            DataType::Int16 => reserve_run_source_take(
                downcast::<Int16RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            DataType::Int32 => reserve_run_source_take(
                downcast::<Int32RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            DataType::Int64 => reserve_run_source_take(
                downcast::<Int64RunArray>(array)?,
                encoded,
                selection,
                budget,
            )?,
            _ => {
                return Err(Error::IncompatibleSchema(
                    "masked Arrow run-end type must be Int16, Int32, or Int64".to_owned(),
                ));
            }
        },
        DataType::BinaryView => {
            let array = downcast::<BinaryViewArray>(array)?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())?;
        }
        DataType::Utf8View => {
            let array = downcast::<StringViewArray>(array)?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())?;
        }
        // List views and dictionaries share their child/value arrays. All
        // scalar/fixed-width storage was fully charged by the shallow layout.
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn projected_byte_len(array: &dyn Array, source_type: &DataType, index: usize) -> Result<usize> {
    if index >= array.len() {
        return Err(Error::IncompatibleSchema(
            "Arrow byte projection index exceeds its source array".to_owned(),
        ));
    }
    if array.is_null(index)
        && !matches!(
            source_type,
            DataType::Dictionary(_) | DataType::Union(..) | DataType::RunEndEncoded(_)
        )
    {
        return Ok(0);
    }
    let bytes = match source_type {
        DataType::Binary => downcast::<BinaryArray>(array)?.value(index).len(),
        DataType::LargeBinary => downcast::<LargeBinaryArray>(array)?.value(index).len(),
        DataType::BinaryView => downcast::<BinaryViewArray>(array)?.value(index).len(),
        DataType::FixedSizeBinary(_) => downcast::<FixedSizeBinaryArray>(array)?.value(index).len(),
        DataType::Utf8 => downcast::<StringArray>(array)?.value(index).len(),
        DataType::LargeUtf8 => downcast::<LargeStringArray>(array)?.value(index).len(),
        DataType::Utf8View => downcast::<StringViewArray>(array)?.value(index).len(),
        DataType::Dictionary(dictionary) => {
            macro_rules! dictionary_len {
                ($key:ty) => {{
                    let dictionary_array = downcast::<DictionaryArray<$key>>(array)?;
                    if dictionary_array.keys().is_null(index) {
                        0
                    } else {
                        let key = usize::try_from(dictionary_array.keys().value(index)).map_err(
                            |_| {
                                Error::IncompatibleSchema(
                                    "Arrow dictionary key is negative or exceeds usize".to_owned(),
                                )
                            },
                        )?;
                        projected_byte_len(
                            dictionary_array.values().as_ref(),
                            dictionary.value(),
                            key,
                        )?
                    }
                }};
            }
            match dictionary.key() {
                DataType::Int8 => dictionary_len!(Int8Type),
                DataType::Int16 => dictionary_len!(Int16Type),
                DataType::Int32 => dictionary_len!(Int32Type),
                DataType::Int64 => dictionary_len!(Int64Type),
                DataType::UInt8 => dictionary_len!(UInt8Type),
                DataType::UInt16 => dictionary_len!(UInt16Type),
                DataType::UInt32 => dictionary_len!(UInt32Type),
                DataType::UInt64 => dictionary_len!(UInt64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "Arrow dictionary byte projection key is not an integer".to_owned(),
                    ));
                }
            }
        }
        DataType::Union(fields, _) => {
            let union = downcast::<UnionArray>(array)?;
            let type_id = union.type_id(index);
            let (_, field) = fields
                .iter()
                .find(|(candidate, _)| *candidate == type_id)
                .ok_or_else(|| {
                    Error::IncompatibleSchema(format!(
                        "Arrow union byte projection has unknown type ID {type_id}"
                    ))
                })?;
            projected_byte_len(
                union.child(type_id).as_ref(),
                field.data_type(),
                union.value_offset(index),
            )?
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().data_type() {
            DataType::Int16 => {
                let run = downcast::<Int16RunArray>(array)?;
                projected_byte_len(
                    run.values().as_ref(),
                    encoded.values().data_type(),
                    run.get_physical_index(index),
                )?
            }
            DataType::Int32 => {
                let run = downcast::<Int32RunArray>(array)?;
                projected_byte_len(
                    run.values().as_ref(),
                    encoded.values().data_type(),
                    run.get_physical_index(index),
                )?
            }
            DataType::Int64 => {
                let run = downcast::<Int64RunArray>(array)?;
                projected_byte_len(
                    run.values().as_ref(),
                    encoded.values().data_type(),
                    run.get_physical_index(index),
                )?
            }
            _ => {
                return Err(Error::IncompatibleSchema(
                    "Arrow run-end byte projection type is invalid".to_owned(),
                ));
            }
        },
        DataType::Boolean | DataType::UInt16 => 5,
        DataType::Int8 => 4,
        DataType::UInt8 => 3,
        DataType::Int16 => 6,
        DataType::Int32 | DataType::Decimal32 { .. } => 12,
        DataType::UInt32 => 10,
        DataType::Int64 | DataType::Decimal64 { .. } => 21,
        DataType::UInt64 => 20,
        DataType::Float16 => 16,
        DataType::Float32 => 24,
        DataType::Float64 => 32,
        DataType::Decimal128 { .. } => 41,
        DataType::Decimal256 { .. } => 78,
        DataType::Timestamp(..)
        | DataType::Date32
        | DataType::Date64
        | DataType::Time32(_)
        | DataType::Time64(_)
        | DataType::Duration(_)
        | DataType::Interval(_) => 128,
        _ => 0,
    };
    Ok(bytes)
}

#[derive(Default)]
struct CountingWriter {
    bytes: usize,
}

impl std::fmt::Write for CountingWriter {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        self.bytes = self.bytes.checked_add(value.len()).ok_or(std::fmt::Error)?;
        Ok(())
    }
}

fn reserve_formatted_payload(
    array: &dyn Array,
    selection: SourceSelection<'_>,
    target_is_view: bool,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let options = FormatOptions::default();
    let formatter = ArrayFormatter::try_new(array, &options)?;
    let mut total = 0usize;
    let mut maximum = 0usize;
    selection.try_for_each(array.len(), |index| {
        if array.is_null(index) {
            return Ok(());
        }
        let mut counter = CountingWriter::default();
        formatter.value(index).write(&mut counter)?;
        total = total.checked_add(counter.bytes).ok_or_else(|| {
            Error::IncompatibleSchema("Arrow formatted payload exceeds usize".to_owned())
        })?;
        maximum = maximum.max(counter.bytes);
        Ok(())
    })?;
    budget.add_bytes(total)?;
    // Utf8View formatting keeps one growable String alive while appending the
    // final view buffers. Account for its largest selected logical value.
    if target_is_view {
        budget.add_bytes(maximum)?;
    }
    Ok(())
}

fn reserve_cast_output_payload(
    array: &dyn Array,
    source_type: &DataType,
    target_type: &DataType,
    selection: SourceSelection<'_>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match target_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => reserve_formatted_payload(
            array,
            selection,
            matches!(target_type, DataType::Utf8View),
            budget,
        ),
        DataType::Binary | DataType::LargeBinary => {
            let mut bytes = 0usize;
            selection.try_for_each(array.len(), |index| {
                bytes = bytes
                    .checked_add(projected_byte_len(array, source_type, index)?)
                    .ok_or_else(|| {
                        Error::IncompatibleSchema(
                            "Arrow cast output payload exceeds usize".to_owned(),
                        )
                    })?;
                Ok(())
            })?;
            budget.add_bytes(bytes)
        }
        DataType::List(_)
        | DataType::LargeList(_)
        | DataType::FixedSizeList(..)
        | DataType::Struct(_)
        | DataType::Map(_)
        | DataType::Union(..)
        | DataType::Dictionary(_)
        | DataType::RunEndEncoded(_) => {
            reserve_source_children_and_payload(array, source_type, selection, budget)
        }
        _ => Ok(()),
    }
}

fn reserve_selected_source_take(
    array: &dyn Array,
    selected: &[u32],
    target_type: &DataType,
    output_copies: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let source_type = DataType::from_arrow(array.data_type())?;
    let selection = SourceSelection::Indices(selected);
    reserve_source_selection(array, &source_type, selection, budget)?;
    for _ in 0..output_copies {
        reserve_cast_output_payload(array, &source_type, target_type, selection, budget)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn arrow_cast_exposed(
    array: &ArrayRef,
    expected: &ArrowDataType,
    safe: bool,
    exposure: Option<&BooleanBuffer>,
    target: &Field,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let Some(exposure) = exposure else {
        if array.data_type() == expected {
            return Ok(Arc::clone(array));
        }
        budget.add_array(target.data_type(), array.len())?;
        let source_type = DataType::from_arrow(array.data_type())?;
        let full = [(0, array.len())];
        reserve_cast_output_payload(
            array.as_ref(),
            &source_type,
            target.data_type(),
            SourceSelection::Ranges(&full),
            budget,
        )?;
        return arrow_cast(array, expected, safe);
    };
    let selected_count = exposure.count_set_bits();
    if selected_count == 0 {
        return default_array(target, array.len(), Some(exposure), budget);
    }
    let phase = budget.mark();
    let output = (|| -> Result<ArrayRef> {
        // The masked kernel retains several arrays at once: selection/scatter
        // indices, a compact source, its compact cast, the scattered target, and
        // (for a required Field) the placeholder plus final zip output. Charge
        // every target-shaped buffer plus the selected source's actual physical
        // layout and copied payload before invoking Arrow's take kernel.
        budget.add_array(&DataType::UInt32, selected_count)?;
        budget.add_array(&DataType::UInt32, array.len())?;
        budget.add_array(target.data_type(), selected_count)?;
        budget.add_array(target.data_type(), array.len())?;
        if !target.is_nullable() {
            budget.add_array(target.data_type(), 1)?;
            budget.add_array(target.data_type(), array.len())?;
        }
        let mut selected = Vec::new();
        selected
            .try_reserve_exact(selected_count)
            .map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "masked Arrow cast index allocation failed: {error}"
                ))
            })?;
        for index in 0..array.len() {
            if exposure.value(index) {
                let source = u32::try_from(index).map_err(|_| {
                    Error::IncompatibleSchema(
                        "masked Arrow cast source index exceeds UInt32".to_owned(),
                    )
                })?;
                selected.push(source);
            }
        }
        reserve_selected_source_take(
            array.as_ref(),
            &selected,
            target.data_type(),
            if target.is_nullable() { 2 } else { 3 },
            budget,
        )?;

        reserve_vec_bytes::<Option<u32>>(budget, array.len())?;
        let mut scatter = Vec::new();
        scatter.try_reserve_exact(array.len()).map_err(|error| {
            Error::IncompatibleSchema(format!(
                "masked Arrow cast scatter allocation failed: {error}"
            ))
        })?;
        let mut target_index = 0usize;
        for index in 0..array.len() {
            if exposure.value(index) {
                scatter.push(Some(u32::try_from(target_index).map_err(|_| {
                    Error::IncompatibleSchema(
                        "masked Arrow cast target index exceeds UInt32".to_owned(),
                    )
                })?));
                target_index += 1;
            } else {
                scatter.push(None);
            }
        }
        let compact = take(array.as_ref(), &UInt32Array::from(selected), None)?;
        let compact = arrow_cast(&compact, expected, safe)?;
        let scattered = take(compact.as_ref(), &UInt32Array::from(scatter), None)?;
        if target.is_nullable() {
            return Ok(scattered);
        }
        let mask = BooleanArray::new(exposure.clone(), None);
        let placeholder = crate::arrow::value::physical_placeholder_for_field(target)?;
        let placeholder = crate::arrow::value::array_from_values(target, &[&placeholder])?;
        let (scattered, placeholder) = if contains_dictionary(target.data_type()) {
            align_nested_dictionaries(
                target,
                &scattered,
                &placeholder,
                Some(exposure),
                None,
                budget,
            )?
        } else {
            (scattered, placeholder)
        };
        let placeholder = Scalar::new(placeholder);
        zip(&mask, &scattered.as_ref(), &placeholder).map_err(Into::into)
    })()?;

    // Compact sources, index arrays, scatter output, and placeholders are
    // phase-local. Retain only the returned target array before sibling
    // columns continue against the shared operation budget.
    budget.restore(phase);
    let full = [(0, output.len())];
    reserve_source_selection(
        output.as_ref(),
        target.data_type(),
        SourceSelection::Ranges(&full),
        budget,
    )?;
    if contains_dictionary(target.data_type()) {
        reserve_new_dictionary_vocabularies(&output, array, target.data_type(), budget)?;
    }
    Ok(output)
}

fn fill_nulls(
    field: &Field,
    array: ArrayRef,
    data_type_semantics: bool,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if data_type_semantics && field.data_type().is_default_value(&Value::Null)? {
        return Ok(array);
    }
    if let DataType::Dictionary(dictionary) = field.data_type() {
        return fill_dictionary_nulls(field, dictionary, array, exposure, budget);
    }
    let phase = budget.mark();
    let logical = logical_validity_buffer(array.as_ref(), field.data_type(), budget)?;
    let default_count = (0..array.len())
        .filter(|index| is_exposed(exposure, *index) && logical.is_null(*index))
        .count();
    if default_count == 0 {
        budget.restore(phase);
        return Ok(array);
    }
    let exposed_count = exposure.map_or(array.len(), BooleanBuffer::count_set_bits);
    if default_count == exposed_count && contains_dictionary(field.data_type()) {
        budget.restore(phase);
        return default_array(field, array.len(), None, budget);
    }

    // Reserve the one-row scalar and both parts of the final zip before
    // constructing any default. The source-range walk charges only values the
    // truthy side copies; exposed nulls are charged through canonical defaults.
    budget.add_default_scalar_scratch(field.data_type())?;
    if has_derived_logical_nulls(field.data_type()) {
        budget.add_bitmap(1)?;
    }
    if contains_dictionary(field.data_type()) {
        budget.add_repeated_default_without_dictionary_values(field.data_type(), default_count)?;
    } else {
        budget.add_repeated_default(field.data_type(), default_count)?;
    }
    let full = [(0, array.len())];
    let truthy_ranges = selected_child_ranges(
        SourceSelection::Ranges(&full),
        array.len(),
        array.len(),
        |index| !is_exposed(exposure, index) || logical.is_valid(index),
        |index| Ok((index, index + 1)),
        budget,
    )?;
    let source_type = DataType::from_arrow(array.data_type())?;
    reserve_source_selection(
        array.as_ref(),
        &source_type,
        SourceSelection::Ranges(&truthy_ranges),
        budget,
    )?;
    if exposure.is_some() {
        budget.add_bitmap(array.len())?;
    }

    let source_for_retention = Arc::clone(&array);
    let default = if data_type_semantics {
        field.data_type().default_arrow_scalar()?
    } else {
        field.default_arrow_scalar()?
    };
    if is_logically_null(default.array().as_ref(), 0) {
        budget.restore(phase);
        return Ok(array);
    }
    let mask = match exposure {
        None => logical.inner().clone(),
        Some(exposure) => BooleanBuffer::collect_bool(array.len(), |index| {
            !exposure.value(index) || logical.is_valid(index)
        }),
    };
    let mask = BooleanArray::new(mask, None);
    let default = default.into_array();
    let (array, default) = if contains_dictionary(field.data_type()) {
        budget.add_bitmap(array.len())?;
        let live = BooleanBuffer::collect_bool(array.len(), |index| {
            is_exposed(exposure, index) && logical.is_valid(index)
        });
        align_nested_dictionaries(field, &array, &default, Some(&live), None, budget)?
    } else {
        (array, default)
    };
    let default = Scalar::new(default);
    let truthy: &dyn Array = array.as_ref();
    let output = zip(&mask, &truthy, &default)?;

    // Only the zip output survives this phase. Release the scalar, mask, and
    // range-planning reservations, then retain the exact two output parts in
    // the operation-wide aggregate for following columns.
    budget.restore(phase);
    if contains_dictionary(field.data_type()) {
        budget.add_repeated_default_without_dictionary_values(field.data_type(), default_count)?;
    } else {
        budget.add_repeated_default(field.data_type(), default_count)?;
    }
    reserve_source_selection(
        source_for_retention.as_ref(),
        &source_type,
        SourceSelection::Ranges(&truthy_ranges),
        budget,
    )?;
    if contains_dictionary(field.data_type()) {
        reserve_new_dictionary_vocabularies(
            &output,
            &source_for_retention,
            field.data_type(),
            budget,
        )?;
    }
    Ok(output)
}

fn fill_dictionary_nulls(
    field: &Field,
    dictionary: &crate::DictionaryType,
    array: ArrayRef,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    macro_rules! fill {
        ($key:ty) => {{ fill_dictionary_nulls_typed::<$key>(field, dictionary, array, exposure, budget) }};
    }
    match dictionary.key() {
        DataType::Int8 => fill!(Int8Type),
        DataType::Int16 => fill!(Int16Type),
        DataType::Int32 => fill!(Int32Type),
        DataType::Int64 => fill!(Int64Type),
        DataType::UInt8 => fill!(UInt8Type),
        DataType::UInt16 => fill!(UInt16Type),
        DataType::UInt32 => fill!(UInt32Type),
        DataType::UInt64 => fill!(UInt64Type),
        key => Err(Error::Unsupported {
            kind: key.name(),
            reason: format!(
                "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        }),
    }
}

#[allow(clippy::too_many_lines)]
fn fill_dictionary_nulls_typed<K>(
    field: &Field,
    dictionary: &crate::DictionaryType,
    array: ArrayRef,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef>
where
    K: ArrowDictionaryKeyType,
    K::Native: TryFrom<usize>,
{
    let source = downcast::<DictionaryArray<K>>(&array)?;
    let phase = budget.mark();
    let logical = logical_validity_buffer(source, field.data_type(), budget)?;
    let repair_count = (0..source.len())
        .filter(|index| is_exposed(exposure, *index) && logical.is_null(*index))
        .count();
    if repair_count == 0 {
        budget.restore(phase);
        return Ok(array);
    }

    let value_type = dictionary.value();
    budget.add_default_scalar_scratch(value_type)?;
    if has_derived_logical_nulls(value_type) {
        budget.add_bitmap(1)?;
    }
    let default = value_type.default_arrow_scalar()?;
    if is_logically_null(default.array().as_ref(), 0) {
        budget.restore(phase);
        return Ok(array);
    }

    // Compact to values referenced by rows that survive the repair. Raw
    // dictionary vocabularies are not part of the logical output and may be
    // arbitrarily wider than the key capacity or materialization budget.
    let values = source.values();
    let mut used = scratch_vec::<usize>(budget, source.len(), "dictionary live-value indices")?;
    for index in 0..source.len() {
        if is_exposed(exposure, index) && logical.is_null(index) {
            continue;
        }
        if source.keys().is_valid(index) {
            used.push(source.keys().value(index).as_usize());
        }
    }
    used.sort_unstable();
    used.dedup();
    if used.iter().any(|index| *index >= values.len()) {
        return Err(Error::IncompatibleSchema(
            "dictionary key points outside its values array".to_owned(),
        ));
    }

    reserve_vec_bytes::<usize>(budget, used.len())?;
    let mut by_value = used.clone();
    let compare_values = make_yggdryl_key_comparator(value_type, values, budget)?;
    by_value.sort_unstable_by(|left, right| compare_values(*left, *right));
    let mut representatives = scratch_vec::<usize>(
        budget,
        by_value.len().saturating_add(1),
        "dictionary compact vocabulary",
    )?;
    let mut mappings =
        scratch_vec::<(usize, usize)>(budget, by_value.len(), "dictionary key remapping")?;
    for old in by_value {
        let group = if representatives
            .last()
            .is_some_and(|prior| compare_values(*prior, old) == Ordering::Equal)
        {
            representatives.len() - 1
        } else {
            representatives.push(old);
            representatives.len() - 1
        };
        mappings.push((old, group));
    }
    mappings.sort_unstable_by_key(|(old, _)| *old);

    // Only reachable representatives may be retained in the output
    // vocabulary. Searching the raw vocabulary here would make a one-row
    // dictionary over a very long run-end encoded value array take work
    // proportional to the hidden logical length.
    let compare_default = make_yggdryl_comparator(value_type, values, default.array(), budget)?;
    let mut default_index = representatives
        .iter()
        .position(|index| compare_default(*index, 0) == Ordering::Equal);
    let mut appended_default = false;
    if default_index.is_none() {
        default_index = Some(representatives.len());
        appended_default = true;
    }
    let default_index = default_index.ok_or_else(|| {
        Error::IncompatibleSchema("dictionary default index planning failed".to_owned())
    })?;
    let default_key = K::Native::try_from(default_index).map_err(|_| {
        Error::IncompatibleSchema(format!(
            "dictionary live values plus its default exceed the {} key capacity",
            dictionary.key()
        ))
    })?;

    budget.add_array_layout(field.data_type(), source.len())?;
    reserve_vec_bytes::<u32>(budget, representatives.len())?;
    let selected = representatives
        .iter()
        .map(|index| {
            u32::try_from(*index).map_err(|_| {
                Error::IncompatibleSchema(
                    "dictionary value index exceeds Arrow UInt32 take capacity".to_owned(),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let selected = UInt32Array::from(selected);
    let compact = if selected.is_empty() {
        None
    } else {
        budget.add_array(&DataType::UInt32, selected.len())?;
        reserve_source_selection(
            values.as_ref(),
            value_type,
            SourceSelection::Indices(selected.values()),
            budget,
        )?;
        Some(take(values.as_ref(), &selected, None)?)
    };
    let output_values = if appended_default {
        match compact {
            None => default.to_array(),
            Some(compact) => {
                let default_array = default.to_array();
                let (compact, default_array) = if contains_dictionary(value_type) {
                    let value_field = Field::new("dictionary", value_type.clone(), true);
                    align_nested_dictionaries(
                        &value_field,
                        &compact,
                        &default_array,
                        None,
                        None,
                        budget,
                    )?
                } else {
                    (compact, default_array)
                };
                reserve_concat_copy(compact.as_ref(), value_type, budget)?;
                budget.add_repeated_default(value_type, 1)?;
                concat(&[compact.as_ref(), default_array.as_ref()])?
            }
        }
    } else {
        compact.ok_or_else(|| {
            Error::IncompatibleSchema("dictionary compact vocabulary is empty".to_owned())
        })?
    };

    let mut keys = arrow_array::builder::PrimitiveBuilder::<K>::with_capacity(source.len());
    for index in 0..source.len() {
        if is_exposed(exposure, index) && logical.is_null(index) {
            keys.append_value(default_key);
        } else if source.keys().is_null(index) {
            keys.append_null();
        } else {
            let old = source.keys().value(index).as_usize();
            let position = mappings
                .binary_search_by_key(&old, |(candidate, _)| *candidate)
                .map_err(|_| {
                    Error::IncompatibleSchema(
                        "dictionary live key was not present in its compact mapping".to_owned(),
                    )
                })?;
            let key = K::Native::try_from(mappings[position].1).map_err(|_| {
                Error::IncompatibleSchema(
                    "dictionary compact key exceeds its physical key type".to_owned(),
                )
            })?;
            keys.append_value(key);
        }
    }
    let keys = keys.finish();
    let output = Arc::new(DictionaryArray::<K>::try_new(keys, output_values)?) as ArrayRef;

    budget.restore(phase);
    budget.add_array_layout(field.data_type(), source.len())?;
    reserve_new_dictionary_vocabularies(&output, &array, field.data_type(), budget)?;
    Ok(output)
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow concat's nested layout dispatch.
fn reserve_concat_copy(
    array: &dyn Array,
    data_type: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match data_type {
        DataType::BinaryView => {
            let array = downcast::<BinaryViewArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())?;
        }
        DataType::Utf8View => {
            let array = downcast::<StringViewArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, array.data_buffers().len())?;
        }
        // Dictionary inputs are vocabulary-aligned before concat, so Arrow
        // allocates only the concatenated key array and retains one vocab Arc.
        DataType::Dictionary(_) => budget.add_array_layout(data_type, array.len())?,
        DataType::List(child) => {
            let array = downcast::<ListArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let values = array.values().slice(start, end - start);
            reserve_concat_copy(values.as_ref(), child.data_type(), budget)?;
        }
        DataType::LargeList(child) => {
            let array = downcast::<LargeListArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let values = array.values().slice(start, end - start);
            reserve_concat_copy(values.as_ref(), child.data_type(), budget)?;
        }
        // Arrow concat preserves every ListView backing child, including
        // ranges not referenced by a logical view.
        DataType::ListView(child) => {
            let array = downcast::<ListViewArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.data_type(), budget)?;
        }
        DataType::LargeListView(child) => {
            let array = downcast::<LargeListViewArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.data_type(), budget)?;
        }
        DataType::FixedSizeList(child, _) => {
            let array = downcast::<FixedSizeListArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            reserve_concat_copy(array.values().as_ref(), child.data_type(), budget)?;
        }
        DataType::Struct(fields) => {
            let array = downcast::<StructArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            for (field, child) in fields.iter().zip(array.columns()) {
                reserve_concat_copy(child.as_ref(), field.data_type(), budget)?;
            }
        }
        DataType::Map(map) => {
            let array = downcast::<MapArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            let start = array.offsets()[0].as_usize();
            let end = array.offsets()[array.len()].as_usize();
            let entries: ArrayRef = Arc::new(array.entries().slice(start, end - start));
            reserve_concat_copy(entries.as_ref(), map.entries().data_type(), budget)?;
        }
        // Union uses MutableArrayData, which visits every physical child, not
        // only the active branch selected by each logical row.
        DataType::Union(fields, _) => {
            let array = downcast::<UnionArray>(array)?;
            budget.add_array_layout(data_type, array.len())?;
            for (type_id, field) in fields {
                reserve_concat_copy(array.child(type_id).as_ref(), field.data_type(), budget)?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let array = downcast::<RunArray<$run>>(array)?;
                    let values = array.values_slice();
                    budget.add_physical_slots(values.len())?;
                    budget.add_array(encoded.run_ends().data_type(), values.len())?;
                    reserve_concat_copy(values.as_ref(), encoded.values().data_type(), budget)?
                }};
            }
            match encoded.run_ends().data_type() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {
            let full = [(0, array.len())];
            reserve_source_selection(array, data_type, SourceSelection::Ranges(&full), budget)?;
        }
    }
    Ok(())
}

fn contains_dictionary(data_type: &DataType) -> bool {
    match data_type {
        DataType::Dictionary(_) => true,
        DataType::List(field)
        | DataType::ListView(field)
        | DataType::FixedSizeList(field, _)
        | DataType::LargeList(field)
        | DataType::LargeListView(field) => contains_dictionary(field.data_type()),
        DataType::Struct(fields) => fields
            .iter()
            .any(|field| contains_dictionary(field.data_type())),
        DataType::Union(fields, _) => fields
            .iter()
            .any(|(_, field)| contains_dictionary(field.data_type())),
        DataType::Map(map) => contains_dictionary(map.entries().data_type()),
        DataType::RunEndEncoded(encoded) => contains_dictionary(encoded.values().data_type()),
        _ => false,
    }
}

fn dictionary_values_ref<'a>(
    array: &'a dyn Array,
    dictionary: &crate::DictionaryType,
) -> Result<&'a ArrayRef> {
    macro_rules! values {
        ($key:ty) => {{ Ok(downcast::<DictionaryArray<$key>>(array)?.values()) }};
    }
    match dictionary.key() {
        DataType::Int8 => values!(Int8Type),
        DataType::Int16 => values!(Int16Type),
        DataType::Int32 => values!(Int32Type),
        DataType::Int64 => values!(Int64Type),
        DataType::UInt8 => values!(UInt8Type),
        DataType::UInt16 => values!(UInt16Type),
        DataType::UInt32 => values!(UInt32Type),
        DataType::UInt64 => values!(UInt64Type),
        key => Err(Error::Unsupported {
            kind: key.name(),
            reason: format!(
                "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        }),
    }
}

fn checked_valid_payload_bytes(
    len: usize,
    mut is_valid: impl FnMut(usize) -> bool,
    mut value_len: impl FnMut(usize) -> usize,
) -> Result<usize> {
    (0..len).try_fold(0usize, |bytes, index| {
        if !is_valid(index) {
            return Ok(bytes);
        }
        bytes
            .checked_add(value_len(index))
            .ok_or_else(|| Error::IncompatibleSchema("Arrow payload bytes exceed usize".to_owned()))
    })
}

#[allow(clippy::too_many_lines)] // Mirrors every nested Arrow container layout.
fn reserve_new_materialized_array_without_dictionary_values(
    output: &ArrayRef,
    source: &ArrayRef,
    data_type: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if Arc::ptr_eq(output, source) {
        return Ok(());
    }
    if !matches!(data_type, DataType::RunEndEncoded(_)) {
        budget.add_array_layout(data_type, output.len())?;
    }
    match data_type {
        DataType::Binary => {
            let output = downcast::<BinaryArray>(output.as_ref())?;
            budget.add_bytes(checked_valid_payload_bytes(
                output.len(),
                |index| output.is_valid(index),
                |index| output.value(index).len(),
            )?)?;
        }
        DataType::LargeBinary => {
            let output = downcast::<LargeBinaryArray>(output.as_ref())?;
            budget.add_bytes(checked_valid_payload_bytes(
                output.len(),
                |index| output.is_valid(index),
                |index| output.value(index).len(),
            )?)?;
        }
        DataType::Utf8 => {
            let output = downcast::<StringArray>(output.as_ref())?;
            budget.add_bytes(checked_valid_payload_bytes(
                output.len(),
                |index| output.is_valid(index),
                |index| output.value(index).len(),
            )?)?;
        }
        DataType::LargeUtf8 => {
            let output = downcast::<LargeStringArray>(output.as_ref())?;
            budget.add_bytes(checked_valid_payload_bytes(
                output.len(),
                |index| output.is_valid(index),
                |index| output.value(index).len(),
            )?)?;
        }
        DataType::BinaryView => reserve_new_view_buffers(
            downcast::<BinaryViewArray>(output.as_ref())?.data_buffers(),
            downcast::<BinaryViewArray>(source.as_ref())?.data_buffers(),
            budget,
        )?,
        DataType::Utf8View => reserve_new_view_buffers(
            downcast::<StringViewArray>(output.as_ref())?.data_buffers(),
            downcast::<StringViewArray>(source.as_ref())?.data_buffers(),
            budget,
        )?,
        DataType::List(child) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<ListArray>(output.as_ref())?.values(),
            downcast::<ListArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::LargeList(child) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<LargeListArray>(output.as_ref())?.values(),
            downcast::<LargeListArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::ListView(child) => reserve_new_materialized_array_without_dictionary_values(
            downcast::<ListViewArray>(output.as_ref())?.values(),
            downcast::<ListViewArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::LargeListView(child) => {
            reserve_new_materialized_array_without_dictionary_values(
                downcast::<LargeListViewArray>(output.as_ref())?.values(),
                downcast::<LargeListViewArray>(source.as_ref())?.values(),
                child.data_type(),
                budget,
            )?;
        }
        DataType::FixedSizeList(child, _) => {
            reserve_new_materialized_array_without_dictionary_values(
                downcast::<FixedSizeListArray>(output.as_ref())?.values(),
                downcast::<FixedSizeListArray>(source.as_ref())?.values(),
                child.data_type(),
                budget,
            )?;
        }
        DataType::Struct(fields) => {
            let output = downcast::<StructArray>(output.as_ref())?;
            let source = downcast::<StructArray>(source.as_ref())?;
            for ((field, output), source) in
                fields.iter().zip(output.columns()).zip(source.columns())
            {
                reserve_new_materialized_array_without_dictionary_values(
                    output,
                    source,
                    field.data_type(),
                    budget,
                )?;
            }
        }
        DataType::Map(map) => {
            let output: ArrayRef =
                Arc::new(downcast::<MapArray>(output.as_ref())?.entries().clone());
            let source: ArrayRef =
                Arc::new(downcast::<MapArray>(source.as_ref())?.entries().clone());
            reserve_new_materialized_array_without_dictionary_values(
                &output,
                &source,
                map.entries().data_type(),
                budget,
            )?;
        }
        DataType::Union(fields, _) => {
            let output = downcast::<UnionArray>(output.as_ref())?;
            let source = downcast::<UnionArray>(source.as_ref())?;
            for (type_id, field) in fields {
                reserve_new_materialized_array_without_dictionary_values(
                    output.child(type_id),
                    source.child(type_id),
                    field.data_type(),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let output = downcast::<RunArray<$run>>(output.as_ref())?;
                    let source = downcast::<RunArray<$run>>(source.as_ref())?;
                    budget.add_physical_slots(output.values().len())?;
                    budget.add_array(encoded.run_ends().data_type(), output.values().len())?;
                    reserve_new_materialized_array_without_dictionary_values(
                        output.values(),
                        source.values(),
                        encoded.values().data_type(),
                        budget,
                    )?;
                }};
            }
            match encoded.run_ends().data_type() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn reserve_new_view_buffers(
    output: &[arrow_buffer::Buffer],
    source: &[arrow_buffer::Buffer],
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, output.len())?;
    let shared_prefix = output
        .iter()
        .zip(source)
        .take_while(|(output, source)| output.ptr_eq(source))
        .count();
    if output.len() == source.len() && shared_prefix == source.len() {
        return Ok(());
    }

    let phase = budget.mark();
    let mut source_buffers =
        scratch_vec::<(usize, usize)>(budget, source.len(), "ByteView shared-buffer identities")?;
    source_buffers.extend(
        source
            .iter()
            .map(|buffer| (buffer.as_ptr() as usize, buffer.len())),
    );
    source_buffers.sort_unstable();
    source_buffers.dedup();
    let mut new_buffers =
        scratch_vec::<(usize, usize)>(budget, output.len(), "ByteView new-buffer identities")?;
    for buffer in output {
        let identity = (buffer.as_ptr() as usize, buffer.len());
        if source_buffers.binary_search(&identity).is_err() {
            new_buffers.push(identity);
        }
    }
    new_buffers.sort_unstable();
    new_buffers.dedup();
    let new_bytes = new_buffers.iter().try_fold(0usize, |bytes, (_, len)| {
        bytes.checked_add(*len).ok_or_else(|| {
            Error::IncompatibleSchema("ByteView backing bytes exceed usize".to_owned())
        })
    })?;
    budget.restore(phase);
    budget.add_bytes(new_bytes)?;
    Ok(())
}

#[allow(clippy::too_many_lines)] // Mirrors every nested Arrow container layout.
fn reserve_new_dictionary_vocabularies(
    output: &ArrayRef,
    source: &ArrayRef,
    data_type: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    match data_type {
        DataType::Dictionary(dictionary) => {
            let output_values = dictionary_values_ref(output.as_ref(), dictionary)?;
            let source_values = dictionary_values_ref(source.as_ref(), dictionary)?;
            let shared = if Arc::ptr_eq(output_values, source_values)
                || byte_array_storage_ptr_eq(
                    output_values.as_ref(),
                    source_values.as_ref(),
                    dictionary.value(),
                )? {
                true
            } else {
                let phase = budget.mark();
                reserve_to_data_scratch(output_values, budget)?;
                reserve_to_data_scratch(source_values, budget)?;
                let shared = output_values.to_data().ptr_eq(&source_values.to_data());
                budget.restore(phase);
                shared
            };
            if !shared {
                reserve_new_materialized_array_without_dictionary_values(
                    output_values,
                    source_values,
                    dictionary.value(),
                    budget,
                )?;
                reserve_new_dictionary_vocabularies(
                    output_values,
                    source_values,
                    dictionary.value(),
                    budget,
                )?;
            }
        }
        DataType::Struct(fields) => {
            let output = downcast::<StructArray>(output.as_ref())?;
            let source = downcast::<StructArray>(source.as_ref())?;
            for ((field, output), source) in
                fields.iter().zip(output.columns()).zip(source.columns())
            {
                reserve_new_dictionary_vocabularies(output, source, field.data_type(), budget)?;
            }
        }
        DataType::List(child) => reserve_new_dictionary_vocabularies(
            downcast::<ListArray>(output.as_ref())?.values(),
            downcast::<ListArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::LargeList(child) => reserve_new_dictionary_vocabularies(
            downcast::<LargeListArray>(output.as_ref())?.values(),
            downcast::<LargeListArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::ListView(child) => reserve_new_dictionary_vocabularies(
            downcast::<ListViewArray>(output.as_ref())?.values(),
            downcast::<ListViewArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::LargeListView(child) => reserve_new_dictionary_vocabularies(
            downcast::<LargeListViewArray>(output.as_ref())?.values(),
            downcast::<LargeListViewArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::FixedSizeList(child, _) => reserve_new_dictionary_vocabularies(
            downcast::<FixedSizeListArray>(output.as_ref())?.values(),
            downcast::<FixedSizeListArray>(source.as_ref())?.values(),
            child.data_type(),
            budget,
        )?,
        DataType::Map(map) => {
            let output: ArrayRef =
                Arc::new(downcast::<MapArray>(output.as_ref())?.entries().clone());
            let source: ArrayRef =
                Arc::new(downcast::<MapArray>(source.as_ref())?.entries().clone());
            reserve_new_dictionary_vocabularies(
                &output,
                &source,
                map.entries().data_type(),
                budget,
            )?;
        }
        DataType::Union(fields, _) => {
            let output = downcast::<UnionArray>(output.as_ref())?;
            let source = downcast::<UnionArray>(source.as_ref())?;
            for (type_id, field) in fields {
                reserve_new_dictionary_vocabularies(
                    output.child(type_id),
                    source.child(type_id),
                    field.data_type(),
                    budget,
                )?;
            }
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! reserve_run {
                ($run:ty) => {{
                    let output = downcast::<RunArray<$run>>(output.as_ref())?;
                    let source = downcast::<RunArray<$run>>(source.as_ref())?;
                    reserve_new_dictionary_vocabularies(
                        output.values(),
                        source.values(),
                        encoded.values().data_type(),
                        budget,
                    )?;
                }};
            }
            match encoded.run_ends().data_type() {
                DataType::Int16 => reserve_run!(Int16Type),
                DataType::Int32 => reserve_run!(Int32Type),
                DataType::Int64 => reserve_run!(Int64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[allow(clippy::too_many_lines)] // Mirrors Arrow ArrayData buffers and child trees.
fn reserve_to_data_scratch(array: &ArrayRef, budget: &mut MaterializationBudget) -> Result<()> {
    let buffer_count = match array.data_type() {
        ArrowDataType::Null
        | ArrowDataType::Struct(_)
        | ArrowDataType::FixedSizeList(_, _)
        | ArrowDataType::RunEndEncoded(_, _) => 0,
        ArrowDataType::Binary
        | ArrowDataType::LargeBinary
        | ArrowDataType::Utf8
        | ArrowDataType::LargeUtf8
        | ArrowDataType::ListView(_)
        | ArrowDataType::LargeListView(_)
        | ArrowDataType::Union(_, arrow_schema::UnionMode::Dense) => 2,
        ArrowDataType::BinaryView => downcast::<BinaryViewArray>(array.as_ref())?
            .data_buffers()
            .len()
            .checked_add(1)
            .ok_or_else(|| {
                Error::IncompatibleSchema("BinaryView buffer count exceeds usize".to_owned())
            })?,
        ArrowDataType::Utf8View => downcast::<StringViewArray>(array.as_ref())?
            .data_buffers()
            .len()
            .checked_add(1)
            .ok_or_else(|| {
                Error::IncompatibleSchema("Utf8View buffer count exceeds usize".to_owned())
            })?,
        _ => 1,
    };
    reserve_vec_bytes::<arrow_buffer::Buffer>(budget, buffer_count)?;

    macro_rules! one_child {
        ($child:expr) => {{
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            reserve_to_data_scratch($child, budget)?;
        }};
    }
    match array.data_type() {
        ArrowDataType::List(_) => one_child!(downcast::<ListArray>(array.as_ref())?.values()),
        ArrowDataType::LargeList(_) => {
            one_child!(downcast::<LargeListArray>(array.as_ref())?.values());
        }
        ArrowDataType::ListView(_) => {
            one_child!(downcast::<ListViewArray>(array.as_ref())?.values());
        }
        ArrowDataType::LargeListView(_) => {
            one_child!(downcast::<LargeListViewArray>(array.as_ref())?.values());
        }
        ArrowDataType::FixedSizeList(_, _) => {
            one_child!(downcast::<FixedSizeListArray>(array.as_ref())?.values());
        }
        ArrowDataType::Struct(fields) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, fields.len())?;
            for child in downcast::<StructArray>(array.as_ref())?.columns() {
                reserve_to_data_scratch(child, budget)?;
            }
        }
        ArrowDataType::Map(_, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            let entries: ArrayRef =
                Arc::new(downcast::<MapArray>(array.as_ref())?.entries().clone());
            reserve_to_data_scratch(&entries, budget)?;
        }
        ArrowDataType::Dictionary(key, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 1)?;
            macro_rules! values {
                ($key:ty) => {
                    reserve_to_data_scratch(
                        downcast::<DictionaryArray<$key>>(array.as_ref())?.values(),
                        budget,
                    )?
                };
            }
            match key.as_ref() {
                ArrowDataType::Int8 => values!(Int8Type),
                ArrowDataType::Int16 => values!(Int16Type),
                ArrowDataType::Int32 => values!(Int32Type),
                ArrowDataType::Int64 => values!(Int64Type),
                ArrowDataType::UInt8 => values!(UInt8Type),
                ArrowDataType::UInt16 => values!(UInt16Type),
                ArrowDataType::UInt32 => values!(UInt32Type),
                ArrowDataType::UInt64 => values!(UInt64Type),
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "dictionary key is not a supported integer".to_owned(),
                    ));
                }
            }
        }
        ArrowDataType::Union(fields, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, fields.len())?;
            let array = downcast::<UnionArray>(array.as_ref())?;
            for (type_id, _) in fields.iter() {
                reserve_to_data_scratch(array.child(type_id), budget)?;
            }
        }
        ArrowDataType::RunEndEncoded(run_ends, _) => {
            reserve_vec_bytes::<arrow_data::ArrayData>(budget, 2)?;
            reserve_vec_bytes::<arrow_buffer::Buffer>(budget, 1)?;
            match run_ends.data_type() {
                ArrowDataType::Int16 => {
                    reserve_to_data_scratch(
                        downcast::<Int16RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                ArrowDataType::Int32 => {
                    reserve_to_data_scratch(
                        downcast::<Int32RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                ArrowDataType::Int64 => {
                    reserve_to_data_scratch(
                        downcast::<Int64RunArray>(array.as_ref())?.values(),
                        budget,
                    )?;
                }
                _ => {
                    return Err(Error::IncompatibleSchema(
                        "run-end type is not a supported signed integer".to_owned(),
                    ));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn byte_array_storage_ptr_eq(
    left: &dyn Array,
    right: &dyn Array,
    data_type: &DataType,
) -> Result<bool> {
    macro_rules! shared {
        ($array:ty) => {{
            let left = downcast::<$array>(left)?;
            let right = downcast::<$array>(right)?;
            left.offsets().ptr_eq(right.offsets())
                && byte_slices_ptr_eq(left.value_data(), right.value_data())
                && null_buffers_ptr_eq(left.nulls(), right.nulls())
        }};
    }
    Ok(match data_type {
        DataType::Binary => shared!(BinaryArray),
        DataType::LargeBinary => shared!(LargeBinaryArray),
        DataType::Utf8 => shared!(StringArray),
        DataType::LargeUtf8 => shared!(LargeStringArray),
        DataType::FixedSizeBinary(_) => {
            let left = downcast::<FixedSizeBinaryArray>(left)?;
            let right = downcast::<FixedSizeBinaryArray>(right)?;
            byte_slices_ptr_eq(left.value_data(), right.value_data())
                && null_buffers_ptr_eq(left.nulls(), right.nulls())
        }
        _ => false,
    })
}

fn byte_slices_ptr_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len() && (left.is_empty() || std::ptr::eq(left.as_ptr(), right.as_ptr()))
}

fn replace_array_children(
    array: &ArrayRef,
    children: Vec<ArrayRef>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if array_children_unchanged(array, &children)? {
        return Ok(Arc::clone(array));
    }

    // The old recursive ArrayData tree and every replacement child-data tree
    // coexist until the rebuilt root takes ownership of the new child Vec.
    let phase = budget.mark();
    reserve_to_data_scratch(array, budget)?;
    reserve_vec_bytes::<arrow_data::ArrayData>(budget, children.len())?;
    for child in &children {
        reserve_to_data_scratch(child, budget)?;
    }
    let data = array.to_data();
    if data.child_data().len() != children.len() {
        return Err(Error::IncompatibleSchema(
            "dictionary alignment produced the wrong child count".to_owned(),
        ));
    }
    let children = children.into_iter().map(|child| child.to_data()).collect();
    let output = make_array(data.into_builder().child_data(children).build()?);
    // ArrayData handle Vecs are phase-local. The returned concrete array owns
    // the already-reserved child arrays/buffers, not these temporary clones.
    budget.restore(phase);
    Ok(output)
}

fn array_children_unchanged(array: &ArrayRef, children: &[ArrayRef]) -> Result<bool> {
    let unchanged = match array.data_type() {
        ArrowDataType::Struct(_) => {
            let source = downcast::<StructArray>(array.as_ref())?;
            source.columns().len() == children.len()
                && source
                    .columns()
                    .iter()
                    .zip(children)
                    .all(|(source, child)| Arc::ptr_eq(source, child))
        }
        ArrowDataType::List(_) => {
            children.len() == 1
                && Arc::ptr_eq(
                    downcast::<ListArray>(array.as_ref())?.values(),
                    &children[0],
                )
        }
        ArrowDataType::LargeList(_) => {
            children.len() == 1
                && Arc::ptr_eq(
                    downcast::<LargeListArray>(array.as_ref())?.values(),
                    &children[0],
                )
        }
        ArrowDataType::ListView(_) => {
            children.len() == 1
                && Arc::ptr_eq(
                    downcast::<ListViewArray>(array.as_ref())?.values(),
                    &children[0],
                )
        }
        ArrowDataType::LargeListView(_) => {
            children.len() == 1
                && Arc::ptr_eq(
                    downcast::<LargeListViewArray>(array.as_ref())?.values(),
                    &children[0],
                )
        }
        ArrowDataType::FixedSizeList(_, _) => {
            children.len() == 1
                && Arc::ptr_eq(
                    downcast::<FixedSizeListArray>(array.as_ref())?.values(),
                    &children[0],
                )
        }
        ArrowDataType::Map(_, _) => {
            let source = downcast::<MapArray>(array.as_ref())?.entries();
            let target = children
                .first()
                .and_then(|child| child.as_any().downcast_ref::<StructArray>());
            target.is_some_and(|target| {
                source.columns().len() == target.columns().len()
                    && source
                        .columns()
                        .iter()
                        .zip(target.columns())
                        .all(|(source, target)| Arc::ptr_eq(source, target))
                    && null_buffers_ptr_eq(source.nulls(), target.nulls())
            })
        }
        ArrowDataType::Union(fields, _) => {
            let source = downcast::<UnionArray>(array.as_ref())?;
            fields.len() == children.len()
                && fields
                    .iter()
                    .zip(children)
                    .all(|((type_id, _), child)| Arc::ptr_eq(source.child(type_id), child))
        }
        ArrowDataType::RunEndEncoded(run_ends, _) if children.len() == 2 => {
            match run_ends.data_type() {
                ArrowDataType::Int16 => Arc::ptr_eq(
                    downcast::<Int16RunArray>(array.as_ref())?.values(),
                    &children[1],
                ),
                ArrowDataType::Int32 => Arc::ptr_eq(
                    downcast::<Int32RunArray>(array.as_ref())?.values(),
                    &children[1],
                ),
                ArrowDataType::Int64 => Arc::ptr_eq(
                    downcast::<Int64RunArray>(array.as_ref())?.values(),
                    &children[1],
                ),
                _ => false,
            }
        }
        _ => false,
    };
    Ok(unchanged)
}

fn null_buffers_ptr_eq(
    left: Option<&arrow_buffer::NullBuffer>,
    right: Option<&arrow_buffer::NullBuffer>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left.inner().ptr_eq(right.inner()),
        _ => false,
    }
}

#[allow(clippy::too_many_lines)]
fn align_nested_dictionaries(
    field: &Field,
    left: &ArrayRef,
    right: &ArrayRef,
    left_exposure: Option<&BooleanBuffer>,
    right_exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<(ArrayRef, ArrayRef)> {
    if !contains_dictionary(field.data_type()) {
        return Ok((Arc::clone(left), Arc::clone(right)));
    }
    if left.data_type() != right.data_type() {
        return Err(Error::IncompatibleSchema(
            "dictionary alignment inputs have different physical datatypes".to_owned(),
        ));
    }
    if left_exposure.is_some_and(|exposure| exposure.len() != left.len())
        || right_exposure.is_some_and(|exposure| exposure.len() != right.len())
    {
        return Err(Error::IncompatibleSchema(
            "dictionary alignment exposure has the wrong length".to_owned(),
        ));
    }

    match field.data_type() {
        DataType::Dictionary(dictionary) => align_dictionary_arrays(
            field,
            dictionary,
            left,
            right,
            left_exposure,
            right_exposure,
            budget,
        ),
        DataType::Struct(fields) => {
            let left_struct = downcast::<StructArray>(left.as_ref())?;
            let right_struct = downcast::<StructArray>(right.as_ref())?;
            let left_child_exposure = visible_array_exposure(left.as_ref(), left_exposure, budget)?;
            let right_child_exposure =
                visible_array_exposure(right.as_ref(), right_exposure, budget)?;
            let mut left_children =
                scratch_vec::<ArrayRef>(budget, fields.len(), "left Struct child arrays")?;
            let mut right_children =
                scratch_vec::<ArrayRef>(budget, fields.len(), "right Struct child arrays")?;
            for (index, child_field) in fields.iter().enumerate() {
                let (left_child, right_child) = align_nested_dictionaries(
                    child_field,
                    left_struct.column(index),
                    right_struct.column(index),
                    left_child_exposure.as_ref(),
                    right_child_exposure.as_ref(),
                    budget,
                )?;
                left_children.push(left_child);
                right_children.push(right_child);
            }
            Ok((
                replace_array_children(left, left_children, budget)?,
                replace_array_children(right, right_children, budget)?,
            ))
        }
        DataType::List(child) => {
            let left_list = downcast::<ListArray>(left.as_ref())?;
            let right_list = downcast::<ListArray>(right.as_ref())?;
            let left_child_exposure = range_exposure(
                left_list.values().len(),
                left_list.len(),
                left_exposure,
                |row| left_list.is_valid(row),
                |row| {
                    offset_pair(
                        i64::from(left_list.offsets()[row]),
                        i64::from(left_list.offsets()[row + 1]),
                    )
                },
                budget,
            )?;
            let right_child_exposure = range_exposure(
                right_list.values().len(),
                right_list.len(),
                right_exposure,
                |row| right_list.is_valid(row),
                |row| {
                    offset_pair(
                        i64::from(right_list.offsets()[row]),
                        i64::from(right_list.offsets()[row + 1]),
                    )
                },
                budget,
            )?;
            let (left_child, right_child) = align_nested_dictionaries(
                child,
                left_list.values(),
                right_list.values(),
                left_child_exposure.as_ref(),
                right_child_exposure.as_ref(),
                budget,
            )?;
            Ok((
                replace_array_children(left, vec![left_child], budget)?,
                replace_array_children(right, vec![right_child], budget)?,
            ))
        }
        DataType::LargeList(child) => {
            let left_list = downcast::<LargeListArray>(left.as_ref())?;
            let right_list = downcast::<LargeListArray>(right.as_ref())?;
            let left_child_exposure = range_exposure(
                left_list.values().len(),
                left_list.len(),
                left_exposure,
                |row| left_list.is_valid(row),
                |row| offset_pair(left_list.offsets()[row], left_list.offsets()[row + 1]),
                budget,
            )?;
            let right_child_exposure = range_exposure(
                right_list.values().len(),
                right_list.len(),
                right_exposure,
                |row| right_list.is_valid(row),
                |row| offset_pair(right_list.offsets()[row], right_list.offsets()[row + 1]),
                budget,
            )?;
            let (left_child, right_child) = align_nested_dictionaries(
                child,
                left_list.values(),
                right_list.values(),
                left_child_exposure.as_ref(),
                right_child_exposure.as_ref(),
                budget,
            )?;
            Ok((
                replace_array_children(left, vec![left_child], budget)?,
                replace_array_children(right, vec![right_child], budget)?,
            ))
        }
        DataType::ListView(child) => {
            let left_list = downcast::<ListViewArray>(left.as_ref())?;
            let right_list = downcast::<ListViewArray>(right.as_ref())?;
            let left_child_exposure = range_exposure(
                left_list.values().len(),
                left_list.len(),
                left_exposure,
                |row| left_list.is_valid(row),
                |row| {
                    offset_size(
                        i64::from(left_list.offsets()[row]),
                        i64::from(left_list.sizes()[row]),
                    )
                },
                budget,
            )?;
            let right_child_exposure = range_exposure(
                right_list.values().len(),
                right_list.len(),
                right_exposure,
                |row| right_list.is_valid(row),
                |row| {
                    offset_size(
                        i64::from(right_list.offsets()[row]),
                        i64::from(right_list.sizes()[row]),
                    )
                },
                budget,
            )?;
            let (left_child, right_child) = align_nested_dictionaries(
                child,
                left_list.values(),
                right_list.values(),
                left_child_exposure.as_ref(),
                right_child_exposure.as_ref(),
                budget,
            )?;
            Ok((
                replace_array_children(left, vec![left_child], budget)?,
                replace_array_children(right, vec![right_child], budget)?,
            ))
        }
        DataType::LargeListView(child) => {
            let left_list = downcast::<LargeListViewArray>(left.as_ref())?;
            let right_list = downcast::<LargeListViewArray>(right.as_ref())?;
            let left_child_exposure = range_exposure(
                left_list.values().len(),
                left_list.len(),
                left_exposure,
                |row| left_list.is_valid(row),
                |row| offset_size(left_list.offsets()[row], left_list.sizes()[row]),
                budget,
            )?;
            let right_child_exposure = range_exposure(
                right_list.values().len(),
                right_list.len(),
                right_exposure,
                |row| right_list.is_valid(row),
                |row| offset_size(right_list.offsets()[row], right_list.sizes()[row]),
                budget,
            )?;
            let (left_child, right_child) = align_nested_dictionaries(
                child,
                left_list.values(),
                right_list.values(),
                left_child_exposure.as_ref(),
                right_child_exposure.as_ref(),
                budget,
            )?;
            Ok((
                replace_array_children(left, vec![left_child], budget)?,
                replace_array_children(right, vec![right_child], budget)?,
            ))
        }
        DataType::FixedSizeList(child, size) => {
            let left_list = downcast::<FixedSizeListArray>(left.as_ref())?;
            let right_list = downcast::<FixedSizeListArray>(right.as_ref())?;
            let width = usize::try_from(*size)
                .map_err(|_| Error::IncompatibleSchema("fixed-list size is negative".to_owned()))?;
            let left_child_exposure = range_exposure(
                left_list.values().len(),
                left_list.len(),
                left_exposure,
                |row| left_list.is_valid(row),
                |row| {
                    let start = usize::try_from(left_list.value_offset(row)).map_err(|_| {
                        Error::IncompatibleSchema(
                            "fixed-list offset is negative or exceeds usize".to_owned(),
                        )
                    })?;
                    Ok((start, start + width))
                },
                budget,
            )?;
            let right_child_exposure = range_exposure(
                right_list.values().len(),
                right_list.len(),
                right_exposure,
                |row| right_list.is_valid(row),
                |row| {
                    let start = usize::try_from(right_list.value_offset(row)).map_err(|_| {
                        Error::IncompatibleSchema(
                            "fixed-list offset is negative or exceeds usize".to_owned(),
                        )
                    })?;
                    Ok((start, start + width))
                },
                budget,
            )?;
            let (left_child, right_child) = align_nested_dictionaries(
                child,
                left_list.values(),
                right_list.values(),
                left_child_exposure.as_ref(),
                right_child_exposure.as_ref(),
                budget,
            )?;
            Ok((
                replace_array_children(left, vec![left_child], budget)?,
                replace_array_children(right, vec![right_child], budget)?,
            ))
        }
        DataType::Map(map) => {
            let left_map = downcast::<MapArray>(left.as_ref())?;
            let right_map = downcast::<MapArray>(right.as_ref())?;
            let left_entry_exposure = range_exposure(
                left_map.entries().len(),
                left_map.len(),
                left_exposure,
                |row| left_map.is_valid(row),
                |row| {
                    offset_pair(
                        i64::from(left_map.offsets()[row]),
                        i64::from(left_map.offsets()[row + 1]),
                    )
                },
                budget,
            )?;
            let right_entry_exposure = range_exposure(
                right_map.entries().len(),
                right_map.len(),
                right_exposure,
                |row| right_map.is_valid(row),
                |row| {
                    offset_pair(
                        i64::from(right_map.offsets()[row]),
                        i64::from(right_map.offsets()[row + 1]),
                    )
                },
                budget,
            )?;
            let left_entries: ArrayRef = Arc::new(left_map.entries().clone());
            let right_entries: ArrayRef = Arc::new(right_map.entries().clone());
            let (left_entries, right_entries) = align_nested_dictionaries(
                map.entries(),
                &left_entries,
                &right_entries,
                left_entry_exposure.as_ref(),
                right_entry_exposure.as_ref(),
                budget,
            )?;
            Ok((
                replace_array_children(left, vec![left_entries], budget)?,
                replace_array_children(right, vec![right_entries], budget)?,
            ))
        }
        DataType::Union(fields, _) => {
            let left_union = downcast::<UnionArray>(left.as_ref())?;
            let right_union = downcast::<UnionArray>(right.as_ref())?;
            let mut left_children =
                scratch_vec::<ArrayRef>(budget, fields.len(), "left Union child arrays")?;
            let mut right_children =
                scratch_vec::<ArrayRef>(budget, fields.len(), "right Union child arrays")?;
            for (type_id, child_field) in fields {
                let left_child = left_union.child(type_id);
                let right_child = right_union.child(type_id);
                let left_child_exposure = selected_index_exposure(
                    left_child.len(),
                    left_union.len(),
                    left_exposure,
                    |row| {
                        (left_union.type_id(row) == type_id).then(|| left_union.value_offset(row))
                    },
                    budget,
                )?;
                let right_child_exposure = selected_index_exposure(
                    right_child.len(),
                    right_union.len(),
                    right_exposure,
                    |row| {
                        (right_union.type_id(row) == type_id).then(|| right_union.value_offset(row))
                    },
                    budget,
                )?;
                let (left_child, right_child) = align_nested_dictionaries(
                    child_field,
                    left_child,
                    right_child,
                    left_child_exposure.as_ref(),
                    right_child_exposure.as_ref(),
                    budget,
                )?;
                left_children.push(left_child);
                right_children.push(right_child);
            }
            Ok((
                replace_array_children(left, left_children, budget)?,
                replace_array_children(right, right_children, budget)?,
            ))
        }
        DataType::RunEndEncoded(encoded) => {
            macro_rules! align_run {
                ($run:ty) => {{
                    let left_run = downcast::<RunArray<$run>>(left.as_ref())?;
                    let right_run = downcast::<RunArray<$run>>(right.as_ref())?;
                    let left_value_exposure = selected_index_exposure(
                        left_run.values().len(),
                        left_run.len(),
                        left_exposure,
                        |row| Some(left_run.run_ends().get_physical_index(row)),
                        budget,
                    )?;
                    let right_value_exposure = selected_index_exposure(
                        right_run.values().len(),
                        right_run.len(),
                        right_exposure,
                        |row| Some(right_run.run_ends().get_physical_index(row)),
                        budget,
                    )?;
                    let (left_values, right_values) = align_nested_dictionaries(
                        encoded.values(),
                        left_run.values(),
                        right_run.values(),
                        left_value_exposure.as_ref(),
                        right_value_exposure.as_ref(),
                        budget,
                    )?;
                    let left_run_ends: ArrayRef = Arc::new(PrimitiveArray::<$run>::new(
                        left_run.run_ends().inner().clone(),
                        None,
                    ));
                    let right_run_ends: ArrayRef = Arc::new(PrimitiveArray::<$run>::new(
                        right_run.run_ends().inner().clone(),
                        None,
                    ));
                    Ok((
                        replace_array_children(left, vec![left_run_ends, left_values], budget)?,
                        replace_array_children(right, vec![right_run_ends, right_values], budget)?,
                    ))
                }};
            }
            match encoded.run_ends().data_type() {
                DataType::Int16 => align_run!(Int16Type),
                DataType::Int32 => align_run!(Int32Type),
                DataType::Int64 => align_run!(Int64Type),
                _ => Err(Error::IncompatibleSchema(
                    "run-end type is not a supported signed integer".to_owned(),
                )),
            }
        }
        _ => Ok((Arc::clone(left), Arc::clone(right))),
    }
}

fn align_dictionary_arrays(
    field: &Field,
    dictionary: &crate::DictionaryType,
    left: &ArrayRef,
    right: &ArrayRef,
    left_exposure: Option<&BooleanBuffer>,
    right_exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<(ArrayRef, ArrayRef)> {
    macro_rules! align {
        ($key:ty) => {{
            align_dictionary_arrays_typed::<$key>(
                field,
                dictionary,
                left,
                right,
                left_exposure,
                right_exposure,
                budget,
            )
        }};
    }
    match dictionary.key() {
        DataType::Int8 => align!(Int8Type),
        DataType::Int16 => align!(Int16Type),
        DataType::Int32 => align!(Int32Type),
        DataType::Int64 => align!(Int64Type),
        DataType::UInt8 => align!(UInt8Type),
        DataType::UInt16 => align!(UInt16Type),
        DataType::UInt32 => align!(UInt32Type),
        DataType::UInt64 => align!(UInt64Type),
        key => Err(Error::Unsupported {
            kind: key.name(),
            reason: format!(
                "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        }),
    }
}

#[derive(Clone, Copy)]
enum DictionaryCandidate {
    Left(usize),
    Right(usize),
}

fn compare_dictionary_candidates(
    left: DictionaryCandidate,
    right: DictionaryCandidate,
    compare_left: &DynComparator,
    compare_right: &DynComparator,
    compare_cross: &DynComparator,
) -> Ordering {
    match (left, right) {
        (DictionaryCandidate::Left(left), DictionaryCandidate::Left(right)) => {
            compare_left(left, right)
        }
        (DictionaryCandidate::Right(left), DictionaryCandidate::Right(right)) => {
            compare_right(left, right)
        }
        (DictionaryCandidate::Left(left), DictionaryCandidate::Right(right)) => {
            compare_cross(left, right)
        }
        (DictionaryCandidate::Right(left), DictionaryCandidate::Left(right)) => {
            compare_cross(right, left).reverse()
        }
    }
}

fn dictionary_live_indices<K: ArrowDictionaryKeyType>(
    source: &DictionaryArray<K>,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<Vec<usize>> {
    let mut used = scratch_vec::<usize>(budget, source.len(), "dictionary alignment live values")?;
    for row in 0..source.len() {
        if is_exposed(exposure, row) && source.keys().is_valid(row) {
            let index = source.keys().value(row).as_usize();
            if index >= source.values().len() {
                return Err(Error::IncompatibleSchema(
                    "dictionary key points outside its values array".to_owned(),
                ));
            }
            used.push(index);
        }
    }
    used.sort_unstable();
    used.dedup();
    Ok(used)
}

fn remap_dictionary_to_values<K>(
    field: &Field,
    source: &DictionaryArray<K>,
    exposure: Option<&BooleanBuffer>,
    mappings: &[(usize, usize)],
    values: ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef>
where
    K: ArrowDictionaryKeyType,
    K::Native: TryFrom<usize>,
{
    budget.add_array_layout(field.data_type(), source.len())?;
    let fallback = (!values.is_empty())
        .then(|| K::Native::try_from(0).ok())
        .flatten();
    let mut keys = arrow_array::builder::PrimitiveBuilder::<K>::with_capacity(source.len());
    for row in 0..source.len() {
        if source.keys().is_null(row) {
            keys.append_null();
            continue;
        }
        let old = source.keys().value(row).as_usize();
        if is_exposed(exposure, row) {
            let position = mappings
                .binary_search_by_key(&old, |(candidate, _)| *candidate)
                .map_err(|_| {
                    Error::IncompatibleSchema(
                        "live dictionary key is absent from its vocabulary remap".to_owned(),
                    )
                })?;
            let key = K::Native::try_from(mappings[position].1).map_err(|_| {
                Error::IncompatibleSchema(
                    "dictionary compact key exceeds its physical key type".to_owned(),
                )
            })?;
            keys.append_value(key);
        } else if let Some(fallback) = fallback {
            keys.append_value(fallback);
        } else {
            keys.append_null();
        }
    }
    Ok(Arc::new(DictionaryArray::<K>::try_new(
        keys.finish(),
        values,
    )?))
}

fn take_dictionary_candidates(
    values: &ArrayRef,
    value_type: &DataType,
    selected: &[usize],
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if selected.is_empty() {
        return Ok(arrow_array::new_empty_array(values.data_type()));
    }
    budget.add_array(&DataType::UInt32, selected.len())?;
    reserve_vec_bytes::<u32>(budget, selected.len())?;
    let indices = selected
        .iter()
        .map(|index| {
            u32::try_from(*index).map_err(|_| {
                Error::IncompatibleSchema(
                    "dictionary value index exceeds Arrow UInt32 take capacity".to_owned(),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let indices = UInt32Array::from(indices);
    reserve_source_selection(
        values.as_ref(),
        value_type,
        SourceSelection::Indices(indices.values()),
        budget,
    )?;
    take(values.as_ref(), &indices, None).map_err(Into::into)
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn align_dictionary_arrays_typed<K>(
    field: &Field,
    dictionary: &crate::DictionaryType,
    left: &ArrayRef,
    right: &ArrayRef,
    left_exposure: Option<&BooleanBuffer>,
    right_exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<(ArrayRef, ArrayRef)>
where
    K: ArrowDictionaryKeyType,
    K::Native: TryFrom<usize>,
{
    let left_source = downcast::<DictionaryArray<K>>(left.as_ref())?;
    let right_source = downcast::<DictionaryArray<K>>(right.as_ref())?;
    if Arc::ptr_eq(left_source.values(), right_source.values()) {
        return Ok((Arc::clone(left), Arc::clone(right)));
    }
    let left_used = dictionary_live_indices(left_source, left_exposure, budget)?;
    let right_used = dictionary_live_indices(right_source, right_exposure, budget)?;
    let value_type = dictionary.value();
    let compare_left = make_yggdryl_key_comparator(value_type, left_source.values(), budget)?;
    let compare_right = make_yggdryl_key_comparator(value_type, right_source.values(), budget)?;
    let compare_cross = make_yggdryl_comparator(
        value_type,
        left_source.values(),
        right_source.values(),
        budget,
    )?;

    // Reusing either vocabulary is allocation-free for its owner and makes
    // Arrow's recursive zip path share, rather than concatenate, that
    // vocabulary. Compare only reachable entries: a transparent wrapper may
    // have a tiny physical representation but an arbitrarily long hidden
    // logical vocabulary.
    let mut right_to_left =
        scratch_vec::<(usize, usize)>(budget, right_used.len(), "dictionary right-to-left remap")?;
    if right_used.len() <= HASHED_NAME_INDEX_THRESHOLD {
        for right_index in &right_used {
            let Some(left_index) = left_used
                .iter()
                .copied()
                .find(|left_index| compare_cross(*left_index, *right_index) == Ordering::Equal)
            else {
                right_to_left.clear();
                break;
            };
            right_to_left.push((*right_index, left_index));
        }
    }
    if right_to_left.len() == right_used.len() {
        right_to_left.sort_unstable_by_key(|(old, _)| *old);
        let right = remap_dictionary_to_values(
            field,
            right_source,
            right_exposure,
            &right_to_left,
            Arc::clone(left_source.values()),
            budget,
        )?;
        return Ok((Arc::clone(left), right));
    }

    let mut left_to_right =
        scratch_vec::<(usize, usize)>(budget, left_used.len(), "dictionary left-to-right remap")?;
    if left_used.len() <= HASHED_NAME_INDEX_THRESHOLD {
        for left_index in &left_used {
            let Some(right_index) = right_used
                .iter()
                .copied()
                .find(|right_index| compare_cross(*left_index, *right_index) == Ordering::Equal)
            else {
                left_to_right.clear();
                break;
            };
            left_to_right.push((*left_index, right_index));
        }
    }
    if left_to_right.len() == left_used.len() {
        left_to_right.sort_unstable_by_key(|(old, _)| *old);
        let left = remap_dictionary_to_values(
            field,
            left_source,
            left_exposure,
            &left_to_right,
            Arc::clone(right_source.values()),
            budget,
        )?;
        return Ok((left, Arc::clone(right)));
    }

    let mut candidates = scratch_vec::<DictionaryCandidate>(
        budget,
        left_used.len().saturating_add(right_used.len()),
        "dictionary reachable vocabulary",
    )?;
    candidates.extend(left_used.iter().copied().map(DictionaryCandidate::Left));
    candidates.extend(right_used.iter().copied().map(DictionaryCandidate::Right));
    candidates.sort_unstable_by(|left, right| {
        compare_dictionary_candidates(*left, *right, &compare_left, &compare_right, &compare_cross)
    });

    let mut representatives = scratch_vec::<DictionaryCandidate>(
        budget,
        candidates.len(),
        "dictionary semantic representatives",
    )?;
    let mut left_groups =
        scratch_vec::<(usize, usize)>(budget, left_used.len(), "dictionary left compact remap")?;
    let mut right_groups =
        scratch_vec::<(usize, usize)>(budget, right_used.len(), "dictionary right compact remap")?;
    for candidate in candidates {
        let group = if representatives.last().is_some_and(|prior| {
            compare_dictionary_candidates(
                *prior,
                candidate,
                &compare_left,
                &compare_right,
                &compare_cross,
            ) == Ordering::Equal
        }) {
            representatives.len() - 1
        } else {
            representatives.push(candidate);
            representatives.len() - 1
        };
        match candidate {
            DictionaryCandidate::Left(old) => left_groups.push((old, group)),
            DictionaryCandidate::Right(old) => right_groups.push((old, group)),
        }
    }

    let last = representatives.len().checked_sub(1);
    if last.is_some_and(|last| K::Native::try_from(last).is_err()) {
        return Err(Error::IncompatibleSchema(format!(
            "dictionary reachable values exceed the {} key capacity",
            dictionary.key()
        )));
    }

    let mut left_selected = scratch_vec::<usize>(
        budget,
        representatives.len(),
        "dictionary left representative indices",
    )?;
    let mut right_selected = scratch_vec::<usize>(
        budget,
        representatives.len(),
        "dictionary right representative indices",
    )?;
    let mut group_output = scratch_vec::<usize>(
        budget,
        representatives.len(),
        "dictionary representative output mapping",
    )?;
    group_output.resize(representatives.len(), 0);
    for (group, representative) in representatives.iter().enumerate() {
        if let DictionaryCandidate::Left(index) = representative {
            group_output[group] = left_selected.len();
            left_selected.push(*index);
        }
    }
    let left_count = left_selected.len();
    for (group, representative) in representatives.iter().enumerate() {
        if let DictionaryCandidate::Right(index) = representative {
            group_output[group] = left_count + right_selected.len();
            right_selected.push(*index);
        }
    }
    for (_, group) in &mut left_groups {
        *group = group_output[*group];
    }
    for (_, group) in &mut right_groups {
        *group = group_output[*group];
    }
    left_groups.sort_unstable_by_key(|(old, _)| *old);
    right_groups.sort_unstable_by_key(|(old, _)| *old);

    let left_values =
        take_dictionary_candidates(left_source.values(), value_type, &left_selected, budget)?;
    let right_values =
        take_dictionary_candidates(right_source.values(), value_type, &right_selected, budget)?;
    let value_field = Field::new("dictionary", value_type.clone(), true);
    let (left_values, right_values) = align_nested_dictionaries(
        &value_field,
        &left_values,
        &right_values,
        None,
        None,
        budget,
    )?;
    let values = match (left_values.is_empty(), right_values.is_empty()) {
        (false, false) => {
            reserve_concat_copy(left_values.as_ref(), value_type, budget)?;
            reserve_concat_copy(right_values.as_ref(), value_type, budget)?;
            concat(&[left_values.as_ref(), right_values.as_ref()])?
        }
        (false, true) => left_values,
        (true, false) => right_values,
        (true, true) => arrow_array::new_empty_array(left_source.values().data_type()),
    };
    let left = remap_dictionary_to_values(
        field,
        left_source,
        left_exposure,
        &left_groups,
        Arc::clone(&values),
        budget,
    )?;
    let right = remap_dictionary_to_values(
        field,
        right_source,
        right_exposure,
        &right_groups,
        values,
        budget,
    )?;
    Ok((left, right))
}

fn ensure_list_child_physical(
    field: &Field,
    array: ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    if field.is_nullable()
        || exposed_logical_null_count(array.as_ref(), field.data_type(), None)? == 0
    {
        Ok(array)
    } else {
        // Arrow validates a List child Field independently of the parent List
        // validity bitmap. Hidden child slots therefore need a present
        // canonical value even when their parent row is null.
        fill_nulls(field, array, false, None, budget)
    }
}

fn default_array(
    field: &Field,
    len: usize,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let arrow_type = field.to_arrow_ref()?.data_type().clone();
    if len == 0 {
        return Ok(arrow_array::new_empty_array(&arrow_type));
    }
    if let Some(exposure) = exposure {
        if exposure.len() != len {
            return Err(Error::IncompatibleSchema(
                "missing-field exposure mask has the wrong length".to_owned(),
            ));
        }
    }
    let exposed = exposure.map_or(len, BooleanBuffer::count_set_bits);
    let hidden = len - exposed;
    if field.is_nullable() {
        budget.add_null_array(field.data_type(), len)?;
        return Ok(new_null_array(&arrow_type, len));
    }
    if exposed != 0 && hidden != 0 {
        if let DataType::Dictionary(dictionary) = field.data_type() {
            let exposure = exposure.ok_or_else(|| {
                Error::IncompatibleSchema(
                    "mixed missing dictionary exposure requires a mask".to_owned(),
                )
            })?;
            return default_dictionary_array(field, dictionary, exposure, budget);
        }
    }

    let phase = budget.mark();
    reserve_missing_output(field, exposed, hidden, budget)?;
    let single_scalar_output = len == 1 && (exposed == 1 || hidden == 1);
    if exposed != 0 && !single_scalar_output {
        reserve_field_default_scalar(field, budget)?;
    }
    if hidden != 0 && !single_scalar_output {
        budget.add_null_scalar_scratch(field.data_type())?;
    }

    let output = match (exposed, hidden) {
        (0, _) => {
            if len != 1 {
                budget.add_array(&DataType::UInt32, len)?;
            }
            let placeholder = crate::arrow::value::physical_placeholder_for_field(field)?;
            let placeholder = crate::arrow::value::array_from_values(field, &[&placeholder])?;
            repeat_scalar(&placeholder, len)?
        }
        (_, 0) => {
            if len != 1 {
                budget.add_array(&DataType::UInt32, len)?;
            }
            let default = field.default_arrow_scalar()?.into_array();
            repeat_scalar(&default, len)?
        }
        _ => {
            let exposure = exposure.ok_or_else(|| {
                Error::IncompatibleSchema("mixed missing-field exposure requires a mask".to_owned())
            })?;
            let default = field.default_arrow_scalar()?.into_array();
            let placeholder = crate::arrow::value::physical_placeholder_for_field(field)?;
            let placeholder = crate::arrow::value::array_from_values(field, &[&placeholder])?;
            let mask = BooleanArray::new(exposure.clone(), None);
            let (default, placeholder) = if contains_dictionary(field.data_type()) {
                align_nested_dictionaries(field, &default, &placeholder, None, None, budget)?
            } else {
                (default, placeholder)
            };
            let default = Scalar::new(default);
            let placeholder = Scalar::new(placeholder);
            zip(&mask, &default, &placeholder)?
        }
    };

    budget.restore(phase);
    reserve_missing_output(field, exposed, hidden, budget)?;
    Ok(output)
}

fn reserve_field_default(
    field: &Field,
    rows: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    if field.is_nullable() {
        budget.add_null_array(field.data_type(), rows)
    } else {
        budget.add_repeated_default(field.data_type(), rows)
    }
}

fn default_dictionary_array(
    field: &Field,
    dictionary: &crate::DictionaryType,
    exposure: &BooleanBuffer,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    macro_rules! build {
        ($key:ty) => {{ default_dictionary_array_typed::<$key>(field, dictionary, exposure, budget) }};
    }
    match dictionary.key() {
        DataType::Int8 => build!(Int8Type),
        DataType::Int16 => build!(Int16Type),
        DataType::Int32 => build!(Int32Type),
        DataType::Int64 => build!(Int64Type),
        DataType::UInt8 => build!(UInt8Type),
        DataType::UInt16 => build!(UInt16Type),
        DataType::UInt32 => build!(UInt32Type),
        DataType::UInt64 => build!(UInt64Type),
        key => Err(Error::Unsupported {
            kind: key.name(),
            reason: format!(
                "expected an integer dictionary key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        }),
    }
}

fn default_dictionary_array_typed<K>(
    field: &Field,
    dictionary: &crate::DictionaryType,
    exposure: &BooleanBuffer,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef>
where
    K: ArrowDictionaryKeyType,
    K::Native: TryFrom<usize>,
{
    let phase = budget.mark();
    budget.add_array_layout(field.data_type(), exposure.len())?;
    budget.add_default_scalar_scratch(dictionary.value())?;
    let zero = K::Native::try_from(0).map_err(|_| {
        Error::IncompatibleSchema("dictionary key cannot represent zero".to_owned())
    })?;
    let values = dictionary.value().default_arrow_scalar()?.into_array();
    let mut keys = arrow_array::builder::PrimitiveBuilder::<K>::with_capacity(exposure.len());
    for index in 0..exposure.len() {
        if exposure.value(index) {
            keys.append_value(zero);
        } else {
            keys.append_null();
        }
    }
    let output = Arc::new(DictionaryArray::<K>::try_new(keys.finish(), values)?) as ArrayRef;
    budget.restore(phase);
    budget.add_array_layout(field.data_type(), exposure.len())?;
    budget.add_repeated_default(dictionary.value(), 1)?;
    Ok(output)
}

fn reserve_field_default_scalar(field: &Field, budget: &mut MaterializationBudget) -> Result<()> {
    if field.is_nullable() {
        budget.add_null_scalar_scratch(field.data_type())
    } else {
        budget.add_default_scalar_scratch(field.data_type())
    }
}

fn reserve_missing_output(
    field: &Field,
    exposed: usize,
    hidden: usize,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    reserve_field_default(field, exposed, budget)?;
    budget.add_null_array(field.data_type(), hidden)
}

fn repeat_scalar(array: &ArrayRef, len: usize) -> Result<ArrayRef> {
    if len == 1 && array.len() == 1 {
        return Ok(Arc::clone(array));
    }
    let indices = UInt32Array::from_value(0, len);
    take(array.as_ref(), &indices, None).map_err(Into::into)
}

fn list_child(expected: &ArrowDataType) -> Result<ArrowFieldRef> {
    match expected {
        ArrowDataType::List(field)
        | ArrowDataType::ListView(field)
        | ArrowDataType::LargeList(field)
        | ArrowDataType::LargeListView(field)
        | ArrowDataType::FixedSizeList(field, _) => Ok(Arc::clone(field)),
        _ => Err(internal_target_error("list")),
    }
}

fn ensure_unambiguous_names(fields: &arrow_schema::Fields) -> Result<()> {
    for (index, field) in fields.iter().enumerate() {
        if fields[..index]
            .iter()
            .any(|prior| prior.name().eq_ignore_ascii_case(field.name()))
        {
            return Err(Error::IncompatibleSchema(format!(
                "ASCII-case-insensitive field name {:?} is ambiguous",
                field.name()
            )));
        }
    }
    Ok(())
}

const HASHED_NAME_INDEX_THRESHOLD: usize = 16;

fn folded_field_mapping(
    source: &arrow_schema::Fields,
    target: &[Field],
) -> Result<Vec<Option<usize>>> {
    if source.len().max(target.len()) <= HASHED_NAME_INDEX_THRESHOLD {
        ensure_unambiguous_names(source)?;
        ensure_unambiguous_target_names(target)?;
        return target
            .iter()
            .map(|field| folded_field_index(source, field.name()))
            .collect();
    }

    let mut source_index = HashMap::with_capacity(source.len());
    for (index, field) in source.iter().enumerate() {
        let folded = field.name().to_ascii_lowercase();
        if source_index.insert(folded, index).is_some() {
            return Err(Error::IncompatibleSchema(format!(
                "ASCII-case-insensitive field name {:?} is ambiguous",
                field.name()
            )));
        }
    }
    let mut target_names = HashSet::with_capacity(target.len());
    let mut mapping = Vec::with_capacity(target.len());
    for field in target {
        let folded = field.name().to_ascii_lowercase();
        if !target_names.insert(folded.clone()) {
            return Err(Error::IncompatibleSchema(format!(
                "ASCII-case-insensitive target field name {:?} is ambiguous",
                field.name()
            )));
        }
        mapping.push(source_index.get(&folded).copied());
    }
    Ok(mapping)
}

fn ensure_unambiguous_target_names(fields: &[Field]) -> Result<()> {
    for (index, field) in fields.iter().enumerate() {
        if fields[..index]
            .iter()
            .any(|prior| prior.name().eq_ignore_ascii_case(field.name()))
        {
            return Err(Error::IncompatibleSchema(format!(
                "ASCII-case-insensitive target field name {:?} is ambiguous",
                field.name()
            )));
        }
    }
    Ok(())
}

fn folded_field_index(fields: &arrow_schema::Fields, name: &str) -> Result<Option<usize>> {
    let mut found = None;
    for (index, field) in fields.iter().enumerate() {
        if field.name().eq_ignore_ascii_case(name) {
            if found.is_some() {
                return Err(Error::IncompatibleSchema(format!(
                    "ASCII-case-insensitive field name {name:?} matches multiple source columns"
                )));
            }
            found = Some(index);
        }
    }
    Ok(found)
}

fn downcast<T: Array + 'static>(array: &dyn Array) -> Result<&T> {
    array.as_any().downcast_ref::<T>().ok_or_else(|| {
        Error::IncompatibleSchema(format!(
            "Arrow array implementation does not match datatype {:?}",
            array.data_type()
        ))
    })
}

fn internal_target_error(kind: &'static str) -> Error {
    Error::Unsupported {
        kind,
        reason: "validated target projected an unexpected Arrow datatype".to_owned(),
    }
}
