//! The erased data type covering every supported constructor.

use core::fmt;
use std::collections::BTreeMap;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    metadata, AnyTimeUnit, Binary, Boolean, DataType, DataTypeError, DataTypeId, Date32, Date64,
    Decimal128, Decimal256, Duration, FixedSizeBinary, Float32, Float64, Int16, Int32, Int64, Int8,
    LargeBinary, LargeList, LargeUtf8, List, Map, Struct, Time32, Time64, TimeUnit, Timestamp,
    UInt16, UInt32, UInt64, UInt8, Utf8,
};

/// Delegates an expression to the concrete type inside every variant.
macro_rules! delegate {
    ($value:expr, $inner:ident => $body:expr) => {
        match $value {
            Self::Boolean($inner) => $body,
            Self::Int8($inner) => $body,
            Self::Int16($inner) => $body,
            Self::Int32($inner) => $body,
            Self::Int64($inner) => $body,
            Self::UInt8($inner) => $body,
            Self::UInt16($inner) => $body,
            Self::UInt32($inner) => $body,
            Self::UInt64($inner) => $body,
            Self::Float32($inner) => $body,
            Self::Float64($inner) => $body,
            Self::Decimal128($inner) => $body,
            Self::Decimal256($inner) => $body,
            Self::Utf8($inner) => $body,
            Self::LargeUtf8($inner) => $body,
            Self::Binary($inner) => $body,
            Self::LargeBinary($inner) => $body,
            Self::FixedSizeBinary($inner) => $body,
            Self::Date32($inner) => $body,
            Self::Date64($inner) => $body,
            Self::Time32($inner) => $body,
            Self::Time64($inner) => $body,
            Self::Timestamp($inner) => $body,
            Self::Duration($inner) => $body,
            Self::List($inner) => $body,
            Self::LargeList($inner) => $body,
            Self::Struct($inner) => $body,
            Self::Map($inner) => $body,
        }
    };
}

/// Generates the `From<concrete>` conversion for every variant.
macro_rules! from_impls {
    ($($variant:ident: $concrete:ty),+ $(,)?) => {$(
        impl From<$concrete> for AnyDataType {
            fn from(data_type: $concrete) -> Self {
                Self::$variant(data_type)
            }
        }
    )+};
}

/// The erased [`DataType`]: one variant per supported constructor, each
/// wrapping the concrete type, so heterogeneous collections — a
/// [`Struct`]'s fields, a schema, a binding-held type — can hold any data
/// type behind a single `Sized` value.
///
/// `AnyDataType` implements [`DataType`] itself by delegating to the wrapped
/// value; its `from_arrow` is the dispatcher across every supported Arrow
/// type, and its byte encoding is the wrapped type's payload behind a
/// [`DataTypeId`] tag.
///
/// ```
/// use yggdryl_schema::{AnyDataType, DataType, DataTypeId, Int32};
///
/// let any = AnyDataType::from(Int32);
/// assert_eq!(any.type_id(), DataTypeId::Int32);
/// assert_eq!(AnyDataType::from_arrow(&any.to_arrow()), Ok(any));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AnyDataType {
    /// [`Boolean`].
    Boolean(Boolean),
    /// [`Int8`].
    Int8(Int8),
    /// [`Int16`].
    Int16(Int16),
    /// [`Int32`].
    Int32(Int32),
    /// [`Int64`].
    Int64(Int64),
    /// [`UInt8`].
    UInt8(UInt8),
    /// [`UInt16`].
    UInt16(UInt16),
    /// [`UInt32`].
    UInt32(UInt32),
    /// [`UInt64`].
    UInt64(UInt64),
    /// [`Float32`].
    Float32(Float32),
    /// [`Float64`].
    Float64(Float64),
    /// [`Decimal128`].
    Decimal128(Decimal128),
    /// [`Decimal256`].
    Decimal256(Decimal256),
    /// [`Utf8`].
    Utf8(Utf8),
    /// [`LargeUtf8`].
    LargeUtf8(LargeUtf8),
    /// [`Binary`].
    Binary(Binary),
    /// [`LargeBinary`].
    LargeBinary(LargeBinary),
    /// [`FixedSizeBinary`].
    FixedSizeBinary(FixedSizeBinary),
    /// [`Date32`].
    Date32(Date32),
    /// [`Date64`].
    Date64(Date64),
    /// [`Time32`].
    Time32(Time32),
    /// [`Time64`].
    Time64(Time64),
    /// [`Timestamp`] over an erased unit.
    Timestamp(Timestamp<AnyTimeUnit>),
    /// [`Duration`].
    Duration(Duration),
    /// [`List`] over an erased child.
    List(List<AnyDataType>),
    /// [`LargeList`] over an erased child.
    LargeList(LargeList<AnyDataType>),
    /// [`Struct`].
    Struct(Struct),
    /// [`Map`].
    Map(Map),
}

