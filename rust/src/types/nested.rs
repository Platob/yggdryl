//! Nested datatype layouts and shared child collections.

use std::cmp::Ordering;

use std::collections::{BTreeMap, HashSet};

use std::fmt;

use std::ops::Index;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use smol_str::{SmolStr, format_smolstr};

use crate::types::parser::{ListKind, Parser, normalized};

use crate::types::typed::define_field_types;

use crate::types::{Scalar, invalid};

use crate::types::dictionary::DictionaryType;
use crate::types::structure::StructureType;
use crate::types::runend::RunEndEncodedType;
use crate::types::union::UnionFields;
use crate::{DataType, DataTypeId, Error, Field, Result, TypedField, UnionMode, Value};
use crate::types::mapping::MappingType;

#[cfg(feature = "arrow")]
/// Nested Arrow planning, exposure, and logical-null traversal.
pub(crate) mod casts {
    
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

    mod dictionary {
        
        use super::*;

        pub(crate) fn contains_dictionary(dtype: &DataType) -> bool {
            match dtype {
                DataType::Dictionary(_) => true,
                DataType::List(field)
                | DataType::ListView(field)
                | DataType::FixedSizeList(field, _)
                | DataType::LargeList(field)
                | DataType::LargeListView(field) => contains_dictionary(field.dtype()),
                DataType::Structure(fields) => fields
                    .iter()
                    .any(|field| contains_dictionary(field.dtype())),
                DataType::Union(fields, _) => fields
                    .iter()
                    .any(|(_, field)| contains_dictionary(field.dtype())),
                DataType::Mapping(map) => contains_dictionary(map.entries().dtype()),
                DataType::RunEndEncoded(encoded) => contains_dictionary(encoded.values().dtype()),
                _ => false,
            }
        }

        pub(crate) fn dictionary_values_ref<'a>(
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

