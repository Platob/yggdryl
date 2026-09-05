//! Binary typed scalar aliases.

use std::sync::{Arc, OnceLock};

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

/// Borrowing access shared by every opaque-byte representation.
pub trait BytesValue: crate::ScalarValue {
    /// Borrow the payload bytes.
    fn as_bytes(&self) -> &[u8];
}

impl From<Vec<u8>> for Scalar {
    fn from(value: Vec<u8>) -> Self {
        if value.is_empty() {
            static EMPTY: OnceLock<Arc<[u8]>> = OnceLock::new();
            return Self::Bytes(Arc::clone(EMPTY.get_or_init(|| Arc::from([]))));
        }
        Self::Bytes(value.into())
    }
}

impl From<&[u8]> for Scalar {
    fn from(value: &[u8]) -> Self {
        // Borrowed bytes are the shape every reader hands out, so taking them
        // directly saves the caller a `to_vec` whose only purpose was this call.
        if value.is_empty() {
            return Self::from(Vec::<u8>::new());
        }
        Self::Bytes(Arc::from(value))
    }
}

impl<const N: usize> From<&[u8; N]> for Scalar {
    fn from(value: &[u8; N]) -> Self {
        Self::from(value.as_slice())
    }
}

impl From<Arc<[u8]>> for Scalar {
    fn from(value: Arc<[u8]>) -> Self {
        if value.is_empty() {
            return Self::from(Vec::<u8>::new());
        }
        Self::Bytes(value)
    }
}

define_scalar_type!(
    BinaryScalar,
    super::Binary,
    "binary",
    crate::DataType::Binary
);
define_scalar_type!(
    FixedSizeBinaryScalar,
    super::FixedSizeBinary,
    "fixed_size_binary"
);
define_scalar_type!(
    LargeBinaryScalar,
    super::LargeBinary,
    "large_binary",
    crate::DataType::LargeBinary
);
define_scalar_type!(
    BinaryViewScalar,
    super::BinaryView,
    "binary_view",
    crate::DataType::BinaryView
);
