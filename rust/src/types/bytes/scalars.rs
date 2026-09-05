//! Binary values and typed scalar aliases.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};

use crate::types::Scalar;
use crate::types::typed::define_scalar_type;
use crate::{DataType, DataTypeId, DataTypeKind, Result, ScalarFamily, ScalarValue};

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

/// One exact opaque-byte storage representation.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub enum Bytes {
    /// Binary with 32-bit offsets.
    Binary(Binary),
    /// Fixed-width binary.
    FixedSizeBinary(FixedSizeBinary),
    /// Binary with 64-bit offsets.
    LargeBinary(LargeBinary),
    /// Binary view storage.
    BinaryView(BinaryView),
}

impl Bytes {
    /// Borrow the bytes independently of their storage layout.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Binary(value) => value.as_bytes(),
            Self::FixedSizeBinary(value) => value.as_bytes(),
            Self::LargeBinary(value) => value.as_bytes(),
            Self::BinaryView(value) => value.as_bytes(),
        }
    }
}

impl fmt::Display for Bytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.as_bytes() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl PartialEq for Bytes {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Bytes {}

impl PartialOrd for Bytes {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Bytes {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl Hash for Bytes {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

const _: () = assert!(std::mem::size_of::<Bytes>() == 24);

macro_rules! bytes_value {
    ($leaf:ident, $marker:ty, $variant:ident, $id:ident, $dtype:ident) => {
        impl ScalarValue for $leaf {
            type Family = Bytes;
            type Type = $marker;

            const ID: DataTypeId = DataTypeId::$id;
            const KIND: DataTypeKind = DataTypeKind::Bytes;

            fn dtype(&self) -> Result<DataType> {
                Ok(DataType::$dtype)
            }

            fn into_family(self) -> Self::Family {
                Bytes::$variant(self)
            }

            fn from_family(family: &Self::Family) -> Option<&Self> {
                match family {
                    Bytes::$variant(value) => Some(value),
                    _ => None,
                }
            }

            fn into_scalar(self) -> Scalar {
                Scalar::Bytes(Bytes::$variant(self))
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::Bytes(Bytes::$variant(value)) => Some(value),
                    _ => None,
                }
            }
        }

        impl BytesValue for $leaf {
            fn as_bytes(&self) -> &[u8] {
                <$leaf>::as_bytes(self)
            }
        }
    };
}

bytes_value!(Binary, super::BinaryType, Binary, Binary, Binary);
bytes_value!(
    LargeBinary,
    super::LargeBinaryType,
    LargeBinary,
    LargeBinary,
    LargeBinary
);
bytes_value!(
    BinaryView,
    super::BinaryViewType,
    BinaryView,
    BinaryView,
    BinaryView
);

impl ScalarValue for FixedSizeBinary {
    type Family = Bytes;
    type Type = super::FixedSizeBinaryType;

    const ID: DataTypeId = DataTypeId::FixedSizeBinary;
    const KIND: DataTypeKind = DataTypeKind::Bytes;

    fn dtype(&self) -> Result<DataType> {
        self.clone().into_scalar().dtype()
    }

    fn into_family(self) -> Self::Family {
        Bytes::FixedSizeBinary(self)
    }

    fn from_family(family: &Self::Family) -> Option<&Self> {
        match family {
            Bytes::FixedSizeBinary(value) => Some(value),
            _ => None,
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Bytes(Bytes::FixedSizeBinary(self))
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Bytes(Bytes::FixedSizeBinary(value)) => Some(value),
            _ => None,
        }
    }
}

impl BytesValue for FixedSizeBinary {
    fn as_bytes(&self) -> &[u8] {
        Self::as_bytes(self)
    }
}

impl ScalarFamily for Bytes {
    const KIND: DataTypeKind = DataTypeKind::Bytes;

    fn id(&self) -> DataTypeId {
        match self {
            Self::Binary(_) => DataTypeId::Binary,
            Self::FixedSizeBinary(_) => DataTypeId::FixedSizeBinary,
            Self::LargeBinary(_) => DataTypeId::LargeBinary,
            Self::BinaryView(_) => DataTypeId::BinaryView,
        }
    }

    fn dtype(&self) -> Result<DataType> {
        match self {
            Self::Binary(_) => Ok(DataType::Binary),
            Self::FixedSizeBinary(value) => ScalarValue::dtype(value),
            Self::LargeBinary(_) => Ok(DataType::LargeBinary),
            Self::BinaryView(_) => Ok(DataType::BinaryView),
        }
    }

    fn into_scalar(self) -> Scalar {
        Scalar::Bytes(self)
    }

    fn from_scalar(value: &Scalar) -> Option<&Self> {
        match value {
            Scalar::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

impl From<Vec<u8>> for Scalar {
    fn from(value: Vec<u8>) -> Self {
        if value.is_empty() {
            static EMPTY: OnceLock<Arc<[u8]>> = OnceLock::new();
            return Self::Bytes(Bytes::Binary(Binary::new(Arc::clone(
                EMPTY.get_or_init(|| Arc::from([])),
            ))));
        }
        Self::Bytes(Bytes::Binary(Binary::from(value)))
    }
}

impl From<&[u8]> for Scalar {
    fn from(value: &[u8]) -> Self {
        // Borrowed bytes are the shape every reader hands out, so taking them
        // directly saves the caller a `to_vec` whose only purpose was this call.
        if value.is_empty() {
            return Self::from(Vec::<u8>::new());
        }
        Self::Bytes(Bytes::Binary(Binary::new(Arc::<[u8]>::from(value))))
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
        Self::Bytes(Bytes::Binary(Binary::new(value)))
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