impl DataType for AnyDataType {
    fn type_id(&self) -> DataTypeId {
        delegate!(self, inner => inner.type_id())
    }

    fn to_arrow(&self) -> ArrowDataType {
        delegate!(self, inner => inner.to_arrow())
    }

    fn arrow_metadata(&self) -> BTreeMap<String, String> {
        delegate!(self, inner => inner.arrow_metadata())
    }

    fn from_arrow(data_type: &ArrowDataType) -> Result<Self, DataTypeError> {
        match data_type {
            ArrowDataType::Boolean => Boolean::from_arrow(data_type).map(Self::Boolean),
            ArrowDataType::Int8 => Int8::from_arrow(data_type).map(Self::Int8),
            ArrowDataType::Int16 => Int16::from_arrow(data_type).map(Self::Int16),
            ArrowDataType::Int32 => Int32::from_arrow(data_type).map(Self::Int32),
            ArrowDataType::Int64 => Int64::from_arrow(data_type).map(Self::Int64),
            ArrowDataType::UInt8 => UInt8::from_arrow(data_type).map(Self::UInt8),
            ArrowDataType::UInt16 => UInt16::from_arrow(data_type).map(Self::UInt16),
            ArrowDataType::UInt32 => UInt32::from_arrow(data_type).map(Self::UInt32),
            ArrowDataType::UInt64 => UInt64::from_arrow(data_type).map(Self::UInt64),
            ArrowDataType::Float32 => Float32::from_arrow(data_type).map(Self::Float32),
            ArrowDataType::Float64 => Float64::from_arrow(data_type).map(Self::Float64),
            ArrowDataType::Decimal128(..) => {
                Decimal128::from_arrow(data_type).map(Self::Decimal128)
            }
            ArrowDataType::Decimal256(..) => {
                Decimal256::from_arrow(data_type).map(Self::Decimal256)
            }
            ArrowDataType::Utf8 => Utf8::from_arrow(data_type).map(Self::Utf8),
            ArrowDataType::LargeUtf8 => LargeUtf8::from_arrow(data_type).map(Self::LargeUtf8),
            ArrowDataType::Binary => Binary::from_arrow(data_type).map(Self::Binary),
            ArrowDataType::LargeBinary => LargeBinary::from_arrow(data_type).map(Self::LargeBinary),
            ArrowDataType::FixedSizeBinary(_) => {
                FixedSizeBinary::from_arrow(data_type).map(Self::FixedSizeBinary)
            }
            ArrowDataType::Date32 => Date32::from_arrow(data_type).map(Self::Date32),
            ArrowDataType::Date64 => Date64::from_arrow(data_type).map(Self::Date64),
            ArrowDataType::Time32(_) => Time32::from_arrow(data_type).map(Self::Time32),
            ArrowDataType::Time64(_) => Time64::from_arrow(data_type).map(Self::Time64),
            ArrowDataType::Timestamp(..) => Timestamp::from_arrow(data_type).map(Self::Timestamp),
            ArrowDataType::Duration(_) => Duration::from_arrow(data_type).map(Self::Duration),
            ArrowDataType::List(_) => List::from_arrow(data_type).map(Self::List),
            ArrowDataType::LargeList(_) => LargeList::from_arrow(data_type).map(Self::LargeList),
            ArrowDataType::Struct(_) => Struct::from_arrow(data_type).map(Self::Struct),
            ArrowDataType::Map(..) => Map::from_arrow(data_type).map(Self::Map),
            other => Err(DataTypeError::ArrowTypeMismatch {
                expected: "a supported data type",
                actual: other.clone(),
            }),
        }
    }

