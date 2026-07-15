//! The **fixed-width data-type descriptor**: the [`FixedDataType`] sub-trait (of the root
//! [`DataType`](crate::io::DataType) / [`TypedDataType`](crate::io::TypedDataType)) and the
//! concrete zero-sized [`PrimitiveType`] that implements them for a [`NativeType`].

use core::marker::PhantomData;

use super::NativeType;
use crate::io::{DataType, DataTypeId, TypedDataType};

/// The **fixed-width** descriptor sub-trait — for a [`TypedDataType`] whose element is a
/// [`NativeType`]. It provides the shared width/name/Arrow logic as **default methods** (the
/// "pre-implementations"), so a concrete fixed descriptor supplies nothing extra.
pub trait FixedDataType: TypedDataType
where
    Self::Native: NativeType,
{
    /// The element type's name — mutualized from the [`NativeType`].
    fn native_name(&self) -> &'static str {
        <Self::Native as NativeType>::NAME
    }

    /// The element type's fixed byte width — mutualized from the [`NativeType`].
    fn native_width(&self) -> usize {
        <Self::Native as NativeType>::WIDTH
    }
}

/// The **typed, zero-sized** descriptor of a fixed-width primitive `T` (`U8DataType =
/// PrimitiveType<u8>`) — the concrete implementor of [`DataType`] / [`TypedDataType`] /
/// [`FixedDataType`]. It carries `T`'s name and width at the type level.
///
/// ```
/// use yggdryl_core::io::DataType;
/// use yggdryl_core::io::fixed::PrimitiveType;
///
/// let dt = <PrimitiveType<i32>>::new();
/// assert_eq!(dt.name(), "i32");
/// assert_eq!(dt.byte_width(), 4);
/// ```
pub struct PrimitiveType<T: NativeType>(PhantomData<T>);

impl<T: NativeType> PrimitiveType<T> {
    /// The type name as a **compile-time constant** — the optimized accessor, no method call
    /// or vtable (`PrimitiveType::<i32>::NAME == "i32"`).
    pub const NAME: &'static str = T::NAME;

    /// The fixed byte width as a **compile-time constant** (`PrimitiveType::<i32>::BYTE_WIDTH == 4`).
    pub const BYTE_WIDTH: usize = T::WIDTH;

    /// The (only) value of this zero-sized descriptor.
    pub const fn new() -> Self {
        Self(PhantomData)
    }

    /// The type name — a `const fn` accessor usable in const context (mirrors the erased
    /// [`DataType::name`], but monomorphized and inlinable).
    pub const fn type_name(&self) -> &'static str {
        T::NAME
    }

    /// The fixed byte width — a `const fn` accessor.
    pub const fn width(&self) -> usize {
        T::WIDTH
    }
}

impl<T: NativeType> DataType for PrimitiveType<T> {
    fn name(&self) -> &'static str {
        T::NAME
    }

    fn byte_width(&self) -> usize {
        T::WIDTH
    }

    fn type_id(&self) -> DataTypeId {
        T::TYPE_ID
    }
    // `to_arrow` is the centralized `DataType` default (`type_id().to_arrow(byte_width())`).
}

impl<T: NativeType> TypedDataType for PrimitiveType<T> {
    type Native = T;
}

impl<T: NativeType> FixedDataType for PrimitiveType<T> {}

// Zero-sized: every value is identical, so the value-semantics impls are trivial.
impl<T: NativeType> Default for PrimitiveType<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: NativeType> Clone for PrimitiveType<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: NativeType> Copy for PrimitiveType<T> {}

impl<T: NativeType> PartialEq for PrimitiveType<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T: NativeType> Eq for PrimitiveType<T> {}

impl<T: NativeType> core::hash::Hash for PrimitiveType<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        T::NAME.hash(state);
    }
}

impl<T: NativeType> core::fmt::Debug for PrimitiveType<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "PrimitiveType<{}>", T::NAME)
    }
}

// The Arrow data-type mapping now lives in one place — `DataTypeId::to_arrow` /
// `DataTypeId::from_arrow` — so it is total across the whole broadened type space and the erased
// `Field` and typed fields share it. (The old per-primitive `native_from_arrow` /
// `arrow_from_name` helpers were only correct for the base 10 primitives.)
