use yggdryl::DataType;
use yggdryl::field::floating;

use crate::typed::assert_typed_marker;

#[test]
fn floating_markers_cover_every_width() {
    assert_typed_marker::<floating::Float16>(DataType::Float16);
    assert_typed_marker::<floating::Float32>(DataType::Float32);
    assert_typed_marker::<floating::Float64>(DataType::Float64);
}
