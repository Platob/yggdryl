//! [`FieldType`] — the root descriptor of a named, nullable column, shared by every family.

use super::{DataTypeCategory, DataTypeId};

/// The **generic field** root trait — the object-safe view of a named, nullable column
/// descriptor (a schema can hold `&dyn FieldType`). Both the fixed family's erased
/// [`Field`](crate::io::fixed::Field) / typed [`TypedField`](crate::io::fixed::TypedField) and
/// the variable family's [`ByteField`](crate::io::var::ByteField) implement it, and the category
/// predicates (mirrored from [`DataType`](super::DataType)) let a schema drill down over a
/// field's type without matching — each a couple of integer comparisons on the [`DataTypeId`].
pub trait FieldType {
    /// The column name.
    fn name(&self) -> &str;

    /// The element type's name (e.g. `"i64"`).
    fn type_name(&self) -> &'static str;

    /// The element type's byte width.
    fn byte_width(&self) -> usize;

    /// Whether the column admits nulls.
    fn nullable(&self) -> bool;

    /// The element type's [`DataTypeId`] — the single source of truth the `is_*` predicates
    /// below reduce to.
    fn type_id(&self) -> DataTypeId;

    /// The element type's coarse [`DataTypeCategory`] — derived from the
    /// [`type_id`](FieldType::type_id).
    fn category(&self) -> DataTypeCategory {
        self.type_id().category()
    }

    /// Whether the element type has a fixed byte width.
    fn is_fixed_width(&self) -> bool {
        self.type_id().is_fixed_width()
    }

    /// Whether the element type is variable-length.
    fn is_variable_length(&self) -> bool {
        self.type_id().is_variable_length()
    }

    /// Whether the element type is any integer.
    fn is_integer(&self) -> bool {
        self.type_id().is_integer()
    }

    /// Whether the element type is an unsigned integer.
    fn is_unsigned_integer(&self) -> bool {
        self.type_id().is_unsigned_integer()
    }

    /// Whether the element type is a signed integer.
    fn is_signed_integer(&self) -> bool {
        self.type_id().is_signed_integer()
    }

    /// Whether the element type is a signed number (signed integer or float).
    fn is_signed(&self) -> bool {
        self.type_id().is_signed()
    }

    /// Whether the element type is a float.
    fn is_floating(&self) -> bool {
        self.type_id().is_floating()
    }

    /// Whether the element type is a scaled decimal.
    fn is_decimal(&self) -> bool {
        self.type_id().is_decimal()
    }

    /// Whether the element type is a temporal value.
    fn is_temporal(&self) -> bool {
        self.type_id().is_temporal()
    }

    /// Whether the element type is any number.
    fn is_numeric(&self) -> bool {
        self.type_id().is_numeric()
    }

    /// Whether the element type is a UTF-8 string.
    fn is_utf8(&self) -> bool {
        self.type_id().is_utf8()
    }

    /// Whether the element type is opaque binary.
    fn is_binary(&self) -> bool {
        self.type_id().is_binary()
    }

    /// Whether the element type is a nested / composite type (struct, list, or map).
    fn is_nested(&self) -> bool {
        self.type_id().is_nested()
    }

    /// Whether the element type is a struct.
    fn is_struct(&self) -> bool {
        self.type_id().is_struct()
    }

    /// Whether the element type is a list.
    fn is_list(&self) -> bool {
        self.type_id().is_list()
    }

    /// Whether the element type is a map.
    fn is_map(&self) -> bool {
        self.type_id().is_map()
    }
}
