//! Fixed-width decimal field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(
    Decimal32Type,
    "decimal32",
    crate::DataType::Decimal32 { .. }
);
define_field_types!(
    Decimal64Type,
    "decimal64",
    crate::DataType::Decimal64 { .. }
);
define_field_types!(
    Decimal128Type,
    "decimal128",
    crate::DataType::Decimal128 { .. }
);
define_field_types!(
    Decimal256Type,
    "decimal256",
    crate::DataType::Decimal256 { .. }
);

/// A Decimal32-typed field.
pub type Decimal32Field = TypedField<Decimal32Type>;
/// A Decimal64-typed field.
pub type Decimal64Field = TypedField<Decimal64Type>;
/// A Decimal128-typed field.
pub type Decimal128Field = TypedField<Decimal128Type>;
/// A Decimal256-typed field.
pub type Decimal256Field = TypedField<Decimal256Type>;
