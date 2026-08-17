//! Binary and UTF-8 field markers.

use super::typed::define_field_types;

define_field_types!(Binary, "binary", crate::DataType::Binary);
define_field_types!(
    FixedSizeBinary,
    "fixed_size_binary",
    crate::DataType::FixedSizeBinary(_)
);
define_field_types!(LargeBinary, "large_binary", crate::DataType::LargeBinary);
define_field_types!(BinaryView, "binary_view", crate::DataType::BinaryView);
define_field_types!(Utf8, "utf8", crate::DataType::Utf8);
define_field_types!(LargeUtf8, "large_utf8", crate::DataType::LargeUtf8);
define_field_types!(Utf8View, "utf8_view", crate::DataType::Utf8View);