        #[allow(clippy::too_many_lines)]
        pub(crate) fn align_nested_dictionaries(
            field: &Field,
            left: &ArrayRef,
            right: &ArrayRef,
            left_exposure: Option<&BooleanBuffer>,
            right_exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<(ArrayRef, ArrayRef)> {
            if !contains_dictionary(field.dtype()) {
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

            match field.dtype() {
                DataType::Dictionary(dictionary) => align_dictionary_arrays(
                    field,
                    dictionary,
                    left,
                    right,
                    left_exposure,
                    right_exposure,
                    budget,
                ),
                DataType::Structure(fields) => {
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
                DataType::Mapping(map) => {
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
                    match encoded.run_ends().dtype() {
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

        pub(crate) fn align_dictionary_arrays(
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

        pub(crate) fn dictionary_live_indices<K: ArrowDictionaryKeyType>(
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

        pub(crate) fn remap_dictionary_to_values<K>(
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
            budget.add_array_layout(field.dtype(), source.len())?;
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

        pub(crate) fn take_dictionary_candidates(
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
        pub(crate) fn align_dictionary_arrays_typed<K>(
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
    }
    mod plans {
        
        use super::*;

        impl ArrayCastPlan {
            pub(crate) fn cast_struct_array(
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
            pub(crate) fn cast_list_array(
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
            pub(crate) fn cast_map_array(
                &self,
                array: ArrayRef,
                source_map: &crate::MappingType,
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
                let DataType::Mapping(target_map) = self.field.dtype() else {
                    return Err(internal_target_error("map"));
                };
                if !unchanged || source_map != target_map {
                    validate_map_invariants(target_map, output.as_ref(), exposure, budget)?;
                }
                Ok(output)
            }
        }
    }
    mod repair {
        use super::*;

        pub(crate) fn fill_nulls(
            field: &Field,
            array: ArrayRef,
            dtype_semantics: bool,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            if dtype_semantics && field.dtype().is_default_value(&Scalar::Null)? {
                return Ok(array);
            }
            if let DataType::Dictionary(dictionary) = field.dtype() {
                return fill_dictionary_nulls(field, dictionary, array, exposure, budget);
            }
            let phase = budget.mark();
            let logical = logical_validity_buffer(array.as_ref(), field.dtype(), budget)?;
            let default_count = (0..array.len())
                .filter(|index| is_exposed(exposure, *index) && logical.is_null(*index))
                .count();
            if default_count == 0 {
                budget.restore(phase);
                return Ok(array);
            }
            let exposed_count = exposure.map_or(array.len(), BooleanBuffer::count_set_bits);
            if default_count == exposed_count && contains_dictionary(field.dtype()) {
                budget.restore(phase);
                return default_array(field, array.len(), None, budget);
            }

            // Reserve the one-row scalar and both parts of the final zip before
            // constructing any default. The source-range walk charges only values the
            // truthy side copies; exposed nulls are charged through canonical defaults.
            budget.add_default_scalar_scratch(field.dtype())?;
            if has_derived_logical_nulls(field.dtype()) {
                budget.add_bitmap(1)?;
            }
            if contains_dictionary(field.dtype()) {
                budget.add_repeated_default_without_dictionary_values(field.dtype(), default_count)?;
            } else {
                budget.add_repeated_default(field.dtype(), default_count)?;
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
            let default = if dtype_semantics {
                field.dtype().default_arrow_array()?
            } else {
                field.default_arrow_array()?
            };
            if is_logically_null(default.as_ref(), 0) {
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
            let (array, default) = if contains_dictionary(field.dtype()) {
                budget.add_bitmap(array.len())?;
                let live = BooleanBuffer::collect_bool(array.len(), |index| {
                    is_exposed(exposure, index) && logical.is_valid(index)
                });
                align_nested_dictionaries(field, &array, &default, Some(&live), None, budget)?
            } else {
                (array, default)
            };
            let default = ArrowScalar::new(default);
            let truthy: &dyn Array = array.as_ref();
            let output = zip(&mask, &truthy, &default)?;

            // Only the zip output survives this phase. Release the scalar, mask, and
            // range-planning reservations, then retain the exact two output parts in
            // the operation-wide aggregate for following columns.
            budget.restore(phase);
            if contains_dictionary(field.dtype()) {
                budget.add_repeated_default_without_dictionary_values(field.dtype(), default_count)?;
            } else {
                budget.add_repeated_default(field.dtype(), default_count)?;
            }
            reserve_source_selection(
                source_for_retention.as_ref(),
                &source_type,
                SourceSelection::Ranges(&truthy_ranges),
                budget,
            )?;
            if contains_dictionary(field.dtype()) {
                reserve_new_dictionary_vocabularies(&output, &source_for_retention, field.dtype(), budget)?;
            }
            Ok(output)
        }

        pub(crate) fn fill_dictionary_nulls(
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
        pub(crate) fn fill_dictionary_nulls_typed<K>(
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
            let logical = logical_validity_buffer(source, field.dtype(), budget)?;
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
            let default = value_type.default_arrow_array()?;
            if is_logically_null(default.as_ref(), 0) {
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
            let compare_default = make_yggdryl_comparator(value_type, values, &default, budget)?;
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

            budget.add_array_layout(field.dtype(), source.len())?;
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
                    None => Arc::clone(&default),
                    Some(compact) => {
                        let default_array = Arc::clone(&default);
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
            budget.add_array_layout(field.dtype(), source.len())?;
            reserve_new_dictionary_vocabularies(&output, &array, field.dtype(), budget)?;
            Ok(output)
        }

        #[allow(clippy::too_many_lines)] // Mirrors Arrow concat's nested layout dispatch.
        pub(crate) fn replace_array_children(
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

        pub(crate) fn array_children_unchanged(array: &ArrayRef, children: &[ArrayRef]) -> Result<bool> {
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

        pub(crate) fn null_buffers_ptr_eq(
            left: Option<&arrow_buffer::NullBuffer>,
            right: Option<&arrow_buffer::NullBuffer>,
        ) -> bool {
            match (left, right) {
                (None, None) => true,
                (Some(left), Some(right)) => left.inner().ptr_eq(right.inner()),
                _ => false,
            }
        }

        pub(crate) fn ensure_list_child_physical(
            field: &Field,
            array: ArrayRef,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            if field.is_nullable() || exposed_logical_null_count(array.as_ref(), field.dtype(), None)? == 0
            {
                Ok(array)
            } else {
                // Arrow validates a List child Field independently of the parent List
                // validity bitmap. Hidden child slots therefore need a present
                // canonical value even when their parent row is null.
                fill_nulls(field, array, false, None, budget)
            }
        }

        pub(crate) fn default_array(
            field: &Field,
            len: usize,
            exposure: Option<&BooleanBuffer>,
            budget: &mut MaterializationBudget,
        ) -> Result<ArrayRef> {
            let arrow_type = field.clone().into_arrow_ref()?.data_type().clone();
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
                budget.add_null_array(field.dtype(), len)?;
                return Ok(new_null_array(&arrow_type, len));
            }
            if exposed != 0 && hidden != 0 {
                if let DataType::Dictionary(dictionary) = field.dtype() {
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
                budget.add_null_scalar_scratch(field.dtype())?;
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
                    let default = field.default_arrow_array()?;
                    repeat_scalar(&default, len)?
                }
                _ => {
                    let exposure = exposure.ok_or_else(|| {
                        Error::IncompatibleSchema("mixed missing-field exposure requires a mask".to_owned())
                    })?;
                    let default = field.default_arrow_array()?;
                    let placeholder = crate::arrow::value::physical_placeholder_for_field(field)?;
                    let placeholder = crate::arrow::value::array_from_values(field, &[&placeholder])?;
                    let mask = BooleanArray::new(exposure.clone(), None);
                    let (default, placeholder) = if contains_dictionary(field.dtype()) {
                        align_nested_dictionaries(field, &default, &placeholder, None, None, budget)?
                    } else {
                        (default, placeholder)
                    };
                    let default = ArrowScalar::new(default);
                    let placeholder = ArrowScalar::new(placeholder);
                    zip(&mask, &default, &placeholder)?
                }
            };

            budget.restore(phase);
            reserve_missing_output(field, exposed, hidden, budget)?;
            Ok(output)
        }

        pub(crate) fn default_dictionary_array(
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

        pub(crate) fn default_dictionary_array_typed<K>(
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
            budget.add_array_layout(field.dtype(), exposure.len())?;
            budget.add_default_scalar_scratch(dictionary.value())?;
            let zero = K::Native::try_from(0).map_err(|_| {
                Error::IncompatibleSchema("dictionary key cannot represent zero".to_owned())
            })?;
            let values = dictionary.value().default_arrow_array()?;
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
            budget.add_array_layout(field.dtype(), exposure.len())?;
            budget.add_repeated_default(dictionary.value(), 1)?;
            Ok(output)
        }

        pub(crate) fn repeat_scalar(array: &ArrayRef, len: usize) -> Result<ArrayRef> {
            if len == 1 && array.len() == 1 {
                return Ok(Arc::clone(array));
            }
            let indices = UInt32Array::from_value(0, len);
            take(array.as_ref(), &indices, None).map_err(Into::into)
        }

        pub(crate) fn list_child(expected: &ArrowDataType) -> Result<ArrowFieldRef> {
            match expected {
                ArrowDataType::List(field)
                | ArrowDataType::ListView(field)
                | ArrowDataType::LargeList(field)
                | ArrowDataType::LargeListView(field)
                | ArrowDataType::FixedSizeList(field, _) => Ok(Arc::clone(field)),
                _ => Err(internal_target_error("list")),
            }
        }

        pub(crate) fn ensure_unambiguous_names(fields: &arrow_schema::Fields) -> Result<()> {
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

        pub(crate) fn folded_field_mapping(
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

        pub(crate) fn ensure_unambiguous_target_names(fields: &[Field]) -> Result<()> {
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

        pub(crate) fn folded_field_index(
            fields: &arrow_schema::Fields,
            name: &str,
        ) -> Result<Option<usize>> {
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
    }

    pub(crate) use dictionary::*;
    pub(crate) use repair::*;

    const HASHED_NAME_INDEX_THRESHOLD: usize = 16;

    pub(crate) fn validate_map_invariants(
        map: &crate::MappingType,
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
            DataType::Structure(fields) => fields
                .iter()
                .any(|field| requires_yggdryl_key_comparator(field.dtype())),
            DataType::Mapping(map) => requires_yggdryl_key_comparator(map.entries().dtype()),
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
            DataType::Structure(fields) => {
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
            DataType::Mapping(map) => {
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
                return arrow_cast_exposed(
                    &array,
                    expected,
                    plan.safe(),
                    exposure,
                    &plan.field,
                    budget,
                );
            }
        };
        if rebuilt.data_type() == expected {
            Ok(rebuilt)
        } else {
            arrow_cast_exposed(
                &rebuilt,
                expected,
                plan.safe(),
                exposure,
                &plan.field,
                budget,
            )
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
            DataType::Structure(_) | DataType::Mapping(_) => true,
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
                | DataType::Structure(_)
                | DataType::Union(_, _)
                | DataType::Dictionary(_)
                | DataType::Mapping(_)
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
}

// ------------------------------------------------------------------------
// Shared child collections and validated nested datatype construction.
// ------------------------------------------------------------------------

/// One nested or wrapper datatype.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum NestedType {
    /// Variable list with 32-bit offsets.
    List(Arc<Field>),
    /// Variable list view with 32-bit offsets.
    ListView(Arc<Field>),
    /// Fixed-length list.
    FixedSizeList(Arc<Field>, i32),
    /// Variable list with 64-bit offsets.
    LargeList(Arc<Field>),
    /// Variable list view with 64-bit offsets.
    LargeListView(Arc<Field>),
    /// Named children, as the structure family holds them.
    Structure(StructureType),
    /// Tagged union fields and layout.
    Union(UnionFields, UnionMode),
    /// Dictionary key and value types.
    Dictionary(Arc<DictionaryType>),
    /// Keys to values, as the mapping family holds them.
    Mapping(MappingType),
    /// Run-end encoded child fields.
    RunEndEncoded(Arc<RunEndEncodedType>),
    /// Self-describing semi-structured values.
    Variant,
}

impl NestedType {
    /// Return the exact datatype identifier.
    pub const fn id(&self) -> DataTypeId {
        match self {
            Self::List(_) => DataTypeId::List,
            Self::ListView(_) => DataTypeId::ListView,
            Self::FixedSizeList(..) => DataTypeId::FixedSizeList,
            Self::LargeList(_) => DataTypeId::LargeList,
            Self::LargeListView(_) => DataTypeId::LargeListView,
            Self::Structure(StructureType::Struct(_)) => DataTypeId::Struct,
            Self::Structure(StructureType::Tuple2(_)) => DataTypeId::Tuple2,
            Self::Union(..) => DataTypeId::Union,
            Self::Dictionary(_) => DataTypeId::Dictionary,
            Self::Mapping(MappingType::Map(_)) => DataTypeId::Map,
            Self::Mapping(MappingType::SortedMap(_)) => DataTypeId::SortedMap,
            Self::RunEndEncoded(_) => DataTypeId::RunEndEncoded,
            Self::Variant => DataTypeId::Variant,
        }
    }

    /// Return whether this family member wraps another datatype transparently.
    pub const fn is_wrapper(&self) -> bool {
        matches!(
            Self::id(self),
            DataTypeId::Dictionary | DataTypeId::RunEndEncoded
        )
    }
}

impl From<NestedType> for DataType {
    fn from(value: NestedType) -> Self {
        match value {
            NestedType::List(item) => Self::List(item),
            NestedType::ListView(item) => Self::ListView(item),
            NestedType::FixedSizeList(item, length) => Self::FixedSizeList(item, length),
            NestedType::LargeList(item) => Self::LargeList(item),
            NestedType::LargeListView(item) => Self::LargeListView(item),
            NestedType::Structure(structure) => Self::Structure(structure),
            NestedType::Union(fields, mode) => Self::Union(fields, mode),
            NestedType::Dictionary(dtype) => Self::Dictionary(dtype),
            NestedType::Mapping(mapping) => Self::Mapping(mapping),
            NestedType::RunEndEncoded(dtype) => Self::RunEndEncoded(dtype),
            NestedType::Variant => Self::Variant,
        }
    }
}

impl TryFrom<&DataType> for NestedType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::List(item) => Ok(Self::List(Arc::clone(item))),
            DataType::ListView(item) => Ok(Self::ListView(Arc::clone(item))),
            DataType::FixedSizeList(item, length) => {
                Ok(Self::FixedSizeList(Arc::clone(item), *length))
            }
            DataType::LargeList(item) => Ok(Self::LargeList(Arc::clone(item))),
            DataType::LargeListView(item) => Ok(Self::LargeListView(Arc::clone(item))),
            DataType::Structure(structure) => Ok(Self::Structure(structure.clone())),
            DataType::Union(fields, mode) => Ok(Self::Union(fields.clone(), *mode)),
            DataType::Dictionary(dtype) => Ok(Self::Dictionary(Arc::clone(dtype))),
            DataType::Mapping(mapping) => Ok(Self::Mapping(mapping.clone())),
            DataType::RunEndEncoded(dtype) => Ok(Self::RunEndEncoded(Arc::clone(dtype))),
            DataType::Variant => Ok(Self::Variant),
            other => Err(Error::InvalidDataType {
                kind: "nested",
                reason: format_smolstr!("expected a nested datatype, got {other}"),
            }),
        }
    }
}

impl DataType {
    /// Creates the self-describing semi-structured Variant type.
    ///
    /// It takes no parameters: shredding is physical layout, while each value
    /// is the ordinary [`crate::Scalar`] tree. Parentheses distinguish the
    /// finite [`Self::dense_union`] input form in the grammar.
    #[must_use]
    pub const fn variant() -> Self {
        Self::Variant
    }

}

/// One child with its collection, if it is one, replaced by what it holds.
pub(crate) fn exploded(child: &Field) -> Field {
    let held = match child.dtype() {
        DataType::List(item)
        | DataType::ListView(item)
        | DataType::FixedSizeList(item, _)
        | DataType::LargeList(item)
        | DataType::LargeListView(item) => Some((item.dtype().clone(), item.is_nullable())),
        DataType::Mapping(map) => Some((map.entries().dtype().clone(), map.entries().is_nullable())),
        DataType::RunEndEncoded(encoded) => Some((
            encoded.values().dtype().clone(),
            encoded.values().is_nullable(),
        )),
        DataType::Dictionary(dictionary) => Some((dictionary.value().clone(), false)),
        _ => None,
    };
    match held {
        Some((dtype, element_nullable)) => {
            let mut exploded =
                Field::new(child.name(), dtype, child.is_nullable() || element_nullable);
            // The column's own annotations describe the column, not the
            // collection layout, so they survive the expansion.
            let _ = exploded.set_metadata(child.metadata_iter());
            exploded
        }
        None => child.clone(),
    }
}

/// Report a path that names no child, and the names that exist beside it.
pub(crate) fn missing_child(node: &DataType, path: &str) -> Error {
    let names: Vec<&str> = (0..node.field_len())
        .filter_map(|index| node.get_field_at(index))
        .map(Field::name)
        .collect();
    Error::InvalidRecord {
        path: format_smolstr!("$.{path}"),
        reason: crate::text::expected_got(
            format_smolstr!("a child among {names:?}"),
            format_smolstr!("{path:?}"),
        ),
    }
}

pub(crate) fn cmp_field_slices(left: &[Field], right: &[Field]) -> Ordering {
    let mut left = left.iter();
    let mut right = right.iter();
    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) => {
                let order = cmp_fields(left, right);
                if order != Ordering::Equal {
                    return order;
                }
            }
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
        }
    }
}

pub(crate) fn cmp_fields(left: &Field, right: &Field) -> Ordering {
    left.cmp(right)
}

pub(crate) fn validate_map_entries(entries: &Field) -> Result<()> {
    if entries.is_nullable() {
        return Err(invalid("Map", "entries field must be non-null"));
    }
    let DataType::Structure(children) = entries.dtype() else {
        return Err(invalid("Map", "entries field must contain a struct"));
    };
    if children.len() != 2 {
        return Err(invalid(
            "Map",
            "entries struct must contain exactly key and value fields",
        ));
    }
    if children[0].is_nullable() {
        return Err(invalid("Map", "key field must be non-null"));
    }
    Ok(())
}

pub(crate) fn validate_dictionary_key(key: &DataType) -> Result<()> {
    if is_valid_dictionary_key(key) {
        Ok(())
    } else {
        Err(invalid(
            "Dictionary",
            format_smolstr!(
                "expected an integer key datatype (int8, int16, int32, int64, uint8, uint16, uint32, or uint64), got {key}"
            ),
        ))
    }
}

pub(crate) fn validate_run_ends(run_ends: &Field) -> Result<()> {
    // Two independent rules; report the one that actually fired so a caller
    // fixes the right half.
    if run_ends.is_nullable() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a non-null run_ends field, got nullable field {:?}",
                run_ends.name()
            ),
        ));
    }
    if !run_ends.dtype().is_run_ends_type() {
        return Err(invalid(
            "RunEndEncoded",
            format_smolstr!(
                "expected a run_ends datatype of int16, int32, or int64, got {}",
                run_ends.dtype()
            ),
        ));
    }
    Ok(())
}

fn is_valid_dictionary_key(key: &DataType) -> bool {
    key.is_integer()
}

pub(crate) fn validate_fields(fields: &[Field], kind: &'static str) -> Result<()> {
    reject_duplicate_field_names(fields, kind)?;
    fields.iter().try_for_each(Field::validate)
}

pub(crate) fn validate_union_fields(fields: &UnionFields) -> Result<()> {
    validate_union_values(fields.as_fields(), true)
}

pub(crate) fn validate_union_values(values: &[(i8, Field)], validate_children: bool) -> Result<()> {
    let mut seen = 0_u128;
    for (index, (type_id, field)) in values.iter().enumerate() {
        if *type_id < 0 {
            return Err(invalid(
                "Union",
                format_smolstr!("type id must be non-negative: {type_id}"),
            ));
        }
        let mask = 1_u128 << *type_id;
        if seen & mask != 0 {
            return Err(invalid(
                "Union",
                format_smolstr!("duplicate type id: {type_id}"),
            ));
        }
        seen |= mask;
        if values[..index]
            .iter()
            .any(|(_, previous)| previous.name() == field.name())
        {
            return Err(invalid(
                "Union",
                format_smolstr!("duplicate field name {:?}", field.name()),
            ));
        }
        if validate_children {
            field.validate()?;
        }
    }
    Ok(())
}

pub(crate) fn reject_duplicate_field_names(fields: &[Field], kind: &'static str) -> Result<()> {
    const HASHED_DUPLICATE_CHECK_THRESHOLD: usize = 16;

    if fields.len() > HASHED_DUPLICATE_CHECK_THRESHOLD {
        let mut names = HashSet::with_capacity(fields.len());
        for field in fields {
            if !names.insert(field.name()) {
                return Err(invalid(
                    kind,
                    format_smolstr!("duplicate field name {:?}", field.name()),
                ));
            }
        }
        return Ok(());
    }

    for (index, field) in fields.iter().enumerate() {
        if fields[..index]
            .iter()
            .any(|previous| previous.name() == field.name())
        {
            return Err(invalid(
                kind,
                format_smolstr!("duplicate field name {:?}", field.name()),
            ));
        }
    }
    Ok(())
}

/// Subscripting a datatype reaches a nested **child**, never metadata.
///
/// The same semantic [`Field`] carries, so a caller walking a schema gets a
/// child from every node in the graph. The string is resolved by
/// [`DataType::get_field_by_path`] - an exact name first, a dotted path after -
/// and that method is the non-panicking form.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let row = DataType::from_fields([DataType::Int64.required_field("id")])?;
/// assert_eq!(row["id"].dtype(), &DataType::Int64);
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this datatype has no child with that name - including when it is
/// not a nested datatype at all, which has no children by definition.
impl Index<&str> for DataType {
    type Output = Field;

    fn index(&self, path: &str) -> &Self::Output {
        self.get_field_by_path(path)
            .unwrap_or_else(|| panic!("{path:?} is not a child of the datatype {self}"))
    }
}

/// Subscripting a datatype by position reaches that nested child.
///
/// ```
/// use yggdryl::DataType;
///
/// # fn main() -> yggdryl::Result<()> {
/// let items = DataType::list(DataType::utf8().nullable_field("item"));
/// assert_eq!(items[0].name(), "item");
/// # Ok(())
/// # }
/// ```
///
/// # Panics
///
/// Panics when this datatype has no child at that position.
impl Index<usize> for DataType {
    type Output = Field;

    fn index(&self, index: usize) -> &Self::Output {
        self.get_field_at(index).unwrap_or_else(|| {
            panic!(
                "the datatype {self} has {} children, so position {index} is out of range",
                self.field_len()
            )
        })
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
        let dtype = self.dtype.merge(&other.dtype, how, recode)?;
        let mut merged = Self::new(self.name.clone(), dtype, self.nullable || other.nullable);
        // One rule, on `Metadata` itself: the union of both, this field
        // winning any key they disagree on.
        merged.set_metadata(self.metadata.merge_with(&other.metadata)?.iter())?;
        if self.dictionary_id != 0 && other.dictionary_id != 0 {
            merged.set_dictionary_options(
                self.dictionary_id,
                self.dictionary_is_ordered && other.dictionary_is_ordered,
            )?;
        }
        Ok(merged)
    }
}

define_field_types!(
    FixedSizeListType,
    FixedSizeList,
    crate::DataType::FixedSizeList(..)
);

define_field_types!(
    LargeListViewType,
    LargeListView,
    crate::DataType::LargeListView(_)
);

// The variant lives with the nested family: it is the self-describing
// sibling of the union whose grammar it shares (`variant` bare, `variant(...)`
// as dense-union sugar), and its Arrow storage is a struct of two binaries.
define_field_types!(VariantType, Variant, crate::DataType::Variant);

define_field_types!(
    DictionaryTypeMarker,
    Dictionary,
    crate::DataType::Dictionary(_)
);

define_field_types!(MapTypeMarker, Map, crate::DataType::Mapping(_));

define_field_types!(
    RunEndEncodedTypeMarker,
    RunEndEncoded,
    crate::DataType::RunEndEncoded(_)
);

/// A variant-typed field.
pub type VariantField = TypedField<VariantType>;

// ------------------------------------------------------------------------
// Nested datatype grammar.
// ------------------------------------------------------------------------
impl Parser<'_> {
    pub(crate) fn parse_list(&mut self, kind: ListKind, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected a list child in (), [], {}, or <>"))?;
        let field = self.parse_field_or_type("item", true, depth)?;
        self.expect_symbol(close)?;
        Ok(match kind {
            ListKind::List => DataType::list(field),
            ListKind::ListView => DataType::list_view(field),
            ListKind::LargeList => DataType::large_list(field),
            ListKind::LargeListView => DataType::large_list_view(field),
        })
    }

    pub(crate) fn parse_fixed_size_list(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected fixed-size-list parameters"))?;
        let field = self.parse_field_or_type("item", true, depth)?;
        self.expect_separator("expected a list length after the child")?;
        self.consume_label("length");
        let length = self.parse_i32("list length")?;
        self.expect_symbol(close)?;
        DataType::fixed_size_list(field, length)
    }

    pub(crate) fn parse_struct(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected struct fields"))?;
        let collection_close = if self.peek_symbol() == Some('[') {
            self.index += 1;
            Some(']')
        } else {
            None
        };
        let body_close = collection_close.unwrap_or(close);
        let mut fields = Vec::new();
        while self.peek_symbol() != Some(body_close) {
            fields.push(self.parse_named_field(depth)?);
            if self.peek_symbol() == Some(body_close) {
                break;
            }
            self.expect_separator("expected ',' between struct fields")?;
        }
        self.expect_symbol(body_close)?;
        if collection_close.is_some() {
            self.expect_symbol(close)?;
        }
        DataType::from_fields(fields)
    }

    pub(crate) fn parse_dictionary(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected dictionary key and value types"))?;
        self.consume_label("key");
        let key = self.parse_type(depth)?;
        self.expect_separator("expected dictionary value type")?;
        self.consume_label("value");
        let value = self.parse_type(depth)?;
        self.expect_symbol(close)?;
        DataType::dictionary(key, value)
    }

    pub(crate) fn parse_map(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected map parameters"))?;

        if self.peek_word_is("field") {
            let entries = self.parse_explicit_field(depth, Some("entries"))?;
            let mut keys_sorted = false;
            if self.consume_separator() {
                self.consume_label("keys_sorted");
                keys_sorted = self.parse_bool("keys_sorted")?;
            }
            self.expect_symbol(close)?;
            return DataType::map(entries, keys_sorted);
        }

        self.consume_label("key");
        let key = self.parse_type(depth)?;
        self.expect_separator("expected map value type")?;
        self.consume_label("value");
        let value = self.parse_type(depth)?;
        let mut keys_sorted = false;
        if self.consume_separator() {
            self.consume_label("keys_sorted");
            keys_sorted = self.parse_bool("keys_sorted")?;
        }
        self.expect_symbol(close)?;
        DataType::map_of(key, value, keys_sorted)
    }

    pub(crate) fn parse_run_end(&mut self, depth: usize) -> Result<DataType> {
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected run-end and value fields"))?;
        let run_ends = self.parse_field_or_type("run_ends", false, depth)?;
        self.expect_separator("expected encoded values field")?;
        let values = self.parse_field_or_type("values", true, depth)?;
        self.expect_symbol(close)?;
        DataType::run_end_encoded(run_ends, values)
    }

    /// Parse the optional `('crs')` / `('crs', 'algorithm')` parameters.
    ///
    /// Bare `geometry` and `geography` fill the defaults, so the parameters
    /// appear exactly when they say something. A geometry given an edge
    /// algorithm is refused by name at the algorithm's own position -
    /// straight planar lines need none - and an unknown algorithm reports the
    /// accepted vocabulary.
    pub(crate) fn parse_union(&mut self, keyword: &str, depth: usize) -> Result<DataType> {
        let is_variant = keyword == "variant";
        let close = self
            .consume_opening()
            .ok_or_else(|| self.error_here("expected union or variant members"))?;
        let mut mode = if keyword == "denseunion" || is_variant {
            UnionMode::Dense
        } else {
            UnionMode::Sparse
        };
        if self.peek_union_mode(close) {
            let position = self.current_position();
            mode = self.parse_union_mode()?;
            if is_variant && mode == UnionMode::Sparse {
                return Err(self.error_at(position, "variant layout must be dense"));
            }
            if self.peek_symbol() != Some(close) {
                self.expect_separator("expected ',' after union mode")?;
            }
        }

        let collection_close = if self.peek_symbol() == Some('[') {
            self.index += 1;
            Some(']')
        } else {
            None
        };
        let body_close = collection_close.unwrap_or(close);
        let mut fields = Vec::new();
        let mut next_id = 0_i16;

        while self.peek_symbol() != Some(body_close) {
            let member_close = if self.peek_symbol() == Some('(') {
                self.index += 1;
                Some(')')
            } else {
                None
            };

            let type_id = if let Some(value) = self.peek_integer() {
                let position = self.current_position();
                self.index += 1;
                let id = i8::try_from(value)
                    .map_err(|_| self.error_at(position, "union type id must fit in i8"))?;
                if is_variant && i16::from(id) != next_id {
                    return Err(
                        self.error_at(position, "variant type ids must be sequential from zero")
                    );
                }
                if !self.consume_symbol('=')
                    && !self.consume_symbol(':')
                    && !self.consume_symbol(',')
                {
                    return Err(self.error_here("expected '=', ':', or ',' after union type id"));
                }
                id
            } else {
                i8::try_from(next_id).map_err(|_| {
                    self.error_here(if is_variant {
                        "a variant cannot contain more than 128 members"
                    } else {
                        "a union cannot contain more than 128 members"
                    })
                })?
            };
            next_id = i16::from(type_id) + 1;
            let field =
                self.parse_field_or_type(&format_smolstr!("member_{type_id}"), true, depth)?;
            if let Some(member_close) = member_close {
                self.expect_symbol(member_close)?;
            }
            fields.push((type_id, field));

            if self.peek_symbol() == Some(body_close) {
                break;
            }
            self.expect_separator("expected ',' between union members")?;
        }
        self.expect_symbol(body_close)?;

        if collection_close.is_some() {
            if self.consume_separator() {
                let position = self.current_position();
                mode = self.parse_union_mode()?;
                if is_variant && mode == UnionMode::Sparse {
                    return Err(self.error_at(position, "variant layout must be dense"));
                }
            }
            self.expect_symbol(close)?;
        }
        if is_variant {
            DataType::dense_union(fields.into_iter().map(|(_, field)| field))
        } else {
            DataType::union(fields, mode)
        }
    }

    pub(crate) fn parse_union_mode(&mut self) -> Result<UnionMode> {
        let value = self.parse_text("union mode")?;
        match normalized(&value).as_str() {
            "dense" => Ok(UnionMode::Dense),
            "sparse" => Ok(UnionMode::Sparse),
            _ => Err(self.error_here("union mode must be dense or sparse")),
        }
    }
}

// ------------------------------------------------------------------------
// Nested values and typed scalar aliases.
// ------------------------------------------------------------------------

/// Borrowing access shared by every nested value shape.
pub trait NestedValue: crate::Value {
    /// Return the number of direct children.
    fn len(&self) -> usize;
    /// Return whether this value has no direct children.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Iterate over direct sequence values, mapping keys, or record values.
    fn children(&self) -> Children<'_>;
}

/// One ordered sequence of scalar children.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Sequence(Arc<[Scalar]>);

impl Sequence {
    /// Construct an ordered sequence.
    pub fn new(values: impl Into<Arc<[Scalar]>>) -> Self {
        Self(values.into())
    }

    /// Borrow the ordered values.
    pub fn as_slice(&self) -> &[Scalar] {
        self.0.as_ref()
    }

    /// Consume this value and return its shared children.
    pub fn into_inner(self) -> Arc<[Scalar]> {
        self.0
    }
}

impl fmt::Display for Sequence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_slice())
    }
}

/// One deterministic record sorted by field name.
#[repr(transparent)]
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Record(Arc<BTreeMap<SmolStr, Scalar>>);

impl Record {
    /// Construct a sorted record.
    pub fn new(values: impl Into<Arc<BTreeMap<SmolStr, Scalar>>>) -> Self {
        Self(values.into())
    }

    /// Borrow the sorted fields.
    pub fn as_map(&self) -> &BTreeMap<SmolStr, Scalar> {
        self.0.as_ref()
    }

    /// Consume this value and return its shared fields.
    pub fn into_inner(self) -> Arc<BTreeMap<SmolStr, Scalar>> {
        self.0
    }
}

impl fmt::Display for Record {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.as_map())
    }
}

/// A borrowed iterator over sequence values or mapping keys.
pub enum Children<'a> {
    /// Sequence values.
    Sequence(std::slice::Iter<'a, Scalar>),
    /// Mapping keys.
    Mapping(std::slice::Iter<'a, (Scalar, Scalar)>),
    /// Record field values in sorted name order.
    Record(std::collections::btree_map::Values<'a, SmolStr, Scalar>),
}

impl<'a> Iterator for Children<'a> {
    type Item = &'a Scalar;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next(),
            Self::Mapping(entries) => entries.next().map(|(key, _)| key),
            Self::Record(entries) => entries.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let length = self.len();
        (length, Some(length))
    }
}

impl DoubleEndedIterator for Children<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Sequence(values) => values.next_back(),
            Self::Mapping(entries) => entries.next_back().map(|(key, _)| key),
            Self::Record(entries) => entries.next_back(),
        }
    }
}

impl ExactSizeIterator for Children<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Sequence(values) => values.len(),
            Self::Mapping(entries) => entries.len(),
            Self::Record(entries) => entries.len(),
        }
    }
}

impl std::iter::FusedIterator for Children<'_> {}

// A nested shape is its own scalar family, exactly as `Boolean` is.
macro_rules! nested_value {
    ($leaf:ident, $variant:ident, $id:ident) => {
        impl Value for $leaf {
            fn dtype(&self) -> Result<DataType> {
                Scalar::$variant(self.clone()).dtype()
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$variant(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$variant(value) => Some(value),
                    _ => None,
                }
            }
        }
    };
}

nested_value!(Sequence, Sequence, List);

nested_value!(Record, Record, Struct);

impl NestedValue for Sequence {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Sequence(self.as_slice().iter())
    }
}

impl NestedValue for Record {
    fn len(&self) -> usize {
        self.as_map().len()
    }

    fn children(&self) -> Children<'_> {
        Children::Record(self.as_map().values())
    }
}

impl<'a> IntoIterator for &'a Scalar {
    type Item = &'a Scalar;
    type IntoIter = Children<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<Scalar>> for Scalar {
    fn from(value: Vec<Scalar>) -> Self {
        Self::from_sequence(value)
    }
}

impl FromIterator<Scalar> for Scalar {
    fn from_iter<T: IntoIterator<Item = Scalar>>(iter: T) -> Self {
        Self::from_sequence(iter)
    }
}

impl Index<usize> for Scalar {
    type Output = Scalar;

    fn index(&self, index: usize) -> &Self::Output {
        &self.as_sequence().expect("value is not a sequence")[index]
    }
}

impl Index<&Scalar> for Scalar {
    type Output = Scalar;

    fn index(&self, key: &Scalar) -> &Self::Output {
        self.get_key(key).expect("mapping key is not present")
    }
}

impl Index<&str> for Scalar {
    type Output = Scalar;

    fn index(&self, key: &str) -> &Self::Output {
        self.get_key_str(key).expect("mapping key is not present")
    }
}
