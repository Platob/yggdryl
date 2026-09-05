use yggdryl::DataType;
use yggdryl::types::integer;

use super::typed::assert_typed_marker;

#[test]
fn integer_markers_cover_every_signed_and_unsigned_width() {
    assert_typed_marker::<integer::Int8Type>(DataType::Int8);
    assert_typed_marker::<integer::Int16Type>(DataType::Int16);
    assert_typed_marker::<integer::Int32Type>(DataType::Int32);
    assert_typed_marker::<integer::Int64Type>(DataType::Int64);
    assert_typed_marker::<integer::UInt8Type>(DataType::UInt8);
    assert_typed_marker::<integer::UInt16Type>(DataType::UInt16);
    assert_typed_marker::<integer::UInt32Type>(DataType::UInt32);
    assert_typed_marker::<integer::UInt64Type>(DataType::UInt64);
}
