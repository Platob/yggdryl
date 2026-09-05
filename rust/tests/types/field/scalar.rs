use yggdryl::DataType;
use yggdryl::types::boolean;

use super::typed::assert_typed_marker;

#[test]
fn scalar_markers_cover_null_and_boolean() {
    assert_typed_marker::<boolean::NullType>(DataType::Null);
    assert_typed_marker::<boolean::BooleanType>(DataType::Boolean);
}
