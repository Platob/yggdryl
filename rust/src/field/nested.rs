//! Nested, dictionary, map, and run-end encoded field markers.

use super::typed::define_field_types;

define_field_types!(List, "list", crate::DataType::List(_));
define_field_types!(ListView, "list_view", crate::DataType::ListView(_));
define_field_types!(
    FixedSizeList,
    "fixed_size_list",
    crate::DataType::FixedSizeList(..)
);
define_field_types!(LargeList, "large_list", crate::DataType::LargeList(_));
define_field_types!(
    LargeListView,
    "large_list_view",
    crate::DataType::LargeListView(_)
);
define_field_types!(Struct, "struct", crate::DataType::Struct(_));
define_field_types!(Union, "union", crate::DataType::Union(..));
define_field_types!(Dictionary, "dictionary", crate::DataType::Dictionary(_));
define_field_types!(Map, "map", crate::DataType::Map(_));
define_field_types!(
    RunEndEncoded,
    "run_end_encoded",
    crate::DataType::RunEndEncoded(_)
);
