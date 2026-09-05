use yggdryl::DataType;
use yggdryl::types::{bytes, text};

use super::typed::assert_typed_marker;

#[test]
fn binary_markers_cover_bytes_and_utf8_layouts() {
    assert_typed_marker::<bytes::BinaryType>(DataType::Binary);
    assert_typed_marker::<bytes::FixedSizeBinaryType>(DataType::fixed_size_binary(16).unwrap());
    assert_typed_marker::<bytes::LargeBinaryType>(DataType::LargeBinary);
    assert_typed_marker::<bytes::BinaryViewType>(DataType::BinaryView);
    assert_typed_marker::<text::Utf8Type>(DataType::Utf8);
    assert_typed_marker::<text::LargeUtf8Type>(DataType::LargeUtf8);
    assert_typed_marker::<text::Utf8ViewType>(DataType::Utf8View);
}
