//! The erased data type covering every supported constructor.

use core::fmt;
use std::collections::BTreeMap;

use arrow_schema::DataType as ArrowDataType;

use crate::{
    metadata, AnyTime32Unit, AnyTime64Unit, AnyTimeUnit, BinaryType, BooleanType, DataType,
    DataTypeError, DataTypeId, Date32Type, Date64Type, Decimal128Type, Decimal256Type, Duration,
    DurationType, FixedSizeBinaryType, Float32Type, Float64Type, Int16Type, Int32Type, Int64Type,
    Int8Type, LargeBinaryType, LargeListType, LargeUtf8Type, ListType, MapType, StructType,
    TemporalType, Time, Time32Type, Time32Unit, Time64Type, Time64Unit, TimeUnit, Timestamp,
    TimestampType, UInt16Type, UInt32Type, UInt64Type, UInt8Type, Utf8Type,
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
/// [`StructType`]'s fields, a schema, a binding-held type — can hold any data
/// type behind a single `Sized` value.
///
/// `AnyDataType` implements [`DataType`] itself by delegating to the wrapped
/// value; its `from_arrow` is the dispatcher across every supported Arrow
/// type, and its byte encoding is the wrapped type's payload behind a
/// [`DataTypeId`] tag.
///
/// ```
/// use yggdryl_schema::{AnyDataType, DataType, DataTypeId, Int32Type};
///
/// let any = AnyDataType::from(Int32Type);
/// assert_eq!(any.type_id(), DataTypeId::Int32);
/// assert_eq!(AnyDataType::from_arrow(&any.to_arrow()), Ok(any));
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum AnyDataType {
    /// [`BooleanType`].
    Boolean(BooleanType),
    /// [`Int8Type`].
    Int8(Int8Type),
    /// [`Int16Type`].
    Int16(Int16Type),
    /// [`Int32Type`].
    Int32(Int32Type),
    /// [`Int64Type`].
    Int64(Int64Type),
    /// [`UInt8Type`].
    UInt8(UInt8Type),
    /// [`UInt16Type`].
    UInt16(UInt16Type),
    /// [`UInt32Type`].
    UInt32(UInt32Type),
    /// [`UInt64Type`].
    UInt64(UInt64Type),
    /// [`Float32Type`].
    Float32(Float32Type),
    /// [`Float64Type`].
    Float64(Float64Type),
    /// [`Decimal128Type`].
    Decimal128(Decimal128Type),
    /// [`Decimal256Type`].
    Decimal256(Decimal256Type),
    /// [`Utf8Type`].
    Utf8(Utf8Type),
    /// [`LargeUtf8Type`].
    LargeUtf8(LargeUtf8Type),
    /// [`BinaryType`].
    Binary(BinaryType),
    /// [`LargeBinaryType`].
    LargeBinary(LargeBinaryType),
    /// [`FixedSizeBinaryType`].
    FixedSizeBinary(FixedSizeBinaryType),
    /// [`Date32Type`].
    Date32(Date32Type),
    /// [`Date64Type`].
    Date64(Date64Type),
    /// [`Time32Type`] over an erased unit.
    Time32(Time32Type<AnyTime32Unit>),
    /// [`Time64Type`] over an erased unit.
    Time64(Time64Type<AnyTime64Unit>),
    /// [`TimestampType`] over an erased unit.
    Timestamp(TimestampType<AnyTimeUnit>),
    /// [`DurationType`] over an erased unit.
    Duration(DurationType<AnyTimeUnit>),
    /// [`ListType`] over an erased child.
    List(ListType<AnyDataType>),
    /// [`LargeListType`] over an erased child.
    LargeList(LargeListType<AnyDataType>),
    /// [`StructType`].
    Struct(StructType),
    /// [`MapType`].
    Map(MapType),
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
            ArrowDataType::Boolean => BooleanType::from_arrow(data_type).map(Self::Boolean),
            ArrowDataType::Int8 => Int8Type::from_arrow(data_type).map(Self::Int8),
            ArrowDataType::Int16 => Int16Type::from_arrow(data_type).map(Self::Int16),
            ArrowDataType::Int32 => Int32Type::from_arrow(data_type).map(Self::Int32),
            ArrowDataType::Int64 => Int64Type::from_arrow(data_type).map(Self::Int64),
            ArrowDataType::UInt8 => UInt8Type::from_arrow(data_type).map(Self::UInt8),
            ArrowDataType::UInt16 => UInt16Type::from_arrow(data_type).map(Self::UInt16),
            ArrowDataType::UInt32 => UInt32Type::from_arrow(data_type).map(Self::UInt32),
            ArrowDataType::UInt64 => UInt64Type::from_arrow(data_type).map(Self::UInt64),
            ArrowDataType::Float32 => Float32Type::from_arrow(data_type).map(Self::Float32),
            ArrowDataType::Float64 => Float64Type::from_arrow(data_type).map(Self::Float64),
            ArrowDataType::Decimal128(..) => {
                Decimal128Type::from_arrow(data_type).map(Self::Decimal128)
            }
            ArrowDataType::Decimal256(..) => {
                Decimal256Type::from_arrow(data_type).map(Self::Decimal256)
            }
            ArrowDataType::Utf8 => Utf8Type::from_arrow(data_type).map(Self::Utf8),
            ArrowDataType::LargeUtf8 => LargeUtf8Type::from_arrow(data_type).map(Self::LargeUtf8),
            ArrowDataType::Binary => BinaryType::from_arrow(data_type).map(Self::Binary),
            ArrowDataType::LargeBinary => {
                LargeBinaryType::from_arrow(data_type).map(Self::LargeBinary)
            }
            ArrowDataType::FixedSizeBinary(_) => {
                FixedSizeBinaryType::from_arrow(data_type).map(Self::FixedSizeBinary)
            }
            ArrowDataType::Date32 => Date32Type::from_arrow(data_type).map(Self::Date32),
            ArrowDataType::Date64 => Date64Type::from_arrow(data_type).map(Self::Date64),
            ArrowDataType::Time32(_) => Time32Type::from_arrow(data_type).map(Self::Time32),
            ArrowDataType::Time64(_) => Time64Type::from_arrow(data_type).map(Self::Time64),
            ArrowDataType::Timestamp(..) => {
                TimestampType::from_arrow(data_type).map(Self::Timestamp)
            }
            ArrowDataType::Duration(_) => DurationType::from_arrow(data_type).map(Self::Duration),
            ArrowDataType::List(_) => ListType::from_arrow(data_type).map(Self::List),
            ArrowDataType::LargeList(_) => {
                LargeListType::from_arrow(data_type).map(Self::LargeList)
            }
            ArrowDataType::Struct(_) => StructType::from_arrow(data_type).map(Self::Struct),
            ArrowDataType::Map(..) => MapType::from_arrow(data_type).map(Self::Map),
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
                TimestampType::from_arrow_parts(data_type, metadata_map).map(Self::Timestamp)
            }
            Some("duration") => {
                DurationType::from_arrow_parts(data_type, metadata_map).map(Self::Duration)
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
        // Every concrete encoding already leads with its DataTypeId tag, so
        // the erased encoding is exactly the concrete one.
        delegate!(self, inner => inner.to_bytes())
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DataTypeError> {
        // Peek the tag to pick the constructor; the concrete decoder
        // re-validates it.
        let [tag, ..] = bytes else {
            return Err(DataTypeError::InvalidByteLength {
                expected: 1,
                actual: bytes.len(),
            });
        };
        match DataTypeId::from_u8(*tag)? {
            DataTypeId::Boolean => BooleanType::from_bytes(bytes).map(Self::Boolean),
            DataTypeId::Int8 => Int8Type::from_bytes(bytes).map(Self::Int8),
            DataTypeId::Int16 => Int16Type::from_bytes(bytes).map(Self::Int16),
            DataTypeId::Int32 => Int32Type::from_bytes(bytes).map(Self::Int32),
            DataTypeId::Int64 => Int64Type::from_bytes(bytes).map(Self::Int64),
            DataTypeId::UInt8 => UInt8Type::from_bytes(bytes).map(Self::UInt8),
            DataTypeId::UInt16 => UInt16Type::from_bytes(bytes).map(Self::UInt16),
            DataTypeId::UInt32 => UInt32Type::from_bytes(bytes).map(Self::UInt32),
            DataTypeId::UInt64 => UInt64Type::from_bytes(bytes).map(Self::UInt64),
            DataTypeId::Float32 => Float32Type::from_bytes(bytes).map(Self::Float32),
            DataTypeId::Float64 => Float64Type::from_bytes(bytes).map(Self::Float64),
            DataTypeId::Decimal128 => Decimal128Type::from_bytes(bytes).map(Self::Decimal128),
            DataTypeId::Decimal256 => Decimal256Type::from_bytes(bytes).map(Self::Decimal256),
            DataTypeId::Utf8 => Utf8Type::from_bytes(bytes).map(Self::Utf8),
            DataTypeId::LargeUtf8 => LargeUtf8Type::from_bytes(bytes).map(Self::LargeUtf8),
            DataTypeId::Binary => BinaryType::from_bytes(bytes).map(Self::Binary),
            DataTypeId::LargeBinary => LargeBinaryType::from_bytes(bytes).map(Self::LargeBinary),
            DataTypeId::FixedSizeBinary => {
                FixedSizeBinaryType::from_bytes(bytes).map(Self::FixedSizeBinary)
            }
            DataTypeId::Date32 => Date32Type::from_bytes(bytes).map(Self::Date32),
            DataTypeId::Date64 => Date64Type::from_bytes(bytes).map(Self::Date64),
            DataTypeId::Time32 => Time32Type::from_bytes(bytes).map(Self::Time32),
            DataTypeId::Time64 => Time64Type::from_bytes(bytes).map(Self::Time64),
            DataTypeId::Timestamp => TimestampType::from_bytes(bytes).map(Self::Timestamp),
            DataTypeId::Duration => DurationType::from_bytes(bytes).map(Self::Duration),
            DataTypeId::List => ListType::from_bytes(bytes).map(Self::List),
            DataTypeId::LargeList => LargeListType::from_bytes(bytes).map(Self::LargeList),
            DataTypeId::Struct => StructType::from_bytes(bytes).map(Self::Struct),
            DataTypeId::Map => MapType::from_bytes(bytes).map(Self::Map),
        }
    }
}

impl fmt::Display for AnyDataType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        delegate!(self, inner => inner.fmt(f))
    }
}

