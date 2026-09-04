//! Nested null repair, defaults, and child rebuilding.

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
