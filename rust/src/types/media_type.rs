//! The media type as a datatype: a column of canonical
//! `type/subtype; charset=...` names with their content codings.
//!
//! The value itself lives beside [`crate::MediaType`] - a media type is what a
//! record layer is read and written under, and there is one of those. This is
//! only what makes that value a datatype, a field and a scalar.
//!
//! Storage is Arrow's `Utf8`: the base, the charset and the codings are one
//! canonical rendering, so a cell holds what the value spells and the
//! extension name keeps it a media type across a round trip. The value is
//! wider than the scalar enum - a base, a charset and a shared coding list -
//! so a scalar holds it behind one shared pointer, exactly as a URL is held.
//!
//! ```
//! use yggdryl::{DataType, MediaType, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! assert_eq!(DataType::from_str("mediatype")?, DataType::MediaType);
//! let json = MediaType::from_str("application/json")?;
//! assert_eq!(DataType::MediaType.scalar("application/json")?, Scalar::from(json));
//! # Ok(())
//! # }
//! ```

use std::sync::Arc;

use crate::types::scalar::Value;
use crate::types::typed::define_field_types;
use crate::{DataType, MediaType, Result, Scalar};

/// Arrow casts owned by the media type datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::cast::{arrow_cast_exposed, downcast};
    use crate::types::cast::columns::is_exposed;
    use crate::{DataType, Field, MediaType};

    /// Parse and canonicalize every exposed text cell into media type Utf8 storage.
    pub(crate) fn ingest_media_type_array(
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
                &Field::new(field.name(), DataType::utf8(), true),
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
            let parsed = MediaType::from_str(raw).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {raw:?} does not read as media type: {error}",
                    field.name()
                ))
            })?;
            let canonical = parsed.to_string();
            payload = payload.saturating_add(canonical.len());
            values.push(Some(canonical));
        }
        budget.add_bytes(payload)?;
        Ok(Arc::new(StringArray::from(values)))
    }
}

// ------------------------------------------------------------------------
// Media type field marker and typed aliases.
// ------------------------------------------------------------------------

define_field_types!(MediaTypeType, MediaType);


// ------------------------------------------------------------------------
// [`MediaType`] as a scalar value.
//
// The value type itself lives beside [`crate::MediaType`]; this is only what
// makes it a scalar. It is held behind one shared pointer: a media type
// carries a base, a charset and a coding list, which is wider than the scalar
// enum, and a column of them is cloned once per row.
// ------------------------------------------------------------------------

impl Value for MediaType {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::MediaType)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::MediaType(Arc::new(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::MediaType(value) => Some(value),
            _ => None,
        }
    }
}

impl From<MediaType> for Scalar {
    fn from(value: MediaType) -> Self {
        Self::MediaType(Arc::new(value))
    }
}

/// The Arrow extension name preserving [`crate::DataType::MediaType`] over its
/// Utf8 storage.
pub(crate) const MEDIATYPE_EXTENSION_NAME: &str = "yggdryl.mediatype";
