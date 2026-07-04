//! The [`Utf8Type`] data type.

use crate::{BinaryType, DataError, DataType, Logical, TypedDataType, TypedLogical};

/// The Apache Arrow `utf8` data type: a variable-length UTF-8 string (native
/// `String`, 32-bit offsets).
///
/// It is a **logical** type over [`BinaryType`] storage — a string *is* a byte
/// sequence, reinterpreted as UTF-8 text — so [`storage`](Logical::storage) hands back
/// the `binary` physical type, while [`to_arrow`](DataType::to_arrow) is Arrow's
/// distinct `Utf8`. Its byte codec is UTF-8: [`native_to_bytes`](TypedDataType::native_to_bytes)
/// takes the string's bytes, and [`native_from_bytes`](TypedDataType::native_from_bytes)
/// **validates** them (non-UTF-8 bytes are a
/// [`DataError::Io`]-wrapped [`InvalidUtf8`](yggdryl_core::IOError::InvalidUtf8)),
/// unlike `binary`'s identity codec. Its scalar
/// (`yggdryl_scalar::Utf8Scalar`) holds the value as a core
/// [`Utf8Buffer`](yggdryl_core::Utf8Buffer), plugging the string into the
/// positioned-IO layer with a typed `char` view.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataType, DataTypeId, Logical, Utf8Type, TypedDataType};
///
/// assert_eq!(Utf8Type.name(), "utf8");
/// assert_eq!(Utf8Type.arrow_format(), "u");
/// assert_eq!((Utf8Type.byte_width(), Utf8Type.bit_width()), (None, None));
/// assert_eq!(Utf8Type::ID, DataTypeId::Utf8);
/// assert_eq!(Utf8Type.storage().name(), "binary"); // a string is stored as binary bytes
///
/// // The byte codec is UTF-8 and validates on the way back.
/// let bytes = Utf8Type.native_to_bytes(&"héllo".to_string());
/// assert_eq!(Utf8Type.native_from_bytes(&bytes).unwrap(), "héllo");
/// assert!(Utf8Type.native_from_bytes(&[0xFF]).is_err()); // not valid UTF-8
/// assert_eq!(Utf8Type.default_value(), String::new());
///
/// // from_arrow is the exact inverse of to_arrow (Arrow's Utf8).
/// assert_eq!(Utf8Type.to_arrow(), arrow_schema::DataType::Utf8);
/// assert_eq!(Utf8Type::from_arrow(&Utf8Type.to_arrow()).unwrap(), Utf8Type);
/// assert!(Utf8Type::from_arrow(&arrow_schema::DataType::Binary).is_err());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Utf8Type;

impl Utf8Type {
    /// This type's [`DataTypeId`](crate::DataTypeId).
    pub const ID: crate::DataTypeId = crate::DataTypeId::Utf8;
}

impl DataType for Utf8Type {
    fn name(&self) -> &str {
        "utf8"
    }

    fn arrow_format(&self) -> String {
        "u".to_string()
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Utf8
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        match data_type {
            arrow_schema::DataType::Utf8 => Ok(Self),
            other => Err(DataError::IncompatibleArrowType {
                expected: "Utf8Type".to_string(),
                got: other.to_string(),
            }),
        }
    }
}

impl Logical for Utf8Type {
    type Storage = BinaryType;

    fn storage(&self) -> &BinaryType {
        // `BinaryType` is a zero-size unit, so one shared static backs every borrow.
        static STORAGE: BinaryType = BinaryType;
        &STORAGE
    }
}

impl TypedDataType<String> for Utf8Type {
    fn native_to_bytes(&self, value: &String) -> Vec<u8> {
        value.as_bytes().to_vec()
    }

    fn native_from_bytes(&self, bytes: &[u8]) -> Result<String, DataError> {
        // Unlike `binary`, the string codec validates: the bytes must be UTF-8.
        String::from_utf8(bytes.to_vec()).map_err(|error| {
            DataError::from(yggdryl_core::IOError::InvalidUtf8 {
                offset: error.utf8_error().valid_up_to(),
            })
        })
    }

    fn default_value(&self) -> String {
        String::new()
    }
}

impl TypedLogical<String> for Utf8Type {}
