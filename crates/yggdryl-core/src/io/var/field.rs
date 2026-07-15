//! [`ByteField`] — a named, nullable descriptor for a variable-length column (`Utf8Field =
//! ByteField<Utf8>`).

use core::marker::PhantomData;

use super::dtype::OFFSET_WIDTH;
use super::{ByteType, VarElement};
use crate::io::fixed::Field;
use crate::io::{DataTypeId, FieldType, Headers};

/// The **variable-length field** sub-trait — the sibling of
/// [`FixedField`](crate::io::fixed::FixedField) for a variable-length column descriptor.
pub trait VarField: FieldType {}

/// A **typed** variable-length field — a name + nullability with the element kind `E` fixed at
/// compile time. [`erase`](ByteField::erase) drops to a runtime [`Field`].
///
/// ```
/// use yggdryl_core::io::var::{ByteField, Utf8};
/// use yggdryl_core::io::FieldType;
///
/// let f = <ByteField<Utf8>>::new("city", true);
/// assert_eq!(f.name(), "city");
/// assert!(f.nullable() && f.is_utf8() && f.is_variable_length());
/// ```
pub struct ByteField<E: VarElement> {
    name: String,
    nullable: bool,
    metadata: Headers,
    _element: PhantomData<E>,
}

impl<E: VarElement> ByteField<E> {
    /// Builds a typed field from a name and its nullability, with empty metadata.
    pub fn new(name: &str, nullable: bool) -> Self {
        Self {
            name: name.to_string(),
            nullable,
            metadata: Headers::new(),
            _element: PhantomData,
        }
    }

    /// The field's metadata [`Headers`].
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// A fresh field with the given metadata [`Headers`] attached.
    pub fn with_metadata(mut self, metadata: Headers) -> Self {
        self.metadata = metadata;
        self
    }

    /// A fresh field with one extra `key = value` metadata entry.
    pub fn with_metadata_entry(mut self, key: &str, value: &str) -> Self {
        self.metadata.insert(key, value);
        self
    }

    /// The typed data-type descriptor.
    pub fn data_type(&self) -> ByteType<E> {
        ByteType::new()
    }

    /// The erased runtime [`Field`], metadata preserved.
    pub fn erase(&self) -> Field {
        Field::new(&self.name, &self.data_type(), self.nullable)
            .with_metadata(self.metadata.clone())
    }

    /// This field as an [`arrow_schema::Field`] (feature `arrow`), via the erased [`Field`].
    #[cfg(feature = "arrow")]
    pub fn to_arrow(&self) -> arrow_schema::Field {
        self.erase().to_arrow()
    }

    /// Builds a typed var field from an [`arrow_schema::Field`], or `None` if its logical type is
    /// not this kind `E` (feature `arrow`). Any user metadata is preserved.
    #[cfg(feature = "arrow")]
    pub fn from_arrow(field: &arrow_schema::Field) -> Option<Self> {
        let erased = Field::from_arrow(field)?;
        (FieldType::type_id(&erased) == E::TYPE_ID).then(|| {
            Self::new(erased.name(), erased.nullable()).with_metadata(erased.metadata().clone())
        })
    }
}

impl<E: VarElement> FieldType for ByteField<E> {
    fn name(&self) -> &str {
        &self.name
    }

    fn type_name(&self) -> &'static str {
        E::NAME
    }

    fn byte_width(&self) -> usize {
        OFFSET_WIDTH
    }

    fn nullable(&self) -> bool {
        self.nullable
    }

    fn type_id(&self) -> DataTypeId {
        E::TYPE_ID
    }
}

impl<E: VarElement> VarField for ByteField<E> {}

impl<E: VarElement> Clone for ByteField<E> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            nullable: self.nullable,
            metadata: self.metadata.clone(),
            _element: PhantomData,
        }
    }
}

impl<E: VarElement> PartialEq for ByteField<E> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.nullable == other.nullable
            && self.metadata == other.metadata
    }
}

impl<E: VarElement> Eq for ByteField<E> {}

impl<E: VarElement> core::fmt::Debug for ByteField<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ByteField")
            .field("name", &self.name)
            .field("type", &E::NAME)
            .field("nullable", &self.nullable)
            .finish()
    }
}
