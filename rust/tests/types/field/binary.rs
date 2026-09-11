use yggdryl::DataType;
use yggdryl::types::{bytes, string};

use super::typed::assert_typed_marker;

#[test]
fn binary_markers_cover_bytes_and_utf8_layouts() {
    assert_typed_marker::<bytes::BinaryType>(DataType::Binary);
    assert_typed_marker::<bytes::FixedSizeBinaryType>(DataType::fixed_size_binary(16).unwrap());
    assert_typed_marker::<bytes::LargeBinaryType>(DataType::LargeBinary);
    assert_typed_marker::<bytes::BinaryViewType>(DataType::BinaryView);
    assert_typed_marker::<string::Utf8Type>(DataType::Utf8);
    assert_typed_marker::<string::LargeUtf8Type>(DataType::LargeUtf8);
    assert_typed_marker::<string::Utf8ViewType>(DataType::Utf8View);
    assert_typed_marker::<string::StringType>(DataType::from_str("string(windows-1252)").unwrap());
}
