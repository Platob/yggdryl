use yggdryl::DataType;
use yggdryl::field::scalar;

use crate::typed::assert_typed_marker;

#[test]
fn scalar_markers_cover_null_and_boolean() {
    assert_typed_marker::<scalar::Null>(DataType::Null);
    assert_typed_marker::<scalar::Boolean>(DataType::Boolean);
}
