//! Fixed-width signed and unsigned integer data types.

mod int16;
mod int32;
mod int64;
mod int8;
mod integer_type;
mod uint16;
mod uint32;
mod uint64;
mod uint8;

pub use int16::Int16Type;
pub use int32::Int32Type;
pub use int64::Int64Type;
pub use int8::Int8Type;
pub use integer_type::IntegerType;
pub use uint16::UInt16Type;
pub use uint32::UInt32Type;
pub use uint64::UInt64Type;
pub use uint8::UInt8Type;
