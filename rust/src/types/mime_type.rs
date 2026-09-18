//! The MIME type as a datatype: a column of canonical `type/subtype` names.
//!
//! The value itself lives beside [`crate::MimeType`] - a MIME type is what a
//! handle says its bytes are, and there is one of those, not one for media
//! routing and another for columns. This is only what makes that value a
//! datatype, a field and a scalar.
//!
//! Storage is Arrow's `Utf8`, which is what the canonical name is; the
//! extension name is what keeps a MIME column a MIME column across a round
//! trip. Case and parameters canonicalize on the way in, exactly as
//! [`crate::MimeType::from_str`] canonicalizes them everywhere else.
//!
//! ```
//! use yggdryl::{DataType, MimeType, Scalar};
//!
//! # fn main() -> yggdryl::Result<()> {
//! assert_eq!(DataType::from_str("mimetype")?, DataType::MimeType);
//! assert_eq!(
//!     DataType::MimeType.scalar("APPLICATION/JSON")?,
//!     Scalar::MimeType(MimeType::JSON),
//! );
//! # Ok(())
//! # }
//! ```

use crate::types::scalar::{Value, text_scalar_value};
use crate::types::typed::define_field_types;
use crate::{DataType, MimeType, Result, Scalar, TypedField};

#[cfg(feature = "arrow")]
/// Arrow casts owned by the MIME type datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::cast::{arrow_cast_exposed, downcast};
    use crate::types::nested::casts::is_exposed;
    use crate::{DataType, Field, MimeType};

    /// Parse and canonicalize every exposed text cell into MIME type Utf8 storage.
    pub(crate) fn ingest_mime_type_array(
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
            let parsed = MimeType::from_str(raw).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {raw:?} does not read as MIME type: {error}",
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
// MIME type field marker and typed aliases.
// ------------------------------------------------------------------------

define_field_types!(MimeTypeType, MimeType, crate::DataType::MimeType);

/// A MIME-type-typed field.
pub type MimeTypeField = TypedField<MimeTypeType>;

// ------------------------------------------------------------------------
// [`MimeType`] as a scalar value.
//
// The value type itself lives beside [`crate::MimeType`]; this is only what
// makes it a scalar.
// ------------------------------------------------------------------------

text_scalar_value!(MimeType, MimeType, DataTypeId::MimeType, DataType::MimeType);

/// The Arrow extension name preserving [`crate::DataType::MimeType`] over its
/// Utf8 storage.
pub(crate) const MIMETYPE_EXTENSION_NAME: &str = "yggdryl.mimetype";
