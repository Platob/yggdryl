use yggdryl::DataType;
use yggdryl::field::integer;

use crate::typed::assert_typed_marker;

#[test]
fn integer_markers_cover_every_signed_and_unsigned_width() {
    assert_typed_marker::<integer::Int8>(DataType::Int8);
    assert_typed_marker::<integer::Int16>(DataType::Int16);
    assert_typed_marker::<integer::Int32>(DataType::Int32);
    assert_typed_marker::<integer::Int64>(DataType::Int64);
    assert_typed_marker::<integer::UInt8>(DataType::UInt8);
    assert_typed_marker::<integer::UInt16>(DataType::UInt16);
    assert_typed_marker::<integer::UInt32>(DataType::UInt32);
    assert_typed_marker::<integer::UInt64>(DataType::UInt64);
}