    fn from_arrow_parts(
        data_type: &ArrowDataType,
        metadata_map: &BTreeMap<String, String>,
    ) -> Result<Self, DataTypeError> {
        match metadata_map.get(metadata::TYPE).map(String::as_str) {
            // The `ygg.type` marker names the anchored type to restore.
            Some("timestamp") => {
                Timestamp::from_arrow_parts(data_type, metadata_map).map(Self::Timestamp)
            }
            Some(other) => Err(DataTypeError::InvalidMetadata {
                key: metadata::TYPE,
                value: other.to_owned(),
            }),
            None => {
                if let Some(key) = metadata_map
                    .keys()
                    .find(|key| key.starts_with(metadata::PREFIX))
                {
                    return Err(DataTypeError::UnknownMetadata { key: key.clone() });
                }
                Self::from_arrow(data_type)
            }
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![self.type_id().to_u8()];
        out.extend(delegate!(self, inner => inner.to_bytes()));
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        let [tag, payload @ ..] = bytes else {
            return Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
            });
        };
        match DataTypeId::from_u8(*tag)? {
            DataTypeId::Boolean => Boolean::from_bytes(payload).map(Self::Boolean),
            DataTypeId::Int8 => Int8::from_bytes(payload).map(Self::Int8),
            DataTypeId::Int16 => Int16::from_bytes(payload).map(Self::Int16),
            DataTypeId::Int32 => Int32::from_bytes(payload).map(Self::Int32),
            DataTypeId::Int64 => Int64::from_bytes(payload).map(Self::Int64),
            DataTypeId::UInt8 => UInt8::from_bytes(payload).map(Self::UInt8),
            DataTypeId::UInt16 => UInt16::from_bytes(payload).map(Self::UInt16),
            DataTypeId::UInt32 => UInt32::from_bytes(payload).map(Self::UInt32),
            DataTypeId::UInt64 => UInt64::from_bytes(payload).map(Self::UInt64),
            DataTypeId::Float32 => Float32::from_bytes(payload).map(Self::Float32),
            DataTypeId::Float64 => Float64::from_bytes(payload).map(Self::Float64),
            DataTypeId::Decimal128 => Decimal128::from_bytes(payload).map(Self::Decimal128),
            DataTypeId::Decimal256 => Decimal256::from_bytes(payload).map(Self::Decimal256),
            DataTypeId::Utf8 => Utf8::from_bytes(payload).map(Self::Utf8),
            DataTypeId::LargeUtf8 => LargeUtf8::from_bytes(payload).map(Self::LargeUtf8),
            DataTypeId::Binary => Binary::from_bytes(payload).map(Self::Binary),
            DataTypeId::LargeBinary => LargeBinary::from_bytes(payload).map(Self::LargeBinary),
            DataTypeId::FixedSizeBinary => {
                FixedSizeBinary::from_bytes(payload).map(Self::FixedSizeBinary)
            }
            DataTypeId::Date32 => Date32::from_bytes(payload).map(Self::Date32),
            DataTypeId::Date64 => Date64::from_bytes(payload).map(Self::Date64),
            DataTypeId::Time32 => Time32::from_bytes(payload).map(Self::Time32),
            DataTypeId::Time64 => Time64::from_bytes(payload).map(Self::Time64),
            DataTypeId::Timestamp => Timestamp::from_bytes(payload).map(Self::Timestamp),
            DataTypeId::Duration => Duration::from_bytes(payload).map(Self::Duration),
            DataTypeId::List => List::from_bytes(payload).map(Self::List),
            DataTypeId::LargeList => LargeList::from_bytes(payload).map(Self::LargeList),
            DataTypeId::Struct => Struct::from_bytes(payload).map(Self::Struct),
            DataTypeId::Map => Map::from_bytes(payload).map(Self::Map),
        }
    }
}

impl fmt::Display for AnyDataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        delegate!(self, inner => inner.fmt(f))
    }
}

// `Timestamp` is generic over its unit, so its conversion erases the unit
// into [`AnyTimeUnit`] instead of coming from the macro.
impl<U: TimeUnit> From<Timestamp<U>> for AnyDataType {
    fn from(timestamp: Timestamp<U>) -> Self {
        Self::Timestamp(Timestamp::from_parts(
            AnyTimeUnit::from(timestamp.unit().unit_id()),
            timestamp.timezone().map(Into::into),
        ))
    }
}

from_impls!(
    Boolean: Boolean,
    Int8: Int8,
    Int16: Int16,
    Int32: Int32,
    Int64: Int64,
    UInt8: UInt8,
    UInt16: UInt16,
    UInt32: UInt32,
    UInt64: UInt64,
    Float32: Float32,
    Float64: Float64,
    Decimal128: Decimal128,
    Decimal256: Decimal256,
    Utf8: Utf8,
    LargeUtf8: LargeUtf8,
    Binary: Binary,
    LargeBinary: LargeBinary,
    FixedSizeBinary: FixedSizeBinary,
    Date32: Date32,
    Date64: Date64,
    Time32: Time32,
    Time64: Time64,
    Duration: Duration,
    List: List<AnyDataType>,
    LargeList: LargeList<AnyDataType>,
    Struct: Struct,
    Map: Map,
);
