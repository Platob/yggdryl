//! Binary and UTF-8 field markers.

use crate::TypedField;
use crate::types::typed::define_field_types;

define_field_types!(BinaryType, "binary", crate::DataType::Binary);
define_field_types!(
    FixedSizeBinaryType,
    "fixed_size_binary",
    crate::DataType::FixedSizeBinary(_)
);
define_field_types!(
    LargeBinaryType,
    "large_binary",
    crate::DataType::LargeBinary
);
define_field_types!(BinaryViewType, "binary_view", crate::DataType::BinaryView);

/// A variable binary-typed field.
pub type BinaryField = TypedField<BinaryType>;
/// A fixed-size binary-typed field.
pub type FixedSizeBinaryField = TypedField<FixedSizeBinaryType>;
/// A large binary-typed field.
pub type LargeBinaryField = TypedField<LargeBinaryType>;
/// A binary-view-typed field.
pub type BinaryViewField = TypedField<BinaryViewType>;
