//! Nested Arrow planning, exposure, and logical-null traversal.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arrow_array::types::{
    ArrowDictionaryKeyType, Int8Type, Int16Type, Int32Type, Int64Type, RunEndIndexType, UInt8Type,
    UInt16Type, UInt32Type, UInt64Type,
};
use arrow_array::{
    Array, ArrayRef, BooleanArray, Decimal256Array, DictionaryArray, FixedSizeListArray,
    Float16Array, Float32Array, Float64Array, Int16RunArray, Int32RunArray, Int64RunArray,
    LargeListArray, LargeListViewArray, ListArray, ListViewArray, MapArray, PrimitiveArray,
    RunArray, Scalar as ArrowScalar, StructArray, UInt32Array, UnionArray, make_array,
    new_null_array,
};
use arrow_buffer::{ArrowNativeType, BooleanBuffer, BooleanBufferBuilder};
use arrow_ord::ord::{DynComparator, make_comparator};
use arrow_schema::{DataType as ArrowDataType, FieldRef as ArrowFieldRef, SortOptions};
use arrow_select::{concat::concat, take::take, zip::zip};

use crate::arrow::{Error, Result};
use crate::types::budget::{
    MaterializationBudget, SourceSelection, reserve_concat_copy, reserve_field_default_scalar,
    reserve_missing_output, reserve_new_dictionary_vocabularies, reserve_source_selection,
    reserve_to_data_scratch, reserve_vec_bytes, scratch_vec, selected_child_ranges,
};
use crate::types::cast::arrow_cast_exposed;
use crate::types::cast::{
    ArrayCastPlan, ListPlanKind, StructColumnPlan, downcast, internal_target_error,
};
use crate::types::decimal::casts::DecimalText;
use crate::{DataType, Field, Scalar, UnionMode};

mod dictionary;
mod plans;
mod repair;

pub(crate) use dictionary::*;
pub(crate) use repair::*;

const HASHED_NAME_INDEX_THRESHOLD: usize = 16;

