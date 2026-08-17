use yggdryl::field::nested;
use yggdryl::{DataType, Field, UnionMode};

use crate::typed::assert_typed_marker;

#[test]
fn nested_markers_cover_every_child_layout() {
    let item = || Field::new("item", DataType::Utf8, true);
    assert_typed_marker::<nested::List>(DataType::list(item()));
    assert_typed_marker::<nested::ListView>(DataType::list_view(item()));
    assert_typed_marker::<nested::FixedSizeList>(DataType::fixed_size_list(item(), 3).unwrap());
    assert_typed_marker::<nested::LargeList>(DataType::large_list(item()));
    assert_typed_marker::<nested::LargeListView>(DataType::large_list_view(item()));
    assert_typed_marker::<nested::Struct>(DataType::from_fields([item()]).unwrap());
    assert_typed_marker::<nested::Union>(DataType::union([(4, item())], UnionMode::Dense).unwrap());
    assert_typed_marker::<nested::Dictionary>(
        DataType::dictionary(DataType::Int16, DataType::Utf8).unwrap(),
    );
    assert_typed_marker::<nested::Map>(
        DataType::map_of(DataType::Utf8, DataType::Int64, false).unwrap(),
    );
    assert_typed_marker::<nested::RunEndEncoded>(
        DataType::run_end_encoded(
            Field::new("run_ends", DataType::Int32, false),
            Field::new("values", DataType::Utf8, true),
        )
        .unwrap(),
    );
}
