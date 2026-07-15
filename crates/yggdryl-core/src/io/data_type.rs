//! The **data-type descriptor** root traits: the erased [`DataType`] and the generic typed
//! [`TypedDataType`]. These are the family-agnostic contracts every typed layer extends — the
//! fixed primitives via `FixedDataType` and the variable types via `VarDataType` — so they live
//! at the `io` root rather than inside any one family.

use super::{DataTypeCategory, DataTypeId};

/// A runtime **type descriptor** — the object-safe, erased root every data type exposes: its
/// name, its byte width, its [`type_id`](DataType::type_id), and its Arrow mapping. Held behind
/// `&dyn DataType` it lets a [`FieldType`](super::FieldType) or a schema treat every column's
/// type uniformly, and the category predicates ([`is_integer`](DataType::is_integer) …) let a
/// caller drill down fast — each is a couple of integer comparisons on the
/// [`DataTypeId`](DataTypeId), never a `match` on the concrete type.
pub trait DataType {
    /// The stable, lower-case type name, e.g. `"u8"`, `"i32"`, `"utf8"`.
    fn name(&self) -> &'static str;

    /// The fixed width of one value in bytes. For a variable-length type this is the width of
    /// one *offset* (its fixed portion); [`is_fixed_width`](DataType::is_fixed_width) tells the
    /// two apart.
    fn byte_width(&self) -> usize;

    /// The type's [`DataTypeId`] — the **single source of truth** the identity and every `is_*`
    /// predicate reduces to.
    fn type_id(&self) -> DataTypeId;

    /// The coarse [`DataTypeCategory`] bucket — derived from the [`type_id`](DataType::type_id).
    fn category(&self) -> DataTypeCategory {
        self.type_id().category()
    }

    /// Whether the type has a fixed byte width.
    fn is_fixed_width(&self) -> bool {
        self.type_id().is_fixed_width()
    }

    /// Whether the type is variable-length.
    fn is_variable_length(&self) -> bool {
        self.type_id().is_variable_length()
    }

    /// Whether the type is any integer.
    fn is_integer(&self) -> bool {
        self.type_id().is_integer()
    }

    /// Whether the type is an unsigned integer.
    fn is_unsigned_integer(&self) -> bool {
        self.type_id().is_unsigned_integer()
    }

    /// Whether the type is a signed integer.
    fn is_signed_integer(&self) -> bool {
        self.type_id().is_signed_integer()
    }

    /// Whether the type is a signed number (signed integer or float).
    fn is_signed(&self) -> bool {
        self.type_id().is_signed()
    }

    /// Whether the type is a float.
    fn is_floating(&self) -> bool {
        self.type_id().is_floating()
    }

    /// Whether the type is a scaled decimal.
    fn is_decimal(&self) -> bool {
        self.type_id().is_decimal()
    }

    /// Whether the type is a temporal value (date, time, timestamp, duration).
    fn is_temporal(&self) -> bool {
        self.type_id().is_temporal()
    }

    /// Whether the type is any number.
    fn is_numeric(&self) -> bool {
        self.type_id().is_numeric()
    }

    /// Whether the type is a UTF-8 string.
    fn is_utf8(&self) -> bool {
        self.type_id().is_utf8()
    }

    /// Whether the type is opaque binary.
    fn is_binary(&self) -> bool {
        self.type_id().is_binary()
    }

    /// Whether the type is a nested / composite type (struct, list, or map).
    fn is_nested(&self) -> bool {
        self.type_id().is_nested()
    }

    /// Whether the type is a struct.
    fn is_struct(&self) -> bool {
        self.type_id().is_struct()
    }

    /// Whether the type is a list.
    fn is_list(&self) -> bool {
        self.type_id().is_list()
    }

    /// Whether the type is a map.
    fn is_map(&self) -> bool {
        self.type_id().is_map()
    }

    /// The matching (or closest) Arrow [`DataType`](arrow_schema::DataType) (feature `arrow`) —
    /// a **total** default derived from the [`type_id`](DataType::type_id) and byte width via the
    /// centralized [`DataTypeId::to_arrow`](DataTypeId::to_arrow) mapping, so every descriptor
    /// (fixed, variable, fixed-size) shares one definition.
    #[cfg(feature = "arrow")]
    fn to_arrow(&self) -> arrow_schema::DataType {
        self.type_id().to_arrow(self.byte_width())
    }
}

/// The **generic typed** descriptor root: a [`DataType`] that names its logical element type
/// as an associated type. Both the fixed and variable families extend it, so code can be
/// generic over "a descriptor whose element is `T`" without caring about the shape.
pub trait TypedDataType: DataType {
    /// The logical element type this descriptor describes.
    type Native;
}
