//! The typed data-type layer: the base traits and one module per type
//! category, one file per type.

mod binary;
mod boolean;
mod data_type;
mod decimal;
mod error;
mod float;
mod integer;
mod list;
mod logical_type;
mod macros;
mod nested_type;
mod primitive_type;
mod string;
mod temporal;

pub use binary::{Binary, FixedSizeBinary, LargeBinary};
pub use boolean::Boolean;
pub use data_type::DataType;
pub use decimal::{Decimal128, Decimal256};
pub use error::DataTypeError;
pub use float::{Float32, Float64};
pub use integer::{Int16, Int32, Int64, Int8, UInt16, UInt32, UInt64, UInt8};
pub use list::{LargeList, List};
pub use logical_type::LogicalType;
pub use nested_type::NestedType;
pub use primitive_type::PrimitiveType;
pub use string::{LargeUtf8, Utf8};
pub use temporal::{Date32, Date64, Duration, Time32, Time64, TimeUnit, Timestamp};
