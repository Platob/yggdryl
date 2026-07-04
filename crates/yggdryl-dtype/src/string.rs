//! The [`StringType`] data type.

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
/// (`yggdryl_scalar::StringScalar`) holds the value as a core
/// [`StringBuffer`](yggdryl_core::StringBuffer), plugging the string into the
/// positioned-IO layer with a typed `char` view.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, DataType, DataTypeId, Logical, StringType, TypedDataType};
///
/// assert_eq!(StringType.name(), "utf8");
/// assert_eq!(StringType.arrow_format(), "u");
/// assert_eq!((StringType.byte_width(), StringType.bit_width()), (None, None));
/// assert_eq!(StringType::ID, DataTypeId::Utf8);
/// assert_eq!(StringType.storage().name(), "binary"); // a string is stored as binary bytes
///
/// // The byte codec is UTF-8 and validates on the way back.
/// let bytes = StringType.native_to_bytes(&"héllo".to_string());
/// assert_eq!(StringType.native_from_bytes(&bytes).unwrap(), "héllo");
/// assert!(StringType.native_from_bytes(&[0xFF]).is_err()); // not valid UTF-8
/// assert_eq!(StringType.default_value(), String::new());
///
/// // from_arrow is the exact inverse of to_arrow (Arrow's Utf8).
/// assert_eq!(StringType.to_arrow(), arrow_schema::DataType::Utf8);
/// assert_eq!(StringType::from_arrow(&StringType.to_arrow()).unwrap(), StringType);
/// assert!(StringType::from_arrow(&arrow_schema::DataType::Binary).is_err());
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct StringType;

impl StringType {
    /// This type's [`DataTypeId`](crate::DataTypeId).
    pub const ID: crate::DataTypeId = crate::DataTypeId::Utf8;
}

impl DataType for StringType {
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
                expected: "StringType".to_string(),
                got: other.to_string(),
            }),
        }
    }
}

impl Logical for StringType {
    type Storage = BinaryType;

    fn storage(&self) -> &BinaryType {
        // `BinaryType` is a zero-size unit, so one shared static backs every borrow.
        static STORAGE: BinaryType = BinaryType;
        &STORAGE
    }
}

impl TypedDataType<String> for StringType {
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

impl TypedLogical<String> for StringType {}
