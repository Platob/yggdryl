//! Arrow casts owned by the URL datatype.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StringArray};
use arrow_buffer::BooleanBuffer;
use arrow_schema::DataType as ArrowDataType;

use crate::arrow::{Error, Result};
use crate::types::budget::MaterializationBudget;
use crate::types::cast::{arrow_cast_exposed, downcast};
use crate::types::nested::casts::is_exposed;
use crate::{DataType, Field, Url};

/// Parse and canonicalize every exposed text cell into URL Utf8 storage.
pub(crate) fn ingest_url_array(
    array: &ArrayRef,
    field: &Field,
    exposure: Option<&BooleanBuffer>,
    budget: &mut MaterializationBudget,
) -> Result<ArrayRef> {
    let text = if array.data_type() == &ArrowDataType::Utf8 {
        Arc::clone(array)
    } else {
        arrow_cast_exposed(
            array,
            &ArrowDataType::Utf8,
            true,
            exposure,
            &Field::new(field.name(), DataType::Utf8, true),
            budget,
        )?
    };
    let source = downcast::<StringArray>(text.as_ref())?;
    budget.add_array(field.dtype(), source.len())?;
    let mut values = Vec::with_capacity(source.len());
    let mut payload = 0_usize;
    for index in 0..source.len() {
        if !is_exposed(exposure, index) || source.is_null(index) {
            values.push(None);
            continue;
        }
        let raw = source.value(index);
        let url = Url::from_str(raw).map_err(|error| {
            Error::IncompatibleSchema(format!(
                "field {:?} row {index}: {raw:?} does not read as url: {error}",
                field.name()
            ))
        })?;
        let canonical = url.to_string();
        payload = payload.saturating_add(canonical.len());
        values.push(Some(canonical));
    }
    budget.add_bytes(payload)?;
    Ok(Arc::new(StringArray::from(values)))
}
