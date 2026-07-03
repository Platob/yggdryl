//! The [`Binary`] data type.

use crate::{DataError, DataType, RawDataType};

/// The Apache Arrow `binary` data type: a variable-length sequence of bytes
/// (native `Vec<u8>`, 32-bit offsets).
///
/// It is Arrow's variable-size binary layout — childless like a
/// [`Primitive`](crate::Primitive) but with no fixed width (`byte_width` is
/// `None`), so it is not a `Primitive` in this model's fixed-width sense. The
/// typed byte codec is the identity: every byte slice is a valid `binary` value,
/// so `native_from_bytes` never errors. Its scalar
/// ([`BinaryScalar`](crate::BinaryScalar)) holds the bytes as a core
/// [`ByteBuffer`](yggdryl_core::ByteBuffer), plugging the value straight into the
/// positioned-IO layer.
///
/// ```
/// use yggdryl_data::{arrow_schema, Binary, DataType, DataTypeId, RawDataType};
///
/// assert_eq!(Binary.name(), "binary");
/// assert_eq!(Binary.arrow_format(), "z");
/// assert_eq!((Binary.byte_width(), Binary.bit_width()), (None, None));
/// assert_eq!(Binary::ID, DataTypeId::Binary);
///
/// // The byte codec is the identity: any bytes are a valid binary value.
/// let bytes = Binary.native_to_bytes(&vec![1, 2, 3]);
/// assert_eq!(Binary.native_from_bytes(&bytes).unwrap(), vec![1, 2, 3]);
/// assert_eq!(Binary.default_value(), Vec::<u8>::new());
///
/// // from_arrow is the exact inverse of to_arrow.
/// assert_eq!(Binary.to_arrow(), arrow_schema::DataType::Binary);
/// assert_eq!(Binary::from_arrow(&Binary.to_arrow()).unwrap(), Binary);
/// assert!(Binary::from_arrow(&arrow_schema::DataType::Int64).is_err());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Binary;

impl Binary {
    /// This type's [`DataTypeId`](crate::DataTypeId).
    pub const ID: crate::DataTypeId = crate::DataTypeId::Binary;
}

impl RawDataType for Binary {
    fn name(&self) -> &str {
        "binary"
    }

    fn arrow_format(&self) -> String {
        "z".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Binary
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        match data_type {
            arrow_schema::DataType::Binary => Ok(Self),
            other => Err(DataError::IncompatibleArrowType {
                expected: "Binary".to_string(),
                got: other.to_string(),
            }),
        }
    }
}

impl DataType<Vec<u8>> for Binary {
    type Scalar = super::BinaryScalar;

    fn native_to_bytes(&self, value: &Vec<u8>) -> Vec<u8> {
        value.clone()
    }

    fn native_from_bytes(&self, bytes: &[u8]) -> Result<Vec<u8>, DataError> {
        Ok(bytes.to_vec())
    }

    fn default_value(&self) -> Vec<u8> {
        Vec::new()
    }

    /// The default binary scalar: the empty byte sequence.
    fn default_scalar(&self) -> Self::Scalar {
        super::BinaryScalar::new(Vec::new())
    }
}
