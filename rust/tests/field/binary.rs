use yggdryl::DataType;
use yggdryl::types::{bytes, text};

use crate::typed::assert_typed_marker;

#[test]
fn binary_markers_cover_bytes_and_utf8_layouts() {
    assert_typed_marker::<bytes::Binary>(DataType::Binary);
    assert_typed_marker::<bytes::FixedSizeBinary>(DataType::fixed_size_binary(16).unwrap());
    assert_typed_marker::<bytes::LargeBinary>(DataType::LargeBinary);
    assert_typed_marker::<bytes::BinaryView>(DataType::BinaryView);
    assert_typed_marker::<text::Utf8>(DataType::Utf8);
    assert_typed_marker::<text::LargeUtf8>(DataType::LargeUtf8);
    assert_typed_marker::<text::Utf8View>(DataType::Utf8View);
}
