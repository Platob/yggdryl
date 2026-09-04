//! Fixed-width decimal field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Decimal32, "decimal32", crate::DataType::Decimal32 { .. });
define_field_types!(Decimal64, "decimal64", crate::DataType::Decimal64 { .. });
define_field_types!(Decimal128, "decimal128", crate::DataType::Decimal128 { .. });
define_field_types!(Decimal256, "decimal256", crate::DataType::Decimal256 { .. });

/// A Decimal32-typed field.
pub type Decimal32Field = TypedField<Decimal32>;
/// A Decimal64-typed field.
pub type Decimal64Field = TypedField<Decimal64>;
/// A Decimal128-typed field.
pub type Decimal128Field = TypedField<Decimal128>;
/// A Decimal256-typed field.
pub type Decimal256Field = TypedField<Decimal256>;
