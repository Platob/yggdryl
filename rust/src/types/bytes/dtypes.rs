//! Validated binary datatype construction.

use crate::types::validate_non_negative;
use crate::{DataType, Result};

impl DataType {
    /// Creates a fixed-size binary type after validating its width.
    pub fn fixed_size_binary(width: i32) -> Result<Self> {
        validate_non_negative("FixedSizeBinary", "width", width)?;
        Ok(Self::FixedSizeBinary(width))
    }
}
