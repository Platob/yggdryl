//! The [`NestedFields`] children-field accessors, shared by [`DataType`] and
//! [`Field`].

use crate::error::SchemaError;
use crate::field::Field;

/// The children-field accessors shared by [`DataType`](crate::DataType) and
/// [`Field`](crate::Field) — both are `NestedFields`, so the same lookups work on a
/// data type and on a field. An implementor supplies
/// [`children_fields`](NestedFields::children_fields) (empty by default, for a leaf);
/// [`child_field_at`](NestedFields::child_field_at) /
/// [`child_field_by`](NestedFields::child_field_by) / [`child_field`](NestedFields::child_field)
/// are provided over it.
pub trait NestedFields {
    /// The child fields, in order. Empty for a leaf (a primitive type, or a field of
    /// one); a nested type or field overrides it.
    fn children_fields(&self) -> &[Box<dyn Field>] {
        &[]
    }

    /// The child field at `index`, if any.
    fn child_field_at(&self, index: usize) -> Option<&dyn Field> {
        self.children_fields().get(index).map(|field| &**field)
    }

    /// The child field named `name`. With `case_sensitive` false (the usual
    /// default), an exact match is preferred and an ASCII case-insensitive match is
    /// the fallback; with it true, only an exact match counts.
    fn child_field_by(&self, name: &str, case_sensitive: bool) -> Option<&dyn Field> {
        if let Some(field) = self.children_fields().iter().find(|f| f.name() == name) {
            return Some(&**field);
        }
        if !case_sensitive {
            if let Some(field) = self
                .children_fields()
                .iter()
                .find(|f| f.name().eq_ignore_ascii_case(name))
            {
                return Some(&**field);
            }
        }
        None
    }

    /// Looks up a child by `index`, by `name`, or by both. Given both, the `index` is
    /// tried first (fast) and accepted only when its field's name also matches,
    /// otherwise the search falls back to `name`. Given one, that selector is used.
    /// Errors [`NoChildSelector`](SchemaError::NoChildSelector) when neither is given.
    fn child_field(
        &self,
        index: Option<usize>,
        name: Option<&str>,
        case_sensitive: bool,
    ) -> Result<Option<&dyn Field>, SchemaError> {
        match (index, name) {
            (Some(index), Some(name)) => {
                if let Some(field) = self.child_field_at(index) {
                    let hit = field.name() == name
                        || (!case_sensitive && field.name().eq_ignore_ascii_case(name));
                    if hit {
                        return Ok(Some(field));
                    }
                }
                Ok(self.child_field_by(name, case_sensitive))
            }
            (Some(index), None) => Ok(self.child_field_at(index)),
            (None, Some(name)) => Ok(self.child_field_by(name, case_sensitive)),
            (None, None) => Err(SchemaError::NoChildSelector),
        }
    }
}
