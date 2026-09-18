//! Validated locations as one generic scalar value.
//!
//! A URL is the crate's [`Url`](crate::Url) - the same parsed value a handle
//! addresses itself by - carried as a column. Parsing canonicalizes: a scheme
//! and percent-encoding fold to their canonical case, a bare platform path
//! becomes a `file:` URL, and re-parsing the canonical text answers the same
//! value. Arrow stores that text as Utf8; its extension name preserves the
//! datatype on a field round trip, so a column that went out as a URL comes
//! back as one rather than as prose that happens to look like a location.
//!
//! Ordering is the canonical text's, which is Arrow's own string ordering:
//! there is no numeric component to sort by, as there is for
//! [`Version`](crate::Version).
//!
//! The value is held behind one shared pointer. A parsed URL is sixteen times
//! the width of the scalar root, and a column of them is cloned once per row,
//! so a row clone moves a reference count rather than a URI.

use std::sync::Arc;

use crate::types::scalar::Value;
use crate::types::typed::define_field_types;
use crate::{DataType, Result, Scalar, TypedField, Url};

/// Arrow casts owned by the URL datatype.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::types::budget::MaterializationBudget;
    use crate::types::cast::{arrow_cast_exposed, downcast};
    use crate::types::cast::columns::is_exposed;
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
}

// ------------------------------------------------------------------------
// URL field marker and typed aliases.
// ------------------------------------------------------------------------

define_field_types!(UrlType, Url, crate::DataType::Url);

/// A URL-typed field.
pub type UrlField = TypedField<UrlType>;

// ------------------------------------------------------------------------
// [`Url`] as a scalar value.
//
// The value type itself lives in [`crate::uri`] - a URL is what a handle
// addresses itself by, and there is one of those, not one for locations and
// another for columns. This is only what makes that value a scalar.
// ------------------------------------------------------------------------

impl Value for Url {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::Url)
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Url(Arc::new(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Url(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Url> for Scalar {
    fn from(value: Url) -> Self {
        Self::Url(Arc::new(value))
    }
}

/// The Arrow extension name preserving [`crate::DataType::Url`] over its Utf8
/// storage.
pub(crate) const URL_EXTENSION_NAME: &str = "yggdryl.url";