pub(crate) fn validate_map_invariants(
    map: &crate::MapType,
    array: &dyn Array,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<()> {
    let array = downcast::<MapArray>(array)?;
    let phase = budget.mark();
    let keys = array.entries().column(0);
    let Some([key_field, _]) = map.entries().dtype().as_fields() else {
        return Err(Error::IncompatibleSchema(
            "map entries must contain key and value fields".to_owned(),
        ));
    };
    let compare = make_yggdryl_key_comparator(key_field.dtype(), keys, budget)?;
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
    // allocation across the complete array instead of creating a map and
    // schema for every logical row.
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
            if logical_null_at(keys.as_ref(), key_field.dtype(), index)? {
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

pub(crate) fn requires_yggdryl_key_comparator(dtype: &DataType) -> bool {
    match dtype {
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
        | DataType::LargeListView(child) => requires_yggdryl_key_comparator(child.dtype()),
        DataType::Struct(fields) => fields
            .iter()
            .any(|field| requires_yggdryl_key_comparator(field.dtype())),
        DataType::Map(map) => requires_yggdryl_key_comparator(map.entries().dtype()),
        _ => false,
    }
}

pub(crate) fn has_derived_logical_nulls(dtype: &DataType) -> bool {
    matches!(
        dtype,
        DataType::Null | DataType::Dictionary(_) | DataType::Union(..) | DataType::RunEndEncoded(_)
    )
}

pub(crate) fn wrap_yggdryl_nulls(
    left: &ArrayRef,
    right: &ArrayRef,
    dtype: &DataType,
    compare: DynComparator,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    if has_derived_logical_nulls(dtype) {
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

pub(crate) fn dictionary_key_comparator<K: ArrowDictionaryKeyType>(
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

pub(crate) fn run_key_comparator<R: RunEndIndexType>(
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
        encoded.values().dtype(),
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

#[allow(clippy::too_many_lines)] // Recurses only where native Scalar ordering differs from Arrow.
pub(crate) fn make_yggdryl_key_comparator(
    dtype: &DataType,
    array: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    make_yggdryl_comparator(dtype, array, array, budget)
}

#[allow(clippy::too_many_lines)] // Recurses only where native Scalar ordering differs from Arrow.
pub(crate) fn make_yggdryl_comparator(
    dtype: &DataType,
    left: &ArrayRef,
    right: &ArrayRef,
    budget: &mut MaterializationBudget,
) -> Result<DynComparator> {
    if !requires_yggdryl_key_comparator(dtype) {
        if has_derived_logical_nulls(dtype) {
            budget.add_bitmap(left.len())?;
            if !Arc::ptr_eq(left, right) {
                budget.add_bitmap(right.len())?;
            }
        }
        return make_comparator(left.as_ref(), right.as_ref(), SortOptions::default())
            .map_err(Into::into);
    }

    let compare: DynComparator = match dtype {
        DataType::Float16 => {
            let left_values = downcast::<Float16Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Float16Array>(right.as_ref())?.values().clone();
            Box::new(move |left, right| {
                crate::Float16::from_f16(left_values[left])
                    .cmp(&crate::Float16::from_f16(right_values[right]))
            })
        }
        DataType::Float32 => {
            let left_values = downcast::<Float32Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Float32Array>(right.as_ref())?.values().clone();
            Box::new(move |left, right| {
                crate::Float32::from_f32(left_values[left])
                    .cmp(&crate::Float32::from_f32(right_values[right]))
            })
        }
        DataType::Float64 => {
            let left_values = downcast::<Float64Array>(left.as_ref())?.values().clone();
            let right_values = downcast::<Float64Array>(right.as_ref())?.values().clone();
            Box::new(move |left, right| {
                crate::Float64::from_f64(left_values[left])
                    .cmp(&crate::Float64::from_f64(right_values[right]))
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
                make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
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
                make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
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
                make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
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
                make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
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
                make_yggdryl_comparator(child.dtype(), &left_values, &right_values, budget)?;
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
                    make_yggdryl_comparator(field.dtype(), left, right, budget)
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
                map.entries().dtype(),
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
                        field.dtype(),
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
            return match encoded.run_ends().dtype() {
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
    // sequence. Every other sensitive wrapper follows ordinary Scalar nulls.
    if matches!(dtype, DataType::Union(..)) {
        Ok(compare)
    } else {
        wrap_yggdryl_nulls(left, right, dtype, compare, budget)
    }
}

pub(crate) fn cast_dictionary_planned(
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

pub(crate) fn cast_union_planned(
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

pub(crate) fn run_value_exposure<R: RunEndIndexType>(
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

pub(crate) fn cast_run_planned(
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

pub(crate) fn contains_struct(dtype: &DataType) -> bool {
    match dtype {
        DataType::Struct(_) | DataType::Map(_) => true,
        DataType::List(field)
        | DataType::ListView(field)
        | DataType::FixedSizeList(field, _)
        | DataType::LargeList(field)
        | DataType::LargeListView(field) => contains_struct(field.dtype()),
        DataType::Union(fields, _) => fields
            .iter()
            .any(|(_, field)| contains_struct(field.dtype())),
        DataType::Dictionary(dictionary) => contains_struct(dictionary.value()),
        DataType::RunEndEncoded(encoded) => contains_struct(encoded.values().dtype()),
        _ => false,
    }
}

pub(crate) fn is_reconcilable_nested(dtype: &DataType) -> bool {
    matches!(
        dtype,
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
pub(crate) fn is_logically_null(array: &dyn Array, index: usize) -> bool {
    array
        .logical_nulls()
        .is_some_and(|nulls| nulls.is_null(index))
}

pub(crate) fn is_exposed(exposure: Option<&BooleanBuffer>, index: usize) -> bool {
    exposure.is_none_or(|exposure| exposure.value(index))
}

pub(crate) fn dictionary_logical_null_at<K: ArrowDictionaryKeyType>(
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

pub(crate) fn run_logical_null_at<R: RunEndIndexType>(
    array: &dyn Array,
    encoded: &crate::RunEndEncodedType,
    index: usize,
) -> Result<bool> {
    let array = downcast::<RunArray<R>>(array)?;
    logical_null_at(
        array.values().as_ref(),
        encoded.values().dtype(),
        array.get_physical_index(index),
    )
}

pub(crate) fn logical_null_at(array: &dyn Array, dtype: &DataType, index: usize) -> Result<bool> {
    if index >= array.len() {
        return Err(Error::IncompatibleSchema(
            "logical-null index exceeds its Arrow array".to_owned(),
        ));
    }
    match dtype {
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
                field.dtype(),
                array.value_offset(index),
            )
        }
        DataType::RunEndEncoded(encoded) => match encoded.run_ends().dtype() {
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

pub(crate) fn run_exposed_logical_null_count<R: RunEndIndexType>(
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
            encoded.values().dtype(),
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

pub(crate) fn exposed_logical_null_count(
    array: &dyn Array,
    dtype: &DataType,
    exposure: Option<&BooleanBuffer>,
) -> Result<usize> {
    if exposure.is_some_and(|exposure| exposure.len() != array.len()) {
        return Err(Error::IncompatibleSchema(
            "logical-null exposure has the wrong length".to_owned(),
        ));
    }
    if let DataType::RunEndEncoded(encoded) = dtype {
        return match encoded.run_ends().dtype() {
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
        if is_exposed(exposure, index) && logical_null_at(array, dtype, index)? {
            null_count += 1;
        }
    }
    Ok(null_count)
}

pub(crate) fn logical_validity_buffer(
    array: &dyn Array,
    dtype: &DataType,
    budget: &mut MaterializationBudget,
) -> Result<arrow_buffer::NullBuffer> {
    budget.add_bitmap(array.len())?;
    let mut builder = BooleanBufferBuilder::new(array.len());
    for index in 0..array.len() {
        builder.append(!logical_null_at(array, dtype, index)?);
    }
    Ok(arrow_buffer::NullBuffer::new(builder.build()))
}

pub(crate) fn visible_array_exposure(
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

pub(crate) fn offset_pair(start: i64, end: i64) -> Result<(usize, usize)> {
    let start = usize::try_from(start).map_err(|_| {
        Error::IncompatibleSchema("nested Arrow offset is negative or exceeds usize".to_owned())
    })?;
    let end = usize::try_from(end).map_err(|_| {
        Error::IncompatibleSchema("nested Arrow offset is negative or exceeds usize".to_owned())
    })?;
    Ok((start, end))
}

pub(crate) fn offset_size(start: i64, size: i64) -> Result<(usize, usize)> {
    let (start, size) = offset_pair(start, size)?;
    let end = start.checked_add(size).ok_or_else(|| {
        Error::IncompatibleSchema("nested Arrow offset plus size exceeds usize".to_owned())
    })?;
    Ok((start, end))
}

pub(crate) fn range_exposure<Valid, Range>(
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

pub(crate) fn selected_index_exposure<Index>(
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

pub(crate) fn union_mode_matches(core: UnionMode, arrow: arrow_schema::UnionMode) -> bool {
    matches!(
        (core, arrow),
        (UnionMode::Sparse, arrow_schema::UnionMode::Sparse)
            | (UnionMode::Dense, arrow_schema::UnionMode::Dense)
    )
}
