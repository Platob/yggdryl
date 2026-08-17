use yggdryl::DataType;
use yggdryl::field::binary;

use crate::typed::assert_typed_marker;

#[test]
fn binary_markers_cover_bytes_and_utf8_layouts() {
    assert_typed_marker::<binary::Binary>(DataType::Binary);
    assert_typed_marker::<binary::FixedSizeBinary>(DataType::fixed_size_binary(16).unwrap());
    assert_typed_marker::<binary::LargeBinary>(DataType::LargeBinary);
    assert_typed_marker::<binary::BinaryView>(DataType::BinaryView);
    assert_typed_marker::<binary::Utf8>(DataType::Utf8);
    assert_typed_marker::<binary::LargeUtf8>(DataType::LargeUtf8);
    assert_typed_marker::<binary::Utf8View>(DataType::Utf8View);
}
