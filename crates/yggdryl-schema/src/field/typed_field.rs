//! The field implementation covering every data type.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::{DataType, Field};

/// The shared handle to a [`TypedField`]; the `Arc` clone is the cheap
/// sharing mechanism.
pub type TypedFieldRef<T> = Arc<TypedField<T>>;

/// The concrete [`Field`] implementation generic over its data type — the one
/// implementation that gives every [`DataType`] its corresponding field
/// (`TypedField<Int32Type>`, `TypedField<Utf8Type>`, …).
///
/// Metadata is a `BTreeMap` (not a `HashMap`) so iteration order — and with
/// it `Hash` and the byte encoding — is deterministic.
///
/// ```
/// use yggdryl_schema::{Field, Int32Type, TypedField};
///
/// let field = TypedField::from_parts("id", Int32Type, false, Default::default());
/// let arrow = field.to_arrow();
/// assert_eq!(TypedField::from_arrow(&arrow), Ok(field));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct TypedField<T: DataType> {
    name: String,
    data_type: T,
    nullable: bool,
    metadata: BTreeMap<String, String>,
}

impl<T: DataType> Field for TypedField<T> {
    type DataType = T;

    fn from_parts(
        name: impl Into<String>,
        data_type: T,
        nullable: bool,
        metadata: BTreeMap<String, String>,
    ) -> Self {
        Self {
            name: name.into(),
            data_type,
            nullable,
            metadata,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn data_type(&self) -> &T {
        &self.data_type
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn metadata(&self) -> &BTreeMap<String, String> {
        &self.metadata
    }
}

impl<T: DataType> fmt::Display for TypedField<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.data_type)?;
        if self.nullable {
            f.write_str("?")?;
        }
        Ok(())
    }
}
