//! [`ByteType`] — the zero-sized descriptor of a variable-length byte type (`Utf8DataType =
//! ByteType<Utf8>`, `BinaryDataType = ByteType<Binary>`).

use core::marker::PhantomData;

use super::VarElement;
use crate::io::{DataType, DataTypeId};

/// The width of one offset in the variable-length layout — an `i32` (Arrow's 32-bit offsets).
pub(crate) const OFFSET_WIDTH: usize = core::mem::size_of::<i32>();

/// The **variable-length** descriptor sub-trait — the sibling of
/// [`FixedDataType`](crate::io::fixed::FixedDataType) for types whose values are not a fixed
/// byte width (strings, binary). A concrete var descriptor reports `is_fixed_width() == false`
/// (its [`byte_width`](DataType::byte_width) is the width of one *offset*); this trait carries
/// the shared var-descriptor helpers.
pub trait VarDataType: DataType {
    /// Whether values are length-prefixed (the common variable-length layout). Defaults to
    /// `true`; a future fixed-size-list-like var type could override it.
    fn is_length_prefixed(&self) -> bool {
        true
    }
}

/// The typed descriptor of a variable-length byte type `E` — the concrete implementor of
/// [`DataType`] and [`VarDataType`]. `Utf8DataType = ByteType<Utf8>`.
///
/// ```
/// use yggdryl_core::io::var::{ByteType, Utf8};
/// use yggdryl_core::io::DataType;
///
/// let dt = <ByteType<Utf8>>::new();
/// assert_eq!(dt.name(), "utf8");
/// assert!(dt.is_utf8() && dt.is_variable_length() && !dt.is_fixed_width());
/// ```
pub struct ByteType<E: VarElement>(PhantomData<E>);

impl<E: VarElement> ByteType<E> {
    /// The type name as a compile-time constant.
    pub const NAME: &'static str = E::NAME;

    /// The (only) value of this zero-sized descriptor.
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<E: VarElement> DataType for ByteType<E> {
    fn name(&self) -> &'static str {
        E::NAME
    }

    fn byte_width(&self) -> usize {
        OFFSET_WIDTH // the fixed portion of a variable-length value is its 32-bit offset
    }

    fn type_id(&self) -> DataTypeId {
        E::TYPE_ID
    }
    // `to_arrow` is the centralized `DataType` default.
}

impl<E: VarElement> VarDataType for ByteType<E> {}

impl<E: VarElement> Default for ByteType<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: VarElement> Clone for ByteType<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: VarElement> Copy for ByteType<E> {}

impl<E: VarElement> PartialEq for ByteType<E> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<E: VarElement> Eq for ByteType<E> {}

impl<E: VarElement> core::hash::Hash for ByteType<E> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        E::NAME.hash(state);
    }
}

impl<E: VarElement> core::fmt::Debug for ByteType<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "ByteType<{}>", E::NAME)
    }
}
