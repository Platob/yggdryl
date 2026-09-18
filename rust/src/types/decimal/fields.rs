//! Fixed-width decimal field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Decimal32Type, Decimal32, crate::DataType::Decimal32 { .. });
define_field_types!(Decimal64Type, Decimal64, crate::DataType::Decimal64 { .. });
define_field_types!(
    Decimal128Type,
    Decimal128,
    crate::DataType::Decimal128 { .. }
);
define_field_types!(
    Decimal256Type,
    Decimal256,
    crate::DataType::Decimal256 { .. }
);

/// A Decimal128-typed field.
pub type Decimal128Field = TypedField<Decimal128Type>;
