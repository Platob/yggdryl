//! Nested datatype layouts and shared child collections.

#[cfg(feature = "arrow")]
pub(crate) mod casts;

mod dtypes;
mod fields;
mod parser;
mod scalars;

pub use dtypes::{DictionaryType, MapType, RunEndEncodedType, UnionFields};
pub(crate) use dtypes::{
    cmp_fields, validate_dictionary_key, validate_fields, validate_map_entries, validate_run_ends,
    validate_union_fields,
};
pub use fields::*;
pub use scalars::{
    Children, DictionaryScalar, FixedSizeListScalar, LargeListScalar, LargeListViewScalar,
    ListScalar, ListViewScalar, MapScalar, Mapping, Nested, NestedValue, Record,
    RunEndEncodedScalar, Sequence, StructScalar, UnionScalar, VariantScalar,
};
