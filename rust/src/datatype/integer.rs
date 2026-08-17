//! Integer predicates used by constructors and run-end validation.

use super::DataType;

impl DataType {
    /// Returns whether this is a signed or unsigned integer type.
    pub const fn is_integer(&self) -> bool {
        matches!(
            self,
            Self::Int8
                | Self::Int16
                | Self::Int32
                | Self::Int64
                | Self::UInt8
                | Self::UInt16
                | Self::UInt32
                | Self::UInt64
        )
    }

    pub(super) const fn is_run_ends_type(&self) -> bool {
        matches!(self, Self::Int16 | Self::Int32 | Self::Int64)
    }
}
