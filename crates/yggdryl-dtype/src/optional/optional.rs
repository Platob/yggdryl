//! The [`Optional`] base trait: the untyped surface of an optional data type.

use crate::{Logical, Union, UnionType};

/// The untyped surface every optional data type carries: a logical value-or-null
/// type over a [`UnionType`] storage, exposing its value variant's Arrow field.
///
/// It refines [`Logical<Storage = UnionType>`](Logical) — an optional is *stored* as
/// the sparse two-variant union between [`NullType`](crate::NullType) and the value
/// type. The dynamic [`OptionalType`](crate::OptionalType) implements it over an
/// arbitrary value type; a statically-typed optional also implements the typed
/// [`TypedOptional`](crate::TypedOptional) (via
/// [`TypedOptionalType<D>`](crate::TypedOptionalType)), which adds the concrete
/// value-type accessor and the byte codec.
///
/// ```
/// use yggdryl_dtype::{DataType, Int64Type, Logical, Optional, OptionalType};
///
/// let optional = OptionalType::new(&Int64Type);
/// assert_eq!(optional.value_field().name(), "int64");
/// assert_eq!(optional.storage().name(), "union"); // from Logical
/// ```
pub trait Optional: Logical<Storage = UnionType> {
    /// The Arrow field of the optional's value variant — the
    /// [`VALUE_TYPE_ID`](UnionType::VALUE_TYPE_ID) child of its
    /// [`storage`](Logical::storage) union.
    fn value_field(&self) -> arrow_schema::FieldRef {
        self.storage()
            .fields()
            .iter()
            .find(|(id, _)| *id == UnionType::VALUE_TYPE_ID)
            .map(|(_, field)| field.clone())
            .expect("an optional union has a value variant")
    }
}
