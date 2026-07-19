//! [`MapField`] — the **map schema**: a map column's name, nullability, metadata, the child **key**
//! and **value** [`ColumnField`]s, and a `keys_sorted` flag. It is the value-typed descriptor a
//! [`MapSerie`](super::MapSerie) reports and a [`ColumnField::Map`](super::super::ColumnField)
//! carries. (A later phase maps it onto an Arrow `Field(Map)`; kept clean but with no Arrow
//! dependency now.)

use crate::datatype_id::DataTypeId;
use crate::headers::Headers;
use crate::typed::nested::ColumnField;

/// A map column's schema — its `name`, `nullable` flag, free-form `metadata`, the child `key` and
/// `value` field descriptors, and whether the keys are sorted within each map. Value-typed (`Clone`
/// / `PartialEq` / `Eq` / `Hash`) so a schema keys a map, sits in a set, and travels over a wire —
/// equal iff every field matches, and equal schemas hash equal.
///
/// ```
/// use yggdryl_core::datatype_id::DataTypeId;
/// use yggdryl_core::typed::{ColumnField, HeaderField, MapField};
///
/// let key = ColumnField::Leaf(HeaderField::new(Some("key"), DataTypeId::Utf8, false));
/// let value = ColumnField::Leaf(HeaderField::new(Some("value"), DataTypeId::I32, true));
/// let field = MapField::new(Some("prices"), key, value).with_keys_sorted(true);
///
/// assert_eq!(field.data_type_id(), DataTypeId::Map);
/// assert_eq!(field.key().data_type_id(), DataTypeId::Utf8);
/// assert_eq!(field.value().data_type_id(), DataTypeId::I32);
/// assert!(field.keys_sorted());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MapField {
    name: Option<Box<str>>,
    nullable: bool,
    metadata: Headers,
    key: Box<ColumnField>,
    value: Box<ColumnField>,
    keys_sorted: bool,
}

impl MapField {
    /// A map schema from its `name` and the child `key` / `value` fields (non-nullable, keys
    /// unsorted, no extra metadata).
    pub fn new(name: Option<&str>, key: ColumnField, value: ColumnField) -> Self {
        MapField {
            name: name.map(Into::into),
            nullable: false,
            metadata: Headers::new(),
            key: Box::new(key),
            value: Box::new(value),
            keys_sorted: false,
        }
    }

    /// The child **key** field.
    pub fn key(&self) -> &ColumnField {
        &self.key
    }

    /// The child **value** field.
    pub fn value(&self) -> &ColumnField {
        &self.value
    }

    /// Whether the keys are **sorted** within each map (an Arrow schema hint).
    pub fn keys_sorted(&self) -> bool {
        self.keys_sorted
    }

    /// Sets whether the keys are sorted within each map.
    pub fn set_keys_sorted(&mut self, keys_sorted: bool) {
        self.keys_sorted = keys_sorted;
    }

    /// [`set_keys_sorted`](MapField::set_keys_sorted), chainable.
    pub fn with_keys_sorted(mut self, keys_sorted: bool) -> Self {
        self.keys_sorted = keys_sorted;
        self
    }

    /// The map's name, if set.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Sets the map's name.
    pub fn set_name(&mut self, name: &str) {
        self.name = Some(name.into());
    }

    /// [`set_name`](MapField::set_name), chainable.
    pub fn with_name(mut self, name: &str) -> Self {
        self.set_name(name);
        self
    }

    /// Whether the map admits null entries (a null map, distinct from an empty one).
    pub fn nullable(&self) -> bool {
        self.nullable
    }

    /// Sets whether the map admits null entries.
    pub fn set_nullable(&mut self, nullable: bool) {
        self.nullable = nullable;
    }

    /// [`set_nullable`](MapField::set_nullable), chainable.
    pub fn with_nullable(mut self, nullable: bool) -> Self {
        self.nullable = nullable;
        self
    }

    /// The free-form metadata map (borrowed).
    pub fn metadata(&self) -> &Headers {
        &self.metadata
    }

    /// The free-form metadata map (mutable) — annotate the map with any header.
    pub fn metadata_mut(&mut self) -> &mut Headers {
        &mut self.metadata
    }

    /// The map's [`DataTypeId`] — always [`Map`](DataTypeId::Map).
    pub fn data_type_id(&self) -> DataTypeId {
        DataTypeId::Map
    }
}
