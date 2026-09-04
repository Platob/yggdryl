//! Recursive dictionary alignment and remapping.

use super::*;

pub(crate) fn contains_dictionary(dtype: &DataType) -> bool {
    match dtype {
        DataType::Dictionary(_) => true,
        DataType::List(field)
        | DataType::ListView(field)
        | DataType::FixedSizeList(field, _)
        | DataType::LargeList(field)
        | DataType::LargeListView(field) => contains_dictionary(field.dtype()),
        DataType::Struct(fields) => fields
            .iter()
            .any(|field| contains_dictionary(field.dtype())),
        DataType::Union(fields, _) => fields
            .iter()
            .any(|(_, field)| contains_dictionary(field.dtype())),
        DataType::Map(map) => contains_dictionary(map.entries().dtype()),
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
