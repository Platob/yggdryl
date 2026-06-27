//! [`StructSerie`] — a struct (record) column backed by an Arrow `StructArray`. Its
//! child fields are themselves [`Serie`]s, built recursively, so arbitrarily nested
//! structures resolve through the same [factory](crate::from_arrow).

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StructArray};
use arrow_schema::DataType as ADataType;
use yggdryl_schema::Field;

use crate::error::SerieResult;
use crate::nested::NestedSerie;
use crate::scalar::Scalar;
use crate::serie::{dispatch, Serie, SerieRef};

/// A struct column: a [`Serie`] per child field (built recursively), addressable by
/// index or name.
#[derive(Debug, Clone)]
pub struct StructSerie {
    field: Field,
    array: StructArray,
    children: Vec<SerieRef>,
}

impl StructSerie {
    /// Wraps a field and a matching `StructArray`, building each child column
    /// **recursively** (so nested structs/lists/maps resolve too). Used by the
    /// [factory](crate::from_arrow); fallible because a child column may fail to build.
    pub(crate) fn from_parts(field: Field, array: ArrayRef) -> SerieResult<StructSerie> {
        let array = array
            .as_any()
            .downcast_ref::<StructArray>()
            .expect("data type matched the struct array")
            .clone();
        let children = match array.data_type() {
            ADataType::Struct(fields) => fields
                .iter()
                .zip(array.columns())
                .map(|(f, col)| dispatch(Field::from_arrow(f.as_ref()), col.clone()))
                .collect::<SerieResult<Vec<_>>>()?,
            _ => Vec::new(),
        };
        Ok(StructSerie {
            field,
            array,
            children,
        })
    }

    /// The child columns, in field order.
    pub fn children(&self) -> &[SerieRef] {
        &self.children
    }
}

impl Serie for StructSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        Arc::new(self.array.clone())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.array.len()
    }

    fn null_count(&self) -> usize {
        self.array.null_count()
    }

    fn is_null(&self, index: usize) -> bool {
        index >= self.array.len() || self.array.is_null(index)
    }

    /// A readable `{name=value, …}` rendering of the record at `index`.
    fn value_at(&self, index: usize) -> Scalar {
        if self.is_null(index) {
            return Scalar::Null;
        }
        let mut text = String::from("{");
        for (i, child) in self.children.iter().enumerate() {
            if i > 0 {
                text.push_str(", ");
            }
            text.push_str(child.name());
            text.push('=');
            text.push_str(&child.value_at(index).to_string());
        }
        text.push('}');
        Scalar::Other(text)
    }
}

impl NestedSerie for StructSerie {
    fn child_count(&self) -> usize {
        self.children.len()
    }

    fn child(&self, index: usize) -> Option<SerieRef> {
        self.children.get(index).cloned()
    }

    fn child_by_name(&self, name: &str) -> Option<SerieRef> {
        self.children.iter().find(|c| c.name() == name).cloned()
    }
}
