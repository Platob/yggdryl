use yggdryl::DataType;
use yggdryl::types::floating;

use super::typed::assert_typed_marker;

#[test]
fn floating_markers_cover_every_width() {
    assert_typed_marker::<floating::Float16Type>(DataType::Float16);
    assert_typed_marker::<floating::Float32Type>(DataType::Float32);
    assert_typed_marker::<floating::Float64Type>(DataType::Float64);
}