// The temporal types are generic over their unit, so their conversions erase
// the unit into the matching `Any*Unit` instead of coming from the macro.
impl<U: TimeUnit> From<TimestampType<U>> for AnyDataType {
    fn from(timestamp: TimestampType<U>) -> Self {
        Self::Timestamp(TimestampType::from_parts(
            AnyTimeUnit::from(timestamp.unit().unit_id()),
            timestamp.timezone().map(Into::into),
        ))
    }
}

impl<U: TimeUnit> From<DurationType<U>> for AnyDataType {
    fn from(duration: DurationType<U>) -> Self {
        Self::Duration(DurationType::from_parts(AnyTimeUnit::from(
            duration.unit().unit_id(),
        )))
    }
}

impl<U: Time32Unit> From<Time32Type<U>> for AnyDataType {
    fn from(time: Time32Type<U>) -> Self {
        Self::Time32(Time32Type::from_parts(
            // Every `Time32Unit` id is a 32-bit time unit, so the erased
            // construction never fails.
            AnyTime32Unit::from_unit_id(time.unit().unit_id())
                .expect("Time32Unit is restricted to 32-bit time units"),
        ))
    }
}

impl<U: Time64Unit> From<Time64Type<U>> for AnyDataType {
    fn from(time: Time64Type<U>) -> Self {
        Self::Time64(Time64Type::from_parts(
            // Every `Time64Unit` id is a 64-bit time unit, so the erased
            // construction never fails.
            AnyTime64Unit::from_unit_id(time.unit().unit_id())
                .expect("Time64Unit is restricted to 64-bit time units"),
        ))
    }
}

from_impls!(
    Boolean: BooleanType,
    Int8: Int8Type,
    Int16: Int16Type,
    Int32: Int32Type,
    Int64: Int64Type,
    UInt8: UInt8Type,
    UInt16: UInt16Type,
    UInt32: UInt32Type,
    UInt64: UInt64Type,
    Float32: Float32Type,
    Float64: Float64Type,
    Decimal128: Decimal128Type,
    Decimal256: Decimal256Type,
    Utf8: Utf8Type,
    LargeUtf8: LargeUtf8Type,
    Binary: BinaryType,
    LargeBinary: LargeBinaryType,
    FixedSizeBinary: FixedSizeBinaryType,
    Date32: Date32Type,
    Date64: Date64Type,
    List: ListType<AnyDataType>,
    LargeList: LargeListType<AnyDataType>,
    Struct: StructType,
    Map: MapType,
);
