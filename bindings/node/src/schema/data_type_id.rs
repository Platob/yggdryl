//! `DataTypeId` — the Arrow data-type discriminant exposed to JS.

use napi_derive::napi;

/// A `u8` discriminant identifying an Arrow data type and its category.
///
/// Mirrors `yggdryl_schema::DataTypeId`; keep the variants and their values in
/// sync with the core enum.
#[napi(namespace = "schema")]
pub enum DataTypeId {
    Null = 0x00,
    Boolean = 0x01,
    Int8 = 0x02,
    Int16 = 0x03,
    Int32 = 0x04,
    Int64 = 0x05,
    UInt8 = 0x06,
    UInt16 = 0x07,
    UInt32 = 0x08,
    UInt64 = 0x09,
    Float16 = 0x0A,
    Float32 = 0x0B,
    Float64 = 0x0C,
    Binary = 0x0D,
    LargeBinary = 0x0E,
    FixedSizeBinary = 0x0F,
    Utf8 = 0x10,
    LargeUtf8 = 0x11,
    BinaryView = 0x12,
    LargeBinaryView = 0x13,
    Decimal128 = 0x40,
    Decimal256 = 0x41,
    Date32 = 0x42,
    Date64 = 0x43,
    Time32 = 0x44,
    Time64 = 0x45,
    Timestamp = 0x46,
    Duration = 0x47,
    Interval = 0x48,
    Dictionary = 0x49,
    List = 0x80,
    LargeList = 0x81,
    FixedSizeList = 0x82,
    Struct = 0x83,
    Map = 0x84,
    Union = 0x85,
}
