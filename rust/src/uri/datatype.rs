//! The `uri` family: the `url` and `urn` datatypes over the values this
//! folder parses.
//!
//! [`Uri`] is every identifier, and a column holds one of its two leaves -
//! a [`Url`], a location, or a [`Urn`], a name - the same parsed values a
//! handle addresses itself by. Parsing canonicalizes: a scheme and
//! percent-encoding fold to their canonical case, a bare platform path
//! becomes a `file:` URL, a URN's namespace folds to lower case, and
//! re-parsing the canonical text answers the same value. Arrow stores that
//! text as Utf8; each leaf's extension name preserves it on a field round
//! trip, so a column that went out as a URL comes back as one rather than as
//! prose that happens to look like a location, and a column of names comes
//! back as names.
//!
//! Ordering is the canonical text's, which is Arrow's own string ordering:
//! there is no numeric component to sort by, as there is for
//! [`Version`](crate::Version).
//!
//! Each value is held behind one shared pointer. A parsed identifier is
//! sixteen times the width of the scalar root, and a column of them is cloned
//! once per row, so a row clone moves a reference count rather than a URI.

use std::fmt;
use std::sync::Arc;

use crate::value::{DataTypeValue, Value};
use crate::{DataType, DataTypeId, DataTypeKind, Result, Scalar, Uri, Url, Urn};

/// Arrow casts owned by the family: one ingest per leaf.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, StringArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::arrow::{Error, Result};
    use crate::budget::MaterializationBudget;
    use crate::cast::columns::is_exposed;
    use crate::cast::{arrow_cast_exposed, downcast};
    use crate::{DataType, Field, Url, Urn};

    /// Parse and canonicalize every exposed text cell into URN Utf8 storage.
    pub(crate) fn ingest_urn_array(
        array: &ArrayRef,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        ingest_canonical_text(array, field, exposure, budget, "urn", |raw| {
            Urn::from_str(raw).map(|urn| urn.to_string())
        })
    }

    /// Parse and canonicalize every exposed text cell into URL Utf8 storage.
    pub(crate) fn ingest_url_array(
        array: &ArrayRef,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        ingest_canonical_text(array, field, exposure, budget, "url", |raw| {
            Url::from_str(raw).map(|url| url.to_string())
        })
    }

    /// Parse every exposed text cell through `parse` into the canonical text
    /// it names, refusing a cell that does not read as one by field and row.
    fn ingest_canonical_text(
        array: &ArrayRef,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
        name: &str,
        parse: impl Fn(&str) -> crate::Result<String>,
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
            let canonical = parse(raw).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: {raw:?} does not read as {name}: {error}",
                    field.name()
                ))
            })?;
            payload = payload.saturating_add(canonical.len());
            values.push(Some(canonical));
        }
        budget.add_bytes(payload)?;
        Ok(Arc::new(StringArray::from(values)))
    }
}

// ------------------------------------------------------------------------
// The family payload: one leaf per shape an identifier takes.
// ------------------------------------------------------------------------

/// The uri family's datatype payload: one leaf per shape an identifier takes.
///
/// A leaf has no parameter: what the scheme decides is what the leaf is, so
/// the leaf says it and the value it holds - a [`Url`] or a [`Urn`] - is the
/// narrowing of one [`Uri`] that shape names.
///
/// ```
/// use yggdryl::{DataType, DataTypeId, UriType};
///
/// assert_eq!(UriType::Url.id(), DataTypeId::Url);
/// assert_eq!(UriType::from_id(DataTypeId::Urn), Some(UriType::Urn));
/// assert_eq!(UriType::from_id(DataTypeId::Utf8String), None);
/// assert_eq!(DataType::url(), DataType::Uri(UriType::Url));
/// assert_eq!(DataType::urn().to_string(), "urn");
/// assert_eq!(DataType::urn().uri_type(), Some(UriType::Urn));
/// assert_eq!(UriType::Urn.family(), "uri");
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum UriType {
    /// A location: hierarchical, with a host unless `file:`, never a name.
    #[default]
    Url,
    /// A name: `urn:<namespace>:<specific>`, carrying no authority.
    Urn,
}

impl UriType {
    /// Every leaf in identifier order.
    pub const ALL: [Self; 2] = [Self::Url, Self::Urn];

