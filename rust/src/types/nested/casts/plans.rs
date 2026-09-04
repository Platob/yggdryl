//! Struct, list, and map cast execution.

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
        let DataType::Map(target_map) = self.field.dtype() else {
            return Err(internal_target_error("map"));
        };
        if !unchanged || source_map != target_map.as_ref() {
            validate_map_invariants(target_map, output.as_ref(), exposure, budget)?;
        }
        Ok(output)
    }
}
