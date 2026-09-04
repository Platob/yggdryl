//! Binary and UTF-8 field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(Binary, "binary", crate::DataType::Binary);
define_field_types!(
    FixedSizeBinary,
    "fixed_size_binary",
    crate::DataType::FixedSizeBinary(_)
);
define_field_types!(LargeBinary, "large_binary", crate::DataType::LargeBinary);
define_field_types!(BinaryView, "binary_view", crate::DataType::BinaryView);

/// A variable binary-typed field.
pub type BinaryField = TypedField<Binary>;
/// A fixed-size binary-typed field.
pub type FixedSizeBinaryField = TypedField<FixedSizeBinary>;
/// A large binary-typed field.
pub type LargeBinaryField = TypedField<LargeBinary>;
/// A binary-view-typed field.
pub type BinaryViewField = TypedField<BinaryView>;