    /// Return the exact datatype identifier.
    #[must_use]
    pub const fn id(self) -> DataTypeId {
        match self {
            Self::Url => DataTypeId::Url,
            Self::Urn => DataTypeId::Urn,
        }
    }

    /// The leaf one identifier names, or `None` for an identifier of another
    /// family.
    #[must_use]
    pub const fn from_id(id: DataTypeId) -> Option<Self> {
        match id {
            DataTypeId::Url => Some(Self::Url),
            DataTypeId::Urn => Some(Self::Urn),
            _ => None,
        }
    }

    /// The canonical name of this leaf.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.id().as_str()
    }

    /// The family's name, `uri`, as a datatype spells it.
    #[must_use]
    pub const fn family(self) -> &'static str {
        "uri"
    }

    /// A leaf has no parameter to refuse.
    ///
    /// # Errors
    ///
    /// Never; the signature is the family contract's.
    pub const fn validate(self) -> Result<()> {
        Ok(())
    }

    /// The Arrow extension name preserving this leaf over its Utf8 storage.
    pub(crate) const fn extension_name(self) -> &'static str {
        match self {
            Self::Url => URL_EXTENSION_NAME,
            Self::Urn => URN_EXTENSION_NAME,
        }
    }

    /// The Arrow storage every leaf lays out: its canonical text.
    pub(crate) const fn arrow_storage() -> arrow_schema::DataType {
        arrow_schema::DataType::Utf8
    }
}

impl DataTypeValue for UriType {
    const FAMILY: &'static str = "uri";

    type Sidecar = ();

    fn id(&self) -> DataTypeId {
        Self::id(*self)
    }

    fn kind(&self) -> DataTypeKind {
        DataTypeKind::Text
    }

    fn validate(&self) -> Result<()> {
        Self::validate(*self)
    }

    fn into_dtype(self) -> DataType {
        DataType::Uri(self)
    }

    fn from_dtype(dtype: &DataType) -> Option<Self> {
        match dtype {
            DataType::Uri(leaf) => Some(*leaf),
            _ => None,
        }
    }
}

impl fmt::Display for UriType {
    /// The canonical spelling, which [`crate::DataType`]'s grammar reads back.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<UriType> for DataType {
    fn from(value: UriType) -> Self {
        Self::Uri(value)
    }
}

impl DataType {
    /// The `url` leaf: a column of locations.
    #[must_use]
    pub const fn url() -> Self {
        Self::Uri(UriType::Url)
    }

    /// The `urn` leaf: a column of names.
    #[must_use]
    pub const fn urn() -> Self {
        Self::Uri(UriType::Urn)
    }

    /// The uri leaf this datatype is, or `None` for another family.
    #[must_use]
    pub const fn uri_type(&self) -> Option<UriType> {
        match self {
            Self::Uri(leaf) => Some(*leaf),
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------
// [`Url`] and [`Urn`] as scalar values.
//
// The value types themselves live beside this file - an identifier is what a
// handle addresses itself by, and there is one of those, not one for
// locations and another for columns. This is only what makes each a scalar.
// ------------------------------------------------------------------------

impl Value for Url {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::url())
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

impl Value for Urn {
    fn dtype(&self) -> Result<DataType> {
        Ok(DataType::urn())
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Urn(Arc::new(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Urn(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Urn> for Scalar {
    fn from(value: Urn) -> Self {
        Self::Urn(Arc::new(value))
    }
}

impl Scalar {
    /// The identifier a `url` or a `urn` scalar holds: the one [`Uri`] both
    /// leaves narrow, borrowed.
    #[must_use]
    pub fn as_uri(&self) -> Option<&Uri> {
        match self {
            Self::Url(url) => Some(AsRef::<Uri>::as_ref(&**url)),
            Self::Urn(urn) => Some(AsRef::<Uri>::as_ref(&**urn)),
            _ => None,
        }
    }
}

/// The Arrow extension name preserving the `url` leaf over its Utf8 storage.
pub(crate) const URL_EXTENSION_NAME: &str = "yggdryl.url";

/// The Arrow extension name preserving the `urn` leaf over its Utf8 storage.
pub(crate) const URN_EXTENSION_NAME: &str = "yggdryl.urn";
