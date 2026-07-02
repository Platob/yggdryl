//! The integer identifier of a data type's constructor.

use core::fmt;

use crate::DataTypeError;

/// The largest identifier currently assigned; update when appending a
/// variant.
const MAX_TYPE_ID: u8 = DataTypeId::Map as u8;

/// The integer identifier of a [`DataType`](crate::DataType) constructor,
/// shared by every parameterization of that constructor (every
/// [`Decimal128`](crate::Decimal128) is `DataTypeId::Decimal128`, every
/// [`List`](crate::List) is `DataTypeId::List`).
///
/// Discriminants are explicit and append-only: new identifiers are only ever
/// added after the last one, and a published value is never repurposed, so
/// the ids are stable across versions and safe to persist.
///
/// ```
/// use yggdryl_schema::{DataType, DataTypeId, Int8};
///
/// assert_eq!(Int8.type_id(), DataTypeId::Int8);
/// assert_eq!(DataTypeId::from_u8(DataTypeId::Int8.to_u8()), Ok(DataTypeId::Int8));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum DataTypeId {
    /// [`Boolean`](crate::Boolean).
    Boolean = 0,
    /// [`Int8`](crate::Int8).
    Int8 = 1,
    /// [`Int16`](crate::Int16).
    Int16 = 2,
    /// [`Int32`](crate::Int32).
    Int32 = 3,
    /// [`Int64`](crate::Int64).
    Int64 = 4,
    /// [`UInt8`](crate::UInt8).
    UInt8 = 5,
    /// [`UInt16`](crate::UInt16).
    UInt16 = 6,
    /// [`UInt32`](crate::UInt32).
    UInt32 = 7,
    /// [`UInt64`](crate::UInt64).
    UInt64 = 8,
    /// [`Float32`](crate::Float32).
    Float32 = 9,
    /// [`Float64`](crate::Float64).
    Float64 = 10,
    /// [`Decimal128`](crate::Decimal128).
    Decimal128 = 11,
    /// [`Decimal256`](crate::Decimal256).
    Decimal256 = 12,
    /// [`Utf8`](crate::Utf8).
    Utf8 = 13,
    /// [`LargeUtf8`](crate::LargeUtf8).
    LargeUtf8 = 14,
    /// [`Binary`](crate::Binary).
    Binary = 15,
    /// [`LargeBinary`](crate::LargeBinary).
    LargeBinary = 16,
    /// [`FixedSizeBinary`](crate::FixedSizeBinary).
    FixedSizeBinary = 17,
    /// [`Date32`](crate::Date32).
    Date32 = 18,
    /// [`Date64`](crate::Date64).
    Date64 = 19,
    /// [`Time32`](crate::Time32).
    Time32 = 20,
    /// [`Time64`](crate::Time64).
    Time64 = 21,
    /// [`Timestamp`](crate::Timestamp).
    Timestamp = 22,
    /// [`Duration`](crate::Duration).
    Duration = 23,
    /// [`List`](crate::List).
    List = 24,
    /// [`LargeList`](crate::LargeList).
    LargeList = 25,
    /// [`Struct`](crate::Struct).
    Struct = 26,
    /// [`Map`](crate::Map).
    Map = 27,
}

impl DataTypeId {
    /// The identifier's stable integer value.
    pub fn to_u8(&self) -> u8 {
        *self as u8
    }

    /// Builds the identifier from its stable integer value, rejecting
    /// unassigned values.
    ///
    /// ```
    /// use yggdryl_schema::{DataTypeError, DataTypeId};
    ///
    /// assert_eq!(DataTypeId::from_u8(0), Ok(DataTypeId::Boolean));
    /// assert!(matches!(
    ///     DataTypeId::from_u8(200),
    ///     Err(DataTypeError::UnknownTypeId { id: 200, .. })
    /// ));
    /// ```
    pub fn from_u8(id: u8) -> Result<Self, DataTypeError> {
        match id {
            0 => Ok(Self::Boolean),
            1 => Ok(Self::Int8),
            2 => Ok(Self::Int16),
            3 => Ok(Self::Int32),
            4 => Ok(Self::Int64),
            5 => Ok(Self::UInt8),
            6 => Ok(Self::UInt16),
            7 => Ok(Self::UInt32),
            8 => Ok(Self::UInt64),
            9 => Ok(Self::Float32),
            10 => Ok(Self::Float64),
            11 => Ok(Self::Decimal128),
            12 => Ok(Self::Decimal256),
            13 => Ok(Self::Utf8),
            14 => Ok(Self::LargeUtf8),
            15 => Ok(Self::Binary),
            16 => Ok(Self::LargeBinary),
            17 => Ok(Self::FixedSizeBinary),
            18 => Ok(Self::Date32),
            19 => Ok(Self::Date64),
            20 => Ok(Self::Time32),
            21 => Ok(Self::Time64),
            22 => Ok(Self::Timestamp),
            23 => Ok(Self::Duration),
            24 => Ok(Self::List),
            25 => Ok(Self::LargeList),
            26 => Ok(Self::Struct),
            27 => Ok(Self::Map),
            _ => Err(DataTypeError::UnknownTypeId {
                id,
                max: MAX_TYPE_ID,
            }),
        }
    }

    /// Serializes the identifier as its one-byte value.
    pub fn to_bytes(&self) -> Vec<u8> {
        vec![self.to_u8()]
    }

    /// Deserializes the identifier from the encoding produced by
    /// [`to_bytes`](DataTypeId::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        match bytes {
            [id] => Self::from_u8(*id),
            _ => Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
            }),
        }
    }
}

impl fmt::Display for DataTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Boolean => "boolean",
            Self::Int8 => "int8",
            Self::Int16 => "int16",
            Self::Int32 => "int32",
            Self::Int64 => "int64",
            Self::UInt8 => "uint8",
            Self::UInt16 => "uint16",
            Self::UInt32 => "uint32",
            Self::UInt64 => "uint64",
            Self::Float32 => "float32",
            Self::Float64 => "float64",
            Self::Decimal128 => "decimal128",
            Self::Decimal256 => "decimal256",
            Self::Utf8 => "utf8",
            Self::LargeUtf8 => "large_utf8",
            Self::Binary => "binary",
            Self::LargeBinary => "large_binary",
            Self::FixedSizeBinary => "fixed_size_binary",
            Self::Date32 => "date32",
            Self::Date64 => "date64",
            Self::Time32 => "time32",
            Self::Time64 => "time64",
            Self::Timestamp => "timestamp",
            Self::Duration => "duration",
            Self::List => "list",
            Self::LargeList => "large_list",
            Self::Struct => "struct",
            Self::Map => "map",
        })
    }
}
