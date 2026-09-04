//! Nested datatype layouts and shared child collections.

mod dtypes;
mod parser;

pub use dtypes::{DictionaryType, FieldKey, Fields, MapType, RunEndEncodedType, UnionFields};
pub(crate) use dtypes::{
    cmp_fields, validate_dictionary_key, validate_fields, validate_map_entries, validate_run_ends,
    validate_union_fields,
};
