//! [`TypedField<DT, T>`] — the data-type-typed extension of [`Field`].

use yggdryl_dtype::DataType;

use crate::Field;

/// A [`Field`] that exposes its concrete data type `DT` and the native value type `T`
/// its values take.
///
/// Where the base [`Field`] surfaces only the Arrow data type, `TypedField` returns the
/// concrete [`yggdryl_dtype`] type via [`data_type`](TypedField::data_type), and its `T`
/// parameter names the Rust value type the [`scalar`](https://docs.rs/yggdryl-scalar)
/// layer pairs with the field. Carrying the two generic parameters (`DT: DataType` and
/// the native `T`), it is **Rust-only**, like `TypedConverter<S, T>` in the core; the
/// bindings expose the concrete fields (which fix `DT` and `T`) and the byte-level
/// [`Field`] surface.
///
/// ```
/// use yggdryl_dtype::{DataType, I64Type};
/// use yggdryl_field::{I64Field, TypedField};
///
/// let field = I64Field::new("id", true);
/// let dt: I64Type = field.data_type();
/// assert_eq!(dt.name(), "int64");
/// ```
pub trait TypedField<DT: DataType, T>: Field {
    /// The field's concrete data type.
    fn data_type(&self) -> DT;
}
