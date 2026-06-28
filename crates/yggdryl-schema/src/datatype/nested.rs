//! The [`NestedType`] — containers of other fields or types (list, struct, map,
//! union, dictionary, run-end encoding).

use super::{DataType, DataTypeId};
use crate::Field;

/// A nested (container) type. Its children are [`Field`]s (named) or bare
/// [`DataType`]s, depending on the container.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NestedType {
    /// A list of one element [`Field`].
    List(Box<Field>),
    /// A composite of named, typed [`Field`]s. A struct-typed field **is** a schema.
    Struct(Vec<Field>),
    /// A map from a `key` type to a `value` type.
    Map {
        /// The key type.
        key: Box<DataType>,
        /// The value type.
        value: Box<DataType>,
    },
    /// A union of typed alternatives.
    Union(Vec<Field>),
    /// Dictionary encoding: a `key` index type into a `value` dictionary type.
    Dictionary {
        /// The integer index type.
        key: Box<DataType>,
        /// The dictionary value type.
        value: Box<DataType>,
    },
    /// Run-end encoding: a `run_ends` integer type and a `values` type.
    RunEndEncoded {
        /// The run-ends integer type.
        run_ends: Box<DataType>,
        /// The values type.
        values: Box<DataType>,
    },
}

impl NestedType {
    /// The [`DataTypeId`] of this type.
    pub fn type_id(&self) -> DataTypeId {
        use NestedType::*;
        match self {
            List(_) => DataTypeId::List,
            Struct(_) => DataTypeId::Struct,
            Map { .. } => DataTypeId::Map,
            Union(_) => DataTypeId::Union,
            Dictionary { .. } => DataTypeId::Dictionary,
            RunEndEncoded { .. } => DataTypeId::RunEndEncoded,
        }
    }

    /// The canonical name (`"list"`, `"struct"`, …).
    pub fn name(&self) -> &'static str {
        self.type_id().name()
    }

    /// The immediate child [`Field`]s — a list's element, a struct's members or a
    /// union's alternatives. The key/value containers hold child *types*, not fields,
    /// and report an empty slice here.
    pub fn fields(&self) -> &[Field] {
        match self {
            NestedType::List(item) => std::slice::from_ref(item),
            NestedType::Struct(fields) | NestedType::Union(fields) => fields,
            _ => &[],
        }
    }
}
