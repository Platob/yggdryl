use yggdryl::DataType;
use yggdryl::types::decimal;

use super::typed::assert_typed_marker;

#[test]
fn decimal_markers_cover_every_physical_width() {
    assert_typed_marker::<decimal::Decimal32>(DataType::decimal32(9, 2).unwrap());
    assert_typed_marker::<decimal::Decimal64>(DataType::decimal64(18, 2).unwrap());
    assert_typed_marker::<decimal::Decimal128>(DataType::decimal128(38, 2).unwrap());
    assert_typed_marker::<decimal::Decimal256>(DataType::decimal256(76, 2).unwrap());
}
