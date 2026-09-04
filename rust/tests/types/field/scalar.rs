use yggdryl::DataType;
use yggdryl::types::boolean;

use super::typed::assert_typed_marker;

#[test]
fn scalar_markers_cover_null_and_boolean() {
    assert_typed_marker::<boolean::Null>(DataType::Null);
    assert_typed_marker::<boolean::Boolean>(DataType::Boolean);
}
