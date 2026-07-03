//! The [`Null`] field.

use crate::RawField;
use yggdryl_dtype::{DataError, RawDataType};

/// A `null` field: a name paired with the [`null`](yggdryl_dtype::Null) data type.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::RawDataType;
/// use yggdryl_field::{Null, RawField};
///
/// let gap = Null::new("gap", true);
/// assert_eq!(gap.name(), "gap");
/// assert_eq!(gap.data_type().name(), "null");
/// assert!(gap.is_nullable());
/// assert_eq!(Null::from_arrow(&gap.to_arrow()).unwrap(), gap);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Null {
    name: String,
    data_type: yggdryl_dtype::Null,
    nullable: bool,
}

impl Null {
    /// A `null` field named `name`.
    pub fn new(name: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            data_type: yggdryl_dtype::Null,
            nullable,
        }
    }
}

impl RawField<yggdryl_dtype::Null> for Null {
    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &yggdryl_dtype::Null {
        &self.data_type
    }

    fn is_nullable(&self) -> bool {
        self.nullable
    }

    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
        <yggdryl_dtype::Null as RawDataType>::from_arrow(field.data_type())?;
        crate::raw_field::validate_field_metadata(field, "Null")?;
        Ok(Self::new(field.name(), field.is_nullable()))
    }
}
