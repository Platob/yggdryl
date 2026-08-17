//! Fixed-width decimal field markers.

use super::typed::define_field_types;

define_field_types!(Decimal32, "decimal32", crate::DataType::Decimal32 { .. });
define_field_types!(Decimal64, "decimal64", crate::DataType::Decimal64 { .. });
define_field_types!(Decimal128, "decimal128", crate::DataType::Decimal128 { .. });
define_field_types!(Decimal256, "decimal256", crate::DataType::Decimal256 { .. });
