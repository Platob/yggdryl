//! Binary values and typed scalar aliases.

use std::fmt;
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;

/// Borrowing access shared by every opaque-byte representation.
pub trait BytesValue: crate::ScalarValue {
    /// Borrow the payload bytes.
    fn as_bytes(&self) -> &[u8];
}

macro_rules! bytes_leaf {
    ($name:ident) => {
        #[doc = concat!("One `", stringify!($name), "` byte value.")]
        #[repr(transparent)]
        #[derive(
            Clone, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
        )]
        #[serde(transparent)]
        pub struct $name(Arc<[u8]>);

        impl $name {
            /// Construct this byte representation from shared storage.
            pub fn new(value: impl Into<Arc<[u8]>>) -> Self {
                Self(value.into())
            }

            /// Borrow the payload bytes.
            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_ref()
            }

            /// Consume this value and return its shared storage.
            pub fn into_inner(self) -> Arc<[u8]> {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.as_bytes() {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl From<Vec<u8>> for $name {
            fn from(value: Vec<u8>) -> Self {
                Self::new(Arc::<[u8]>::from(value))
            }
        }

        impl From<Arc<[u8]>> for $name {
            fn from(value: Arc<[u8]>) -> Self {
                Self::new(value)
            }
        }
    };
}

bytes_leaf!(Binary);
bytes_leaf!(FixedSizeBinary);
bytes_leaf!(LargeBinary);
bytes_leaf!(BinaryView);

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
    super::BinaryType,
    "binary",
    crate::DataType::Binary
);
define_scalar_type!(
    FixedSizeBinaryScalar,
    super::FixedSizeBinaryType,
    "fixed_size_binary"
);
define_scalar_type!(
    LargeBinaryScalar,
    super::LargeBinaryType,
    "large_binary",
    crate::DataType::LargeBinary
);
define_scalar_type!(
    BinaryViewScalar,
    super::BinaryViewType,
    "binary_view",
    crate::DataType::BinaryView
);
