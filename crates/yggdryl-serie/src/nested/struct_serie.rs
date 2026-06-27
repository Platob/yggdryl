//! [`StructSerie`] — a struct (record) column. Its child fields are themselves
//! [`Serie`]s; they may be **lazy** (e.g. a computed range or a cast result), in which
//! case the backing `StructArray` is built on demand. [`materialize`](Serie::materialize)
//! realises every child and assembles the array with a zero-copy buffer transfer.

use std::any::Any;
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, StructArray};
use arrow_schema::DataType as ADataType;
use yggdryl_schema::{DataType, Field};

use crate::error::{SerieError, SerieResult};
use crate::nested::NestedSerie;
use crate::scalar::Scalar;
use crate::serie::{dispatch, Serie, SerieRef};

/// A struct column: a [`Serie`] per child field. Built either from an Arrow
/// `StructArray` (the cached `array`) or from child columns that may still be lazy (the
/// `array` is then assembled on demand / on `materialize`).
#[derive(Debug, Clone)]
pub struct StructSerie {
    field: Field,
    children: Vec<SerieRef>,
    /// The materialised struct array, when available (the `from_parts` path or after
    /// `materialize`); `None` while the column is lazy.
    array: Option<StructArray>,
}

/// Assembles a `StructArray` from `children` (a zero-copy transfer — it references each
/// child's buffers).
fn struct_array(children: &[SerieRef]) -> SerieResult<StructArray> {
    let fields = children
        .iter()
        .map(|c| c.field().to_arrow().map(Arc::new))
        .collect::<Result<Vec<_>, _>>()?;
    let arrays = children.iter().map(|c| c.array()).collect::<Vec<_>>();
    StructArray::try_new(fields.into(), arrays, None).map_err(|e| SerieError::Arrow(e.to_string()))
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
            children,
            array: Some(array),
        })
    }

    /// Builds a struct column named `name` from its child columns — the one-line
    /// constructor (each child's field, including its name, becomes a struct field). The
    /// children must all have the same length. The result is **lazy**: the backing
    /// `StructArray` is assembled on demand, so lazy children stay lazy until
    /// [`materialize`](Serie::materialize).
    pub fn from_children(
        name: impl Into<String>,
        children: Vec<SerieRef>,
    ) -> SerieResult<StructSerie> {
        if let Some(first) = children.first() {
            let len = first.len();
            if let Some(bad) = children.iter().find(|c| c.len() != len) {
                return Err(SerieError::Arrow(format!(
                    "struct children must have equal length: '{}' has {} but '{}' has {}",
                    bad.name(),
                    bad.len(),
                    first.name(),
                    len
                )));
            }
        }
        let field = Field::new(
            name,
            DataType::struct_(children.iter().map(|c| c.field().clone()).collect()),
            true,
        );
        Ok(StructSerie {
            field,
            children,
            array: None,
        })
    }

    /// The child columns, in field order.
    pub fn children(&self) -> &[SerieRef] {
        &self.children
    }

    /// The number of rows (the children's length).
    fn rows(&self) -> usize {
        match &self.array {
            Some(a) => a.len(),
            None => self.children.first().map_or(0, |c| c.len()),
        }
    }
}

impl Serie for StructSerie {
    fn field(&self) -> &Field {
        &self.field
    }

    fn array(&self) -> ArrayRef {
        match &self.array {
            Some(a) => Arc::new(a.clone()),
            None => {
                Arc::new(struct_array(&self.children).expect("validated struct children build"))
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn len(&self) -> usize {
        self.rows()
    }

    fn null_count(&self) -> usize {
        self.array.as_ref().map_or(0, |a| a.null_count())
    }

    fn is_null(&self, index: usize) -> bool {
        match &self.array {
            Some(a) => index >= a.len() || a.is_null(index),
            None => index >= self.rows(),
        }
    }

    fn is_materialized(&self) -> bool {
        self.array.is_some()
    }

    /// Realises every (possibly lazy) child and assembles the backing `StructArray` with
    /// a zero-copy buffer transfer.
    fn materialize(&self) -> SerieRef {
        if self.array.is_some() {
            return Arc::new(self.clone());
        }
        let children: Vec<SerieRef> = self.children.iter().map(|c| c.materialize()).collect();
        let array = struct_array(&children).expect("validated struct children build");
        Arc::new(StructSerie {
            field: self.field.clone(),
            children,
            array: Some(array),
        })
    }

    fn as_nested(&self) -> Option<&dyn NestedSerie> {
        Some(self)
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
}
