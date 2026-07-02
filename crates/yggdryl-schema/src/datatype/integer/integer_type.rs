//! The subtrait every integer data type satisfies.

use crate::{
    Int16Type, Int32Type, Int64Type, Int8Type, NumericType, UInt16Type, UInt32Type, UInt64Type,
    UInt8Type,
};

/// A [`NumericType`] whose values are whole numbers, signed or not.
///
/// ```
/// use yggdryl_schema::{Int8Type, IntegerType, UInt64Type};
///
/// assert!(Int8Type::SIGNED);
/// assert!(!UInt64Type::SIGNED);
/// ```
pub trait IntegerType: NumericType {
    /// Whether the values carry a sign.
    const SIGNED: bool;
}

impl IntegerType for Int8Type {
    const SIGNED: bool = true;
}
impl IntegerType for Int16Type {
    const SIGNED: bool = true;
}
impl IntegerType for Int32Type {
    const SIGNED: bool = true;
}
impl IntegerType for Int64Type {
    const SIGNED: bool = true;
}
impl IntegerType for UInt8Type {
    const SIGNED: bool = false;
}
impl IntegerType for UInt16Type {
    const SIGNED: bool = false;
}
impl IntegerType for UInt32Type {
    const SIGNED: bool = false;
}
impl IntegerType for UInt64Type {
    const SIGNED: bool = false;
}
