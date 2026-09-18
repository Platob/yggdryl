use yggdryl::DataType;

use super::typed::assert_typed_marker;

#[test]
fn decimal_markers_cover_every_physical_width() {
    assert_typed_marker::<yggdryl::types::Decimal32Type>(DataType::decimal32(9, 2).unwrap());
    assert_typed_marker::<yggdryl::types::Decimal64Type>(DataType::decimal64(18, 2).unwrap());
    assert_typed_marker::<yggdryl::types::Decimal128Type>(DataType::decimal128(38, 2).unwrap());
    assert_typed_marker::<yggdryl::types::Decimal256Type>(DataType::decimal256(76, 2).unwrap());
}
